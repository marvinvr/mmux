#!/bin/sh
# ci/boot-test.sh <mmux-binary>
#
# Best-effort TUI boot smoke: actually start the interface under a real pseudo-terminal and
# confirm it draws a frame without panicking, then tear it down. This is the one check that
# exercises the TUI itself (ratatui init, terminal setup, the render loop) rather than just
# the CLI subcommands. It's deliberately kept NON-BLOCKING in CI — TTY tests are the most
# flake-prone — so a failure here is a signal to investigate, not a release gate (yet).
#
# We run `mmux --inner`, which renders the TUI directly, so there's no nested-tmux problem:
# the tmux session here exists only to hand mmux the pseudo-terminal it needs.
set -eu

BIN="${1:?usage: boot-test.sh <mmux-binary>}"
case "$BIN" in /*) : ;; *) BIN="$(pwd)/$BIN" ;; esac
[ -x "$BIN" ] || { echo "boot-test: $BIN is not an executable file" >&2; exit 1; }

if ! command -v tmux >/dev/null 2>&1; then
	echo "boot-test: tmux not found; skipping (not a failure)"
	exit 0
fi

dir="$(mktemp -d)"
sess="mmux-boot-$$"
# shellcheck disable=SC2329  # invoked indirectly via the trap below
cleanup() {
	tmux kill-session -t "$sess" 2>/dev/null || true
	rm -rf "$dir"
}
trap cleanup EXIT INT TERM

# MMUX_DIR points the inner TUI at an empty scratch dir (no agents to restore, no git panel),
# so it just comes up on an empty sidebar and rests — exactly enough to prove it renders.
MMUX_DIR="$dir" tmux new-session -d -s "$sess" -x 200 -y 50 "MMUX_DIR='$dir' MMUX_INNER=1 '$BIN' --inner"

i=0
while [ "$i" -lt 15 ]; do
	sleep 1
	pane="$(tmux capture-pane -t "$sess" -p 2>/dev/null || true)"
	case "$pane" in
	*panicked*|*RUST_BACKTRACE*|*"thread 'main'"*)
		echo "boot-test: the TUI panicked on start:" >&2
		printf '%s\n' "$pane" >&2
		exit 1
		;;
	esac
	# Any non-whitespace content means a frame was drawn.
	if printf '%s' "$pane" | tr -d ' \n\r\t' | grep -q .; then
		echo "boot-test: TUI rendered a frame ✓"
		exit 0
	fi
	i=$((i + 1))
done

echo "boot-test: the TUI did not render within the timeout. Last pane capture:" >&2
tmux capture-pane -t "$sess" -p 2>/dev/null || true
exit 1
