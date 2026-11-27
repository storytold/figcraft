#!/usr/bin/env bash
# App-owned GUI-only Nightly for Apple silicon, compile step only (no signing material):
#
#   $DIST/figcraft-<version>-macos-aarch64-nightly.zip   FigCraft Nightly.app, unsigned
#
# Usage: packaging/macos/nightly.sh --arch aarch64
#
# Required env (nightly-common.sh): FIGCRAFT_BUILD_CHANNEL=nightly-v1, FIGCRAFT_VERSION,
# FIGCRAFT_BUILD_SHA, FIGCRAFT_BUILD_DATE, absolute CARGO_TARGET_DIR and DIST; CRAFT_FONTS_DIR optional.
#
# Not publishable by itself: the trusted step (nightly-sign.sh) signs, notarizes and staples it. No CLI,
# DMG, document or URL types, and no stable package.sh. The bundle is not run here; the version query
# runs on the signed bundle (nightly-verify.sh).
set -euo pipefail
ARCH=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) [[ $# -ge 2 ]] || { echo 'missing --arch value' >&2; exit 2; }; ARCH="$2"; shift 2 ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unsupported nightly argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$ARCH" == aarch64 && "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'only native macOS aarch64 nightly packaging is supported' >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/../nightly-common.sh"
. "$HERE/nightly-lib.sh"
nightly_refuse_signing_env
nightly_init macos "$ARCH" aarch64-apple-darwin
# Keep in sync with LSMinimumSystemVersion in Info.plist.in (as package.sh).
export MACOSX_DEPLOYMENT_TARGET=11.0
OUTPUT="$DIST/figcraft-$VERSION-macos-aarch64-nightly.zip"
[[ ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || nightly_die 'refusing to replace an existing nightly archive'
nightly_compile_gui

APP="$NIGHTLY_WORK/stage/$MAC_NIGHTLY_APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$NIGHTLY_GUI" "$APP/Contents/MacOS/$MAC_NIGHTLY_EXE"
chmod 755 "$APP/Contents/MacOS/$MAC_NIGHTLY_EXE"
cp "$ROOT/assets/app-icon/figcraft-nightly.icns" "$APP/Contents/Resources/figcraft-nightly.icns"
nightly_copy_notices "$APP/Contents/Resources"
# The private mktemp cwd is the helper authority root: stage the template inside it.
cp "$HERE/Info.plist.in" "$NIGHTLY_WORK/Info.plist.in"
(cd "$NIGHTLY_WORK" && python3 "$HERE/nightly_plist.py" --template Info.plist.in \
  --output "stage/$MAC_NIGHTLY_APP/Contents/Info.plist" --version "$VERSION" --source-commit "$FIGCRAFT_BUILD_SHA")
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"
mac_nightly_check_bundle "$NIGHTLY_WORK/stage" "$VERSION"

# No resource forks, extended attributes or ACLs: the Launcher restores none, so nothing may depend on them.
ditto -c -k --norsrc --noextattr --noacl --keepParent "$APP" "$NIGHTLY_WORK/archive.zip"
nightly_publish_archive "$NIGHTLY_WORK/archive.zip" "$OUTPUT"
