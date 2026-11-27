#!/usr/bin/env bash
# Check an Apple-silicon Nightly archive as the Launcher unpacks it, then ask the GUI its version.
# For a job without signing material (it refuses to run with any), after nightly-sign.sh:
#
#   nightly-verify.sh --input /abs/figcraft-<version>-macos-aarch64-nightly.zip --expect signed [--team ID]
#   nightly-verify.sh --input /abs/figcraft-<version>-macos-aarch64-nightly.zip --expect unsigned
#
# Both: the archive holds only the GUI-only Nightly bundle with the exact FigCraftVersion, unpacked
# without extended attributes. `signed` (the feed artifact): Developer ID of the team (--team, else
# NIGHTLY_APPLE_TEAM_ID), hardened runtime, secure timestamp, stapled ticket, Gatekeeper accepts it as
# notarized. `unsigned` (nightly.sh's compile output, e.g. pull-request CI) checks no signature.
# Last, the bundle's GUI answers its version query with an empty environment, both plainly and in the
# Launcher's form `--launch-profile nightly-v1 --profile-root <fresh dir> --version` (the shared producer
# smoke): each must exit 0 within 15 seconds, print exactly `FigCraft <version> (nightly-v1)` and
# write nothing to standard error.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=nightly-lib.sh
. "$HERE/nightly-lib.sh"

INPUT='' EXPECT='' TEAM="${NIGHTLY_APPLE_TEAM_ID:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --input | --expect | --team)
      [[ $# -ge 2 ]] || mac_nightly_die "missing $1 value"
      case "$1" in --input) INPUT="$2" ;; --expect) EXPECT="$2" ;; --team) TEAM="$2" ;; esac
      shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) mac_nightly_die "unsupported argument: $1" ;;
  esac
done
for name in "${MAC_NIGHTLY_SECRETS[@]}" MACOS_SIGN_IDENTITY MACOS_KEYCHAIN; do
  [[ -z "${!name:-}" ]] || mac_nightly_die "refusing to run the payload with signing material in the environment ($name)"
done
[[ "$EXPECT" == signed || "$EXPECT" == unsigned ]] || mac_nightly_die '--expect must be signed or unsigned'
[[ "$EXPECT" == unsigned || "$TEAM" =~ ^[A-Z0-9]{10}$ ]] || mac_nightly_die 'a signed check needs the Apple team ID (--team or NIGHTLY_APPLE_TEAM_ID)'
[[ "$INPUT" == /* && -f "$INPUT" && ! -L "$INPUT" ]] || mac_nightly_die '--input must be an absolute regular file'
VERSION="$(mac_nightly_version_from_asset "$(basename -- "$INPUT")")" || mac_nightly_die 'the input is not a prepared Nightly archive name'
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || mac_nightly_die 'needs an Apple silicon Mac'

WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/figcraft-nightly-verify.XXXXXX")"
trap 'rm -rf -- "$WORK"' EXIT
mac_nightly_check_zip_entries "$INPUT"
ditto -x -k --norsrc --noextattr "$INPUT" "$WORK/payload"
mac_nightly_check_bundle "$WORK/payload" "$VERSION"
APP="$WORK/payload/$MAC_NIGHTLY_APP"

if [[ "$EXPECT" == signed ]]; then
  codesign --verify --strict --deep --verbose=2 "$APP"
  mac_nightly_check_signature "$(codesign -d --verbose=2 "$APP" 2>&1)" "$TEAM" \
    || mac_nightly_die 'the bundle is not the required Developer ID signature'
  mac_nightly_validate_staple "$APP" || mac_nightly_die 'no notarization ticket is stapled into the bundle'
  for attempt in 1 2 3 4 5; do
    assessment="$(spctl --assess --type execute -vv "$APP" 2>&1)" || true
    mac_nightly_check_assessment "$assessment" && break
    [[ "$attempt" = 5 ]] || sleep 10
  done
  printf '%s\n' "$assessment"
  mac_nightly_check_assessment "$assessment" || mac_nightly_die 'Gatekeeper does not accept the bundle as notarized'
fi

# macOS has no timeout(1): perl's alarm survives its exec and ends a query that hangs (a dialog, say).
query() {
  local out
  out="$(env -i HOME="$WORK/home" TMPDIR="$WORK/tmp/" PATH=/usr/bin:/bin \
    /usr/bin/perl -e 'alarm 15; exec @ARGV or die' "$APP/Contents/MacOS/$MAC_NIGHTLY_EXE" "$@" 2>"$WORK/stderr")" \
    || mac_nightly_die "the version query ($*) failed or did not finish within 15 s"
  [[ ! -s "$WORK/stderr" ]] || mac_nightly_die "the version query ($*) wrote to standard error: $(head -c 2000 "$WORK/stderr")"
  [[ "$out" == "FigCraft $VERSION (nightly-v1)" ]] || mac_nightly_die "the version query ($*) reports '$out', not FigCraft $VERSION (nightly-v1)"
}
mkdir -m 700 "$WORK/home" "$WORK/tmp" "$WORK/profile"
query --version
query --launch-profile nightly-v1 --profile-root "$WORK/profile" --version
echo "==> verified ($EXPECT): FigCraft $VERSION (nightly-v1), plain and in the Launcher's profile form"
