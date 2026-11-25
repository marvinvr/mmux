#!/usr/bin/env node
// The `mmux-cli` npm package's entry point (installed as the `mmux` command). It ships no binary itself: npm installs exactly one
// `mmux-<platform>` optionalDependency (picked by its os/cpu fields) holding the prebuilt
// release binary, and this shim finds it and runs it. No postinstall, no downloads.
"use strict";

const { spawn } = require("child_process");

const DOCS = "https://github.com/marvinvr/mmux/blob/main/docs/02-installation.md";

// `${process.platform}-${process.arch}` → the package carrying that binary.
const PACKAGES = {
  "darwin-arm64": "mmux-darwin-arm64",
  "linux-x64": "mmux-linux-x64",
  "linux-arm64": "mmux-linux-arm64",
};

function fail(msg) {
  process.stderr.write(`mmux: ${msg}\n`);
  process.exit(1);
}

const key = `${process.platform}-${process.arch}`;
const pkg = PACKAGES[key];
if (!pkg) {
  let why = `no prebuilt npm binary for ${key}.`;
  if (key === "darwin-x64") {
    why +=
      " Intel Macs aren't shipped prebuilt — install via Homebrew or from source." +
      " (On an Apple-silicon Mac, this means Node is running under Rosetta: use an arm64 Node.)";
  } else if (process.platform === "win32") {
    why += " mmux runs on macOS and Linux only (it needs tmux); on Windows, use WSL.";
  }
  fail(`${why}\nSee ${DOCS}`);
}

let bin;
try {
  bin = require.resolve(`${pkg}/bin/mmux`);
} catch {
  fail(
    `the platform package ${pkg} is not installed. It is an optional dependency of mmux-cli, ` +
      `so it is skipped by --omit=optional / --no-optional. Reinstall with:\n` +
      `  npm install -g mmux-cli\nSee ${DOCS}`,
  );
}

const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });

// Signals sent to this shim's pid (`kill`, a process manager) reach the real binary too.
// Terminal-generated ones (Ctrl+C, hangup) already hit the whole foreground process group,
// so the shim just ignores them and lets mmux decide.
const forward = (sig) => () => child.kill(sig);
const handlers = { SIGTERM: forward("SIGTERM"), SIGINT: () => {}, SIGQUIT: () => {}, SIGHUP: () => {} };
for (const [sig, fn] of Object.entries(handlers)) process.on(sig, fn);

child.on("error", (e) => fail(`could not run ${bin}: ${e.message}`));
child.on("exit", (code, signal) => {
  if (signal) {
    if (signal === "SIGKILL" && process.platform === "darwin") {
      process.stderr.write(
        `mmux: the binary was killed by macOS. If this persists, re-sign it:\n` +
          `  codesign --force --sign - "${bin}"\n`,
      );
    }
    // Die the same way, so callers see the real signal rather than a plain exit code.
    for (const [sig, fn] of Object.entries(handlers)) process.removeListener(sig, fn);
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
