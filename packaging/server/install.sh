#!/usr/bin/env bash
# Install or upgrade figcraft-server on a Debian or Ubuntu machine, behind Caddy (TLS), as a
# server open to the public. See docs/server.md ("Running a public server").
#
#   sudo ./install.sh --host figcraft.example.com --email you@example.com
#
# The binary comes from this folder when it is an unpacked release archive; else it is built from
# the source tree around this script (a copy of the repository); else it is the latest release for
# this machine (checked against SHA256SUMS.txt); else it is built from a fresh clone of the latest
# release tag. Builds run as an unprivileged user, with a temporary swap file under 2 GB of memory.
# It makes the `figcraft` user, its data directory and /etc/figcraft-server/server.toml (only when
# missing: your edits are kept), installs the systemd units and its own Caddy site, opens SSH, 80
# and 443 in ufw and starts it all. Safe to run again: it upgrades and restarts only on change.
#
# Options:
#   --host HOST        the DNS name people use (its A/AAAA records must point at this machine)
#   --email EMAIL      for Let's Encrypt and, in a new config, the abuse contact shown to users
#   --terms URL        in a new config: your terms of use (https://...)
#   --private          in a new config: only people with the server key publish (not open)
#   --version X.Y.Z    install this release (default: the latest)
#   --from-source      build from source even when a release archive exists
#   --ref REF          the git branch or tag to clone and build (default: vX.Y.Z with --version,
#                      else the latest release tag; a source tree around this script is built as is)
#   --backups          also enable the nightly backup timer (stops the server for a few seconds)
#   --no-firewall      leave ufw alone
#   --firewall-anyway  enable ufw even though other services listen on this machine
#   -h, --help         this text
set -euo pipefail

REPO="storytold/figcraft"
CONFIG_DIR=/etc/figcraft-server
CONFIG="$CONFIG_DIR/server.toml"
DATA=/var/lib/figcraft-server
BIN=/usr/local/bin/figcraft-server
BUILD=/var/cache/figcraft-build
BUILDER=figcraft-build
# Root's own work (downloads, its clone, the swap file, the built binary once copied out): never
# a place the build user can touch.
WORK=/var/cache/figcraft-install
SERVICE=figcraft-server
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The source tree this script sits in (packaging/server/ of a copy or checkout), if any.
LOCAL_SRC=""
if [ -f "$HERE/../../Cargo.toml" ] && [ -d "$HERE/../../apps/figcraft-server" ]; then
  LOCAL_SRC="$(cd "$HERE/../.." && pwd)"
fi

HOST=""
EMAIL=""
TERMS=""
OPEN=true
VERSION=""
FROM_SOURCE=false
REF=""
BACKUPS=false
FIREWALL=true
FIREWALL_ANYWAY=false

say() { printf '==> %s\n' "$*"; }
warn() { printf 'install.sh: warning: %s\n' "$*" >&2; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

MARK="# figcraft-server (install.sh)"

caddy_main() { # caddy_main FILE EMAIL: the main Caddyfile to install, on stdout
  local file="$1" email="$2"
  if [ -f "$file" ] && grep -qE '^[[:space:]]*import[[:space:]]+figcraft-server\.caddy[[:space:]]*$' "$file"; then
    cat "$file" # it imports our site already: kept exactly as it is
  elif [ ! -f "$file" ] || ! grep -qvE '^[[:space:]]*(#|$|:80|\{|\}|root|file_server)' "$file"; then
    # Missing, or only the example site the package ships: ours, with the certificate email.
    printf '%s\n{\n\temail %s\n}\n\nimport figcraft-server.caddy\n' "$MARK" "$email"
  else
    # Someone else's sites: kept, with one line added that imports ours.
    cat "$file"
    printf '\nimport figcraft-server.caddy\n'
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    # For tests (packaging/server/install-test.sh): print the Caddyfile this script would write.
    --caddy-main) caddy_main "${2:-}" "${3:-}"; exit 0 ;;
    --host) HOST="${2:-}"; shift 2 ;;
    --email) EMAIL="${2:-}"; shift 2 ;;
    --terms) TERMS="${2:-}"; shift 2 ;;
    --private) OPEN=false; shift ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --from-source) FROM_SOURCE=true; shift ;;
    --ref) REF="${2:-}"; shift 2 ;;
    --backups) BACKUPS=true; shift ;;
    --no-firewall) FIREWALL=false; shift ;;
    --firewall-anyway) FIREWALL_ANYWAY=true; shift ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "run it as root (sudo ./install.sh ...)"
