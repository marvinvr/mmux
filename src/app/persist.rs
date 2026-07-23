//! Persisting the live agents/terminals and restoring them when a directory is
//! reopened — after a quit, a crash, or a self-update restart — so it feels like
//! nothing happened.
//!
//! The on-disk format and the resume mechanics live in [`crate::restore`] and
//! [`crate::agent`]; this is the `App`-side glue: snapshot on change (and once
//! more as the loop exits, with fresh cwds), and rebuild on the next start.

use super::session::{Kind, Recipe, Session};
use super::App;
use crate::agent::Resume;
use crate::restore::{self, SnapKind, Snapshot, State};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

impl App {
    /// The launch directory (canonical) — the key for this workspace's state file.
    /// For a manifest workspace this is the manifest's dir, NOT `projects[0]` (the
    /// first member folder), so a workspace and that folder opened solo each keep
    /// their own state.
    pub(crate) fn root_dir(&self) -> &Path {
        &self.root
    }

    /// Save the restore state only when the set of agents/terminals has actually
    /// changed since the last save — called every `tick`, so it must stay cheap.
    /// (Cursor moves and `cd`s don't trigger a write; the authoritative cwd/sel
    /// snapshot is taken once more from `run()` as the loop exits via [`save_state`].)
    pub(crate) fn maybe_save_state(&mut self) {
        let sig = self.session_signature();
        if self.restore_sig != Some(sig) {
            self.restore_sig = Some(sig);
            self.save_state();
        } else if self.refresh_agent_ids() {
            // Codex can create its rollout after the structural save that added
            // the row. Persist the id as soon as discovery succeeds.
            self.save_state();
        }
    }

    /// A cheap structural fingerprint of the restorable rows (name/kind/project),
    /// used to decide whether [`maybe_save_state`](Self::maybe_save_state) needs
    /// to rewrite the file. Excludes cwd/sel on purpose — those ride along on the
    /// writes a structural change triggers and on the final pre-exec write.
    fn session_signature(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in &self.sessions {
            if !is_restorable(s) {
                continue;
            }
            s.name.hash(&mut h);
            kind_tag(s.kind).hash(&mut h);
            s.project.hash(&mut h);
        }
        h.finish()
    }

    /// Snapshot every running agent/terminal to `~/.mmux/state/<hash>.yaml`. Reads
    /// each pane's **live** cwd (so a `cd` survives) and resolves each agent's resume
    /// id first (see [`refresh_agent_ids`](Self::refresh_agent_ids)). Processes are
    /// excluded — they return from config.
    pub(crate) fn save_state(&mut self) {
        self.refresh_agent_ids();
        let root = self.root_dir().to_path_buf();
        let mut sessions = Vec::new();
        for s in &self.sessions {
            if !is_restorable(s) {
                continue;
            }
            let kind = match s.kind {
                Kind::Agent => SnapKind::Agent,
                Kind::Terminal => SnapKind::Terminal,
                Kind::Process => continue,
            };
            let cwd = s
                .pane
                .as_ref()
                .and_then(|p| p.cwd())
                .unwrap_or_else(|| s.recipe.cwd.clone());
            let (tool, session_id) = match &s.agent {
                Some(r) => (Some(r.tool), r.id.clone()),
                None => (None, None),
            };
            sessions.push(Snapshot {
                name: s.name.clone(),
                kind,
                project: s.project,
                project_dir: self.projects.get(s.project).map(|p| {
                    crate::config::canonical(&p.cfg.dir)
                        .to_string_lossy()
                        .into_owned()
                }),
                cmd: s.recipe.cmd.clone(),
                args: s.recipe.args.clone(),
                cwd: cwd.to_string_lossy().into_owned(),
                env: s.recipe.env.clone(),
                tool,
                session_id,
            });
        }
        let state = State {
            version: restore::VERSION,
            sel: self.sel,
            sessions,
        };
        restore::save(&root, &state);
    }

