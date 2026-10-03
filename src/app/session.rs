//! The unified pane-backed session model.
//!
//! Agents, plain terminals and defined processes are all the same thing under
//! the hood: a [`Recipe`] (what to run) plus an optional live
//! [`Pane`]. [`Session`] owns the spawn/stop lifecycle so the rest of the app
//! never has to special-case "is this an agent or a process".

use crate::config::{AgentDef, ProcessDef};
use crate::pane::{Notify, Pane};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Source of [`Session::id`]. There is one `App` per process, so a process-wide
/// counter is the app's counter — ids are never reused within a run.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// How long after an agent's terminal title last changed we still count it as
/// "working" when it does not emit explicit OSC 9;4 progress state. This is the
/// fallback window behind [`Session::busy`], shared by the sidebar spinner and
/// the close confirmation so the two never disagree about "is it working".
const TITLE_IDLE: Duration = Duration::from_secs(2);

/// Which sidebar bucket a session belongs to. Drives ordering, the badge, and
/// the placeholder wording — never the lifecycle, which is identical for all.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Terminal,
    Process,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Running,
    Exited,
    /// Exited abnormally on its own (non-zero status, not a deliberate stop).
    /// Surfaced as a red badge for processes; agents/terminals treat it like
    /// `Exited`. See [`crate::pane::Pane::crashed`].
    Failed,
}

/// Everything needed to (re)spawn a pane identically. `PartialEq` lets a live
/// [reload](super::App::reload) tell whether a process's command actually changed
/// (and so needs restarting) rather than just matching it by name.
#[derive(Clone, PartialEq, Eq)]
pub struct Recipe {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl Recipe {
    pub fn agent(def: &AgentDef, dir: &Path) -> Recipe {
        Recipe {
            cmd: def.cmd.clone(),
            args: def.args.clone(),
            cwd: resolve(dir, &def.cwd),
            env: def.env.clone(),
        }
    }

    pub fn process(def: &ProcessDef, dir: &Path) -> Recipe {
        Recipe {
            cmd: def.cmd.clone(),
            args: def.args.clone(),
            cwd: resolve(dir, &def.cwd),
            env: def.env.clone(),
        }
    }

    /// A one-off shell line run in `dir` — a new worktree's
    /// [`setup:`](crate::config::WorktreeConfig::setup) command. It's an ordinary
    /// terminal session, so you watch it work in the main pane and the row prunes
    /// itself the moment it exits cleanly.
    pub fn shell_line(dir: &Path, line: &str) -> Recipe {
        Recipe {
            cmd: "sh".into(),
            args: vec!["-c".into(), line.to_string()],
            cwd: dir.to_path_buf(),
            env: BTreeMap::new(),
        }
    }

    /// A plain login shell rooted at `dir`.
    pub fn shell(dir: &Path) -> Recipe {
        Recipe {
            cmd: default_shell(),
            args: Vec::new(),
            cwd: dir.to_path_buf(),
            env: BTreeMap::new(),
        }
    }

