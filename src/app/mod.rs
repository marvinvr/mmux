//! The interactive TUI. This module owns the [`App`] state and the event loop;
//! the behaviour is split across focused submodules:
//!
//! - [`session`] — the unified pane-backed [`Session`] model + its lifecycle.
//! - [`nav`] — the sidebar navigation list and the selection cursor.
//! - [`lifecycle`] — spawn/start/stop/restart actions and the live config reload.
//! - [`input`] — keyboard, mouse and paste handling.
//! - [`keymap`] — pure key-event → PTY-byte translation.
//! - [`view`] — all rendering (layout, sidebar, panes, footer).

mod diff;
mod git;
mod highlight;
mod input;
mod keymap;
mod lifecycle;
mod linkbrowse;
mod nav;
mod overlay;
mod persist;
pub(crate) mod picker;
mod procform;
mod session;
mod view;

pub(crate) use session::{Kind, Recipe, Session, Status};
use diff::DiffView;
use git::{first_line, GitPanel, JobDone};
use overlay::Overlay;
use input::Selection;
use nav::Nav;
use view::Regions;

use crate::config::{Config, NotifyConfig, NotifyMechanism, Workspace};
use crate::notify::{self, Note};
use crate::pane::Notify;
use crate::update::{self, InstallKind, UpdateMsg};
use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::collections::HashMap;
use std::io::{stdout, Stdout, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Which region currently receives keyboard input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Sidebar,
    Terminal,
    Right,
}

/// Background self-update progress, surfaced as the quiet bottom-right footer badge.
/// Driven by [`crate::update`]: a check runs at startup (and every few hours after). A
/// self-managed (native binary) install downloads a found update in the background and goes
/// straight to `Ready`; a Homebrew install parks in `Available` until the user confirms the
/// `brew upgrade`. Either way the badge then invites an in-place restart to apply it.
pub(crate) enum UpdateState {
    /// Nothing known/in flight. The periodic timer may start a check from here.
    Idle,
    /// A version check is running.
    Checking,
    /// A newer `version` exists but needs the user to act — only a Homebrew install rests
    /// here (self-managed installs auto-download instead). Applying it opens the confirm
    /// that runs `brew upgrade mmux`.
    Available(String),
    /// A newer `version` is being downloaded/installed in the background.
    Installing(String),
    /// `version` is installed and staged; the badge offers the restart.
    Ready(String),
    /// Not a managed install, so self-update can't act here. Terminal: the periodic
    /// re-check and the About card's `c` only fire from `Idle`, so we never spin on it.
    Unsupported,
}

pub(crate) struct App {
    /// The workspace's projects in load order. `projects[0]` is the directory mmux
    /// was launched in; the rest come from its `linked-projects`. Each owns its
    /// config, per-agent instance counters, and its own right panel.
    projects: Vec<Project>,
    /// The project whose panel is shown and whose launchers act by default. Tracks
    /// the selected row's project, so the panel "follows" wherever you navigate.
    active: usize,
    /// Agents, plain terminals and processes for every project, each tagged with its
    /// project index. Filtered by project + [`Kind`] to build each sidebar group.
    sessions: Vec<Session>,

    sel: usize, // index into build_nav()
    /// Per-project memory of the last selected nav row, so returning to a project
    /// (via `[`/`]` or clicking its box) restores where you were. `None` ⇒ no row
    /// selected there yet, so we land on the project's first row.
    last_proj_sel: Vec<Option<Nav>>,
    focus: Focus,
    pending_leader: bool,
    should_quit: bool,
    flash: Option<(String, Instant)>, // transient footer note (e.g. the reload result)
    last_inner: (u16, u16),           // last main-pane inner size (rows, cols)
    last_click: Option<(usize, Instant)>, // (nav idx, time) for double-click detection
    /// Monotonic start, used only to drive the working-spinner frame so every agent's
    /// spinner rotates in step regardless of per-frame redraw jitter.
    start: Instant,

