#!/usr/bin/env bash
# The nightly launcher (packaging/linux/nightly-launch.sh) in each form the formats install it:
# beside the GUI as figcraft-bin in a tarball, at /usr/lib in a package, inside an AppImage, and the
# Launcher's own protocol passed through. Runs in a second with a recording stub; it builds nothing
# and opens no window.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/deb/usr/lib/figcraft-nightly" "$work/app" "$work/home with space"

recorder() {
  cat >"$1" <<'RECORDER'
#!/bin/sh
printf 'CONFIG=%s\nDATA=%s\n' "${XDG_CONFIG_HOME:-}" "${XDG_DATA_HOME:-}"
printf '<%s>\n' "$@"
RECORDER
  chmod 755 "$1"
}
install_launcher() {
  awk -v real="$2" '{ gsub(/@REAL_BIN@/, real); print }' "$HERE/nightly-launch.sh" >"$1"
  chmod 755 "$1"
}

# The tarball's standard layout: `figcraft` is the launcher and the GUI beside it is `figcraft-bin`.
recorder "$work/bin/figcraft-bin"
# shellcheck disable=SC2016 # $(dirname "$0") stays literal in the launcher: it resolves at run time
install_launcher "$work/bin/figcraft" '"$(dirname "$0")/figcraft-bin"'
# deb/rpm: the wrapper at /usr/bin, the GUI under /usr/lib. The absolute path cannot resolve in a
# test, so this form is checked by its text (and by the packaging tests that stage a package).
install_launcher "$work/deb/figcraft-nightly" '/usr/lib/figcraft-nightly/figcraft'
grep -Fxq 'real_bin=/usr/lib/figcraft-nightly/figcraft' "$work/deb/figcraft-nightly" \
  || { echo "the package wrapper does not start the packaged GUI" >&2; exit 1; }
# AppImage: AppRun runs the launcher, which starts the GUI in the AppDir's usr/bin.
mkdir -p "$work/app/usr/bin"
recorder "$work/app/usr/bin/figcraft"
install_launcher "$work/app/figcraft-nightly" "\"$work/app/usr/bin/figcraft\""

export HOME="$work/home with space"
unset XDG_CONFIG_HOME XDG_DATA_HOME
profile="$HOME/.local/share/figcraft-nightly/profile"

check() {
  local launcher="$1" want="$2"
  shift 2
  local actual
  actual="$("$launcher" "$@")"
  [[ "$actual" == "$want" ]] || { printf '%s %s: unexpected launch\nwant:\n%s\ngot:\n%s\n' "${launcher#"$work"/}" "$*" "$want" "$actual" >&2; exit 1; }
}

for launcher in "$work/bin/figcraft" "$work/app/figcraft-nightly"; do
  # A plain launch starts the nightly profile and passes the files after the delimiter.
  check "$launcher" "$(printf 'CONFIG=%s\nDATA=%s\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<design with spaces.figcraft>\n<--other-name.figcraft>\n' \
    "$HOME/.config/figcraft-nightly" "$HOME/.local/share/figcraft-nightly" "$profile")" \
    'design with spaces.figcraft' '--other-name.figcraft'
  # A delimiter the caller already wrote is not doubled.
  check "$launcher" "$(printf 'CONFIG=%s\nDATA=%s\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<%s>\n' \
    "$HOME/.config/figcraft-nightly" "$HOME/.local/share/figcraft-nightly" "$profile" 'a.figcraft')" \
    -- 'a.figcraft'
  # The Launcher's own protocol passes through untouched, in either order, with no second pair and
  # without exporting this channel's folders over the root it named.
  check "$launcher" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<%s>\n' \
    "$work/launcher profile" 'chosen.figcraft')" \
    --launch-profile nightly-v1 --profile-root "$work/launcher profile" -- 'chosen.figcraft'
  check "$launcher" "$(printf 'CONFIG=\nDATA=\n<--profile-root>\n<%s>\n<-->\n' "$work/launcher profile")" \
    --profile-root "$work/launcher profile" --
  # A version query goes straight to the GUI: no exports, no profile, nothing created.
  for flag in --version -V; do
    check "$launcher" "$(printf 'CONFIG=\nDATA=\n<%s>\n' "$flag")" "$flag"
  done
  [[ ! -e "$profile" ]] || { echo "${launcher#"$work"/}: a version or Launcher launch created the profile" >&2; exit 1; }
done

# The wrapper hands the GUI the XDG bases it overrode, so the stable folders under them stay fenced.
stable_recorder="$work/bin/stable-bases"
cat >"$stable_recorder" <<'RECORDER'
#!/bin/sh
printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "${FIGCRAFT_STABLE_XDG_CONFIG_HOME:-}" "${FIGCRAFT_STABLE_XDG_DATA_HOME:-}"
RECORDER
chmod 755 "$stable_recorder"
install_launcher "$work/bin/stable-launch" "\"$stable_recorder\""
check "$work/bin/stable-launch" "$(printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "$HOME/.config" "$HOME/.local/share")"
XDG_CONFIG_HOME="$work/xdg config" XDG_DATA_HOME="$work/xdg data" \
  check "$work/bin/stable-launch" "$(printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "$work/xdg config" "$work/xdg data")"

echo "PASS: the nightly launcher starts the nightly-v1 profile in every form and passes a supplied one through"
