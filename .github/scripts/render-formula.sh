#!/usr/bin/env bash
# Render the Homebrew formula for a tag from the template, resolving the version, tag, and
# the source + macOS-binary sha256s. Shared by two callers:
#   - the release `verify` job renders it to a temp file and `brew install --formula`s it,
#     so the formula is proven to install before it can ever reach the tap;
#   - bump-formula.sh renders it into a tap checkout and pushes it (on `promote`).
#
# Env:
#   TAG       required, e.g. v0.1.1
#   DIST      dir holding the macOS release tarballs (default: dist)
#   TEMPLATE  formula template (default: .github/homebrew/mmux.rb)
#   OUT       output path (default: mmux.rb)
set -euo pipefail

: "${TAG:?TAG required (e.g. v0.1.1)}"
VERSION="${TAG#v}"
REPO="marvinvr/mmux"
DIST="${DIST:-dist}"
TEMPLATE="${TEMPLATE:-.github/homebrew/mmux.rb}"
OUT="${OUT:-mmux.rb}"

# Portable sha256: this script also runs on macOS (the release verify job renders + installs
# the formula there), where `sha256sum` doesn't exist but `shasum` does.
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; }
sha_file() { sha256 <"$1" | cut -d' ' -f1; }

SOURCE_URL="https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz"
SOURCE_SHA="$(curl -fsSL "$SOURCE_URL" | sha256 | cut -d' ' -f1)"
MAC_ARM_SHA="$(sha_file "${DIST}/mmux-aarch64-apple-darwin.tar.gz")"
MAC_X86_SHA="$(sha_file "${DIST}/mmux-x86_64-apple-darwin.tar.gz")"

echo "Rendering formula for ${VERSION}:" >&2
echo "  source:  $SOURCE_SHA" >&2
echo "  mac arm: $MAC_ARM_SHA" >&2
echo "  mac x86: $MAC_X86_SHA" >&2

sed -e "s|VERSION_PLACEHOLDER|${VERSION}|g" \
	-e "s|TAG_PLACEHOLDER|${TAG}|g" \
	-e "s|SOURCE_SHA_PLACEHOLDER|${SOURCE_SHA}|g" \
	-e "s|MAC_ARM_SHA_PLACEHOLDER|${MAC_ARM_SHA}|g" \
	-e "s|MAC_X86_SHA_PLACEHOLDER|${MAC_X86_SHA}|g" \
	"$TEMPLATE" >"$OUT"

# Sanity check: no placeholder survived the substitution.
if grep -q "PLACEHOLDER" "$OUT"; then
	echo "ERROR: unresolved placeholder left in $OUT:" >&2
	grep "PLACEHOLDER" "$OUT" >&2
	exit 1
fi

echo "Wrote ${OUT}" >&2
