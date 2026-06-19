//! Session lifecycle driven from the sidebar: spawning new agents/terminals,
//! the start/stop/restart key actions, and the live config reload.

use super::git::{GitPanel, Overlay};
use super::nav::Nav;
use super::picker::Picker;
use super::session::{Kind, Recipe, Session, Status};
use super::{App, Focus};
use crate::config::Config;
use std::path::PathBuf;
use std::time::Instant;

impl App {
    pub(crate) fn spawn_agent(&mut self, pi: usize, t: usize) {
        let def = self.projects[pi].cfg.agents[t].clone();
        let recipe = Recipe::agent(&def, &self.projects[pi].cfg.dir);
        self.projects[pi].counts[t] += 1;
        let name = format!("{} #{}", def.name, self.projects[pi].counts[t]);
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Agent, recipe, pi);
        s.spawn(rows, cols);
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
    }

    pub(crate) fn spawn_terminal(&mut self, pi: usize) {
        let recipe = Recipe::shell(&self.projects[pi].cfg.dir);
        self.projects[pi].term_count += 1;
        let name = format!("Terminal #{}", self.projects[pi].term_count);
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Terminal, recipe, pi);
        s.spawn(rows, cols);
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
    }

    /// Raise the Ctrl+P fuzzy file picker over the active project. Listing happens
    /// up front in [`Picker::new`]; the modal then eats keys until Enter/Esc.
    pub(crate) fn open_picker(&mut self) {
        let dir = self.projects[self.active].cfg.dir.clone();
        self.overlay = Some(Overlay::Picker(Picker::new(self.active, dir)));
    }

    /// Open `rel` (relative to project `pi`'s dir) in the user's editor as a new
    /// terminal-kind session, then focus it — so it takes over the main pane. Reuses
    /// the same spawn path as [`spawn_terminal`](Self::spawn_terminal); the editor
    /// row reads as a normal terminal and can be closed/returned-to like any other.
    pub(crate) fn open_in_editor(&mut self, pi: usize, rel: String) {
        let dir = self.projects[pi].cfg.dir.clone();
        let recipe = Recipe::editor(&dir, &rel);
        let base = std::path::Path::new(&rel)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.clone());
        let name = format!("✎ {base}");
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Terminal, recipe, pi);
        s.ephemeral = true; // vanish once the editor quits (see prune_ephemeral)
        s.spawn(rows, cols);
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
        self.focus = Focus::Terminal;
    }

    /// Open the current row: spawn launchers, (re)start dead sessions, focus the pane.
    pub(crate) fn activate(&mut self) {
        let Some(nav) = self.current_nav() else {
            return;
        };
        match nav {
            Nav::NewAgent(p, t) => {
                self.spawn_agent(p, t);
                self.focus = Focus::Terminal;
            }
            Nav::NewTerminal(p) => {
                self.spawn_terminal(p);
                self.focus = Focus::Terminal;
            }
            Nav::Session(i) => {
                if !self.sessions[i].is_running() {
                    let (rows, cols) = self.last_inner;
                    self.sessions[i].spawn(rows, cols);
                }
                if let Some(p) = self.sessions[i].pane.as_ref() {
                    p.clear_attention();
                }
                self.focus = Focus::Terminal;
            }
            Nav::Panel => self.focus = Focus::Right,
        }
    }

    pub(crate) fn do_start(&mut self) {
        match self.current_nav() {
            Some(Nav::NewAgent(p, t)) => self.spawn_agent(p, t),
            Some(Nav::NewTerminal(p)) => self.spawn_terminal(p),
            Some(Nav::Session(i)) if !self.sessions[i].is_running() => {
                let (rows, cols) = self.last_inner;
                self.sessions[i].spawn(rows, cols);
            }
            _ => {}
        }
    }

    pub(crate) fn do_stop(&mut self) {
        if let Some(Nav::Session(i)) = self.current_nav() {
            match self.sessions[i].kind {
                // Agents and terminals are throwaway instances: closing one removes
                // it for good rather than leaving an exited husk in the sidebar.
                Kind::Agent | Kind::Terminal => self.close_session(i),
                // Processes are config-defined entries: stop but keep the row so it
                // can be started again in place.
                _ => self.sessions[i].stop(),
            }
        }
    }

    /// Kill an agent/terminal session and drop it from the sidebar entirely.
    fn close_session(&mut self, i: usize) {
        self.sessions[i].kill();
        self.sessions.remove(i);
        // Selection is positional; the nav list just shrank. Keep the cursor in
        // range and hand focus back to the sidebar (its pane is gone).
        let navlen = self.build_nav().len();
        self.sel = self.sel.min(navlen.saturating_sub(1));
        self.focus = Focus::Sidebar;
    }

    /// Drop any ephemeral session (the Ctrl+P editor) whose program has exited, so
    /// quitting the editor makes its row vanish instead of leaving an "exited" husk.
    /// Called once per loop from [`tick`](super::App::tick).
    pub(crate) fn prune_ephemeral(&mut self) {
        let dead: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.ephemeral && matches!(s.status(), Status::Exited))
            .map(|(i, _)| i)
            .collect();
        if dead.is_empty() {
            return;
        }
        // If we're sitting in one of the dying panes, fall back to the sidebar.
        let focused_dead = self.focus == Focus::Terminal
            && matches!(self.current_nav(), Some(Nav::Session(i)) if dead.contains(&i));
        // Remove back-to-front so the earlier indices stay valid.
        for &i in dead.iter().rev() {
            self.sessions[i].kill();
            self.sessions.remove(i);
        }
        if focused_dead {
            self.focus = Focus::Sidebar;
        }
        let navlen = self.build_nav().len();
        self.sel = self.sel.min(navlen.saturating_sub(1));
    }

    pub(crate) fn do_restart(&mut self) {
        match self.current_nav() {
            Some(Nav::Session(i)) => {
                let (rows, cols) = self.last_inner;
                self.sessions[i].spawn(rows, cols);
            }
            Some(Nav::NewAgent(p, t)) => self.spawn_agent(p, t),
            Some(Nav::NewTerminal(p)) => self.spawn_terminal(p),
            Some(Nav::Panel) => {
                if let Some(g) = self.active_git_mut() {
                    g.refresh();
                }
            }
            None => {}
        }
    }

    /// Re-read every loaded project's `mmux.yaml` (+ the global config) in place and
    /// merge changes into the live session without disturbing running panes: new
    /// processes/agents appear, edited recipes are picked up on the next (re)start,
    /// and a removed process that's still running is kept as an orphan rather than
    /// killed. Bound to `R` / `Ctrl-b R`.
    ///
    /// The *set* of projects is fixed for the session — changing `linked-projects`
    /// needs a reopen — so this only refreshes each project's own agents/processes/
    /// panel, keyed by its directory.
    pub(crate) fn reload(&mut self) {
        // Reload each project's config by dir. A project whose config fails to load
        // keeps its current one (recorded as `None`) instead of aborting the reload.
        let dirs: Vec<PathBuf> = self.projects.iter().map(|p| p.cfg.dir.clone()).collect();
        let mut new_cfgs: Vec<Option<Config>> = Vec::with_capacity(dirs.len());
        let mut failed = 0usize;
        for dir in &dirs {
            match Config::load(dir) {
                Ok(mut c) => {
                    c.linked_projects.clear(); // structural changes need a reopen
                    new_cfgs.push(Some(c));
                }
                Err(_) => {
                    failed += 1;
                    new_cfgs.push(None);
                }
            }
        }

        // Processes: reconcile by (project, name) in one pass, preserving live panes
        // and refreshing recipes. Agents/terminals are spawned instances, left as-is.
        let (mut old_procs, others): (Vec<Session>, Vec<Session>) = std::mem::take(&mut self.sessions)
            .into_iter()
            .partition(|s| s.kind == Kind::Process);

        let mut next_procs: Vec<Session> = Vec::new();
        let mut added = 0usize;
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else {
                // Config failed to load: keep this project's processes untouched.
                let (mine, rest): (Vec<Session>, Vec<Session>) =
                    old_procs.into_iter().partition(|s| s.project == pi);
                next_procs.extend(mine);
                old_procs = rest;
                continue;
            };
            let dir = ncfg.dir.clone();
            for p in &ncfg.processes {
                let recipe = Recipe::process(p, &dir);
                match old_procs.iter().position(|it| it.project == pi && it.name == p.name) {
                    Some(pos) => {
                        let mut item = old_procs.remove(pos);
                        item.recipe = recipe; // an edited command takes effect on next restart
                        next_procs.push(item);
                    }
                    None => {
                        next_procs.push(Session::new(p.name.clone(), Kind::Process, recipe, pi));
                        added += 1;
                    }
                }
            }
        }
        // Dropped processes (config removed them): keep running ones as orphans.
        let mut orphaned = 0usize;
        for mut item in old_procs {
            if item.is_running() {
                next_procs.push(item);
                orphaned += 1;
            } else {
                item.kill();
            }
        }
        self.sessions = others;
        self.sessions.extend(next_procs);

        // Agents: remap each project's "#N" counters by name so suffixes stay unique.
        let mut added_agents = 0usize;
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else { continue };
            let old_names: Vec<String> =
                self.projects[pi].cfg.agents.iter().map(|a| a.name.clone()).collect();
            let old_counts = self.projects[pi].counts.clone();
            self.projects[pi].counts = ncfg
                .agents
                .iter()
                .map(|a| {
                    old_names
                        .iter()
                        .position(|o| o == &a.name)
                        .map(|i| old_counts[i])
                        .unwrap_or(0)
                })
                .collect();
            added_agents += ncfg
                .agents
                .iter()
                .filter(|a| !old_names.iter().any(|o| o == &a.name))
                .count();
        }

        // Git panels: a project that became (or stopped being) a repo gains/loses its
        // panel. A live panel is left untouched so its cursor/staging survive reload.
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else { continue };
            let want = ncfg.git_panel_enabled() && crate::git::is_repo(&ncfg.dir);
            match (want, self.projects[pi].git.is_some()) {
                (true, false) => self.projects[pi].git = Some(GitPanel::new(ncfg.dir.clone())),
                (false, true) => self.projects[pi].git = None,
                _ => {}
            }
        }

        // Commit the refreshed configs.
        for (pi, ncfg) in new_cfgs.into_iter().enumerate() {
            if let Some(ncfg) = ncfg {
                self.projects[pi].cfg = ncfg;
            }
        }

        // The nav list may have grown or shrunk; keep the selection in range.
        let navlen = self.build_nav().len();
        self.sel = self.sel.min(navlen.saturating_sub(1));

        let mut parts: Vec<String> = Vec::new();
        if added > 0 {
            parts.push(format!("+{added} process(es)"));
        }
        if added_agents > 0 {
            parts.push(format!("+{added_agents} agent(s)"));
        }
        if orphaned > 0 {
            parts.push(format!("{orphaned} orphaned"));
        }
        if failed > 0 {
            parts.push(format!("{failed} unreadable"));
        }
        let summary = if parts.is_empty() {
            "no changes".into()
        } else {
            parts.join(", ")
        };
        self.flash = Some((format!("reloaded — {summary}"), Instant::now()));
    }
}
