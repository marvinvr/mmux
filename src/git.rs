//! Thin, synchronous wrappers over the `git` CLI for the native git panel.
//!
//! Pure data in / data out — no `App` or UI types — so the panel layer stays
//! declarative and this file can be unit-reasoned in isolation. Errors come back
//! as plain strings (git's own stderr), which the app surfaces in the footer
//! `flash`. Everything here shells out to `git`; nothing is cached.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Unit separator — a byte that can't occur in a path or a commit subject, so we
/// use it to delimit `--format` fields instead of guessing at spaces/tabs.
const US: char = '\u{1f}';

/// A changed path in the working tree, from `git status --porcelain=v2`.
#[derive(Clone)]
pub struct FileEntry {
    pub path: String,
    /// Has changes staged in the index (porcelain v2 `X` column ≠ `.`).
    pub staged: bool,
    /// Has changes in the worktree (porcelain v2 `Y` column ≠ `.`).
    pub unstaged: bool,
    pub untracked: bool,
    /// A single status letter for display: `M`/`A`/`D`/`R`/`U`/`?`.
    pub glyph: char,
}

/// One line of recent history, from `git log`.
#[derive(Clone)]
pub struct Commit {
    /// Full 40-char object id — what the mutating ops (revert / reset) and `Y` act on,
    /// since an abbreviated hash can be ambiguous.
    pub hash: String,
    /// Abbreviated hash, shown in the Commits box and copied by `y`.
    pub short: String,
    pub summary: String,
}

/// A local branch, from `git branch`. Listed most-recently-committed first.
#[derive(Clone)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    /// Upstream tracking note, de-bracketed: e.g. `ahead 2`, `behind 1`, `gone`, ``.
    pub track: String,
}

/// A snapshot of `git status` for the panel header + file list.
pub struct Status {
    pub branch: String,
    pub files: Vec<FileEntry>,
}

/// True if `dir` is inside a git work tree (drives whether the panel exists).
pub fn is_repo(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read the working-tree status: the current branch and the changed files.
/// Failures (not a repo, git missing) collapse to an empty status rather than an
/// error — the panel just shows "clean".
pub fn status(dir: &Path) -> Status {
    // `-uall` lists untracked files individually; the default collapses a brand-new
    // folder to a single `dir/` entry. That trailing-slash path has no leaf the tree
    // can place, so it rendered as a nameless row — expanding it nests new files under
    // their folder like any other change (and matches the post-stage view).
    let raw = run(dir, &["status", "--porcelain=v2", "--branch", "-uall"]).unwrap_or_default();
    let mut st = Status { branch: String::new(), files: Vec::new() };
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            st.branch = rest.trim().to_string();
        } else if let Some(e) = parse_change(line) {
            st.files.push(e);
        }
    }
    st
}

/// Parse one porcelain-v2 entry line into a [`FileEntry`]. Returns `None` for the
/// header (`#`) and ignored (`!`) lines we don't display.
fn parse_change(line: &str) -> Option<FileEntry> {
    match line.as_bytes().first()? {
        // Ordinary change: "1 XY sub mH mI mW hH hI <path>"
        b'1' => {
            let parts: Vec<&str> = line.splitn(9, ' ').collect();
            if parts.len() < 9 {
                return None;
            }
            let (x, y) = xy(parts[1]);
            Some(FileEntry {
                path: parts[8].to_string(),
                staged: x != '.',
                unstaged: y != '.',
                untracked: false,
                glyph: glyph(x, y),
            })
        }
        // Rename/copy: "2 XY sub mH mI mW hH hI <score> <path>\t<orig>"
        b'2' => {
            let parts: Vec<&str> = line.splitn(10, ' ').collect();
            if parts.len() < 10 {
                return None;
            }
            let (x, y) = xy(parts[1]);
            let path = parts[9].split('\t').next().unwrap_or(parts[9]).to_string();
            Some(FileEntry {
                path,
                staged: x != '.',
                unstaged: y != '.',
                untracked: false,
                glyph: glyph(x, y),
            })
        }
        // Unmerged (conflict): "u XY ... <path>"
        b'u' => {
            let path = line.rsplit(' ').next().unwrap_or("").to_string();
            (!path.is_empty()).then(|| FileEntry {
                path,
                staged: false,
                unstaged: true,
                untracked: false,
                glyph: 'U',
            })
        }
        // Untracked: "? <path>"
        b'?' => line.get(2..).map(|path| FileEntry {
            path: path.to_string(),
            staged: false,
            unstaged: true,
            untracked: true,
            glyph: '?',
        }),
        _ => None,
    }
}

