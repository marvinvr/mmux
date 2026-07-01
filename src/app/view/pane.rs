//! Rendering of the two terminal regions — the main pane and the right panel —
//! plus their titles and "nothing running yet" placeholders. Both regions share
//! [`render_screen`]/[`render_placeholder`]; the wrappers only differ in which
//! recipe/rect/resize target they touch.

use super::theme::status_label;
use crate::app::git::{DiffKind, DiffLine, DiffView, PreviewImage};
use crate::app::input::SelTarget;
use crate::app::nav::Nav;
use crate::app::session::Kind;
use crate::app::{App, Focus};
use crate::pane::Pane;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use tui_term::widget::{Cursor, PseudoTerminal};

impl App {
    pub(crate) fn render_main(&mut self, f: &mut Frame, area: Rect) {
        let nav = self.current_nav();
        let focus = self.focus;
        let border = if focus == Focus::Terminal {
            Color::Magenta
        } else {
            Color::DarkGray
        };
        // A git diff preview re-titles the pane (file path + ±counts); otherwise the
        // usual session/launcher name. Navigation lives entirely in the footer now, so
        // the title bar carries no tap targets.
        let title: Line = match &self.diff {
            Some(v) => diff_title(v),
            None => Line::from(self.main_title(nav)),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border));
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.regions.main = Some(area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        self.regions.main_inner = Some(inner);

        // The diff preview takes over the pane as a read-only pager — no PTY resize,
        // no drag-to-copy (it isn't a vt100 grid). An image file shows its picture
        // instead of a textual diff.
        if let Some(v) = self.diff.as_mut() {
            if let Some(img) = v.image.as_mut() {
                // Real pixels via sixel where the terminal supports it (the encoded
                // picture is stashed and painted on top of this frame in `run_loop`);
                // otherwise the half-block fallback drawn straight into the buffer.
                if self.sixel {
                    match img.sixel(inner.width, inner.height, self.cell_px) {
                        Some(data) => self.pending_sixel = Some((inner, data.to_string())),
                        None => render_image(f, inner, img),
                    }
                } else {
                    render_image(f, inner, img);
                }
            } else {
                render_diff(f, inner, v);
            }
            return;
        }

        self.resize_current(inner.height, inner.width);

        match nav.and_then(|n| self.pane_at(n)) {
            Some(pane) => render_screen(f, inner, pane, focus == Focus::Terminal),
            None => render_placeholder(f, inner, &self.placeholder_text(nav)),
        }
        self.paint_selection(f, inner, SelTarget::Main);
    }

    /// The right column: the active project's native git panel (changed files,
    /// staging, recent commits). A plain placeholder when the project isn't a repo.
    pub(crate) fn render_right(&mut self, f: &mut Frame, area: Rect) {
        self.regions.right = Some(area);
        let focused = self.focus == Focus::Right;
        let hits = match self.active_git() {
            Some(g) => super::git::render_git(f, area, g, focused),
            None => {
                let border = if focused { Color::Magenta } else { Color::DarkGray };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(" git ")
                    .border_style(Style::default().fg(border));
                let inner = block.inner(area);
                f.render_widget(block, area);
                render_placeholder(f, inner, "Not a git repository.");
                super::git::GitRows::default()
            }
        };
        self.regions.git_rows = hits.rows;
        self.regions.git_branch_rows = hits.branches;
        self.regions.git_changes = hits.changes_area;
        self.regions.git_branches = hits.branches_area;
    }

    /// Paint the active drag selection (if it targets this pane and has actually
    /// moved) as a reverse-video overlay on top of the just-rendered screen. The
    /// selection is in buffer coordinates, so project each line to a viewport row at
    /// the pane's current scrollback offset and skip whatever scrolled out of view.
    fn paint_selection(&self, f: &mut Frame, inner: Rect, target: SelTarget) {
        let Some(sel) = self.drag else { return };
        if !sel.moved || sel.target != target {
            return;
        }
        let off = self
            .current_nav()
            .and_then(|n| self.pane_at(n))
            .map(|p| p.scrollback_offset())
            .unwrap_or(0) as i32;
        let (lo, sc, hi, ec) = sel.ordered();
        let style = Style::default().add_modifier(Modifier::REVERSED);
        let last = inner.width.saturating_sub(1);
        let height = inner.height as i32;
        let buf = f.buffer_mut();
        for line in lo..=hi {
            let sr = line + off; // viewport row of this buffer line right now
            if sr < 0 || sr >= height {
                continue;
            }
            let row = sr as u16;
            let (c0, c1) = if lo == hi {
                (sc, ec)
            } else if line == lo {
                (sc, last)
            } else if line == hi {
                (0, ec)
            } else {
                (0, last)
            };
            for col in c0..=c1.min(last) {
                if let Some(cell) = buf.cell_mut((inner.x + col, inner.y + row)) {
                    cell.set_style(style);
                }
            }
        }
    }

    pub(crate) fn main_title(&self, nav: Option<Nav>) -> String {
        let multi = self.projects.len() > 1;
        match nav {
            Some(Nav::Session(i)) => format!(
                " {} — {} ",
                self.sessions[i].name,
                status_label(self.sessions[i].status())
            ),
            Some(Nav::NewAgent(p, t)) => {
                let a = &self.projects[p].cfg.agents[t].name;
                if multi {
                    format!(" New {a} · {} ", self.projects[p].cfg.display_name())
                } else {
                    format!(" New {a} ")
                }
            }
            Some(Nav::NewTerminal(p)) => {
                if multi {
                    format!(" New Terminal · {} ", self.projects[p].cfg.display_name())
                } else {
                    " New Terminal ".into()
                }
            }
            Some(Nav::NewProcess(p)) => {
                if multi {
                    format!(" New Process · {} ", self.projects[p].cfg.display_name())
                } else {
                    " New Process ".into()
                }
            }
            Some(Nav::Panel) => " git ".into(),
            None => " mmux ".into(),
        }
    }

