# FigCraft egui 0.37 migration plan

Status update (2026-10-25): The migration integrates main `92682728` and pins egui 0.37, winit and craft-window. Required-font CI run `egui037-fig-review-full-ci-20261025-0803` passed 3,306 test executions, zero failures and nine ignored across 71 summaries, including WASM and all three plugin bundles. Screenshot encoding now precedes a shared final-write/reply gate: timeout cancellation cannot overtake a committed file, and a fresh deadline check rejects a late write. A regression through the actual arrival path failed with the previous write behavior and passed with the gate. Native visible GUI/browser workflows, native IME, paired app performance and current-source platform/release installation remain open. Human review and human merge are required.

The original planning record below is historical.

Status: implemented locally, with host CI qualification recorded in the current qualification
section below and in `docs/egui-037-local-followups.md`. Earlier sections retain the original
planning baselines. Native/browser workflows and paired app performance remain open.
No publication approval exists for this document or any derived branch/PR.

Scope: the exact checkout
`/home/codex/work/CraftApps/egui037-migration-20261022/figcraft-implementation`. This plan covers
FigCraft only. Suite-wide fork work, shared `craft-libs` work and publication approval remain with
the parent workflow. This revision resolves the adversarial review of the previous draft in place;
the review file is
`/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/plan-reviews/figcraft-v1.md`.

Capture acceptance follows the suite contract: a queued-not-issued hidden request and
each settling frame must request `RequestPaintWhileHidden` from logic; only an issued
screenshot command forces paint itself. Callback arrival has no implicit wake, so the
live request owns the viewport waker. Record known issuance UI bounds and a geometry
epoch, reject/retry any intervening resize/DPI/zoom/surface change even if it changes
back, and derive pixel scale from returned dimensions only under an unchanged epoch.
Default limits are 4 requests, 128 MiB/image and 256 MiB aggregate. Count reserved
in-flight and completed images together.


## 1. Source identity and what was inspected

### 1.1 Checkout and baseline

- FigCraft integration checkout: `8167f13a6a416ca61fcb505a7eaf6be383f6c5c0`. Earlier baselines
  (`80f916326fb94f3a88cb24e64f396f564a0ebc06`, `8d790607ff1ad40e32343ca2116d8fdcf3244c8d`) are
  historical review anchors only.
- The worktree contains the migration implementation; this document is not a claim that those
  changes are compiled, tested or publishable.
- Repository instructions: `AGENTS.md` and `CLAUDE.md` are identical; there are no nested
  `AGENTS.md`/`CLAUDE.md` files under `crates/`, `apps/`, `plugins/` or `xtask/`.
- Current lock: `egui`, `eframe`, `egui-wgpu`, `egui-winit`, `egui_extras`, `ecolor`, `emath`,
  `epaint` all `0.36.2`; `winit` `0.30.13`; `wgpu` `30.0.1`; `accesskit` `0.24.1`;
  `wasm-bindgen` `0.2.129`.
- Git dependencies: `craft-docpath` rev `82a24b006eefa036b43c09a67fb9a3fbd17ffe63`;
  `craft-window` rev `1f805378d454dac48b5c214029d254055fdf961b` (that revision's workspace
  declares egui 0.36 and cannot be used unchanged with 0.37).
- Build input: tracked `.cargo/config.toml` sets `CRAFT_FONTS_DIR=../craft-fonts` (relative,
  optional). It is locally skip-worktree, so it did not appear in `git status`; it is not a
  path patch and does not change dependency resolution. CI intentionally builds without craft-fonts.
- There are no `vendor/` directories, source patches or `[patch]` sections in this repository.
  `plugins/` is a separate workspace and does not use egui.

### 1.2 Target revisions and publication boundary

The production pins are **not** the candidate fork SHAs. They are the final reviewed
`storytold/egui`, `storytold/winit` and `storytold/craft-libs` immutable revisions produced by the
suite's reviewed patch series, published with fresh human approval. Those SHAs do not exist yet at
this planning revision. Do not publish or implement a manifest that names a candidate or a branch.

