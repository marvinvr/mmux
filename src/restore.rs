//! Persisted workspace state, so reopening a directory brings the same agents and
//! terminals back — after a quit, a crash, or a [self-update](crate::update)
//! restart.
//!
//! mmux's panes are PTYs the inner process owns directly, so once that process
//! goes away (quit/crash, or the in-place re-exec an update does) they're gone. To
//! make a reopen seamless we snapshot the live agents/terminals to a small YAML
//! file under `~/.mmux/state/` keyed by the same canonical-dir hash tmux uses,
//! then rebuild them on the next start (see [`crate::app`]). Claude/Codex rows
//! additionally carry their session id so they *resume* their conversation rather
//! than starting cold (see [`crate::agent`]); a terminal carries its **live** cwd
//! so a `cd` survives.
//!
//! This is a convenience, never load-bearing: a missing or unparsable file just
//! means a fresh start, so every read/write swallows its errors.

use crate::agent::Tool;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The persisted snapshot of one workspace's restorable sessions.
#[derive(Serialize, Deserialize)]
pub struct State {
    /// Schema version, so a future change can reject an incompatible file.
    pub version: u32,
    /// The nav row that was selected, restored after the rows are rebuilt.
    #[serde(default)]
    pub sel: usize,
    /// Agents and terminals to bring back, in display order. Processes are
    /// deliberately omitted — they come back from config (autostart) or a click.
    pub sessions: Vec<Snapshot>,
}

/// One restorable session. Mirrors the bits of `Session`/`Recipe` needed to
/// respawn it; the `tool`/`session_id` pair is present only for Claude/Codex.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub kind: SnapKind,
    /// Legacy positional member identity. Kept so older state files still deserialize
    /// and standalone projects remain cheap to restore.
    pub project: usize,
    /// Stable member identity added with the workspace manager. Reordering a manifest
    /// can no longer send an agent or terminal into a different project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The pane's working directory at save time (so a shell's `cd` survives),
    /// falling back to the recipe's launch dir when the live cwd can't be read.
    pub cwd: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// The two session kinds that get restored (processes never do).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SnapKind {
    Agent,
    Terminal,
}

/// The current schema version written into [`State::version`].
pub const VERSION: u32 = 1;

/// The state file for the workspace rooted at `root`:
/// `~/.mmux/state/<session-name>.yaml`, sharing tmux's canonical-dir hash so it's
/// one file per workspace. `None` if `$HOME` is unset.
pub fn path_for(root: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let name = crate::tmux::session_name(&crate::config::canonical(root));
    Some(
        PathBuf::from(home)
            .join(".mmux")
            .join("state")
            .join(format!("{name}.yaml")),
    )
}

/// Write `state` for the workspace at `root`, creating `~/.mmux/state/` as needed.
/// Best-effort: any failure is silently dropped (the feature is non-critical).
pub fn save(root: &Path, state: &State) {
    let Some(path) = path_for(root) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_yaml::to_string(state) {
        let _ = std::fs::write(&path, text);
    }
}

/// Load the saved state for the workspace at `root`, or `None` if there's no file
/// (or it can't be read/parsed — treated the same as "nothing to restore").
pub fn load(root: &Path) -> Option<State> {
    let path = path_for(root)?;
    let text = std::fs::read_to_string(&path).ok()?;
    serde_yaml::from_str(&text).ok()
}

/// Enrich an existing positional snapshot with stable project directory identities.
/// Called immediately before a workspace editor writes a potentially reordered
/// manifest. Best-effort like all restore state: no file or an unreadable one is fine.
pub fn bind_project_dirs(root: &Path, projects: &[PathBuf]) {
    let Some(mut state) = load(root) else { return };
    if state.version != VERSION {
        return;
    }
    let mut changed = false;
    for snap in &mut state.sessions {
        if snap.project_dir.is_none() {
            if let Some(dir) = projects.get(snap.project) {
                snap.project_dir =
                    Some(crate::config::canonical(dir).to_string_lossy().into_owned());
                changed = true;
            }
        }
    }
    if changed {
        save(root, &state);
    }
}
