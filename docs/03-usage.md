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
  ` <name> — <status> ` and turns its border magenta when focused. It also doubles as a
  read-only [diff preview](#the-diff-preview) when you click a file in the git panel.
- **Git panel (right).** Appears automatically when the directory is a git repository. See
  [The Git Panel](#the-git-panel).
- **Footer (bottom).** Context-dependent key hints rendered as **clickable** shortcut chips.

### Status and Attention

- **Process rows** show a leading status glyph + a matching name color: green `●` running, a dim
  `·` when it's not running (whether it finished on its own, was stopped, or was never started —
  they all read alike), and a red `○` only when it **crashed** (exited non-zero on its own). For a
  long-lived process what you want to know is "is it up, and did it die badly".
- **Agent and terminal rows** hold color back for the one thing that matters there — "does it
  need *me*". A leading glyph + name color carry the whole state: a busy agent shows a small gray
  **spinner** (rotating braille dots) before its name (a running terminal keeps a static `·`), a
  session that **crashed** (exited non-zero on its own) or **failed to launch** shows a red hollow
  `○`, and a session waiting on you lights up **green** (`●`). So when you scan the sidebar, the
  only colored agent is the one to go look at — and the spinning ones are still grinding. Unlike
  processes, agents and terminals **don't linger once they exit cleanly** — quitting an agent
  (`/quit`, Ctrl-D) or `exit`ing a terminal removes its row outright rather than leaving a dim
  "exited" husk. A crash is the exception: it stays put, painted red, so you don't miss it.
- Every session row shows a dim **subtitle** — the terminal title the program sets (e.g. what
  Claude is currently doing, including its own working/idle animation), falling back to its last
  error.
- For an **agent**, the green state is driven by its terminal title going quiet: an agent animates
  its title while working, so once it's been static for ~2s mmux reads it as idle/awaiting you and
  lights the row green — and it drops back to the gray spinner the moment the title starts moving
  again. This reflects the agent's real state, so it holds even while you're viewing the pane:
  selecting an idle agent does not make it look busy. For a **terminal** (which has no such
  animation) the trigger is the bell instead, and — being a momentary ping — it *is* suppressed on
  the pane you're actively viewing. The bell / notification escape *separately* raises a
  [desktop notification](05-notifications.md), and process rows show that bell as a trailing green
  `●`, since their name already signals up/down.

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
| `L` | [Link another project](04-configuration.md#linked-projects) into the workspace (also the button at the bottom of the sidebar) |
| `U` | Restart to apply a staged [self-update](04-configuration.md#auto-update) (only when the `↻` badge is showing; you can also click it) |
| `[` · `]` | Switch to the previous / next project ([linked projects](04-configuration.md#linked-projects); no-op with one project) |
| `Tab` | Jump to the git panel (or into the selected pane if there is no panel) |
| `d` | Detach (the session keeps running in the background) |
| `q` | Quit mmux (asks to confirm first if any agent/terminal/process is still running) |
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
| `Ctrl-b` `q` | Quit mmux (same confirmation as `q` when anything is running) |
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
| `↑` / `k` · `↓` / `j` | Move the cursor (with a diff preview open, it follows the cursor) |
| `Tab` | Toggle between the Changes and Branches boxes |
| `Enter` · `Space` | Stage/unstage the file, directory, or whole repo under the cursor — or, in Branches, switch to the branch |
| `v` | Preview the selected file's diff in the main pane (press again to close) |
| `a` | Stage all changes |
| `c` | Commit (opens a message prompt) |
| `n` | New branch (opens a name prompt; creates and switches) |
| `d` | Discard the selected path (destructive — asks for confirmation) |
| `s` | Stash (`git stash push -u`, includes untracked; recover with `git stash pop`) |
| `p` · `P` | Pull · Push (run in the background; the result is flashed in the footer) |
| `b` | Jump to the Branches box |
| `r` | Refresh |
| `h` · `←` · `Esc` | Close the diff preview if open, else back to the sidebar |

Staging is whole-file, whole-directory, or whole-repo — there is no hunk staging. Committing with
nothing staged stages everything first. Pull and push never block the UI; a second press while
one is in flight is ignored. The panel re-reads git state on a short throttle, so commits an
agent makes in the main pane show up on their own.

### The Diff Preview

Single-click a changed file (or press `v`) to show its diff in the **main pane** — where an agent
usually lives — as a read-only, colour-coded pager (additions green, deletions red, `@@` hunks
cyan). It is a *live preview*: as you move the Changes cursor it follows along, and it re-reads
the file on the same throttle as the panel, so an agent's edits to the shown file appear as they
happen. The header reads `Δ <path>  +added −removed`; the diff is `HEAD` vs the working tree, so
staged and unstaged edits show together (a brand-new file shows as all-added).

Click the preview (or it's already in front in compact mode) to scroll it with `↑↓` / `j` `k`,
`PgUp`/`PgDn` / `Space`, `Ctrl-d`/`Ctrl-u`, and `g`/`G`. Close it with `Esc` (or `q` / `h`); it
also clears the moment you select a session or switch projects.

## The File Picker

Press **`Ctrl+P`** from anywhere — including inside a focused pane — to open a fuzzy file picker
for the active project. (This deliberately shadows a pane's own `Ctrl+P`.)

| Key | Action |
| --- | --- |
| *type* | Filter the file list |
| `↑` / `Ctrl-p` / `Ctrl-k` · `↓` / `Ctrl-n` / `Ctrl-j` | Move the highlight |
| `Enter` | Open the highlighted file in your editor |
| `Esc` | Cancel |

The list includes hidden files, and — even though it otherwise honours `.gitignore` — it also
surfaces gitignored env files (`.env`, `.env.local`, `.envrc`, …) so you can edit them. The chosen
file opens in `$VISUAL`/`$EDITOR` (else `micro`, else `nano`) as a temporary terminal row marked
`✎ <file>`. That row disappears on its own when you quit the editor.

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

## Linking Another Project

The **`+ Link another project`** button pinned inside the bottom of the sidebar box (or the `L` key)
opens a small directory browser to add a [linked project](04-configuration.md#linked-projects)
without leaving mmux:

- It opens one level **above** your launch directory, so sibling clones (the common `../proj2`) are
  right there.
- **Type** to filter the current folder; `↑`/`↓` move; `→` (or `Tab`) descends into a directory and
  `←` goes back up.
- A short **preview** of the highlighted directory shows the path it would be linked as, its git
  branch, and whether it has its own `mmux.yaml`. Folders already in the workspace are tagged
  `linked` and can't be added twice.
- **`Enter`** links the highlighted directory; `Esc` cancels.

The chosen path is appended to the launch directory's `linked-projects:` (**preserving your existing
comments and layout**) and the project appears as a **new sidebar box immediately** — running panes
are untouched. Its processes start stopped, like any linked project. Removing a link still needs a
reopen.

## Mouse

mmux drives its own focus, scrollback, and copy from the mouse. Mouse events are otherwise **not**
forwarded into the focused program — the one exception is the wheel over a full-screen program (see
**Main pane** below), which has no scrollback of its own to scroll.

- **Sidebar.** Single-click selects a row. For an **agent or terminal** it also jumps into the
  running session (clearing its attention dot); double-click a `+ New Agent`/`+ New Terminal`
  launcher to spawn it (`+ New Process` opens the guided form), or a stopped session to start and
  enter it. **Processes behave differently** — they are monitored, not driven: clicking one
  selects it (its output shows in the main pane) but keeps focus on the sidebar, and
  double-click **restarts** it in place — start if stopped, respawn if running — without jumping
  in (the `r` key does the same). In a [multi-project](04-configuration.md#linked-projects)
  workspace, clicking another project's box switches to it. The **`+ Link another project`** button
  inside the bottom of the sidebar box opens the [project browser](#linking-another-project).
- **Git panel.** Single-click focuses a box and selects a row; on a changed file it also
  [previews the file's diff](#the-diff-preview) in the main pane. Double-click a file to
  stage/unstage it or a branch to switch to it. The scroll wheel moves the cursor (and the open
  preview follows it).
- **Main pane.** Single-click focuses it. **Click and drag to select text and copy it** to the
  clipboard on release (a footer flash confirms `copied N chars`); dragging to the top or bottom
  edge auto-scrolls through scrollback while held. The scroll wheel scrolls scrollback (wheel up
  reveals older lines) — but over a **full-screen program** (nano, micro, `less`, vim, …), which
  runs on the alternate screen and has no scrollback, the wheel is handed to the program instead:
  forwarded as a mouse-wheel event if it tracks the mouse, otherwise as arrow keys. When a
  [diff preview](#the-diff-preview) occupies the pane it's a read-only pager instead — the wheel
  and the keys scroll the diff.
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
- `q` (or `Ctrl-b q`) **quits**, ending the inner tmux session and killing every agent,
  terminal, and process. Because that's destructive, mmux asks you to confirm whenever
  anything is still running — and the confirmation offers `d` to detach instead. With
  nothing running, `q` quits immediately.
- Run `mmux` again in the same directory to reattach.
- **Your session comes back.** Even after a `q` (or a crash, or a restart-to-update), reopening a
  directory **restores the agents and terminals** you had open: **Claude and Codex agents resume
  their conversation**, and **terminals reopen in the directory you left them in** (as a fresh
  shell — history, env, and background jobs don't carry over). Anything that can't resume starts
  fresh; processes come back via autostart or a click. To start clean instead, **close the sessions
  (`x`) before quitting** — only what's still open is remembered.
- Run **`mmux attach`** (alias `mmux a`) to open a picker of every running mmux session on the
  machine, plus recently opened directories that aren't currently running. Each row leads with the
  project's name (its `mmux.yaml` `name:`, else the folder) and shows its directory beside it in
  dim text. The picker has an always-on search bar at the top: just start typing to fuzzy-filter the
  list by name or directory — no need to focus it first, and the best match stays selected. `↑`/`↓`
  move, `Enter`/`→` or left-click opens, `Backspace` trims the query, `Esc` clears it (then cancels
  on a second press), `Ctrl+C` cancels. Choosing a recent directory opens (or creates) its session.
