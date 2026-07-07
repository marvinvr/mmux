# Quick Start

This gets you from nothing to a running mmux session. For prerequisites and other install
methods, see [Installation](02-installation.md).

## 1. Install

macOS (Apple Silicon) & Linux (arm64 & x86_64):

```sh
curl -fsSL https://mmux.org/install.sh | sh
```

On macOS you can use Homebrew instead (`brew install marvinvr/mmux/mmux`), or build from source
with [Rust](https://rustup.rs) (`cargo install --path .`).

mmux needs **tmux** on your `PATH`. The [git panel](03-usage.md#the-git-panel) uses the `git`
CLI, and the [Ctrl+P file picker](03-usage.md#the-file-picker) opens files in your `$EDITOR`.

## 2. Set Up a Project

```sh
cd ~/some/project
mmux init
```

`mmux init` is an interactive wizard: it offers the built-in agent presets — Claude, Codex,
Gemini, Amp, opencode, and Grok — as an **inline checkbox picker** you arrow through (`space` to
toggle, `d` to flip danger mode, `⏎` to confirm; installed ones start checked), asks for any start
commands you want to watch, and lets you list other projects you want in the same workspace as
[linked projects](04-configuration.md#linked-projects). On a machine with no global config yet,
your agents are saved to `~/.mmux/config.yaml` so they are available in every project; the rest
goes in this project's `mmux.yaml`.

To change your agents later, run **`mmux agents`** (the same picker, agents only) or press **`a`**
in the sidebar — both write to the global config and take effect on the next open / reload.

You can skip the wizard and write [`mmux.yaml`](04-configuration.md) by hand, or rely entirely on
a global config — either file alone is enough.

## 3. Open It

```sh
mmux
```

You land in the TUI. The sidebar lists your agents, terminals, and processes; the right column
shows git if the directory is a repository.

- Move with `↑`/`↓` (or `j`/`k`).
- Press `Enter` on a `+ New …` row to add an agent or terminal (a process opens a short form first).
- Press `Enter` on a running row to jump into its pane; `Ctrl-b h` returns to the sidebar.
- Press `Tab` to focus the git panel.
- Press `d` to detach (the session keeps running) or `q` to quit.

Run `mmux` again in the same directory to reattach. The full key reference is in
[Usage](03-usage.md).

## 4. Reattach From Anywhere

```sh
mmux a        # alias for `mmux attach`
```

This opens a picker of every running mmux session on the machine, plus directories you have
opened recently. Choose one to jump straight back in — handy after reconnecting over SSH.

## Validate Without Launching

```sh
mmux agents   # add/remove the built-in agent harnesses in your global config
mmux check    # print the effective merged config, no TUI
mmux docs     # print a self-contained setup & config guide
```
