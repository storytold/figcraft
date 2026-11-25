#!/usr/bin/env bash
# Build and package FigCraft for Linux (<arch> is x86_64 or aarch64, or riscv64 when cross-compiling):
#
#   $DIST/figcraft-<version>-linux-<arch>.AppImage  any distro with glibc >= the build host's
#   $DIST/figcraft-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/figcraft-<version>-linux-<arch>.deb       Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/figcraft-<version>-linux-<arch>.rpm       Fedora, openSUSE, RHEL, ...
#   $DIST/figcraft-<version>-linux-<arch>.tar.gz    plain FHS-style tree (bin/, share/)
#
# figcraft-cli is built and shipped next to figcraft once apps/figcraft-cli exists.
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"] [--arch-tar-dir DIRECTORY]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage
# (downloaded into $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04,
# glibc 2.35) so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli,
# zsyncmake (the zsync package) for the AppImage's .zsync.
#
# Cross-compiling (CI's linux-riscv64 job; tar only): CROSS_ARCH (e.g. riscv64), CROSS_TARGET
# (the Rust target), CROSS_COMPILE (binutils prefix for strip) and optionally EMULATOR (e.g.
# qemu-riscv64) to run the --version smoke test.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=ai.storyteller.figcraft

verify_sha256() {
  local expected="$1"
  local file="$2"
  printf '%s  %s\n' "$expected" "$file" | sha256sum -c -
}

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
ARCH_TAR_DIR=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    --arch-tar-dir)
      [[ -n "${2:-}" ]] || { echo "error: --arch-tar-dir needs a directory" >&2; exit 2; }
      ARCH_TAR_DIR="$2"
      shift 2
      ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="${CROSS_ARCH:-$(uname -m)}"
case "$ARCH" in
  riscv64) DEB_ARCH=riscv64 ;;
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export FIGCRAFT_MAINTAINER="${FIGCRAFT_MAINTAINER:-FigCraft maintainers <figcraft@storyteller.ai>}"
IS_NIGHTLY=0
if [[ "${FIGCRAFT_BUILD_CHANNEL:-}" == nightly-v1 || "$VERSION" == *-nightly* ]]; then
  IS_NIGHTLY=1
  export FIGCRAFT_BUILD_CHANNEL=nightly-v1
  export FIGCRAFT_OVERRIDE_VERSION="$VERSION"
fi
BASENAME="figcraft-$VERSION-linux-$ARCH"

if [[ -n "$ARCH_TAR_DIR" && "$IS_NIGHTLY" != 1 ]]; then
  echo "error: --arch-tar-dir is only for Nightly builds" >&2
  exit 2
fi

echo "==> FigCraft $VERSION for Linux $ARCH ($FORMATS; ${BINS[*]}; channel=${FIGCRAFT_BUILD_CHANNEL:-stable})"

if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked "${CARGO_PKGS[@]}" ${CROSS_TARGET:+--target "$CROSS_TARGET"})
fi
BIN="$CARGO_TARGET_DIR/${CROSS_TARGET:+$CROSS_TARGET/}release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
for bin in "${BINS[@]}"; do
  install -Dm755 "$BIN/$bin" "$STAGE/usr/bin/$bin"
  "${CROSS_COMPILE:-}strip" "$STAGE/usr/bin/$bin" 2>/dev/null || true
