//! The interactive TUI. This module owns the [`App`] state and the event loop;
//! the behaviour is split across focused submodules:
//!
//! - [`session`] — the unified pane-backed [`Session`] model + its lifecycle.
//! - [`nav`] — the sidebar navigation list and the selection cursor.
//! - [`lifecycle`] — spawn/start/stop/restart actions and the live config reload.
//! - [`input`] — keyboard, mouse and paste handling.
//! - [`keymap`] — pure key-event → PTY-byte translation.
//! - [`view`] — all rendering (layout, sidebar, panes, footer).

mod git;
mod input;
mod keymap;
mod lifecycle;
mod nav;
mod picker;
mod session;
mod view;

pub(crate) use session::{Kind, Recipe, Session, Status};
use git::{first_line, GitPanel, JobDone, Overlay};
use input::Selection;
use nav::Nav;
use view::Regions;

use crate::config::{Config, NotifyConfig, NotifyMechanism, Workspace};
use crate::notify::{self, Note};
use crate::pane::Notify;
use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::Terminal;
use std::collections::HashMap;
use std::io::{stdout, Stdout, Write};
use std::time::{Duration, Instant};

/// Which region currently receives keyboard input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Sidebar,
    Terminal,
    Right,
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

    // View scratch, recomputed every frame.
    compact: bool,     // single-column (phone) mode
    regions: Regions,  // per-frame hit rects + sidebar row map
    drag: Option<Selection>, // in-progress mouse drag-to-copy selection

    /// An active modal overlay (commit prompt / branch switcher / Ctrl+P file
    /// picker), drawn over the whole UI and eating all keys while open.
    overlay: Option<Overlay>,

    // Notifications.
    in_tmux: bool,                          // wrap notification OSCs in tmux passthrough?
    last_notified: HashMap<String, Instant>, // per-session throttle, keyed by name
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
                sessions.push(Session::new(
                    p.name.clone(),
                    Kind::Process,
                    Recipe::process(p, &proj.cfg.dir),
                    pi,
                ));
            }
        }

        let nproj = projects.len();
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
            compact: false,
            regions: Regions::default(),
            drag: None,
            overlay: None,
            // Our stdout flows through the tmux jail, so notification OSCs need the
            // passthrough wrapper to reach the real terminal.
            in_tmux: std::env::var_os("TMUX").is_some(),
            last_notified: HashMap::new(),
        };

        // Surface any non-fatal workspace-load problems (missing linked dirs, etc.).
        if !ws.warnings.is_empty() {
            app.flash = Some((ws.warnings.join(" · "), Instant::now()));
        }

        let (rows, cols) = app.last_inner;
        for i in auto {
            app.sessions[i].spawn(rows, cols);
        }
        app
    }

    /// The workspace-level config (the root project's) — drives the sidebar title
    /// and the notification settings.
    fn root_cfg(&self) -> &Config {
        &self.projects[0].cfg
    }

    /// Per-loop housekeeping. First follow the selection — the project of the
    /// selected row becomes active, so its git panel is the one shown. Then drain
    /// any finished background pull/push jobs (flashing the result) and give the
    /// visible panel a throttled refresh so external commits show up.
    pub(crate) fn tick(&mut self) {
        // Reap finished throwaway panes (the Ctrl+P editor) before anything reads the
        // selection, so a just-quit editor row is gone this frame.
        self.prune_ephemeral();
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
            self.flash = Some((msg, Instant::now()));
        }
        // Keep the visible panel fresh (throttled).
        let active = self.active;
        if let Some(g) = self.projects[active].git.as_mut() {
            g.maybe_refresh();
        }
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

pub fn run(ws: Workspace) -> Result<()> {
    let mut app = App::new(ws);

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let res = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    res
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
        if app.should_quit {
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
