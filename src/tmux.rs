//! All tmux interaction, in one place:
//!
//! - [`launch`] — the `mmux` (no-arg) entry point: an attach-or-create wrapper that
//!   runs the actual TUI inside an invisible, per-directory tmux session so it
//!   survives disconnects and is a singleton per directory.
//! - [`attach_picker`] — the `mmux attach` session picker.
//! - [`detach`] — detach the current client (the TUI asks for this via `d`).

use anyhow::{Context, Result};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn launch() -> Result<()> {
    launch_in(std::env::current_dir()?)
}

/// `launch`, but for an explicit directory. Used by the no-arg entry point (with the
/// current dir) and by the attach picker when opening a *recent* directory that has no
/// running session yet — in both cases it attaches-or-creates that directory's session.
pub fn launch_in(dir: PathBuf) -> Result<()> {
    if crate::config::config_path(&dir).is_none() && crate::config::global_config_path().is_none() {
        // Nothing set up here yet — treat it as a first run and walk the user
        // through setup instead of erroring out.
        crate::wizard::run(&dir)?;
        // If they declined to write anything, there's still nothing to launch.
        if crate::config::config_path(&dir).is_none() && crate::config::global_config_path().is_none() {
            return Ok(());
        }
    }

    if which_tmux().is_none() {
        eprintln!("tmux not found on PATH. mmux uses tmux to keep sessions alive — please install it.");
        std::process::exit(1);
    }

    // Canonicalize so `dir`, `dir/`, and symlinks all map to the same session.
    let canon = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
    record_recent(&canon);
    let name = session_name(&canon);
    let exe = std::env::current_exe().context("locating mmux binary")?;
    let exe = exe.to_string_lossy().into_owned();

    if !session_exists(&name) {
        let (cols, rows) = ratatui::crossterm::terminal::size().unwrap_or((120, 40));
        let dir_str = canon.to_string_lossy().into_owned();
        let status = Command::new("tmux")
            .args([
                "new-session",
                "-d",
                "-s",
                &name,
                "-x",
                &cols.to_string(),
                "-y",
                &rows.to_string(),
                "-c",
                &dir_str,
                "-e",
                "MMUX_INNER=1",
                "-e",
                &format!("MMUX_DIR={dir_str}"),
                "--",
                &exe,
                "--inner",
            ])
            .status()
            .context("starting tmux session")?;
        // If creation failed it's almost always a lost race with another `mmux`
        // that created the session first — fall through to attach in that case.
        if status.success() {
            // The project name (configured `name:`, else the directory basename) becomes
            // the outer terminal's tab title via tmux's set-titles, below.
            let title = crate::config::Config::load(&dir)
                .map(|c| c.display_name())
                .unwrap_or_else(|_| {
                    canon
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "mmux".into())
                });
            configure_session(&name, &title);
        }
    }

    // Attach. `env_remove("TMUX")` lets this work even when already inside tmux.
    let status = Command::new("tmux")
        .env_remove("TMUX")
        .args(["attach-session", "-t", &name])
        .status()
        .context("attaching tmux session")?;
    std::process::exit(status.code().unwrap_or(0));
}

/// Detach the attached tmux client; the session (and the inner TUI) keep running.
pub fn detach() {
    let _ = Command::new("tmux").arg("detach-client").status();
}

/// Deterministic, tmux-safe session name from a canonical path.
/// Hex output (`[0-9a-f]`) avoids tmux's illegal `.`/`:` characters entirely.
fn session_name(canon: &Path) -> String {
    let mut h = DefaultHasher::new();
    canon.hash(&mut h);
    format!("mmux-{:016x}", h.finish())
}

fn session_exists(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Make tmux invisible and non-interfering for THIS session only (never `-g`).
/// `title` is the project name pushed to the outer terminal's tab via set-titles.
fn configure_session(name: &str, title: &str) {
    // `#` is the format-escape in set-titles-string, so a literal name must double it.
    let titles_string = title.replace('#', "##");
    let opts = [
        ("status", "off"),            // no tmux status bar
        ("prefix", "None"),           // don't steal keys — everything goes to the TUI
        ("prefix2", "None"),          //
        ("mouse", "off"),             // let the TUI's own mouse handling work
        ("set-clipboard", "on"),      // pass our OSC 52 copies through to the outer terminal
        ("allow-passthrough", "on"),  // let our notification OSCs reach the outer terminal
        ("destroy-unattached", "off"), // keep running after detach (default; explicit)
        ("detach-on-destroy", "on"),  // when the TUI exits, detach cleanly
        ("window-size", "latest"),    // track the single attached client
        ("set-titles", "on"),         // let tmux set the outer terminal's tab title…
        ("set-titles-string", titles_string.as_str()), // …to the project name
    ];
    for (k, v) in opts {
        let _ = Command::new("tmux")
            .args(["set-option", "-t", name, k, v])
            .status();
    }
}

fn which_tmux() -> Option<()> {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .filter(|s| s.success())
        .map(|_| ())
}

/// One row in the attach picker: either a running tmux session, or a recent
/// directory with no live session (`running == false`, selecting it launches one).
struct Entry {
    name: String,
    dir: String,
    running: bool,
    attached: bool,
}

/// `mmux attach` / `mmux a`: pick any running mmux session anywhere and join it.
pub fn attach_picker() -> Result<()> {
    if which_tmux().is_none() {
        eprintln!("tmux not found on PATH.");
        std::process::exit(1);
    }
    let entries = build_entries();
    if entries.is_empty() {
        println!("No running or recent mmux sessions.");
        return Ok(());
    }
    let Some(i) = pick(&entries)? else { return Ok(()) };
    let entry = &entries[i];
    if entry.running {
        // Live session — just join it.
        let status = Command::new("tmux")
            .env_remove("TMUX")
            .args(["attach-session", "-t", &entry.name])
            .status()
            .context("attaching tmux session")?;
        std::process::exit(status.code().unwrap_or(0));
    }
    // Recent directory with no live session — attach-or-create it, exactly as if the
    // user had run `mmux` there.
    launch_in(PathBuf::from(&entry.dir))
}

/// The picker's rows: every running `mmux-*` session first, then recent directories
/// (from `~/.mmux/history`) that have no live session, most-recent-first.
fn build_entries() -> Vec<Entry> {
    let mut entries = list_sessions();
    let running: HashSet<String> = entries.iter().map(|e| e.name.clone()).collect();
    for dir in read_recents() {
        // Recents are stored canonical, so this hash matches the session `launch_in`
        // would attach-or-create for that directory.
        let name = session_name(Path::new(&dir));
        if running.contains(&name) {
            continue;
        }
        entries.push(Entry { name, dir, running: false, attached: false });
    }
    entries
}

/// All running `mmux-*` tmux sessions, with the directory each was opened for.
fn list_sessions() -> Vec<Entry> {
    let out = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}\t#{session_attached}"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut sessions = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let name = parts.next().unwrap_or("").to_string();
        if !name.starts_with("mmux-") {
            continue;
        }
        let attached = parts.next().unwrap_or("0").trim() != "0";
        let dir = session_dir(&name).unwrap_or_else(|| name.clone());
        sessions.push(Entry { name, dir, running: true, attached });
    }
    sessions
}

