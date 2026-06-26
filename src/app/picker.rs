//! The Ctrl+P fuzzy file picker.
//!
//! A modal that lists the active project's files (via `rg --files`, falling back
//! to `git ls-files` and then a shallow manual walk) and fuzzy-filters them as you
//! type. Enter opens the highlighted file in an editor pane — mirroring the user's
//! shell `fe` widget (`rg | fzf -> micro`). The picker is held in
//! [`Overlay::Picker`](super::git::Overlay) and rendered by `view::git::render_picker`.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// List the files under `dir`, mirroring the user's `fe` widget: ripgrep including
/// hidden files with the usual noise directories excluded. Degrades to tracked
/// files (`git ls-files`) and finally a manual walk if neither tool is present.
fn list_files(dir: &Path) -> Vec<String> {
    let globs = [
        "!.git/*",
        "!node_modules/**",
        "!dist/**",
        "!build/**",
        "!coverage/**",
        "!.next/**",
        "!.nuxt/**",
        "!vendor/**",
    ];
    let mut cmd = Command::new("rg");
    cmd.arg("--files").arg("--hidden");
    for g in globs {
        cmd.arg("--glob").arg(g);
    }
    cmd.current_dir(dir);
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let mut files = lines(&out.stdout);
            // `--hidden` surfaces dotfiles, but ripgrep still honours .gitignore, so
            // commonly-edited yet typically-ignored config (.env, .env.local, .envrc, …)
            // never shows up. A positive `--glob` overrides ignore logic, so a second
            // whitelist pass adds those back; the dir excludes keep nested
            // node_modules/.env and the like out.
            merge_unique(&mut files, list_env_files(dir, &globs));
            return files;
        }
    }
    // ripgrep missing/failed → fall back to tracked files.
    if let Ok(out) = Command::new("git").args(["ls-files"]).current_dir(dir).output() {
        if out.status.success() {
            let v = lines(&out.stdout);
            if !v.is_empty() {
                return v;
            }
        }
    }
    // Last resort: a shallow manual walk, skipping the same noise dirs.
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_string)
        .collect()
}

/// A whitelist pass for env-style config files (`.env`, `.env.local`, `.envrc`, …)
/// the main listing skips because they're gitignored. The positive `.env*` glob
/// overrides ripgrep's ignore logic; `excludes` are the same dir-noise globs the
/// main pass uses, so nested `node_modules/.env` and friends stay out.
fn list_env_files(dir: &Path, excludes: &[&str]) -> Vec<String> {
    let mut cmd = Command::new("rg");
    cmd.arg("--files").arg("--hidden").arg("--glob").arg(".env*");
    for g in excludes {
        cmd.arg("--glob").arg(g);
    }
    cmd.current_dir(dir);
    match cmd.output() {
        Ok(out) if out.status.success() => lines(&out.stdout),
        _ => Vec::new(),
    }
}

/// Append items from `extra` not already present in `into`, preserving order. The
/// env pass returns a handful of paths, so the linear membership check is cheap.
fn merge_unique(into: &mut Vec<String>, extra: Vec<String>) {
    for item in extra {
        if !into.contains(&item) {
            into.push(item);
        }
    }
}

const EXCLUDED_DIRS: &[&str] = &[
    ".git", "node_modules", "dist", "build", "coverage", ".next", ".nuxt", "vendor",
];
/// Cap the manual-walk fallback so a giant tree can't stall the picker.
const WALK_CAP: usize = 5000;

fn walk(root: &Path, cur: &Path, out: &mut Vec<String>) {
    if out.len() >= WALK_CAP {
        return;
    }
    let Ok(rd) = std::fs::read_dir(cur) else {
        return;
    };
    for entry in rd.flatten() {
        if out.len() >= WALK_CAP {
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if !EXCLUDED_DIRS.contains(&name.as_ref()) {
                walk(root, &entry.path(), out);
            }
        } else if ft.is_file() {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
}

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
