#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.figcraft-nightly/figcraft"
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  exec "$real_bin" "$@"
fi
config_base="${XDG_CONFIG_HOME:-${HOME:?HOME is required}/.config}"
data_base="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
export XDG_CONFIG_HOME="$config_base/figcraft-nightly"
export XDG_DATA_HOME="$data_base/figcraft-nightly"
exec "$real_bin" "$@"
