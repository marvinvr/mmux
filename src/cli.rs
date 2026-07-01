//! Command-line surface: argument dispatch and the non-TUI subcommands
//! (`init`, `check`, `--help`, `--version`). The actual work lives elsewhere —
//! this module just decides which entry point to run.

use crate::config::Config;
use anyhow::Result;
use std::path::PathBuf;

/// Parse `std::env::args()` and run the matching entry point. This is the whole
/// body of `main`.
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("mmux {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("init") {
        return crate::wizard::run(&std::env::current_dir()?);
    }
    if args.first().map(String::as_str) == Some("check") {
        return check();
    }
    if matches!(args.first().map(String::as_str), Some("docs") | Some("doc")) {
        print_docs();
        return Ok(());
    }
    if matches!(args.first().map(String::as_str), Some("attach") | Some("a")) {
        return crate::tmux::attach_picker();
    }

    // The inner process runs the actual TUI. The outer process is the tmux
    // attach-or-create wrapper. We distinguish via the --inner flag / env var
    // that the wrapper sets when it spawns the tmux session.
    let inner = std::env::var("MMUX_INNER").is_ok() || args.iter().any(|a| a == "--inner");
    if inner {
        let dir = std::env::var_os("MMUX_DIR")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let ws = Config::load_workspace(&dir)?;
        return crate::app::run(ws);
    }

    crate::tmux::launch()
}

/// Validate the effective workspace config in the current directory (root project
/// plus any `linked-projects`) without launching anything.
fn check() -> Result<()> {
    let dir = std::env::current_dir()?;
    let ws = Config::load_workspace(&dir)?;
    for w in &ws.warnings {
        println!("warning: {w}");
    }
    let multi = ws.projects.len() > 1;
    for cfg in &ws.projects {
        if multi {
            println!("\nproject {} ({})", cfg.display_name(), cfg.dir.display());
        }
        println!(
            "ok: {} — {} agent(s), {} process(es)",
            cfg.display_name(),
            cfg.agents.len(),
            cfg.processes.len()
        );
        for a in &cfg.agents {
            println!("  agent   {:<16} {} {:?}", a.name, a.cmd, a.args);
        }
        for p in &cfg.processes {
            println!(
                "  process {:<16} {} {:?}  (cwd: {}, autostart: {})",
                p.name,
                p.cmd,
                p.args,
                p.cwd.as_deref().unwrap_or("."),
                p.autostart
            );
        }
        if cfg.git_panel_enabled() {
            println!("  git     panel");
        } else {
            println!("  git     panel disabled");
        }
        let n = cfg.notifications.clone().unwrap_or_default();
        if n.enabled {
            println!(
                "  notify  {:?}  (unfocused-only: {}, throttle: {}s)",
                n.mechanism, n.only_when_unfocused, n.throttle_secs
            );
        } else {
            println!("  notify  disabled");
        }
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"mmux — persistent, per-directory multiplexer for AI agents & dev processes
https://mmux.org

USAGE:
    mmux            Open (or reattach to) the mmux session for the current directory
    mmux attach     Pick any running or recent mmux session and open it (alias: mmux a)
    mmux init       Interactive setup: pick agents, your start command & linked projects
    mmux check      Validate the effective config (global + project) and exit
    mmux docs       Explain what mmux is + how to write the config. If you (or an
                    AI agent) need setup instructions, run this — it prints them.
    mmux --help     Show this help

Each directory gets exactly one mmux session, kept alive inside tmux. Run `mmux`
again in the same directory to reattach to whatever was already running.

Config: mmux.yaml in the directory, layered on top of an optional global
~/.mmux/config.yaml (project values override the global ones). A private
mmux.local.yml deep-overrides the project file. See `mmux init`/`mmux docs`.
Add `linked-projects` to show other projects in one sidebar (see `mmux docs`).

KEYS (sidebar):  ↑/↓ move · [ ] switch project · Enter open · s start · x close · r restart · R reload config · ? about · d detach · q quit
KEYS (terminal): keys go to the focused pane · Ctrl-b then h=back d=detach x=close R=reload b=send Ctrl-b"#
    );
}

