//! The left sidebar: section headers plus one row per nav entry. Every row is
//! built by [`App::nav_row`], so adding a row kind is a single match arm.

use super::theme::{
    agent_glyph_style, badge, entry_line, header, project_header, status_style, ACTIVE_BORDER,
    IDLE_BORDER, SPINNER,
};
use crate::app::nav::Nav;
use crate::app::session::Kind;
use crate::app::{App, Focus, Status};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

#[derive(Default)]
struct AgentActivity {
    working: usize,
    ready: usize,
    failed: usize,
}

impl App {
    pub(crate) fn render_sidebar(&mut self, f: &mut Frame, area: Rect) {
        // The whole left column routes clicks to the sidebar.
        self.regions.sidebar = Some(area);
        if self.projects.len() > 1 {
            self.render_sidebar_projects(f, area);
        } else {
            self.render_sidebar_single(f, area);
        }
    }

    /// One bordered box per project, stacked top-to-bottom. The active box expands to
    /// fill the available height; inactive boxes stay collapsed. In compact (phone)
    /// mode the git panel — which has no column of its own there — is appended as a
    /// final box so it stays reachable. Works in wide and compact alike.
    fn render_sidebar_projects(&mut self, f: &mut Frame, area: Rect) {
        self.regions.rows.clear();
        let nav = self.build_nav();
        // Border columns + one content-padding cell on each side.
        let inner_w = area.width.saturating_sub(4);
        let order = self.project_display_order();
        let active_pos = order.iter().position(|&pi| pi == self.active).unwrap_or(0);

        // The boxes to stack: agent-active projects plus the sticky selected project
        // first (stable within each activity bucket), then the compact-only git box.
        // Keep the real project index in the tuple because display order no longer
        // equals `App.projects` order.
        let mut blocks: Vec<(String, bool, Option<usize>, Vec<Line>, Vec<(u16, usize)>)> = order
            .into_iter()
            .map(|pi| {
                let active = pi == self.active;
                let (lines, rows) = if active {
                    self.project_lines(pi, &nav, inner_w)
                } else {
                    (self.collapsed_project_lines(pi, inner_w), Vec::new())
                };
                (
                    self.projects[pi].cfg.display_name(),
                    active,
                    Some(pi),
                    lines,
                    rows,
                )
            })
            .collect();
        if self.compact && self.active_git().is_some() {
            if let Some(pos) = nav.iter().position(|n| matches!(n, Nav::Panel)) {
                let row = self.nav_row(pos, Nav::Panel, inner_w);
                blocks.push(("git".to_string(), false, None, vec![row], vec![(0, pos)]));
            }
        }

        // Inactive repos need one extra row for branch + change state; non-git
        // projects stay at one summary row. The active project gets everything left.
        let collapsed_heights: Vec<u16> = blocks
            .iter()
            .map(|(_, _, _, lines, _)| lines.len() as u16 + 2)
            .collect();
        let heights = box_heights(&collapsed_heights, active_pos, area.height);
        let chunks = Layout::vertical(heights.iter().map(|h| Constraint::Length(*h))).split(area);

        for (i, (name, active, project, lines, rows)) in blocks.into_iter().enumerate() {
            let rect = chunks[i];
            if rect.height == 0 {
                continue;
            }
            let title_style = if active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let border = if active { ACTIVE_BORDER } else { IDLE_BORDER };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(format!(" {name} "), title_style))
                .border_style(Style::default().fg(border));
            let inner = padded_inner(block.inner(rect));
            f.render_widget(block, rect);
            // Remember the actual project index so clicks stay correct after the
            // activity-based display ordering. The trailing git box has no project.
            if let Some(pi) = project {
                self.regions.project_boxes.push((rect, pi));
            }
            // Map each row's local line index to an absolute screen `y` for click
            // routing, skipping any the box is too short to actually show.
            for (ly, pos) in rows {
                let ry = inner.y + ly;
                if ry < inner.y + inner.height {
                    self.regions.rows.push((ry, pos));
                }
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
    }

    fn agent_activity(&self, pi: usize) -> AgentActivity {
        let mut activity = AgentActivity::default();
        for s in self
            .sessions
            .iter()
            .filter(|s| s.project == pi && s.kind == Kind::Agent)
        {
            if s.is_running() {
                if s.busy() {
                    activity.working += 1;
                } else {
                    activity.ready += 1;
                }
            } else if matches!(s.status(), Status::Failed) || s.error.is_some() {
                activity.failed += 1;
            }
        }
        activity
    }

    /// Compact inactive-project content: an agent-activity row when there is any,
    /// plus — only for a git repo — the current branch and changed-path count.
    fn collapsed_project_lines(&self, pi: usize, width: u16) -> Vec<Line<'static>> {
        let activity = self.agent_activity(pi);

        let mut spans: Vec<Span<'static>> = Vec::new();
        if activity.working > 0 {
            spans.push(Span::styled(
                format!("{} {} working", self.spinner(), activity.working),
                Style::default().fg(Color::Gray),
            ));
        }
        if activity.ready > 0 {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("● {} ready", activity.ready),
                Style::default()
                    .fg(super::theme::ATTN)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if activity.failed > 0 {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("○ {} failed", activity.failed),
                Style::default().fg(Color::Red),
            ));
        }
        let mut lines = Vec::new();
        if !spans.is_empty() {
            lines.push(Line::from(spans));
        }

