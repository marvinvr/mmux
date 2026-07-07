//! The "Link another project" directory browser.
//!
//! A modal raised from the button at the bottom of the sidebar (or `L`). It walks the
//! filesystem one directory at a time, shows a quick preview of each candidate (is it
//! a git repo, does it have its own `mmux.yaml`), and links one into the *live*
//! workspace. The chosen directory is appended to the root project's `linked-projects:`
//! and loaded in place by [`App::link_project`](super::App) — no reopen.
//!
//! Listing and preview are deliberately fork-free: repo-ness is a `.git` existence
//! check and the branch is read straight from `.git/HEAD`, so browsing never shells out
//! to `git`. State lives in [`Overlay::LinkProject`](super::overlay::Overlay); keys in
//! [`App::linkbrowse_key`](super::overlay); drawn by
//! [`view::overlay::render_linkbrowse`](super::view).

use crate::config;
use std::path::{Path, PathBuf};

/// Directories never worth listing as link targets (VCS/build/dependency noise).
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "coverage", ".next", ".nuxt", "vendor",
    ".venv", "__pycache__",
];
/// Cap on directories listed for one level, so browsing a huge folder stays snappy.
const LIST_CAP: usize = 1000;

/// One candidate directory in the current listing, with cheap preview flags.
pub(crate) struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    /// Has a `.git` (so linking it brings up its own git panel).
    pub is_repo: bool,
    /// Has an `mmux.yaml`/`.yml` of its own.
    pub has_config: bool,
    /// Already in the workspace (the root or an existing link) — not linkable again.
    pub already: bool,
}

/// Richer preview for the highlighted entry, recomputed on every selection change.
pub(crate) struct Preview {
    /// The path that would be written to `linked-projects` (e.g. `../proj2`).
    pub rel: String,
    /// Current branch, when `.git/HEAD` named one (None for non-repos/detached HEAD).
    pub branch: Option<String>,
    /// The configured workspace name from its `mmux.yaml`, if it has one.
    pub name: Option<String>,
    pub has_config: bool,
    pub already: bool,
}

pub(crate) struct LinkBrowser {
    /// Root project dir (canonical) — link paths are written relative to this.
    root: PathBuf,
    /// Canonical dirs already in the workspace, used to flag/block re-linking.
    loaded: Vec<PathBuf>,
    /// The directory currently being browsed.
    cwd: PathBuf,
    /// All of `cwd`'s subdirectories (noise filtered).
    entries: Vec<DirEntry>,
    /// Indices into `entries` matching `query`, in display order.
    shown: Vec<usize>,
    pub query: String,
    sel: usize,
    /// Preview of the highlighted entry (None when the listing is empty).
    pub preview: Option<Preview>,
    /// A transient message under the list (e.g. "already in this workspace").
    pub error: Option<String>,
}

impl LinkBrowser {
    /// Open a browser for a workspace rooted at `root`, with `loaded` the canonical
    /// dirs of every project already in it. Starts one level up so nearby projects (the
    /// common `../proj2`) are visible immediately.
    pub(crate) fn new(root: PathBuf, loaded: Vec<PathBuf>) -> LinkBrowser {
        let root = config::canonical(&root);
        let cwd = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
        let mut b = LinkBrowser {
            root,
            loaded,
            cwd,
            entries: Vec::new(),
            shown: Vec::new(),
            query: String::new(),
            sel: 0,
            preview: None,
            error: None,
        };
        b.relist();
        b
    }

    /// Re-read `cwd`'s subdirectories into `entries`, dropping the filter.
    fn relist(&mut self) {
        self.entries = list_dirs(&self.cwd, &self.loaded);
        self.query.clear();
        self.refilter();
    }

    /// Recompute `shown` from `query` (case-insensitive substring), snap the cursor to
    /// the top, and refresh the preview.
    pub(crate) fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.shown = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.sel = 0;
        self.update_preview();
    }

    /// Move the cursor, wrapping at the ends (fzf-style).
    pub(crate) fn move_sel(&mut self, delta: i32) {
        if self.shown.is_empty() {
            return;
        }
        let len = self.shown.len() as i32;
        self.sel = (self.sel as i32 + delta).rem_euclid(len) as usize;
        self.update_preview();
    }

    /// Descend into the highlighted directory (if any).
    pub(crate) fn descend(&mut self) {
        if let Some(e) = self.selected() {
            self.cwd = e.path.clone();
            self.relist();
        }
    }

    /// Go up to the parent directory (if any).
    pub(crate) fn ascend(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.relist();
        }
    }

    fn selected(&self) -> Option<&DirEntry> {
        self.shown.get(self.sel).map(|&i| &self.entries[i])
    }

    /// The directory to link on ⏎: the highlighted one, unless it's already loaded.
    /// Sets `error` and returns `None` when there's nothing linkable under the cursor.
    pub(crate) fn pick(&mut self) -> Option<PathBuf> {
        match self.shown.get(self.sel).map(|&i| &self.entries[i]) {
            None => {
                self.error = Some("nothing to link here".into());
                None
            }
            Some(e) if e.already => {
                self.error = Some(format!("“{}” is already in this workspace", e.name));
                None
            }
            Some(e) => Some(e.path.clone()),
        }
    }

    fn update_preview(&mut self) {
        self.error = None;
        self.preview = self.selected().map(|e| Preview {
            rel: config::relative_path(&self.root, &config::canonical(&e.path)),
            branch: head_branch(&e.path),
            name: e.has_config.then(|| config::project_name(&e.path)),
            has_config: e.has_config,
            already: e.already,
        });
    }

    /// The display entry at ranked row `row`, for the renderer's scrolling window.
    pub(crate) fn entry_at(&self, row: usize) -> Option<&DirEntry> {
        self.shown.get(row).map(|&i| &self.entries[i])
    }

    pub(crate) fn sel(&self) -> usize {
        self.sel
    }

    pub(crate) fn count(&self) -> usize {
        self.shown.len()
    }

    /// A short, ~home-relative label for the directory being browsed (the title).
    pub(crate) fn cwd_label(&self) -> String {
        display_path(&self.cwd)
    }
}

/// List the subdirectories of `dir`, dropping VCS/build noise and hidden dirs, sorted
/// by name and capped. Each carries cheap preview flags — repo-ness is a `.git`
/// existence check, config-ness an `mmux.yaml` check — so no `git` is forked here.
fn list_dirs(dir: &Path, loaded: &[PathBuf]) -> Vec<DirEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<DirEntry> = Vec::new();
    for entry in rd.flatten() {
        if out.len() >= LIST_CAP {
            break;
        }
        let Ok(ft) = entry.file_type() else { continue };
        // Follow symlinks to directories too — a linked clone may well be a symlink.
        let is_dir = ft.is_dir() || (ft.is_symlink() && entry.path().is_dir());
        if !is_dir {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let already = loaded.contains(&config::canonical(&path));
        out.push(DirEntry {
            is_repo: path.join(".git").exists(),
            has_config: config::config_path(&path).is_some(),
            already,
            name,
            path,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// The current branch of `dir`, read straight from `.git/HEAD` (no `git` fork). `None`
/// when it isn't a normal repo checkout (detached HEAD, a `.git` file/worktree, …).
fn head_branch(dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(dir.join(".git").join("HEAD")).ok()?;
    head.trim().strip_prefix("ref: refs/heads/").map(str::to_string)
}

/// A directory path shortened for display: `~/…` for paths under `$HOME`.
fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(rest) = p.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}
