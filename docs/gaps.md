# Where FigCraft falls short of Figma

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-17 · **Change:** minor (the 2026-10-17 landings: `.fig` import fidelity, budgets for large files, the import report, a `.fig` copy from Save as and paste from another design app (gap 1); menus, the Actions palette, editor fixes, Selection colors and smart-selection spacing (gap 4); catalog counts re-measured (gap 5); bounded canvas pan overscan; reviewed Russian catalog and integer plurals; `.fig` export: instance sizes, variables with modes, shared styles, variable-width strokes; sharing with a join code; text box sizes measured against documents re-saved by other software; live sessions per document; .fig export checked by opening written files in other software; experimental .fig export; keyboard shortcut parity suite and bindings; Simplified Chinese draft catalog; Latin American Spanish overlay; prototyping tail: gamepad triggers, widgets in prototypes, mode lookups; plugin directory on figcraft-server; Japanese and Korean draft catalogs and CJK UI fonts; plugin widgets and the plugin panel tail; French, German and Brazilian Portuguese draft catalogs; cursor chat and spotlight in live sessions; layers and assets panels behaviour suite and fixes; vector multi-edit and edit tail; dev resources links; extended collections tail; branch version history and finer list merges; comments and version history behaviour suite and fixes; canvas navigation, guides and grids behaviour suite and fixes; `.fig` import: editable vector networks, more layer kinds, variable prototype actions; `.fig` import, best effort: gap 1 down to its tail; Windows QA pass: everyday workflows on Windows 11, eight fixes incl. unsaved work kept after a crash or a forced quit; Display P3: gamut warning and soft-proof (true P3 blocked upstream); several windows; localization layer with a draft Spanish catalog; hosting figcraft-server: wss in the desktop app, server releases, deployment guide; plugin panels and settings; canvas fixes: boolean hit testing on the result, group drags, frame outlines and slices; code connect; stretch and scatter brushes, pen pressure; text on a path and pattern fills; team libraries on the server and swap library; branches: create, update from main, review and merge; animated GIF fills and TIFF images; Open recent, outlines for the selection and mask outlines: menu breadth 100%; extended collections, variables import/export, gradient stop and variant bindings: gap 14 down to its tail) · **Target:** Figma Design (desktop app 126.10.7)

Every known shortfall against Figma, one entry each, ranked by how much it stops a Figma user from
doing real work in FigCraft. Agents: pick work from the top unless the owner says otherwise. The
numbers behind the ranking are in [`target-app-parity.md`](target-app-parity.md). Hours are one
Opus 5.5 agent working sequentially, tests included, calibrated as described there. "Estimated"
everywhere unless marked measured.

## Ranked

