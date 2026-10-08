//! Serving the [control socket](crate::control): each request the listener thread
//! hands over is run here, on the UI thread, against the live `App` — drained a
//! bounded batch per [`tick`](super::App::tick).
//!
//! Two rules keep a scripted caller from disturbing the person at the keyboard:
//!
//! - **The cursor never moves.** A control action goes through
//!   [`keep_selection`](App::keep_selection), which puts the selection back on the
//!   same row *by identity* after the session list changes under it, and never takes
//!   focus. Every mutating action flashes a `ctl:` footer note instead, so it's seen.
//! - **Input is paced, never slept.** Typed text and the Enter that submits it are
//!   separate writes ~150 ms apart (an agent's TUI reads a paste followed instantly by
//!   `\r` as a newline in the paste). They wait in the `deferred` queue, flushed each
//!   tick, so the UI thread never blocks.
//!
//! Each tick also samples every agent's [`busy`](Session::busy) signal into
//! `last_working_at`, which is what lets `mmux wait`/`mmux ask` (client-side polling of
//! `status`) tell "finished what I sent" from "hasn't started on it yet".

use super::commit::ScheduleAction;
use super::keymap::{encode_key, parse_key_name};
use super::nav::Nav;
use super::session::{Kind, Session, Status};
use super::{App, Focus};
use crate::control::{
    Cmd, CommitDone, CommitThen, Done, Hello, LastInfo, Listing, NewKind, ProjectInfo, ReadOut,
    ReloadDone, Request, Response, SessionInfo, StatusInfo, WorktreeDone,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Requests run per tick, so a burst of callers can't stall a frame.
const MAX_PER_TICK: usize = 16;
/// Gap between a pasted prompt and the Enter that submits it.
const ENTER_DELAY: Duration = Duration::from_millis(150);
/// Gap between named keys, so `Escape` then `i` isn't read as one `Alt+i`.
const KEY_GAP: Duration = Duration::from_millis(30);
/// `mmux new` refuses callers this deep: an agent may start agents that start agents,
/// but no further (see [`Session::depth`]).
const DEPTH_LIMIT: u32 = 3;
/// How many non-empty screen lines make up a session's status line.
const STATUS_LINES: usize = 5;
/// How far back the status line looks for those lines. Bounded, because `status` is
/// what `mmux wait` polls several times a second.
const STATUS_TAIL: usize = 60;
/// The screen tail `last` falls back to when there's no transcript to read.
const LAST_SCREEN_LINES: usize = 40;
/// A first prompt typed into an agent (one that can't take it as a launch argument)
/// waits until the screen has been still this long — its TUI is up and listening…
const PROMPT_SETTLE: Duration = Duration::from_millis(800);
/// …or this long at most, so an agent that never stops animating still gets it.
const PROMPT_MAX_WAIT: Duration = Duration::from_secs(10);
/// `read`'s default tail length.
const READ_LINES: usize = 200;
/// How far ahead `commit --in` may schedule. The timer dies with mmux anyway; this
/// mostly keeps a typo (`--in 30h` for `30m`) from passing silently.
const MAX_COMMIT_DELAY: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Input waiting for its turn: the bytes go to session `id` once `due` has passed.
pub(crate) struct Deferred {
    id: u64,
    due: Instant,
    bytes: Vec<u8>,
}

/// A first prompt waiting to be typed into a freshly started agent once its screen
/// settles (see [`PROMPT_SETTLE`]).
pub(crate) struct PendingPrompt {
    id: u64,
    text: String,
    since: Instant,
    /// Fingerprint of the screen last tick, and since when it has looked like that.
    screen: u64,
    still_since: Instant,
}

/// Where the cursor was before a control action, by identity rather than position.
enum Anchor {
    Session(u64),
    /// A launcher row, by its project's directory: removing a worktree shifts indices.
    Launcher(PathBuf, Nav),
    Row(Nav),
}

type Reply = Result<Value, String>;

impl App {
    /// What an agent launched now in project `pi` is told about running inside mmux
    /// ([`crate::agent::mmux_note`], naming the worktree it's in) — `None` unless its
    /// pane could act on it: this session serves the socket and `control.from-panes`
    /// lets panes use it.
    pub(super) fn mmux_note(&self, pi: usize) -> Option<String> {
        if self.control.is_none() || !self.root_cfg().control_from_panes() {
            return None;
        }
        let base = self.worktree_base(pi);
        let worktree = self.projects[pi]
            .worktree
            .as_ref()
            .map(|wt| (wt.branch.as_str(), base.as_deref()));
        Some(crate::agent::mmux_note(worktree))
    }

    /// Run the control requests waiting on the socket, then release any input that
    /// is due. Called from [`tick`](super::App::tick).
    pub(crate) fn serve_control(&mut self) {
        self.track_activity();
        for _ in 0..MAX_PER_TICK {
            let Some(incoming) = self.control.as_ref().and_then(|s| s.try_recv()) else {
                break;
            };
            if incoming.expired() {
                continue;
            }
            let resp = match self.handle_control(&incoming.req) {
                Ok(data) => Response::ok(data),
                Err(e) => Response::err(e),
            };
            let _ = incoming.reply.send(resp);
        }
        self.deliver_pending_prompts();
        self.flush_deferred_input();
    }

    /// Stamp every agent seen working this tick — the sidebar spinner's own predicate.
    fn track_activity(&mut self) {
        let now = Instant::now();
        for s in &mut self.sessions {
            if s.kind == Kind::Agent && s.busy() {
                s.last_working_at = Some(now);
            }
        }
    }

    /// Type each waiting first prompt once its agent's screen has drawn something and
    /// then held still for [`PROMPT_SETTLE`] (or [`PROMPT_MAX_WAIT`] has passed). A
    /// prompt whose agent has gone or died is dropped.
    fn deliver_pending_prompts(&mut self) {
        if self.prompts.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut ready: Vec<(u64, String)> = Vec::new();
        let sessions = &self.sessions;
        self.prompts.retain_mut(|p| {
            let Some(pane) = sessions
                .iter()
                .find(|s| s.id == p.id)
                .and_then(|s| s.pane.as_ref())
                .filter(|pane| pane.is_running())
            else {
                return false;
            };
            let screen = pane
                .with_screen(|s| {
                    let text = s.contents();
                    match text.trim().is_empty() {
                        true => 0,
                        false => {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            text.hash(&mut h);
                            h.finish().max(1)
                        }
                    }
                })
                .unwrap_or(0);
            if screen != p.screen {
                p.screen = screen;
                p.still_since = now;
            }
            let settled = screen != 0 && now.duration_since(p.still_since) >= PROMPT_SETTLE;
            if settled || now.duration_since(p.since) >= PROMPT_MAX_WAIT {
                ready.push((p.id, std::mem::take(&mut p.text)));
                return false;
            }
            true
        });
        for (id, text) in ready {
            if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
                self.type_input(i, &text, true);
            }
        }
    }

    fn handle_control(&mut self, req: &Request) -> Reply {
        if req.caller.is_some()
            && !matches!(req.cmd, Cmd::Hello)
            && !self.root_cfg().control_from_panes()
        {
            return Err(
                "control from inside mmux panes is off here (`control: from-panes: false`)".into(),
            );
        }
        match &req.cmd {
            Cmd::Hello => to_value(Hello {
                version: env!("CARGO_PKG_VERSION").to_string(),
                pid: std::process::id(),
                root: self.root.to_string_lossy().into_owned(),
                projects: self
                    .projects
                    .iter()
                    .map(|p| p.dir.to_string_lossy().into_owned())
                    .collect(),
            }),
            Cmd::Ls => to_value(self.listing()),
            Cmd::Status { target } => {
                let i = self.resolve_target(req, target)?;
                to_value(StatusInfo {
                    session: self.session_info(i),
                    status_line: status_line(&self.sessions[i]),
                })
            }
            Cmd::Last { target } => {
                let i = self.resolve_target(req, target)?;
                let s = &self.sessions[i];
                to_value(LastInfo {
                    id: s.handle(),
                    name: s.name.clone(),
                    tool: s.agent.as_ref().map(|r| r.tool),
                    session_id: s.agent.as_ref().and_then(|r| r.id.clone()),
                    cwd: s.recipe.cwd.to_string_lossy().into_owned(),
                    transcripts: s
                        .agent
                        .as_ref()
                        .and_then(|r| crate::agent::session_root(r.tool, &s.recipe.env))
                        .map(|p| p.to_string_lossy().into_owned()),
                    screen: s
                        .pane
                        .as_ref()
                        .and_then(|p| p.text_tail(LAST_SCREEN_LINES))
                        .unwrap_or_default(),
                })
            }
            Cmd::Read { target, lines } => {
                let i = self.resolve_target(req, target)?;
                let s = &self.sessions[i];
                let text = s
                    .pane
                    .as_ref()
                    .and_then(|p| p.text_tail(lines.unwrap_or(READ_LINES)))
                    .unwrap_or_default();
                to_value(ReadOut {
                    id: s.handle(),
                    name: s.name.clone(),
                    text,
                })
            }
            Cmd::Send {
                target,
                text,
                enter,
            } => {
                let i = self.resolve_target(req, target)?;
                self.control_send(i, text, *enter)
            }
            Cmd::Keys { target, keys } => {
                let i = self.resolve_target(req, target)?;
                self.control_keys(i, keys)
            }
            Cmd::New {
                kind,
                template,
                project,
                command,
                prompt,
            } => self.control_new(
                req,
                *kind,
                template.as_deref(),
                project.as_deref(),
                command.as_deref(),
                prompt.as_deref(),
            ),
            Cmd::Start { target } => {
                let i = self.resolve_target(req, target)?;
                if self.sessions[i].is_running() {
                    return Err(format!(
                        "“{}” is already running — use `mmux restart`",
                        self.sessions[i].name
                    ));
                }
                self.keep_selection(|app| app.start_session(i));
                self.control_done(i, "started")
            }
            Cmd::Restart { target } => {
                let i = self.resolve_target(req, target)?;
                self.keep_selection(|app| app.start_session(i));
                self.control_done(i, "restarted")
            }
            Cmd::Stop { target, force } | Cmd::Close { target, force } => {
                let i = self.resolve_target(req, target)?;
                // An agent closing itself is always "working" — it's running this very
                // command — so the busy guard would only ever stand in its way.
                let own = self.caller_index(req) == Some(i);
                self.control_close(i, *force || own)
            }
            Cmd::WorktreeNew {
                branch,
                project,
                agent,
                prompt,
            } => self.control_worktree_new(
                req,
                branch.as_deref(),
                project.as_deref(),
                agent.as_deref(),
                prompt.as_deref(),
            ),
            Cmd::WorktreeRm {
                target,
                force,
                confirm,
            } => self.control_worktree_rm(req, target, *force, *confirm),
            Cmd::Commit {
                project,
                message,
                then,
                delay_ms,
            } => self.control_commit(
                req,
                project.as_deref(),
                message.as_deref(),
                *then,
                *delay_ms,
            ),
            Cmd::CommitCancel { project } => {
                let pi = self.target_project(req, project.as_deref())?;
                let name = self.projects[pi].label();
                if !self.cancel_schedule_for(&self.projects[pi].dir.clone()) {
                    return Err(format!("no commit is scheduled in {name}"));
                }
                let message = format!("cancelled the scheduled commit in {name}");
                self.flash(format!("ctl: {message}"));
                to_value(CommitDone {
                    project: self.project_info(pi),
                    message,
                })
            }
            Cmd::Reload => {
                let r = self.keep_selection(|app| app.reload());
                // Whatever did load is applied either way; the caller most likely just
                // edited a config, so a broken one is the answer it needs.
                if !r.errors.is_empty() {
                    return Err(format!("{}\n{}", r.message, r.errors.join("\n")));
                }
                to_value(ReloadDone { message: r.message })
            }
        }
    }

    /// Every project (in sidebar order) and its sessions (agents, terminals, processes).
    fn listing(&self) -> Listing {
        let order = self.project_display_order();
        let mut sessions = Vec::new();
        for &pi in &order {
            for kind in [Kind::Agent, Kind::Terminal, Kind::Process] {
                for (i, nest) in self.section_sessions(pi, kind) {
                    sessions.push(SessionInfo {
                        nest,
                        ..self.session_info(i)
                    });
                }
            }
        }
        let projects = order.iter().map(|&pi| self.project_info(pi)).collect();
        Listing {
            name: self.root_cfg().display_name(),
            root: self.root.to_string_lossy().into_owned(),
            projects,
            sessions,
        }
    }

    fn project_info(&self, pi: usize) -> ProjectInfo {
        let p = &self.projects[pi];
        ProjectInfo {
            name: p.label(),
            dir: p.dir.to_string_lossy().into_owned(),
            active: pi == self.active,
            agents: p.cfg.agents.iter().map(|a| a.name.clone()).collect(),
            worktree_of: p
                .worktree
                .as_ref()
                .map(|w| w.parent.to_string_lossy().into_owned()),
            scheduled_commit: self.scheduled_info(&p.dir),
        }
    }

    fn session_info(&self, i: usize) -> SessionInfo {
        let s = &self.sessions[i];
        let now = Instant::now();
        let working = s.kind == Kind::Agent && s.busy();
        let ms = |t: Instant| now.duration_since(t).as_millis() as u64;
        let quiet_since = s.last_working_at.max(s.launched_at);
        SessionInfo {
            id: s.handle(),
            kind: match s.kind {
                Kind::Agent => "agent",
                Kind::Terminal => "terminal",
                Kind::Process => "process",
            }
            .to_string(),
            name: s.name.clone(),
            project: self.projects[s.project].label(),
            project_dir: self.projects[s.project].dir.to_string_lossy().into_owned(),
            status: match s.status() {
                Status::Stopped => "stopped",
                Status::Running => "running",
                Status::Exited => "exited",
                Status::Failed => "failed",
            }
            .to_string(),
            // The sidebar spinner's own predicate, so "working" means what you see.
            working,
            attention: s.attention(),
            title: s.pane.as_ref().map(|p| p.title()).filter(|t| !t.is_empty()),
            error: s.error.clone(),
            idle_for_ms: match working {
                true => Some(0),
                false => quiet_since.map(ms),
            },
            input_age_ms: s.last_input_at.map(ms),
            worked_since_input: match (s.last_input_at, s.last_working_at) {
                (Some(input), Some(work)) => work > input,
                _ => false,
            },
            input_pending: self.deferred.iter().any(|d| d.id == s.id)
                || self.prompts.iter().any(|p| p.id == s.id),
            parent: s
                .parent
                .and_then(|p| self.sessions.iter().find(|c| c.id == p))
                .map(Session::handle),
            nest: 0,
        }
    }

    /// The reply for a mutating action on session `i`, with its footer note.
    fn control_done(&mut self, i: usize, verb: &str) -> Reply {
        let message = format!("{verb} {}", self.sessions[i].name);
        self.flash(format!("ctl: {message}"));
        to_value(Done {
            session: self.session_info(i),
            message,
        })
    }

    fn control_send(&mut self, i: usize, text: &str, enter: bool) -> Reply {
        if !self.sessions[i]
            .pane
            .as_ref()
            .is_some_and(|p| p.is_running())
        {
            return Err(format!(
                "“{}” is not running — `mmux start` it first",
                self.sessions[i].name
            ));
        }
        self.type_input(i, text, enter);
        self.control_done(i, "sent input to")
    }

    /// Paste `text` into session `i` the way a real terminal would, then press Enter
    /// [`ENTER_DELAY`] later as its own write. No-op without a pane.
    fn type_input(&mut self, i: usize, text: &str, enter: bool) {
        let Some(pane) = self.sessions[i].pane.as_ref() else {
            return;
        };
        let id = self.sessions[i].id;
        if !text.is_empty() {
            let bytes = pane.paste_input(text);
            self.queue_input(id, Duration::ZERO, bytes);
        }
        if enter {
            self.queue_input(id, ENTER_DELAY, b"\r".to_vec());
        }
        self.sessions[i].last_input_at = Some(Instant::now());
        self.flush_deferred_input();
    }

    fn control_keys(&mut self, i: usize, keys: &[String]) -> Reply {
        let Some(pane) = self.sessions[i].pane.as_ref().filter(|p| p.is_running()) else {
            return Err(format!(
                "“{}” is not running — `mmux start` it first",
                self.sessions[i].name
            ));
        };
        let flags = pane.kitty_flags();
        let id = self.sessions[i].id;
        let mut gap = Duration::ZERO;
        for key in keys {
            // Like tmux send-keys: a key name is pressed, anything else is typed.
            let bytes = match parse_key_name(key) {
                Some(k) => encode_key(&k, flags),
                None => key.clone().into_bytes(),
            };
            if !bytes.is_empty() {
                self.queue_input(id, gap, bytes);
                gap = KEY_GAP;
            }
        }
        // Only a submitting key makes this input an agent should answer: an Escape or
        // an arrow must not set `wait` looking for work it will never do.
        let submits =
            keys.iter()
                .filter_map(|k| parse_key_name(k))
                .any(|k| match (k.code, k.modifiers) {
                    (KeyCode::Enter, KeyModifiers::NONE) => true,
                    (KeyCode::Char(c), KeyModifiers::CONTROL) => matches!(c, 'm' | 'j' | 'M' | 'J'),
                    _ => false,
                });
        if submits {
            self.sessions[i].last_input_at = Some(Instant::now());
        }
        self.flush_deferred_input();
        self.control_done(i, "sent keys to")
    }

    fn control_new(
        &mut self,
        req: &Request,
        kind: NewKind,
        template: Option<&str>,
        project: Option<&str>,
        command: Option<&str>,
        prompt: Option<&str>,
    ) -> Reply {
        check_depth(req)?;
        let prompt = prompt.filter(|p| !p.trim().is_empty());
        if prompt.is_some() && kind != NewKind::Agent {
            return Err("--prompt is for agents — use --cmd for a terminal".into());
        }
        // Typed with its Enter straight into a booting agent, a command would be
        // submitted as a prompt: refuse rather than guess.
        if command.is_some_and(|c| !c.trim().is_empty()) && kind != NewKind::Terminal {
            return Err("--cmd is for terminals — use --prompt for an agent".into());
        }
        let pi = self.target_project(req, project)?;
        let i = self.spawn_new(req, pi, kind, template, command, prompt)?;
        self.control_done(i, "started")
    }

    /// Start a new agent or terminal in project `pi` for a control caller, returning
    /// its index — shared by `new` and `worktree new`.
    fn spawn_new(
        &mut self,
        req: &Request,
        pi: usize,
        kind: NewKind,
        template: Option<&str>,
        command: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<usize, String> {
        let mut s = match kind {
            NewKind::Agent => {
                let t = self.agent_template(pi, template)?;
                self.new_agent_session(pi, t)
            }
            NewKind::Terminal => self.new_terminal_session(pi),
        };
        s.depth = req.depth + 1;
        // Remember who asked, so the sidebar can nest the new row under its spawner.
        s.parent = req
            .caller
            .as_deref()
            .and_then(parse_handle)
            .filter(|id| self.sessions.iter().any(|c| c.id == *id));
        let id = s.id;
        // A detected agent (Claude/Codex/Pi/Grok) takes the prompt on its command line
        // (first launch only); any other has it typed in once its screen settles.
        let typed = match prompt {
            Some(p) if s.agent.is_some() => {
                s.first_prompt = Some(p.to_string());
                None
            }
            other => other,
        };
        self.keep_selection(|app| app.launch_session(s));
        let i = self.sessions.len() - 1;
        if let Some(text) = typed {
            let now = Instant::now();
            self.prompts.push(PendingPrompt {
                id,
                text: text.to_string(),
                since: now,
                screen: 0,
                still_since: now,
            });
        }
        if let Some(cmd) = command.map(str::trim).filter(|c| !c.is_empty()) {
            // Typed into the shell rather than run via `sh -c`, so the terminal (and
            // the command's output) stays after it finishes. The PTY holds it until
            // the shell reads its first line.
            self.queue_input(id, Duration::ZERO, format!("{cmd}\r").into_bytes());
            self.flush_deferred_input();
        }
        if let Some(e) = self.sessions[i].error.clone() {
            return Err(format!("“{}” failed to start: {e}", self.sessions[i].name));
        }
        Ok(i)
    }

    /// `worktree new`: cut a worktree (the `w` gesture, minus bringing it into view)
    /// and optionally start an agent in it.
    fn control_worktree_new(
        &mut self,
        req: &Request,
        branch: Option<&str>,
        project: Option<&str>,
        agent: Option<&str>,
        prompt: Option<&str>,
    ) -> Reply {
        check_depth(req)?;
        let prompt = prompt.filter(|p| !p.trim().is_empty());
        let root = self.family_root(self.target_project(req, project)?);
        let with_agent = agent.is_some() || prompt.is_some();
        // A bad template is refused before anything is cut, so it leaves no checkout
        // behind. (It's resolved again in the worktree, whose config is its own.)
        if with_agent {
            self.agent_template(root, agent)?;
        }
        let branch = match branch.map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => b.to_string(),
            None => {
                let taken: Vec<String> = crate::git::branches(&self.projects[root].dir)
                    .into_iter()
                    .map(|b| b.name)
                    .collect();
                crate::worktree::generate_name(&taken)
            }
        };
        let cut = self.keep_selection(|app| app.cut_worktree(root, &branch))?;
        let mut message = format!("created ⑂ {}{}", cut.branch, cut.copied_note());
        if cut.setup.is_some() {
            message.push_str(" · running its setup");
        }
        let started = match with_agent {
            true => match self.spawn_new(req, cut.pi, NewKind::Agent, agent, None, prompt) {
                Ok(i) => Some(i),
                Err(e) => {
                    self.flash(format!("ctl: {message}"));
                    return Err(format!("{message}, but its agent didn't start: {e}"));
                }
            },
            false => None,
        };
        if let Some(i) = started {
            message.push_str(&format!(" · started {}", self.sessions[i].name));
        }
        self.flash(format!("ctl: {message}"));
        to_value(WorktreeDone {
            project: self.project_info(cut.pi),
            branch: cut.branch,
            message,
            agent: started.map(|i| self.session_info(i)),
        })
    }

    /// The session whose pane sent `req`, if it came from one of ours.
    fn caller_index(&self, req: &Request) -> Option<usize> {
        let id = req.caller.as_deref().and_then(parse_handle)?;
        self.sessions.iter().position(|s| s.id == id)
    }

    /// `worktree rm`: the `X` gesture. Without `force` it refuses whatever the modal
    /// would have warned about — uncommitted changes, an agent still at work — and
    /// the worktree the human is looking at. The caller's own worktree also needs
    /// `confirm`: removing it closes the pane asking, so the first try only explains
    /// that, and names the flag that goes through with it.
    fn control_worktree_rm(
        &mut self,
        req: &Request,
        target: &str,
        force: bool,
        confirm: bool,
    ) -> Reply {
        let (pi, branch) = self.target_worktree(target)?;
        let caller = self.caller_index(req);
        let own = caller.is_some_and(|i| self.sessions[i].project == pi);
        if !force {
            if !crate::git::is_clean(&self.projects[pi].dir) {
                return Err(format!(
                    "⑂ {branch} has uncommitted changes — commit them, or pass --force to discard them"
                ));
            }
            let working: Vec<String> = self
                .sessions
                .iter()
                .enumerate()
                // The caller is busy running this command; that's no reason to refuse.
                .filter(|&(i, s)| {
                    s.project == pi && s.kind == Kind::Agent && s.busy() && Some(i) != caller
                })
                .map(|(_, s)| format!("{} {}", s.handle(), s.name))
                .collect();
            if !working.is_empty() {
                return Err(format!(
                    "⑂ {branch} has agents at work ({}) — wait for them, or pass --force",
                    working.join(", ")
                ));
            }
            if pi == self.active {
                return Err(format!(
                    "⑂ {branch} is the project in view — pass --force to remove it anyway"
                ));
            }
        }
        if own && !confirm {
            let others = self
                .sessions
                .iter()
                .enumerate()
                .filter(|&(i, s)| s.project == pi && Some(i) != caller)
                .count();
            let others = match others {
                0 => String::new(),
                1 => " and the 1 other session in it".to_string(),
                n => format!(" and the {n} other sessions in it"),
            };
            let dirty = match crate::git::is_clean(&self.projects[pi].dir) {
                true => "",
                false => " Its uncommitted changes are discarded.",
            };
            return Err(format!(
                "not removed: ⑂ {branch} is the worktree you are running in. Removing it closes \
your own pane{others} — this conversation ends there, and nothing you do after it runs — then \
deletes the checkout; the branch goes too, but only if it's merged.{dirty} If its work should \
land, merge it first (`mmux commit --merge`), and finish anything you still owe the user. \
To go ahead, run the same command again with --confirm."
            ));
        }
        let project = self.project_info(pi);
        let message = self
            .keep_selection(|app| app.take_worktree(pi, &branch))
            .ok_or_else(|| format!("⑂ {branch} is gone"))??;
        self.flash(format!("ctl: {message}"));
        to_value(WorktreeDone {
            project,
            branch,
            message,
            agent: None,
        })
    }

    /// `commit`: the git panel's `c` — with the caller's message, or a generated one —
    /// or, given a delay, its `S`. Whatever can be known to fail is refused before
    /// anything is staged.
    fn control_commit(
        &mut self,
        req: &Request,
        project: Option<&str>,
        message: Option<&str>,
        then: CommitThen,
        delay_ms: Option<u64>,
    ) -> Reply {
        let pi = self.target_project(req, project)?;
        let dir = self.projects[pi].dir.clone();
        let name = self.projects[pi].label();
        if !crate::git::is_repo(&dir) {
            return Err(format!("{name} is not a git repository"));
        }
        // The push runs on the git panel's worker; without a panel there is none.
        if then == CommitThen::Push && self.projects[pi].git.is_none() {
            return Err(format!(
                "{name} has its git panel off, and the panel is what pushes — drop --push"
            ));
        }
        if then == CommitThen::Merge && self.projects[pi].worktree.is_none() {
            return Err(format!(
                "{name} is not a worktree — --merge merges a worktree into its base"
            ));
        }
        let action = ScheduleAction::from(then);
        let message = message
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        let text = match delay_ms.map(Duration::from_millis) {
            Some(delay) if delay > MAX_COMMIT_DELAY => {
                return Err(format!(
                    "--in {} is too far ahead — a week at most",
                    crate::ctl::human(delay)
                ));
            }
            // Changes and the merge are judged when the timer fires, as with `S`.
            Some(delay) => {
                let replaced = self.schedule_commit(dir, delay, action, message);
                let mut text = format!(
                    "{} scheduled in {} for {name}",
                    action.label(),
                    crate::ctl::human(delay)
                );
                if replaced {
                    text.push_str(", replacing the previous schedule");
                }
                text
            }
            None => {
                if crate::git::status(&dir).files.is_empty() {
                    return Err(format!("nothing to commit in {name}"));
                }
                if then == CommitThen::Merge {
                    self.merge_target(pi)
                        .map_err(|e| format!("won't commit to merge: {e}"))?;
                }
                match message {
                    Some(msg) => match self.commit_and_follow(&dir, &msg, action) {
                        Ok(done) => done,
                        Err(e) => {
                            self.flash(format!("ctl: {e}"));
                            return Err(e);
                        }
                    },
                    None => {
                        self.generate_and_commit(dir, action)?;
                        format!("generating a message for {name}, then {}…", action.label())
                    }
                }
            }
        };
        self.flash(format!("ctl: {text}"));
        to_value(CommitDone {
            project: self.project_info(pi),
            message: text,
        })
    }

    /// The worktree a user-typed name refers to (its branch, directory or path),
    /// with its branch.
    fn target_worktree(&self, spec: &str) -> Result<(usize, String), String> {
        let found: Vec<(usize, String)> = self
            .projects_matching(spec)
            .into_iter()
            .filter_map(|pi| Some((pi, self.projects[pi].worktree.as_ref()?.branch.clone())))
            .collect();
        match found.as_slice() {
            [one] => Ok(one.clone()),
            [] => {
                let all: Vec<String> = self
                    .projects
                    .iter()
                    .filter_map(|p| p.worktree.as_ref().map(|w| w.branch.clone()))
                    .collect();
                Err(match all.is_empty() {
                    true => format!("no worktree “{spec}” — there are none open"),
                    false => format!("no worktree “{spec}” — one of: {}", all.join(", ")),
                })
            }
            _ => Err(format!("worktree “{spec}” is ambiguous — pass its path")),
        }
    }

    /// `stop`/`close`: a process stops in place (running its `stop:` teardown, like
    /// `x`); an agent or terminal is closed for good — refused while it has live work
    /// (the same signal the close confirmation keys on) unless `force`.
    fn control_close(&mut self, i: usize, force: bool) -> Reply {
        let s = &self.sessions[i];
        if s.kind == Kind::Process {
            let was_running = s.is_running();
            self.sessions[i].stop();
            if was_running {
                self.run_stop_command(i);
            }
            return self.control_done(i, "stopped");
        }
        let busy = match s.kind {
            Kind::Agent => s.busy(),
            _ => s.is_running(),
        };
        if busy && !force {
            let noun = match s.kind {
                Kind::Agent => "is working",
                _ => "is still running",
            };
            return Err(format!(
                "“{}” {noun} — pass --force to close it anyway",
                s.name
            ));
        }
        let mut info = self.session_info(i);
        info.status = "closed".to_string();
        let message = format!("closed {}", self.sessions[i].name);
        self.keep_selection(|app| {
            app.sessions[i].kill();
            app.sessions.remove(i);
        });
        self.flash(format!("ctl: {message}"));
        to_value(Done {
            session: info,
            message,
        })
    }

    /// Run `f` — which may add, remove or restart sessions — and then put the cursor
    /// back on the row it was on, found by identity since positions shift. Focus stays
    /// where it was unless the focused pane itself went away.
    pub(super) fn keep_selection<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let anchor = self.current_nav().map(|n| match n {
            Nav::Session(i) => Anchor::Session(self.sessions[i].id),
            Nav::NewAgent(p, _) | Nav::NewTerminal(p) | Nav::NewProcess(p) => {
                Anchor::Launcher(self.projects[p].dir.clone(), n)
            }
            other => Anchor::Row(other),
        });
        let out = f(self);
        let want = anchor.and_then(|a| match a {
            Anchor::Session(id) => self
                .sessions
                .iter()
                .position(|s| s.id == id)
                .map(Nav::Session),
            Anchor::Launcher(dir, n) => {
                let p = self.projects.iter().position(|p| p.dir == dir)?;
                Some(match n {
                    Nav::NewAgent(_, t) => Nav::NewAgent(p, t),
                    Nav::NewTerminal(_) => Nav::NewTerminal(p),
                    Nav::NewProcess(_) => Nav::NewProcess(p),
                    other => other,
                })
            }
            Anchor::Row(n) => Some(n),
        });
        let nav = self.build_nav();
        match want.and_then(|w| nav.iter().position(|n| *n == w)) {
            Some(pos) => self.sel = pos,
            None => {
                self.sel = self.sel.min(nav.len().saturating_sub(1));
                if self.focus == Focus::Terminal {
                    self.focus = Focus::Sidebar;
                }
            }
        }
        out
    }

    /// Queue `bytes` for session `id`, `gap` after whatever is already queued for it
    /// (or after now), so a caller's writes land in the order it made them.
    fn queue_input(&mut self, id: u64, gap: Duration, bytes: Vec<u8>) {
        let now = Instant::now();
        let after = self
            .deferred
            .iter()
            .filter(|d| d.id == id)
            .map(|d| d.due)
            .max()
            .unwrap_or(now)
            .max(now);
        self.deferred.push(Deferred {
            id,
            due: after + gap,
            bytes,
        });
    }

    /// Write every queued input whose time has come, in order per session. Input for
    /// a session that has since gone is dropped.
    fn flush_deferred_input(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut waiting: Vec<u64> = Vec::new();
        let mut k = 0;
        while k < self.deferred.len() {
            let d = &self.deferred[k];
            if d.due > now || waiting.contains(&d.id) {
                waiting.push(d.id);
                k += 1;
                continue;
            }
            let d = self.deferred.remove(k);
            if let Some(p) = self
                .sessions
                .iter()
                .find(|s| s.id == d.id)
                .and_then(|s| s.pane.as_ref())
            {
                p.send(d.bytes);
            }
        }
    }

    /// Resolve a target to a session index: `self`, `s<N>`, a name (`Claude #2`,
    /// matched ignoring case and spaces, then as a prefix), or `project/name`. Several
    /// matches narrow to the caller's project; if that doesn't settle it, the error
    /// lists them.
    fn resolve_target(&self, req: &Request, target: &str) -> Result<usize, String> {
        let target = target.trim();
        let by_id = |handle: &str| {
            parse_handle(handle).and_then(|id| self.sessions.iter().position(|s| s.id == id))
        };
        if target.eq_ignore_ascii_case("self") {
            let caller = req
                .caller
                .as_deref()
                .ok_or("`self` only works from inside an mmux pane (MMUX_SESSION is not set)")?;
            return by_id(caller).ok_or_else(|| format!("your session {caller} is gone"));
        }
        if parse_handle(target).is_some() {
            return by_id(target).ok_or_else(|| format!("no session {target} — try `mmux ls`"));
        }
        let want = norm(target);
        let mut found: Vec<usize> = self.sessions_named(|n| n == want, None);
        if found.is_empty() {
            if let Some((proj, name)) = target.rsplit_once('/') {
                let projects = self.projects_matching(proj);
                let want = norm(name);
                found = self.sessions_named(|n| n == want, Some(&projects));
                if found.is_empty() {
                    found = self.sessions_named(|n| n.starts_with(&want), Some(&projects));
                }
            }
        }
        if found.is_empty() {
            found = self.sessions_named(|n| n.starts_with(&want), None);
        }
        if found.len() > 1 {
            if let Some(pi) = self.caller_project(req) {
                let mine: Vec<usize> = found
                    .iter()
                    .copied()
                    .filter(|&i| self.sessions[i].project == pi)
                    .collect();
                if mine.len() == 1 {
                    found = mine;
                }
            }
        }
        match found.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("no session matches “{target}” — try `mmux ls`")),
            many => Err(format!(
                "“{target}” is ambiguous: {} — use the id or project/name",
                many.iter()
                    .map(|&i| format!(
                        "{} {} ({})",
                        self.sessions[i].handle(),
                        self.sessions[i].name,
                        self.projects[self.sessions[i].project].label()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Indices of the sessions whose normalized name passes `pred`, optionally only
    /// within `projects`.
    fn sessions_named(
        &self,
        pred: impl Fn(&str) -> bool,
        projects: Option<&[usize]>,
    ) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| projects.is_none_or(|ps| ps.contains(&s.project)))
            .filter(|(_, s)| pred(&norm(&s.name)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Projects a user-typed name refers to: its display name/branch, its directory's
    /// basename, or its path.
    fn projects_matching(&self, spec: &str) -> Vec<usize> {
        let want = norm(spec);
        let path = crate::config::canonical(Path::new(spec));
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                norm(&p.label()) == want
                    || norm(&p.cfg.display_name()) == want
                    || p.dir
                        .file_name()
                        .is_some_and(|b| norm(&b.to_string_lossy()) == want)
                    || p.dir == path
            })
            .map(|(pi, _)| pi)
            .collect()
    }

    /// The project the caller is in: its own pane's project, else the deepest project
    /// containing its working directory.
    fn caller_project(&self, req: &Request) -> Option<usize> {
        if let Some(i) = req
            .caller
            .as_deref()
            .and_then(parse_handle)
            .and_then(|id| self.sessions.iter().position(|s| s.id == id))
        {
            return Some(self.sessions[i].project);
        }
        let cwd = crate::config::canonical(Path::new(req.cwd.as_deref()?));
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| cwd.starts_with(&p.dir))
            .max_by_key(|(_, p)| p.dir.components().count())
            .map(|(pi, _)| pi)
    }

    /// Which project `new` should start in: the named one, else the caller's, else the
    /// only one.
    fn target_project(&self, req: &Request, spec: Option<&str>) -> Result<usize, String> {
        let names = || {
            self.projects
                .iter()
                .map(|p| p.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(spec) = spec {
            return match self.projects_matching(spec).as_slice() {
                [pi] => Ok(*pi),
                [] => Err(format!("no project “{spec}” — one of: {}", names())),
                _ => Err(format!("project “{spec}” is ambiguous — pass its path")),
            };
        }
        if let Some(pi) = self.caller_project(req) {
            return Ok(pi);
        }
        match self.projects.len() {
            1 => Ok(0),
            _ => Err(format!(
                "which project? pass --project (one of: {})",
                names()
            )),
        }
    }

    /// Which of project `pi`'s agent templates `new agent` means: the named one (by
    /// name, or by command like `claude`), else the first.
    fn agent_template(&self, pi: usize, spec: Option<&str>) -> Result<usize, String> {
        let agents = &self.projects[pi].cfg.agents;
        if agents.is_empty() {
            return Err(format!(
                "{} has no agents configured — run `mmux agents`",
                self.projects[pi].label()
            ));
        }
        let Some(spec) = spec else {
            return Ok(0);
        };
        let want = norm(spec);
        agents
            .iter()
            .position(|a| norm(&a.name) == want)
            .or_else(|| {
                agents.iter().position(|a| {
                    Path::new(&a.cmd)
                        .file_name()
                        .is_some_and(|b| norm(&b.to_string_lossy()) == want)
                })
            })
            .ok_or_else(|| {
                format!(
                    "no agent “{spec}” in {} — one of: {}",
                    self.projects[pi].label(),
                    agents
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

/// `new` and `worktree new` refuse callers [`DEPTH_LIMIT`] deep.
fn check_depth(req: &Request) -> Result<(), String> {
    match req.depth >= DEPTH_LIMIT {
        true => Err(format!(
            "refusing to start a session {} levels deep (MMUX_DEPTH) — agents spawning agents stops here",
            req.depth + 1
        )),
        false => Ok(()),
    }
}

/// The last few non-empty lines of a session's buffer — where an agent keeps its own
/// status line and input box. Empty for a session with no pane.
fn status_line(s: &Session) -> Vec<String> {
    let text = s
        .pane
        .as_ref()
        .and_then(|p| p.text_tail(STATUS_TAIL))
        .unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .rev()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .take(STATUS_LINES)
        .map(str::to_string)
        .collect();
    lines.reverse();
    lines
}

/// `s12` → 12.
fn parse_handle(handle: &str) -> Option<u64> {
    handle
        .strip_prefix('s')
        .or_else(|| handle.strip_prefix('S'))?
        .parse()
        .ok()
}

/// A name as matching sees it: case and whitespace ignored, so `claude#2` is `Claude #2`.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn to_value<T: serde::Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_parse_only_as_s_and_digits() {
        assert_eq!(parse_handle("s12"), Some(12));
        assert_eq!(parse_handle("S3"), Some(3));
        assert_eq!(parse_handle("s"), None);
        assert_eq!(parse_handle("shell"), None);
        assert_eq!(parse_handle("12"), None);
    }

    #[test]
    fn names_match_ignoring_case_and_spaces() {
        assert_eq!(norm("Claude #2"), norm("claude#2"));
        assert_eq!(norm("Dev server"), "devserver");
    }
}