/// Split a porcelain `XY` field into its (index, worktree) status chars.
fn xy(field: &str) -> (char, char) {
    let b = field.as_bytes();
    let x = b.first().map(|c| *c as char).unwrap_or('.');
    let y = b.get(1).map(|c| *c as char).unwrap_or('.');
    (x, y)
}

/// The display glyph: prefer the worktree status, fall back to the index status.
fn glyph(x: char, y: char) -> char {
    match if y != '.' { y } else { x } {
        '.' => 'M',
        other => other,
    }
}

/// The last `n` commits as `(full hash, short hash, subject)` triples (newest first).
pub fn log(dir: &Path, n: usize) -> Vec<Commit> {
    let fmt = format!("--pretty=format:%H{US}%h{US}%s");
    let raw = run(dir, &["log", &format!("-{n}"), &fmt]).unwrap_or_default();
    raw.lines()
        .filter_map(|l| {
            let mut it = l.split(US);
            let hash = it.next()?.to_string();
            let short = it.next()?.to_string();
            let summary = it.next().unwrap_or("").to_string();
            (!hash.is_empty()).then(|| Commit { hash, short, summary })
        })
        .collect()
}

/// The full unified diff of a single commit (`git show`), for the diff pager. `--no-color`
/// guards against a `color.ui = always` config; the commit message + header sit before the
/// first hunk and are dropped by the pager's parser, leaving just the per-file diffs.
/// Returns the raw text (empty on a bad hash).
pub fn show(dir: &Path, hash: &str) -> String {
    run(dir, &["show", "--no-color", hash]).unwrap_or_default()
}

/// The full commit message (subject + body) of `hash`, for the `m` copy action.
pub fn commit_message(dir: &Path, hash: &str) -> Result<String, String> {
    run(dir, &["log", "-1", "--format=%B", hash]).map(|s| s.trim_end().to_string())
}

/// Revert `hash` — create a new commit that undoes it (`--no-edit` keeps the default
/// message, no editor). Non-destructive but can hit conflicts, surfaced as the error.
pub fn revert(dir: &Path, hash: &str) -> Result<String, String> {
    let out = run(dir, &["revert", "--no-edit", hash])?;
    Ok(out.lines().next().unwrap_or("reverted").trim().to_string())
}

/// Soft-reset HEAD to `hash` — move the branch tip back but leave every later change
/// staged in the index (nothing in the working tree is touched, so it's recoverable).
pub fn soft_reset(dir: &Path, hash: &str) -> Result<String, String> {
    run(dir, &["reset", "--soft", hash]).map(drop)?;
    Ok(format!("reset to {}", &hash[..hash.len().min(7)]))
}

/// Local branches, most-recently-committed first, with the current one flagged.
pub fn branches(dir: &Path) -> Vec<Branch> {
    let fmt = format!("--format=%(HEAD){US}%(refname:short){US}%(upstream:track,nobracket)");
    let raw = run(dir, &["branch", "--sort=-committerdate", &fmt]).unwrap_or_default();
    raw.lines()
        .filter_map(|l| {
            let mut it = l.split(US);
            let head = it.next()?;
            let name = it.next()?.to_string();
            let track = it.next().unwrap_or("").trim().to_string();
            (!name.is_empty()).then(|| Branch {
                name,
                current: head.trim() == "*",
                track,
            })
        })
        .collect()
}

/// Aggregate staged state of a directory subtree: are all, some, or none of its
/// changed files staged. Drives the tree checkbox.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    None,
    Partial,
    All,
}