/// Path to the recents log (`~/.mmux/history`).
fn history_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".mmux").join("history"))
}

/// Push `canon` to the front of the MRU recents list (deduped, capped at 50).
/// Best-effort: every IO error is ignored — recents are a pure convenience, and a
/// lost race between two simultaneous launches only costs one stale entry.
fn record_recent(canon: &Path) {
    let Some(path) = history_path() else { return };
    let dir = canon.to_string_lossy().into_owned();
    let mut dirs: Vec<String> = std::fs::read_to_string(&path)
        .map(|t| recent_lines(&t))
        .unwrap_or_default();
    dirs.retain(|d| d != &dir);
    dirs.insert(0, dir);
    dirs.truncate(50);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, dirs.join("\n") + "\n");
}

/// Recent directories that still exist on disk, most-recent-first. Opportunistically
/// rewrites the log to drop entries whose directory is gone.
fn read_recents() -> Vec<String> {
    let Some(path) = history_path() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let all = recent_lines(&text);
    let live: Vec<String> = all.iter().filter(|d| Path::new(d).is_dir()).cloned().collect();
    if live.len() != all.len() {
        let _ = std::fs::write(&path, live.join("\n") + "\n");
    }
    live
}

/// Non-empty, trimmed lines of the recents log.
fn recent_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Read the directory a session was opened for from its `MMUX_DIR` session env var.
fn session_dir(name: &str) -> Option<String> {
    let out = Command::new("tmux")
        .args(["show-environment", "-t", name, "MMUX_DIR"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("MMUX_DIR=").map(|p| p.trim().to_string()))
}

fn pretty(dir: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if let Some(rest) = dir.strip_prefix(home.as_ref()) {
            return format!("~{rest}");
        }
    }
    dir.to_string()
}

/// A tiny ratatui picker. Returns the chosen entry's index, or None if cancelled.
fn pick(entries: &[Entry]) -> Result<Option<usize>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let mut sel = 0usize;
    let mut chosen: Option<usize> = None;
    let mut row_y: Vec<u16> = Vec::new();

    let res = (|| -> Result<()> {
        loop {
            terminal.draw(|f| {
                let area = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(" mmux — sessions ")
                    .border_style(Style::default().fg(Color::Magenta));
                let inner = block.inner(area);
                f.render_widget(block, area);

                row_y.clear();
                let mut lines: Vec<Line> = Vec::new();
                let mut y = inner.y;
                for (i, e) in entries.iter().enumerate() {
                    // A dim header introduces the recents the moment we cross from the
                    // last running session into the not-running ones.
                    if i > 0 && entries[i - 1].running && !e.running {
                        lines.push(Line::from(Span::styled(
                            "  recent (not running)".to_string(),
                            Style::default().fg(Color::DarkGray),
                        )));
                        y += 1;
                    }
                    let bar = if i == sel { "▌ " } else { "  " };
                    let style = if i == sel {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else if e.running {
                        Style::default().fg(Color::Gray)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    let mut spans = vec![Span::styled(format!("{bar}{}", pretty(&e.dir)), style)];
                    if e.attached {
                        spans.push(Span::styled(
                            "  (attached)".to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    let mut line = Line::from(spans);
                    if i == sel {
                        line.style = Style::default().bg(Color::Rgb(45, 45, 60));
                    }
                    lines.push(line);
                    row_y.push(y);
                    y += 1;
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " ↑↓ move · Enter/click open · q quit ",
                    Style::default().fg(Color::DarkGray),
                )));
                f.render_widget(Paragraph::new(lines), inner);
            })?;

            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('j') | KeyCode::Down => {
                            if sel + 1 < entries.len() {
                                sel += 1;
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => sel = sel.saturating_sub(1),
                        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                            chosen = Some(sel);
                            break;
                        }
                        _ => {}
                    },
                    Event::Mouse(m) => {
                        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                            if let Some(i) = row_y.iter().position(|y| *y == m.row) {
                                if i < entries.len() {
                                    chosen = Some(i);
                                    break;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    res?;
    Ok(chosen)
}
