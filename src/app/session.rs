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
use std::time::Duration;

/// How long after an agent's terminal title last changed we still count it as
/// "working". Agents animate the title while busy; once it's been static this
/// long the agent is treated as idle/awaiting you. This is the single window
/// behind [`Session::busy`], shared by the sidebar spinner and the close
/// confirmation so the two never disagree about "is it working".
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
    /// Resume bookkeeping for a Claude/Codex agent: lets a (re)start reattach to
    /// the same conversation rather than start cold. `None` for terminals,
    /// processes, and any agent that isn't one of the two we support. See
    /// [`crate::agent`] and [`crate::restore`].
    pub agent: Option<crate::agent::Resume>,
    /// Optional teardown command (a shell line) run in `recipe.cwd` after this session's
    /// process stops — on an explicit stop and on quit, but not on a restart. Carried
    /// from a config-defined process's [`stop:`](crate::config::ProcessDef::stop); `None`
    /// for agents, terminals, and processes without one. See [`Session::stop_command`].
    pub stop: Option<String>,
}

impl Session {
    pub fn new(name: String, kind: Kind, recipe: Recipe, project: usize) -> Session {
        Session {
            name,
            kind,
            pane: None,
            error: None,
            recipe,
            project,
            agent: None,
            stop: None,
        }
    }

    /// The teardown command for this session, if it declares a [`stop`](Self::stop) — a
    /// `sh -c` invocation of it in the recipe's `cwd`, carrying the recipe's env, with
    /// stdio silenced. Returns a ready-to-run [`Command`] (never spawned here) so the
    /// caller decides how to run it: fire-and-forget on a stop, or waited-on at quit.
    /// `None` for agents/terminals and any process without a (non-blank) `stop:`.
    pub fn stop_command(&self) -> Option<Command> {
        let stop = self.stop.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
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
        // Append any Claude/Codex resume flags. The first launch *creates* the
        // session (`--session-id`); after that, and for a restored agent, launches
        // *resume* it (`--resume` / `codex resume`).
        if let Some(r) = self.agent.as_mut() {
            r.mark_launch();
        }
        let mut args = self.recipe.args.clone();
        if let Some(r) = self.agent.as_ref() {
            args.extend(r.launch_args());
        }
        match Pane::spawn(
            &self.recipe.cmd,
            &args,
            &self.recipe.cwd,
            &self.recipe.env,
            rows,
            cols,
        ) {
            Ok(p) => {
                self.pane = Some(p);
                self.error = None;
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

    /// Whether this session looks like it's actively working: it's running and its
    /// terminal title changed within `within`. Agents animate the title (a spinner /
    /// moving glyph) while busy and leave it static once idle, so a running-but-quiet
    /// agent is treated as "needs you" rather than busy. See the sidebar's `nav_row`.
    pub fn working(&self, within: Duration) -> bool {
        self.is_running() && self.pane.as_ref().map(|p| p.title_active(within)).unwrap_or(false)
    }

    /// Whether this agent is *visibly* working right now — running with a live,
    /// still-changing title, i.e. exactly when its sidebar row shows the rotating
    /// spinner (see [`working`](Self::working) and the sidebar's `nav_row`). The
    /// close confirmation keys on this so it fires for the same agents that spin:
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
    let cmd = ["micro", "nano", "vim", "vi"].into_iter().find(|c| on_path(c)).unwrap_or("vi");
    (cmd.to_string(), Vec::new())
}

/// Whether `bin` is found in any `PATH` entry.
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}