        let Some(g) = self.projects[pi].git.as_ref() else {
            return lines;
        };

        let (git, git_style) = if g.files.is_empty() {
            ("git ✓".to_string(), Style::default().fg(Color::DarkGray))
        } else {
            (
                format!("git ±{}", g.files.len()),
                Style::default().fg(Color::Yellow),
            )
        };
        let git_w = git.chars().count();
        let branch_w = (width as usize).saturating_sub(git_w + 1);
        let branch = if branch_w == 0 {
            String::new()
        } else {
            super::git::truncate_middle(
                if g.branch.is_empty() {
                    "HEAD"
                } else {
                    &g.branch
                },
                branch_w,
            )
        };
        let mut line = Line::from(Span::styled(branch, Style::default().fg(Color::Magenta)));
        let pad = (width as usize).saturating_sub(line.width() + git_w);
        line.spans.push(Span::raw(" ".repeat(pad)));
        line.spans.push(Span::styled(git, git_style));
        lines.push(line);
        lines
    }

    /// Build one project's AGENTS/TERMINAL/PROCESSES lines (no project header — the
    /// box title carries the name) plus each row's line index within the box mapped
    /// to its global nav position, for click routing.
    fn project_lines(
        &self,
        pi: usize,
        nav: &[Nav],
        width: u16,
    ) -> (Vec<Line<'static>>, Vec<(u16, usize)>) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut rows: Vec<(u16, usize)> = Vec::new();
        self.push_proj_section(
            &mut lines,
            &mut rows,
            "AGENTS",
            true,
            nav,
            width,
            move |app, n| section_matches(app, n, pi, Kind::Agent),
        );
        self.push_proj_section(
            &mut lines,
            &mut rows,
            "TERMINAL",
            false,
            nav,
            width,
            move |app, n| section_matches(app, n, pi, Kind::Terminal),
        );
        self.push_proj_section(
            &mut lines,
            &mut rows,
            "PROCESSES",
            false,
            nav,
            width,
            move |app, n| section_matches(app, n, pi, Kind::Process),
        );
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
            .border_style(Style::default().fg(IDLE_BORDER));
        let inner = padded_inner(block.inner(area));
        f.render_widget(block, area);

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
                lines.push(project_header(
                    &self.projects[pi].cfg.display_name(),
                    pi == self.active,
                    inner.width,
                ));
                y += 1;
            }
            self.section(
                &mut lines,
                &mut y,
                "AGENTS",
                true,
                &nav,
                inner.width,
                move |app, n| section_matches(app, n, pi, Kind::Agent),
            );
            self.section(
                &mut lines,
                &mut y,
                "TERMINAL",
                false,
                &nav,
                inner.width,
                move |app, n| section_matches(app, n, pi, Kind::Terminal),
            );
            self.section(
                &mut lines,
                &mut y,
                "PROCESSES",
                false,
                &nav,
                inner.width,
                move |app, n| section_matches(app, n, pi, Kind::Process),
            );
        }
        // In compact mode the git panel is also a sidebar entry.
        if self.compact && self.active_git().is_some() {
            self.section(
                &mut lines,
                &mut y,
                "GIT",
                false,
                &nav,
                inner.width,
                |_, n| matches!(n, Nav::Panel),
            );
        }

        f.render_widget(Paragraph::new(lines), inner);
    }

    /// The sidebar block title: the launch directory's display name. For a manifest
    /// this is the workspace name, not the first member project's name.
    fn root_title(&self) -> String {
        self.root_cfg().display_name()
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

    /// The current frame of the working spinner. Time-based (200 ms/frame, a full
    /// turn every ~0.8 s) rather than frame-counted, so every agent's spinner rotates
    /// in step and at a steady speed no matter how often the UI happens to repaint.
    pub(crate) fn spinner(&self) -> &'static str {
        let i = (self.start.elapsed().as_millis() / 200 % SPINNER.len() as u128) as usize;
        SPINNER[i]
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
                match s.kind {
                    // Processes keep the "is it up" model: a status badge plus a
                    // green-when-running name, with the bell as a trailing dot.
                    Kind::Process => entry_line(
                        &format!("{} {}", badge(s.status()), s.name),
                        sel,
                        status_style(s.status()),
                        s.subtitle().as_deref(),
                        s.attention(),
                        width,
                    ),
                    // Agents/terminals: the leading glyph + name color carry the whole
                    // state (busy → gray spinner, needs-you → green `●`, stopped → dim
                    // `○`), so there's no separate trailing dot.
                    //
                    // An *agent* "needs you" when it's running but its terminal title has
                    // gone static: it animates the title while working, so a quiet title
                    // means it's idle/awaiting input. This is the agent's actual state, so
                    // it holds even while you're viewing the pane — selecting an idle agent
                    // must not make it look like it's working again. A *terminal* has no
                    // such signal, so it falls back to the bell, which (being a momentary
                    // ping) is acknowledged — suppressed — on the pane you're viewing.
                    _ => {
                        let attn = match s.kind {
                            Kind::Agent => s.is_running() && !s.busy(),
                            _ => s.attention() && !(sel && self.focus == Focus::Terminal),
                        };
                        // A busy agent gets the rotating spinner before its name; a
                        // terminal has no "working" notion, so it keeps a static dot.
                        let working = match s.kind {
                            Kind::Agent => self.spinner(),
                            _ => "·",
                        };
                        let (glyph, base) =
                            agent_glyph_style(s.status(), attn, s.error.is_some(), working);
                        entry_line(
                            &format!("{glyph} {}", s.name),
                            sel,
                            base,
                            s.subtitle().as_deref(),
                            false,
                            width,
                        )
                    }
                }
            }
            Nav::Panel => {
                let branch = self
                    .active_git()
                    .map(|g| g.branch.clone())
                    .unwrap_or_default();
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

/// Does nav entry `n` belong in project `pi`'s `kind` section? The launcher row for that
/// kind (`+ New …`) plus every session of that project and kind. This is the one matcher
/// shared by the single- and multi-project sidebar layouts (which otherwise track rows in
/// different coordinate systems).
fn section_matches(app: &App, n: Nav, pi: usize, kind: Kind) -> bool {
    match n {
        Nav::NewAgent(p, _) if kind == Kind::Agent => p == pi,
        Nav::NewTerminal(p) if kind == Kind::Terminal => p == pi,
        Nav::NewProcess(p) if kind == Kind::Process => p == pi,
        Nav::Session(i) => app.sessions[i].project == pi && app.sessions[i].kind == kind,
        _ => false,
    }
}

/// One cell of breathing room between sidebar content and each vertical border.
/// Applied to both the single-project drawer and every workspace project box so
/// headers, rows, collapsed summaries, and their right-aligned git state agree.
fn padded_inner(area: Rect) -> Rect {
    let pad = (area.width / 2).min(1);
    Rect {
        x: area.x + pad,
        width: area.width.saturating_sub(pad * 2),
        ..area
    }
}

/// Vertical heights for the stacked per-project boxes: every inactive box gets its
/// compact content height (one row, or two with git), while the active box absorbs
/// the remaining space. If the sidebar is too short, split it evenly and give the
/// remainder to the active box so the selected project remains the most visible one.
fn box_heights(collapsed_heights: &[u16], active: usize, total: u16) -> Vec<u16> {
    const MIN_BOX: u16 = 3; // top border + ≥1 row + bottom border
    let n = collapsed_heights.len();
    if n == 0 {
        return Vec::new();
    }
    let active = active.min(n - 1);
    let collapsed: u16 = collapsed_heights
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != active)
        .map(|(_, height)| *height)
        .sum();
    if collapsed + MIN_BOX <= total {
        let mut h = collapsed_heights.to_vec();
        h[active] = total - collapsed;
        return h;
    }

    let each = total / n as u16;
    let mut h = vec![each; n];
    h[active] += total % n as u16;
    h
}
