#!/bin/sh
# mmux installer — https://mmux.org/install.sh
#
#   curl -fsSL https://mmux.org/install.sh | sh
#
# Downloads the right prebuilt binary for this machine (macOS arm64/x86_64, or Linux
# arm64/x86_64 as a static musl build) from the GitHub Releases page, verifies its
# checksum, and drops it on your PATH. No sudo: it installs to ~/.local/bin by default.
#
# A binary installed this way keeps itself up to date automatically — mmux checks for new
# releases in the background and swaps itself in place (see https://mmux.org docs).
#
# Environment overrides:
#   MMUX_BIN_DIR   install location (default: ~/.local/bin)
#   MMUX_VERSION   install a specific version, e.g. 0.8.1 (default: latest)
set -eu

REPO="marvinvr/mmux"
BIN_DIR="${MMUX_BIN_DIR:-$HOME/.local/bin}"

info() { printf 'mmux: %s\n' "$1"; }
warn() { printf 'mmux: warning: %s\n' "$1" >&2; }
err() {
	printf 'mmux: error: %s\n' "$1" >&2
	exit 1
}

command -v curl >/dev/null 2>&1 || err "curl is required but not found"
command -v tar >/dev/null 2>&1 || err "tar is required but not found"

# Map this machine to one of the shipped release targets.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
Darwin)
	case "$arch" in
	arm64 | aarch64) target="aarch64-apple-darwin" ;;
	x86_64) target="x86_64-apple-darwin" ;;
	*) err "unsupported macOS architecture: $arch" ;;
	esac
	;;
Linux)
	case "$arch" in
	aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
	x86_64 | amd64) target="x86_64-unknown-linux-musl" ;;
	*) err "unsupported Linux architecture: $arch" ;;
	esac
	;;
*)
	err "unsupported OS: $os (mmux ships macOS and Linux binaries; build from source otherwise)"
	;;
esac

# Resolve the version: an explicit MMUX_VERSION, else the tag the "latest release" URL
# redirects to (a plain web redirect — no API token, no rate limit).
if [ -n "${MMUX_VERSION:-}" ]; then
	version="${MMUX_VERSION#v}"
else
	latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")"
	version="${latest_url##*/}" # …/releases/tag/vX.Y.Z
	version="${version#v}"
fi
case "$version" in
[0-9]*) : ;;
*) err "could not determine the latest version (got '$version')" ;;
esac

asset="mmux-${target}.tar.gz"
base="https://github.com/$REPO/releases/download/v${version}"

info "installing v${version} (${target}) to ${BIN_DIR}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

curl -fsSL "$base/$asset" -o "$tmp/$asset" || err "download failed: $base/$asset"

# Verify against the release's checksums.txt when a sha tool is available. Best-effort:
# skipped (with a note) if neither is present, since the HTTPS download already
# authenticates the bytes.
if curl -fsSL "$base/checksums.txt" -o "$tmp/checksums.txt" 2>/dev/null; then
	expected="$(grep " ${asset}\$" "$tmp/checksums.txt" | awk '{print $1}')"
	if [ -n "$expected" ]; then
		if command -v sha256sum >/dev/null 2>&1; then
			actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
		elif command -v shasum >/dev/null 2>&1; then
			actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
		else
			actual=""
			warn "no sha256 tool found; skipping checksum verification"
		fi
		if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
			err "checksum mismatch for $asset (expected $expected, got $actual)"
		fi
	fi
fi

tar -xzf "$tmp/$asset" -C "$tmp" || err "failed to extract $asset"
[ -f "$tmp/mmux" ] || err "archive did not contain the mmux binary"

mkdir -p "$BIN_DIR"
chmod 755 "$tmp/mmux"
# A relocated ad-hoc-signed macOS binary gets SIGKILL'd ("Killed: 9") on first run unless
# re-signed in place.
if [ "$os" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
	codesign --force --sign - "$tmp/mmux" 2>/dev/null || true
fi
mv "$tmp/mmux" "$BIN_DIR/mmux"

info "installed $BIN_DIR/mmux"

# Nudge, don't fail, on the two things that make the first run smoother.
case ":$PATH:" in
*":$BIN_DIR:"*) : ;;
*) warn "$BIN_DIR is not on your PATH — add it, e.g.  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
command -v tmux >/dev/null 2>&1 || warn "tmux is not installed — mmux needs it to run (install it with your package manager)"

info "done — run 'mmux' to start"
