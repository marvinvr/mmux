//! Worktrees as projects: adopting them, cutting them, merging them back, and
//! taking them away — plus the one-dev-stack-per-repository swap and the reaper
//! that clears finished checkouts up.
//!
//! A worktree is an ordinary [`Project`](super::Project) whose directory is a linked
//! checkout of another project's repository. **Nothing in the session model changes**:
//! it spawns agents, terminals and processes the same way, gets its own git panel, and
//! is navigated the same way — which is exactly why a pane started from a worktree's
//! box runs in that checkout without a line of code aimed at the problem. So what
//! lives here is only the part that is genuinely about worktrees.
//!
//! The git plumbing is in [`crate::git`]; paths, generated names and new-checkout
//! setup are in [`crate::worktree`]. See
//! [Architecture](../docs/06-architecture.md#worktrees-are-projects).

use super::git::first_line;
use super::overlay::{Confirmed, Overlay};
use super::session::{Kind, Recipe, Session};
use super::{App, Focus, Project, Swap, Worktree};
use crate::config::{self, Config};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the selection must rest in a checkout before its dev stack follows you
/// there. `active` tracks the selection *cursor*, so without a dwell, arrowing down
/// the sidebar past a worktree would tear a dev server down and stand it back up. Long
/// enough that browsing is free, short enough that settling in feels immediate.
pub(super) const SWAP_DWELL: Duration = Duration::from_secs(3);

/// How long a swap waits for the old checkout's teardown commands before starting the
/// new one anyway. A wedged `stop:` must not strand the stack in limbo.
const SWAP_DRAIN_WAIT: Duration = Duration::from_secs(20);

/// How often the idle-worktree reaper does its git checks. The idle clocks themselves
/// are refreshed every tick (free, in memory); this only paces the `git` forks that
/// decide whether a *candidate* is really finished.
const REAP_SCAN_EVERY: Duration = Duration::from_secs(60);

/// Where a worktree branch's base is remembered. It lives in the **repository**
/// config, which every checkout shares, so it survives restarts, is readable from
/// either side, and goes away with the repo rather than lingering in mmux state.
fn base_key(branch: &str) -> String {
    format!("branch.{branch}.mmuxbase")
}