    // View scratch, recomputed every frame.
    compact: bool,     // single-column (phone) mode
    regions: Regions,  // per-frame hit rects + sidebar row map
    drag: Option<Selection>, // in-progress mouse drag-to-copy selection
    /// A URL under the current left-press, opened on release if the press doesn't
    /// become a drag (a drag is a copy instead). Set in `on_left_down`, consumed in
    /// `on_left_up`. See [`input`](crate::app::input).
    pending_url: Option<String>,
    /// Edge auto-scroll direction for a held drag: `1` reveals older history, `-1`
    /// moves toward the present, `0` not at an edge. Applied each `tick` so the
    /// selection keeps extending while the cursor sits still at a pane edge.
    drag_scroll: i32,

    /// An active modal overlay (commit prompt / branch switcher / Ctrl+P file
    /// picker), drawn over the whole UI and eating all keys while open.
    overlay: Option<Overlay>,

    /// The git panel's diff preview: when set, it takes over the main pane (a
    /// read-only pager of the changed file under the Changes cursor) instead of the
    /// selected session. Set by clicking a file / `v`; follows the cursor; cleared
    /// when a session is selected (see [`git`] and [`App::diff_upkeep`]).
    diff: Option<DiffView>,

    /// Whether to draw an image diff as a real sixel picture (terminal supports it,
    /// detected once at startup) rather than the half-block fallback. See `run_loop`.
    sixel: bool,
    /// Terminal cell size in pixels `(w, h)`, needed to scale a sixel to a cell area.
    /// Best-effort from the terminal; a sane default when it doesn't report pixels.
    cell_px: (u16, u16),
    /// Set during a frame when an image preview should be painted as a sixel: the pane
    /// rect + the encoded bytes. Consumed just after the draw (like the notification
    /// escapes) and written on top of the frame. `last_sixel` remembers what's currently
    /// on screen so we only re-emit on a change — tmux keeps rendering the untouched
    /// cells, so a static picture costs nothing per frame.
    pending_sixel: Option<(Rect, String)>,
    last_sixel: Option<(Rect, String)>,

    // Notifications.
    in_tmux: bool,                          // wrap notification OSCs in tmux passthrough?
    last_notified: HashMap<String, Instant>, // per-session throttle, keyed by name

    // Background self-update (Homebrew + native-binary installs). Workers report
    // check/install results over `update_rx`, drained in `tick`; the badge reflects
    // `update`. `restart` is set when the user applies a staged update, unwinding the
    // loop into an in-place re-exec.
    update: UpdateState,
    update_tx: Sender<UpdateMsg>,
    update_rx: Receiver<UpdateMsg>,
    last_update_check: Instant,
    restart: bool,
    /// Last saved structural fingerprint of the restorable rows, so the restore
    /// state file is only rewritten when the set of agents/terminals changes.
    /// See [`persist`].
    restore_sig: Option<u64>,
}

/// One project in the workspace: its config plus the runtime state scoped to it
/// (per-agent instance counters and its native git panel).
struct Project {
    cfg: Config,
    counts: Vec<usize>, // per-agent-template instance counter
    term_count: usize,  // running total, for "Terminal #N" naming
    /// This project's git panel — present when the project dir is a git repo,
    /// `None` otherwise. Each linked project tracks its own repo.
    git: Option<GitPanel>,
}

impl Project {
    fn new(cfg: Config) -> Project {
        let dir = cfg.dir.clone();
        let counts = vec![0; cfg.agents.len()];
        let git =
            (cfg.git_panel_enabled() && crate::git::is_repo(&dir)).then(|| GitPanel::new(dir));
        Project { cfg, counts, term_count: 0, git }
    }
}

