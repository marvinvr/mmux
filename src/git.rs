//! Thin, synchronous wrappers over the `git` CLI for the native git panel.
//!
//! Pure data in / data out — no `App` or UI types — so the panel layer stays
//! declarative and this file can be unit-reasoned in isolation. Errors come back
//! as plain strings (git's own stderr), which the app surfaces in the footer
//! `flash`. Everything here shells out to `git`; nothing is cached.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Whether this repository has somewhere to fetch from. Kept separate from
/// [`fetch`] so the periodic worker does not repeatedly launch doomed network jobs
/// for local-only repositories.
pub fn has_remote(dir: &Path) -> bool {
    run(dir, &["remote"])
        .map(|s| s.lines().any(|line| !line.trim().is_empty()))
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
    let mut st = Status {
        branch: String::new(),
        files: Vec::new(),
    };
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
            (!hash.is_empty()).then(|| Commit {
                hash,
                short,
                summary,
            })
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
    Dir {
        label: String,
        path: String,
        depth: usize,
        staged: Stage,
    },
    File {
        idx: usize,
        depth: usize,
    },
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
            out.push(TreeRow::Dir {
                label,
                path: path.clone(),
                depth,
                staged: stage_of(s, t),
            });
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

/// Bounded context for a commit-message generator. A non-empty index wins,
/// matching `git commit`; otherwise this describes what mmux will stage-all.
pub fn commit_context(dir: &Path) -> Result<String, String> {
    const MAX_FILES: usize = 40;
    const PER_FILE: usize = 12 * 1024;
    const TOTAL: usize = 64 * 1024;
    let st = status(dir);
    let staged = st.files.iter().any(|f| f.staged);
    let files: Vec<&FileEntry> = st
        .files
        .iter()
        .filter(|f| !staged || f.staged)
        .take(MAX_FILES)
        .collect();
    if files.is_empty() {
        return Err("nothing to commit".to_string());
    }
    let mut out = String::from("Recent commit subjects (match this repository's style):\n");
    for commit in log(dir, 8) {
        push_capped(&mut out, &format!("- {}\n", commit.summary), TOTAL);
    }
    push_capped(
        &mut out,
        if staged {
            "\nChanges staged for this commit:\n"
        } else {
            "\nNo changes are staged; mmux will stage and commit all of these changes:\n"
        },
        TOTAL,
    );
    for file in files {
        if out.len() >= TOTAL {
            break;
        }
        let state = if file.untracked {
            "untracked"
        } else {
            "changed"
        };
        push_capped(
            &mut out,
            &format!("\n--- {} ({state}) ---\n", file.path),
            TOTAL,
        );
        let detail = if file.untracked && !staged {
            read_text_prefix(&dir.join(&file.path), PER_FILE)
        } else if staged {
            capped(
                &run_lossy(
                    dir,
                    &[
                        "diff",
                        "--cached",
                        "--no-color",
                        "--no-ext-diff",
                        "--unified=3",
                        "--",
                        file.path.as_str(),
                    ],
                ),
                PER_FILE,
            )
        } else {
            capped(
                &run_lossy(
                    dir,
                    &[
                        "diff",
                        "HEAD",
                        "--no-color",
                        "--no-ext-diff",
                        "--unified=3",
                        "--",
                        file.path.as_str(),
                    ],
                ),
                PER_FILE,
            )
        };
        push_capped(
            &mut out,
            if detail.is_empty() {
                "[binary or unavailable]\n"
            } else {
                &detail
            },
            TOTAL,
        );
    }
    if st.files.iter().filter(|f| !staged || f.staged).count() > MAX_FILES {
        push_capped(&mut out, "\n[additional changed files omitted]\n", TOTAL);
    }
    Ok(out)
}

fn read_text_prefix(path: &Path, limit: usize) -> String {
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = file.take(limit as u64 + 1).read_to_end(&mut bytes);
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return String::new();
    }
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[truncated]\n");
    }
    text
}

fn capped(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]\n", &text[..end])
}