/// A duration as something a footer note can say: `40m`, `2h`, `3d`.
fn human_duration(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    match mins {
        0 => "moments".to_string(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m => format!("{}d", m / (60 * 24)),
    }
}

impl App {
    pub(crate) fn family_root(&self, pi: usize) -> usize {
        match self.projects[pi].worktree.as_ref() {
            Some(wt) => self
                .projects
                .iter()
                .position(|p| p.dir == wt.parent)
                .unwrap_or(pi),
            None => pi,
        }
    }

    /// Reconcile the loaded projects with the worktrees that exist on disk, returning
    /// `(adopted, dropped)`.
    ///
    /// Worktrees are **discovered, never bookkept.** `git worktree list` is the only
    /// source of truth: there is no state file to go stale or repair, one removed from
    /// a shell simply stops appearing, and one created outside mmux (under our root)
    /// is picked up on the next reload. Only worktrees under
    /// [`crate::worktree::root_for`] are adopted — one you keep elsewhere is yours to
    /// manage, not ours to open.
    pub(crate) fn sync_worktree_projects(&mut self) -> (usize, usize) {
        // (parent dir, worktree dir, branch) for everything adoptable.
        let mut found: Vec<(PathBuf, PathBuf, String)> = Vec::new();
        for pi in 0..self.projects.len() {
            // One level only: a worktree's own listing reports its siblings, which the
            // parent that owns them already covers.
            if self.projects[pi].worktree.is_some() {
                continue;
            }
            let dir = self.projects[pi].dir.clone();
            let entries = crate::git::worktrees(&dir);
            // The first entry is the repository's main checkout. Only *it* adopts
            // worktrees — a linked worktree the user opened directly is just a project.
            let Some(main) = entries.first() else {
                continue;
            };
            if config::canonical(&main.path) != dir {
                continue;
            }
            // Canonical on both sides, so a symlinked `$HOME` can't defeat the
            // "is this one of ours" test.
            let Some(root) = crate::worktree::root_for(&dir).map(|r| config::canonical(&r)) else {
                continue;
            };
            for entry in entries.iter().skip(1) {
                let path = config::canonical(&entry.path);
                if entry.branch.is_empty() || !path.starts_with(&root) || !path.is_dir() {
                    continue;
                }
                found.push((dir.clone(), path, entry.branch.clone()));
            }
        }

        // Gone from disk ⇒ gone from the sidebar. Keyed on the **directory existing**
        // rather than on absence from `found`: a `git` invocation that fails for any
        // transient reason yields an empty listing, and that must never be read as
        // "every worktree was deleted" — which would kill their panes.
        let stale: HashSet<usize> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.worktree.is_some() && !p.dir.is_dir())
            .map(|(pi, _)| pi)
            .collect();
        let dropped = stale.len();
        self.remove_projects(&stale);

        let mut adopted = 0usize;
        for (parent, path, branch) in found {
            if self.projects.iter().any(|p| p.dir == path) {
                continue;
            }
            // A worktree is a checkout of the same repo, so it carries the project's
            // own `mmux.yaml` — its agents and processes come along for free.
            let Ok(mut cfg) = Config::load(&path) else {
                continue;
            };
            cfg.workspace = None; // a worktree is a project, never a manifest
            let pi = self.projects.len();
            let mut project = Project::new(cfg);
            project.worktree = Some(Worktree {
                parent,
                branch,
                // Freshly adopted: give it a full idle window before the reaper can
                // look at it, so reopening mmux never immediately clears one away.
                last_busy: Instant::now(),
            });
            self.projects.push(project);
            self.last_proj_sel.push(None);
            self.push_project_processes(pi);
            adopted += 1;
        }
        if adopted > 0 || dropped > 0 {
            self.reset_project_priority();
        }
        (adopted, dropped)
    }

    /// Append project `pi`'s configured process rows, stopped. Autostart is
    /// deliberately **not** applied here: a worktree's stack starts when the stack
    /// follows you into it, so opening one never races its parent for a port.
    fn push_project_processes(&mut self, pi: usize) {
        let dir = self.projects[pi].cfg.dir.clone();
        for p in self.projects[pi].cfg.processes.clone() {
            let mut s = Session::new(p.name.clone(), Kind::Process, Recipe::process(&p, &dir), pi);
            s.stop = p.stop.clone();
            self.sessions.push(s);
        }
    }

    /// Refresh the git panels of a repository's checkouts — the main one and every
    /// worktree of it — after an operation that moved refs under them.
    fn refresh_repo_panels(&mut self, repo: &Path) {
        for p in self.projects.iter_mut() {
            let mine = p.dir == repo || p.worktree.as_ref().is_some_and(|w| w.parent == repo);
            if mine {
                if let Some(g) = p.git.as_mut() {
                    g.refresh();
                }
            }
        }
    }

    /// A generated two-word branch name that isn't already taken in the active repo —
    /// what the new-worktree prompt opens pre-filled with, and what `Ctrl+R` rerolls.
    pub(crate) fn generated_branch_name(&self) -> String {
        let taken: Vec<String> = self
            .active_git()
            .map(|g| g.branches.iter().map(|b| b.name.clone()).collect())
            .unwrap_or_default();
        crate::worktree::generate_name(&taken)
    }

    /// Cut a worktree for `branch` off the active project's repository and open it as
    /// a project. An existing branch is checked out; a new one is created from
    /// whatever the main checkout has out, and that base is remembered for `M`.
    pub(crate) fn create_worktree(&mut self, branch: &str) {
        let branch = branch.trim().to_string();
        if branch.is_empty() {
            return;
        }
        // Always act on the repository's main checkout: `git worktree` bookkeeping,
        // the base branch and parentage all belong there, not to a linked worktree.
        let dir = self.projects[self.family_root(self.active)].dir.clone();
        let Some(repo) = crate::git::main_worktree(&dir).map(|p| config::canonical(&p)) else {
            self.flash("not a git repository");
            return;
        };
        let Some(path) = crate::worktree::path_for(&repo, &branch) else {
            self.flash("can't locate ~/.mmux (is HOME set?)");
            return;
        };
        if path.exists() {
            self.flash(format!("a worktree for “{branch}” already exists"));
            return;
        }
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.flash(format!("couldn't create the worktree directory — {e}"));
                return;
            }
        }
        // A directory deleted behind git's back leaves an entry that would otherwise
        // block re-creating the same branch's worktree here.
        crate::git::worktree_prune(&repo);

        let existing = crate::git::branch_exists(&repo, &branch);
        let base = crate::git::status(&repo).branch;
        if !existing && base.is_empty() {
            self.flash("main checkout is on a detached HEAD — check out a branch first");
            return;
        }
        let from = (!existing).then_some(base.as_str());
        if let Err(e) = crate::git::worktree_add(&repo, &path, &branch, from) {
            self.flash(first_line(&e));
            return;
        }
        if !existing {
            let _ = crate::git::config_set(&repo, &base_key(&branch), &base);
        }

        // The gitignored files a checkout needs to actually run (see `worktree::prepare`).
        let wcfg = self.projects[self.family_root(self.active)]
            .cfg
            .worktrees
            .clone();
        let copied = crate::worktree::prepare(&repo, &path, wcfg.as_ref());

        self.sync_worktree_projects();
        let canon = config::canonical(&path);
        let Some(pi) = self.projects.iter().position(|p| p.dir == canon) else {
            self.flash(format!("created “{branch}”, but the workspace is full"));
            return;
        };
        self.focus_project(pi);

        // The one-time setup command runs as an ordinary terminal in the new project:
        // you watch it work, and the row prunes itself when it finishes.
        let setup = wcfg
            .as_ref()
            .and_then(|w| w.setup.clone())
            .filter(|s| !s.trim().is_empty());
        if let Some(setup) = setup {
            let (rows, cols) = self.last_inner;
            let recipe = Recipe::shell_line(&canon, &setup);
            let mut s = Session::new("⚙ setup".into(), Kind::Terminal, recipe, pi);
            s.spawn(rows, cols);
            self.sessions.push(s);
            self.select_session(self.sessions.len() - 1);
            self.focus = Focus::Terminal;
        }
        let extra = if copied.is_empty() {
            String::new()
        } else {
            format!(" · copied {}", copied.join(", "))
        };
        self.flash(format!("worktree ⑂ {branch}{extra}"));
    }

    /// Where worktree `pi` merges back to: what mmux recorded when it cut the branch,
    /// falling back to whatever the main checkout has out now (for a branch made
    /// outside mmux, or a repo config that was cleaned).
    pub(super) fn worktree_base(&self, pi: usize) -> Option<String> {
        let wt = self.projects[pi].worktree.as_ref()?;
        crate::git::config_get(&wt.parent, &base_key(&wt.branch))
            .or_else(|| Some(crate::git::status(&wt.parent).branch).filter(|b| !b.is_empty()))
    }

    /// `M`: merge the active worktree back into the branch it came from. Every
    /// precondition is checked *here*, before the modal, so a refusal names the thing
    /// to fix instead of failing later with git's version of it.
    pub(crate) fn merge_worktree_prompt(&mut self) {
        let pi = self.active;
        let Some(wt) = self.projects[pi].worktree.as_ref() else {
            self.flash("not a worktree — M merges a worktree into its base branch");
            return;
        };
        let (branch, repo) = (wt.branch.clone(), wt.parent.clone());
        if !crate::git::is_clean(&self.projects[pi].dir) {
            self.flash(format!(
                "⑂ {branch} has uncommitted changes — commit or stash first"
            ));
            return;
        }
        let Some(base) = self.worktree_base(pi) else {
            self.flash("can't tell what this branched from");
            return;
        };
        // The merge happens in the main checkout, so it has to be sitting on the base
        // branch and be clean. Merging into a branch that isn't out, or over someone's
        // work in progress, is exactly the surprise worktrees are meant to avoid.
        let on = crate::git::status(&repo).branch;
        if on != base {
            self.flash(format!(
                "main checkout is on {on}, not {base} — switch it first"
            ));
            return;
        }
        if !crate::git::is_clean(&repo) {
            self.flash(format!(
                "{base} has uncommitted changes — commit or stash first"
            ));
            return;
        }
        self.overlay = Some(Overlay::confirm(
            "Merge worktree",
            format!(
                "Merge ⑂ {branch} into {base}?\n\
                 Removing afterwards deletes the checkout and the branch."
            ),
            "y merge & remove · m merge only · n cancel",
            Confirmed::MergeWorktree {
                project: pi,
                branch,
                base,
                remove: true,
            },
        ));
    }

    /// Scheduled counterpart to `M`: merge without removing the worktree, after
    /// re-checking the remembered base and main checkout at execution time.
    pub(crate) fn merge_worktree_scheduled(&mut self, pi: usize) {
        let Some(wt) = self.projects.get(pi).and_then(|p| p.worktree.as_ref()) else {
            self.flash("scheduled merge failed — project is no longer a worktree");
            return;
        };
        let (branch, repo) = (wt.branch.clone(), wt.parent.clone());
        let Some(base) = self.worktree_base(pi) else {
            self.flash("scheduled merge failed — can't tell what this branched from");
            return;
        };
        let on = crate::git::status(&repo).branch;
        if on != base {
            self.flash(format!(
                "scheduled merge failed — main checkout is on {on}, not {base}"
            ));
            return;
        }
        if !crate::git::is_clean(&repo) {
            self.flash(format!(
                "scheduled merge failed — {base} has uncommitted changes"
            ));
            return;
        }
        self.merge_worktree(pi, &branch, &base, false);
    }

    /// Run the confirmed merge. `remove` folds the usual follow-up — the branch is
    /// merged, so the checkout has done its job — into the same keystroke.
    pub(crate) fn merge_worktree(&mut self, pi: usize, branch: &str, base: &str, remove: bool) {
        // The modal can outlive the project it was opened on (a reload, another
        // removal), so re-check identity rather than trusting the stashed index.
        let Some(wt) = self.projects.get(pi).and_then(|p| p.worktree.as_ref()) else {
            return;
        };
        if wt.branch != branch {
            return;
        }
        let repo = wt.parent.clone();
        match crate::git::merge(&repo, branch) {
            Ok(msg) => self.flash(format!("{} → {base}", first_line(&msg))),
            Err(e) => {
                self.flash(first_line(&e));
                return;
            }
        }
        if remove {
            self.remove_worktree(pi, branch);
        }
        self.refresh_repo_panels(&repo);
    }

    /// `X`: ask before taking a worktree away. A checkout with uncommitted changes
    /// gets the blunter wording, since removing it really does lose them.
    pub(crate) fn remove_worktree_prompt(&mut self) {
        let pi = self.active;
        let Some(wt) = self.projects[pi].worktree.as_ref() else {
            self.flash("not a worktree — X removes a worktree");
            return;
        };
        let branch = wt.branch.clone();
        let repo = wt.parent.clone();
        // The warning is worth only as much as it is specific, so it names what this
        // removal would actually cost: uncommitted work is gone for good, unmerged
        // commits survive on a kept branch, and merged/pushed work costs nothing.
        let dirty = !crate::git::is_clean(&self.projects[pi].dir);
        let state = self
            .worktree_base(pi)
            .map(|base| (crate::git::integration(&repo, &branch, &base), base));
        let (body, hint) = if dirty {
            (
                format!(
                    "⑂ {branch} has uncommitted changes.\n\
                     Removing deletes the checkout and loses them for good."
                ),
                "y remove & discard · n cancel",
            )
        } else {
            let detail = match &state {
                Some((i, base)) if i.merged => {
                    format!("It's merged into {base} — nothing is lost.")
                }
                Some((i, _)) if i.pushed => {
                    "It's pushed to its upstream — the commits stay on the remote,\n\
                     and the branch is kept."
                        .to_string()
                }
                Some((i, base)) if i.ahead > 0 => format!(
                    "It has {} commit(s) not in {base} and not pushed.\n\
                     The checkout goes; the branch is kept, so they aren't lost.",
                    i.ahead
                ),
                _ => "The branch is deleted too, unless it has unmerged commits.".to_string(),
            };
            (
                format!("Remove the worktree for ⑂ {branch}?\n{detail}"),
                "y remove · n cancel",
            )
        };
        self.overlay = Some(Overlay::confirm(
            "Remove worktree",
            body,
            hint,
            Confirmed::RemoveWorktree {
                project: pi,
                branch,
            },
        ));
    }

    /// Take a worktree away: its panes die with the project, the checkout is removed,
    /// and the branch is deleted **if git agrees it's merged**. An unmerged branch is
    /// deliberately kept and said so — the checkout is disposable, the commits aren't.
    pub(crate) fn remove_worktree(&mut self, pi: usize, branch: &str) {
        if let Some(note) = self.take_worktree(pi, branch) {
            self.flash(note);
        }
    }

    /// The removal itself, shared by the `X` confirmation and the idle reaper.
    /// Returns the note to show, or `None` when it wasn't a worktree to begin with.
    fn take_worktree(&mut self, pi: usize, branch: &str) -> Option<String> {
        let wt = self.projects.get(pi).and_then(|p| p.worktree.as_ref())?;
        // The modal (or the reaper's scan) can outlive the project it targeted, so
        // re-check identity rather than trusting the stashed index.
        if wt.branch != branch {
            return None;
        }
        let repo = wt.parent.clone();
        let path = self.projects[pi].dir.clone();

        // If this checkout is the one currently holding the family's dev stack, hand
        // it back to the parent instead of letting it die with the directory —
        // merging a worktree shouldn't cost you your dev server.
        let root = self.family_root(pi);
        let root_dir = self.projects[root].dir.clone();
        let returning: Vec<String> = if root == pi {
            Vec::new()
        } else {
            self.sessions
                .iter()
                .filter(|s| {
                    s.project == pi
                        && s.kind == Kind::Process
                        && s.is_running()
                        && self.projects[root]
                            .cfg
                            .processes
                            .iter()
                            .any(|p| p.name == s.name)
                })
                .map(|s| s.name.clone())
                .collect()
        };
        // Stop them here rather than through the removal, so their teardown commands
        // can be waited on before the parent's copies start (see `Swap::Draining`).
        let mut children = Vec::new();
        let stopping: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.project == pi && returning.contains(&s.name))
            .map(|(i, _)| i)
            .collect();
        for i in stopping {
            if let Some(mut cmd) = self.sessions[i].stop_command() {
                if let Ok(child) = cmd.spawn() {
                    children.push(child);
                }
            }
            self.sessions[i].stop();
        }

        // Panes first — the checkout can't go while something is still living in it.
        self.remove_projects(&HashSet::from([pi]));

        // Indices moved with the removal, so re-find the parent by directory.
        if !returning.is_empty() {
            if let Some(root) = self.projects.iter().position(|p| p.dir == root_dir) {
                self.swap = Some(Swap::Draining {
                    project: root,
                    names: returning,
                    children,
                    deadline: Instant::now() + SWAP_DRAIN_WAIT,
                });
                self.poll_swap_drain();
            }
        }
        // Forced: the confirmation already said what would be lost, and the reaper
        // only ever gets here with a clean tree.
        if let Err(e) = crate::git::worktree_remove(&repo, &path, true) {
            // Still on disk, so put its project back rather than losing sight of it.
            self.sync_worktree_projects();
            return Some(format!("couldn't remove worktree — {}", first_line(&e)));
        }
        let note = match crate::git::delete_branch(&repo, branch) {
            Ok(()) => format!("removed ⑂ {branch}"),
            Err(_) => format!("removed ⑂ {branch}'s checkout — branch kept (unmerged)"),
        };
        crate::git::config_unset(&repo, &base_key(branch));
        self.refresh_repo_panels(&repo);
        Some(note)
    }

    /// Clear away worktrees that are **finished**: nothing running in them, a clean
    /// tree, every commit already merged or pushed, and idle for
    /// [`worktrees.reap`](crate::config::WorktreeConfig::reap).
    ///
    /// This is what makes a worktree genuinely disposable — cut one, let an agent work
    /// in it, merge it, and it tidies itself up instead of accumulating. It is
    /// deliberately timid: every condition must hold, and the last one means it only
    /// ever removes a *checkout* whose contents are already somewhere else. Nothing it
    /// deletes is unrecoverable, and it says what it did.
    ///
    /// The git checks are a handful of forks per candidate, so the scan runs on
    /// [`REAP_SCAN_EVERY`] rather than every frame, and only touches worktrees whose
    /// idle clock has already run out.
    pub(crate) fn step_worktree_reaper(&mut self) {
        let now = Instant::now();
        // Refresh every worktree's idle clock first — cheap, in-memory, every tick, so
        // a long-running agent keeps its checkout alive without any git work at all.
        for pi in 0..self.projects.len() {
            if self.projects[pi].worktree.is_none() {
                continue;
            }
            let busy = pi == self.active
                || self
                    .sessions
                    .iter()
                    .any(|s| s.project == pi && s.pane.is_some());
            if busy {
                if let Some(wt) = self.projects[pi].worktree.as_mut() {
                    wt.last_busy = now;
                }
            }
        }
        if now < self.next_reap_scan {
            return;
        }
        self.next_reap_scan = now + REAP_SCAN_EVERY;

        // Collect first: the checks fork `git`, and removing shifts project indices.
        let mut ripe: Vec<(usize, String, Duration)> = Vec::new();
        for pi in 0..self.projects.len() {
            let Some(wt) = self.projects[pi].worktree.as_ref() else {
                continue;
            };
            let idle = wt.last_busy.elapsed();
            // The reap delay is the *parent's* setting: worktrees share the repo's
            // config, and it's the project you configured that owns the policy.
            let root = self.family_root(pi);
            let Some(after) =
                config::worktree_reap_after(self.projects[root].cfg.worktrees.as_ref())
            else {
                continue;
            };
            if idle < after {
                continue;
            }
            let (branch, repo, dir) = (
                wt.branch.clone(),
                wt.parent.clone(),
                self.projects[pi].dir.clone(),
            );
            if !crate::git::is_clean(&dir) {
                continue;
            }
            // No base ⇒ nothing to measure "merged" against; leave it alone.
            let Some(base) = self.worktree_base(pi) else {
                continue;
            };
            if crate::git::integration(&repo, &branch, &base).is_safe() {
                ripe.push((pi, branch, idle));
            }
        }
        // Back to front, so the earlier indices stay valid as projects are removed.
        ripe.sort_by(|a, b| b.0.cmp(&a.0));
        for (pi, branch, idle) in ripe {
            if self.take_worktree(pi, &branch).is_some() {
                self.flash(format!(
                    "cleared ⑂ {branch} — finished and idle {}",
                    human_duration(idle)
                ));
            }
        }
    }

    // ── The dev stack follows the checkout you're in ──────────────────────────

    /// The processes that would move into `to`: ones running in a *sibling* checkout
    /// of the same repository that `to` also defines. Empty ⇒ nothing to do, which is
    /// the common case and the reason switching around costs nothing.
    ///
    /// Only processes both sides define move. Same repo means the same `mmux.yaml`, so
    /// in practice that's all of them — but it guarantees mmux never stops something it
    /// has no way to start again.
    fn stack_movable_to(&self, to: usize) -> Vec<String> {
        let root = self.family_root(to);
        let mut names: Vec<String> = Vec::new();
        for s in &self.sessions {
            if s.kind != Kind::Process || !s.is_running() || s.project == to {
                continue;
            }
            if self.family_root(s.project) != root {
                continue;
            }
            if !self.projects[to]
                .cfg
                .processes
                .iter()
                .any(|p| p.name == s.name)
            {
                continue;
            }
            if !names.contains(&s.name) {
                names.push(s.name.clone());
            }
        }
        names
    }

    /// Per-tick driver for the stack swap (see [`Swap`]).
    ///
    /// One checkout of a repository runs its processes at a time, so a worktree's dev
    /// server reuses its parent's ports and there is never a question of which branch
    /// is on :3000. What moves is the set of *running* process names — with nothing
    /// running, switching checkouts does nothing at all.
    pub(crate) fn step_stack_swap(&mut self) {
        // A move already under way owns the stack until it lands.
        if matches!(self.swap, Some(Swap::Draining { .. })) {
            self.poll_swap_drain();
            return;
        }
        if self.stack_movable_to(self.active).is_empty() {
            self.swap = None;
            return;
        }
        match self.swap {
            Some(Swap::Waiting { project, since }) if project == self.active => {
                if since.elapsed() >= SWAP_DWELL {
                    self.begin_stack_swap(self.active);
                }
            }
            // First tick resting here, or the selection moved on to somewhere else:
            // restart the dwell against the current project.
            _ => {
                self.swap = Some(Swap::Waiting {
                    project: self.active,
                    since: Instant::now(),
                })
            }
        }
    }

    /// Stop the family's copies of the moving processes and hand off to the drain
    /// phase. Stopping goes through the ordinary teardown path, so a `stop:` (a
    /// `docker compose down`) runs exactly as it would on a manual stop.
    fn begin_stack_swap(&mut self, to: usize) {
        let names = self.stack_movable_to(to);
        if names.is_empty() {
            self.swap = None;
            return;
        }
        let root = self.family_root(to);
        let stopping: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.kind == Kind::Process
                    && s.is_running()
                    && s.project != to
                    && names.contains(&s.name)
                    && self.family_root(s.project) == root
            })
            .map(|(i, _)| i)
            .collect();
        let mut children = Vec::new();
        for i in stopping {
            if let Some(mut cmd) = self.sessions[i].stop_command() {
                if let Ok(child) = cmd.spawn() {
                    children.push(child);
                }
            }
            self.sessions[i].stop();
        }
        self.swap = Some(Swap::Draining {
            project: to,
            names,
            children,
            deadline: Instant::now() + SWAP_DRAIN_WAIT,
        });
        // Nothing to wait for (no `stop:` anywhere) — killing the panes freed the
        // ports already, so land it in the same tick.
        self.poll_swap_drain();
    }

    /// Finish a swap once the old checkout's teardown commands have exited. Starting
    /// before they do would hand the new dev server a port the old stack is still
    /// holding — the one ordering that makes same-port worktrees actually work.
    fn poll_swap_drain(&mut self) {
        let Some(Swap::Draining {
            project,
            names,
            children,
            deadline,
        }) = &mut self.swap
        else {
            return;
        };
        children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        if !children.is_empty() && Instant::now() < *deadline {
            return;
        }
        let (to, names) = (*project, std::mem::take(names));
        self.swap = None;
        let (rows, cols) = self.last_inner;
        let mut started = 0usize;
        for name in &names {
            let found = self.sessions.iter().position(|s| {
                s.project == to && s.kind == Kind::Process && s.name == *name && !s.is_running()
            });
            if let Some(i) = found {
                self.sessions[i].spawn(rows, cols);
                started += 1;
            }
        }
        if started > 0 {
            let label = self.projects.get(to).map(|p| p.label()).unwrap_or_default();
            self.flash(format!("stack → {label} ({started} running)"));
        }
    }
}
