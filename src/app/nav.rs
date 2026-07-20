//! The sidebar navigation model: the ordered list of selectable rows and the
//! cursor (`App::sel`) into it.
//!
//! Selection is still positional — `sel` is an index into [`App::build_nav`],
//! which is rebuilt on demand. A future change (proposal "step 7") would replace
//! this with a stable identity (`SessionId`); confining the model here makes that
//! swap a single-file change.

use super::session::Kind;
use super::{App, Focus};
use crate::pane::Pane;

/// One selectable sidebar row, in display order. Launchers carry the project they
/// act on; `Session(i)` indexes the flat `sessions` vec (its project is on the row).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    NewAgent(usize, usize), // (project, agent template): launcher for projects[p].cfg.agents[t]
    NewTerminal(usize),     // (project): launcher for a plain shell in that project
    NewProcess(usize),      // (project): launcher for the "+ New Process" form in that project
    Session(usize),         // a live/exited session: self.sessions[i]
    Panel,                  // the active project's right panel (only listed in compact mode)
}

impl App {
    /// The ordered nav list. Wide layouts contain every project in display order;
    /// compact layouts contain only the active project, because project switching is
    /// handled by the mobile project picker. The compact git-panel row comes last.
    pub(crate) fn build_nav(&self) -> Vec<Nav> {
        let mut nav = Vec::new();
        let order = if self.compact && self.projects.len() > 1 {
            vec![self.active]
        } else {
            self.project_display_order()
        };
        for pi in order {
            let proj = &self.projects[pi];
            for t in 0..proj.cfg.agents.len() {
                nav.push(Nav::NewAgent(pi, t));
            }
            self.push_sessions(&mut nav, pi, Kind::Agent);
            nav.push(Nav::NewTerminal(pi));
            self.push_sessions(&mut nav, pi, Kind::Terminal);
            nav.push(Nav::NewProcess(pi));
            self.push_sessions(&mut nav, pi, Kind::Process);
        }
        if self.compact && self.active_git().is_some() {
            nav.push(Nav::Panel);
        }
        nav
    }

    /// Stable-partition projects by agent presence. A project enters the upper group
    /// when its first agent is created. If the selected project loses its last agent,
    /// `sticky_agent_project` holds it there until selection moves elsewhere; selecting
    /// an already-quiet project never promotes it. Agent counts and states do not rank
    /// projects within a group; display names sort case-insensitively, with manifest
    /// order as the stable tie-breaker for equal names.
    pub(crate) fn project_display_order(&self) -> Vec<usize> {
        let mut active = Vec::new();
        let mut quiet = Vec::new();
        for pi in 0..self.projects.len() {
            if self.project_has_agents(pi) || self.sticky_agent_project == Some(pi) {
                active.push(pi);
            } else {
                quiet.push(pi);
            }
        }
        active.sort_by_cached_key(|&pi| self.projects[pi].cfg.display_name().to_lowercase());
        quiet.sort_by_cached_key(|&pi| self.projects[pi].cfg.display_name().to_lowercase());
        active.extend(quiet);
        active
    }

    pub(crate) fn project_has_agents(&self, pi: usize) -> bool {
        self.sessions
            .iter()
            .any(|s| s.project == pi && s.kind == Kind::Agent)
    }

    fn push_sessions(&self, nav: &mut Vec<Nav>, pi: usize, kind: Kind) {
        for (i, s) in self.sessions.iter().enumerate() {
            if s.project == pi && s.kind == kind {
                nav.push(Nav::Session(i));
            }
        }
    }

    /// Which project a nav row belongs to (the shared panel row belongs to none).
    pub(crate) fn project_of(&self, nav: Nav) -> Option<usize> {
        match nav {
            Nav::NewAgent(p, _) | Nav::NewTerminal(p) | Nav::NewProcess(p) => Some(p),
            Nav::Session(i) => Some(self.sessions[i].project),
            Nav::Panel => None,
        }
    }