| Item | Exact value / status |
|---|---|
| Upstream egui 0.37.0 release commit | `841a328ba2dd8f6996beb7189ebb7fc00e668a2a` (annotated tag object differs) |
| Upstream egui 0.36.2 control | `49682f8baa058bf49e011035cfbd6e825f88a5ef` |
| Published storytold egui 0.36.2 | `767b1cad294120689a514bb4bd68c077e46679b9` |
| Published storytold winit 0.30.13 | `ffae5198770d7730e718043f035a772b7dbc3b1b` |
| aiongg egui candidate | `aiongg/egui` `craft/0.37` `a4f9a17f64a6bae61350fb85804c9df0d9865916` — comparison input only |
| aiongg winit candidate | `aiongg/winit` `craft/0.30.13-lang` `8b35388bd719e8cc86a95971ba0bc026c9a8f058` — comparison input only |
| Final storytold egui | `STORYTOLD_EGUI_FINAL_REV` — unknown until the reviewed E1–E21 + E21b/P7b series is approved and published |
| Final storytold winit | `STORYTOLD_WINIT_FINAL_REV` — unknown until W1–W27 + W15P are approved and published |
| craft-libs merged baseline | `3832c47ef68a0e6214f2c1e85f3526b6adcb686e` (PR #199 merged: “craft-ui: update egui to 0.37”) |
| craft-libs PR #199 head / audit merge | `b68de8c5f4f1c060904d211a7b1320aa0f7bce78` / `ec8840af8c5fda695cf195cdd05e5ae078415c07` — **unsafe stale history**; do not merge or reuse either tree |
| Local craft-libs integration | `3832c47` plus the reviewed capture-completion and text-alignment helpers in the local `craft-libs-implementation` worktree; no published commit yet |
| Final craft-libs | `STORYTOLD_CRAFT_LIBS_FINAL_REV` — unknown until the reviewed helper stage is approved and published atop `3832c47` |

Verified source facts for the craft-libs anchor: the historical port source `8614e4a` contains no
`crates/window/Cargo.toml` and no `crates/platform`; `b68de8c` is a stale tree that omits current
`craft-platform` and reverts security, settings, docking and printer fixes. The merged
`3832c47` tree contains `crates/window`, `crates/platform` and the 0.37 workspace. The local
implementation must stay on that merged base and must not reuse either PR head as its tree.

Language work: `storytold/craft-libs#198` is an issue/proposal, not a PR or a merged dependency.
FigCraft consumes the final fork revisions; it does not copy language helpers from `craft-ui`.

### 1.3 Evidence consulted (historical, not fresh validation)

- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/lifecycle-source-audit.md` is the
  authoritative lifecycle evidence for this plan.
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/ime-surrounding-gap.md` is the
  authoritative E21b/P7b gap inventory.
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/upstream-discussions.json`:
  issue/PR snapshots for #8535, #8489, #8555, #8583, #8512, #8657, #8577, #8731, #8736, #8763,
  #8764 and #8748.
- `/home/codex/work/ssd2/task-evidence/egui037-audit-20261022/{fork,stock}/figcraft.errors`:
  historical compiler diagnostics, used only as clues.
- `/home/codex/work/ssd2/task-evidence/craft-fork-egui037-20261021/RESULTS.md`,
  `craft-fork-ai-lines-20261022/SURVEY.md`, `craft-fork-plan-20261020/PLAN.md`.

No build, test, benchmark, browser run or GUI check was performed for this plan.

## 2. Decision summary

1. Target the final reviewed storytold egui/winit revisions, not stock 0.37 alone and not the
   aiongg candidates. FigCraft needs the language/bidi/IME patches, W15P PRIMARY, E21b outgoing
   surrounding text and the egui-wgpu fixes carried by the reviewed series.
2. Keep FigCraft's existing `App::logic` + `App::ui(&mut Ui)` architecture. Do not rewrite to
   `App::update`; it does not exist. Do not call `Context::root_ui` from `App::ui`; the root `Ui`
   is already borrowed (upstream #8535).
3. Treat the real eframe lifecycle: native and web both call `App::logic` **inside** the
   `Context::run_ui` closure, once per layout pass. `run_logic` is not distinguishable through
   `ViewportInfo::visible()`. Design logic work to be safe and idempotent in both callback paths,
   and use a narrow first-pass guard only for genuinely once-per-layout-frame UI setup.
4. Keep command/undo/persistence contracts. UI input is consumed through an explicit one-shot
   action queue/token; do not gate all commands on `current_pass_index() == 0`, which cannot be
   implemented in `FigApp::run`/`set_props` (neither receives a `Context`) and would starve fresh
   work.
5. Use a small local completion-slot adapter in FigCraft, matching the shared capture contract,
   with a weak callback reference, per-request cancellation, bounded aggregate image bytes,
   capture-time viewport/geometry/DPI and viewport-aware wakeup. Keep request IDs, paths, replies and
   timeouts in FigCraft.
6. Correct the actual a11y edge handling: FigCraft's panels are `resizable(false)`, so egui does not
   create `left_panel.__resize`/`right_panel.__resize`. FigCraft's custom edges are
   `Id::unique(("panel_edge", "left"/"right"))` with a focusable `Sense::click_and_drag()`.
   Make them non-focusable and test the real IDs. Preserve prefs.panel_widths; fixed
   panels and disabled eframe persistence require no PanelState migration.
7. Add E21b/P7b outgoing IME surrounding text as an atomic compatibility prerequisite: bounded
   transient payload, built-in TextEdit producer, Wayland adapter, and a real model/preedit/UTF-8
   index adapter for FigCraft's custom editor. Never pass `None` placeholders as the completed port.
8. Keep logical text at shaping, clipboard, accessibility, save and IME boundaries. Do not feed
   display-bidi (visually reordered) strings into egui shaping, clipboard or IME. FigCraft does not
   use `craft-ui`'s `DockArea`; if it later adopts it, use the explicit opt-in direction policy with
   unchanged saved physical splits and logical tab order.
9. Use pinned licensed font fixtures from the egui plan's section 10.6. Keep FigCraft's central
   craft-fonts policy, do not copy font binaries into the repository, and do not claim color-emoji
   coverage while `epaint/color_fonts` is disabled.
10. Stage reviewable PRs with a dependency DAG and rollback. Every build/test uses
    `maintenance/task.py run/status/finish`; no direct cargo test/build is acceptance.

## 3. Baseline audit

### 3.1 Manifests, entry points and feature targets

| Target | Manifest / source | Current features / role | 0.37 action |
|---|---|---|---|
| Desktop app | `apps/figcraft/Cargo.toml`, `apps/figcraft/src/main.rs` | eframe `default_fonts`, `wgpu`, `accesskit`, `wayland`, `x11`; wgpu renderer; Windows `windows 0.62`; macOS `fmv-macos-events`; Linux `winit` `wayland-csd-adwaita-notitle` | Keep feature shape; add final eframe/egui revisions; enable `clipboard` explicitly; keep `system_fonts` off initially; repoint `craft-window` to the final craft-libs revision |
| Browser app | `apps/figcraft-web/Cargo.toml`, `src/main.rs`, `src/web.rs` | wasm32 eframe `default_fonts`, `wgpu`, `accesskit`; `WebRunner`; WebGPU/WebGL2 | Update eframe/egui; keep `web_screen_reader` disabled as at baseline; `web_file` is an egui feature used by eframe for wasm, not a FigCraft feature to “verify” independently |
| UI crate | `crates/ui-egui/Cargo.toml`, `src/lib.rs` | egui/egui_extras 0.36, no eframe; about 1,478 egui references; no kittest dev-dependency | Update egui/egui_extras; add `default_fonts` and `egui_kittest` 0.37 for strict a11y tests; add the local capture adapter; mechanical API migration |
| Text/render | `crates/text`, `crates/render` | no egui dependency; own harfrust/skrifa/tiny-skia | No code change expected; verify duplicate harfrust/skrifa versions and UI/document metrics |
| CLI/server/MCP | `apps/figcraft-cli`, `apps/figcraft-server`, `crates/mcp` | no egui/eframe (MCP headless) | No dependency change; keep commands and outputs |
| Plugins | `plugins/` separate workspace | no egui | No change |

Entry points:

- Native: `apps/figcraft/src/main.rs` `main()` → `eframe::run_native`; `App::logic`,
  `App::raw_input_hook`, `App::ui`, `App::on_exit`.
- Child windows: `crates/ui-egui/src/windows.rs` `Windows`, `viewport`, `show`, `draw`,
  `enter`/`leave`, `as_window`.
- Browser: `apps/figcraft-web/src/web.rs` `start`, `WebShell::ui`, `WebServices`.
- Headless/UI tests: `Context::default().run_ui(...)` in many `crates/ui-egui/src/*_tests.rs`
  files; `crates/ui-egui/src/idle_tests.rs` has the frame/idle harness; `windows_tests.rs` has the
  `run_logic` hidden-window harness.
- Render/control: `crates/ui-egui/src/canvas/render_worker.rs`; `crates/ui-egui/src/control.rs`.

Build/test entry points:

- `.github/workflows/ci.yml`: Linux fmt, clippy `--workspace --all-targets --locked -D warnings`,
  workspace tests, `xtask layers`, `xtask assets`, `xtask wasm`.
- `xtask/src/main.rs`: `ci`, `layers`, `assets`, `wasm`, `bench`, `plugins`.
- `crates/ui-egui/src/perf_tests.rs`: ignored `perf_ui_frames` and `report_components` benchmarks,
  currently printing arithmetic means only.
- `apps/figcraft-cli/src/perf.rs`: release engine/render timings, currently a mean of several runs.
- `apps/figcraft-web`: `trunk build --release`; `packaging/web/package.sh` enforces the 24 MiB
  wasm cap.
- Windows ARM workflow only packages/installs; Windows x64 build/test and visible GUI checks must be
  run separately through the VM workflow. FreeBSD is a nightly VM build/package job; macOS is
  release-only and has no routine UI test job.

### 3.2 Current lifecycle: what is already true

On 0.36.2 the app already uses `eframe::App::logic` and `eframe::App::ui`:

- `apps/figcraft/src/main.rs` `App::logic` drains the control queue, hidden requests, Apple events,
  close requests, title updates and recovery.
- `App::ui` wraps `FigApp::ui` in `windows::guard`.
- `crates/ui-egui/src/lib.rs` `FigApp::ui` calls `self.logic(&ctx)`, `autosave_tick`,
  `auto_save_tick`, then `windows::begin`, `frame` (which also calls `control::drain`), `settle`,
  `show`.
- `crates/ui-egui/src/windows.rs` `draw` swaps a child window's pane into the app, calls
  `app.logic(&ctx)`, then frames it under `windows::guard`.
- `FigApp::logic` installs language/fonts/theme, computes UI zoom from `ctx.content_rect()`,
  creates the render worker and scans input for CJK fallback.
- Commands go through `FigApp::run`/`run_user`/`run_background`; the engine owns history snapshots.

The authoritative source sequence is not “logic once per native frame”:

- Native `crates/eframe/src/native/epi_integration.rs::update` calls `app.logic` then `app.ui`
  inside `Context::run_ui`.
- Web `crates/eframe/src/web/app_runner.rs` does the same for visible tabs and calls
  `Context::run_logic` for hidden tabs; native `run.rs` can force a full `PassMode::Full` UI for
  invisible windows and a `LogicOnly` pass for overdue redraws.
- `Context::run_logic` refreshes only viewport/window focus information; input events, time, layout
  and normal pass state remain from the previous UI pass.
- After a normally completed `Context::run_ui`, `current_pass_index()` is zero, including after
  multiple passes (`ContextImpl::end_pass` takes the output; `run_dyn` restores the count only
  before the next UI pass). Do not write tests that expect a stale nonzero index after completion.
- `ViewportInfo::visible()` derives from minimized/occluded and can be `None`; it does not identify
  `run_logic` versus a forced hidden UI pass. Visibility is not the tick phase.

The migration must therefore make background work idempotent in both callback paths and move
input/geometry/theme setup to the UI phase. A narrow first-pass guard is valid for once-per-layout
UI setup; it must never suppress fresh control commands, widget actions, background completions or
deadline work merely because a pass index is nonzero.

### 3.3 Historical compiler diagnostics (clues only)

| Location | Diagnostic | Meaning for this plan |
|---|---|---|
| `crates/ui-egui/src/control.rs:84`, `idle_tests.rs:275` | `egui::Event::Screenshot` variant not found | 0.37 uses `ScreenshotCallback`; replace event polling |
| `crates/ui-egui/src/control.rs:119`, `idle_tests.rs:275` | `egui::UserData` not found | Removed with the callback API |
| `crates/ui-egui/src/control.rs:367` + 11 test files | `Event::MouseWheel` missing field `source` | Add `MouseWheelSource` |
| `crates/ui-egui/src/dialogs.rs:108` | `Role::SelectableLabel` not found | 0.37 keeps deprecated `pub type WidgetType = Role`; only renamed variants such as `SelectableLabel` are gone. Map to `Role::Button` |
| `crates/ui-egui/src/canvas/frame_labels.rs:63` | non-exhaustive match missing `Shape::Band` | Add the 0.37 shape variant |
| `crates/ui-egui/src/panels/typography.rs:1312` | `ScrollArea` state `offset` private | Use `clamped_offset()` |
| Many files | `Id::new`/`Ui::id()` deprecation warnings | CI clippy `-D warnings` turns these into failures; replace mechanically |

`clip_rect_margin` is not removed in 0.37; it remains as a deprecated unused field. Remove its use
because it has no effect, not because it fails to compile. The audit stopped at these errors and did
not compile the app, tests, Windows, wasm or examples. A fresh managed build after the final pin is
required; this table is not acceptance.

## 4. Dependency and patch plan

### 4.1 What must be pinned

Add a root `[patch.crates-io]` block in `Cargo.toml` (consumer root; dependency patch sections do
not propagate). The production block uses the final reviewed storytold revisions. The aiongg SHAs
below are recorded only as candidate/comparison inputs and must never appear in the production
manifest.

```toml
[patch.crates-io]
egui = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
eframe = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
egui-wgpu = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
egui-winit = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
egui_extras = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
ecolor = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
epaint = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
emath = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
epaint_default_fonts = { git = "https://github.com/storytold/egui", rev = "STORYTOLD_EGUI_FINAL_REV" }
winit = { git = "https://github.com/storytold/winit", rev = "STORYTOLD_WINIT_FINAL_REV" }
```

Only patch crates FigCraft actually resolves. `ecolor`, `emath` and `epaint_default_fonts` are pulled
by egui/default fonts; leaving any of them on crates.io creates mixed egui-family versions. Add
`egui_glow` only if a glow build is introduced. If a crate is not resolved in the final graph,
remove its patch row and record the `cargo tree` evidence.

`egui_extras` must be patched because `crates/ui-egui/src/dialogs/credits.rs` uses
`TableBuilder`/`TableRow`/`TableBody`; a 0.36 `egui_extras` would pull a second egui 0.36.

`craft-window` is a git dependency, not a path patch. Pin it to the final craft-libs revision whose
workspace requires egui 0.37. Do not keep `1f805378` (egui 0.36) with a 0.37 egui patch: Cargo
cannot apply a 0.37 patch to a `^0.36` requirement. `craft-docpath` is independent of egui; prefer
the same final craft-libs revision for both crates after confirming its source hash, or keep its
existing rev if the final commit changes unrelated behavior this task has not reviewed.

FigCraft currently has no `craft-ui` dependency. Keep its capture adapter local in
`crates/ui-egui/src/control.rs`, following the CL-2/CL-2F ownership, cancellation, byte-limit
and viewport-wakeup contract. Eleven existing creative consumers adopt the shared helper;
DeckCraft, FigCraft and ArtCraft Native use local adapters. Reuse the reviewed conformance cases
and compare behavior with the shared prototype; do not add the entire crate solely for capture.

### 4.2 Feature decisions

| Manifest | Decision | Reason / test |
|---|---|---|
| Root `[workspace.dependencies]` | `egui = "0.37"`, `egui_extras = "0.37"`, `eframe = "0.37"` | Pin all three and verify in the lock |
| `crates/ui-egui/Cargo.toml` | `egui = { workspace = true, features = ["default_fonts"] }` (or the workspace dep carries it) | 0.37 egui default features are empty; `theme::install_fonts` calls `FontDefinitions::default()` and tests without eframe need real fallback fonts |
| `apps/figcraft/Cargo.toml` | Keep `default_fonts`; enable `clipboard` explicitly; keep `wgpu`, `accesskit`, `wayland`, `x11` | eframe 0.37 `default_fonts` forwards to egui; `clipboard` is optional in 0.37 |
| `apps/figcraft-web/Cargo.toml` | Keep `default_fonts`, `wgpu`, `accesskit`; keep `web_screen_reader` disabled | Browser has no system-font provider; keep the 24 MiB wasm budget |
| eframe `system_fonts` | Off initially | FigCraft already scans system fonts and installs only needed faces; enabling fontique adds a second scan and nondeterminism. Evaluate later with a separate measured PR |
| `clipboard` | Explicitly enable native `eframe/clipboard` in F2 to preserve 0.36 OS text clipboard behavior; verify E4/W15P and image alpha | 0.36 forced the integration feature; 0.37 makes it optional. Keep existing arboard image handling and browser clipboard paths separately tested |
| `epaint/color_fonts` | Off by default | 0.37 color-glyph support is behind this feature. Do not claim color-emoji coverage in the normal build; if browser/native color emoji becomes an acceptance requirement, enable it in a separate feature job with the pinned Noto Color Emoji fixture and a color-glyph test |
| `egui_extras` | `default-features = false`; no `svg` feature unless used | Current crate only uses tables |

### 4.3 Lockfile and duplicate checks

Every command below is a payload for the managed workflow in section 7.1; do not run it directly:

```sh
cargo tree --locked -p figcraft -i egui
cargo tree --locked -p figcraft -i eframe
cargo tree --locked -p figcraft -i winit
cargo tree --locked -p figcraft -d
cargo metadata --locked --format-version 1
```

Acceptance:

- Exactly one source/version for `egui`, `epaint`, `emath`, `ecolor`, `eframe`, `egui-wgpu`,
  `egui-winit`, `egui_extras` and `winit` (and `egui_glow` if a glow build is enabled).
- `Cargo.lock` records the final storytold egui/winit/craft-libs revisions; no aiongg candidate SHA
  and no moving branch remains.
- No registry 0.36 egui-family package remains; no duplicate `craft-window`.
- `wgpu` resolves to one version. Current FigCraft lock is `30.0.1` while the candidate lock says
  `30.0.0`; both satisfy `30.0`, but the final lock must be inspected for wasm-bindgen and backend
  compatibility. Do not “fix” this by allowing two wgpu versions.
- `cargo check --locked` native and wasm; `cargo test --locked --workspace`; clippy with
  `-D warnings`.
- All patched git sources are public and reachable without credentials. No machine-local path,
  branch-only or private-repository patch is publishable.

## 5. Fork patch inventory and integration decisions

The final fork's `PATCHES.md` is the provenance record. Every row below is kept in the reviewed
series unless its decision says otherwise; none is deleted merely because FigCraft does not call the
API directly. The candidate revision is a comparison input, not the production source.

| Patch | Crates | Provenance / upstream | FigCraft impact | Decision / removal condition |
|---|---|---|---|---|
| E1 `8f3abf06` | eframe | suite ViewportHooks; not upstream | Compiles `NativeOptions`; no direct use | Keep for suite; remove when upstream has equivalent |
| E2+E3 `9dd799ab` | egui-wgpu | surface fit + error scope; #8361 related | Large/multi-monitor windows, GPU setup errors | Keep; test large surface/error path on Windows/Linux real GPU |
| E4 `66bacf23` | egui-winit | Wayland clipboard via winit (#8657 draft) | Text copy/paste on Hyprland | Keep the regular clipboard path; PRIMARY is **not** accepted through the candidate's arboard fallback |
| **W15P + E4 P2b** | winit + egui-winit | winit-owned PRIMARY on the existing connection/event queue, per-seat `PrimarySelectionManagerState`; no second regular `wl_data_device` | Pointer- and keyboard-selected PRIMARY on Wayland | Required prerequisite and integration stage; native acceptance on wlroots/KWin/Mutter. arboard only as a measured fallback if W15P fails a proven requirement |
| E5 `0e27d87e`, `777977b7`, `f7e89e23`, `5b2b8e19` | epaint/egui | bidi #8577 + fixes | RTL UI/document text, marks, caret | Keep; RTL/caret tests; remove FigCraft double-processing where egui now shapes logical text |
| E6 `0abfabcf` | egui | IME focus | Avoids composition interruption | Keep; IME test |
| E7 `0670c58a` | egui-winit | Windows letter keys | Non-Latin layouts | Keep; Windows layout test |
| E8 `1b03c10f` | egui | TextEdit glyph overshoot | Thai/Vietnamese/Arabic marks | Keep; screenshot/geometry |
| E9 `b2f5cafc`, `e1341a3d`, `5105330d`, `75a5b3d3` | egui | RTL chrome | Menus/scrollbars/headers/title bars | Keep; RTL screenshots |
| E10 `dc90c13e`, `c8e573f7`, `34b2c0d4` | eframe web | key/text-agent fixes | Browser shortcuts and IME | Keep; browser test |
| E11 `92e61afd` | epaint | single-thread vello flush | Avoids panic under `multithreading` | Keep; compile/test; FigCraft does not enable vello multithreading |
| E12 `e955bb18`, `485c0b89`, `0185887c`, `7015ac23`, `4067b2c4`, `620e070e` | epaint/egui | LayoutJob direction, RTL alignment, L1, linear reorder | RTL layout and text painting | Keep; RTL tests |
| E13 `5e4da7f4` | epaint/egui | grapheme caret/hit/delete | Emoji/accented text editing | Keep; editor tests |
| E14 `cbd77549`, `b3722ac0` | epaint/egui | fallback baseline align | CJK/Indic fallback rows | Keep; CJK/Indic screenshot |
| E15 `f1cae609` | epaint/egui | LineBreaker trait/language | Thai/Japanese breaking if enabled | Keep; default SimpleLineBreaker preserves 0.36 |
| E16 `b3ef9771`, `bed6dfe3` | epaint/egui | visual cursor/affinity | RTL cursor/Home/End | Keep; RTL caret tests |
| E17 `520435df` | egui | UI language, AccessKit language | Screen-reader language; bidi runs | Keep; use `Context::set_language`; AccessKit test |
| E18 `89de8f3c` | eframe | `Frame::set_ui_language` | Optional language sync | Keep; can use Context directly |
| E19 `d64047bf`, `642b74f7`, `baeb4515` | egui-winit/egui | Latin layout chars | Shortcuts on non-Latin layouts | Keep; Windows/Linux layout test |
| E20 `23955535` | egui | widget text language | RTL/localized widget text | Keep; RTL widget screenshot |
| E21 `a70470b0` | egui-winit/egui | IME `DeleteSurroundingBytes` | macOS/Wayland IME | Keep; IME test |
| **E21b/P7b** | egui, egui-winit, winit W27 | outgoing IME surrounding text; confirmed missing at candidate `a4f9a17f` | Custom FigCraft text editor and Wayland predictive IME | Required new prerequisite; see 6.5. Not existing E21 capability |

Removal rule: a row may be removed only after an upstream release in use contains the same fix and
the app tests pass against that release. Do not remove an unrelated fork fix because FigCraft does not
exercise it directly; the fork is shared across the suite.

Optional new provenance patches, only if measurements require them:

- **E22 text tessellation (#8731):** upstream reports ~61% more instructions per plain-text
  `tessellate` in 0.37. If FigCraft's three-way text-heavy benchmark exceeds the threshold, carry
  the issue's row-level fast path with author/provenance recorded, plus a pixel/vertex/index
  equality test for mixed and color rows. Do not change color-glyph semantics.
- **E23 DX12 shader compile (#8736):** upstream reports ~40 ms extra pipeline creation on FXC. If
  Windows startup exceeds the threshold, carry a float/`select` implementation (not clamp-only)
  that preserves clamp/repeat/mirrored-repeat and color-glyph filtering, with startup and image
  tests. A pipeline-overridable constant is acceptable only if it compiles on all backends.
- **E24 TextEdit click-away (#8763 / PR #8764):** #8763 is the issue; #8764 is an unmerged proposed
  fix, not released behavior. If FigCraft's fields or text-selection actions reproduce the 0.37
  selection collapse, carry the reviewed fix or add the app-level regression test. A necessary app
  workaround must preserve the range and undo semantics and have its own removal condition.
- **E25 hidden deferred windows (#8748):** #8748 is closed and unmerged. Prefer the 0.37
  `RequestPaintWhileHidden` path in FigCraft's window reconciliation before adding a fork patch.
  If a child cannot be created, updated or closed while the root is hidden, record the failing test
  and then adopt a reviewed fork fix with provenance; do not treat #8748 as available behavior.

## 6. Concrete API and behavior migration

### 6.1 Mechanical compile/deprecation changes

| File / symbol | Current | 0.37 change | Reason / test |
|---|---|---|---|
| throughout `crates/ui-egui`, `apps` | `egui::Id::new(x)` (443 uses) | `egui::Id::unique(x)` | 0.37 deprecates `Id::new`; same fixed-seed hash. `cargo clippy -D warnings` fails otherwise |
| 63 uses, e.g. `tabs.rs`, `layers.rs`, `right.rs`, `widgets.rs` | `ui.id()` | `ui.scope_id()` | Renamed; same scope ID |
| any `UiBuilder::new().id(...)` (none found now) | `.id(id)` | `.scope_id(id)` | Deprecated; verify by grep after codemod |
| `crates/ui-egui/src/dialogs.rs` `tab_strip` | `WidgetType::SelectableLabel` | `egui::Role::Button` | `WidgetType` survives as deprecated `pub type WidgetType = Role`; only `SelectableLabel` is gone |
| `crates/ui-egui/src/dialogs/credits.rs` `sort_header` | `WidgetType::Button`/`Label` | `egui::Role::Button`/`Role::Label` | Same mapping; aliases may compile but should not produce warnings |
| `crates/ui-egui/src/control.rs` `ui.input` scroll branch; 11 test files | `Event::MouseWheel { unit, delta, phase, modifiers }` | add `source: egui::MouseWheelSource` | Use `Wheel` for synthetic wheel input, `Unknown` where unspecified |
| `crates/ui-egui/src/canvas/frame_labels.rs` `turn` | exhaustive `Shape` match | add `Shape::Band(_)` | 0.37 `BandShape`; label drawing never adds one, leave it unchanged with a comment or rotate via its `angle` if a Band ever appears |
| `crates/ui-egui/src/panels/typography.rs` `font_picker` | `shown.state.offset.y` | `shown.state.clamped_offset().y` | `State::offset` private/deprecated; `clamped_offset` is the drawn position |
| `crates/ui-egui/src/control.rs` `PendingShot`/`drain` | polls `Event::Screenshot` | capture-slot callback | API removed; see 6.3 |
| tests in `idle_tests.rs` | synthetic `Event::Screenshot` | complete a test capture slot | API removed |

Codemod safety:

- `Id::unique` and `Id::new` use the same hash; no persisted state changes. `Ui::scope_id` and
  `Ui::id` are the same value.
- Do not replace tuple salts with formatted strings.
- After the codemod, grep for `Id::new`, `Ui::id(`, `UiBuilder::id(`, `SelectableLabel`,
  `Event::Screenshot`, `UserData`, `State.offset` and ensure only intentional comments remain.
- Run `cargo fmt` and `cargo clippy --workspace --all-targets --locked -- -D warnings`; a clean
  `cargo check` is not enough because deprecations are warnings.
- F2 adds `egui_kittest` 0.37 as a dev-dependency for the accessibility/focus smoke tests, using
  strict missing-name behavior and the default missing-glyph policy from its first commit. Only a
  deliberate arbitrary-script fixture may opt into `MissingGlyphPolicy::Tofu`; it must not mask a
  production glyph gap and is not a substitute for the pinned fixtures in 6.6. If kittest adoption
  is deferred, equivalent custom assertions must still fail on unnamed controls and missing glyphs;
  a green suite that silently draws tofu is not accepted.

### 6.2 Panel IDs, saved state and accessibility focus order

0.36 `Panel::left("left_panel")` used `Id::unique("left_panel")`. 0.37 `Panel::left` takes
`impl AsIdSalt` and computes `parent_ui.scope_id().with_salt(IdSalt::new("left_panel"))`
(`crates/egui/src/containers/panel.rs` at the final source). FigCraft's
`crates/ui-egui/src/a11y.rs::focusables` currently excludes the old panel resize IDs, but its
panels are `.resizable(false)` (`crates/ui-egui/src/lib.rs`) so egui creates no
`left_panel.__resize`/`right_panel.__resize` widgets at all. The real custom edges are:

```rust
// crates/ui-egui/src/lib.rs::panel_edge
let id = egui::Id::new(("panel_edge", name)); // "left" / "right"
let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
```

`Sense::click_and_drag()` includes `FOCUSABLE`, so these edges can enter the F6/Tab order today.
The fix is:

- Make the custom edge non-focusable with `egui::Sense::CLICK | egui::Sense::DRAG` (keeping hover,
  click and drag behavior), and also exclude the exact IDs
  `Id::unique(("panel_edge", "left"))` / `Id::unique(("panel_edge", "right"))` from
  `focusables` as a defense-in-depth assertion.
- Do not derive or test `Id::new("left_panel").with("__resize")` as the live edge; that is dead
  code while FigCraft's panels are non-resizable. If a future PR enables `Panel::resizable(true)`,
  derive the real resize ID from the recorded root `Ui::scope_id()`:
  `root_scope.with("left_panel").with("__resize")`, and test it.
- Record the root scope in `a11y::begin` from the passed root `Ui` (change the call from `&ctx` to
  the root `&Ui`/`ui.scope_id()`) so a future enablement is testable without guessing a global ID.

Panel widths come from `prefs.panel_widths` and `fit_panels`. Both panels are
`resizable(false).exact_size(...)`; eframe persistence is off. There is no durable egui
PanelState to migrate, and copying it cannot affect fixed geometry. Preserve actual preference
save/reopen and test root-scoped IDs plus geometry/edge hit tests; remove the invented migration.
AccessKit assertions use roles/names rather than raw IDs.

### 6.3 Screenshot callbacks, correlation and automation confinement

Current `crates/ui-egui/src/control.rs`:

- `PendingShot` stores `id`, `path`, `reply`, `since`, `viewport`.
- `drain` scans `ctx.input` for `Event::Screenshot` and completes every pending shot for that
  viewport in arrival order.
- `shot_reply` encodes PNG and writes via `figcraft_engine::file_access::write`.
- `idle_tests` manufactures `Event::Screenshot`; `windows_tests` only tests the hidden-window error.
- `apps/figcraft/src/control_server.rs` marks the request cancelled after the transport timeout, but
  `PendingShot` does not retain that flag and `drain` only checks `now - since < SHOT_TIMEOUT`.

0.37 replaces the event with a one-shot `ScreenshotCallback`; `UserData` and `Event::Screenshot`
are gone. The callback receives `Arc<ColorImage>` and does not request a repaint. Design:

1. Implement the suite capture contract locally in `crates/ui-egui/src/control.rs`, using the
   reviewed CL-2/CL-2F semantics and conformance cases. FigCraft, DeckCraft and ArtCraft Native
   have local adapters; eleven existing craft-ui consumers share its helper. The local slot owns one result per admitted request and
   exposes a weak handle to the callback; the callback holds only a `Weak` reference, never a strong
   `Context`/app/queue, so a late capture cannot retain the app through shutdown.
2. On request, FigCraft records its own `ShotRequest { id, path: Option<String>, reply, deadline, serial, viewport,
   document_generation, capture_spec }` and admits it only if:
   - the capture is for a live viewport and document generation;
   - the request count and aggregate pending bytes are within the app budget;
   - the `CaptureSpec` records capture-time logical viewport rectangle, pixels-per-point, zoom and
     expected surface size. Revalidate before issuance after resize/DPI changes.
3. `ui.screenshot` sends `ViewportCommand::Screenshot(ScreenshotCallback::new(...))` to the target
   viewport (`send_viewport_cmd_to` for a child; `request_screenshot` for the current one). The
   callback performs only `slot.try_complete(image)`; no PNG encoding, file I/O, command, lock held
   across wakeup, or mutable app access.
4. The completion slot stores the image and a `CancelToken` copied from
   `ControlRequest.cancelled`. It must release its lock before waking the owning viewport. Wake with
   `Context::request_repaint_of(capture_spec.viewport)`, never a bare `request_repaint()` from a
   detached callback; a stored `Context` clone is held only by the live slot and is dropped with the
   request. A live request requires an owning-viewport waker; fail admission if none exists, rather
   than waiting indefinitely for an unrelated frame.
5. On the UI thread, `control::drain_ui` takes completed slots, rejects cancelled/timed-out/stale
   generations, validates image dimensions against the capture spec (or returns a documented scale
   mismatch error), matches by serial and calls `shot_reply` **inside the existing
   `windows::guard` / `file_access::confinement`**. A late callback after timeout or close drops its
   image without writing or replying; a reused viewport ID cannot bind an old capture to a new
   document.
6. Admission bounds aggregate image bytes, not only the number of requests: reserve
   `surface_pixels × 4` (plus a declared margin) before issuing, cap request count independently,
   and release both on completion/cancel/timeout. After drain/shutdown the retained pending image
   bytes must be zero. A post-callback size check alone does not bound peak memory.
7. Time out unfinished serials as today (transport timeout and `SHOT_TIMEOUT`), cancel the slot and
   send the same error once. Handle channel-disconnect as cancellation.

Required tests:

- Two requests in one frame complete in reverse order; each reply carries its own request ID/path,
  correct dimensions and capture-time DPI/geometry metadata.
- Cancel before callback, callback after timeout, window close, document replacement, viewport
  reuse, and a child-viewport capture: no stale file write/reply, wakeup reaches the captured
  viewport, and retained bytes return to zero.
- Aggregate admission rejects an oversized/concurrent batch before issuing, then accepts a smaller
  request and releases memory after drain.
- A path outside the automation write root is rejected when completed from the slot; a callback
  outside the roots cannot write.
- Admitted hidden/minimized screenshot or injected-input requests remain queued; logic
  sends `RequestPaintWhileHidden` through settling until UI consumes them. The prior
  screenshot error is returned only on deadline expiry if no paint arrives, exactly once.
  This intentionally replaces immediate hidden rejection for these two control methods.
- A translucent pixel round-trips straight RGBA; premultiplied alpha is not darkened.
- The callback never holds a strong `Context` cycle; dropping the app/pending list releases slots
  and a late callback is a no-op.

Do not use `Event::Screenshot` polling anywhere. Do not put PNG encoding or path resolution in the
callback.

Preserve both `shot_reply` branches: Some(path) writes through the automation-root guard;
None returns `{format:"png", width, height, data}` with base64 PNG. Test path-less capture
under restricted roots and compare decoded pixels, without inventing a filesystem requirement.
Use the suite's 4-request, 128 MiB/image, 256 MiB in-flight/completed image budget, checked
arithmetic and reject-before-issue behavior. Encode one result at a time with a bounded PNG
writer (128 MiB output cap); reserve checked `4 * ceil(png_bytes / 3)` base64 bytes before
encoding, and cap queued encoded replies at 256 MiB. Return a structured size/busy error instead
of an unbounded encoding allocation. Account scratch/readback RSS separately. Test boundary,
one-over, overflow, late/cancel and zero retained/queued bytes. Geometry follows the suite's
issuance epoch validation: pixels-only callback, target UI logical content rect, reject any
intervening resize/DPI/zoom/surface change and derive scale from returned dimensions.

### 6.4 Frame driver: background work, UI setup, first pass and hidden logic

#### Target structure

Keep the existing `FigApp::ui(&mut Ui)` root entry and split it into a phase-safe background tick and
a visible UI setup. Names are proposals, not existing APIs:

```rust
pub(crate) struct FrameClock { /* monotonic origin + injected test time */ }

