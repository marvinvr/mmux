//! The left sidebar: section headers plus one row per nav entry. Every row is
//! built by [`App::nav_row`], so adding a row kind is a single match arm.

use super::theme::{
    agent_glyph_style, badge, entry_line, header, project_header, status_style, ACTIVE_BORDER,
    IDLE_BORDER, SPINNER, WORKTREE_ACTIVE_BORDER, WORKTREE_BORDER,
};
use crate::app::nav::Nav;
use crate::app::session::Kind;
use crate::app::{App, Focus, Status};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
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

    /// One bordered box per visible project, stacked top-to-bottom. Wide mode shows
    /// every project with the active one expanded. Compact mode shows only the active
    /// project; its footer opens the project picker for the rest. The compact git panel
    /// is appended as a final box so it stays reachable.
    fn render_sidebar_projects(&mut self, f: &mut Frame, area: Rect) {
        self.regions.rows.clear();
        let nav = self.build_nav();
        // Border columns + one content-padding cell on each side.
        let inner_w = area.width.saturating_sub(4);
        let order = if self.compact {
            vec![self.active]
        } else {
            self.project_display_order()
        };
        let active_pos = order.iter().position(|&pi| pi == self.active).unwrap_or(0);

        // Projects with agent rows come first; a selected project whose last agent
        // just closed stays in that bucket. The compact-only git box follows them.
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
                // A worktree is titled by its branch, since the directory it lives in
                // is a hash under `~/.mmux` that nobody thinks in.
                (self.project_label(pi), active, Some(pi), lines, rows)
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
        // When the column fits, the active box absorbs the slack exactly as it always
        // has. When it doesn't, every box keeps its natural height and the column
        // becomes taller than the viewport — which is what there is to scroll.
        let natural: u16 = collapsed_heights.iter().sum();
        let (heights, column_h) = if natural <= area.height {
            (
                box_heights(&collapsed_heights, active_pos, area.height),
                area.height,
            )
        } else {
            (collapsed_heights, natural)
        };

        // Lay the whole column out off-screen at full height, then blit the visible
        // window. Rendering into the frame directly can't express "this box starts
        // above the top edge", and clipping every widget by hand would put the same
        // arithmetic in five places.
        let column = Rect {
            x: 0,
            y: 0,
            width: area.width,
            height: column_h,
        };
        let chunks = Layout::vertical(heights.iter().map(|h| Constraint::Length(*h))).split(column);
        let mut scratch = Buffer::empty(column);
        // Hit rects, gathered in column coordinates and translated once at the end.
        let mut col_rows: Vec<(u16, usize)> = Vec::new();
        let mut col_boxes: Vec<(Rect, usize)> = Vec::new();

        for (i, (name, active, project, lines, rows)) in blocks.into_iter().enumerate() {
            let mut rect = chunks[i];
            if rect.height == 0 {
                continue;
            }
            let worktree = project
                .map(|pi| self.projects[pi].worktree.is_some())
                .unwrap_or(false);
            // Indent a worktree by a column so it reads as hanging off the box above
            // it. Everything downstream (inner area, click regions) is derived from
            // this rect, so the inset carries through on its own.
            if worktree && rect.width > 2 {
                rect.x += 1;
                rect.width -= 1;
            }
            let title_style = if active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if worktree {
                Style::default().fg(WORKTREE_BORDER)
            } else {
                Style::default().fg(Color::Gray)
            };
            let border = match (worktree, active) {
                (true, true) => WORKTREE_ACTIVE_BORDER,
                (true, false) => WORKTREE_BORDER,
                (false, true) => ACTIVE_BORDER,
                (false, false) => IDLE_BORDER,
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(format!(" {name} "), title_style))
                .border_style(Style::default().fg(border));
            let inner = padded_inner(block.inner(rect));
            block.render(rect, &mut scratch);
            // Remember the actual project index so clicks stay correct after the
            // activity-based display ordering. The trailing git box has no project.
            if let Some(pi) = project {
                col_boxes.push((rect, pi));
            }
            // Map each row's local line index to a column `y` for click routing,
            // skipping any the box is too short to actually show.
            for (ly, pos) in rows {
                let ry = inner.y + ly;
                if ry < inner.y + inner.height {
                    col_rows.push((ry, pos));
                }
            }
            Paragraph::new(lines).render(inner, &mut scratch);
        }

        let selected_y = col_rows
            .iter()
            .find(|(_, pos)| *pos == self.sel)
            .map(|(y, _)| *y);
        self.settle_sidebar_scroll(column_h, area.height, selected_y);
        let off = self.sidebar_scroll;

        blit_window(&scratch, f.buffer_mut(), area, off);
        for (y, pos) in col_rows {
            if let Some(sy) = visible_y(y, area, off) {
                self.regions.rows.push((sy, pos));
            }
        }
        // A partly-scrolled box stays clickable over the part you can see.
        for (rect, pi) in col_boxes {
            if let Some(rect) = clip_to_window(rect, area, off) {
                self.regions.project_boxes.push((rect, pi));
            }
        }
    }

    /// Keep [`sidebar_scroll`](App) sane, and bring the selected row into view **when
    /// the selection moves**. The distinction is what makes the wheel usable: browsing
    /// away from the cursor has to stay put rather than snapping back every frame, but
    /// pressing `j`/`k` (or anything else that moves the cursor) should always follow.
    fn settle_sidebar_scroll(&mut self, column_h: u16, view_h: u16, selected_y: Option<u16>) {
        if self.sel != self.sidebar_scroll_sel {
            self.sidebar_scroll_sel = self.sel;
            if let Some(y) = selected_y {
                if y < self.sidebar_scroll {
                    self.sidebar_scroll = y;
                } else if y >= self.sidebar_scroll + view_h {
                    self.sidebar_scroll = y + 1 - view_h;
                }
            }
        }
        // The column shrinks as boxes collapse and sessions close, so an offset that
        // was fine last frame can be past the end of this one.
        self.sidebar_scroll = self.sidebar_scroll.min(column_h.saturating_sub(view_h));
    }

    /// Scroll the sidebar by `delta` rows (positive scrolls down). The clamp against
    /// the column height happens at render time, which is the only place that knows
    /// how tall the column currently is.
    pub(crate) fn scroll_sidebar(&mut self, delta: i32) {
        let next = (self.sidebar_scroll as i32 + delta).max(0) as u16;
        self.sidebar_scroll = next;
        // Wheeling is browsing, not selecting: keep the cursor where it is, and stop
        // the next frame's follow from yanking the view back to it.
        self.sidebar_scroll_sel = self.sel;
    }
}

