//! Resume support for four agents mmux ships presets for: **Claude Code**,
//! **Codex**, **Pi**, and **Grok**. This is deliberately *not* configurable — detection is
//! purely the launch command's basename, and each tool's quirks live here:
//!
//! - **Claude** and **Grok** let us *own* the session id: we mint a UUID, start
//!   them with `--session-id <uuid>`, and later reattach with `--resume <uuid>`.
//!   That means several instances in one directory each resume their own conversation.
//! - **Pi** also accepts an owned `--session-id <uuid>`, but uses that same flag
//!   both to create the session and to reopen it.
//! - **Codex** has no "set the id" flag — it only resumes one we *discover*. So we
//!   start it plain, find the session it wrote under `~/.codex/sessions`, and
//!   reattach with `codex resume <uuid>`.
//!
//! Claude, Pi, and Grok's minted ids are authoritative — mmux launches by them and
//! resumes by them, so each instance keeps its own thread and several in one
//! directory never get mixed up. Codex hands us no id, so a fresh Codex agent
//! has to *discover* the session it just created via [`sessions_for`]. Claude and
//! Codex both write one transcript per conversation tagged with its `cwd`. Codex
//! candidates are matched against the pane's launch time, so an existing conversation
//! from the same directory can never be adopted.
//! Used by [`crate::app`] to persist and restore agents across a quit/crash/self-update
//! reopen (see [`crate::restore`]).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    Pi,
    Grok,
}

impl Tool {
    /// Detect a resumable agent from its launch command by basename, so
    /// `claude`, `/opt/homebrew/bin/claude`, `codex`, `pi`, and `grok` all match.
    pub fn detect(cmd: &str) -> Option<Tool> {
        match Path::new(cmd).file_name()?.to_str()? {
            "claude" => Some(Tool::Claude),
            "codex" => Some(Tool::Codex),
            "pi" => Some(Tool::Pi),
            "grok" => Some(Tool::Grok),
            _ => None,
        }
    }

    /// Whether mmux assigns the session id at launch (Claude/Pi/Grok) rather than having
    /// to discover it afterwards (Codex).
    pub fn owns_id(self) -> bool {
        matches!(self, Tool::Claude | Tool::Pi | Tool::Grok)
    }

    /// The launch args that hand this agent its first prompt, or `None` when it can't
    /// take one on the command line (the caller then types it in). Claude and Codex
    /// both start an interactive session already working on a trailing positional
    /// prompt. A prompt that looks like a flag goes after `--` so it stays a prompt.
    pub fn prompt_args(self, prompt: &str) -> Option<Vec<String>> {
        if !matches!(self, Tool::Claude | Tool::Codex) {
            return None;
        }
        Some(match prompt.starts_with('-') {
            true => vec!["--".into(), prompt.into()],
            false => vec![prompt.into()],
        })
    }

    /// The launch args that append `text` to this agent's system prompt — never
    /// replace it. Claude and Pi take `--append-system-prompt`; Grok its documented
    /// `--rules` (`--append-system-prompt` is only a hidden alias there). Codex has no
    /// such flag, so the text goes in as a `-c developer_instructions=…` config override,
    /// which `-c` parses as TOML; that shadows any `developer_instructions` in the user's
    /// `config.toml` for this launch. Every launch passes them, resumes included: each
    /// tool keeps a single copy (Claude reuses its recorded prompt, Pi diffs a named
    /// section, Grok rewrites its one system message, Codex keeps the first launch's
    /// in history), and each re-renders from the current flags at some point — Pi on
    /// every reopen, the others when they rebuild context (a compaction, say).
    pub fn context_args(self, text: &str) -> Vec<String> {
        match self {
            Tool::Claude | Tool::Pi => vec!["--append-system-prompt".into(), text.into()],
            Tool::Grok => vec!["--rules".into(), text.into()],
            Tool::Codex => vec![
                "-c".into(),
                format!("developer_instructions={}", toml_string(text)),
            ],
        }
    }
}

