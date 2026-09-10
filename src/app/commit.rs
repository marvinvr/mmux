//! AI-assisted and scheduled commits. Message generation is deliberately an
//! external, bounded, cancellable job: mmux feeds a capped diff to an installed
//! Codex or Claude CLI, never grants it write access, and owns the actual git
//! mutation after a one-line subject comes back.

use super::git::first_line;
use super::overlay::{Overlay, PromptKind};
use super::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const GENERATION_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const SCHEDULE_DELAYS: [(&str, Duration); 7] = [
    ("15m", Duration::from_secs(15 * 60)),
    ("30m", Duration::from_secs(30 * 60)),
    ("1h", Duration::from_secs(60 * 60)),
    ("2h", Duration::from_secs(2 * 60 * 60)),
    ("3h", Duration::from_secs(3 * 60 * 60)),
    ("6h", Duration::from_secs(6 * 60 * 60)),
    ("12h", Duration::from_secs(12 * 60 * 60)),
];

#[derive(Clone, Copy)]
pub(crate) enum ScheduleAction {
    Commit,
    Push,
    Merge,
}

impl ScheduleAction {
    fn label(self) -> &'static str {
        match self {
            ScheduleAction::Commit => "commit",
            ScheduleAction::Push => "commit & push",
            ScheduleAction::Merge => "commit & merge",
        }
    }
}

pub(crate) struct ScheduleForm {
    pub selected: usize,
    pub existing: bool,
    pub worktree: bool,
}

impl ScheduleForm {
    pub(crate) fn labels() -> impl Iterator<Item = &'static str> {
        SCHEDULE_DELAYS.iter().map(|(label, _)| *label)
    }

    fn move_selection(&mut self, delta: i32) {
        self.selected =
            (self.selected as i32 + delta).clamp(0, SCHEDULE_DELAYS.len() as i32 - 1) as usize;
    }
}

pub(crate) struct ScheduledCommit {
    dir: PathBuf,
    due: Instant,
    action: ScheduleAction,
    generation: Option<u64>,
}

pub(crate) struct MessageJob {
    id: u64,
    cancel: Arc<AtomicBool>,
    purpose: MessagePurpose,
}

enum MessagePurpose {
    Prompt {
        dir: PathBuf,
    },
    Submitted {
        dir: PathBuf,
        push: bool,
    },
    Scheduled {
        dir: PathBuf,
        action: ScheduleAction,
    },
}

pub(crate) struct MessageDone {
    id: u64,
    result: Result<String, String>,
}

#[derive(Clone, Copy)]
enum Provider {
    Codex,
    Claude,
}

impl Provider {
    fn executable(self) -> &'static str {
        match self {
            Provider::Codex => "codex",
            Provider::Claude => "claude",
        }
    }

    fn command(self, dir: &Path) -> Command {
        let mut cmd = Command::new(self.executable());
        cmd.current_dir(dir);
        match self {
            // Luna is OpenAI's cost-sensitive tier. Read-only + ephemeral prevents a
            // commit-message request from turning into an agent session or touching files.
            Provider::Codex => {
                cmd.args([
                    "exec",
                    "--model",
                    "gpt-5.6-luna",
                    "--sandbox",
                    "read-only",
                    "--ephemeral",
                    "--skip-git-repo-check",
                    "--color",
                    "never",
                    "-",
                ]);
            }
            // `haiku` is Anthropic's rolling cheap-model alias, rather than a dated
            // snapshot. The prompt already contains everything it needs.
            Provider::Claude => {
                cmd.args([
                    "--print",
                    "--model",
                    "haiku",
                    "--no-session-persistence",
                    "--permission-mode",
                    "dontAsk",
                    "--tools",
                    "",
                    "--output-format",
                    "text",
                ]);
            }
        }
        cmd
    }
}

impl App {
    /// `c`: open the ordinary editable prompt and, when a supported CLI exists,
    /// begin filling it in. The overlay carries the job id so typing can kill the
    /// exact child and a late response can never overwrite user text.
    pub(crate) fn git_commit_prompt(&mut self) {
        let dir = self.active_git().map(|g| g.dir.clone());
        let generation = dir.as_ref().and_then(|dir| {
            let has_changes = self.active_git().is_some_and(|g| !g.files.is_empty());
            has_changes
                .then(|| {
                    self.start_message_job(dir.clone(), MessagePurpose::Prompt { dir: dir.clone() })
                })
                .flatten()
        });
        self.overlay = Some(Overlay::commit(generation));
    }

