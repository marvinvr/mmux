//! Shared state for managing the built-in agent harnesses ([`crate::config::PRESETS`]).
//! It lists every preset as a toggleable row — tracking which are configured, whether
//! each runs in danger mode, and whether the command is on `PATH` — and produces the
//! [`AgentDraft`](crate::config::AgentDraft) list to write to the **global** config
//! (`~/.mmux/config.yaml`), the natural home for agents you reuse across projects.
//!
//! One model, three front-ends: the in-TUI popup (sidebar `a` — driven by
//! [`agentmgr_key`](crate::app) and drawn by `view::git::render_agentmgr`), the terminal
//! `mmux agents` picker, and the `mmux init` agent step (both in [`crate::wizard`]). Any
//! non-preset agents configured by hand are preserved untouched (kept in
//! [`AgentManager::custom`] and re-emitted on save); the manager only ever adds, drops,
//! or re-flags the presets it knows.

use crate::config;

/// One preset's row. `args` carries the agent's current CLI args so a toggle preserves
/// anything hand-added — danger is just the preset's flag being present or not, flipped
/// in place by [`Row::toggle_danger`].
pub(crate) struct Row {
    pub name: &'static str,
    pub cmd: &'static str,
    pub blurb: &'static str,
    /// The preset's danger flag, if it has one (every shipped preset does).
    pub danger_flag: Option<&'static str>,
    /// Whether this agent is configured (shown/spawnable).
    pub enabled: bool,
    /// Whether the agent's command was found on `PATH` — a hint that enabling it will
    /// actually launch. Probed once when the manager is built (see [`on_path`]).
    pub installed: bool,
    /// The agent's args — seeded from the existing config entry, else empty.
    pub args: Vec<String>,
}

impl Row {
    /// Whether danger mode is on — the preset's flag is present in `args`.
    pub(crate) fn danger(&self) -> bool {
        matches!(self.danger_flag, Some(f) if self.args.iter().any(|a| a == f))
    }

    /// Flip danger mode: add or remove the preset's flag, leaving any other args be.
    pub(crate) fn toggle_danger(&mut self) {
        let Some(f) = self.danger_flag else { return };
        if self.args.iter().any(|a| a == f) {
            self.args.retain(|a| a != f);
        } else {
            self.args.push(f.to_string());
        }
    }
}

pub(crate) struct AgentManager {
    pub rows: Vec<Row>,
    /// Non-preset agents to carry through untouched on save.
    pub custom: Vec<config::AgentDraft>,
    pub cursor: usize,
}

impl AgentManager {
    /// Build the manager from the presets and the *global* config's current agents: a
    /// row per preset (enabled + args seeded from the matching global entry, if any),
    /// plus any non-preset global agents stashed in `custom` to re-emit verbatim. Used
    /// by the in-TUI popup and `mmux agents` (both edit an existing global config).
    pub(crate) fn new() -> AgentManager {
        let current = config::global_agents();
        let rows = config::PRESETS
            .iter()
            .map(|p| {
                let existing = current.iter().find(|a| a.name == p.name);
                Row {
                    name: p.name,
                    cmd: p.cmd,
                    blurb: p.blurb,
                    danger_flag: p.danger,
                    enabled: existing.is_some(),
                    installed: on_path(p.cmd),
                    args: existing.map(|a| a.args.clone()).unwrap_or_default(),
                }
            })
            .collect();
        let custom = current
            .into_iter()
            .filter(|a| config::preset_by_name(&a.name).is_none())
            .map(|a| config::AgentDraft { name: a.name, cmd: a.cmd, args: a.args })
            .collect();
        AgentManager { rows, custom, cursor: 0 }
    }

