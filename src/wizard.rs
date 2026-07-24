//! Interactive `mmux init` — setup for a project or a multi-project workspace.
//!
//! [`run`] starts with the project/workspace choice unless the caller supplied one
//! explicitly. Project setup walks through agents and start commands; workspace
//! setup shares its folder picker with the manifest-only TUI popup.
//!
//! WHERE it writes is decided by a single fact: does a global
//! `~/.mmux/config.yaml` exist yet?
//!
//!   * **No global yet**  → the agents you pick seed a fresh GLOBAL config (your
//!     reusable defaults for every project); the start command goes in the
//!     project's `./mmux.yaml`.
//!   * **Global exists**  → everything (agents included) goes in `./mmux.yaml`,
//!     layering on top of your global agents.
//!
//! Bare `mmux` with no local or global config calls [`run_first`], so onboarding
//! uses this exact same choice instead of silently assuming a project.
//!
//! [`run_agents`] remains the focused command-line twin of the in-TUI agent manager.
//!
//! The agents step is an inline, arrow-navigable checkbox picker over the built-in
//! [`crate::agentmgr::AgentManager`] (shared with the in-TUI popup and `mmux agents`, so
//! all three stay in step) — no full-screen takeover. The pure half (`split_command` +
//! the `build_*` YAML formatters) is unit-tested
//! and hand-formats commented YAML to match `config::STARTER`'s voice — we don't
//! derive `Serialize`, which would emit `null`s and drop the comments. The
//! interactive half is thin stdin/stdout prompting and never runs without a TTY.

use crate::agentmgr::{AgentManager, Mode};
use crate::config::{self, yaml_args, yaml_scalar};
use crate::workspacemgr::WorkspaceManager;
use anyhow::{Context, Result};
use ratatui::crossterm::{
    cursor::MoveToPreviousLine,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitKind {
    Project,
    Workspace,
}

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
    /// Optional teardown shell line run when the process stops or mmux quits; blank = none.
    stop: String,
    autostart: bool,
}

/// Run `mmux init`, optionally skipping the first choice. A non-interactive
/// invocation keeps the historical project-starter behaviour; workspace setup
/// needs a terminal for its folder picker.
pub fn run(dir: &Path, requested: Option<InitKind>) -> Result<()> {
    run_inner(dir, requested, false)
}

/// First-run entry point used by bare `mmux` when neither config layer exists.
pub fn run_first(dir: &Path) -> Result<()> {
    run_inner(dir, None, true)
}

fn run_inner(dir: &Path, requested: Option<InitKind>, first_run: bool) -> Result<()> {
    if !io::stdin().is_terminal() {
        return match requested {
            Some(InitKind::Workspace) => {
                anyhow::bail!("`mmux init workspace` needs an interactive terminal")
            }
            _ => crate::config::write_starter(dir),
        };
    }

    let existing = configured_kind(dir)?;
    setup_intro(first_run);
    let kind = match requested {
        Some(kind) => Some(kind),
        None => select_kind(existing)?,
    };
    let Some(kind) = kind else {
        println!("{}", dim("Nothing written."));
        return Ok(());
    };

    if existing == Some(InitKind::Workspace)
        && kind == InitKind::Project
        && !confirm(
            "This directory is a workspace. Convert it to a single project?\n\
             Its member list will be removed; other settings will be kept.",
            false,
        )?
    {
        println!("{}", dim("Nothing written."));
        return Ok(());
    }

    match kind {
        InitKind::Project => run_project(dir, existing == Some(InitKind::Workspace)),
        InitKind::Workspace => run_workspace(dir),
    }
}

