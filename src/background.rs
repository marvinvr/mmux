//! Reading a Claude Code agent's background work off its screen: sub-agents, shells
//! and monitors it started with `run_in_background` and will be woken by (a
//! `task-notification` starts a new turn when one finishes or reports). While any
//! run, Claude sits at an empty prompt looking idle, yet it isn't done — so `mmux
//! wait` holds on until they're gone.
//!
//! The screen is the one live source. The transcript records launches and
//! notifications, but a sub-agent notifies "completed" while it still waits on work
//! of its own (and notifies again later), a shell killed from `/tasks` or lost with
//! its process records no end, and the `turn_duration` record's
//! `pendingBackgroundAgentCount` is a turn-end snapshot that leaves shells out.
//! Claude's footer, below the input box, is redrawn from its live task registry:
//!
//! ```text
//! ──────────────────────────────────────────────
//! ❯
//! ──────────────────────────────────────────────
//!   Opus 5.5 | mmux • main                          ← a custom statusline
//!   ⏵⏵ auto mode on · 2 shells · ← for agents       ← the mode line's task pill
//!
//!   ⏺ main                                          ← the sub-agent tree
//!   ◯ general-purpose  Fix the parser    1m 2s · ↓ 30k tokens
//! ```
//!
//! The pill is one ` · `-separated segment worded as Claude words it (`1 shell`,
//! `2 shells, 1 monitor`, `3 background tasks`, …); a segment is only counted when it
//! is exactly that, so a custom statusline mentioning shells doesn't count. Sub-agents
//! are listed in the tree under `main` while they run, and the tree goes when the last
//! one finishes — `/tasks to see subagents` alone means finished ones. Lines above the
//! input box (the transcript, `· 3 shells still running` on a done line) are history,
//! never read. Pure functions over screen lines, so the parsing is tested directly.
//!
//! Grok (see [`parse_grok`]) wakes the same way, and lists its live work in collapsible
//! sections stacked right on top of its input box, gone once the last entry ends:
//!
//! ```text
//!      Worked for 8.9s                                   ← the transcript ends
//!    ▾ Subagents 1
//!    ⋅ Subagent Sleep 30 then report · Wait 30…   grok-4.7 20s
//!    ▾ Tasks 1
//!    ⋅ Run Sleep 35 seconds in background                    20s
//!   ╭──────────────────────────────────────────╮
//!   │ ❯                                        │
//! ```

use serde::{Deserialize, Serialize};

/// The input box's rules are drawn the full width; anything shorter is content.
const MIN_RULE: usize = 10;

