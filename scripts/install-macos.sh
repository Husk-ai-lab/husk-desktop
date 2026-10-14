#!/bin/sh
# Installs Husk AI on macOS from the latest GitHub release, for builds that are not notarised.
#
#   curl -fsSL https://raw.githubusercontent.com/Husk-ai-lab/husk-desktop/main/scripts/install-macos.sh | sh
#
# Why it exists: a DMG downloaded in a browser gets the quarantine flag, and Gatekeeper blocks an
# app that is not signed with a Developer ID and notarised. A file fetched by curl is not
# quarantined, so the app opens normally. This is a workaround for testers, not a replacement for
# signing: it does not make the app notarised.
#
# What it does: picks the DMG for this Mac's CPU, downloads it with SHA256SUMS from the same
# release, refuses to continue if the checksum does not match, copies "Husk AI.app" into
# /Applications (or ~/Applications when that is not writable) and unmounts the image.
# The checksum proves the download is the file the release holds; it is not a signature.
#
# Optional environment:
#   HUSK_VERSION=0.1.1        install this version instead of the latest release
#   HUSK_INSTALL_DIR=/path    install here instead of /Applications
#   HUSK_RELEASES_URL=...     base URL of the releases (testing only)

set -eu

REPO_RELEASES="${HUSK_RELEASES_URL:-https://github.com/Husk-ai-lab/husk-desktop/releases}"
APP_NAME="Husk AI.app"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS only"
command -v curl >/dev/null 2>&1 || die "curl is required"

case "$(uname -m)" in
  arm64 | aarch64) triple="aarch64-apple-darwin" ;;
  x86_64) triple="x86_64-apple-darwin" ;;
  *) die "unsupported CPU: $(uname -m)" ;;
esac

# --- which version -----------------------------------------------------------------------------
version="${HUSK_VERSION:-}"
if [ -z "$version" ]; then
  # /releases/latest redirects to /releases/tag/vX.Y.Z (no API call, so no rate limit)
  final_url="$(curl -fsSIL -o /dev/null -w '%{url_effective}' "$REPO_RELEASES/latest")" ||
    die "could not find the latest release (is there a published release yet?)"
  tag="${final_url##*/}"
  version="${tag#v}"
fi
case "$version" in
  *[!0-9A-Za-z.+-]* | "") die "unusable version: $version" ;;
esac
tag="v$version"
asset="Husk-AI_${version}_${triple}.dmg"
base="$REPO_RELEASES/download/$tag"

# --- download and verify -----------------------------------------------------------------------
work="$(mktemp -d "${TMPDIR:-/tmp}/husk-install.XXXXXX")"
mount_point="$work/mnt"
mounted=0
cleanup() {
  status=$?
  # detach first: deleting a mounted read-only image fails (and `mount` prints resolved paths,
  # so the mount point cannot be reliably looked up by name)
  if [ "$mounted" = "1" ]; then
    hdiutil detach "$mount_point" -quiet >/dev/null 2>&1 ||
      hdiutil detach "$mount_point" -quiet -force >/dev/null 2>&1 || true
  fi
  rm -rf "$work" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

say "Husk AI $version for $triple"
say "Downloading $asset ..."
curl -fL --progress-bar -o "$work/$asset" "$base/$asset" ||
  die "could not download $base/$asset (does release $tag have a DMG for this Mac?)"
curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS" ||
  die "release $tag has no SHA256SUMS, so the download cannot be verified; not installing"

expected="$(awk -v f="$asset" '$2 == f { print $1 }' "$work/SHA256SUMS")"
[ -n "$expected" ] || die "SHA256SUMS has no entry for $asset; not installing"
actual="$(shasum -a 256 "$work/$asset" | awk '{ print $1 }')"
if [ "$expected" != "$actual" ]; then
  die "checksum mismatch for $asset (expected $expected, got $actual); not installing"
fi
say "Checksum verified."

# --- install -----------------------------------------------------------------------------------
dest_dir="${HUSK_INSTALL_DIR:-/Applications}"
if [ ! -w "$dest_dir" ]; then
  dest_dir="$HOME/Applications"
  mkdir -p "$dest_dir"
fi
dest="$dest_dir/$APP_NAME"

mkdir -p "$mount_point"
hdiutil attach "$work/$asset" -nobrowse -readonly -noautoopen -quiet -mountpoint "$mount_point" ||
  die "could not mount the disk image"
mounted=1
[ -d "$mount_point/$APP_NAME" ] || die "the disk image holds no $APP_NAME"

if [ -e "$dest" ]; then
  say "Replacing the existing $dest"
  rm -rf "$dest"
fi
ditto "$mount_point/$APP_NAME" "$dest"
# harmless when there is no flag (curl does not set one); removes it if some other tool did
xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true

say ""
say "Installed: $dest"
say "Open it from Launchpad or Spotlight, or run:  open \"$dest\""
say "This build is not notarised: it opens because curl does not quarantine downloads."
