//! Rendering for the modal overlays that float above the whole UI.
//!
//! One dispatcher ([`render_overlay`]) fans out to the six modal renderers — the
//! commit / new-branch text prompt, the yes/no confirmation, the Ctrl+P file picker,
//! the "+ New Process" guided form, the agent manager, and the "About mmux" card.
//! They share the modal chrome ([`modal_frame`] over a [`centered`] rect) and a
//! handful of modal-only helpers. The overlay STATE and its key handling live in
//! [`crate::app::overlay`].

use crate::agentmgr::{AgentManager, Mode};
use crate::app::overlay::{Overlay, PromptKind};
use crate::app::picker::Picker;
use crate::app::procform::{ProcForm, Step, STEPS};
use crate::app::UpdateState;
use crate::workspacemgr::WorkspaceManager;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::git::truncate_middle;

/// The scroll offset that keeps row `sel` visible in a `height`-row list window: 0 until
/// the selection would fall past the bottom, then just enough to pin it to the last row.
/// The file picker's simple top-anchored scroll.
fn list_scroll(sel: usize, height: usize) -> usize {
    if height > 0 && sel >= height {
        sel + 1 - height
    } else {
        0
    }
}

// ---- overlay: the commit / new-branch text prompt ----

pub(crate) fn render_overlay(f: &mut Frame, area: Rect, ov: &Overlay) {
    match ov {
        Overlay::Prompt { title, buf, kind } => render_prompt(f, area, title, buf, *kind),
        Overlay::Confirm {
            title, body, hint, ..
        } => render_confirm(f, area, title, body, hint),
        Overlay::Picker(p) => render_picker(f, area, p),
        Overlay::NewProcess(form) => render_procform(f, area, form),
        Overlay::Agents(m) => render_agentmgr(f, area, m),
        Overlay::Workspace(m) => render_workspacemgr(f, area, m),
        // Drawn by `render_about` (it needs live update state), routed there in `draw`.
        Overlay::About => {}
    }
}

/// The mmux mark as half-block pixel art — the same green tile (beveled, with the
/// `m` knocked out) as the web favicon, painted 16 px square into 16 cols × 8 rows
/// via the `▀`/`▄` half-block glyphs (so the cells read square). `pad` leading
/// spaces center it; cells over the knocked-out `m` fall back to the card background.
fn mmux_logo_lines(pad: usize) -> Vec<Line<'static>> {
    // one char per pixel — L light bevel, F face, D deep bevel, m knockout.
    const GRID: [&str; 16] = [
        "LLLLLLLLLLLLLLLD",
        "LFFFFFFFFFFFFFFD",
        "LFFFFFFFFFFFFFFD",
        "LFFFFFFFFFFFFFFD",
        "LFFmmmmmmmmmmFFD",
        "LFFmmmmmmmmmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFmmFFmmFFmmFFD",
        "LFFFFFFFFFFFFFFD",
        "LFFFFFFFFFFFFFFD",
        "LFFFFFFFFFFFFFFD",
        "DDDDDDDDDDDDDDDD",
    ];
    let color = |c: u8| match c {
        b'L' => Some(Color::Rgb(134, 239, 172)),
        b'F' => Some(Color::Rgb(74, 222, 128)),
        b'D' => Some(Color::Rgb(22, 163, 74)),
        _ => None, // knockout → default card background
    };
    (0..8)
        .map(|r| {
            let top = GRID[r * 2].as_bytes();
            let bot = GRID[r * 2 + 1].as_bytes();
            let mut spans = Vec::with_capacity(17);
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            for x in 0..16 {
                // '▀' paints fg in the upper half + bg in the lower; '▄' the inverse.
                // Pick the glyph so a knocked-out half always shows the default bg.
                let (glyph, style) = match (color(top[x]), color(bot[x])) {
                    (Some(t), Some(b)) => ("▀", Style::default().fg(t).bg(b)),
                    (Some(t), None) => ("▀", Style::default().fg(t)),
                    (None, Some(b)) => ("▄", Style::default().fg(b)),
                    (None, None) => (" ", Style::default()),
                };
                spans.push(Span::styled(glyph, style));
            }
            Line::from(spans)
        })
        .collect()
}