/// What a detected agent is told about running inside mmux, via
/// [`Tool::context_args`]. Deliberately static — identical for every pane, so it never
/// breaks the agent's prompt cache; the per-pane details are in its environment.
pub const MMUX_NOTE: &str =
    "You are running inside mmux, a terminal multiplexer for coding agents. \
Your pane is $MMUX_SESSION in project $MMUX_PROJECT (environment variables). The `mmux` CLI \
drives this session from your shell: list and read other agents, send them input, start \
agents or git worktrees, wait for replies. Run `mmux ls --help` for the control reference \
and `mmux docs` for the full guide. Only coordinate other agents when the user asks you to.";

/// `s` as a TOML basic string, for a Codex `-c key=value` override.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Per-session resume bookkeeping for a Claude/Codex/Pi/Grok agent: which tool, the
/// session id we reattach by, and whether the *next* spawn should resume an
/// existing session rather than start a fresh one.
#[derive(Clone)]
pub struct Resume {
    pub tool: Tool,
    /// The session id. Claude/Pi/Grok: minted up front. Codex: `None` until discovered.
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
    /// A fresh resumable agent: Claude/Pi/Grok get a minted id; Codex starts id-less.
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
            (Tool::Claude | Tool::Grok, false, Some(id)) => {
                vec!["--session-id".into(), id.into()]
            }
            (Tool::Claude | Tool::Grok, true, Some(id)) => {
                vec!["--resume".into(), id.into()]
            }
            // Pi uses the same exact-id option to create a missing project session
            // and to reopen one that already exists.
            (Tool::Pi, _, Some(id)) => vec!["--session-id".into(), id.into()],
            // Codex `resume` is a subcommand taking the session UUID.
            (Tool::Codex, true, Some(id)) => vec!["resume".into(), id.into()],
            _ => Vec::new(),
        }
    }
}