impl FigApp {
    /// Consumes queued work/deadlines; safe from App::logic (run_ui or run_logic) and direct tests.
    pub(crate) fn background_tick(&mut self, ctx: &egui::Context, clock: &mut FrameClock);

    /// Fonts/theme/zoom/render worker and per-frame stats; first pass of a visible frame only.
    fn ui_setup(&mut self, ui: &mut egui::Ui);

    /// Direct-harness entry: runs the background tick if App::logic did not, then setup + draw.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !std::mem::take(&mut self.logic_ticked) {
            self.background_tick(ui.ctx(), &mut self.clock);
        }
        let stamp = (ui.ctx().viewport_id(), ui.ctx().cumulative_frame_nr());
        if ui.ctx().current_pass_index() == 0 && self.ui_frame_stamp != Some(stamp) {
            self.ui_frame_stamp = Some(stamp);
            self.ui_setup(ui);
        }
        self.frame(ui);
    }
}
```

`background_tick` owns only phase-safe app-global work:

- drain state-only control requests; admitted screenshot/injected-input `NEEDS_FRAME`
  requests stay queued for the next forced UI pass, never errored just for logic-only
  execution. `window.new` retains its explicit hidden error as a separate app policy;
- live `session.live_poll()` and `background_done` (consumed queue/revision identity);
- recovery tick and autosave checkpoint/scheduling/save-thread joins, driven by monotonic deadlines;
- plugin/live preference persistence triggered by consumed dirty flags;
- title/close/recovery requests that must happen while hidden;
- prototype/after-delay/GIF state advance using monotonic deadlines (once per due deadline, not
  once per callback), with drawing left to `present::show`.

`ui_setup` owns only UI-phase work that needs a real current `Ui`:

- `ctx.options_mut(|o| o.zoom_with_keyboard = false)` once at startup;
- fonts/theme/language and `cjk_needed` (event scan) on the first visible pass;
- `ctx.content_rect()`/`ctx.set_zoom_factor` and `ui.ui_scale` using the actual current geometry;
- `render_worker::Worker::new` and `FrameStats::record` once per visible root frame, not per pass;
- the first-font-frame `skip_draw` flag: draw nothing, request repaint, and let the next frame draw.

`FrameStats::record` and `ui.inspect {framesDrawn, repaintCauses}` count frames, not passes. A
first-pass guard is used only for `ui_setup`; it is not a command or queue guard.

#### Callback wiring

- `apps/figcraft/src/main.rs::App::logic` calls
  `with_app(|fig| windows::guard(fig, ctx, |fig| fig.background_tick(ctx, &mut clock)))` on every
  callback. Wrapping in `windows::guard` preserves `file_access::confined` automation roots and
  panic recovery for autosave/recovery/preference writes. Set `fig.logic_ticked = true` when the tick
  ran.
- `App::ui` calls `fig.ui(ui)` through the existing `windows::guard` wrapper.
- `FigApp::ui` runs the background tick only when `logic_ticked` is false (direct harnesses), then
  `ui_setup` on pass 0, then `frame`.
- `windows::draw` calls a new `child_logic(ctx, id)` for per-viewport state (focus, close request)
  and **not** the app-global `background_tick`. The root frame is the single owner of global
  autosave/live/recovery/background work; a visible child in the same native frame must not repeat
  it. Child `frame` keeps its existing `windows::guard` boundary.
- `FigApp::frame` calls `control::drain_ui` (UI-capable requests queued by the background tick and
  any screenshot completions) instead of the old all-methods `control::drain`. A state-only request
  arriving between passes is consumed by the next `App::logic` call; fresh work is never starved by
  a nonzero pass index.
- `Context::run_logic` does not advance `cumulative_frame_nr`; do not use it as a per-frame identity.
  Repeated hidden ticks are expected and must be safe because their queues/deadlines are
  consumption- or deadline-driven.

#### Hidden/logic-only rules

- `Context::run_logic` leaves input events, time and layout stale. `background_tick` must not call
  `cjk_needed` (event scan), `shortcuts::handle`, `a11y::begin`, `swallowed_paste`, or any code that
  consumes pointer/key events. It may drain state-only control requests and use commands that do
  not read UI input.
- Do not infer phase from `ViewportInfo::visible()`, minimized/occluded state, or a remembered pass
  index. A forced hidden `run_ui` has current input and real geometry; a `run_logic` callback does
  not.
- Use `FrameClock` based on `web_time::Instant` for autosave, recovery and playback deadlines in
  both paths. In UI phase, `ctx.input(|i| i.time)` may be recorded for diagnostics, but deadlines
  never come from stale input time. Tests inject a fake clock.
- Prototype timers, after-delay actions, animated transitions and GIF frame clocks advance from
  `background_tick`, not only `present::show` or canvas drawing. Split the existing timer/action code
  in `crates/ui-egui/src/present.rs::show` (and the canvas GIF clock) into a non-drawing step called
  by the tick; `show` only draws the state the tick produced. Hidden logic must not advance the same
  deadline twice; the visible pass must not re-run delayed actions.
- Timers must schedule a wake-up even while hidden (`request_repaint_after`/eframe's logic-only
  path), otherwise autosave, recovery and live polling stop when no frame paints. Test that a
  hidden `run_logic` at the deadline runs the tick once, not zero and not twice.
- If a background completion would open a dialog or touch UI-only state while hidden, queue a
  visible-only action instead of mutating a pane not swapped in. Commands that need a drawn frame
  (`ui.screenshot`, `ui.input`) remain queued with deadline and request hidden paint.
  Only `window.new` retains its existing explicit error in logic-only mode. Keep one
  injected gesture step in flight across repeated logic ticks until UI applies it once;
  do not combine press/move/release before that pass or replay events after discard.
- Hidden child windows: root logic may need to reconcile deferred viewports. If a child must be
  created/updated/closed while the root is hidden, send `ViewportCommand::RequestPaintWhileHidden`
  from root logic and let the root UI run without presenting. Add a hidden-root/visible-child test.
  #8748 is closed/unmerged; do not assume the child behavior exists without this probe or a reviewed
  fork fix.

#### Multi-pass and side effects

- `Context::run_ui` may call the closure more than once if `request_discard` is used. `App::logic`
  runs once per pass too. Therefore every `background_tick` operation must be idempotent by consumed
  queue, revision/generation or monotonic deadline. A global first-pass guard must not suppress a
  fresh control request or a background completion that arrives between passes.
- User actions are dispatched through an explicit one-shot action queue/token, not by gating all
  commands on a pass index. Each interactive widget queues an `ActionToken` carrying the viewport,
  frame number, source widget ID and command; `FigApp::frame` drains the queue once per token and
  calls `run`/`run_user`/`set_props`. Reads and drawing remain per-pass. `FigApp::run` and
  `set_props` do not receive a `Context`, so they cannot and must not read
  `current_pass_index()`.
- Test a sizing pass (`ctx.request_discard()` once) while clicking a command button: one action, one
  undo entry and one command result. Deliberately replay the same token in the second pass: the
  dedupe must prevent a second undo. Also test that a command/background completion produced during
  pass 1 is still applied (the guard is not a global starve).
- Undo/redo remains engine-owned; no UI-side transaction may span passes.

### 6.5 MouseWheel, input, IME/E21b, RTL, clipboard and PRIMARY

- Add `source: MouseWheelSource` to every construction. `control.rs` synthetic scroll uses
  `Wheel`; unit tests use `Unknown` unless testing source-sensitive scroll behavior.
- `Event::PasteImage` and `Event::MiddleClickPaste` are new but optional for FigCraft; do not replace
  the existing arboard/WebServices paths without a regression test. The canvas paste path must still
  accept image bytes and the browser's `paste` event.
- **PRIMARY:** E4's regular Wayland clipboard is not enough. The selected design is winit-owned
  PRIMARY on the existing connection/event queue and per-seat lifecycle, using SCTK 0.19.2
  `PrimarySelectionManagerState`/device/source/offer; do not bind a second regular
  `wl_data_device`. W15P lands and is reviewed before the E4 P2b integration stage. FigCraft must
  keep password exclusion, not reclaim an unchanged selection, and test pointer/keyboard triggering serials (including keyboard-only focus), per-seat origin/freshness, serial
  freshness, owner/seat changes, empty/absent offers, self-paste and canceled/slow transfers on
  wlroots/KWin/Mutter. arboard is only a measured fallback if W15P cannot meet a proven requirement;
  a documented unsupported limitation needs explicit acceptance, not a compile-only sign-off.
- IME incoming: 0.37 changes composition visuals and deletion. Keep E6/E21; run `ime_tests` and add
  a delete-surrounding-bytes test asserting one undo step. The engine text edit state and egui
  `TextEdit` state must not double-apply commits.
- **E21b/P7b outgoing surrounding text:** the candidate `egui-winit` never calls
  `WindowExtWayland::set_ime_surrounding_text`, and `egui::IMEOutput` has no logical text. Add the
  bounded transient payload (at most 4000 UTF-8 bytes) to `IMEOutput`, preserving Clone/Debug/Eq,
  excluding it from serialized/debug persistence, with committed logical text, cursor/anchor byte
  indices and owning widget/viewport identity. Built-in `TextEdit` produces it after applying
  delete/commit/cursor updates, excluding preedit and remapping indices; a selection that cannot
  fit returns explicit unavailable/empty context, never a truncated “complete” context. Password
  fields emit no text; focus/reset/normal-to-password/no-IME transitions clear backend state.
  `egui-winit` sends it on text/selection/owner changes through W27 on Wayland only; per-window
  caching must not bleed across widgets/windows. PlatformOutput append keeps the final IME output
  and payload together across passes.
- **FigCraft's custom editor adapter:** `crates/ui-egui/src/shortcuts.rs::ime_output` currently
  emits only purpose/rect/cursor_rect/interruption (around line 552). It must supply the real model
  adapter: committed document text for the edited node, preedit removed, caret/anchor converted from
  the engine's character/scalar coordinates to UTF-8 byte indices, bounded to 4000 bytes, with
  explicit unavailable when the selection is too large. Use logical text, never the display-bidi
  string from `bidi.rs`. This adapter is part of the atomic F2 compatibility stage, not a `None`
  placeholder.
- IME tests: outgoing text changes immediately after delete+commit and deletion-only batches;
  scalar/grapheme vs byte indices; supplementary emoji/combining/RTL logical order; preedit
  exclusion and caret movement; long selection/window limits; same-window field switch; two
  windows; focus loss/reset/late serial; password/no-context clears; multi-pass final-output
  selection. Native Wayland Vietnamese/predictive IME verifies the protocol trace and editor
  content/undo; macOS NSRange remains W26's separate path.
- RTL/bidi: E5/E12/E13/E16 change caret/hit/row order. The fork now shapes logical text. FigCraft's
  `bidi.rs` currently pre-reorders names with `figcraft_text::display` and draws cluster sections so
  egui's old shaper does not reverse them again. After E5/E12, remove double processing where egui
  manages the widget; keep only deliberately custom painting that needs visual-order metadata.
  Never pass a display-reordered string to egui shaping, `TextEdit`, clipboard, AccessKit names,
  save, or E21b surrounding text; those boundaries use the logical document string. `rtl_names_tests`
  and a mixed-logical/clipboard/IME test must pass. Any cache keyed by `FontId` must account for
  `FontFamily::Name` and `LayoutJob::direction`. Do not delete `bidi.rs` until E5/E12 tests prove the
  fork covers FigCraft's custom name painting.
- Accessibility language: call `ctx.set_language(i18n::current().map(|l| l.code.to_string()))` in
  `ui_setup` (fork E17). Assert the AccessKit root node's language changes with the UI language.
  This is optional only if the final pinned fork lacks E17; do not use a nonexistent upstream API.
- `Ui::label_inputs_by` / `Response::accessible_name` are available in 0.37; use them only where
  FigCraft currently has unnamed native widgets. Custom `a11y` node builders remain.
- Shared `DockArea` direction: FigCraft keeps its own panels and does not adopt `craft-ui`'s
  `DockArea`. If a future PR does, it must pass an explicit opt-in `DockDirection`, preserve the
  serialized physical splits/rectangles and logical tab order, and use the same rectangles for
  painting/hit/drag/keyboard with bottom-trailing resize grips. FigCraft must not reverse saved
  panel order or feed display-bidi text to the shared dock.

### 6.6 Fonts and glyph coverage

- `crates/ui-egui/src/theme.rs::install_fonts` starts from `FontDefinitions::default()`. With egui
  default features empty, that is `FontDefinitions::empty()` unless the app/UI crate enables
  `default_fonts`. Declare production fallback policy explicitly. The proposed 0.37 kittest
  dev dependency does not transitively enable fonts: only kittest's own dev-dependencies do.
  Explicitly enable `default_fonts` and `monochrome_emoji_fonts` on FigCraft's egui test
  dependency for these fixtures. Audit UI-library/test/native/web graphs independently.
- Keep the optional `CRAFT_FONTS_DIR` build input. Record whether each baseline/candidate run had
  craft-fonts and its exact ref; a no-craft-fonts run is valid for CI but not for a font-metrics
  comparison.
- Keep `script_fonts`, `cjk_needed`, `bidi::script_fallbacks`, `key_symbol_fallbacks` and the
  “English users load none” behavior. Tests: an English first frame installs no extra script face;
  a CJK name adds the expected face; shortcut symbols (⌘⌥⇧↵) have glyphs; a font-picker preview
  renders with the selected family.
- If `system_fonts` is evaluated later, test color emoji and arbitrary scripts, and record startup
  cost. Do not replace FigCraft's deterministic fallback with a blanket font provider in this
  migration.
- Preserve the first-font-frame behavior: when `ui_setup` installs fonts for the first time, set a
  `skip_draw` flag; `FigApp::ui` requests a repaint and returns without drawing. Do not draw one
  frame with missing fonts.
- 0.37 changed text layout/font versions. Pixel/width tests may shift by subpixel amounts. Inspect
  each difference; do not auto-update expected widths. The document renderer still uses FigCraft's
  own harfrust/skrifa stack, so UI and canvas text may legitimately differ.
- **Pinned licensed fixtures (egui plan section 10.6):** fetch
  `craft-fonts` at `2481a8a324b986a45954e90045ed1f8f2efaaf3c` as a test resource and keep the
  repository's central-font policy (no font binaries committed here). Required fixture files,
  hashes and licences:
  - Arabic `fonts/noto-sans-arabic/NotoSansArabic.ttf`, SHA256
    `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`, OFL-1.1.
  - Thai `fonts/noto-sans-thai/NotoSansThai.ttf`, SHA256
    `5a1c559bb539583c8a1fd99d1c5b9491e5e14478c9cd2bd0970d5c3096cc9ef8`, OFL-1.1.
  - CJK SC/Latin `fonts/noto-sans-cjk-sc/NotoSansCJKsc-Regular.otf`, SHA256
    `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`, OFL-1.1.
  - Japanese `fonts/biz-ud-pgothic/BIZUDPGothic-Regular.ttf`, SHA256
    `258d7156c165f2ff774b6efee637c22c3b950de0d8a10e501137061bc8085d01`, OFL-1.1.
  - Hebrew Noto Sans Hebrew variable font, SHA256
    `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88`, OFL-1.1, fixed to
    weight 400/width 100.
  - Color glyphs: Noto Color Emoji v2.047, SHA256
    `39ee3c587e10e89669b9ff32703261d10d5f9c4dd5ad147b6b5a1c5200591817`, OFL-1.1. This
    fixture is used only in the `epaint/color_fonts` feature job; the normal build makes no
    color-emoji claim.
  Preserve the adjacent licence files in the evidence/fixture bundle. If a fixture cannot be
  fetched, record the font-dependent test as unresolved; do not silently rely on host fonts.
- If a controlled Latin/emoji comparison is needed, use the exact `epaint_default_fonts` files and
  hashes from the pinned baseline/candidate, not whatever the host provides. The normal CI
  no-craft-fonts job still runs and must not become a font-metrics gate.

### 6.7 Panels, windows, viewports and GPU

- Panels: keep the root order in `crates/ui-egui/src/lib.rs::frame` (left rail, left panel, right
  panel, central, then custom `panel_edge` overlays). 0.37 panel constructors accept `AsIdSalt` and
  include separator width; test `fit_panels`, `panel_width_tests`, canvas minimum and hit testing.
  Do not replace the custom edge drag with `Panel::resizable` without a UI3 geometry comparison.
- Clip/nesting: keep the root `Ui` and `scope_builder("toolbar")` nesting; custom overlays keep
  their own `with_clip_rect` calls. 0.37 keeps `clip_rect_margin` as a deprecated unused field;
  remove the use rather than expecting a compile error, and test that panel content does not paint
  over the canvas, popups/menus and drag ghosts stay in their intended layers, and hit rects match
  what is visible.
- Docking: FigCraft does not use `craft-ui` docking; do not add it as part of this migration. The
  capture completion slot stays local. If a future PR adopts `DockArea`,
  follow the explicit direction policy in 6.5.
- Child windows: keep `windows.rs` deferred viewports and pane swapping. 0.37 adds
  `RequestPaintWhileHidden`; use only for reconciliation when a child must appear/update/disappear
  while the root is hidden. Keep `embed_viewports` and the screen-reader tab fallback until
  per-viewport AccessKit is proven.
- Title bars: `craft-window` decides `TitleBar::Native/Inset/Custom`; `window_chrome` maps to
  `ViewportBuilder`. 0.37 keeps these builder methods. macOS traffic-light positioning is not
  changed without a Mac; `Frame::set_traffic_lights_position` is available if later measured.
- GPU: `apps/figcraft/src/graphics.rs::backends/configure` still mutates
  `WgpuConfiguration`/`WgpuSetup::CreateNew(create).instance_descriptor.backends`; verify compile
  and preserve DX12-then-GL on Windows. E2/E3 stay. Measure `Renderer::new`/first frame on Windows
  DX12; do not report compile time as UI performance.
- Texture deltas: eframe owns upload/free of `textures_delta`; app code must not drop them. The
  0.37 glyph atlas and color-glyph path change atlas contents and filtering. Keep
  `TextureOptions::LINEAR` and clamp/repeat/mirror behavior where used; test a translucent image and,
  only in the `epaint/color_fonts` job, a color-glyph fixture. Do not use clamp-only as a
  performance “fix” (#8736).

## 7. Test and regression plan

### 7.1 Repository gates through `task.py`

Read `/home/codex/work/maintenance/TASK-JOBS.md` before the first submission. Every future
fmt/clippy/test/benchmark is a new job on one FigCraft task; a fresh target is allocated once, and
`finish` is applied only after all checks, GUI workflows and evidence preservation are complete.
Example template (do not run during planning):

```sh
TASK=figcraft-egui037-migration
SRC=/home/codex/work/CraftApps/egui037-migration-20261022/figcraft
EV=/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/figcraft-validation

