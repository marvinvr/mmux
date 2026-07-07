//! The Ctrl+P fuzzy file picker.
//!
//! A modal that lists the active project's files (via the `ignore` crate — ripgrep's
//! own file-walking engine, run in-process, so nothing external is required) and
//! fuzzy-filters them as you type. Enter opens the highlighted file in an editor pane —
//! mirroring the user's shell `fe` widget (`rg | fzf -> micro`), except both the listing
//! and the fuzzy ranking are built in. The picker is held in
//! [`Overlay::Picker`](super::overlay::Overlay) and rendered by `view::overlay::render_picker`.

use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

pub(crate) struct Picker {
    /// Project index the picker searches and opens files in (the active project).
    /// The listed paths are relative to this project's dir.
    pub(crate) project: usize,
    /// The full candidate set, listed once when the picker opens.
    candidates: Vec<String>,
    /// The live query string.
    pub(crate) query: String,
    /// Indices into `candidates`, filtered + ranked by the current query.
    matches: Vec<usize>,
    /// Cursor into `matches`.
    sel: usize,
}

impl Picker {
    /// Open a picker over `dir`, listing its files up front.
    pub(crate) fn new(project: usize, dir: PathBuf) -> Picker {
        let candidates = list_files(&dir);
        let matches = (0..candidates.len()).collect();
        Picker { project, candidates, query: String::new(), matches, sel: 0 }
    }

    /// Re-filter and re-rank against the current query; the selection snaps to the
    /// best (top) match so it tracks what you're typing.
    pub(crate) fn recompute(&mut self) {
        if self.query.is_empty() {
            self.matches = (0..self.candidates.len()).collect();
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .candidates
                .iter()
                .enumerate()
                .filter_map(|(i, c)| score(&self.query, c).map(|s| (s, i)))
                .collect();
            // Best score first; tie-break on the shorter path (more specific).
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| self.candidates[a.1].len().cmp(&self.candidates[b.1].len()))
            });
            self.matches = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.sel = 0;
    }

    /// Move the cursor, wrapping around the ends (fzf-style).
    pub(crate) fn move_sel(&mut self, delta: i32) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as i32;
        self.sel = (self.sel as i32 + delta).rem_euclid(len) as usize;
    }

    /// The highlighted path (relative to the project dir), if any.
    pub(crate) fn selected(&self) -> Option<&str> {
        self.matches.get(self.sel).map(|&i| self.candidates[i].as_str())
    }

    /// The path at ranked row `row`, for the renderer's scrolling window.
    pub(crate) fn path_at(&self, row: usize) -> Option<&str> {
        self.matches.get(row).map(|&i| self.candidates[i].as_str())
    }

    pub(crate) fn sel(&self) -> usize {
        self.sel
    }

    pub(crate) fn match_count(&self) -> usize {
        self.matches.len()
    }
}

/// List the files under `dir` with the `ignore` crate (ripgrep's walking engine, run
/// in-process): include hidden files and **don't** honour `.gitignore`, so
/// commonly-edited yet typically-ignored files (`.env`, local notes, generated config)
/// show up too. The heavy build/artifact directories in [`EXCLUDED_DIRS`] are pruned so
/// they can't flood the list now that gitignore isn't filtering them. `.ignore` /
/// `.rgignore` files are still honoured, so a project can tune the picker with one.
fn list_files(dir: &Path) -> Vec<String> {
    WalkBuilder::new(dir)
        .hidden(false) // include dotfiles
        .git_ignore(false) // don't honour .gitignore …
        .git_global(false) // … or the global gitignore …
        .git_exclude(false) // … or .git/info/exclude
        // Prune the noise dirs by name (depth 0 is the root itself — never prune that).
        .filter_entry(|e| {
            e.depth() == 0 || e.file_name().to_str().is_none_or(|n| !EXCLUDED_DIRS.contains(&n))
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(dir).ok().map(|p| p.to_string_lossy().into_owned()))
        .collect()
}

/// Directories pruned from the picker listing. Since we no longer honour `.gitignore`,
/// this names the heavy build/artifact trees we don't want flooding the list.
const EXCLUDED_DIRS: &[&str] = &[
    ".git", "node_modules", "dist", "build", "coverage", ".next", ".nuxt", "vendor",
    "target", ".venv", "venv", "__pycache__",
];

/// Fuzzy subsequence score, case-insensitive. `None` if `needle` is not a
/// subsequence of `hay`. Higher is better: consecutive matches and matches right
/// after a path/word boundary score more, so `apge` favours `app/page.tsx`.
///
/// Shared with the `mmux attach` session picker (`tmux::rank`), which scores project
/// names + directories with the same boundary-aware heuristic.
pub(crate) fn score(needle: &str, hay: &str) -> Option<i32> {
    let n: Vec<char> = needle
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if n.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = hay.chars().collect();
    let mut hi = 0usize;
    let mut total = 0i32;
    let mut last: Option<usize> = None;
    for &nc in &n {
        loop {
            if hi >= h.len() {
                return None;
            }
            if h[hi].to_ascii_lowercase() == nc {
                let mut s = 1;
                if hi > 0 && last == Some(hi - 1) {
                    s += 5; // consecutive run
                }
                let boundary = hi == 0 || matches!(h[hi - 1], '/' | '_' | '-' | '.' | ' ');
                if boundary {
                    s += 10; // start of a path/word segment
                }
                total += s;
                last = Some(hi);
                hi += 1;
                break;
            }
            hi += 1;
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn subsequence_required() {
        assert!(score("xyz", "src/app.rs").is_none());
        assert!(score("app", "src/app.rs").is_some());
    }

    #[test]
    fn boundary_start_beats_midword() {
        // A match anchored at a segment boundary (a prefix) should outrank the same
        // query starting mid-token.
        let prefix = score("app", "app.rs").unwrap();
        let midword = score("app", "zapp.rs").unwrap();
        assert!(prefix > midword, "prefix={prefix} midword={midword}");
    }

    #[test]
    fn empty_query_matches_anything() {
        assert_eq!(score("", "whatever"), Some(0));
    }
}