/// Draws the card and returns the clickable link hitboxes (screen rect → URL) for
/// [`Regions::links`](super::Regions), so a click on a link opens it in the browser.
pub(crate) fn render_about(
    f: &mut Frame,
    area: Rect,
    update: &UpdateState,
    can_update: bool,
) -> Vec<(Rect, String)> {
    let version = env!("CARGO_PKG_VERSION");

    // The update status line + the action key (if any) it unlocks.
    let (status, status_style, action): (String, Style, Option<&str>) = if !can_update {
        (
            "self-update off for this build".into(),
            Style::default().fg(Color::DarkGray),
            None,
        )
    } else {
        match update {
            UpdateState::Idle => (
                "✓ up to date".into(),
                Style::default().fg(super::theme::ATTN),
                Some("c check"),
            ),
            UpdateState::Checking => (
                "checking for updates…".into(),
                Style::default().fg(Color::Gray),
                Some("c check"),
            ),
            // Brew install with an update pending confirmation — `u` runs `brew upgrade`.
            UpdateState::Available(v) => (
                format!("↻ v{v} available"),
                Style::default()
                    .fg(super::theme::ATTN)
                    .add_modifier(Modifier::BOLD),
                Some("u update"),
            ),
            UpdateState::Installing(v) => (
                format!("↻ installing v{v}…"),
                Style::default().fg(Color::Gray),
                None,
            ),
            UpdateState::Ready(v) => (
                format!("↻ v{v} ready"),
                Style::default()
                    .fg(super::theme::ATTN)
                    .add_modifier(Modifier::BOLD),
                Some("u restart to update"),
            ),
            // A check ran and found this isn't a managed install — self-update can't act,
            // so there's no `c` to offer. Same wording as the synchronous off-build case.
            UpdateState::Unsupported => (
                "self-update off (unmanaged install)".into(),
                Style::default().fg(Color::DarkGray),
                None,
            ),
        }
    };
    let hint = match action {
        Some(a) => format!("{a} · esc close"),
        None => "esc close".into(),
    };

    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let label = Style::default().fg(Color::DarkGray);
    let link = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED);
    // (label, link text, URL). Rendered as a line *and* registered as a click target
    // below, so the two stay in sync. The label is fixed-width, so the URL always
    // starts at the same column.
    let link_rows: [(&str, &str, &str); 2] = [
        ("built by  ", "marvinvr.ch", "https://marvinvr.ch"),
        (
            "source    ",
            "github.com/marvinvr/mmux",
            "https://github.com/marvinvr/mmux",
        ),
    ];
    let text_lines: Vec<Line> = vec![
        Line::from(Span::styled(format!("mmux v{version}"), bold)),
        Line::from(Span::styled(
            "persistent terminals for your coding agents",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(link_rows[0].0, label),
            Span::styled(link_rows[0].1, link),
        ]),
        Line::from(vec![
            Span::styled(link_rows[1].0, label),
            Span::styled(link_rows[1].1, link),
        ]),
        Line::from(""),
        Line::from(Span::styled(status, status_style)),
        Line::from(""),
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
    ];
    // The two link lines' index within the text block (rows 3 and 4 above).
    let link_line_idx = [3u16, 4u16];

    // Top the card with the mmux mark (half-block pixel art), centered over the text.
    let text_widest = text_lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let n_text = text_lines.len();
    let mut lines = mmux_logo_lines(text_widest.saturating_sub(16) / 2);
    lines.push(Line::from(""));
    lines.extend(text_lines);
    // Where the text block starts within `lines` (past the logo + its blank line).
    let text_top = (lines.len() - n_text) as u16;

    let widest = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let w = (widest as u16 + 4).clamp(24, 64);
    let h = lines.len() as u16 + 2;
    let inner = modal_frame(f, area, w, h, " About mmux ", Color::Cyan);
    if inner.width == 0 || inner.height == 0 {
        return Vec::new();
    }
    f.render_widget(Paragraph::new(lines), inner);

    // Register each link's on-screen span as a click target.
    let right = inner.x + inner.width;
    let bottom = inner.y + inner.height;
    let mut links = Vec::new();
    for (i, (label_s, link_s, url)) in link_rows.iter().enumerate() {
        let x = inner.x + label_s.chars().count() as u16;
        let y = inner.y + text_top + link_line_idx[i];
        if x >= right || y >= bottom {
            continue; // clipped off a tiny card
        }
        let width = (link_s.chars().count() as u16).min(right - x);
        links.push((
            Rect {
                x,
                y,
                width,
                height: 1,
            },
            url.to_string(),
        ));
    }
    links
}