    /// Make sure every agent is bound to the conversation id it should resume by,
    /// **without mixing up** several agents in the same directory.
    ///
    /// Claude *owns* its ids: each `Claude #N` is minted a UUID and launched with
    /// `--session-id`, so it already writes — and resumes — its own thread. Those ids
    /// are authoritative and are never reassigned. Codex has no such flag, so a fresh
    /// Codex agent has to *discover* the id of the session it just created by looking
    /// at the transcripts the tool recorded for its cwd; once it has one, it sticks.
    ///
    /// So: reserve every id already in use, then let only an id-less agent (a Codex
    /// first launch) adopt the first conversation created after that pane launched.
    /// It must also be a conversation no sibling has claimed. We
    /// deliberately do **not** chase the newest transcript for an already-bound agent.
    /// That directory is a shared history — closed agents, `/resume` targets, and plain
    /// `claude`/`codex` runs all pile up there — so "newest for this cwd" is *not*
    /// "the conversation this pane is in". Treating it that way is what made an idle
    /// agent restore onto a recently-active sibling's session.
    fn refresh_agent_ids(&mut self) -> bool {
        // Reserve every id already in use so discovery below can't hand the same
        // conversation to two agents.
        let mut claimed: HashSet<String> = self
            .sessions
            .iter()
            .filter_map(|s| s.agent.as_ref().and_then(|r| r.id.clone()))
            .collect();
        let mut changed = false;
        for s in &mut self.sessions {
            let Some(r) = s.agent.as_mut() else { continue };
            // Claude ids are owned; a Codex agent that already found its session keeps
            // it. Only an id-less Codex first launch still needs to discover one.
            if r.tool.owns_id() || r.id.is_some() {
                continue;
            }
            if !r.discovery_due() {
                continue;
            }
            let ranked = crate::agent::sessions_for(r.tool, &s.recipe.cwd);
            let started_at = r.started_at.unwrap_or(SystemTime::now());
            let found = ranked
                .into_iter()
                .filter(|(id, created)| *created >= started_at && !claimed.contains(id))
                .min_by_key(|(_, created)| *created);
            if let Some((id, _)) = found {
                claimed.insert(id.clone());
                r.id = Some(id);
                r.discover_at = None;
                changed = true;
            } else {
                r.defer_discovery();
            }
        }
        changed
    }

    /// Rebuild the saved agents/terminals after a self-update restart: respawn
    /// each (Claude/Codex resumed, everything else cold) at its saved cwd, bump
    /// the per-project name counters past the restored `#N`, and restore the
    /// selection. A no-op when there's no state file.
    pub(crate) fn restore_sessions(&mut self) {
        let root = self.root_dir().to_path_buf();
        let Some(state) = restore::load(&root) else {
            return;
        };
        if state.version != restore::VERSION {
            return;
        }
        let (rows, cols) = self.last_inner;
        for snap in state.sessions {
            let project = match snap.project_dir.as_deref() {
                Some(dir) => {
                    let wanted = crate::config::canonical(Path::new(dir));
                    self.projects
                        .iter()
                        .position(|p| crate::config::canonical(&p.cfg.dir) == wanted)
            }
                None => (snap.project < self.projects.len()).then_some(snap.project),
            };
            let Some(project) = project else {
                continue; // a workspace folder went away — skip its rows
            };
            let recipe = Recipe {
                cmd: snap.cmd,
                args: snap.args,
                cwd: PathBuf::from(&snap.cwd),
                env: snap.env,
            };
            let kind = match snap.kind {
                SnapKind::Agent => Kind::Agent,
                SnapKind::Terminal => Kind::Terminal,
            };
            let mut s = Session::new(snap.name, kind, recipe, project);
            if let Some(tool) = snap.tool {
                s.agent = Some(Resume::restored(tool, snap.session_id));
            }
            self.bump_counters(&s);
            s.spawn(rows, cols);
            self.sessions.push(s);
        }
        let navlen = self.build_nav().len();
        self.sel = state.sel.min(navlen.saturating_sub(1));
    }

    /// Advance a project's instance counters so a restored `Claude #2` /
    /// `Terminal #3` can't be re-issued to a freshly created row.
    fn bump_counters(&mut self, s: &Session) {
        let pi = s.project;
        let Some((base, n)) = split_instance(&s.name) else {
            return;
        };
        match s.kind {
            Kind::Agent => {
                if let Some(t) = self.projects[pi]
                    .cfg
                    .agents
                    .iter()
                    .position(|a| a.name == base)
                {
                    let c = &mut self.projects[pi].counts[t];
                    *c = (*c).max(n);
                }
            }
            Kind::Terminal if base == "Terminal" => {
                self.projects[pi].term_count = self.projects[pi].term_count.max(n);
            }
            _ => {}
        }
    }
}

/// A session worth saving: a *running* agent or terminal. Crashed/exited husks
/// and config-defined processes are skipped.
fn is_restorable(s: &Session) -> bool {
    matches!(s.kind, Kind::Agent | Kind::Terminal) && s.is_running()
}

/// Stable per-kind tag for the structural signature (Kind isn't `Hash`).
fn kind_tag(k: Kind) -> u8 {
    match k {
        Kind::Agent => 0,
        Kind::Terminal => 1,
        Kind::Process => 2,
    }
}

/// Split an instance name like `Claude #2` into `("Claude", 2)`; `None` for names
/// without a ` #<n>` suffix (e.g. a `✎ file.rs` editor row).
fn split_instance(name: &str) -> Option<(&str, usize)> {
    let (base, num) = name.rsplit_once(" #")?;
    Some((base, num.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::split_instance;

    #[test]
    fn splits_instance_names() {
        assert_eq!(split_instance("Claude #2"), Some(("Claude", 2)));
        assert_eq!(split_instance("Terminal #10"), Some(("Terminal", 10)));
        assert_eq!(split_instance("✎ main.rs"), None);
        assert_eq!(split_instance("Claude"), None);
    }
}