fn run_project(dir: &Path, converting: bool) -> Result<()> {
    // No global config yet ⇒ agents become the user's global defaults.
    let agents_in_global = crate::config::global_config_path().is_none();
    // Write to whichever project file already exists, else a fresh mmux.yaml.
    let local_path = crate::config::config_path(dir).unwrap_or_else(|| dir.join("mmux.yaml"));

    project_intro(&local_path, agents_in_global);

    let agents = ask_agents()?;
    let processes = ask_processes()?;

    let mut wrote: Vec<String> = Vec::new();

    // Agents to global only when there's no global yet AND the user picked some.
    let wrote_global_agents = agents_in_global && !agents.is_empty();
    if wrote_global_agents {
        if let Some(path) = crate::config::global_config_target() {
            write_new(&path, &build_global_yaml(&agents))?;
            wrote.push(pretty(&path));
        }
    }

    // The project file gets the start command and — unless the agents went
    // global — the agents too.
    let local_agents: &[Agent] = if agents_in_global { &[] } else { &agents };
    let name = dir_name(dir);
    let local = build_local_yaml(&name, local_agents, wrote_global_agents, &processes);
    if write_project(
        dir,
        &local_path,
        &local,
        &name,
        local_agents,
        &processes,
        converting,
    )? {
        wrote.push(pretty(&local_path));
    }

    summary(&wrote);
    if !wrote.is_empty() {
        println!("{}", dim("Run `mmux` here to open the project."));
    }
    Ok(())
}

fn configured_kind(dir: &Path) -> Result<Option<InitKind>> {
    if crate::config::config_path(dir).is_none() && crate::config::local_config_path(dir).is_none()
    {
        return Ok(None);
    }
    let cfg = crate::config::Config::load(dir)?;
    Ok(Some(if cfg.workspace.is_some() {
        InitKind::Workspace
    } else {
        InitKind::Project
    }))
}

fn setup_intro(first_run: bool) {
    println!("\n{}", bold("mmux setup"));
    if first_run {
        println!("{}", dim("No mmux configuration was found."));
    }
}

fn select_kind(existing: Option<InitKind>) -> Result<Option<InitKind>> {
    println!("\n{}", bold("What do you want to set up here?"));
    println!("{}", dim("↑↓ choose · ⏎ continue · esc cancel"));
    let mut selected = existing.unwrap_or(InitKind::Project);
    let mut out = io::stdout();
    enable_raw_mode()?;
    let result = loop {
        draw_kind_rows(&mut out, selected, existing)?;
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => match (k.code, k.modifiers) {
                (KeyCode::Up, _)
                | (KeyCode::Down, _)
                | (KeyCode::Char('j'), _)
                | (KeyCode::Char('k'), _) => {
                    selected = match selected {
                        InitKind::Project => InitKind::Workspace,
                        InitKind::Workspace => InitKind::Project,
                    };
                }
                (KeyCode::Enter, _) => break Some(selected),
                (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => break None,
                _ => {}
            },
            _ => {}
        }
        execute!(out, MoveToPreviousLine(2))?;
    };
    let _ = disable_raw_mode();
    println!();
    Ok(result)
}

fn draw_kind_rows(
    out: &mut io::Stdout,
    selected: InitKind,
    existing: Option<InitKind>,
) -> Result<()> {
    execute!(out, Clear(ClearType::FromCursorDown))?;
    for (kind, name, description) in [
        (
            InitKind::Project,
            "Project",
            "One codebase with its own processes and Git.",
        ),
        (
            InitKind::Workspace,
            "Workspace",
            "Several project folders in one mmux session.",
        ),
    ] {
        let marker = if selected == kind { "› " } else { "  " };
        let name = if selected == kind {
            paint(CYAN_BOLD, &format!("{name:<10}"))
        } else {
            format!("{name:<10}")
        };
        let current = if existing == Some(kind) {
            dim("  currently configured")
        } else {
            String::new()
        };
        write!(out, "{marker}{name} {}{current}\r\n", dim(description))?;
    }
    out.flush()?;
    Ok(())
}