/// The agent manager: one checkbox row per built-in harness (enabled + a launch-mode
/// tag), its blurb dimmed alongside, with the cursor row marked. Toggled/saved in
/// [`agentmgr_key`](crate::app::overlay); the write targets the global config.
fn render_agentmgr(f: &mut Frame, area: Rect, m: &AgentManager) {
    let w = area.width.saturating_sub(6).clamp(40, 68);
    // Intro + blank + one line per row + blank + hint, inside the two borders.
    let h = (m.rows.len() as u16 + 6).min(area.height);
    let inner = modal_frame(f, area, w, h, " Agents ", Color::Magenta);
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Available in every project · saved to ~/.mmux/config.yaml",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];
    for (i, r) in m.rows.iter().enumerate() {
        let selected = i == m.cursor;
        let (checkbox, check_style) = if r.enabled {
            ("[x] ", Style::default().fg(Color::Green))
        } else {
            ("[ ] ", Style::default().fg(Color::DarkGray))
        };
        let name_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if r.enabled {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        // A green ✓ highlights the harnesses actually found on PATH; not-installed ones
        // are left unmarked (never blocked — you can still enable them). Fixed-width so
        // the following columns stay aligned.
        let (install, install_style) = if r.installed {
            ("✓ ", Style::default().fg(Color::Green))
        } else {
            ("  ", Style::default())
        };
        // A fixed-width (8) mode cell so the blurbs stay aligned whatever mode is shown.
        let mode = r.mode();
        let mode_color = match mode {
            Mode::Normal => Color::DarkGray,
            Mode::Auto => Color::Cyan,
            Mode::Danger => Color::Yellow,
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "› " } else { "  " },
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(checkbox, check_style),
            Span::styled(install, install_style),
            Span::styled(format!("{:<10}", r.name), name_style),
            Span::styled(
                format!("{:<8}", mode.label()),
                Style::default().fg(mode_color),
            ),
            Span::styled(
                format!("  {}", r.blurb),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let body = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height - 1,
    };
    f.render_widget(Paragraph::new(lines), body);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "space toggle · m mode · ⏎ save · esc cancel · ✓ on PATH",
            Style::default().fg(Color::DarkGray),
        ))),
        Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width,
            height: 1,
        },
    );
}

/// The manifest workspace manager: editable name plus a scrollable checkbox list.
/// Tags help distinguish ready project folders from plain directories without making
/// configuration or git a requirement.
fn render_workspacemgr(f: &mut Frame, area: Rect, m: &WorkspaceManager) {
    let w = area.width.saturating_sub(6).clamp(42, 76);
    let h = area.height.saturating_sub(4).clamp(9, 22);
    let inner = modal_frame(f, area, w, h, " Workspace ", Color::Magenta);
    if inner.width == 0 || inner.height < 5 {
        return;
    }

    let name_style = if m.editing_name {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Name  ", Style::default().fg(Color::DarkGray)),
            Span::styled(m.name.clone(), name_style),
            Span::styled(
                if m.editing_name {
                    "  editing"
                } else {
                    "  n edit"
                },
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );

    let list_y = inner.y + 2;
    let hint_y = inner.y + inner.height - 1;
    let status_y = hint_y.saturating_sub(1);
    let list_h = status_y.saturating_sub(list_y) as usize;
    let scroll = list_scroll(m.cursor, list_h);
    let mut lines = Vec::new();
    for i in scroll..scroll + list_h {
        let Some(r) = m.rows.get(i) else { break };
        let selected = i == m.cursor;
        let checkbox = if r.enabled { "[x]" } else { "[ ]" };
        let checkbox_style = if r.enabled {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label = if r.path == "." {
            ".  (this directory)".to_string()
        } else {
            r.path.clone()
        };
        let label_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if r.enabled {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mut tags = Vec::new();
        if r.configured {
            tags.push("mmux");
        }
        if r.git {
            tags.push("git");
        }
        if !r.exists {
            tags.push("missing");
        }
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "› " } else { "  " },
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(format!("{checkbox} "), checkbox_style),
            Span::styled(label, label_style),
            Span::styled(
                if tags.is_empty() {
                    String::new()
                } else {
                    format!("  {}", tags.join(" · "))
                },
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: list_y,
            width: inner.width,
            height: list_h as u16,
        },
    );

    let status = m.error.clone().unwrap_or_else(|| {
        format!(
            "{} / {} projects selected",
            m.selected_count(),
            crate::config::MAX_PROJECTS
        )
    });
    let status_style = if m.error.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(status, status_style))),
        Rect {
            x: inner.x,
            y: status_y,
            width: inner.width,
            height: 1,
        },
    );
    let hint = if m.editing_name {
        "type a name · ⏎ done · esc done"
    } else {
        "space toggle · J/K order · a all · n name · ⏎ save · esc cancel"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))),
        Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        },
    );
    if m.editing_name {
        let cx = inner.x + 6 + (m.name.chars().count() as u16).min(inner.width.saturating_sub(7));
        f.set_cursor_position((cx, inner.y));
    }
}

