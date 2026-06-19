//! The native git panel — a small, lazygit-flavoured replacement for an embedded
//! git TUI in the right column.
//!
//! It renders as three bordered boxes (Changes · Branches · Recent, see
//! [`view::git`](super::view::git)). The keyboard cursor lives in one [`Section`]
//! at a time — Changes or Branches — with Recent display-only. This is **not** a
//! pane: there's no child process and no vt100 grid, just state we draw ourselves
//! and drive with our own keys ([`App::key_git`](super::input)). One panel lives
//! per project; the visible one is the active project's.
//!
//! Network ops (pull/push) block, so they run on a throwaway thread and report
//! back over a channel drained in [`App::tick`](super::App::tick). Status, branch
//! and log reads are cheap synchronous forks, refreshed on a throttle while the
//! panel is visible and immediately after any mutating action.

use crate::git::{self, Branch, Commit, FileEntry, Stage, TreeRow};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::App;

/// How often the visible panel re-reads git state (picks up commits an agent makes
/// in the main pane). A couple of cheap `git` forks.
const REFRESH_EVERY: Duration = Duration::from_millis(1500);
/// How many recent commits to keep for the history box.
const LOG_LINES: usize = 20;

/// Which sub-box the keyboard cursor drives. Recent is display-only, so not a section.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Changes,
    Branches,
}

/// The result of a backgrounded network op, sent back from its worker thread.
pub(crate) struct JobDone {
    pub verb: &'static str, // "pull" / "push"
    pub result: Result<String, String>,
}

pub(crate) struct GitPanel {
    pub dir: PathBuf,
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
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
    /// A network op in flight: the present-tense label to show ("pushing…").
    pub busy: Option<&'static str>,
    last_refresh: Option<Instant>,
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
            ahead: 0,
            behind: 0,
            files: Vec::new(),
            rows: Vec::new(),
            branches: Vec::new(),
            log: Vec::new(),
            section: Section::Changes,
            cursor: 0,
            branch_cursor: 0,
            busy: None,
            last_refresh: None,
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
        self.ahead = st.ahead;
        self.behind = st.behind;
        self.files = st.files;
        self.rows = git::tree_rows(&self.files);
        self.branches = git::branches(&self.dir);
        self.log = git::log(&self.dir, LOG_LINES);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.branch_cursor = self.branch_cursor.min(self.branches.len().saturating_sub(1));
        self.last_refresh = Some(Instant::now());
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

    /// Move the cursor within the active section.
    pub(crate) fn move_cursor(&mut self, delta: i32) {
        match self.section {
            Section::Changes => self.cursor = step(self.cursor, self.rows.len(), delta),
            Section::Branches => {
                self.branch_cursor = step(self.branch_cursor, self.branches.len(), delta)
            }
        }
    }

    pub(crate) fn toggle_section(&mut self) {
        self.section = match self.section {
            Section::Changes => Section::Branches,
            Section::Branches => Section::Changes,
        };
    }

    pub(crate) fn selected_branch(&self) -> Option<&Branch> {
        self.branches.get(self.branch_cursor)
    }