/// `mmux agents` — a focused, agents-only version of the wizard for the terminal: show
/// the built-in harnesses with their current on/off state, let the user re-pick them
/// (+ cycle each one's launch mode), and write the result to the **global** config. It's the
/// command-line twin of the in-TUI agent manager (the sidebar's `a` popup); both edit
/// the same file and preserve any non-preset agents you added by hand.
pub fn run_agents() -> Result<()> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("`mmux agents` needs an interactive terminal");
    }
    let Some(path) = config::global_config_target() else {
        anyhow::bail!("can't locate ~/.mmux (is HOME set?)");
    };
    println!("\n{}", bold("Manage agents"));
    println!(
        "{}",
        dim(&format!(
            "Built-in AI coding harnesses, saved to {}.",
            pretty(&path)
        ))
    );
    // Seeded from the current global config (its agents pre-checked); custom agents are
    // preserved on save. Cancel leaves the file untouched.
    let mut m = AgentManager::new();
    if !select_agents(&mut m)? {
        println!("{}", dim("No changes."));
        return Ok(());
    }
    let drafts = m.drafts();
    config::write_agents(&path, &drafts).with_context(|| format!("writing {}", pretty(&path)))?;
    println!("\n{}", bold("Done."));
    println!(
        "  • {} {}",
        pretty(&path),
        dim(&format!("({} agent(s) configured)", drafts.len()))
    );
    println!(
        "{}",
        dim("Open mmux — or press R inside it — to see the change.")
    );
    Ok(())
}

/// Create or edit a directory-level workspace manifest. Immediate child directories
/// are discovered automatically; existing outside paths remain available as rows.
fn run_workspace(dir: &Path) -> Result<()> {
    let agents_in_global = crate::config::global_config_path().is_none();
    let agents = if agents_in_global {
        println!();
        println!(
            "{}",
            dim("First run — choose agents to make available in every workspace project.")
        );
        ask_agents()?
    } else {
        Vec::new()
    };
    let mut m = WorkspaceManager::new(dir)?;
    let path = config::workspace_config_path(&m.root);

    println!("\n{}", bold("Workspace setup"));
    println!(
        "{}",
        dim("A workspace opens several project folders together in one persistent session.")
    );
    println!(
        "{}",
        dim(&format!(
            "Select projects below {} · saved to {}.",
            pretty(&m.root),
            pretty(&path)
        ))
    );
    m.name = ask("Workspace name", Some(&m.name))?;
    if !select_workspace(&mut m)? {
        println!("{}", dim("Nothing written."));
        return Ok(());
    }
    let folders = m.folders();
    let mut wrote = Vec::new();
    if agents_in_global && !agents.is_empty() {
        if let Some(global_path) = crate::config::global_config_target() {
            write_new(&global_path, &build_global_yaml(&agents))?;
            wrote.push(pretty(&global_path));
        }
    }
    crate::restore::bind_project_dirs(&m.root, &m.original_projects);
    config::write_workspace(&path, &m.name, &folders)
        .with_context(|| format!("writing {}", pretty(&path)))?;
    wrote.push(pretty(&path));

    summary(&wrote);
    println!(
        "{}",
        dim(&format!("{} project(s) selected.", folders.len()))
    );
    println!("{}", dim("Run `mmux` here to open the workspace."));
    println!(
        "{}",
        dim("Run `mmux init` inside a member project to configure its processes.")
    );
    Ok(())
}

// ── interactive agent picker ─────────────────────────────────────────────────
//
// An inline checkbox list, NOT a full-screen takeover: we raw-mode the terminal, print
// the rows in place, and redraw just those N lines on each keypress (cursor up N →
// clear-to-end → reprint). The surrounding prompt output (the "Agents" header, earlier
// answers) stays put in the scrollback. Shared by `mmux init` and `mmux agents`.

/// Drive an [`AgentManager`] as an interactive checkbox picker in the terminal. ↑/↓ or
/// `j`/`k` move, `space` toggles an agent, `m` cycles its launch mode (normal → auto →
/// danger), `a` selects all/none, ⏎ confirms, Esc/`q`/Ctrl-C cancels. Returns whether the
/// user confirmed (⏎) rather than cancelled. Raw mode is always restored before returning.
fn select_agents(m: &mut AgentManager) -> Result<bool> {
    if m.rows.is_empty() {
        return Ok(true);
    }
    // These two lines are static context above the redrawn rows.
    println!(
        "{}",
        dim("↑↓ move · space toggle · m mode · a all · ⏎ done · esc skip")
    );
    println!(
        "{}",
        dim("A green ✓ marks the harnesses found on your PATH.")
    );
    let height = m.rows.len() as u16;
    let mut out = io::stdout();
    enable_raw_mode()?;
    let result = agent_select_loop(&mut out, m, height);
    let _ = disable_raw_mode();
    println!(); // land the cursor below the (final, still-visible) rows
    result
}