fn push_capped(out: &mut String, text: &str, limit: usize) {
    let room = limit.saturating_sub(out.len());
    if room == 0 {
        return;
    }
    out.push_str(&capped(text, room));
    if out.len() > limit {
        out.truncate(limit);
    }
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

/// Quietly refresh remote-tracking refs without touching the worktree or current
/// branch. This is only called from the git panel's periodic background worker.
pub fn fetch(dir: &Path) -> Result<String, String> {
    run(dir, &["fetch", "--quiet"]).map(|_| "fetched".into())
}

/// `git push` (blocks on the network — run off the UI thread). A branch with no
/// upstream is *published* rather than refused: we set the upstream to the default
/// remote on the way out, so the first push of a new branch just works instead of
/// making the user drop to a shell for `-u`.
pub fn push(dir: &Path) -> Result<String, String> {
    if upstream(dir).is_some() {
        return run(dir, &["push"]).map(|_| "pushed".into());
    }
    let remote = default_remote(dir)?;
    // `HEAD`, not the branch name, so a detached HEAD fails as git's own error
    // rather than publishing to a same-named branch by accident.
    run(dir, &["push", "--set-upstream", &remote, "HEAD"])?;
    Ok(format!("pushed → {remote} (upstream set)"))
}

/// The current branch's upstream (`origin/main`), or `None` when it has none — the
/// case [`push`] publishes through.
fn upstream(dir: &Path) -> Option<String> {
    let out = run(
        dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()?;
    let name = out.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Which remote to publish a new branch to: `origin` when it exists (the overwhelming
/// case), else the only remote, else the first listed. With several non-`origin` remotes
/// there's nothing better to guess at, so we take the first and let the flash name the
/// upstream it set.
fn default_remote(dir: &Path) -> Result<String, String> {
    let out = run(dir, &["remote"])?;
    let names: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.contains(&"origin") {
        return Ok("origin".into());
    }
    names
        .first()
        .map(|s| (*s).to_string())
        .ok_or_else(|| "no remote configured".to_string())
}

/// Stash all changes, including untracked files (`git stash push -u`). Recoverable
/// with `git stash pop`. Errors (e.g. "No local changes to save") surface as-is.
pub fn stash(dir: &Path) -> Result<String, String> {
    let out = run(dir, &["stash", "push", "-u"])?;
    Ok(out.lines().next().unwrap_or("stashed").trim().to_string())
}

// ── Worktrees ────────────────────────────────────────────────────────────────
//
// A linked worktree is a second checkout of the same repository on its own branch.
// mmux loads one as an ordinary project (see [`crate::worktree`]), so everything
// here is deliberately plain: list them, add one, remove one, merge one back. The
// repository config is shared by every checkout, which is what makes
// [`config_get`]/[`config_set`] a durable place to remember a branch's base.

/// One entry from `git worktree list`: a checkout of the repository and the branch
/// it has out. The first entry is always the repository's **main** worktree.
#[derive(Clone)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// The checked-out branch, empty for a detached HEAD.
    pub branch: String,
}

/// Every checkout of the repository containing `dir`, main worktree first. This is
/// the only place that knows `--porcelain`'s layout: blank-line separated records,
/// each opened by `worktree <path>`, with `branch refs/heads/<name>` naming the
/// branch (absent when detached). A non-repo yields an empty list rather than an
/// error — callers treat "has no worktrees" and "isn't a repo" the same way.
pub fn worktrees(dir: &Path) -> Vec<WorktreeEntry> {
    let raw = run(dir, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    let mut out: Vec<WorktreeEntry> = Vec::new();
    for line in raw.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            out.push(WorktreeEntry {
                path: PathBuf::from(path.trim()),
                branch: String::new(),
            });
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            if let Some(last) = out.last_mut() {
                last.branch = branch.trim().to_string();
            }
        }
    }
    out
}

/// The repository's main worktree — the checkout that owns `.git`. Adding, removing
/// and merging all run there: a linked worktree cannot remove itself, and the branch
/// a worktree merges back into lives in the main checkout.
pub fn main_worktree(dir: &Path) -> Option<PathBuf> {
    worktrees(dir).into_iter().next().map(|w| w.path)
}

/// Whether `name` is an existing local branch — decides whether creating a worktree
/// checks a branch out or cuts a new one.
pub fn branch_exists(dir: &Path, name: &str) -> bool {
    run(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
    .is_ok()
}

/// Whether the working tree at `dir` has nothing changed at all (tracked *or*
/// untracked). The gate on merging a worktree back and on removing one without
/// discarding work.
pub fn is_clean(dir: &Path) -> bool {
    status(dir).files.is_empty()
}

/// Create a linked worktree at `path`. `base` picks the two cases apart: `Some(b)`
/// cuts `branch` fresh off `b`, `None` checks out a branch that already exists. Git
/// refuses to have one branch checked out in two worktrees, and that refusal comes
/// back verbatim as the error.
pub fn worktree_add(
    dir: &Path,
    path: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<(), String> {
    let path = path.to_string_lossy().into_owned();
    match base {
        Some(base) => run(dir, &["worktree", "add", &path, "-b", branch, base]).map(drop),
        None => run(dir, &["worktree", "add", &path, branch]).map(drop),
    }
}

/// Remove a linked worktree and its directory. Without `force`, git refuses when the
/// checkout has changes — which is exactly the confirmation mmux wants to surface
/// rather than paper over.
pub fn worktree_remove(dir: &Path, path: &Path, force: bool) -> Result<(), String> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    run(dir, &args).map(drop)
}

/// Forget worktrees whose directories have gone away. Cheap, and it keeps a
/// hand-deleted directory from leaving a stale entry that blocks re-creating the
/// same branch's worktree later.
pub fn worktree_prune(dir: &Path) {
    let _ = run(dir, &["worktree", "prune"]);
}

/// What already holds a branch's commits — i.e. how much a checkout is still the
/// only copy of anything. This is the whole basis for throwing a worktree away, by
/// hand or automatically: a branch that is merged, or fully pushed, has nothing left
/// that only exists here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integration {
    /// Every commit on the branch is already contained in its base.
    pub merged: bool,
    /// It has an upstream and nothing left to push to it.
    pub pushed: bool,
    /// Commits the base branch doesn't have — `0` exactly when `merged`.
    pub ahead: usize,
}

impl Integration {
    /// Whether the branch's work is safe somewhere other than this checkout.
    pub fn is_safe(&self) -> bool {
        self.merged || self.pushed
    }
}

/// Measure `branch` against `base` and against its upstream. Three cheap revision
/// walks; shared by the removal confirmation (so its warning is proportional to what
/// would actually be lost) and by the idle-worktree reaper (so it only ever clears
/// away work that is already somewhere else).
pub fn integration(dir: &Path, branch: &str, base: &str) -> Integration {
    let merged = run(dir, &["merge-base", "--is-ancestor", branch, base]).is_ok();
    let ahead = if merged {
        0
    } else {
        run(dir, &["rev-list", "--count", &format!("{base}..{branch}")])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    // No upstream ⇒ nothing was ever pushed; an upstream with an empty
    // `upstream..branch` range ⇒ the remote has everything.
    let pushed = run(
        dir,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            &format!("{branch}@{{u}}"),
        ],
    )
    .ok()
    .map(|upstream| {
        let range = format!("{}..{}", upstream.trim(), branch);
        run(dir, &["rev-list", "--count", &range])
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .is_some_and(|n| n == 0)
    })
    .unwrap_or(false);
    Integration {
        merged,
        pushed,
        ahead,
    }
}

/// Delete a local branch. `-d`, never `-D`: an unmerged branch is *kept* and git's
/// refusal is returned, so removing a worktree can't silently drop work.
pub fn delete_branch(dir: &Path, name: &str) -> Result<(), String> {
    run(dir, &["branch", "-d", name]).map(drop)
}

/// Merge `branch` into whatever is checked out at `dir`, fast-forwarding when it can
/// (`--no-edit` keeps git's default message and opens no editor). Merging a branch
/// that is checked out in *another* worktree is fine — git only forbids checking one
/// branch out twice.
pub fn merge(dir: &Path, branch: &str) -> Result<String, String> {
    let out = run(dir, &["merge", "--no-edit", branch])?;
    Ok(out.lines().next().unwrap_or("merged").trim().to_string())
}

/// Read a repository config value, `None` when unset or blank. Every checkout of a
/// repository shares one config, so a value written from any worktree is readable
/// from all of them — which is why mmux remembers a branch's base here instead of in
/// a state file that could drift from the repo.
pub fn config_get(dir: &Path, key: &str) -> Option<String> {
    let value = run(dir, &["config", "--get", key]).ok()?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub fn config_set(dir: &Path, key: &str, value: &str) -> Result<(), String> {
    run(dir, &["config", key, value]).map(drop)
}

/// Drop a config key. Unsetting one that was never set exits non-zero, which isn't an
/// error here — so this reports nothing.
pub fn config_unset(dir: &Path, key: &str) {
    let _ = run(dir, &["config", "--unset", key]);
}

/// The subject line of `dir`'s HEAD commit. A worktree's sidebar box already carries
/// its branch in the title, so it shows this instead — the box labels itself with the
/// work as soon as any lands. Empty on a branch with no commits yet.
pub fn head_subject(dir: &Path) -> String {
    run(dir, &["log", "-1", "--format=%s"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
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
        // Native-panel jobs have nowhere useful to accept an interactive credential
        // prompt. Credential helpers still work; a missing credential fails instead
        // of leaving a hidden background worker stuck on terminal input.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
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

    /// A throwaway repo one commit deep, with a bare repo wired up as `origin`.
    /// Returns (worktree, remote), both under a pid+`tag`-keyed temp dir.
    fn scratch_repo(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("mmux-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (work, remote) = (root.join("work"), root.join("remote.git"));
        std::fs::create_dir_all(&work).unwrap();
        run(&root, &["init", "--bare", remote.to_str().unwrap()]).unwrap();
        run(&work, &["init", "-b", "main"]).unwrap();
        run(&work, &["config", "user.email", "t@t.t"]).unwrap();
        run(&work, &["config", "user.name", "t"]).unwrap();
        std::fs::write(work.join("f"), "x").unwrap();
        run(&work, &["add", "-A"]).unwrap();
        run(&work, &["commit", "-m", "init"]).unwrap();
        run(
            &work,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        )
        .unwrap();
        (work, remote)
    }

    /// The reason `push` is more than `run(&["push"])`: a branch with no upstream gets
    /// published instead of refused, and the upstream sticks so the next push is ordinary.
    #[test]
    fn push_publishes_a_branch_with_no_upstream() {
        let (work, remote) = scratch_repo("push");
        assert!(upstream(&work).is_none());

        let flash = push(&work).expect("first push should publish, not fail");
        assert!(flash.contains("origin"), "flash names the remote: {flash}");
        assert_eq!(upstream(&work).as_deref(), Some("origin/main"));
        // The commit really landed on the remote — not just a config write.
        assert!(run(&remote, &["rev-parse", "main"]).is_ok());

        // Now that an upstream exists, the plain path takes over.
        assert_eq!(push(&work).unwrap(), "pushed");
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// `origin` wins when present; otherwise the first remote is the only sane guess.
    #[test]
    fn default_remote_prefers_origin() {
        let (work, _) = scratch_repo("remote");
        run(&work, &["remote", "add", "aaa", "/nowhere"]).unwrap();
        assert_eq!(default_remote(&work).unwrap(), "origin");

        run(&work, &["remote", "remove", "origin"]).unwrap();
        assert_eq!(default_remote(&work).unwrap(), "aaa");

        run(&work, &["remote", "remove", "aaa"]).unwrap();
        assert_eq!(default_remote(&work).unwrap_err(), "no remote configured");
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// The worktree round-trip mmux actually performs: cut a branch into a linked
    /// checkout, remember its base in the shared repo config, merge it back, and take
    /// the worktree away again.
    #[test]
    fn worktree_add_merge_and_remove_round_trip() {
        let (work, _) = scratch_repo("worktree");
        let wt = work.parent().unwrap().join("wt-otter");

        // A fresh branch is cut off the current one and checked out beside it.
        assert!(!branch_exists(&work, "brave-otter"));
        worktree_add(&work, &wt, "brave-otter", Some("main")).unwrap();
        assert!(branch_exists(&work, "brave-otter"));
        assert!(is_clean(&wt));

        // Both checkouts are listed, main first, each with its own branch.
        let list = worktrees(&work);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].branch, "main");
        assert_eq!(canon(&list[0].path), canon(&work));
        assert_eq!(list[1].branch, "brave-otter");
        assert_eq!(main_worktree(&wt).map(|p| canon(&p)), Some(canon(&work)));

        // The base is remembered in the repository config, so it is readable from
        // the worktree as well as from main.
        config_set(&work, "branch.brave-otter.mmuxbase", "main").unwrap();
        assert_eq!(
            config_get(&wt, "branch.brave-otter.mmuxbase").as_deref(),
            Some("main")
        );

        // Work in the worktree, then merge it back from the main checkout.
        std::fs::write(wt.join("f"), "changed").unwrap();
        assert!(!is_clean(&wt));
        run(&wt, &["commit", "-am", "edit"]).unwrap();
        assert!(is_clean(&wt));
        merge(&work, "brave-otter").unwrap();
        assert_eq!(std::fs::read_to_string(work.join("f")).unwrap(), "changed");

        // Removing takes the checkout away; the now-merged branch deletes cleanly.
        worktree_remove(&work, &wt, false).unwrap();
        assert!(!wt.exists());
        assert_eq!(worktrees(&work).len(), 1);
        delete_branch(&work, "brave-otter").unwrap();
        assert!(!branch_exists(&work, "brave-otter"));

        config_unset(&work, "branch.brave-otter.mmuxbase");
        assert!(config_get(&work, "branch.brave-otter.mmuxbase").is_none());
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// The two refusals mmux relies on rather than re-implementing: a dirty worktree
    /// isn't removed without `--force`, and an unmerged branch isn't deleted at all.
    #[test]
    fn worktree_removal_protects_uncommitted_and_unmerged_work() {
        let (work, _) = scratch_repo("worktree-guard");
        let wt = work.parent().unwrap().join("wt-guard");
        worktree_add(&work, &wt, "spicy-toaster", Some("main")).unwrap();
        // A commit main never saw, so the branch is genuinely unmerged below…
        std::fs::write(wt.join("f"), "committed").unwrap();
        run(&wt, &["commit", "-am", "work"]).unwrap();
        // …plus a change that was never committed at all.
        std::fs::write(wt.join("f"), "uncommitted").unwrap();
        assert!(worktree_remove(&work, &wt, false).is_err());
        assert!(wt.exists());
        // …but the user can still say yes.
        worktree_remove(&work, &wt, true).unwrap();
        assert!(!wt.exists());

        // The branch carries a commit main never saw, so `-d` declines to drop it.
        assert!(delete_branch(&work, "spicy-toaster").is_err());
        assert!(branch_exists(&work, "spicy-toaster"));
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// An existing branch is checked out rather than re-created, and git's own refusal
    /// to have one branch out in two places is what stops a double checkout.
    #[test]
    fn worktree_add_checks_out_an_existing_branch() {
        let (work, _) = scratch_repo("worktree-existing");
        run(&work, &["branch", "noble-parsnip"]).unwrap();
        let wt = work.parent().unwrap().join("wt-existing");
        worktree_add(&work, &wt, "noble-parsnip", None).unwrap();
        assert_eq!(worktrees(&work)[1].branch, "noble-parsnip");

        // The same branch a second time is git's error, surfaced as-is.
        let twice = work.parent().unwrap().join("wt-existing-2");
        assert!(worktree_add(&work, &twice, "noble-parsnip", None).is_err());
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// The judgement the reaper and the removal warning both rest on: work is only
    /// disposable once the base branch or a remote already has it.
    #[test]
    fn integration_tracks_merged_and_pushed_separately() {
        let (work, _) = scratch_repo("integration");
        let wt = work.parent().unwrap().join("wt-integration");
        worktree_add(&work, &wt, "feral-ferret", Some("main")).unwrap();

        // Brand new branch, no commits of its own: already contained in main.
        let fresh = integration(&work, "feral-ferret", "main");
        assert_eq!(
            fresh,
            Integration {
                merged: true,
                pushed: false,
                ahead: 0
            }
        );
        assert!(fresh.is_safe());

        // A commit only this checkout has: not merged, not pushed, nothing to fall
        // back on — the case that must never be reaped.
        std::fs::write(wt.join("f"), "work").unwrap();
        run(&wt, &["commit", "-am", "work"]).unwrap();
        let stranded = integration(&work, "feral-ferret", "main");
        assert_eq!(
            stranded,
            Integration {
                merged: false,
                pushed: false,
                ahead: 1
            }
        );
        assert!(!stranded.is_safe());

        // Pushing is enough on its own — the remote now holds it, merged or not.
        run(&wt, &["push", "--set-upstream", "origin", "feral-ferret"]).unwrap();
        let pushed = integration(&work, "feral-ferret", "main");
        assert!(pushed.pushed && !pushed.merged && pushed.ahead == 1);
        assert!(pushed.is_safe());

        // A further local commit strands it again until it's pushed or merged.
        std::fs::write(wt.join("f"), "more").unwrap();
        run(&wt, &["commit", "-am", "more"]).unwrap();
        assert!(!integration(&work, "feral-ferret", "main").is_safe());

        // Merging is the other way home.
        merge(&work, "feral-ferret").unwrap();
        let merged = integration(&work, "feral-ferret", "main");
        assert!(merged.merged && merged.ahead == 0 && merged.is_safe());
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// Every read the git panel makes is scoped to the directory it's given, so a
    /// worktree's panel shows *that* checkout and nothing else. `GitPanel` holds one
    /// `dir` per project and the app draws the active project's panel, so this is the
    /// property the whole "the panel follows the checkout you're in" behaviour rests on.
    #[test]
    fn panel_reads_are_scoped_to_their_own_checkout() {
        let (work, _) = scratch_repo("scoped");
        let wt = work.parent().unwrap().join("wt-scoped");
        worktree_add(&work, &wt, "tidy-walrus", Some("main")).unwrap();

        // Change one file in each checkout, differently.
        std::fs::write(work.join("only-main"), "m").unwrap();
        std::fs::write(wt.join("only-wt"), "w").unwrap();
        std::fs::write(wt.join("f"), "edited").unwrap();

        let main = status(&work);
        let side = status(&wt);

        // Each reports its own branch…
        assert_eq!(main.branch, "main");
        assert_eq!(side.branch, "tidy-walrus");

        // …and strictly its own changed paths.
        let names = |s: &Status| {
            let mut v: Vec<String> = s.files.iter().map(|f| f.path.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(names(&main), vec!["only-main".to_string()]);
        assert_eq!(names(&side), vec!["f".to_string(), "only-wt".to_string()]);

        // The diff the preview pane renders is scoped the same way: `f` is modified in
        // the worktree and untouched in main.
        assert!(diff(&wt, "f", false).contains("edited"));
        assert!(diff(&work, "f", false).is_empty());

        // As is the commit log each panel shows.
        run(&wt, &["commit", "-am", "worktree only"]).unwrap();
        assert_eq!(log(&wt, 1)[0].summary, "worktree only");
        assert_eq!(log(&work, 1)[0].summary, "init");
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    #[test]
    fn head_subject_reads_the_tip_commit() {
        let (work, _) = scratch_repo("subject");
        assert_eq!(head_subject(&work), "init");
        let _ = std::fs::remove_dir_all(work.parent().unwrap());
    }

    /// Temp dirs are symlinked on macOS (`/tmp` → `/private/tmp`), so paths that come
    /// back from git are compared canonically.
    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

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
            [TreeRow::Dir {
                label, depth: 0, ..
            }, TreeRow::File { idx: 0, depth: 1 }, TreeRow::File { idx: 1, depth: 0 }] => {
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
            [TreeRow::Dir {
                label,
                path,
                depth: 0,
                ..
            }] => {
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
            [TreeRow::Dir {
                path,
                depth: 0,
                staged: Stage::Partial,
                ..
            }, TreeRow::File { depth: 1, .. }, TreeRow::File { depth: 1, .. }] => {
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
        let e =
            parse_change("2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs\told.rs").unwrap();
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
