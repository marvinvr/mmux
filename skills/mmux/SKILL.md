---
name: mmux
description: Drive a running mmux (terminal multiplexer for AI agents, terminals and dev processes) from the command line. Use when running inside an mmux pane (MMUX_SESSION / MMUX_SOCKET is set), or when asked to check on, list, spawn, prompt, wait for, or close other coding agents in mmux; to read a dev server's or process's logs/errors; to start, stop or restart a process; to delegate a prompt to another agent (Claude, Codex, …) and read its reply; to cut or remove a git worktree with its own agent for parallel work; or to type text/keys into another terminal session.
---

# mmux control CLI

mmux is a TUI a human is watching: a sidebar of **agents**, **terminals** and **processes** per
project. Every running mmux serves a private socket, and the `mmux <verb>` commands below drive it —
from inside one of its panes or from anywhere on the machine. Everything you do shows up in the
human's sidebar, and they can open and take over any session.

`mmux ls --help` prints the full reference.

## Which mmux, which session

- **Inside an mmux pane** you get `MMUX_SOCKET` (your mmux), `MMUX_SESSION` (your own id, e.g. `s7`),
  `MMUX_PROJECT` (your project's directory) and `MMUX_DEPTH`. Commands talk to your own mmux.
- **Outside**, a command finds the mmux for the current directory (a project, a workspace member, or
  a worktree). `-C <dir>` picks another, and also counts as your directory for choosing a project
  (bare names, `new`). If none is running, the error lists the running ones.
- **Targets `<t>`**: an id (`s12`, stable while the session lives — prefer it), a name (`"Claude #2"`;
  case and spaces ignored, so `claude#2` works, as does a unique prefix), `project/name` when a name
  exists in several projects, or `self` (the pane you run in; only inside that mmux). An ambiguous
  name errors and lists the candidates. Bare names prefer your own project.

## Verbs

```sh
mmux ls                              # projects (+ agent templates) and sessions: id kind name state title
mmux status s12                      # state, title, last ~5 non-empty screen lines
mmux read "Dev server" -n 80         # last 80 lines of output (default 200, -n 0 = all)
mmux last s12                        # an agent's last reply (Claude/Codex/Pi/Grok: from transcript)
mmux send s12 "fix the failing test" # paste text, then Enter (--no-enter: don't; `-` = stdin)
mmux keys s12 Escape                 # press keys: Enter Escape Tab BTab BSpace Space Up Down Left Right
                                     #   Home End PageUp PageDown Delete Insert F1-F12, C-/M-/S- prefixes
mmux new agent codex -p api --prompt "…"   # start an agent (template by name or command; default: first)
mmux new terminal --cmd "npm test"   # start a terminal, type a command into it (stays open)
mmux start|restart "Dev server"      # start anything not running / (re)start any session
mmux stop "Dev server" [--force]     # process: stop in place (runs its stop: teardown)
mmux close s12 [--force]             # agent/terminal: close for good (busy => refused without --force)
mmux close self                      # close your own pane (no --force needed)
mmux wait s12 [-t 10m] [--settle 1.5s] [--idle|--exit]   # until the agent is done (default) / ended
mmux ask "why is CI red?"            # new agent -> wait -> print its reply
mmux worktree new [branch] [-p project] [--agent <template>] [--prompt "…"]
                                     # cut a git worktree (+ an agent in it)
mmux worktree rm <branch> [--force]  # remove it: sessions close, checkout goes
                                     # (your own: warns first that it ends you)
mmux reload                          # reload the config live, like R — after editing mmux.yaml;
                                     #   fails with the parse error if a config doesn't load
```

- States: an agent is `working` (its sidebar row spins) or `idle` (`idle 42s`); others are
  `running`, `stopped`, `exited`, `failed`. A trailing `!` means it wants attention.
- Flags may go anywhere; use `--` before text that starts with a dash: `mmux send s3 -- --help`.
- `keys`: a recognised key name is pressed, any other word is typed as text.
- `--prompt` is for `new agent`, `--cmd` for `new terminal`; the other combination is refused.
- **Avoid pagers in terminals you drive.** A command that opens `less` swallows every later `send`.
  Use `git --no-pager log`, or prefix the command: `mmux new terminal --cmd "PAGER=cat git log -5"`.
- **`stop` on an agent or terminal closes it** (same as `close`). Only processes stop in place.
- Durations: `500ms`, `1.5s`, `90s`, `10m`, `2h`, or bare seconds.

## Workflows

**Read a dev server's errors**
```sh
mmux ls                              # find the process row
mmux read "Dev server" -n 150        # plain text, no screenshots needed
```

**Restart a process after a config change**
```sh
mmux restart "Dev server" && sleep 3 && mmux read "Dev server" -n 40
```

**Delegate to a helper agent and get its answer**
```sh
mmux ask "summarize what changed on this branch"          # new agent in this project
mmux ask --agent codex -p api "why does /health 500?"     # specific template + project
git diff | mmux ask -                                     # prompt from stdin
mmux ask --to s14 "now write the fix"                     # follow-up, same conversation
mmux ask --close "one-off question"                       # close the helper afterwards
```
The helper's id goes to stderr (`mmux: asked s14 Claude #3 — waiting for its reply`) so you can
follow up with `--to`. It stays in the sidebar unless `--close`. On timeout (`-t`, default 10m) `ask`
exits 2 and leaves it working: `mmux wait s14 && mmux last s14` picks it back up.

**Long waits: run them in the background**

`ask` and `wait` block until the helper is done, which can take many minutes. Your harness's
foreground command limit is usually shorter (Claude Code: 2 minutes by default, 10 at most), and a
blocking call holds up your turn. If your harness can run a command in the background and wake you
when it exits (Claude Code: Bash with `run_in_background: true`), do that, with a `-t` longer
than the work. You stay free to work, and the helper's reply arrives as the command's output. If a
blocking call gets killed anyway, the helper keeps working: `mmux wait <id> && mmux last <id>`
picks it back up.

```sh
# Second opinion while you keep working: one background command, woken with the review.
# (stderr names the helper, e.g. s14, for a later `ask --to s14`.)
mmux ask -t 1h --agent codex "review src/parser.rs for edge cases; list concrete bugs"

# Fan out: start helpers (each prints its id), then one background wait per helper,
# so you're woken by each one as it finishes, with its reply.
mmux new agent claude --prompt "add tests for src/parser.rs"           # s14 Claude #3 — started …
mmux new agent codex --prompt "document the new --strict flag in docs/"   # s15 Codex #1 — started …
mmux wait s14 -t 1h && mmux last s14     # background
mmux wait s15 -t 1h && mmux last s15     # background
```

**Step by step instead of `ask`**
```sh
mmux new agent claude --prompt "add tests for src/parser.rs"   # prints: s14 Claude #3 — started …
mmux wait s14 -t 20m                 # long: in the background (see above)
mmux last s14
```

**Parallel work in an isolated checkout**
```sh
mmux worktree new fix-auth --prompt "fix the login redirect bug, commit when done"
# prints: created ⑂ fix-auth … / project: fix-auth <checkout dir> / agent: s15 Claude #1
mmux wait s15 -t 30m && mmux last s15     # in the background
mmux worktree rm fix-auth            # after it's merged or pushed
```
The `project:` line (`--json`: `data.project.dir`) is the checkout, for your own `git -C`. A
worktree is its own project named after its branch: `-p fix-auth`, `fix-auth/Claude #1`. Omit
the branch for a generated name. The branch is deleted on `rm` only if it's merged; unmerged
commits stay on the kept branch. `rm` refuses uncommitted changes, an agent at work, or the
worktree the human is looking at, unless `--force` (which discards uncommitted changes), and never
removes the worktree you run in. Merging is up to you (`git merge`/`gh pr create`).

**Check on sibling agents**
```sh
mmux ls                              # who is working / idle / wants attention
mmux status s9                       # what is on its screen right now
```
"Done" (for `wait`/`ask`) means the agent stopped working on what it was sent — for Claude, its
transcript must also show that turn at rest. Input is `send`, a first prompt, or `keys` that include
`Enter`/`C-m`/`C-j`; input it never visibly starts on stops holding `wait` after 20 s. An agent that
stopped to ask a question or a permission prompt is also done, so check `mmux status` and answer
with `send`/`keys` if needed.

## Scripting: `--json` and exit codes

Every verb takes `--json` and prints `{"ok": true, "data": …}` or `{"ok": false, "error": "…"}` —
usage errors (missing target or prompt, bad `--cmd`/`--prompt` use) included.
- `ls` → `data.projects[]` (`name`, `dir`, `active`, `agents`, `worktree_of`) and `data.sessions[]`.
- `worktree new`/`rm` → `project` (as in `ls`), `branch`, `message`, and `agent` (a session) if one
  was started.
- A session: `id`, `kind`, `name`, `project`, `project_dir`, `status`, `working`, `attention`,
  `title`, `error`, `idle_for_ms`, `input_age_ms`, `worked_since_input`, `input_pending`.
- `status` adds `status_line[]`; `read` → `id`, `name`, `text`; `last`/`ask` → `id`, `name`,
  `reply`, `source` (`transcript` or `screen`); actions → the session plus `message`.

Exit codes: `0` success · `1` error or refusal (bad target, not running, busy, depth limit…) ·
`2` `wait`/`ask` timed out.

## Etiquette and safety

- **The human sees everything.** Actions never move their cursor or focus, but each one flashes a
  `ctl:` note in their footer and new sessions appear in their sidebar. Act like a guest.
- **Only close, stop, remove or `--force` what you started** (or were asked to). Never `--force` close an
  agent that is working for someone else. Don't `stop` agents or terminals: it closes them.
- **Don't type into sessions you don't own** (especially the one the human is working in) without
  a reason. Prefer read-only verbs: `ls`, `status`, `read`, `last`.
- **Prefer `read`/`last` over screenshots or scraping**; they return plain text.
- **Depth limit.** `new` (and `ask` without `--to`) is refused at `MMUX_DEPTH` 3: agents can start
  helpers, helpers can start one more level, no further. Don't try to work around it.
- **Clean up.** Use `ask --close` for throwaway questions, or `close` helpers when finished,
  unless the human may want to read them.
- **Refusals are policy.** If you get "control from inside mmux panes is off here", the project set
  `control: from-panes: false`; stop driving mmux there. "did not answer in time" means the TUI is
  paused (e.g. native copy mode); retry later.
