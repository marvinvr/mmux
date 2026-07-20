//! Overlay state + input: the modal framework that floats above the whole UI.
//!
//! An [`Overlay`] is a single modal — a text [`Prompt`](Overlay::Prompt), a yes/no
//! [`Confirm`](Overlay::Confirm), the Ctrl+P file [`Picker`](Overlay::Picker), the
//! guided [`NewProcess`](Overlay::NewProcess) form, the [`Agents`](Overlay::Agents)
//! manager, the [`Workspace`](Overlay::Workspace) manager, the compact project
//! switcher, or the stateless
//! [`About`](Overlay::About) card. While one is open it eats
//! every key (see [`App::overlay_key`]); the rendering lives in [`super::view::overlay`].

use super::git::first_line;
use super::procform::{ProcForm, Step};
use super::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A modal over the whole UI: either a one-line text prompt (commit message /
/// new-branch name) or a yes/no confirmation (destructive discard). While open it eats
/// every key (see [`App::overlay_key`]).
pub(crate) enum Overlay {
    Prompt {
        title: &'static str,
        buf: String,
        kind: PromptKind,
    },
    Confirm {
        title: &'static str,
        body: String,
        /// The footer hint line, e.g. `"y discard · n cancel"` — wording varies per action.
        hint: &'static str,
        action: Confirmed,
    },
    /// The Ctrl+P fuzzy file picker (state in [`super::picker`]).
    Picker(super::picker::Picker),
    /// The "About mmux" card: version, the project's home/source links, and a live
    /// self-update status with the keys to check / apply. Stateless — it reads
    /// [`App::update`](super::App) at render time (see [`super::view::overlay::render_about`]).
    About,
    /// The "+ New Process" guided form (state in [`super::procform`]).
    NewProcess(super::procform::ProcForm),
    /// The agent manager (sidebar `a`): toggle the built-in harnesses on/off and cycle
    /// each one's launch mode, then write them to the global config (state in
    /// [`crate::agentmgr`]).
    Agents(crate::agentmgr::AgentManager),
    /// The workspace manifest manager (sidebar `w` in a manifest session): edit the
    /// workspace name, toggle folders, and reorder them (state in
    /// [`crate::workspacemgr`]).
    Workspace(crate::workspacemgr::WorkspaceManager),
    /// Compact project switcher. This is a stable project index rather than a row
    /// position because agent activity can change the display order while it is open.
    Projects { selected: usize },
}

#[derive(Clone, Copy)]
pub(crate) enum PromptKind {
    /// A commit-message prompt. `push` carries whether submitting should also kick off a
    /// background push: the prompt starts `false` (⏎ commits), and `Ctrl+⏎` upgrades it to
    /// commit-&-push.
    Commit {
        push: bool,
    },
    NewBranch,
}

/// The deferred action a [`Overlay::Confirm`] runs when accepted.
#[derive(Clone)]
pub(crate) enum Confirmed {
    /// Discard all changes under this pathspec (a file, a dir, or `.` for everything).
    Discard { path: String },
    /// Quit mmux. The inner tmux session ends with it, killing every running pane,
    /// so this is gated behind the modal whenever anything is still alive.
    Quit,
    /// Delete the named process from project `project`'s `mmux.yaml` (and stop any live
    /// instance), then reload. See [`App::delete_process`](super::App::delete_process).
    DeleteProcess { project: usize, name: String },
    /// Close the still-running agent/terminal named `name` in project `project` — kills
    /// its pane and drops the row. Identified by name (not index) so it survives a prune
    /// while the modal is open. See [`App::close_named_session`](super::App::close_named_session).
    CloseSession { project: usize, name: String },
    /// Revert the commit `hash` (`git revert --no-edit`) — a new commit undoing it.
    Revert { hash: String },
    /// Soft-reset HEAD to `hash` (`git reset --soft`) — move the tip back, keep the later
    /// changes staged. Recoverable, but rewrites the branch, so it's gated behind confirm.
    SoftReset { hash: String },
    /// Run `brew upgrade mmux` to apply the offered self-update on a Homebrew install
    /// (see [`App::start_brew_upgrade`](super::App::start_brew_upgrade)).
    BrewUpgrade { version: String },
}

impl Overlay {
    pub(crate) fn commit() -> Overlay {
        Overlay::Prompt {
            title: "Commit message",
            buf: String::new(),
            kind: PromptKind::Commit { push: false },
        }
    }

    pub(crate) fn new_branch(prefill: String) -> Overlay {
        Overlay::Prompt {
            title: "New branch",
            buf: prefill,
            kind: PromptKind::NewBranch,
        }
    }

    pub(crate) fn confirm(
        title: &'static str,
        body: String,
        hint: &'static str,
        action: Confirmed,
    ) -> Overlay {
        Overlay::Confirm {
            title,
            body,
            hint,
            action,
        }
    }

    /// The pre-quit confirmation. Quitting tears down the inner tmux session, stopping
    /// every running pane — but reopening the directory restores the agents/terminals
    /// (see [`crate::restore`]), so this is a calm heads-up, not a danger gate. Detach
    /// (offered right in the modal) keeps everything running live, uninterrupted.
    pub(crate) fn quit() -> Overlay {
        Overlay::Confirm {
            title: "Quit mmux?",
            body: "This stops all your agents, terminals, and processes.\n\
                   Detach instead to keep them running in the background."
                .into(),
            hint: "y quit · d detach · n cancel",
            action: Confirmed::Quit,
        }
    }

    pub(crate) fn new_process(project: usize) -> Overlay {
        Overlay::NewProcess(super::procform::ProcForm::new(project))
    }

    /// The process form pre-filled to edit `def` (see [`super::procform::ProcForm::edit`]).
    pub(crate) fn edit_process(project: usize, def: &crate::config::ProcessDef) -> Overlay {
        Overlay::NewProcess(super::procform::ProcForm::edit(project, def))
    }

    /// The agent manager, seeded from the presets + the current global config.
    pub(crate) fn agents() -> Overlay {
        Overlay::Agents(crate::agentmgr::AgentManager::new())
    }

    pub(crate) fn workspace(root: &std::path::Path) -> anyhow::Result<Overlay> {
        Ok(Overlay::Workspace(
            crate::workspacemgr::WorkspaceManager::new(root)?,
        ))
    }

    pub(crate) fn projects(selected: usize) -> Overlay {
        Overlay::Projects { selected }
    }
}

impl App {
    /// Keys while a modal overlay is open: text entry for a prompt, list nav + live
    /// filter for the branch switcher. We resolve the keystroke into one action and
    /// apply it *after* the borrow ends, so we never reassign `overlay` mid-borrow.
    pub(crate) fn overlay_key(&mut self, k: KeyEvent) {
        // The guided process form carries enough state (and needs to read project
        // config for validation) that it gets its own handler.
        if matches!(self.overlay, Some(Overlay::NewProcess(_))) {
            self.procform_key(k);
            return;
        }
        // The About card's actions (check / apply update) need `&mut self`, so it gets
        // its own handler rather than threading through the `Act` resolution below.
        if matches!(self.overlay, Some(Overlay::About)) {
            self.about_key(k);
            return;
        }
        // The agent manager writes the global config and reloads on save, so it also
        // needs `&mut self` and gets its own handler.
        if matches!(self.overlay, Some(Overlay::Agents(_))) {
            self.agentmgr_key(k);
            return;
        }
        if matches!(self.overlay, Some(Overlay::Workspace(_))) {
            self.workspacemgr_key(k);
            return;
        }
        if matches!(self.overlay, Some(Overlay::Projects { .. })) {
            self.projects_key(k);
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
                // ⏎ submits as-is; Ctrl+⏎ on a commit prompt upgrades it to commit-&-push —
                // but that needs a terminal that reports the modifier (see `run`'s keyboard
                // enhancement flags), and tmux drops it unless server-wide extended-keys is
                // on, which we don't touch. So Ctrl+P (below) is the always-reliable path:
                // a plain control byte that survives tmux on any terminal.
                KeyCode::Enter => {
                    let kind = match kind {
                        PromptKind::Commit { push } => PromptKind::Commit {
                            push: *push || ctrl,
                        },
                        other => *other,
                    };
                    Act::Submit(kind, buf.clone())
                }
                // Ctrl+P: reliable commit-&-push (no-op on a non-commit prompt).
                KeyCode::Char('p') if ctrl => match kind {
                    PromptKind::Commit { .. } => {
                        Act::Submit(PromptKind::Commit { push: true }, buf.clone())
                    }
                    _ => Act::None,
                },
                KeyCode::Backspace => {
                    buf.pop();
                    Act::None
                }
                // Type printable keys; swallow other control chords so they don't land in
                // the buffer (Ctrl+P is handled above).
                KeyCode::Char(c) if !ctrl => {
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
            // Handled above by their dedicated key handlers; arms kept for match
            // exhaustiveness.
            Some(Overlay::NewProcess(_))
            | Some(Overlay::About)
            | Some(Overlay::Agents(_))
            | Some(Overlay::Workspace(_))
            | Some(Overlay::Projects { .. }) => Act::None,
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

    /// Apply a submitted text prompt: commit the index, or create+switch a branch.
    pub(crate) fn overlay_submit(&mut self, kind: PromptKind, buf: String) {
        let buf = buf.trim().to_string();
        if buf.is_empty() {
            return;
        }
        match kind {
            PromptKind::Commit { push } => match self.active_git_mut().map(|g| g.commit(&buf)) {
                Some(Ok(s)) if push => {
                    // Commit landed — chain a background push (its result overwrites this
                    // flash from `tick` when the network op returns).
                    self.git_start("push");
                    self.flash(format!("{} · pushing…", first_line(&s)));
                }
                Some(Ok(s)) => self.flash(first_line(&s)),
                Some(Err(e)) => self.flash(first_line(&e)),
                _ => {}
            },
            PromptKind::NewBranch => match self.active_git_mut().map(|g| g.create_branch(&buf)) {
                Some(Ok(())) => self.flash(format!("switched to {buf}")),
                Some(Err(e)) => self.flash(first_line(&e)),
                _ => {}
            },
        }
    }

    /// Run an accepted [`Overlay::Confirm`] action (called after the modal closes).
    pub(crate) fn overlay_confirm(&mut self, action: Confirmed) {
        match action {
            Confirmed::Discard { path } => match self.active_git_mut().map(|g| g.discard(&path)) {
                Some(Ok(())) => self.flash(format!("discarded {path}")),
                Some(Err(e)) => self.flash(first_line(&e)),
                None => {}
            },
            Confirmed::Quit => self.should_quit = true,
            Confirmed::DeleteProcess { project, name } => self.delete_process(project, &name),
            Confirmed::CloseSession { project, name } => self.close_named_session(project, &name),
            Confirmed::Revert { hash } => {
                let r = self.active_git_mut().map(|g| g.revert(&hash));
                self.flash_result(r);
            }
            Confirmed::SoftReset { hash } => {
                let r = self.active_git_mut().map(|g| g.soft_reset(&hash));
                self.flash_result(r);
            }
            Confirmed::BrewUpgrade { version } => self.start_brew_upgrade(version),
        }
    }

    /// Keys for the "About mmux" card: `c` runs a manual update check, `u` applies an
    /// update (restarts in place for a staged one, or confirms the `brew upgrade` for a
    /// brew install), and Esc/`q`/`?` close it. Both update actions are guarded inside
    /// their handlers, so they're harmless no-ops when nothing's pending.
    fn about_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => self.overlay = None,
            KeyCode::Char('c') | KeyCode::Char('C') => self.check_now(),
            KeyCode::Char('u') | KeyCode::Char('U') => {
                self.overlay = None;
                self.apply_update();
            }
            _ => {}
        }
    }

    /// Keys for the agent manager: navigate the preset rows, `space` toggles an agent
    /// on/off, `m` cycles its launch mode (normal → auto → danger), ⏎ saves (writes the
    /// global config + reloads), Esc/`q` cancels. Taken out of `self.overlay` for the
    /// duration since saving needs `&mut self`; put back unless the user saved/cancelled.
    fn agentmgr_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Agents(mut m)) = self.overlay.take() else {
            return;
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return, // cancelled — overlay stays cleared
            KeyCode::Up | KeyCode::Char('k') => m.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => m.move_cursor(1),
            KeyCode::Char(' ') => m.toggle_enabled(),
            KeyCode::Char('m') => m.cycle_mode(),
            KeyCode::Enter => {
                self.apply_agent_manager(&m); // writes the global config, reloads, flashes
                return;
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Agents(m));
    }

    /// Keys for the manifest workspace manager. `n` edits its display name; the list
    /// uses the same checkbox vocabulary as the agent manager, with `J/K` additionally
    /// changing persisted manifest order. Saving structural edits reconciles additions
    /// and removals live; manifest reordering applies on reopen.
    fn workspacemgr_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Workspace(mut m)) = self.overlay.take() else {
            return;
        };
        if m.editing_name {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => {
                    m.editing_name = false;
                    m.error = None;
                }
                KeyCode::Backspace => {
                    m.name.pop();
                    m.error = None;
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    m.name.push(c);
                    m.error = None;
                }
                _ => {}
            }
            self.overlay = Some(Overlay::Workspace(m));
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return,
            KeyCode::Up | KeyCode::Char('k') => m.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => m.move_cursor(1),
            KeyCode::Char('K') => m.reorder(-1),
            KeyCode::Char('J') => m.reorder(1),
            KeyCode::Char(' ') => m.toggle_enabled(),
            KeyCode::Char('a') => m.toggle_all(),
            KeyCode::Char('n') => m.editing_name = true,
            KeyCode::Enter if m.validate() => {
                self.apply_workspace_manager(&m);
                return;
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Workspace(m));
    }

    /// Keys for the compact project switcher. Enter activates the chosen project and
    /// restores the sidebar row last selected there.
    fn projects_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Projects { mut selected }) = self.overlay.take() else {
            return;
        };
        let order = self.project_display_order();
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('[') => {
                let pos = order.iter().position(|&pi| pi == selected).unwrap_or(0);
                selected = order[pos.saturating_sub(1)];
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char(']') => {
                let pos = order.iter().position(|&pi| pi == selected).unwrap_or(0);
                selected = order[(pos + 1).min(order.len().saturating_sub(1))];
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                self.focus_project(selected);
                self.focus = super::Focus::Sidebar;
                return;
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Projects { selected });
    }

    /// Move the compact project picker's selection from mouse-wheel input. Keeping
    /// this with the keyboard handler ensures both use the same live display order.
    pub(crate) fn move_projects_picker(&mut self, delta: i32) {
        let selected = match self.overlay.as_ref() {
            Some(Overlay::Projects { selected }) => *selected,
            _ => return,
        };
        let order = self.project_display_order();
        let pos = order.iter().position(|&pi| pi == selected).unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, order.len() as i32 - 1) as usize;
        if let Some(Overlay::Projects { selected }) = self.overlay.as_mut() {
            *selected = order[next];
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
            Step::Name | Step::Command | Step::Cwd | Step::Stop => match k.code {
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
                // A duplicate name is rejected — but when editing, the entry keeping its
                // own name isn't a duplicate of itself.
                let dup = self.projects[form.project]
                    .cfg
                    .processes
                    .iter()
                    .any(|p| p.name == val && form.edit.as_deref() != Some(p.name.as_str()));
                if dup {
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
                form.step = Step::Stop;
                form.buf = form.stop.clone();
            }
            Step::Stop => {
                form.stop = val; // optional — blank means no teardown command
                form.step = Step::Review;
                form.buf.clear();
            }
            Step::Review => {}
        }
        form.error = None;
    }
}
