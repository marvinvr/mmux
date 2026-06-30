# Configuration

mmux is configured with YAML. There are two files, merged at launch — plus an optional private
[`mmux.local.yml`](#local-overrides--mmuxlocalyml) override for the project.

## The Two Files

| File | Purpose |
| --- | --- |
| `~/.mmux/config.yaml` | **Global defaults** — the things you want in every project, typically your agents. |
| `./mmux.yaml` | **Per-project** — this directory's processes, name, and links. `./mmux.yml` is also accepted (`.yaml` wins if both exist). |

Either file alone is enough. If **neither** exists when you run `mmux`, it treats the directory
as a first run and launches the [`mmux init`](01-quick-start.md#2-set-up-a-project) wizard.

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
```

## Field Reference

### Top level

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string | Workspace label; shown in the sidebar and the terminal's tab title. Defaults to the directory basename. |
| `agents` | list | [Agent](#agent) templates you spawn on demand. |
| `processes` | list | [Process](#process) definitions you start/stop and watch. |
| `git-panel` | map | [Git panel](#git-panel) settings. |
| `notifications` | map | [Notification](05-notifications.md) settings. |
| `auto-update` | map | [Self-update](#auto-update) settings (Homebrew installs only). |
| `linked-projects` | list of paths | Sibling directories to open in one sidebar. Honored **only** in the launch directory's config. |

### Agent

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string, **required** | Label shown in the sidebar. |
| `cmd` | string, **required** | Executable on your `PATH`. |
| `args` | list of strings | Defaults to `[]`. |
| `cwd` | string | Relative to the config file's directory; defaults to the project directory. |
| `env` | map | Environment overrides. |

### Process

Same as [Agent](#agent), plus:

| Field | Type | Notes |
| --- | --- | --- |
| `autostart` | bool | Start automatically when mmux first opens. Defaults to `false`. Honored only in the launch directory (linked projects' processes start stopped). |

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

When mmux was installed with Homebrew (the only install path it ships today), it keeps itself
current in the background:

- **On startup, and every 6 hours** thereafter (sessions can run for days), it checks for a newer
  release. The timer runs from each session's startup, so independent sessions stagger their checks
  rather than all hitting the tap at once. First it cheaply notices if a **sibling mmux session
  already upgraded** the on-disk
  binary; otherwise it checks the [tap formula](https://github.com/marvinvr/homebrew-mmux) with a
  single lightweight request — it does **not** run `brew update`.
- **If a newer version exists, it installs it in the background** (`brew update` + `brew upgrade
  mmux`). This only swaps the on-disk binary; nothing you're running is disturbed.
- **Once the update is staged, a quiet `↻ restart to update` badge** appears in the bottom-right.
  Press **`U`** (in the sidebar) or click the badge to restart **in place** onto the new version —
  no need to quit and relaunch. It's never automatic, so a long task is yours to interrupt when
  convenient.
- **To check or apply on demand,** open the [About card](03-usage.md#the-about-card) with **`?`**:
  it shows the running version and the live update status, with `c` to check now and `u` to apply a
  staged update.
- **The restart brings your work back.** Replacing the running process ends the live panes, but the
  new one restores them the same way reopening a directory does — Claude/Codex agents resume their
  conversation, terminals reopen where you left them — so applying an update doesn't cost you your
  place. See [Session restore](03-usage.md#detaching-reattaching-and-the-attach-picker).

It is **inert** for non-Homebrew installs (e.g. `cargo install`) and dev builds — no badge, no
network calls. Turn it off per-project or globally with `auto-update: { enabled: false }`, or for a
single run with the `MMUX_NO_UPDATE` environment variable.

## Linked Projects

Working in several clones of a repo at once (`./app`, `../app2`, `../app3` — the
"clones instead of worktrees" setup)? List the siblings under `linked-projects` and they all open
in **one** mmux, each as its own group in the sidebar:

```yaml
# in ./app/mmux.yaml
linked-projects:
  - ../app2
  - ../app3
```

- Switch between projects with `[` and `]`. The git panel **follows the active project** — when
  you select a row in `app2`, the panel shows `app2`'s git — and each project's panel stays alive
  in the background, so switching back is instant.
- Paths are resolved relative to the config file. Loading is **one level deep** and
  **de-duplicated by canonical path**, so you can drop the *same* config into every clone (even
  one that lists itself) and it will never expand recursively.
- At most **8 projects** load in total (the launch directory plus up to 7 links). A missing,
  unreadable, or over-the-cap sibling is skipped with a warning; only the launch directory
  failing to load aborts startup.
- The launch directory is always the first group, so opening mmux from any clone keeps "where you
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
- edited commands take effect on the next start;
- a process whose definition you removed keeps running as an "orphan" rather than being killed;
- the git panel is gained or lost if the directory's repo status changed;
- a one-line footer flash summarizes what changed.

Reload refreshes each *already-loaded* project in place. It does **not** re-read the
`linked-projects` list to *drop* a project — only [linking another project](#linked-projects) grows
the workspace live; removing one needs a reopen.

## Adding a Process From the TUI

You don't have to hand-edit YAML to add a process. The `+ New Process` launcher opens a
[guided form](03-usage.md#adding-a-process) that appends the entry to your `mmux.yaml`,
preserving the file's existing comments and layout, then reloads. (It can't set `env`, though —
that still needs a hand edit.)

Likewise, the `+ Link another project` button (or `L`) writes a new entry into the launch
directory's `linked-projects:` — same comment-preserving append — and adds the project to the live
workspace; see [Linking Another Project](03-usage.md#linking-another-project).
