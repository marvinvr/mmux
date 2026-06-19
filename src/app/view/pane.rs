//! Rendering of the two terminal regions — the main pane and the right panel —
//! plus their titles and "nothing running yet" placeholders. Both regions share
//! [`render_screen`]/[`render_placeholder`]; the wrappers only differ in which
//! recipe/rect/resize target they touch.

use super::theme::status_label;
use crate::app::input::SelTarget;
use crate::app::nav::Nav;
use crate::app::session::Kind;
use crate::app::{App, Focus};
use crate::pane::Pane;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use tui_term::widget::{Cursor, PseudoTerminal};

impl App {
    pub(crate) fn render_main(&mut self, f: &mut Frame, area: Rect, compact_bar: bool) {
        let nav = self.current_nav();
        let base = self.main_title(nav);
        let focus = self.focus;
        let border = if focus == Focus::Terminal {
            Color::Magenta
        } else {
            Color::DarkGray
        };
        let title = if compact_bar {
            format!(" ☰ {}", base.trim())
        } else {
            base
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border));
        let inner = block.inner(area);
        f.render_widget(block, area);
        self.regions.main = Some(area);
        if compact_bar {
            // Left third of the title bar = open the menu; right = open the panel.
            self.regions.menu = Some(Rect {
                x: area.x,
                y: area.y,
                width: (area.width / 3).max(3),
                height: 1,
            });
            if self.active_git().is_some() {
                let half = area.width / 2;
                let rz = Rect {
                    x: area.x + half,
                    y: area.y,
                    width: area.width - half,
                    height: 1,
                };
                self.regions.panel_btn = Some(self.draw_panel_button(f, rz));
            }
        }
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        self.regions.main_inner = Some(inner);

        self.resize_current(inner.height, inner.width);

        match nav.and_then(|n| self.pane_at(n)) {
            Some(pane) => render_screen(f, inner, pane, focus == Focus::Terminal),
            None => render_placeholder(f, inner, &self.placeholder_text(nav)),
        }
        self.paint_selection(f, inner, SelTarget::Main);
    }

    /// The right column: the active project's native git panel (changed files,
    /// staging, recent commits). A plain placeholder when the project isn't a repo.
    pub(crate) fn render_right(&mut self, f: &mut Frame, area: Rect, compact_bar: bool) {
        self.regions.right = Some(area);
        if compact_bar {
            // The whole title bar returns to the menu.
            self.regions.menu = Some(Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            });
        }
        let focused = self.focus == Focus::Right;
        let hits = match self.active_git() {
            Some(g) => super::git::render_git(f, area, g, focused, compact_bar),
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
    /// highlight follows the same flow shape as the extracted text.
    fn paint_selection(&self, f: &mut Frame, inner: Rect, target: SelTarget) {
        let Some(sel) = self.drag else { return };
        if !sel.moved || sel.target != target {
            return;
        }
        let (sr, sc, er, ec) = sel.ordered_in(inner);
        let style = Style::default().add_modifier(Modifier::REVERSED);
        let last = inner.width.saturating_sub(1);
        let buf = f.buffer_mut();
        for row in sr..=er {
            let (c0, c1) = if sr == er {
                (sc, ec)
            } else if row == sr {
                (sc, last)
            } else if row == er {
                (0, ec)
            } else {
                (0, last)
            };
            for col in c0..=c1 {
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

fn render_placeholder(f: &mut Frame, area: Rect, msg: &str) {
    f.render_widget(
        Paragraph::new(msg.to_string())
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