    pub(crate) fn cancel_message_generation(&mut self, id: u64) {
        if let Some(pos) = self.message_jobs.iter().position(|job| job.id == id) {
            self.message_jobs[pos].cancel.store(true, Ordering::Relaxed);
            self.message_jobs.remove(pos);
        }
    }

    /// Empty submit means “use the suggestion and commit as soon as it arrives”.
    /// No second model call is launched; the prompt job simply changes ownership.
    pub(crate) fn submit_pending_generation(&mut self, id: u64, push: bool) {
        let Some(job) = self.message_jobs.iter_mut().find(|job| job.id == id) else {
            return;
        };
        let dir = match &job.purpose {
            MessagePurpose::Prompt { dir } => dir.clone(),
            _ => return,
        };
        job.purpose = MessagePurpose::Submitted { dir, push };
        self.flash(if push {
            "generating message, then committing & pushing…"
        } else {
            "generating message, then committing…"
        });
    }

    /// `S`: select a delay and the action that should follow the generated commit.
    /// Opening the form again also exposes cancellation for the active project.
    pub(crate) fn git_schedule_prompt(&mut self) {
        let dir = self.projects[self.active].dir.clone();
        let existing = self.scheduled_commits.iter().find(|s| s.dir == dir);
        let selected = existing
            .map(|s| {
                let left = s.due.saturating_duration_since(Instant::now());
                let mut best = (0, Duration::MAX);
                for (i, (_, delay)) in SCHEDULE_DELAYS.iter().enumerate() {
                    let distance = if *delay >= left {
                        *delay - left
                    } else {
                        left - *delay
                    };
                    if distance < best.1 {
                        best = (i, distance);
                    }
                }
                best.0
            })
            .unwrap_or(0);
        let existing = existing.is_some();
        self.overlay = Some(Overlay::Schedule(ScheduleForm {
            selected,
            existing,
            worktree: self.active_is_worktree(),
        }));
    }

