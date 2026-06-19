//! Interactive `mmux init` — a first-run setup wizard.
//!
//! One flow, two entry points: [`run`] walks the user through the three things a
//! workspace needs — which agents to offer, how the project starts, and any
//! sibling `linked-projects` to show alongside it — then writes the YAML.
//!
//! WHERE it writes is decided by a single fact: does a global
//! `~/.mmux/config.yaml` exist yet?
//!
//!   * **No global yet**  → the agents you pick seed a fresh GLOBAL config (your
//!     reusable defaults for every project); the start command + linked projects
//!     go in the project's `./mmux.yaml`.
//!   * **Global exists**  → everything (agents included) goes in `./mmux.yaml`,
//!     layering on top of your global agents.
//!
//! So the wizard never needs to know how it was triggered — `mmux init` and the
//! "no config anywhere, just run `mmux`" first-run path call the same [`run`].
//!
//! The pure half (`split_command` + the `build_*` YAML formatters) is unit-tested
//! and hand-formats commented YAML to match `config::STARTER`'s voice — we don't
//! derive `Serialize`, which would emit `null`s and drop the comments. The
//! interactive half is thin stdin/stdout prompting and never runs without a TTY.

use anyhow::{Context, Result};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

/// A preset agent offered in the wizard. `danger` is the single argument added
/// when the user opts into "danger mode"; without it the agent gets `args: []`.
struct Preset {
    name: &'static str,
    cmd: &'static str,
    danger: &'static str,
    blurb: &'static str,
}

/// Claude and Codex only, by design — the two agents most users start with.
const PRESETS: &[Preset] = &[
    Preset {
        name: "Claude",
        cmd: "claude",
        danger: "--dangerously-skip-permissions",
        blurb: "Anthropic Claude Code",
    },
    Preset {
        name: "Codex",
        cmd: "codex",
        danger: "--dangerously-bypass-approvals-and-sandbox",
        blurb: "OpenAI Codex CLI",
    },
];

/// A chosen agent, ready to render into YAML.
#[derive(Debug, Clone, PartialEq)]
struct Agent {
    name: String,
    cmd: String,
    args: Vec<String>,
}

/// A chosen start command (an mmux "process"), ready to render into YAML.
#[derive(Debug, Clone, PartialEq)]
struct Process {
    name: String,
    cmd: String,
    args: Vec<String>,
    cwd: String,
    autostart: bool,
}

/// Run the wizard for `dir`, writing the global and/or project config per the
/// rule in the module docs. Falls back to the static starter when stdin isn't a
/// terminal (piped/headless), so scripted `mmux init` keeps working.
pub fn run(dir: &Path) -> Result<()> {
    if !io::stdin().is_terminal() {
        return crate::config::write_starter(dir);
    }

    // No global config yet ⇒ agents become the user's global defaults.
    let agents_in_global = crate::config::global_config_path().is_none();
    // Write to whichever project file already exists, else a fresh mmux.yaml.
    let local_path = crate::config::config_path(dir).unwrap_or_else(|| dir.join("mmux.yaml"));

    intro(&local_path, agents_in_global);

    let agents = ask_agents()?;
    let processes = ask_processes()?;
    let linked = ask_linked(dir)?;

    let mut wrote: Vec<String> = Vec::new();

    // Agents to global only when there's no global yet AND the user picked some.
    let wrote_global_agents = agents_in_global && !agents.is_empty();
    if wrote_global_agents {
        if let Some(path) = crate::config::global_config_target() {
            write_new(&path, &build_global_yaml(&agents))?;
            wrote.push(pretty(&path));
        }
    }

    // The project file gets the start command, linked projects, and — unless the
    // agents went global — the agents too.
    let local_agents: &[Agent] = if agents_in_global { &[] } else { &agents };
    let name = dir_name(dir);
    let local = build_local_yaml(&name, local_agents, wrote_global_agents, &processes, &linked);
    if write_local(&local_path, &local)? {
        wrote.push(pretty(&local_path));
    }

    summary(&wrote);
    Ok(())
}

// ── interactive sections ────────────────────────────────────────────────────

fn ask_agents() -> Result<Vec<Agent>> {
    header("Agents");
    println!("{}", dim("Interactive AI coding agents you spawn from the sidebar."));
    let mut out = Vec::new();
    for p in PRESETS {
        if !confirm(&format!("Use {} ({})?", p.name, p.blurb), true)? {
            continue;
        }
        let danger = confirm(
            &format!("  Run {} in danger mode (skip permission/approval prompts)?", p.name),
            false,
        )?;
        let args = if danger { vec![p.danger.to_string()] } else { Vec::new() };
        out.push(Agent { name: p.name.to_string(), cmd: p.cmd.to_string(), args });
    }
    Ok(out)
}

