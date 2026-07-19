//! Shared state for creating and editing a directory-level workspace manifest.
//!
//! Like [`crate::agentmgr::AgentManager`], this is deliberately front-end-neutral:
//! [`crate::wizard`] drives it as the inline `mmux workspace` checkbox picker, while
//! the TUI wraps the same rows in an overlay. Discovery is shallow on purpose — a
//! workspace manifest names its immediate project folders, not an arbitrarily deep
//! filesystem tree. Already-configured paths outside the directory are retained as
//! rows so opening the manager can never silently discard them.

use crate::config;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One selectable manifest member.
pub(crate) struct Row {
    /// The exact path written under `workspace.folders`, relative where possible.
    pub path: String,
    /// Whether the path is currently part of the manifest.
    pub enabled: bool,
    /// Whether it currently resolves to a directory.
    pub exists: bool,
    /// Whether the directory has its own mmux project config.
    pub configured: bool,
    /// Whether the directory is a git repository.
    pub git: bool,
}

pub(crate) struct WorkspaceManager {
    pub root: PathBuf,
    pub name: String,
    pub rows: Vec<Row>,
    /// Loaded member directories before editing. Used to bind legacy positional
    /// restore snapshots to stable identities before a reorder is written.
    pub original_projects: Vec<PathBuf>,
    pub cursor: usize,
    /// TUI-only name-edit mode. The terminal frontend asks for the name before it
    /// enters raw mode, but keeping the edit buffer here lets both frontends still
    /// share one model and validation path.
    pub editing_name: bool,
    pub error: Option<String>,
}

impl WorkspaceManager {
    /// Discover `root` itself plus its immediate child directories, seeded from an
    /// existing manifest when present. Existing members keep manifest order; newly
    /// discovered candidates follow alphabetically.
    pub(crate) fn new(root: &Path) -> Result<WorkspaceManager> {
        let root = config::canonical(root);
        let existing =
            if config::config_path(&root).is_some() || config::local_config_path(&root).is_some() {
                Some(config::Config::load(&root)?)
            } else {
                None
            };
        let folders = existing
            .as_ref()
            .and_then(|c| c.workspace.as_ref())
            .map(|w| w.folders.clone())
            .unwrap_or_default();
        let name = existing
            .as_ref()
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| dir_name(&root));
        let original_projects = if existing.as_ref().is_some_and(|c| c.workspace.is_some()) {
            config::Config::load_workspace(&root)?
                .projects
                .into_iter()
                .map(|c| c.dir)
                .collect()
        } else {
            Vec::new()
        };

        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        for path in &folders {
            if seen.insert(path.clone()) {
                rows.push(row(&root, path, true));
            }
        }

        // `.` is useful when a directory is both manifest and project. Keep it near
        // the top without disturbing an existing manifest's explicit order.
        if seen.insert(".".to_string()) {
            rows.push(row(&root, ".", false));
        }

        let mut discovered: Vec<String> = std::fs::read_dir(&root)
            .with_context(|| format!("reading {}", root.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if ignored_child(&name) || !entry.path().is_dir() {
                    return None;
                }
                Some(name)
            })
            .collect();
        discovered.sort_by_key(|s| s.to_lowercase());
        for path in discovered {
            if seen.insert(path.clone()) {
                rows.push(row(&root, &path, false));
            }
        }

        Ok(WorkspaceManager {
            root,
            name,
            rows,
            original_projects,
            cursor: 0,
            editing_name: false,
            error: None,
        })
    }

    pub(crate) fn move_cursor(&mut self, delta: i32) {
        let len = self.rows.len() as i32;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
        self.error = None;
    }

    pub(crate) fn toggle_enabled(&mut self) {
        let Some(current) = self.rows.get(self.cursor) else {
            return;
        };
        if !current.enabled && self.selected_count() >= config::MAX_PROJECTS {
            self.error = Some(format!(
                "a workspace can contain at most {} projects",
                config::MAX_PROJECTS
            ));
            return;
        }
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.enabled = !r.enabled;
        }
        self.error = None;
    }

    /// Select every candidate up to the workspace cap, or clear the selection when
    /// every row that can fit is already selected.
    pub(crate) fn toggle_all(&mut self) {
        let all_on = !self.rows.is_empty()
            && self
                .rows
                .iter()
                .take(config::MAX_PROJECTS)
                .all(|r| r.enabled)
            && self
                .rows
                .iter()
                .skip(config::MAX_PROJECTS)
                .all(|r| !r.enabled);
        for (i, r) in self.rows.iter_mut().enumerate() {
            r.enabled = !all_on && i < config::MAX_PROJECTS;
        }
        self.error = None;
    }

    /// Move the highlighted row, which also defines the persisted sidebar order.
    pub(crate) fn reorder(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let to = (self.cursor as i32 + delta).clamp(0, self.rows.len() as i32 - 1) as usize;
        if to != self.cursor {
            self.rows.swap(self.cursor, to);
            self.cursor = to;
        }
        self.error = None;
    }

    pub(crate) fn selected_count(&self) -> usize {
        self.rows.iter().filter(|r| r.enabled).count()
    }

    pub(crate) fn folders(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.path.clone())
            .collect()
    }

    pub(crate) fn validate(&mut self) -> bool {
        if self.name.trim().is_empty() {
            self.error = Some("give the workspace a name".into());
            return false;
        }
        if self.selected_count() == 0 {
            self.error = Some("select at least one project folder".into());
            return false;
        }
        if self.selected_count() > config::MAX_PROJECTS {
            self.error = Some(format!(
                "reduce the selection to {} projects",
                config::MAX_PROJECTS
            ));
            return false;
        }
        self.error = None;
        true
    }
}

fn row(root: &Path, path: &str, enabled: bool) -> Row {
    let full = root.join(path);
    Row {
        path: path.to_string(),
        enabled,
        exists: full.is_dir(),
        configured: config::config_path(&full).is_some(),
        git: full.join(".git").exists(),
    }
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "workspace".into())
}

fn ignored_child(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "dist" | "build" | "vendor" | "coverage"
        )
}
