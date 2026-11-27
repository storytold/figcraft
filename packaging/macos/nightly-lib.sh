# shellcheck shell=bash
# Shared checks for the Apple-silicon Nightly scripts (nightly.sh, nightly-sign.sh, nightly-verify.sh).
# Stable packaging (package.sh, verify.sh) does not source this file. The text checks take tool output
# as arguments, so packaging/macos/nightly_sign_tests.py runs them without macOS.

MAC_NIGHTLY_APP='FigCraft Nightly.app'
MAC_NIGHTLY_ID='ai.storyteller.figcraft.nightly'
# The GUI's file name inside the bundle (CFBundleExecutable / CFBundleIconFile name their own).
MAC_NIGHTLY_EXE='FigCraft'
# The version key this app's Info.plist.in carries (package.sh substitutes it too).
MAC_NIGHTLY_VERSION_KEY='FigCraftVersion'
# The release environment's names, as stable release.yml passes them (packaging/macos/import-cert.sh).
MAC_NIGHTLY_SECRETS=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD KEYCHAIN_PASSWORD APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)

mac_nightly_die() { printf 'error: %s\n' "$*" >&2; exit 2; }

# The exact prepared version from the feed asset name, or failure.
mac_nightly_version_from_asset() {
  local name="$1" version
  [[ "$name" =~ ^figcraft-(.+)-macos-aarch64-nightly\.zip$ ]] || return 1
  version="${BASH_REMATCH[1]}"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-nightly\.[0-9]{8}\.[0-9]+\.[0-9]+\.g[0-9a-f]{12}$ ]] || return 1
  printf '%s\n' "$version"
}