impl App {
    fn new(ws: Workspace) -> App {
        let projects: Vec<Project> = ws.projects.into_iter().map(Project::new).collect();

        // One flat session list across every project; each process becomes a row
        // tagged with its project. Note which autostart so we can spawn them below —
        // but only for the root project (`pi == 0`): autostarting linked projects'
        // processes isn't wanted, so they start stopped like any manual process.
        let mut sessions = Vec::new();
        let mut auto = Vec::new();
        for (pi, proj) in projects.iter().enumerate() {
            for p in &proj.cfg.processes {
                if p.autostart && pi == 0 {
                    auto.push(sessions.len());
                }
                let mut s = Session::new(
                    p.name.clone(),
                    Kind::Process,
                    Recipe::process(p, &proj.cfg.dir),
                    pi,
                );
                s.stop = p.stop.clone();
                sessions.push(s);
            }
        }

        let nproj = projects.len();
        let (update_tx, update_rx) = mpsc::channel();
        let mut app = App {
            projects,
            active: 0,
            sessions,
            sel: 0,
            last_proj_sel: vec![None; nproj],
            focus: Focus::Sidebar,
            pending_leader: false,
            should_quit: false,
            flash: None,
            last_inner: (24, 80),
            last_click: None,
            start: Instant::now(),
            compact: false,
            regions: Regions::default(),
            drag: None,
            pending_url: None,
            drag_scroll: 0,
            overlay: None,
            diff: None,
            sixel: detect_sixel(),
            cell_px: detect_cell_px(),
            pending_sixel: None,
            last_sixel: None,
            // Our stdout flows through the tmux jail, so notification OSCs need the
            // passthrough wrapper to reach the real terminal.
            in_tmux: std::env::var_os("TMUX").is_some(),
            last_notified: HashMap::new(),
            update: UpdateState::Idle,
            update_tx,
            update_rx,
            last_update_check: Instant::now(),
            restart: false,
            restore_sig: None,
        };

        // Surface any non-fatal workspace-load problems (missing linked dirs, etc.).
        if !ws.warnings.is_empty() {
            app.flash = Some((ws.warnings.join(" · "), Instant::now()));
        }

        // Kick off a background update check when allowed (brew install, not a dev build,
        // not opted out). The worker re-verifies it's brew-managed before doing anything.
        if update::permitted(app.root_cfg().auto_update_enabled()) {
            app.update = UpdateState::Checking;
            update::spawn_check(app.update_tx.clone());
        }

        let (rows, cols) = app.last_inner;
        for i in auto {
            app.sessions[i].spawn(rows, cols);
        }

        // Bring the previous agents/terminals back (Claude/Codex resumed). This runs
        // on every fresh start — after a quit, a crash, or a self-update restart — and
        // is a no-op when there's no saved state. It's safe to do unconditionally: the
        // tmux singleton means a *new* inner process only starts when there's no live
        // session to attach to, so there are never live panes to clobber.
        app.restore_sessions();
        app
    }

    /// The workspace-level config (the root project's) — drives the sidebar title
    /// and the notification settings.
    fn root_cfg(&self) -> &Config {
        &self.projects[0].cfg
    }

