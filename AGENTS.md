# AGENTS.md

Orientation for AI agents (and humans) working on **mmux**. Read this first; it's the map.

## What mmux is

A persistent, per-directory terminal multiplexer for AI coding agents and dev processes.
Single Rust binary, runs in any terminal (incl. over SSH). You get a ratatui TUI: a left
**sidebar** (Agents you spawn on demand · Terminals · Processes you start/stop/watch) and a
main pane showing the selected program's live terminal, plus an optional persistent **right
panel** (e.g. lazygit). The whole TUI runs inside an invisible, per-directory **tmux session**
so it survives detach/disconnect and is a singleton per directory. A config can list
sibling **`linked-projects`** (e.g. extra clones) so several directories share one sidebar,
each its own group; the active project's panel is the one shown ("follow active").

User-facing docs are in `README.md`. This file is about the *code*.

## Build & check

```sh
cargo build            # primary check while iterating
cargo build --release  # optimized binary at target/release/mmux
cargo test             # unit tests (currently: keymap encoding)
mmux check             # validate the effective (global+project) config, no TUI
```

Notes:
- **Don't try to run the TUI in a headless/non-TTY shell** — it needs a real terminal
  ("Device not configured" otherwise). Verify changes with `cargo build` + `cargo test` +
  `mmux check`; the maintainer runs the interactive TUI and reports back.
- **macOS install gotcha:** a freshly `cp`'d ad-hoc-signed binary gets SIGKILL ("Killed: 9",
  exit 137) on first exec. Fix with `codesign --force --sign - <path>` after copying, or use
  `cargo install --path .` (which handles it).
- Commits in this repo do **not** use a co-author trailer.

## Module map

Two halves: a thin CLI/tmux outer shell, and the `app/` TUI it launches.

```
src/
  main.rs        Entry point — just calls cli::run().
  cli.rs         Arg dispatch + non-TUI subcommands (init wizard / check / docs / --version).
  tmux.rs        ALL tmux interaction: launch (attach-or-create the per-dir session),
                 attach_picker (`mmux attach`), detach. Session name = hash of canonical dir.
                 First run with no config anywhere runs the init wizard instead of erroring.
  wizard.rs      Interactive `mmux init` (and the first-run path): asks agents (Claude/Codex
                 presets ± danger mode), the start command, and linked-projects, then writes
                 the YAML. Agents seed a fresh GLOBAL config when none exists, else go local;
                 the project file always gets the rest. Pure builders unit-tested.
  config.rs      mmux.yaml + ~/.mmux/config.yaml load & merge (project overrides global;
                 agents/processes merge by name) + load_workspace (root + linked-projects,
                 one level deep, path-deduped → Workspace). Clean — touch sparingly.
  pane.rs        One PTY-backed pane: portable-pty + a vt100 parser on a reader thread, a
                 writer thread draining an input channel, a reaper thread. vt100 Callbacks
                 capture the OSC title, bell + notification OSCs (9/777/99). Clean — touch sparingly.
  notify.rs      Pure formatting of the desktop-notification escape (OSC 9/777/bell) + the
                 tmux passthrough wrapping + the `command` fallback. Unit-tested.
  app/
    mod.rs       The `App` struct (all state) + the `Project` struct (per-project cfg /
                 counters / panel) + new() + the run()/run_loop() event loop + follow-active
                 + per-project panel auto-restart (tick/restart_active_panel) + notification
                 drain/emit (collect_notifications, after each draw) + focus/resize helpers.
    session.rs   Session / Recipe / Kind / Status. The UNIFIED pane-backed model: one
                 spawn()/stop()/kill() lifecycle for agents, terminals, processes and panel.
    nav.rs       The sidebar nav list (build_nav) + the selection cursor (sel) + pane_at /
                 move_sel / select_session / focused_pane.
    lifecycle.rs Actions driven from the sidebar: spawn_agent/spawn_terminal, activate,
                 do_start/do_stop/do_restart, and reload (live config re-read & reconcile).
    input.rs     on_key (sidebar keys + the Ctrl-b leader for panes), on_mouse (focus
                 routing, hamburger buttons, scroll wheel), on_paste, plus the `hit` helper.
    keymap.rs    encode_key: pure crossterm-key → PTY-byte translation. Unit-tested.
    view/
      mod.rs     draw(): responsive layout split, the per-frame Regions (hit rects), footer,
                 panel button. COMPACT_W / MAIN_MIN live here.
      sidebar.rs render_sidebar: sections + one row per nav entry via nav_row().
      pane.rs    render_main / render_right (share render_screen/render_placeholder),
                 main_title, placeholder_text.
      theme.rs   Colors + entry_line / header / status_style / badge.
```

## Core model — read before editing the TUI

