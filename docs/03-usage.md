# Usage

This is the complete guide to the mmux interface: the layout, every keybinding, mouse support,
and the git panel. For how to configure what appears, see [Configuration](04-configuration.md).

## The Interface

```text
┌─ project ──────┬───────────────────────────┬─ git ──────────┐
│ AGENTS         │  claude — running         │ Changes        │
│ ▌ claude #1    │                           │  [✓] src/      │
│   + New Claude  │  (the focused program's  │  [ ] README.md │
│ TERMINAL       │   live terminal screen)   │ Branches       │
│   + New Terminal│                          │ * main         │
│ PROCESSES      │                           │ Recent         │
│ ● Dev server   │                           │  a1b2c3 fix …  │
│   + New Process │                          │                │
└────────────────┴───────────────────────────┴────────────────┘
 ↑↓ move   ⏎ open   s start   x close   r restart   d detach   q quit
```

- **Sidebar (left).** One group of sections per project: `AGENTS`, `TERMINAL`, and `PROCESSES`
  (the headings are uppercase; `TERMINAL` is singular). Each section lists its running sessions
  plus a green `+ New …` launcher row. The selected row is marked with a `▌` bar and a
  highlight.
- **Main pane (center).** The live terminal of whatever is selected. Its title bar reads
  ` <name> — <status> ` and turns its border magenta when focused.
