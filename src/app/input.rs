//! Input handling: keyboard (sidebar + pane, with the Ctrl-b leader), mouse
//! (focus routing, hamburger buttons, scrollback wheel) and paste.

use super::git::{Confirmed, Overlay, PromptKind, Section};
use super::keymap::encode_key;
use super::nav::Nav;
use super::view::FooterAction;
use super::{App, Focus};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::time::Duration;

impl App {
    pub(crate) fn on_key(&mut self, k: KeyEvent) {
        if k.kind != KeyEventKind::Press {
            return;
        }
        // An open modal eats every key, regardless of which region has focus.
        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }
        // Global Ctrl+P raises the fuzzy file picker from anywhere. This deliberately
        // shadows an in-pane Ctrl+P (shell/readline previous-command) — the chosen
        // tradeoff for matching the user's shell muscle memory.
        if k.code == KeyCode::Char('p') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_leader = false; // don't leave a half-entered Ctrl-b chord armed
            self.open_picker();
            return;
        }
        match self.focus {
            Focus::Sidebar => self.key_sidebar(k),
            Focus::Terminal => self.key_pane(k),
            Focus::Right => self.key_git(k),
        }
    }

    fn key_sidebar(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') => self.sel = 0,
            KeyCode::Char('G') => self.sel = self.build_nav().len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.activate(),
            KeyCode::Char('s') => self.do_start(),
            KeyCode::Char('x') => self.do_stop(),
            KeyCode::Char('r') => self.do_restart(),
            KeyCode::Char('R') => self.reload(),
            KeyCode::Char('d') => crate::tmux::detach(),
            // Jump between projects (no-op in a single-project workspace).
            KeyCode::Char(']') => self.jump_project(1),
            KeyCode::Char('[') => self.jump_project(-1),
            KeyCode::Tab => {
                if self.active_git().is_some() {
                    self.focus = Focus::Right;
                } else if self.current_nav().and_then(|n| self.pane_at(n)).is_some() {
                    self.focus = Focus::Terminal;
                }
            }
            _ => {}
        }
    }

    fn key_pane(&mut self, k: KeyEvent) {
        // Leader (Ctrl-b) prefixed commands.
        if self.pending_leader {
            self.pending_leader = false;
            match k.code {
                KeyCode::Char('d') => crate::tmux::detach(),
                KeyCode::Char('x') => {
                    self.do_stop();
                    self.focus = Focus::Sidebar;
                }
                KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => self.focus = Focus::Sidebar,
                KeyCode::Char('R') => {
                    self.reload();
                    self.focus = Focus::Sidebar; // surface any newly added items
                }
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('b') => self.send_focused(vec![0x02]), // literal Ctrl-b
                _ => {}
            }
            return;
        }
        if k.code == KeyCode::Char('b') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_leader = true;
            return;
        }
        let bytes = encode_key(&k);
        if !bytes.is_empty() {
            // Typing snaps back to the live view if we'd scrolled into history.
            if let Some(p) = self.focused_pane() {
                p.reset_scroll();
            }
            self.send_focused(bytes);
            self.clear_focused_attention();
        }
    }

    /// Keys for the native git panel (`Focus::Right`): navigation + git actions.
    /// There are no pane bytes here, so Esc/h/Tab simply return to the sidebar.
    fn key_git(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(g) = self.active_git_mut() {
                    g.move_cursor(1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(g) = self.active_git_mut() {
                    g.move_cursor(-1);
                }
            }
            // Tab moves between the Changes and Branches boxes; ⏎/space acts on the
            // active one (stage a file / switch to a branch).
            KeyCode::Tab => self.git_section_toggle(),
            KeyCode::Char(' ') | KeyCode::Enter => self.git_activate(),
            KeyCode::Char('b') => self.git_focus_branches(),
            KeyCode::Char('a') => self.git_stage_all(),
            KeyCode::Char('d') => self.git_discard_prompt(),
            KeyCode::Char('s') => self.git_stash(),
            KeyCode::Char('c') => self.git_commit_prompt(),
            KeyCode::Char('n') => self.git_newbranch_prompt(),
            KeyCode::Char('p') => self.git_start("pull"),
            KeyCode::Char('P') => self.git_start("push"),
            KeyCode::Char('r') => {
                if let Some(g) = self.active_git_mut() {
                    g.refresh();
                }
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => self.focus = Focus::Sidebar,
            _ => {}
        }
    }

    /// Keys while a modal overlay is open: text entry for a prompt, list nav + live
    /// filter for the branch switcher. We resolve the keystroke into one action and
    /// apply it *after* the borrow ends, so we never reassign `overlay` mid-borrow.
    fn overlay_key(&mut self, k: KeyEvent) {
        enum Act {
            None,
            Close,
            Submit(PromptKind, String),
            Confirm(Confirmed),
            OpenFile(usize, String),
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let act = match &mut self.overlay {
            // The fuzzy file picker: type to filter, ↑/↓ (or Ctrl-p/n, Ctrl-k/j) to
            // move, ⏎ opens the highlighted file, Esc cancels.
            Some(Overlay::Picker(p)) => match k.code {
                KeyCode::Esc => Act::Close,
                KeyCode::Enter => match p.selected() {
                    Some(path) => Act::OpenFile(p.project, path.to_string()),
                    None => Act::Close,
                },
                KeyCode::Up => {
                    p.move_sel(-1);
                    Act::None
                }
                KeyCode::Down => {
                    p.move_sel(1);
                    Act::None
                }
                KeyCode::Char('p') | KeyCode::Char('k') if ctrl => {
                    p.move_sel(-1);
                    Act::None
                }
                KeyCode::Char('n') | KeyCode::Char('j') if ctrl => {
                    p.move_sel(1);
                    Act::None
                }
                KeyCode::Backspace => {
                    p.query.pop();
                    p.recompute();
                    Act::None
                }
                KeyCode::Char(c) if !ctrl => {
                    p.query.push(c);
                    p.recompute();
                    Act::None
                }
                _ => Act::None,
            },
            Some(Overlay::Prompt { buf, kind, .. }) => match k.code {
                KeyCode::Esc => Act::Close,
                KeyCode::Enter => Act::Submit(*kind, buf.clone()),
                KeyCode::Backspace => {
                    buf.pop();
                    Act::None
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    Act::None
                }
                _ => Act::None,
            },
            // A confirmation: y/⏎ accepts, anything else (n/Esc) cancels.
            Some(Overlay::Confirm { action, .. }) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    Act::Confirm(action.clone())
                }
                _ => Act::Close,
            },
            None => Act::None,
        };
        match act {
            Act::None => {}
            Act::Close => self.overlay = None,
            Act::Submit(kind, buf) => {
                self.overlay = None;
                self.overlay_submit(kind, buf);
            }
            Act::Confirm(action) => {
                self.overlay = None;
                self.overlay_confirm(action);
            }
            Act::OpenFile(pi, path) => {
                self.overlay = None;
                self.open_in_editor(pi, path);
            }
        }
    }

    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_at(m.column, m.row, 3),
            MouseEventKind::ScrollDown => self.scroll_at(m.column, m.row, -3),
            MouseEventKind::Down(MouseButton::Left) => self.on_left_down(m.column, m.row),
            MouseEventKind::Drag(MouseButton::Left) => self.on_left_drag(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.on_left_up(),
            _ => {}
        }
    }

    /// Left press: route focus exactly as before, and — when it lands inside a
    /// live pane — arm a drag-to-copy selection anchored at that cell.
    fn on_left_down(&mut self, c: u16, r: u16) {
        self.drag = None;
        // Footer shortcut buttons sit on their own row below everything else.
        if let Some(&(_, action)) = self.regions.footer_btns.iter().find(|(rect, _)| hit(Some(*rect), c, r)) {
            self.footer_action(action);
            return;
        }
        // Hamburger buttons first — they overlay the top row of other regions.
        if hit(self.regions.panel_btn, c, r) {
            if self.active_git().is_some() {
                self.focus = Focus::Right;
            }
            return;
        }
        if hit(self.regions.menu, c, r) {
            self.focus = Focus::Sidebar;
            return;
        }
        if hit(self.regions.sidebar, c, r) {
            self.on_sidebar_click(r);
            return;
        }
        if hit(self.regions.right, c, r) {
            if self.active_git().is_some() {
                self.focus = Focus::Right;
                self.on_git_click(c, r);
            }
            return;
        }
        if hit(self.regions.main, c, r) {
            if let Some(nav) = self.current_nav() {
                if self.pane_at(nav).is_some() {
                    self.focus = Focus::Terminal;
                    self.clear_focused_attention();
                    self.arm_selection(SelTarget::Main, c, r);
                }
            }
        }
    }

    /// Run the action behind a clicked footer button. Each mirrors the matching
    /// keybinding; pane-context actions that consume the pane drop back to the
    /// sidebar, exactly as their `Ctrl-b` chords do.
    fn footer_action(&mut self, action: FooterAction) {
        let in_pane = matches!(self.focus, Focus::Terminal | Focus::Right);
        match action {
            FooterAction::Activate => self.activate(),
            FooterAction::Start => self.do_start(),
            FooterAction::Stop => {
                self.do_stop();
                if in_pane {
                    self.focus = Focus::Sidebar;
                }
            }
            FooterAction::Restart => self.do_restart(),
            FooterAction::Reload => {
                self.reload();
                self.focus = Focus::Sidebar; // surface any newly added items
            }
            FooterAction::Detach => crate::tmux::detach(),
            FooterAction::Quit => self.should_quit = true,
            FooterAction::FocusPanel => {
                if self.active_git().is_some() {
                    self.focus = Focus::Right;
                }
            }
            FooterAction::FocusSidebar => self.focus = Focus::Sidebar,
            FooterAction::SendLeaderB => self.send_focused(vec![0x02]),
            FooterAction::GitSection => self.git_section_toggle(),
            FooterAction::GitActivate => self.git_activate(),
            FooterAction::GitDiscard => self.git_discard_prompt(),
            FooterAction::GitStash => self.git_stash(),
            FooterAction::GitCommit => self.git_commit_prompt(),
            FooterAction::GitNewBranch => self.git_newbranch_prompt(),
            FooterAction::GitPull => self.git_start("pull"),
            FooterAction::GitPush => self.git_start("push"),
        }
    }

    /// Held-drag: extend the armed selection to the cell under the cursor.
    fn on_left_drag(&mut self, c: u16, r: u16) {
        if let Some(sel) = self.drag.as_mut() {
            let cell = clamp_cell(sel.inner, c, r);
            if cell != sel.anchor {
                sel.moved = true;
            }
            sel.head = cell;
        }
    }

    /// Release: a real drag copies its contents; a plain click just clears.
    fn on_left_up(&mut self) {
        if let Some(sel) = self.drag.take() {
            if sel.moved {
                self.copy_selection(sel);
            }
        }
    }

    /// Anchor a selection at `(c, r)` inside the target pane's content rect.
    fn arm_selection(&mut self, target: SelTarget, c: u16, r: u16) {
        let Some(inner) = self.regions.main_inner else { return };
        let cell = clamp_cell(inner, c, r);
        self.drag = Some(Selection { target, inner, anchor: cell, head: cell, moved: false });
    }

    /// Extract the selected text from the target pane and put it on the clipboard.
    fn copy_selection(&mut self, sel: Selection) {
        let (sr, sc, er, ec) = sel.ordered_in(sel.inner);
        let pane = match sel.target {
            SelTarget::Main => self.current_nav().and_then(|n| self.pane_at(n)),
        };
        let Some(pane) = pane else { return };
        // vt100's `contents_between` takes an exclusive end column; +1 to include
        // the cell the cursor was released on.
        let end_col = (ec + 1).min(sel.inner.width);
        let Some(raw) = pane.contents_between(sr, sc, er, end_col) else { return };
        let text = trim_block(&raw);
        if text.is_empty() {
            return;
        }
        let n = text.chars().count();
        crate::clipboard::copy(&text);
        self.flash = Some((format!("copied {n} chars"), std::time::Instant::now()));
    }

    fn on_sidebar_click(&mut self, row: u16) {
        let Some(idx) = self
            .regions
            .rows
            .iter()
            .find(|(y, _)| *y == row)
            .map(|(_, i)| *i)
        else {
            // Missed a row. If the click landed in another project's box (its title,
            // border, or trailing whitespace), switch to that project — restoring the
            // row last selected there. Either way, focus the sidebar so the nav
            // keybindings (d detach, s/x/r, …) become available again.
            if let Some(&(_, pi)) = self
                .regions
                .project_boxes
                .iter()
                .find(|(rect, _)| row >= rect.y && row < rect.y + rect.height)
            {
                if pi != self.active {
                    self.focus_project(pi);
                }
            }
            self.focus = Focus::Sidebar;
            return;
        };
        let now = std::time::Instant::now();
        let double = matches!(self.last_click, Some((p, t))
            if p == idx && now.duration_since(t) < Duration::from_millis(400));
        self.last_click = Some((idx, now));
        self.sel = idx;
        match self.current_nav() {
            // Launchers: single click selects, double click spawns.
            Some(Nav::NewAgent(..)) | Some(Nav::NewTerminal(_)) => {
                if double {
                    self.activate();
                } else {
                    self.focus = Focus::Sidebar;
                }
            }
            // The git panel: single click opens it.
            Some(Nav::Panel) => self.focus = Focus::Right,
            // Items: single click jumps in if live; double click starts + jumps in.
            Some(nav) => {
                if double {
                    self.activate();
                } else if self.pane_at(nav).is_some() {
                    self.focus = Focus::Terminal;
                    self.clear_focused_attention();
                } else {
                    self.focus = Focus::Sidebar;
                }
            }
            None => {}
        }
    }

    /// A click anywhere in the git column: focus the box that was clicked — so even a
    /// click in a box's whitespace activates that section — and, on a row, select it,
    /// with a double-click staging the file / switching to the branch.
    fn on_git_click(&mut self, c: u16, r: u16) {
        match self.git_section_at(c, r) {
            Some(Section::Changes) => {
                if let Some(g) = self.active_git_mut() {
                    g.section = Section::Changes;
                }
                if let Some(&(_, idx)) = self.regions.git_rows.iter().find(|(y, _)| *y == r) {
                    let double = self.is_double_click(idx);
                    if let Some(g) = self.active_git_mut() {
                        g.cursor = idx.min(g.rows.len().saturating_sub(1));
                    }
                    if double {
                        self.git_toggle_stage();
                    }
                }
            }
            Some(Section::Branches) => {
                if let Some(g) = self.active_git_mut() {
                    g.section = Section::Branches;
                }
                if let Some(&(_, idx)) = self.regions.git_branch_rows.iter().find(|(y, _)| *y == r) {
                    // Branch keys live in a separate namespace so a file and a branch
                    // at the same index aren't mistaken for a double-click together.
                    let double = self.is_double_click(BRANCH_CLICK_KEY + idx);
                    if let Some(g) = self.active_git_mut() {
                        g.branch_cursor = idx.min(g.branches.len().saturating_sub(1));
                    }
                    if double {
                        self.git_switch_selected();
                    }
                }
            }
            // Recent box or a border: focus already moved to the column; leave the
            // active section unchanged.
            None => {}
        }
    }

    /// Which git box (if any) the point `(c, r)` falls in. Branches is tested first
    /// since the boxes don't overlap, but order keeps it unambiguous.
    fn git_section_at(&self, c: u16, r: u16) -> Option<Section> {
        if hit(self.regions.git_branches, c, r) {
            Some(Section::Branches)
        } else if hit(self.regions.git_changes, c, r) {
            Some(Section::Changes)
        } else {
            None
        }
    }

    /// Double-click test against `last_click`, keyed by an opaque `key` so different
    /// row kinds (sidebar / files / branches) don't read as clicks on each other.
    fn is_double_click(&mut self, key: usize) -> bool {
        let now = std::time::Instant::now();
        let double = matches!(self.last_click, Some((p, t))
            if p == key && now.duration_since(t) < Duration::from_millis(400));
        self.last_click = Some((key, now));
        double
    }

    /// Scroll the region under the cursor. In the main pane `delta > 0` reveals
    /// older scrollback (wheel up); the git panel isn't a buffer, so the wheel
    /// moves its file cursor instead.
    fn scroll_at(&mut self, col: u16, row: u16, delta: i32) {
        if hit(self.regions.right, col, row) {
            // Scroll moves (and activates) the box under the cursor; over Recent or a
            // border it does nothing.
            if let Some(sec) = self.git_section_at(col, row) {
                if let Some(g) = self.active_git_mut() {
                    g.section = sec;
                    g.move_cursor(if delta > 0 { -1 } else { 1 });
                }
            }
        } else if hit(self.regions.main, col, row) {
            if let Some(p) = self.current_nav().and_then(|n| self.pane_at(n)) {
                p.scroll(delta);
            }
        }
    }

    pub(crate) fn on_paste(&mut self, s: String) {
        // Paste into an open text prompt (commit message); otherwise to the pane.
        if let Some(Overlay::Prompt { buf, .. }) = &mut self.overlay {
            buf.push_str(&s);
        } else if self.focus == Focus::Terminal {
            self.send_focused(s.into_bytes());
        }
    }
}

