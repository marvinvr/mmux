#!/usr/bin/env bash
# scripts/npm-placeholders.sh — ONE-TIME: reserve the four npm package names with a 0.0.0
# placeholder, so Trusted Publishing can then be configured on each (npm only allows that on
# a package that exists). Run once, by hand, after `npm login`. Already-published names are
# skipped, so it's safe to re-run. The release workflow publishes the real versions after.
# See docs/08-contributing.md → "npm Publishing (one-time setup)".
set -euo pipefail

NAMES="mmux-cli mmux-darwin-arm64 mmux-linux-x64 mmux-linux-arm64"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

npm whoami >/dev/null 2>&1 || { echo "not logged in to npm — run \`npm login\` first" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

for name in $NAMES; do
	if npm view "$name" version >/dev/null 2>&1; then
		echo "$name already exists on npm — skipping"
		continue
	fi
	dir="$tmp/$name"
	mkdir -p "$dir"
	cp "$ROOT/LICENSE" "$dir/LICENSE"
	cat >"$dir/package.json" <<JSON
{
  "name": "$name",
  "version": "0.0.0",
  "description": "Placeholder reserving the name for mmux (https://mmux.org). Real releases follow; install with: npm install -g mmux-cli",
  "license": "GPL-3.0-or-later",
  "homepage": "https://mmux.org",
  "repository": { "type": "git", "url": "git+https://github.com/marvinvr/mmux.git" }
}
JSON
	printf '%s\n' "# $name" "" "Placeholder reserving this name for [mmux](https://mmux.org). Install the real thing with \`npm install -g mmux-cli\`." >"$dir/README.md"
	echo "publishing placeholder $name@0.0.0"
	npm publish "$dir" --access public
done
echo "done — now configure a Trusted Publisher for each package (see docs/08-contributing.md)"
