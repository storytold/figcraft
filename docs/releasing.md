# Releasing FigCraft

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
installers for macOS, Windows, Linux and the web, plus a FreeBSD tarball and the collaboration
server for Linux, then creates or updates a **draft** GitHub Release named `FigCraft v<version>`.
Nobody sees a draft until a maintainer publishes it.

This is FigCraft's copy of the shared [release playbook](release-playbook.md) (PhotoCraft is the
reference implementation). User-facing names say **FigCraft**. Files, binaries and ids stay
lowercase (`figcraft-<version>-<platform>-<arch>.<ext>`, `ai.storyteller.figcraft`).

## FigCraft's names

| Thing | Value |
|---|---|
| Display name | FigCraft |
| Packages / binaries | `figcraft` (app), `figcraft-cli` (headless CLI), `figcraft-web` (browser) |
| Bundle / app id | `ai.storyteller.figcraft` (CLI signing identifier `ai.storyteller.figcraft-cli`) |
| Native document | `.figcraft`, MIME `application/x-figcraft`, UTI `ai.storyteller.figcraft.document` |
| Also opens | `.svg`, `.png`, `.jpg`/`.jpeg` ("Open with", never the default app) |
| WiX UpgradeCode | `7F8B9827-D065-41B7-A514-2A454B21F484` (fixed forever; never reuse it in another app) |
| Linux categories | `Graphics;2DGraphics;VectorGraphics;X-Design;` (`Design` isn't a registered freedesktop category, so `desktop-file-validate` rejects it) |

## Cutting a release

1. **Bump the version** on `main`. It lives only in `[workspace.package] version` in the root
   `Cargo.toml`:

   ```sh
   cargo run -p xtask -- version              # prints the current version, e.g. 0.1.0
   cargo run -p xtask -- version set 0.2.0    # or 0.2.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   (`cargo xtask …` is the same once `.cargo/config.toml` has the `xtask = "run -p xtask --"`
   alias.) Commit `Cargo.toml` + `Cargo.lock`.
2. **Fast-forward `release` to `main`** and push (release managers only; the ruleset enforces it).
   The workflow starts by itself.
3. **Wait for the draft** (`FigCraft v0.2.0`, tagged `v0.2.0` on the pushed commit) with every
   artifact and `SHA256SUMS.txt`. Notes are generated from the merged PRs.
4. **Check it.** Any `::warning::` in the job summaries means a signing secret was missing and
   that artifact is unsigned: fix the secret and re-run, don't publish.
5. **Publish** the draft. Versions with a pre-release suffix (`-rc.1`) are marked pre-release.

Pushing to `release` again before publishing rebuilds the same draft and replaces its assets.
After the draft is published, the workflow refuses to touch that version again, so bump first.

**Dry runs:** `gh workflow run release.yml --ref <branch> -f version=0.2.0-rc.1` builds every
unsigned platform (Linux, Flatpak, FreeBSD, Web, the server) on any branch. Only the macOS,
Windows and draft-release jobs declare `environment: release`; off the `release` branch the
environment refuses them (0 steps) and no release is created. The `version` input is applied with
`xtask version set` before building, so the binaries report it too.

## The default server

Share ▸ Online uses the server in Preferences, which starts as the public one built into the app
(`figcraft_engine::live::DEFAULT_SERVER`). Distributions that would rather not ship that default
set `FIGCRAFT_DEFAULT_SERVER` when building: to their own server's address (`wss://host/`), or to
nothing for a build with no default, where online sharing stays off until the user enters a
server (sharing on the local network works either way):

```sh
FIGCRAFT_DEFAULT_SERVER= cargo build --release -p figcraft          # no default server
FIGCRAFT_DEFAULT_SERVER=wss://design.example.org/ cargo build --release -p figcraft
```

The release workflow leaves it unset. The public server's terms are in
[`online-sharing-terms.md`](online-sharing-terms.md).

## The default font

New text starts in Inter (`figcraft_doc::DEFAULT_FONT`). A distribution whose users mostly write a
script Inter lacks (Thai, say) can name another family with `FIGCRAFT_DEFAULT_FONT` when building;
unset or empty means Inter. Pick a family the build embeds from craft-fonts (the web build has no
other fonts) or one its users have installed:

```sh
FIGCRAFT_DEFAULT_FONT="Noto Sans Thai" cargo build --release -p figcraft
```

Files record each text's family, so the variable only changes what new text starts in: a file
saved by one build keeps its fonts in a build with another default. Like any other family, it
renders only where it is available (embedded or installed); elsewhere the text falls back to the
faces that are. The release workflow leaves it unset.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| macOS 11+ (universal) | `figcraft-<v>-macos-universal.dmg`, `figcraft-cli-<v>-macos-universal.zip` | `macos-15-xlarge` |
| Windows 10+ x64 / x86 / arm64 | `figcraft-<v>-windows-<arch>.msi`, `figcraft-<v>-windows-<arch>-portable.zip` | `windows-latest` (arm64 cross-compiled) |
| Linux x86_64 / aarch64 | `figcraft-<v>-linux-<arch>.{AppImage,AppImage.zsync,deb,rpm,tar.gz,flatpak}` | `ubuntu-22.04` / `ubuntu-22.04-arm` (Flatpak: 24.04) |
| Linux riscv64 | `figcraft-<v>-linux-riscv64.tar.gz` (CLI smoke-tested under QEMU; needs glibc ≥ 2.39) | cross-compiled on `ubuntu-24.04` against ports.ubuntu.com riscv64 libraries |
| FreeBSD 14 x86_64 | `figcraft-<v>-freebsd-x86_64.tar.gz` | FreeBSD 14.3 VM on `ubuntu-latest` |
| Web | `figcraft-web-<v>.zip` (static site; [`packaging/web/README.md`](../packaging/web/README.md)) | `ubuntu-latest` |
| Server, Linux x86_64 / aarch64 | `figcraft-server-<v>-linux-<arch>.tar.gz` (the collaboration server; [`server.md`](server.md)) | `ubuntu-22.04` / `ubuntu-22.04-arm` |

`figcraft-cli` ships in every desktop package (and its own macOS zip) when `apps/figcraft-cli`
is in the workspace: `packaging/env.sh` sets `BINS`/`HAS_CLI` from that, and `package.ps1`,
`figcraft.wxs` (`HasCli`), `nfpm.yaml` (`#cli` lines) and the Flatpak bundle follow it.

`figcraft --version` prints the version and, in CI builds, the commit (`FIGCRAFT_BUILD_SHA`,
read by `apps/figcraft/build.rs`).

**The collaboration server** (`apps/figcraft-server`, binary `figcraft-server`, see
[`server.md`](server.md)) is built by the `server` job with `packaging/server/package.sh`: a
headless binary, so not part of `BINS` (which every desktop package installs) and no installer,
desktop file or fonts. Its archive holds the binary, the licences, `server.md` and the deployment
examples from `packaging/server/` (systemd unit, Caddyfile, nginx; the Dockerfile builds from
the sources). Linux only for now; elsewhere operators build it with
`cargo build --release -p figcraft-server`.

**Fonts.** Every build job, the web one included (`web-craft-fonts`), checks out
[craft-fonts](https://github.com/storytold/craft-fonts) at `craft-fonts-ref` (in `release.yml`) and
builds with `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED=1`, so releases embed its fonts and fail
rather than ship without them. The browser has no system fonts, so a web build without craft-fonts
can't draw text.
`crates/text/build.rs` reads both `fonts/manifest.txt` and `fonts/latin-manifest.txt`, so the pin
must combine the Latin presentation catalog and the current core UI faces. Required web builds
validate the emitted Inter Regular/SemiBold and Noto Sans Georgian Regular faces after filtering;
a missing required face, style or asset fails the required web build. Excluded Adobe rows
are skipped before resolving their asset paths.
Native builds retain the permitted Latin presentation faces. Source Sans 3 and Source Serif 4
are filtered on every target under FigCraft's asset policy. The web build keeps its smaller
allowlist: Inter, Noto Sans Georgian and Archivo SemiBold when supplied. Packages carry each
embedded font's licence as `OFL-<family>.txt`
(`copy_font_licences` in `packaging/env.sh`; `Contents/Resources/Licenses` in the macOS app).

### macOS

`packaging/macos/package.sh [--arch universal|aarch64|x86_64]` builds both Apple targets with
`MACOSX_DEPLOYMENT_TARGET=11.0`, joins them with `lipo` and assembles `FigCraft.app`
(`Contents/MacOS/FigCraft`, `Contents/Resources/FigCraft.icns`, `Info.plist` from
`Info.plist.in`). The `.icns` is generated at packaging time with `iconutil` from the committed
`assets/app-icon/figcraft-<size>.png` (made by `cargo xtask icon`); it is never committed.
Signing goes inside-out with the hardened runtime and a timestamp (empty entitlements), then the
app is notarized and stapled, put on a DMG with an `Applications` link (`hdiutil makehybrid` +
`convert`), and the DMG is signed, notarized and stapled. The CLI is signed with the same
identity and notarized as a zip. `packaging/macos/verify.sh` checks the artifacts as shipped
(strict with secrets, warnings without).

Locally (no certificates: ad-hoc signature, no notarization):

```sh
packaging/macos/package.sh --arch aarch64     # quick, host-only; omit --arch for universal
packaging/macos/verify.sh --arch aarch64
hdiutil attach -nobrowse -readonly dist/release/figcraft-*-macos-aarch64.dmg   # inspect, then detach
```

### Windows

`packaging/windows/package.ps1 -Arch x64|x86|arm64` builds with `+crt-static` (no VC++
redistributable), checks each exe's PE machine and subsystem (app GUI, CLI console), signs the
exes, builds the MSI with WiX v5 (`figcraft.wxs`: per-machine, Start Menu shortcut, owner of
`.figcraft`, "Open with" for SVG/PNG/JPEG, App Paths, setup wizard with art drawn from the app
icon), validates ICE50, signs the MSI and zips a portable build. `sign.ps1` uses a `.pfx`
(`WINDOWS_CERTIFICATE*`) or Azure Trusted Signing (`AZURE_*`), else warns and leaves files
unsigned. `apps/figcraft/build.rs` embeds the icon and version info with `winresource`.
`.github/workflows/windows-arm64.yml` installs the arm64 MSI on real ARM64 hardware nightly.

Locally on Windows: `dotnet tool install -g wix --version 5.0.2`,
`wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`, then
`pwsh packaging/windows/package.ps1 -Arch x64`.

### Linux

`packaging/linux/package.sh [--formats "appimage deb rpm tar"]` stages one FHS tree (binaries,
`ai.storyteller.figcraft.desktop`, hicolor icons 16–512 px, AppStream metainfo, a
shared-mime-info file for `.figcraft`) and builds every format from it: `.tar.gz`; `.deb` and
`.rpm` with [nfpm](https://nfpm.goreleaser.com/) (`nfpm.yaml` lists the dlopen'd runtime
libraries as dependencies); and an `.AppImage` with update information and a `.zsync`, whose
`AppRun` registers the desktop entry and icons on launch so Wayland docks show the right icon
(`FIGCRAFT_NO_DESKTOP_INTEGRATION=1` opts out; covered by `apprun-test.sh`) and runs the bundled
`figcraft-cli` for `FigCraft.AppImage cli <args>`.
`flatpak-bundle.sh` repackages the tarball into a `.flatpak` with
`flatpak/ai.storyteller.figcraft.bundle.yml` (freedesktop 26.08). `flatpak/ai.storyteller.figcraft.yml`
is the from-source manifest for a Flathub submission (not built in CI; vendor crates with
`flatpak-cargo-generator.py` first). Both manifests keep the same runtime and `finish-args`
(packaging-lint checks). Binaries are built on Ubuntu 22.04 and need glibc ≥ 2.35.
The `linux-riscv64` job cross-compiles with `CROSS_ARCH`/`CROSS_TARGET`/`CROSS_COMPILE` and
`EMULATOR=qemu-riscv64` and packages only the tarball (`--formats tar`).

### FreeBSD

The `freebsd` job runs `packaging/freebsd/package.sh` in a FreeBSD 14.3 VM
(`vmactions/freebsd-vm`, pinned by commit) and writes a `/usr/local`-style tarball.
`packaging/freebsd/package.sh --dry-run` checks the layout on any OS with stub binaries
(packaging-lint runs it). `.github/workflows/freebsd.yml` builds, tests and packages nightly.

```sh
tar -xzf figcraft-<v>-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
```

### Web

`packaging/web/package.sh` runs `trunk build --release` in `apps/figcraft-web` (`Trunk.toml`:
`public_url = "./"`, output `dist/web`), fails if a `.wasm` exceeds 24 MiB, and zips the site
with sample `_headers`/`.htaccess` and the hosting guide. The app uses WebGPU with a WebGL2
fallback; `?webgl` forces WebGL2. 0.1.0: wasm 17.7 MiB raw, 6.7 MiB gzip, 4.4 MiB Brotli.
`apps/figcraft-web/.cargo/config.toml` builds it with wasm SIMD (`+simd128`), which keeps the
glyph rasterizer's scalar fallback out of the module.

The wasm is built with the size-optimised `web-release` Cargo profile (`opt-level = "s"`, fat
LTO, one codegen unit; `data-cargo-profile` in `apps/figcraft-web/index.html`) and then
`wasm-opt -Oz`, which needs the wasm features rustc emits by default passed explicitly
(`data-wasm-opt-params`). Sizes: 0.2.0 21.1 MiB and 0.3.0 25.8 MiB (failed the gate; the plugin
interpreter and collaboration code landed) with the desktop `release` profile; 0.3.1 17.2 MiB
with `web-release`. The desktop builds keep `release`.

## Secrets

All signing secrets are **`release`-environment secrets** (deployment branches: `release` only),
never repo or org secrets. Names (identical in every app): `APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`,
`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`, `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
`AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`.
Each is optional: a missing one gives an unsigned artifact and a `::warning::`. An org admin
provisions the environment, ruleset and secrets, and
adds the repo to the org's Default runner group so `macos-15-xlarge` can pick up the job.

## Checks

- `.github/workflows/ci.yml` (PRs and `main`, Linux only): fmt, clippy `-D warnings`, tests,
  layering, asset attribution, wasm32 check. No macOS in CI (playbook).
- `.github/workflows/packaging-lint.yml` (changes to `packaging/` or workflows): actionlint,
  shellcheck, the AppRun test, a PowerShell parse, the MSI icon check, the FreeBSD dry run, a
  grep for other products' names, xmllint, `desktop-file-validate`, `appstreamcli validate` and
  the Flatpak manifest agreement check.