    /// Set the transient footer note (shown for a few seconds). One place so the
    /// `Instant::now()` stamping isn't open-coded at every call site.
    fn flash(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    /// Flash the first line of an active-panel op's result (Ok or Err alike). No-op when
    /// there's no active panel. Use for ops whose Ok payload is itself the message to show.
    /// Here (not in [`git`]) so the overlay handlers in [`overlay`] can reach it too — a
    /// sibling module can't see another's private items, but a descendant of `app` can.
    fn flash_result(&mut self, r: Option<Result<String, String>>) {
        if let Some(res) = r {
            let (Ok(s) | Err(s)) = res;
            self.flash(first_line(&s));
        }
    }

    /// Flash only on error (silent on success). For ops returning `Result<(), String>`.
    fn flash_err(&mut self, r: Option<Result<(), String>>) {
        if let Some(Err(e)) = r {
            self.flash(first_line(&e));
        }
    }

    /// Per-loop housekeeping. First follow the selection — the project of the
    /// selected row becomes active, so its git panel is the one shown. Then drain
    /// any finished background pull/push jobs (flashing the result) and give the
    /// visible panel a throttled refresh so external commits show up.
    pub(crate) fn tick(&mut self) {
        // Keep a held-at-edge drag selection auto-scrolling even when the mouse
        // isn't moving (crossterm only emits drag events on movement).
        self.step_drag_scroll();
        // Reap agents/terminals whose program has exited before anything reads the
        // selection, so a just-quit agent or editor row is gone this frame.
        self.prune_exited();
        if let Some(n) = self.current_nav() {
            if let Some(p) = self.project_of(n) {
                self.active = p;
                // Remember this row so returning to the project restores it.
                if let Some(slot) = self.last_proj_sel.get_mut(p) {
                    *slot = Some(n);
                }
            }
        }
        // Drain finished network jobs from every project's panel (even ones not
        // shown) so their channels can't back up; flash each outcome.
        let mut done: Vec<JobDone> = Vec::new();
        for proj in self.projects.iter_mut() {
            if let Some(g) = proj.git.as_mut() {
                done.extend(g.poll_jobs());
            }
        }
        for j in done {
            let msg = match j.result {
                Ok(s) => s,
                Err(e) => format!("{} failed — {}", j.verb, first_line(&e)),
            };
            self.flash(msg);
        }
        // Keep the visible panel fresh (throttled).
        let active = self.active;
        if let Some(g) = self.projects[active].git.as_mut() {
            g.maybe_refresh();
        }
        // Drop a stale diff preview, or refresh it so an agent's live edits show.
        self.diff_upkeep();
        // Advance the background self-update (drain workers, run the periodic re-check).
        self.poll_update();
        // Persist the live agents/terminals when they change, so a self-update
        // restart can bring them back (cheap no-op when nothing changed).
        self.maybe_save_state();
    }

    /// Drive the self-update state machine: drain finished check/install steps, auto-start
    /// the install for a found update, and re-check every few hours on a long-running session.
    fn poll_update(&mut self) {
        while let Ok(msg) = self.update_rx.try_recv() {
            match msg {
                // A newer version exists. A self-managed install stages it immediately in
                // the background (the badge appears once it's Ready). A brew install can't
                // be swapped underneath brew, so it parks in `Available` and waits for the
                // user to confirm the `brew upgrade` (see `apply_update`).
                UpdateMsg::Available { version, kind } => match kind {
                    InstallKind::SelfManaged => {
                        update::spawn_install(self.update_tx.clone(), version.clone(), kind);
                        self.update = UpdateState::Installing(version);
                    }
                    InstallKind::Brew => {
                        self.update = UpdateState::Available(version);
                    }
                },
                // Installed and ready: light the badge and announce it once via the flash.
                UpdateMsg::Installed(v) => {
                    self.flash(format!("update {v} ready — press U or click ↻ to restart"));
                    self.update = UpdateState::Ready(v);
                }
                // A check came back current, or a step failed (network/brew): fall back
                // to idle so the periodic timer retries. A failed *install* (we found an
                // update but couldn't apply it) gets one quiet flash; a failed *check*
                // (commonly just offline) stays silent. A staged update is left untouched.
                UpdateMsg::UpToDate => {
                    if matches!(self.update, UpdateState::Checking) {
                        self.update = UpdateState::Idle;
                    }
                }
                UpdateMsg::Failed(reason) => {
                    if matches!(self.update, UpdateState::Installing(_)) {
                        self.flash(format!("update failed — {reason}; will retry later"));
                    }
                    if !matches!(self.update, UpdateState::Ready(_)) {
                        self.update = UpdateState::Idle;
                    }
                }
                // Not a managed install: settle into the terminal `Unsupported` state so the
                // About card reads "self-update off" instead of an eternal "checking…".
                // Silent — no flash, no badge — since there's nothing the user can do.
                UpdateMsg::NotManaged => {
                    if matches!(self.update, UpdateState::Checking) {
                        self.update = UpdateState::Unsupported;
                    }
                }
            }
        }
        // Periodic re-check, only while idle (don't disturb an in-flight or staged update).
        if matches!(self.update, UpdateState::Idle)
            && self.last_update_check.elapsed() >= update::CHECK_EVERY
            && update::permitted(self.root_cfg().auto_update_enabled())
        {
            self.last_update_check = Instant::now();
            self.update = UpdateState::Checking;
            update::spawn_check(self.update_tx.clone());
        }
    }

    /// Act on the current update, if any. A staged update ([`UpdateState::Ready`]) restarts
    /// in place onto the new binary (sets the flag the event loop unwinds on to re-exec —
    /// which necessarily ends the live panes). A brew update pending confirmation
    /// ([`UpdateState::Available`]) opens the confirm that runs `brew upgrade mmux`.
    /// Harmless in any other state. Shared by the About card `u`, the sidebar `U`, and the
    /// footer badge click.
    pub(crate) fn apply_update(&mut self) {
        match &self.update {
            UpdateState::Ready(_) => self.restart = true,
            UpdateState::Available(v) => {
                let v = v.clone();
                self.overlay = Some(overlay::Overlay::confirm(
                    "Update mmux",
                    format!("Update to v{v}? This runs `brew upgrade mmux` for you."),
                    "y update · n cancel",
                    overlay::Confirmed::BrewUpgrade { version: v },
                ));
            }
            _ => {}
        }
    }

    /// Run the confirmed `brew upgrade mmux` in the background (from the `Available` confirm
    /// in [`apply_update`](Self::apply_update)). Guarded on the state so a stale confirm is a
    /// no-op; the result flows back through [`poll_update`](Self::poll_update) to `Ready`.
    pub(crate) fn start_brew_upgrade(&mut self, version: String) {
        if matches!(self.update, UpdateState::Available(_)) {
            update::spawn_install(self.update_tx.clone(), version.clone(), InstallKind::Brew);
            self.update = UpdateState::Installing(version);
        }
    }

    /// Kick a fresh update check on demand (the About popup's `c` key). Mirrors the
    /// periodic re-check's gate exactly — only starts one while idle and permitted, so a
    /// check/install already in flight or a staged update is left alone. The result flows
    /// back through [`poll_update`](Self::poll_update) and the popup reflects it live.
    pub(crate) fn check_now(&mut self) {
        if matches!(self.update, UpdateState::Idle)
            && update::permitted(self.root_cfg().auto_update_enabled())
        {
            self.last_update_check = Instant::now();
            self.update = UpdateState::Checking;
            update::spawn_check(self.update_tx.clone());
        }
    }

    /// Whether self-update is even on the table for this build — the cheap, synchronous
    /// gate the About popup uses to decide whether to offer the check. This is the config
    /// / dev-build / opt-out test only; the install-kind check is the worker's job, so a
    /// permitted but unmanaged build still reads as updatable here until a check says otherwise.
    pub(crate) fn can_self_update(&self) -> bool {
        update::permitted(self.root_cfg().auto_update_enabled())
    }

    /// Open the "About mmux" card (the `?` key / footer chip). Stateless overlay; its
    /// content is read live from `self.update` when drawn.
    pub(crate) fn open_about(&mut self) {
        self.overlay = Some(overlay::Overlay::About);
    }

    /// Drain every pane's captured notifications and return the escape bytes to
    /// write to the controlling terminal this frame (empty when there's nothing to
    /// send or notifications are off). Runs each loop, right after the draw.
    /// Side effect: external-command notifications are fired here directly.
    pub(crate) fn collect_notifications(&mut self) -> Vec<u8> {
        // Always drain, even when disabled or focused, so the per-pane queues can't
        // grow without bound.
        let mut drained: Vec<(usize, Vec<Notify>)> = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            let n = s.take_notifications();
            if !n.is_empty() {
                drained.push((i, n));
            }
        }
        let cfg = self.root_cfg().notifications.clone().unwrap_or_default();
        if !cfg.enabled {
            return Vec::new();
        }

        let focused = self.focused_session_index();
        let mut out = Vec::new();
        for (i, notes) in drained {
            if cfg.only_when_unfocused && Some(i) == focused {
                continue;
            }
            let name = self.sessions[i].name.clone();
            self.emit_notification(&name, &notes, &cfg, &mut out);
        }
        out
    }