/// Copy the visible window of a full-height sidebar column into the frame.
fn blit_window(src: &Buffer, dst: &mut Buffer, area: Rect, off: u16) {
    for y in 0..area.height {
        let sy = y + off;
        if sy >= src.area.height {
            break;
        }
        for x in 0..area.width {
            dst[(area.x + x, area.y + y)] = src[(x, sy)].clone();
        }
    }
}

/// A column row's on-screen `y`, or `None` when it's scrolled out of the window.
fn visible_y(y: u16, area: Rect, off: u16) -> Option<u16> {
    (y >= off && y - off < area.height).then(|| area.y + y - off)
}

/// A column rect clipped to the visible window, in screen coordinates. `None` when
/// none of it is on screen.
fn clip_to_window(rect: Rect, area: Rect, off: u16) -> Option<Rect> {
    let top = rect.y.max(off);
    let bottom = (rect.y + rect.height).min(off + area.height);
    (bottom > top).then(|| Rect {
        x: area.x + rect.x,
        y: area.y + top - off,
        width: rect.width,
        height: bottom - top,
    })
}

impl App {

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
        // A worktree's title already *is* its branch, so repeating it here would waste
        // the only line it gets. It shows the tip commit's subject instead — which is
        // what turns a generated name into something you recognise.
        let (label, label_style) = match self.projects[pi].worktree.is_some() {
            true if !g.head_subject.is_empty() => (
                g.head_subject.as_str(),
                Style::default().fg(Color::DarkGray),
            ),
            true => ("no commits yet", Style::default().fg(Color::DarkGray)),
            false if g.branch.is_empty() => ("HEAD", Style::default().fg(Color::Magenta)),
            false => (g.branch.as_str(), Style::default().fg(Color::Magenta)),
        };
        let git_w = git.chars().count();
        let label_w = (width as usize).saturating_sub(git_w + 1);
        let label = if label_w == 0 {
            String::new()
        } else {
            super::git::truncate_middle(label, label_w)
        };
        let mut line = Line::from(Span::styled(label, label_style));
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
        // Rows are recorded relative to the top of the (possibly taller than the
        // viewport) content, then translated to screen coordinates once the scroll
        // offset is known below.
        let mut y = 0u16;

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