fn ask_processes() -> Result<Vec<Process>> {
    header("How do you start this project?");
    println!(
        "{}",
        dim("Commands you start/stop and watch in mmux (dev server, watcher, …). Blank to skip.")
    );
    let mut out = Vec::new();
    loop {
        let label = if out.is_empty() { "Start command (e.g. npm run dev)" } else { "Another start command" };
        let Some((cmd, args)) = split_command(&ask(label, None)?) else {
            break; // blank line → done adding processes
        };
        let default_name = if out.is_empty() { "Dev server".to_string() } else { capitalize(&cmd) };
        let name = ask("  Name for this step", Some(&default_name))?;
        let cwd = ask("  Working directory (relative to the project)", Some("."))?;
        let autostart = confirm("  Start it automatically when mmux opens?", false)?;
        out.push(Process { name, cmd, args, cwd, autostart });
        if !confirm("Add another start command?", false)? {
            break;
        }
    }
    Ok(out)
}

fn ask_linked(dir: &Path) -> Result<Vec<String>> {
    header("Other projects");
    println!(
        "{}",
        dim("Show sibling directories (e.g. extra clones) alongside this one — each its own sidebar group.")
    );
    let mut out: Vec<String> = Vec::new();
    if !confirm("Link another project directory now?", false)? {
        return Ok(out);
    }
    loop {
        let path = ask("  Path (relative to here, e.g. ../myproject2)", None)?;
        if path.is_empty() {
            break;
        }
        if !dir.join(&path).is_dir() {
            println!("{}", dim(&format!("  note: {path} isn't a directory yet — linking it anyway.")));
        }
        if !out.contains(&path) {
            out.push(path);
        }
        if !confirm("  Link another?", false)? {
            break;
        }
    }
    Ok(out)
}

// ── pure builders (unit-tested) ──────────────────────────────────────────────

/// Split a typed command line into `(cmd, args)` on whitespace. `None` for a
/// blank line. Quoting isn't handled — the generated file is the place to add it.
fn split_command(input: &str) -> Option<(String, Vec<String>)> {
    let mut it = input.split_whitespace().map(str::to_string);
    let cmd = it.next()?;
    Some((cmd, it.collect()))
}

/// The global `~/.mmux/config.yaml`: just the agents, plus a commented panel hint.
fn build_global_yaml(agents: &[Agent]) -> String {
    let mut s = String::new();
    s.push_str("# mmux global config (~/.mmux/config.yaml).\n");
    s.push_str("# Agents here are available in EVERY project. A project's mmux.yaml can\n");
    s.push_str("# override or add to them by name.\n\n");
    s.push_str("agents:\n");
    s.push_str(&agent_items(agents));
    s.push_str("\n# An always-on right-hand panel for every project (optional):\n");
    s.push_str("# right_panel:\n");
    s.push_str("#   cmd: lazygit\n");
    s.push_str("#   title: lazygit\n");
    s.push_str("#   width: 44\n");
    s
}

/// The project `./mmux.yaml`. Sections with content are written live; empty ones
/// are left as commented examples so the file documents how to grow. When the
/// agents went to the global config, `agents_elsewhere` swaps the agents block
/// for a pointer to it.
fn build_local_yaml(
    name: &str,
    agents: &[Agent],
    agents_elsewhere: bool,
    procs: &[Process],
    linked: &[String],
) -> String {
    let mut s = String::new();
    s.push_str("# mmux workspace config.\n");
    s.push_str("# Run `mmux` in this directory to open (or reattach to) the session.\n\n");
    s.push_str(&format!("name: {}\n\n", yaml_scalar(name)));

    // Agents
    if agents_elsewhere {
        s.push_str("# Your agents live in the global config (~/.mmux/config.yaml). Add a\n");
        s.push_str("# project-only agent here to extend or override them by name:\n");
        s.push_str("# agents:\n");
        s.push_str("#   - name: Claude\n#     cmd: claude\n#     args: [\"--dangerously-skip-permissions\"]\n\n");
    } else if !agents.is_empty() {
        s.push_str("# Agents: interactive programs you spawn on demand from the sidebar.\n");
        s.push_str("agents:\n");
        s.push_str(&agent_items(agents));
        s.push('\n');
    } else {
        s.push_str("# Agents: interactive programs you spawn on demand from the sidebar.\n");
        s.push_str("# agents:\n");
        s.push_str("#   - name: Claude\n#     cmd: claude\n#     args: [\"--dangerously-skip-permissions\"]\n\n");
    }

    // Processes
    s.push_str("# Processes: commands you start/stop and watch. cwd is relative to this file.\n");
    if procs.is_empty() {
        s.push_str("# processes:\n");
        s.push_str("#   - name: Dev server\n#     cmd: npm\n#     args: [\"run\", \"dev\"]\n#     autostart: false\n\n");
    } else {
        s.push_str("processes:\n");
        s.push_str(&process_items(procs));
        s.push('\n');
    }

    // Linked projects
    s.push_str("# Linked projects: sibling dirs (e.g. extra clones) shown alongside this one,\n");
    s.push_str("# each its own sidebar group. One level deep, de-duplicated by path.\n");
    if linked.is_empty() {
        s.push_str("# linked-projects:\n");
        s.push_str("#   - ../myproject2\n");
    } else {
        s.push_str("linked-projects:\n");
        for l in linked {
            s.push_str(&format!("  - {}\n", yaml_scalar(l)));
        }
    }
    s
}

