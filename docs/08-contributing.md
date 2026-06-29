# Contributing

How to build, test, and extend mmux — and how to keep this documentation honest.

## Build & Check

```sh
cargo build            # primary check while iterating
cargo build --release  # optimized binary at target/release/mmux
cargo test             # unit tests
mmux check             # validate the effective (global + project) config, no TUI
```

> **Don't run the TUI in a headless / non-TTY shell.** It needs a real terminal ("Device not
> configured" otherwise). Verify changes with `cargo build` + `cargo test` + `mmux check`, and
> let the maintainer drive the interactive TUI and report back.

### macOS install gotcha

A freshly `cp`'d ad-hoc-signed binary gets `SIGKILL` ("Killed: 9", exit 137) on first exec. Fix
it with `codesign --force --sign - <path>` after copying, or use `cargo install --path .` (which
handles it). See [Installation](02-installation.md#from-source).

## Conventions

- **One inherent `impl App`, split across files.** `app/mod.rs` defines the struct; `nav.rs`,
  `lifecycle.rs`, `input.rs`, `app/git.rs`, `view/*` each add `impl App { … }`. Methods called
  from a sibling module must be `pub(crate)`; struct fields stay private (descendant modules see
  them). Put a new method in the file that matches its concern, not in `mod.rs`.
- **Keep the unified Session model unified.** Agents, terminals, and processes share one
  `Vec<Session>` and one `spawn`/`stop`/`kill` lifecycle, tagged by `Kind`. Do **not** add
  per-kind collections or per-kind lifecycle methods — that triplication was deliberately removed.
  The git panel is **not** a `Session`; it's `Project.git: Option<GitPanel>`.
- **Refactors are behaviour-preserving.** This codebase has been restructured deliberately; match
  existing behaviour unless a change is the explicit point.
- **Clickable areas go through `Regions`.** Write the rect during render, read it in `on_mouse`.
  `Regions` is reset every frame — there is no cross-frame persistence.
- **vt100 is 0.16, via `tui_term`.** Title/bell/OSCs arrive through the `Callbacks` trait
  (`pane.rs`), not `Screen` methods. Import from `tui_term::vt100` so versions can't skew — never
  add a direct `vt100` dependency.
- **ratatui 0.30 / crossterm 0.29** via the `crossterm_0_29` feature. Always use
  `ratatui::crossterm::*`, never a standalone crossterm dep.
- **No logging crate.** There is no `log`/`tracing`. Errors surface as `Session.error` strings
  shown in the pane/sidebar, or the footer `flash`.
- **Comments explain *why*.** Match the existing density and tone — they capture load-bearing
  rationale (e.g. the cursor double-invert note in `view/pane.rs`, the OSC-9 ConEmu-progress
  guard in `pane.rs`).
- **No co-author trailer** on commits in this repo.

## Tests

`cargo test` currently covers the pure, easily-isolated pieces: `keymap::encode_key`, the
`input.rs` cell/selection geometry (`cell_at`, `Selection::ordered`), the `picker.rs` fuzzy
score, `notify.rs` escape formatting, parts of `config.rs`, and the `wizard.rs` YAML builders.
The PTY/TUI layers are verified by hand by the maintainer.

## Release

Releases are tag-driven. Pushing a `v*` tag (e.g. `git tag v0.1.4 && git push origin v0.1.4`)
runs `.github/workflows/release.yml`, which:

1. builds a release binary for each target (`aarch64-apple-darwin`, `x86_64-apple-darwin`,
   `x86_64-unknown-linux-gnu`);
2. creates the GitHub Release with the tarballs attached;
3. rewrites the formula in the `marvinvr/homebrew-mmux` tap to point at the new binaries.

`mmux.yaml` also defines a "Build (all platforms)" process for producing `dist/` binaries
locally (macOS arm64 native + a static Linux musl build via `cargo-zigbuild`).

## Planned and Known Limits

- **Persistence covers detach/disconnect only.** tmux keeps the TUI alive across closing the
  terminal or dropping SSH, but a crash of the TUI process or a reboot still loses live panes. The
  planned v2 is a **daemon + thin client** split.
- **Stable selection (the natural next refactor).** Replace the positional `sel`/`build_nav()`
  with a selection-by-identity model (e.g. a `SessionId`) in `nav.rs`. It removes the
  rebuild-and-clamp dance; `nav.rs` is intentionally the single file that would change.
- **Mouse (except the wheel) isn't forwarded into panes.** Clicks and drag drive mmux's own focus,
  scrollback, and copy — so an inner program's mouse (e.g. a TUI you run in a terminal) doesn't
  receive button/motion events. The wheel is the exception: over a full-screen program on the
  alternate screen it's handed through (`Pane::wheel_input`) as a wheel event or arrow keys.
- **Copy is drag-select only.** A keyboard copy-mode is still future.
- **Attention detection is bell/OSC-based.** It keys off the terminal bell and program-emitted
  notification OSCs (9/777/99). An idle "agent went quiet" heuristic is still future.
- **Linked projects are a flat list.** A project can be *added* live — the `+ Link another project`
  button (or `L`) appends to `linked-projects` and loads it in place — but *removing* one, or any
  other hand-edit to the list, needs a reopen (`R` reload only refreshes loaded projects in place).
  Named, nestable workspaces are still future.

## Maintaining This Documentation

**This documentation is part of the codebase. Every contributor — human or AI agent — is expected
to keep it accurate and to fix errors the moment they spot one.** A wrong doc is worse than no
doc, and this tree drifted badly once already (it described a `lazygit` right panel long after it
became a native git panel). Don't let that happen again.

The rules:

- **`docs/*.md` is the canonical source of truth.** When you change behavior — keys, config
  fields, commands, the config schema, notifications, the git panel, layout, or any
  user-observable detail — update the relevant `docs/` page **in the same change**.
- **Keep these aligned:**
  - `docs/*.md` — the canonical, human- and agent-facing documentation.
  - `README.md` — a short overview, quick start, and links **only**; it points here.
  - `AGENTS.md` — orientation, this covenant, build/check, and project constraints. `CLAUDE.md`
    is a symlink to it.
  - the embedded `mmux docs` / `mmux --help` text in `src/cli.rs`, and the YAML the
    `mmux init` wizard writes in `src/wizard.rs` — these are documentation surfaces too. Keep them
    in step with the config schema.
- **When you touch the architecture**, update [Architecture](06-architecture.md) and the
  [Module Map](07-module-map.md): add new files to the map, correct any role that has changed, and
  record the *why* behind a non-obvious decision.
- **If you read something here that's wrong, fix it** — even if it's outside the change you came
  to make. Leave the tree more correct than you found it.

The goal is a codebase that stays easy to navigate, with every important decision written down,
clear, and current.
