//! The control socket: how scripts and agents — inside mmux or anywhere else on the
//! machine — drive a running session without the TUI (`mmux ls`, `read`, `send`,
//! `new`, …; the command side is [`crate::ctl`]).
//!
//! mmux's panes are PTYs the inner process owns, not tmux panes, so tmux can't reach
//! them; the socket goes into the inner process itself. Each session serves one Unix
//! socket at `~/.mmux/run/<tmux session name>.sock` — the same canonical-dir hash that
//! names the tmux session and the restore file — inside a `0700` directory, so only the
//! owning user can connect.
//!
//! The wire format is one JSON [`Request`] line in, one JSON [`Response`] line out,
//! then the connection closes. A listener thread accepts, parses, and hands each
//! request to the UI thread over an `mpsc` channel, which `App::tick` drains (see
//! `app/control.rs`) — the same shape as the git and update workers — so every request
//! runs against the live `App` state between frames and nothing is shared across threads.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// How long a connection may take to send its request line.
const REQUEST_WAIT: Duration = Duration::from_secs(5);
/// How long the listener waits for the UI thread to answer. The loop drains requests
/// every frame, so this only runs out when it is frozen (native-copy mode waits for a key).
const REPLY_WAIT: Duration = Duration::from_secs(10);
/// How long a client waits for the answer — a little past [`REPLY_WAIT`], so the
/// server's own "did not answer" reply arrives rather than a bare timeout.
const CLIENT_WAIT: Duration = Duration::from_secs(15);

/// The socket this process serves, once [`Server::start`] succeeded. Panes export it
/// as `MMUX_SOCKET` (see `Session::spawn`), which is how a program inside finds home.
static ADVERTISED: OnceLock<PathBuf> = OnceLock::new();

/// The socket this process serves, if any — for the panes' `MMUX_SOCKET`.
pub fn advertised_socket() -> Option<&'static PathBuf> {
    ADVERTISED.get()
}

/// `~/.mmux/run`, where every session's socket lives. `None` if `$HOME` is unset.
pub fn run_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".mmux").join("run"))
}

/// The socket for the session rooted at `root` (canonicalized here, like tmux's name).
pub fn socket_path(root: &Path) -> Option<PathBuf> {
    let canon = crate::config::canonical(root);
    Some(run_dir()?.join(format!("{}.sock", crate::tmux::session_name(&canon))))
}

/// One request: what to do, plus who is asking.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    /// The caller's own session handle (`s<N>`) when it runs inside an mmux pane — its
    /// `MMUX_SESSION`. Resolves `self`, prefers the caller's project for bare names, and
    /// is what `control.from-panes: false` refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    /// The caller's working directory, so a bare `Claude #1` or `new agent` resolves in
    /// the project the caller is standing in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The caller's `MMUX_DEPTH` (0 outside mmux). See `Session::depth`.
    #[serde(default)]
    pub depth: u32,
    #[serde(flatten)]
    pub cmd: Cmd,
}

/// The verbs. Targets are `s<N>`, a session name (`Claude #2`), `project/name`, or
/// `self`; see `App::resolve_target`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Cmd {
    /// Identify this session: its root and project dirs. Used by client discovery.
    Hello,
    /// Every project and session.
    Ls,
    /// One session plus its status line (title + the last lines of its screen).
    Status { target: String },
    /// The tail of a session's buffer (scrollback + screen) as text. `lines` 0 = all.
    Read {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<usize>,
    },
    /// Paste `text` into the session, then (unless `enter` is false) press Enter.
    Send {
        target: String,
        text: String,
        #[serde(default = "yes")]
        enter: bool,
    },
    /// Press named keys (`Enter`, `C-c`, `Escape`, `Up`, …); unknown words are typed.
    Keys { target: String, keys: Vec<String> },
    /// Start a new agent (from a template) or terminal (optionally running `command`).
    New {
        kind: NewKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// An agent's first prompt. Claude and Codex take it as a launch argument (first
        /// launch only — a restart never repeats it); any other agent has it typed in
        /// once its screen settles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
    },
    /// What the client needs to find an agent's last reply: its transcript identity
    /// (read client-side, off the UI thread) plus the screen tail as a fallback.
    Last { target: String },
    /// Start a stopped/exited/failed session.
    Start { target: String },
    /// Stop a process in place (teardown included); close an agent/terminal.
    Stop {
        target: String,
        #[serde(default)]
        force: bool,
    },
    /// (Re)start a session whether or not it is running.
    Restart { target: String },
    /// Close an agent/terminal (refused while busy unless `force`); stop a process.
    Close {
        target: String,
        #[serde(default)]
        force: bool,
    },
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NewKind {
    Agent,
    Terminal,
}

