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
| `config.rs` | The `mmux.yaml` + `~/.mmux/config.yaml` schema, load & merge (project over global; agents/processes by name), and `load_workspace` (root + linked projects, one level deep, path-deduped, capped at 8). Also the comment-preserving YAML editors used by the in-TUI forms — `append_process` / `append_linked_project` (sharing `splice_block_item`) and, for editing/deleting a process by name, `replace_process` / `remove_process` (sharing `named_item_span`) — plus `join_command` (inverse of `split_command`, to pre-fill the edit form) and `relative_path` for the link writer. Clean — touch sparingly. |
| `pane.rs` | One PTY-backed pane: `portable-pty` + a `vt100` parser on a reader thread, a writer thread draining an input channel, a reaper thread. A `Callbacks` impl captures the OSC title, bell, and notification OSCs (9/777/99); `contents_block` stitches scrollback text for copy; `cwd()` reads the child's live working directory (`/proc` on Linux, `proc_pidinfo` on macOS) so a `cd` survives a restore; `wheel_input`/`mouse_input` (sharing the `mouse_seq` encoder) translate the wheel and clicks/drags into the inner program's negotiated mouse protocol. Clean — touch sparingly. |
| `notify.rs` | Pure formatting of the desktop-notification escape (OSC 9/777/bell), the tmux passthrough wrapping, and the `command` fallback. Unit-tested. |
| `update.rs` | Background self-update for Homebrew installs: a cheap version check — first a local "did a sibling already upgrade the on-disk binary?" test, then curl the tap formula and compare versions (unit-tested) — that gates a background `brew upgrade`, both reporting over an `mpsc` channel; plus the in-place re-exec (`exec_restart`) that applies a staged update. Inert for non-brew/dev builds. |
| `agent.rs` | Hardcoded, no-config resume support for the two preset agents, detected by command basename: Claude (mmux *owns* the session id — `--session-id` / `--resume`) and Codex (`codex resume`). `sessions_for` re-derives an agent's *current* conversation id at save time from the tool's transcripts (newest for the cwd), so an in-agent `/resume`/`/new`/`/clear` is followed. Mints UUIDs without a crate. Unit-tested. |
| `restore.rs` | The per-workspace [restore-state](06-architecture.md#session-restore) file (`~/.mmux/state/<session-hash>.yaml`): the serde model (agents/terminals + their cwd and Claude/Codex session id) and best-effort save/load. Lets a reopen (quit/crash/update) bring everything back. |
| `clipboard.rs` | `copy(text)` — writes an OSC 52 escape to stdout **and** pipes to a local helper (`pbcopy`/`wl-copy`/`xclip`/`xsel`), with a hand-rolled base64 encoder. |
| `git.rs` | Stateless, synchronous wrappers over the `git` CLI for the native git panel: parse `status`/`log`/`branch`; stage/discard/commit/switch/pull/push/stash; `diff` one file for the preview; build the flattened changed-files tree. Errors come back as plain strings. |

## The TUI (`src/app/`)

One inherent `impl App`, split across files. `mod.rs` defines the struct; the others each add
`impl App { … }` for their concern.

| File | Responsibility |
| --- | --- |
| `mod.rs` | The `App` struct (all state) and the `Project` struct (per-project cfg / counters / git panel) + `new()` (which restores saved sessions on startup) + the `run()`/`run_loop()` event loop (a final state save as it exits, then the apply-update re-exec) + `tick()` (follow-active, git job drain, panel refresh, exited-agent/terminal pruning, self-update polling, restore-state save) + the `UpdateState` badge state machine + notification drain/emit + focus/resize helpers. |
| `session.rs` | The unified pane-backed model: `Session` / `Recipe` / `Kind` / `Status`. One `spawn()`/`stop()`/`kill()` lifecycle for agents, terminals, and processes; `spawn` appends any Claude/Codex resume flags from the optional `agent` slot. `Recipe` builders: `agent`/`process`/`shell`/`editor`. |
| `nav.rs` | The sidebar nav list (`build_nav`) + the positional selection cursor (`sel`) + resolvers: `project_of`/`current_nav`/`pane_at`/`move_sel`/`jump_project`/`focus_project`/`select_session`/`focused_pane`. |
| `lifecycle.rs` | Sidebar-driven actions: `spawn_agent` (tags Claude/Codex with a resume slot) / `spawn_terminal`, `open_new_process`/`finish_new_process` (append or, in edit mode, `replace_process`), `edit_selected`/`delete_selected`/`delete_process` (the process edit & confirm-delete flow), `open_picker`/`open_in_editor`, `open_link_browser`/`link_project` (grow the workspace with a linked project), `activate`, `do_start`/`do_stop`/`do_restart`, `close_session`/`prune_exited`, and `reload` (live config re-read & reconcile — a running process whose command changed is restarted). |
| `persist.rs` | [Session restore](06-architecture.md#session-restore): `save_state` snapshots the live agents/terminals (live cwd + `refresh_agent_ids`, which re-derives each agent's current conversation id) on structural change and as the loop exits; `restore_sessions` rebuilds them on startup (Claude/Codex resumed, shells at their cwd) and bumps the name counters. Backed by `restore.rs` / `agent.rs`. |
| `input.rs` | `on_key` (overlay routing → global `Ctrl+P` → focus dispatch), `key_sidebar`/`key_pane` (with the `Ctrl-b` leader)/`key_git`, `overlay_key`/`procform_key`/`linkbrowse_key`/`about_key`, `on_mouse` (click/drag/wheel routing), `forward_mouse` (hand the event to a mouse-tracking inner program unless Shift bypasses), `on_paste`, the drag-select-to-clipboard machinery, footer-button actions, and the `hit`/`button_code` helpers. `Selection`/`SelTarget` live here. |
| `keymap.rs` | `encode_key`: pure crossterm-key → PTY-byte translation. Unit-tested. |
| `git.rs` | The native git panel: `GitPanel` (refresh, cursor/section nav, stage/commit/discard/stash, backgrounded pull/push), the `Overlay`/`PromptKind`/`Confirmed` modal enums (`Overlay` includes the stateless `About` card), the `DiffView` main-pane diff preview (build/follow/scroll/`diff_upkeep`), and the `impl App` git-action methods. **Not** a pane. |
| `picker.rs` | The `Ctrl+P` fuzzy file picker: list files with the `ignore` crate (ripgrep's walking engine, in-process — no external `rg`; `.gitignore` deliberately not honoured, heavy build/artifact dirs pruned) and rank them. Fuzzy score is unit-tested. |
| `procform.rs` | State for the process guided form (Name → Command → Cwd → Review) with an autostart toggle and per-field validation. Shared by add (`ProcForm::new`) and edit (`ProcForm::edit`, pre-filled; `edit: Some(original_name)` routes the write to `replace_process` and exempts the entry from the duplicate-name check). |
| `linkbrowse.rs` | State for the `+ Link another project` directory browser: a fork-free filesystem walk (subdirs, filtered) with cheap per-dir previews (`.git`/`mmux.yaml` checks, branch from `.git/HEAD`), live filter + cursor, and the path the highlighted dir would be linked as. Applied by `lifecycle::link_project`. |

### View (`src/app/view/`)

| File | Responsibility |
| --- | --- |
| `mod.rs` | `draw()`: the responsive layout split, the per-frame `Regions` hit-rects, and the footer — `footer_segments()` returns a left + a right-aligned cluster of clickable shortcut chips (`FooterAction`/`Seg`, laid out by `layout_segs`), which in compact mode become the `menu`/`git`/`✕ close` toggle in the two bottom corners. `COMPACT_W` / `MAIN_MIN` live here. |
| `sidebar.rs` | `render_sidebar`: per-project boxes (or a single drawer), the `AGENTS`/`TERMINAL`/`PROCESSES` (+ compact `GIT`) sections, one styled row per nav entry via `nav_row()`, and the `+ Link another project` button reserved on the bottom inner row of a sidebar box (`reserve_link_row`). |
| `pane.rs` | `render_main` / `render_right` (share `render_screen`/`render_placeholder`), `main_title`, `placeholder_text`, the diff preview (`render_diff`/`diff_title`), the focused-pane cursor placement, and `paint_selection` (the drag-selection overlay). `render_right` delegates the real panel to `view::git`. |
| `git.rs` | All git rendering: the three bordered git boxes, the five overlay renderers (commit/new-branch prompt, discard confirm, file picker, process form, link-project browser), and `render_about` (the About card, drawn from `draw` so it can read live update state). |
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