    /// An editor opening `rel` (relative to `dir`): `$VISUAL`/`$EDITOR` if set, else the
    /// first of `micro`/`nano`/`vim`/`vi` on `PATH`. Mirrors the user's Ctrl+P-opens-micro habit.
    pub fn editor(dir: &Path, rel: &str) -> Recipe {
        let (cmd, mut args) = editor_command();
        args.push(rel.to_string());
        Recipe {
            cmd,
            args,
            cwd: dir.to_path_buf(),
            env: BTreeMap::new(),
        }
    }
}

pub struct Session {
    pub name: String,
    pub kind: Kind,
    pub pane: Option<Pane>,
    pub error: Option<String>,
    pub recipe: Recipe,
    /// Index of the workspace project (see [`crate::app`]) this session belongs to.
    /// Drives which sidebar group it lands in; the lifecycle is identical regardless.
    pub project: usize,
    /// Resume bookkeeping for a Claude/Codex/Pi/Grok agent: lets a (re)start reattach to
    /// the same conversation rather than start cold. `None` for terminals,
    /// processes, and any agent that isn't one of the four we support. See
    /// [`crate::agent`] and [`crate::restore`].
    pub agent: Option<crate::agent::Resume>,
    /// Optional teardown command (a shell line) run in `recipe.cwd` after this session's
    /// process stops — on an explicit stop and on quit, but not on a restart. Carried
    /// from a config-defined process's [`stop:`](crate::config::ProcessDef::stop); `None`
    /// for agents, terminals, and processes without one. See [`Session::stop_command`].
    pub stop: Option<String>,
    /// Stable runtime identity, shown as `s<N>`. Unlike the session's index into
    /// `App.sessions` — which shifts whenever a row comes or goes — it names the same
    /// row for the life of the process, which is what the
    /// [control socket](crate::control) addresses. Not persisted: a reopen assigns
    /// fresh ids.
    pub id: u64,
    /// The owning project's canonical directory, exported to the pane as
    /// `MMUX_PROJECT` so a program inside knows which project it runs in.
    pub project_dir: PathBuf,
    /// How many control hops deep this session was created: 1 for anything the user or
    /// the config started, the caller's depth + 1 for a `mmux new` issued from inside a
    /// pane. Exported as `MMUX_DEPTH` — the brake on agents spawning agents unboundedly.
    pub depth: u32,
    /// When input last arrived through the control socket (`mmux send`, `mmux keys`
    /// with a submitting key, or a first prompt). Lets a caller tell "working on what I sent" from "was already
    /// working before".
    pub last_input_at: Option<Instant>,
    /// When this agent was last seen [`busy`](Self::busy), sampled every tick. Against
    /// `last_input_at` it answers "has it worked on my input yet?" — what `mmux wait`
    /// and `mmux ask` need to tell "done" from "not started".
    pub last_working_at: Option<Instant>,
    /// When the pane was last spawned — the start of its idle clock before it ever works.
    pub launched_at: Option<Instant>,
    /// A first prompt to hand over as a launch argument on the next spawn, then drop:
    /// consumed there, so a restart (which resumes the conversation) never repeats it.
    /// Never persisted. See [`crate::agent::Tool::prompt_args`].
    pub first_prompt: Option<String>,
    /// Whether launching this agent appends [`crate::agent::MMUX_NOTE`] to its system
    /// prompt: only while its pane could actually use the `mmux` CLI — the control
    /// socket is served and `control.from-panes` allows it. Set by the app when the row
    /// is created and refreshed on reload; ignored without [`agent`](Self::agent).
    pub mmux_note: bool,
}

impl Session {
    pub fn new(
        name: String,
        kind: Kind,
        recipe: Recipe,
        project: usize,
        project_dir: &Path,
    ) -> Session {
        Session {
            name,
            kind,
            pane: None,
            error: None,
            recipe,
            project,
            agent: None,
            stop: None,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            project_dir: project_dir.to_path_buf(),
            depth: 1,
            last_input_at: None,
            last_working_at: None,
            launched_at: None,
            first_prompt: None,
            mmux_note: false,
        }
    }

    /// The control-socket handle for this row: `s<N>`.
    pub fn handle(&self) -> String {
        format!("s{}", self.id)
    }

    /// The recipe's environment plus mmux's identity variables, so a program in the
    /// pane can find this mmux (`MMUX_SOCKET`) and address itself and its project
    /// (`MMUX_SESSION`, `MMUX_PROJECT`). Recipe values win — they are explicit config.
    fn pane_env(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("MMUX_SESSION".to_string(), self.handle());
        env.insert(
            "MMUX_PROJECT".to_string(),
            self.project_dir.to_string_lossy().into_owned(),
        );
        env.insert("MMUX_DEPTH".to_string(), self.depth.to_string());
        if let Some(socket) = crate::control::advertised_socket() {
            env.insert(
                "MMUX_SOCKET".to_string(),
                socket.to_string_lossy().into_owned(),
            );
        }
        env.extend(self.recipe.env.clone());
        env
    }

