# mmux Documentation

mmux is a persistent, per-directory terminal multiplexer for AI coding agents and dev
processes. It is a single Rust binary that runs in any terminal — including over SSH — and
gives you a TUI for spawning agents, watching processes, and tracking git, all scoped to the
directory you launched it in.

Type `mmux` in a project directory and you get:

- a left **sidebar**, split per project into **Agents** (Claude, Codex, … you spawn on
  demand), **Terminals** (throwaway shells), and **Processes** (dev servers, scripts you
  start/stop and watch);
- a **main pane** showing the selected program's live terminal;
- a built-in **git panel** in the right column, shown automatically whenever the directory is
  a git repository.

The whole TUI runs inside an invisible, per-directory **tmux session**. That gives mmux its two
defining properties:

- **One mmux per directory.** The session is keyed to the directory's canonical path, so a
  second `mmux` in the same place attaches to the one that is already running rather than
  starting a new one.
- **It survives.** Closing the terminal or dropping an SSH connection detaches but does not
  kill it. Run `mmux` again — or `mmux attach` from anywhere — to rejoin exactly where you
  left off.

When a program rings the terminal bell or emits a notification escape (for example, Claude Code
announcing it is done), mmux lights a red attention dot on the sidebar row **and** raises a
native desktop notification — even across an SSH hop.

## Why mmux

A coding agent is a long-running, interactive process that you leave alone and come back to.
Plain terminal tabs lose that work the moment the connection drops, and juggling several agents
plus a dev server plus git across tabs is its own chore. mmux keeps every one of those in a
single, persistent, per-directory surface:

- spawn and restart agents from one list, each in its own pane;
- start, stop, and tail your processes without leaving the multiplexer;
- stage, commit, branch, and push from a native git panel — no separate tool;
- get notified when an agent needs you, wherever your terminal is running;
- group several clones of a repo into one sidebar with [linked projects](04-configuration.md#linked-projects).

## The Mental Model

```text
your terminal  (local · ssh · tmux client)
      │
      ▼
┌─────────────────────────────────────────────────┐
│  tmux session   (one per directory · invisible)  │  ← survives detach / disconnect
│  ┌─────────────────────────────────────────────┐ │
│  │  mmux  (ratatui TUI)                         │ │
│  │   sidebar    │   main pane    │   git panel  │ │
│  │   agents     │   the focused  │   changes    │ │
│  │   terminals  │   program's    │   branches   │ │
│  │   processes  │   live screen  │   recent     │ │
│  └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
      every agent / terminal / process = one PTY + a vt100 parser
```

The TUI is the *inner* process. The bare `mmux` command you type is the *outer* process: a thin
wrapper that attaches to — or creates — the tmux session and launches the inner TUI inside it.
See [Architecture](06-architecture.md) for how the two halves fit together.

## Recommended Reading Order

1. **[Quick Start](01-quick-start.md)** — install and open your first session in under a minute.
2. **[Installation](02-installation.md)** — prerequisites, Homebrew, building from source, the
   macOS code-signing gotcha.
3. **[Usage](03-usage.md)** — the interface tour, every keybinding, the mouse, and the git panel.
4. **[Configuration](04-configuration.md)** — `mmux.yaml`, the global/project merge, and linked
   projects.
5. **[Notifications](05-notifications.md)** — how attention detection and desktop notifications
   work, including over SSH.

Working **on** mmux rather than with it? Start with **[Architecture](06-architecture.md)** and
the **[Module Map](07-module-map.md)**, then read **[Contributing](08-contributing.md)** — which
also explains how this documentation is maintained.
