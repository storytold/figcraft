# shellcheck shell=bash
# Shared by package.sh and verify.sh after packaging/env.sh has been sourced.

create_dmg() {
  local stage="$1" image="$2"
  # makehybrid -hfs synthesizes FinderInfo on resources (even plain .txt files), invalidating
  # strict bundle verification. create preserves the signed app's metadata. It attaches a
  # temporary device internally: retry transient "Resource busy" failures on release runners.
  retry hdiutil create -ov -fs HFS+ -volname FigCraft -srcfolder "$stage" \
    -format UDZO -imagekey zlib-level=9 "$image"
}

# Check the finished, read-only image and a normal installation copy, without repairing either.
# Integrity is mandatory for ad-hoc builds too; notarization is checked only when requested.
verify_dmg_app() (
  set -euo pipefail
  image="$1"
  notarized="${2:-0}"
  check_work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/figcraft-dmg-check.XXXXXX")"
  mounted=0
  # Invoked by the EXIT trap, including failures under errexit. ShellCheck 0.9 calls the
  # nested callback unreachable (SC2317); 0.11 calls it unused (SC2329).
  # shellcheck disable=SC2317,SC2329
  cleanup_dmg_check() {
    local check_exit=$?
    trap - EXIT
    if [[ "$mounted" = 1 ]]; then
      if ! retry hdiutil detach "$check_work/mounted"; then
        echo "error: could not detach $check_work/mounted; leaving its directory intact" >&2
        exit 1
      fi
    fi
    rm -rf "$check_work"
    exit "$check_exit"
  }
  trap cleanup_dmg_check EXIT
  mkdir -p "$check_work/mounted" "$check_work/copied"
  hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$check_work/mounted" "$image"
  mounted=1

  check_app() {
    local app="$1"
    # Require the app directory itself, not a link to a signed app outside the image.
    if [[ -L "$app" || ! -d "$app" ]]; then
      echo "error: DMG has no FigCraft.app: $app" >&2
      return 1
    fi
    codesign --verify --deep --strict --verbose=2 "$app"
    if [[ "$notarized" = 1 ]]; then
      retry xcrun stapler validate "$app"
      retry spctl --assess --type execute -vvv "$app"
    fi
  }

  check_app "$check_work/mounted/FigCraft.app"
  ditto "$check_work/mounted/FigCraft.app" "$check_work/copied/FigCraft.app"
  check_app "$check_work/copied/FigCraft.app"
)
