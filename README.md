# mmux

A persistent, per-directory terminal multiplexer for AI coding agents and dev processes.

Type `mmux` in a directory and you get a TUI with a left sidebar split into **Agents**
(Claude, Codex, … spawned on demand) and **Processes** (servers, ansible, stripe, …
that you start/stop and watch), plus an optional persistent **right panel** (e.g. lazygit).
The whole thing runs inside an invisible tmux session keyed to that directory, so:

- there is exactly **one** mmux per directory, and
- it keeps running after you close the terminal / drop SSH — run `mmux` again to reattach.

Agent sidebar entries show a **subtitle** (the terminal title the program sets, e.g. what
Claude is doing) and a red **●** when the program rings the bell (needs attention).

## Install

Requires [Rust](https://rustup.rs) and `tmux` on your PATH (and `lazygit` if you use the
default right panel).

```sh
cargo install --path .
```

Or build and drop the binary somewhere on your PATH yourself. On Apple Silicon, **re-sign
after copying** — macOS kills a freshly `cp`'d ad-hoc-signed binary with "Killed: 9":

```sh
cargo build --release
cp target/release/mmux ~/.local/bin/mmux
codesign --force --sign - ~/.local/bin/mmux   # macOS only; avoids "Killed: 9"
```

## Use

```sh
cd ~/some/project
mmux init      # writes a starter mmux.yaml (optional if you have a global config)
mmux           # open / reattach

mmux a         # or `mmux attach`: pick ANY running mmux session and join it
```

### Keys

