//! Shared state for managing the built-in agent harnesses ([`crate::config::PRESETS`]).
//! It lists every preset as a toggleable row — tracking which are configured, which
//! launch [`Mode`] each runs in, and whether the command is on `PATH` — and produces the
//! [`AgentDraft`](crate::config::AgentDraft) list to write to the **global** config
//! (`~/.mmux/config.yaml`), the natural home for agents you reuse across projects.
//!
//! One model, three front-ends: the in-TUI popup (sidebar `a` — driven by
//! [`agentmgr_key`](crate::app) and drawn by `view::overlay::render_agentmgr`), the terminal
//! `mmux agents` picker, and the `mmux init` agent step (both in [`crate::wizard`]). Any
//! non-preset agents configured by hand are preserved untouched (kept in
//! [`AgentManager::custom`] and re-emitted on save); the manager only ever adds, drops,
//! or re-flags the presets it knows.

use crate::config;

/// The launch posture an agent runs in. mmux cycles a row forward through the modes its
/// preset actually offers: `Normal` is always available; `Auto`/`Danger` only when the
/// preset ships flags for them (see [`crate::config::AgentPreset`]). Each maps to a
/// sequence of CLI tokens present in the row's `args`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    /// The harness's own interactive default — every action prompts.
    Normal,
    /// Auto-accept file edits; still prompt for riskier actions (shell, network).
    Auto,
    /// Skip all approvals ("danger" / yolo).
    Danger,
}

impl Mode {
    /// The short tag shown next to a row (empty for the plain default).
    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Normal => "",
            Mode::Auto => "auto",
            Mode::Danger => "danger",
        }
    }
}

/// One preset's row. `args` carries the agent's current CLI args so cycling the mode
/// preserves anything hand-added — a [`Mode`] is just the presence (or absence) of the
/// preset's `auto`/`danger` flag tokens, rewritten in place by [`Row::cycle_mode`].
pub(crate) struct Row {
    pub name: &'static str,
    pub cmd: &'static str,
    pub blurb: &'static str,
    /// The preset's auto-accept-edits flags, if it has that mode.
    pub auto_flags: Option<&'static [&'static str]>,
    /// The preset's danger flags, if it has that mode (every shipped preset does).
    pub danger_flags: Option<&'static [&'static str]>,
    /// Whether this agent is configured (shown/spawnable).
    pub enabled: bool,
    /// Whether the agent's command was found on `PATH` — a hint that enabling it will
    /// actually launch. Probed once when the manager is built (see [`on_path`]).
    pub installed: bool,
    /// The agent's args — seeded from the existing config entry, else empty.
    pub args: Vec<String>,
}

impl Row {
    /// The mode this row currently launches in, read back from its `args`: danger wins
    /// over auto if both flag sets are somehow present, else auto, else the plain default.
    pub(crate) fn mode(&self) -> Mode {
        if self.danger_flags.is_some_and(|s| contains_seq(&self.args, s)) {
            Mode::Danger
        } else if self.auto_flags.is_some_and(|s| contains_seq(&self.args, s)) {
            Mode::Auto
        } else {
            Mode::Normal
        }
    }

    /// The modes this row can cycle through, in order — always Normal, then Auto and
    /// Danger for whichever the preset ships flags for.
    fn modes(&self) -> Vec<Mode> {
        let mut modes = vec![Mode::Normal];
        if self.auto_flags.is_some() {
            modes.push(Mode::Auto);
        }
        if self.danger_flags.is_some() {
            modes.push(Mode::Danger);
        }
        modes
    }

    /// Advance to the next available mode, wrapping past the last back to Normal.
    pub(crate) fn cycle_mode(&mut self) {
        let modes = self.modes();
        let cur = modes.iter().position(|&m| m == self.mode()).unwrap_or(0);
        self.set_mode(modes[(cur + 1) % modes.len()]);
    }

    /// Rewrite `args` to launch in `mode`: strip every known mode-flag sequence (so we
    /// never stack two modes), then append the target's, leaving hand-added args be.
    fn set_mode(&mut self, mode: Mode) {
        if let Some(s) = self.auto_flags {
            strip_seq(&mut self.args, s);
        }
        if let Some(s) = self.danger_flags {
            strip_seq(&mut self.args, s);
        }
        let seq = match mode {
            Mode::Normal => None,
            Mode::Auto => self.auto_flags,
            Mode::Danger => self.danger_flags,
        };
        if let Some(s) = seq {
            self.args.extend(s.iter().map(|t| t.to_string()));
        }
    }
}

/// Whether `args` contains `seq` as a contiguous run of tokens.
fn contains_seq(args: &[String], seq: &[&str]) -> bool {
    !seq.is_empty() && args.windows(seq.len()).any(|w| w.iter().zip(seq).all(|(a, b)| a == b))
}

/// Remove every contiguous occurrence of `seq` from `args` (flag *and* its value token).
fn strip_seq(args: &mut Vec<String>, seq: &[&str]) {
    if seq.is_empty() {
        return;
    }
    let mut i = 0;
    while i + seq.len() <= args.len() {
        if args[i..i + seq.len()].iter().zip(seq).all(|(a, b)| a == b) {
            args.drain(i..i + seq.len());
        } else {
            i += 1;
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
                    auto_flags: p.auto,
                    danger_flags: p.danger,
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
                    auto_flags: p.auto,
                    danger_flags: p.danger,
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

    /// Cycle the highlighted row forward through its available launch modes.
    pub(crate) fn cycle_mode(&mut self) {
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.cycle_mode();
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
        // A Claude-shaped preset: a multi-token auto mode and a single-token danger mode.
        Row {
            name,
            cmd: "x",
            blurb: "",
            auto_flags: Some(&["--permission-mode", "auto"]),
            danger_flags: Some(&["--dangerously-skip-permissions"]),
            enabled,
            installed: true,
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn cycle_walks_normal_auto_danger_and_wraps() {
        let mut r = row("Claude", true, &["--keep"]);
        assert_eq!(r.mode(), Mode::Normal);

        r.cycle_mode();
        assert_eq!(r.mode(), Mode::Auto);
        assert_eq!(r.args, vec!["--keep", "--permission-mode", "auto"]);

        r.cycle_mode();
        assert_eq!(r.mode(), Mode::Danger);
        // Auto's tokens are stripped before danger's is added; hand-added arg survives.
        assert_eq!(r.args, vec!["--keep", "--dangerously-skip-permissions"]);

        r.cycle_mode();
        assert_eq!(r.mode(), Mode::Normal);
        assert_eq!(r.args, vec!["--keep"]);
    }

    #[test]
    fn cycle_skips_auto_when_the_preset_has_none() {
        let mut r = row("Grok", true, &[]);
        r.auto_flags = None; // danger-only harness (Amp/opencode/Grok)
        assert_eq!(r.mode(), Mode::Normal);
        r.cycle_mode();
        assert_eq!(r.mode(), Mode::Danger); // auto is skipped
        r.cycle_mode();
        assert_eq!(r.mode(), Mode::Normal);
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