    /// Emit one throttled notification for `name` from a batch of captured events,
    /// appending escape bytes to `out` (or firing an external command in-place).
    fn emit_notification(&mut self, name: &str, notes: &[Notify], cfg: &NotifyConfig, out: &mut Vec<u8>) {
        let throttled = self
            .last_notified
            .get(name)
            .map(|t| t.elapsed() < Duration::from_secs(cfg.throttle_secs))
            .unwrap_or(false);
        if throttled {
            return;
        }
        let note = build_note(name, notes);
        match cfg.mechanism {
            NotifyMechanism::Command => notify::run_command(cfg.command.as_deref(), &note),
            mech => {
                if let Some(bytes) = note.escape_bytes(mech, self.in_tmux) {
                    out.extend_from_slice(&bytes);
                }
            }
        }
        self.last_notified.insert(name.to_string(), Instant::now());
    }

    /// The `sessions` index that currently holds keyboard focus, if any — used to
    /// suppress notifications for the pane you're already looking at.
    fn focused_session_index(&self) -> Option<usize> {
        if self.focus != Focus::Terminal {
            return None;
        }
        match self.current_nav()? {
            Nav::Session(i) => Some(i),
            _ => None,
        }
    }

    fn send_focused(&self, bytes: Vec<u8>) {
        if let Some(p) = self.focused_pane() {
            p.send(bytes);
        }
    }