- **`Session`** (`app/session.rs`) is the one pane abstraction. It owns `spawn(rows,cols)`
  (= start *and* restart: kill any existing pane, spawn fresh) and `stop()` (kill, keep the
  exited pane). Agents, terminals and processes all live in **one** `Vec<Session>` on `App`,
  distinguished by `Kind` and tagged with a `project` index. Filter by `project`+`Kind` to
  build each sidebar group. **Do not** re-introduce per-kind collections or per-kind spawn/stop
  methods — that triplication is exactly what was removed.
- **Workspaces / projects.** `App.projects: Vec<Project>` holds the root project (the dir mmux
  was launched in, `[0]`) plus any `linked-projects`. Each `Project` owns its `cfg`, per-agent
  `counts`/`term_count`, and **its own** right panel. `App.active` tracks the selected row's
  project; `tick()` keeps it in sync and lazily spawns that project's panel ("follow active").
  Single-project workspaces render exactly as before (no project header).
- **The right panel** is a `Kind::Panel` `Session` living on each `Project` (not the `sessions`
  vec) because it has its own column and throttled auto-restart. Only the **active** project's
  panel is shown/rendered (`active_panel()`/`active_panel_w()`); others stay alive in the
  background once spawned. It still reuses `Session::spawn`; `App.panel_size` is the shared
  column's last size.
- **Navigation is positional**: `App.sel` indexes the `Vec<Nav>` returned by `build_nav()`,
  which is rebuilt on demand and is grouped per project. `Nav::Session(i)` indexes into
  `sessions`; launchers (`NewAgent`/`NewTerminal`) carry their project. This is the one place
  that's deliberately *not* yet ideal — see "Planned" below.
- **`Focus`** = which region gets keys: `Sidebar` (nav keys), `Terminal` (main pane), `Right`
  (panel). `focused_pane()` resolves it.
- **`Regions`** (`view/mod.rs`) is per-frame mouse geometry: rendering writes the rects, input
  reads them. It's reset at the top of every `draw()`. If you add a clickable area, set its rect
  during render and test it in `on_mouse`.

## Conventions

- **One inherent `impl App`, split across files.** `app/mod.rs` defines the struct; `nav.rs`,
  `lifecycle.rs`, `input.rs`, `view/*` each add `impl App { … }`. Methods called from a sibling
  module must be `pub(crate)`; struct fields stay private (descendant modules can see them).
  Put a new method in the file that matches its concern, not in `mod.rs`.
- **Refactors are behaviour-preserving.** This codebase was recently restructured; match
  existing behaviour unless a change is the explicit point. Keep comments at the existing
  density and tone (they explain *why*, e.g. the cursor double-invert note in `view/pane.rs`).
- **vt100 is 0.16**: title/bell arrive via the `Callbacks` trait (`pane.rs`), not `Screen`
  methods. `tui_term::vt100` is re-exported so versions don't skew — import from there.
- **ratatui 0.30 / crossterm 0.29** via the `crossterm_0_29` feature; always use
  `ratatui::crossterm::*`, never a standalone crossterm dep.
- No `log`/`tracing` crate — there is no logging subsystem. Errors surface as `Session.error`
  strings shown in the pane/sidebar, or the footer `flash`.

## Planned / known limits

- **Persistence** covers detach/disconnect only (tmux keeps the TUI alive). A TUI crash or
  reboot still loses live panes — the planned v2 is a **daemon + thin client** split.
- **Stable selection (the natural next refactor):** replace positional `sel`/`build_nav()`
  with a selection-by-identity model in `nav.rs` (e.g. a `SessionId`). It removes the
  rebuild-and-clamp dance; `nav.rs` is intentionally the single file that would change.
- Attention detection keys off the terminal **bell** and program-emitted notification OSCs
  (9/777/99), both captured in `pane.rs`. They drive the sidebar dot *and* a desktop
  notification (`notify.rs` → emitted from `collect_notifications` after each draw, wrapped in
  tmux passthrough so it survives the jail and works over SSH). An idle "agent went quiet"
  heuristic is still future. Mouse events inside a focused pane aren't forwarded to the program
  yet (so lazygit's mouse won't work — clicks/wheel drive mmux's own focus + scrollback). Copy is **mouse
  drag-select only** (drag in a pane → clipboard via OSC 52 + a `pbcopy`/`xclip` fallback, see
  `clipboard.rs`); a keyboard copy-mode is still future.
- **Multi-directory workspaces** (`linked-projects`) group sibling dirs in one sidebar, the
  *set* fixed at launch — changing the list needs a reopen (`R` reload only refreshes each
  loaded project in place). Loading is one level deep + path-deduped (`config::load_workspace`),
  so a config shared verbatim across clones never recurses. Cross-project grouping into named
  workspaces (beyond plain sibling lists) is still future.