/// One row in the flattened changed-files tree. `depth` is the indent level, starting
/// at 0 for the repo's top-level entries — there is no whole-repo root row (stage
/// everything with `a` instead). Every row is a selectable stage target: a
/// [`Dir`](TreeRow::Dir) stages its `path`, a [`File`](TreeRow::File) (whose `idx`
/// points back into the slice handed to [`tree_rows`]) stages just itself.
pub enum TreeRow {
    Dir { label: String, path: String, depth: usize, staged: Stage },
    File { idx: usize, depth: usize },
}

/// Group changed files into a directory tree and flatten it depth-first (subdirs before
/// files, both alphabetical) into render-ready rows, with the repo's top-level entries
/// at `depth` 0 (no whole-repo root row). Single-child directory chains are compressed
/// onto one row (`src/app/view`) so the tree stays shallow in a narrow column. Returns
/// empty when there are no changes.
pub fn tree_rows(files: &[FileEntry]) -> Vec<TreeRow> {
    if files.is_empty() {
        return Vec::new();
    }
    #[derive(Default)]
    struct Node {
        dirs: BTreeMap<String, Node>,
        files: Vec<(String, usize)>, // (leaf name, original index)
    }
    let mut root = Node::default();
    for (idx, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let mut node = &mut root;
        if f.path.ends_with('/') {
            // A whole directory git wouldn't descend into: an untracked dir in the
            // default mode, or an embedded repo / un-init'd submodule even under `-uall`.
            // It has no file leaf, so materialise every component as a directory node —
            // the folder renders as a named, collapsed `dir/` row instead of a nameless
            // checkbox (the empty-row bug).
            for c in &parts {
                node = node.dirs.entry(c.to_string()).or_default();
            }
        } else {
            let (leaf, dirs) = parts.split_last().unwrap(); // non-empty: checked above
            for c in dirs {
                node = node.dirs.entry(c.to_string()).or_default();
            }
            node.files.push((leaf.to_string(), idx));
        }
    }
    /// (staged, total) changed files in a subtree, for the aggregate checkbox.
    fn tally(node: &Node, files: &[FileEntry]) -> (usize, usize) {
        let (mut staged, mut total) = (0, 0);
        for (_, idx) in &node.files {
            total += 1;
            if files[*idx].staged {
                staged += 1;
            }
        }
        for sub in node.dirs.values() {
            let (s, t) = tally(sub, files);
            staged += s;
            total += t;
        }
        (staged, total)
    }
    fn stage_of(staged: usize, total: usize) -> Stage {
        if staged == 0 {
            Stage::None
        } else if staged == total {
            Stage::All
        } else {
            Stage::Partial
        }
    }
    fn walk(node: &Node, depth: usize, prefix: &str, files: &[FileEntry], out: &mut Vec<TreeRow>) {
        for (name, sub) in &node.dirs {
            let mut label = name.clone();
            let mut cur = sub;
            // Fold a single-child chain (no files, exactly one subdir) onto this row.
            while cur.files.is_empty() && cur.dirs.len() == 1 {
                let (n, s) = cur.dirs.iter().next().unwrap();
                label.push('/');
                label.push_str(n);
                cur = s;
            }
            let path = if prefix.is_empty() {
                label.clone()
            } else {
                format!("{prefix}/{label}")
            };
            let (s, t) = tally(cur, files);
            out.push(TreeRow::Dir { label, path: path.clone(), depth, staged: stage_of(s, t) });
            walk(cur, depth + 1, &path, files, out);
        }
        let mut leaves = node.files.clone();
        leaves.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, idx) in leaves {
            out.push(TreeRow::File { idx, depth });
        }
    }
    let mut out = Vec::new();
    walk(&root, 0, "", files, &mut out);
    out
}

pub fn stage(dir: &Path, path: &str) -> Result<(), String> {
    run(dir, &["add", "--", path]).map(drop)
}

pub fn unstage(dir: &Path, path: &str) -> Result<(), String> {
    run(dir, &["restore", "--staged", "--", path]).map(drop)
}

pub fn stage_all(dir: &Path) -> Result<(), String> {
    run(dir, &["add", "-A"]).map(drop)
}

