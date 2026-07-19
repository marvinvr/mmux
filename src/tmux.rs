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
        self, DisableBracketedPaste, DisableMouseCapture, EnableMouseCapture, Event, KeyCode,
        KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
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
    if crate::config::global_config_path().is_none() {
        // No global config ⇒ no agents seeded (the wizard homes agents globally),
        // so treat it as a first run and walk the user through setup — even when a
        // local config already exists, since it may define processes but no agents.
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
    reset_outer_terminal();
    std::process::exit(status.code().unwrap_or(0));
}

/// Detach the attached tmux client; the session (and the inner TUI) keep running.
pub fn detach() {
    let _ = Command::new("tmux").arg("detach-client").status();
}

/// Scrub terminal modes the inner TUI turned on, on the *outer* terminal we hold here.
/// When a client detaches — or the TUI exits and tmux tears the session down — tmux
/// doesn't reliably reset the private modes the inner TUI set, so mouse tracking is left
/// enabled and every mouse move leaks into the shell as `35;36;18M` junk (likewise
/// bracketed paste). The inner TUI's own teardown writes into the pane *as tmux is
/// destroying it*, so it races; this wrapper owns the real terminal once `attach-session`
/// returns, so it's the reliable place to clean up. Idempotent if tmux already did.
fn reset_outer_terminal() {
    let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
}

