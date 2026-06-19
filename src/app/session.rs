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
}

/// Everything needed to (re)spawn a pane identically.
#[derive(Clone)]
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
        }
    }

    pub fn status(&self) -> Status {
        match &self.pane {
            None => Status::Stopped,
            Some(p) => {
                if p.is_running() {
                    Status::Running
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
        match Pane::spawn(
            &self.recipe.cmd,
            &self.recipe.args,
            &self.recipe.cwd,
            &self.recipe.env,
            rows,
            cols,
        ) {
            Ok(p) => {
                self.pane = Some(p);
                self.error = None;
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
