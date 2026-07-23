//! Resume support for the two agents mmux ships presets for: **Claude Code** and
//! **Codex**. This is deliberately *not* configurable — detection is purely the
//! launch command's basename, and each tool's quirks live here:
//!
//! - **Claude** lets us *own* the session id: we mint a UUID, start it with
//!   `--session-id <uuid>`, and later reattach with `--resume <uuid>`. That means
//!   several `Claude #N` in one directory each resume their own conversation.
//! - **Codex** has no "set the id" flag — it only resumes one we *discover*. So we
//!   start it plain, find the session it wrote under `~/.codex/sessions`, and
//!   reattach with `codex resume <uuid>`.
//!
//! Claude's minted id is authoritative — mmux launches by it and resumes by it, so
//! each `Claude #N` keeps its own thread and several in one directory never get mixed
//! up. Codex hands us no id, so a fresh Codex agent has to *discover* the session it
//! just created via [`sessions_for`]: both tools write one transcript per conversation
//! tagged with its `cwd`. Codex candidates are matched against the pane's launch time,
//! so an existing conversation from the same directory can never be adopted.
//! Used by [`crate::app`] to persist and restore agents across a quit/crash/self-update
//! reopen (see [`crate::restore`]).

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Avoid scanning Codex's transcript tree on every UI tick while its new rollout
/// file is still being created.
const DISCOVERY_RETRY: Duration = Duration::from_millis(500);

/// A resumable agent CLI mmux knows how to reattach across a restart.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    /// Detect a resumable agent from its launch command by basename, so
    /// `claude`, `/opt/homebrew/bin/claude`, and `codex` all match.
    pub fn detect(cmd: &str) -> Option<Tool> {
        match Path::new(cmd).file_name()?.to_str()? {
            "claude" => Some(Tool::Claude),
            "codex" => Some(Tool::Codex),
            _ => None,
        }
    }

    /// Whether mmux assigns the session id at launch (Claude) rather than having
    /// to discover it afterwards (Codex).
    pub fn owns_id(self) -> bool {
        matches!(self, Tool::Claude)
    }
}

/// Per-session resume bookkeeping for a Claude/Codex agent: which tool, the
/// session id we reattach by, and whether the *next* spawn should resume an
/// existing session rather than start a fresh one.
#[derive(Clone)]
pub struct Resume {
    pub tool: Tool,
    /// The session id. Claude: minted up front. Codex: `None` until discovered.
    pub id: Option<String>,
    /// `false` for a brand-new agent (its first launch *creates* the session);
    /// `true` afterwards and for any restored agent (launches *resume* it).
    pub resume: bool,
    /// When an id-less Codex pane was launched. Its rollout must have been created
    /// at or after this instant; otherwise it belongs to an older conversation.
    pub started_at: Option<SystemTime>,
    /// Monotonic throttle for retrying discovery until Codex writes its rollout.
    pub discover_at: Option<Instant>,
}

impl Resume {
    /// A fresh resumable agent: Claude gets a minted id; Codex starts id-less.
    pub fn new(tool: Tool) -> Resume {
        let id = tool.owns_id().then(mint_uuid);
        Resume {
            tool,
            id,
            resume: false,
            started_at: None,
            discover_at: None,
        }
    }

    /// Restore a resumable agent from saved state — always reattaches.
    pub fn restored(tool: Tool, id: Option<String>) -> Resume {
        Resume {
            tool,
            id,
            resume: true,
            started_at: None,
            discover_at: None,
        }
    }

    /// Mark the start of a plain Codex launch whose new id is not known yet.
    pub fn mark_launch(&mut self) {
        if self.tool == Tool::Codex && self.id.is_none() {
            self.started_at = Some(SystemTime::now());
            self.discover_at = Some(Instant::now() + DISCOVERY_RETRY);
        }
    }