        let selected_y = self
            .regions
            .rows
            .iter()
            .find(|(_, pos)| *pos == self.sel)
            .map(|(y, _)| *y);
        self.settle_sidebar_scroll(lines.len() as u16, inner.height, selected_y);
        let off = self.sidebar_scroll;
        // Drop the rows that scrolled out of view, then move the rest onto the screen.
        self.regions
            .rows
            .retain(|(y, _)| visible_y(*y, inner, off).is_some());
        for (y, _) in self.regions.rows.iter_mut() {
            *y = inner.y + *y - off;
        }
        f.render_widget(Paragraph::new(lines).scroll((off, 0)), inner);
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
                    // An *agent* "needs you" when its explicit progress state is clear,
                    // falling back to a title that has gone static for older agents. This
                    // is the agent's actual state, so it holds even while you're viewing
                    // the pane — selecting an idle agent must not make it look like it's
                    // working again. A *terminal* has no
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

/// Per-box heights for the sidebar column: every collapsed box keeps its natural
/// height and the **active** one absorbs the slack, so the column is filled exactly.
///
/// The caller only reaches here when the natural total fits the viewport — when it
/// doesn't, the column is laid out at full height and *scrolled* instead, which is why
/// there is no "squeeze everything" branch to get wrong.
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
    // Collapsed boxes keep their natural height; the active one takes what's left, so
    // the column is filled exactly. Saturating arithmetic keeps a violated
    // precondition to a squeezed box rather than a panic.
    let mut h = collapsed_heights.to_vec();
    h[active] = total.saturating_sub(collapsed).max(MIN_BOX);
    h
}

#[cfg(test)]
mod tests {
    use super::box_heights;

    /// Collapsed boxes keep their size and the active one takes the rest, filling the
    /// column exactly — whichever box is active.
    #[test]
    fn active_box_absorbs_the_remaining_height() {
        assert_eq!(box_heights(&[3, 3, 3], 1, 30), vec![3, 24, 3]);
        assert_eq!(box_heights(&[3, 4, 3], 0, 20), vec![13, 4, 3]);
        for active in 0..3 {
            let h = box_heights(&[3, 3, 3], active, 30);
            assert_eq!(h.iter().sum::<u16>(), 30, "column not filled at {active}");
        }
    }

    /// Degenerate inputs must not panic: no boxes, an out-of-range active index, or a
    /// column too short for what it was handed (the precondition the caller upholds).
    #[test]
    fn degenerate_columns_are_safe() {
        assert!(box_heights(&[], 0, 30).is_empty());
        assert_eq!(box_heights(&[3, 3], 5, 30).len(), 2);
        let squeezed = box_heights(&[3, 3, 3], 0, 2);
        assert_eq!(squeezed[0], 3, "active box keeps a usable minimum");
    }
}
