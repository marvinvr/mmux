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

    /// Stable-partition projects by useful background activity: a running agent, a
    /// running configured process, or Git changes. If the selected project loses its
    /// last signal, `sticky_priority_project` holds it there until selection moves
    /// elsewhere; selecting an already-quiet lone project never promotes it. Activity
    /// counts and states do not rank projects within a group; display names sort
    /// case-insensitively, with manifest order as the stable tie-breaker.
    ///
    /// Worktrees never leave their parent's side: a repository's checkouts are
    /// partitioned and sorted as one **family**, so a busy worktree lifts the whole
    /// group rather than being torn out of it and stranded at the top of the sidebar.
    /// The family you are working in leads the list outright — see
    /// [`arrange_families`].
    pub(crate) fn project_display_order(&self) -> Vec<usize> {
        let roots: Vec<usize> = (0..self.projects.len())
            .map(|pi| self.family_root(pi))
            .collect();
        // Activity is pooled onto the family root, so the block moves together.
        let mut hot = vec![false; self.projects.len()];
        for pi in 0..self.projects.len() {
            if self.priority_projects.get(pi).copied().unwrap_or(false)
                || self.sticky_priority_project == Some(pi)
            {
                hot[roots[pi]] = true;
            }
        }
        let keys: Vec<SortKey> = (0..self.projects.len())
            .map(|pi| self.project_sort_key(pi, &roots))
            .collect();
        arrange_families(&roots, &hot, Some(self.active), &keys)
    }

    /// Sort key that keeps a family contiguous and in a fixed shape: the family's
    /// name (then its root index, so two repositories that happen to share a display
    /// name still can't interleave), then the parent ahead of its worktrees, then the
    /// worktree's own branch.
    fn project_sort_key(&self, pi: usize, roots: &[usize]) -> SortKey {
        let root = roots[pi];
        let family = self.projects[root].cfg.display_name().to_lowercase();
        let own = self.projects[pi].label().to_lowercase();
        let depth = u8::from(self.projects[pi].worktree.is_some());
        (family, root, depth, own)
    }

    fn project_has_priority(&self, pi: usize) -> bool {
        self.sessions.iter().any(|s| {
            s.project == pi && s.is_running() && matches!(s.kind, Kind::Agent | Kind::Process)
        }) || self.projects[pi]
            .git
            .as_ref()
            .is_some_and(|git| !git.files.is_empty())
    }

    /// Rebuild the cached priority group after session or Git state changes. The
    /// cache makes a transition atomic from navigation's point of view: capture the
    /// selected row under the old order, update groups, then find that same row in
    /// the new order.
    pub(crate) fn sync_project_priority(&mut self) {
        let selected = self.current_nav();
        let next: Vec<bool> = (0..self.projects.len())
            .map(|pi| self.project_has_priority(pi))
            .collect();
        if self.priority_projects.get(self.active) == Some(&true)
            && next.get(self.active) == Some(&false)
        {
            self.sticky_priority_project = Some(self.active);
        }
        self.priority_projects = next;
        if let Some(row) = selected {
            if let Some(pos) = self.build_nav().iter().position(|item| *item == row) {
                self.sel = pos;
            }
        }
    }

    /// Reset after initialization or workspace membership changes, where old project
    /// indices are not meaningful and therefore must not create a sticky transition.
    pub(crate) fn reset_project_priority(&mut self) {
        self.priority_projects = (0..self.projects.len())
            .map(|pi| self.project_has_priority(pi))
            .collect();
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
            self.sticky_priority_project = None;
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

/// A project's place in the sidebar: family name, family root, parent-before-worktree,
/// own name. Built by `App::project_sort_key`.
type SortKey = (String, usize, u8, String);

/// Order the projects for display, given each one's family root, whether its family
/// carries background activity, the active project, and its sort key.
///
/// Three groups, each sorted by key: **the active family**, then families with
/// activity, then everything else. Sorting by a key that leads with the family root
/// is what makes a family a block — worktrees always sit directly under the checkout
/// they were cut from, and the block moves as one.
///
/// The active family leads only when it *is* a family. A lone project is still not
/// promoted merely for being selected — browsing with `[`/`]` must not reshuffle the
/// list under the cursor — but sibling checkouts of one repository are a single unit
/// of work you move between constantly, so they stay together at the top while you do.
fn arrange_families(
    roots: &[usize],
    hot: &[bool],
    active: Option<usize>,
    keys: &[SortKey],
) -> Vec<usize> {
    let lead = active
        .and_then(|pi| roots.get(pi).copied())
        .filter(|&root| roots.iter().filter(|&&r| r == root).count() > 1);
    let mut groups: [Vec<usize>; 3] = Default::default();
    for (pi, &root) in roots.iter().enumerate() {
        let g = if lead == Some(root) {
            0
        } else if hot[root] {
            1
        } else {
            2
        };
        groups[g].push(pi);
    }
    let mut out = Vec::with_capacity(roots.len());
    for mut group in groups {
        group.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
        out.append(&mut group);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Projects `names`, each `(family root, name, is_worktree)`.
    fn keys(spec: &[(usize, &str, bool)]) -> Vec<SortKey> {
        spec.iter()
            .map(|&(root, name, wt)| {
                (
                    spec[root].1.to_string(),
                    root,
                    u8::from(wt),
                    name.to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn worktrees_sit_under_their_parent_and_the_family_moves_as_one() {
        // 0: "zed" (quiet), 1: "app", 2/3: worktrees of "app" — one of them busy.
        let roots = [0, 1, 1, 1];
        let hot = [false, true, false, false];
        let k = keys(&[
            (0, "zed", false),
            (1, "app", false),
            (1, "brave-otter", true),
            (1, "calm-yak", true),
        ]);
        // Activity on the family lifts parent + both worktrees above the quiet project,
        // parent first, worktrees in name order.
        assert_eq!(arrange_families(&roots, &hot, None, &k), vec![1, 2, 3, 0]);
    }

    #[test]
    fn the_active_family_leads_from_either_end() {
        let roots = [0, 1, 1];
        let k = keys(&[(0, "zed", false), (1, "app", false), (1, "brave-otter", true)]);
        // "zed" is the busy one, but working in the worktree (2) — or in its parent
        // (1) — puts the whole family first anyway.
        let hot = [true, false, false];
        assert_eq!(arrange_families(&roots, &hot, Some(2), &k), vec![1, 2, 0]);
        assert_eq!(arrange_families(&roots, &hot, Some(1), &k), vec![1, 2, 0]);
        assert_eq!(arrange_families(&roots, &hot, Some(0), &k), vec![0, 1, 2]);
    }

    #[test]
    fn a_lone_project_is_not_promoted_by_being_selected() {
        let roots = [0, 1];
        let hot = [true, false];
        let k = keys(&[(0, "zed", false), (1, "app", false)]);
        // Selecting the quiet "app" leaves it below the busy "zed".
        assert_eq!(arrange_families(&roots, &hot, Some(1), &k), vec![0, 1]);
    }

    #[test]
    fn families_sharing_a_display_name_still_cannot_interleave() {
        let roots = [0, 0, 2, 2];
        let hot = [false, false, false, false];
        let k = keys(&[
            (0, "app", false),
            (0, "brave-otter", true),
            (2, "app", false),
            (2, "calm-yak", true),
        ]);
        assert_eq!(arrange_families(&roots, &hot, None, &k), vec![0, 1, 2, 3]);
    }
}