# shellcheck source=/dev/null
. /etc/os-release
case " ${ID:-} ${ID_LIKE:-} " in
  *" debian "* | *" ubuntu "*) ;;
  *) die "this script is for Debian and Ubuntu (see docs/server.md for other systems)" ;;
esac

ask() { # ask VAR "question" FLAG: read an answer from the terminal when the value wasn't given
  local var="$1" question="$2" answer
  if [ -z "${!var}" ]; then
    [ -t 0 ] || die "give $3 (no terminal to ask on)"
    read -r -p "$question " answer
    printf -v "$var" '%s' "$answer"
  fi
}
ask HOST "DNS name people will use (e.g. figcraft.example.com):" "--host"
ask EMAIL "Email for Let's Encrypt and the abuse contact shown to users:" "--email"
case "$HOST" in
  "" | *[!A-Za-z0-9.-]* | .* | *. ) die "--host must be a DNS name like figcraft.example.com" ;;
esac
case "$EMAIL" in
  *@*.*) ;;
  *) die "--email must be an email address" ;;
esac
case "$TERMS" in
  "" | https://* | http://*) ;;
  *) die "--terms must be a web address (https://...)" ;;
esac
case "$EMAIL$TERMS" in
  *[[:space:]\"\\]*) die "--email and --terms can't contain spaces, quotes or backslashes" ;;
esac

export DEBIAN_FRONTEND=noninteractive
say "Packages"
apt-get update -qq
apt-get install -y -qq curl ca-certificates tar caddy ufw >/dev/null
install -d -o root -g root -m 700 "$WORK"

ARCH="$(uname -m)"
case "$ARCH" in
  amd64) ARCH=x86_64 ;;
  arm64) ARCH=aarch64 ;;
esac

# --- The binary -----------------------------------------------------------------------------
NEW_BIN=""
FILES="$HERE" # where the units and the Caddyfile come from

from_release() { # download and check the release archive; sets NEW_BIN and FILES
  local version="$VERSION" name dir
  if [ -z "$version" ]; then
    version="$(curl --proto '=https' -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n 1)" || return 1
    [ -n "$version" ] || return 1
  fi
  name="figcraft-server-$version-linux-$ARCH"
  dir="$WORK/release"
  rm -rf "$dir"
  mkdir -p "$dir"
  say "Downloading $name"
  curl --proto '=https' -fsSL -o "$dir/$name.tar.gz" "https://github.com/$REPO/releases/download/v$version/$name.tar.gz" || return 1
  curl --proto '=https' -fsSL -o "$dir/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/v$version/SHA256SUMS.txt" || return 1
  (cd "$dir" && grep " $name.tar.gz\$" SHA256SUMS.txt | sha256sum -c --quiet -) || die "the download doesn't match SHA256SUMS.txt"
  tar -C "$dir" -xzf "$dir/$name.tar.gz"
  NEW_BIN="$dir/$name/figcraft-server"
  FILES="$dir/$name"
}

SWAP=""
cleanup() {
  if [ -n "$SWAP" ]; then
    swapoff "$SWAP" 2>/dev/null || true
    rm -f "$SWAP"
  fi
}
trap cleanup EXIT

as_builder() { # run a command as the unprivileged build user, with its own toolchain
  local jobs=()
  if [ -n "${CARGO_BUILD_JOBS:-}" ]; then jobs=("CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS"); fi
  runuser -u "$BUILDER" -- env HOME="$BUILD" RUSTUP_HOME="$BUILD/rustup" CARGO_HOME="$BUILD/cargo" \
    CARGO_TARGET_DIR="$BUILD/target" "${jobs[@]}" "$@"
}