/// The `- name:/cmd:/args:` block items for an agents list (indented two spaces).
fn agent_items(agents: &[Agent]) -> String {
    let mut s = String::new();
    for a in agents {
        s.push_str(&format!("  - name: {}\n", yaml_scalar(&a.name)));
        s.push_str(&format!("    cmd: {}\n", yaml_scalar(&a.cmd)));
        s.push_str(&format!("    args: {}\n", yaml_args(&a.args)));
    }
    s
}

/// The block items for a processes list.
fn process_items(procs: &[Process]) -> String {
    let mut s = String::new();
    for p in procs {
        s.push_str(&format!("  - name: {}\n", yaml_scalar(&p.name)));
        s.push_str(&format!("    cmd: {}\n", yaml_scalar(&p.cmd)));
        s.push_str(&format!("    args: {}\n", yaml_args(&p.args)));
        s.push_str(&format!("    cwd: {}\n", yaml_scalar(&p.cwd)));
        s.push_str(&format!("    autostart: {}\n", p.autostart));
    }
    s
}

/// Render an argument list as a YAML flow sequence of double-quoted scalars.
/// JSON-style quoting (via `{:?}`) is valid YAML, so this stays correct for args
/// with spaces or quotes.
fn yaml_args(args: &[String]) -> String {
    let inner: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
    format!("[{}]", inner.join(", "))
}

/// A scalar value, quoted only when YAML would otherwise mis-parse it. Keeps the
/// common case (`name: Dev server`, `cwd: .`) clean while staying safe for input
/// containing `:`, `#`, quotes, brackets, or an indicator first character.
fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s == s.trim()
        && !s.contains(['#', ':', '"', '\'', '[', ']', '{', '}', '\n'])
        && !s.starts_with(['-', '?', '&', '*', '!', '|', '>', '%', '@', '`', ',']);
    if plain {
        s.to_string()
    } else {
        format!("{s:?}")
    }
}

// ── file writing ─────────────────────────────────────────────────────────────

/// Write `contents` to `path`, creating parent directories. Used for the global
/// file, which by the time we get here is known not to exist.
fn write_new(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Write the project file, asking before clobbering an existing one. Returns
/// whether it actually wrote.
fn write_local(path: &Path, contents: &str) -> Result<bool> {
    if path.exists() && !confirm(&format!("{} already exists. Overwrite it?", pretty(path)), false)? {
        println!("{}", dim(&format!("Left {} unchanged.", pretty(path))));
        return Ok(false);
    }
    write_new(path, contents)?;
    Ok(true)
}

// ── prompts ──────────────────────────────────────────────────────────────────

/// Read one trimmed line, or `None` at EOF (Ctrl-D).
fn read_line() -> Result<Option<String>> {
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

/// Ask a free-text question. Empty input (or EOF) yields `default` (or "").
fn ask(question: &str, default: Option<&str>) -> Result<String> {
    let hint = match default {
        Some(d) if !d.is_empty() => format!(" {}", dim(&format!("[{d}]"))),
        _ => String::new(),
    };
    print!("{question}{hint}: ");
    io::stdout().flush()?;
    Ok(match read_line()? {
        Some(s) if !s.is_empty() => s,
        _ => default.unwrap_or("").to_string(),
    })
}

/// Ask a yes/no question with a default. Empty input (or EOF) takes the default.
fn confirm(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{question} {} ", dim(hint));
        io::stdout().flush()?;
        match read_line()? {
            None => return Ok(default_yes),
            Some(s) => match s.to_ascii_lowercase().as_str() {
                "" => return Ok(default_yes),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => println!("{}", dim("Please answer y or n.")),
            },
        }
    }
}

// ── presentation ─────────────────────────────────────────────────────────────

fn intro(local_path: &Path, agents_in_global: bool) {
    println!("\n{}", bold("mmux setup"));
    if agents_in_global {
        println!("{}", dim("First run — your agents go in the global config (~/.mmux/config.yaml,"));
        println!("{}", dim("used in every project); this project's setup goes in its mmux.yaml."));
    } else {
        println!("{}", dim(&format!("Configuring {}.", pretty(local_path))));
    }
    println!("{}", dim("Press Enter to accept each [default]."));
}

fn summary(wrote: &[String]) {
    println!();
    if wrote.is_empty() {
        println!("{}", dim("Nothing written."));
        return;
    }
    println!("{}", bold("Done.") );
    for w in wrote {
        println!("  • {w}");
    }
    println!("{}", dim("Edit those to fine-tune, then run `mmux`."));
}

/// Display a path with `$HOME` collapsed to `~`.
fn pretty(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        if let Some(rest) = s.strip_prefix(home.to_string_lossy().as_ref()) {
            return format!("~{rest}");
        }
    }
    s.into_owned()
}