done
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
if [[ "$IS_NIGHTLY" = 1 ]]; then
  # The nightly entry point is the shared launcher (packaging/linux/nightly-launch.sh): every
  # format starts the GUI on the Launcher's nightly-v1 profile, so nothing reads or writes the
  # stable app's user state. The deb/rpm wrapper and the AppImage entry below specialise its path.
  awk -v real='"$(dirname "$0")/figcraft"' '{ gsub(/@REAL_BIN@/, real); print }' \
    "$HERE/nightly-launch.sh" >"$STAGE/usr/bin/figcraft-nightly"
  chmod 755 "$STAGE/usr/bin/figcraft-nightly"
  # The entry starts the launcher, names the Nightly and registers no file types (a Nightly never
  # takes a document association from the stable app). `#` delimits, so \(...\|...\) is alternation.
  sed -e 's|^StartupWMClass=.*|StartupWMClass=ai.storyteller.figcraft.nightly-v1|' \
      -e "s#^Exec=figcraft\([[:space:]]\|$\)#Exec=figcraft-nightly\1#" \
      -e "s|^TryExec=figcraft$|TryExec=figcraft-nightly|" \
      -e "s|^Name=FigCraft$|Name=FigCraft Nightly|" \
      -e '/^MimeType=/d' \
      "$STAGE/usr/share/applications/$APP_ID.desktop" >"$STAGE/usr/share/applications/$APP_ID.desktop.tmp"
  mv "$STAGE/usr/share/applications/$APP_ID.desktop.tmp" "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
# No document associations for a Nightly: the mime package, the desktop's MimeType lines and the
# metainfo mediatypes all stay out, so it never takes a file type from the stable app.
if [[ "$IS_NIGHTLY" = 0 ]]; then
  install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
fi
mkdir -p "$STAGE/usr/share/metainfo"
# The AppStream metadata carries the Nightly's name and launcher and no document types. Its id,
# launchable and icon keep the stable values here: the deb/rpm, the AppImage and the Flatpak each
# rename them to the id they actually install (and the Flatpak's differs from the AppImage's).
META_NIGHTLY=()
if [[ "$IS_NIGHTLY" = 1 ]]; then
  META_NIGHTLY=(-e "s|<name>FigCraft</name>|<name>FigCraft Nightly</name>|g"
    -e 's|<binary>figcraft</binary>|<binary>figcraft-nightly</binary>|g'
    -e '/<mediatype>/d')
fi
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$FIGCRAFT_BUILD_DATE/g" "${META_NIGHTLY[@]}" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/figcraft"
copy_docs "$STAGE/usr/share/doc/figcraft"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