/// The "+ New Process" guided form: a "Step N of 4" header, the fields already
/// entered (dim, for context), the active input or the review screen, an optional
/// validation warning, and a key hint pinned to the bottom row.
fn render_procform(f: &mut Frame, area: Rect, form: &ProcForm) {
    let w = area.width.saturating_sub(6).clamp(34, 72);
    let h = 13u16.min(area.height);
    // Same form for adding and editing; the title and the Review verb reflect which.
    let editing = form.edit.is_some();
    let title = if editing {
        " Edit process "
    } else {
        " New process "
    };
    let inner = modal_frame(f, area, w, h, title, Color::Magenta);
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    let review_hint = if editing {
        "←→ autostart · ⏎ save · esc cancel"
    } else {
        "←→ autostart · ⏎ create · esc cancel"
    };
    let (label, hint) = match form.step {
        Step::Name => ("Name", "⏎ next · esc cancel"),
        Step::Command => ("Command", "⏎ next · esc cancel"),
        Step::Cwd => ("Working dir", "⏎ next (blank ok) · esc cancel"),
        Step::Stop => ("Stop command", "⏎ next (blank ok) · esc cancel"),
        Step::Review => ("Review", review_hint),
    };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Step {} of {STEPS} · {label}", form.step_index()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // The screen row the text cursor should sit on (set on text steps only).
    let mut input_y: Option<u16> = None;
    match form.step {
        Step::Review => {
            lines.push(field_line("Name", &form.name));
            lines.push(field_line("Command", &form.command));
            let cwd = if form.cwd.trim().is_empty() {
                "(project root)"
            } else {
                form.cwd.trim()
            };
            lines.push(field_line("Working dir", cwd));
            let stop = if form.stop.trim().is_empty() {
                "(none)"
            } else {
                form.stop.trim()
            };
            lines.push(field_line("Stop cmd", stop));
            let mark = if form.autostart { " yes " } else { " no " };
            lines.push(Line::from(vec![
                Span::styled("Autostart    ", Style::default().fg(Color::Gray)),
                Span::styled(
                    mark,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        step => {
            // Echo the fields gathered so far for context.
            if step != Step::Name {
                lines.push(field_line("Name", &form.name));
            }
            if matches!(step, Step::Cwd | Step::Stop) {
                lines.push(field_line("Command", &form.command));
            }
            if step == Step::Stop {
                let cwd = if form.cwd.trim().is_empty() {
                    "(project root)"
                } else {
                    form.cwd.trim()
                };
                lines.push(field_line("Working dir", cwd));
            }
            let prompt = match step {
                Step::Name => "A label for the sidebar, e.g. Dev server",
                Step::Command => "The command to run, e.g. npm run dev",
                Step::Cwd => "Directory relative to the project (blank = project root)",
                Step::Stop => "Optional teardown run in the dir, e.g. docker compose down",
                Step::Review => "",
            };
            lines.push(Line::from(Span::styled(
                prompt,
                Style::default().fg(Color::DarkGray),
            )));
            input_y = Some(inner.y + lines.len() as u16);
            lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(Color::Magenta)),
                Span::styled(form.buf.clone(), Style::default().fg(Color::White)),
            ]));
        }
    }
    if let Some(err) = &form.error {
        lines.push(Line::from(Span::styled(
            format!("⚠ {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Body above, hint pinned to the last inner row.
    let body = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height - 1,
    };
    f.render_widget(Paragraph::new(lines), body);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))),
        Rect {
            x: inner.x,
            y: inner.y + inner.height - 1,
            width: inner.width,
            height: 1,
        },
    );

    if let Some(y) = input_y {
        let cx = inner.x + 2 + (form.buf.chars().count() as u16).min(inner.width.saturating_sub(3));
        f.set_cursor_position((cx, y));
    }
}

/// A dim `label: value` context line in the process form.
fn field_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::styled(value.to_string(), Style::default().fg(Color::Gray)),
    ])
}