/// A v4 UUID from `/dev/urandom`, formatted `8-4-4-4-12`. Enough for Claude/Pi/Grok's
/// `--session-id` without pulling in the `uuid`/`rand` crates. Falls back to a
/// time-seeded value if `/dev/urandom` is somehow unreadable; a collision there
/// could at worst fail a launch or resume the wrong conversation, never corrupt
/// anything.
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
pub fn sessions_for(
    tool: Tool,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> Vec<(String, SystemTime)> {
    match session_root(tool, env) {
        Some(root) => scan_sessions(tool, &root, cwd),
        None => Vec::new(),
    }
}

/// Where `tool` keeps its per-conversation transcripts: `$CLAUDE_CONFIG_DIR/projects` /
/// `$CODEX_HOME/sessions`, else under `$HOME`. The variable is looked up in the agent's
/// own recipe `env` first, then in ours (which its pane inherits) — so both discovery
/// and `mmux last` look where the agent actually writes.
pub fn session_root(tool: Tool, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let (var, dir, sub) = match tool {
        Tool::Claude => ("CLAUDE_CONFIG_DIR", ".claude", "projects"),
        Tool::Codex => ("CODEX_HOME", ".codex", "sessions"),
        // Pi/Grok ids are minted before launch, so their on-disk session trees never
        // need to be scanned to discover which conversation belongs to a pane.
        Tool::Pi | Tool::Grok => return None,
    };
    let base = env
        .get(var)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os(var).map(PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    Some(match base {
        Some(base) => base.join(sub),
        None => home()?.join(dir).join(sub),
    })
}

/// The transcripts under `root` whose recorded `cwd` matches, newest first. Split
/// from [`sessions_for`] so the home-independent scan is unit-testable.
fn scan_sessions(tool: Tool, root: &Path, cwd: &Path) -> Vec<(String, SystemTime)> {
    let Some(want) = cwd.to_str() else {
        return Vec::new();
    };
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
            Tool::Pi | Tool::Grok => None,
        };
        if let Some((id, file_cwd)) = meta {
            if file_cwd == want {
                // Codex UUIDv7 ids embed creation time. File mtime instead tracks
                // activity and made old but recently-used sessions look new.
                let started_at = match tool {
                    Tool::Codex => codex_id_time(&id).unwrap_or(mtime),
                    Tool::Claude | Tool::Pi | Tool::Grok => mtime,
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
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let cwd = head.lines().find_map(|l| json_str_field(l, "cwd"))?;
    Some((id, cwd))
}

/// Pull `(session_id, cwd)` out of a Codex rollout file's first line without a
/// JSON parser — the header is a single line of `"key":"value"` pairs.
fn read_codex_meta(path: &Path) -> Option<(String, String)> {
    let mut buf = [0u8; 4096];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .ok()?;
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

/// The last reply agent `tool` wrote to conversation `id`, read from its own
/// transcript — the words it answered with, free of TUI chrome. Claude: the text
/// blocks of the latest assistant message in `~/.claude/projects/<dir>/<id>.jsonl`.
/// Codex: the latest agent message in its `rollout-…-<id>.jsonl`. `None` for Pi/Grok,
/// or when the transcript is missing or holds no reply yet. Reads only the file's
/// tail unless the reply sits further back.
pub fn last_reply(tool: Tool, id: &str, cwd: &Path, root: &Path) -> Option<String> {
    let path = transcript_path(tool, id, cwd, root)?;
    // Most replies are in the last few hundred KiB; widen only when they aren't.
    for window in [256 * 1024, 4 * 1024 * 1024, u64::MAX] {
        let (text, whole) = read_tail(&path, window)?;
        let lines: Vec<&str> = text.lines().collect();
        let found = match tool {
            Tool::Claude => claude_reply(&lines),
            Tool::Codex => codex_reply(&lines),
            Tool::Pi | Tool::Grok => None,
        };
        if found.is_some() || whole {
            return found;
        }
    }
    None
}

/// Where conversation `id` is recorded under `root` (its tool's [`session_root`]).
/// Claude files it under its launch directory with every non-alphanumeric character
/// turned into `-`; that guess is checked, then every project directory is (Claude's
/// own naming has shifted before). Codex dates its rollouts, so the tree is walked for
/// the file ending in the id.
fn transcript_path(tool: Tool, id: &str, cwd: &Path, root: &Path) -> Option<PathBuf> {
    let file = format!("{id}.jsonl");
    match tool {
        Tool::Claude => {
            let slug: String = cwd
                .to_string_lossy()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            let guess = root.join(slug).join(&file);
            if guess.is_file() {
                return Some(guess);
            }
            std::fs::read_dir(root)
                .ok()?
                .flatten()
                .map(|e| e.path().join(&file))
                .find(|p| p.is_file())
        }
        Tool::Codex => {
            let mut files = Vec::new();
            collect_jsonl(root, &mut files, 0);
            let suffix = format!("-{file}");
            files
                .into_iter()
                .map(|(_, p)| p)
                .find(|p| p.to_string_lossy().ends_with(&suffix))
        }
        Tool::Pi | Tool::Grok => None,
    }
}

/// The last `window` bytes of `path` as text, starting on a line boundary, and
/// whether that is the whole file.
fn read_tail(path: &Path, window: u64) -> Option<(String, bool)> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(window);
    // Start one byte early: through the first newline is then exactly the partial
    // record to drop — just that newline when the window already began on a line.
    let from = start.saturating_sub(1);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    if start > 0 {
        let cut = buf
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |n| n + 1);
        buf.drain(..cut);
    }
    // Cut on a byte boundary first, so a character split by the seek never survives.
    Some((String::from_utf8_lossy(&buf).into_owned(), start == 0))
}

/// The text of the latest assistant message among Claude transcript `lines`. Claude
/// writes one record per content block, all sharing the message's `id`, so the
/// message's text blocks are gathered back together; thinking and tool calls are
/// skipped, as are subagent (sidechain) records.
fn claude_reply(lines: &[&str]) -> Option<String> {
    let records: Vec<serde_json::Value> = lines
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|v: &serde_json::Value| {
            v["type"] == "assistant" && v["isSidechain"].as_bool() != Some(true)
        })
        .collect();
    let texts = |v: &serde_json::Value| -> Vec<String> {
        v["message"]["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .filter(|t| !t.trim().is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let last = records.iter().rev().find(|v| !texts(v).is_empty())?;
    let parts: Vec<String> = match last["message"]["id"].as_str() {
        Some(mid) => records
            .iter()
            .filter(|v| v["message"]["id"].as_str() == Some(mid))
            .flat_map(texts)
            .collect(),
        None => texts(last),
    };
    Some(parts.join("\n\n").trim().to_string())
}

/// Where a Claude agent stands on input it was sent at some instant, judged from its
/// transcript — the gate `mmux wait`/`ask` add on top of the working signal, which an
/// agent's boot-time title flash can satisfy before it has touched the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Nothing to judge by (no transcript tree, no dated records): trust the screen.
    Unknown,
    /// Nothing recorded since the input: it hasn't registered it (yet), or the input
    /// was never a prompt (`--no-enter` text, a bare key).
    Pending,
    /// Mid-turn: a prompt or tool result still waiting for the model's next step.
    Open,
    /// At rest: a final answer, an interruption, a local command's output — or a tool
    /// call, running or held at a permission prompt, which the working signal tells
    /// apart.
    Settled,
}

/// [`Turn`] for Claude conversation `id` (filed under `cwd` in transcript tree `root`)
/// with input sent at `since`. A transcript not written yet while its tree exists is
/// [`Turn::Pending`]: a new agent only creates it when its first prompt lands.
pub fn claude_turn(id: &str, cwd: &Path, root: &Path, since: SystemTime) -> Turn {
    let Some(path) = transcript_path(Tool::Claude, id, cwd, root) else {
        return match root.is_dir() {
            true => Turn::Pending,
            false => Turn::Unknown,
        };
    };
    match read_tail(&path, 256 * 1024) {
        Some((text, _)) => claude_turn_in(&text.lines().collect::<Vec<_>>(), since),
        None => Turn::Unknown,
    }
}

/// [`claude_turn`] over transcript `lines`. Subagent (sidechain) and meta records are
/// skipped; "since" allows a little slack, as the input's instant is reconstructed
/// from an age.
fn claude_turn_in(lines: &[&str], since: SystemTime) -> Turn {
    let since = since
        .checked_sub(Duration::from_millis(200))
        .unwrap_or(since);
    let (mut dated, mut reacted, mut last) = (false, false, None);
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["isSidechain"].as_bool() == Some(true) || v["isMeta"].as_bool() == Some(true) {
            continue;
        }
        let step = match v["type"].as_str() {
            Some("user") => claude_user_step(&v["message"]["content"]),
            Some("assistant") => claude_assistant_step(&v["message"]),
            _ => None,
        };
        let Some(step) = step else { continue };
        if let Some(at) = v["timestamp"].as_str().and_then(parse_timestamp) {
            dated = true;
            reacted |= at >= since;
        }
        last = Some(step);
    }
    match (dated, reacted, last) {
        (false, _, _) | (_, _, None) => Turn::Unknown,
        (true, false, _) => Turn::Pending,
        (true, true, Some(step)) => step,
    }
}

/// A user record's place in the turn: a tool result or a prompt keeps it open; an
/// interruption or a local command's output (`/clear`, …) ends it.
fn claude_user_step(content: &serde_json::Value) -> Option<Turn> {
    let text = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return Some(Turn::Open);
            }
            let texts: Vec<&str> = blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            if texts.is_empty() {
                return None;
            }
            texts.join("\n")
        }
        _ => return None,
    };
    let text = text.trim_start();
    let ends = [
        "[Request interrupted",
        "<local-command-stdout>",
        "<local-command-stderr>",
    ];
    Some(match ends.iter().any(|e| text.starts_with(e)) {
        true => Turn::Settled,
        false => Turn::Open,
    })
}