/// One answer. `data`'s shape depends on the verb (see the `*Info` types below).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl Response {
    pub fn ok(data: Value) -> Response {
        Response {
            ok: true,
            error: None,
            data,
        }
    }

    pub fn err(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            data: Value::Null,
        }
    }
}

/// One session, as every verb reports it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SessionInfo {
    /// `s<N>` — the stable handle to address it by.
    pub id: String,
    /// `agent` | `terminal` | `process`.
    pub kind: String,
    pub name: String,
    /// The project's display name (a worktree's is its branch).
    pub project: String,
    pub project_dir: String,
    /// `running` | `stopped` | `exited` | `failed`.
    pub status: String,
    /// An agent actively working — exactly when its sidebar row spins.
    pub working: bool,
    /// It rang the bell / raised a notification you haven't looked at.
    pub attention: bool,
    /// The program's terminal title (the sidebar subtitle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The last spawn error, if it failed to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How long it has been quiet: since it last worked, or since it launched if it
    /// never has. `0` while working; absent when it never ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_for_ms: Option<u64>,
    /// How long ago control input (`send`, `keys` with Enter, a first prompt) last
    /// reached it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_age_ms: Option<u64>,
    /// It was seen working after that input arrived — so a quiet agent now means
    /// "done with what I sent", not "hasn't started yet".
    #[serde(default)]
    pub worked_since_input: bool,
    /// Control input for it is still queued (a paced Enter, a first prompt waiting for
    /// the agent's screen to settle).
    #[serde(default)]
    pub input_pending: bool,
}

/// A project, as `ls` reports it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProjectInfo {
    pub name: String,
    pub dir: String,
    /// The project in view in the TUI.
    pub active: bool,
    /// Agent templates `new agent` accepts here.
    pub agents: Vec<String>,
    /// For a worktree: the directory of the checkout it was cut from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_of: Option<String>,
}

/// `ls`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Listing {
    pub name: String,
    pub root: String,
    pub projects: Vec<ProjectInfo>,
    pub sessions: Vec<SessionInfo>,
}

/// `status`: the session plus its status line.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StatusInfo {
    #[serde(flatten)]
    pub session: SessionInfo,
    /// The last few non-empty lines of its screen — an agent's own status/input area.
    pub status_line: Vec<String>,
}

/// `read`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ReadOut {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// `last`, as the server answers it. The transcript itself is read by the client.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LastInfo {
    pub id: String,
    pub name: String,
    /// The resumable agent CLI, when mmux knows it (`claude`, `codex`, `pi`, `grok`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<crate::agent::Tool>,
    /// Its conversation id — Codex's only once discovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The directory the agent was launched in (what its transcript is filed under).
    pub cwd: String,
    /// Where its tool keeps transcripts, resolved server-side against the agent's own
    /// environment (`CLAUDE_CONFIG_DIR`/`CODEX_HOME`), which the client's may not share.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcripts: Option<String>,
    /// The tail of its screen, for agents without a readable transcript.
    pub screen: String,
}

/// `last`/`ask`, as the client prints it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Reply {
    pub id: String,
    pub name: String,
    pub reply: String,
    /// `transcript` (the agent's own record of its answer) or `screen` (a scrape).
    pub source: String,
}