    pub(crate) fn schedule_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Schedule(mut form)) = self.overlay.take() else {
            return;
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => {}
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                form.move_selection(-1);
                self.overlay = Some(Overlay::Schedule(form));
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') => {
                form.move_selection(1);
                self.overlay = Some(Overlay::Schedule(form));
            }
            KeyCode::Enter => self.set_schedule(form.selected, ScheduleAction::Push),
            KeyCode::Char('c') => self.set_schedule(form.selected, ScheduleAction::Commit),
            KeyCode::Char('m') if form.worktree => {
                self.set_schedule(form.selected, ScheduleAction::Merge)
            }
            KeyCode::Char('x') if form.existing => self.cancel_active_schedule(),
            _ => self.overlay = Some(Overlay::Schedule(form)),
        }
    }

    fn set_schedule(&mut self, selected: usize, action: ScheduleAction) {
        let Some((label, delay)) = SCHEDULE_DELAYS.get(selected).copied() else {
            return;
        };
        let dir = self.projects[self.active].dir.clone();
        self.cancel_schedule_for(&dir);
        self.scheduled_commits.push(ScheduledCommit {
            dir,
            due: Instant::now() + delay,
            action,
            generation: None,
        });
        self.flash(format!("{} scheduled in {label}", action.label()));
    }

    fn cancel_active_schedule(&mut self) {
        let dir = self.projects[self.active].dir.clone();
        if self.cancel_schedule_for(&dir) {
            self.flash("scheduled commit cancelled");
        }
    }

    fn cancel_schedule_for(&mut self, dir: &Path) -> bool {
        let Some(pos) = self.scheduled_commits.iter().position(|s| s.dir == dir) else {
            return false;
        };
        if let Some(id) = self.scheduled_commits[pos].generation {
            self.cancel_message_generation(id);
        }
        self.scheduled_commits.remove(pos);
        true
    }

    pub(crate) fn active_schedule_label(&self) -> Option<String> {
        let dir = &self.projects[self.active].dir;
        let scheduled = self.scheduled_commits.iter().find(|s| &s.dir == dir)?;
        if scheduled.generation.is_some() {
            return Some("generating…".to_string());
        }
        let left = scheduled.due.saturating_duration_since(Instant::now());
        let secs = left.as_secs();
        Some(if secs < 60 * 60 {
            format!("scheduled {}m", secs.div_ceil(60).max(1))
        } else {
            format!("scheduled {}h", secs.div_ceil(60 * 60))
        })
    }

    /// Tick-owned state machine: deadlines stage everything and start generation;
    /// completed suggestions either fill the open prompt or perform the deferred git
    /// action. All model and network waiting stays off the UI thread.
    pub(crate) fn step_commit_automation(&mut self) {
        self.start_due_schedules();
        while let Ok(done) = self.message_rx.try_recv() {
            let Some(pos) = self.message_jobs.iter().position(|job| job.id == done.id) else {
                continue; // cancelled jobs may still race one final channel send
            };
            let job = self.message_jobs.remove(pos);
            match job.purpose {
                MessagePurpose::Prompt { .. } => {
                    let mut error = None;
                    if let Some(Overlay::Prompt {
                        buf,
                        kind: PromptKind::Commit { .. },
                        generation,
                        ..
                    }) = &mut self.overlay
                    {
                        if *generation == Some(done.id) {
                            *generation = None;
                            match done.result {
                                Ok(msg) if buf.is_empty() => *buf = msg,
                                Ok(_) => {}
                                Err(e) => error = Some(e),
                            }
                        }
                    }
                    if let Some(e) = error {
                        self.flash(format!("message generation failed — {}", first_line(&e)));
                    }
                }
                MessagePurpose::Submitted { dir, push } => match done.result {
                    Ok(msg) => self.finish_generated_commit(
                        &dir,
                        &msg,
                        if push {
                            ScheduleAction::Push
                        } else {
                            ScheduleAction::Commit
                        },
                    ),
                    Err(e) => self.flash(format!("message generation failed — {}", first_line(&e))),
                },
                MessagePurpose::Scheduled { dir, action } => {
                    self.scheduled_commits
                        .retain(|s| s.generation != Some(done.id));
                    match done.result {
                        Ok(msg) => self.finish_generated_commit(&dir, &msg, action),
                        Err(e) => {
                            self.flash(format!("scheduled commit failed — {}", first_line(&e)))
                        }
                    }
                }
            }
        }
    }

    fn start_due_schedules(&mut self) {
        let now = Instant::now();
        let due: Vec<(PathBuf, ScheduleAction)> = self
            .scheduled_commits
            .iter()
            .filter(|s| s.generation.is_none() && now >= s.due)
            .map(|s| (s.dir.clone(), s.action))
            .collect();
        for (dir, action) in due {
            if installed_providers().is_empty() {
                self.cancel_schedule_for(&dir);
                self.flash(
                    "scheduled commit skipped — install Codex or Claude to generate a message",
                );
                continue;
            }
            if crate::git::status(&dir).files.is_empty() {
                self.cancel_schedule_for(&dir);
                self.flash("scheduled commit skipped — nothing changed");
                continue;
            }
            if let Err(e) = crate::git::stage_all(&dir) {
                self.cancel_schedule_for(&dir);
                self.flash(format!("scheduled commit failed — {}", first_line(&e)));
                continue;
            }
            let purpose = MessagePurpose::Scheduled {
                dir: dir.clone(),
                action,
            };
            let Some(id) = self.start_message_job(dir.clone(), purpose) else {
                self.cancel_schedule_for(&dir);
                self.flash(
                    "scheduled commit skipped — install Codex or Claude to generate a message",
                );
                continue;
            };
            if let Some(s) = self.scheduled_commits.iter_mut().find(|s| s.dir == dir) {
                s.generation = Some(id);
            }
        }
    }

    fn finish_generated_commit(&mut self, dir: &Path, msg: &str, action: ScheduleAction) {
        let Some(pi) = self.projects.iter().position(|p| p.dir == dir) else {
            return;
        };
        let st = crate::git::status(dir);
        if !st.files.iter().any(|f| f.staged) {
            if let Err(e) = crate::git::stage_all(dir) {
                self.flash(first_line(&e));
                return;
            }
        }
        let committed = match crate::git::commit(dir, msg) {
            Ok(s) => s,
            Err(e) => {
                self.flash(first_line(&e));
                return;
            }
        };
        if let Some(g) = self.projects[pi].git.as_mut() {
            g.refresh();
        }
        match action {
            ScheduleAction::Commit => self.flash(first_line(&committed)),
            ScheduleAction::Push => {
                if let Some(g) = self.projects[pi].git.as_mut() {
                    g.start_or_queue_push();
                }
                self.flash(format!("{} · pushing…", first_line(&committed)));
            }
            ScheduleAction::Merge => self.merge_worktree_scheduled(pi),
        }
    }

    fn start_message_job(&mut self, dir: PathBuf, purpose: MessagePurpose) -> Option<u64> {
        let providers = installed_providers();
        if providers.is_empty() {
            return None;
        }
        let id = self.next_message_job;
        self.next_message_job = self.next_message_job.wrapping_add(1).max(1);
        let cancel = Arc::new(AtomicBool::new(false));
        self.message_jobs.push(MessageJob {
            id,
            cancel: cancel.clone(),
            purpose,
        });
        spawn_message_job(self.message_tx.clone(), id, dir, providers, cancel);
        Some(id)
    }
}

