//! Serving the [control socket](crate::control): each request the listener thread
//! hands over is run here, on the UI thread, against the live `App` — drained a
//! bounded batch per [`tick`](super::App::tick).
//!
//! Two rules keep a scripted caller from disturbing the person at the keyboard:
//!
//! - **The cursor never moves.** A control action goes through
//!   [`keep_selection`](App::keep_selection), which puts the selection back on the
//!   same row *by identity* after the session list changes under it, and never takes
//!   focus. Every mutating action flashes a `ctl:` footer note instead, so it's seen.
//! - **Input is paced, never slept.** Typed text and the Enter that submits it are
//!   separate writes ~150 ms apart (an agent's TUI reads a paste followed instantly by
//!   `\r` as a newline in the paste). They wait in the `deferred` queue, flushed each
//!   tick, so the UI thread never blocks.
//!
//! Each tick also samples every agent's [`busy`](Session::busy) signal into
//! `last_working_at`, which is what lets `mmux wait`/`mmux ask` (client-side polling of
//! `status`) tell "finished what I sent" from "hasn't started on it yet".

use super::keymap::{encode_key, parse_key_name};
use super::nav::Nav;
use super::session::{Kind, Session, Status};
use super::{App, Focus};
use crate::control::{
    Cmd, Done, Hello, LastInfo, Listing, NewKind, ProjectInfo, ReadOut, Request, Response,
    SessionInfo, StatusInfo,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// Requests run per tick, so a burst of callers can't stall a frame.
const MAX_PER_TICK: usize = 16;
/// Gap between a pasted prompt and the Enter that submits it.
const ENTER_DELAY: Duration = Duration::from_millis(150);
/// Gap between named keys, so `Escape` then `i` isn't read as one `Alt+i`.
const KEY_GAP: Duration = Duration::from_millis(30);
/// `mmux new` refuses callers this deep: an agent may start agents that start agents,
/// but no further (see [`Session::depth`]).
const DEPTH_LIMIT: u32 = 3;
/// How many non-empty screen lines make up a session's status line.
const STATUS_LINES: usize = 5;
/// How far back the status line looks for those lines. Bounded, because `status` is
/// what `mmux wait` polls several times a second.
const STATUS_TAIL: usize = 60;
/// The screen tail `last` falls back to when there's no transcript to read.
const LAST_SCREEN_LINES: usize = 40;
/// A first prompt typed into an agent (one that can't take it as a launch argument)
/// waits until the screen has been still this long — its TUI is up and listening…
const PROMPT_SETTLE: Duration = Duration::from_millis(800);
/// …or this long at most, so an agent that never stops animating still gets it.
const PROMPT_MAX_WAIT: Duration = Duration::from_secs(10);
/// `read`'s default tail length.
const READ_LINES: usize = 200;

/// Input waiting for its turn: the bytes go to session `id` once `due` has passed.
pub(crate) struct Deferred {
    id: u64,
    due: Instant,
    bytes: Vec<u8>,
}

/// A first prompt waiting to be typed into a freshly started agent once its screen
/// settles (see [`PROMPT_SETTLE`]).
pub(crate) struct PendingPrompt {
    id: u64,
    text: String,
    since: Instant,
    /// Fingerprint of the screen last tick, and since when it has looked like that.
    screen: u64,
    still_since: Instant,
}

/// Where the cursor was before a control action, by identity rather than position.
enum Anchor {
    Session(u64),
    Row(Nav),
}

type Reply = Result<Value, String>;

impl App {
    /// Run the control requests waiting on the socket, then release any input that
    /// is due. Called from [`tick`](super::App::tick).
    pub(crate) fn serve_control(&mut self) {
        self.track_activity();
        for _ in 0..MAX_PER_TICK {
            let Some(incoming) = self.control.as_ref().and_then(|s| s.try_recv()) else {
                break;
            };
            if incoming.expired() {
                continue;
            }
            let resp = match self.handle_control(&incoming.req) {
                Ok(data) => Response::ok(data),
                Err(e) => Response::err(e),
            };
            let _ = incoming.reply.send(resp);
        }
        self.deliver_pending_prompts();
        self.flush_deferred_input();
    }

    /// Stamp every agent seen working this tick — the sidebar spinner's own predicate.
    fn track_activity(&mut self) {
        let now = Instant::now();
        for s in &mut self.sessions {
            if s.kind == Kind::Agent && s.busy() {
                s.last_working_at = Some(now);
            }
        }
    }

    /// Type each waiting first prompt once its agent's screen has drawn something and
    /// then held still for [`PROMPT_SETTLE`] (or [`PROMPT_MAX_WAIT`] has passed). A
    /// prompt whose agent has gone or died is dropped.
    fn deliver_pending_prompts(&mut self) {
        if self.prompts.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut ready: Vec<(u64, String)> = Vec::new();
        let sessions = &self.sessions;
        self.prompts.retain_mut(|p| {
            let Some(pane) = sessions
                .iter()
                .find(|s| s.id == p.id)
                .and_then(|s| s.pane.as_ref())
                .filter(|pane| pane.is_running())
            else {
                return false;
            };
            let screen = pane
                .with_screen(|s| {
                    let text = s.contents();
                    match text.trim().is_empty() {
                        true => 0,
                        false => {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            text.hash(&mut h);
                            h.finish().max(1)
                        }
                    }
                })
                .unwrap_or(0);
            if screen != p.screen {
                p.screen = screen;
                p.still_since = now;
            }
            let settled = screen != 0 && now.duration_since(p.still_since) >= PROMPT_SETTLE;
            if settled || now.duration_since(p.since) >= PROMPT_MAX_WAIT {
                ready.push((p.id, std::mem::take(&mut p.text)));
                return false;
            }
            true
        });
        for (id, text) in ready {
            if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
                self.type_input(i, &text, true);
            }
        }
    }

    fn handle_control(&mut self, req: &Request) -> Reply {
        if req.caller.is_some()
            && !matches!(req.cmd, Cmd::Hello)
            && !self.root_cfg().control_from_panes()
        {
            return Err(
                "control from inside mmux panes is off here (`control: from-panes: false`)".into(),
            );
        }
        match &req.cmd {
            Cmd::Hello => to_value(Hello {
                version: env!("CARGO_PKG_VERSION").to_string(),
                pid: std::process::id(),
                root: self.root.to_string_lossy().into_owned(),
                projects: self
                    .projects
                    .iter()
                    .map(|p| p.dir.to_string_lossy().into_owned())
                    .collect(),
            }),
            Cmd::Ls => to_value(self.listing()),
            Cmd::Status { target } => {
                let i = self.resolve_target(req, target)?;
                to_value(StatusInfo {
                    session: self.session_info(i),
                    status_line: status_line(&self.sessions[i]),
                })
            }
            Cmd::Last { target } => {
                let i = self.resolve_target(req, target)?;
                let s = &self.sessions[i];
                to_value(LastInfo {
                    id: s.handle(),
                    name: s.name.clone(),
                    tool: s.agent.as_ref().map(|r| r.tool),
                    session_id: s.agent.as_ref().and_then(|r| r.id.clone()),
                    cwd: s.recipe.cwd.to_string_lossy().into_owned(),
                    transcripts: s
                        .agent
                        .as_ref()
                        .and_then(|r| crate::agent::session_root(r.tool, &s.recipe.env))
                        .map(|p| p.to_string_lossy().into_owned()),
                    screen: s
                        .pane
                        .as_ref()
                        .and_then(|p| p.text_tail(LAST_SCREEN_LINES))
                        .unwrap_or_default(),
                })
            }
            Cmd::Read { target, lines } => {
                let i = self.resolve_target(req, target)?;
                let s = &self.sessions[i];
                let text = s
                    .pane
                    .as_ref()
                    .and_then(|p| p.text_tail(lines.unwrap_or(READ_LINES)))
                    .unwrap_or_default();
                to_value(ReadOut {
                    id: s.handle(),
                    name: s.name.clone(),
                    text,
                })
            }
            Cmd::Send {
                target,
                text,
                enter,
            } => {
                let i = self.resolve_target(req, target)?;
                self.control_send(i, text, *enter)
            }
            Cmd::Keys { target, keys } => {
                let i = self.resolve_target(req, target)?;
                self.control_keys(i, keys)
            }
            Cmd::New {
                kind,
                template,
                project,
                command,
                prompt,
            } => self.control_new(
                req,
                *kind,
                template.as_deref(),
                project.as_deref(),
                command.as_deref(),
                prompt.as_deref(),
            ),
            Cmd::Start { target } => {
                let i = self.resolve_target(req, target)?;
                if self.sessions[i].is_running() {
                    return Err(format!(
                        "“{}” is already running — use `mmux restart`",
                        self.sessions[i].name
                    ));
                }
                self.keep_selection(|app| app.start_session(i));
                self.control_done(i, "started")
            }
            Cmd::Restart { target } => {
                let i = self.resolve_target(req, target)?;
                self.keep_selection(|app| app.start_session(i));
                self.control_done(i, "restarted")
            }
            Cmd::Stop { target, force } | Cmd::Close { target, force } => {
                let i = self.resolve_target(req, target)?;
                self.control_close(i, *force)
            }
        }
    }

    /// Every project (in sidebar order) and its sessions (agents, terminals, processes).
    fn listing(&self) -> Listing {
        let order = self.project_display_order();
        let mut sessions = Vec::new();
        for &pi in &order {
            for kind in [Kind::Agent, Kind::Terminal, Kind::Process] {
                for (i, s) in self.sessions.iter().enumerate() {
                    if s.project == pi && s.kind == kind {
                        sessions.push(self.session_info(i));
                    }
                }
            }
        }
        let projects = order
            .iter()
            .map(|&pi| {
                let p = &self.projects[pi];
                ProjectInfo {
                    name: p.label(),
                    dir: p.dir.to_string_lossy().into_owned(),
                    active: pi == self.active,
                    agents: p.cfg.agents.iter().map(|a| a.name.clone()).collect(),
                    worktree_of: p
                        .worktree
                        .as_ref()
                        .map(|w| w.parent.to_string_lossy().into_owned()),
                }
            })
            .collect();
        Listing {
            name: self.root_cfg().display_name(),
            root: self.root.to_string_lossy().into_owned(),
            projects,
            sessions,
        }
    }

    fn session_info(&self, i: usize) -> SessionInfo {
        let s = &self.sessions[i];
        let now = Instant::now();
        let working = s.kind == Kind::Agent && s.busy();
        let ms = |t: Instant| now.duration_since(t).as_millis() as u64;
        let quiet_since = s.last_working_at.max(s.launched_at);
        SessionInfo {
            id: s.handle(),
            kind: match s.kind {
                Kind::Agent => "agent",
                Kind::Terminal => "terminal",
                Kind::Process => "process",
            }
            .to_string(),
            name: s.name.clone(),
            project: self.projects[s.project].label(),
            project_dir: self.projects[s.project].dir.to_string_lossy().into_owned(),
            status: match s.status() {
                Status::Stopped => "stopped",
                Status::Running => "running",
                Status::Exited => "exited",
                Status::Failed => "failed",
            }
            .to_string(),
            // The sidebar spinner's own predicate, so "working" means what you see.
            working,
            attention: s.attention(),
            title: s.pane.as_ref().map(|p| p.title()).filter(|t| !t.is_empty()),
            error: s.error.clone(),
            idle_for_ms: match working {
                true => Some(0),
                false => quiet_since.map(ms),
            },
            input_age_ms: s.last_input_at.map(ms),
            worked_since_input: match (s.last_input_at, s.last_working_at) {
                (Some(input), Some(work)) => work > input,
                _ => false,
            },
            input_pending: self.deferred.iter().any(|d| d.id == s.id)
                || self.prompts.iter().any(|p| p.id == s.id),
        }
    }

    /// The reply for a mutating action on session `i`, with its footer note.
    fn control_done(&mut self, i: usize, verb: &str) -> Reply {
        let message = format!("{verb} {}", self.sessions[i].name);
        self.flash(format!("ctl: {message}"));
        to_value(Done {
            session: self.session_info(i),
            message,
        })
    }

    fn control_send(&mut self, i: usize, text: &str, enter: bool) -> Reply {
        if !self.sessions[i]
            .pane
            .as_ref()
            .is_some_and(|p| p.is_running())
        {
            return Err(format!(
                "“{}” is not running — `mmux start` it first",
                self.sessions[i].name
            ));
        }
        self.type_input(i, text, enter);
        self.control_done(i, "sent input to")
    }

    /// Paste `text` into session `i` the way a real terminal would, then press Enter
    /// [`ENTER_DELAY`] later as its own write. No-op without a pane.
    fn type_input(&mut self, i: usize, text: &str, enter: bool) {
        let Some(pane) = self.sessions[i].pane.as_ref() else {
            return;
        };
        let id = self.sessions[i].id;
        if !text.is_empty() {
            let bytes = pane.paste_input(text);
            self.queue_input(id, Duration::ZERO, bytes);
        }
        if enter {
            self.queue_input(id, ENTER_DELAY, b"\r".to_vec());
        }
        self.sessions[i].last_input_at = Some(Instant::now());
        self.flush_deferred_input();
    }

    fn control_keys(&mut self, i: usize, keys: &[String]) -> Reply {
        let Some(pane) = self.sessions[i].pane.as_ref().filter(|p| p.is_running()) else {
            return Err(format!(
                "“{}” is not running — `mmux start` it first",
                self.sessions[i].name
            ));
        };
        let flags = pane.kitty_flags();
        let id = self.sessions[i].id;
        let mut gap = Duration::ZERO;
        for key in keys {
            // Like tmux send-keys: a key name is pressed, anything else is typed.
            let bytes = match parse_key_name(key) {
                Some(k) => encode_key(&k, flags),
                None => key.clone().into_bytes(),
            };
            if !bytes.is_empty() {
                self.queue_input(id, gap, bytes);
                gap = KEY_GAP;
            }
        }
        // Only a submitting key makes this input an agent should answer: an Escape or
        // an arrow must not set `wait` looking for work it will never do.
        let submits =
            keys.iter()
                .filter_map(|k| parse_key_name(k))
                .any(|k| match (k.code, k.modifiers) {
                    (KeyCode::Enter, KeyModifiers::NONE) => true,
                    (KeyCode::Char(c), KeyModifiers::CONTROL) => matches!(c, 'm' | 'j' | 'M' | 'J'),
                    _ => false,
                });
        if submits {
            self.sessions[i].last_input_at = Some(Instant::now());
        }
        self.flush_deferred_input();
        self.control_done(i, "sent keys to")
    }

    fn control_new(
        &mut self,
        req: &Request,
        kind: NewKind,
        template: Option<&str>,
        project: Option<&str>,
        command: Option<&str>,
        prompt: Option<&str>,
    ) -> Reply {
        if req.depth >= DEPTH_LIMIT {
            return Err(format!(
                "refusing to start a session {} levels deep (MMUX_DEPTH) — agents spawning agents stops here",
                req.depth + 1
            ));
        }
        let prompt = prompt.filter(|p| !p.trim().is_empty());
        if prompt.is_some() && kind != NewKind::Agent {
            return Err("--prompt is for agents — use --cmd for a terminal".into());
        }
        // Typed with its Enter straight into a booting agent, a command would be
        // submitted as a prompt: refuse rather than guess.
        if command.is_some_and(|c| !c.trim().is_empty()) && kind != NewKind::Terminal {
            return Err("--cmd is for terminals — use --prompt for an agent".into());
        }
        let pi = self.target_project(req, project)?;
        let mut s = match kind {
            NewKind::Agent => {
                let t = self.agent_template(pi, template)?;
                self.new_agent_session(pi, t)
            }
            NewKind::Terminal => self.new_terminal_session(pi),
        };
        s.depth = req.depth + 1;
        let id = s.id;
        // Claude and Codex take the prompt on their command line (first launch only);
        // anything else has it typed in once its screen settles.
        let typed = match prompt {
            Some(p)
                if s.agent
                    .as_ref()
                    .is_some_and(|r| r.tool.prompt_args(p).is_some()) =>
            {
                s.first_prompt = Some(p.to_string());
                None
            }
            other => other,
        };
        self.keep_selection(|app| app.launch_session(s));
        let i = self.sessions.len() - 1;
        if let Some(text) = typed {
            let now = Instant::now();
            self.prompts.push(PendingPrompt {
                id,
                text: text.to_string(),
                since: now,
                screen: 0,
                still_since: now,
            });
        }
        if let Some(cmd) = command.map(str::trim).filter(|c| !c.is_empty()) {
            // Typed into the shell rather than run via `sh -c`, so the terminal (and
            // the command's output) stays after it finishes. The PTY holds it until
            // the shell reads its first line.
            self.queue_input(id, Duration::ZERO, format!("{cmd}\r").into_bytes());
            self.flush_deferred_input();
        }
        if let Some(e) = self.sessions[i].error.clone() {
            return Err(format!("“{}” failed to start: {e}", self.sessions[i].name));
        }
        self.control_done(i, "started")
    }

    /// `stop`/`close`: a process stops in place (running its `stop:` teardown, like
    /// `x`); an agent or terminal is closed for good — refused while it has live work
    /// (the same signal the close confirmation keys on) unless `force`.
    fn control_close(&mut self, i: usize, force: bool) -> Reply {
        let s = &self.sessions[i];
        if s.kind == Kind::Process {
            let was_running = s.is_running();
            self.sessions[i].stop();
            if was_running {
                self.run_stop_command(i);
            }
            return self.control_done(i, "stopped");
        }
        let busy = match s.kind {
            Kind::Agent => s.busy(),
            _ => s.is_running(),
        };
        if busy && !force {
            let noun = match s.kind {
                Kind::Agent => "is working",
                _ => "is still running",
            };
            return Err(format!(
                "“{}” {noun} — pass --force to close it anyway",
                s.name
            ));
        }
        let mut info = self.session_info(i);
        info.status = "closed".to_string();
        let message = format!("closed {}", self.sessions[i].name);
        self.keep_selection(|app| {
            app.sessions[i].kill();
            app.sessions.remove(i);
        });
        self.flash(format!("ctl: {message}"));
        to_value(Done {
            session: info,
            message,
        })
    }

    /// Run `f` — which may add, remove or restart sessions — and then put the cursor
    /// back on the row it was on, found by identity since positions shift. Focus stays
    /// where it was unless the focused pane itself went away.
    pub(super) fn keep_selection<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let anchor = self.current_nav().map(|n| match n {
            Nav::Session(i) => Anchor::Session(self.sessions[i].id),
            other => Anchor::Row(other),
        });
        let out = f(self);
        let want = anchor.and_then(|a| match a {
            Anchor::Session(id) => self
                .sessions
                .iter()
                .position(|s| s.id == id)
                .map(Nav::Session),
            Anchor::Row(n) => Some(n),
        });
        let nav = self.build_nav();
        match want.and_then(|w| nav.iter().position(|n| *n == w)) {
            Some(pos) => self.sel = pos,
            None => {
                self.sel = self.sel.min(nav.len().saturating_sub(1));
                if self.focus == Focus::Terminal {
                    self.focus = Focus::Sidebar;
                }
            }
        }
        out
    }

    /// Queue `bytes` for session `id`, `gap` after whatever is already queued for it
    /// (or after now), so a caller's writes land in the order it made them.
    fn queue_input(&mut self, id: u64, gap: Duration, bytes: Vec<u8>) {
        let now = Instant::now();
        let after = self
            .deferred
            .iter()
            .filter(|d| d.id == id)
            .map(|d| d.due)
            .max()
            .unwrap_or(now)
            .max(now);
        self.deferred.push(Deferred {
            id,
            due: after + gap,
            bytes,
        });
    }

    /// Write every queued input whose time has come, in order per session. Input for
    /// a session that has since gone is dropped.
    fn flush_deferred_input(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut waiting: Vec<u64> = Vec::new();
        let mut k = 0;
        while k < self.deferred.len() {
            let d = &self.deferred[k];
            if d.due > now || waiting.contains(&d.id) {
                waiting.push(d.id);
                k += 1;
                continue;
            }
            let d = self.deferred.remove(k);
            if let Some(p) = self
                .sessions
                .iter()
                .find(|s| s.id == d.id)
                .and_then(|s| s.pane.as_ref())
            {
                p.send(d.bytes);
            }
        }
    }

    /// Resolve a target to a session index: `self`, `s<N>`, a name (`Claude #2`,
    /// matched ignoring case and spaces, then as a prefix), or `project/name`. Several
    /// matches narrow to the caller's project; if that doesn't settle it, the error
    /// lists them.
    fn resolve_target(&self, req: &Request, target: &str) -> Result<usize, String> {
        let target = target.trim();
        let by_id = |handle: &str| {
            parse_handle(handle).and_then(|id| self.sessions.iter().position(|s| s.id == id))
        };
        if target.eq_ignore_ascii_case("self") {
            let caller = req
                .caller
                .as_deref()
                .ok_or("`self` only works from inside an mmux pane (MMUX_SESSION is not set)")?;
            return by_id(caller).ok_or_else(|| format!("your session {caller} is gone"));
        }
        if parse_handle(target).is_some() {
            return by_id(target).ok_or_else(|| format!("no session {target} — try `mmux ls`"));
        }
        let want = norm(target);
        let mut found: Vec<usize> = self.sessions_named(|n| n == want, None);
        if found.is_empty() {
            if let Some((proj, name)) = target.rsplit_once('/') {
                let projects = self.projects_matching(proj);
                let want = norm(name);
                found = self.sessions_named(|n| n == want, Some(&projects));
                if found.is_empty() {
                    found = self.sessions_named(|n| n.starts_with(&want), Some(&projects));
                }
            }
        }
        if found.is_empty() {
            found = self.sessions_named(|n| n.starts_with(&want), None);
        }
        if found.len() > 1 {
            if let Some(pi) = self.caller_project(req) {
                let mine: Vec<usize> = found
                    .iter()
                    .copied()
                    .filter(|&i| self.sessions[i].project == pi)
                    .collect();
                if mine.len() == 1 {
                    found = mine;
                }
            }
        }
        match found.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("no session matches “{target}” — try `mmux ls`")),
            many => Err(format!(
                "“{target}” is ambiguous: {} — use the id or project/name",
                many.iter()
                    .map(|&i| format!(
                        "{} {} ({})",
                        self.sessions[i].handle(),
                        self.sessions[i].name,
                        self.projects[self.sessions[i].project].label()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// Indices of the sessions whose normalized name passes `pred`, optionally only
    /// within `projects`.
    fn sessions_named(
        &self,
        pred: impl Fn(&str) -> bool,
        projects: Option<&[usize]>,
    ) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| projects.is_none_or(|ps| ps.contains(&s.project)))
            .filter(|(_, s)| pred(&norm(&s.name)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Projects a user-typed name refers to: its display name/branch, its directory's
    /// basename, or its path.
    fn projects_matching(&self, spec: &str) -> Vec<usize> {
        let want = norm(spec);
        let path = crate::config::canonical(Path::new(spec));
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                norm(&p.label()) == want
                    || norm(&p.cfg.display_name()) == want
                    || p.dir
                        .file_name()
                        .is_some_and(|b| norm(&b.to_string_lossy()) == want)
                    || p.dir == path
            })
            .map(|(pi, _)| pi)
            .collect()
    }

    /// The project the caller is in: its own pane's project, else the deepest project
    /// containing its working directory.
    fn caller_project(&self, req: &Request) -> Option<usize> {
        if let Some(i) = req
            .caller
            .as_deref()
            .and_then(parse_handle)
            .and_then(|id| self.sessions.iter().position(|s| s.id == id))
        {
            return Some(self.sessions[i].project);
        }
        let cwd = crate::config::canonical(Path::new(req.cwd.as_deref()?));
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| cwd.starts_with(&p.dir))
            .max_by_key(|(_, p)| p.dir.components().count())
            .map(|(pi, _)| pi)
    }

    /// Which project `new` should start in: the named one, else the caller's, else the
    /// only one.
    fn target_project(&self, req: &Request, spec: Option<&str>) -> Result<usize, String> {
        let names = || {
            self.projects
                .iter()
                .map(|p| p.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(spec) = spec {
            return match self.projects_matching(spec).as_slice() {
                [pi] => Ok(*pi),
                [] => Err(format!("no project “{spec}” — one of: {}", names())),
                _ => Err(format!("project “{spec}” is ambiguous — pass its path")),
            };
        }
        if let Some(pi) = self.caller_project(req) {
            return Ok(pi);
        }
        match self.projects.len() {
            1 => Ok(0),
            _ => Err(format!(
                "which project? pass --project (one of: {})",
                names()
            )),
        }
    }

    /// Which of project `pi`'s agent templates `new agent` means: the named one (by
    /// name, or by command like `claude`), else the first.
    fn agent_template(&self, pi: usize, spec: Option<&str>) -> Result<usize, String> {
        let agents = &self.projects[pi].cfg.agents;
        if agents.is_empty() {
            return Err(format!(
                "{} has no agents configured — run `mmux agents`",
                self.projects[pi].label()
            ));
        }
        let Some(spec) = spec else {
            return Ok(0);
        };
        let want = norm(spec);
        agents
            .iter()
            .position(|a| norm(&a.name) == want)
            .or_else(|| {
                agents.iter().position(|a| {
                    Path::new(&a.cmd)
                        .file_name()
                        .is_some_and(|b| norm(&b.to_string_lossy()) == want)
                })
            })
            .ok_or_else(|| {
                format!(
                    "no agent “{spec}” in {} — one of: {}",
                    self.projects[pi].label(),
                    agents
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

/// The last few non-empty lines of a session's buffer — where an agent keeps its own
/// status line and input box. Empty for a session with no pane.
fn status_line(s: &Session) -> Vec<String> {
    let text = s
        .pane
        .as_ref()
        .and_then(|p| p.text_tail(STATUS_TAIL))
        .unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .rev()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .take(STATUS_LINES)
        .map(str::to_string)
        .collect();
    lines.reverse();
    lines
}

/// `s12` → 12.
fn parse_handle(handle: &str) -> Option<u64> {
    handle
        .strip_prefix('s')
        .or_else(|| handle.strip_prefix('S'))?
        .parse()
        .ok()
}

/// A name as matching sees it: case and whitespace ignored, so `claude#2` is `Claude #2`.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn to_value<T: serde::Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_parse_only_as_s_and_digits() {
        assert_eq!(parse_handle("s12"), Some(12));
        assert_eq!(parse_handle("S3"), Some(3));
        assert_eq!(parse_handle("s"), None);
        assert_eq!(parse_handle("shell"), None);
        assert_eq!(parse_handle("12"), None);
    }

    #[test]
    fn names_match_ignoring_case_and_spaces() {
        assert_eq!(norm("Claude #2"), norm("claude#2"));
        assert_eq!(norm("Dev server"), "devserver");
    }
}