    pub(crate) fn placeholder_text(&self, nav: Option<Nav>) -> String {
        match nav {
            Some(Nav::NewAgent(p, t)) => {
                format!("Press Enter to launch a new {}.", self.projects[p].cfg.agents[t].name)
            }
            Some(Nav::NewTerminal(_)) => "Press Enter to open a new terminal.".into(),
            Some(Nav::NewProcess(_)) => {
                "Press Enter to define a new process and save it to mmux.yaml.".into()
            }
            Some(Nav::Session(i)) => {
                let s = &self.sessions[i];
                if let Some(e) = &s.error {
                    let verb = if s.kind == Kind::Terminal { "open" } else { "start" };
                    return format!("Failed to {verb} {}:\n\n{e}", s.name);
                }
                match s.kind {
                    Kind::Process => {
                        format!("{} is stopped.\n\nPress Enter or 's' to start it.", s.name)
                    }
                    Kind::Terminal => {
                        format!("{} has no live terminal.\n\nPress Enter or 'r' to reopen.", s.name)
                    }
                    _ => format!("{} has no live terminal.\n\nPress Enter or 'r' to restart.", s.name),
                }
            }
            Some(Nav::Panel) => "Git panel — press Enter to open it.".into(),
            None => "No agents or processes configured.\nEdit mmux.yaml and reopen.".into(),
        }
    }
}

/// Render a live pane's vt100 screen into `area`. On the focused pane we suppress
/// tui-term's overlay cursor and place the host's real (hardware) cursor instead:
/// a REVERSED overlay cell under an inverting hardware cursor double-inverts into
/// an invisible "black on black" cursor.
fn render_screen(f: &mut Frame, area: Rect, pane: &Pane, focused: bool) {
    pane.with_screen(|screen| {
        let mut term = PseudoTerminal::new(screen);
        if focused {
            term = term.cursor(Cursor::default().visibility(false));
        }
        f.render_widget(term, area);
        if focused && !screen.hide_cursor() {
            let (crow, ccol) = screen.cursor_position();
            let cx = area.x + ccol.min(area.width.saturating_sub(1));
            let cy = area.y + crow.min(area.height.saturating_sub(1));
            f.set_cursor_position((cx, cy));
        }
    });
}

/// The diff pane's title: ` Δ path/to/file.rs  +12 −3 `, the counts coloured like
/// the diff body. For an image preview it's ` ▦ path/to/logo.png  1200×800 ` instead.
fn diff_title(v: &DiffView) -> Line<'static> {
    if let Some(img) = &v.image {
        return Line::from(vec![
            Span::raw(" ▦ "),
            Span::styled(
                v.path.clone(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}×{} ", img.dims.0, img.dims.1),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
    }
    let mut spans = Vec::new();
    spans.push(Span::raw(" Δ "));
    spans.push(Span::styled(
        v.path.clone(),
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(format!("  +{}", v.added), Style::default().fg(Color::Green)));
    spans.push(Span::styled(format!(" −{} ", v.removed), Style::default().fg(Color::Red)));
    Line::from(spans)
}

/// Render the diff body as a scrolled, colour-coded pager: additions green,
/// deletions red, `@@` hunk headers cyan, context grey.
fn render_diff(f: &mut Frame, area: Rect, v: &DiffView) {
    if v.lines.is_empty() {
        render_placeholder(f, area, "No textual diff — new, empty, or binary file.");
        return;
    }
    let lines: Vec<Line> = v.lines.iter().map(diff_line).collect();
    f.render_widget(Paragraph::new(lines).scroll((v.scroll as u16, 0)), area);
}

/// Paint a decoded image into `area` as centred half-block cells (`▀`, top pixel as
/// foreground, bottom as background), written straight into the buffer — the same
/// technique as [`App::paint_selection`], so it needs no terminal graphics protocol and
/// passes cleanly through the tmux jail. The grid is aspect-fit and may be smaller than
/// `area`, so it's centred; leftover cells keep the pane background.
fn render_image(f: &mut Frame, area: Rect, img: &mut PreviewImage) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let grid = img.grid(area.width, area.height);
    let Some(first) = grid.first() else {
        render_placeholder(f, area, "Could not render image.");
        return;
    };
    let cols = first.len() as u16;
    let rows = grid.len() as u16;
    let pad_x = area.width.saturating_sub(cols) / 2;
    let pad_y = area.height.saturating_sub(rows) / 2;
    let buf = f.buffer_mut();
    for (ry, line) in grid.iter().enumerate() {
        let y = area.y + pad_y + ry as u16;
        for (rx, cell) in line.iter().enumerate() {
            if let Some(c) = buf.cell_mut((area.x + pad_x + rx as u16, y)) {
                c.set_char('▀');
                c.set_fg(Color::Rgb(cell.top.0, cell.top.1, cell.top.2));
                c.set_bg(Color::Rgb(cell.bottom.0, cell.bottom.1, cell.bottom.2));
            }
        }
    }
}

fn diff_line(l: &DiffLine) -> Line<'static> {
    let color = match l.kind {
        DiffKind::Add => Color::Green,
        DiffKind::Del => Color::Red,
        DiffKind::Hunk => Color::Cyan,
        DiffKind::Context => Color::Gray,
    };
    Line::from(Span::styled(l.text.clone(), Style::default().fg(color)))
}

fn render_placeholder(f: &mut Frame, area: Rect, msg: &str) {
    f.render_widget(
        Paragraph::new(msg.to_string())
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
