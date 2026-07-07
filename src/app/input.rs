//! Input handling: keyboard (sidebar + pane, with the Ctrl-b leader), mouse
//! (focus routing, hamburger buttons, scrollback wheel) and paste.

use super::git::Section;
use super::keymap::encode_key;
use super::nav::Nav;
use super::overlay::Overlay;
use super::procform::Step;
use super::session::Kind;
use super::view::FooterAction;
use super::{App, Focus};
use crate::pane::MouseAction;
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
        // The `Ctrl-b` leader is global: armed from any focus, the next key is an mmux
        // command (detach / quit / close / back / reload / literal Ctrl-b). This makes
        // "leave" one motor pattern everywhere — you never have to first hop back to the
        // sidebar. In a focused pane it also intercepts `Ctrl-b` before `key_pane` would
        // forward it to the program as bytes.
        if self.pending_leader {
            self.pending_leader = false;
            self.leader_command(k);
            return;
        }
        if k.code == KeyCode::Char('b') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_leader = true;
            return;
        }
        match self.focus {
            Focus::Sidebar => self.key_sidebar(k),
            Focus::Terminal => self.key_pane(k),
            Focus::Right => self.key_git(k),
        }
    }

    /// A `Ctrl-b`-prefixed command. Global (armed in [`on_key`](Self::on_key) from any
    /// focus) so detach / quit / close / back are one motor pattern everywhere — you're
    /// never forced to return to the sidebar first. Actions that consume a session or
    /// land you "outside" a pane drop focus to the sidebar and clear any diff preview
    /// still occupying the main pane. `b` sends a literal `Ctrl-b`, but only a focused
    /// pane has anywhere to send it (`send_focused` is a no-op otherwise).
    fn leader_command(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('d') => crate::tmux::detach(),
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('x') => {
                self.do_stop();
                self.clear_diff();
                self.focus = Focus::Sidebar;
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => {
                self.clear_diff();
                self.focus = Focus::Sidebar;
            }
            KeyCode::Char('R') => {
                self.reload();
                self.clear_diff();
                self.focus = Focus::Sidebar; // surface any newly added items
            }
            KeyCode::Char('b') => self.send_focused(vec![0x02]), // literal Ctrl-b to the pane
            _ => {}
        }
    }

    fn key_sidebar(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('g') => self.sel = 0,
            KeyCode::Char('G') => self.sel = self.build_nav().len().saturating_sub(1),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.activate(),
            KeyCode::Char('s') => self.do_start(),
            KeyCode::Char('x') => self.do_stop(),
            KeyCode::Char('r') => self.do_restart(),
            KeyCode::Char('R') => self.reload(),
            // Process-only: edit reopens the guided form on the selected process; delete
            // asks to confirm, then removes it from the config. No-ops on other rows.
            KeyCode::Char('e') => self.edit_selected(),
            KeyCode::Char('D') => self.delete_selected(),
            // Manage the built-in agent harnesses (add/remove, danger mode) — the popup
            // writes to the global config and reloads. Available from any sidebar row.
            KeyCode::Char('a') => self.open_agent_manager(),
            // Apply a staged self-update (only acts when the "↻ restart to update" badge
            // is showing); otherwise a no-op.
            KeyCode::Char('U') => self.apply_update(),
            // The About card: version + links + a manual update check/apply.
            KeyCode::Char('?') => self.open_about(),
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
        // A diff preview occupies the main pane as a read-only pager: keys scroll it
        // (vi/less-style) rather than reaching any underlying PTY, and Esc/q/h close
        // it back to the git panel it came from.
        if self.diff.is_some() {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Char('j') | KeyCode::Down => self.diff_scroll(1),
                KeyCode::Char('k') | KeyCode::Up => self.diff_scroll(-1),
                KeyCode::Char('d') if ctrl => self.diff_scroll(10),
                KeyCode::Char('u') if ctrl => self.diff_scroll(-10),
                KeyCode::Char(' ') | KeyCode::PageDown => self.diff_scroll(20),
                KeyCode::PageUp => self.diff_scroll(-20),
                KeyCode::Char('g') => self.diff_scroll(i32::MIN / 2),
                KeyCode::Char('G') => self.diff_scroll(i32::MAX / 2),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left => {
                    self.clear_diff();
                    self.focus = Focus::Right;
                }
                _ => {}
            }
            return;
        }
        // The `Ctrl-b` leader is handled globally in `on_key` before we reach here, so a
        // focused pane only has to translate the keystroke to bytes for the program.
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
                self.git_preview_follow();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(g) = self.active_git_mut() {
                    g.move_cursor(-1);
                }
                self.git_preview_follow();
            }
            // Tab cycles Changes → Branches → Commits; ⏎/space acts on the active one
            // (stage a file / switch to a branch / show a commit's diff); v previews the
            // file's or commit's diff.
            KeyCode::Tab => self.git_section_toggle(),
            KeyCode::Char(' ') | KeyCode::Enter => self.git_activate(),
            KeyCode::Char('v') => self.git_toggle_diff(),
            KeyCode::Char('b') => self.git_focus_branches(),
            KeyCode::Char('a') => self.git_toggle_all(),
            KeyCode::Char('d') => self.git_discard_prompt(),
            KeyCode::Char('s') => self.git_stash(),
            KeyCode::Char('c') => self.git_commit_prompt(),
            KeyCode::Char('n') => self.git_newbranch_prompt(),
            KeyCode::Char('p') => self.git_start("pull"),
            KeyCode::Char('P') => self.git_start("push"),
            // Commits-box actions (each gates on the active section, so they're no-ops in
            // the other boxes): copy short/full hash, copy message, revert, uncommit.
            KeyCode::Char('y') => self.git_copy_commit_hash(false),
            KeyCode::Char('Y') => self.git_copy_commit_hash(true),
            KeyCode::Char('m') => self.git_copy_commit_message(),
            KeyCode::Char('t') => self.git_revert_prompt(),
            KeyCode::Char('u') => self.git_soft_reset_prompt(),
            KeyCode::Char('r') => {
                if let Some(g) = self.active_git_mut() {
                    g.refresh();
                }
            }
            // Esc/h backs out one level: close the diff preview if open, else leave
            // the panel for the sidebar.
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => {
                if self.diff.is_some() {
                    self.clear_diff();
                } else {
                    self.focus = Focus::Sidebar;
                }
            }
            _ => {}
        }
    }

    pub(crate) fn on_mouse(&mut self, m: MouseEvent) {
        // A modal overlay captures the mouse just as it captures keys: only its own
        // link hitboxes (the About card's URLs) are live; every other click is
        // swallowed so it can't leak to the sidebar/footer behind the modal.
        if self.overlay.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                if let Some(url) = self
                    .regions
                    .links
                    .iter()
                    .find(|(rect, _)| hit(Some(*rect), m.column, m.row))
                    .map(|(_, u)| u.clone())
                {
                    self.open_url(&url);
                }
            }
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_at(m.column, m.row, 3),
            MouseEventKind::ScrollDown => self.scroll_at(m.column, m.row, -3),
            // A program tracking the mouse gets the click/drag/release/motion
            // itself (so micro/vim/… place the cursor, select, …); holding Shift
            // bypasses to mmux's own drag-to-copy. Otherwise fall through to the
            // native focus/selection routing below.
            _ if self.forward_mouse(&m) => {}
            MouseEventKind::Down(MouseButton::Left) => self.on_left_down(m.column, m.row),
            MouseEventKind::Drag(MouseButton::Left) => self.on_left_drag(m.column, m.row),
            MouseEventKind::Up(MouseButton::Left) => self.on_left_up(),
            _ => {}
        }
    }

    /// Forward a click/drag/release/motion to the inner program when it's
    /// tracking the mouse and the event lands inside its live pane; returns true
    /// if it consumed the event. Shift held (the copy escape hatch), a diff
    /// pager, or any region other than the main pane all decline, so mmux's own
    /// focus routing and drag-to-copy handle those instead.
    fn forward_mouse(&mut self, m: &MouseEvent) -> bool {
        if m.modifiers.contains(KeyModifiers::SHIFT) {
            return false;
        }
        if self.diff.is_some() || !hit(self.regions.main, m.column, m.row) {
            return false;
        }
        let action = match m.kind {
            MouseEventKind::Down(_) => MouseAction::Down,
            MouseEventKind::Up(_) => MouseAction::Up,
            MouseEventKind::Drag(_) => MouseAction::Drag,
            MouseEventKind::Moved => MouseAction::Move,
            _ => return false, // wheel handled separately
        };
        let button = match m.kind {
            MouseEventKind::Down(b) | MouseEventKind::Up(b) | MouseEventKind::Drag(b) => button_code(b),
            _ => 0,
        };
        let (ox, oy) = self.regions.main_inner.map_or((0, 0), |r| (r.x, r.y));
        let sent = match self.current_nav().and_then(|n| self.pane_at(n)) {
            Some(p) => match p.mouse_input(action, button, m.column, m.row, ox, oy) {
                Some(bytes) => {
                    p.reset_scroll(); // a forwarded event acts on the live view
                    p.send(bytes);
                    true
                }
                None => false,
            },
            None => false,
        };
        if sent && matches!(m.kind, MouseEventKind::Down(_)) {
            // A press focuses the pane so the following keys reach the program.
            self.focus = Focus::Terminal;
            self.clear_focused_attention();
        }
        sent
    }

    /// Left press: route focus exactly as before, and — when it lands inside a
    /// live pane — arm a drag-to-copy selection anchored at that cell.
    fn on_left_down(&mut self, c: u16, r: u16) {
        self.drag = None;
        self.pending_url = None;
        // Footer shortcut buttons sit on their own row below everything else.
        if let Some(&(_, action)) = self.regions.footer_btns.iter().find(|(rect, _)| hit(Some(*rect), c, r)) {
            self.footer_action(action);
            return;
        }
        // The standalone "Link another project" box at the bottom of the sidebar.
        if hit(self.regions.link_btn, c, r) {
            self.open_link_browser();
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
            // A diff preview owns the pane: focus it for keyboard scrolling, and arm a
            // drag-to-copy over its text (the pager isn't a vt100 grid, so it has its
            // own cell mapping that skips the gutter + sign columns).
            if self.diff.is_some() {
                self.focus = Focus::Terminal;
                self.arm_diff_selection(c, r);
                return;
            }
            if let Some(nav) = self.current_nav() {
                if self.pane_at(nav).is_some() {
                    self.focus = Focus::Terminal;
                    self.clear_focused_attention();
                    self.arm_selection(SelTarget::Main, c, r);
                    // Remember a URL under the press: a plain click (no drag) opens it
                    // on release, a drag copies instead. See `on_left_up`.
                    self.pending_url = self.url_under(c, r);
                }
            }
        }
    }

    /// The web URL under screen cell `(c, r)` in the live main pane, if the cell sits
    /// on one. Reads the displayed row from the vt100 screen (already reflecting any
    /// scrollback) and extracts the link token — see [`url_at`].
    fn url_under(&self, c: u16, r: u16) -> Option<String> {
        let inner = self.regions.main_inner?;
        if !hit(Some(inner), c, r) {
            return None;
        }
        let nav = self.current_nav()?;
        let pane = self.pane_at(nav)?;
        let row = (r - inner.y) as usize;
        let col = (c - inner.x) as usize;
        pane.with_screen(|s| {
            let (_, cols) = s.size();
            let text = s.rows(0, cols).nth(row)?;
            url_at(&text, col)
        })
        .flatten()
    }

    /// Open `url` in the browser and flash a confirmation. Central so both the About
    /// card and pane-content links share one path.
    fn open_url(&mut self, url: &str) {
        crate::open::url(url);
        self.flash(format!("opened {url}"));
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
            FooterAction::EditProcess => self.edit_selected(),
            FooterAction::DeleteProcess => self.delete_selected(),
            FooterAction::Reload => {
                self.reload();
                self.focus = Focus::Sidebar; // surface any newly added items
            }
            FooterAction::Detach => crate::tmux::detach(),
            FooterAction::Quit => self.request_quit(),
            FooterAction::FocusPanel => {
                if self.active_git().is_some() {
                    self.focus = Focus::Right;
                }
            }
            FooterAction::FocusSidebar => self.focus = Focus::Sidebar,
            FooterAction::CloseToMain => self.focus = Focus::Terminal,
            FooterAction::SendLeaderB => self.send_focused(vec![0x02]),
            FooterAction::ApplyUpdate => self.apply_update(),
            FooterAction::About => self.open_about(),
            FooterAction::ManageAgents => self.open_agent_manager(),
            FooterAction::GitSection => self.git_section_toggle(),
            FooterAction::GitActivate => self.git_activate(),
            FooterAction::GitStageAll => self.git_toggle_all(),
            FooterAction::GitDiff => self.git_toggle_diff(),
            FooterAction::DiffClose => {
                self.clear_diff();
                self.focus = Focus::Right;
            }
            FooterAction::GitDiscard => self.git_discard_prompt(),
            FooterAction::GitStash => self.git_stash(),
            FooterAction::GitCommit => self.git_commit_prompt(),
            FooterAction::GitNewBranch => self.git_newbranch_prompt(),
            FooterAction::GitPull => self.git_start("pull"),
            FooterAction::GitPush => self.git_start("push"),
            FooterAction::GitCopyHash => self.git_copy_commit_hash(false),
            FooterAction::GitCopyMessage => self.git_copy_commit_message(),
            FooterAction::GitRevert => self.git_revert_prompt(),
            FooterAction::GitSoftReset => self.git_soft_reset_prompt(),
        }
    }

    /// Held-drag: extend the armed selection to the cell under the cursor, and arm
    /// edge auto-scroll — dragging at/over the top edge reveals older lines, the
    /// bottom edge newer ones. The actual scrolling repeats from `tick` so it keeps
    /// going while the button is held still at the edge (no further mouse events).
    fn on_left_drag(&mut self, c: u16, r: u16) {
        let Some(inner) = self.regions.main_inner else { return };
        self.drag_scroll = if inner.height == 0 || self.drag.is_none() {
            0
        } else if r <= inner.y {
            1 // top edge → reveal earlier lines
        } else if r + 1 >= inner.y + inner.height {
            -1 // bottom edge → reveal later lines
        } else {
            0
        };
        // The diff pager scrolls its own offset; a live pane scrolls its scrollback.
        if self.diff.is_some() {
            self.update_diff_head(inner, c, r);
        } else {
            self.update_drag_head(inner, c, r);
        }
    }

    /// Release: a real drag copies its contents; a plain click on a URL opens it;
    /// any other plain click just clears.
    fn on_left_up(&mut self) {
        self.drag_scroll = 0;
        let pending = self.pending_url.take();
        if let Some(sel) = self.drag.take() {
            if sel.moved {
                match sel.target {
                    SelTarget::Main => self.copy_selection(sel),
                    SelTarget::Diff => self.copy_diff_selection(sel),
                }
                return; // a drag is a copy, never a link click
            }
        }
        if let Some(url) = pending {
            self.open_url(&url);
        }
    }

    /// Anchor a selection at `(c, r)` inside the target pane's content rect, in the
    /// pane's current scrollback frame of reference.
    fn arm_selection(&mut self, target: SelTarget, c: u16, r: u16) {
        let Some(inner) = self.regions.main_inner else { return };
        self.drag_scroll = 0;
        let off = self.current_offset();
        let cell = cell_at(inner, off, c, r);
        self.drag = Some(Selection { target, anchor: cell, head: cell, moved: false });
    }

    /// Scrollback offset of the pane the main selection is over (0 when live or
    /// when there's no pane).
    fn current_offset(&self) -> usize {
        self.current_nav()
            .and_then(|n| self.pane_at(n))
            .map(|p| p.scrollback_offset())
            .unwrap_or(0)
    }

    /// Move the drag head to the cell under `(c, r)` at the pane's current offset.
    fn update_drag_head(&mut self, inner: Rect, c: u16, r: u16) {
        let off = self.current_offset();
        let cell = cell_at(inner, off, c, r);
        if let Some(sel) = self.drag.as_mut() {
            if cell != sel.anchor {
                sel.moved = true;
            }
            sel.head = cell;
        }
    }

    /// Anchor a diff selection at `(c, r)`. Endpoints are `(line index, content column)`
    /// — see [`diff_cell`](Self::diff_cell).
    fn arm_diff_selection(&mut self, c: u16, r: u16) {
        let Some(inner) = self.regions.main_inner else { return };
        self.drag_scroll = 0;
        let cell = self.diff_cell(inner, c, r);
        self.drag = Some(Selection { target: SelTarget::Diff, anchor: cell, head: cell, moved: false });
    }

    /// Move the diff drag head to the cell under `(c, r)`.
    fn update_diff_head(&mut self, inner: Rect, c: u16, r: u16) {
        let cell = self.diff_cell(inner, c, r);
        if let Some(sel) = self.drag.as_mut() {
            if cell != sel.anchor {
                sel.moved = true;
            }
            sel.head = cell;
        }
    }

    /// Map an absolute mouse position to a diff cell `(line index, content column)`. The
    /// row resolves to a line in the body (accounting for the pager scroll) and the
    /// column is measured from where the code starts — past the `gutter`-wide number and
    /// the sign — and clamped into that line's content, so the gutter and `+`/`-` are
    /// never part of a selection.
    fn diff_cell(&self, inner: Rect, c: u16, r: u16) -> (i32, u16) {
        let Some(v) = self.diff.as_ref() else { return (0, 0) };
        if v.lines.is_empty() {
            return (0, 0);
        }
        let ir = r.clamp(inner.y, inner.y + inner.height.saturating_sub(1)) - inner.y;
        let i = (ir as i32 + v.scroll as i32).clamp(0, v.lines.len() as i32 - 1);
        let start = inner.x + v.gutter as u16 + 2; // number + separating space + sign
        let len = v.lines[i as usize].content().chars().count() as u16;
        let cx = c.clamp(inner.x, inner.x + inner.width.saturating_sub(1));
        let col = cx.saturating_sub(start).min(len.saturating_sub(1));
        (i, col)
    }

    /// Extract the selected diff text — each line's code only (gutter number and sign
    /// stripped, see [`DiffLine::content`](crate::app::git::DiffLine::content)) — and put
    /// it on the clipboard.
    fn copy_diff_selection(&mut self, sel: Selection) {
        let Some(v) = self.diff.as_ref() else { return };
        let (lo, sc, hi, ec) = sel.ordered();
        let mut out: Vec<String> = Vec::new();
        for i in lo..=hi {
            let Some(line) = v.lines.get(i as usize) else { continue };
            let chars: Vec<char> = line.content().chars().collect();
            // Column span per line: full-line for the middle, open-ended at the edges.
            let (a, b) = if lo == hi {
                (sc, ec)
            } else if i == lo {
                (sc, u16::MAX)
            } else if i == hi {
                (0, ec)
            } else {
                (0, u16::MAX)
            };
            let start = (a as usize).min(chars.len());
            let end = (b as usize).saturating_add(1).min(chars.len()); // inclusive → exclusive
            out.push(chars[start..end.max(start)].iter().collect());
        }
        let text = trim_block(&out.join("\n"));
        if text.is_empty() {
            return;
        }
        let n = text.chars().count();
        crate::clipboard::copy(&text);
        self.flash(format!("copied {n} chars"));
    }

    /// One auto-scroll step in the armed direction, re-pinning the drag head to the
    /// edge it's held against. Called from `tick`, so it repeats on its own while
    /// the cursor sits at a pane edge. A no-op unless a drag is held at an edge; the
    /// scroll itself clamps, so it stops cleanly at the ends of the buffer.
    pub(crate) fn step_drag_scroll(&mut self) {
        let dir = self.drag_scroll;
        if dir == 0 || self.drag.is_none() {
            return;
        }
        let Some(inner) = self.regions.main_inner else { return };
        // The diff pager scrolls its own offset and re-pins the head to the edge line;
        // dir 1 (top edge) reveals earlier lines, so its scroll delta is negative.
        if matches!(self.drag, Some(sel) if sel.target == SelTarget::Diff) {
            self.diff_scroll(-dir * DRAG_SCROLL_STEP);
            let Some(v) = self.diff.as_ref() else { return };
            let line = if dir > 0 {
                v.scroll
            } else {
                (v.scroll + inner.height.saturating_sub(1) as usize).min(v.lines.len().saturating_sub(1))
            } as i32;
            if let Some(sel) = self.drag.as_mut() {
                sel.moved = true;
                sel.head.0 = line;
            }
            return;
        }
        if let Some(p) = self.current_nav().and_then(|n| self.pane_at(n)) {
            p.scroll(dir * DRAG_SCROLL_STEP);
        }
        let off = self.current_offset();
        let edge_row = if dir > 0 { 0 } else { inner.height.saturating_sub(1) };
        let line = edge_row as i32 - off as i32;
        if let Some(sel) = self.drag.as_mut() {
            if (line, sel.head.1) != sel.anchor {
                sel.moved = true;
            }
            sel.head.0 = line;
        }
    }

    /// Extract the selected text from the target pane and put it on the clipboard.
    fn copy_selection(&mut self, sel: Selection) {
        let (lo, sc, hi, ec) = sel.ordered();
        let pane = match sel.target {
            SelTarget::Main => self.current_nav().and_then(|n| self.pane_at(n)),
            SelTarget::Diff => return, // handled by `copy_diff_selection`
        };
        let Some(pane) = pane else { return };
        // `contents_block` takes an exclusive end column; +1 to include the cell
        // the cursor was released on.
        let Some(raw) = pane.contents_block(lo, hi, sc, ec + 1) else { return };
        let text = trim_block(&raw);
        if text.is_empty() {
            return;
        }
        let n = text.chars().count();
        crate::clipboard::copy(&text);
        self.flash(format!("copied {n} chars"));
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
        // Selecting any sidebar row other than the panel reveals that row in the main
        // pane, so drop any open diff preview occupying it.
        if !matches!(self.current_nav(), Some(Nav::Panel)) {
            self.clear_diff();
        }
        match self.current_nav() {
            // Launchers: single click selects, double click spawns / opens the form.
            Some(Nav::NewAgent(..)) | Some(Nav::NewTerminal(_)) | Some(Nav::NewProcess(_)) => {
                if double {
                    self.activate();
                } else {
                    self.focus = Focus::Sidebar;
                }
            }
            // The git panel: single click opens it.
            Some(Nav::Panel) => self.focus = Focus::Right,
            // Processes are monitored, not driven: a click selects one — its output
            // shows in the main pane — but never grabs keyboard focus, so you stay in
            // the sidebar and your keys keep driving the nav. Double-click restarts it
            // in place (start if stopped, respawn if running) without jumping in.
            Some(Nav::Session(i)) if self.sessions[i].kind == Kind::Process => {
                if double {
                    self.do_restart();
                }
                self.focus = Focus::Sidebar;
            }
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
    /// with a double-click staging the file / switching to the branch / (in Commits)
    /// showing the commit diff.
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
                    } else {
                        // Single click previews the file's diff in the main pane (it
                        // then follows the cursor). On a folder/root row this leaves
                        // the previous preview up.
                        self.git_open_diff();
                        // When the main pane isn't visible beside the panel (compact, or
                        // a narrow split where the focused panel borrows the main
                        // column), reveal the preview by focusing it.
                        if self.diff.is_some() && self.regions.main.is_none() {
                            self.focus = Focus::Terminal;
                        }
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
            Some(Section::Commits) => {
                if let Some(g) = self.active_git_mut() {
                    g.section = Section::Commits;
                }
                if let Some(&(_, idx)) = self.regions.git_commit_rows.iter().find(|(y, _)| *y == r) {
                    if let Some(g) = self.active_git_mut() {
                        g.commit_cursor = idx.min(g.log.len().saturating_sub(1));
                    }
                    // Single click shows the commit's diff (it then follows the cursor);
                    // reveal it when the main pane isn't visible beside the panel.
                    self.git_show_commit();
                    if self.diff.is_some() && self.regions.main.is_none() {
                        self.focus = Focus::Terminal;
                    }
                }
            }
            // A border: focus already moved to the column; leave the active section
            // unchanged.
            None => {}
        }
    }

    /// Which git box (if any) the point `(c, r)` falls in. Tested in order; the boxes
    /// don't overlap, so order only disambiguates a shared border pixel.
    fn git_section_at(&self, c: u16, r: u16) -> Option<Section> {
        if hit(self.regions.git_branches, c, r) {
            Some(Section::Branches)
        } else if hit(self.regions.git_commits, c, r) {
            Some(Section::Commits)
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
            // Scroll moves (and activates) the box under the cursor; over a border it
            // does nothing.
            if let Some(sec) = self.git_section_at(col, row) {
                let follows = sec != Section::Branches; // Changes / Commits drive a preview
                if let Some(g) = self.active_git_mut() {
                    g.section = sec;
                    g.move_cursor(if delta > 0 { -1 } else { 1 });
                }
                // Keep an open preview in step when scrolling the file / commit list.
                if follows {
                    self.git_preview_follow();
                }
            }
        } else if hit(self.regions.main, col, row) {
            // The diff preview is a pager, not a vt100 buffer — scroll its offset.
            if self.diff.is_some() {
                self.diff_scroll(if delta > 0 { -3 } else { 3 });
            } else if let Some(p) = self.current_nav().and_then(|n| self.pane_at(n)) {
                // On the alternate screen (nano, micro, less, …) there's no
                // scrollback to reveal, so hand the wheel to the program instead;
                // on the normal screen `wheel_input` returns `None` and we drive
                // our own scrollback as before.
                let (ox, oy) = self.regions.main_inner.map_or((0, 0), |r| (r.x, r.y));
                match p.wheel_input(delta > 0, 3, col, row, ox, oy) {
                    Some(bytes) => p.send(bytes),
                    None => p.scroll(delta),
                }
            }
        }
    }

    pub(crate) fn on_paste(&mut self, s: String) {
        // Paste into an open text prompt (commit message / a form's text step);
        // otherwise to the pane.
        match &mut self.overlay {
            Some(Overlay::Prompt { buf, .. }) => buf.push_str(&s),
            Some(Overlay::NewProcess(form)) if form.step != Step::Review => {
                form.buf.push_str(&s);
            }
            Some(Overlay::LinkProject(b)) => {
                b.query.push_str(&s);
                b.refilter();
            }
            _ if self.focus == Focus::Terminal => self.send_focused(s.into_bytes()),
            _ => {}
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

/// Extract the web URL under character column `col` of `line`, if any. Expands to the
/// whitespace-delimited token at the click, strips a wrapping pair and trailing
/// prose punctuation, and accepts only `http(s)://…` or `www.…` (the latter gains an
/// `https://` scheme). Deliberately conservative — a file path, a `v0.8.0`, or a
/// `foo.bar()` in code must never open a browser.
fn url_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if col >= chars.len() || chars[col].is_whitespace() {
        return None;
    }
    // Grow to the whitespace-delimited token around the clicked column.
    let mut start = col;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = col + 1;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let mut tok = &chars[start..end];
    // Peel a symmetric wrapper, e.g. (url) <url> [url] "url" 'url'.
    while tok.len() >= 2
        && matches!(
            (tok[0], tok[tok.len() - 1]),
            ('(', ')') | ('<', '>') | ('[', ']') | ('{', '}') | ('"', '"') | ('\'', '\'')
        )
    {
        tok = &tok[1..tok.len() - 1];
    }
    // Then any stray leading opener / trailing punctuation that clings to prose.
    let mut lo = 0;
    while lo < tok.len() && matches!(tok[lo], '(' | '<' | '[' | '{' | '"' | '\'') {
        lo += 1;
    }
    let mut hi = tok.len();
    while hi > lo
        && matches!(tok[hi - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | '>' | ']' | '}' | '"' | '\'')
    {
        hi -= 1;
    }
    let url: String = tok[lo..hi].iter().collect();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(url)
    } else if lower.starts_with("www.") && url.len() > 4 {
        Some(format!("https://{url}"))
    } else {
        None
    }
}

/// The base xterm button code for a crossterm mouse button (0 left, 1 middle,
/// 2 right) — what `Pane::mouse_input` expects.
fn button_code(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// What a drag-to-copy selection is happening over. The live main pane (a vt100 grid)
/// and the diff pager are selectable; the git panel is native text and isn't.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelTarget {
    /// The live vt100 pane — endpoints are pane buffer cells.
    Main,
    /// The diff pager — endpoints are `(line index, content column)`, where the column
    /// is measured from where the code starts (past the gutter number + sign), so the
    /// gutter and the `+`/`-` never fall inside the selection.
    Diff,
}

/// An in-progress (or just-released) mouse drag selection over a pane. Endpoints
/// are stored in *buffer* coordinates `(line, col)`: `col` is inner-relative, and
/// `line` is `inner_row - scrollback_offset` — a fixed buffer line that doesn't
/// move as the viewport scrolls (negative = scrolled up into history). The screen
/// row at any moment is `line + offset`; this is what lets a selection span more
/// than one screenful as the view auto-scrolls under the drag.
#[derive(Clone, Copy)]
pub(crate) struct Selection {
    pub target: SelTarget,
    pub anchor: (i32, u16),
    pub head: (i32, u16),
    pub moved: bool,
}

impl Selection {
    /// Endpoints in reading order as inclusive `(start_line, start_col, end_line,
    /// end_col)`, so start precedes end (by line, then column).
    pub(crate) fn ordered(&self) -> (i32, u16, i32, u16) {
        let (a, b) = (self.anchor, self.head);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        (s.0, s.1, e.0, e.1)
    }
}

/// Map an absolute mouse position to a buffer cell `(line, col)` inside `inner` at
/// scrollback offset `off`. The position is clamped to the content rect first, so
/// a drag past an edge lands on the edge row/column.
fn cell_at(inner: Rect, off: usize, c: u16, r: u16) -> (i32, u16) {
    let col = c.clamp(inner.x, inner.x + inner.width.saturating_sub(1)) - inner.x;
    let row = r.clamp(inner.y, inner.y + inner.height.saturating_sub(1)) - inner.y;
    (row as i32 - off as i32, col)
}

/// Lines auto-scrolled per step while a drag is held against a pane edge. Matches
/// the wheel step so the feel is consistent.
const DRAG_SCROLL_STEP: i32 = 3;

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

#[cfg(test)]
mod tests {
    use super::*;

    // A 10×5 content rect inset by the pane border at (1, 1).
    const INNER: Rect = Rect { x: 1, y: 1, width: 10, height: 5 };

    fn sel(anchor: (i32, u16), head: (i32, u16)) -> Selection {
        Selection { target: SelTarget::Main, anchor, head, moved: true }
    }

    #[test]
    fn cell_at_is_inner_relative_and_offset_adjusted() {
        // Live view (offset 0): line == inner row, col == inner col.
        assert_eq!(cell_at(INNER, 0, 3, 2), (1, 2));
        // Scrolled 3 into history: the same screen row names an older buffer line.
        assert_eq!(cell_at(INNER, 3, 3, 2), (-2, 2));
    }

    #[test]
    fn cell_at_clamps_past_the_edges() {
        // Above/left of the rect clamps to the top-left cell.
        assert_eq!(cell_at(INNER, 0, 0, 0), (0, 0));
        // Below/right clamps to the bottom-right cell (row 4, col 9).
        assert_eq!(cell_at(INNER, 0, 99, 99), (4, 9));
        // At an offset, the clamped top edge is an older line by `off`.
        assert_eq!(cell_at(INNER, 4, 0, 0), (-4, 0));
    }

    #[test]
    fn url_at_finds_link_under_the_click() {
        let line = "see https://example.com/path for details";
        // Anywhere inside the token resolves to the whole URL.
        assert_eq!(url_at(line, 4).as_deref(), Some("https://example.com/path"));
        assert_eq!(url_at(line, 20).as_deref(), Some("https://example.com/path"));
        // Whitespace and non-URL words don't.
        assert_eq!(url_at(line, 3), None); // the space
        assert_eq!(url_at(line, 0), None); // "see"
        assert_eq!(url_at(line, 100), None); // past the end
    }

    #[test]
    fn url_at_strips_wrappers_and_trailing_punctuation() {
        assert_eq!(url_at("(https://a.co)", 5).as_deref(), Some("https://a.co"));
        assert_eq!(url_at("<https://a.co>", 5).as_deref(), Some("https://a.co"));
        assert_eq!(url_at("visit https://a.co.", 10).as_deref(), Some("https://a.co"));
        assert_eq!(url_at("\"https://a.co\",", 5).as_deref(), Some("https://a.co"));
    }

    #[test]
    fn url_at_schemes_www_and_rejects_bare_words() {
        assert_eq!(url_at("www.example.com", 2).as_deref(), Some("https://www.example.com"));
        // No scheme and not www → not a link (paths, versions, code).
        assert_eq!(url_at("src/app/input.rs", 4), None);
        assert_eq!(url_at("v0.8.0", 2), None);
        assert_eq!(url_at("foo.bar()", 1), None);
        assert_eq!(url_at("example.com", 2), None);
    }

    #[test]
    fn ordered_sorts_by_line_then_column() {
        // Head above the anchor (smaller line) becomes the start.
        assert_eq!(sel((5, 2), (2, 9)).ordered(), (2, 9, 5, 2));
        // Same line: the smaller column leads.
        assert_eq!(sel((3, 8), (3, 2)).ordered(), (3, 2, 3, 8));
        // Already in order is unchanged.
        assert_eq!(sel((-4, 1), (0, 7)).ordered(), (-4, 1, 0, 7));
    }
}