    /// What `d` discards given the cursor: a `(pathspec, confirmation prompt)` pair for
    /// the selected file, directory, or the whole tree (root). `None` only when the
    /// tree is empty.
    pub(crate) fn discard_target(&self) -> Option<(String, String)> {
        match self.rows.get(self.cursor) {
            Some(TreeRow::Root { .. }) => {
                Some((".".into(), "Discard ALL changes in the working tree?".into()))
            }
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

    /// Stage / unstage whatever the cursor is on, then refresh. The cursor can be a
    /// file (just that path), a directory (everything under it — `git add <dir>`), or
    /// the root (the whole repo). Already-staged targets toggle back to unstaged, so
    /// the checkbox flips. Whole-file/-dir only — no hunk staging.
    pub(crate) fn toggle_selected(&mut self) -> Result<(), String> {
        let res = match self.rows.get(self.cursor) {
            None => return Ok(()),
            Some(TreeRow::Root { staged }) => {
                if *staged == Stage::All {
                    git::unstage_all(&self.dir)
                } else {
                    git::stage_all(&self.dir)
                }
            }
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

    pub(crate) fn stage_all(&mut self) -> Result<(), String> {
        let res = git::stage_all(&self.dir);
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

    /// Kick off a background pull/push. A no-op if one is already running, so a
    /// double-tap can't launch two.
    pub(crate) fn start_job(&mut self, verb: &'static str, f: fn(&Path) -> Result<String, String>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(if verb == "pull" { "pulling…" } else { "pushing…" });
        let tx = self.tx.clone();
        let dir = self.dir.clone();
        thread::spawn(move || {
            let _ = tx.send(JobDone { verb, result: f(&dir) });
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

/// A modal over the whole UI: either a one-line text prompt (commit message /
/// new-branch name) or a yes/no confirmation (destructive discard). While open it eats
/// every key (see [`App::overlay_key`](super::input)).
pub(crate) enum Overlay {
    Prompt {
        title: &'static str,
        buf: String,
        kind: PromptKind,
    },
    Confirm {
        title: &'static str,
        body: String,
        action: Confirmed,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum PromptKind {
    Commit,
    NewBranch,
}

/// The deferred action a [`Overlay::Confirm`] runs when accepted.
#[derive(Clone)]
pub(crate) enum Confirmed {
    /// Discard all changes under this pathspec (a file, a dir, or `.` for everything).
    Discard { path: String },
}

impl Overlay {
    pub(crate) fn commit() -> Overlay {
        Overlay::Prompt {
            title: "Commit message",
            buf: String::new(),
            kind: PromptKind::Commit,
        }
    }

    pub(crate) fn new_branch(prefill: String) -> Overlay {
        Overlay::Prompt {
            title: "New branch",
            buf: prefill,
            kind: PromptKind::NewBranch,
        }
    }

    pub(crate) fn confirm(title: &'static str, body: String, action: Confirmed) -> Overlay {
        Overlay::Confirm { title, body, action }
    }
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

    /// Toggle which box (Changes ↔ Branches) the cursor drives.
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

    /// Enter/Space: stage the cursor's file/dir/root (Changes) or switch to the
    /// branch (Branches).
    pub(crate) fn git_activate(&mut self) {
        match self.active_git().map(|g| g.section) {
            Some(Section::Changes) => self.git_toggle_stage(),
            Some(Section::Branches) => self.git_switch_selected(),
            None => {}
        }
    }

    pub(crate) fn git_toggle_stage(&mut self) {
        if let Some(Err(e)) = self.active_git_mut().map(|g| g.toggle_selected()) {
            self.flash_git(first_line(&e));
        }
    }

    pub(crate) fn git_stage_all(&mut self) {
        if let Some(Err(e)) = self.active_git_mut().map(|g| g.stage_all()) {
            self.flash_git(first_line(&e));
        }
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
            Some(Ok(())) => self.flash_git(format!("switched to {name}")),
            Some(Err(e)) => self.flash_git(first_line(&e)),
            _ => {}
        }
    }

    /// Kick off a background pull (`verb == "pull"`) or push.
    pub(crate) fn git_start(&mut self, verb: &'static str) {
        let f: fn(&Path) -> Result<String, String> =
            if verb == "pull" { git::pull } else { git::push };
        if let Some(g) = self.active_git_mut() {
            g.start_job(verb, f);
        }
    }

    /// Open the discard confirmation for the cursor's file/dir/root. Discard is
    /// destructive, so it always routes through the yes/no modal.
    pub(crate) fn git_discard_prompt(&mut self) {
        if let Some((path, body)) = self.active_git().and_then(|g| g.discard_target()) {
            self.overlay = Some(Overlay::confirm("Discard", body, Confirmed::Discard { path }));
        }
    }

    /// Stash everything (recoverable with `git stash pop`), flashing the outcome.
    pub(crate) fn git_stash(&mut self) {
        match self.active_git_mut().map(|g| g.stash()) {
            Some(Ok(s)) => self.flash_git(first_line(&s)),
            Some(Err(e)) => self.flash_git(first_line(&e)),
            None => {}
        }
    }

    pub(crate) fn git_commit_prompt(&mut self) {
        self.overlay = Some(Overlay::commit());
    }

    pub(crate) fn git_newbranch_prompt(&mut self) {
        self.overlay = Some(Overlay::new_branch(String::new()));
    }

    /// Apply a submitted text prompt: commit the index, or create+switch a branch.
    pub(crate) fn overlay_submit(&mut self, kind: PromptKind, buf: String) {
        let buf = buf.trim().to_string();
        if buf.is_empty() {
            return;
        }
        match kind {
            PromptKind::Commit => match self.active_git_mut().map(|g| g.commit(&buf)) {
                Some(Ok(s)) => self.flash_git(first_line(&s)),
                Some(Err(e)) => self.flash_git(first_line(&e)),
                _ => {}
            },
            PromptKind::NewBranch => match self.active_git_mut().map(|g| g.create_branch(&buf)) {
                Some(Ok(())) => self.flash_git(format!("switched to {buf}")),
                Some(Err(e)) => self.flash_git(first_line(&e)),
                _ => {}
            },
        }
    }

    /// Run an accepted [`Overlay::Confirm`] action (called after the modal closes).
    pub(crate) fn overlay_confirm(&mut self, action: Confirmed) {
        match action {
            Confirmed::Discard { path } => match self.active_git_mut().map(|g| g.discard(&path)) {
                Some(Ok(())) => self.flash_git(format!("discarded {path}")),
                Some(Err(e)) => self.flash_git(first_line(&e)),
                None => {}
            },
        }
    }

    fn flash_git(&mut self, msg: String) {
        self.flash = Some((msg, Instant::now()));
    }
}