from_source() { # build the source tree around this script, else a fresh clone; sets NEW_BIN and FILES
  local mem_kb src ref="$REF" copy="$BUILD/src"
  apt-get install -y -qq build-essential git pkg-config >/dev/null
  id -u "$BUILDER" >/dev/null 2>&1 || useradd --system --home-dir "$BUILD" --shell /usr/sbin/nologin "$BUILDER"
  install -d -o "$BUILDER" -g "$BUILDER" -m 755 "$BUILD"
  if [ ! -x "$BUILD/cargo/bin/cargo" ]; then
    as_builder sh -c "curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path" >/dev/null
  else
    as_builder "$BUILD/cargo/bin/rustup" update stable >/dev/null
  fi
  # The source root installs files from is root's own (the copy beside this script, or root's
  # clone); the build user only ever builds a copy of it.
  if [ -n "$LOCAL_SRC" ]; then
    src="$LOCAL_SRC"
    say "Building the source in $LOCAL_SRC: this takes a while"
  else
    src="$WORK/src"
    if [ -d "$src/.git" ]; then
      git -C "$src" fetch --quiet --tags origin
    else
      git clone --quiet "https://github.com/$REPO" "$src" ||
        die "can't clone $REPO (a private repository: copy the source to this machine and run its packaging/server/install.sh)"
    fi
    if [ -z "$ref" ]; then
      if [ -n "$VERSION" ]; then ref="v$VERSION"; else ref="$(git -C "$src" tag --list 'v*' --sort=-v:refname | head -n 1)"; fi
      [ -n "$ref" ] || ref=main
    fi
    say "Building $ref from a clone: this takes a while"
    # A branch as it is upstream now, or a tag.
    git -C "$src" checkout --quiet --force --detach "origin/$ref" 2>/dev/null || git -C "$src" checkout --quiet --force --detach "$ref"
  fi
  # The build user clears and fills its own copy (root never touches paths it controls).
  as_builder rm -rf "$copy"
  as_builder mkdir -p "$copy"
  tar -C "$src" --exclude=./target --exclude=./plugins/target --exclude=./.git -cf - . | as_builder tar -C "$copy" -xf -
  # The compiler needs about 2 GB: add a temporary swap file on smaller machines.
  mem_kb="$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)"
  if [ "$mem_kb" -lt 2000000 ] && [ -z "$(swapon --show --noheadings)" ]; then
    SWAP="$WORK/swapfile"
    say "Adding a temporary 2 GB swap file for the build"
    fallocate -l 2G "$SWAP" 2>/dev/null || dd if=/dev/zero of="$SWAP" bs=1M count=2048 status=none
    chmod 600 "$SWAP"
    mkswap "$SWAP" >/dev/null
    swapon "$SWAP"
    export CARGO_BUILD_JOBS=1
  fi
  (cd "$copy" && as_builder "$BUILD/cargo/bin/cargo" build --release --locked -p figcraft-server)
  cleanup
  SWAP=""
  # Copied out with the build user's rights into a file root made: a link the build user planted
  # can't make root read (or install) anything else.
  NEW_BIN="$WORK/figcraft-server.built"
  rm -f "$NEW_BIN"
  as_builder cat "$BUILD/target/release/figcraft-server" >"$NEW_BIN" || die "the build left no binary"
  chmod 755 "$NEW_BIN"
  FILES="$src/packaging/server"
}

if [ -x "$HERE/figcraft-server" ] && [ "$FROM_SOURCE" = false ]; then
  NEW_BIN="$HERE/figcraft-server" # run from an unpacked release archive
elif [ -n "$LOCAL_SRC" ] && [ -z "$VERSION" ]; then
  from_source # the source copied to (or checked out on) this machine, as it is
elif [ "$FROM_SOURCE" = true ] || ! from_release; then
  [ "$FROM_SOURCE" = true ] || say "No release archive for linux-$ARCH: building from source instead"
  from_source
fi
[ -x "$NEW_BIN" ] || die "no figcraft-server binary to install"
# The units and the Caddyfile beside this script when it has them (a checkout or an archive).
[ -f "$HERE/figcraft-backup.timer" ] && FILES="$HERE"
for f in figcraft-server.service Caddyfile figcraft-backup.service figcraft-backup.timer; do
  [ -f "$FILES/$f" ] || die "$FILES/$f is missing"
done

# The server's own account runs the binary, from here on too (root never runs what was built).
id -u figcraft >/dev/null 2>&1 || useradd --system --home-dir "$DATA" --shell /usr/sbin/nologin figcraft
as_server() { runuser -u figcraft -- "$@"; }

