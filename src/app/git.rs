//! The native git panel — a small, lazygit-flavoured replacement for an embedded
//! git TUI in the right column.
//!
//! It renders as three bordered boxes (Changes · Branches · Commits, see
//! [`view::git`](super::view::git)). The keyboard cursor lives in one [`Section`]
//! at a time — Tab cycles between them. This is **not** a pane: there's no child
//! process and no vt100 grid, just state we draw ourselves and drive with our own
//! keys ([`App::key_git`](super::input)). One panel lives per project; the visible
//! one is the active project's. A selected commit's diff opens in the centre pager
//! (`git show`, see [`DiffView`]).
//!
//! Network ops (pull/push) block, so they run on a throwaway thread and report
//! back over a channel drained in [`App::tick`](super::App::tick). Status, branch
//! and log reads are cheap synchronous forks, refreshed on a throttle while the
//! panel is visible and immediately after any mutating action.

use crate::git::{self, Branch, Commit, FileEntry, Stage, TreeRow};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::diff::DiffView;
use super::overlay::{Confirmed, Overlay};
use super::App;

/// How often the visible panel re-reads git state (picks up commits an agent makes
/// in the main pane). A couple of cheap `git` forks.
const REFRESH_EVERY: Duration = Duration::from_millis(1500);
/// Inactive project boxes need only branch + changed-path counts. Refresh those less
/// often and without paying for branches/log until the project becomes visible.
const STATUS_REFRESH_EVERY: Duration = Duration::from_secs(5);
/// How many recent commits to keep for the Commits box. Enough to scroll through real
/// history; reading this many `git log` subjects is still a sub-millisecond fork.
const LOG_LINES: usize = 200;

/// Which sub-box the keyboard cursor drives. Tab cycles through all three.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Changes,
    Branches,
    Commits,
}

/// The result of a backgrounded network op, sent back from its worker thread.
pub(crate) struct JobDone {
    pub verb: &'static str, // "pull" / "push"
    pub result: Result<String, String>,
}

/// The two backgrounded remote ops. One place ties each to its busy-label, its
/// `JobDone.verb` tag, and the `git` fn it runs — the string used to be spelled out
/// separately in `git_start` and `start_job`.
pub(crate) enum RemoteOp {
    Pull,
    Push,
}
impl RemoteOp {
    fn label(&self) -> &'static str {
        match self {
            RemoteOp::Pull => "pulling…",
            RemoteOp::Push => "pushing…",
        }
    }
    fn verb(&self) -> &'static str {
        match self {
            RemoteOp::Pull => "pull",
            RemoteOp::Push => "push",
        }
    }
    fn run(&self, dir: &std::path::Path) -> Result<String, String> {
        match self {
            RemoteOp::Pull => git::pull(dir),
            RemoteOp::Push => git::push(dir),
        }
    }
}

pub(crate) struct GitPanel {
    pub dir: PathBuf,
    pub branch: String,
    pub files: Vec<FileEntry>,
    /// The flattened directory tree drawn in the Changes box (root → dirs → files),
    /// rebuilt from `files` on every refresh. The Changes cursor indexes this.
    pub rows: Vec<TreeRow>,
    pub branches: Vec<Branch>,
    pub log: Vec<Commit>,
    /// Which box currently has the cursor.
    pub section: Section,
    /// Cursor into `rows` (the Changes box's tree).
    pub cursor: usize,
    /// Cursor into `branches` (the Branches box).
    pub branch_cursor: usize,
    /// Cursor into `log` (the Commits box).
    pub commit_cursor: usize,
    /// A network op in flight: the present-tense label to show ("pushing…").
    pub busy: Option<&'static str>,
    last_refresh: Option<Instant>,
    last_status_refresh: Option<Instant>,
    tx: Sender<JobDone>,
    rx: Receiver<JobDone>,
}

impl GitPanel {
    /// Build a panel for `dir` and take an initial snapshot.
    pub(crate) fn new(dir: PathBuf) -> GitPanel {
        let (tx, rx) = mpsc::channel();
        let mut g = GitPanel {
            dir,
            branch: String::new(),
            files: Vec::new(),
            rows: Vec::new(),
            branches: Vec::new(),
            log: Vec::new(),
            section: Section::Changes,
            cursor: 0,
            branch_cursor: 0,
            commit_cursor: 0,
            busy: None,
            last_refresh: None,
            last_status_refresh: None,
            tx,
            rx,
        };
        g.refresh();
        g
    }

