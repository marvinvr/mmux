//! Top-level rendering: the responsive layout split, per-frame hit regions, the
//! footer, and the panel button. Per-pane drawing lives in the submodules.

mod git;
mod pane;
mod sidebar;
pub(crate) mod theme;

use super::git::Section;
use super::{App, Focus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Below this terminal width we switch to single-column (phone) mode.
const COMPACT_W: u16 = 60;
/// Minimum main-pane width required before we also show the right panel as its own column.
const MAIN_MIN: u16 = 36;

/// Per-frame mouse-hit geometry. Recomputed every `draw`; consumed by input
/// routing. `rows` maps a screen `y` in the sidebar to a nav index.
#[derive(Default)]
pub(crate) struct Regions {
    pub sidebar: Option<Rect>,
    pub main: Option<Rect>,
    pub right: Option<Rect>,
    pub menu: Option<Rect>,      // tap → open the left sidebar drawer
    pub panel_btn: Option<Rect>, // tap → open the right panel
    pub link_btn: Option<Rect>,  // tap → open the "Link another project" browser
    pub rows: Vec<(u16, usize)>,
    // Footer shortcut buttons: each `[key label]` chip and the action it fires.
    pub footer_btns: Vec<(Rect, FooterAction)>,
    // Inner content rect (inside the borders) of the main pane, for mapping a mouse
    // drag to buffer cells and painting the selection highlight.
    pub main_inner: Option<Rect>,
    // The git panel's changed-file rows: screen `y` → file index, for click-to-stage.
    pub git_rows: Vec<(u16, usize)>,
    // The git panel's branch rows: screen `y` → branch index, for click-to-switch.
    pub git_branch_rows: Vec<(u16, usize)>,
    // The git panel's box areas, so a click in a box's whitespace focuses that section.
    pub git_changes: Option<Rect>,
    pub git_branches: Option<Rect>,
    // Per-project sidebar box rects (multi-project mode, wide or compact): a click
    // that misses a row but lands in one of these switches to that project.
    pub project_boxes: Vec<(Rect, usize)>,
}

/// What clicking a footer button does. Each variant mirrors an existing
/// keybinding so a click is just a discoverable alias for the key.
#[derive(Clone, Copy)]
pub(crate) enum FooterAction {
    Activate,
    Start,
    Stop,
    Restart,
    Reload,
    /// Open the "Link another project" directory browser.
    LinkProject,
    Detach,
    Quit,
    FocusPanel,
    FocusSidebar,
    SendLeaderB,
    /// Restart into a staged self-update (the bottom-right badge).
    ApplyUpdate,
    // Git panel actions (mirror the keys in `key_git`).
    GitSection,
    GitActivate,
    GitDiff,
    DiffClose,
    GitDiscard,
    GitStash,
    GitCommit,
    GitNewBranch,
    GitPull,
    GitPush,
}

/// One footer segment: either a plain non-clickable hint, or a bracketed,
/// clickable shortcut button.
enum Seg {
    Hint(String),
    Btn { key: String, label: String, action: FooterAction },
}

impl Seg {
    fn hint(s: &str) -> Seg {
        Seg::Hint(s.into())
    }
    fn btn(key: &str, label: &str, action: FooterAction) -> Seg {
        Seg::Btn { key: key.into(), label: label.into(), action }
    }
}

impl App {
    pub(crate) fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        let content = v[0];
        let footer = v[1];

        // Reset per-frame hit regions.
        self.regions = Regions::default();

        self.compact = content.width < COMPACT_W;
        // Keep the selection valid (nav length changes between compact/wide).
        let navlen = self.build_nav().len();
        if navlen > 0 && self.sel >= navlen {
            self.sel = navlen - 1;
        }

        if self.compact {
            // Single column: the drawer, or the focused pane with hamburger button(s).
            match self.focus {
                Focus::Sidebar => self.render_sidebar(f, content),
                Focus::Right => self.render_right(f, content, true),
                Focus::Terminal => self.render_main(f, content, true),
            }
        } else {
            let sw = (content.width / 3)
                .clamp(20, 36)
                .min(content.width.saturating_sub(10));
            let has_right = self.active_git().is_some();
            // The git column matches the sidebar's width (`sw`), so the two flanking
            // columns read as a pair.
            let show_right = has_right && content.width >= 2 * sw + MAIN_MIN;
            if show_right {
                let h = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(sw),
                        Constraint::Min(1),
                        Constraint::Length(sw),
                    ])
                    .split(content);
                self.render_sidebar(f, h[0]);
                self.render_main(f, h[1], false);
                self.render_right(f, h[2], false);
            } else {
                let h = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(sw), Constraint::Min(1)])
                    .split(content);
                self.render_sidebar(f, h[0]);
                // No room for a 3rd column: the git panel shares the main area when
                // focused (reach it via Tab or the sidebar entry).
                if self.focus == Focus::Right && self.active_git().is_some() {
                    self.render_right(f, h[1], false);
                } else {
                    self.render_main(f, h[1], false);
                }
            }
        }

        // A modal overlay (commit prompt / branch switcher) floats above everything.
        if let Some(ov) = self.overlay.as_ref() {
            git::render_overlay(f, content, ov);
        }

        self.render_footer(f, footer);
    }

    fn render_footer(&mut self, f: &mut Frame, area: Rect) {
        self.regions.footer_btns.clear();
        // A recent reload (or its error) takes over the footer for a few seconds. The
        // update badge still floats on top of it (rendered last), so it's never hidden.
        let flashing = matches!(&self.flash, Some((_, at)) if at.elapsed() < std::time::Duration::from_secs(4));
        if flashing {
            if let Some((msg, _)) = &self.flash {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(" {msg} "),
                        Style::default().fg(Color::Black).bg(Color::Yellow),
                    ))),
                    area,
                );
            }
            self.render_update_badge(f, area);
            return;
        }

        let bg = Color::Cyan;
        let bar = Style::default().fg(Color::Black).bg(bg); // separators, label text
        let key = bar.add_modifier(Modifier::BOLD); // the shortcut glyph pops
        let dim = Style::default().fg(Color::Rgb(0, 70, 75)).bg(bg); // braces + hints

        let mut spans: Vec<Span> = Vec::new();
        let mut btns: Vec<(Rect, FooterAction)> = Vec::new();
        let mut x = area.x;
        let pad = |spans: &mut Vec<Span>, x: &mut u16| {
            spans.push(Span::styled(" ", bar));
            *x += 1;
        };
        pad(&mut spans, &mut x);
        for (i, seg) in self.footer_segments().iter().enumerate() {
            if i > 0 {
                pad(&mut spans, &mut x);
            }
            match seg {
                Seg::Hint(t) => {
                    spans.push(Span::styled(t.clone(), dim));
                    x += t.chars().count() as u16;
                }
                Seg::Btn { key: k, label, action } => {
                    let start = x;
                    spans.push(Span::styled("[", dim));
                    spans.push(Span::styled(k.clone(), key));
                    spans.push(Span::styled(" ", bar));
                    spans.push(Span::styled(label.clone(), bar));
                    spans.push(Span::styled("]", dim));
                    x += 3 + (k.chars().count() + label.chars().count()) as u16;
                    btns.push((Rect { x: start, y: area.y, width: x - start, height: 1 }, *action));
                }
            }
        }
        pad(&mut spans, &mut x);

        f.render_widget(Paragraph::new(Line::from(spans)), area);
        self.regions.footer_btns = btns;
        // The self-update badge floats on the right, drawn after the shortcuts so it sits
        // above them and registers its own click target.
        self.render_update_badge(f, area);
    }

    /// The bottom-right self-update badge: a faint "updating…" while brew runs in the
    /// background, then a clickable "↻ restart to update" once the new binary is staged.
    /// Deliberately quiet — present and discoverable, never modal or alarming. A click
    /// (or `U` in the sidebar) restarts in place onto the new version.
    fn render_update_badge(&mut self, f: &mut Frame, area: Rect) {
        use super::UpdateState;
        let (text, style, clickable) = match &self.update {
            UpdateState::Installing(v) => (
                format!(" ↻ updating to v{v}… "),
                Style::default().fg(Color::DarkGray),
                false,
            ),
            UpdateState::Ready(v) => (
                format!(" ↻ restart to update → v{v} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme::ATTN)
                    .add_modifier(Modifier::BOLD),
                true,
            ),
            UpdateState::Idle | UpdateState::Checking => return,
        };
        let w = (text.chars().count() as u16).min(area.width);
        if w == 0 {
            return;
        }
        let rect = Rect { x: area.x + area.width - w, y: area.y, width: w, height: 1 };
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), rect);
        if clickable {
            // Insert at the front: the badge is drawn over the shortcut chips, so it must
            // also win hit-testing where they overlap (`on_left_down` takes the first match).
            self.regions.footer_btns.insert(0, (rect, FooterAction::ApplyUpdate));
        }
    }

    /// The footer's segments for the current focus/layout: plain hints plus the
    /// clickable shortcut buttons. Each button's action mirrors its keybinding.
    fn footer_segments(&self) -> Vec<Seg> {
        use FooterAction::*;
        match self.focus {
            // A focused diff preview is a pager: scroll + close, plus the usual way back.
            Focus::Terminal if self.diff.is_some() => {
                let mut v = vec![Seg::hint("↑↓ scroll"), Seg::btn("esc", "close", DiffClose)];
                v.push(if self.compact {
                    Seg::btn("☰", "menu", FocusSidebar)
                } else {
                    Seg::btn("h", "back", DiffClose)
                });
                v
            }
            Focus::Sidebar if self.compact => vec![
                Seg::hint("↑↓ move"),
                Seg::btn("⏎", "open", Activate),
                Seg::btn("q", "quit", Quit),
            ],
            Focus::Sidebar => {
                let mut v = vec![
                    Seg::hint("↑↓ move"),
                    Seg::btn("⏎", "open", Activate),
                    Seg::hint("dbl-click +New"),
                    Seg::btn("s", "start", Start),
                    Seg::btn("x", "close", Stop),
                    Seg::btn("r", "restart", Restart),
                    Seg::btn("R", "reload", Reload),
                    Seg::btn("L", "link", LinkProject),
                ];
                if self.projects.len() > 1 {
                    v.push(Seg::hint("[ ] project"));
                }
                if self.active_git().is_some() {
                    v.push(Seg::btn("Tab", "git", FocusPanel));
                }
                v.push(Seg::btn("d", "detach", Detach));
                v.push(Seg::btn("q", "quit", Quit));
                v
            }
            // The git panel: clickable buttons mirroring its keymap, like the other
            // panels. The ⏎ action is section-aware (stage a file / switch a branch).
            Focus::Right => {
                let section = self.active_git().map(|g| g.section);
                let activate = match section {
                    Some(Section::Branches) => "switch",
                    _ => "stage",
                };
                let mut v = vec![
                    Seg::hint("↑↓ move"),
                    Seg::btn("Tab", "section", GitSection),
                    Seg::btn("⏎", activate, GitActivate),
                ];
                // Diff preview and discard both target a file in the changes tree, so
                // only offer them there; stash is whole-tree and always available.
                if section != Some(Section::Branches) {
                    let diff_label = if self.diff.is_some() { "close" } else { "diff" };
                    v.push(Seg::btn("v", diff_label, GitDiff));
                    v.push(Seg::btn("d", "discard", GitDiscard));
                }
                v.extend([
                    Seg::btn("s", "stash", GitStash),
                    Seg::btn("c", "commit", GitCommit),
                    Seg::btn("n", "branch", GitNewBranch),
                    Seg::btn("p", "pull", GitPull),
                    Seg::btn("P", "push", GitPush),
                ]);
                v.push(if self.compact {
                    Seg::btn("☰", "menu", FocusSidebar)
                } else {
                    Seg::btn("h", "back", FocusSidebar)
                });
                v
            }
            _ if self.compact => vec![
                Seg::hint("keys → pane"),
                Seg::btn("☰", "menu", FocusSidebar),
                Seg::btn("Ctrl-b d", "detach", Detach),
            ],
            Focus::Terminal => vec![
                Seg::hint("keys → pane"),
                Seg::hint("drag = copy"),
                Seg::hint("Ctrl-b →"),
                Seg::btn("h", "back", FocusSidebar),
                Seg::btn("d", "detach", Detach),
                Seg::btn("x", "close", Stop),
                Seg::btn("b", "send Ctrl-b", SendLeaderB),
            ],
        }
    }

    /// Draw the right-panel "open" button right-aligned in `zone` (a top-border row),
    /// returning the rect it occupies so clicks can be routed to it.
    pub(crate) fn draw_panel_button(&self, f: &mut Frame, zone: Rect) -> Rect {
        let name = self
            .active_git()
            .map(|g| if g.branch.is_empty() { "git" } else { g.branch.as_str() })
            .unwrap_or("git");
        let label = format!(" {name} ☰ ");
        let w = (label.chars().count() as u16).clamp(1, zone.width);
        let rect = Rect {
            x: zone.x + zone.width.saturating_sub(w),
            y: zone.y,
            width: w,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                label,
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ))),
            rect,
        );
        rect
    }
}
