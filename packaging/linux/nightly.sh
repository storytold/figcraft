#!/usr/bin/env bash
# App-owned GUI-only nightly tar: no FUSE/AppImage, CLI, association or install hooks.
set -euo pipefail
ARCH=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) [[ $# -ge 2 ]] || { echo 'missing --arch value' >&2; exit 2; }; ARCH="$2"; shift 2 ;;
    -h|--help) echo 'Usage: nightly.sh --arch x86_64'; exit 0 ;;
    *) echo "unsupported nightly argument: $1" >&2; exit 2 ;;
  esac
done
[[ "$ARCH" == x86_64 && "$(uname -s)" == Linux ]] || { echo 'only native Linux x86_64 nightly packaging is supported' >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../nightly-common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../nightly-common.sh"
nightly_init linux "$ARCH" x86_64-unknown-linux-gnu
OUTPUT="$DIST/figcraft-$VERSION-linux-x86_64-nightly.tar.gz"
[[ ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || nightly_die 'refusing to replace an existing nightly archive'
nightly_build_gui
STAGE="$NIGHTLY_WORK/stage"
mkdir -p "$STAGE"
# The standard nightly layout: the archive's entry point `figcraft` is the shared launcher, which
# starts the GUI on the Launcher's nightly-v1 profile, and the GUI itself sits beside it as
# `figcraft-bin`. An unpacked tar is isolated too, even when started straight from its folder.
cp "$NIGHTLY_GUI" "$STAGE/figcraft-bin"
chmod 755 "$STAGE/figcraft-bin"
awk -v real='"$(dirname "$0")/figcraft-bin"' '{ gsub(/@REAL_BIN@/, real); print }' \
  "$HERE/nightly-launch.sh" >"$STAGE/figcraft"
chmod 755 "$STAGE/figcraft"
cp "$ROOT/assets/app-icon/figcraft-nightly.png" "$STAGE/figcraft-nightly.png"
nightly_copy_notices "$STAGE"
# The private mktemp cwd is the helper authority root, not a CLI path parent.
(cd "$NIGHTLY_WORK" && python3 "$ROOT/packaging/nightly-archive.py" linux stage archive.tar.gz)
nightly_publish_archive "$NIGHTLY_WORK/archive.tar.gz" "$OUTPUT"