python3 /home/codex/work/maintenance/task.py run \
  --job-id "$TASK-ci-01" --kind build --task "$TASK" \
  --source "$SRC" --evidence "$EV" \
  -- cargo xtask ci
python3 /home/codex/work/maintenance/task.py status "$TASK-ci-01" --tail-bytes 8192

# One new job ID per intentional command, same task/source/evidence:
python3 /home/codex/work/maintenance/task.py run \
  --job-id "$TASK-ui-tests-01" --kind build --task "$TASK" \
  --source "$SRC" --evidence "$EV" \
  -- cargo test --locked -p figcraft-ui-egui
python3 /home/codex/work/maintenance/task.py status "$TASK-ui-tests-01" --tail-bytes 8192
```

After review and evidence preservation, inspect `task.py finish JOB_ID`, then apply with
`--apply --owner-confirmed --checks-complete --evidence-preserved` and poll status. The payload after
`--` may be `cargo fmt --all -- --check`, clippy, workspace tests, `xtask layers/assets/wasm`, or a
targeted suite, but never a direct host command or a nested build driver. `cargo xtask ci` is
equivalent for the repository; run individual payloads when evidence must identify a failing gate.
`cargo test --workspace` builds without craft-fonts in CI; a with-craft-fonts configuration is a
separate job and records the exact font ref/hash.

Targeted suites, each its own job: `figcraft-ui-egui`, `figcraft-engine`, `figcraft --bin figcraft`,
`figcraft-web`, `figcraft-cli`. Then native Windows x64 and wasm workflows. macOS/FreeBSD remain
unresolved unless real evidence is available; a compile-only result is not a platform pass.

### 7.2 New/updated regression tests

| Test area | Assertion | Mutation / broken probe that must make it fail |
|---|---|---|
| Native/web lifecycle | Visible single and multipass: `App::logic` runs per pass, background work is idempotent; `ui_setup` runs once on pass 0; completed `run_ui` leaves pass index 0 | Remove the idempotence/consume guard: duplicate autosave/live work must be detected. Add a test that expects a stale nonzero pass index: it must fail |
| Hidden logic | Repeated `run_logic` after a completed multipass calls `background_tick` each time (pass index 0), advances deadlines once per due instant, never consumes UI events, never runs `ui_setup` | Suppress background work on pass index alone: hidden/fresh-queue test fails. Call `cjk_needed`/`shortcuts` in logic-only: stale-event test fails |
| Forced hidden UI paint | `RequestPaintWhileHidden`/full hidden pass runs `App::logic` + `App::ui`; `ui_setup`/input handling work because it is a real `run_ui`; visibility flag is not used to choose phase | Infer `LogicOnly` from `visible()==false`: hidden-paint setup test fails |
| Restore/buffered input | Restore with buffered events dispatches one gesture once; a background completion between passes is not starved | Gate all commands on pass 0: completion/action test fails |
| Multi-pass commands | A `request_discard` sizing pass executes one queued action, creates one undo entry, returns one result; replaying the same action token does nothing | Remove token dedupe: duplicate history entry must fail |
| Two windows/two viewports | Root plus visible child in one native frame: one app-global live poll/autosave/recovery/background completion, not one per child | Remove child/global split: two-window duplicate test fails |
| Panel IDs/a11y | Actual `Id::unique(("panel_edge","left"/"right"))` is absent from `focusables`; F6/Tab never focuses it; the next control gets focus; panel geometry unchanged | Revert to `Sense::click_and_drag()` or the old `left_panel.__resize` filter: focus test fails on the real ID |
| Fixed panel geometry | Left/right fixed sizes and preferences remain identical with fresh or legacy egui memory; no persisted PanelState migration | Make a fixed panel resizable or alter its width: geometry assertion fails |
| Screenshot correlation | Two requests, reverse completion, each reply has its own ID/path/image/spec; callback outside automation roots rejected; hidden screenshot queues then paints or expires once | Match by viewport only or encode on render thread: cross-reply/root test fails |
| Screenshot cancellation/lifetime | Cancel-before-completion, timeout then late callback, window close, viewport reuse, document replacement: no file/reply; child wakeup reaches captured viewport; aggregate bytes return to zero | Strong callback context or unbounded queue: leak/retained-bytes test fails |
| Screenshot alpha | Translucent pixel round-trips straight RGBA; premultiplied alpha not darkened | Use premultiplied bytes directly: pixel assertion fails |
| Accessibility | Roles/names for toolbar, panes, tabs, tree items, spin buttons, combo boxes; root language; every input has a name; strict kittest missing-name/glyph failure on a new unnamed/tofu fixture | Remove a node label/role or opt into tofu silently: named-control/glyph assertion fails |
| Fonts/glyphs | English loads no extra face; CJK name adds one; shortcut symbols and font-picker preview have glyphs; no tofu in pinned fixture; no-craft-fonts CI still runs | Disable `default_fonts` or rely on host fonts: glyph/panic test fails |
| Input/IME/E21b | Mouse wheel source; IME commit/delete one undo; outgoing surrounding text after delete+commit and deletion-only; preedit excluded; UTF-8 byte indices; long selection unavailable; focus/password clear; two windows | Set payload `None`, omit preedit remap, or pass display-bidi text: protocol/editor test fails |
| RTL/logical boundaries | Bidi caret/hit/row order; clipboard, AccessKit name, save and IME use logical text; no double reversal with E5/E12 | Feed display text to shaping/clipboard/IME: logical-boundary test fails |
| Commands/undo/save | Rectangle/text edit → undo restores exactly; save/reopen compares `document.inspect`; no changed command ids/parity | Skip history snapshot/change save order: hash/inspect mismatch |
| Geometry/screenshots | 1440×900 and 800×500: panel widths, canvas min, toolbar/tab hit rects, scale factors 100/125/150/200%; PNG dimensions and stable non-empty regions | Move a panel edge or change separator width: exact geometry assertion fails |
| Viewports/hidden child | Main + deferred child; close asks on unsaved; title follows document; screenshot targets right viewport; hidden root + visible child opens/updates/closes via `RequestPaintWhileHidden` (#8748 probe) | Remove request-paint: child state remains stale; treat closed #8748 as merged: test cannot pass |
| #8763/#8764 selection | Select text, click a toolbar/context action; intended range is used, pointer and keyboard agree; one undo | Remove selection preservation: selected range becomes stale/collapsed |
| Idle/performance hooks | `ui.inspect` reports no idle repaint and stable frame counts; autosave/recovery run while hidden; prototype playback does not duplicate actions after a sizing pass | Tick from `present::show` only: hidden/duplicate-action test fails |

Pixel comparisons:

- Prefer geometry, shape counts and semantic AccessKit assertions for control logic.
- Use screenshot/pixel comparisons for synthetic scenes: empty document, frame with text, selected
  frame with handles, RTL name, CJK name, font picker, dark/light, 100% and 150%.
- Fixtures are generated in code (`figcraft_engine::synthetic`,
  `figcraft_engine::synthetic_ds`) or original assets already in the repository; no product
  screenshots or third-party assets.
- Tolerance: exact geometry; for antialiasing/font raster changes allow at most 0.1% of pixels
  differing by more than 2/255 per channel and no pixel differing by more than 4/255; text layout
  bounding boxes may differ by ≤0.5 point after review. Any larger diff needs a written
  before/after reason and reviewer approval. Never auto-update a golden.
- Screenshot evidence is stored outside the repository in the durable task evidence directory;
  reference screenshots of other products stay under the gitignored `plan/` tree and are never
  committed.

Deliberate negative controls: wrong capture/viewport association, missing accessible name,
missing glyph, dropped callback, forced duplicate action, stale hidden input and an unbounded
capture queue must each fail the targeted test; remove the deliberate defect after confirming.

### 7.3 Native, browser and platform workflows

- Linux: headless tests plus a real X11 and Wayland session for window, clipboard, PRIMARY, IME and
  panel geometry. The planning host has no inherited `DISPLAY`/`WAYLAND_DISPLAY` and no compositor;
  the parent can access an i915 render node but this worker sandbox may hide `/dev/dri`. That is
  availability evidence only, not a qualified GPU/compositor result. Provision a display/window
  manager and record versions; if unavailable, record the gate as unresolved.
- Windows x64: build/test in a uniquely owned clone, then launch the actual app on the interactive
  desktop, drive the control channel, capture window screenshots at 100/150/200%, test DX12 startup,
  IME/non-Latin shortcuts, save/reopen and multi-window. The sealed baseline uses the Microsoft
  Basic Render Driver; do not present it as a physical-GPU result. Do not count session-0 launches.
- wasm: `trunk build --release`; load the page with WebGPU and `?webgl`; test open/import, edit, undo,
  export/download, live join, browser clipboard copy/paste, responsive panel geometry and, only if
  `epaint/color_fonts` is enabled in that build, color emoji; record browser/OS/GPU. The browser has
  no native file paths and no control server; state whether automation or manual evidence was used.
- macOS/FreeBSD: no routine UI test job. A release-workflow build is not a UI pass. Mark macOS
  window/traffic-light/IME/PRIMARY and FreeBSD runtime rows explicitly unresolved unless a real
  machine is available.

## 8. Performance and measurement plan

### 8.1 Baseline and three-way controls

Before modifying production code, from clean baseline `80f9163` with the old manifest/lock, record:

- exact `Cargo.lock` hash, `rustc -Vv`, `cargo -V`, target triple, features, `CRAFT_FONTS_DIR`
  presence/ref, `FIGCRAFT_EMBED_CJK`;
- OS version, CPU, RAM, GPU/adapter/backend/driver, DPI/scale, window size, renderer (software vs
  real GPU), release profile, `WGPU_BACKEND`;
- raw samples, command lines and timings in the durable evidence directory.

Measure three sources on the same machine, checkout fixtures, fonts, features, DPI and viewport:
unchanged 0.36.2 baseline (`49682f8b`/FigCraft `80f9163` lock), stock 0.37.0
(`841a328b`) via a pinned minimal adapter/harness where the app cannot compile unchanged, and the
final selected storytold fork. Record all three source/lock identities. An unavailable stock
equivalence is an attribution limitation, not evidence that an upstream regression is unavoidable.
Candidate runs use paired A/B/A/B ordering; do not compare compile duration with UI frame time.

### 8.2 Operations and issue-specific workloads

1. `figcraft-cli perf --layers 1000,10000,50000`: build/open, save/load, full render, retained pan,
   one-layer edit, hit test, hover, node edit, undo/redo, boolean drag, normalize, inspect,
   select-all+move. Existing rows in `apps/figcraft-cli/src/perf.rs::measure`.
2. `perf_ui_frames` / `report_components`: idle, hover, wheel pan, one layer selected, Prototype
   tab, all screens expanded, scroll layers, filter, keyboard navigation, bottom-layer reveal,
   collapse all, Assets panel first/idle/scroll, edit a component, instance with many properties,
   72-variant set. Existing rows in `crates/ui-egui/src/perf_tests.rs`.
3. **#8731 text tessellation:** fixed synthetic document with 120–240 text labels; layout once, then
   `ctx.tessellate` the same shapes for 30 steady repetitions (10 warmups); report per-call time,
   vertex/index counts and raw samples. Run 0.36, stock 0.37 and final fork. A row-level fast path is
   only accepted with mixed/color-row equality.
4. **#8736 DX12:** measure renderer/pipeline construction separately from first frame on Windows
   x64, 20 fresh processes, with `WGPU_BACKEND=dx12`; compare 0.36, stock 0.37 and final fork.
   Keep a software/renderer control row and identify the actual adapter/driver.
5. **#8512 font startup:** measure system-font discovery and first missing-glyph lookup separately,
   20 fresh processes, with and without `CRAFT_FONTS_DIR`/`system_fonts`; record coverage and
   provider settings. Do not enable fontique by default as a performance shortcut.
6. Native startup: process spawn to first successful `ui.inspect`/first painted frame; 20 fresh
   processes; separate shader/renderer creation logs.
7. Idle repaint/CPU: 60 s hidden and visible idle; `framesDrawn`, `repaintCauses`, OS CPU. Must be
   zero frames and ~0% CPU when idle.
8. Memory: Linux `/usr/bin/time -v` or `/proc` peak RSS; Windows private bytes; record peak for
   10k/50k documents, startup and capture admission.
9. Screenshot/control latency: `ui.screenshot` request to reply (including completion-slot drain,
   cancellation and pending-bytes counters), `engine.execute`, save/reopen, window open/focus/close.
10. Browser: wasm size, first interactive, open/edit/undo/export, frame time in Chromium; WebGPU and
    WebGL2 separately, with software fallback vs real GPU identified.

### 8.3 Statistics and thresholds

- At least 10 warmups and 30 steady timed iterations for operations; at least 20 fresh processes
  for startup; at least 1,000 frames for any frame distribution and p99. Slow full renders also use
  30 samples, not 5. Report raw samples, median, p95, min/max, MAD and sample count; report p99 only
  where the sample count supports it. A coefficient of variation >5% triggers a rerun on an idle
  machine and a recorded explanation. Modify `crates/ui-egui/src/perf_tests.rs` and
  `apps/figcraft-cli/src/perf.rs` to emit raw samples and these aggregates; the current means-only
  output is not acceptance evidence.
- Three-way acceptance on paired baseline/candidate (shared suite floors are no looser than 5%
  median / 10% p95; FigCraft keeps its stricter engine/render/UI gates):
  - engine/render/UI median ≤ +3% and p95 ≤ +5% relative; absolute floor 0.05 ms for sub-ms
    operations. If evidence shows the stricter gate is noisy, record an explicit, pre-approved
    exception; never silently relax after seeing a failure.
  - text tessellation ≤ +5% median (if upstream #8731 remains unpatched, the expected +61% is a
    blocking regression, not an accepted number); report stock 0.37 and fork separately.
  - Windows DX12 startup ≤ +5% median / +10% p95 over 20 fresh processes; if #8736 remains unpatched,
    measure and require an owner-approved waiver or a provenance-recorded patch.
  - font discovery/first missing glyph: report absolute timings and coverage separately; a
    repeatable >5% median or >10% p95 regression is investigated, not accepted as default cost.
  - idle visible/hidden: zero unscheduled app repaint requests after settling; scheduled
    work keeps its documented deadline, and there is no new busy-loop.
  - peak memory ≤ +3% or +64 MiB, whichever is reached first, for the same scenario; capture pending bytes
    return to zero after drain.
  - screenshot/control latency ≤ +10% p95; save/reopen ≤ +5%.
- A regression triggers profiling and a written cause/fix; it is never accepted by changing a golden
  or deleting a test. Software-renderer and real-GPU results are separate rows. A stock-0.37-only
  failure is still an attribution result, not a FigCraft fork pass.

The required X0 dependency is the complete P8 handoff. E24/E25's selection-action and
hidden-child regressions map to fork P7c (conditional fixes after reproduction), not optional
app fallbacks. E21b requires owner/context generation even for Unavailable, Available-empty
separate from unsupported, W27 committed clear transitions, latest-pass None authoritative,
old-focus-epoch invalidation and current-epoch mismatched serials applying incoming edits once
while deferring outgoing until a match. F2's adapter and native protocol tests use that exact
contract, including empty→non-empty and password→normal transitions. P2b's bounded async
transfer/cancel/routing and P7's emitted-key release pairing are also X0 prerequisites.

Clipboard routing in F2 is by action/focused editor: built-in TextEdit text Copy/Cut/Paste
uses egui-winit; custom document/rich HTML/image operations retain Desktop Services' arboard
handle and its wayland-data-control support. A single input action has one consuming path;
never write both owners for the same action or mirror unchanged selection on a timer. PRIMARY
belongs exclusively to W15P; Services never claims PRIMARY. Independent data-control and regular
data-device handles are not themselves a duplicate regular wl_data_device on one connection.
Test alternating TextEdit/document copy, HTML/plain/image fallback with alpha, focus/window
changes and one paste/undo per action on supported compositors. Preserve the existing Services
read-timeout behavior as a separately measured baseline; P2b's nonblocking guarantee is for the
egui integration path, not an unimplemented rewrite of all Services clipboard calls.

The existing craft-window pin 1f805378 is branch-derived; verify and retain exact fetchability
and source/executable evidence for baseline/rollback. Unmerged provenance alone does not prove
the commit is unreachable. F2 still switches to the final reviewed craft-libs revision.

## 9. Reviewable PR stages and dependency DAG

```
External prerequisites (not FigCraft PRs)
  X0: reviewed + approved storytold/egui final revision (complete P8: E1-E21 + P2b/P7/P7b/P7c)
  X1: reviewed + approved storytold/winit final revision (W1-W27 + W15P)
  X2: reviewed + approved storytold/craft-libs final revision
      (merged PR #199 base 3832c47; reviewed capture/text helpers; craft-window/platform/quit;
       reviewed CL-2/CL-2F capture contract for the local adapter; no b68de8c/ec8840a tree)
  X3: optional E22-E25 only after measurements, with provenance and fresh review

