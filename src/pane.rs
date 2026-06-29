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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
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
    /// When `title` last changed — but *not counting* its first set, which is just
    /// startup. Agents animate their terminal title (a spinner / moving glyph) while
    /// busy and leave it static when idle, so the sidebar treats a running agent whose
    /// title has gone quiet as "needs you". `None` until the title changes a second
    /// time, so a freshly launched agent reads as idle rather than briefly working.
    pub title_changed_at: Option<Instant>,
    /// Latched on bell; cleared when the user views/interacts with the pane.
    pub bell: bool,
    /// Notifications captured since the last drain — one per bell ring or
    /// notification OSC. Drained by the app each loop via `Pane::take_notifications`.
    pub notifications: Vec<Notify>,
}

impl vt100::Callbacks for PaneEvents {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let title = String::from_utf8_lossy(title).trim().to_string();
        if title != self.title {
            // Mark "activity" only on a real change *after* the first title. The
            // initial empty→something set is just the program starting up, not it
            // working — counting it would spin a freshly launched agent for ~2s. And
            // an idle agent that keeps re-emitting the *same* static title never gets
            // here, so it correctly stays quiet.
            if !self.title.is_empty() {
                self.title_changed_at = Some(Instant::now());
            }
            self.title = title;
        }
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
    /// Set the moment *we* kill the child (stop/restart), so its non-zero exit
    /// reads as a deliberate stop rather than a crash. See [`Pane::crashed`].
    killed: Arc<AtomicBool>,
    /// The child's exit code once it has been reaped — `None` while it's still
    /// running (or briefly, just after exit, before `wait()` returns).
    exit_code: Arc<Mutex<Option<u32>>>,
    /// The child process id, used to read its live working directory so a
    /// terminal's `cd` can survive a restart (see [`Pane::cwd`]).
    pid: Option<u32>,
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
        let pid = child.process_id();

