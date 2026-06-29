//! Input handling: keyboard (sidebar + pane, with the Ctrl-b leader), mouse
//! (focus routing, hamburger buttons, scrollback wheel) and paste.

use super::git::{Confirmed, Overlay, PromptKind, Section};
use super::keymap::encode_key;
use super::nav::Nav;
use super::procform::{ProcForm, Step};
use super::session::Kind;
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
            // Link another project directory into the workspace (also the button at the
            // bottom of the sidebar).
            KeyCode::Char('L') => self.open_link_browser(),
            // Apply a staged self-update (only acts when the "↻ restart to update" badge
            // is showing); otherwise a no-op.
            KeyCode::Char('U') => self.apply_update(),
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
                KeyCode::Char('q') => self.request_quit(),
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
                self.git_preview_follow();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(g) = self.active_git_mut() {
                    g.move_cursor(-1);
                }
                self.git_preview_follow();
            }
            // Tab moves between the Changes and Branches boxes; ⏎/space acts on the
            // active one (stage a file / switch to a branch); v previews the file's diff.
            KeyCode::Tab => self.git_section_toggle(),
            KeyCode::Char(' ') | KeyCode::Enter => self.git_activate(),
            KeyCode::Char('v') => self.git_toggle_diff(),
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

    /// Keys while a modal overlay is open: text entry for a prompt, list nav + live
    /// filter for the branch switcher. We resolve the keystroke into one action and
    /// apply it *after* the borrow ends, so we never reassign `overlay` mid-borrow.
    fn overlay_key(&mut self, k: KeyEvent) {
        // The guided process form carries enough state (and needs to read project
        // config for validation) that it gets its own handler.
        if matches!(self.overlay, Some(Overlay::NewProcess(_))) {
            self.procform_key(k);
            return;
        }
        // The link browser likewise needs `&mut self` on ⏎ (to grow the workspace),
        // so it's taken out and driven by its own handler.
        if matches!(self.overlay, Some(Overlay::LinkProject(_))) {
            self.linkbrowse_key(k);
            return;
        }
        enum Act {
            None,
            Close,
            Detach,
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
            // A confirmation: y/⏎ accepts, anything else (n/Esc) cancels. The quit
            // confirm additionally offers d to detach — keeping every pane alive.
            Some(Overlay::Confirm { action, .. }) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    Act::Confirm(action.clone())
                }
                KeyCode::Char('d') if matches!(action, Confirmed::Quit) => Act::Detach,
                _ => Act::Close,
            },
            // Handled above by `procform_key` / `linkbrowse_key`; arms kept for match
            // exhaustiveness.
            Some(Overlay::NewProcess(_)) | Some(Overlay::LinkProject(_)) => Act::None,
            None => Act::None,
        };
        match act {
            Act::None => {}
            Act::Close => self.overlay = None,
            Act::Detach => {
                self.overlay = None;
                crate::tmux::detach();
            }
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

    /// Keys for the "+ New Process" form. We take the form out of `self.overlay` for
    /// the duration so the handler can freely read project config (for validation)
    /// while mutating the form, then put it back unless the user finished/cancelled.
    fn procform_key(&mut self, k: KeyEvent) {
        let Some(Overlay::NewProcess(mut form)) = self.overlay.take() else {
            return;
        };
        match form.step {
            // Text steps: type to edit, ⏎ advances (after validation), Esc cancels.
            Step::Name | Step::Command | Step::Cwd => match k.code {
                KeyCode::Esc => return, // cancelled — leave overlay cleared
                KeyCode::Enter => self.procform_advance(&mut form),
                KeyCode::Backspace => {
                    form.buf.pop();
                    form.error = None;
                }
                KeyCode::Char(c) => {
                    form.buf.push(c);
                    form.error = None;
                }
                _ => {}
            },
            // Review: toggle autostart, ⏎ writes the process, Esc cancels.
            Step::Review => match k.code {
                KeyCode::Esc => return,
                KeyCode::Char('y') | KeyCode::Char('Y') => form.autostart = true,
                KeyCode::Char('n') | KeyCode::Char('N') => form.autostart = false,
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Tab => {
                    form.autostart = !form.autostart;
                }
                KeyCode::Enter => {
                    self.finish_new_process(&form); // sets its own flash + selection
                    return;
                }
                _ => {}
            },
        }
        self.overlay = Some(Overlay::NewProcess(form));
    }

    /// Validate the current text step and move to the next one, committing the buffer
    /// into the matching field and loading the next field's value to edit. Validation
    /// failures set `form.error` and stay put.
    fn procform_advance(&mut self, form: &mut ProcForm) {
        let val = form.buf.trim().to_string();
        match form.step {
            Step::Name => {
                if val.is_empty() {
                    form.error = Some("name can't be empty".into());
                    return;
                }
                if self.projects[form.project].cfg.processes.iter().any(|p| p.name == val) {
                    form.error = Some(format!("a process named “{val}” already exists"));
                    return;
                }
                form.name = val;
                form.step = Step::Command;
                form.buf = form.command.clone();
            }
            Step::Command => {
                if val.is_empty() {
                    form.error = Some("command can't be empty".into());
                    return;
                }
                form.command = val;
                form.step = Step::Cwd;
                form.buf = form.cwd.clone();
            }
            Step::Cwd => {
                form.cwd = val; // optional — blank means the project root
                form.step = Step::Review;
                form.buf.clear();
            }
            Step::Review => {}
        }
        form.error = None;
    }

    /// Keys for the "Link another project" browser. Like the process form it's taken
    /// out of `self.overlay` for the duration: ⏎ links the highlighted directory (which
    /// needs `&mut self` to grow the workspace, so it's done after the take), ←/→ walk
    /// the tree, and typing filters the current level.
    fn linkbrowse_key(&mut self, k: KeyEvent) {
        let Some(Overlay::LinkProject(mut b)) = self.overlay.take() else {
            return;
        };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return, // cancelled — leave the overlay cleared
            KeyCode::Enter => {
                if let Some(dir) = b.pick() {
                    self.link_project(dir); // grows the workspace, flashes, stays closed
                    return;
                }
            }
            KeyCode::Right | KeyCode::Tab => b.descend(),
            KeyCode::Left => b.ascend(),
            KeyCode::Up => b.move_sel(-1),
            KeyCode::Down => b.move_sel(1),
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => b.move_sel(-1),
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => b.move_sel(1),
            // Backspace edits the filter, or — once it's empty — steps up a level.
            KeyCode::Backspace => {
                if b.query.is_empty() {
                    b.ascend();
                } else {
                    b.query.pop();
                    b.refilter();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                b.query.push(c);
                b.refilter();
            }
            _ => {}
        }
        self.overlay = Some(Overlay::LinkProject(b));
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
        // The "Link another project" button pinned to the sidebar's bottom row.
        if hit(self.regions.link_btn, c, r) {
            self.open_link_browser();
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
            // A diff preview owns the pane: clicking focuses it for keyboard scrolling,
            // but there's no vt100 grid to drag-select.
            if self.diff.is_some() {
                self.focus = Focus::Terminal;
                return;
            }
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
            FooterAction::LinkProject => self.open_link_browser(),
            FooterAction::Detach => crate::tmux::detach(),
            FooterAction::Quit => self.request_quit(),
            FooterAction::FocusPanel => {
                if self.active_git().is_some() {
                    self.focus = Focus::Right;
                }
            }
            FooterAction::FocusSidebar => self.focus = Focus::Sidebar,
            FooterAction::SendLeaderB => self.send_focused(vec![0x02]),
            FooterAction::ApplyUpdate => self.apply_update(),
            FooterAction::GitSection => self.git_section_toggle(),
            FooterAction::GitActivate => self.git_activate(),
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
            1 // top edge → reveal older history
        } else if r + 1 >= inner.y + inner.height {
            -1 // bottom edge → back toward the present
        } else {
            0
        };
        self.update_drag_head(inner, c, r);
    }

    /// Release: a real drag copies its contents; a plain click just clears.
    fn on_left_up(&mut self) {
        self.drag_scroll = 0;
        if let Some(sel) = self.drag.take() {
            if sel.moved {
                self.copy_selection(sel);
            }
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
                let changes = sec == Section::Changes;
                if let Some(g) = self.active_git_mut() {
                    g.section = sec;
                    g.move_cursor(if delta > 0 { -1 } else { 1 });
                }
                // Keep an open preview in step when scrolling the file list.
                if changes {
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

/// Which pane a drag-to-copy selection is happening over. Only the main pane is
/// selectable — the git panel is native text, not a vt100 grid.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelTarget {
    Main,
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
    fn ordered_sorts_by_line_then_column() {
        // Head above the anchor (smaller line) becomes the start.
        assert_eq!(sel((5, 2), (2, 9)).ordered(), (2, 9, 5, 2));
        // Same line: the smaller column leads.
        assert_eq!(sel((3, 8), (3, 2)).ordered(), (3, 2, 3, 8));
        // Already in order is unchanged.
        assert_eq!(sel((-4, 1), (0, 7)).ordered(), (-4, 1, 0, 7));
    }
}
