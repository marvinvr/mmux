#!/usr/bin/env bash
# scripts/npm-package.sh — assemble (and optionally publish) the npm packages from release
# tarballs. The npm channel is an ADDITIONAL install path; it repackages the exact binaries
# the release already built + verified, nothing is rebuilt.
#
#   scripts/npm-package.sh <dist-dir> <version> <out-dir> [--publish]
#
#   <dist-dir>  holds mmux-<target>.tar.gz for every target below (the release assets /
#               CI artifacts; `gh release download vX.Y.Z -D <dir>` works for a dry run)
#   <version>   package version, e.g. 0.13.19 (the tag without its `v`)
#   <out-dir>   staging dir: packages are assembled under <out-dir>/pkg/, tarballs packed
#               into <out-dir>/ (`npm pack`)
#   --publish   also `npm publish --provenance --access public` each one — platform packages
#               first, the main `mmux` package last (so its optionalDependencies already
#               resolve). A version that's already on the registry is skipped, so a re-run
#               of a half-finished publish just completes it.
#
# Layout (the esbuild/Biome pattern):
#   mmux-darwin-arm64, mmux-linux-x64, mmux-linux-arm64 — just the binary, gated by
#     os/cpu so npm installs only the matching one. No `libc` field: the Linux binaries are
#     static musl and run on glibc and musl distros alike, so restricting libc would only
#     wrongly exclude one of them.
#   mmux — npm/mmux/bin/mmux.js (resolves + runs the platform binary) with the platform
#     packages as exact-version optionalDependencies. No postinstall, no downloads.
set -euo pipefail

DIST="${1:?usage: npm-package.sh <dist-dir> <version> <out-dir> [--publish]}"
VERSION="${2:?usage: npm-package.sh <dist-dir> <version> <out-dir> [--publish]}"
OUT="${3:?usage: npm-package.sh <dist-dir> <version> <out-dir> [--publish]}"
PUBLISH="${4:-}"
[ -z "$PUBLISH" ] || [ "$PUBLISH" = "--publish" ] || { echo "unknown flag: $PUBLISH" >&2; exit 2; }
VERSION="${VERSION#v}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_URL="git+https://github.com/marvinvr/mmux.git"
HOMEPAGE="https://github.com/marvinvr/mmux"

# release target triple | npm package suffix | os | cpu
TARGETS="
aarch64-apple-darwin|darwin-arm64|darwin|arm64
x86_64-unknown-linux-musl|linux-x64|linux|x64
aarch64-unknown-linux-musl|linux-arm64|linux|arm64
"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
rm -rf "$OUT/pkg"
mkdir -p "$OUT/pkg"

order=() # package dirs, in publish order
optdeps=""

for row in $TARGETS; do
	IFS='|' read -r target suffix os cpu <<<"$row"
	tarball="$DIST/mmux-$target.tar.gz"
	[ -f "$tarball" ] || { echo "missing $tarball" >&2; exit 1; }
	name="mmux-$suffix"
	dir="$OUT/pkg/$suffix"
	mkdir -p "$dir/bin"
	tar -xzf "$tarball" -C "$dir/bin" mmux
	chmod 755 "$dir/bin/mmux"
	cp "$ROOT/LICENSE" "$dir/LICENSE"
	cat >"$dir/package.json" <<JSON
{
  "name": "$name",
  "version": "$VERSION",
  "description": "The mmux binary for $os ($cpu). Install \`mmux\` instead.",
  "license": "GPL-3.0-or-later",
  "homepage": "$HOMEPAGE",
  "repository": { "type": "git", "url": "$REPO_URL" },
  "os": ["$os"],
  "cpu": ["$cpu"],
  "files": ["bin/mmux"],
  "preferUnplugged": true
}
JSON
	printf '%s\n' "This is the $os-$cpu binary for [mmux]($HOMEPAGE). Install \`mmux\` instead: \`npm install -g mmux\`." >"$dir/README.md"
	order+=("$dir")
	optdeps="${optdeps:+$optdeps,
}    \"$name\": \"$VERSION\""
done

main="$OUT/pkg/mmux"
mkdir -p "$main/bin"
cp "$ROOT/npm/mmux/bin/mmux.js" "$main/bin/mmux.js"
chmod 755 "$main/bin/mmux.js"
cp "$ROOT/npm/mmux/README.md" "$main/README.md"
cp "$ROOT/LICENSE" "$main/LICENSE"
cat >"$main/package.json" <<JSON
{
  "name": "mmux",
  "version": "$VERSION",
  "description": "A persistent terminal multiplexer for AI agents and dev processes",
  "license": "GPL-3.0-or-later",
  "homepage": "$HOMEPAGE",
  "repository": { "type": "git", "url": "$REPO_URL" },
  "bugs": { "url": "$HOMEPAGE/issues" },
  "keywords": ["tmux", "terminal", "multiplexer", "ai", "agents", "claude", "codex", "tui"],
  "bin": { "mmux": "bin/mmux.js" },
  "files": ["bin/mmux.js"],
  "engines": { "node": ">=16" },
  "optionalDependencies": {
$optdeps
  }
}
JSON
order+=("$main")

for dir in "${order[@]}"; do
	(cd "$OUT" && npm pack "$dir" --silent >/dev/null)
done
echo "packed into $OUT:"
ls -1 "$OUT"/*.tgz

[ "$PUBLISH" = "--publish" ] || exit 0

for dir in "${order[@]}"; do
	name="$(node -p "require('$dir/package.json').name")"
	if npm view "$name@$VERSION" version >/dev/null 2>&1; then
		echo "$name@$VERSION already published — skipping"
		continue
	fi
	echo "publishing $name@$VERSION"
	npm publish "$dir" --provenance --access public
done
