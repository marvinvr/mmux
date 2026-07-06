//! Shared styling primitives for the sidebar and pane chrome.

use crate::app::Status;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// "Needs your attention" accent, reserved for an agent that's gone idle/awaiting you
/// (or a terminal that rang the bell). Color is the scarce signal here: busy agents
/// recede to gray, so the only thing the eye lands on is the one that's ready for you.
/// Red stays for genuine errors.
pub(crate) const ATTN: Color = Color::Green;

/// The git diff pager's palette. Added/removed lines carry a subtle full-row **tint**
/// (dark enough that the syntax-highlit foreground stays legible on top) rather than
/// flat green/red text, with the `+`/`-` kept as a saturated sign in the gutter — the
/// modern GitHub/editor look. The line-number gutter and `@@` hunk headers are quiet.
pub(crate) const DIFF_ADD_BG: Color = Color::Rgb(16, 42, 28);
pub(crate) const DIFF_DEL_BG: Color = Color::Rgb(52, 22, 26);
pub(crate) const DIFF_ADD_SIGN: Color = Color::Rgb(63, 185, 80);
pub(crate) const DIFF_DEL_SIGN: Color = Color::Rgb(248, 81, 73);
pub(crate) const DIFF_GUTTER: Color = Color::Rgb(110, 118, 129);
pub(crate) const DIFF_HUNK: Color = Color::Rgb(88, 166, 255);
/// Background tint behind a `▸ path` file divider in a multi-file commit diff — a muted
/// blue-gray bar so each file's start reads as a section break.
pub(crate) const DIFF_FILE_BG: Color = Color::Rgb(33, 43, 60);

pub(crate) fn status_style(s: Status) -> Style {
    match s {
        Status::Running => Style::default().fg(Color::Green),
        Status::Failed => Style::default().fg(Color::Red),
        // A process that finished cleanly reads the same as one that was never
        // started or was stopped: just "not running", not a dim-gray husk.
        Status::Exited | Status::Stopped => Style::default().fg(Color::Gray),
    }
}

pub(crate) fn status_label(s: Status) -> &'static str {
    match s {
        Status::Running => "running",
        Status::Exited => "exited",
        Status::Stopped => "stopped",
        Status::Failed => "crashed",
    }
}

/// The leading status glyph for processes and the panel.
pub(crate) fn badge(s: Status) -> &'static str {
    match s {
        Status::Running => "●",
        Status::Failed => "○",
        // Finished and never-started share the dim dot — both are just "not running".
        Status::Exited | Status::Stopped => "·",
    }
}

/// Frames of the "working" spinner shown before a busy agent's name — a rotating
/// braille snake confined to the cell's *middle* two rows (dots 2,3,5,6) so it sits
/// dead-center on the name's baseline instead of floating high (top rows) or sinking
/// low (bottom rows). That band only has four dot positions, hence four frames; the
/// caller ([`crate::app::App::spinner`]) holds each one longer to keep the spin rate
/// steady, and indexes by time so every agent's spinner turns in step.
pub(crate) const SPINNER: [&str; 4] = ["⠲", "⠴", "⠦", "⠖"];

/// The leading glyph + name style for an agent or terminal row. Unlike a process
/// (where green = "it's up" is what you want to know), an agent's useful signal is
/// "does it need *me*". So color is held back until it does: an idle/awaiting agent
/// (or a bell) lights the row green (`●`); while it's running-and-busy it recedes to
/// gray, showing the `working` glyph the caller passes (an animated [`SPINNER`] frame
/// for agents, a plain `·` for terminals); when it's not running it's a dim hollow
/// `○`, turning red if it **crashed** (exited non-zero on its own) or failed to start.
/// A clean exit never reaches here — the row is pruned (see `prune_exited`).
pub(crate) fn agent_glyph_style(
    s: Status,
    attention: bool,
    error: bool,
    working: &'static str,
) -> (&'static str, Style) {
    if attention {
        return ("●", Style::default().fg(ATTN).add_modifier(Modifier::BOLD));
    }
    match s {
        Status::Running => (working, Style::default().fg(Color::Gray)),
        // A crash (Failed) or a launch error is the one thing that turns the row red:
        // the agent/terminal died and is worth a look. A clean exit doesn't linger.
        Status::Failed => ("○", Style::default().fg(Color::Red)),
        _ if error => ("○", Style::default().fg(Color::Red)),
        _ => ("○", Style::default().fg(Color::DarkGray)),
    }
}

/// A section header line ("AGENTS", "PROCESSES", …).
pub(crate) fn header(t: &str) -> Line<'static> {
    Line::from(Span::styled(
        t.to_string(),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ))
}

/// A workspace project header — the heading above a project's AGENTS/TERMINAL/
/// PROCESSES groups (only shown when the workspace has more than one project). The
/// active project (the one whose panel is shown) gets a full-width highlight bar;
/// `width` is the sidebar's inner width so the bar spans the whole row.
pub(crate) fn project_header(name: &str, active: bool, width: u16) -> Line<'static> {
    let fg = if active { Color::White } else { Color::Gray };
    let mut line = Line::from(Span::styled(
        format!(" {name} "),
        Style::default().fg(fg).add_modifier(Modifier::BOLD),
    ));
    if active {
        let pad = width.saturating_sub(line.width() as u16);
        if pad > 0 {
            line.spans.push(Span::raw(" ".repeat(pad as usize)));
        }
        line.style = Style::default().bg(Color::Rgb(40, 40, 70));
    }
    line
}

/// One sidebar row: selection bar, label, optional dim subtitle, optional red bell.
/// `width` is the sidebar's inner width so the selection highlight can be padded
/// to span the full row, not just the text.
pub(crate) fn entry_line(
    label: &str,
    selected: bool,
    base: Style,
    subtitle: Option<&str>,
    attention: bool,
    width: u16,
) -> Line<'static> {
    let bar = if selected { "▌ " } else { "  " };
    let name_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let mut spans = vec![Span::styled(format!("{bar}{label}"), name_style)];
    if let Some(s) = subtitle {
        if !s.is_empty() {
            spans.push(Span::styled(
                format!(" {s}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    if attention {
        spans.push(Span::styled(
            " ●".to_string(),
            Style::default().fg(ATTN).add_modifier(Modifier::BOLD),
        ));
    }
    let mut line = Line::from(spans);
    if selected {
        // Pad with trailing spaces so the highlight bar fills the whole row width
        // rather than stopping at the end of the text.
        let pad = width.saturating_sub(line.width() as u16);
        if pad > 0 {
            line.spans.push(Span::raw(" ".repeat(pad as usize)));
        }
        line.style = Style::default().bg(Color::Rgb(45, 45, 60));
    }
    line
}