    /// Re-read status, branches and log from disk; keep both cursors in range. Cheap;
    /// called after every mutating action and (throttled) each visible frame.
    pub(crate) fn refresh(&mut self) {
        let st = git::status(&self.dir);
        self.branch = st.branch;
        self.files = st.files;
        self.rows = git::tree_rows(&self.files);
        self.branches = git::branches(&self.dir);
        self.log = git::log(&self.dir, LOG_LINES);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.branch_cursor = self.branch_cursor.min(self.branches.len().saturating_sub(1));
        self.commit_cursor = self.commit_cursor.min(self.log.len().saturating_sub(1));
        self.last_refresh = Some(Instant::now());
        self.last_status_refresh = self.last_refresh;
    }

    /// Throttled refresh for the per-frame tick, so external changes show up without
    /// forking `git` on every one of the loop's ~20 frames/sec.
    pub(crate) fn maybe_refresh(&mut self) {
        let due = self
            .last_refresh
            .map(|t| t.elapsed() >= REFRESH_EVERY)
            .unwrap_or(true);
        if due {
            self.refresh();
        }
    }

    /// Slower status-only refresh for collapsed background projects. The full branch
    /// list and commit log are refreshed when this project becomes active again.
    pub(crate) fn maybe_refresh_status(&mut self) {
        let due = self
            .last_status_refresh
            .map(|t| t.elapsed() >= STATUS_REFRESH_EVERY)
            .unwrap_or(true);
        if !due {
            return;
        }
        let st = git::status(&self.dir);
        self.branch = st.branch;
        self.files = st.files;
        self.rows = git::tree_rows(&self.files);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.last_status_refresh = Some(Instant::now());
    }

    /// Move the cursor within the active section.
    pub(crate) fn move_cursor(&mut self, delta: i32) {
        match self.section {
            Section::Changes => self.cursor = step(self.cursor, self.rows.len(), delta),
            Section::Branches => {
                self.branch_cursor = step(self.branch_cursor, self.branches.len(), delta)
            }
            Section::Commits => self.commit_cursor = step(self.commit_cursor, self.log.len(), delta),
        }
    }

    /// Tab cycles the cursor forward through the three boxes.
    pub(crate) fn toggle_section(&mut self) {
        self.section = match self.section {
            Section::Changes => Section::Branches,
            Section::Branches => Section::Commits,
            Section::Commits => Section::Changes,
        };
    }

    pub(crate) fn selected_branch(&self) -> Option<&Branch> {
        self.branches.get(self.branch_cursor)
    }

    /// The commit under the Commits cursor (what `y`/`m`/revert/reset/⏎ act on).
    pub(crate) fn selected_commit(&self) -> Option<&Commit> {
        self.log.get(self.commit_cursor)
    }

    /// What `d` discards given the cursor: a `(pathspec, confirmation prompt)` pair for
    /// the selected file, directory, or the whole tree (root). `None` only when the
    /// tree is empty.
    pub(crate) fn discard_target(&self) -> Option<(String, String)> {
        match self.rows.get(self.cursor) {
            Some(TreeRow::Dir { path, .. }) => {
                Some((path.clone(), format!("Discard all changes in {path}/ ?")))
            }
            Some(TreeRow::File { idx, .. }) => self
                .files
                .get(*idx)
                .map(|f| (f.path.clone(), format!("Discard changes to {} ?", f.path))),
            None => None,
        }
    }

    /// Throw away all changes under `path` (see [`git::discard`]), then refresh.
    pub(crate) fn discard(&mut self, path: &str) -> Result<(), String> {
        let res = git::discard(&self.dir, path);
        self.refresh();
        res
    }

    /// Stash everything (see [`git::stash`]), then refresh.
    pub(crate) fn stash(&mut self) -> Result<String, String> {
        let res = git::stash(&self.dir);
        self.refresh();
        res
    }

    /// Stage / unstage whatever the cursor is on, then refresh. The cursor can be a file
    /// (just that path) or a directory (everything under it — `git add <dir>`); stage the
    /// whole repo with `a` instead. Already-staged targets toggle back to unstaged, so the
    /// checkbox flips. Whole-file/-dir only — no hunk staging.
    pub(crate) fn toggle_selected(&mut self) -> Result<(), String> {
        let res = match self.rows.get(self.cursor) {
            None => return Ok(()),
            Some(TreeRow::Dir { path, staged, .. }) => {
                let (path, all) = (path.clone(), *staged == Stage::All);
                if all {
                    git::unstage(&self.dir, &path)
                } else {
                    git::stage(&self.dir, &path)
                }
            }
            Some(TreeRow::File { idx, .. }) => {
                let f = &self.files[*idx];
                let (path, staged) = (f.path.clone(), f.staged);
                if staged {
                    git::unstage(&self.dir, &path)
                } else {
                    git::stage(&self.dir, &path)
                }
            }
        };
        self.refresh();
        res
    }