/// Offsets a branch-row index into a separate `last_click` key space from file
/// rows, so a double-click is only ever detected within one row kind.
const BRANCH_CLICK_KEY: usize = 1 << 24;

/// True if `(col, row)` falls inside `rect` (when present).
fn hit(rect: Option<Rect>, col: u16, row: u16) -> bool {
    rect.is_some_and(|r| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
}

/// Which pane a drag-to-copy selection is happening over. Only the main pane is
/// selectable — the git panel is native text, not a vt100 grid.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelTarget {
    Main,
}

/// An in-progress (or just-released) mouse drag selection over a pane. `anchor`
/// and `head` are absolute screen cells clamped into `inner` (the pane's content
/// rect captured at press time).
#[derive(Clone, Copy)]
pub(crate) struct Selection {
    pub target: SelTarget,
    pub inner: Rect,
    pub anchor: (u16, u16),
    pub head: (u16, u16),
    pub moved: bool,
}

impl Selection {
    /// The selection as inner-relative, inclusive `(start_row, start_col, end_row,
    /// end_col)`, ordered in reading flow so start precedes end. `inner` lets the
    /// caller re-map against the current frame's content rect.
    pub(crate) fn ordered_in(&self, inner: Rect) -> (u16, u16, u16, u16) {
        let rel = |(col, row): (u16, u16)| {
            let c = col.clamp(inner.x, inner.x + inner.width.saturating_sub(1)) - inner.x;
            let r = row.clamp(inner.y, inner.y + inner.height.saturating_sub(1)) - inner.y;
            (r, c) // (row, col) for natural reading-order comparison
        };
        let a = rel(self.anchor);
        let b = rel(self.head);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        (s.0, s.1, e.0, e.1)
    }
}

/// Clamp an absolute mouse position to a cell inside `inner` (absolute coords).
fn clamp_cell(inner: Rect, c: u16, r: u16) -> (u16, u16) {
    let col = c.clamp(inner.x, inner.x + inner.width.saturating_sub(1));
    let row = r.clamp(inner.y, inner.y + inner.height.saturating_sub(1));
    (col, row)
}

/// Tidy extracted text for the clipboard: vt100 pads rows with blanks, so trim
/// trailing whitespace per line and drop any trailing blank lines.
fn trim_block(s: &str) -> String {
    s.split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