    fn clear_focused_attention(&self) {
        if let Some(p) = self.focused_pane() {
            p.clear_attention();
        }
    }

    fn resize_current(&mut self, rows: u16, cols: u16) {
        self.last_inner = (rows, cols);
        if let Some(nav) = self.current_nav() {
            if let Some(p) = self.pane_at_mut(nav) {
                p.resize(rows, cols);
            }
        }
    }
}

/// Whether to render image diffs as real sixel pictures. `MMUX_SIXEL=0/1` forces it
/// off/on; otherwise we ask tmux whether the outer terminal supports sixel.
fn detect_sixel() -> bool {
    match std::env::var("MMUX_SIXEL").ok().as_deref() {
        Some("0") | Some("off") | Some("false") | Some("no") => false,
        Some("1") | Some("on") | Some("true") | Some("yes") => true,
        _ => crate::tmux::client_supports_sixel(),
    }
}

/// The terminal's pixel-per-cell size, for scaling a sixel to a cell area. Derived from
/// the reported window pixel size ÷ its cell grid; a common 10×20 fallback when the
/// terminal (or tmux) doesn't report pixel dimensions.
fn detect_cell_px() -> (u16, u16) {
    match ratatui::crossterm::terminal::window_size() {
        Ok(ws) if ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0 => {
            ((ws.width / ws.columns).max(1), (ws.height / ws.rows).max(1))
        }
        _ => (10, 20),
    }
}

pub fn run(ws: Workspace) -> Result<()> {
    let mut app = App::new(ws);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    // Best-effort: ask the terminal to disambiguate escape codes (the kitty keyboard
    // protocol) so distinct chords — notably `Ctrl+⏎` in the commit prompt — arrive with
    // their modifier instead of collapsing to a bare ⏎. Only the mildest flag: no
    // release/repeat events, so inner-pane key forwarding (`encode_key`) is unchanged.
    // A terminal that doesn't support it ignores the CSI (and, over the tmux jail, so
    // does a client without extended-keys passthrough — which we can't enable per-session,
    // it's a server option) — there `Ctrl+⏎` just falls back to a plain ⏎, so the commit
    // prompt also takes `Ctrl+P` (a plain control byte) as an always-reliable commit-&-push.
    // Popped on teardown.
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let res = run_loop(&mut terminal, &mut app);

    // Final snapshot while the panes are still alive, so the next open — after this
    // quit, or the update restart below — restores them with each one's freshest cwd.
    if res.is_ok() {
        app.save_state();
    }

    disable_raw_mode()?;
    // Undo the keyboard-protocol request (best-effort — a no-op where it was ignored).
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    // On a real quit (never a self-update restart, where the processes come straight
    // back), run any process `stop:` teardown commands and wait for them — so something
    // like `docker compose down` finishes before mmux, and its tmux session, go away.
    // The panes are already ending; this is the last thing before the process exits.
    if app.should_quit {
        app.run_stop_commands_on_quit();
    }
    res?;

    // The user applied a staged self-update: with the terminal restored, re-exec the
    // freshly-installed binary in place (same tmux pane). On success this never returns.
    if app.restart {
        let e = update::exec_restart();
        eprintln!("mmux: could not restart into the update ({e}); reopen with `mmux`.");
    }
    Ok(())
}