    /// Stage everything — or, when it's all already staged, unstage everything. This is
    /// the `a` key, and the home of the whole-repo toggle the (now removed) root row used
    /// to host.
    pub(crate) fn toggle_all(&mut self) -> Result<(), String> {
        let all_staged = !self.files.is_empty() && self.files.iter().all(|f| f.staged);
        let res = if all_staged {
            git::unstage_all(&self.dir)
        } else {
            git::stage_all(&self.dir)
        };
        self.refresh();
        res
    }

    /// Commit the staged index — but if nothing is staged, stage everything first so a
    /// plain "just commit" captures the whole working tree.
    pub(crate) fn commit(&mut self, msg: &str) -> Result<String, String> {
        if !self.files.iter().any(|f| f.staged) {
            git::stage_all(&self.dir)?;
        }
        let res = git::commit(&self.dir, msg);
        self.refresh();
        res
    }

    pub(crate) fn switch(&mut self, name: &str) -> Result<(), String> {
        let res = git::switch(&self.dir, name);
        self.refresh();
        res
    }

    pub(crate) fn create_branch(&mut self, name: &str) -> Result<(), String> {
        let res = git::create_branch(&self.dir, name);
        self.refresh();
        res
    }

    /// Revert a commit (see [`git::revert`]), then refresh so the new commit shows.
    pub(crate) fn revert(&mut self, hash: &str) -> Result<String, String> {
        let res = git::revert(&self.dir, hash);
        self.refresh();
        res
    }

    /// Soft-reset HEAD to a commit (see [`git::soft_reset`]), then refresh so the moved
    /// tip and the newly-staged changes show.
    pub(crate) fn soft_reset(&mut self, hash: &str) -> Result<String, String> {
        let res = git::soft_reset(&self.dir, hash);
        self.refresh();
        res
    }

    /// Kick off a background pull/push. A no-op if one is already running, so a
    /// double-tap can't launch two.
    pub(crate) fn start_job(&mut self, op: RemoteOp) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(op.label());
        let tx = self.tx.clone();
        let dir = self.dir.clone();
        thread::spawn(move || {
            let _ = tx.send(JobDone { verb: op.verb(), result: op.run(&dir) });
        });
    }

    /// Drain finished network jobs; on completion clear `busy` and refresh. Returns
    /// the finished jobs so the app can flash their outcome. Called from `tick`.
    pub(crate) fn poll_jobs(&mut self) -> Vec<JobDone> {
        let mut done = Vec::new();
        while let Ok(j) = self.rx.try_recv() {
            done.push(j);
        }
        if !done.is_empty() {
            self.busy = None;
            self.refresh();
        }
        done
    }
}

/// Clamp-step a cursor within `len` items.
fn step(cursor: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (cursor as i32 + delta).clamp(0, len as i32 - 1) as usize
}

/// First line of a (possibly multi-line) git message — keeps the footer flash to
/// one line. Shared with [`App::tick`](super::App::tick) for job results.
pub(crate) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

/// Git-panel actions driven from the right column's keymap / footer buttons. Each
/// resolves the active project's panel, runs the op, and flashes the outcome; the
/// panel refreshes itself after every mutation.
impl App {
    pub(crate) fn active_git(&self) -> Option<&GitPanel> {
        self.projects[self.active].git.as_ref()
    }
    pub(crate) fn active_git_mut(&mut self) -> Option<&mut GitPanel> {
        self.projects[self.active].git.as_mut()
    }

    /// Tab: cycle the cursor through the Changes → Branches → Commits boxes.
    pub(crate) fn git_section_toggle(&mut self) {
        if let Some(g) = self.active_git_mut() {
            g.toggle_section();
        }
    }

    /// Jump the cursor straight to the Branches box.
    pub(crate) fn git_focus_branches(&mut self) {
        if let Some(g) = self.active_git_mut() {
            g.section = Section::Branches;
        }
    }

    /// Enter/Space: stage the cursor's file/dir/root (Changes), switch to the branch
    /// (Branches), or show the commit's diff in the main pane (Commits).
    pub(crate) fn git_activate(&mut self) {
        match self.active_git().map(|g| g.section) {
            Some(Section::Changes) => self.git_toggle_stage(),
            Some(Section::Branches) => self.git_switch_selected(),
            Some(Section::Commits) => self.git_show_commit(),
            None => {}
        }
    }

