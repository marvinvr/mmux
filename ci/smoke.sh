#!/bin/sh
# ci/smoke.sh <mmux-binary> <expected-version>
#
# The one shared smoke test, reused by every CI layer (build gate, release verify,
# scheduled healthcheck). It exercises only the non-TTY subcommands — the ones that run
# before mmux ever touches tmux — so it works on any runner or minimal container without a
# terminal. What it actually catches: a binary that won't execute at all (wrong arch, musl
# vs glibc, the macOS "Killed: 9" after relocation), and a broken config validator.
set -eu

BIN="${1:?usage: smoke.sh <mmux-binary> <expected-version>}"
EXPECT="${2:?usage: smoke.sh <mmux-binary> <expected-version>}"
case "$BIN" in /*) : ;; *) BIN="$(pwd)/$BIN" ;; esac
[ -x "$BIN" ] || { echo "smoke: $BIN is not an executable file" >&2; exit 1; }

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗ %s\033[0m\n' "$1" >&2; exit 1; }

echo "smoke-testing $BIN (expect v$EXPECT)"

# 1. It runs at all, and reports the version we shipped. This is the check that catches
#    virtually every distribution break.
out="$("$BIN" --version 2>/dev/null)" || fail "--version exited nonzero (binary won't run here)"
[ "$out" = "mmux $EXPECT" ] || fail "--version: got '$out', expected 'mmux $EXPECT'"
pass "--version = mmux $EXPECT"

# 2. --help renders.
"$BIN" --help 2>/dev/null | grep -q "USAGE" || fail "--help did not print USAGE"
pass "--help"

# 3. docs guide renders.
"$BIN" docs >/dev/null 2>&1 || fail "docs exited nonzero"
pass "docs"

# 4 + 5. The config validator accepts a valid file and rejects an invalid one.
d="$(mktemp -d)"
trap 'rm -rf "$d"' EXIT INT TERM

cat > "$d/mmux.yaml" <<'YAML'
agents:
  - name: shell
    cmd: bash
processes:
  - name: greet
    cmd: echo
    args: ["hi"]
YAML
( cd "$d" && "$BIN" check >/dev/null 2>&1 ) || fail "check rejected a valid config"
pass "check (valid → exit 0)"

cat > "$d/mmux.yaml" <<'YAML'
agents:
  - name: broken
YAML
if ( cd "$d" && "$BIN" check >/dev/null 2>&1 ); then
	fail "check accepted an invalid config (missing required 'cmd')"
fi
pass "check (invalid → nonzero)"

echo "smoke: all checks passed"
