//! A single PTY-backed pane: spawns a command in a pseudo-terminal, parses its
//! output with vt100 (tracking the OSC title and the bell for the sidebar), and
//! exposes interactive input + resize.

use anyhow::Result;
use bytes::Bytes;
use portable_pty::{
    native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtyPair, PtySize,
};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tui_term::vt100;

/// A notification captured from a pane's output: either a bare bell (no text) or
/// a desktop-notification OSC the inner program emitted itself. The app layer adds
/// the session name and decides whether/how to surface it (see `app::App`).
#[derive(Clone, Default)]
pub struct Notify {
    pub title: Option<String>,
    pub body: Option<String>,
}

/// vt100 callbacks that capture the bits we surface (sidebar bell + notifications).
///
/// In vt100 0.16 the OSC title, bell, and unhandled OSCs are delivered through this
/// trait rather than via methods on `Screen`, so we record them here and read them
/// back via `Parser::callbacks()`.
#[derive(Default)]
pub struct PaneEvents {
    pub title: String,
    /// Latched on bell; cleared when the user views/interacts with the pane.
    pub bell: bool,
    /// Notifications captured since the last drain — one per bell ring or
    /// notification OSC. Drained by the app each loop via `Pane::take_notifications`.
    pub notifications: Vec<Notify>,
}

impl vt100::Callbacks for PaneEvents {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).trim().to_string();
    }
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
        self.notifications.push(Notify::default());
    }
    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.bell = true;
        self.notifications.push(Notify::default());
    }
    /// Desktop-notification OSCs vt100 doesn't handle itself: OSC 9 (iTerm2-style),
    /// OSC 777 (`notify;title;body`), and a best-effort OSC 99 (kitty). `params` is
    /// the OSC body already split on `;`.
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
        match params.first().copied() {
            // OSC 9 ; <message>. Only the single-message form — `OSC 9 ; 4 ; …` is
            // ConEmu progress reporting, not a notification.
            Some(b"9") if params.len() == 2 => {
                let body = text(params[1]);
                if !body.is_empty() {
                    self.notifications.push(Notify { title: None, body: Some(body) });
                }
            }
            // OSC 777 ; notify ; <title> ; <body>
            Some(b"777") if params.len() >= 3 && params[1] == b"notify" => {
                let title = Some(text(params[2])).filter(|t| !t.is_empty());
                let body = params.get(3).map(|b| text(b)).filter(|b| !b.is_empty());
                if title.is_some() || body.is_some() {
                    self.notifications.push(Notify { title, body });
                }
            }
            // OSC 99 (kitty) — best-effort: surface the trailing payload as the body.
            Some(b"99") if params.len() >= 2 => {
                if let Some(body) = params.last().map(|b| text(b)).filter(|b| !b.is_empty()) {
                    self.notifications.push(Notify { title: None, body: Some(body) });
                }
            }
            _ => {}
        }
    }
}

pub type SharedParser = Arc<Mutex<vt100::Parser<PaneEvents>>>;

pub struct Pane {
    parser: SharedParser,
    tx: Sender<Bytes>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    running: Arc<AtomicBool>,
    size: (u16, u16),
}