    pub(crate) fn git_toggle_stage(&mut self) {
        let r = self.active_git_mut().map(|g| g.toggle_selected());
        self.flash_err(r);
    }

    pub(crate) fn git_toggle_all(&mut self) {
        let r = self.active_git_mut().map(|g| g.toggle_all());
        self.flash_err(r);
    }

    /// Switch to the branch under the Branches cursor (no-op if already current).
    pub(crate) fn git_switch_selected(&mut self) {
        let name = self
            .active_git()
            .and_then(|g| g.selected_branch())
            .filter(|b| !b.current)
            .map(|b| b.name.clone());
        if let Some(name) = name {
            self.git_switch(&name);
        }
    }

    pub(crate) fn git_switch(&mut self, name: &str) {
        match self.active_git_mut().map(|g| g.switch(name)) {
            Some(Ok(())) => self.flash(format!("switched to {name}")),
            Some(Err(e)) => self.flash(first_line(&e)),
            _ => {}
        }
    }

    /// Kick off a background pull (`verb == "pull"`) or push (any other value).
    pub(crate) fn git_start(&mut self, verb: &'static str) {
        let op = if verb == "pull" { RemoteOp::Pull } else { RemoteOp::Push };
        if let Some(g) = self.active_git_mut() {
            g.start_job(op);
        }
    }

    /// Open the discard confirmation for the cursor's file/dir/root. Discard is
    /// destructive, so it always routes through the yes/no modal.
    pub(crate) fn git_discard_prompt(&mut self) {
        if let Some((path, body)) = self.active_git().and_then(|g| g.discard_target()) {
            self.overlay = Some(Overlay::confirm(
                "Discard",
                body,
                "y discard · n cancel",
                Confirmed::Discard { path },
            ));
        }
    }

    /// Stash everything (recoverable with `git stash pop`), flashing the outcome.
    pub(crate) fn git_stash(&mut self) {
        let r = self.active_git_mut().map(|g| g.stash());
        self.flash_result(r);
    }

    pub(crate) fn git_commit_prompt(&mut self) {
        self.overlay = Some(Overlay::commit());
    }

    pub(crate) fn git_newbranch_prompt(&mut self) {
        self.overlay = Some(Overlay::new_branch(String::new()));
    }

    /// Open (or replace) the centre-pane diff preview for the file under the Changes
    /// cursor. A no-op when the cursor is on a directory/root row, so navigating onto
    /// a folder leaves the last file's diff up rather than blanking the pane.
    pub(crate) fn git_open_diff(&mut self) {
        let proj = self.active;
        let built = self.active_git().and_then(|g| match g.rows.get(g.cursor) {
            Some(TreeRow::File { idx, .. }) => {
                g.files.get(*idx).map(|f| DiffView::build(proj, &g.dir, f))
            }
            _ => None,
        });
        if let Some(view) = built {
            self.diff = Some(view);
        }
    }

    /// Show the selected commit's full diff (`git show`) in the centre pager. Only acts
    /// in the Commits section — the same method backs Enter, `v`, and a commit click.
    pub(crate) fn git_show_commit(&mut self) {
        let proj = self.active;
        let built = self
            .active_git()
            .filter(|g| g.section == Section::Commits)
            .and_then(|g| g.selected_commit().map(|c| DiffView::build_commit(proj, &g.dir, c)));
        if let Some(view) = built {
            self.diff = Some(view);
        }
    }

    /// Copy the selected commit's hash — the short hash, or the full 40-char id with
    /// `full`. Commits-section only; flashes what it copied.
    pub(crate) fn git_copy_commit_hash(&mut self, full: bool) {
        let hash = self
            .active_git()
            .filter(|g| g.section == Section::Commits)
            .and_then(|g| g.selected_commit())
            .map(|c| if full { c.hash.clone() } else { c.short.clone() });
        if let Some(hash) = hash {
            crate::clipboard::copy(&hash);
            self.flash(format!("copied {hash}"));
        }
    }

    /// Copy the selected commit's full message (subject + body) to the clipboard.
    /// Commits-section only.
    pub(crate) fn git_copy_commit_message(&mut self) {
        let msg = self
            .active_git()
            .filter(|g| g.section == Section::Commits)
            .and_then(|g| g.selected_commit().map(|c| (g.dir.clone(), c.hash.clone())))
            .map(|(dir, hash)| git::commit_message(&dir, &hash));
        match msg {
            Some(Ok(m)) if !m.is_empty() => {
                crate::clipboard::copy(&m);
                self.flash("copied commit message");
            }
            Some(Err(e)) => self.flash(first_line(&e)),
            _ => {}
        }
    }