    pub(crate) fn current_nav(&self) -> Option<Nav> {
        self.build_nav().get(self.sel).copied()
    }

    /// The [`Kind`] of the selected session row, or `None` on a launcher / panel row.
    /// Drives the sidebar footer's action chips (processes get edit/delete, not close).
    pub(crate) fn selected_kind(&self) -> Option<Kind> {
        match self.current_nav() {
            Some(Nav::Session(i)) => Some(self.sessions[i].kind),
            _ => None,
        }
    }

    /// Whether the selected session is running (false on a launcher / panel row) — lets
    /// the footer show `x stop` only for a process that's actually up.
    pub(crate) fn selected_running(&self) -> bool {
        matches!(self.current_nav(), Some(Nav::Session(i)) if self.sessions[i].is_running())
    }

    pub(crate) fn pane_at(&self, nav: Nav) -> Option<&Pane> {
        match nav {
            Nav::Session(i) => self.sessions[i].pane.as_ref(),
            // The git panel is native, not pane-backed; launchers have no pane.
            Nav::Panel | Nav::NewAgent(..) | Nav::NewTerminal(_) | Nav::NewProcess(_) => None,
        }
    }

    pub(crate) fn pane_at_mut(&mut self, nav: Nav) -> Option<&mut Pane> {
        match nav {
            Nav::Session(i) => self.sessions[i].pane.as_mut(),
            Nav::Panel | Nav::NewAgent(..) | Nav::NewTerminal(_) | Nav::NewProcess(_) => None,
        }
    }

    pub(crate) fn move_sel(&mut self, delta: i32) {
        let len = self.build_nav().len() as i32;
        if len == 0 {
            return;
        }
        self.sel = (self.sel as i32 + delta).clamp(0, len - 1) as usize;
    }

    /// Switch the cursor to project `delta` away (`]` / `[`).
    pub(crate) fn jump_project(&mut self, delta: i32) {
        let order = self.project_display_order();
        if order.len() < 2 {
            return;
        }
        let pos = order.iter().position(|&pi| pi == self.active).unwrap_or(0) as i32;
        let target = (pos + delta).clamp(0, order.len() as i32 - 1) as usize;
        self.focus_project(order[target]);
    }

    /// Move the cursor into project `pi`, landing on the row last selected there (if
    /// it still exists and still belongs to `pi`) or that project's first row.
    pub(crate) fn focus_project(&mut self, pi: usize) {
        let changed = self.active != pi;
        if changed {
            self.sticky_agent_project = None;
        }
        self.active = pi;
        let nav = self.build_nav();
        let remembered = self.last_proj_sel.get(pi).copied().flatten();
        let pos = remembered
            .and_then(|want| nav.iter().position(|n| *n == want))
            // Guard against the positional-nav edge (a closed/shifted session): only
            // honor the remembered row if it still resolves to this project.
            .filter(|&pos| self.project_of(nav[pos]) == Some(pi))
            .or_else(|| nav.iter().position(|n| self.project_of(*n) == Some(pi)));
        if let Some(pos) = pos {
            self.sel = pos;
            // The preview is scoped to one project's repo; a switch invalidates it.
            if changed {
                self.clear_diff();
            }
        }
    }

    /// Put the cursor on session index `i`, wherever it lands in the nav order.
    pub(crate) fn select_session(&mut self, i: usize) {
        if let Some(pos) = self.build_nav().iter().position(|n| *n == Nav::Session(i)) {
            self.sel = pos;
        }
    }

    /// The pane that currently has keyboard focus (main selection or right panel).
    pub(crate) fn focused_pane(&self) -> Option<&Pane> {
        match self.focus {
            // The right column is the native git panel — no pane to forward keys to.
            Focus::Right | Focus::Sidebar => None,
            Focus::Terminal => self.current_nav().and_then(|n| self.pane_at(n)),
        }
    }
}