    /// Whether an id-less Codex rollout is due for another discovery attempt.
    pub fn discovery_due(&self) -> bool {
        self.tool == Tool::Codex
            && self.id.is_none()
            && self
                .discover_at
                .is_none_or(|deadline| Instant::now() >= deadline)
    }

    /// Delay the next attempt after Codex has not written a matching rollout yet.
    pub fn defer_discovery(&mut self) {
        self.discover_at = Some(Instant::now() + DISCOVERY_RETRY);
    }

    /// The extra CLI args to append to the recipe for the *current* launch.
    /// A Codex first launch (or any id-less state) appends nothing — it starts
    /// a plain session, and the id is discovered later.
    pub fn launch_args(&self) -> Vec<String> {
        match (self.tool, self.resume, self.id.as_deref()) {
            (Tool::Claude, false, Some(id)) => vec!["--session-id".into(), id.into()],
            (Tool::Claude, true, Some(id)) => vec!["--resume".into(), id.into()],
            // Codex `resume` is a subcommand taking the session UUID.
            (Tool::Codex, true, Some(id)) => vec!["resume".into(), id.into()],
            _ => Vec::new(),
        }
    }
}

/// A v4 UUID from `/dev/urandom`, formatted `8-4-4-4-12`. Enough for Claude's
/// `--session-id` without pulling in the `uuid`/`rand` crates. Falls back to a
/// time-seeded value if `/dev/urandom` is somehow unreadable; a collision there
/// would at worst resume the wrong conversation, never corrupt anything.
pub fn mint_uuid() -> String {
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let nanos: u128 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        b.copy_from_slice(&nanos.to_le_bytes()); // u128 → exactly 16 bytes
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 1
    let h = |r: &[u8]| r.iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        h(&b[0..4]),
        h(&b[4..6]),
        h(&b[6..8]),
        h(&b[8..10]),
        h(&b[10..16])
    )
}

/// Every conversation `tool` recorded for `cwd`, as `(session_id, started_at)` and
/// **newest first** — used to discover the session a freshly launched Codex agent
/// just created (see the module docs). Both tools write one `*.jsonl` per session:
/// Claude under `~/.claude/projects/<dir>/<id>.jsonl` (id is the filename, `cwd`
/// is recorded in the opening lines), Codex under `~/.codex/sessions/YYYY/MM/DD/`
/// (id and `cwd` in the first `session_meta` line). Best-effort: an unreadable
/// home or tree yields an empty list.
pub fn sessions_for(tool: Tool, cwd: &Path) -> Vec<(String, SystemTime)> {
    match session_root(tool) {
        Some(root) => scan_sessions(tool, &root, cwd),
        None => Vec::new(),
    }
}

/// Where `tool` keeps its per-conversation transcripts under `$HOME`.
fn session_root(tool: Tool) -> Option<PathBuf> {
    let home = home()?;
    Some(match tool {
        Tool::Claude => home.join(".claude").join("projects"),
        Tool::Codex => home.join(".codex").join("sessions"),
    })
}

