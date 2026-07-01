# Installation

## Prerequisites

| Tool | Required? | Used for |
| --- | --- | --- |
| **tmux** | Yes | mmux runs its TUI inside a per-directory tmux session. Without `tmux` on your `PATH`, `mmux` prints an error and exits. |
| **git** | For the git panel | The built-in [git panel](03-usage.md#the-git-panel) shells out to the `git` CLI. It only appears when the directory is a git repository. |
| an editor | Optional | The [Ctrl+P file picker](03-usage.md#the-file-picker) opens the chosen file with `$VISUAL`/`$EDITOR`, falling back to the first of `micro`, `nano`, `vim`, `vi` on `PATH`. |

The agents and processes you configure (`claude`, `codex`, `npm`, …) need to be installed
separately — mmux just launches them.

## Homebrew

```sh
brew install marvinvr/mmux/mmux
```

This installs a prebuilt binary on macOS (arm64 / x86_64) and Linux (x86_64). Other platforms
fall back to building from source, which pulls in Rust automatically.

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

## Prebuilt Binaries

Every tagged release attaches per-platform tarballs to the
[GitHub Releases](https://github.com/marvinvr/mmux/releases) page:

- `aarch64-apple-darwin` (macOS arm64)
- `x86_64-apple-darwin` (macOS Intel)
- `x86_64-unknown-linux-gnu` (Linux x86_64)

Download, extract the `mmux` binary onto your `PATH`, and (on macOS) re-sign it as shown above.

## Verify

```sh
mmux --version
mmux check        # validates your config without launching the TUI
```