/// Unstage everything (the inverse of [`stage_all`]) — empties the index back to HEAD.
pub fn unstage_all(dir: &Path) -> Result<(), String> {
    run(dir, &["restore", "--staged", "--", "."]).map(drop)
}

/// Discard **all** changes under a pathspec — a file, a directory, or `.` for the
/// whole tree (destructive). Unstage it, restore tracked content from HEAD, then
/// remove untracked leftovers. `restore` only ever touches tracked files and `clean`
/// only untracked ones, so together they fully reset the path without `clean` deleting
/// committed content. `restore` "fails" when nothing tracked matches (e.g. a path with
/// only new files) — not a real error here — so only `clean`'s result is surfaced.
/// `clean -d` (not `-x`) removes untracked dirs but keeps gitignored files.
pub fn discard(dir: &Path, path: &str) -> Result<(), String> {
    let _ = run(dir, &["restore", "--staged", "--", path]);
    let _ = run(dir, &["restore", "--worktree", "--source=HEAD", "--", path]);
    run(dir, &["clean", "-fd", "--", path]).map(drop)
}

/// Commit the staged index. Returns git's first output line for the footer flash.
pub fn commit(dir: &Path, msg: &str) -> Result<String, String> {
    let out = run(dir, &["commit", "-m", msg])?;
    Ok(out.lines().next().unwrap_or("committed").trim().to_string())
}

/// A unified diff of one path for the preview pane. For a tracked file we diff
/// `HEAD` against the working tree, so staged *and* unstaged edits show together
/// ("what changed in this file since the last commit"). A brand-new untracked file
/// has nothing in HEAD, so we diff it against `/dev/null` to render it all-added.
/// Returns the raw diff text (empty when there's nothing to show).
pub fn diff(dir: &Path, path: &str, untracked: bool) -> String {
    if untracked {
        // `--no-index` exits non-zero precisely *because* the files differ (the whole
        // point here), so read stdout regardless of the status code.
        run_lossy(dir, &["diff", "--no-index", "--", "/dev/null", path])
    } else {
        run(dir, &["diff", "HEAD", "--", path]).unwrap_or_default()
    }
}

pub fn switch(dir: &Path, name: &str) -> Result<(), String> {
    run(dir, &["switch", name]).map(drop)
}

pub fn create_branch(dir: &Path, name: &str) -> Result<(), String> {
    run(dir, &["switch", "-c", name]).map(drop)
}

/// `git pull` (blocks on the network — run off the UI thread).
pub fn pull(dir: &Path) -> Result<String, String> {
    run(dir, &["pull"]).map(|_| "pulled".into())
}

/// `git push` (blocks on the network — run off the UI thread).
pub fn push(dir: &Path) -> Result<String, String> {
    run(dir, &["push"]).map(|_| "pushed".into())
}

/// Stash all changes, including untracked files (`git stash push -u`). Recoverable
/// with `git stash pop`. Errors (e.g. "No local changes to save") surface as-is.
pub fn stash(dir: &Path) -> Result<String, String> {
    let out = run(dir, &["stash", "push", "-u"])?;
    Ok(out.lines().next().unwrap_or("stashed").trim().to_string())
}