has() { local format="$1"; case " $FORMATS " in *" $format "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  # GUI only: a Nightly has no CLI (the stable tarball keeps shipping it).
  if [[ "$IS_NIGHTLY" = 1 ]]; then rm -f "$WORK/tar/$BASENAME/bin/figcraft-cli"; fi
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# The Arch handoff keeps the same basename as the plain tarball in a separate directory.
# Flatpak, OCI and direct tar consumers still get the original binary, not this launcher.
if [[ -n "$ARCH_TAR_DIR" ]]; then
  mkdir -p "$ARCH_TAR_DIR"
  if [[ "$(cd "$ARCH_TAR_DIR" && pwd -P)" == "$(cd "$DIST" && pwd -P)" ]]; then
    echo "error: the Arch tar directory must differ from the release directory" >&2
    exit 2
  fi
  bash "$HERE/nightly-arch-tar.sh" "$STAGE/usr" "$WORK/arch-tar" "$ARCH_TAR_DIR" "$BASENAME"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  NFPM_STAGE="$STAGE"
  export NFPM_NAME="figcraft"
  export NFPM_BIN="figcraft"
  export NFPM_CLI="figcraft-cli"
  if [[ "$IS_NIGHTLY" = 1 ]]; then
    NIGHTLY_DESK_ID="$APP_ID.nightly-v1"
    NIGHTLY_ICON="$ROOT/assets/app-icon/figcraft-nightly.png"
    [[ -f "$NIGHTLY_ICON" ]] || NIGHTLY_ICON="$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png"
    export NFPM_NAME="figcraft-nightly"
    export NFPM_BIN="figcraft-nightly"
    export NFPM_CLI="figcraft-nightly-cli"
    NFPM_STAGE="$WORK/nfpm-root"
    rm -rf "$NFPM_STAGE"
    cp -R "$STAGE" "$NFPM_STAGE"
    install -Dm755 "$NFPM_STAGE/usr/bin/figcraft" "$NFPM_STAGE/usr/lib/figcraft-nightly/figcraft"
    # The shared launcher (packaging/linux/nightly-launch.sh) starts the GUI on the Launcher's
    # nightly-v1 profile, so the package never reads or writes the stable app's user state.
    awk -v real='/usr/lib/figcraft-nightly/figcraft' '{ gsub(/@REAL_BIN@/, real); print }' \
      "$HERE/nightly-launch.sh" >"$NFPM_STAGE/usr/bin/$NFPM_BIN"
    chmod 755 "$NFPM_STAGE/usr/bin/$NFPM_BIN"
    rm -f "$NFPM_STAGE/usr/bin/figcraft"
    if [[ "$HAS_CLI" = 1 ]]; then
      mv "$NFPM_STAGE/usr/bin/figcraft-cli" "$NFPM_STAGE/usr/bin/$NFPM_CLI"
    fi
    mv "$NFPM_STAGE/usr/share/doc/figcraft" "$NFPM_STAGE/usr/share/doc/$NFPM_NAME"
    sed -e "s#^Exec=figcraft\([[:space:]]\|$\)#Exec=$NFPM_BIN\1#" \
        -e "s|^TryExec=figcraft$|TryExec=$NFPM_BIN|" \
        -e "s|^Icon=$APP_ID$|Icon=$NIGHTLY_DESK_ID|" \
        -e "s|^Name=FigCraft$|Name=FigCraft Nightly|" \
        -e '/^MimeType=/d' \
        "$NFPM_STAGE/usr/share/applications/$APP_ID.desktop" >"$NFPM_STAGE/usr/share/applications/$NIGHTLY_DESK_ID.desktop"
    rm -f "$NFPM_STAGE/usr/share/applications/$APP_ID.desktop"
    sed -e "s|<id>$APP_ID</id>|<id>$NIGHTLY_DESK_ID</id>|g" \
        -e "s|<name>FigCraft</name>|<name>FigCraft Nightly</name>|g" \
        -e "s|<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>|<launchable type=\"desktop-id\">$NIGHTLY_DESK_ID.desktop</launchable>|g" \
        -e "s|<icon type=\"stock\">$APP_ID</icon>|<icon type=\"stock\">$NIGHTLY_DESK_ID</icon>|g" \
        -e "s|<binary>figcraft</binary>|<binary>$NFPM_BIN</binary>|g" \
        -e '/<mediatype>/d' \
        "$NFPM_STAGE/usr/share/metainfo/$APP_ID.metainfo.xml" >"$NFPM_STAGE/usr/share/metainfo/$NIGHTLY_DESK_ID.metainfo.xml"
    rm -f "$NFPM_STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
    rm -rf "$NFPM_STAGE/usr/share/mime"
    rm -rf "$NFPM_STAGE/usr/share/icons/hicolor"
    install -Dm644 "$NIGHTLY_ICON" \
      "$NFPM_STAGE/usr/share/icons/hicolor/256x256/apps/$NIGHTLY_DESK_ID.png"
  fi
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  # Without the CLI, drop its `#cli` lines.
  DROP_LIB_TREE=
  # shellcheck disable=SC2016
  [[ -d "$NFPM_STAGE/usr/lib" ]] || DROP_LIB_TREE='\|^  - src: \${STAGE}/usr/lib/|,+2d;'
  cli_filter='/#cli$/d'
  [[ "$HAS_CLI" = 1 ]] && cli_filter='s/ #cli$//'
  # shellcheck disable=SC2016
  sed -e "${DROP_LIB_TREE}s|\${STAGE}|$NFPM_STAGE|g;s|\${NFPM_BIN}|$NFPM_BIN|g;s|\${NFPM_CLI}|$NFPM_CLI|g" -e "$cli_filter" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPIMAGETOOL_VERSION=1.9.1
  case "$ARCH" in
    x86_64) APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 ;;
    aarch64) APPIMAGETOOL_SHA256=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158 ;;
    *) APPIMAGETOOL_SHA256='' ;;
  esac
  # appimagetool otherwise fetches the unpinned type2-runtime `continuous` build and embeds it.
  APPIMAGE_RUNTIME_VERSION=20251108
  case "$ARCH" in
    x86_64) APPIMAGE_RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    aarch64) APPIMAGE_RUNTIME_SHA256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
    *) APPIMAGE_RUNTIME_SHA256='' ;;
  esac
  if [[ -z "$APPIMAGE_RUNTIME_SHA256" ]]; then
    echo "no pinned AppImage runtime for $ARCH; build the AppImage on x86_64 or aarch64" >&2
    exit 2
  fi
  APPDIR="$WORK/FigCraft.AppDir"
  cp -R "$STAGE" "$APPDIR"
  # AppRun is a script, not a symlink: it installs the .desktop entry and icons into
  # $XDG_DATA_HOME before launching, so Wayland compositors can resolve the window's
  # app_id to our icon instead of the generic Wayland logo (opt-out env in the
  # script header). Logic is covered by packaging/linux/apprun-test.sh.
  install -Dm755 "$HERE/AppRun" "$APPDIR/AppRun"
  if [[ "$IS_NIGHTLY" = 1 ]]; then
    APPIMAGE_ID="$APP_ID.nightly-v1"
    NIGHTLY_ICON="$ROOT/assets/app-icon/figcraft-nightly.png"
    [[ -f "$NIGHTLY_ICON" ]] || NIGHTLY_ICON="$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png"
    # AppRun runs the staged launcher (`usr/bin/figcraft-nightly`, which starts the GUI beside it).
    # Its `$(dirname "$0")` form resolves the same way under the AppImage runtime and an extracted
    # AppDir, and never needs the runtime's `$APPDIR` to be exported.
    # shellcheck disable=SC2016
    awk -v old_id="$APP_ID" -v new_id="$APPIMAGE_ID" '
      $0 == "DESK_ID=" old_id { print "DESK_ID=" new_id; next }
      $0 == "exec \"$APPDIR/usr/bin/figcraft\" \"$@\"" {
        print "exec \"$APPDIR/usr/bin/figcraft-nightly\" \"$@\""
        next
      }
      { print }
    ' "$APPDIR/AppRun" >"$APPDIR/AppRun.tmp"
    mv "$APPDIR/AppRun.tmp" "$APPDIR/AppRun"
    chmod 755 "$APPDIR/AppRun"
    sed -e "s|^Icon=$APP_ID$|Icon=$APPIMAGE_ID|" \
        -e "s|^Name=FigCraft$|Name=FigCraft Nightly|" \
        -e '/^MimeType=/d' \
        "$APPDIR/usr/share/applications/$APP_ID.desktop" >"$APPDIR/usr/share/applications/$APPIMAGE_ID.desktop"
    rm -f "$APPDIR/usr/share/applications/$APP_ID.desktop"
    cp "$APPDIR/usr/share/applications/$APPIMAGE_ID.desktop" "$APPDIR/$APPIMAGE_ID.desktop"
    # No document associations for a Nightly: the mime package, the desktop's MimeType lines (above)
    # and the metainfo mediatypes all stay out, so it never takes a file type from the stable app.
    rm -rf "$APPDIR/usr/share/mime"
    sed -e "s|<id>$APP_ID</id>|<id>$APPIMAGE_ID</id>|g" \
        -e "s|<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>|<launchable type=\"desktop-id\">$APPIMAGE_ID.desktop</launchable>|g" \
        -e "s|<icon type=\"stock\">$APP_ID</icon>|<icon type=\"stock\">$APPIMAGE_ID</icon>|g" \
        -e "s|<binary>figcraft</binary>|<binary>figcraft-nightly</binary>|g" \
        -e '/<mediatype>/d' \
        "$APPDIR/usr/share/metainfo/$APP_ID.metainfo.xml" >"$APPDIR/usr/share/metainfo/$APPIMAGE_ID.metainfo.xml"
    rm -f "$APPDIR/usr/share/metainfo/$APP_ID.metainfo.xml"
    rm -rf "$APPDIR/usr/share/icons/hicolor"
    install -Dm644 "$NIGHTLY_ICON" \
      "$APPDIR/usr/share/icons/hicolor/256x256/apps/$APPIMAGE_ID.png"
    cp "$NIGHTLY_ICON" "$APPDIR/$APPIMAGE_ID.png"
    ln -s "$APPIMAGE_ID.png" "$APPDIR/.DirIcon"
  else
    cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
    cp "$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
    ln -s "$APP_ID.png" "$APPDIR/.DirIcon"
  fi

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [[ -z "$TOOL" ]]; then
    if [[ -z "$APPIMAGETOOL_SHA256" ]]; then
      echo "no pinned appimagetool for $ARCH; set APPIMAGETOOL or put appimagetool on PATH" >&2
      exit 2
    fi
    TOOL="$CARGO_TARGET_DIR/appimagetool-$APPIMAGETOOL_VERSION-$ARCH.AppImage"
    if [[ ! -x "$TOOL" ]]; then
      download="$TOOL.download"
      curl --proto '=https' -fsSL -o "$download" \
        "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$ARCH.AppImage"
      verify_sha256 "$APPIMAGETOOL_SHA256" "$download"
      chmod 755 "$download"
      mv "$download" "$TOOL"
    fi
    verify_sha256 "$APPIMAGETOOL_SHA256" "$TOOL"
  fi
  RUNTIME="$CARGO_TARGET_DIR/appimage-runtime-$APPIMAGE_RUNTIME_VERSION-$ARCH"
  if [[ ! -f "$RUNTIME" ]]; then
    download="$RUNTIME.download"
    curl --proto '=https' -fsSL -o "$download" \
      "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$ARCH"
    verify_sha256 "$APPIMAGE_RUNTIME_SHA256" "$download"
    mv "$download" "$RUNTIME"
  fi
  verify_sha256 "$APPIMAGE_RUNTIME_SHA256" "$RUNTIME"
  # Absolute, because appimagetool runs in $DIST below (CARGO_TARGET_DIR or APPIMAGETOOL may be
  # relative, e.g. target/agent-<name>).
  OUT="$(cd "$DIST" && pwd)/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  RUNTIME="$(cd "$(dirname "$RUNTIME")" && pwd)/$(basename "$RUNTIME")"
  # A .zsync left by an earlier run would hide a missing zsyncmake and describe another file.
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate, AppImageLauncher and the like read it from the
  # file and fetch only the blocks that changed in a newer release, through the .zsync published
  # next to each AppImage on GitHub Releases. `latest` is the newest published release that is
  # not a pre-release. A fork's builds point at its own releases through GITHUB_REPOSITORY.
  REPO="${GITHUB_REPOSITORY:-storytold/figcraft}"
  UPDATE_ARGS=()
  if [[ "$IS_NIGHTLY" = 0 ]]; then
    UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|figcraft-*-linux-$ARCH.AppImage.zsync"
    UPDATE_ARGS=(-u "$UPDATE_INFO")
  else
    # Nightly releases are GitHub pre-releases (`latest` only selects stable releases), so point
    # zsync update metadata at pre-release tags (`latest-pre`) rather than stable `latest`.
    UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest-pre|figcraft-*-linux-$ARCH.AppImage.zsync"
    UPDATE_ARGS=(-u "$UPDATE_INFO")
  fi
  # Extract-and-run: works without FUSE (containers, CI). The output embeds the static runtime,
  # so users don't need libfuse2 either. With zsyncmake on the host (CI installs the zsync
  # package) appimagetool also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream --runtime-file "$RUNTIME" "${UPDATE_ARGS[@]}" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [[ -s "$OUT.zsync" ]]; then
    echo "wrote $OUT.zsync"
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

if [[ -z "${CROSS_TARGET:-}" ]]; then
  "$STAGE/usr/bin/$(smoke_bin)" --version
elif [[ -n "${EMULATOR:-}" ]]; then
  "$EMULATOR" "$STAGE/usr/bin/$(smoke_bin)" --version
fi
echo "==> done"
ls -lh "$DIST"