    /// A first-run manager for `mmux init`: a row per preset with **installed ones
    /// pre-checked** (a sensible default when there's no config to read yet) and no
    /// custom agents. Reads nothing from disk.
    pub(crate) fn fresh() -> AgentManager {
        let rows = config::PRESETS
            .iter()
            .map(|p| {
                let installed = on_path(p.cmd);
                Row {
                    name: p.name,
                    cmd: p.cmd,
                    blurb: p.blurb,
                    danger_flag: p.danger,
                    enabled: installed,
                    installed,
                    args: Vec::new(),
                }
            })
            .collect();
        AgentManager { rows, custom: Vec::new(), cursor: 0 }
    }

    pub(crate) fn move_cursor(&mut self, delta: i32) {
        let len = self.rows.len() as i32;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, len - 1) as usize;
    }

    pub(crate) fn toggle_enabled(&mut self) {
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.enabled = !r.enabled;
        }
    }

    pub(crate) fn toggle_danger(&mut self) {
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.toggle_danger();
        }
    }

    /// Select all rows, or clear them all if every row is already selected — the
    /// terminal picker's `a` shortcut for "all / none".
    pub(crate) fn toggle_all(&mut self) {
        let all_on = self.rows.iter().all(|r| r.enabled);
        for r in &mut self.rows {
            r.enabled = !all_on;
        }
    }

    /// The full agents list to write to the global config: every enabled preset (in
    /// preset order) followed by the preserved custom agents.
    pub(crate) fn drafts(&self) -> Vec<config::AgentDraft> {
        let mut out: Vec<config::AgentDraft> = self
            .rows
            .iter()
            .filter(|r| r.enabled)
            .map(|r| config::AgentDraft {
                name: r.name.to_string(),
                cmd: r.cmd.to_string(),
                args: r.args.clone(),
            })
            .collect();
        for c in &self.custom {
            out.push(config::AgentDraft {
                name: c.name.clone(),
                cmd: c.cmd.clone(),
                args: c.args.clone(),
            });
        }
        out
    }
}

/// Whether `cmd` resolves to a runnable file — an explicit path checked directly, else
/// a bare name looked up across `$PATH` (the shell's own resolution). Just a display
/// hint, so it's best-effort: a false negative only dims a `✓`.
fn on_path(cmd: &str) -> bool {
    use std::path::Path;
    if cmd.contains('/') {
        return Path::new(cmd).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(cmd).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &'static str, enabled: bool, args: &[&str]) -> Row {
        Row {
            name,
            cmd: "x",
            blurb: "",
            danger_flag: Some("--yolo"),
            enabled,
            installed: true,
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn danger_toggle_flips_the_flag_only() {
        let mut r = row("Gemini", true, &["--keep"]);
        assert!(!r.danger());
        r.toggle_danger();
        assert!(r.danger());
        assert_eq!(r.args, vec!["--keep", "--yolo"]);
        r.toggle_danger();
        assert!(!r.danger());
        assert_eq!(r.args, vec!["--keep"]); // hand-added arg survives
    }

    #[test]
    fn toggle_all_selects_then_clears() {
        let mut m = AgentManager {
            rows: vec![row("A", true, &[]), row("B", false, &[])],
            custom: vec![],
            cursor: 0,
        };
        m.toggle_all(); // not all on → turn all on
        assert!(m.rows.iter().all(|r| r.enabled));
        m.toggle_all(); // all on → turn all off
        assert!(m.rows.iter().all(|r| !r.enabled));
    }

    #[test]
    fn drafts_emit_enabled_presets_then_customs() {
        let m = AgentManager {
            rows: vec![row("Claude", true, &["--dangerously-skip-permissions"]), row("Codex", false, &[])],
            custom: vec![config::AgentDraft { name: "MyBot".into(), cmd: "mybot".into(), args: vec![] }],
            cursor: 0,
        };
        let d = m.drafts();
        let names: Vec<&str> = d.iter().map(|a| a.name.as_str()).collect();
        // Disabled Codex is dropped; the custom agent is preserved after the presets.
        assert_eq!(names, ["Claude", "MyBot"]);
        assert_eq!(d[0].args, vec!["--dangerously-skip-permissions"]);
    }
}