/// The picker's key loop: draw the rows, read one key, apply it, repeat until the user
/// confirms or cancels. The final rows stay on screen showing the chosen state.
fn agent_select_loop(out: &mut io::Stdout, m: &mut AgentManager, height: u16) -> Result<bool> {
    let mut first = true;
    loop {
        draw_agent_rows(out, m, height, first)?;
        first = false;
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => match (k.code, k.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => m.move_cursor(-1),
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => m.move_cursor(1),
                (KeyCode::Char(' '), _) => m.toggle_enabled(),
                (KeyCode::Char('m'), _) => m.cycle_mode(),
                (KeyCode::Char('a'), _) => m.toggle_all(),
                (KeyCode::Enter, _) => return Ok(true),
                (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => return Ok(false),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(false),
                _ => {}
            },
            _ => {}
        }
    }
}

/// Redraw the `height` checkbox rows in place: on every pass but the first, step the
/// cursor back up over them and clear to the end, then reprint. Each row is
/// `‹cursor› [x] ✓ Name   auto/danger  blurb`, coloured by state.
fn draw_agent_rows(out: &mut io::Stdout, m: &AgentManager, height: u16, first: bool) -> Result<()> {
    if !first {
        execute!(out, MoveToPreviousLine(height))?;
    }
    execute!(out, Clear(ClearType::FromCursorDown))?;
    for (i, r) in m.rows.iter().enumerate() {
        let selected = i == m.cursor;
        let marker = if selected { "› " } else { "  " };
        let checkbox = if r.enabled {
            paint(GREEN, "[x]")
        } else {
            dim("[ ]")
        };
        let install = if r.installed {
            paint(GREEN, "✓")
        } else {
            " ".to_string()
        };
        let name = format!("{:<10}", r.name);
        let name = if selected {
            paint(CYAN_BOLD, &name)
        } else if r.enabled {
            name
        } else {
            dim(&name)
        };
        // Fixed-width (6, matching "danger") mode cell so the blurbs stay aligned.
        let mode = match r.mode() {
            Mode::Normal => "      ".to_string(),
            Mode::Auto => paint(CYAN, &format!("{:<6}", "auto")),
            Mode::Danger => paint(YELLOW, &format!("{:<6}", "danger")),
        };
        let blurb = dim(&format!("  {}", r.blurb));
        // Raw mode: rows need an explicit carriage-return + line-feed.
        write!(out, "{marker}{checkbox} {install} {name} {mode}{blurb}\r\n")?;
    }
    out.flush()?;
    Ok(())
}

// ── interactive workspace picker ─────────────────────────────────────────────

fn select_workspace(m: &mut WorkspaceManager) -> Result<bool> {
    println!(
        "{}",
        dim("↑↓ move · space toggle · J/K reorder · a all/none · ⏎ save · esc cancel")
    );
    println!(
        "{}",
        dim(&format!(
            "Choose up to {} folders; their row order is saved in the manifest.",
            config::MAX_PROJECTS
        ))
    );
    // A bounded window keeps a parent containing many directories usable without
    // taking over the whole terminal. The extra line is the live count/error footer.
    let term_h = ratatui::crossterm::terminal::size()
        .map(|(_, h)| h)
        .unwrap_or(24);
    let visible = m
        .rows
        .len()
        .min(term_h.saturating_sub(8).clamp(5, 15) as usize)
        .max(1);
    let height = visible as u16 + 1;
    let mut out = io::stdout();
    enable_raw_mode()?;
    let result = workspace_select_loop(&mut out, m, visible, height);
    let _ = disable_raw_mode();
    println!();
    result
}