    /// Open the revert confirmation for the selected commit (Commits section only).
    pub(crate) fn git_revert_prompt(&mut self) {
        if let Some(c) = self
            .active_git()
            .filter(|g| g.section == Section::Commits)
            .and_then(|g| g.selected_commit())
        {
            let (hash, short, subject) = (c.hash.clone(), c.short.clone(), c.summary.clone());
            self.overlay = Some(Overlay::confirm(
                "Revert commit",
                format!("Revert {short} \"{subject}\"?\nCreates a new commit undoing it."),
                "y revert · n cancel",
                Confirmed::Revert { hash },
            ));
        }
    }

    /// Open the soft-reset ("uncommit to here") confirmation for the selected commit.
    /// Commits section only.
    pub(crate) fn git_soft_reset_prompt(&mut self) {
        if let Some(c) = self
            .active_git()
            .filter(|g| g.section == Section::Commits)
            .and_then(|g| g.selected_commit())
        {
            let (hash, short, subject) = (c.hash.clone(), c.short.clone(), c.summary.clone());
            self.overlay = Some(Overlay::confirm(
                "Reset to commit",
                format!(
                    "Soft-reset HEAD to {short} \"{subject}\"?\n\
                     Later commits become staged changes; nothing is lost."
                ),
                "y reset · n cancel",
                Confirmed::SoftReset { hash },
            ));
        }
    }

    /// Keep the open diff in step with the cursor (called after a cursor move). Does
    /// nothing unless a preview is already open — moving the cursor never *opens* one.
    /// Section-aware: in Commits it re-shows the newly-selected commit, elsewhere it
    /// re-reads the file under the Changes cursor.
    pub(crate) fn git_preview_follow(&mut self) {
        if self.diff.is_none() {
            return;
        }
        match self.active_git().map(|g| g.section) {
            Some(Section::Commits) => self.git_show_commit(),
            _ => self.git_open_diff(),
        }
    }

    /// `v`: open the current file's / commit's diff, or close it if one's already showing.
    pub(crate) fn git_toggle_diff(&mut self) {
        if self.diff.is_some() {
            self.diff = None;
        } else if self.active_git().map(|g| g.section) == Some(Section::Commits) {
            self.git_show_commit();
        } else {
            self.git_open_diff();
        }
    }

    pub(crate) fn clear_diff(&mut self) {
        self.diff = None;
    }

    /// Scroll the open diff pager by `delta` lines (positive = down), clamped to the
    /// body. A no-op when no diff is open.
    pub(crate) fn diff_scroll(&mut self, delta: i32) {
        if let Some(v) = self.diff.as_mut() {
            let max = v.lines.len().saturating_sub(1) as i32;
            v.scroll = (v.scroll as i32 + delta).clamp(0, max) as usize;
        }
    }

    /// Per-tick upkeep for the preview: drop it when its project is no longer active
    /// or its file stopped being changed (committed/discarded), and otherwise re-read
    /// it on a throttle so an agent's edits to the shown file appear live. Scroll is
    /// preserved across a re-read (clamped to the new length).
    pub(crate) fn diff_upkeep(&mut self) {
        let Some(view) = self.diff.as_ref() else {
            return;
        };
        if view.project != self.active {
            self.diff = None;
            return;
        }
        // A commit diff is static and tied to no working-tree file — keep it until the
        // user closes it or selects a session (it's never in `g.files` to re-find).
        if view.commit.is_some() {
            return;
        }
        let (path, scroll, due, is_image) = (
            view.path.clone(),
            view.scroll,
            view.built_at.elapsed() >= REFRESH_EVERY,
            view.image.is_some(),
        );
        let entry = self
            .active_git()
            .and_then(|g| g.files.iter().find(|f| f.path == path).cloned().map(|f| (g.dir.clone(), f)));
        match entry {
            None => self.diff = None, // committed or discarded — nothing left to show
            // A text diff re-reads on the throttle so an agent's live edits show; an
            // image is decoded once (re-click to refresh) — re-decoding it every 1.5s
            // would be needless work on the UI thread for no real gain.
            Some((dir, f)) if due && !is_image => {
                let mut nv = DiffView::build(self.active, &dir, &f);
                nv.scroll = scroll.min(nv.lines.len().saturating_sub(1));
                self.diff = Some(nv);
            }
            Some(_) => {}
        }
    }
}