fn installed_providers() -> Vec<Provider> {
    // Deterministic when both are installed; an invocation/auth failure falls through
    // to the other CLI. Each provider uses its cheap non-snapshot model name above.
    [Provider::Claude, Provider::Codex]
        .into_iter()
        .filter(|provider| executable_exists(provider.executable()))
        .collect()
}

fn executable_exists(name: &str) -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|dir| {
            let path = dir.join(name);
            path.is_file()
        })
    })
}

fn spawn_message_job(
    tx: Sender<MessageDone>,
    id: u64,
    dir: PathBuf,
    providers: Vec<Provider>,
    cancel: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let isolated = env::temp_dir().join(format!("mmux-commit-{}-{id}", std::process::id()));
        let result = crate::git::commit_context(&dir).and_then(|context| {
            std::fs::create_dir(&isolated)
                .map_err(|e| format!("couldn't create isolated generator directory — {e}"))?;
            let prompt = format!(
                "Write the git commit subject for the changes below. Return exactly one plain-text line: no quotes, markdown, explanation, or body. Be specific, use the repository's existing style when clear, prefer imperative wording, and stay within 72 characters.\n\n{context}"
            );
            let mut last_error = "no commit-message provider available".to_string();
            for provider in providers {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".to_string());
                }
                match run_provider(provider, &isolated, &prompt, &cancel) {
                    Ok(output) => match clean_subject(&output) {
                        Ok(subject) => return Ok(subject),
                        Err(e) => last_error = e,
                    },
                    Err(e) => last_error = e,
                }
            }
            Err(last_error)
        });
        let _ = std::fs::remove_dir(&isolated);
        let _ = tx.send(MessageDone { id, result });
    });
}

fn run_provider(
    provider: Provider,
    dir: &Path,
    prompt: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut cmd = provider.command(dir);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start {} — {e}", provider.executable()))?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(prompt.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("couldn't prompt {} — {e}", provider.executable()));
        }
    }
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".to_string());
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                if out.status.success() {
                    return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
                }
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let detail = if stderr.trim().is_empty() {
                    &stdout
                } else {
                    &stderr
                };
                return Err(first_line(detail));
            }
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if started.elapsed() >= GENERATION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} timed out", provider.executable()));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn clean_subject(output: &str) -> Result<String, String> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("```"))
        .unwrap_or("")
        .trim_start_matches("Commit message:")
        .trim()
        .trim_matches(|c| matches!(c, '`' | '"' | '\''))
        .trim();
    if line.is_empty() {
        return Err("generator returned an empty message".to_string());
    }
    let mut subject = line.to_string();
    if subject.chars().count() > 72 {
        subject = subject.chars().take(72).collect();
        if let Some((whole_words, _)) = subject.rsplit_once(' ') {
            subject = whole_words.to_string();
        }
    }
    Ok(subject)
}
