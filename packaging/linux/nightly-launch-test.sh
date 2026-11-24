#!/usr/bin/env bash
# The nightly launcher (packaging/linux/nightly-launch.sh) in each form the formats install it:
# beside the GUI in a tarball, at /usr/lib in a package, and inside an AppImage. Runs in a second
# with a recording stub; it builds nothing and opens no window.
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

# The tarball: the launcher beside the GUI it starts.
recorder "$work/bin/figcraft"
install_launcher "$work/bin/figcraft-nightly" '"$(dirname "$0")/figcraft"'
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

for launcher in "$work/bin/figcraft-nightly" "$work/app/figcraft-nightly"; do
  name="${launcher#"$work"/}"
  # A plain launch starts the nightly profile and passes the files after the delimiter.
  actual="$("$launcher" 'design with spaces.figcraft' '--other-name.figcraft')"
  want="$(printf 'CONFIG=%s\nDATA=%s\n<--launch-profile>\n<nightly-v1>\n<--profile-root>\n<%s>\n<-->\n<design with spaces.figcraft>\n<--other-name.figcraft>\n' \
    "$HOME/.config/figcraft-nightly" "$HOME/.local/share/figcraft-nightly" "$profile")"
  [[ "$actual" == "$want" ]] || { printf '%s: unexpected launch\nwant:\n%s\ngot:\n%s\n' "$name" "$want" "$actual" >&2; exit 1; }
  # A version query goes straight to the GUI: no exports, no profile, nothing created.
  for flag in --version -V; do
    actual="$("$launcher" "$flag")"
    want="$(printf 'CONFIG=\nDATA=\n<%s>\n' "$flag")"
    [[ "$actual" == "$want" ]] || { printf '%s %s: unexpected query\nwant:\n%s\ngot:\n%s\n' "$name" "$flag" "$want" "$actual" >&2; exit 1; }
  done
  [[ ! -e "$profile" ]] || { echo "$name: a version query created the profile" >&2; exit 1; }
done

# An explicit profile directory wins, for a caller that manages its own.
FIGCRAFT_NIGHTLY_PROFILE="$work/chosen profile" "$work/bin/figcraft-nightly" file.figcraft >"$work/out"
grep -Fxq "<$work/chosen profile>" "$work/out" || { echo "FIGCRAFT_NIGHTLY_PROFILE ignored: $(cat "$work/out")" >&2; exit 1; }

echo "PASS: the nightly launcher starts the nightly-v1 profile in every form"