/// Background work an agent's screen shows running.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Background {
    /// Sub-agents (the tree under `main`, or a `local agents` pill).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub agents: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shells: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub monitors: u32,
    /// Work Claude doesn't break down: `N background tasks` (a mix of kinds) and
    /// background dynamic workflows.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tasks: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Background {
    /// `2 agents, 1 shell` — what `mmux status` shows after `background:`.
    pub fn describe(&self) -> String {
        [
            (self.agents, "agent"),
            (self.shells, "shell"),
            (self.monitors, "monitor"),
            (self.tasks, "task"),
        ]
        .into_iter()
        .filter(|&(n, _)| n > 0)
        .map(|(n, what)| format!("{n} {what}{}", if n == 1 { "" } else { "s" }))
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// The background work Claude Code's footer shows on `lines` (a screen, top to
/// bottom), or `None` when there is none — or no input box to find a footer under.
pub fn parse(lines: &[String]) -> Option<Background> {
    let rule = lines.iter().rposition(|l| is_rule(l))?;
    let mut bg = Background::default();
    let (mut pill_agents, mut tree_agents) = (0, 0);
    let mut in_tree = false;
    for line in &lines[rule + 1..] {
        let text = line.trim();
        if text.is_empty() {
            in_tree = false;
            continue;
        }
        let label = text.trim_start_matches(|c: char| !c.is_alphanumeric());
        let glyph = label.len() < text.len();
        if in_tree {
            // A row is its status glyph, the agent type, two spaces, its description
            // (or a collapsed `… +3 more`). Anything else ends the tree — so a
            // statusline's `⎇ main` followed by the mode line opens none.
            match more(label) {
                Some(n) if glyph => {
                    tree_agents += n;
                    continue;
                }
                None if glyph && label.contains("  ") => {
                    tree_agents += 1;
                    continue;
                }
                _ => in_tree = false,
            }
        }
        if glyph && label == "main" {
            in_tree = true;
            continue;
        }
        for piece in text.split('·').flat_map(|seg| seg.split(", ")) {
            let Some((n, what)) = piece.trim().split_once(' ') else {
                continue;
            };
            let Ok(n) = n.parse::<u32>() else { continue };
            match what {
                "shell" | "shells" => bg.shells += n,
                "monitor" | "monitors" => bg.monitors += n,
                "local agent" | "local agents" => pill_agents += n,
                "background task"
                | "background tasks"
                | "background dynamic workflow"
                | "background dynamic workflows" => bg.tasks += n,
                _ => {}
            }
        }
    }
    // Sub-agents show in the tree, or (if it's hidden) in the pill — never both.
    bg.agents = tree_agents.max(pill_agents);
    (bg != Background::default()).then_some(bg)
}

/// The background work Grok lists above its input box (see the module docs): each
/// section's header carries its count, so a collapsed one counts as much as an open one.
/// Only the run of section lines touching the box is read — the transcript above it is
/// history.
pub fn parse_grok(lines: &[String]) -> Option<Background> {
    let input = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with('╭'))?;
    let mut bg = Background::default();
    for line in lines[..input].iter().rev().map(|l| l.trim()) {
        let label = line.trim_start_matches(|c: char| !c.is_alphanumeric());
        // Blank lines and the scrollbar's `█` sit between the sections; text that
        // starts flush, without a glyph, is the transcript.
        if label.is_empty() {
            continue;
        }
        if label.len() == line.len() {
            break;
        }
        let header = line.starts_with(['▾', '▸', '▶', '▼', '►']);
        match label.split_once(' ') {
            Some(("Subagents", n)) if header => bg.agents += n.trim().parse().unwrap_or(0),
            Some(("Tasks", n)) if header => bg.tasks += n.trim().parse().unwrap_or(0),
            _ if header => break,
            _ => {}
        }
    }
    (bg != Background::default()).then_some(bg)
}

/// A full-width horizontal rule, as Claude draws above and below its input box.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= MIN_RULE && t.chars().all(|c| c == '─')
}