/// Print the self-contained "how mmux works + how to configure it" guide. This is
/// what `mmux docs` shows — written so a human or an AI agent can read it once and
/// know how to set up the project / global YAML without opening any other file.
fn print_docs() {
    println!(
        r##"mmux — persistent, per-directory multiplexer for AI agents & dev processes
===========================================================================

Home & docs: https://mmux.org

WHAT IT IS
    Type `mmux` in a directory and you get a TUI: a left sidebar split into
    Agents (Claude, Codex, … you spawn on demand), Terminals, and Processes
    (servers, scripts, … you start/stop and watch), plus a built-in git panel
    shown automatically when the directory is a git repo. The whole TUI runs
    inside an invisible tmux session keyed to that directory, so there is exactly
    ONE mmux per directory and it keeps running after you close the terminal or
    drop SSH. Run `mmux` again to reattach.

    Agent rows show a subtitle (the terminal title the program sets, e.g. what
    Claude is doing) and turn green when the agent goes idle (i.e. needs you).

CONFIG: TWO LAYERS, MERGED
    mmux reads a GLOBAL config and a PROJECT config and merges them at launch:

      ~/.mmux/config.yaml   global defaults — your agents everywhere
      ./mmux.yaml           per-project — this directory's processes etc.
                            (./mmux.yml also accepted)

    Project values win. `agents` and `processes` merge BY NAME (a project entry
    with the same name replaces the global one; otherwise it's appended). `name`,
    `git-panel`, `notifications`, and `auto-update` are overridden whole if the
    project sets them.
    Relative `cwd`s always resolve against the PROJECT directory, so a global agent
    runs in whatever project you're in. Either file alone is enough; you don't need both.

PROJECT FILE — ./mmux.yaml
    Run `mmux init` for an interactive setup wizard, or write it yourself:

      # `name` is optional; defaults to the directory's name.
      name: my-workspace

      agents:
        - name: Claude                 # label shown in the sidebar
          cmd: claude                  # executable on your PATH
          args: ["--dangerously-skip-permissions"]
          # cwd: .                      # optional, relative to this file
          # env: {{ KEY: value }}        # optional environment overrides

      processes:
        - name: Dev server
          cmd: npm
          args: ["run", "dev"]
          cwd: .                       # optional, relative to this file
          autostart: false             # start automatically when mmux opens?
          # env: {{ NODE_ENV: development }}

      # The git panel appears automatically when this dir is a git repo.
      # It needs no config; disable it with:
      # git-panel:
      #   enabled: false

LOCAL OVERRIDE — ./mmux.local.yml   (optional, project-only, usually git-ignored)
    A private per-developer file layered on top of ./mmux.yaml — for machine-specific
    tweaks (quiet notifications, a beta agent binary, a process only you run). Unlike
    the wholesale global→project merge, it is merged DEEPLY: nested maps merge key by
    key (a local `notifications: {{ enabled: false }}` keeps the project's mechanism),
    agents/processes merge by name (a same-named entry field-by-field, so you can
    override just one agent's args), and plain lists/scalars are replaced. There is no
    global counterpart. Layering order: global → project → local.

GLOBAL FILE — ~/.mmux/config.yaml
    Same schema. Put the things you want in EVERY project here — typically your
    agents — and keep per-project bits (processes, name) in the project file:

      agents:
        - name: Claude
          cmd: claude
          args: ["--dangerously-skip-permissions"]
        - name: Codex
          cmd: codex
          args: ["--dangerously-bypass-approvals-and-sandbox"]

LINKED PROJECTS — one sidebar for several projects
    Want several projects open together? List them under `linked-projects` and they
    all open in ONE mmux, each its own group in the sidebar — extra clones of a repo,
    a related repo, a service, whatever you want side by side. Switch between them
    with [ and ]; the git panel follows whichever project you're on.

      # in ./app/mmux.yaml
      linked-projects:
        - ../app2
        - ../api
        - ../docs

    It's loaded ONE level deep and de-duplicated by path, so you can drop the very
    same config into every project (even one that lists itself) and it will never
    expand recursively — at most 8 projects load. Changing the list takes effect
    on the next `mmux` (a reopen), not on `R` reload.

FIELD REFERENCE
    top level   name (str, optional) · agents[] · processes[] · git-panel (optional)
                · notifications (optional) · auto-update (optional)
                · linked-projects[] (paths, root config only)
    agent       name* · cmd* · args[] · cwd · env{{}}
    process     name* · cmd* · args[] · cwd · env{{}} · autostart (bool)
    git-panel   enabled (bool, default true; the panel is automatic for git repos)
    auto-update enabled (bool, default true; Homebrew installs only — checks on start
                and every 6 hours, installs in the background, shows a "restart to
                update" badge. Off for non-brew/dev builds or with MMUX_NO_UPDATE=1)
    (* required. cwd is relative to the file's directory. Omitted lists/maps are empty.)

QUICK START
    cd ~/some/project
    mmux init      # interactive setup wizard (agents, start command, linked projects)
    mmux check     # print the effective merged config without launching the TUI
    mmux           # open / reattach
    mmux a         # `mmux attach`: pick ANY running mmux session and join it"##
    );
}