/// An assistant record's place in the turn. An explicit `stop_reason` settles it
/// (`end_turn`, or `tool_use` — a tool step, running or awaiting permission). Records
/// written mid-stream carry none; then a text or tool block counts as settled (the
/// working signal covers what follows) and a lone thinking block does not.
fn claude_assistant_step(message: &serde_json::Value) -> Option<Turn> {
    if message["stop_reason"].as_str().is_some() {
        return Some(Turn::Settled);
    }
    let blocks = message["content"].as_array()?;
    Some(
        match blocks
            .iter()
            .any(|b| b["type"] == "text" || b["type"] == "tool_use")
        {
            true => Turn::Settled,
            false => Turn::Open,
        },
    )
}

/// An RFC 3339 UTC timestamp as Claude writes them (`2025-06-01T12:34:56.789Z`).
fn parse_timestamp(s: &str) -> Option<SystemTime> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let rest = s.get(19..)?;
    let (frac, zone) = match rest.strip_prefix('.') {
        Some(r) => r.split_at(r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len())),
        None => ("", rest),
    };
    if zone != "Z" && zone != "+00:00" {
        return None;
    }
    let millis: u64 = format!("{frac:0<3}").get(..3)?.parse().ok()?;
    // Days since the epoch for a proleptic Gregorian date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let secs = u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + sec).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs) + Duration::from_millis(millis))
}

