//! Session lifecycle driven from the sidebar: spawning new agents/terminals,
//! the start/stop/restart key actions, and the live config reload.

use super::git::GitPanel;
use super::nav::Nav;
use super::overlay::{Confirmed, Overlay};
use super::picker::Picker;
use super::procform::ProcForm;
use super::session::{Kind, Recipe, Session, Status};
use super::{App, Focus, Project};
use crate::config::{self, Config};
use std::collections::HashSet;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

/// How long [`run_stop_commands_on_quit`](App::run_stop_commands_on_quit) waits for
/// process teardown commands to finish before giving up, so a misbehaving `stop:` can't
/// wedge quit indefinitely. Generous enough for a real `docker compose down`.
const STOP_QUIT_WAIT: Duration = Duration::from_secs(30);

impl App {
    /// True when any agent, terminal, or process still has a live pane — i.e. quitting
    /// right now would kill running work.
    fn any_running(&self) -> bool {
        self.sessions.iter().any(Session::is_running)
    }

    /// `q` / the quit chip. Quitting ends the inner tmux session and kills every pane,
    /// so when anything is still running we confirm first (the modal offers detach as
    /// the non-destructive alternative). With nothing alive, quit straight away.
    pub(crate) fn request_quit(&mut self) {
        if self.any_running() {
            self.overlay = Some(Overlay::quit());
        } else {
            self.should_quit = true;
        }
    }

