# Architecture

This is the conceptual map of the codebase, for anyone — human or agent — working **on** mmux.
For a file-by-file index, see the [Module Map](07-module-map.md). For conventions and the
documentation covenant, see [Contributing](08-contributing.md).

mmux has two halves: a thin **CLI / tmux outer shell**, and the **`app/` TUI** that runs inside
it.

## The Two Halves and the Inner/Outer Split

The same binary plays two roles, distinguished by the `MMUX_INNER` environment variable (or the
`--inner` flag):

- **Outer process** — the bare `mmux` you type. It does no TUI work: it finds (or creates) the
  per-directory tmux session and attaches to it. See `cli.rs` → `tmux.rs`.
- **Inner process** — launched *by tmux* with `MMUX_INNER=1` set. It reads the workspace config
  and runs the ratatui TUI (`app::run`). This is what you actually see.

```text
mmux                      cli::run() dispatch
  │
  ├─ init/check/docs/      → wizard / config validation / printed guide
  │  attach
  │
  └─ (no subcommand)       → tmux::launch()
        │
        ├─ outer: tmux new-session -d … mmux --inner   (then attach-session)
        │
        └─ inner (MMUX_INNER set): Config::load_workspace → app::run(ws)
```

### The tmux Jail

The TUI runs inside an invisible, per-directory tmux session. This is what makes mmux persistent
and singleton-per-directory:

- **Session name** = a hash of the directory's *canonical* path, formatted `mmux-{:016x}`. Hex
  avoids tmux's illegal `.`/`:` characters; canonicalizing first means `dir`, `dir/`, and
  symlinks all map to the same session.
- **Singleton.** A second `mmux` in the same directory computes the same name and attaches
  instead of creating. If creation loses a race, mmux falls through to attaching to the winner.
- **Invisible & non-interfering.** `configure_session` sets options on *that session only*
  (never `-g`): status bar off, no tmux prefix (mmux owns its own `Ctrl-b` leader), `mouse on`
  (so tmux enables outer-terminal mouse reporting and forwards events to the TUI, which sets its
  own mouse mode — `off` silently drops wheel/clicks when attached over SSH), `destroy-unattached
  off` (survives detach),
  `allow-passthrough on` (lets [notification](05-notifications.md) escapes reach the outer
  terminal), and `set-clipboard on` (lets OSC 52 copies through). The outer terminal's tab title
  is set to the project name.
- Attaching runs with `TMUX` unset, so mmux works even when launched inside another tmux.

