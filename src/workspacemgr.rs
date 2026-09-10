//! Shared state for creating and editing a directory-level workspace manifest.
//!
//! Like [`crate::agentmgr::AgentManager`], this is deliberately front-end-neutral:
//! [`crate::wizard`] drives it as the inline `mmux init workspace` checkbox picker, while
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
    /// Live fuzzy filter over the row paths. Both frontends type straight into it —
    /// a parent directory with dozens of children is unusable without one — and it
    /// only ever hides rows: selection, order, and the saved manifest are untouched.
    pub filter: String,
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
            filter: String::new(),
            editing_name: false,
            error: None,
        })
    }

    /// Row indices the filter lets through, in manifest order. Matching is the file
    /// picker's fuzzy scorer, but the score only decides *whether* a row shows: manifest
    /// order is the meaningful order here, so matches are never re-ranked.
    pub(crate) fn visible(&self) -> Vec<usize> {
        let q = self.filter.trim();
        if q.is_empty() {
            return (0..self.rows.len()).collect();
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| crate::app::picker::score(q, &r.path).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.settle_cursor();
    }

    pub(crate) fn pop_filter(&mut self) {
        self.filter.pop();
        self.settle_cursor();
    }

    /// Drop the filter, reporting whether there was one. Frontends use the answer to
    /// give `Esc` the search-bar behavior: clear first, cancel on a second press.
    pub(crate) fn clear_filter(&mut self) -> bool {
        if self.filter.is_empty() {
            return false;
        }
        self.filter.clear();
        self.settle_cursor();
        true
    }

    /// Keep the cursor on a row the filter still shows.
    fn settle_cursor(&mut self) {
        let visible = self.visible();
        if !visible.contains(&self.cursor) {
            self.cursor = visible.first().copied().unwrap_or(0);
        }
        self.error = None;
    }

    pub(crate) fn move_cursor(&mut self, delta: i32) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let pos = visible.iter().position(|&i| i == self.cursor).unwrap_or(0) as i32;
        let to = (pos + delta).clamp(0, visible.len() as i32 - 1) as usize;
        self.cursor = visible[to];
        self.error = None;
    }

    pub(crate) fn toggle_enabled(&mut self) {
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.enabled = !r.enabled;
        }
        self.error = None;
    }

    /// Select every candidate the filter shows, or clear them when they all already are.
    pub(crate) fn toggle_all(&mut self) {
        let visible = self.visible();
        let all_on = !visible.is_empty() && visible.iter().all(|&i| self.rows[i].enabled);
        for i in visible {
            self.rows[i].enabled = !all_on;
        }
        self.error = None;
    }

    /// Move the highlighted row, which also defines the persisted manifest order.
    pub(crate) fn reorder(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        // Moving a row past hidden neighbours would rewrite manifest order in ways the
        // filtered view can't show, so ordering waits until the search is cleared.
        if !self.filter.trim().is_empty() {
            self.error = Some("clear the search to reorder".into());
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

#[cfg(test)]
mod tests {
    use super::WorkspaceManager;

    /// A manager over a throwaway directory holding `names` as child folders.
    fn manager(tag: &str, names: &[&str]) -> WorkspaceManager {
        let root = std::env::temp_dir().join(format!("mmux-wsmgr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in names {
            std::fs::create_dir_all(root.join(name)).expect("creating a child directory");
        }
        WorkspaceManager::new(&root).expect("discovering the directory")
    }

    #[test]
    fn filtering_hides_rows_without_touching_selection_or_order() {
        let mut m = manager("filter", &["api", "web", "worker"]);
        let order: Vec<String> = m.rows.iter().map(|r| r.path.clone()).collect();
        m.toggle_all();
        for c in "wor".chars() {
            m.push_filter(c);
        }
        let shown: Vec<&str> = m
            .visible()
            .iter()
            .map(|&i| m.rows[i].path.as_str())
            .collect();
        assert_eq!(shown, vec!["worker"], "fuzzy match on the folder path");
        assert!(
            m.rows.iter().map(|r| r.path.clone()).eq(order),
            "a search must not reorder or drop rows"
        );
        assert_eq!(m.selected_count(), 4, "a search must not change selection");

        // The cursor follows the filter, and returns to a full list on Esc.
        assert_eq!(m.rows[m.cursor].path, "worker");
        assert!(m.clear_filter(), "the first Esc reports a cleared search");
        assert!(!m.clear_filter(), "a second Esc has nothing left to clear");
        assert_eq!(m.visible().len(), m.rows.len());
    }

    #[test]
    fn all_and_reorder_respect_the_active_search() {
        let mut m = manager("scoped", &["api", "web"]);
        for c in "web".chars() {
            m.push_filter(c);
        }
        m.toggle_all();
        assert_eq!(m.selected_count(), 1, "`all` covers only the shown rows");
        m.reorder(-1);
        assert!(
            m.error.is_some(),
            "ordering is refused while rows are hidden"
        );
    }
}
