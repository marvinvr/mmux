# Installation

## Prerequisites

| Tool | Required? | Used for |
| --- | --- | --- |
| **tmux** | Yes | mmux runs its TUI inside a per-directory tmux session. Without `tmux` on your `PATH`, `mmux` prints an error and exits. |
| **git** | For the git panel | The built-in [git panel](03-usage.md#the-git-panel) shells out to the `git` CLI. It only appears when the directory is a git repository. |
| an editor | Optional | The [Ctrl+P file picker](03-usage.md#the-file-picker) opens the chosen file with `$VISUAL`/`$EDITOR`, falling back to the first of `micro`, `nano`, `vim`, `vi` on `PATH`. |

The agents and processes you configure (`claude`, `codex`, `npm`, …) need to be installed
separately — mmux just launches them.

## Install Script (recommended)

Works on macOS and Linux, arm64 and x86_64:

```sh
curl -fsSL https://mmux.org/install.sh | sh
```

It downloads the right prebuilt binary for your machine (a static [musl](https://musl.libc.org/)
build on Linux, so it runs on any distro regardless of glibc version), verifies its checksum, and
installs it to `~/.local/bin` — no `sudo`. On macOS it re-signs the binary for you (see the
[code-signing note](#from-source) below).

A binary installed this way **keeps itself up to date**: mmux checks for new releases in the
background and swaps itself in place, showing a quiet `↻ restart to update` badge when a new
version is staged. See [Auto-Update](04-configuration.md#auto-update).

Two environment overrides:

| Variable | Effect |
| --- | --- |
| `MMUX_BIN_DIR` | Install to a different directory (default `~/.local/bin`). |
| `MMUX_VERSION` | Install a specific version, e.g. `MMUX_VERSION=0.8.1` (default: latest). |

If `~/.local/bin` isn't on your `PATH`, the script prints the line to add.

## Homebrew (macOS)

```sh
brew install marvinvr/mmux/mmux
```

This installs a prebuilt binary on macOS (arm64 / x86_64). A brew install notifies you when a new
version is out and applies it with `brew upgrade mmux` once you confirm (see
[Auto-Update](04-configuration.md#auto-update)). On Linux, `brew` builds mmux from source (pulling
in Rust automatically) — prefer the install script there.

## From Source

Requires [Rust](https://rustup.rs).

```sh
cargo install --path .
```

`cargo install` handles code-signing on macOS for you. If you instead build and copy the binary
yourself, **re-sign it after copying** — macOS sends a freshly `cp`'d ad-hoc-signed binary
`SIGKILL` ("Killed: 9", exit 137) on first run:

```sh
cargo build --release
cp target/release/mmux ~/.local/bin/mmux
codesign --force --sign - ~/.local/bin/mmux   # macOS only; avoids "Killed: 9"
```

Source builds don't self-update (they carry no release marker) — pull and rebuild to upgrade.

## Prebuilt Binaries

Every tagged release attaches per-platform tarballs (and a `checksums.txt`) to the
[GitHub Releases](https://github.com/marvinvr/mmux/releases) page:

- `aarch64-apple-darwin` (macOS arm64)
- `x86_64-apple-darwin` (macOS Intel)
- `aarch64-unknown-linux-musl` (Linux arm64, static)
- `x86_64-unknown-linux-musl` (Linux x86_64, static)

The [install script](#install-script-recommended) picks the right one automatically. To do it by
hand, download and extract the `mmux` binary onto your `PATH`, and (on macOS) re-sign it as shown
above.

## Verify

```sh
mmux --version
mmux check        # validates your config without launching the TUI
```