/// A collapsed tree's `… +3 more` row: how many it stands for.
fn more(label: &str) -> Option<u32> {
    let rest = label.strip_suffix("more")?.trim_end();
    let n = rest.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    n.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "────────────────────────────────────────────────────────────";

    fn screen(footer: &[&str]) -> Vec<String> {
        [
            "⏺ Starting a background sleep for observation",
            "✻ Cogitated for 1m 53s · done 3:17 PM · 3 shells still running",
            "",
            RULE,
            "❯ ",
            RULE,
        ]
        .iter()
        .chain(footer)
        .map(|s| s.to_string())
        .collect()
    }

    /// The real footer of an idle Claude Code 2.1 agent with three background shells
    /// and one sub-agent running.
    #[test]
    fn reads_shells_and_the_agent_tree_of_an_idle_agent() {
        let lines = screen(&[
            "  Opus 5.5 | mmux • main",
            "  5h 16% (4h 42m) • 7d 40% (3d 18h) | ctx 11% (105k/1M)",
            "  ⏵⏵ auto mode on · 3 shells · ← for agents",
            "  ",
            "  ⏺ main",
            "  ◯ general-purpose  Sleep briefly for observation          7s · ↓ 32.6k tokens",
        ]);
        let bg = parse(&lines).unwrap();
        assert_eq!(
            bg,
            Background {
                agents: 1,
                shells: 3,
                ..Background::default()
            }
        );
        assert_eq!(bg.describe(), "1 agent, 3 shells");
    }

    #[test]
    fn finished_sub_agents_and_history_are_not_background_work() {
        // Only the hint for finished sub-agents is left, and the done line above the
        // input box still says shells were running back then.
        let lines = screen(&[
            "  Opus 5.5 | mmux • main",
            "  ⏵⏵ auto mode on · /tasks to see subagents · ← for agents",
        ]);
        assert_eq!(parse(&lines), None);
        assert_eq!(parse(&screen(&["  ? for shortcuts"])), None);
        // No input box on screen, no footer to read.
        let no_box = ["  ⏵⏵ auto mode on · 2 shells".to_string()];
        assert_eq!(parse(&no_box), None);
    }

    #[test]
    fn reads_every_pill_wording() {
        let bg = |pill: &str| parse(&screen(&[pill]));
        assert_eq!(
            bg("  ⏵⏵ accept edits on · 1 shell · /tasks to see subagents"),
            Some(Background {
                shells: 1,
                ..Background::default()
            })
        );
        assert_eq!(
            bg("  2 shells, 1 monitor · ← for agents"),
            Some(Background {
                shells: 2,
                monitors: 1,
                ..Background::default()
            })
        );
        assert_eq!(
            bg("  ⏵⏵ auto mode on · 4 background tasks"),
            Some(Background {
                tasks: 4,
                ..Background::default()
            })
        );
        assert_eq!(bg("  · 1 background dynamic workflow").unwrap().tasks, 1);
        assert_eq!(bg("  · 2 local agents").unwrap().agents, 2);
        // Ambient work Claude isn't waiting on, and statusline text that only
        // mentions shells, don't count.
        for quiet in [
            "  · 1 Artifact comment monitor",
            "  · 2 cloud sessions",
            "  · dreaming",
            "  my-prompt | 2 shells open in tmux",
            "  ctx 11% (105k/1M) · 5 shellsort runs",
        ] {
            assert_eq!(bg(quiet), None, "{quiet}");
        }
    }

    #[test]
    fn a_pill_and_a_tree_for_the_same_agents_count_once() {
        let lines = screen(&[
            "  · 2 local agents",
            "",
            "  ⏺ main",
            "  ◯ Explore  Find the parser   12s",
            "  ◯ Plan  Draft the change     3s",
        ]);
        assert_eq!(parse(&lines).unwrap().agents, 2);
    }

    #[test]
    fn a_collapsed_tree_counts_what_it_hides() {
        let lines = screen(&[
            "  ⏵⏵ auto mode on · ← for agents",
            "",
            "  ⏺ main",
            "  ◯ general-purpose  One   4s",
            "  ◯ general-purpose  Two   3s",
            "  … +3 more",
            "some later line",
        ]);
        assert_eq!(parse(&lines).unwrap().agents, 5);
    }

    /// Grok 1.0 at an idle prompt with a background sub-agent and shell, as `mmux read`
    /// showed it, then after both finished and it woke to report.
    #[test]
    fn reads_grok_s_sections_above_its_input_box() {
        let running: Vec<String> = [
            "     ❯ Spawn ONE background subagent (run in background)      3:37 PM",
            "     Worked for 8.9s",
            "   ▾ Subagents 1",
            "   ⋅ Subagent Sleep 30 then report · Wait 30 seconds…   grok-4.7 20s",
            "   ▾ Tasks 1",
            "   ⋅ Run Sleep 35 seconds in background                          20s",
            "  ╭──────────────────────────────────────────────────────────────────╮",
            "  │ ❯                                                                │",
            "  ╰────────────────────────────── Grok 4.7 (xhigh) · always-approve ─╯",
            "  Shift+Tab:mode  │  Ctrl+.:shortcuts",
        ]
        .map(String::from)
        .to_vec();
        let bg = parse_grok(&running).unwrap();
        assert_eq!(bg.describe(), "1 agent, 1 task");
        // A collapsed section still counts what it hides.
        let mut collapsed = running.clone();
        collapsed.splice(2..4, ["   ▸ Subagents 3".to_string()]);
        assert_eq!(parse_grok(&collapsed).unwrap().agents, 3);

        let done: Vec<String> = [
            "     Worked for 14s",
            "     ◈ Ran sleep 40s then write bg.txt",
            "     bg finished                                                3:36 PM",
            "     Worked for 4.8s                                                  █",
            "  ╭──────────────────────────────────────────────────────────────────╮",
            "  │ ❯                                                                │",
            "  ╰────────────────────────────── Grok 4.7 (xhigh) · always-approve ─╯",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(parse_grok(&done), None);
        // The transcript quoting a header isn't the live list.
        let quoted = ["   ▾ Tasks 2", "     Worked for 3s", "  ╭───╮"].map(String::from);
        assert_eq!(parse_grok(&quoted), None);
        assert_eq!(parse_grok(&running[..6]), None, "no input box, no list");
    }

    #[test]
    fn a_statusline_branch_named_main_is_no_tree() {
        let lines = screen(&["  ⎇ main", "  ⏵⏵ auto mode on · ← for agents"]);
        assert_eq!(parse(&lines), None);
    }
}