/// Deterministic, tmux-safe session name from a canonical path.
/// Hex output (`[0-9a-f]`) avoids tmux's illegal `.`/`:` characters entirely.
/// Also reused to key the per-workspace [restore state](crate::restore) file.
pub(crate) fn session_name(canon: &Path) -> String {
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
        ("mouse", "on"),              // tmux enables outer-terminal mouse reporting and, since the
                                      // TUI sets its own mouse mode, forwards events to it rather
                                      // than acting itself. `off` silently drops wheel/clicks when
                                      // attached over SSH (nothing tells the terminal to report).
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

/// Whether the attached client's terminal supports sixel graphics, per tmux's own
/// capability detection (`client_termfeatures`). mmux draws *through* tmux, and tmux
/// (3.4+, built with sixel) renders sixel natively for capable clients — so this is the
/// reliable gate for showing a real-pixel image preview instead of the half-block
/// fallback: when tmux says the client does sixel, tmux will faithfully render what we
/// emit (and it never advertises it for a client that can't). Cheap; queried once.
pub(crate) fn client_supports_sixel() -> bool {
    Command::new("tmux")
        .args(["display-message", "-p", "#{client_termfeatures}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("sixel"))
        .unwrap_or(false)
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
    /// The project's display name (its `mmux.yaml` `name:`, else the folder), shown
    /// as the row's primary label with `dir` trailing in dim text.
    display: String,
    dir: String,
    running: bool,
    attached: bool,
    /// Manifest workspaces form the first group in the default picker order, whether
    /// currently running or only present in recents.
    workspace: bool,
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
        reset_outer_terminal();
        std::process::exit(status.code().unwrap_or(0));
    }
    // Recent directory with no live session — attach-or-create it, exactly as if the
    // user had run `mmux` there.
    launch_in(PathBuf::from(&entry.dir))
}

/// The picker's rows: workspace manifests first (running or recent), then running
/// standalone projects, then recent standalone projects. Each group is MRU-ordered.
fn build_entries() -> Vec<Entry> {
    let recents = read_recents();
    // MRU rank by canonical directory: lower index = more recently used. Recents and
    // each session's `MMUX_DIR` are both stored canonical, so they key the same map.
    let rank: HashMap<&str, usize> =
        recents.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();

    // Active sessions, sorted most-recently-used first. A session whose directory isn't
    // in the log (e.g. `MMUX_DIR` couldn't be read) sorts last, stably.
    let mut entries = list_sessions();
    entries.sort_by_key(|e| rank.get(e.dir.as_str()).copied().unwrap_or(usize::MAX));

    let running: HashSet<String> = entries.iter().map(|e| e.name.clone()).collect();
    for dir in &recents {
        // Recents are stored canonical, so this hash matches the session `launch_in`
        // would attach-or-create for that directory.
        let name = session_name(Path::new(dir));
        if running.contains(&name) {
            continue;
        }
        let (display, workspace) = crate::config::project_identity(Path::new(dir));
        entries.push(Entry {
            name,
            display,
            dir: dir.clone(),
            running: false,
            attached: false,
            workspace,
        });
    }
    entries.sort_by_key(|e| {
        let group = if e.workspace { 0 } else if e.running { 1 } else { 2 };
        (group, rank.get(e.dir.as_str()).copied().unwrap_or(usize::MAX))
    });
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
        let (display, workspace) = crate::config::project_identity(Path::new(&dir));
        sessions.push(Entry { name, display, dir, running: true, attached, workspace });
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

/// A tiny ratatui picker with an always-present fuzzy search bar. Returns the chosen
/// entry's index (into `entries`), or None if cancelled.
///
/// The search bar is never the selection: the first match is highlighted by default
/// and `↑`/`↓` move through the list. You don't have to focus the bar — the moment you
/// type a letter it fuzzy-filters by name + directory, `Backspace` trims the query, and
/// `Esc` clears it (quitting on a second press).
fn pick(entries: &[Entry]) -> Result<Option<usize>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(out))?;

    let mut query = String::new();
    // `filtered`: the entry indices to show, in display order. Recomputed only when the
    // query changes. `sel` indexes into `filtered`, so it never points at the search bar.
    let mut filtered: Vec<usize> = rank(entries, &query);
    let mut sel = 0usize;
    let mut chosen: Option<usize> = None;
    // (screen row, entry index) for each visible result, for click routing.
    let mut row_y: Vec<(u16, usize)> = Vec::new();

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

                // The search bar: always drawn, never selected. A caret marks it as the
                // live input target; an empty query shows a dim hint in its place.
                let mut search = vec![Span::styled("  ", Style::default())];
                if query.is_empty() {
                    search.push(Span::styled("▏", Style::default().fg(Color::Magenta)));
                    search.push(Span::styled(
                        " type to search",
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    search.push(Span::styled(
                        query.clone(),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ));
                    search.push(Span::styled("▏", Style::default().fg(Color::Magenta)));
                }
                lines.push(Line::from(search));
                lines.push(Line::from(""));

                if filtered.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  no matches",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                for (pos, &ei) in filtered.iter().enumerate() {
                    let e = &entries[ei];
                    // The natural view is grouped; a ranked search deliberately drops
                    // headers because its results mix all three groups.
                    let group = entry_group(e);
                    if query.is_empty()
                        && (pos == 0 || entry_group(&entries[filtered[pos - 1]]) != group)
                    {
                        if pos > 0 {
                            lines.push(Line::from(""));
                        }
                        lines.push(Line::from(Span::styled(
                            format!("  {}", entry_group_label(group)),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    let selected = pos == sel;
                    let bar = if selected { "▌ " } else { "  " };
                    let style = if selected {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else if e.running {
                        Style::default().fg(Color::Gray)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    // Project name is the primary key; its directory trails in dim
                    // text so the name reads first and the path stays secondary.
                    let mut spans = vec![
                        Span::styled(format!("{bar}{}", e.display), style),
                        Span::styled(
                            format!("  {}", pretty(&e.dir)),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ];
                    if e.attached {
                        spans.push(Span::styled(
                            "  (attached)".to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    let mut line = Line::from(spans);
                    if selected {
                        line.style = Style::default().bg(Color::Rgb(45, 45, 60));
                    }
                    // The line's eventual screen row is its index in `lines` from the top.
                    let y = inner.y + lines.len() as u16;
                    lines.push(line);
                    row_y.push((y, ei));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " ↑↓ move · type to search · Enter/click open · Esc clear/quit ",
                    Style::default().fg(Color::DarkGray),
                )));
                f.render_widget(Paragraph::new(lines), inner);
            })?;

            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        // Raw mode delivers Ctrl+C as a key, not a SIGINT — handle it
                        // ourselves so it always cancels the picker.
                        if k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            break;
                        }
                        match k.code {
                            // Esc clears the query first (search-bar convention), then quits.
                            KeyCode::Esc => {
                                if query.is_empty() {
                                    break;
                                }
                                query.clear();
                                filtered = rank(entries, &query);
                                sel = 0;
                            }
                            KeyCode::Down => {
                                if sel + 1 < filtered.len() {
                                    sel += 1;
                                }
                            }
                            KeyCode::Up => sel = sel.saturating_sub(1),
                            KeyCode::Enter | KeyCode::Right => {
                                if let Some(&ei) = filtered.get(sel) {
                                    chosen = Some(ei);
                                    break;
                                }
                            }
                            KeyCode::Backspace => {
                                query.pop();
                                filtered = rank(entries, &query);
                                sel = 0;
                            }
                            // Any other printable key types into the search bar — no need
                            // to focus it first. Control/Alt chords stay free for shortcuts.
                            KeyCode::Char(c)
                                if !k
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                query.push(c);
                                filtered = rank(entries, &query);
                                sel = 0;
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(m) => {
                        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                            if let Some(ei) =
                                row_y.iter().find(|(y, _)| *y == m.row).map(|(_, ei)| *ei)
                            {
                                chosen = Some(ei);
                                break;
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

/// The entry indices to display for `query`, in order. An empty query keeps the natural
/// order (workspaces, active projects, then recents). A non-empty query fuzzy-matches each
/// entry's name + directory and ranks the survivors best-first — reusing the file
/// picker's boundary-aware scorer — dropping the entries that don't match at all.
fn rank(entries: &[Entry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let hay = format!("{}  {}", e.display, pretty(&e.dir));
            crate::app::picker::score(query, &hay).map(|s| (s, i))
        })
        .collect();
    // Best score first; tie-break on the shorter name (the more specific match).
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| entries[a.1].display.len().cmp(&entries[b.1].display.len()))
    });
    scored.into_iter().map(|(_, i)| i).collect()
}

fn entry_group(entry: &Entry) -> u8 {
    if entry.workspace {
        0
    } else if entry.running {
        1
    } else {
        2
    }
}

fn entry_group_label(group: u8) -> &'static str {
    match group {
        0 => "workspaces",
        1 => "running projects",
        _ => "recent (not running)",
    }
}

#[cfg(test)]
mod tests {
    use super::session_name;
    use std::path::Path;

    #[test]
    fn session_name_is_deterministic_and_tmux_safe() {
        // The singleton-per-directory guarantee rests on this being stable for a
        // given path within a run, and free of tmux's illegal `.`/`:` characters.
        let a = session_name(Path::new("/Users/me/project"));
        let b = session_name(Path::new("/Users/me/project"));
        assert_eq!(a, b, "the same path must map to the same session name");

        let hex = a.strip_prefix("mmux-").expect("session names are mmux-prefixed");
        assert_eq!(hex.len(), 16, "16 hex digits of the path hash");
        assert!(
            hex.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "lowercase hex only — no `.`/`:` tmux forbids: {a}"
        );
    }

    #[test]
    fn session_name_differs_by_path() {
        assert_ne!(session_name(Path::new("/a/one")), session_name(Path::new("/a/two")));
    }
}
