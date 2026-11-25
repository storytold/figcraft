#!/usr/bin/env bash
# Both nightly launchers, in each form the formats install them.
#
# The plain nightly tar's `figcraft` is the canonical shared template
# (packaging/linux/nightly-launch.sh, rendered from craft-common-actions's
# scripts/nightly-launcher.sh.in): it starts `figcraft-bin` beside itself on the nightly-v1 profile
# under the Nightly package identity and answers a version query with one plain `--version`.
#
# The distro launchers (`packaging/linux/nightly-package-launch.sh`, with @REAL_BIN@ substituted per
# format) keep their own profile folder and the stable-XDG export the GUI's fence reads: beside the
# GUI in a distro tree, at /usr/lib in a package, and inside an AppImage. Runs in a second with a
# recording stub; it builds nothing and opens no window.
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
install_package_launcher() {
  awk -v real="$2" '{ gsub(/@REAL_BIN@/, real); print }' "$HERE/nightly-package-launch.sh" >"$1"
  chmod 755 "$1"
}

export HOME="$work/home with space"
unset XDG_CONFIG_HOME XDG_DATA_HOME
identity=ai.storyteller.figcraft.nightly
tar_profile="$HOME/.local/share/$identity"
profile="$HOME/.local/share/figcraft-nightly/profile"

check() {
  local launcher="$1" want="$2"
  shift 2
  local actual
  actual="$("$launcher" "$@")"
  [[ "$actual" == "$want" ]] || { printf '%s %s: unexpected launch\nwant:\n%s\ngot:\n%s\n' "${launcher#"$work"/}" "$*" "$want" "$actual" >&2; exit 1; }
}

# ---- the plain nightly tar: the canonical template, the GUI beside it ----
recorder "$work/bin/figcraft-bin"
install -m755 "$HERE/nightly-launch.sh" "$work/bin/figcraft"
tar="$work/bin/figcraft"
check "$tar" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<design with spaces.figcraft>\n<--other-name.figcraft>\n' \
  "$tar_profile")" 'design with spaces.figcraft' '--other-name.figcraft'
check "$tar" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n' "$tar_profile")"
# Both version spellings reach the GUI as one plain `--version`, and no launch creates the profile.
for flag in --version -V; do
  check "$tar" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<--version>\n' "$tar_profile")" "$flag"
done
# The ArtCraft Launcher starts the entry point with its own profile pair; the template hands that
# line to the GUI unchanged and keeps no second pair of its own.
check "$tar" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<chosen.figcraft>\n' \
  "$work/launcher profile")" --launch-profile nightly-v1 --profile-root "$work/launcher profile" -- chosen.figcraft
# Anything else is wrapped in the template's own root, including a profile line that does not open
# the arguments.
check "$tar" "$(printf 'CONFIG=\nDATA=\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<--profile-root>\n<%s>\n<-->\n' \
  "$tar_profile" "$work/launcher profile")" --profile-root "$work/launcher profile" --
[[ ! -e "$tar_profile" ]] || { echo "a version or launch created the tar profile" >&2; exit 1; }
# XDG_DATA_HOME decides the root when it is set.
XDG_DATA_HOME="$work/xdg data" check "$tar" "$(printf 'CONFIG=\nDATA=%s\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<%s>\n' \
  "$work/xdg data" "$work/xdg data/$identity" 'a.figcraft')" 'a.figcraft'
# The template starts the GUI beside itself or nothing.
mv "$work/bin/figcraft-bin" "$work/moved-aside"
if "$tar" 'a.figcraft' >/dev/null 2>&1; then
  echo "the tar launcher started without its GUI beside it" >&2
  exit 1
fi
mv "$work/moved-aside" "$work/bin/figcraft-bin"

# ---- the distro formats: the package launcher, their own profile folder ----
# deb/rpm: the wrapper at /usr/bin, the GUI under /usr/lib. The absolute path cannot resolve in a
# test, so this form is checked by its text (and by the packaging tests that stage a package).
install_package_launcher "$work/deb/figcraft-nightly" '/usr/lib/figcraft-nightly/figcraft'
grep -Fxq 'real_bin=/usr/lib/figcraft-nightly/figcraft' "$work/deb/figcraft-nightly" \
  || { echo "the package wrapper does not start the packaged GUI" >&2; exit 1; }
# AppImage: the nightly entry runs the launcher, which starts the GUI in the AppDir's usr/bin.
mkdir -p "$work/app/usr/bin"
recorder "$work/app/usr/bin/figcraft"
install_package_launcher "$work/app/figcraft-nightly" "\"$work/app/usr/bin/figcraft\""
# A distro tree: bin/figcraft-nightly beside bin/figcraft-bin.
mkdir -p "$work/tree/bin"
recorder "$work/tree/bin/figcraft-bin"
# shellcheck disable=SC2016 # $(dirname "$0") stays literal in the launcher: it resolves at run time
install_package_launcher "$work/tree/bin/figcraft-nightly" '"$(dirname "$0")/figcraft-bin"'

for launcher in "$work/tree/bin/figcraft-nightly" "$work/app/figcraft-nightly"; do
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

# The distro wrapper hands the GUI the XDG bases it overrode, so the stable folders under them stay
# fenced. The tar's canonical template cannot: it exports nothing (see the nightly release notes).
stable_recorder="$work/bin/stable-bases"
cat >"$stable_recorder" <<'RECORDER'
#!/bin/sh
printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "${FIGCRAFT_STABLE_XDG_CONFIG_HOME:-}" "${FIGCRAFT_STABLE_XDG_DATA_HOME:-}"
RECORDER
chmod 755 "$stable_recorder"
install_package_launcher "$work/bin/stable-launch" "\"$stable_recorder\""
check "$work/bin/stable-launch" "$(printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "$HOME/.config" "$HOME/.local/share")"
XDG_CONFIG_HOME="$work/xdg config" XDG_DATA_HOME="$work/xdg data" \
  check "$work/bin/stable-launch" "$(printf 'STABLE_CONFIG=%s\nSTABLE_DATA=%s\n' "$work/xdg config" "$work/xdg data")"

echo "PASS: the tar's template launcher and the distro launcher start the nightly profile in every form"
