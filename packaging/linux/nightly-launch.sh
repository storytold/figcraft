#!/bin/sh
# FigCraft Nightly launch wrapper, shared by every nightly Linux format (the flat .tar.gz, the Arch
# handoff, the deb/rpm and the AppImage entry). It starts the GUI on the Launcher's nightly-v1
# profile under a per-channel directory, so a nightly install never reads or writes the stable
# app's settings, recovery copies, plugins, font store or session state.
#
# The GUI binary this starts is substituted where the script is installed: beside the script as
# figcraft-bin in a tarball, $APPDIR/usr/bin/figcraft in an AppImage, /usr/lib/figcraft-nightly/… in
# a deb/rpm, and bin/.figcraft-nightly/figcraft in the Arch handoff.
#
# The Launcher speaks this same protocol (`--launch-profile nightly-v1 --profile-root <dir> --`),
# so its command line passes straight through rather than being wrapped in a second pair. A bare
# `--version` or `-V` goes straight to the binary too: the answer must be the plain version line,
# and a version query never creates the profile. Everything else starts the nightly profile.
set -eu
real_bin=@REAL_BIN@
case "${1:-}" in
  --launch-profile | --profile-root | --version | -V) exec "$real_bin" "$@" ;;
esac
config_base="${XDG_CONFIG_HOME:-${HOME:?HOME is required}/.config}"
data_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
# The bases in effect before the override below: the GUI's profile fence still refuses the stable
# FigCraft folders under them (profile_root.rs reads FIGCRAFT_STABLE_<key>).
export FIGCRAFT_STABLE_XDG_CONFIG_HOME="$config_base"
export FIGCRAFT_STABLE_XDG_DATA_HOME="$data_base"
# Belt and braces for anything that still asks the process environment for a folder.
export XDG_CONFIG_HOME="$config_base/figcraft-nightly"
export XDG_DATA_HOME="$data_base/figcraft-nightly"
# One profile per channel, chosen here only: a caller cannot point the Nightly at another folder,
# so the fence the GUI applies can never be handed the stable app's config or data directory.
profile="$XDG_DATA_HOME/profile"
# A `--` the caller wrote already separates the files from the options: drop it, so the app is
# handed exactly one and no file name arrives as `--`.
if [ "${1:-}" = "--" ]; then shift; fi
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile" -- "$@"
