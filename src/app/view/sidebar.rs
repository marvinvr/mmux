//! The left sidebar: section headers plus one row per nav entry. Every row is
//! built by [`App::nav_row`], so adding a row kind is a single match arm.

use super::theme::{badge, entry_line, header, project_header, status_style};
use crate::app::nav::Nav;
use crate::app::session::Kind;
use crate::app::{App, Focus};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

impl App {
    pub(crate) fn render_sidebar(&mut self, f: &mut Frame, area: Rect) {
        // A multi-project workspace becomes one bordered box per project, stacked
        // vertically: the active project's box expands to fill the column, the others
        // shrink to just their rows. This holds in both wide and compact (phone) mode.
        // A single project keeps the original single-box drawer.
        if self.projects.len() > 1 {
            self.render_sidebar_projects(f, area);
        } else {
            self.render_sidebar_single(f, area);
        }
    }

    /// One bordered box per project, stacked top-to-bottom. The active box expands to
    /// fill the leftover height; inactive boxes are content-sized (no whitespace). In
    /// compact (phone) mode the git panel — which has no column of its own there — is
    /// appended as a final box so it stays reachable. Works in wide and compact alike.
    fn render_sidebar_projects(&mut self, f: &mut Frame, area: Rect) {
        self.regions.sidebar = Some(area);
        self.regions.rows.clear();
        let nav = self.build_nav();
        let n = self.projects.len();
        let inner_w = area.width.saturating_sub(2); // minus the box's two border columns

        // The boxes to stack: one per project, then (compact only) a git box. Each is
        // (title, is_active_project, lines, row map).
        let mut blocks: Vec<(String, bool, Vec<Line>, Vec<(u16, usize)>)> = (0..n)
            .map(|pi| {
                let (lines, rows) = self.project_lines(pi, &nav, inner_w);
                (self.projects[pi].cfg.display_name(), pi == self.active, lines, rows)
            })
            .collect();
        if self.compact && self.active_git().is_some() {
            if let Some(pos) = nav.iter().position(|n| matches!(n, Nav::Panel)) {
                let row = self.nav_row(pos, Nav::Panel, inner_w);
                blocks.push(("git".to_string(), false, vec![row], vec![(0, pos)]));
            }
        }

        // The active project's box absorbs the slack; everything else is content-sized.
        let content_h: Vec<u16> = blocks.iter().map(|(_, _, l, _)| l.len() as u16 + 2).collect();
        let heights = box_heights(&content_h, self.active, area.height);
        let chunks = Layout::vertical(heights.iter().map(|h| Constraint::Length(*h))).split(area);

        for (i, (name, active, lines, rows)) in blocks.into_iter().enumerate() {
            let rect = chunks[i];
            if rect.height == 0 {
                continue;
            }
            let title_style = if active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let border = if active { Color::Cyan } else { Color::DarkGray };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(format!(" {name} "), title_style))
                .border_style(Style::default().fg(border));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            // Remember each project box's area so a click in its whitespace switches
            // projects (the trailing git box, index >= n, is not a project).
            if i < n {
                self.regions.project_boxes.push((rect, i));
            }
            // Map each row's local line index to an absolute screen `y` for click
            // routing, skipping any the box is too short to actually show.
            for (ly, pos) in rows {
                let y = inner.y + ly;
                if y < inner.y + inner.height {
                    self.regions.rows.push((y, pos));
                }
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
    }

    /// Build one project's AGENTS/TERMINAL/PROCESSES lines (no project header — the
    /// box title carries the name) plus each row's line index within the box mapped
    /// to its global nav position, for click routing.
    fn project_lines(&self, pi: usize, nav: &[Nav], width: u16) -> (Vec<Line<'static>>, Vec<(u16, usize)>) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut rows: Vec<(u16, usize)> = Vec::new();
        self.push_proj_section(&mut lines, &mut rows, "AGENTS", true, nav, width, move |app, n| match n {
            Nav::NewAgent(p, _) => p == pi,
            Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Agent,
            _ => false,
        });
        self.push_proj_section(&mut lines, &mut rows, "TERMINAL", false, nav, width, move |app, n| match n {
            Nav::NewTerminal(p) => p == pi,
            Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Terminal,
            _ => false,
        });
        self.push_proj_section(&mut lines, &mut rows, "PROCESSES", false, nav, width, move |app, n| match n {
            Nav::NewProcess(p) => p == pi,
            Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Process,
            _ => false,
        });
        (lines, rows)
    }

    /// Like [`Self::section`], but writes into caller-owned `lines`/`rows` (row
    /// indices local to the box) instead of the shared per-frame state.
    fn push_proj_section(
        &self,
        lines: &mut Vec<Line<'static>>,
        rows: &mut Vec<(u16, usize)>,
        title: &str,
        first: bool,
        nav: &[Nav],
        width: u16,
        want: impl Fn(&App, Nav) -> bool,
    ) {
        if !first {
            lines.push(Line::from(""));
        }
        lines.push(header(title));
        for (pos, n) in nav.iter().enumerate() {
            if want(self, *n) {
                rows.push((lines.len() as u16, pos));
                lines.push(self.nav_row(pos, *n, width));
            }
        }
    }

    fn render_sidebar_single(&mut self, f: &mut Frame, area: Rect) {
        let title = self.root_title();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.regions.sidebar = Some(area);
        // In compact mode, the drawer gets a button on the top-right to open the panel.
        if self.compact && self.active_git().is_some() {
            let half = area.width / 2;
            let rz = Rect {
                x: area.x + half,
                y: area.y,
                width: area.width - half,
                height: 1,
            };
            self.regions.panel_btn = Some(self.draw_panel_button(f, rz));
        }

        self.regions.rows.clear();
        let nav = self.build_nav();
        let mut lines: Vec<Line> = Vec::new();
        let mut y = inner.y;

        // One group of sections per project. With a single project we drop the
        // project header entirely so the layout reads exactly as it did before.
        let multi = self.projects.len() > 1;
        for pi in 0..self.projects.len() {
            if multi {
                if !lines.is_empty() {
                    lines.push(Line::from(""));
                    y += 1;
                }
                lines.push(project_header(&self.projects[pi].cfg.display_name(), pi == self.active, inner.width));
                y += 1;
            }
            self.section(&mut lines, &mut y, "AGENTS", true, &nav, inner.width, move |app, n| match n {
                Nav::NewAgent(p, _) => p == pi,
                Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Agent,
                _ => false,
            });
            self.section(&mut lines, &mut y, "TERMINAL", false, &nav, inner.width, move |app, n| match n {
                Nav::NewTerminal(p) => p == pi,
                Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Terminal,
                _ => false,
            });
            self.section(&mut lines, &mut y, "PROCESSES", false, &nav, inner.width, move |app, n| match n {
                Nav::NewProcess(p) => p == pi,
                Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == Kind::Process,
                _ => false,
            });
        }
        // In compact mode the git panel is also a sidebar entry.
        if self.compact && self.active_git().is_some() {
            self.section(&mut lines, &mut y, "GIT", false, &nav, inner.width, |_, n| {
                matches!(n, Nav::Panel)
            });
        }

        f.render_widget(Paragraph::new(lines), inner);
    }

    /// The sidebar block title: the root (launch) project's display name.
    fn root_title(&self) -> String {
        self.projects[0].cfg.display_name()
    }

    /// Emit a header and every nav entry for which `want` is true, recording each
    /// row's screen `y` so clicks can be mapped back to a nav index. `first` marks
    /// the first section of its group (no leading blank line).
    fn section(
        &mut self,
        lines: &mut Vec<Line<'static>>,
        y: &mut u16,
        title: &str,
        first: bool,
        nav: &[Nav],
        width: u16,
        want: impl Fn(&App, Nav) -> bool,
    ) {
        if !first {
            lines.push(Line::from(""));
            *y += 1;
        }
        lines.push(header(title));
        *y += 1;
        for (pos, n) in nav.iter().enumerate() {
            if want(self, *n) {
                lines.push(self.nav_row(pos, *n, width));
                self.regions.rows.push((*y, pos));
                *y += 1;
            }
        }
    }

    /// Build the styled line for nav entry `n` at nav position `pos`. `width` is the
    /// sidebar's inner width, used to stretch the selection highlight full-width.
    fn nav_row(&self, pos: usize, n: Nav, width: u16) -> Line<'static> {
        let sel = self.sel == pos;
        match n {
            Nav::NewAgent(p, t) => entry_line(
                &format!("+ New {}", self.projects[p].cfg.agents[t].name),
                sel,
                Style::default().fg(Color::Green),
                None,
                false,
                width,
            ),
            Nav::NewTerminal(_) => entry_line(
                "+ New Terminal",
                sel,
                Style::default().fg(Color::Green),
                None,
                false,
                width,
            ),
            Nav::NewProcess(_) => entry_line(
                "+ New Process",
                sel,
                Style::default().fg(Color::Green),
                None,
                false,
                width,
            ),
            Nav::Session(i) => {
                let s = &self.sessions[i];
                let label = match s.kind {
                    Kind::Process => format!("{} {}", badge(s.status()), s.name),
                    _ => s.name.clone(),
                };
                // The bell is suppressed on the agent/terminal you're actively viewing.
                let attn = match s.kind {
                    Kind::Agent | Kind::Terminal => {
                        s.attention() && !(sel && self.focus == Focus::Terminal)
                    }
                    _ => s.attention(),
                };
                entry_line(&label, sel, status_style(s.status()), s.subtitle().as_deref(), attn, width)
            }
            Nav::Panel => {
                let branch = self.active_git().map(|g| g.branch.clone()).unwrap_or_default();
                entry_line(
                    "git",
                    sel,
                    Style::default().fg(Color::Magenta),
                    (!branch.is_empty()).then_some(branch.as_str()),
                    false,
                    width,
                )
            }
        }
    }
}