/// Any mutating verb: the session it acted on, and what happened.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Done {
    #[serde(flatten)]
    pub session: SessionInfo,
    pub message: String,
}

/// `hello`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Hello {
    pub version: String,
    pub pid: u32,
    pub root: String,
    pub projects: Vec<String>,
}

/// A request handed to the UI thread, with the channel its answer goes back on.
pub struct Incoming {
    pub req: Request,
    pub reply: Sender<Response>,
    /// When the listener handed it over. One still queued past [`REPLY_WAIT`] (the loop
    /// was frozen in native-copy mode) was already answered with a timeout, so it is
    /// dropped rather than acted on behind its caller's back.
    pub at: Instant,
}

impl Incoming {
    /// Whether the caller has already been told this request timed out.
    pub fn expired(&self) -> bool {
        self.at.elapsed() >= REPLY_WAIT
    }
}

/// The listening side, owned by `App`. Dropping it removes the socket file, so a
/// quit (or the re-exec of a self-update, which drops it first) leaves nothing behind.
pub struct Server {
    rx: Receiver<Incoming>,
    path: PathBuf,
}

impl Server {
    /// Bind the socket for the session rooted at `root`. A leftover file from a crashed
    /// run (nothing answers on it) is replaced; a live one is left alone — that
    /// directory is already served.
    pub fn start(root: &Path) -> Result<Server, String> {
        let path = socket_path(root).ok_or("can't locate ~/.mmux (is HOME set?)")?;
        let dir = path.parent().ok_or("bad socket path")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err("another mmux already serves this directory".into());
            }
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                // One thread per connection, so a client that never finishes its
                // request line can't hold up the next one.
                thread::spawn(move || serve(stream, tx));
            }
        });
        let _ = ADVERTISED.set(path.clone());
        Ok(Server { rx, path })
    }

    /// The next waiting request, if any. Never blocks.
    pub fn try_recv(&self) -> Option<Incoming> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answer one connection: read its request line, pass it to the UI thread, write back
/// whatever comes out (or why nothing did).
fn serve(stream: UnixStream, tx: Sender<Incoming>) {
    let _ = stream.set_read_timeout(Some(REQUEST_WAIT));
    let mut line = String::new();
    match BufReader::new(&stream).read_line(&mut line) {
        // A bare connect-and-close is a liveness probe (see `live`): nothing to answer.
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }
    let resp = match serde_json::from_str::<Request>(&line) {
        Err(e) => Response::err(format!("bad request: {e}")),
        Ok(req) => {
            let (reply, answer) = mpsc::channel();
            let at = Instant::now();
            if tx.send(Incoming { req, reply, at }).is_err() {
                Response::err("mmux is shutting down")
            } else {
                answer.recv_timeout(REPLY_WAIT).unwrap_or_else(|_| {
                    Response::err(
                        "mmux did not answer in time (native copy mode waits for a key in the TUI)",
                    )
                })
            }
        }
    };
    let mut out = serde_json::to_string(&resp).unwrap_or_default();
    out.push('\n');
    let _ = (&stream).write_all(out.as_bytes());
}

/// Send one request to the socket at `path` and read the answer.
pub fn call(path: &Path, req: &Request) -> Result<Response, String> {
    let mut stream =
        UnixStream::connect(path).map_err(|e| format!("can't reach {}: {e}", path.display()))?;
    let _ = stream.set_read_timeout(Some(CLIENT_WAIT));
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("can't send to mmux: {e}"))?;
    let mut answer = String::new();
    BufReader::new(stream)
        .read_line(&mut answer)
        .map_err(|e| format!("no answer from mmux: {e}"))?;
    if answer.trim().is_empty() {
        return Err("mmux closed the connection without answering".into());
    }
    serde_json::from_str(&answer).map_err(|e| format!("unreadable answer from mmux: {e}"))
}