CHANGED=false
if ! cmp -s "$NEW_BIN" "$BIN"; then
  install -m 755 "$NEW_BIN" "$BIN.new"
  mv -f "$BIN.new" "$BIN"
  say "Installed $(as_server "$BIN" --version)"
  CHANGED=true
fi

# --- Data and settings ----------------------------------------------------------------------
install -d -o figcraft -g figcraft -m 700 "$DATA"
install -d -m 755 "$CONFIG_DIR"
if [ ! -f "$CONFIG" ]; then
  say "Writing $CONFIG"
  {
    printf '# figcraft-server settings (docs/server.md, "Configuration"). install.sh never overwrites this file.\n'
    printf 'listen = "127.0.0.1:47900"\n'
    printf 'data = "%s"\n' "$DATA"
    printf 'public_url = "wss://%s/"\n' "$HOST"
    printf '# Caddy on this machine: its X-Forwarded-For names the client.\n'
    printf 'trusted_proxy = "127.0.0.1"\n'
    printf '# Anyone may publish without the key (false: only people with the key).\n'
    printf 'open_publish = %s\n' "$OPEN"
    printf '# Shown to people where they publish: how to reach you about abuse, and your terms.\n'
    printf 'operator_contact = "%s"\n' "$EMAIL"
    printf 'terms_url = "%s"\n' "$TERMS"
    printf '# Files published without the key are deleted after this many days unopened.\n'
    printf 'idle_days = 30\n'
    printf 'max_file_mb = 8\n'
    printf 'max_plugin_mb = 8\n'
    printf '# All files together (leave room for the nightly backups on the same disk), and per address.\n'
    printf 'max_total_mb = 6144\n'
    printf 'max_files_per_ip = 20\n'
    printf 'max_mb_per_ip = 256\n'
    printf '# Per address: an office behind one address shares these (see docs/server.md for every limit).\n'
    printf 'messages_per_second = 600\n'
    printf 'connects_per_minute = 60\n'
  } >"$CONFIG.new"
  chmod 644 "$CONFIG.new"
  # Checked before it replaces anything.
  as_server "$BIN" --config "$CONFIG.new" check >/dev/null || { rm -f "$CONFIG.new"; die "the new settings don't read (see above)"; }
  mv -f "$CONFIG.new" "$CONFIG"
  CHANGED=true
fi
as_server "$BIN" --config "$CONFIG" check >/dev/null ||
  die "$CONFIG isn't right for this figcraft-server (see above; a release older than open hosting needs --from-source)"

install_file() { # install_file SOURCE TARGET MODE: true when it changed
  if cmp -s "$1" "$2"; then return 1; fi
  install -m "$3" "$1" "$2"
}

# --- systemd --------------------------------------------------------------------------------
UNITS=false
install_file "$FILES/figcraft-server.service" /etc/systemd/system/figcraft-server.service 644 && UNITS=true
install_file "$FILES/figcraft-backup.service" /etc/systemd/system/figcraft-backup.service 644 && UNITS=true
install_file "$FILES/figcraft-backup.timer" /etc/systemd/system/figcraft-backup.timer 644 && UNITS=true
if [ "$UNITS" = true ]; then
  systemctl daemon-reload
  CHANGED=true
fi
systemctl enable --quiet "$SERVICE"
if [ "$CHANGED" = true ] || ! systemctl is-active --quiet "$SERVICE"; then
  say "Starting figcraft-server"
  systemctl restart "$SERVICE"
fi
if [ "$BACKUPS" = true ]; then
  systemctl enable --now --quiet figcraft-backup.timer
fi

# --- Caddy ----------------------------------------------------------------------------------
# Our site lives in a file of its own; the main Caddyfile only imports it (caddy_main). A
# Caddyfile with other sites is never replaced: it only gains the import line, once.
CADDYFILE=/etc/caddy/Caddyfile
SITE_FILE=/etc/caddy/figcraft-server.caddy
SITE="$(mktemp)"
MAIN="$(mktemp)"
{
  printf '%s: written by packaging/server/install.sh (run it again to update).\n' "$MARK"
  sed -e '/^#/d' -e "s/figcraft\.example\.com/$HOST/" "$FILES/Caddyfile" | sed -e '/./,$!d'
} >"$SITE"
caddy_main "$CADDYFILE" "$EMAIL" >"$MAIN"
CADDY_CHANGED=false
for f in "$CADDYFILE" "$SITE_FILE"; do
  if [ -f "$f" ]; then cp -p "$f" "$f.before-figcraft"; else rm -f "$f.before-figcraft"; fi