/// Like [`run`] but hands back stdout no matter the exit status. Some porcelain
/// (notably `diff --no-index`) exits non-zero *because* there's output to show, so
/// the usual success/failure split would throw the diff away.
fn run_lossy(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Run `git <args>` in `dir`, returning stdout on success or git's stderr (falling
/// back to stdout) on failure. The single choke point every wrapper goes through.
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git not found: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe(path: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            staged: false,
            unstaged: true,
            untracked: false,
            glyph: 'M',
        }
    }

    /// The repo's top-level entries sit at depth 0 (no whole-repo root row); a
    /// single-child chain collapses onto one header; root files sort after subdirectories.
    #[test]
    fn tree_compresses_chains_without_root() {
        let files = vec![fe("src/app/view/git.rs"), fe("README.md")];
        let rows = tree_rows(&files);
        match &rows[..] {
            [TreeRow::Dir { label, depth: 0, .. }, TreeRow::File { idx: 0, depth: 1 }, TreeRow::File { idx: 1, depth: 0 }] => {
                assert_eq!(label, "src/app/view")
            }
            _ => panic!("unexpected tree shape"),
        }
    }

    /// A directory git won't descend into arrives as a trailing-slash path with no file
    /// leaf (an embedded repo, or an untracked dir when git collapses it). It must render
    /// as a named, collapsed directory row — never the nameless file row that was the bug.
    #[test]
    fn trailing_slash_dir_is_named_not_nameless() {
        let rows = tree_rows(&[fe("embedded/")]);
        match &rows[..] {
            [TreeRow::Dir { label, path, depth: 0, .. }] => {
                assert_eq!(label, "embedded");
                assert_eq!(path, "embedded");
            }
            _ => panic!("expected a single named Dir row for a trailing-slash entry"),
        }
    }

    /// A directory with two children isn't compressed; it carries its `git add` path
    /// and an aggregate staged state (one of two staged → partial) at depth 0.
    #[test]
    fn tree_dir_path_and_partial_stage() {
        let mut files = vec![fe("src/a.rs"), fe("src/b.rs")];
        files[0].staged = true;
        let rows = tree_rows(&files);
        match &rows[..] {
            [TreeRow::Dir { path, depth: 0, staged: Stage::Partial, .. }, TreeRow::File { depth: 1, .. }, TreeRow::File { depth: 1, .. }] => {
                assert_eq!(path, "src")
            }
            _ => panic!("unexpected tree shape"),
        }
    }

    // ── parse_change: one porcelain-v2 status line → FileEntry ───────────────
    #[test]
    fn parse_change_ordinary_index_and_worktree_states() {
        // "1 XY sub mH mI mW hH hI <path>" — worktree-modified, unstaged.
        let e = parse_change("1 .M N... 100644 100644 100644 aaaa bbbb src/main.rs").unwrap();
        assert_eq!(e.path, "src/main.rs");
        assert!(!e.staged && e.unstaged && !e.untracked);
        assert_eq!(e.glyph, 'M');

        // Staged-only (index column set, worktree clean): glyph falls back to the index status.
        let e = parse_change("1 A. N... 100644 100644 100644 aaaa bbbb new.rs").unwrap();
        assert!(e.staged && !e.unstaged);
        assert_eq!(e.glyph, 'A');

        // Staged AND unstaged at once.
        let e = parse_change("1 MM N... 100644 100644 100644 aaaa bbbb both.rs").unwrap();
        assert!(e.staged && e.unstaged);
        assert_eq!(e.glyph, 'M');
    }

    #[test]
    fn parse_change_keeps_paths_with_spaces() {
        // v2 ordinary entries leave the path as the unquoted rest of the line.
        let e = parse_change("1 .M N... 100644 100644 100644 aaaa bbbb my file.txt").unwrap();
        assert_eq!(e.path, "my file.txt");
    }

    #[test]
    fn parse_change_rename_uses_new_path_before_the_tab() {
        // "2 XY sub mH mI mW hH hI <score> <new>\t<orig>" — keep the new name.
        let e = parse_change("2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs\told.rs").unwrap();
        assert_eq!(e.path, "new.rs");
        assert!(e.staged);
        assert_eq!(e.glyph, 'R');
    }

    #[test]
    fn parse_change_unmerged_and_untracked() {
        let u = parse_change("u UU N... 100644 100644 100644 100644 a b c conflict.rs").unwrap();
        assert_eq!(u.path, "conflict.rs");
        assert_eq!(u.glyph, 'U');
        assert!(u.unstaged && !u.staged);

        let q = parse_change("? whatever.log").unwrap();
        assert_eq!(q.path, "whatever.log");
        assert!(q.untracked && q.unstaged && !q.staged);
        assert_eq!(q.glyph, '?');
    }

    #[test]
    fn parse_change_skips_headers_ignored_and_malformed() {
        assert!(parse_change("# branch.head main").is_none());
        assert!(parse_change("! ignored.txt").is_none());
        assert!(parse_change("").is_none());
        // An ordinary line with too few fields is rejected, not panicked on.
        assert!(parse_change("1 .M too short").is_none());
    }
}