No alpha blockers: the alpha gate in [`roadmap.md`](roadmap.md#alpha-gate) passes. Beta blockers are marked.

| # | Gap | Kind | Hours | Doc |
|---|---|---|---|---|
| 1 | `.fig` files: best-effort import with editable vector networks and stand-ins for non-design layers landed, then (2026-10-17) more fidelity on synthetic files, budgets for large files, an import report and paste from another design app; an experimental export, also as a copy from Save as (round trips through our own importer; written files opened in other software, where layers, fills, gradients, strokes, shadows, vectors, components and text draw as in FigCraft, instances keep their size and variables with modes and colour styles come across); real-file checks and the small tail left (**beta blocker** until checked on real files, gap 3) | File format | 5–10 | [file-format-parity](file-format-parity.md) |
| 2 | Multiplayer and sharing are a preview, without accounts (**beta blocker**) | Feature | 42–84 | [collaboration-parity](collaboration-parity.md) |
| 3 | Nothing checked against real Figma files | Fidelity | 15–30 | [target-app-parity](target-app-parity.md) |
| 4 | Interaction fidelity not compared tool by tool | UI/UX | 40–60 | [ui-parity](ui-parity.md) |
| 5 | Localization: English, reviewed Russian, plus unreviewed Spanish (and Latin American Spanish), French, German, Brazilian Portuguese, Japanese, Korean, Simplified Chinese, Indonesian and Czech drafts; Turkish and Swedish catalogs | Localization | 29–51 | [localization-parity](localization-parity.md) |
| 6 | Plugins: widgets take clicks only, a self-hosted directory but no public catalogue; Figma's plugins don't run | Ecosystem | 8–26 | [target-app-parity](target-app-parity.md) |
| 7 | CPU canvas where Figma uses the GPU | Hardware/performance | 30–60 | [hardware-parity](hardware-parity.md) |
| 8 | Platform QA: Linux and the browser left (Windows pass done 2026-10-11) | Stability/platforms | 10–20 | [hardware-parity](hardware-parity.md) |
| 9 | Figma Draw: pen pressure on macOS/Linux tablets (multi-edit, brushes, pressure, text on a path and pattern fills landed) | Feature | 1–3 | [target-app-parity](target-app-parity.md) |
| 10 | Team libraries: no team-wide lists or accounts; analytics are counts | Feature | 4–8 | [components-parity](components-parity.md) |
| 11 | Spell check | Feature | 4–8 | [target-app-parity](target-app-parity.md) |
| 12 | Video fills; HEIC images | File format | 5–10 | [file-format-parity](file-format-parity.md) |
| 13 | Vector tail: none left (boolean hit testing on the result landed 2026-10-11) | Feature | — | [target-app-parity](target-app-parity.md) |
| 14 | Variables: extended collections checked on real libraries (the tail landed 2026-10-12) | Feature | 0–1 | [components-parity](components-parity.md) |
| 15 | Prototyping: media triggers and actions; controllers on macOS, Linux and FreeBSD (controller buttons on key/gamepad triggers, widgets in prototypes landed) | Feature | 2–5 | [prototyping-parity](prototyping-parity.md) |
| 16 | Dev Mode: import lines inside generated layer trees; the editor extension is left to the MCP server (code connect, Dev Mode plugins and dev resources landed) | Feature | 0–1 | [dev-mode-parity](dev-mode-parity.md) |
| 17 | Branching: branches in live sessions, lists other than fills, strokes and effects merged whole (a branch's own version history and per-item list merges landed 2026-10-12) | Feature | 1–2 | [collaboration-parity](collaboration-parity.md) |
| 18 | Display P3 on screen: blocked upstream (egui-wgpu picks an 8-bit sRGB surface); gamut warning and soft-proof sRGB landed, as did several windows | Hardware | 4–8 | [hardware-parity](hardware-parity.md) |
| 19 | AI features | AI | 30–60 | [target-app-parity](target-app-parity.md) |
| 20 | Mobile apps (view, mirror, comment) | Platforms | 20–40 | [hardware-parity](hardware-parity.md) |
| 21 | Menu items without a command: none left (landed 2026-10-11) | Feature | — | [parity-checklist](parity-checklist.md) |
| 22 | Screen readers never run; browser has no accessibility tree | UI/UX | 4–8 | [ui-parity](ui-parity.md) |
| 23 | CMYK output intent for print PDFs | File format | 2–4 | [file-format-parity](file-format-parity.md) |

## Entries

### 1. `.fig` files: the import's tail
- **Landed (2026-10-11):** `.fig` files open (best effort): the zip, its deflate/zstd chunks and the kiwi schema are decoded generically and mapped by field name onto the REST importer, so pages, the layer tree, geometry, paints (styles resolved), strokes, effects, text with styled runs, vectors, auto layout (grid included), constraints, components, sets, instances (expanded from their main with overrides, derived layout and property values), styles, variables with modes and bindings, prototyping and image fills from the archive come across; anything else is a warning (`crates/format/src/fig`). Then (2026-10-11): vectors open as editable vector networks (per-point corner radius, handle mirroring, end caps; outlines when the network blob is missing or damaged), sticky notes, shapes with text, connectors, tables, code blocks, slides and similar layers import as the closest frames, text layers and vectors with a warning, and set-variable, set-variable-mode and conditional prototype actions come across. `file.open`,
  `file.import`, drag and drop, the CLI and MCP's `open_file` read `.fig`; the file is never saved
  over.
- **Landed (2026-10-12):** an experimental export: `file.export {format: "fig"}`, File ▸ Export… ▸
  .fig (experimental), `figcraft-cli export --format fig` and MCP's `export` write the whole document
  with a kiwi encoder mirroring the decoder and a schema of the fields written: the layer tree,
  geometry, paints with image bytes, strokes, effects, text with styled runs, vector networks, auto
  layout, constraints, components, sets, instances with their overrides and property values, and
  prototype interactions; shared style links, variables, comments, Dev Mode data and the rest are
  left out with a warning ([file-format-parity](file-format-parity.md#export-to-fig-experimental)).
  The import gained ellipse arcs, boolean operation types and component set descriptions. Then
  (2026-10-12): written files were checked by opening them in other software: layers, fills,
  gradients, strokes, shadows, vectors, components and text draw as in FigCraft, after the export
  learned to say that the document and its pages are visible and opaque, to number its schema as
  the format does, to write outlines, text settings, standard font style names and line height
  units. Then (2026-10-13): instances keep their own size (an override addressed by the main's id,
  derived data for their layers, a scale factor), variables with modes and bindings, shared styles
  and variable-width strokes are written and read back; checked there: resized and scaled instances keep their size and lay their layers out for it, variables with their modes and colour bindings resolve there (a frame set to another mode shows that mode's colours, also inside instances), colour, text, effect and grid styles are listed, and variable-width strokes that are the same on both sides of the path draw with their width; number, string and boolean bindings and bindings on effects, layout guides and gradient stops come across as the values they give but did not show as bound there (their entries now also name the layer field they set, not yet checked there), and strokes wider on one side of the path are written as filled outlines, no longer editable strokes there (the stroke itself is kept as plugin data and our own import puts it back; not yet checked there),
  like effects such as glass and noise, images and prototype interactions.
- **Landed (2026-10-17, community batches #727 and #728, then #745):** the import keeps an
  instance's own identity and the cached copies of library components a file carries, applies
  instance overrides by key and order, lets local text values and instance overrides win over a
  shared text style, and reads grid track sizes (fixed, content-sized and fill), slot content,
  labels of connectors saved with their drawn path (text and background, as a static snapshot:
  the ends aren't live), skip ink, paragraph options and hanging lists, list start numbers,
  vertical trim, per-point stroke joins, editable text on a path, image adjustments (the tint
  sign shared with the REST import) and images carried in the file's message; layers inside
  instances keep their own size and resizing as overrides (#731), and auto layout and text that
  record no sizing hug (#732). Large files open within budgets set from the file's own size (one
  value per compressed byte, 20M–128M values, lower ceilings on 32-bit targets; a 15.4 MB
  synthetic file with 300,000 layers opens whole). After a `.fig`, REST JSON or SVG import an
  import report lists what was left out or changed (translated where the catalog has the note;
  `file.open {data}` returns a `summary`), and imported file bytes stay out of the command
  journal. File ▸ Save as… to a `.fig` name writes an export copy and the document keeps its
  `.figcraft` path. Layers copied from another design app paste (`edit.pasteDesignLayers`, and
  Cmd/Ctrl+V when the clipboard's HTML holds a `.fig` canvas): read like a `.fig` file, with its
  warnings, size-checked (256 MB) before decoding.
- **Missing:** elbowed or curved connectors without a saved path (drawn straight), labels of
  connectors without a saved snapshot, table cells beyond their own layers, filled areas of a
  vector with their own fills (#634, held for an owner decision on the document model), variables
  bound to fields the editor can't bind and video fills aren't read; text lays out again with
  local fonts. Nothing has been checked against real-world files yet (gap 3): the 2026-10-17
  changes were tested on files built in code, and paste from another design app on clipboard HTML
  made from our own export, never on a real clipboard. The export was checked in other software
  on our example documents and small test files only.
- **Evidence:** fixtures generated in code (`crates/format/src/fig/tests.rs` and `tail_tests.rs`: kiwi
  round trips, every mapped feature, vector network blobs, truncation, bit flips and junk never panic); `crates/engine/src/fig_import_tests.rs`.
  2026-10-17: `crates/format/src/fig/{connector_label,slot,source_identity,text_style,text_list,text_paragraph,text_path,text_skip_ink,image_filter,auto_layout_sizing}_tests.rs`,
  `map_connector_snapshot_budget_tests.rs`, `crates/engine/src/{save_format,design_paste,instance_resizing}_tests.rs`,
  `crates/ui-egui/src/{import_report,save_as}_tests.rs`.
  Export: 60 generated documents round trip through the importer layer by layer, written files
  decode with the generic decoder and list parents first, the encoder agrees with the decoder,
  junk documents never panic (`crates/format/src/fig/export_tests.rs`,
  `crates/engine/src/fig_export_tests.rs`).
- **Impact:** existing work can be brought across, but only real files will show how much of it.
- **Estimate:** 5–10 h for the read tail (real-file fixes, table cells, connectors without a saved path); checking the export against real files a further 5–15 h (with gap 3). Unchanged on 2026-10-17: most of the read tail is real-file fixes, which nothing has measured yet.

### 2. Multiplayer and sharing are a preview, without accounts
- **Missing:** accounts, teams and projects, per-person permissions, audio,
  shared comments and version history. Comments and versions are local to the file.
- **Evidence:** the core landed in #103 (`crates/collab`: client-scoped ids, change sets, server
  ordering and validation, client reconciliation, binary messages, randomized convergence tests over
  real commands), and live sessions run over the network (`crates/collab-net`: hosting from the
  app, invite links that open the app, document sync between apps, live cursors, selections,
  text carets, avatars and follow, cursor chat and spotlight, and the browser build joins) with per-person undo, refused
  edits rolled back with a reason, reconnects that resume and send what was made offline, and
  styles, variables and comments that several people create at once all kept; sessions are per
  document (each open file has its own or none, several at once, closing a file ends only its
  own); files are shared with a short join code (online through the server in Preferences, or on
  the local network with a code that carries the host's address) and joined by typing it; the app
  remembers the files it put online and lists them on the home screen (open again as owner, stop
  and start sharing, rename, remove, forget), per computer; a standalone
  `figcraft-server` keeps many files (snapshot plus log, compacted; restarts lose nothing
  confirmed), takes files published from the app and hands out share links that let people edit
  or only view; the desktop app and the browser reach it over `wss://` through a TLS reverse
  proxy, and releases ship it with a deployment guide (`docs/server.md`); server message limits
  follow `max_file_mb`, so files above 64 MiB publish and open (#705), a damaged generation field
  no longer drops a log's flushed edits (#687), and the browser build joins from a `#join=` link
  (#712: unit-tested, not run in a browser). But there are no accounts (a link is the access)
  (`docs/collab-protocol.md`).
- **Impact:** Figma's defining feature; most teams use it every day. Solo designers can work
  without it.
- **Estimate:** 42–84 h: accounts, teams and per-person permissions, shared comments and versions
  (numbers from before the server landed; re-measured next).

### 3. Nothing checked against real Figma files
- **Missing:** a corpus of real documents (design systems, app screens, prototypes) imported and
  compared with Figma's own PNG/SVG/PDF exports.
- **Evidence:** every corpus in the repo is synthetic (`crates/svg/tests/corpus`, the synthetic
  design system in `crates/engine/src/synthetic_ds.rs`). Scores were self-graded by the agents that
  built the features.
- **Impact:** unknown fidelity in auto layout sizing, text metrics, effects and component
  resolution on real files; the numbers in this repo can't be trusted beyond "estimated" until it
  exists.
- **Measured so far:** text box sizes and auto layout results of our own example documents,
  against the same documents re-saved by other software (which lays text out again). Three text
  rules explained every difference and are applied (whole-pixel line heights, advances on a
  1024-per-em grid, letter spacing between glyphs only): all 230 self-sizing text boxes in 7
  documents now come out to the pixel (24 of 131 before, in the design-system example), and 208 of
  its 231 other layers match in size and position (115 before). The other 23 hang on one grid
  auto layout frame whose row positions came back as written while its cells got shorter. One font
  family (Inter), left-aligned text; real files, other fonts, effects and component resolution are
  still unmeasured (`crates/text/src/metrics_tests.rs`, and a local check over such documents in
  `crates/engine/src/text_metrics_local_tests.rs`).
- **Estimate:** 15–30 h once `.fig` or REST import of real files is available; needs files whose
  licence allows use (owner to supply or approve; never committed if not redistributable, see
  craftrules `standards/test-corpora.md`).

### 4. Interaction fidelity not compared tool by tool
- **Missing:** a side-by-side pass of every tool and panel: cursors, modifier keys while dragging,
  hover states, popovers, menus, snapping thresholds.
- **Landed (2026-10-11):** behaviour suites written from the public documentation for auto layout,
  selection and transform, text, and canvas navigation with rulers, guides, layout grids, outline
  mode and measuring, then (2026-10-12) comments and version history, and the layers and Assets
  panels (`crates/engine/src/*_behaviour_tests.rs`, `crates/ui-egui/src/layers_behaviour_tests.rs`),
  each with its fixes; then keyboard shortcuts per platform (`shortcut_parity_tests.rs`: 129
  documented rows, 126 bound, 1 moved off a documented conflict, 2 missing, 48 undocumented
  bindings pinned), with 16 rows newly bound, Control suspending snapping on macOS and shifted
  symbols on other keyboard layouts; then the everyday partials of
  [`ui-parity.md`](ui-parity.md): number fields, outline mode's options, Hide when publishing, a
  cursor for every tool and modifier, panel widths, dark pages (`field_tests`, `cursor_tests`,
  `cursors_tests`, `outline_options_tests`, `hide_publishing_tests`, `panel_width_tests`,
  `theme_pages_tests`).
- **Landed (2026-10-17, community batches #733, #735, #736 and #729):** submenus open after a
  short hover, keep their place while the pointer heads for them (a safe triangle) and reopen
  closed (`menu_aim_tests`; timings tuned by feel, not measured); the Actions palette has tabs,
  ranked search over every command, a keyboard highlight and recents (`palette_tests`); menu items
  that need a value ask for it; live font preview on hover; the colour picker opens on a new fill
  or stroke; Shift-drag on a selected layer moves the selection; a double-click near a thin stroke
  enters vector edit; points dropped on a point merge; a draggable divider between Pages and
  Layers; Paste over and Paste properties from the keyboard; then (#753) Selection colors for mixed
  selections and smart-selection spacing fields and canvas handles (no reorder rings). Each was checked in the app (on
  Windows; the submenus by their author), none side by side with Figma, none on macOS.
- **Evidence:** ROADMAP item 7 ("Left: tool by tool, panel by panel against the incumbent").
- **Impact:** Figma users notice the feel within minutes; it decides whether they stay.
- **Estimate:** 40–60 h. Detail in [`ui-parity.md`](ui-parity.md).

### 5. Localization: English, reviewed Russian, plus unreviewed drafts
- **Missing:** native reviews of the Spanish, French, German, Brazilian Portuguese, Japanese,
  Korean, Simplified Chinese, Indonesian and Czech drafts and the Latin American Spanish
  overlay, the strings added since each catalog was drafted (1–5% of each), Hindi, Arabic and
  Vietnamese, localized engine messages and number formatting; CJK UI text on the web without the
  craft-fonts CJK build.
- **Evidence (measured from source):** the layer landed 2026-10-11: catalogs in
  `crates/ui-egui/locales/` (measured 2026-10-17: 1,680 UI keys plus 561 command labels and 31
  menu wordings, 2,272 entries), Preferences ▸ Language, `pref.language`; `en.toml` is complete,
  Spanish is held to completeness by the check, and the French, German, Brazilian Portuguese,
  Japanese, Korean and Simplified Chinese machine drafts cover 95–99% (strings added since they
  were drafted show English), with `es-419.toml` a Latin American overlay listing only what
  differs from Spanish. Russian (reviewed and approved by VanTan, one/few/many integer plurals)
  covers 98.8%; Indonesian and Czech drafts, a Turkish catalog whose header says a native speaker
  reviewed it, and a Swedish catalog (with its command labels since 2026-10-17) cover 98.7–99.6% ([localization-parity](localization-parity.md#languages)).
  CJK UI text uses an installed CJK face the desktop app loads when something on screen needs it
  (`crates/ui-egui/src/cjk.rs`). Engine error messages and Dev Mode values stay English.
- **Impact:** non-English designers get machine drafts nobody has reviewed yet, and English
  errors.
- **Estimate:** 3–7 h per language plus native review, 4–6 h for engine messages and numbers;
  29–51 h for the key languages left and the drafts' follow-ups (kept on 2026-10-17: the
  Indonesian draft takes 2–3 h off, about what bringing the drafts that slipped to 95% back to
  every string adds). Detail in
  [`localization-parity.md`](localization-parity.md); how to add one in
  [`translating.md`](translating.md).

### 6. Plugins: widgets take clicks only, a self-hosted directory but no public catalogue; Figma's plugins don't run
- **Landed (#101):** sandboxed WebAssembly plugins (wasmi, metered, memory cap, no WASI, also in
  the browser) acting through the command registry, one undo step per run, manifest permissions,
  Plugins menu, Run last plugin, Manage plugins, `plugin.*` commands, a Rust SDK and two examples
  ([`plugins.md`](plugins.md)). Plugin panels and settings (ABI v2, 2026-10-11): a plugin shows a
  panel of fields and buttons drawn with FigCraft's widgets and is called back for each button or
  changed value (one undo step per callback, same sandbox limits), driven headless with
  `plugin.ui` / `plugin.event` / `plugin.close`; per-plugin settings kept with the preferences.
  Widgets (plugin API 3, 2026-10-12): a plugin draws a layer of its own on the canvas (Plugins ▸
  Widgets, `widget.insert`), validated and built with locked layers inside, called back for clicks
  on the parts it names (one undo step each, a fresh metered sandbox, the state kept in the layer
  so it saves, copies, undoes, exports and syncs), `widget.click` / `widget.list` for agents, a
  Counter example; panels of several plugins side by side; the colour picker on plugin colour
  fields. A plugin directory on `figcraft-server` (2026-10-12): bundles published with the server
  key and checked in full (`figcraft-cli plugin publish`), browsed, installed after a permissions
  confirmation and updated from Plugins ▸ Manage plugins ▸ Browse, downloads checked against their
  SHA-256, `plugin.directory.*` for scripts and MCP, moderation with `figcraft-server plugins`.
- **Missing:** a public catalogue and Community files, publisher accounts and signatures, ratings
  and install counts, background update checks, the directory in the browser build; widgets take only clicks (no
  text input, property menus, hover or timers) and draw only frames, text, shapes and images;
  no images or sliders in panels. Figma's own plugins are JavaScript against Figma's plugin API and
  don't run.
- **Impact:** teams that depend on specific Figma plugins (icons, content, accessibility checks)
  can't bring them. FigCraft's CLI, control channel and MCP server cover agent automation better
  than Figma does.
- **Estimate:** 8–26 h for the missing pieces of FigCraft's own API. Running Figma's plugins
  needs a JS engine and an API shim: **owner decision needed**.

### 7. CPU canvas where Figma uses the GPU
- **Missing:** GPU rendering of the canvas. FigCraft renders with tiny-skia on the CPU (banded,
  parallel, retained) and shows the image through wgpu. A new zoom can't reuse the retained
  image; when renders from scratch take a frame or more, the whole view is previewed with a
  sixteenth of the pixels first, then refined.
- **Landed (2026-10-15):** a bounded render margin is reused during panning and refilled before
  the visible view reaches its edge. The extra rendered area is capped at 1.6× visible pixels;
  this reduces blank strips without changing the CPU renderer.
- **Evidence (measured, `cargo xtask bench`):** a 50k-layer page ~8 ms zoomed out with levels of
  detail, single-layer edits at 10k layers ~0.2 ms; full 1440×900 renders 7–15 ms at 10k layers.
  Heavy effects (glass, background blur, noise) and 4K/5K canvases at 2x are where CPU time grows.
- **Impact:** large files and effect-heavy designs; battery on laptops.
- **Estimate:** 30–60 h for a GPU path (vello or wgpu shaders) held to the CPU renderer as the
  oracle.

### 8. Platform QA: Linux and the browser left
- **Missing:** regular runs on Linux and the browser; real screen readers; a repeat of the Windows
  pass on other machines (scale factors above 100%, pen tablets, file drops from Explorer).
- **Evidence:** CI tests on Linux only (`.github/workflows/ci.yml`); release builds exist for every
  platform (v0.2.0) but no issue has been filed yet. Windows: QA pass done 2026-10-11, findings in
  PR (Windows 11, the desktop app driven over the control channel, the same workflow through MCP):
  eight bugs fixed, among them unsaved changes lost when the app was killed or crashed,
  Ctrl+arrows and Ctrl+Backspace acting on the whole line in text, scrubbed fields snapping back
  and skewed linear gradients. Linux and the BSDs (2026-10-17, #693): clipboard reads run on a
  helper thread that gives up after 2 s, and images copied in other apps paste under Wayland
  through the data-control protocol; compiled in CI, not run on a Linux desktop here. Wayland
  compositors without server-side decorations (GNOME) give the window winit's Adwaita controls
  (#592: checked by its author on GNOME; KDE not run).
- **Impact:** unknown bugs on the platforms where FigCraft is most attractive (Linux has no Figma
  desktop app).
- **Estimate:** 10–20 h, partly human (hardware).

### 9. Figma Draw: tablet pressure on macOS and Linux
- **Missing:** pen pressure on macOS and Linux tablets (winit reports none there).
- **Landed (2026-10-12):** multi-edit: several vector layers in edit mode at once (Enter or
  double-click with them selected, `vector.edit.begin {ids}`); points selected, marqueed, dragged,
  nudged, deleted, mirrored and rounded across layers in one undo step; the pen, bend, paint bucket,
  cut and eraser work in the layer last pressed (`vector_multi_behaviour_tests`).
- **Landed:** text on a path (`text.onPath`, `text.pathOptions`, `text.removeFromPath`; the
  Design panel's Typography section; canvas, PNG, SVG and PDF as outlines; SVG `<textPath>` imports
  as text on a path), pattern fills (`PaintKind::Pattern`, `fill.pattern`, the colour picker's
  Pattern tab; SVG `<pattern>`, PDF tiling patterns), stretch and scatter brushes (`stroke.brush`,
  eight presets drawn in code; canvas, PNG, SVG, PDF and outline stroke) and pen pressure for the
  Pencil (Windows pens and browser styluses; recorded as the stroke's width profile, which brushes
  follow).
- **Evidence:** winit reports no pressure on macOS and Linux tablets.
- **Impact:** illustration-flavoured work moved into Figma Draw in 2025.
- **Estimate:** 1–3 h once winit (or a platform tablet API behind it) reports pressure there.

### 10. Team libraries: no team-wide lists or accounts; analytics are counts
- **Missing:** libraries listed for a whole team or organization (each file enables a library by
  its link), accounts and per-person library permissions, analytics beyond instance counts per
  component and file (insertions and detaches over time), update notices pushed by the server
  (apps ask every 30 seconds while the Assets panel shows).
- **Evidence:** a file's library is published to `figcraft-server` under the file and enabled in
  other files by a share link, with per-asset versions, updates with overrides kept, usage counts
  and swapping a whole library by names ([`server.md`](server.md#team-libraries),
  [`components-parity.md`](components-parity.md)).
- **Impact:** design-system teams.
- **Estimate:** 4–8 h, the accounts part after gap 2.

### 11. Spell check
- **Missing:** spelling underline and suggestions while editing text.
- **Evidence:** no spell-check code in the repo.
- **Estimate:** 4–8 h with a permissively licensed dictionary and checker (provenance per
  `ATTRIBUTION.md`).

### 12. Video fills; HEIC images
- **Landed (2026-10-11):** animated GIF fills (the file kept as is, the first frame on the canvas
  and in exports, played in the presenter and the preview) and TIFF images.
- **Missing:** Figma takes JPG, PNG, HEIC, WebP, GIF and TIFF images and MP4, MOV and WebM video
  (help center, checked 2026-10-10). FigCraft decodes PNG, JPEG, WebP, GIF, TIFF and BMP; HEIC and
  video need decoders that are C libraries (libheif, FFmpeg) or each platform's media framework.
- **Estimate:** 5–10 h (video playback in the presenter is most of it; it can reuse the GIF clock).

### 13. Vector tail: none left
- **Landed (2026-10-12):** several Bezier handles selected with Shift-click and dragged together,
  sharp points beside curved segments rounded, a stroke cap and join per point (canvas, PNG, SVG,
  PDF, outline stroke), the Pen outside edit mode connecting to its own points and continuing a
  selected open path. Left: corner smoothing beyond rectangles and frames, the transform box around
  points of several layers, per-point caps and joins on dashed, variable-width and brush strokes.
- **Landed (2026-10-11):** boolean groups are clicked where their result is (a click in a
  subtracted hole reaches what's behind; with the group or an operand selected its operands are
  hit, so each can still be picked; "Select layer" lists the group only where its result is), live
  boolean editing (operands selected by double-click or Cmd/Ctrl-click;
  moving, resizing or editing an operand's points redraws the result during the drag, with every
  operand outlined; per-operand operations from the boolean controls and `node.set`; a drag frame
  recombines only the operands from the edited one up), Design panel Arc fields (Start, Sweep,
  Ratio; Mixed across ellipses; bindable), the shape builder on strokes and open paths (strokes take
  part as their outlined areas), one-sided variable widths (Alt-drag a width handle; canvas, PNG,
  SVG, PDF and outline stroke).
- **Estimate:** none left (vector QA with real files is part of gap 3).

### 14. Variables: extended collections on real libraries
- **Landed:** extended collections (`library.extendCollection`: a library collection's modes and
  variables in a collection of this file, values overridden per mode with reset, following library
  updates, variables of its own), variables import/export (`variable.export` / `variable.import`:
  the REST local variables JSON, merged by name; CSS custom properties), gradient stops and variant
  properties bound to variables.
- **Landed (2026-10-12):** variables a library adds to a collection this file extends are a library
  update of their own (`library.updates` lists them by name, without an `id` until they have a
  copy) and arrive inheriting when accepted; a library collection that is itself an extended
  collection (a brand theme extending a core one) can be extended again, its values flowing down
  the chain as each file accepts updates, every level's overrides kept.
- **Missing:** checked with synthetic files only; extending a collection of the same file isn't
  offered (extending takes library collections).
- **Estimate:** 0–1 h.

### 15. Prototyping: media triggers and actions, controllers off Windows
- **Landed (2026-10-12):** key/gamepad triggers take controller buttons (`Gamepad A` … `Gamepad
  Home`, the standard layout's 17 buttons; a Controller menu beside the key field; REST files'
  controller triggers import); the player reads controllers on Windows and in the browser (Gamepad
  API) and takes `ui.input {gamepad}` on the control channel; widgets react to clicks in a
  presented prototype, on the prototype's own copy; `proto.evaluate` computes expressions (mode
  lookups included) where a layer's interaction would run (`crates/engine/src/proto_tail_tests.rs`,
  `crates/ui-egui/src/present_tail_tests.rs`, `crates/format/src/rest/gamepad_tests.rs`).
- **Missing:** video triggers (on media hit/end) and play/pause/mute/seek actions (the REST
  importer warns and skips them, `crates/format/src/rest/proto.rs`); controllers on macOS, Linux
  and FreeBSD (the usual Rust controller library links libudev on Linux; a pure-Rust reader per
  platform, or an owner decision on that dependency); PlayStation and Switch button labels.
- **Estimate:** 2–5 h, the media part after video fills (gap 12).

### 16. Dev Mode: the tail after dev resources
- **Landed (2026-10-11):** code connect: a component or set links to its code component per
  language (React, SwiftUI, Compose): an import path, the name and a template whose `{property}`
  and `{property ? "a" : "b"}` placeholders take the instance's values, escaped per language and
  never executed. `inspect.code`, `inspect.componentCode` and the Inspect panel show that snippet
  instead of generated code (inside generated layer trees too); the panel's Connect to code edits
  it with a live preview. Links are stored on the component (saved, synced, undoable) and travel
  as `figcraft.connect.json` (`codeConnect.export` / `codeConnect.import`, validated and capped)
  so code owners can version them. Plugins whose manifest asks only to read run from the Inspect
  tab.
- **Landed (2026-10-12):** dev resources: links for developers (a URL and a name) on any layer
  (`devResource.add` / `edit` / `remove` / `list`, undoable, saved and synced). An instance also
  lists its main component's links (and its set's) read-only with where they come from; layers
  inside an instance show their main layer's. URLs are checked as web links (http or https, a
  host, no spaces, capped) and never fetched: a link added without a name is named after its URL,
  and a link opens only when clicked. The Inspect tab lists, adds, edits and removes them;
  `get_node` / `inspect_document` return them as `devResources`.
- **Missing:** import lines for connected instances inside a
  generated layer tree (only the selected layer's own import is shown); publishing mappings from
  the code repository with a CLI. An editor extension is out of scope: the MCP server already
  serves design data, code and the connected snippets to agents and editors.
- **Estimate:** 0–1 h.

### 17. Branching and merging
- **Landed (2026-10-11):** branches of a file (`branch.*`): File ▸ Create branch… (or the Branches
  list in version history) copies the file into a branch tab that remembers the file as it was (its
  base) and makes its ids in its own id space; edits on the branch are saved with the main file.
  Review and merge (the banner on a branch tab) shows the main file and the branch side by side or
  as an overlay, what changed on each side since the base (layers with their property changes as
  Inspect shows them; styles, variables, comments) and the conflicts: the same property changed on
  both sides, or a layer deleted on one side while the other edited it or what's inside it. Each
  conflict is resolved for the branch or main; nothing conflicting merges until it is. Merging is
  one undo step on the main file and archives the branch; Update from main brings main's changes
  into the branch the same way; branches can be archived without merging. Three-way merge over the
  collaboration change sets (`figcraft_collab::merge`), fuzz-tested with random real commands.
- **Landed (2026-10-12):** a branch's own version history: versions saved on a branch tab (by hand
  or autosaved) go into the branch, not the main file, and are viewed, restored (one undo step on
  the branch), renamed, deleted and duplicated there; they are saved in the file with the branch.
  Merging adds a version to the main file's history (Merged "name"); the merged or archived
  branch keeps its own history and opens with it. Fills, strokes and effects merge item by item:
  different items changed on each side, or items added at the end on one side, merge without a
  conflict; only the same item changed both ways, an item removed on one side, or different items
  added on both sides conflict (as the whole list).
- **Missing:** branches of a file in a live session are refused; other lists (text runs, export
  settings, guides…) still conflict as a whole; the branch history has no entries of its own for
  its creation or updates from main (only saved versions, autosaves and the merge on main).
- **Estimate:** 1–2 h.

### 18. Display P3 on screen (several windows landed)
- **Landed (2026-10-11):** several windows: File ▸ New window, a tab's Open in new window or a tab
  dragged off the strip; each window has its own tabs, panels and canvas and draws only when it
  needs to; closing one asks about unsaved changes; `window.new`, `window.list`, `window.focus` and
  `window` on any control request.
- **Missing:** wide-gamut colour on the canvas (egui draws sRGB, so P3 colours outside sRGB clip on
  screen; exports keep them); a screen-reader tree in windows other than the main one (eframe 0.36
  gives only the main window one); window positions restored at the next start.
- **Estimate:** 4–8 h.

### 19. AI features
- **Missing:** Figma AI (first draft, rename layers, remove background, image generation and
  editing, visual search, Make).
- **Estimate:** 30–60 h with openly licensed local models. **Owner decision needed.**

### 20. Mobile apps
- **Missing:** Figma's iOS and Android apps view and comment on files and mirror prototypes.
  FigCraft's web build runs in mobile browsers but is not designed for touch.
- **Estimate:** 20–40 h for a touch-first viewer and mirror.

### 21. Menu items without a command (measured): none left
- **Landed (2026-10-11):** `file.openRecent` (File ▸ Open recent: recent files kept in the
  preferences, at most 20, greyed when the file is gone and dropped when picked, Clear recent),
  `view.outlinesSelection` (outline mode for the selected layers only) and `view.maskOutlines`;
  the plugin pair landed in #101. Every item in the menu catalog has a command
  ([`parity-checklist.md`](parity-checklist.md)).
- **Estimate:** none left.

### 22. Screen readers never run; browser has no accessibility tree
- **Evidence:** AccessKit names and keyboard navigation landed (#96), checked by tests only;
  eframe's web backend doesn't send the tree yet.
- **Estimate:** 4–8 h plus human QA with VoiceOver, NVDA, Narrator and Orca.

### 23. CMYK output intent for print PDFs
- **Evidence:** CMYK conversion is a documented naive formula; no CMYK ICC profile can ship in the
  repository (licensing).
- **Estimate:** 2–4 h once an openly licensed profile is found.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | minor | The 2026-10-17 landings. Gap 1: a new Landed entry (community batches #727 and #728, then #745: instance identity and cached library components, overrides by key, text style precedence, grid track sizes, slot content, labels of connectors saved with their path, skip ink, paragraph options, list start numbers, vertical trim, per-point stroke joins, text on a path, image adjustments, images in the message, instance layers' size and resizing overrides, Hug where nothing is recorded; budgets from the file's size; the import report; a `.fig` copy from Save as; paste from another design app), its Missing list cut to what is still left, and the wrong "whether other apps open its files is unknown" replaced by what was checked; all on synthetic files, so the beta blocker and hours stand. Gap 2: server limits, log recovery and web join links in the evidence. Gap 4: menus, the Actions palette and the editor fixes (#733, #735, #736, #729), then Selection colors and smart-selection spacing (#753) landed, none compared side by side. Gap 5: catalog counts re-measured (2,272 entries; drafts at 95–99%; Indonesian, Czech, Turkish and Swedish listed). Gap 8: the Linux and BSD clipboard changes and Wayland window controls, not run there by the maintainers. Ranking and hours unchanged |
| 2026-10-15 | minor | Gap 7: bounded canvas overscan for smoother panning. Estimates and ranking unchanged. |
| 2026-10-14 | minor | Russian catalog (M9), reviewed and approved by VanTan: all 2,165 current entries, Русский in Preferences and regional ru tags. Russian integer plurals use one/few/many; contributor display name VanTan added. Existing estimates unchanged. |
| 2026-10-13 | minor | Gap 1: the experimental `.fig` export writes instance sizes (resized, filling an auto layout, scaled), variables with modes and bindings, shared styles and variable-width strokes, and the import reads them back. Checked by opening written files in other software: resized and scaled instances keep their size and lay their layers out for it, variables with their modes and colour bindings resolve there (a frame set to another mode shows that mode's colours, also inside instances), colour, text, effect and grid styles are listed, and variable-width strokes that are the same on both sides of the path draw with their width; number, string and boolean bindings and bindings on effects, layout guides and gradient stops come across as the values they give but did not show as bound there (their entries now also name the layer field they set, not yet checked there), and strokes wider on one side of the path are written as filled outlines, no longer editable strokes there (the stroke itself is kept as plugin data and our own import puts it back; not yet checked there). Hours and the ranking unchanged |
| 2026-10-12 | minor | Gap 2: your online files landed (the app records the files it put on a server with their owner tokens, Home ▸ Online lists them and opens them again as their owner, with sharing, rename, removal and forget from the list; one request per server tells which are still there; tabs mark files in a live session). What is left is unchanged: accounts, teams and per-person permissions, shared comments and versions. Hours unchanged |
| 2026-10-12 | minor | Gap 2: sharing with a join code (Share ▸ Online or Local network, one Start sharing button, a short code to give others, one join field for a code or a link; the server names links with codes and limits wrong ones) added to the landed list; usability, so the estimate and ranking are unchanged |
| 2026-10-12 | minor | Gap 3: first measured fidelity data. Text box sizes and auto layout results of our example documents were compared with the same documents re-saved by other software; three text rules (whole-pixel line heights, advances on a 1024-per-em grid, letter spacing between glyphs only) were found and applied, and all 230 self-sizing text boxes in 7 documents now match to the pixel (one font family). Real files are still unchecked: hours and the ranking unchanged |
| 2026-10-12 | minor | Gap 2: live sessions are per document (each open file has its own session or none, several run at once, the window and the `collab.*` commands follow the file in front, closing a file ends its session only) added to the landed list; a fix, so the estimate and ranking are unchanged |
| 2026-10-12 | minor | Gap 1: the experimental `.fig` export now opens elsewhere: written files were checked by opening them in other software: layers, fills, gradients, strokes, shadows, vectors, components and text draw as in FigCraft; effects such as glass and noise, images and prototype interactions are still unchecked there, and variables are not written. Hours and the ranking unchanged |
| 2026-10-12 | minor | Gap 1: an experimental `.fig` export landed (the whole document, from the same public format notes as the import; checked only by round trips through our own importer); export hours 15–30 h → 5–15 h (checks against real files); gap 1's read tail (5–10 h) and the ranking unchanged |
| 2026-10-12 | minor | Gap 4: keyboard shortcut parity suite per platform (129 documented rows: 126 bound, 1 moved, 2 missing; 48 undocumented bindings pinned) added to the landed list; estimate and ranking unchanged |
| 2026-10-12 | minor | Simplified Chinese draft catalog (gap 5): `zh-Hans.toml` translates every UI string (1,371 keys; plurals give only `other`, and `plural_one` never picks "one"), command label (524) and menu wording (29), machine-drafted in a concise UI style with full-width punctuation and marked as a draft awaiting native review; Preferences ▸ Language lists 简体中文, System maps zh, zh-CN, zh-SG and zh-Hans to it, and Traditional Chinese (zh-Hant, zh-TW, zh-HK, zh-MO) has no catalog and stays English (or the next OS language), never Simplified. A Chinese UI draws Han in a Chinese face first (Microsoft YaHei, PingFang SC, Noto Sans CJK SC), ahead of the craft-fonts build's Japanese BIZ UDPGothic, and changing the UI language lays the CJK faces out again (switching from Japanese to Chinese no longer keeps the Japanese face in front). String counts re-measured: 1,371 UI keys, 524 command labels and 29 menu wordings (1,924 entries); the French, German, Brazilian Portuguese, Japanese and Korean drafts have 1,899 of them (the 25 newer strings read English). Gap 5 31–54 h → 29–51 h, rank unchanged |
| 2026-10-12 | minor | Latin American Spanish overlay (gap 5): `es-419.toml` lists only the 87 entries (61 UI strings, 26 command labels) whose wording differs in Latin American usage (agregar for añadir, presionar for pulsar, configuración for settings, Enter for the Intro key, control for a gamepad, resorte for a spring) and reads `es.toml`, then English, for the rest (a regional catalog overlays its language's); same tú register as Spanish; a draft awaiting native review. Preferences ▸ Language lists Español (Latinoamérica); System maps es-419, es-MX, es-AR, es-CO and the other Latin American and US Spanish tags to it, es-ES and es to Spanish. Gap 5 32–55 h → 31–54 h, rank unchanged |
| 2026-10-12 | minor | Gap 15: controller buttons on key/gamepad triggers (Windows and the browser), widgets reacting in prototypes and `proto.evaluate` landed; gap 15 now media plus controllers off Windows, 3–6 h → 2–5 h; ranking unchanged |
| 2026-10-12 | minor | Plugin directory on `figcraft-server` (publish with the server key, browse, install after a permissions confirmation, hash-checked downloads, updates by version, operator moderation) added to gap 6's landed list; gap 6 10–30 h → 8–26 h; ranking unchanged |
| 2026-10-12 | minor | Japanese and Korean draft catalogs and CJK UI fonts (gap 5): `ja.toml` and `ko.toml` translate every UI string (1,351 keys; plurals give only `other`, and `plural_one` never picks "one" in either), command label (519) and menu wording (29), machine-drafted and marked as drafts awaiting native review; Preferences ▸ Language lists 日本語 and 한국어, System picks ja* and ko*. CJK UI text renders: once the UI language, the open file's names or comments, typed text or the language picker needs Japanese kana, Hangul or Han, the desktop app appends one installed face per script to egui's fallbacks (Windows Yu Gothic UI / Meiryo / Malgun Gothic / Microsoft YaHei, macOS Hiragino Sans / Apple SD Gothic Neo / PingFang SC, Linux Noto Sans CJK; the craft-fonts build's BIZ UDPGothic first), reading only that face's file and sharing its bytes with egui (English users load none: startup and memory unchanged); the web shows Japanese and Korean only when built with craft-fonts' Noto Sans CJK (`FIGCRAFT_EMBED_CJK`), else it keeps them out of the picker and System. Gap 5 40–65 h → 32–55 h, rank unchanged |
| 2026-10-12 | minor | Plugin widgets (API 3) and the panel tail (panels side by side, colour picker on plugin fields) added to gap 6's landed list; gap 6 now 10–30 h; ranking unchanged |
| 2026-10-12 | minor | French, German and Brazilian Portuguese draft catalogs (gap 5): every UI string, command label and menu wording (1,899 entries) machine-drafted in each and marked as drafts awaiting native review, falling back to English per key (new strings need only English and Spanish); 0 counts as singular in French and Brazilian Portuguese; System picks the first OS language with a catalog (pt-PT and pt read pt-BR); long translations elide with a tooltip in dropdowns, two-column labels and the home cards. Gap 5 45–75 h → 40–65 h, rank unchanged |
| 2026-10-12 | minor | Cursor chat and spotlight in live sessions (gap 2): `/` opens a chat bubble at your pointer, a spotlight makes everyone follow your view; ephemeral presence the host validates and rate-limits. Gap 2 45–90 h → 42–84 h |
| 2026-10-12 | minor | Layers and Assets panels behaviour suite and fixes added to gap 4's landed list; ranking and hours unchanged |
| 2026-10-12 | minor | Vector multi-edit and the vector edit tail, with a behaviour suite written from the public documentation (5 tests, about 30 table rows and steps, and a junk-parameter sweep of every vector command in multi-edit mode): several vector layers are edited at once (Enter, double-click or `vector.edit.begin {ids}`; a press on another layer's point, handle or segment makes it the active layer; Shift-click, marquee and lasso select across layers; dragging, the arrows, Delete, delete and heal, mirroring, smoothing, corner radius and point caps and joins reach every layer's selected points in one undo step; the pen, bend, paint bucket, cut and eraser work in the active layer); several Bezier handles are selected with Shift-click and dragged together (`vector.edit.moveHandles`); a sharp point next to a curved segment rounds (the curve is cut where the arc meets it and keeps its shape); a point can have its own stroke cap and join (`vector.edit.strokeCap`, `vector.edit.strokeJoin`, the Point section of the Design panel; canvas, PNG, SVG and PDF, outline stroke); the Pen outside edit mode connects to one of its own points and ends the path, and continues a selected open path from an end point. Left: corner smoothing beyond rectangles, the transform box around points of several layers, per-point caps and joins on dashed, variable-width and brush strokes. Gap 9 2–5 h → 1–3 h (pen pressure on macOS and Linux tablets left, blocked on winit). Shapes & vector 98%/72% → 99%/73%, 2–4 h → 1–3 h; Figma Draw extras 80%/55% → 88%/60%, 2–5 h → 1–3 h (gap 9: pen pressure on macOS and Linux tablets left). Totals recomputed by the doc's method, which rows since the last recompute had left stale: the area table's weighted breadth is 83.7% (~83%; core 94.1%, ~94%) and ready 63.1% (~63%; core 72.1%, ~72%), hours 133–284 (core 38–83); Features (depth) 61% → 63%, 46–94 h → 34–76 h; ready for real work 52.6% (~52%); mainstream weekly areas 71.0% → 72.1% (× 0.9 × 0.9 × 0.85 = 49.7%, ~49%, rounded down); essentials unchanged (~57%); full parity ~306–595 h → ~294–572 h (on top of the rows merged since: ~301–587 h → ~289–564 h; area-table totals recomputed on the merged table and rounded down: breadth ~84%, ready ~63%, core ~94%/~72%, mainstream weekly areas 72.4%, ~49%) |
| 2026-10-12 | minor | Dev resources (gap 16): links for developers (a URL and a name) on any layer: `devResource.add` / `edit` / `remove` / `list`, undoable, saved and synced; an instance also lists its main component's (and that component's set's) links read-only with where they come from, and layers inside an instance show their main layer's; URLs are checked as web links (http or https, a host, no spaces, at most 2,048 characters) and never fetched (a link added without a name is named after its URL), opened only when clicked; the Inspect tab lists, adds, edits and removes them; `get_node` and `inspect_document` return them as `devResources`. The editor extension stays out of scope (the MCP server serves agents and editors). Dev Mode breadth 93% → 96%, ready 70% → 72%, 1–3 h → 0–1 h; feature breadth (~82%) and ready for real work (~51%) unchanged at this rounding; full parity ~304–593 h → ~303–591 h |
| 2026-10-12 | minor | Extended collections' tail (gap 14): variables a library adds to a collection this file extends now count as a library update of their own (listed by name without a copy yet) and arrive inheriting when accepted, instead of waiting for the library's next other update; a library collection that is itself an extended collection can be extended again, values flowing down the chain as each file accepts updates with every level's overrides kept (checked end to end). Styles & variables breadth 94% → 95%, ready unchanged (76%), 3–5 h → 2–4 h; gap 14 1–2 h → 0–1 h (real-library checks left); feature breadth (~82%) and ready for real work (~51%) unchanged at this rounding; full parity ~305–594 h → ~304–593 h |
| 2026-10-12 | minor | A branch's own version history and finer list merges (gap 17): versions saved or autosaved on a branch tab go into the branch (viewed, restored, renamed, deleted and duplicated there, saved in the file with it); merging adds a version to the main file's history and the merged or archived branch keeps its own; fills, strokes and effects merge item by item, conflicting only when the same item changed both ways, an item was removed on one side or both sides added different items. Comments, versions, find, misc breadth 85% → 87%, ready 60% → 61%, 2–5 h → 1–4 h; gap 17 1–3 h → 1–2 h; feature breadth (~82%) and ready for real work (~51%) unchanged at this rounding; full parity ~306–595 h → ~305–594 h |
| 2026-10-12 | minor | Comments and version history behaviour suite and fixes added to gap 4's landed list (gap 17 unchanged: a branch's own version history is still left); ranking and hours unchanged |
| 2026-10-11 | minor | `.fig` import's tail (gap 1): vectors open as editable vector networks (points, curved segments, filled areas with their winding, per-point corner radius and handle mirroring, end caps from the end points), falling back to outlines when the network is missing or damaged; sticky notes, shapes with text, connectors, tables, code blocks, slides and similar layers import as the closest frames, text layers and vectors with a warning instead of being dropped; set-variable, set-variable-mode and conditional prototype actions come across. New parsers fuzzed on fixtures generated in code. File formats 55% → 58%, 8–15 h → 5–10 h; import/export breadth 80% → 82%, ready 50% → 54%; feature and area totals and ready for real work (~49%) unchanged; beta ~173–310 h → ~170–305 h; full parity ~321–618 h → ~318–613 h |
| 2026-10-11 | minor | `.fig` import, best effort (gap 1): `.fig` files open (best effort): the zip, its deflate/zstd chunks and the kiwi schema are decoded generically and mapped by field name onto the REST importer, so pages, the layer tree, geometry, paints (styles resolved), strokes, effects, text with styled runs, vectors (as outlines), auto layout (grid included), constraints, components, sets, instances (expanded from their main with overrides, derived layout and property values), styles, variables with modes and bindings, prototyping and image fills from the archive come across; anything else is a warning. Gap 1 30–60 h → 8–15 h (vector networks, other layer kinds, real-file checks; export 15–30 h on top, as before) |
| 2026-10-11 | minor | Display P3 honest minimum (gap 18): the colour picker of a Display P3 file warns when a colour is outside sRGB (Outside sRGB, with what the canvas and exports do), and View ▸ Soft-proof sRGB (`view.proofSrgb`) stripes the canvas pixels sRGB can't show. Why the canvas still shows sRGB: wgpu 30 can present Display P3 or extended-range sRGB surfaces (Metal, DX12, Vulkan, WebGPU canvases), but eframe/egui-wgpu 0.36 configure an 8-bit sRGB surface with no option for the format or colour space, and egui's renderer writes sRGB-encoded UI colours; showing P3 needs those two upstream (or a patched egui-wgpu). Hardware 13–27 h → 12–26 h; gap 18 6–12 h → 5–11 h; percentages, feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~363–697 h → ~362–696 h |
| 2026-10-11 | minor | Several windows (gap 18): File ▸ New window, a tab's Open in new window or a tab dragged off the tab strip opens files in another window with its own tabs, panels and canvas, sharing the files, preferences, libraries and clipboard; each window draws only when it needs to (idle: ~0% CPU), closing one closes its files after asking about unsaved changes (the main window too, which takes over another window instead of quitting; the last window quits), and the control channel and MCP open, list, focus and capture windows (`window.new`, `window.list`, `window.focus`, `window` on any request); the screen-reader tree covers the main window only (eframe 0.36), so with a screen reader running files stay tabs; one window in the browser. Hardware 55% → 60%, 13–27 h → 10–21 h; gap 18 6–12 h → 4–8 h (P3 on screen left); feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~363–697 h → ~360–691 h |
| 2026-10-11 | minor | Localization layer landed (gap 5): per-language catalogs baked in at build time with English text as keys and fallback, placeholders and plurals, command labels by id, Preferences ▸ Language (System follows the OS or the browser) applied live and `pref.language`; English catalog and a complete Spanish draft awaiting native review. Localization 10% → 20%, 60–100 h → 45–75 h; ready for real work unchanged (~47%); full parity ~363–697 h → ~348–672 h |
| 2026-10-11 | minor | Hosting `figcraft-server`: the desktop app and `figcraft-cli` open `wss://` links (rustls with Mozilla's root certificates, always verified; `FIGCRAFT_CA_FILE` adds a private authority), releases ship the server for Linux x86_64 and aarch64, it stops gracefully on SIGTERM or Ctrl+C, and `docs/server.md` has a deployment guide (systemd unit, Caddy, nginx, Docker); percentages and hours unchanged until the next re-measure |
| 2026-10-11 | minor | Plugin panels and settings landed (gap 6, ABI v2; v1 plugins unchanged): a plugin shows a panel of notes, fields (text, number, checkbox, menu, list, colour) and buttons drawn with FigCraft's widgets; a button or a changed value calls it back with every value (same limits per callback, one undo step per callback that edits, rolled back on failure), its module staying loaded while the panel is open; `plugin.ui` / `plugin.event` / `plugin.close` drive panels headless; per-plugin settings (64 KB) kept with the preferences; the Swatch grid example gained a palette panel that remembers the last palette. Plugins & widgets breadth 35% → 45%, ready 20% → 28%, 25–55 h → 15–40 h; ecosystem 20% → 28%; feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~373–712 h → ~363–697 h |
| 2026-10-11 | minor | Canvas fixes (gap 13 closed): boolean groups are clicked where their result is (a click in a subtracted hole reaches what's behind; with the group or an operand selected its operands are hit, so each can still be picked), a layer dragged inside a group stays in the group (frames and auto layout still take other drops), View ▸ Frame outlines and View ▸ Slices draw on the canvas, and the idle tests wait for the render thread instead of counting frames. Shapes & vector 1–3 h → 1–2 h (vector QA with real files left); percentages, feature breadth (~79%), ready for real work (~47%) and full parity hours unchanged at this rounding |
| 2026-10-11 | minor | Code connect and Dev Mode plugins (gap 16): a component or set links to its code component per language (import path, name, a template with `{property}` and `{property ? "a" : "b"}` placeholders); `inspect.code`, `inspect.componentCode` and the Inspect panel show that snippet with the instance's values (also inside generated layer trees), edited in Connect to code with a live preview; links are saved, synced and undoable, and travel as `figcraft.connect.json` (`codeConnect.export` / `codeConnect.import`). Plugins that only read run from the Inspect tab. Dev Mode breadth 80% → 90%, ready 62% → 64%, 4–8 h → 1–3 h; feature breadth (~79%) and ready for real work (~47%) unchanged; full parity ~378–722 h → ~375–717 h |
| 2026-10-11 | minor | Stretch and scatter brushes and pen pressure landed (gap 9): eight brushes drawn in code (four stretched along the path, four scattered with spacing, size, rotation and scatter jitter from a seed) in the stroke settings, drawn everywhere strokes are and flattened by outline stroke; the Pencil records pen pressure as the stroke's width profile (Windows pens, browser styluses; macOS and Linux tablets report none) and can draw with a brush; Figma Draw extras breadth 45% → 65%, ready 30% → 45%, 10–18 h → 5–9 h; Strokes 2–4 h → 1–3 h; Hardware 50% → 55%, 15–30 h → 13–27 h; feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~378–722 h → ~373–712 h |
| 2026-10-11 | minor | Text on a path and pattern fills landed (gap 9): text follows a path or shape (start, side, alignment; edited in place; canvas, PNG, SVG and PDF; SVG `<textPath>` imports as text on a path) and pattern fills tile a layer (grid or half-shifted rows or columns, scale, spacing, alignment; live as the source changes; SVG `<pattern>`, PDF tiling patterns); Figma Draw extras breadth 45% → 65%, ready 30% → 45%, 10–18 h → 5–10 h; Text 8–14 h → 7–13 h; feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~380–725 h → ~375–715 h |
| 2026-10-11 | minor | Team libraries on `figcraft-server` landed (gap 10): a file's library is published to the server under the file and enabled in other files by a share link (per-asset versions; updates noticed in the background and accepted with overrides kept), the server counts which files use which components, and a whole library can be swapped for another by names; Team libraries breadth 40% → 70%, ready 25% → 40%, 10–20 h → 4–8 h; collaboration 12% → 13%, 55–110 h → 49–98 h; feature breadth (~79%) and ready for real work (~47%) unchanged; full parity ~380–725 h → ~375–715 h |
| 2026-10-11 | minor | Branches landed (gap 17): File ▸ Create branch… copies the file into a branch tab that remembers its base; Review and merge shows both sides' changes (layers with Inspect-style property changes, styles, variables, comments) and the conflicts (the same property changed on both sides, or a layer deleted on one side and edited on the other), each resolved for the branch or main; merging is one undo step and archives the branch; Update from main; branches are saved in the `.figcraft` file. Comments, versions, find, misc breadth 80% → 85%, ready 58% → 60%, 4–8 h → 2–5 h; collaboration 12% → 13%; menu breadth 204/204 (the catalog gained File ▸ Create branch…); feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~380–725 h → ~378–722 h |
| 2026-10-11 | minor | Animated GIF fills and TIFF images (gap 12, without video and HEIC): GIF fills keep the original file, show the first frame on the canvas and in exports and play in the presenter and the preview (frames decoded on first play, at most 500 frames and 128 MB; always looping; the window wakes only for a GIF's next frame); TIFF images open, import, place and paste. Paint breadth 85% → 87%, 5–9 h → 4–7 h; gap 12 6–12 h → 5–10 h (video playback and HEIC left, both needing non-Rust decoders); feature breadth (~79%), ready for real work (~47%) and full parity hours unchanged at this rounding |
| 2026-10-11 | minor | The last three catalogued menu items got commands (gap 21): File ▸ Open recent (recent files kept in the preferences, at most 20, a file that's gone greyed in the menu and dropped when picked, Clear recent), View ▸ Outlines ▸ Show outlines for selection and View ▸ Mask outlines: menu breadth 98.5% → 100% (203/203); Canvas & navigation breadth 95% → 97%, 3–6 h → 2–5 h; feature breadth (~79%), ready for real work (~47%) and full parity hours unchanged at this rounding |
| 2026-10-11 | minor | Extended collections, variables import/export and gradient stop and variant property bindings landed (gap 14): Styles & variables breadth 85% → 92%, ready 70% → 72%, 4–8 h → 1–2 h; feature breadth unchanged (~79%); ready for real work unchanged (~47%); full parity ~385–730 h → ~380–725 h |
| 2026-10-11 | minor | Collaboration wording after the standalone server: `figcraft-server` keeps many files, persisted as a snapshot plus a log of what changed (compacted, checksummed, damaged files skipped and reported), restarts lose nothing confirmed and apps resume where they were; the app publishes its file to it; share links per file let people edit or only view (the server refuses a viewer's edits), are revocable and stored as hashes; TLS comes from a reverse proxy (the desktop app has none of its own yet); percentages and hours unchanged until the next re-measure |
| 2026-10-11 | minor | Live boolean editing, Design panel arc fields, the shape builder on strokes and open paths and one-sided variable widths landed (gap 13): Shapes & vector breadth 92% → 97%, ready 66% → 68%, 8–14 h → 1–3 h; feature breadth ~78% → ~79%; ready for real work unchanged (~47%); full parity ~390–745 h → ~385–730 h |
| 2026-10-11 | minor | Collaboration wording after the collaboration polish: styles, variables, collections, modes and comments created in a live session take ids from their creator's id space (concurrent creations no longer overwrite each other; the server checks it), others' text carets and selections show on the canvas, and typing in a layer someone else is typing in says so; percentages unchanged |
| 2026-10-11 | minor | Wording only: live sessions (#104), then the browser joining them and `figcraft://` invite links opening the desktop app; percentages and hours unchanged until the next re-measure |
| 2026-10-10 | minor | Merged main after #101–#103: WASM plugins (plugins area 0% → 35% breadth, 20% ready; ecosystem 5% → 20%) and the multiplayer core without networking (multiplayer 5% → 15% breadth, 5% ready; collaboration 8% → 12%); menu breadth 97.5% → 98.5% (200/203); ready for real work ~46% → ~47%; full parity 420–790 h → 390–745 h; beta 210–380 h → 200–365 h. Fixed the stated weight sums (areas 1–16 sum to 94, total 113; percentages were already computed with those) |
| 2026-10-10 | minor | Marked beta blockers; no alpha blockers (the alpha gate passes) |
| 2026-10-10 | major | Created from the 2026-10-10 re-measure against Figma desktop 126.10.7 (replaces the gap list that lived in ROADMAP.md) |