# Every signing and notarization value is set, and the team is one Apple team ID. No ad-hoc fallback.
mac_nightly_require_secrets() {
  local name
  for name in "${MAC_NIGHTLY_SECRETS[@]}"; do
    [[ -n "${!name:-}" ]] || mac_nightly_die "$name is not set: Nightly signing and notarization are required, with no ad-hoc fallback"
  done
  [[ "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || mac_nightly_die 'APPLE_TEAM_ID is not an Apple team ID'
  if [[ -n "${NIGHTLY_APPLE_TEAM_ID:-}" && "$NIGHTLY_APPLE_TEAM_ID" != "$APPLE_TEAM_ID" ]]; then
    mac_nightly_die 'the signing team differs from the reviewed Nightly team (NIGHTLY_APPLE_TEAM_ID)'
  fi
}

# This app's import-cert.sh makes its own private directory under RUNNER_TEMP and puts the keychain
# inside it, so the path is not fixed: accept only a regular file of this run, named for the app,
# below the private work directory (and never through a link).
mac_nightly_keychain_is_ours() {
  local keychain="$1" work="$2" parent
  [[ -n "$keychain" && "$keychain" == /* && ! -L "$keychain" && -f "$keychain" ]] || return 1
  case "$keychain" in
    "$work"/figcraft-signing.keychain-db | "$work"/*/figcraft-signing.keychain-db) ;;
    *) return 1 ;;
  esac
  parent="$(dirname -- "$keychain")"
  [[ "$parent" == "$work" || "$parent" == "$work"/* ]] && [[ ! -L "$parent" ]]
}

# One line of `security find-identity -v -p codesigning`: the identity is a Developer ID Application of the team.
mac_nightly_identity_is_team() {
  local line="$1" identity="$2" team="$3"
  [[ "$line" == *" $identity "* && "$line" == *'"Developer ID Application: '*" ($team)\""* ]]
}

# `codesign -d --verbose=2` of the signed bundle: the Nightly identifier, a Developer ID Application of the
# team, a secure timestamp and the hardened runtime (the checks of verify.sh, made strict).
mac_nightly_check_signature() {
  local details="$1" team="$2" ok=0
  grep -Fqx "Identifier=$MAC_NIGHTLY_ID" <<<"$details" || { echo "error: the bundle is not signed as $MAC_NIGHTLY_ID" >&2; ok=1; }
  grep -q '^Authority=Developer ID Application:' <<<"$details" || { echo 'error: not signed with a Developer ID Application identity' >&2; ok=1; }
  grep -qx "TeamIdentifier=$team" <<<"$details" || { echo 'error: signed by a different team' >&2; ok=1; }
  grep -q '^Timestamp=' <<<"$details" || { echo 'error: the signature has no secure timestamp' >&2; ok=1; }
  grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' <<<"$details" || { echo 'error: not signed with the hardened runtime' >&2; ok=1; }
  return "$ok"
}

# `spctl --assess --type execute -vv`: Gatekeeper accepts the bundle as notarized.
mac_nightly_check_assessment() {
  grep -q 'accepted' <<<"$1" && grep -qx 'source=Notarized Developer ID' <<<"$1"
}

# Before anything is unpacked: every ZIP entry is a regular file or directory inside the one Nightly
# bundle, with no absolute path, `..`, backslash or empty component, and no link or special file.
mac_nightly_check_zip_entries() {
  python3 -I - "$1" "$MAC_NIGHTLY_APP" <<'PY' || mac_nightly_die 'the archive has an entry outside the bundle, a link or a special file'
import stat, sys, zipfile
archive, app = sys.argv[1:]
with zipfile.ZipFile(archive) as z:
    for info in z.infolist():
        name = info.filename
        parts = name.rstrip("/").split("/")
        mode = info.external_attr >> 16
        if (name.startswith("/") or "\\" in name or "\0" in name or parts[0] != app
                or any(part in ("", ".", "..") for part in parts)
                or stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR)):
            sys.exit(f"refused entry: {name!r}")
PY
}

# An unpacked payload directory holds exactly the GUI-only Nightly bundle: one thin arm64 FigCraft
# executable, the Nightly identity and the exact version (no numeric fallback), no CLI, no links or special
# files, no document or URL types.
mac_nightly_check_bundle() {
  local payload="$1" version="$2" app entries plist key value
  app="$payload/$MAC_NIGHTLY_APP"
  entries="$(ls -A "$payload")"
  [[ "$entries" == "$MAC_NIGHTLY_APP" && -d "$app" && ! -L "$app" ]] || mac_nightly_die "the archive must hold exactly $MAC_NIGHTLY_APP"
  [[ -z "$(find "$payload" ! -type f ! -type d -print -quit)" ]] || mac_nightly_die 'the bundle contains links or special files'
  [[ "$(ls -A "$app/Contents/MacOS")" == "$MAC_NIGHTLY_EXE" ]] || mac_nightly_die 'Contents/MacOS must hold only the FigCraft GUI (no CLI)'
  [[ -f "$app/Contents/MacOS/$MAC_NIGHTLY_EXE" && -x "$app/Contents/MacOS/$MAC_NIGHTLY_EXE" ]] || mac_nightly_die 'missing executable GUI'
  [[ "$(lipo -archs "$app/Contents/MacOS/$MAC_NIGHTLY_EXE")" == arm64 ]] || mac_nightly_die 'the GUI must be thin arm64'
  plist="$app/Contents/Info.plist"
  for key in CFBundleIdentifier CFBundleExecutable "$MAC_NIGHTLY_VERSION_KEY"; do
    value="$(plutil -extract "$key" raw -o - "$plist")" || mac_nightly_die "Info.plist has no $key"
    case "$key" in
      CFBundleIdentifier) [[ "$value" == "$MAC_NIGHTLY_ID" ]] ;;
      CFBundleExecutable) [[ "$value" == "$MAC_NIGHTLY_EXE" ]] ;;
      "$MAC_NIGHTLY_VERSION_KEY") [[ "$value" == "$version" ]] ;;
    esac || mac_nightly_die "Info.plist $key is not the prepared Nightly identity"
  done
  for key in CFBundleDocumentTypes CFBundleURLTypes UTExportedTypeDeclarations UTImportedTypeDeclarations; do
    if plutil -extract "$key" xml1 -o - "$plist" >/dev/null 2>&1; then mac_nightly_die "Nightly declares $key"; fi
  done
}

# `stapler validate` asks Apple's CloudKit service, which sometimes drops the connection (error 68)
# though the ticket is there (verify.sh).
mac_nightly_validate_staple() {
  local target="$1" attempt
  for attempt in 1 2 3 4 5; do
    xcrun stapler validate "$target" && return 0
    [[ "$attempt" = 5 ]] || sleep $((attempt * 20))
  done
  return 1
}