fn workspace_select_loop(
    out: &mut io::Stdout,
    m: &mut WorkspaceManager,
    visible: usize,
    height: u16,
) -> Result<bool> {
    let mut first = true;
    loop {
        draw_workspace_rows(out, m, visible, height, first)?;
        first = false;
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => match (k.code, k.modifiers) {
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => m.move_cursor(-1),
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => m.move_cursor(1),
                (KeyCode::Char('K'), _) => m.reorder(-1),
                (KeyCode::Char('J'), _) => m.reorder(1),
                (KeyCode::Char(' '), _) => m.toggle_enabled(),
                (KeyCode::Char('a'), _) => m.toggle_all(),
                (KeyCode::Enter, _) if m.validate() => return Ok(true),
                (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => return Ok(false),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(false),
                _ => {}
            },
            _ => {}
        }
    }
}

fn draw_workspace_rows(
    out: &mut io::Stdout,
    m: &WorkspaceManager,
    visible: usize,
    height: u16,
    first: bool,
) -> Result<()> {
    if !first {
        execute!(out, MoveToPreviousLine(height))?;
    }
    execute!(out, Clear(ClearType::FromCursorDown))?;
    let start = if m.cursor >= visible {
        m.cursor + 1 - visible
    } else {
        0
    };
    for i in start..start + visible {
        if let Some(r) = m.rows.get(i) {
            let selected = i == m.cursor;
            let marker = if selected { "› " } else { "  " };
            let checkbox = if r.enabled {
                paint(GREEN, "[x]")
            } else {
                dim("[ ]")
            };
            let label = if r.path == "." {
                ".  (this directory)".to_string()
            } else {
                r.path.clone()
            };
            let label = if selected {
                paint(CYAN_BOLD, &label)
            } else if r.enabled {
                label
            } else {
                dim(&label)
            };
            let mut badges = Vec::new();
            if r.configured {
                badges.push("mmux");
            }
            if r.git {
                badges.push("git");
            }
            if !r.exists {
                badges.push("missing");
            }
            let badge = if badges.is_empty() {
                String::new()
            } else {
                dim(&format!("  {}", badges.join(" · ")))
            };
            write!(out, "{marker}{checkbox} {label}{badge}\r\n")?;
        } else {
            write!(out, "\r\n")?;
        }
    }
    let status = m
        .error
        .clone()
        .unwrap_or_else(|| format!("{} / {} selected", m.selected_count(), config::MAX_PROJECTS));
    let status = if m.error.is_some() {
        paint(YELLOW, &status)
    } else {
        dim(&status)
    };
    write!(out, "{status}\r\n")?;
    out.flush()?;
    Ok(())
}

// ── interactive sections ────────────────────────────────────────────────────

fn ask_agents() -> Result<Vec<Agent>> {
    header("Agents");
    println!(
        "{}",
        dim("Interactive AI coding agents you spawn from the sidebar.")
    );
    // A fresh picker with installed harnesses pre-checked (see AgentManager::fresh).
    let mut m = AgentManager::fresh();
    if !select_agents(&mut m)? {
        return Ok(Vec::new()); // esc → skip agents for now (add them later with `a`)
    }
    Ok(m.drafts()
        .into_iter()
        .map(|d| Agent {
            name: d.name,
            cmd: d.cmd,
            args: d.args,
        })
        .collect())
}

