#!/bin/bash
# Installs the Husk AI shell on macOS from GitHub Releases, without the Gatekeeper prompt.
# A file fetched with curl carries no com.apple.quarantine attribute, so macOS does not check it.
#
#   curl -fsSL https://raw.githubusercontent.com/Husk-ai-lab/husk-desktop/main/scripts/install-macos.sh | bash
#
# HUSK_VERSION=0.2.0   install that version instead of the latest published release
# HUSK_INSTALL_DIR=... install somewhere other than /Applications
set -euo pipefail

REPO="Husk-ai-lab/husk-desktop"
APP="Husk AI.app"
DEST="${HUSK_INSTALL_DIR:-/Applications}"

fail() { echo "error: $*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || fail "this installer is for macOS only"

case "$(uname -m)" in
  arm64) ARCH=aarch64 ;;
  x86_64) ARCH=x86_64 ;;
  *) fail "unsupported CPU: $(uname -m)" ;;
esac

VERSION="${HUSK_VERSION:-}"
if [ -z "$VERSION" ]; then
  # releases/latest redirects to the tag of the latest published release; no API rate limit.
  TAG=$(curl -fsSI "https://github.com/$REPO/releases/latest" | awk -F/ 'tolower($0) ~ /^location:/ {print $NF}' | tr -d '\r')
  [ -n "$TAG" ] || fail "could not find the latest release of $REPO"
  VERSION="${TAG#v}"
fi

DMG="Husk-AI_${VERSION}_${ARCH}-apple-darwin.dmg"
BASE="https://github.com/$REPO/releases/download/v$VERSION"

WORK=$(mktemp -d)
MNT=""
cleanup() {
  [ -n "$MNT" ] && hdiutil detach -quiet "$MNT" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "Downloading Husk AI $VERSION ($ARCH)..."
curl -fL --progress-bar -o "$WORK/$DMG" "$BASE/$DMG" || fail "download failed: $BASE/$DMG"
curl -fsSL -o "$WORK/SHA256SUMS" "$BASE/SHA256SUMS" || fail "download failed: $BASE/SHA256SUMS"

EXPECTED=$(awk -v f="$DMG" '{n=$2; sub(/^\*/, "", n)} n == f {print $1}' "$WORK/SHA256SUMS")
[ -n "$EXPECTED" ] || fail "$DMG is not listed in SHA256SUMS"
ACTUAL=$(shasum -a 256 "$WORK/$DMG" | awk '{print $1}')
[ "$EXPECTED" = "$ACTUAL" ] || fail "SHA-256 mismatch for $DMG (expected $EXPECTED, got $ACTUAL)"
echo "SHA-256 verified."

MNT=$(hdiutil attach -nobrowse -readonly -noautoopen "$WORK/$DMG" 2>/dev/null | awk -F'\t' '/\/Volumes\// {print $NF}')
[ -d "$MNT/$APP" ] || fail "$APP not found in $DMG"

if pgrep -f "$DEST/$APP/Contents/MacOS/" >/dev/null 2>&1; then
  echo "Quitting the running Husk AI..."
  osascript -e 'quit app "Husk AI"' >/dev/null 2>&1 || true
  sleep 2
fi

SUDO=""
mkdir -p "$DEST" 2>/dev/null || true
[ -w "$DEST" ] || SUDO="sudo"
$SUDO rm -rf "$DEST/$APP"
$SUDO ditto "$MNT/$APP" "$DEST/$APP"
# In case an earlier browser download left the attribute behind.
$SUDO xattr -dr com.apple.quarantine "$DEST/$APP" 2>/dev/null || true

echo "Installed Husk AI $VERSION in $DEST/$APP"
[ -n "${HUSK_INSTALL_DIR:-}" ] || open "$DEST/$APP"