impl Pane {
    pub fn spawn(
        cmd: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
        rows: u16,
        cols: u16,
    ) -> Result<Pane> {
        let rows = rows.max(1);
        let cols = cols.max(1);

        let pty = native_pty_system();
        let PtyPair { master, slave } = pty.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut builder = CommandBuilder::new(cmd);
        for a in args {
            builder.arg(a);
        }
        builder.cwd(cwd);
        // Advertise a capable terminal so full-color TUIs (claude/codex) behave.
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        for (k, v) in env {
            builder.env(k, v);
        }

        let mut child = slave.spawn_command(builder)?;
        drop(slave); // close our handle to the slave in the parent
        let killer = child.clone_killer();

        let parser: SharedParser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            rows,
            cols,
            5000, // scrollback
            PaneEvents::default(),
        )));
        let running = Arc::new(AtomicBool::new(true));

        // Reader thread: blocking reads from the PTY feed the vt100 parser.
        {
            let mut reader = master.try_clone_reader()?;
            let parser = parser.clone();
            let running = running.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = parser.lock() {
                                p.process(&buf[..n]);
                            }
                        }
                    }
                }
                running.store(false, Ordering::SeqCst);
            });
        }

        // Writer thread: owns the single PTY writer, drains the input channel.
        let (tx, rx) = mpsc::channel::<Bytes>();
        {
            let mut writer = master.take_writer()?;
            thread::spawn(move || {
                while let Ok(bytes) = rx.recv() {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        }

        // Reaper: wait on the child so it doesn't linger as a zombie.
        thread::spawn(move || {
            let _ = child.wait();
        });

        Ok(Pane {
            parser,
            tx,
            master,
            killer,
            running,
            size: (rows, cols),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Bytes::from(bytes));
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        // Resize both the PTY (sends SIGWINCH so the child reflows) and the parser.
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_size(rows, cols);
        }
    }

    pub fn title(&self) -> String {
        self.parser
            .lock()
            .ok()
            .map(|p| p.callbacks().title.clone())
            .unwrap_or_default()
    }

    pub fn attention(&self) -> bool {
        self.parser
            .lock()
            .ok()
            .map(|p| p.callbacks().bell)
            .unwrap_or(false)
    }

    pub fn clear_attention(&self) {
        if let Ok(mut p) = self.parser.lock() {
            p.callbacks_mut().bell = false;
        }
    }

    /// Drain the notifications captured since the last call (bell rings + the
    /// program's own notification OSCs). The app turns these into desktop popups.
    pub fn take_notifications(&self) -> Vec<Notify> {
        self.parser
            .lock()
            .ok()
            .map(|mut p| std::mem::take(&mut p.callbacks_mut().notifications))
            .unwrap_or_default()
    }

    /// Scroll the view into scrollback. `delta > 0` reveals older lines;
    /// `delta < 0` moves back toward the live present. Clamped to [0, len].
    pub fn scroll(&self, delta: i32) {
        if let Ok(mut p) = self.parser.lock() {
            let screen = p.screen_mut();
            let next = (screen.scrollback() as i32 + delta).max(0) as usize;
            screen.set_scrollback(next);
        }
    }

    /// Current scrollback offset (0 = live view, larger = further into history).
    /// Selection coordinates are anchored relative to this, so they stay on the
    /// same content as the view scrolls.
    pub fn scrollback_offset(&self) -> usize {
        self.parser
            .lock()
            .ok()
            .map(|p| p.screen().scrollback())
            .unwrap_or(0)
    }

    /// Snap back to the live view (scrollback offset 0).
    pub fn reset_scroll(&self) {
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_scrollback(0);
        }
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    /// Run `f` with the current vt100 screen under lock. Returns `None` if the
    /// lock is poisoned.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> Option<R> {
        self.parser.lock().ok().map(|p| f(p.screen()))
    }

    /// Plain-text contents of a flow selection spanning buffer lines `lo..=hi`,
    /// stitched across scrollback. A *line* here is `viewport_row - scrollback_offset`
    /// (so it names a fixed buffer line regardless of the current scroll position;
    /// negative values are lines that have scrolled up into history). The selection
    /// runs from `(lo, sc)` through `(hi, ec)` in reading order: the first line from
    /// `sc` to its end, whole lines in between, then the last line up to (but not
    /// including) `ec`.
    ///
    /// vt100 only exposes the visible window, so we walk the offset to bring each
    /// line into view, read it, and restore the offset before returning.
    pub fn contents_block(&self, lo: i32, hi: i32, sc: u16, ec: u16) -> Option<String> {
        if hi < lo {
            return Some(String::new());
        }
        let mut p = self.parser.lock().ok()?;
        let saved = p.screen().scrollback();
        let (_, cols) = p.screen().size();
        let ec = ec.min(cols);
        let mut out = String::new();
        for line in lo..=hi {
            // Pick the offset that lands this line on a visible row: history lines
            // (line < 0) sit at row 0 under offset `-line`; live lines (line >= 0)
            // are already on screen at offset 0, row `line`.
            let off = (-line).max(0) as usize;
            p.screen_mut().set_scrollback(off);
            let screen = p.screen();
            let row = (line + off as i32).max(0) as u16;
            let (c0, c1) = if lo == hi {
                (sc, ec)
            } else if line == lo {
                (sc, cols)
            } else if line == hi {
                (0, ec)
            } else {
                (0, cols)
            };
            if c1 > c0 {
                if let Some(text) = screen.rows(c0, c1 - c0).nth(row as usize) {
                    out.push_str(&text);
                }
            }
            // Join with newlines, but keep a soft-wrapped logical line on one line
            // (matching vt100's own `contents_between`).
            if line != hi && !screen.row_wrapped(row) {
                out.push('\n');
            }
        }
        p.screen_mut().set_scrollback(saved);
        Some(out)
    }
}