fn ask_processes() -> Result<Vec<Process>> {
    header("How do you start this project?");
    println!(
        "{}",
        dim("Commands you start/stop and watch in mmux (dev server, watcher, …). Blank to skip.")
    );
    let mut out = Vec::new();
    loop {
        let label = if out.is_empty() {
            "Start command (e.g. npm run dev)"
        } else {
            "Another start command"
        };
        let Some((cmd, args)) = split_command(&ask(label, None)?) else {
            break; // blank line → done adding processes
        };
        let default_name = if out.is_empty() {
            "Dev server".to_string()
        } else {
            capitalize(&cmd)
        };
        let name = ask("  Name for this step", Some(&default_name))?;
        let cwd = ask("  Working directory (relative to the project)", Some("."))?;
        let stop = ask("  Stop command — runs in that dir on stop/quit (e.g. docker compose down), blank for none", None)?;
        let autostart = confirm("  Start it automatically when mmux opens?", false)?;
        out.push(Process {
            name,
            cmd,
            args,
            cwd,
            stop,
            autostart,
        });
        if !confirm("Add another start command?", false)? {
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
    s.push_str(config::GLOBAL_HEADER);
    s.push_str("agents:\n");
    s.push_str(&agent_items(agents));
    s.push_str(config::GLOBAL_GIT_PANEL_HINT);
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
) -> String {
    let mut s = String::new();
    s.push_str(config::PROJECT_HEADER);
    s.push('\n');
    s.push_str(&format!("name: {}\n\n", yaml_scalar(name)));

    // Agents
    if agents_elsewhere {
        s.push_str("# Your agents live in the global config (~/.mmux/config.yaml). Add a\n");
        s.push_str("# project-only agent here to extend or override them by name:\n");
        s.push_str(config::PROJECT_AGENTS_EXAMPLE);
    } else if !agents.is_empty() {
        s.push_str(config::PROJECT_AGENTS_COMMENT);
        s.push_str("agents:\n");
        s.push_str(&agent_items(agents));
        s.push('\n');
    } else {
        s.push_str(config::PROJECT_AGENTS_COMMENT);
        s.push_str(config::PROJECT_AGENTS_EXAMPLE);
    }

    // Processes
    s.push_str(config::PROJECT_PROCESSES_COMMENT);
    if procs.is_empty() {
        s.push_str(config::PROJECT_PROCESSES_EXAMPLE);
    } else {
        s.push_str("processes:\n");
        s.push_str(&process_items(procs));
        s.push('\n');
    }

    // Project setup leaves workspace creation to the other `mmux init` branch; this
    // commented example documents the shape without emitting a live block.
    s.push_str(config::PROJECT_WORKSPACE_COMMENT);
    s.push_str(config::PROJECT_WORKSPACE_EXAMPLE);
    s
}

/// The `- name:/cmd:/args:` block items for an agents list (indented two spaces).
/// Rendered by the runtime config writer's [`config::render_agent_item`] so the wizard
/// and the in-TUI agent manager emit byte-identical items from one place.
fn agent_items(agents: &[Agent]) -> String {
    let mut s = String::new();
    for a in agents {
        let draft = config::AgentDraft {
            name: a.name.clone(),
            cmd: a.cmd.clone(),
            args: a.args.clone(),
        };
        s.push_str(&config::render_agent_item(&draft, 2));
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
        if !p.stop.trim().is_empty() {
            s.push_str(&format!("    stop: {}\n", yaml_scalar(&p.stop)));
        }
        s.push_str(&format!("    autostart: {}\n", p.autostart));
    }
    s
}

// ── file writing ─────────────────────────────────────────────────────────────

/// Write `contents` to `path`, creating parent directories. Used for the global
/// file, which by the time we get here is known not to exist.
fn write_new(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Write the project setup. Existing files are updated section-by-section so
/// notification, panel, and other unrelated settings survive a rerun.
fn write_project(
    dir: &Path,
    path: &Path,
    fresh_contents: &str,
    name: &str,
    agents: &[Agent],
    processes: &[Process],
    converting: bool,
) -> Result<bool> {
    if path.exists() && !converting && !confirm(&format!("Update {}?", pretty(path)), false)? {
        println!("{}", dim(&format!("Left {} unchanged.", pretty(path))));
        return Ok(false);
    }
    if path.exists() {
        let agents: Vec<config::AgentDraft> = agents
            .iter()
            .map(|a| config::AgentDraft {
                name: a.name.clone(),
                cmd: a.cmd.clone(),
                args: a.args.clone(),
            })
            .collect();
        let processes: Vec<config::ProcessDraft> = processes
            .iter()
            .map(|p| config::ProcessDraft {
                name: p.name.clone(),
                cmd: p.cmd.clone(),
                args: p.args.clone(),
                cwd: (p.cwd != ".").then(|| p.cwd.clone()),
                autostart: p.autostart,
                stop: (!p.stop.trim().is_empty()).then(|| p.stop.clone()),
            })
            .collect();
        config::write_project_setup(path, name, &agents, &processes)?;
    } else {
        write_new(path, fresh_contents)?;
    }
    if converting {
        let workspace_path = config::workspace_config_path(dir);
        if workspace_path != path {
            config::remove_workspace(&workspace_path)?;
        }
    }
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

fn project_intro(local_path: &Path, agents_in_global: bool) {
    println!("\n{}", bold("Project setup"));
    if agents_in_global {
        println!(
            "{}",
            dim("First run — your agents go in the global config (~/.mmux/config.yaml,")
        );
        println!(
            "{}",
            dim("used in every project); this project's setup goes in its mmux.yaml.")
        );
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
    println!("{}", bold("Done."));
    for w in wrote {
        println!("  • {w}");
    }
    println!("{}", dim("Edit those files to fine-tune the setup."));
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
    if color() {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn dim(text: &str) -> String {
    if color() {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// SGR colour codes for the interactive picker rows.
const GREEN: &str = "32";
const YELLOW: &str = "33";
const CYAN: &str = "36";
const CYAN_BOLD: &str = "1;36";

/// Wrap `text` in the SGR `code` (only when stdout is a terminal), for the picker.
fn paint(code: &str, text: &str) -> String {
    if color() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn header(text: &str) {
    println!("\n{}", bold(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn agent(name: &str, cmd: &str, args: &[&str]) -> Agent {
        Agent {
            name: name.into(),
            cmd: cmd.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn split_command_handles_blanks_and_args() {
        assert_eq!(
            split_command("npm run dev"),
            Some(("npm".into(), vec!["run".into(), "dev".into()]))
        );
        assert_eq!(
            split_command("  cargo  watch -x run "),
            Some((
                "cargo".into(),
                vec!["watch".into(), "-x".into(), "run".into()]
            ))
        );
        assert_eq!(split_command("solo"), Some(("solo".into(), vec![])));
        assert_eq!(split_command("   "), None);
        assert_eq!(split_command(""), None);
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
            stop: "docker compose down".into(),
            autostart: true,
        }];
        let yaml = build_local_yaml("myproj", &[], true, &procs);
        let cfg: Config = serde_yaml::from_str(&yaml).expect("local parses");
        assert_eq!(cfg.name.as_deref(), Some("myproj"));
        assert!(cfg.agents.is_empty()); // they're in the global file
        assert_eq!(cfg.processes.len(), 1);
        assert_eq!(cfg.processes[0].name, "Dev server");
        assert!(cfg.processes[0].autostart);
        assert_eq!(
            cfg.processes[0].stop.as_deref(),
            Some("docker compose down")
        );
        // The workspace example is commented documentation, never a live block.
        assert!(yaml.contains("# workspace:"));
        assert!(cfg.workspace.is_none());
    }

    #[test]
    fn local_yaml_with_local_agents_round_trips() {
        let agents = vec![agent(
            "Claude",
            "claude",
            &["--dangerously-skip-permissions"],
        )];
        let cfg: Config =
            serde_yaml::from_str(&build_local_yaml("p", &agents, false, &[])).expect("parses");
        assert_eq!(cfg.agents.len(), 1);
        assert!(cfg.processes.is_empty());
        assert!(cfg.workspace.is_none());
    }

    #[test]
    fn empty_local_yaml_is_still_valid() {
        // Nothing chosen: every section is a comment, but it must still parse.
        let cfg: Config = serde_yaml::from_str(&build_local_yaml("p", &[], true, &[]))
            .expect("commented-only file parses");
        assert!(cfg.agents.is_empty());
        assert!(cfg.processes.is_empty());
    }
}
