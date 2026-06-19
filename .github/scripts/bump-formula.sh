#!/usr/bin/env bash
# Render the Homebrew formula from .github/homebrew/mmux.rb and push it to the tap.
# Invoked by .github/workflows/release.yml on a vX.Y.Z tag. Expects:
#   - TAG env var (e.g. v0.1.1)
#   - the release binaries already downloaded into ./dist/
#   - an SSH deploy key for the tap loaded into the agent (GIT_SSH_COMMAND set)
set -euo pipefail

: "${TAG:?TAG env var required (e.g. v0.1.1)}"
VERSION="${TAG#v}"
REPO="marvinvr/mmux"
TAP_SSH="git@github.com:marvinvr/homebrew-mmux.git"
DIST="${DIST:-dist}"
TEMPLATE=".github/homebrew/mmux.rb"

sha_file() { sha256sum "$1" | cut -d' ' -f1; }

echo "Computing checksums for $TAG ..."
SOURCE_URL="https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz"
SOURCE_SHA="$(curl -fsSL "$SOURCE_URL" | sha256sum | cut -d' ' -f1)"
MAC_ARM_SHA="$(sha_file "${DIST}/mmux-aarch64-apple-darwin.tar.gz")"
MAC_X86_SHA="$(sha_file "${DIST}/mmux-x86_64-apple-darwin.tar.gz")"
LINUX_X86_SHA="$(sha_file "${DIST}/mmux-x86_64-unknown-linux-gnu.tar.gz")"

echo "  source:    $SOURCE_SHA"
echo "  mac arm:   $MAC_ARM_SHA"
echo "  mac x86:   $MAC_X86_SHA"
echo "  linux x86: $LINUX_X86_SHA"

git clone "$TAP_SSH" tap
mkdir -p tap/Formula
sed -e "s|VERSION_PLACEHOLDER|${VERSION}|g" \
    -e "s|TAG_PLACEHOLDER|${TAG}|g" \
    -e "s|SOURCE_SHA_PLACEHOLDER|${SOURCE_SHA}|g" \
    -e "s|MAC_ARM_SHA_PLACEHOLDER|${MAC_ARM_SHA}|g" \
    -e "s|MAC_X86_SHA_PLACEHOLDER|${MAC_X86_SHA}|g" \
    -e "s|LINUX_X86_SHA_PLACEHOLDER|${LINUX_X86_SHA}|g" \
    "$TEMPLATE" > tap/Formula/mmux.rb

# Sanity check: no placeholder survived the substitution.
if grep -q "PLACEHOLDER" tap/Formula/mmux.rb; then
  echo "ERROR: unresolved placeholder left in formula:" >&2
  grep "PLACEHOLDER" tap/Formula/mmux.rb >&2
  exit 1
fi

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