done
install_file "$SITE" "$SITE_FILE" 644 && CADDY_CHANGED=true
install_file "$MAIN" "$CADDYFILE" 644 && CADDY_CHANGED=true
rm -f "$SITE" "$MAIN"
if [ "$CADDY_CHANGED" = true ]; then
  if ! caddy validate --config "$CADDYFILE" --adapter caddyfile >/dev/null 2>&1; then
    for f in "$CADDYFILE" "$SITE_FILE"; do
      if [ -f "$f.before-figcraft" ]; then cp -p "$f.before-figcraft" "$f"; else rm -f "$f"; fi
    done
    die "Caddy doesn't accept the new configuration (the old one is back; see: caddy validate --config $CADDYFILE)"
  fi
  systemctl reload caddy || systemctl restart caddy
fi
systemctl enable --now --quiet caddy

# --- Firewall -------------------------------------------------------------------------------
if [ "$FIREWALL" = true ]; then
  # The ports sshd really listens on (sudo doesn't pass SSH_CONNECTION on).
  SSH_PORTS="$(sshd -T 2>/dev/null | awk '/^port / {print $2}' | sort -u)"
  [ -n "$SSH_PORTS" ] || SSH_PORTS=22
  say "Firewall: SSH ($(echo "$SSH_PORTS" | tr '\n' ' ' | sed 's/ $//')), 80 and 443"
  for p in $SSH_PORTS; do ufw allow "$p/tcp" >/dev/null; done
  ufw allow 80/tcp >/dev/null
  ufw allow 443/tcp >/dev/null
  if ufw status | grep -q '^Status: active'; then
    :
  else
    # Other services reachable from outside would be cut off by enabling ufw.
    OTHERS=""
    for a in $(ss -Hltn | awk '{print $4}'); do
      case "$a" in 127.* | "[::1]:"* | ::1:*) continue ;; esac
      port="${a##*:}"
      case " $(echo "$SSH_PORTS" | tr '\n' ' ') 80 443 " in *" $port "*) continue ;; esac
      OTHERS="$OTHERS $port"
    done
    if [ -n "$OTHERS" ] && [ "$FIREWALL_ANYWAY" = false ]; then
      warn "other services listen on port(s)$OTHERS: ufw was not enabled (rules added; allow those ports, then 'ufw enable', or run again with --firewall-anyway)"
    else
      ufw --force enable >/dev/null
    fi
  fi
fi

# --- Done -----------------------------------------------------------------------------------
for _ in $(seq 1 50); do
  curl -fs http://127.0.0.1:47900/healthz >/dev/null && break
  sleep 0.2
done
curl -fs http://127.0.0.1:47900/healthz >/dev/null || die "figcraft-server isn't answering: journalctl -u $SERVICE"
ADMIN="sudo -u figcraft figcraft-server --config $CONFIG"
cat <<EOF

figcraft-server is running behind Caddy.

  Publish:   in FigCraft, Share > Publish to server..., address wss://$HOST/
             $(if [ "$OPEN" = true ]; then echo "(open to the public: no key needed; each file gets an owner token)"; else echo "(with the key: $ADMIN key)"; fi)
  Check:     curl https://$HOST/healthz      (once DNS points here and Caddy has its certificate)
  Logs:      journalctl -u $SERVICE -f

  Moderate:  $ADMIN files list --sort size
             $ADMIN files remove FILE
             $ADMIN reports
             $ADMIN block-ip ADDRESS      (unblock-ip, blocked)
  Server key: $ADMIN key
  Settings:  $CONFIG (then: systemctl restart $SERVICE)
  Backups:   systemctl enable --now figcraft-backup.timer   (nightly, to /var/backups/figcraft-server;
             the archives hold every file and the server key: keep them as private as the server)
  Upgrade:   run this script again

You are responsible for what people publish here: keep the abuse contact and terms current.
Files are not end-to-end encrypted: whoever runs the server can read them.
EOF
