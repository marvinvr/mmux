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
  `lifecycle.rs`, `input.rs`, `overlay.rs`, `app/git.rs`, `view/*` each add `impl App { … }`. Methods called
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
  add a direct `vt100` dependency. `Cargo.toml` temporarily patches it to the exact upstream-PR
  commit that retains rows displaced by top-aligned scroll regions; remove the override once that
  fix ships in the selected 0.16 release.
- **ratatui 0.30 / crossterm 0.29** via the `crossterm_0_29` feature. Always use
  `ratatui::crossterm::*`, never a standalone crossterm dep.
- **No logging crate.** There is no `log`/`tracing`. Errors surface as `Session.error` strings
  shown in the pane/sidebar, or the footer `flash`.
- **Comments explain *why*.** Match the existing density and tone — they capture load-bearing
  rationale (e.g. the cursor double-invert note in `view/pane.rs`, the OSC-9 ConEmu-progress
  guard in `pane.rs`).
- **No co-author trailer** on commits in this repo.

## Tests

`cargo test` covers the pure, easily-isolated pieces: `keymap::encode_key`, the `input.rs`
cell/selection geometry (`cell_at`, `Selection::ordered`), the `pane.rs` mouse-sequence encoding
and top-aligned-scroll-region history regression,
the `picker.rs` fuzzy score, `notify.rs` escape formatting, the `config/` module (the
project-over-global `merge` precedence in `config/mod.rs` and the comment-preserving YAML splicer in
`config/yaml.rs`), `git::parse_change` porcelain parsing
plus the changed-files tree, `tmux::session_name` hashing, `agent.rs` session-id parsing,
`update.rs` version comparison + release-redirect parsing, and the `wizard.rs` YAML builders. The
PTY/TUI layers are verified by hand by the maintainer — with one automated exception below.

## Continuous Integration

`.github/workflows/ci.yml` runs on every push to `main` and every PR, all jobs in parallel:

- **`lint`** — `shellcheck` + a shebang-aware syntax check over `ci/*.sh`, `web/install.sh`, and
  `.github/scripts/*.sh`. (No `cargo fmt`/`clippy` gate — this codebase's style is hand-tuned, so a
  formatter gate would fail spuriously.)
- **`test`** — `cargo test`.
- **`build+smoke`** — the reusable `.github/workflows/_build.yml`: builds the release binary for
  all three shipped targets, **each on its own native-arch runner** (macOS arm `macos-14`, Linux
  x86 `ubuntu-latest` / arm64 `ubuntu-24.04-arm`), then runs `ci/smoke.sh` on the
  binary it just built — relocated and (macOS) re-signed exactly as an install would, so the
  "does it actually execute on this platform" question is answered on every commit, not just at a
  tag. Linux targets are static musl builds (run on any distro regardless of glibc). Builds are
  stamped `MMUX_RELEASE=1` — the marker the self-updater checks (see `src/update.rs`).
- **`boot-test`** (non-blocking) — `ci/boot-test.sh` boots the real TUI under a pseudo-terminal
  (`mmux --inner` inside a throwaway tmux session). It fails on an observed panic, and **skips**
  (with diagnostics) if it can't drive a frame — some headless CI PTYs never report pane content
  back, so "no frame" isn't treated as a failure. Kept `continue-on-error` regardless; it's a
  signal, not a gate. It renders fully when run locally.

`ci/smoke.sh <binary> <version>` is the shared smoke test reused everywhere: it exercises only the
non-TTY subcommands (`--version` must match, `--help`, `docs`, and `check` on a valid vs. invalid
config), which is enough to catch essentially every distribution break.

## Release

Releases are tag-driven, and **staged**: a tag is verified on real per-platform runners before it's
distributed, so a broken build never reaches users. Pushing a `v*` tag (e.g.
`git tag v0.1.4 && git push origin v0.1.4`) runs `.github/workflows/release.yml`:

1. **`build`** — the same reusable `_build.yml` (three native-arch builds + smoke), uploading the
   tarballs as artifacts.
2. **`publish-prerelease`** — attaches the tarballs + a `checksums.txt` and creates the GitHub
   Release as a **prerelease**. GitHub's `releases/latest` excludes prereleases, so install.sh, the
   in-app self-updater, and Homebrew all keep serving the previous version at this point.
3. **`verify`** (parallel, per platform) — installs the staged release the way a user would and
   smoke-tests it: `web/install.sh` pinned to the tag on each runner; on macOS, the Homebrew formula
   is rendered against the real asset URLs/shas and `brew install --formula`'d + `brew test`'d
   locally (proving it before it touches the tap); on Linux, `ci/verify-distros.sh` runs the
   installer inside Alpine (musl, no glibc), Ubuntu 20.04 (old glibc), Debian, and Fedora containers.
4. **`promote`** — only if every verify leg passed: flips the release to `latest` and pushes the
   `marvinvr/homebrew-mmux` tap formula bump (macOS binaries; Linux installs via the script, so the
   tap builds Linux from source). This is the single point where the new version becomes live.
5. **`notify-failure`** — if build/publish/verify failed (so nothing was promoted), opens an issue
   assigned to the maintainer. Users are unaffected; fix forward with a new patch tag.

The formula render is shared by `.github/scripts/render-formula.sh` (used by both the verify job's
local test and `bump-formula.sh`'s tap push).

**Post-release safety net.** `.github/workflows/healthcheck.yml` runs weekly (and on demand),
re-installing the *current* `latest` via the LIVE `mmux.org/install.sh` and Homebrew tap across all
platforms + distro containers. If a live channel has rotted, it auto-invokes
`.github/workflows/yank.yml`, which demotes the release to a prerelease — so `releases/latest` (and
therefore install.sh + the self-updater) fall back to the last good version with no rebuild — and
reverts the tap. `yank.yml` can also be run manually (`workflow_dispatch`) against any tag.

`mmux.yaml` also defines a "Build (all platforms)" process for producing `dist/` binaries
locally (macOS arm64 native + a static Linux musl build via `cargo-zigbuild`).

## Planned and Known Limits

- **Persistence covers detach/disconnect only.** tmux keeps the TUI alive across closing the
  terminal or dropping SSH, but a crash of the TUI process or a reboot still loses live panes. The
  planned v2 is a **daemon + thin client** split.
- **Stable selection (the natural next refactor).** Replace the positional `sel`/`build_nav()`
  with a selection-by-identity model (e.g. a `SessionId`) in `nav.rs`. It removes the
  rebuild-and-clamp dance; `nav.rs` is intentionally the single file that would change.
- **Copy is drag-select only.** A keyboard copy-mode is still future. (Note: over a program that
  tracks the mouse, the drag goes to *it* — hold Shift to drag-select for the clipboard instead.)
- **Attention detection is bell/OSC-based.** It keys off the terminal bell and program-emitted
  notification OSCs (9/777/99). An idle "agent went quiet" heuristic is still future.
- **Workspace manifests are flat and structural.** They load at most 10 member folders and do not
  nest. `R` adds and removes folders live; removal kills that member's panes and compacts runtime
  project indices without touching its Git worktree. Reordering members needs a reopen. Restore
  snapshots carry both the legacy member index and the canonical member directory, so manifest
  reordering resolves safely.

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