/// Vertical heights for the stacked per-project boxes: inactive boxes keep their
/// content height, the active box expands to absorb the remaining space. The result
/// always sums to `total` (so the active box runs to the bottom, no dead space). If
/// the content can't all fit, the active box keeps a minimum and inactive boxes are
/// trimmed from the bottom.
fn box_heights(content: &[u16], active: usize, total: u16) -> Vec<u16> {
    const MIN_BOX: u16 = 3; // top border + ≥1 row + bottom border
    let n = content.len();
    let mut h: Vec<u16> = content.iter().map(|&c| c.max(MIN_BOX)).collect();
    if n == 0 || total == 0 {
        return h;
    }
    let inactive: u16 = (0..n).filter(|&i| i != active).map(|i| h[i]).sum();
    if inactive + MIN_BOX <= total {
        h[active] = total - inactive; // active fills the rest
        return h;
    }
    // Doesn't all fit: the active box keeps a minimum, inactive boxes give back
    // height from the bottom until it fits.
    h[active] = MIN_BOX.min(total);
    let mut over = (inactive + h[active]).saturating_sub(total);
    for i in (0..n).rev() {
        if over == 0 {
            break;
        }
        if i != active {
            let cut = over.min(h[i].saturating_sub(MIN_BOX));
            h[i] -= cut;
            over -= cut;
        }
    }
    if over > 0 {
        h[active] = h[active].saturating_sub(over).max(1);
    }
    h
}
