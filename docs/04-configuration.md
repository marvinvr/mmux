# Configuration

mmux is configured with YAML. There are two files, merged at launch — plus an optional private
[`mmux.local.yml`](#local-overrides--mmuxlocalyml) override for the project.

## The Two Files

| File | Purpose |
| --- | --- |
| `~/.mmux/config.yaml` | **Global defaults** — the things you want in every project, typically your agents. |
| `./mmux.yaml` | **Per-project** — this directory's processes, name, and links. `./mmux.yml` is also accepted (`.yaml` wins if both exist). |

Either file alone is a valid config. But the **global** file is what the first-run check looks
for: if `~/.mmux/config.yaml` doesn't exist when you run `mmux` — even when a project `./mmux.yaml`
is already present — it treats this as a first run and launches the
[`mmux init`](01-quick-start.md#2-set-up-a-project) wizard, since a project file may define
processes but no agents.

Run `mmux check` to print the effective merged config without launching the TUI, or `mmux docs`
for a self-contained guide printed straight to the terminal (handy for humans and AI agents
setting up a config).

## How the Merge Works

The project file is layered on top of the global one, and **project values win**:

- `name`, `git-panel`, `notifications`, and `auto-update` — the project's value replaces the
  global one **wholesale** if set. (There is no field-level merge: a project `notifications:` block
  that sets only `enabled: true` does **not** inherit the global `mechanism`/`throttle_secs` — unset
  sub-fields fall back to their built-in defaults, not to the global value.)
- `agents` and `processes` — merged **by name**: a project entry with the same `name` replaces
  the global one; otherwise it is appended.
- `linked-projects` — the project's list, or the global's if the project lists none.
- A relative `cwd` always resolves against the **project** directory — even for an agent or
  process defined in the global config. So a global `claude` agent runs in whatever project you
  opened, not in `$HOME`.

## Local Overrides — `mmux.local.yml`

Alongside the project file you can drop a **`mmux.local.yml`** (or `mmux.local.yaml`; `.yaml`
wins if both exist) — a private, per-developer override, layered on top of `mmux.yaml`. It's the
file you keep out of git (add it to `.gitignore`) for machine-specific tweaks: disabling
notifications on your laptop, pointing an agent at a beta binary, adding a process only you run.

It exists **only** for the project file — there is **no** global counterpart, by design.

Unlike the wholesale global→project merge above, the local file is merged **deeply**, so it
overrides exactly what it names and leaves everything else intact:

- **Nested maps merge key by key.** A local `notifications:` block that sets only
  `enabled: false` keeps the project's `mechanism`, `throttle_secs`, etc. — the opposite of the
  global→project rule, where an unset sub-field falls back to its built-in default.
- **`agents` and `processes` merge by `name`**, just like the global merge — and a same-named
  entry is itself merged field-by-field, so a partial override (e.g. just new `args` for `Claude`)
  needn't restate `cmd`. A name not already present is appended.
- **Plain lists** (`args`, `linked-projects`) and scalars are **replaced** wholesale. An empty
  list (`processes: []`) therefore *clears* the project's — a handy way to say "none here".

The layering order is: global → project → local, so a local value wins over both. Run
`mmux check` to print the fully merged result, and `R` ([live reload](#live-reload)) picks up
local edits without dropping panes.

```yaml
# ./mmux.local.yml — git-ignored; just your overrides
notifications:
  enabled: false        # quiet on this machine; mechanism/throttle inherited

agents:
  - name: Claude        # tweak one field; cmd is inherited from mmux.yaml
    args: ["--model", "claude-opus-4-8"]
```

## Example

`~/.mmux/config.yaml` (global defaults — your agents everywhere):

```yaml
agents:
  - name: Claude
    cmd: claude
    args: ["--dangerously-skip-permissions"]
  - name: Codex
    cmd: codex
    args: ["--dangerously-bypass-approvals-and-sandbox"]
```

`./mmux.yaml` (per-project — just this directory's bits):

```yaml
# name is optional; it defaults to the directory's name.
name: my-workspace

processes:
  - name: Dev server
    cmd: npm
    args: ["run", "dev"]
    cwd: .            # relative to this file
    autostart: false  # start automatically when mmux opens?
    # stop: docker compose down   # optional: run in cwd when stopped or on quit
```

The optional **`stop`** is a teardown for a process that leaves something running behind
it — a Docker stack, a tunnel, a background daemon. mmux runs it (via `sh -c`, in the
process's `cwd`) **after the process stops**: when you press `x` on a running one, and for
every still-running process when you quit mmux (so a `docker compose up` gets its
`docker compose down`). It does **not** run on a [restart](03-usage.md#the-sidebar) (`r`)
— the process is coming right back — nor for a process that was already stopped. On a
manual stop it runs in the background; on quit mmux waits for it to finish before the tmux
session goes away.

## Field Reference

### Top level

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string | Workspace label; shown in the sidebar and the terminal's tab title. Defaults to the directory basename. |
| `agents` | list | [Agent](#agent) templates you spawn on demand. |
| `processes` | list | [Process](#process) definitions you start/stop and watch. |
| `git-panel` | map | [Git panel](#git-panel) settings. |
| `notifications` | map | [Notification](05-notifications.md) settings. |
| `auto-update` | map | [Self-update](#auto-update) settings (Homebrew + script-installed binaries). |
| `linked-projects` | list of paths | Other project directories to open together in one sidebar — not just clones, any related project. Honored **only** in the launch directory's config. |

### Agent

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string, **required** | Label shown in the sidebar. |
| `cmd` | string, **required** | Executable on your `PATH`. |
| `args` | list of strings | Defaults to `[]`. |
| `cwd` | string | Relative to the config file's directory; defaults to the project directory. |
| `env` | map | Environment overrides. |

**Built-in presets.** mmux ships presets for the common harnesses — **Claude** (`claude`),
**Codex** (`codex`), **Gemini** (`gemini`), **Amp** (`amp`), **opencode** (`opencode`), and
**Grok** (`grok`, xAI's Grok Build) — each with the flag that opts it out of permission/approval
prompts ("danger mode"). The [`mmux init`](01-quick-start.md#2-set-up-a-project) wizard offers them
as an **inline checkbox picker** — arrow keys to move, `space` to toggle, `d` to flip danger mode,
`a` for all/none, `⏎` to confirm (installed harnesses start pre-checked) — and whatever you pick
seeds your **global** config so it's available everywhere.

**Managing agents.** Two ways, both editing the **global** `~/.mmux/config.yaml` (the natural home
for agents you reuse across projects) and preserving any non-preset agents you added by hand:

- **In the TUI:** press **`a`** in the sidebar for the agent manager — a popup of every preset with
  a checkbox, a `danger` tag, and a green `✓` on the ones found on your `PATH` (purely a hint — you
  can enable any of them). `space` toggles an agent on/off, `d` flips its danger flag, `⏎` saves and
  [reloads](#live-reload) so the sidebar updates immediately.
- **From the terminal:** run **`mmux agents`** — the same inline checkbox picker as the setup
  wizard (arrow keys · `space` · `d` · `a` · `⏎`), agents only. It takes effect the next time you
  open mmux (or press `R` inside it).

### Process

Same as [Agent](#agent), plus:

| Field | Type | Notes |
| --- | --- | --- |
| `autostart` | bool | Start automatically when mmux first opens. Defaults to `false`. Honored only in the launch directory (linked projects' processes start stopped). |
| `stop` | string | Optional teardown command run in the process's `cwd` **after it stops** — when you stop it (`x`) and when you quit mmux, but **not** on a restart. A shell line (run via `sh -c`), so `docker compose down` and friends work. Unset ⇒ nothing runs. |

### Git panel

| Field | Type | Notes |
| --- | --- | --- |
| `enabled` | bool | Defaults to `true`. Set to `false` to hide the panel. |

The [git panel](03-usage.md#the-git-panel) is built in and shown automatically whenever the
directory is a git repository. There is no command, width, or title to configure — its width
follows the left sidebar. The only knob is turning it off:

```yaml
git-panel:
  enabled: false
```

### Notifications

See [Notifications](05-notifications.md) for the full reference. Defaults are sensible (on,
`osc9`, unfocused-only, 5-second throttle), so most configs omit this block entirely.

### Auto-update

| Field | Type | Notes |
| --- | --- | --- |
| `enabled` | bool | Defaults to `true`. Set to `false` to turn background self-update off. |

See [Auto-Update](#auto-update) for how it behaves. The block only exists to turn it off:

```yaml
auto-update:
  enabled: false
```

## Auto-Update

When mmux was installed a way it can update — via the [install script](02-installation.md#install-script-recommended)
or [Homebrew](02-installation.md#homebrew-macos) — it keeps an eye out for new releases in the
background. Two behaviors, depending on how it was installed:

**On startup, and every 6 hours** thereafter (sessions can run for days), it checks for a newer
release. The timer runs from each session's startup, so independent sessions stagger their checks.
The version check follows the GitHub [`releases/latest`](https://github.com/marvinvr/mmux/releases/latest)
redirect — a single lightweight request, no API token. It first cheaply notices if a **sibling mmux
session already upgraded** the on-disk binary and skips straight to offering the restart.

- **Script-installed binaries update themselves silently.** When a newer version exists, mmux
  downloads the release tarball and swaps the binary in place (its own file is user-writable), then
  shows a quiet `↻ restart to update` badge in the bottom-right. Nothing you're running is
  disturbed.
- **Homebrew installs ask first.** mmux can't swap a brew-managed binary out from under brew, so it
  shows an `↻ update available` badge instead. Press **`U`** (or click it) and confirm, and mmux runs
  `brew upgrade mmux` for you in the background; once it's done, the badge becomes `↻ restart to
  update`. (If you run `brew upgrade mmux` yourself in another terminal, mmux notices and offers the
  restart too.)

Either way:

- **Applying is never automatic.** Press **`U`** (in the sidebar) or click the badge to restart **in
  place** onto the new version — no need to quit and relaunch. A long task is yours to interrupt when
  convenient.
- **To check or apply on demand,** open the [About card](03-usage.md#the-about-card) with **`?`**: it
  shows the running version and the live update status, with `c` to check now and `u` to apply.
- **The restart brings your work back.** Replacing the running process ends the live panes, but the
  new one restores them the same way reopening a directory does — Claude/Codex agents resume their
  conversation, terminals reopen where you left them — so applying an update doesn't cost you your
  place. See [Session restore](03-usage.md#detaching-reattaching-and-the-attach-picker).

It is **inert** for unmanaged installs — source builds (`cargo install`), binaries in a location you
can't write, and dev builds — with no badge and no network calls. Turn it off per-project or globally
with `auto-update: { enabled: false }`, or for a single run with the `MMUX_NO_UPDATE` environment
variable.

## Linked Projects

Linked projects are **any other projects you want open together** in one workspace — not just extra
clones. Common cases: several clones of a repo (`./app`, `../app2` — the "clones instead of
worktrees" setup), a related repo, a service you run alongside. List them under `linked-projects`
and they all open in **one** mmux, each as its own group in the sidebar:

```yaml
# in ./app/mmux.yaml
linked-projects:
  - ../app2      # another clone
  - ../api       # a related repo
  - ../docs      # anything you want side by side
```

- Switch between projects with `[` and `]`. The git panel **follows the active project** — when
  you select a row in `app2`, the panel shows `app2`'s git — and each project's panel stays alive
  in the background, so switching back is instant.
- Paths are resolved relative to the config file. Loading is **one level deep** and
  **de-duplicated by canonical path**, so you can drop the *same* config into every project (even
  one that lists itself) and it will never expand recursively.
- At most **8 projects** load in total (the launch directory plus up to 7 links). A missing,
  unreadable, or over-the-cap link is skipped with a warning; only the launch directory
  failing to load aborts startup.
- The launch directory is always the first group, so opening mmux from any project keeps "where you
  are" on top.
- You can **add** a link without restarting: the `+ Link another project` button at the bottom of
  the sidebar (or `L`) opens a [browser](03-usage.md#linking-another-project) that writes the chosen
  path into the launch directory's `linked-projects:` and loads it in place as a new sidebar box.
  **Removing** a link, or any other hand-edit to `linked-projects`, still takes effect only on the
  next `mmux` (a reopen), not on a [reload](#live-reload).

## Live Reload

Press `R` (or `Ctrl-b R`) to re-read every loaded project's `mmux.yaml` and the global config
**without losing running panes**:

- newly added processes and agents appear;
- an edited process command takes effect immediately: a process that's **running** is restarted
  onto the new command (a stopped one just picks it up on its next start);
- a process whose definition you removed keeps running as an "orphan" rather than being killed;
- the git panel is gained or lost if the directory's repo status changed;
- a one-line footer flash summarizes what changed (added / restarted / orphaned / unreadable).

Reload refreshes each *already-loaded* project in place. It does **not** re-read the
`linked-projects` list to *drop* a project — only [linking another project](#linked-projects) grows
the workspace live; removing one needs a reopen.

## Adding a Process From the TUI

You don't have to hand-edit YAML to manage processes. The `+ New Process` launcher opens a
[guided form](03-usage.md#adding-editing-and-deleting-a-process) that appends the entry to your
`mmux.yaml`, preserving the file's existing comments and layout, then reloads. If there's no
`mmux.yaml` yet (or it's empty), the first add writes a fully documented file — the same header,
`mmux docs` pointer, and commented example sections `mmux init` produces — rather than a bare
`processes:` block. The same form
**edits** an existing process (`e`), splicing the change back into its entry, and `D` **deletes**
one (with a confirmation) — both comment-preserving, both followed by a reload. (The form can't set
`env`, though — that still needs a hand edit.)

Likewise, the `+ Link another project` button (or `L`) writes a new entry into the launch
directory's `linked-projects:` — same comment-preserving append — and adds the project to the live
workspace; see [Linking Another Project](03-usage.md#linking-another-project).
