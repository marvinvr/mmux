# Module Map

A file-by-file index of the source tree. For the concepts behind it, read
[Architecture](06-architecture.md) first. Keep this map current — see the
[documentation covenant](08-contributing.md#maintaining-this-documentation).

The binary has two halves: a thin CLI/tmux outer shell, and the `app/` TUI it launches.

## Outer Shell (`src/`)

| File | Responsibility |
| --- | --- |
| `main.rs` | Entry point — declares the module tree and calls `cli::run()`. |
| `cli.rs` | Argument dispatch and the non-TUI subcommands: `--help`/`--version`, `init`, `check`, `docs`/`doc`, `attach`/`a`, and the inner-vs-outer (`MMUX_INNER`/`--inner`) split. Also holds the embedded `mmux docs` setup guide. |
| `tmux.rs` | All tmux interaction: attach-or-create the per-directory session (`launch`/`launch_in`), configure it invisible, `detach`, the recents log (`~/.mmux/history`), and the `mmux attach` picker. Session name = hash of the canonical dir. |
| `wizard.rs` | Interactive `mmux init` (and the first-run path): asks for agents (Claude/Codex presets ± danger mode), start commands, and linked projects, then writes commented YAML. Agents seed a fresh global config when none exists, else go local. Pure builders are unit-tested. |
| `config.rs` | The `mmux.yaml` + `~/.mmux/config.yaml` schema, load & merge (project over global; agents/processes by name), and `load_workspace` (root + linked projects, one level deep, path-deduped, capped at 8). Also the comment-preserving `append_process` YAML writer used by the `+ New Process` form. Clean — touch sparingly. |
| `pane.rs` | One PTY-backed pane: `portable-pty` + a `vt100` parser on a reader thread, a writer thread draining an input channel, a reaper thread. A `Callbacks` impl captures the OSC title, bell, and notification OSCs (9/777/99), and `contents_block` stitches scrollback text for copy. Clean — touch sparingly. |
| `notify.rs` | Pure formatting of the desktop-notification escape (OSC 9/777/bell), the tmux passthrough wrapping, and the `command` fallback. Unit-tested. |
| `clipboard.rs` | `copy(text)` — writes an OSC 52 escape to stdout **and** pipes to a local helper (`pbcopy`/`wl-copy`/`xclip`/`xsel`), with a hand-rolled base64 encoder. |
| `git.rs` | Stateless, synchronous wrappers over the `git` CLI for the native git panel: parse `status`/`log`/`branch`; stage/discard/commit/switch/pull/push/stash; `diff` one file for the preview; build the flattened changed-files tree. Errors come back as plain strings. |

## The TUI (`src/app/`)

One inherent `impl App`, split across files. `mod.rs` defines the struct; the others each add
`impl App { … }` for their concern.

| File | Responsibility |
| --- | --- |
| `mod.rs` | The `App` struct (all state) and the `Project` struct (per-project cfg / counters / git panel) + `new()` + the `run()`/`run_loop()` event loop + `tick()` (follow-active, git job drain, panel refresh, ephemeral pruning) + notification drain/emit + focus/resize helpers. |
| `session.rs` | The unified pane-backed model: `Session` / `Recipe` / `Kind` / `Status`. One `spawn()`/`stop()`/`kill()` lifecycle for agents, terminals, and processes. `Recipe` builders: `agent`/`process`/`shell`/`editor`. |
| `nav.rs` | The sidebar nav list (`build_nav`) + the positional selection cursor (`sel`) + resolvers: `project_of`/`current_nav`/`pane_at`/`move_sel`/`jump_project`/`focus_project`/`select_session`/`focused_pane`. |
| `lifecycle.rs` | Sidebar-driven actions: `spawn_agent`/`spawn_terminal`, `open_new_process`/`finish_new_process`, `open_picker`/`open_in_editor`, `activate`, `do_start`/`do_stop`/`do_restart`, `close_session`/`prune_ephemeral`, and `reload` (live config re-read & reconcile). |
| `input.rs` | `on_key` (overlay routing → global `Ctrl+P` → focus dispatch), `key_sidebar`/`key_pane` (with the `Ctrl-b` leader)/`key_git`, `overlay_key`/`procform_key`, `on_mouse` (click/drag/wheel routing), `on_paste`, the drag-select-to-clipboard machinery, footer-button actions, and the `hit` helper. `Selection`/`SelTarget` live here. |
| `keymap.rs` | `encode_key`: pure crossterm-key → PTY-byte translation. Unit-tested. |
| `git.rs` | The native git panel: `GitPanel` (refresh, cursor/section nav, stage/commit/discard/stash, backgrounded pull/push), the `Overlay`/`PromptKind`/`Confirmed` modal enums, the `DiffView` main-pane diff preview (build/follow/scroll/`diff_upkeep`), and the `impl App` git-action methods. **Not** a pane. |
| `picker.rs` | The `Ctrl+P` fuzzy file picker: list files (`rg --files` → `git ls-files` → manual walk) and rank them. Fuzzy score is unit-tested. |
| `procform.rs` | State for the `+ New Process` guided form (Name → Command → Cwd → Review) with an autostart toggle and per-field validation. |

### View (`src/app/view/`)

| File | Responsibility |
| --- | --- |
| `mod.rs` | `draw()`: the responsive layout split, the per-frame `Regions` hit-rects, the footer (clickable shortcut chips via `FooterAction`/`Seg`), and the panel "open" button. `COMPACT_W` / `MAIN_MIN` live here. |
| `sidebar.rs` | `render_sidebar`: per-project boxes (or a single drawer), the `AGENTS`/`TERMINAL`/`PROCESSES` (+ compact `GIT`) sections, and one styled row per nav entry via `nav_row()`. |
| `pane.rs` | `render_main` / `render_right` (share `render_screen`/`render_placeholder`), `main_title`, `placeholder_text`, the diff preview (`render_diff`/`diff_title`), the focused-pane cursor placement, and `paint_selection` (the drag-selection overlay). `render_right` delegates the real panel to `view::git`. |
| `git.rs` | All git rendering: the three bordered git boxes, and the four overlay renderers (commit/new-branch prompt, discard confirm, file picker, process form). |
| `theme.rs` | Shared styling: colors (incl. the `ATTN` accent + the `SPINNER` frames), `status_style`/`status_label`/`badge`, `agent_glyph_style` (the glyph + attention-aware name color for agent/terminal rows), `header`, `project_header`, and `entry_line` (the universal sidebar row). |

## Repository Layout (beyond `src/`)

| Path | What it is |
| --- | --- |
| `docs/` | This documentation — the canonical source of truth. |
| `README.md` | Short user-facing overview; points here. |
| `AGENTS.md` | Agent/contributor orientation + the documentation covenant. `CLAUDE.md` is a symlink to it. |
| `mmux.yaml` | mmux's own config (build/dev processes — open this repo *with* mmux). |
| `.github/` | The release workflow (tag → per-platform binaries → Homebrew tap bump) and the formula template. |
| `web/` | The `mmux.org` static marketing site (plain HTML/CSS/JS). Has its own `README.md` and `DESIGN.md`; not compiled into the binary. |
| `dist/` | Output of the "Build (all platforms)" process — distributable binaries. |