/// The Ctrl+P fuzzy file picker: a query line, the ranked match list (the
/// selection scrolled into view and highlighted), and a hint/count footer.
fn render_picker(f: &mut Frame, area: Rect, p: &Picker) {
    let w = area.width.saturating_sub(6).clamp(24, 90);
    let h = area.height.saturating_sub(4).clamp(6, 20);
    let inner = modal_frame(f, area, w, h, " Open file ", Color::Magenta);
    if inner.width == 0 || inner.height < 3 {
        return;
    }

    // Rows: query line on top, hint on the bottom, the list between.
    let query_y = inner.y;
    let hint_y = inner.y + inner.height - 1;
    let list_y = inner.y + 1;
    let list_h = hint_y.saturating_sub(list_y) as usize;
    let width = inner.width as usize;

    // Query line: `> typed query`.
    let query = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Magenta)),
        Span::styled(p.query.clone(), Style::default().fg(Color::White)),
    ]);
    f.render_widget(
        Paragraph::new(query),
        Rect {
            x: inner.x,
            y: query_y,
            width: inner.width,
            height: 1,
        },
    );

    // Scroll the list so the selection stays on screen.
    let sel = p.sel();
    let scroll = list_scroll(sel, list_h);
    let mut lines: Vec<Line> = Vec::new();
    for row in scroll..(scroll + list_h) {
        let Some(path) = p.path_at(row) else { break };
        let selected = row == sel;
        // When too long, elide the middle so both the leading dirs and the
        // filename/extension stay visible.
        let mut text = truncate_middle(path, width);
        let style = if selected {
            // Pad to the full width so the highlight bar spans the row.
            let pad = width.saturating_sub(text.chars().count());
            text.push_str(&" ".repeat(pad));
            Style::default().fg(Color::Black).bg(Color::Magenta)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: list_y,
            width: inner.width,
            height: list_h as u16,
        },
    );

    // Footer: match count + the key hints.
    let hint = format!(
        "{} matches · ↑↓ move · ⏎ open · esc cancel",
        p.match_count()
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))),
        Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        },
    );

    // Park the cursor at the end of the query (after the `> ` prompt).
    let cx = inner.x + 2 + (p.query.chars().count() as u16).min(inner.width.saturating_sub(3));
    f.set_cursor_position((cx, query_y));
}

/// A destructive-action confirmation: the question, then `y discard · n cancel`. Red
/// border to signal it can't be undone.
/// A yes/no modal. The body may span several lines (split on `\n`); the box sizes
/// itself to the widest line and the per-action `hint` sits below a blank spacer.
fn render_confirm(f: &mut Frame, area: Rect, title: &str, body: &str, hint: &str) {
    let body_lines: Vec<&str> = body.lines().collect();
    let widest = body_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(hint.chars().count());
    let w = (widest as u16 + 4).clamp(20, 64);
    // Body lines + a blank spacer + the hint, wrapped in the two borders.
    let h = body_lines.len() as u16 + 4;
    let inner = modal_frame(f, area, w, h, format!(" {title} "), Color::Red);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines: Vec<Line> = body_lines
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::White),
            ))
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        hint.to_string(),
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_prompt(f: &mut Frame, area: Rect, title: &str, buf: &str, kind: PromptKind) {
    let w = area.width.saturating_sub(8).min(60).max(20);
    let inner = modal_frame(f, area, w, 4, format!(" {title} "), Color::Magenta);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let hint = match kind {
        PromptKind::Commit { push: false } => "⏎ commit · ^P commit & push · esc cancel",
        PromptKind::Commit { push: true } => "⏎ commit & push · esc cancel",
        PromptKind::NewBranch => "⏎ create & switch · esc cancel",
    };
    let lines = vec![
        Line::from(Span::styled(
            buf.to_string(),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
    // Park the real cursor at the end of the typed text.
    let cx = inner.x + (buf.chars().count() as u16).min(inner.width.saturating_sub(1));
    f.set_cursor_position((cx, inner.y));
}

/// A `w`×`h` rectangle centered inside `area` (clamped to fit).
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// The shared chrome of every modal overlay: center a `w`×`h` box in `area`, then draw
/// its titled, `border`-colored frame over a cleared background and return the inner
/// rect. Each caller keeps its own min-size guard on that inner and fills the body — the
/// guards differ per modal, so they stay at the call sites.
fn modal_frame<'a>(
    f: &mut Frame,
    area: Rect,
    w: u16,
    h: u16,
    title: impl Into<Line<'a>>,
    border: Color,
) -> Rect {
    let rect = centered(area, w, h);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border));
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    inner
}