`mmux attach` is a separate path: it lists running `mmux-*` sessions plus recent directories
(from `~/.mmux/history`) in a small picker — with an always-on, type-to-fuzzy-filter search bar
(reusing the file picker's scorer) — and attaches to or launches the chosen one.

## The Event Loop

`app::run` puts the terminal into raw mode + alternate screen + mouse capture + bracketed paste,
then `run_loop` cycles:

1. **draw** the frame (`App::draw`), which also writes the per-frame mouse hit-rects into
   `App.regions`;
2. **collect notifications** — drain every pane's captured events and write the resulting escape
   bytes straight to stdout (they're non-painting, so this is safe after the frame);
3. break if `should_quit`;
4. **poll** events for 50 ms (so live program output keeps redrawing at ~20 fps even with no
   input), dispatching `Key`/`Mouse`/`Paste`;
5. **tick** — housekeeping (below).

`tick()` runs every loop: it steps drag auto-scroll, prunes exited agent/terminal rows, makes the
selected row's project the **active** one (see [follow-active](#workspaces-and-projects)), drains
finished background git jobs from every project's panel, gives the *visible* git panel a throttled
refresh, and advances the [self-updater](#self-update) (drains its worker channel, runs the
periodic re-check).

## The Unified Session Model

This is the core abstraction. An agent, a terminal, and a process are the same thing under the
hood: a `Recipe` (cmd/args/cwd/env) plus an optional live `Pane`. They live in **one** flat
`Vec<Session>` on `App`, distinguished only by a `Kind` tag and a `project` index.

```rust
enum Kind { Agent, Terminal, Process }   // exactly three — there is no Panel kind

struct Session {
    name, kind, project,        // identity + which sidebar bucket / project
    recipe,                     // everything needed to (re)spawn identically
    pane: Option<Pane>,         // the live PTY-backed terminal, if running
    error: Option<String>,
}
```

- **`Kind` drives presentation only** — sidebar grouping, the status badge, placeholder wording —
  **never the lifecycle.** Filter the one `Vec` by `project` + `Kind` to build each sidebar group.
- **One lifecycle for all three:**
  - `spawn(rows, cols)` kills any existing pane and spawns the recipe fresh — it is **both start
    and restart**; callers decide *when*.
  - `stop()` kills the process but keeps the now-exited pane (so it reads as `Status::Exited`).
  - `kill()` kills and drops the pane entirely (back to `Status::Stopped`).
- A config-defined **process** may carry an optional `stop:` **teardown** command
  (`Session.stop`, e.g. `docker compose down`) — a shell line run in its directory *after it
  stops*: on a manual `x` and, for every still-running process, at quit (`run_stop_commands_on_quit`
  in [`lifecycle.rs`](07-module-map.md), called from `run()`). It never runs on a restart (the
  process is coming right back). A manual stop backgrounds it; quit waits for it, so the teardown
  finishes before the inner tmux session — and mmux — go away. This is teardown *around* the
  lifecycle, not a fourth lifecycle method.
- `Status` (`Stopped`/`Running`/`Exited`/`Failed`) is derived from the `Option<Pane>` and whether
  the process is alive — it is not stored. `Failed` means it exited non-zero *on its own* (a
  deliberate `stop()`/`kill()` is flagged on the pane so it stays `Exited`, not `Failed`); it
  paints the row red for every kind — a process keeps its red badge, and a crashed agent/terminal
  stays put as a red `○` rather than being pruned.
- **Agents and terminals don't linger once they exit cleanly.** `tick()` calls `prune_exited` (in
  `lifecycle.rs`), which drops any agent/terminal that reached `Status::Exited` — so quitting an
  agent from inside it (`/quit`, Ctrl-D) or `exit`ing a terminal makes its sidebar row vanish
  rather than leaving an "exited" husk. A **crash** (`Status::Failed`) is deliberately kept and
  painted red so you notice it. Only **processes** keep their row on a clean stop too, because
  they're config-defined and meant to be restarted in place.

> Do **not** re-introduce per-kind collections or per-kind spawn/stop methods. That triplication
> was deliberately removed; the unified model is the point.

A `Pane` (in `pane.rs`) owns one PTY via `portable-pty`, parsed by `vt100` on a reader thread,
with a writer thread draining an input channel and a reaper thread waiting on the child. A
`Callbacks` impl captures the OSC window title (the sidebar subtitle), the bell, and notification
OSCs. Scrollback is 5000 lines.

## Workspaces and Projects

`App.projects: Vec<Project>` holds the launch directory (`projects[0]`) plus any
[linked projects](04-configuration.md#linked-projects). Each `Project` owns:

- its merged `cfg`;
- per-agent-template instance `counts` and a `term_count` (for `Claude #3`, `Terminal #2`
  naming);
- **its own** git panel: `git: Option<GitPanel>`.

`App.active` is the selected row's project. **Follow-active:** `tick()` keeps `active` in sync
with the selection, so the visible git panel always belongs to the project you're working in; the
others stay alive in the background. `last_proj_sel` remembers each project's last-selected row so
`[`/`]` and clicking restore where you were. A single-project workspace renders exactly as it
always did — no project header.

`load_workspace` (in `config.rs`) builds the workspace: load the root, then each linked project
one level deep, de-duplicated by canonical path, capped at 8, with each linked project's *own*
`linked-projects` cleared so links never chain. Failures become warnings, not errors. The same cap
and de-dup gate `link_project`, which appends a `Project` (and its process rows) to grow the
workspace live — every other path leaves the project set fixed for the session.

## The Git Panel and Overlays

The right column is a **native git panel** — mmux's own UI, not an embedded program. It is **not**
a `Session` and has no PTY: it is `Project.git: Option<GitPanel>`, created once in `Project::new`
when [`git-panel`](04-configuration.md#git-panel) is enabled and the directory is a repo. It draws
three boxes (Changes / Branches / Commits) and is driven by its own keymap — `Tab` cycles the
cursor through all three.

- **`git.rs`** (top level) is a stateless layer of synchronous shell-outs to the `git` CLI —
  status, log, branches, `show`, stage/discard/commit/switch/pull/push, and the commit ops
  (`revert`, soft `reset`, `commit_message`) — returning data or git's stderr as a plain string.
  Nothing is cached.
- **`app/git.rs`** is the `GitPanel` state machine plus the `impl App` git-action methods. It
  refreshes on a 1500 ms throttle while visible (and immediately after any mutation). Network ops
  (pull/push) block, so they run on a throwaway thread and report back over an `mpsc` channel
  drained in `tick()`. The centre-pane `DiffView` pager serves both a live working-file preview
  (follows the Changes cursor, self-refreshes) and a static commit diff (`git show`, rendered with
  per-file dividers, chosen in the Commits box).

`GitPanel` also hosts the **`Overlay`** enum — full-screen modals that eat every key while open:

| Overlay | Raised by | Effect |
| --- | --- | --- |
| `Prompt` | `c` / `n` in the panel | Commit message / new-branch name |
| `Confirm` | `d` in the panel · `t` / `u` on a commit · `D` on a process · `q` | Yes/no guard: destructive discard · revert / uncommit a commit · delete a process · quit |
| `Picker` | `Ctrl+P` anywhere | [Fuzzy file picker](03-usage.md#the-file-picker) → opens a file in an editor pane |
| `NewProcess` | `+ New Process` / `e` on a process | [Guided form](03-usage.md#adding-editing-and-deleting-a-process) → appends to (or edits in place) `mmux.yaml` |
| `LinkProject` | `+ Link another project` (its own sidebar box) | [Directory browser](03-usage.md#linking-another-project) → appends to `linked-projects` and grows the live workspace |
| `Agents` | `a` in the sidebar | [Agent manager](04-configuration.md#agent) → toggle the built-in harnesses on/off + danger mode, rewrite the **global** `agents:` block, and reload |
| `About` | `?` in the sidebar | Version, project links, and the manual self-update check/apply (stateless — reads live `UpdateState`) |

The picker (`picker.rs`) lists files with the `ignore` crate (ripgrep's walking engine, in-process
— so no external `rg` is needed; `.gitignore` is deliberately *not* honoured and heavy
build/artifact dirs are pruned instead) and fuzzy-ranks them. The process form (`procform.rs`) collects fields step-by-step, then
`finish_new_process` writes the entry into `mmux.yaml` via `config::append_process` (new) or
`config::replace_process` (an edit, keyed by the original name) — both raw-text edits that preserve
comments — and reloads; `D` deletes via `config::remove_process` behind a confirmation. The link browser (`linkbrowse.rs`) walks the filesystem
with fork-free previews (repo-ness is a `.git` check, the branch is read from `.git/HEAD`), then
`link_project` appends the path via `config::append_linked_project` (the same raw-text splicer) and
**adds a new `Project` in place** — the one action that grows the project set after launch. The
agent manager (`agentmgr.rs`) seeds its rows from `config::PRESETS` and the current global agents,
then `apply_agent_manager` writes the whole set back with `config::write_agents` — which, unlike the
per-item process splicers, **replaces the entire global `agents:` block** (the manager owns the full
list) while preserving the rest of the file — and reloads so the sidebar updates live.

### The Diff Preview

Clicking a changed file (or `v`) opens `App.diff: Option<DiffView>` — a parsed `git diff` of the
file under the Changes cursor, rendered in the **main pane** as a read-only pager instead of the
selected session. It is **not** a `Session` (no PTY) and **not** an overlay (it doesn't eat keys
globally); it's a third main-pane mode alongside "live pane" and "placeholder", checked first in
`render_main`. It is a *live preview*: `git_preview_follow` rebuilds it as the Changes cursor
moves, and `App::diff_upkeep` (in `tick`) re-reads it on the panel's throttle and drops it when
its file stops being changed, its project is no longer active, or a session is selected. The diff
is `HEAD` vs the working tree (staged + unstaged together); an untracked file is diffed against
`/dev/null` so it shows all-added.

When the changed file is an image (`png`/`jpg`/`gif`/`webp`/`bmp`), `DiffView` carries a decoded
`PreviewImage` instead of text lines, and the pane shows the picture. There are two render paths:

- **Sixel (real pixels), the primary path.** mmux draws its whole UI *through* the invisible tmux
  session; tmux 3.4+ (built with sixel, as Homebrew's is) *renders sixel natively* for terminals
  that support it, so we lean on that instead of fighting a graphics protocol. At startup
  `tmux::client_supports_sixel` asks tmux (`#{client_termfeatures}`) whether the attached terminal
  does sixel — a reliable gate, since tmux only advertises it when it will faithfully render what
  we emit (overridable via `MMUX_SIXEL`). When on, `render_main` doesn't draw the image into the
  ratatui buffer at all: it stashes an `icy_sixel`-encoded string (sized to the pane via the
  terminal's pixel-per-cell, cached per size) in `App.pending_sixel`, and `run_loop`'s `emit_sixel`
  writes it on top of the just-drawn frame — the same after-draw escape channel the notifications
  use. Because tmux diffs its own screen, the sixel is only re-emitted when the picture or its
  placement *changes* (`last_sixel`); leaving the picture forces one full repaint so tmux clears
  the leftover pixels a plain cell diff wouldn't touch.
- **Half-blocks, the fallback.** Where sixel isn't available, `view::pane::render_image` paints the
  image as coloured `▀` cells (top pixel → foreground, bottom → background) straight into the
  buffer — just styled text, so it survives tmux on any terminal (truecolor degrading to 256-colour
  as needed), at the cost of coarse resolution.

Either way the source is decoded once (bounded by a file-size cap and `image::Limits`, since it
runs on the UI thread) and re-encoded/re-rasterized only when the pane's cell size changes;
`diff_upkeep` skips the throttled re-decode for images.

## Self-Update

`update.rs` keeps mmux current without interrupting your work, and `restore.rs` +
`app/persist.rs` make *applying* an update feel like nothing happened. It serves the two managed
install paths — Homebrew and the `mmux.org/install.sh` native binary — from one flow that splits
cleanly along the inner/outer grain:

- **One version check for both.** A check runs at startup and every 6 hours thereafter (timed from
  each session's startup, so independent sessions stagger their checks). It first does a cheap
  *local* test — run the on-disk binary (the `resolve_exe` path) with `--version` and compare to
  ours — so when a **sibling session already upgraded** the binary while we keep running the old
  code, we jump straight to "ready" without touching the network. Otherwise it follows the GitHub
  `releases/latest` redirect and reads the version off the resulting `…/releases/tag/vX.Y.Z` URL —
  a plain web redirect, so no REST API rate limit and no token. (The release tag and the tap
  formula version are bumped from the same CI run, so one source serves both kinds.)
- **Staging depends on the install kind** (`install_kind` classifies it in the worker):
  - **Self-managed** (a stamped release binary — `MMUX_RELEASE` is baked in by CI — in a
    user-writable dir): mmux downloads the tarball for its target, extracts it into a temp dir
    *beside* the live binary, re-signs it on macOS, and `rename`s it over the running file. The
    rename is atomic (same filesystem) and safe while running (the kernel keeps the old inode). It
    goes straight to `UpdateState::Ready`, silently.
  - **Homebrew** (`is_brew_managed`: the exe lives under `$(brew --prefix)`): mmux can't swap a
    Cellar binary without desyncing brew's bookkeeping, so a found update parks in
    `UpdateState::Available` and waits. Applying it opens a confirm that runs `brew upgrade mmux`
    for the user (letting brew's implicit auto-update find the freshly-pushed formula), then lands
    on `Ready`.

  Everything runs on throwaway threads reporting over an `mpsc` channel drained in `tick()` — the
  same shape as the git panel's pull/push jobs.
- **Applying is user-gated.** Running the *new* code means replacing the inner process, which
  necessarily ends the live panes. So `UpdateState::Ready` only lights a quiet bottom-right footer
  badge; pressing `U` or clicking it sets `App.restart`, and `run()` — after restoring the terminal
  — `exec`s the freshly-installed binary **in place**. `resolve_exe` picks the target: the stable
  `brew --prefix` symlink for a brew install (since `current_exe()` may point into a pruned Cellar
  dir), else `current_exe()` — the path we just swapped the new binary onto. Same PID, so tmux
  keeps the pane and the new TUI just redraws; `MMUX_INNER`/`MMUX_DIR` are inherited through the
  exec. The ended panes come back on startup like any other reopen — see
  [Session Restore](#session-restore) — so applying an update isn't disruptive.

The updater is inert for unmanaged installs — source builds (no `MMUX_RELEASE` stamp), binaries in
a dir the user can't write, and dev builds — and is gated by `auto-update` config + the
`MMUX_NO_UPDATE` env var. Because classifying the install needs subprocesses/filesystem probes
(`brew --prefix`, a write test), it's done in the worker, not the synchronous `permitted` gate — so
a permitted but unmanaged build optimistically enters `Checking`, and the worker reports back
`NotManaged` to settle it into the terminal `UpdateState::Unsupported` (no badge; the About card
reads "self-update off"). See [Configuration → Auto-Update](04-configuration.md#auto-update).

## Session Restore

mmux remembers the agents and terminals you had open and brings them back when you reopen a
directory — after a quit, a crash, or a [self-update](#self-update) restart. `app/persist.rs`,
`restore.rs`, and `agent.rs` implement it.

- **Save.** `app/persist.rs` snapshots the live agents/terminals to
  `~/.mmux/state/<session-hash>.yaml` (keyed by the same canonical-dir hash tmux uses, via
  `tmux::session_name`). It writes on every structural change (a cheap fingerprint in `tick()`
  gates the write) and once more from `run()` as the loop exits, with each pane's **freshest** cwd.
  Each save also resolves every agent's resume id (`refresh_agent_ids`): Claude ids are minted by
  mmux and left untouched, while a fresh Codex agent discovers the id of the session it just created
  (`agent::sessions_for`). Only agents/terminals are saved — processes come back from config.
- **Restore.** `App::new` always calls `restore_sessions` on startup; it's a no-op when there's no
  file. This is safe to do unconditionally because the **tmux singleton** means a fresh inner
  process only ever starts when there's no live session to attach to (a detach leaves the inner
  process — and its panes — running, so reattaching never reaches this path). So there are never
  live panes to clobber.
  - **Claude / Codex agents resume their conversation.** `agent.rs` is a hardcoded, no-config
    adapter detected purely by command basename. Claude lets mmux *own* the id (launch with
    `--session-id <uuid>`, reattach with `--resume <uuid>`), so several `Claude #N` in one
    directory each resume their **own** thread — the id is authoritative and is never reassigned
    between agents, so idle agents can't be shuffled onto a recently-active sibling's session.
    Codex has no such flag, so mmux launches it plain, **discovers** the session it created
    (`agent::sessions_for` — the newest transcript for that cwd no sibling has already claimed),
    and reattaches with `codex resume <uuid>`.
  - **Terminals reopen at their live cwd.** `Pane::cwd()` reads the shell's working directory from
    the OS (`/proc/<pid>/cwd` on Linux, `proc_pidinfo` on macOS), so a `cd` survives — though as a
    fresh shell (no history/env/jobs). Editor panes reopen their file the same way.
  - **Processes are left alone** — they're config-defined, so autostart brings them back and the
    rest are a click away. They're never written to the state file.

This is a convenience, never load-bearing: a missing or unparsable state file just means a fresh
start, so every read/write swallows its errors. Closing a session (or all of them) before quitting
removes it from the snapshot, so it's easy to get a clean slate.

## Navigation, Focus, and Regions

- **Navigation is positional.** `App.sel` is an index into the `Vec<Nav>` returned by
  `build_nav()`, rebuilt on demand. `Nav` is one row in display order: the launchers
  (`NewAgent`/`NewTerminal`/`NewProcess`, each carrying its project), `Session(i)` (an index into
  the flat sessions vec), `Panel` (the git panel — listed in nav *only* in compact mode), and
  `Link` (the standalone "+ Link another project" box, always last — it belongs to no project and
  grows the root). Any code that mutates `sessions` must re-clamp `sel` against the freshly built nav. This is the
  one place that is deliberately not yet ideal — see [Planned](08-contributing.md#planned-and-known-limits).
- **Focus** (`Sidebar` / `Terminal` / `Right`) decides which region gets keys. `focused_pane()`
  resolves it — and returns `None` for `Right`, because the git panel is native UI with no PTY to
  forward keys to.
- **Regions** (`view/mod.rs`) is per-frame mouse geometry: rendering writes the rects, input reads
  them, and it's reset at the top of every `draw()`. If you add a clickable area, set its rect
  during render and test it in `on_mouse`.

## Data Flow Summary

- **Output:** program → PTY → reader thread → vt100 parser → `Pane` (title + when it last changed,
  bell, notifications via `Callbacks`). The app reads it through
  `Session::subtitle/attention/working/take_notifications` (`working` keys off the title-change
  time so a quiet agent reads as "needs you"). `Session::busy` (`working` over a fixed ~2s window)
  is the single "is this agent actively working" predicate — it's what both spins the sidebar glyph
  and gates the close-confirmation, so the prompt fires for exactly the agents that show a spinner.
- **Input:** key → `on_key` (overlay first, then global `Ctrl+P`, then the global `Ctrl-b` leader
  via `leader_command`, then by focus). In a pane, `keymap::encode_key` translates the key to PTY
  bytes and `Pane::send` queues them.
- **Notifications:** captured pane events → `collect_notifications` → `notify.rs` builds the OSC
  escape (wrapped in tmux passthrough when inside tmux) → written to stdout → the outer terminal
  renders the popup. See [Notifications](05-notifications.md).
- **Mouse forwarding:** over the main pane, `App::forward_mouse` asks `Pane::mouse_input` to encode
  the click/drag/release/motion as an xterm report **iff** the program negotiated a matching mouse
  mode (and the event is reportable under it) — then sends it to the PTY instead of running mmux's
  own routing. Shift held bypasses this (so drag-to-copy still works), as do the git panel and a
  diff pager. This is what lets micro/vim/… place the cursor on a click.
- **Copy:** a non-forwarded mouse drag → a `Selection` in buffer coordinates → on release
  `Pane::contents_block` stitches the text across scrollback → `clipboard::copy` (OSC 52 + a local
  helper).
- **Wheel:** over the normal screen it drives our own scrollback; over a program on the alternate
  screen (which has none) `Pane::wheel_input` hands the notch to the program — a forwarded
  mouse-wheel event if it tracks the mouse, else synthesized arrow keys ("alternate scroll").
- **Links:** a plain (non-drag) click on a URL opens it in the browser (`open.rs`). In a pane
  `url_under` reads the clicked row off the vt100 screen and `url_at` extracts the link token; on
  mmux's own [About card](03-usage.md#opening-links) the rendered link spans are registered as
  `Regions::links` hitboxes (the only clicks a modal doesn't swallow). A drag is still a copy, so
  you can select a link instead of following it.

## Why It's Built This Way

| Decision | Rationale |
| --- | --- |
| Inner/outer split via tmux | One binary; tmux gives free persistence across detach/disconnect/SSH and a true per-directory singleton. |
| Session name = hash of canonical path | Deterministic, tmux-safe, and collapses `dir`/`dir/`/symlinks to one session. |
| One unified `Session` model | Agents, terminals, and processes differ only in presentation; unifying them removed three-way triplication of spawn/stop/collections. |
| Notifications as terminal escapes | The same code path works locally and over SSH — the popup renders wherever the terminal runs, not where mmux lives. |
| Native git panel (not embedded lazygit) | A panel mmux draws itself integrates with the layout, follows the active project, and needs no external dependency. |
| Positional `sel` confined to `nav.rs` | Keeps the planned move to selection-by-identity a single-file change. |
| Self-update: auto install, user-gated restart | The on-disk swap is safe mid-run, but applying it ends the panes — so the disruptive step waits for you, behind a quiet badge, while a long task runs undisturbed. |
| Restore agents/terminals on every reopen | Snapshot the live sessions and rebuild on start — resuming Claude/Codex by session id and shells at their live cwd — so quitting, a crash, or a "restart to update" all bring your work back. Unconditional because the tmux singleton guarantees a fresh inner process means no live panes to clobber. A throwaway state file, never load-bearing. |

## Planned

The v1 architecture has known limits. Persistence now covers detach/disconnect *and* a
quit/crash/update reopen via [Session Restore](#session-restore) — but restore is a cold respawn
(the conversation/cwd come back, not the live process or its in-flight work; a daemon would fix
that). Selection is positional; a linked project can be added live but only *removed* by a reopen.
These, and the planned daemon/client split, are tracked in
[Contributing → Planned and Known Limits](08-contributing.md#planned-and-known-limits).
