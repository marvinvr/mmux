# AGENTS.md

Orientation for AI agents (and humans) working on **mmux**. Read this first, then the docs.

> `CLAUDE.md` is a symlink to this file.

## What mmux Is

A persistent terminal multiplexer for AI coding agents and dev processes: a single
Rust binary with a ratatui TUI (sidebar of agents/terminals/processes + a main pane + a native git
panel) that runs inside an invisible, per-directory tmux session so it survives detach/disconnect.
The user-facing story is in [`README.md`](README.md); the full picture is in [`docs/`](docs/).

## Where to Look

The canonical documentation is **[`docs/`](docs/)**. Two pages matter most when working on the
code:

- **[docs/06-architecture.md](docs/06-architecture.md)** — the conceptual model: the inner/outer
  tmux split, the unified `Session` model, workspaces/projects + follow-active, the native git
  panel and overlays, navigation/focus/regions, and the data-flow + "why it's built this way".
- **[docs/07-module-map.md](docs/07-module-map.md)** — a file-by-file index of `src/`.

[docs/08-contributing.md](docs/08-contributing.md) covers conventions, tests, the release flow,
and the known limits. Everything user-facing (keys, config, notifications) is in
[docs/03-usage.md](docs/03-usage.md), [docs/04-configuration.md](docs/04-configuration.md), and
[docs/05-notifications.md](docs/05-notifications.md).

## Documentation Maintenance

**This documentation is part of the codebase. Keep it accurate, and correct errors the moment you
spot one** — even outside the change you came to make. A wrong doc is worse than none, and this
tree drifted badly once (it described a `lazygit` right panel long after the panel became native
git). Don't let it happen again.

- **`docs/*.md` is the source of truth.** When you change keys, config fields, commands, the
  schema, notifications, the git panel, or any user-observable behavior, update the relevant
  `docs/` page **in the same change**.
- When you touch the architecture, update [docs/06-architecture.md](docs/06-architecture.md) and
  the [module map](docs/07-module-map.md) — add new files, fix changed roles, record the *why*.
- Keep the lightweight surfaces in step: `README.md` (overview + links only), this file, and the
  documentation surfaces embedded in code — the `mmux docs`/`mmux --help` text in `src/cli.rs` and
  the YAML the wizard writes in `src/wizard.rs`.

The full covenant is in
[docs/08-contributing.md → Maintaining This Documentation](docs/08-contributing.md#maintaining-this-documentation).

## Build & Check

```sh
cargo build            # primary check while iterating
cargo test             # unit tests
mmux check             # validate the effective (global + project) config, no TUI
```

## Constraints

- **Don't run the TUI in a headless/non-TTY shell** — it needs a real terminal. Verify with
  `cargo build` + `cargo test` + `mmux check`; the maintainer runs the interactive TUI and reports
  back. Never start the dev server, app, or anything long-running yourself.
- **macOS:** a freshly `cp`'d ad-hoc-signed binary gets SIGKILL ("Killed: 9"). Re-sign with
  `codesign --force --sign - <path>`, or use `cargo install --path .`.
- **Commits do not use a co-author trailer.**
- **Keep the unified Session model unified** — agents, terminals, and processes share one
  `Vec<Session>` and one `spawn`/`stop`/`kill` lifecycle, tagged by `Kind`. Do **not** add
  per-kind collections or lifecycle methods (that triplication was deliberately removed). The git
  panel is **not** a `Session` — it's `Project.git: Option<GitPanel>`.
- **Pins:** vt100 0.16 via `tui_term::vt100` (never a direct dep; title/bell/OSCs come through the
  `Callbacks` trait); ratatui 0.30 / crossterm 0.29 via the `crossterm_0_29` feature (use
  `ratatui::crossterm::*`). No `log`/`tracing` — errors surface as `Session.error` or the footer
  `flash`.
- **`config/` (`config/mod.rs` + `config/yaml.rs`) and `pane.rs` are clean — touch sparingly.**
  Match the existing comment density; comments explain *why*.
