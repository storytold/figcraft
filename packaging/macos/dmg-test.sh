#!/usr/bin/env bash
# Native signature round-trip regression for #387. No build, certificates or notarization needed.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_WORK="$(mktemp -d "${TMPDIR:-/tmp}/figcraft-dmg-test.XXXXXX")"
trap 'rm -rf "$TEST_WORK"' EXIT
export DIST="$TEST_WORK/dist"
# shellcheck source=../env.sh
. "$HERE/../env.sh"
# shellcheck source=dmg.sh
. "$HERE/dmg.sh"
mkdir -p "$TEST_WORK/stage/FigCraft.app/Contents/MacOS" "$TEST_WORK/stage/FigCraft.app/Contents/Resources" "$TEST_WORK/checks"
APP="$TEST_WORK/stage/FigCraft.app"
cp /usr/bin/true "$APP/Contents/MacOS/FigCraft"
sed -e 's/@VERSION@/0.0.0/g' -e 's/@SHORT_VERSION@/0.0.0/g' -e 's/@BUILD_SHA@/test/g' \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
printf 'Test licence\n' >"$APP/Contents/Resources/License.txt"
ln -s /Applications "$TEST_WORK/stage/Applications"
codesign --force --sign - --timestamp=none "$APP/Contents/MacOS/FigCraft"
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP"

# Invoke in a fresh shell: testing a function directly in `if` would disable its errexit.
verify_image() {
  RUNNER_TEMP="$TEST_WORK/checks" bash -c '
    set -euo pipefail
    . "$1/../env.sh"
    . "$1/dmg.sh"
    verify_dmg_app "$2" 0
  ' _ "$HERE" "$1"
}
assert_detached() {
  if [ -n "$(find "$TEST_WORK/checks" -mindepth 1 -print -quit)" ]; then
    echo "error: image check did not detach and remove its temporary directory" >&2
    exit 1
  fi
}

# Both the app in the read-only image and a ditto installation copy must remain valid.
create_dmg "$TEST_WORK/stage" "$TEST_WORK/good.dmg"
verify_image "$TEST_WORK/good.dmg"
assert_detached

# A copy can acquire metadata independently of the mounted app. Keep real ditto/codesign,
# but inject FinderInfo into the installation copy to prove its own check is mandatory.
mkdir -p "$TEST_WORK/bin"
cat >"$TEST_WORK/bin/ditto" <<'DITTO'
#!/usr/bin/env bash
set -euo pipefail
/usr/bin/ditto "$@"
xattr -wx com.apple.FinderInfo 5445585400000000000000000000000000000000000000000000000000000000 \
  "$2/Contents/Resources/License.txt"
DITTO
chmod +x "$TEST_WORK/bin/ditto"
if PATH="$TEST_WORK/bin:$PATH" verify_image "$TEST_WORK/good.dmg" >"$TEST_WORK/copy.log" 2>&1; then
  echo "error: verifier accepted invalid Finder data added to the installation copy" >&2
  exit 1
fi
grep -q 'Disallowed xattr com.apple.FinderInfo.*copied/FigCraft.app' "$TEST_WORK/copy.log"
assert_detached

# Reproduce the old image builder: it adds FinderInfo to License.txt after signing.
hdiutil makehybrid -hfs -hfs-volume-name FigCraft -hfs-openfolder "$TEST_WORK/stage" \
  -o "$TEST_WORK/bad.dmg" "$TEST_WORK/stage"
if verify_image "$TEST_WORK/bad.dmg" >"$TEST_WORK/rejected.log" 2>&1; then
  echo "error: verifier accepted an app with invalid attached Finder data" >&2
  exit 1
fi
grep -q 'Disallowed xattr com.apple.FinderInfo.*mounted/FigCraft.app' "$TEST_WORK/rejected.log"
assert_detached

mkdir -p "$TEST_WORK/empty"
printf 'No application\n' >"$TEST_WORK/empty/README.txt"
create_dmg "$TEST_WORK/empty" "$TEST_WORK/missing.dmg"
if verify_image "$TEST_WORK/missing.dmg" >"$TEST_WORK/missing.log" 2>&1; then
  echo "error: verifier accepted a DMG without FigCraft.app" >&2
  exit 1
fi
grep -q 'DMG has no FigCraft.app' "$TEST_WORK/missing.log"
assert_detached

echo 'DMG round trip: signed app and installation copy pass; FinderInfo in image/copy and missing app rejected; mounts cleaned up'