    /// The teardown command for this session, if it declares a [`stop`](Self::stop) — a
    /// `sh -c` invocation of it in the recipe's `cwd`, carrying the recipe's env, with
    /// stdio silenced. Returns a ready-to-run [`Command`] (never spawned here) so the
    /// caller decides how to run it: fire-and-forget on a stop, or waited-on at quit.
    /// `None` for agents/terminals and any process without a (non-blank) `stop:`.
    pub fn stop_command(&self) -> Option<Command> {
        let stop = self
            .stop
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(stop)
            .current_dir(&self.recipe.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in &self.recipe.env {
            cmd.env(k, v);
        }
        Some(cmd)
    }

    pub fn status(&self) -> Status {
        match &self.pane {
            None => Status::Stopped,
            Some(p) => {
                if p.is_running() {
                    Status::Running
                } else if p.crashed() {
                    Status::Failed
                } else {
                    Status::Exited
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status(), Status::Running)
    }

    /// (Re)spawn the recipe at the given inner size, replacing any existing pane.
    /// This is both "start" and "restart": callers decide *when* to call it.
    pub fn spawn(&mut self, rows: u16, cols: u16) {
        if let Some(p) = self.pane.as_mut() {
            p.kill();
        }
        // Append any Claude/Codex/Pi/Grok resume flags. The first launch *creates* the
        // session (`--session-id`); after that, and for a restored agent, launches
        // *resume* it (`--resume`, `--session-id`, or `codex resume`).
        if let Some(r) = self.agent.as_mut() {
            r.mark_launch();
        }
        let args = self.launch_argv();
        match Pane::spawn(
            &self.recipe.cmd,
            &args,
            &self.recipe.cwd,
            &self.pane_env(),
            rows,
            cols,
        ) {
            Ok(p) => {
                self.pane = Some(p);
                self.error = None;
                self.launched_at = Some(Instant::now());
                // Subsequent (re)starts of this agent should resume the session
                // this launch just created.
                if let Some(r) = self.agent.as_mut() {
                    r.resume = true;
                }
            }
            Err(e) => {
                self.pane = None;
                self.error = Some(e.to_string());
            }
        }
    }

    /// The full argument list for the next launch: the recipe's args, then mmux's note
    /// and the resume flags, then any first prompt. The note goes ahead of the resume
    /// flags because Codex's `-c` must precede its `resume <id>` subcommand, and ahead
    /// of the prompt because Claude's must precede the `--` that a prompt may bring.
    fn launch_argv(&mut self) -> Vec<String> {
        let mut args = self.recipe.args.clone();
        if let Some(r) = self.agent.as_ref() {
            if self.mmux_note {
                args.extend(r.tool.context_args(crate::agent::MMUX_NOTE));
            }
            args.extend(r.launch_args());
        }
        // A control-supplied first prompt rides this launch only. `take` rather than
        // clone: whether or not the spawn succeeds, it is never sent twice.
        if let Some(prompt) = self.first_prompt.take() {
            if let Some(extra) = self
                .agent
                .as_ref()
                .and_then(|r| r.tool.prompt_args(&prompt))
            {
                args.extend(extra);
                self.last_input_at = Some(Instant::now());
            }
        }
        args
    }

    /// Kill the process but keep the (now-exited) pane so it reads as "exited".
    pub fn stop(&mut self) {
        if let Some(p) = self.pane.as_mut() {
            p.kill();
        }
    }

    /// Kill and drop the pane entirely (used when discarding a dropped process).
    pub fn kill(&mut self) {
        if let Some(p) = self.pane.as_mut() {
            p.kill();
        }
        self.pane = None;
    }

    /// Sidebar subtitle: the program's terminal title, falling back to the last error.
    pub fn subtitle(&self) -> Option<String> {
        self.pane
            .as_ref()
            .map(Pane::title)
            .filter(|s| !s.is_empty())
            .or_else(|| self.error.clone())
    }

    pub fn attention(&self) -> bool {
        self.pane.as_ref().map(Pane::attention).unwrap_or(false)
    }

    /// Whether this session looks like it's actively working. OSC 9;4 progress is
    /// authoritative when the program emits it; animated terminal titles remain the
    /// fallback for older agents. Codex's `Action Required` title is an explicit idle
    /// signal even if another activity signal just changed. See the sidebar's `nav_row`.
    pub fn working(&self, within: Duration) -> bool {
        self.is_running()
            && self.pane.as_ref().is_some_and(|p| {
                !p.title().contains("Action Required")
                    && p.progress_active()
                        .unwrap_or_else(|| p.title_active(within))
            })
    }

    /// Whether this agent is *visibly* working right now — running with an active
    /// progress report or still-changing title, i.e. exactly when its sidebar row
    /// shows the rotating spinner (see [`working`](Self::working) and the sidebar's
    /// `nav_row`). The close confirmation keys on this so it fires for the same agents that spin:
    /// an idle agent (running but quiet, showing the green `●`) reads as done and
    /// closes without a nag.
    pub fn busy(&self) -> bool {
        self.working(TITLE_IDLE)
    }

    /// Drain notifications captured from this session's pane since the last call.
    pub fn take_notifications(&self) -> Vec<Notify> {
        self.pane
            .as_ref()
            .map(Pane::take_notifications)
            .unwrap_or_default()
    }
}

/// Resolve a config-relative `cwd` against the workspace `dir`.
pub fn resolve(dir: &Path, cwd: &Option<String>) -> PathBuf {
    match cwd {
        Some(c) => dir.join(c),
        None => dir.to_path_buf(),
    }
}

/// The user's login shell (`$SHELL`), falling back to `/bin/sh`. In a PTY this
/// starts interactively, so a plain terminal needs no extra args.
pub fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// Resolve the editor command + any leading args: `$VISUAL` then `$EDITOR` (split
/// on whitespace so `"code -w"` works), else the first of `micro`, `nano`, `vim`, `vi`
/// found on `PATH` — falling back to `vi` (the near-universal last resort) if none are.
fn editor_command() -> (String, Vec<String>) {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(var) {
            let mut it = v.split_whitespace().map(str::to_string);
            if let Some(cmd) = it.next() {
                return (cmd, it.collect());
            }
        }
    }
    let cmd = ["micro", "nano", "vim", "vi"]
        .into_iter()
        .find(|c| on_path(c))
        .unwrap_or("vi");
    (cmd.to_string(), Vec::new())
}

/// Whether `bin` is found in any `PATH` entry.
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Resume, Tool, MMUX_NOTE};

    fn agent(cmd: &str, resume: Resume) -> Session {
        let recipe = Recipe {
            cmd: cmd.into(),
            args: vec!["--x".into()],
            cwd: PathBuf::from("/p"),
            env: BTreeMap::new(),
        };
        let mut s = Session::new("A".into(), Kind::Agent, recipe, 0, Path::new("/p"));
        s.agent = Some(resume);
        s.mmux_note = true;
        s
    }

    #[test]
    fn mmux_note_goes_before_codex_resume_and_claude_prompt() {
        let mut s = agent("codex", Resume::restored(Tool::Codex, Some("abc".into())));
        let args = s.launch_argv();
        assert_eq!(args[..2], ["--x", "-c"]);
        assert_eq!(args[3..], ["resume", "abc"]);

        let mut s = agent("claude", Resume::restored(Tool::Claude, Some("id".into())));
        s.first_prompt = Some("-p".into());
        let args = s.launch_argv();
        assert_eq!(
            args,
            [
                "--x",
                "--append-system-prompt",
                MMUX_NOTE,
                "--resume",
                "id",
                "--",
                "-p"
            ]
        );

        s.mmux_note = false;
        assert_eq!(s.launch_argv(), ["--x", "--resume", "id"]);
    }
}
