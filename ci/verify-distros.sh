#!/usr/bin/env bash
# ci/verify-distros.sh <version>
#
# Prove the static musl Linux binary runs across the distro spread that matters, by running
# the real install script inside each and checking `mmux --version`. Alpine has musl libc
# and no glibc at all; Ubuntu 20.04 has an old glibc — those two are the whole point (they'd
# break a glibc-linked binary). Architecture follows the host runner (run this on both the
# x86_64 and arm64 Linux runners to cover both).
#
# Source of the installer:
#   - default: the repo's own web/install.sh (the version being released), mounted read-only.
#   - MMUX_INSTALL_URL set: fetch that URL instead (the healthcheck points this at the LIVE
#     https://mmux.org/install.sh to test the deployed script).
set -euo pipefail

VERSION="${1:?usage: verify-distros.sh <version>}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

command -v docker >/dev/null 2>&1 || { echo "verify-distros: docker is required" >&2; exit 1; }

# Retry a command a few times with backoff — image pulls and in-container package installs are
# the flakiest part, and (via the healthcheck) a spurious failure here can auto-yank a healthy
# release. Real breakage still fails after the retries; a transient blip self-heals.
retry() {
	n=0
	max=3
	until "$@"; do
		n=$((n + 1))
		[ "$n" -ge "$max" ] && return 1
		echo "  (attempt $n failed; retrying in $((n * 5))s)" >&2
		sleep $((n * 5))
	done
}

# The in-container logic is a FIXED string; everything that varies (the package-install line,
# the version, the installer URL) is passed via -e env, so there's no fragile interpolation.
run() {
	image="$1"
	setup="$2"
	echo "=== $image ==="
	retry docker run --rm \
		-v "$REPO_ROOT:/src:ro" \
		-e MMUX_VERSION="$VERSION" \
		-e MMUX_INSTALL_URL="${MMUX_INSTALL_URL:-}" \
		-e SETUP="$setup" \
		"$image" sh -euc '
			eval "$SETUP"
			if [ -n "$MMUX_INSTALL_URL" ]; then
				curl -fsSL "$MMUX_INSTALL_URL" | MMUX_BIN_DIR=/usr/local/bin sh
			else
				MMUX_BIN_DIR=/usr/local/bin sh /src/web/install.sh
			fi
			got="$(mmux --version)"
			[ "$got" = "mmux $MMUX_VERSION" ] || { echo "version mismatch: $got != mmux $MMUX_VERSION" >&2; exit 1; }
		'
	echo "OK: $image"
}

run alpine:latest       "apk add --no-cache curl tar >/dev/null"
run ubuntu:20.04        "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq curl tar ca-certificates >/dev/null"
run debian:stable-slim  "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq curl tar ca-certificates >/dev/null"
run fedora:latest       "dnf install -y -q tar >/dev/null 2>&1 || true"

echo "verify-distros: all distros passed"
