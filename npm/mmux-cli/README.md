# mmux-cli

A persistent terminal multiplexer for AI coding agents and dev processes — a sidebar of
agents, terminals, and processes plus a native git panel, running inside an invisible tmux
session so it survives detach and disconnect.

```sh
npm install -g mmux-cli
```

Prebuilt for macOS (Apple Silicon) and Linux (arm64, x86_64). npm installs only the binary for
your platform (an `mmux-<platform>` optional dependency) — no install scripts, no downloads.
mmux needs **tmux** on your `PATH`.

Update with `npm update -g mmux-cli` (an npm install never updates itself in place).

Other install options (install script, Homebrew, source) and full docs:
https://github.com/marvinvr/mmux