FigCraft
  F0 baseline evidence (no production change)
      |
  F1 lifecycle/action refactor on 0.36:
     background_tick/ui_setup split, child/global split, action tokens, hidden/multipass tests
      |
  F2 atomic 0.37 compatibility PR (depends F1, X0-X2):
     manifests, final patches, lock, Id/Ui/WidgetType/MouseWheel/Band/scroll,
     local capture slot, panel IDs + preference preservation, a11y edge,
     E21b adapter, RTL logical boundaries, kittest strictness
      |
  F3 optional performance/provenance patches (depends F2, F0/F2 measurements)
      |
  F4 workaround removal + docs/ROADMAP/parity notes (depends F2, F3; only with evidence)
```

Each stage is a separate reviewable PR/branch and must be green on its own:

- **F1** includes the queued hidden screenshot/input behavior above and its basic
  paint/timeout regression in the atomic callback migration. It touches `crates/ui-egui/src/lib.rs`, `windows.rs`, `control.rs`, `present.rs` and
  `apps/figcraft/src/main.rs` plus tests; no manifest/dependency change. It proves the new
  background/UI split and action tokens on 0.36 first, including the native/web call sequence,
  hidden logic, forced hidden paint, restore-with-buffered-input and two-window global work.
- **F2** is atomic: root/app/UI manifests, `Cargo.lock`, final patches, `ui-egui`
  source and tests, the E21b model adapter and the panel preference/geometry checks. It must not leave a
  half-updated dependency set. The mechanical ID/WidgetType codemod and capture callback may be
  separate commits inside the PR, but the PR is mergeable and green as a whole.
- **F3** changes only the final patch revision(s) and carries the measurement/provenance table;
  rollback is a revision change plus relock.
- **F4** removes a workaround only after the corresponding test/manual evidence; otherwise retain
  the workaround and record why.

Publication rule: every push/tag/new PR needs fresh explicit human approval after the parent shows
the exact contents and validation. No approval is granted by this plan. Public prose contains no
agent/model/tool signature; contributor credit remains username-only (`@skylartaylor`).

### Rollback

- Preserve baseline `Cargo.toml`, `Cargo.lock` and manifest hashes in evidence.
- Roll back by reverting F2 (and F1 if the lifecycle change is implicated) in reverse order; do not
  keep a 0.36/0.37 feature flag for these APIs.
- Optional F3 patches roll back by restoring the previous final fork revision and re-locking; do not
  rewrite history or force-push.
- If only `craft-window`/`craft-libs` is at fault, repoint to the last merged public revision and
  relock; do not copy the crate into the repository or use a machine-local path.
- Keep the old coherent dependency set available in git history; do not delete retained evidence or
  unreviewed source work. Rehearse restoring the previous executable/lock and reopening a saved
  fixture before accepting the stage.

## 10. Blocking unknowns and acceptance gates

1. **Final storytold revisions:** `STORYTOLD_EGUI_FINAL_REV`, `STORYTOLD_WINIT_FINAL_REV` and
   `STORYTOLD_CRAFT_LIBS_FINAL_REV` do not exist until the reviewed series is approved and published.
   F2 is implemented locally against the review worktrees, but it cannot be published or presented as
   an immutable-pin change before those revisions exist. Candidate aiongg SHAs are comparison inputs.
2. **craft-libs #199:** PR #199 merged as `3832c47ef68a0e6214f2c1e85f3526b6adcb686e`. That merged
   base is the anchor; the earlier `b68de8c` head and `ec8840a` audit merge are stale history and
   neither tree is usable. The local worktree adds only the reviewed capture/text helpers on top of
   `3832c47`; no published revision exists yet.
3. **W15P PRIMARY:** the candidate's arboard fallback is not accepted for GNOME/Mutter. W15P must be
   reviewed/published and E4 P2b integrated with native wlroots/KWin/Mutter evidence; otherwise the
   PRIMARY gate is unresolved/blocked.
4. **E21b/P7b:** outgoing surrounding text is not present in the comparison candidate. The local
   reviewed fork plus FigCraft's custom-editor producer implement the bounded payload and
   receipt/change-cause contract described in `docs/egui-037-local-followups.md`; the Wayland
   adapter and final immutable fork pin still require review, validation and publication. `None`
   placeholders are not a port.
5. **Fork publication reachability:** final storytold repositories must be public/readable; a
   private/branch dependency breaks contributors. Fresh human approval is required before any push.
6. **Language #198 / E5-E21:** the fork is the integration point. If the reviewed series changes or
   is reverted, bump the pin in a separate PR and rerun RTL/IME tests. Do not copy patches into
   FigCraft.
7. **Known 0.37 performance regressions (#8731, #8736, #8512):** decide only after three-way paired
   measurements. If thresholds fail and no provenance-recorded fix is accepted, the migration is
   blocked.
8. **TextEdit click-away (#8763/#8764):** #8764 is a proposed, unmerged fix. Verify FigCraft's
   custom selection actions; if affected, fix/test before merge.
9. **Hidden deferred children (#8748):** closed and unmerged. Verify with
   `RequestPaintWhileHidden`; otherwise adopt a reviewed fork fix or record the unresolved behavior.
10. **Font input:** baseline and candidate must use the same `CRAFT_FONTS_DIR` ref. A
    no-craft-fonts run is valid for CI but not for a font-metrics comparison. Color-emoji claims
    require the `epaint/color_fonts` feature job.
11. **macOS/FreeBSD/real GPU:** no routine UI test evidence. The parent-visible i915 render node is
    unqualified, and this worker sandbox may hide `/dev/dri`; there is no inherited compositor or
    ambient display. Compile/packaging results are not runtime passes. Mark them unresolved until
    real evidence exists.
12. **Lockfile resolution:** one egui family, one winit, one wgpu, one craft-window. If
    Cargo chooses two versions, stop and fix the pins rather than accepting a duplicate.
13. **AccessKit per-viewport:** the pinned 0.37 fork initializes the native AccessKit adapter only
    for the root viewport; creating a secondary viewport does not initialize an adapter. The
    “main window only” limitation remains, and no real screen-reader qualification is claimed.
    Keep the tab fallback until secondary-window accessibility is implemented and tested.
14. **Automation screenshot thread:** the callback must not write files outside `file_access`
    confinement. The shared completion-slot design in 6.3 is required, not optional.

## 11. App-specific acceptance assertions

- Opening a synthetic document, editing a rectangle and text, undoing, saving and reopening gives
  the same `document.inspect` tree; command ids, undo steps and file bytes are unchanged except for
  documented UI metadata.
- The UI at 1440×900 and 800×500 keeps the panel/canvas geometry contract (`fit_panels`,
  `panel_width_tests`), the central canvas hit area, and the toolbar/tab hit rects.
- F6/Tab focus order covers panels and controls but never the actual custom panel edge; AccessKit
  roles and names match `a11y_tests`; the root language follows the UI language.
- A screenshot of the main window and of a deferred child returns the correct viewport, request id,
  capture-time geometry/DPI and one reply, including two requests completed out of order; hidden
  screenshots queue then paint, or return the existing error once on deadline;
  cancellation/timeout/close never writes late.
- English, RTL, CJK and shortcut-symbol fixtures have glyphs using the pinned licensed fixtures; the
  font picker previews without changing the document; no extra script face is loaded for an
  English-only screen.
- RTL names and custom text edit logically: the displayed order is correct, while clipboard,
  AccessKit name, save and IME surrounding text carry the logical string.
- `ui.inspect` reports no idle repaint requests and stable frame counts; autosave/recovery run while
  the window is hidden; prototype playback does not duplicate actions after a sizing pass.
- Windows DX12 starts, draws, opens/saves/reopens, handles IME/non-Latin keys, and shows no Vulkan
  selection unless requested. Linux X11/Wayland copy/paste and PRIMARY are reported only from a real
  session. Browser WebGPU and WebGL2 builds load, edit, undo, export and handle clipboard events.

## 12. Verification limits of this plan

- No source change, dependency resolution, build, test, benchmark, screenshot, browser run or GUI
  interaction was performed. Historical compiler errors are not current acceptance.
- Line numbers and symbol locations come from baseline `80f9163` and can move during implementation;
  anchors are given by file and symbol.
- The final fork/craft-libs merge state is unknown until approved publication. Re-check the exact
  immutable revisions and public reachability immediately before implementation and before every
  publication.
- Upstream issue measurements (#8731, #8736, #8512) are reported by others, not reproduced here;
  this plan requires FigCraft's own three-way paired measurements.
- macOS, FreeBSD, real-GPU, screen-reader and Wayland/PRIMARY behavior remain unproven until
  evidence is recorded from real environments.
- This revision was prepared by static inspection only. Reviewer claims were checked against the
  pinned source where possible; the review's suggestion to use `visible()` as a phase signal was
  rejected because the authoritative lifecycle audit and `run_logic` source show visibility does
  not distinguish logic-only from forced hidden UI.

## 13. References

Primary sources consulted:

- `storytold/egui` final reviewed revision and upstream `0.37.0` `841a328b`:
  `crates/egui/src/context.rs`, `crates/egui/src/ui.rs`, `crates/egui/src/id.rs`,
  `crates/egui/src/id_salt.rs`, `crates/egui/src/sense.rs`,
  `crates/egui/src/containers/panel.rs`, `crates/egui/src/data/input/event.rs`,
  `crates/egui/src/data/screenshot_callback.rs`, `crates/egui/src/viewport.rs`,
  `crates/egui/src/containers/scroll_area.rs`, `crates/eframe/src/epi.rs`,
  `crates/eframe/src/native/*`, `crates/eframe/src/web/app_runner.rs`,
  `crates/egui-wgpu/src/winit.rs`, `crates/egui-winit/src/*`,
  `crates/egui_system_font_provider/src/lib.rs`, `crates/egui_extras/*`.
- aiongg candidate `a4f9a17f`: `PATCHES.md`, the same source files, used only for comparison and
  gap identification.
- `storytold/winit` final reviewed revision; candidate `8b35388b` W17-W27; published
  `ffae5198` W1-W15; W15P PRIMARY design.
- `craft-libs` merged base `3832c47` (PR #199), historical API-port source `8614e4a`/`1764bca`,
  and the reviewed `craft-ui` capture/CL-2F contract as the local adapter reference.
- `craft-libs/docs/egui-037-suite-rollout.md`, `craft-libs/docs/egui-037-migration-plan.md`,
  `egui/docs/egui-037-migration-plan.md` (including section 10.6 font fixtures).
- `/home/codex/work/ssd2/task-evidence/egui037-migration-20261022/lifecycle-source-audit.md`,
  `ime-surrounding-gap.md`, `upstream-discussions.json`.


## Local implementation update — 2026-10-23

The user clarified that reviewed plans remain local and implementation proceeds before publication. The earlier requirement to publish/land this plan first is superseded; pushes and PR publication still require explicit approval.

Canonical `storytold/figcraft` `main` now resolves to `8d790607ff1ad40e32343ca2116d8fdcf3244c8d`. The owned checkout was fast-forwarded to this revision. Earlier baseline SHAs and line anchors above are retained as historical review evidence; use this new revision and its exact lockfile for current-main integration and paired measurements. The original refs remain reachable.

No migration API redesign required by this delta; preserve the merged packaging, test and chrono changes where present.

The complete static delta review and exact old/new object identities are retained in the task evidence: `canonical-diffs-refresh-20261023.json` and `overnight-consumer-review-20261023.md`. This amendment claims no new build, native GUI or performance acceptance.


## Local implementation — stage 1 (2026-10-23)

The owned worktree now contains the first coherent 0.37 compatibility/lifecycle stage described in
`docs/egui-037-local-followups.md`: the 0.37 dependency family and mechanical API ports, the
`background_tick`/`ui_setup` split with a monotonic frame clock and child/global separation, the
local capture-slot adapter (weak callback, identity/deadline/generation/geometry epoch, budget,
straight RGBA, hidden queue + `RequestPaintWhileHidden`), the real non-focusable panel-edge fix,
the root-scope a11y record, and incoming IME `DeleteSurroundingBytes` for the custom editor.
Outgoing E21b surrounding text is implemented as described in `docs/egui-037-local-followups.md`;
web hidden paint remains a recorded prerequisite, not a fake.
No build/test/GUI/browser/performance run was performed for this stage; the parent runs managed
validation and generates the lockfile from its local dependency config.

## Historical local qualification — 2026-10-25

The following counts apply to the earlier `10702311` integration, not the later
`2a3b7d4f` qualification documented in `docs/egui-037-local-followups.md`. The
status update above covers the subsequent `92682728` integration. The intervening
`9f788753` refresh passed 3,247 executions; the runs listed below passed 3,242.

The migration preserves captured main `10702311f345393ee1d8868c3257679befcfa14d`
(53 commits and 114 paths beyond the first integration). The final review commit records
both the migration history and that upstream target as parents. The selected egui revision is
`6d518fc799681571963842f95b55a6b68431b15a`, winit is
`a517b3846ee8457d84ee4975110bfda6797c44dc`, and craft-window is
`25da1314d443171b4adeaad2cd9454b273f35c6f`. Cargo.lock SHA-256 is
`7ba8af4de70ba657a652266ddee2920bfc501c851520a49ac60c8044a5c23cc1`.
The normal UI enables `default_fonts` and `monochrome_emoji_fonts`.

Managed full CI `egui037-figcraft-final-ci-20261024-2312` passed with bundled fonts.
Managed full CI `egui037-figcraft-system-font-ci-20261025-0402` also passed without
bundled craft-fonts, using a licensed DejaVu Sans system-font fixture via XDG_DATA_HOME.
The final qualification run `egui037-figcraft-final-qualification-ci-20261025-0453`
repeated that configuration successfully after the documentation reconciliation.
Both runs completed formatting, strict lint, workspace tests, doctests, translations,
layering, assets, WASM checks, and plugin fmt/lint/tests/release-WASM builds. Each log
contains 3,242 passing executions, zero failures and nine ignored across 70 summaries;
this includes repeated translation checks and is not a count of distinct regressions.
The main doctest stage selects zero tests. Three plugin bundles were hash-verified and
preserved for each configuration.

The no-bundled-font run includes two test fixture corrections: text-fit cases explicitly
scan installed fonts, and a concurrent collaboration test explicitly selects its foreground
document after both joins complete instead of depending on network completion order.
Independent review accepted these changes with the original layout, synchronization and
isolation assertions intact. All 17 field-save regressions remain covered, including
owner-bound deferred saves, same-pass document replacement and later valid edits/saves.
Incoming IME interruption, screenshot geometry admission, and document/window ownership
are implemented. Historical notes below describe the earlier planning stages.

The final review packet binds source, lock, logs, plugin outputs and commit ancestry.
Native visible GUI/browser workflows, native/custom-editor IME, paired performance,
current-source platform runtime and actual release installation/signing remain open.
The system-font result is not a claim that missing document fonts are interchangeable.
Human review and human merge are required; no publication is authorized by this document.