Sidebar: `↑/↓` or `j/k` move · `[`/`]` switch project (with [linked projects](#linked-projects)) ·
`Enter` open · `s` start · `x` stop · `r` restart · `R` reload config ·
`Tab` jump to the right panel · `d` detach · `q` quit.

`R` re-reads `mmux.yaml` (+ the global config) live: newly added processes/agents appear
and edited commands take effect on the next start, all without losing running panes.

Terminal / panel pane: keys go to the program. `Ctrl-b` is the leader — `Ctrl-b h` back to
sidebar, `Ctrl-b d` detach, `Ctrl-b x` stop, `Ctrl-b R` reload config, `Ctrl-b b` send a
literal `Ctrl-b`.

Mouse: **single-click** a running agent/process to jump straight into it; **double-click** a
`+ New …` launcher to spawn it; click the right panel to focus it. The **scroll wheel**
scrolls the pane under the cursor through its scrollback.

### Narrow terminals / phone

When the terminal is narrow (e.g. SSH from a phone) mmux switches to a single-column layout:
one region fills the screen and the title bar carries tap targets — **☰** (top-left) opens the
sidebar drawer, and a **`name ☰`** button (top-right) opens the panel (lazygit). Pick something
in the drawer to view it full-screen; tap **☰** to get back. Everything stays reachable by
keyboard too (`Ctrl-b h` returns to the drawer).

## Config

Two layers, merged at load time — a global `~/.mmux/config.yaml` with each project's
`mmux.yaml` on top. Project values win; `agents` and `processes` merge **by name**;
`right_panel` and `name` are overridden if the project sets them. Relative `cwd`s always
resolve against the project directory, so global definitions run in whatever project you're in.

`~/.mmux/config.yaml` (global defaults):

```yaml
agents:
  - name: Claude
    cmd: claude
    args: ["--dangerously-skip-permissions"]
  - name: Codex
    cmd: codex
    args: ["--dangerously-bypass-approvals-and-sandbox"]

right_panel:
  cmd: lazygit
  title: lazygit
  width: 44
```

`mmux.yaml` (per project — only the project-specific bits):

```yaml
# name is optional; it defaults to the directory name.
name: my-workspace

processes:
  - name: Dev server
    cmd: npm
    args: ["run", "dev"]
    cwd: .            # relative to this file
    autostart: false
```

`mmux check` prints the effective merged config without launching anything.

### Linked projects

Working in several clones of a repo at once (`./app`, `../app2`, `../app3` — the classic
"clones instead of worktrees" setup)? List the siblings under `linked-projects` and they all
open in **one** mmux, each as its own group in the sidebar:

```yaml
# in ./app/mmux.yaml
linked-projects:
  - ../app2
  - ../app3
```

Switch between projects with `[` and `]`. The right panel **follows the active project** — when
you're on `app2`'s rows, the panel shows `app2`'s lazygit — so each clone gets its own panel,
kept alive in the background so switching back is instant.

It's loaded **one level deep** and **de-duplicated by path**, so you can drop the *same* config
into every clone (even one that lists itself) and it will **never expand recursively** — at most
8 projects load; a missing or unreadable sibling is skipped with a warning. The set of projects
is fixed when mmux opens, so adding/removing a link takes effect on the next `mmux`, not on `R`
reload (which refreshes each loaded project in place). The launch directory is always the first
group, so opening mmux from any clone keeps "where you are" on top.

## Notifications

When a session rings the bell — or emits a desktop-notification escape of its own (e.g.
Claude Code announcing it's done) — mmux raises a **native desktop notification**. On by default.

The trick that makes it work everywhere: the notification is sent as a terminal escape
sequence to your terminal emulator, which renders the popup. Because that's just bytes on
the normal output stream, it works **over SSH too** — the popup lands on whatever machine
your terminal runs on, not the remote box mmux lives on. (mmux's tmux jail is configured with
`allow-passthrough on` so the escape reaches the outer terminal.)

```yaml
notifications:
  enabled: true
  mechanism: osc9          # how to deliver the popup (see below)
  only_when_unfocused: true # don't notify the pane you're already looking at
  throttle_secs: 5         # min seconds between popups from the same session
  # command: 'terminal-notifier -title "$MMUX_NOTIFY_TITLE" -message "$MMUX_NOTIFY_BODY"'
```

Mechanisms — `osc9` is the default because it has the widest reach; switch if your
terminal isn't covered:

| `mechanism` | Escape          | Terminals (all work over SSH)               |
|-------------|-----------------|---------------------------------------------|
| `osc9`      | `OSC 9`         | iTerm2, kitty, ghostty, WezTerm (default)   |
| `osc777`    | `OSC 777`       | ghostty, foot, WezTerm, urxvt/VTE           |
| `bell`      | `BEL`           | anything that notifies on bell              |
| `command`   | runs a command  | local only — can't cross an SSH hop         |

`osc9` carries a single message (no separate title), so mmux folds the session name in
(`Claude — needs attention`). iTerm2 and kitty only understand `osc9`; `osc777` adds a
bold title on terminals that support it.

For `mechanism: command`, the command runs via the shell with the notification in
`$MMUX_NOTIFY_TITLE` / `$MMUX_NOTIFY_BODY`; it defaults to `osascript` (macOS) or
`notify-send` (Linux) when `command` is unset.

## Architecture

The binary has two halves: a thin CLI/tmux outer shell, and the `app/` TUI that runs inside it.
See `AGENTS.md` for the full module map and conventions.

- **`main.rs` / `cli.rs`** — argument dispatch and the non-TUI subcommands (`init`, `check`,
  `--help`, `--version`).
- **`tmux.rs`** — all tmux interaction: the `mmux` no-arg entry point (hash the canonical dir
  into a `mmux-<hash>` session name, attach-or-create, configure it invisible, launch
  `mmux --inner`), the `mmux attach` picker, and `detach`.
- **`config.rs`** — YAML config + the global/project merge.
- **`pane.rs`** — one PTY-backed pane via `portable-pty`, output parsed by `vt100`. A
  `Callbacks` impl captures the OSC title, bell, and notification OSCs.
- **`notify.rs`** — builds the desktop-notification escape sequence (and the tmux passthrough
  wrapping) for the configured mechanism; unit-tested.
- **`app/`** — the ratatui TUI, split by concern: `session` (the unified pane-backed model +
  lifecycle), `nav` (sidebar list + selection), `lifecycle` (start/stop/restart + live
  reload), `input` (keyboard/mouse/paste), `keymap` (key→PTY bytes, unit-tested), and
  `view/` (layout, sidebar, panes, footer). `app/mod.rs` owns the `App` state and event loop.

### Known limits (v1)

- Persistence covers detach / disconnect (tmux keeps the TUI alive). A crash of the TUI
  process or a reboot still loses live panes — the planned upgrade is a daemon/client split.
- Attention detection keys off the terminal bell (what Claude Code emits via
  `preferredNotifChannel terminal_bell`) and notification OSCs the program emits itself
  (OSC 9/777/99) — both drive the sidebar dot and a desktop notification (see above). An
  idle "agent went quiet" heuristic is still a future addition.
- Mouse events inside a focused pane are not forwarded to the program yet (so lazygit's mouse
  won't work — use the keyboard); clicks/wheel are used by mmux for focus and scrollback.
- Scrollback is wheel-scrollable (vt100 keeps 5000 lines). To copy, **drag with the mouse**
  across a pane — the highlighted text is sent to your clipboard (via OSC 52, which also works
  over SSH, plus a local `pbcopy`/`xclip` fallback). A keyboard copy-mode is not there yet.
- Sibling directories share one sidebar via [`linked-projects`](#linked-projects), but the set
  is fixed at launch (changing it needs a reopen) and grouping is a flat list — named, nestable
  workspaces are still a future addition.
