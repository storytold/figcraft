#!/usr/bin/env bash
# Trusted signing step for the Apple-silicon Nightly, the stable release's macOS signing route
# (import-cert.sh, then package.sh's codesign, notarytool and stapler steps):
#
#   nightly-sign.sh --arch aarch64 --input /abs/figcraft-<version>-macos-aarch64-nightly.zip \
#                   --output /abs/<dir>/figcraft-<version>-macos-aarch64-nightly.zip
#
# Input: the unsigned archive nightly.sh made. Output: the same bundle signed with the Developer ID and
# hardened runtime, notarized, with the ticket stapled into the bundle, zipped without extended
# attributes. Writes only that file, never replacing one; the temporary keychain and work directory
# are removed on every exit.
#
# Env (all required, no ad-hoc or unnotarized fallback): APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD,
# KEYCHAIN_PASSWORD, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID; optional NIGHTLY_APPLE_TEAM_ID (must
# equal APPLE_TEAM_ID) and RUNNER_TEMP (work directory base).
#
# Never compiles, and never runs anything from the payload: the version query runs on the signed
# bundle in a job without secrets (nightly-verify.sh).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=nightly-lib.sh
. "$HERE/nightly-lib.sh"
# shellcheck source=../nightly-common.sh
. "$HERE/../nightly-common.sh"

ARCH='' INPUT='' OUTPUT=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch | --input | --output)
      [[ $# -ge 2 ]] || mac_nightly_die "missing $1 value"
      case "$1" in --arch) ARCH="$2" ;; --input) INPUT="$2" ;; --output) OUTPUT="$2" ;; esac
      shift 2 ;;
    -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
    *) mac_nightly_die "unsupported argument: $1" ;;
  esac
done
[[ "$ARCH" == aarch64 ]] || mac_nightly_die 'only --arch aarch64 is supported'
[[ "$INPUT" == /* && "$OUTPUT" == /* ]] || mac_nightly_die '--input and --output must be absolute paths'
NAME="$(basename -- "$INPUT")"
VERSION="$(mac_nightly_version_from_asset "$NAME")" || mac_nightly_die 'the input is not a prepared Nightly archive name'
[[ "$(basename -- "$OUTPUT")" == "$NAME" ]] || mac_nightly_die 'the output must keep the archive name'
[[ -f "$INPUT" && ! -L "$INPUT" ]] || mac_nightly_die 'the input must be a regular file'
[[ ! -e "$OUTPUT" && ! -L "$OUTPUT" ]] || mac_nightly_die 'refusing to replace an existing output'
[[ -d "$(dirname -- "$OUTPUT")" && ! -L "$(dirname -- "$OUTPUT")" ]] || mac_nightly_die 'the output directory must exist'
mac_nightly_require_secrets
[[ "$(uname -s)" == Darwin ]] || mac_nightly_die 'signing needs macOS (codesign, notarytool, stapler)'

BASE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
WORK="$(mktemp -d "${BASE%/}/figcraft-nightly-sign.XXXXXX")"
KEYCHAIN=''
cleanup() {
  if [[ -n "$KEYCHAIN" && -e "$KEYCHAIN" ]]; then security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || rm -f -- "$KEYCHAIN"; fi
  rm -rf -- "$WORK"
}
trap cleanup EXIT

# ---- identity: import-cert.sh, into this run's private directory ------------------------------
# An empty GITHUB_ENV makes it print the identity instead of exporting it to later steps. It makes
# its own private directory under RUNNER_TEMP, so the keychain path comes from its output.
imported="$(GITHUB_ENV='' RUNNER_TEMP="$WORK" DIST="$WORK/dist" bash "$HERE/import-cert.sh")"
IDENTITY="$(sed -n 's/^MACOS_SIGN_IDENTITY=//p' <<<"$imported")"
KEYCHAIN="$(sed -n 's/^MACOS_KEYCHAIN=//p' <<<"$imported")"
if ! { [[ "$IDENTITY" =~ ^[0-9A-F]{40}$ ]] && mac_nightly_keychain_is_ours "$KEYCHAIN" "$WORK"; }; then
  mac_nightly_die 'import-cert.sh did not import a signing identity into this run directory'
fi
mac_nightly_identity_is_team "$(security find-identity -v -p codesigning "$KEYCHAIN" | grep -F " $IDENTITY " || true)" "$IDENTITY" "$APPLE_TEAM_ID" \
  || mac_nightly_die "the imported identity is not a Developer ID Application of team $APPLE_TEAM_ID"
echo "==> FigCraft Nightly $VERSION for macOS (aarch64), identity: $IDENTITY"

# ---- the unsigned bundle ---------------------------------------------------------------------------
mac_nightly_check_zip_entries "$INPUT"
ditto -x -k --norsrc --noextattr "$INPUT" "$WORK/payload"
mac_nightly_check_bundle "$WORK/payload" "$VERSION"
APP="$WORK/payload/$MAC_NIGHTLY_APP"

# ---- package.sh's signing and notarization, with a required Developer ID ---------------------------
sign() {
  codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" --timestamp "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [[ "$status" != "Accepted" ]]; then
    if [[ -n "$id" ]]; then
      # Capped: the shared signer bounds a step's output at 256 KiB.
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" | head -c 131072 || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# Stapling and validating both look the ticket up in Apple's CloudKit service, which can drop the
# connection right after notarization (error 68), so retry the pair (package.sh).
staple() {
  local file="$1" attempt
  for attempt in 1 2 3 4 5; do
    if xcrun stapler staple "$file" && xcrun stapler validate "$file"; then
      return
    fi
    if [[ "$attempt" = 5 ]]; then
      echo "error: stapling $(basename "$file") failed after $attempt attempts" >&2
      exit 1
    fi
    echo "stapling $(basename "$file") failed (attempt $attempt); retrying in $((attempt * 20))s" >&2
    sleep $((attempt * 20))
  done
}

# Inside-out: the executable, then the bundle (no --deep on the final signature).
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/$MAC_NIGHTLY_EXE"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"
ditto -c -k --keepParent "$APP" "$WORK/notarize.zip"
notarize "$WORK/notarize.zip"
# The ticket goes into the bundle (Contents/CodeResources): the Launcher's extract restores no xattrs.
staple "$APP"

# ---- the final archive, checked as the Launcher unpacks it -----------------------------------------
ditto -c -k --norsrc --noextattr --noacl --keepParent "$APP" "$WORK/final.zip"
ditto -x -k --norsrc --noextattr "$WORK/final.zip" "$WORK/check"
mac_nightly_check_bundle "$WORK/check" "$VERSION"
CHECK="$WORK/check/$MAC_NIGHTLY_APP"
codesign --verify --strict --deep --verbose=2 "$CHECK"
mac_nightly_check_signature "$(codesign -d --verbose=2 "$CHECK" 2>&1)" "$APPLE_TEAM_ID" \
  || mac_nightly_die 'the signed bundle is not the required Developer ID signature'
mac_nightly_validate_staple "$CHECK" || mac_nightly_die 'no notarization ticket is stapled into the bundle'
# spctl's lookup can lag a few seconds behind notarytool's "Accepted" (verify.sh).
for attempt in 1 2 3 4 5; do
  assessment="$(spctl --assess --type execute -vv "$CHECK" 2>&1)" || true
  mac_nightly_check_assessment "$assessment" && break
  [[ "$attempt" = 5 ]] || sleep 10
done
printf '%s\n' "$assessment"
mac_nightly_check_assessment "$assessment" || mac_nightly_die 'Gatekeeper does not accept the bundle as notarized'

nightly_publish_archive "$WORK/final.zip" "$OUTPUT"
