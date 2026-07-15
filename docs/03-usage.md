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
│ PROCESSES      │                           │ Commits        │
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
| `x` | Close: **removes** an agent/terminal row entirely (asks to confirm first only if there's live work to lose — a **spinning** agent, or a running terminal; an idle agent showing the green `●` closes with no prompt); **stops** a running process (row stays; runs its `stop:` teardown if it has one). Closing lands the cursor on the **previous** row |
| `r` | Restart the selected session (or spawn a launcher) |
| `e` | **Edit** the selected process — reopens the [guided form](#adding-editing-and-deleting-a-process) pre-filled (processes only) |
| `D` | **Delete** the selected process — asks to confirm, then removes it from `mmux.yaml` (processes only) |
| `a` | Open the [agent manager](04-configuration.md#agent) — add/remove the built-in harnesses (Claude, Codex, Gemini, Amp, opencode, Grok) and toggle danger mode; saves to the global config and reloads |
| `R` | [Reload config](04-configuration.md#live-reload) live |
| `U` | Restart to apply a staged [self-update](04-configuration.md#auto-update) (only when the `↻` badge is showing; you can also click it) |
| `?` | Open the [About card](#the-about-card) (version, links, manual update check/apply) |
| `[` · `]` | Switch to the previous / next project ([linked projects](04-configuration.md#linked-projects); no-op with one project) |
| `Tab` | Jump to the git panel (or into the selected pane if there is no panel) |
| `d` | Detach (the session keeps running in the background) |
| `q` | Quit mmux (asks to confirm first if any agent/terminal/process is still running) |
| `Ctrl+P` | Open the [fuzzy file picker](#the-file-picker) (works from any focus) |

**Opening a row** with `Enter`/`l`/`→` does the right thing for its kind: a `+ New Agent`/
`+ New Terminal` launcher spawns and jumps into a fresh pane; `+ New Process` opens the
[guided form](#adding-editing-and-deleting-a-process); a stopped session is (re)started and
focused; the `+ Link another project` row (its own box at the bottom) opens the
[directory browser](#linking-another-project); the git-panel row (in narrow mode) focuses the panel.

> **Agents/terminals vs. processes.** Agents and terminals are throwaway instances, so `x` kills
> the pane and removes the row outright. Processes are **config entries**, so they're managed
> differently: `x` only *stops* a running one (the row stays, to start again), and to change or
> remove a process you **edit** it (`e`, reopens the guided form) or **delete** it (`D`, with a
> confirmation that rewrites `mmux.yaml`). The footer swaps its chips to match: on a process you see
> `edit`/`delete` instead of `close`, and `stop` shows only while it's actually running. A process
> can also carry an optional [`stop:` teardown command](04-configuration.md#process) that mmux runs
> in its directory when it stops (on `x` or on quit) — e.g. `docker compose down`.

## Pane Keys (a focused program)

When the main pane is focused, every keystroke is translated to terminal bytes and sent to the
program. **`Ctrl-b` is the leader**: press it, then one more key for an mmux command. The leader is
**global** — the same chords work no matter what has focus (the sidebar, a pane, or the git panel) —
so "leave" (detach/quit) and "back to the sidebar" are one motor pattern everywhere. You never have
to return to the sidebar first just to reach them.

| Chord | Action |
| --- | --- |
| `Ctrl-b` `h` · `Ctrl-b` `←` · `Ctrl-b` `Esc` | Back to the sidebar |
| `Ctrl-b` `d` | Detach |
| `Ctrl-b` `x` | Close the focused session (removes an agent/terminal — confirmed first only if a **spinning** agent or a running terminal — stops a process), then return to the sidebar |
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
  `[✓]` (green) fully staged · `[~]` (yellow) partially staged · `[ ]` (gray) unstaged. The tree
  starts at the repo's top-level files and folders — there is no whole-repo row taking up the first
  line (stage everything with `a`). The filename color encodes the change: green
  (added/untracked), red (deleted/unmerged), cyan (renamed/copied), yellow (modified). The box
  title shows a `pulling…`/`pushing…` note during network operations.
- **Branches** — local branches, current one marked, with upstream tracking notes (ahead/behind).
- **Commits** — recent history (up to 200 commits), scrollable and selectable. Select a commit to
  show its full diff in the main pane, copy its hash or message, or revert / uncommit it.

Focus the panel with `Tab` (or click it), then:

| Key | Action |
| --- | --- |
| `↑` / `k` · `↓` / `j` | Move the cursor (with a diff preview open, it follows the cursor) |
| `Tab` | Cycle the cursor: Changes → Branches → Commits |
| `Enter` · `Space` | Stage/unstage the file or directory — or switch to the branch, or show the commit's diff |
| `v` | Preview the selected file's / commit's diff in the main pane (press again to close) |
| `a` | Stage all changes (press again to unstage all) |
| `c` | Commit (opens a message prompt; `Ctrl+P` there commits **and** pushes — as does `Ctrl+⏎` where the terminal reports it) |
| `n` | New branch (opens a name prompt; creates and switches) |
| `d` | Discard the selected path (destructive — asks for confirmation) |
| `s` | Stash (`git stash push -u`, includes untracked; recover with `git stash pop`) |
| `p` · `P` | Pull · Push (run in the background; the result is flashed in the footer) |
| `b` | Jump to the Branches box |
| `r` | Refresh |
| `h` · `←` · `Esc` | Close the diff preview if open, else back to the sidebar |

In the **Commits** box these keys act on the selected commit:

| Key | Action |
| --- | --- |
| `y` · `Y` | Copy the short hash · the full 40-char hash |
| `m` | Copy the full commit message (subject + body) |
| `t` | Revert the commit (`git revert`; asks to confirm — creates a new undoing commit) |
| `u` | Uncommit to here (`git reset --soft`; asks to confirm — later commits become staged changes) |

Staging is whole-file or whole-directory (or `a` for the whole repo) — there is no hunk staging.
Committing with nothing staged stages everything first. Commit & push (`Ctrl+P` from the commit
prompt — `Ctrl+⏎` also works where the terminal reports the modifier through tmux) commits and then
kicks off the same background push as `P` — the push result is flashed when it returns. Pushing a
branch that has no upstream publishes it (`--set-upstream` to `origin`, or to the only/first remote
when there's no `origin`) rather than failing. Pull and push never block the UI; a second press while one is in flight is
ignored. Revert and uncommit go through a confirmation and refresh the panel; neither loses work
(revert adds a commit, uncommit keeps the changes staged). The panel re-reads git state on a short
throttle, so commits an agent makes in the main pane show up on their own.

### The Diff Preview

Single-click a changed file (or press `v`) to show its diff in the **main pane** — where an agent
usually lives — as a read-only, syntax-highlighted pager: a dim line-number gutter, the `+`/`-`
sign, then the code coloured by language, with added and removed lines carrying a subtle full-row
green/red **tint** (rather than flat coloured text) and `@@` hunk headers reading as quiet blue
dividers. Highlighting is per-line (the pager never sees the whole file), and is skipped for very
large diffs. It is a *live preview*: as you move the Changes cursor it follows along, and it re-reads
the file on the same throttle as the panel, so an agent's edits to the shown file appear as they
happen. The header reads `Δ <path>  +added −removed`; the diff is `HEAD` vs the working tree, so
staged and unstaged edits show together (a brand-new file shows as all-added).

The same pager shows a **commit** when you select one in the Commits box (⏎ / `v` / click): its full
`git show` diff across every file it touched, each file introduced by a bold `▸ path` divider so you
can tell the hunks apart. The header reads `● <hash> <subject>  +added −removed`. Unlike the file
preview a commit diff is static — it doesn't self-refresh — and it stays up until you close it or
select a session. (Binary changes read as git's "Binary files differ"; only the working-tree preview
inlines images.)

Click the preview (or it's already in front in compact mode) to scroll it with `↑↓` / `j` `k`,
`PgUp`/`PgDn` / `Space`, `Ctrl-d`/`Ctrl-u`, and `g`/`G`. Close it with `Esc` (or `q` / `h`); it
also clears the moment you select a session or switch projects.

**Drag to select and copy.** Press and drag across the diff to select text — releasing copies it to
the clipboard (OSC 52 + the local helper). The selection covers only the **code**: the line-number
gutter and the `+`/`-` sign column are never included, so a copied hunk pastes as plain source, not
a diff. Dragging past the top or bottom edge auto-scrolls to extend the selection beyond one screen.

**Image files** (`png`, `jpg`/`jpeg`, `gif`, `webp`, `bmp`) preview as the picture itself rather
than git's "Binary files differ", decoded from the working tree. On a **sixel-capable terminal**
you get a real, legible picture (mmux draws through tmux, and tmux renders the sixel natively for
terminals that support it — e.g. WezTerm, foot, Ghostty, recent Konsole/xterm); everywhere else it
falls back to a coarse **half-block** approximation that always works but can't resolve fine detail.
The header shows the file's pixel dimensions (` ▦ <path>  1200×800 `). It's a still preview —
re-click the file to refresh it after a change — and very large or undecodable files fall back to
the plain textual diff. Force the sixel path on or off with `MMUX_SIXEL=1` / `MMUX_SIXEL=0` if the
auto-detection guesses wrong for your terminal.

## The File Picker

Press **`Ctrl+P`** from anywhere — including inside a focused pane — to open a fuzzy file picker
for the active project. (This deliberately shadows a pane's own `Ctrl+P`.)

| Key | Action |
| --- | --- |
| *type* | Filter the file list |
| `↑` / `Ctrl-p` / `Ctrl-k` · `↓` / `Ctrl-n` / `Ctrl-j` | Move the highlight |
| `Enter` | Open the highlighted file in your editor |
| `Esc` | Cancel |

The list includes hidden files and does **not** honour `.gitignore`, so gitignored-but-edited
files (`.env`, local notes, generated config) show up too; heavy build/artifact directories
(`node_modules`, `target`, `dist`, `.venv`, …) are excluded so they don't flood the list. A
project can still hide paths from the picker with an `.ignore`/`.rgignore` file. The chosen
file opens in `$VISUAL`/`$EDITOR` (else the first of `micro`, `nano`, `vim`, `vi` on `PATH`) as a temporary terminal row marked
`✎ <file>`. That row disappears on its own when you quit the editor.

## The About Card

Press **`?`** in the sidebar (or click the `[? about]` footer chip) to open a small card with the
running version, a link home (`marvinvr.ch`) and to the source, and the current
[self-update](04-configuration.md#auto-update) status. The two links are **clickable** — a click
opens them in your browser (see [Opening links](#opening-links)). From it:

| Key | Action |
| --- | --- |
| `c` | Check for updates now (when self-update is available and idle) |
| `u` | Restart to apply a staged update (when one is ready) |
| `Esc` / `q` / `?` | Close |

The status line reflects the background updater live: `up to date`, `checking…`, `downloading
v…`, or `v… ready`. Where mmux can't update itself the check is hidden: a dev build or one with
auto-update disabled reads `self-update off for this build`, and a non-Homebrew install (e.g.
`cargo install`) reads `self-update off (not a Homebrew install)` once the check confirms it.

## Adding, Editing, and Deleting a Process

The `+ New Process` launcher opens a five-step guided form that writes a new process into the
project's `mmux.yaml`:

1. **Name** — must be non-empty and not duplicate an existing process.
2. **Command** — the shell command line (quote-aware).
3. **Working dir** — optional; blank means the project root.
4. **Stop command** — optional; a shell line run in the working dir **after the process stops**
   (on `x` and on quit), blank for none. Handy for a teardown like `docker compose down`.
5. **Review** — toggle autostart, then create.

`Enter` advances (and validates); `Esc` cancels; on the Review step `y`/`n` set autostart on/off
(and `Space`/`Tab`/`←`/`→` toggle it). The entry is appended to `mmux.yaml` **preserving your existing
comments and layout**, the config is reloaded, the new row is selected, and — if you chose
autostart — it is started immediately.

**Editing** — press `e` on a process to reopen the same form **pre-filled** with its current
name, command, working dir, stop command, and autostart. Finishing (`⏎ save` on Review) splices the
change back into the same `mmux.yaml` entry, again **preserving your surrounding comments and
layout**, and reloads. If you edited the command of a process that's **currently running**, the
reload restarts it so the new command takes effect right away — no manual stop/start. (Editing just
the stop command never restarts it — the teardown only ever runs when the process actually stops.)

**Deleting** — press `D` on a process for a confirmation, then `y` removes it from `mmux.yaml`
(stopping any running instance) and reloads so the row disappears. `n`/`Esc` cancels.

> The form cannot set environment variables. For a process that needs `env`, edit `mmux.yaml`
> directly and press `R` to reload.

## Linking Another Project

The **`+ Link another project`** row sits in its own box at the bottom of the sidebar — reach it with
the arrow keys and press `Enter`, or click it. It opens a small directory browser to add a
[linked project](04-configuration.md#linked-projects) — any other project you want in the same
workspace, not just another clone — without leaving mmux:

- It opens one level **above** your launch directory, so nearby projects (the common `../proj2`) are
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

mmux drives its own focus, scrollback, and copy from the mouse. Over the main pane, though, a
program that **tracks the mouse** (micro, vim, lazygit, …) gets clicks, drags, and motion forwarded
to it — so you can click to place the cursor or drag to select inside the program, exactly as in a
plain terminal. Hold **Shift** to bypass that and use mmux's own [drag-to-copy](#scrollback-and-copy)
instead. Programs that don't track the mouse are unaffected: their pane drag-selects as always.

- **Sidebar.** Single-click selects a row. For an **agent or terminal** it also jumps into the
  running session (clearing its attention dot); double-click a `+ New Agent`/`+ New Terminal`
  launcher to spawn it (`+ New Process` opens the guided form), or a stopped session to start and
  enter it. **Processes behave differently** — they are monitored, not driven: clicking one
  selects it (its output shows in the main pane) but keeps focus on the sidebar, and
  double-click **restarts** it in place — start if stopped, respawn if running — without jumping
  in (the `r` key does the same). In a [multi-project](04-configuration.md#linked-projects)
  workspace, clicking another project's box switches to it. The **`+ Link another project`** box
  at the bottom of the sidebar opens the [project browser](#linking-another-project).
- **Git panel.** Single-click focuses a box and selects a row; on a changed file or a commit it also
  [previews the diff](#the-diff-preview) in the main pane. Double-click a file to stage/unstage it or
  a branch to switch to it. The scroll wheel moves the cursor (and the open preview follows it in the
  Changes and Commits boxes).
- **Main pane.** Single-click focuses it. If the program tracks the mouse, the click/drag/release
  goes to it (place the cursor, select in-app); otherwise — or with **Shift** held — **click and
  drag to select text and copy it** to the clipboard on release (a footer flash confirms
  `copied N chars`), and dragging to the top or bottom edge auto-scrolls through scrollback while
  held. The scroll wheel scrolls scrollback (wheel up reveals older lines) — but over a
  **full-screen program** (nano, micro, `less`, vim, …), which runs on the alternate screen and has
  no scrollback, the wheel is handed to the program instead: forwarded as a mouse-wheel event if it
  tracks the mouse, otherwise as arrow keys. When a [diff preview](#the-diff-preview) occupies the
  pane it's a read-only pager instead — the wheel and the keys scroll the diff. A plain click on a
  **URL** in the pane opens it — see [Opening links](#opening-links).
- **Footer.** The shortcut chips are clickable — each is an alias for its keybinding.

## Opening links

Click a link to open it in your default browser. This works in two places:

- **mmux's own UI** — the two links on the [About card](#the-about-card) (`marvinvr.ch` and the
  source repo). A plain click opens them.
- **Pane content** — a URL an agent or program prints (`http(s)://…`, or a bare `www.…`). A plain
  **click** on it opens it; a footer flash confirms `opened <url>`. Only a *click* opens — a
  **drag** across a URL still [selects and copies](#scrollback-and-copy) it as usual, so you can copy
  a link instead of following it. If the program [tracks the mouse](#mouse) (vim, micro, …) a plain
  click goes to it, so **hold Shift** and click to open the link instead. Opening is best-effort via
  the platform opener (`open` on macOS, `xdg-open` on Linux).

## Scrollback and Copy

Each pane keeps 5000 lines of scrollback. Scroll it with the wheel; typing snaps back to the live
view. To copy, **drag-select** across the main pane — the text goes to the clipboard via OSC 52
(which works over SSH and through the tmux jail) plus a local helper
(`pbcopy`/`wl-copy`/`xclip`/`xsel`). Selection can span scrollback with edge auto-scroll. If the
program tracks the mouse it would otherwise eat the drag, so **hold Shift** to drag-select over it.
Only the main pane is selectable; the git panel is native text, not a copyable grid. There is no
keyboard copy-mode yet.

## Narrow Terminals and Phones

When the terminal is narrow (under 60 columns — e.g. SSH from a phone), mmux switches to a
**single-column** layout: one region fills the screen and all navigation lives in the footer's two
bottom corners, so the same corner that opens a panel also closes it.

- On a **pane**: **`[☰ menu]`** (bottom-left) opens the sidebar drawer; **`[git]`** (bottom-right)
  opens the git panel. Typed keys go straight to the program.
- On the **drawer** or the **git panel**: **`[✕ close]`** returns to the pane — bottom-left for the
  drawer (mirroring `[☰ menu]`), bottom-right for the git panel (mirroring `[git]`). The git panel
  keeps its action buttons (stage, commit, push, …) on the left.

Pick something in the drawer to view it full-screen. Everything stays reachable by keyboard
(`Ctrl-b h` returns to the drawer), and in this mode the git panel also appears as a `GIT` entry in
the sidebar.

## Detaching, Reattaching, and the Attach Picker

- `d` (or `Ctrl-b d`) **detaches**: the tmux session and your agents keep running in the
  background. Closing the terminal or dropping SSH detaches the same way.
- `q` (or `Ctrl-b q`) **quits**, ending the inner tmux session and killing every agent,
  terminal, and process. Because that's destructive, mmux asks you to confirm whenever
  anything is still running — and the confirmation offers `d` to detach instead. With
  nothing running, `q` quits immediately. Any still-running process that defines a
  [`stop:` command](04-configuration.md#process) has it run (and waited for) as mmux exits,
  so a `docker compose up` gets its `docker compose down` on the way out.
- Run `mmux` again in the same directory to reattach.
- **Your session comes back.** Even after a `q` (or a crash, or a restart-to-update), reopening a
  directory **restores the agents and terminals** you had open: **Claude and Codex agents resume
  their conversation**, and **terminals reopen in the directory you left them in** (as a fresh
  shell — history, env, and background jobs don't carry over). Anything that can't resume starts
  fresh; processes come back via autostart or a click. To start clean instead, **close the sessions
  (`x`) before quitting** — only what's still open is remembered.
- Run **`mmux attach`** (alias `mmux a`) to open a picker of every running mmux session on the
  machine, plus recently opened directories that aren't currently running. Running sessions come
  first, then the not-running recents; within each group rows are ordered most-recently-used first.
  Each row leads with the project's name (its `mmux.yaml` `name:`, else the folder) and shows its
  directory beside it in dim text. The picker has an always-on search bar at the top: just start typing to fuzzy-filter the
  list by name or directory — no need to focus it first, and the best match stays selected. `↑`/`↓`
  move, `Enter`/`→` or left-click opens, `Backspace` trims the query, `Esc` clears it (then cancels
  on a second press), `Ctrl+C` cancels. Choosing a recent directory opens (or creates) its session.