/// The latest agent message among Codex rollout `lines`: an `agent_message` event,
/// or an assistant `message` item's `output_text` parts.
fn codex_reply(lines: &[&str]) -> Option<String> {
    lines.iter().rev().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        let p = &v["payload"];
        let text = match v["type"].as_str()? {
            "event_msg" if p["type"] == "agent_message" => p["message"].as_str()?.to_string(),
            "response_item" if p["type"] == "message" && p["role"] == "assistant" => p["content"]
                .as_array()?
                .iter()
                .filter(|c| c["type"] == "output_text")
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return None,
        };
        let text = text.trim().to_string();
        (!text.is_empty()).then_some(text)
    })
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
    fn only_claude_and_codex_take_a_prompt_argument() {
        assert_eq!(Tool::Claude.prompt_args("hi"), Some(vec!["hi".to_string()]));
        assert_eq!(
            Tool::Codex.prompt_args("-x"),
            Some(vec!["--".to_string(), "-x".to_string()])
        );
        assert_eq!(Tool::Pi.prompt_args("hi"), None);
        assert_eq!(Tool::Grok.prompt_args("hi"), None);
    }

    #[test]
    fn context_appends_to_the_system_prompt_per_tool() {
        for tool in [Tool::Claude, Tool::Pi] {
            assert_eq!(tool.context_args("hi"), ["--append-system-prompt", "hi"]);
        }
        assert_eq!(Tool::Grok.context_args("hi"), ["--rules", "hi"]);
        assert_eq!(
            Tool::Codex.context_args("say \"hi\"\\\nnow\u{1}"),
            ["-c", r#"developer_instructions="say \"hi\"\\\nnow\u0001""#]
        );
    }

    #[test]
    fn claude_reply_gathers_the_last_messages_text_blocks() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"do it"}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Looking."}]}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"thinking","thinking":"hmm"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done: 3 tests fixed."}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"All green."}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"id":"m3","content":[{"type":"text","text":"subagent"}]}}"#,
            r#"{"type":"system","subtype":"x"}"#,
            "not json",
        ];
        assert_eq!(
            claude_reply(&lines).as_deref(),
            Some("Done: 3 tests fixed.\n\nAll green.")
        );
        assert_eq!(claude_reply(&lines[..1]), None);
    }

    #[test]
    fn codex_reply_takes_the_latest_agent_message() {
        let lines = [
            r#"{"type":"session_meta","payload":{"id":"x"}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"first"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"q"}]}}"#,
        ];
        assert_eq!(codex_reply(&lines).as_deref(), Some("second"));
        assert_eq!(codex_reply(&lines[..2]).as_deref(), Some("first"));
        assert_eq!(codex_reply(&lines[..1]), None);
    }

    #[test]
    fn parses_claude_timestamps() {
        let at =
            |s: &str| parse_timestamp(s).map(|t| t.duration_since(SystemTime::UNIX_EPOCH).unwrap());
        assert_eq!(at("1970-01-01T00:00:00Z"), Some(Duration::ZERO));
        assert_eq!(
            at("2000-03-01T00:00:00.5Z"),
            Some(Duration::from_millis(951_868_800_500))
        );
        assert_eq!(
            at("2025-06-01T12:34:56.789Z"),
            Some(Duration::from_millis(1_748_781_296_789))
        );
        assert_eq!(at("2025-06-01T12:34:56+02:00"), None);
        assert_eq!(at("yesterday"), None);
    }

    #[test]
    fn claude_turn_waits_for_the_turn_on_the_input_to_settle() {
        let since = parse_timestamp("2025-06-01T12:00:00Z").unwrap();
        let old = r#"{"type":"assistant","timestamp":"2025-06-01T11:59:00.000Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"earlier"}]}}"#;
        let prompt = r#"{"type":"user","timestamp":"2025-06-01T12:00:01.000Z","message":{"role":"user","content":"fix it"}}"#;
        let think = r#"{"type":"assistant","timestamp":"2025-06-01T12:00:02.000Z","message":{"stop_reason":null,"content":[{"type":"thinking","thinking":"hmm"}]}}"#;
        let tool = r#"{"type":"assistant","timestamp":"2025-06-01T12:00:03.000Z","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let result = r#"{"type":"user","timestamp":"2025-06-01T12:00:04.000Z","message":{"role":"user","content":[{"type":"tool_result"}]}}"#;
        let done = r#"{"type":"assistant","timestamp":"2025-06-01T12:00:05.000Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Fixed."}]}}"#;
        let side = r#"{"type":"user","isSidechain":true,"timestamp":"2025-06-01T12:00:06.000Z","message":{"role":"user","content":"subagent task"}}"#;
        let stop = r#"{"type":"user","timestamp":"2025-06-01T12:00:04.000Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        let turn = |lines: &[&str]| claude_turn_in(lines, since);
        // An earlier turn's answer is not an answer to this input.
        assert_eq!(turn(&[old]), Turn::Pending);
        assert_eq!(turn(&[old, prompt]), Turn::Open);
        assert_eq!(turn(&[old, prompt, think]), Turn::Open);
        assert_eq!(turn(&[prompt, think, tool]), Turn::Settled);
        assert_eq!(turn(&[prompt, think, tool, result]), Turn::Open);
        assert_eq!(turn(&[prompt, think, tool, result, done]), Turn::Settled);
        assert_eq!(turn(&[prompt, tool, result, done, side]), Turn::Settled);
        assert_eq!(turn(&[prompt, think, stop]), Turn::Settled);
        // Undated or unreadable transcripts can't gate anything.
        assert_eq!(
            turn(&[r#"{"type":"user","message":{"content":"x"}}"#]),
            Turn::Unknown
        );
        assert_eq!(turn(&["not json"]), Turn::Unknown);
    }

    #[test]
    fn read_tail_starts_on_a_line_boundary() {
        let dir = std::env::temp_dir().join(format!("mmux-tail-{}", mint_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(&path, "aaaa\nbbbb\ncccc\n").unwrap();
        let (text, whole) = read_tail(&path, 7).unwrap();
        assert_eq!(text, "cccc\n");
        assert!(!whole);
        // A window that begins exactly on a line keeps that line.
        let (text, _) = read_tail(&path, 5).unwrap();
        assert_eq!(text, "cccc\n");
        let (text, whole) = read_tail(&path, 1024).unwrap();
        assert_eq!(text, "aaaa\nbbbb\ncccc\n");
        assert!(whole);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_by_basename() {
        assert_eq!(Tool::detect("claude"), Some(Tool::Claude));
        assert_eq!(Tool::detect("/opt/homebrew/bin/claude"), Some(Tool::Claude));
        assert_eq!(Tool::detect("codex"), Some(Tool::Codex));
        assert_eq!(Tool::detect("/usr/local/bin/codex"), Some(Tool::Codex));
        assert_eq!(Tool::detect("pi"), Some(Tool::Pi));
        assert_eq!(Tool::detect("/opt/homebrew/bin/pi"), Some(Tool::Pi));
        assert_eq!(Tool::detect("grok"), Some(Tool::Grok));
        assert_eq!(Tool::detect("/Users/me/.grok/bin/grok"), Some(Tool::Grok));
        assert_eq!(Tool::detect("vim"), None);
        assert_eq!(Tool::detect("zsh"), None);
    }

    #[test]
    fn mints_uuid_shaped_ids() {
        let id = mint_uuid();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(&id[14..15], "4"); // version nibble
        assert_ne!(mint_uuid(), mint_uuid());
    }

    #[test]
    fn claude_pi_and_grok_own_ids_codex_does_not() {
        assert!(Tool::Claude.owns_id());
        assert!(!Tool::Codex.owns_id());
        assert!(Tool::Pi.owns_id());
        assert!(Tool::Grok.owns_id());

        // Claude: create then resume.
        let mut r = Resume::new(Tool::Claude);
        let id = r.id.clone().unwrap();
        assert_eq!(
            r.launch_args(),
            vec!["--session-id".to_string(), id.clone()]
        );
        r.resume = true;
        assert_eq!(r.launch_args(), vec!["--resume".to_string(), id]);

        // Codex: id-less first launch is plain; resume only once an id is known.
        let mut c = Resume::new(Tool::Codex);
        assert!(c.id.is_none());
        assert!(c.launch_args().is_empty());
        c.resume = true;
        assert!(c.launch_args().is_empty());
        c.id = Some("abc".into());
        assert_eq!(
            c.launch_args(),
            vec!["resume".to_string(), "abc".to_string()]
        );

        // Pi uses --session-id for both first launch and restore/restart.
        let mut p = Resume::new(Tool::Pi);
        let id = p.id.clone().unwrap();
        assert_eq!(
            p.launch_args(),
            vec!["--session-id".to_string(), id.clone()]
        );
        p.resume = true;
        assert_eq!(p.launch_args(), vec!["--session-id".to_string(), id]);

        // Grok uses long flags for both creating and resuming a conversation.
        let mut g = Resume::new(Tool::Grok);
        let id = g.id.clone().unwrap();
        assert_eq!(
            g.launch_args(),
            vec!["--session-id".to_string(), id.clone()]
        );
        g.resume = true;
        assert_eq!(g.launch_args(), vec!["--resume".to_string(), id]);
    }

    #[test]
    fn parses_codex_meta_fields() {
        let line = r#"{"timestamp":"x","type":"session_meta","payload":{"session_id":"019eff13-03d0-7c73-834c-c9a0c486e170","cwd":"/home/me/proj","originator":"codex-tui"}}"#;
        assert_eq!(
            json_str_field(line, "session_id").as_deref(),
            Some("019eff13-03d0-7c73-834c-c9a0c486e170")
        );
        assert_eq!(
            json_str_field(line, "cwd").as_deref(),
            Some("/home/me/proj")
        );
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
            Some((
                "11111111-1111-4111-8111-111111111111".into(),
                "/home/me/proj".into()
            ))
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
        write(
            &p("dddd4444-4444-4444-8444-444444444444"),
            "{\"type\":\"mode\"}\n",
        );

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