- **Git panel (right).** Appears automatically when the directory is a git repository. See
  [The Git Panel](#the-git-panel).
- **Footer (bottom).** Context-dependent key hints rendered as **clickable** shortcut chips.

### Status and Attention

- **Process rows** show a leading status glyph: `●` running, `○` exited, `·` stopped. Agents and
  terminals convey status by text color instead (green = running, dim = exited/stopped).
- Every session row shows a dim **subtitle** — the terminal title the program sets (e.g. what
  Claude is currently doing), falling back to its last error.
- A red **`●`** attention dot appears when a session rings the bell or emits a notification
  escape. It clears when you focus that session; for agents and terminals it is also suppressed
  on the pane you are actively viewing. The same event raises a
  [desktop notification](05-notifications.md).

## Focus

At any moment one region has keyboard focus: the **sidebar**, the **main pane** (a focused
program receives your keystrokes), or the **git panel**. The keys below are grouped by which
region they apply to.

## Sidebar Keys

| Key | Action |
| --- | --- |
| `↑` / `k` · `↓` / `j` | Move the selection up / down |
| `g` · `G` | Jump to the first / last row |
| `Enter` · `l` · `→` | Open the selected row (see below) |
| `s` | Start: spawn a launcher, or start a stopped session |
| `x` | Close: **removes** an agent/terminal row entirely; **stops** a process (row stays) |
| `r` | Restart the selected session (or spawn a launcher) |
| `R` | [Reload config](04-configuration.md#live-reload) live |
| `[` · `]` | Switch to the previous / next project ([linked projects](04-configuration.md#linked-projects); no-op with one project) |
| `Tab` | Jump to the git panel (or into the selected pane if there is no panel) |
| `d` | Detach (the session keeps running in the background) |
| `q` | Quit mmux |
| `Ctrl+P` | Open the [fuzzy file picker](#the-file-picker) (works from any focus) |

**Opening a row** with `Enter`/`l`/`→` does the right thing for its kind: a `+ New Agent`/
`+ New Terminal` launcher spawns and jumps into a fresh pane; `+ New Process` opens the
[guided form](#adding-a-process); a stopped session is (re)started and focused; the git-panel row
(in narrow mode) focuses the panel.

> **`x` means "close".** For agents and terminals — which are throwaway instances — `x` kills the
> pane and removes the row. For processes — which are defined in your config — `x` stops the
> process but leaves the row so you can start it again. The footer labels this key `close`.

## Pane Keys (a focused program)

When the main pane is focused, every keystroke is translated to terminal bytes and sent to the
program. **`Ctrl-b` is the leader**: press it, then one more key for an mmux command.

| Chord | Action |
| --- | --- |
| `Ctrl-b` `h` · `Ctrl-b` `←` · `Ctrl-b` `Esc` | Back to the sidebar |
| `Ctrl-b` `d` | Detach |
| `Ctrl-b` `x` | Close the focused session (removes an agent/terminal, stops a process), then return to the sidebar |
| `Ctrl-b` `R` | Reload config, then return to the sidebar |
| `Ctrl-b` `q` | Quit mmux |
| `Ctrl-b` `b` | Send a literal `Ctrl-b` to the program (e.g. for an inner tmux) |

The leader is single-shot: one sub-command (or any unrecognized key) disarms it. Typing while
scrolled into [scrollback](#scrollback-and-copy) snaps back to the live view.

## The Git Panel

The git panel is built into mmux — there is nothing to install or configure beyond having a git
repository (disable it with [`git-panel: { enabled: false }`](04-configuration.md#git-panel)). It
is mmux's own UI, not an embedded tool, and is driven entirely by the keyboard and mouse below.

It has three boxes:

- **Changes** — a compressed tree of changed files with staging checkboxes:
  `[✓]` (green) fully staged · `[~]` (yellow) partially staged · `[ ]` (gray) unstaged. The
  filename color encodes the change: red (untracked/deleted/unmerged), green (added), cyan
  (renamed/copied), yellow (modified). The box title shows the branch with `↑ahead`/`↓behind`
  counts and a `pulling…`/`pushing…` note during network operations.
- **Branches** — local branches, current one marked, with upstream tracking notes.
- **Recent** — the last 20 commits, display-only.

Focus the panel with `Tab` (or click it), then:

| Key | Action |
| --- | --- |
| `↑` / `k` · `↓` / `j` | Move the cursor |
| `Tab` | Toggle between the Changes and Branches boxes |
| `Enter` · `Space` | Stage/unstage the file, directory, or whole repo under the cursor — or, in Branches, switch to the branch |
| `a` | Stage all changes |
| `c` | Commit (opens a message prompt) |
| `n` | New branch (opens a name prompt; creates and switches) |
| `d` | Discard the selected path (destructive — asks for confirmation) |
| `s` | Stash (`git stash push -u`, includes untracked; recover with `git stash pop`) |
| `p` · `P` | Pull · Push (run in the background; the result is flashed in the footer) |
| `b` | Jump to the Branches box |
| `r` | Refresh |
| `h` · `←` · `Esc` | Back to the sidebar |

Staging is whole-file, whole-directory, or whole-repo — there is no hunk staging. Committing with
nothing staged stages everything first. Pull and push never block the UI; a second press while
one is in flight is ignored. The panel re-reads git state on a short throttle, so commits an
agent makes in the main pane show up on their own.

## The File Picker

Press **`Ctrl+P`** from anywhere — including inside a focused pane — to open a fuzzy file picker
for the active project. (This deliberately shadows a pane's own `Ctrl+P`.)

| Key | Action |
| --- | --- |
| *type* | Filter the file list |
| `↑` / `Ctrl-p` / `Ctrl-k` · `↓` / `Ctrl-n` / `Ctrl-j` | Move the highlight |
| `Enter` | Open the highlighted file in your editor |
| `Esc` | Cancel |

The chosen file opens in `$VISUAL`/`$EDITOR` (else `micro`, else `nano`) as a temporary terminal
row marked `✎ <file>`. That row disappears on its own when you quit the editor.

## Adding a Process

The `+ New Process` launcher opens a four-step guided form that writes a new process into the
project's `mmux.yaml`:

1. **Name** — must be non-empty and not duplicate an existing process.
2. **Command** — the shell command line (quote-aware).
3. **Working dir** — optional; blank means the project root.
4. **Review** — toggle autostart, then create.

`Enter` advances (and validates); `Esc` cancels; on the Review step `y`/`n` set autostart on/off
(and `Space`/`Tab`/`←`/`→` toggle it). The entry is appended to `mmux.yaml` **preserving your existing
comments and layout**, the config is reloaded, the new row is selected, and — if you chose
autostart — it is started immediately.

> The form cannot set environment variables. For a process that needs `env`, edit `mmux.yaml`
> directly and press `R` to reload.

## Mouse

mmux drives its own focus, scrollback, and copy from the mouse; mouse events are **not** forwarded
into the focused program.

- **Sidebar.** Single-click selects a row (and jumps into a running session, clearing its
  attention dot). Double-click a `+ New Agent`/`+ New Terminal` launcher to spawn it (`+ New
  Process` opens the guided form), or a stopped session to start and enter it. In a [multi-project](04-configuration.md#linked-projects) workspace, clicking
  another project's box switches to it.
- **Git panel.** Single-click focuses a box and selects a row; double-click a file to
  stage/unstage it or a branch to switch to it. The scroll wheel moves the cursor.
- **Main pane.** Single-click focuses it. **Click and drag to select text and copy it** to the
  clipboard on release (a footer flash confirms `copied N chars`); dragging to the top or bottom
  edge auto-scrolls through scrollback while held. The scroll wheel scrolls scrollback (wheel up
  reveals older lines).
- **Footer.** The shortcut chips are clickable — each is an alias for its keybinding.

## Scrollback and Copy

Each pane keeps 5000 lines of scrollback. Scroll it with the wheel; typing snaps back to the live
view. To copy, **drag-select** across the main pane — the text goes to the clipboard via OSC 52
(which works over SSH and through the tmux jail) plus a local helper
(`pbcopy`/`wl-copy`/`xclip`/`xsel`). Selection can span scrollback with edge auto-scroll. Only
the main pane is selectable; the git panel is native text, not a copyable grid. There is no
keyboard copy-mode yet.

## Narrow Terminals and Phones

When the terminal is narrow (under 60 columns — e.g. SSH from a phone), mmux switches to a
**single-column** layout: one region fills the screen and the title bar carries tap targets.

- **`☰`** (top-left) opens the sidebar drawer.
- A **`<branch> ☰`** button (top-right) opens the git panel.

Pick something in the drawer to view it full-screen; tap `☰` to go back. Everything stays
reachable by keyboard (`Ctrl-b h` returns to the drawer), and in this mode the git panel appears
as a `GIT` entry in the sidebar.

## Detaching, Reattaching, and the Attach Picker

- `d` (or `Ctrl-b d`) **detaches**: the tmux session and your agents keep running in the
  background. Closing the terminal or dropping SSH detaches the same way.
- Run `mmux` again in the same directory to reattach.
- Run **`mmux attach`** (alias `mmux a`) to open a picker of every running mmux session on the
  machine, plus recently opened directories that aren't currently running. In the picker:
  `↑`/`↓` or `j`/`k` move, `Enter`/`l`/`→` or left-click opens, `q`/`Esc` cancels. Choosing a
  recent directory opens (or creates) its session.