/// The transcripts under `root` whose recorded `cwd` matches, newest first. Split
/// from [`sessions_for`] so the home-independent scan is unit-testable.
fn scan_sessions(tool: Tool, root: &Path, cwd: &Path) -> Vec<(String, SystemTime)> {
    let Some(want) = cwd.to_str() else { return Vec::new() };
    let mut files = Vec::new();
    collect_jsonl(root, &mut files, 0);
    // Newest first by modification time.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = Vec::new();
    // Cap the scan so a huge history can't stall the (synchronous) save.
    for (mtime, path) in files.into_iter().take(256) {
        let meta = match tool {
            Tool::Claude => read_claude_meta(&path),
            Tool::Codex => read_codex_meta(&path),
        };
        if let Some((id, file_cwd)) = meta {
            if file_cwd == want {
                // Codex UUIDv7 ids embed creation time. File mtime instead tracks
                // activity and made old but recently-used sessions look new.
                let started_at = match tool {
                    Tool::Codex => codex_id_time(&id).unwrap_or(mtime),
                    Tool::Claude => mtime,
                };
                out.push((id, started_at));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Recursively gather `*.jsonl` files under `dir` as `(modified_time, path)`.
/// Shallow by nature (Codex nests only `YYYY/MM/DD`); capped in depth as a guard.
fn collect_jsonl(dir: &Path, out: &mut Vec<(std::time::SystemTime, PathBuf)>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            collect_jsonl(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            out.push((mtime, path));
        }
    }
}

/// Pull `(session_id, cwd)` out of a Claude transcript: the id is the filename
/// stem (`<id>.jsonl`), and the `cwd` is the first one recorded in the opening
/// lines (the `system` entries Claude writes at launch). A brand-new session whose
/// `cwd` line isn't written yet has no match and is skipped — so a just-spawned
/// agent never binds to a stale conversation. Best-effort: `None` on any problem.
fn read_claude_meta(path: &Path) -> Option<(String, String)> {
    let id = path.file_stem()?.to_str()?.to_string();
    let mut buf = [0u8; 8192];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let cwd = head.lines().find_map(|l| json_str_field(l, "cwd"))?;
    Some((id, cwd))
}

/// Pull `(session_id, cwd)` out of a Codex rollout file's first line without a
/// JSON parser — the header is a single line of `"key":"value"` pairs.
fn read_codex_meta(path: &Path) -> Option<(String, String)> {
    let mut buf = [0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let line = head.lines().next()?;
    let session_id = json_str_field(line, "session_id")?;
    // A subagent rollout carries its parent's `session_id` but its own `id`.
    // It is not a resumable top-level TUI conversation.
    if json_str_field(line, "id").is_some_and(|id| id != session_id) {
        return None;
    }
    Some((session_id, json_str_field(line, "cwd")?))
}

/// Decode the Unix-millisecond timestamp stored in a Codex UUIDv7.
fn codex_id_time(id: &str) -> Option<SystemTime> {
    if id.as_bytes().get(14) != Some(&b'7') {
        return None;
    }
    let prefix = id.get(..13)?.replace('-', "");
    let millis = u64::from_str_radix(&prefix, 16).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(millis))
}

/// Extract the value of a `"field":"value"` string entry from a flat JSON line.
fn json_str_field(line: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\":\"");
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_basename() {
        assert_eq!(Tool::detect("claude"), Some(Tool::Claude));
        assert_eq!(Tool::detect("/opt/homebrew/bin/claude"), Some(Tool::Claude));
        assert_eq!(Tool::detect("codex"), Some(Tool::Codex));
        assert_eq!(Tool::detect("/usr/local/bin/codex"), Some(Tool::Codex));
        assert_eq!(Tool::detect("vim"), None);
        assert_eq!(Tool::detect("zsh"), None);
    }

    #[test]
    fn mints_uuid_shaped_ids() {
        let id = mint_uuid();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(&id[14..15], "4"); // version nibble
        assert_ne!(mint_uuid(), mint_uuid());
    }

    #[test]
    fn claude_owns_id_codex_does_not() {
        assert!(Tool::Claude.owns_id());
        assert!(!Tool::Codex.owns_id());

        // Claude: create then resume.
        let mut r = Resume::new(Tool::Claude);
        let id = r.id.clone().unwrap();
        assert_eq!(r.launch_args(), vec!["--session-id".to_string(), id.clone()]);
        r.resume = true;
        assert_eq!(r.launch_args(), vec!["--resume".to_string(), id]);

        // Codex: id-less first launch is plain; resume only once an id is known.
        let mut c = Resume::new(Tool::Codex);
        assert!(c.id.is_none());
        assert!(c.launch_args().is_empty());
        c.resume = true;
        assert!(c.launch_args().is_empty());
        c.id = Some("abc".into());
        assert_eq!(c.launch_args(), vec!["resume".to_string(), "abc".to_string()]);
    }

    #[test]
    fn parses_codex_meta_fields() {
        let line = r#"{"timestamp":"x","type":"session_meta","payload":{"session_id":"019eff13-03d0-7c73-834c-c9a0c486e170","cwd":"/home/me/proj","originator":"codex-tui"}}"#;
        assert_eq!(
            json_str_field(line, "session_id").as_deref(),
            Some("019eff13-03d0-7c73-834c-c9a0c486e170")
        );
        assert_eq!(json_str_field(line, "cwd").as_deref(), Some("/home/me/proj"));
        assert_eq!(json_str_field(line, "missing"), None);
    }

    #[test]
    fn decodes_codex_uuid_v7_time() {
        let id = "019f8e00-3ade-79a2-95fc-b166a5dfa119";
        let millis = codex_id_time(id)
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        assert_eq!(millis, 0x019f8e003ade);
        assert_eq!(codex_id_time("11111111-1111-4111-8111-111111111111"), None);
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn reads_claude_id_from_filename_and_cwd_from_opening_lines() {
        let dir = std::env::temp_dir().join(format!("mmux-claude-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The `mode`/`permission-mode` preamble carries no cwd; the `system` line does.
        let f = dir.join("11111111-1111-4111-8111-111111111111.jsonl");
        write(
            &f,
            "{\"type\":\"mode\",\"sessionId\":\"x\"}\n\
             {\"type\":\"permission-mode\",\"sessionId\":\"x\"}\n\
             {\"type\":\"system\",\"cwd\":\"/home/me/proj\",\"gitBranch\":\"main\"}\n",
        );
        assert_eq!(
            read_claude_meta(&f),
            Some(("11111111-1111-4111-8111-111111111111".into(), "/home/me/proj".into()))
        );
        // A just-launched session with only the preamble has no cwd yet → no match.
        let g = dir.join("22222222-2222-4222-8222-222222222222.jsonl");
        write(&g, "{\"type\":\"mode\",\"sessionId\":\"y\"}\n");
        assert_eq!(read_claude_meta(&g), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_codex_subagent_rollouts() {
        let dir = std::env::temp_dir().join(format!("mmux-codex-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let top = dir.join("top.jsonl");
        write(
            &top,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"top\",\"id\":\"top\",\"cwd\":\"/repo\"}}\n",
        );
        assert_eq!(read_codex_meta(&top), Some(("top".into(), "/repo".into())));

        let child = dir.join("child.jsonl");
        write(
            &child,
            "{\"type\":\"session_meta\",\"payload\":{\"session_id\":\"top\",\"id\":\"child\",\"cwd\":\"/repo\",\"thread_source\":\"subagent\"}}\n",
        );
        assert_eq!(read_codex_meta(&child), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_sessions_newest_first_filtered_by_cwd() {
        let root = std::env::temp_dir().join(format!("mmux-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let body = |cwd: &str| format!("{{\"type\":\"system\",\"cwd\":\"{cwd}\"}}\n");
        // Two conversations for the wanted cwd (older `a`, then newer `b`), one for
        // a different cwd, and one with no cwd line — all under a project subdir.
        let p = |id: &str| root.join("proj").join(format!("{id}.jsonl"));
        write(&p("aaaa1111-1111-4111-8111-111111111111"), &body("/want"));
        std::thread::sleep(std::time::Duration::from_millis(25));
        write(&p("bbbb2222-2222-4222-8222-222222222222"), &body("/want"));
        write(&p("cccc3333-3333-4333-8333-333333333333"), &body("/other"));
        write(&p("dddd4444-4444-4444-8444-444444444444"), "{\"type\":\"mode\"}\n");

        let ids: Vec<String> = scan_sessions(Tool::Claude, &root, Path::new("/want"))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "bbbb2222-2222-4222-8222-222222222222".to_string(),
                "aaaa1111-1111-4111-8111-111111111111".to_string(),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