/// Fold a batch of captured events for one session into a single notification,
/// preferring the most recent one that carries real text over a bare bell.
fn build_note(name: &str, notes: &[Notify]) -> Note {
    let rich = notes
        .iter()
        .rev()
        .find(|n| n.title.is_some() || n.body.is_some());
    match rich {
        Some(n) => Note {
            title: match &n.title {
                Some(t) if !t.is_empty() => format!("{name} — {t}"),
                _ => name.to_string(),
            },
            body: n.body.clone().unwrap_or_else(|| "needs attention".into()),
        },
        None => Note {
            title: name.to_string(),
            body: "needs attention".into(),
        },
    }
}

/// Paint the pending image-diff sixel on top of the frame ratatui just drew (the same
/// after-draw escape channel the notifications use). tmux renders the sixel natively and
/// diffs its own screen, so we only actually emit when the picture or its placement
/// *changed* — a static preview then sits there for free. Suppressed while a modal is
/// open (it would cover it); when the picture goes away we force one full repaint so tmux
/// clears the leftover pixels (a plain cell diff wouldn't touch them).
fn emit_sixel(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    let pending = app.pending_sixel.take().filter(|_| app.overlay.is_none());
    match &pending {
        Some((rect, data)) => {
            // Re-emit only when the picture or its placement changed — an unchanged
            // sixel is still on tmux's screen, so a static preview costs nothing.
            let changed = match &app.last_sixel {
                Some((r, d)) => r != rect || d != data,
                None => true,
            };
            if changed {
                let mut out = stdout();
                write!(out, "\x1b7")?; // save cursor
                // Wipe the previous picture's cells first, so switching to a
                // smaller/differently-shaped image leaves no leftover pixels around it.
                // Both frames are image-mode (the pane buffer there is blank), so erasing
                // to spaces stays consistent with what ratatui believes it drew.
                if let Some((old, _)) = &app.last_sixel {
                    erase_rect(&mut out, *old)?;
                }
                // Jump to the pane's top-left (1-based), draw, restore the cursor.
                write!(out, "\x1b[{};{}H{}\x1b8", rect.y + 1, rect.x + 1, data)?;
                out.flush()?;
                app.last_sixel = Some((*rect, data.clone()));
            }
        }
        None => {
            // Leaving the picture for normal cell content (or a modal opened over it):
            // force one full repaint so tmux overwrites the sixel pixels a plain cell
            // diff wouldn't touch, and redraw at once so no blank frame shows.
            if app.last_sixel.take().is_some() {
                terminal.clear()?;
                terminal.draw(|f| app.draw(f))?;
            }
        }
    }
    Ok(())
}

/// Blank every cell of `r` with cursor-positioned spaces (caller brackets this with
/// save/restore-cursor). Used to wipe a previous sixel before drawing a new one.
fn erase_rect(out: &mut Stdout, r: Rect) -> std::io::Result<()> {
    let blanks = " ".repeat(r.width as usize);
    for y in r.y..r.y.saturating_add(r.height) {
        write!(out, "\x1b[{};{}H{}", y + 1, r.x + 1, blanks)?;
    }
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| app.draw(f))?;
        // Notification escapes are non-painting, so it's safe to write them straight
        // to stdout just after the frame ratatui drew.
        let notes = app.collect_notifications();
        if !notes.is_empty() {
            let mut out = stdout();
            out.write_all(&notes)?;
            out.flush()?;
        }
        emit_sixel(terminal, app)?;
        if app.should_quit || app.restart {
            break;
        }
        // Poll with a timeout so live process output redraws even without input.
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                Event::Paste(s) => app.on_paste(s),
                _ => {}
            }
        }
        app.tick();
    }
    Ok(())
}