/// Whether something is listening at `path` (a stale file refuses the connect).
fn live(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// Find the socket of the mmux session a command is about.
///
/// An explicit `-C <dir>` wins; otherwise a caller inside a pane uses its own mmux
/// (`$MMUX_SOCKET`). Then the directory and each of its ancestors is hashed the way
/// tmux names sessions — which finds a project opened directly *and* a workspace member
/// (the manifest sits above it). Last, every live socket is asked for its project
/// directories and the deepest one containing the directory wins — which covers
/// worktrees, whose checkouts live under `~/.mmux/worktrees` rather than under the
/// project that owns them.
pub fn locate(dir: Option<&Path>) -> Result<PathBuf, String> {
    if dir.is_none() {
        if let Some(sock) = std::env::var_os("MMUX_SOCKET").map(PathBuf::from) {
            if live(&sock) {
                return Ok(sock);
            }
        }
    }
    let start = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let start = crate::config::canonical(&start);
    for ancestor in start.ancestors() {
        if let Some(sock) = socket_path(ancestor) {
            if live(&sock) {
                return Ok(sock);
            }
        }
    }
    let mut best: Option<(usize, PathBuf)> = None;
    let mut running: Vec<String> = Vec::new();
    for (sock, hello) in live_sessions() {
        running.push(hello.root.clone());
        for project in &hello.projects {
            let project = Path::new(project);
            if start.starts_with(project) {
                let depth = project.components().count();
                if best.as_ref().is_none_or(|(d, _)| depth > *d) {
                    best = Some((depth, sock.clone()));
                }
            }
        }
    }
    if let Some((_, sock)) = best {
        return Ok(sock);
    }
    let mut msg = format!("no running mmux found for {}", start.display());
    if running.is_empty() {
        msg.push_str(" (none is running — open one with `mmux`)");
    } else {
        running.sort();
        msg.push_str(&format!(
            "; running: {} — pass -C <dir>",
            running.join(", ")
        ));
    }
    Err(msg)
}

/// Every live session socket with its `hello`, skipping stale files.
fn live_sessions() -> Vec<(PathBuf, Hello)> {
    let Some(dir) = run_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let hello = Request {
        caller: None,
        cwd: None,
        depth: 0,
        cmd: Cmd::Hello,
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sock"))
        .filter_map(|p| {
            let resp = call(&p, &hello).ok().filter(|r| r.ok)?;
            let info: Hello = serde_json::from_value(resp.data).ok()?;
            Some((p, info))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_one_flat_json_object() {
        let req = Request {
            caller: Some("s3".into()),
            cwd: None,
            depth: 1,
            cmd: Cmd::Send {
                target: "Claude #1".into(),
                text: "hi".into(),
                enter: true,
            },
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains(r#""cmd":"send""#), "{line}");
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn omitted_fields_take_their_defaults() {
        let req: Request =
            serde_json::from_str(r#"{"cmd":"send","target":"s1","text":"x"}"#).unwrap();
        assert_eq!(req.depth, 0);
        assert_eq!(req.caller, None);
        assert!(matches!(req.cmd, Cmd::Send { enter: true, .. }));
        let req: Request = serde_json::from_str(r#"{"cmd":"new","kind":"terminal"}"#).unwrap();
        assert!(matches!(
            req.cmd,
            Cmd::New {
                kind: NewKind::Terminal,
                template: None,
                ..
            }
        ));
    }

    #[test]
    fn errors_carry_no_data_and_successes_no_error() {
        let e = serde_json::to_string(&Response::err("nope")).unwrap();
        assert_eq!(e, r#"{"ok":false,"error":"nope"}"#);
        let ok = serde_json::to_string(&Response::ok(serde_json::json!({"a": 1}))).unwrap();
        assert_eq!(ok, r#"{"ok":true,"data":{"a":1}}"#);
    }

    #[test]
    fn socket_path_is_keyed_like_the_tmux_session() {
        let dir = std::env::temp_dir();
        let sock = socket_path(&dir).unwrap();
        let name = crate::tmux::session_name(&crate::config::canonical(&dir));
        assert_eq!(
            sock.file_name().unwrap().to_string_lossy(),
            format!("{name}.sock")
        );
    }
}