/// The project's default name: its canonical directory basename.
fn dir_name(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .ok()
        .as_deref()
        .or(Some(dir))
        .and_then(Path::file_name)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// ANSI styling, but only when stdout is a terminal (so redirected output stays
/// clean). stdin is already known to be a TTY by the time these run.
fn color() -> bool {
    io::stdout().is_terminal()
}

fn bold(text: &str) -> String {
    if color() { format!("\x1b[1m{text}\x1b[0m") } else { text.to_string() }
}

fn dim(text: &str) -> String {
    if color() { format!("\x1b[2m{text}\x1b[0m") } else { text.to_string() }
}

fn header(text: &str) {
    println!("\n{}", bold(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn agent(name: &str, cmd: &str, args: &[&str]) -> Agent {
        Agent { name: name.into(), cmd: cmd.into(), args: args.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn split_command_handles_blanks_and_args() {
        assert_eq!(
            split_command("npm run dev"),
            Some(("npm".into(), vec!["run".into(), "dev".into()]))
        );
        assert_eq!(
            split_command("  cargo  watch -x run "),
            Some(("cargo".into(), vec!["watch".into(), "-x".into(), "run".into()]))
        );
        assert_eq!(split_command("solo"), Some(("solo".into(), vec![])));
        assert_eq!(split_command("   "), None);
        assert_eq!(split_command(""), None);
    }

    #[test]
    fn yaml_scalar_quotes_only_when_needed() {
        assert_eq!(yaml_scalar("Dev server"), "Dev server");
        assert_eq!(yaml_scalar("."), ".");
        assert_eq!(yaml_scalar("../proj2"), "../proj2");
        assert_eq!(yaml_scalar("build:dev"), "\"build:dev\"");
        assert_eq!(yaml_scalar("- weird"), "\"- weird\"");
    }

    #[test]
    fn global_yaml_round_trips() {
        let agents = vec![
            agent("Claude", "claude", &["--dangerously-skip-permissions"]),
            agent("Codex", "codex", &[]),
        ];
        let cfg: Config = serde_yaml::from_str(&build_global_yaml(&agents)).expect("global parses");
        assert_eq!(cfg.agents.len(), 2);
        assert_eq!(cfg.agents[0].name, "Claude");
        assert_eq!(cfg.agents[0].args, vec!["--dangerously-skip-permissions"]);
        assert!(cfg.agents[1].args.is_empty());
    }

    #[test]
    fn local_yaml_with_global_agents_round_trips() {
        let procs = vec![Process {
            name: "Dev server".into(),
            cmd: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            cwd: ".".into(),
            autostart: true,
        }];
        let yaml = build_local_yaml("myproj", &[], true, &procs, &["../other".to_string()]);
        let cfg: Config = serde_yaml::from_str(&yaml).expect("local parses");
        assert_eq!(cfg.name.as_deref(), Some("myproj"));
        assert!(cfg.agents.is_empty()); // they're in the global file
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].name, "Dev server");
        assert!(cfg.processes[0].autostart);
        assert_eq!(cfg.linked_projects, vec!["../other".to_string()]);
    }

    #[test]
    fn local_yaml_with_local_agents_round_trips() {
        let agents = vec![agent("Claude", "claude", &["--dangerously-skip-permissions"])];
        let cfg: Config =
            serde_yaml::from_str(&build_local_yaml("p", &agents, false, &[], &[])).expect("parses");
        assert_eq!(cfg.agents.len(), 1);
        assert!(cfg.processes.is_empty());
        assert!(cfg.linked_projects.is_empty());
    }

    #[test]
    fn empty_local_yaml_is_still_valid() {
        // Nothing chosen: every section is a comment, but it must still parse.
        let cfg: Config = serde_yaml::from_str(&build_local_yaml("p", &[], true, &[], &[]))
            .expect("commented-only file parses");
        assert!(cfg.agents.is_empty());
        assert!(cfg.processes.is_empty());
    }
}
