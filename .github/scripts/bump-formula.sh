#!/usr/bin/env bash
# Render the Homebrew formula and push it to the tap. Invoked by the `promote` job in
# release.yml — i.e. only after every verify leg has passed. Expects:
#   - TAG env var (e.g. v0.1.1)
#   - the release binaries already downloaded into ./dist/
#   - an SSH deploy key for the tap loaded into the agent (GIT_SSH_COMMAND set)
set -euo pipefail

: "${TAG:?TAG env var required (e.g. v0.1.1)}"
VERSION="${TAG#v}"
TAP_SSH="git@github.com:marvinvr/homebrew-mmux.git"
HERE="$(cd "$(dirname "$0")" && pwd)"

git clone "$TAP_SSH" tap
mkdir -p tap/Formula
# Render straight into the tap checkout (shared logic with the verify job's local test).
DIST="${DIST:-dist}" TAG="$TAG" TEMPLATE="$HERE/../homebrew/mmux.rb" OUT="tap/Formula/mmux.rb" \
	bash "$HERE/render-formula.sh"

cd tap
if git diff --quiet; then
	echo "Formula already up to date; nothing to commit."
	exit 0
fi
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git commit -am "mmux ${VERSION}"
git push origin HEAD:main
echo "Pushed mmux ${VERSION} to the tap."
