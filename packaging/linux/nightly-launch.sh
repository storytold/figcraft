#!/bin/sh
# FigCraft Nightly launch wrapper, shared by every nightly Linux format (the .tar.gz, the Arch
# handoff, the deb/rpm and the AppImage entry). It starts the GUI on the Launcher's nightly-v1
# profile under a per-channel directory, so a nightly install never reads or writes the stable
# app's settings, recovery copies, plugins, font store or session state.
#
# The GUI binary this starts is substituted where the script is installed: beside the script in a
# tarball or the AppDir, /usr/lib/figcraft-nightly/figcraft in a deb/rpm.
#
# `--version` and `-V` go straight to the binary: the answer must be the plain version line, and a
# version query never creates the profile. Everything else starts the nightly profile.
set -eu
real_bin=@REAL_BIN@
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
config_base="${XDG_CONFIG_HOME:-${HOME:?HOME is required}/.config}"
data_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
# Belt and braces for anything that still asks the process environment for a folder.
export XDG_CONFIG_HOME="$config_base/figcraft-nightly"
export XDG_DATA_HOME="$data_base/figcraft-nightly"
profile="${FIGCRAFT_NIGHTLY_PROFILE:-$XDG_DATA_HOME/profile}"
exec "$real_bin" --launch-profile nightly-v1 --profile-root "$profile" -- "$@"
