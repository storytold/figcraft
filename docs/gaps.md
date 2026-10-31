# Where FigCraft falls short of Figma

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-31 · **Change:** minor (gap 6: widgets draw lines and SVG pictures (plugin API 6); gap 7: text reads only the fallback fonts it draws with; a board connector's path can be reshaped by dragging its path handles (#1710); gap 10: library analytics count insertions and show insertions and detachments over time; gap 6: a widget opens its plugin's panel (plugin API 5), whose callbacks redraw it; the keyboard's Menu key opens the context menu, as Shift+F10 does; gap 14: a collection of this file extends, following its edits at once; colour picker review fixes for text undo, fitted sidebars, moving controls and gradient variables; gap 6: plugin widgets have hover styles; gap 7: the browser draws a zoom's full render a band at a time; `.fig` import keeps the file's saved line breaks while the text and its box are unedited (#1236); FigJam outer shape outlines and sticky body fallback beside author text (#1341, #1397); Arabic draft catalog; gap 7: the browser refines a zoom's preview once the zoom has paused; gap 17: branches in live sessions, branch history entries, the list-merge rows brought up to date; gap 6: plugin widgets take text input and a property menu (plugin API 4); gap 24: board templates and tables; gap 24: a shared timer and voting sessions; gap 24: the board dot grid, connector ends dragged to re-attach, curved connectors; egui/eframe 0.37 verified for gap 18; known gaps from the #1311 review in gaps 4, 14, 15 and 16; gap 16: measurements kept on the canvas, a read-only layers panel in Dev Mode; gap 24: boards and their tail; Dev Mode tail: import lines in layer trees, compare with versions, an inspect-only canvas; gap 5: reviewed Turkish, Czech and Indonesian drafts, the Japanese draft completed; reviewed Russian catalog and integer plurals; `.fig` export: instance sizes, variables with modes, shared styles, variable-width strokes; sharing with a join code; text box sizes measured against documents re-saved by other software; live sessions per document; .fig export checked by opening written files in other software; experimental .fig export; keyboard shortcut parity suite and bindings; Simplified Chinese draft catalog; Latin American Spanish overlay; prototyping tail: gamepad triggers, widgets in prototypes, mode lookups; plugin directory on figcraft-server; Japanese and Korean draft catalogs and CJK UI fonts; plugin widgets and the plugin panel tail; French, German and Brazilian Portuguese draft catalogs; cursor chat and spotlight in live sessions; layers and assets panels behaviour suite and fixes; vector multi-edit and edit tail; dev resources links; extended collections tail; branch version history and finer list merges; comments and version history behaviour suite and fixes; canvas navigation, guides and grids behaviour suite and fixes; `.fig` import: editable vector networks, more layer kinds, variable prototype actions; `.fig` import, best effort: gap 1 down to its tail; Windows QA pass: everyday workflows on Windows 11, eight fixes incl. unsaved work kept after a crash or a forced quit; Display P3: gamut warning and soft-proof (true P3 blocked upstream); several windows; localization layer with a draft Spanish catalog; hosting figcraft-server: wss in the desktop app, server releases, deployment guide; plugin panels and settings; canvas fixes: boolean hit testing on the result, group drags, frame outlines and slices; code connect; stretch and scatter brushes, pen pressure; text on a path and pattern fills; team libraries on the server and swap library; branches: create, update from main, review and merge; animated GIF fills and TIFF images; Open recent, outlines for the selection and mask outlines: menu breadth 100%; extended collections, variables import/export, gradient stop and variant bindings: gap 14 down to its tail; the 2026-10-17 landings: `.fig` import fidelity, budgets for large files, the import report, a `.fig` copy from Save as and paste from another design app (gap 1); menus, the Actions palette, editor fixes, Selection colors and smart-selection spacing (gap 4); catalog counts re-measured (gap 5); bounded canvas pan overscan) · **Target:** Figma Design (desktop app 126.10.7)

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
| 3 | No representative corpus checked against real Figma files | Fidelity | 15–30 | [target-app-parity](target-app-parity.md) |
| 4 | Interaction fidelity not compared tool by tool | UI/UX | 40–60 | [ui-parity](ui-parity.md) |
| 5 | Localization: English, reviewed Russian and Turkish, plus unreviewed Spanish (and Latin American Spanish), French, German, Brazilian Portuguese, Japanese, Korean, Simplified Chinese, Indonesian and Czech drafts; Turkish and Swedish catalogs; Arabic draft: 2,508 entries, native review and panel mirroring pending | Localization | 29–51 | [localization-parity](localization-parity.md) |
| 6 | Plugins: widgets without timers, a self-hosted directory but no public catalogue; Figma's plugins don't run | Ecosystem | 6–22 | [target-app-parity](target-app-parity.md) |
| 7 | CPU canvas where Figma uses the GPU | Hardware/performance | 30–60 | [hardware-parity](hardware-parity.md) |
| 8 | Platform QA: Linux and the browser left (Windows pass done 2026-10-11) | Stability/platforms | 10–20 | [hardware-parity](hardware-parity.md) |
| 9 | Figma Draw: pen pressure on macOS/Linux tablets (multi-edit, brushes, pressure, text on a path and pattern fills landed) | Feature | 1–3 | [target-app-parity](target-app-parity.md) |
| 10 | Team libraries: no team-wide lists or accounts; analytics cover components only | Feature | 4–8 | [components-parity](components-parity.md) |
| 11 | Spell check | Feature | 4–8 | [target-app-parity](target-app-parity.md) |
| 12 | Video fills; HEIC images | File format | 5–10 | [file-format-parity](file-format-parity.md) |
| 13 | Vector tail: none left (boolean hit testing on the result landed 2026-10-11) | Feature | — | [target-app-parity](target-app-parity.md) |
| 14 | Variables: extended collections checked on real libraries (the tail landed 2026-10-12) | Feature | 0–1 | [components-parity](components-parity.md) |
| 15 | Prototyping: media triggers and actions; controllers on macOS, Linux and FreeBSD (controller buttons on key/gamepad triggers, widgets in prototypes landed) | Feature | 2–5 | [prototyping-parity](prototyping-parity.md) |
| 16 | Dev Mode: renaming the file in Dev Mode to check; the editor extension is left to the MCP server (publishing mappings from the code repository landed 2026-10-30) | Feature | 1–2 | [dev-mode-parity](dev-mode-parity.md) |
| 17 | Branching: text runs merged whole, branches only their author sees in a live session (branches in live sessions, export settings and guides item by item, and history entries for creation and updates landed 2026-10-24) | Feature | 1–2 | [collaboration-parity](collaboration-parity.md) |
| 18 | Display P3 on screen: blocked upstream (egui-wgpu picks an 8-bit sRGB surface); gamut warning and soft-proof sRGB landed, as did several windows | Hardware | 4–8 | [hardware-parity](hardware-parity.md) |
| 19 | AI features | AI | 30–60 | [target-app-parity](target-app-parity.md) |
| 20 | Mobile apps (view, mirror, comment) | Platforms | 20–40 | [hardware-parity](hardware-parity.md) |
| 21 | Menu items without a command: none left (landed 2026-10-11) | Feature | — | [parity-checklist](parity-checklist.md) |
| 22 | Screen readers never run; browser has no accessibility tree | UI/UX | 4–8 | [ui-parity](ui-parity.md) |
| 23 | CMYK output intent for print PDFs | File format | 2–4 | [file-format-parity](file-format-parity.md) |
| 24 | Boards: the whiteboard tail (widgets, mind maps…) | Feature | 20–44 | [boards-parity](boards-parity.md) |

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
- **Fixed (2026-10-26, #1341):** generated FigJam sticky notes and shapes with text keep their
  background, text, typography, text paint and saved child bounds, including sticky author lines,
  as editable frame children. Synthetic fixtures cover generated records, missing bounds,
  unsupported versions, saved vector networks without geometry paths, outer fill/stroke outlines
  when generated geometry is damaged, outer body-text fallback beside a surviving author and
  layer budgets; the issue sample was checked locally only. Native
  board metadata and direct `.jam` opening (#1342) remain separate work.
- **Landed (2026-10-26):** `.fig` import reads vector-region fill stacks; vector editing and path
  operations retain them with their source paint coordinates. `.fig` export still warns and drops
  region fills. Evidence: `crates/format/src/fig/tail_tests.rs`, `crates/engine/src/shapebuilder_region_paints_tests.rs`.
- **Fixed (2026-10-29, #1236):** imported text keeps the line breaks the file saved with it
  (`derivedTextData`), so a font whose advances differ slightly from the file's (another build
  of the family, a stand-in) no longer wraps a word early or splits one in a narrow box. The
  breaks hold only while the box width and everything that moves a break (characters, fonts,
  sizes, spacing, case, axes, features, lists, paragraph settings) are as saved, and are given up
  where a line would come out more than 3% (+1 px) wider than its box. The line breaker itself
  was checked and left as is: trailing spaces and the letter spacing after a line's last glyph
  already stay out of the fit test, box widths aren't rounded and the fit tolerance is 1e-6 px,
  so the differences come from the fonts' advances. Evidence:
  `crates/text/src/saved_breaks_tests.rs`, `crates/format/src/fig/text_wrap_tests.rs`; not
  checked on real files.
- **Missing:** the background of connector labels without a saved snapshot (the label is kept,
  centred on its line), table cells beyond their own layers, live region
  style/variable links, variables
  bound to fields the editor can't bind and video fills aren't read; text lays out again with
  local fonts (an unedited text keeps the file's line breaks, but glyph positions, line widths
  and heights come from the fonts at hand, and an edited one wraps by them), and the saved breaks
  were never compared with real files. Beyond the local FigJam regression check for #1341, there is no real-world corpus (gap 3): the 2026-10-17
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

### 3. No representative corpus checked against real Figma files
- **Missing:** a corpus of real documents (design systems, app screens, prototypes) imported and
  compared with Figma's own PNG/SVG/PDF exports.
- **Evidence:** every corpus in the repo is synthetic (`crates/svg/tests/corpus`, the synthetic
  design system in `crates/engine/src/content/synthetic_ds.rs`). Scores were self-graded by the agents that
  built the features.
- **Impact:** unknown fidelity in auto layout sizing, text metrics, effects and component
  resolution on real files; the numbers in this repo can't be trusted beyond "estimated" until it
  exists.
- **Local regression check (2026-10-26):** the public FigJam issue sample in #1341 was read
  locally to check its generated backgrounds, text and author lines. It is not committed and
  does not establish representative design-file fidelity.
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
  hover states, popovers, menus, snapping thresholds. Known: the Assets panel's export preview
  follows a slice's first export setting, so a second setting with the other Ignore overlapping
  layers value previews differently from what it exports.
- **Landed (2026-10-11):** behaviour suites written from the public documentation for auto layout,
  selection and transform, text, and canvas navigation with rulers, guides, layout grids, outline
  mode and measuring, then (2026-10-12) comments and version history, and the layers and Assets
  panels (`crates/engine/src/*_behaviour_tests.rs`, `crates/ui-egui/src/layers_behaviour_tests.rs`),
  each with its fixes; then keyboard shortcuts per platform (`shortcut_parity_tests.rs`: 129
  documented rows, now 128 bound, 1 moved off a documented conflict, none missing, 57 undocumented
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
  selections and smart-selection spacing fields, canvas handles and reorder rings. Each was checked in the app (on
  Windows; the submenus by their author), none side by side with Figma, none on macOS.
- **Evidence:** ROADMAP item 7 ("Left: tool by tool, panel by panel against the incumbent").
- **Impact:** Figma users notice the feel within minutes; it decides whether they stay.
- **Estimate:** 40–60 h. Detail in [`ui-parity.md`](ui-parity.md).

### 5. Localization: English, reviewed Russian and Turkish, plus unreviewed drafts
- **Missing:** native reviews of the Spanish, French, German, Brazilian Portuguese, Japanese,
  Korean, Simplified Chinese, Indonesian and Czech drafts and the Latin American Spanish
  overlay, the strings added since each catalog was drafted (1–5% of each), the Arabic draft's native review and panel mirroring, Hindi and
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

### 6. Plugins: widgets without timers, a self-hosted directory but no public catalogue; Figma's plugins don't run
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
  Widget fields and property menus (plugin API 4, 2026-10-24): a widget's text layer can be an
  editable field, edited on the canvas with a double-click like any text or with `widget.setText`
  (the value, cleaned and capped, goes into the widget's state and the plugin redraws; one undo
  step), and a widget can carry a property menu of actions, toggles and colour choices, shown in a
  small bar while it's selected and driven with `widget.menu`; the Counter example uses both.
  Widget hover styles (2026-10-30): a widget layer's `hover` (fill, stroke, opacity, a text's
  colour) shows while the pointer is over it on the canvas, as a live preview that is never saved
  or synced and that any command ends (`widget.hover` for agents); the Counter's buttons darken.
  Sliders (2026-10-30): a panel control or command parameter of type `slider` (a number between
  its `min` and `max`) is drawn as a slider with its value beside it; the plugin is called back
  once, when the drag ends.
  Panel images (2026-10-30): an `image` control shows a picture the plugin carries (base64, at most
  256 KB and 4,096 pixels on a side), as wide as a field at most, named by its label.
  Widgets open panels (plugin API 5, 2026-10-30): a widget's callback can show its plugin's panel,
  whose callbacks get the widget's id and state and redraw it, one undo step each.
  Lines and SVG (plugin API 6, 2026-10-31): a widget layer can be a `line` or an `svg`, whose
  picture comes in as locked layers, scaled to fit a size.
- **Missing:** a public catalogue and Community files, publisher accounts and signatures, ratings
  and install counts, background update checks, the directory in the browser build; widgets have no
  timers. Figma's own plugins are JavaScript against
  Figma's plugin API and don't run.
- **Impact:** teams that depend on specific Figma plugins (icons, content, accessibility checks)
  can't bring them. FigCraft's CLI, control channel and MCP server cover agent automation better
  than Figma does.
- **Estimate:** 6–22 h for the missing pieces of FigCraft's own API. Running Figma's plugins
  needs a JS engine and an API shim: **owner decision needed**.

### 7. CPU canvas where Figma uses the GPU
- **Missing:** GPU rendering of the canvas. FigCraft renders with tiny-skia on the CPU (banded,
  parallel, retained) and shows the image through wgpu. A new zoom can't reuse the retained
  image; when renders from scratch take a frame or more, the whole view is previewed with a
  sixteenth of the pixels first, then refined. With no measure to go by (the first render, another
  page, a zoom after quick renders), a watchdog thread previews any such render still running
  after 12 ms (desktop only; the browser renders inline).
- **Landed (2026-10-29):** the browser draws a zoom's full render a band at a time (`Retained::begin`,
  `Steps`; at least a band a frame and at most eight bands, so a frame draws a part of the render
  where it drew the whole of it: 0.7–1.4 s where the render took 3.7–4.0 s in the measurement of
  #1496), so input is handled while it draws: a pan or an edit meanwhile is patched into
  the render once drawn (or it is drawn again, the first shown till then), a zoom drops it. Bands
  overlap by what background blurs read, as the desktop's parallel bands do, and the bands together
  take longer than one render (each repeats its overlap and some per-band work).
- **Landed (2026-10-30):** text reads only the fallback fonts it draws with. Each fallback family
  (`FALLBACK_FAMILIES`, for the characters a run's own font lacks) reads the face nearest the
  run's style, not every face it has; families naming the faces of one file (Noto Sans CJK SC and
  KR) share its bytes; and the faces are worked out once per style until fonts change, not for
  every text laid out. On a Linux machine with Noto Sans CJK, DejaVu Sans and Inter installed, the
  first text laid out read 41 font files (265 MiB, kept in memory) and parsed 41 faces; it now
  reads 3 files (20 MiB) and parses 4 (6 files and 8 faces with a bold run too): 150–190 ms less
  file reading there. Looking up the fallbacks for each text laid out, under the font database's
  write lock, went from about 45 µs to 0.2 µs (the lookup logic timed alone). The faces drawn are
  those drawn before, except where two are equally near the run's style (the one read already is
  kept) and for Hiragino's W3-style names (read as their weight).
- **Landed (2026-10-28):** the browser refines a zoom's preview only once no newer request has
  come for 120 ms (`SETTLE_MS`), or for as long as its last full render took when that is shorter:
  its full renders run on the UI thread, and one between every two wheel steps held the page up
  for seconds. Pans and edits (no preview) are refined at once; the desktop, with its render
  thread, is unchanged.
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

### 10. Team libraries: no team-wide lists or accounts; analytics cover components only
- **Missing:** libraries listed for a whole team or organization (each file enables a library by
  its link), accounts and per-person library permissions, analytics of styles and variables (a
  component's instances, insertions and detachments are counted, the last two week by week),
  update notices pushed by the server (apps ask every 30 seconds while the Assets panel shows).
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
- **Landed (2026-10-22):** the transform box around points selected in several layers at once, in
  the page's axes: its corners and edges scale them, outside a corner rotates them and inside drags
  them, each in its own layer and one undo step for all (`transform_tests`).
- **Landed (2026-10-12):** several Bezier handles selected with Shift-click and dragged together,
  sharp points beside curved segments rounded, a stroke cap and join per point (canvas, PNG, SVG,
  PDF, outline stroke), the Pen outside edit mode connecting to its own points and continuing a
  selected open path. Left: corner smoothing on vectors, corner radius and smoothing on boolean
  groups that contain rectangles, per-point caps and joins on dashed, variable-width and brush
  strokes.
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
- **Landed (2026-10-30):** a collection of this file extends too, as the help center shows (Extend
  collection on its row in the Variables editor, `variable.extendCollection`), following its edits
  at once; an extended collection of this file extends again.
- **Missing:** checked with synthetic files only. A paint style's color offers variables scoped to
  any fill or stroke: FigCraft's own rule, since the help center doesn't say which scopes it takes.
  The help center says an extended collection takes no variables or modes of its own; FigCraft's
  takes variables of its own (modes are refused), kept for now.
- **Estimate:** 0–1 h.

### 15. Prototyping: media triggers and actions, controllers off Windows
- **Landed (2026-10-12):** key/gamepad triggers take controller buttons (`Gamepad A` … `Gamepad
  Home`, the standard layout's 17 buttons; a Controller menu beside the key field; REST files'
  controller triggers import); the player reads controllers on Windows and in the browser (Gamepad
  API) and takes `ui.input {gamepad}` on the control channel; widgets react to clicks in a
  presented prototype, on the prototype's own copy; `proto.evaluate` computes expressions (mode
  lookups included) where a layer's interaction would run (`crates/engine/src/proto_tail_tests.rs`,
  `crates/ui-egui/src/present/tail_tests.rs`, `crates/format/src/rest/gamepad_tests.rs`).
- **Missing:** video triggers (on media hit/end) and play/pause/mute/seek actions (the REST
  importer warns and skips them, `crates/format/src/rest/proto.rs`); controllers on macOS, Linux
  and FreeBSD (the usual Rust controller library links libudev on Linux; a pure-Rust reader per
  platform, or an owner decision on that dependency); PlayStation and Switch button labels.
- **Estimate:** 2–5 h, the media part after video fills (gap 12).

### 16. Dev Mode: the tail after dev resources
- **Landed (2026-10-11):** code connect: a component or set links to its code component per
  language (React, Leptos, Vue, SwiftUI, Compose, ArkUI, Flutter): an import path, the name and a template whose `{property}`
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
- **Landed (2026-10-15):** generated layer trees (React, Leptos, SwiftUI, Compose) start with the
  import lines of the connected components they call, once each, also those in a component's
  definition, in the slots an instance fills and placed by a connected template's instance swaps.
  Compare changes takes any saved version as the before (`dev.compare {version}`; the view's
  Compare with menu), for any top-level frame or section. A Dev Mode behaviour suite: the canvas
  only inspects (presses select, pan and comment; no moving, resizing, drawing, text or vector
  editing), the menus, shortcuts, nudges, opacity keys and paste refuse design edits with a note
  while Dev Mode's own edits still run, drawing tools from the toolbar return to the design; the
  ready list filters and badges Changed items (`inspect.readyForDev {status: changed}`); statuses
  are refused on layers inside frames. Read-only plugins run from the Inspect tab show their
  panels.
- **Landed (2026-10-24):** measurements kept on the canvas: between two layers, along one side of
  a layer or to the layer around it (`devMode.addMeasurement` / `removeMeasurement` /
  `measurements`), stored on the layer they start from (saved, synced, undoable; old files open
  without them), drawn in Dev Mode as labelled redlines that follow the layers, and listed, placed
  and removed in the Inspect tab. The layers panel is read-only in Dev Mode (rows select and
  expand; no renaming, reordering, hiding or locking).
- **Landed (2026-10-24):** files dropped on the canvas, the Pages list and the Assets panel no
  longer edit the design in Dev Mode (a dropped document still opens as a file of its own, and
  the Pages list only opens pages).
- **Landed (2026-10-30):** `figcraft-cli connect publish <file.figcraft> [--mappings
  figcraft.connect.json] [--dry-run]` publishes a code repository's mappings to a design file, so
  code owners keep them with the code and publish them from CI ([cli.md](cli.md)).
- **Missing:** the file name field in the left panel still renames the file in Dev Mode
  (`file.rename`); whether Figma allows that in Dev Mode needs checking before it changes. An
  editor extension is out of scope: the MCP server already serves design data, code and the
  connected snippets to agents and editors.
- **Estimate:** 1–2 h.

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
- **Landed (2026-10-18, #798):** layout grids, export settings, guides, prototype interactions,
  accessibility annotations and Dev Mode resources merge item by item like fills (code links merge
  whole: one per language).
- **Landed (2026-10-24):** branches of a file in a live session: creating, editing, reviewing,
  updating from main and merging work while the file is in a session. A branch stays with the
  person who made it; merging it is one edit of the session document that everyone gets, and one
  step of its author's own undo history. A branch made in the session makes its ids in its author's
  own client space, 2^24 above every id made there on the main file, so the session server takes
  the layers it adds; others' edits on main while it is open are main's changes against the
  branch's own base, and conflicts with them resolve as offline (a layer a peer deleted comes back
  with its id when the branch's edits win). A view link still refuses merging, and merging a branch
  made outside the session that adds layers, styles, variables or comments (ids in another space)
  is refused with a reason instead of being sent for the server to refuse. A branch's history gets
  entries for its creation and each update from main. Engine tests through a session server with
  several people, a fuzz test with random real commands (export settings and guides included,
  offline and live) and a junk-parameter sweep of the branch commands in a session.
- **Missing:** text runs still conflict as a whole (runs are character ranges into the text, so
  lining them up item by item would need a text diff, not positions); branches in a live session
  are their author's only (others can't see or open them: that needs the server to keep them);
  a branch made offline that adds layers merges only outside a session.
- **Estimate:** 1–2 h.

### 18. Display P3 on screen (several windows landed)
- **Landed (2026-10-11):** several windows: File ▸ New window, a tab's Open in new window or a tab
  dragged off the strip; each window has its own tabs, panels and canvas and draws only when it
  needs to; closing one asks about unsaved changes; `window.new`, `window.list`, `window.focus` and
  `window` on any control request.
- **Missing:** wide-gamut colour on the canvas (egui draws sRGB, so P3 colours outside sRGB clip on
  screen; exports keep them); a screen-reader tree in windows other than the main one (eframe 0.37
  gives only the main window one; verified in the fork's viewport setup); window positions restored at
  the next start.
- **Estimate:** 4–8 h.

### 19. AI features
- **Landed (2026-10-27, narrow slice without a model):** MCP agent feedback — comments as the task
  queue (`comment.prompt`, **Copy for AI**, `fix_comments` / `watch_comments`,
  `figcraft://comments`), a bounded in-memory event log, `events.poll` and `wait_for_changes` for
  live feedback. Any MCP client can drive a headless file or an attached app; no in-app model and no
  network dependency.
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

### 24. Boards: the whiteboard tail
- **Landed (2026-10-24, M12):** board files (`file.newBoard`) with sticky notes, shapes with text,
  connectors that re-route when what they join moves, marker, highlighter and eraser, stamps, text
  and sections; a board toolbar and a contextual bar; live sessions sync it all. Then (also M12) a
  timer everyone on the board sees and voting sessions with per-person caps, hidden votes and
  ranked results (`board.timer`, `board.vote`), and five built-in templates (retrospective,
  brainstorm, weekly planner, flow chart, kanban board) from a toolbar picker or `board.template`,
  and tables (`board.table`, `board.tableEdit`, up to 50 × 20) made of auto layout frames.
- **Landed (2026-10-24, M12):** a dot grid on board pages, under the layers; a selected
  connector's ends drag onto another object or empty canvas (`board.reattach`); curved connectors;
  elbows that go around the objects they join.
- **Missing:** timer music, board widgets, saving your own templates, table column resizing and
  cell colours, mind maps, washi tape, code blocks, audio and huddles, quick-create handles,
  the stamp wheel, emotes and stickers, `.jam` files
  ([`boards-parity.md`](boards-parity.md)).
- **User impact:** boards cover a brainstorm, a retrospective, a flow sketch and a timed, voted
  workshop round.
- **Estimate:** 20–44 h.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | minor | Gap 7: text reads only the fallback fonts it draws with: a face per fallback family and style, a file's bytes shared between families, the faces worked out once per style (first text laid out: 41 font files and 265 MiB read down to 3 files and 20 MiB on a Linux machine with Noto Sans CJK). Numbers unchanged |
| 2026-10-31 | minor | Gap 6: widgets draw lines and SVG pictures (plugin API 6), so that item leaves the missing list. |
| 2026-10-31 | minor | Gap 24: connector paths can be reshaped by hand (path handles and `board.reshape`), so that item leaves the missing list. Estimate unchanged. Tests: `board_reshape_tests`, `board_line_tests`. |
| 2026-10-31 | minor | Gap 10: library analytics count insertions, and insertions and detachments week by week for a year (Show usage: the past 30 days, 90 days or year; `library.usage {days}`). Styles and variables aren't counted. Estimate unchanged. Tests: `library_actions_tests`, `inserting_library_instances_counts_them` |
| 2026-10-30 | minor | Gap 6: a widget opens its plugin's panel (plugin API 5), whose callbacks get the widget and redraw it; the Counter example sets its count from a panel. The entry's duplicated Missing text is merged. Estimate unchanged |
| 2026-10-30 | minor | Gap 4: the keyboard's Menu key opens the context menu, as Shift+F10 does (at the focused layer row or below the canvas selection, with the keyboard in the menu). egui already reported the key; FigCraft's keymap had no name or binding for it. `view.contextMenu` now takes `Menu` too, and the Context menus row is done (`layers_keyboard_tests`). Stale rows corrected: smart-selection spacing has had reorder rings since the "M1: smart-selection drag reordering" landing (the row is still partial: irregular grids and several marked layers aren't covered), and gap 4 and the parity summary quoted shortcut counts from before the last bindings (now 128 bound, none missing, 57 undocumented pinned, as `shortcut_parity_tests` counts). |
| 2026-10-30 | minor | Gap 14: a collection of this file extends (Extend collection on its row, `variable.extendCollection`), following its edits at once; extensions of this file extend again. The help center's rule that an extension takes no variables of its own is noted, not applied. Estimate unchanged |
| 2026-10-30 | minor | Gap 24: stamps stick to the objects they land on and move with them; that item leaves the missing list (the stamp wheel, emotes and stickers stay). Estimate unchanged. Test: `board_tests::stamps_stick_to_what_they_land_on` |
| 2026-10-30 | minor | Gap 10: library analytics count detached instances per component and file (reported by the file that detached them, kept by the server beside its usage, shown in Show usage and `library.usage`); insertions and counts over time remain. Estimate unchanged. Tests: `library_detach_tests` (server), `codec_tests::older_library_messages_keep_their_encoding`, `assets::library::tests::add_place_and_update_keep_overrides`, `detach_report_tests` |
| 2026-10-30 | patch | Gap 15: `proto.linkNotice` (the Actions palette, the control channel, MCP) is saved with the preferences at once, as the notice's own "Don't show this again" is, instead of only when the app quits; that note leaves the missing list. Estimate unchanged. Test: `present_links_tests::skipping_the_notice_by_command_is_saved_at_once` |
| 2026-10-30 | minor | Gap 6: plugin panels show pictures: an `image` control (`data` base64, at most 256 KB and 4,096 px a side; panel specs up to 1 MB) drawn at its size up to a field's width, named by its label, which shows instead when the picture can't be decoded. "Images" leaves the missing list; estimate unchanged. Tests: `v2_tests::panels_are_validated_and_keep_entered_values`, `plugin_panel_tests::panel_images_show_named_or_fall_back_to_their_label` |
| 2026-10-30 | minor | Gap 1: labels of connectors without a saved snapshot are kept, centred on their line (without their background), instead of skipped. Not checked on real files; estimate unchanged. Test: `fig::tests::tail::a_labelled_connector_without_a_snapshot_keeps_its_label` |
| 2026-10-30 | minor | Gap 6: widget hover styles landed (a layer's `hover` fill, stroke, opacity or text colour shown while the pointer is over it, as a live preview; `widget.hover`); the entry no longer lists hover. Estimate unchanged. |
| 2026-10-30 | minor | Gap 1: elbowed and curved connectors without a saved path no longer import as straight lines; they are routed as board connectors are (the shared `figcraft_doc::route_points`). Not checked on real files; estimate unchanged. Tests: `fig::tests::tail::connectors_without_a_saved_path_follow_their_line_style` |
| 2026-10-30 | minor | Gap 6: plugin panels and command forms take a `slider` control (a number between a required `min` and `max`), drawn as a slider with its value field beside it; the plugin is called back once when a drag ends, one undo step. "Sliders" leaves the missing list; estimate unchanged. Tests: `v2_tests::panels_are_validated_and_keep_entered_values`, `plugin_panel_tests::a_slider_calls_the_plugin_back_once_when_let_go` |
| 2026-10-30 | minor | Gap 16: `figcraft-cli connect publish` publishes a repository's `figcraft.connect.json` to a design file (links by component name, warnings for links that don't fit, `--dry-run`, exit 1 when nothing links). The CLI part leaves the missing list; the Dev Mode file-rename check remains; estimate unchanged. Tests: `files::tests::connect_publish_links_components_and_saves_the_file`, the CLI argument tests |
| 2026-10-29 | minor | Colour picker review follow-up: opening the first picker preserves a text undo interaction; placement follows the rendered sidebar; opening controls track their widget ids while moving; linked selection-colour names toggle closed; gradient variable-picker presses keep the parent open. Regression tests cover each case; estimates unchanged. |
| 2026-10-29 | minor | Colour pickers dismiss on presses outside their card, including the sidebar, and open 8 pt beside the sidebar aligned to their control (lifted to fit the screen); switching targets resets a dragged position and commits pending edits. Eyedropper sampling stays open. UI regression tests cover added fills/strokes and page colours; headline estimates unchanged. |
| 2026-10-29 | minor | Gap 7: the browser draws a zoom's full render a band at a time (eight at most), so a deep zoom no longer holds the page for the whole render; pans and edits meanwhile are patched in once it is drawn. Numbers unchanged |
| 2026-10-28 | minor | Arabic draft: 1,857 UI strings, 620 command labels, 31 menu wordings; count-neutral plurals and web font gate. Native review and panel mirroring remain open; grade and estimates unchanged. Evidence: `locales/ar.toml`, i18n catalog tests and [translation notes](translating.md#arabic-draft) |
| 2026-10-28 | minor | Gap 7: the browser refines a zoom's preview only once the zoom has paused for 120 ms, so a wheel zoom no longer runs a blocking full render between every two steps. Numbers unchanged |
| 2026-10-27 | minor | M12: the MCP agent feedback workflow landed (comments as tasks, `comment.prompt`, Copy for AI, `fix_comments` / `watch_comments`, `figcraft://comments`, a bounded event log, `events.poll` and `wait_for_changes`). Figma AI features and local models are still not started, so gap 19's missing list and estimate stand |
| 2026-10-27 | minor | M6, #1397: damaged generated FigJam shape geometry falls back to usable outer fill or stroke outlines before a rectangle; a surviving sticky author no longer suppresses the outer body-text fallback. Synthetic regressions cover both outlines, missing/invalid/blank bodies, author retention and layer budgets. Numbers unchanged. |
| 2026-10-26 | minor | M6, #1397: generated FigJam backgrounds read saved vector networks before rectangle fallback; missing or invalid generated text uses the outer text without discarding successful children or exceeding the layer budget. Regression tests cover both paths. Numbers unchanged. |
| 2026-10-26 | minor | FigJam generated sticky notes and shapes with text (#1341): backgrounds, text with its typography and paints, saved bounds and sticky author lines import as editable frame children; synthetic regressions and a local check of the report sample; native board metadata and direct `.jam` opening remain outside this fix. Numbers unchanged. |
| 2026-10-25 | minor | egui/eframe 0.37 verified for gap 18: the remaining limitations are unchanged. egui-wgpu 0.37 still picks an 8-bit sRGB surface with no colour-space option, so P3 colours outside sRGB still clip on screen; eframe 0.37 still gives only the main window an AccessKit adapter, so other windows have no screen-reader tree. Numbers unchanged |
| 2026-10-25 | minor | Known gaps from the #1311 review: gap 4, the export preview follows a slice's first export setting; gap 14, the scopes a paint style's color takes are FigCraft's own rule; gap 15, `proto.linkNotice` from the palette or control channel isn't saved until the next preferences save; gap 16, the file name field still renames in Dev Mode (to check against Figma first). Ranking and hours unchanged |
| 2026-10-24 | minor | Gap 24: the dot grid on board pages, dragging a connector's end to re-attach it, curved connectors and elbows around the objects they join landed; connector waypoints placed by hand stay missing. 30–60 h → 28–56 h; the ranking is unchanged |
| 2026-10-24 | minor | Gap 16: Measurements kept on the canvas (gap 16): `devMode.addMeasurement {from?, to?, side?}` places one between two layers, along one side of a layer or (neither) to the layer around it, stored on the layer it starts from (saved in `.figcraft`, synced, undoable; old files open without them); Dev Mode draws them as labelled redlines that follow the layers, the Inspect tab lists them (those of the selected layer, or every one on the page), places and removes them; `devMode.removeMeasurement` and `devMode.measurements` remove and list them; a copy keeps those to layers copied with it. The layers panel is read-only in Dev Mode: rows select and expand, but don't rename, reorder, hide or lock. Left: publishing mappings with a CLI; 2–4 h → 1–2 h |
| 2026-10-24 | minor | Gap 24: a shared board timer and voting sessions landed (M12); 30–60 h → 26–54 h, ranking unchanged |
| 2026-10-24 | minor | Gap 24: board templates (retrospective, brainstorm, weekly planner, flow chart, kanban board; `board.template`, a toolbar picker) and tables (`board.table`, `board.tableEdit`); 30–60 h → 26–54 h, rank unchanged |
| 2026-10-24 | minor | Gap 6: plugin widgets take text input (editable fields, `widget.setText`) and a property menu (`widget.menu`, a bar over the selected widget), plugin API 4; 8–26 h → 6–22 h; ranking unchanged |
| 2026-10-24 | minor | Gap 17: branches of a file in a live session (create, edit, review, update from main and merge while the file is in a session; the branch stays with its author and makes ids in their own client space far above the main file's; merging is one synced edit and one step of the author's undo history; peers' edits merge against the branch's own base and conflicts resolve as offline; a view link still refuses merging; a branch made outside the session that adds layers is refused with a reason); a branch's history gets entries for its creation and updates from main; the list-merge wording brought up to date with #798 (export settings and guides already merged item by item); text runs still merge whole. Ranking and hours unchanged |
| 2026-10-24 | minor | Boards (M12): a file kind of its own (`file.newBoard`, saved in `.figcraft`, carried to live-session joiners) with sticky notes, shapes with text, connectors that re-route when what they join moves (an engine recompute, the same for local edits, undo and others' edits), marker, highlighter and eraser, stamps with the placer's name, a board toolbar and a contextual bar; prototyping and Dev Mode refuse on boards; design files and boards have their own marks on tabs and file rows. New gap 24 (the board tail, 30–60 h); the ranking above it is unchanged |
| 2026-10-15 | minor | Gap 5: Turkish reviewed by a native speaker (#263), Czech (#469) and Indonesian (#477) drafts, the Japanese draft caught up (#288); Indonesian is no longer missing; counts re-measured (2,177 entries) and corrected: only Spanish is held to completeness, Russian included. Estimate unchanged at this rounding |
| 2026-10-17 | minor | The 2026-10-17 landings. Gap 1: a new Landed entry (community batches #727 and #728, then #745: instance identity and cached library components, overrides by key, text style precedence, grid track sizes, slot content, labels of connectors saved with their path, skip ink, paragraph options, list start numbers, vertical trim, per-point stroke joins, text on a path, image adjustments, images in the message, instance layers' size and resizing overrides, Hug where nothing is recorded; budgets from the file's size; the import report; a `.fig` copy from Save as; paste from another design app), its Missing list cut to what is still left, and the wrong "whether other apps open its files is unknown" replaced by what was checked; all on synthetic files, so the beta blocker and hours stand. Gap 2: server limits, log recovery and web join links in the evidence. Gap 4: menus, the Actions palette and the editor fixes (#733, #735, #736, #729), then Selection colors and smart-selection spacing (#753) landed, none compared side by side. Gap 5: catalog counts re-measured (2,272 entries; drafts at 95–99%; Indonesian, Czech, Turkish and Swedish listed). Gap 8: the Linux and BSD clipboard changes and Wayland window controls, not run there by the maintainers. Ranking and hours unchanged |
| 2026-10-15 | minor | Gap 7: bounded canvas overscan for smoother panning. Estimates and ranking unchanged. |
| 2026-10-15 | minor | Gap 16: import lines inside generated layer trees, compare with any saved version, an inspect-only Dev Mode canvas (menus and shortcuts refuse design edits), Changed in the ready list, statuses only on sections and top-level frames, read-only plugins' panels from the Inspect tab. Left: measurements kept on the canvas, a read-only layers panel, publishing mappings with a CLI; 0–1 h → 2–4 h |
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