        let parser: SharedParser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            rows,
            cols,
            5000, // scrollback
            PaneEvents::default(),
        )));
        let running = Arc::new(AtomicBool::new(true));
        let killed = Arc::new(AtomicBool::new(false));
        let exit_code: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));

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

        // Reaper: wait on the child so it doesn't linger as a zombie, and record
        // its exit code so the sidebar can tell a crash from a clean finish.
        {
            let exit_code = exit_code.clone();
            thread::spawn(move || {
                if let Ok(status) = child.wait() {
                    if let Ok(mut slot) = exit_code.lock() {
                        *slot = Some(status.exit_code());
                    }
                }
            });
        }

        Ok(Pane {
            parser,
            tx,
            master,
            killer,
            running,
            killed,
            exit_code,
            pid,
            size: (rows, cols),
        })
    }

    /// The child's current working directory, read live from the OS — so a
    /// terminal that's been `cd`'d can be respawned where the user left it across
    /// a restart. `None` if we don't have the pid or can't read it (the caller
    /// then falls back to the recipe's launch dir). See [`process_cwd`].
    pub fn cwd(&self) -> Option<PathBuf> {
        process_cwd(self.pid?)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Whether the program exited abnormally *on its own* — a non-zero exit status
    /// that we didn't cause by killing it. Drives the red "crashed" badge; a clean
    /// exit (status 0) or a deliberate stop both read as plain "not running".
    pub fn crashed(&self) -> bool {
        if self.is_running() || self.killed.load(Ordering::SeqCst) {
            return false;
        }
        self.exit_code
            .lock()
            .ok()
            .and_then(|c| *c)
            .is_some_and(|code| code != 0)
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

    /// Whether the program's terminal title changed within `within`. Agents animate
    /// their title while working and leave it static when idle, so a *false* here on
    /// a running agent reads as "it's gone quiet — it's waiting on you".
    pub fn title_active(&self, within: Duration) -> bool {
        self.parser
            .lock()
            .ok()
            .and_then(|p| p.callbacks().title_changed_at)
            .is_some_and(|t| t.elapsed() < within)
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

    /// Translate one wheel notch over this pane into input *for the program*,
    /// or `None` to mean "scroll our own scrollback instead".
    ///
    /// The alternate screen (nano, micro, less, vim, …) has no scrollback for
    /// us to reveal, so a wheel that only nudged our offset would do nothing.
    /// Mirror what a real terminal does there: if the program tracks the mouse,
    /// forward a wheel event in the encoding it negotiated; otherwise synthesize
    /// `lines` arrow keys ("alternate scroll" — what lets less/nano/man scroll),
    /// honoring application-cursor-key mode. On the normal screen we return
    /// `None` and the caller keeps driving our scrollback buffer.
    ///
    /// `up` is the wheel direction; `col`/`row` are absolute screen cells and
    /// `ox`/`oy` the pane's content-area origin, used to place a forwarded
    /// mouse event in the program's own (1-based) coordinate space.
    pub fn wheel_input(&self, up: bool, lines: u16, col: u16, row: u16, ox: u16, oy: u16) -> Option<Vec<u8>> {
        self.with_screen(|s| {
            if !s.alternate_screen() {
                return None;
            }
            let bytes = if s.mouse_protocol_mode() == vt100::MouseProtocolMode::None {
                let seq: &[u8] = match (up, s.application_cursor()) {
                    (true, false) => b"\x1b[A",
                    (false, false) => b"\x1b[B",
                    (true, true) => b"\x1bOA",
                    (false, true) => b"\x1bOB",
                };
                seq.repeat(lines as usize)
            } else {
                // Wheel-up is xterm button 64, wheel-down 65; coords are 1-based.
                let btn = if up { 64 } else { 65 };
                let x = col.saturating_sub(ox) + 1;
                let y = row.saturating_sub(oy) + 1;
                mouse_wheel_seq(btn, x, y, s.mouse_protocol_encoding())
            };
            Some(bytes)
        })
        .flatten()
    }

    pub fn kill(&mut self) {
        // Mark this as a deliberate stop first, so the resulting non-zero exit
        // isn't mistaken for a crash (see [`Pane::crashed`]).
        self.killed.store(true, Ordering::SeqCst);
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

/// Read process `pid`'s current working directory from the OS.
///
/// `cd` lives in the kernel's per-process state, so we can recover it without the
/// process's cooperation: Linux exposes it as the `/proc/<pid>/cwd` symlink;
/// macOS has no `/proc`, so we ask the kernel via `proc_pidinfo`. Either way the
/// result is sanity-checked with `is_dir()` before use, so a wrong read degrades
/// to "no cwd" (caller falls back to the launch dir) rather than a bad path.
#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    let p = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    p.is_dir().then_some(p)
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    // `proc_pidinfo(PROC_PIDVNODEPATHINFO)` fills a `proc_vnodepathinfo`, whose
    // first member `pvi_cdir` is the current-directory vnode. Its NUL-terminated
    // path string sits right after the fixed-size `vnode_info` header (152 bytes)
    // within each `vnode_info_path`. `proc_pidinfo` is in libproc, part of
    // libSystem, so it links without an explicit crate.
    const PROC_PIDVNODEPATHINFO: i32 = 9;
    const VNODE_INFO_SIZE: usize = 152; // sizeof(struct vnode_info)
    const MAXPATHLEN: usize = 1024;
    const BUF_SIZE: usize = 2 * (VNODE_INFO_SIZE + MAXPATHLEN); // sizeof(proc_vnodepathinfo)
    extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            buffersize: i32,
        ) -> i32;
    }
    let mut buf = [0u8; BUF_SIZE];
    let n = unsafe {
        proc_pidinfo(
            pid as i32,
            PROC_PIDVNODEPATHINFO,
            0,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            BUF_SIZE as i32,
        )
    };
    if n < (VNODE_INFO_SIZE + 1) as i32 {
        return None;
    }
    let path = &buf[VNODE_INFO_SIZE..];
    let end = path.iter().position(|&b| b == 0).unwrap_or(0);
    if end == 0 {
        return None;
    }
    let p = PathBuf::from(std::str::from_utf8(&path[..end]).ok()?);
    p.is_dir().then_some(p)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

/// Encode one mouse button press for a program tracking the mouse, in the
/// encoding it negotiated. `btn` is the xterm button code (wheel bit included);
/// `x`/`y` are 1-based cells. Wheel events are press-only (no release).
fn mouse_wheel_seq(btn: u8, x: u16, y: u16, enc: vt100::MouseProtocolEncoding) -> Vec<u8> {
    match enc {
        vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{btn};{x};{y}M").into_bytes(),
        // Legacy single-byte encodings carry value + 32; clamp to their range.
        _ => vec![
            0x1b,
            b'[',
            b'M',
            32u8.saturating_add(btn),
            (x + 32).min(255) as u8,
            (y + 32).min(255) as u8,
        ],
    }
}