    pub(crate) fn spawn_agent(&mut self, pi: usize, t: usize) {
        let def = self.projects[pi].cfg.agents[t].clone();
        let recipe = Recipe::agent(&def, &self.projects[pi].cfg.dir);
        self.projects[pi].counts[t] += 1;
        let name = format!("{} #{}", def.name, self.projects[pi].counts[t]);
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Agent, recipe, pi);
        // Claude/Codex agents get resume bookkeeping so a restart reattaches to the
        // same conversation; any other agent command just spawns plainly.
        if let Some(tool) = crate::agent::Tool::detect(&s.recipe.cmd) {
            s.agent = Some(crate::agent::Resume::new(tool));
        }
        s.spawn(rows, cols);
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
    }

    pub(crate) fn spawn_terminal(&mut self, pi: usize) {
        let recipe = Recipe::shell(&self.projects[pi].cfg.dir);
        self.projects[pi].term_count += 1;
        let name = format!("Terminal #{}", self.projects[pi].term_count);
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Terminal, recipe, pi);
        s.spawn(rows, cols);
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
    }

    /// Raise the "+ New Process" form over project `pi`. The modal eats keys until
    /// the user finishes (writing the process to the config) or cancels.
    pub(crate) fn open_new_process(&mut self, pi: usize) {
        self.overlay = Some(Overlay::new_process(pi));
    }

    /// Raise the agent manager popup (`a`). It manages the built-in harnesses in the
    /// **global** config (`~/.mmux/config.yaml`) — the natural home for agents you reuse
    /// across projects — so a HOME we can't resolve is the one hard stop, flashed rather
    /// than opening a manager that has nowhere to save.
    pub(crate) fn open_agent_manager(&mut self) {
        if config::global_config_target().is_none() {
            self.flash("can't locate ~/.mmux (is HOME set?)");
            return;
        }
        self.overlay = Some(Overlay::agents());
    }

    /// Persist the agent manager's choices to the global config and reload so the
    /// sidebar reflects them immediately (new launchers appear, dropped ones vanish; a
    /// project that also defines an agent still wins the merge). A write failure is
    /// flashed and the overlay simply closes. Called from [`agentmgr_key`](super::input).
    pub(crate) fn apply_agent_manager(&mut self, m: &crate::agentmgr::AgentManager) {
        let Some(path) = config::global_config_target() else {
            self.flash("can't locate ~/.mmux (is HOME set?)");
            return;
        };
        let drafts = m.drafts();
        if let Err(e) = config::write_agents(&path, &drafts) {
            self.flash(format!("couldn't save agents — {e}"));
            return;
        }
        self.reload();
        self.flash(format!("agents updated — {} configured", drafts.len()));
        self.focus = Focus::Sidebar;
    }

    /// Raise the workspace manager. Deliberately available only in a manifest-backed
    /// session: creating a new workspace belongs to the terminal command
    /// (`mmux workspace`), while this popup manages the session the user is already in.
    pub(crate) fn open_workspace_manager(&mut self) {
        if !self.manifest {
            return;
        }
        match Overlay::workspace(&self.root) {
            Ok(overlay) => self.overlay = Some(overlay),
            Err(e) => self.flash(format!("couldn't open workspace editor — {e}")),
        }
    }

    /// Persist workspace identity/membership and reconcile it immediately. Name-only
    /// edits update the current chrome, additions append normally, and removed members
    /// drop their panes and runtime state. Reordering still waits for a reopen.
    pub(crate) fn apply_workspace_manager(&mut self, m: &crate::workspacemgr::WorkspaceManager) {
        let folders = m.folders();
        let before = self
            .cfg
            .workspace
            .as_ref()
            .map(|w| w.folders.clone())
            .unwrap_or_default();
        let path = config::workspace_config_path(&self.root);
        // Capture stable member identities in the restore snapshot before the manifest
        // order changes underneath it.
        self.save_state();
        if let Err(e) = config::write_workspace(&path, &m.name, &folders) {
            self.flash(format!("couldn't save workspace — {e}"));
            return;
        }
        self.reload();
        if folders == before {
            self.flash(format!("workspace updated — {} projects", folders.len()));
        } else if folders.len() == before.len() && folders.iter().all(|f| before.contains(f)) {
            self.flash("workspace saved — ordering applies on reopen");
        }
        self.focus = Focus::Sidebar;
    }

    /// Write a finished form's process to project `pi`'s `mmux.yaml` — appended for a
    /// new process, spliced in place when editing (`form.edit`) — then reload so the
    /// change shows up live. On success the row is selected (and, for a new autostart
    /// process, started); a write failure is flashed and nothing else changes. An edit
    /// whose command changed is picked up by [`reload`](Self::reload), which restarts a
    /// running instance so the new command takes effect without a manual restart.
    pub(crate) fn finish_new_process(&mut self, form: &ProcForm) {
        let pi = form.project;
        let path = crate::config::project_config_path(&self.projects[pi].cfg.dir);
        let (cmd, args) = crate::config::split_command(&form.command);
        let cwd = form.cwd.trim();
        let stop = form.stop.trim();
        let draft = crate::config::ProcessDraft {
            name: form.name.clone(),
            cmd,
            args,
            cwd: (!cwd.is_empty()).then(|| cwd.to_string()),
            autostart: form.autostart,
            stop: (!stop.is_empty()).then(|| stop.to_string()),
        };
        let res = match &form.edit {
            Some(old) => crate::config::replace_process(&path, old, &draft),
            None => crate::config::append_process(&path, &draft),
        };
        if let Err(e) = res {
            self.flash(format!("couldn't save process — {e}"));
            return;
        }
        // Pull the edited/new entry into the live session list, then select it.
        self.reload();
        let verb = if form.edit.is_some() {
            "updated"
        } else {
            "added"
        };
        self.flash(format!("{verb} process “{}”", draft.name));
        if let Some(i) = self
            .sessions
            .iter()
            .position(|s| s.project == pi && s.kind == Kind::Process && s.name == draft.name)
        {
            self.select_session(i);
            // A brand-new "start automatically" process is brought up now, not only on
            // the next open (`reload` adds it stopped). An edit leaves the run state be —
            // reload already restarts it if the command changed.
            if form.edit.is_none() && draft.autostart && !self.sessions[i].is_running() {
                let (rows, cols) = self.last_inner;
                self.sessions[i].spawn(rows, cols);
            }
        }
        self.focus = Focus::Sidebar;
    }

    /// `e`: reopen the guided form on the selected process, pre-filled for editing. A
    /// no-op on any non-process row (agents/terminals are throwaway instances, not
    /// config entries). Finishing the form rewrites the entry via [`finish_new_process`].
    pub(crate) fn edit_selected(&mut self) {
        let Some(Nav::Session(i)) = self.current_nav() else {
            return;
        };
        if self.sessions[i].kind != Kind::Process {
            return;
        }
        let (pi, name) = (self.sessions[i].project, self.sessions[i].name.clone());
        if let Some(def) = self.projects[pi]
            .cfg
            .processes
            .iter()
            .find(|p| p.name == name)
        {
            self.overlay = Some(Overlay::edit_process(pi, def));
        }
    }

    /// `D`: ask to confirm, then delete the selected process from its `mmux.yaml`. A
    /// no-op on any non-process row. The removal + reload happens in [`delete_process`]
    /// once the confirmation is accepted.
    pub(crate) fn delete_selected(&mut self) {
        let Some(Nav::Session(i)) = self.current_nav() else {
            return;
        };
        if self.sessions[i].kind != Kind::Process {
            return;
        }
        let (pi, name) = (self.sessions[i].project, self.sessions[i].name.clone());
        self.overlay = Some(Overlay::confirm(
            "Delete process",
            format!("Remove “{name}” from mmux.yaml?"),
            "y delete · n cancel",
            Confirmed::DeleteProcess { project: pi, name },
        ));
    }

    /// Delete process `name` from project `pi`: stop and drop any live instance, remove
    /// it from the config (preserving surrounding comments), then reload so the row is
    /// gone. A write failure is flashed and the (already-stopped) row simply reappears
    /// on the reload. Called from the delete confirmation ([`overlay_confirm`](super::App::overlay_confirm)).
    pub(crate) fn delete_process(&mut self, pi: usize, name: &str) {
        // Kill the running instance first so reload sees it fully gone rather than
        // keeping it as a running "orphan" of a dropped process.
        if let Some(idx) = self
            .sessions
            .iter()
            .position(|s| s.project == pi && s.kind == Kind::Process && s.name == name)
        {
            self.sessions[idx].kill();
            self.sessions.remove(idx);
        }
        let path = crate::config::project_config_path(&self.projects[pi].cfg.dir);
        if let Err(e) = crate::config::remove_process(&path, name) {
            self.flash(format!("couldn't delete — {e}"));
            return;
        }
        self.reload();
        self.flash(format!("deleted “{name}”"));
        self.focus = Focus::Sidebar;
    }

    /// Raise the Ctrl+P fuzzy file picker over the active project. Listing happens
    /// up front in [`Picker::new`]; the modal then eats keys until Enter/Esc.
    pub(crate) fn open_picker(&mut self) {
        let dir = self.projects[self.active].cfg.dir.clone();
        self.overlay = Some(Overlay::Picker(Picker::new(self.active, dir)));
    }

    /// Open the compact status-rich project switcher. Desktop has direct project boxes,
    /// but keeping the project-count guard here makes the footer action harmless.
    pub(crate) fn open_projects(&mut self) {
        if self.projects.len() > 1 {
            self.overlay = Some(Overlay::projects(self.active));
        }
    }

    /// Open `rel` (relative to project `pi`'s dir) in the user's editor as a new
    /// terminal-kind session, then focus it — so it takes over the main pane. Reuses
    /// the same spawn path as [`spawn_terminal`](Self::spawn_terminal); the editor
    /// row reads as a normal terminal and can be closed/returned-to like any other.
    pub(crate) fn open_in_editor(&mut self, pi: usize, rel: String) {
        // The editor session takes over the main pane, so drop any diff preview —
        // same as selecting something in the sidebar.
        self.clear_diff();
        let dir = self.projects[pi].cfg.dir.clone();
        let recipe = Recipe::editor(&dir, &rel);
        let base = std::path::Path::new(&rel)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| rel.clone());
        let name = format!("✎ {base}");
        let (rows, cols) = self.last_inner;
        let mut s = Session::new(name, Kind::Terminal, recipe, pi);
        s.spawn(rows, cols); // a terminal — it vanishes when the editor quits (see prune_exited)
        self.sessions.push(s);
        self.select_session(self.sessions.len() - 1);
        self.focus = Focus::Terminal;
    }

    /// Open the current row: spawn launchers, (re)start dead sessions, focus the pane.
    pub(crate) fn activate(&mut self) {
        let Some(nav) = self.current_nav() else {
            return;
        };
        // Opening anything but the panel puts a session in the main pane, so drop a
        // diff preview that was occupying it.
        if !matches!(nav, Nav::Panel) {
            self.clear_diff();
        }
        match nav {
            Nav::NewAgent(p, t) => {
                self.spawn_agent(p, t);
                self.focus = Focus::Terminal;
            }
            Nav::NewTerminal(p) => {
                self.spawn_terminal(p);
                self.focus = Focus::Terminal;
            }
            // The process launcher opens the guided form rather than spawning; the
            // modal takes over input, so focus stays on the sidebar.
            Nav::NewProcess(p) => self.open_new_process(p),
            Nav::Session(i) => {
                if !self.sessions[i].is_running() {
                    let (rows, cols) = self.last_inner;
                    self.sessions[i].spawn(rows, cols);
                }
                if let Some(p) = self.sessions[i].pane.as_ref() {
                    p.clear_attention();
                }
                self.focus = Focus::Terminal;
            }
            Nav::Panel => self.focus = Focus::Right,
        }
    }

    pub(crate) fn do_start(&mut self) {
        match self.current_nav() {
            Some(Nav::NewAgent(p, t)) => self.spawn_agent(p, t),
            Some(Nav::NewTerminal(p)) => self.spawn_terminal(p),
            Some(Nav::NewProcess(p)) => self.open_new_process(p),
            Some(Nav::Session(i)) if !self.sessions[i].is_running() => {
                let (rows, cols) = self.last_inner;
                self.sessions[i].spawn(rows, cols);
            }
            _ => {}
        }
    }

    pub(crate) fn do_stop(&mut self) {
        if let Some(Nav::Session(i)) = self.current_nav() {
            match self.sessions[i].kind {
                // Agents and terminals are throwaway instances: closing one removes
                // it for good rather than leaving an exited husk in the sidebar. One
                // with live work — unlike quit — isn't restored on reopen, so we
                // confirm first (mirroring the quit modal). "Live work" is the same
                // signal that spins the sidebar glyph: an agent that's actively
                // working (`busy`), or any running terminal (which has no working
                // signal to go quiet). An idle agent — running but awaiting you, its
                // spinner replaced by the green `●` — reads as done and closes with no
                // nag, so the prompt matches exactly what the sidebar shows.
                Kind::Agent | Kind::Terminal => {
                    let needs_confirm = match self.sessions[i].kind {
                        Kind::Agent => self.sessions[i].busy(),
                        _ => self.sessions[i].is_running(),
                    };
                    if !needs_confirm {
                        self.close_session(i);
                        return;
                    }
                    let name = self.sessions[i].name.clone();
                    let (title, noun) = match self.sessions[i].kind {
                        Kind::Terminal => ("Close terminal?", "terminal"),
                        _ => ("Close agent?", "agent"),
                    };
                    self.overlay = Some(Overlay::confirm(
                        title,
                        format!(
                            "“{name}” is still running. Closing it stops the {noun} and drops it."
                        ),
                        "y close · n cancel",
                        Confirmed::CloseSession {
                            project: self.sessions[i].project,
                            name,
                        },
                    ));
                }
                // Processes are config-defined entries: stop but keep the row so it
                // can be started again in place. A running one that declares a `stop:`
                // fires its teardown command — but only when it was actually running
                // (stopping an already-stopped process has nothing to tear down).
                _ => {
                    let was_running = self.sessions[i].is_running();
                    self.sessions[i].stop();
                    if was_running {
                        self.run_stop_command(i);
                    }
                }
            }
        }
    }

    /// Fire process `i`'s teardown command (its [`stop:`](crate::config::ProcessDef::stop))
    /// in the background, if it declares one. It runs detached on a throwaway thread that
    /// waits on the child (so it's reaped) while the UI stays responsive — a
    /// `docker compose down` can take a moment. The quit path
    /// ([`run_stop_commands_on_quit`](Self::run_stop_commands_on_quit)) waits instead, so
    /// the teardown finishes before mmux (and its tmux session) goes away.
    fn run_stop_command(&mut self, i: usize) {
        let Some(mut cmd) = self.sessions[i].stop_command() else {
            return;
        };
        let name = self.sessions[i].name.clone();
        thread::spawn(move || {
            let _ = cmd.status();
        });
        self.flash(format!("running stop command for “{name}”"));
    }

    /// On quit, run every still-running process's teardown command and **wait** for them,
    /// so something like `docker compose down` completes before mmux — and its tmux
    /// session — disappear. Each process's pane is killed first (it has stopped), then the
    /// commands run in parallel and we poll until they all finish or [`STOP_QUIT_WAIT`]
    /// elapses — a bounded wait so a misbehaving teardown can't wedge quit (Ctrl-C still
    /// escapes, and any straggler is orphaned like a plain kill would leave it). Called
    /// from [`run`](super::run) on a real quit only, never a self-update restart (where the
    /// processes come straight back).
    pub(crate) fn run_stop_commands_on_quit(&mut self) {
        let mut children = Vec::new();
        for s in self.sessions.iter_mut() {
            if s.kind == Kind::Process && s.is_running() {
                if let Some(mut cmd) = s.stop_command() {
                    if let Ok(child) = cmd.spawn() {
                        children.push(child);
                    }
                }
                s.stop();
            }
        }
        if children.is_empty() {
            return;
        }
        // The terminal is already restored here, so a plain line explains the brief pause.
        eprintln!("mmux: running {} process stop command(s)…", children.len());
        let deadline = Instant::now() + STOP_QUIT_WAIT;
        while !children.is_empty() && Instant::now() < deadline {
            // Keep only the ones still running (drop exited / un-waitable children).
            children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
            if children.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Kill an agent/terminal session and drop it from the sidebar entirely, then
    /// land the cursor sensibly within the same section.
    ///
    /// Closing a tab lands on the row that *slides into* the freed slot — the next
    /// sibling of the same kind (agent/terminal) in this project, so the cursor moves
    /// *down*, not back up. Special cases, in order:
    /// - **Last of its section** (no sibling below): fall back to the previous sibling
    ///   above it, since there's nothing below to slide up.
    /// - **Only one of its section** (no siblings at all): land on the section's
    ///   launcher just above the now-empty block — the last `+ New <agent>` row for
    ///   agents, or the `+ New Terminal` row for terminals.
    fn close_session(&mut self, i: usize) {
        let proj = self.sessions[i].project;
        let kind = self.sessions[i].kind;
        if kind == Kind::Agent
            && proj == self.active
            && self
                .sessions
                .iter()
                .filter(|s| s.project == proj && s.kind == Kind::Agent)
                .count()
                == 1
        {
            self.sticky_agent_project = Some(proj);
        }
        // Siblings of the same section (project + kind), in sidebar order — which is
        // just their order in `self.sessions` (see `push_sessions`).
        let siblings: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.project == proj && s.kind == kind)
            .map(|(j, _)| j)
            .collect();
        let at = siblings.iter().position(|&j| j == i).unwrap_or(0);
        // Pick the target as a `Nav` identity so it survives the removal below.
        let target = if at + 1 < siblings.len() {
            Nav::Session(siblings[at + 1]) // the sibling that slides up into this slot
        } else if at > 0 {
            Nav::Session(siblings[at - 1]) // last of the section — step to the previous
        } else {
            // Section is now empty: land on its launcher, the row just above the block.
            match kind {
                Kind::Agent => {
                    Nav::NewAgent(proj, self.projects[proj].cfg.agents.len().saturating_sub(1))
                }
                _ => Nav::NewTerminal(proj),
            }
        };
        self.sessions[i].kill();
        self.sessions.remove(i);
        // Session indices above `i` shifted down by one; a below-sibling target was one
        // of them, so re-point it. Launcher targets are index-free and unaffected.
        let target = match target {
            Nav::Session(j) if j > i => Nav::Session(j - 1),
            other => other,
        };
        let nav = self.build_nav();
        self.sel = nav
            .iter()
            .position(|n| *n == target)
            .unwrap_or_else(|| self.sel.min(nav.len().saturating_sub(1)));
        // Hand focus back to the sidebar (the closed row's pane is gone).
        self.focus = Focus::Sidebar;
    }

    /// Close the agent/terminal identified by (`project`, `name`) — the deferred half of
    /// the [`do_stop`](Self::do_stop) confirmation. Resolved by name at accept time (not a
    /// stashed index) so a prune between opening the modal and confirming can't close the
    /// wrong row. Called from the confirmation ([`overlay_confirm`](super::App::overlay_confirm)).
    pub(crate) fn close_named_session(&mut self, project: usize, name: &str) {
        if let Some(i) = self.sessions.iter().position(|s| {
            s.project == project && s.name == name && matches!(s.kind, Kind::Agent | Kind::Terminal)
        }) {
            self.close_session(i);
        }
    }

    /// Drop any agent or terminal that exited *cleanly*, so quitting an agent from
    /// inside it (`/quit`, Ctrl-D) or `exit`ing a terminal makes its row vanish instead
    /// of leaving an "exited" husk behind — the same way the Ctrl+P editor terminal
    /// already disappears. A **crash** (`Status::Failed`, non-zero exit on its own) is
    /// kept on purpose, painted red, so you notice it died badly. Processes are
    /// config-defined entries, so they keep their (stopped) row to be restarted in
    /// place. Called once per loop from [`tick`](super::App::tick).
    pub(crate) fn prune_exited(&mut self) {
        // The row still exists here, so its project has not demoted yet. Capture its
        // identity before removing anything or shifting session indices.
        let selected = self.current_nav();
        let dead: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind != Kind::Process && matches!(s.status(), Status::Exited))
            .map(|(i, _)| i)
            .collect();
        if dead.is_empty() {
            return;
        }
        // Keep the selected project's box where it is when the last agent quits from
        // inside its pane. A later project switch clears this pin and lets it demote.
        let active_loses_last_agent =
            dead.iter().any(|&i| {
                self.sessions[i].project == self.active && self.sessions[i].kind == Kind::Agent
            }) && !self.sessions.iter().enumerate().any(|(i, s)| {
                !dead.contains(&i) && s.project == self.active && s.kind == Kind::Agent
            });
        if active_loses_last_agent {
            self.sticky_agent_project = Some(self.active);
        }
        // If we're sitting in one of the dying panes, fall back to the sidebar.
        let focused_dead = self.focus == Focus::Terminal
            && matches!(self.current_nav(), Some(Nav::Session(i)) if dead.contains(&i));
        // Remove back-to-front so the earlier indices stay valid.
        for &i in dead.iter().rev() {
            self.sessions[i].kill();
            self.sessions.remove(i);
        }
        if focused_dead {
            self.focus = Focus::Sidebar;
        }
        let selected = selected.and_then(|n| match n {
            Nav::Session(i) if dead.contains(&i) => None,
            Nav::Session(i) => {
                let shift = dead.iter().filter(|&&d| d < i).count();
                Some(Nav::Session(i.saturating_sub(shift)))
            }
            other => Some(other),
        });
        let nav = self.build_nav();
        self.sel = selected
            .and_then(|want| nav.iter().position(|n| *n == want))
            .unwrap_or_else(|| self.sel.min(nav.len().saturating_sub(1)));
    }

    pub(crate) fn do_restart(&mut self) {
        match self.current_nav() {
            Some(Nav::Session(i)) => {
                let (rows, cols) = self.last_inner;
                self.sessions[i].spawn(rows, cols);
            }
            Some(Nav::NewAgent(p, t)) => self.spawn_agent(p, t),
            Some(Nav::NewTerminal(p)) => self.spawn_terminal(p),
            Some(Nav::NewProcess(p)) => self.open_new_process(p),
            Some(Nav::Panel) => {
                if let Some(g) = self.active_git_mut() {
                    g.refresh();
                }
            }
            None => {}
        }
    }

    /// Re-read every loaded project's `mmux.yaml` (+ the global config) in place and
    /// merge changes into the live session: new processes/agents appear, and a removed
    /// process that's still running is kept as an orphan rather than killed. A running
    /// process whose command (recipe) actually changed is **restarted** so the new
    /// command takes effect immediately — you no longer have to stop/start it by hand.
    /// Bound to `R` / `Ctrl-b R`.
    ///
    /// Existing projects are refreshed in place, keyed by directory. In a manifest
    /// workspace, newly listed folders append live and removed folders are dropped with
    /// their panes; retained projects are not reordered until the next open.
    pub(crate) fn reload(&mut self) {
        let mut failed = 0usize;
        let mut workspace_warnings = Vec::new();
        let mut added_projects = Vec::new();
        let mut removed_projects = Vec::new();

        // Reload the manifest before taking the project-dir snapshot below. Removed
        // member indices are compacted first; new canonical member dirs then append in
        // manifest order and participate in the ordinary config reconciliation.
        if self.manifest {
            match Config::load_workspace(&self.root) {
                Ok(ws) => {
                    self.cfg = ws.config;
                    workspace_warnings = ws.warnings;
                    if ws.manifest {
                        // Membership comes from the raw manifest, not only `ws.projects`:
                        // the latter omits temporarily unreadable members, which must keep
                        // their existing panes until explicitly removed from `folders`.
                        let wanted: HashSet<PathBuf> = self
                            .cfg
                            .workspace
                            .as_ref()
                            .into_iter()
                            .flat_map(|w| &w.folders)
                            .map(|folder| config::canonical(&self.root.join(folder)))
                            .collect();
                        removed_projects = self.remove_unlisted_projects(&wanted);
                        let mut loaded: HashSet<PathBuf> = self
                            .projects
                            .iter()
                            .map(|p| config::canonical(&p.cfg.dir))
                            .collect();
                        for cfg in ws.projects {
                            if loaded.insert(config::canonical(&cfg.dir)) {
                                if self.projects.len() >= config::MAX_PROJECTS {
                                    workspace_warnings.push(format!(
                                        "workspace already has {} live projects — ignoring additional members",
                                        config::MAX_PROJECTS
                                    ));
                                    break;
                                }
                                let pi = self.projects.len();
                                self.projects.push(Project::new(cfg));
                                self.last_proj_sel.push(None);
                                added_projects.push(pi);
                            }
                        }
                    }
                }
                Err(_) => failed += 1,
            }
        }

        // Reload each project's config by dir. A project whose config fails to load
        // keeps its current one (recorded as `None`) instead of aborting the reload.
        let dirs: Vec<PathBuf> = self.projects.iter().map(|p| p.cfg.dir.clone()).collect();
        let mut new_cfgs: Vec<Option<Config>> = Vec::with_capacity(dirs.len());
        for dir in &dirs {
            match Config::load(dir) {
                Ok(mut c) => {
                    c.workspace = None; // member projects never expand nested workspaces
                    new_cfgs.push(Some(c));
                }
                Err(_) => {
                    failed += 1;
                    new_cfgs.push(None);
                }
            }
        }

        // Processes: reconcile by (project, name) in one pass, preserving live panes
        // and refreshing recipes. Agents/terminals are spawned instances, left as-is.
        let (mut old_procs, others): (Vec<Session>, Vec<Session>) =
            std::mem::take(&mut self.sessions)
                .into_iter()
                .partition(|s| s.kind == Kind::Process);

        let (rows, cols) = self.last_inner;
        let mut next_procs: Vec<Session> = Vec::new();
        let mut added = 0usize;
        let mut restarted = 0usize;
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else {
                // Config failed to load: keep this project's processes untouched.
                let (mine, rest): (Vec<Session>, Vec<Session>) =
                    old_procs.into_iter().partition(|s| s.project == pi);
                next_procs.extend(mine);
                old_procs = rest;
                continue;
            };
            let dir = ncfg.dir.clone();
            for p in &ncfg.processes {
                let recipe = Recipe::process(p, &dir);
                match old_procs
                    .iter()
                    .position(|it| it.project == pi && it.name == p.name)
                {
                    Some(pos) => {
                        let mut item = old_procs.remove(pos);
                        // Only touch a live pane when the command genuinely changed — then
                        // respawn it so an edited command takes effect right away, instead
                        // of lingering on the old one until a manual restart.
                        if item.recipe != recipe {
                            item.recipe = recipe;
                            if item.is_running() {
                                item.spawn(rows, cols);
                                restarted += 1;
                            }
                        }
                        // The teardown command isn't part of the recipe (editing it must
                        // not restart a running process), so refresh it unconditionally.
                        item.stop = p.stop.clone();
                        next_procs.push(item);
                    }
                    None => {
                        let mut item = Session::new(p.name.clone(), Kind::Process, recipe, pi);
                        item.stop = p.stop.clone();
                        next_procs.push(item);
                        added += 1;
                    }
                }
            }
        }
        // Dropped processes (config removed them): keep running ones as orphans.
        let mut orphaned = 0usize;
        for mut item in old_procs {
            if item.is_running() {
                next_procs.push(item);
                orphaned += 1;
            } else {
                item.kill();
            }
        }
        self.sessions = others;
        self.sessions.extend(next_procs);

        // Agents: remap each project's "#N" counters by name so suffixes stay unique.
        let mut added_agents = 0usize;
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else { continue };
            let old_names: Vec<String> = self.projects[pi]
                .cfg
                .agents
                .iter()
                .map(|a| a.name.clone())
                .collect();
            let old_counts = self.projects[pi].counts.clone();
            self.projects[pi].counts = ncfg
                .agents
                .iter()
                .map(|a| {
                    old_names
                        .iter()
                        .position(|o| o == &a.name)
                        .map(|i| old_counts[i])
                        .unwrap_or(0)
                })
                .collect();
            added_agents += ncfg
                .agents
                .iter()
                .filter(|a| !old_names.iter().any(|o| o == &a.name))
                .count();
        }

        // Git panels: a project that became (or stopped being) a repo gains/loses its
        // panel. A live panel is left untouched so its cursor/staging survive reload.
        for (pi, ncfg) in new_cfgs.iter().enumerate() {
            let Some(ncfg) = ncfg else { continue };
            let want = ncfg.git_panel_enabled() && crate::git::is_repo(&ncfg.dir);
            match (want, self.projects[pi].git.is_some()) {
                (true, false) => self.projects[pi].git = Some(GitPanel::new(ncfg.dir.clone())),
                (false, true) => self.projects[pi].git = None,
                _ => {}
            }
        }

        // Commit the refreshed configs.
        for (pi, ncfg) in new_cfgs.into_iter().enumerate() {
            if let Some(ncfg) = ncfg {
                self.projects[pi].cfg = ncfg;
            }
        }
        if !self.manifest {
            if let Some(project) = self.projects.first() {
                self.cfg = project.cfg.clone();
            }
        }

        // A project brought in by this reload gets the same startup semantics it
        // would have had on a fresh open: configured autostart processes begin now.
        let autostart: Vec<(usize, String)> = added_projects
            .iter()
            .flat_map(|&pi| {
                self.projects[pi]
                    .cfg
                    .processes
                    .iter()
                    .filter(|p| p.autostart)
                    .map(move |p| (pi, p.name.clone()))
            })
            .collect();
        let mut autostarted = 0usize;
        for (pi, name) in autostart {
            if let Some(i) = self.sessions.iter().position(|s| {
                s.project == pi && s.kind == Kind::Process && s.name == name && !s.is_running()
            }) {
                self.sessions[i].spawn(rows, cols);
                autostarted += 1;
            }
        }

        // The nav list may have grown or shrunk; keep the selection in range.
        let navlen = self.build_nav().len();
        if removed_projects.is_empty() {
            self.sel = self.sel.min(navlen.saturating_sub(1));
        } else {
            self.sel = self
                .build_nav()
                .iter()
                .position(|n| self.project_of(*n) == Some(self.active))
                .unwrap_or(0);
            self.focus = Focus::Sidebar;
            // `apply_workspace_manager` snapshots before rewriting the manifest. Replace
            // that snapshot now so removed agents/terminals cannot return on reopen.
            self.save_state();
        }

        let mut parts: Vec<String> = Vec::new();
        if !added_projects.is_empty() {
            parts.push(format!("+{} project(s)", added_projects.len()));
        }
        match removed_projects.as_slice() {
            [] => {}
            [name] => parts.push(format!("removed project “{name}”")),
            names => parts.push(format!(
                "removed {} projects: {}",
                names.len(),
                names.join(", ")
            )),
        }
        if added > 0 {
            parts.push(format!("+{added} process(es)"));
        }
        if added_agents > 0 {
            parts.push(format!("+{added_agents} agent(s)"));
        }
        if restarted > 0 {
            parts.push(format!("{restarted} restarted"));
        }
        if autostarted > 0 {
            parts.push(format!("{autostarted} autostarted"));
        }
        if orphaned > 0 {
            parts.push(format!("{orphaned} orphaned"));
        }
        if failed > 0 {
            parts.push(format!("{failed} unreadable"));
        }
        parts.extend(workspace_warnings);
        let summary = if parts.is_empty() {
            "no changes".into()
        } else {
            parts.join(", ")
        };
        self.flash(format!("reloaded — {summary}"));
    }

    /// Drop manifest members no longer listed, killing their panes and compacting every
    /// positional project reference. A dirty Git worktree is irrelevant: the panel is
    /// only cached UI state, and no repository files are touched here.
    fn remove_unlisted_projects(&mut self, wanted: &HashSet<PathBuf>) -> Vec<String> {
        let remove: HashSet<usize> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| !wanted.contains(&config::canonical(&p.cfg.dir)))
            .map(|(pi, _)| pi)
            .collect();
        if remove.is_empty() {
            return Vec::new();
        }

        let old_active = self.active;
        let mut project_map = vec![None; self.projects.len()];
        let mut removed_names = Vec::new();
        let mut kept = Vec::new();
        for (old, project) in std::mem::take(&mut self.projects).into_iter().enumerate() {
            if remove.contains(&old) {
                removed_names.push(project.cfg.display_name());
            } else {
                project_map[old] = Some(kept.len());
                kept.push(project);
            }
        }
        self.projects = kept;

        let mut sessions = Vec::new();
        for mut session in std::mem::take(&mut self.sessions) {
            if let Some(project) = project_map[session.project] {
                session.project = project;
                sessions.push(session);
                continue;
            }
            // Removing a configured process has the same teardown semantics as stopping
            // it manually, but the workspace-level summary remains the only flash.
            let stop = if session.kind == Kind::Process && session.is_running() {
                session.stop_command()
            } else {
                None
            };
            session.kill();
            if let Some(mut cmd) = stop {
                thread::spawn(move || {
                    let _ = cmd.status();
                });
            }
        }
        self.sessions = sessions;

        self.active = project_map
            .get(old_active)
            .and_then(|mapped| *mapped)
            .unwrap_or_else(|| old_active.min(self.projects.len().saturating_sub(1)));
        self.sticky_agent_project = None;
        self.last_proj_sel = vec![None; self.projects.len()];
        self.clear_diff();
        removed_names
    }
}
