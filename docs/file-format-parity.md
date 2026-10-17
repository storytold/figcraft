# File format parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-17 · **Change:** minor (missing fonts listed on open (#752) and downloaded on request (#754); `.fig` import fidelity from the community batches, budgets for large files, the import report, a `.fig` copy from Save as, paste from another design app; instance layers' size and resizing and Hug on import (#731, #732); SVG/PDF export depth refusal (#208); `.fig` export: instance sizes, variables with modes, shared styles, variable-width strokes; `.fig` export: written files checked by opening them in other software; experimental `.fig` export; export and inspect behaviour suite and fixes; .fig import: editable vector networks and more layer kinds; embedded image colour profiles; `.fig` import, best effort; animated GIF fills and TIFF images; last major: created, formats from the installed Figma's Info.plist and help center, ours from source) · **Target:** Figma Design (desktop app 126.10.7)

Every format Figma reads or writes, with FigCraft's support. Sources: `Info.plist` of
`/Applications/Figma.app` (document types only; nothing inside the bundle's code was read), Figma's
help center (image and video types, export formats), and FigCraft's source.

**Summary (estimated):** ~60% ready. The core interchange formats (SVG, PNG, JPG, PDF) are solid
and tested; Figma's own document, `.fig`, opens (best effort, not yet checked against real-world
files) and can be written, experimentally: written files round trip through FigCraft's own
importer and were checked by opening them in other software (layers, fills, gradients, strokes,
shadows, vectors, components and text draw as in FigCraft); effects such as glass and noise, images
and prototype interactions are still unchecked there.

## Documents

| Format | Figma | FigCraft read | FigCraft write | Fidelity / testing | Hours |
|---|---|---|---|---|---|
| `.fig` (Figma Design, `com.figma.document`, `application/x-figma`) | native (cloud; "Save local copy") | **yes, best effort** (`file.open`, `file.import`, drop, CLI, MCP) | **experimental** ([below](#export-to-fig-experimental)) | `.fig` files open (best effort): the zip, its deflate/zstd chunks and the kiwi schema are decoded generically and mapped by field name onto the REST importer, so pages, the layer tree, geometry, paints (styles resolved), strokes, effects, text with styled runs, vectors as editable vector networks (points, curved segments, filled areas with their winding, per-point corner radius and handle mirroring, end caps; outlines when the network is missing or damaged), auto layout (grid included; a flow that records no primary-axis sizing hugs on that axis, an instance that records no auto layout has its main's), constraints, ellipse arcs, boolean operations, components, sets (with their descriptions), instances (expanded from their main with overrides, derived layout and property values; the size and resizing their layers override stay overrides), styles, variables with modes and bindings, prototyping (set-variable, set-variable-mode and conditional actions included) and image fills from the archive come across; sticky notes, shapes with text, connectors, tables, code blocks, slides and similar layers come in as the closest frames, text layers and vectors, with a warning; anything else is a warning. Since 2026-10-17 (community batches #727, #728): an instance's own identity and the cached library components a file carries, overrides applied by key and order, local text values and instance overrides over a shared text style, grid track sizes (fixed, content-sized, fill), slot content, labels of connectors saved with their drawn path (a static snapshot; the ends aren't live), skip ink, paragraph options and hanging lists, list start numbers, vertical trim, per-point stroke joins, editable text on a path, image adjustments and images carried in the message. Budgets come from the file's own size (one value per compressed byte, 20M–128M, lower ceilings on 32-bit targets; the layer cap follows), so a 15.4 MB synthetic file with 300,000 layers opens whole; the file's bytes stay out of the command journal; an import report lists what was left out or changed once the file opens. Not read: elbowed or curved connectors without a saved path (drawn straight) and labels of connectors without a saved snapshot, table cells beyond their own layers, filled areas with their own fills, variables bound to fields the editor can't bind, video fills. Tested on fixtures generated in code (`crates/format/src/fig/tests.rs`, `tail_tests.rs` and the per-feature `*_tests.rs` beside them, fuzzed) and by reading back what the export writes; no real-world files yet | read tail 5–10, write: checks against real files 5–15 |
| `.jam` (FigJam), `.deck` (Slides) | native to other Figma products | no | no | out of scope | — |
| REST API file JSON (`GET /v1/files/:key`) + local variables JSON + `images/` folder | the API's output | **yes** (`file.open`, `file.import`, CLI) | no | pages, layer tree, geometry, paints, strokes, effects, text runs, vectors, auto layout (sizing modes left out read as hug, as documented), constraints, grids, components, sets, instances with overrides (sizes and resizing included), component properties, styles, prototyping (incl. conditionals and variable modes), variables with modes, aliases, scopes and bindings, image adjustments (the same tint sign as `.fig`); the import report after opening; warnings for what's skipped (video triggers and actions, modifier keys). Unit tests in `crates/format/src/rest/*_tests.rs` | — |
| `.figcraft` (ours: zip of `document.json` + images, versions inside) | — | yes | yes | streamed JSON, 50k layers save ~0.09 s / load ~0.16 s (`cargo xtask bench`); old files keep opening (`large_tests.rs`) | — |
| Local library files | team libraries in the cloud | yes (enable a `.figcraft` as a library) | yes (publish) | per-asset versions, updates with overrides kept | see [collaboration-parity](collaboration-parity.md) |

## Import (place / paste / drop)

| Format | Figma | FigCraft | Notes | Hours |
|---|---|---|---|---|
| SVG | editable vectors | **yes**, editable | corpus of 21 synthetic files with hand-computed expected pixels, structure and round-trip checks; gradients, patterns as image fills, clip paths, masks, shadow/blur filters, blend modes, text as text layers; other filters dropped with a warning; text on one path as text on a path (on several paths as outlines); fuzzed | 2–4 (filters) |
| PNG, JPG | yes | yes | embedded ICC profiles (PNG `iCCP`, JPG APP2) and PNG `sRGB` / `gAMA` / `cHRM` chunks convert the image into the file's colour space; no profile (or CMYK, damaged): taken as sRGB | — |
| GIF | yes (animated) | **yes**, animated | the file is kept as is; the canvas and PNG, JPG and PDF exports show the first frame (SVG exports embed the file, so viewers that animate GIFs animate it), the presenter and the preview play it (frames decoded on first play: at most 500 frames and 128 MB, past that the rest don't play; always loops) | — |
| WebP | yes | yes | embedded ICC profile honoured | — |
| HEIC | yes | **no** | shortcut: no Rust decoder; the usable ones (libheif, libde265) are C libraries, and HEVC carries patents. Revisit when a Rust decoder exists | 2–3 |
| TIFF | Safari only | **yes** | open, import, place, drop (the `image` crate's TIFF decoder; its embedded ICC profile is honoured); SVG exports embed it (and BMP) as PNG, which SVG viewers show | — |
| BMP | — | yes | | — |
| MP4, MOV, WebM (video fills, paid plans) | yes | **no** | shortcut: decoding H.264/HEVC/VP9 needs non-Rust libraries (FFmpeg or each platform's media framework); presenter playback would reuse the animated GIF clock | 4–8 |
| Images over 4096 px | downscaled to 4096 | kept | | — |
| Pasted content from other apps | images, SVG, text, Figma clipboard | images, SVG, text (newest clipboard content wins), **layers copied from another design app, best effort** | Cmd/Ctrl+V reads an image copied in another app (under Wayland through the data-control protocol, not run on Linux here; Linux and BSD clipboard reads give up after 2 s). Copied design layers: the `.fig` canvas in the clipboard's HTML is read like a `.fig` file (with its warnings) and pasted as Paste pastes, in place or into the selected frame (`edit.pasteDesignLayers {html \| data}`, refused over 256 MB before decoding); text copied later pastes as text again. Tested with clipboard HTML made from our own `.fig` export (`design_paste_tests`), never with a real clipboard from other software | with `.fig` |

## Export

SVG and PDF refuse an export whose visible layer tree exceeds 256 descendant levels from any
exported root (the root is depth zero). The error identifies the root and offending layer and
suggests exporting an inner layer or reducing nesting. Hidden subtrees do not count; masks and
subtrees rendered into embedded images do. Refusal returns no partial SVG/PDF and leaves the
existing destination file intact. Selecting an inner root starts a new depth count. This is an
export limit, not a native document nesting limit.

| Format | Figma | FigCraft | Notes | Hours |
|---|---|---|---|---|
| PNG | yes, scales/suffixes | yes | export settings per layer, presets (`2x`, `512w`, `512h`), preview, "Export N layers"; contents only (off: every layer showing in the area) and include bounding box (the layer's own box); slashes in layer names export into folders. The exported area is what the layer paints: shadows towards their offset, outside and centre strokes, unclipped children, a mask group's mask, nothing for hidden layers; slices export everything under them; sides round up to whole pixels | — |
| JPG | yes | yes | white where transparent; fixed quality (90) | — |
| SVG | yes (outline text option, ids) | yes | alpha/luminance masks kept; noise, texture, progressive and background blur, glass embedded as images; outline text (off: `<text>` with one `<tspan>` per line, one style per layer), unique id attributes from layer names, explicit depth-limit refusal, simplified strokes (stroke attributes for centred strokes and for inside and outside strokes on boxes and ellipses) | — |
| PDF | yes | yes | one page per frame, real selectable text in subset fonts, links, vectors and gradients as vectors, effects as images; bleed, crop/bleed marks, page boxes, naive CMYK; checked by rendering the PDF back against PNG exports; explicit depth-limit refusal | CMYK output intent 2–4 |
| WebP | — | yes (ours) | | — |
| Copy as CSS / SVG / PNG / text / link | yes | yes | Copy as PNG: the whole selection at 2x | — |
| Dev Mode code (CSS, iOS SwiftUI, Android Compose) | yes | yes, plus HTML and React | see [dev-mode-parity](dev-mode-parity.md) | — |
| `.fig` ("Save local copy") | yes | **experimental** | the whole file, also as a copy from File ▸ Save as… (the document keeps its `.figcraft` path and unsaved state); see [Export to `.fig`](#export-to-fig-experimental). Round trips through our own importer; written files checked by opening them in other software: layers, fills, gradients, strokes, shadows, vectors, components and text draw as in FigCraft; resized and scaled instances keep their size and lay their layers out for it, variables with their modes and colour bindings resolve there (a frame set to another mode shows that mode's colours, also inside instances), colour, text, effect and grid styles are listed, and variable-width strokes that are the same on both sides of the path draw with their width | 5–15 (non-colour bindings as bindings, checks of effects, images and prototypes, and against real files) |

### Export to `.fig` (experimental)

`file.export {format: "fig"}` (or a `path` ending in `.fig`), File ▸ Export… ▸ .fig (experimental),
`figcraft-cli export <file> -o <out>.fig` (and `convert`) and MCP's `export {format: "fig"}` write
the whole document; so do File ▸ Save as… and `file.save` / `file.saveAs` with a `.fig` path, as a
copy: the document keeps its `.figcraft` path, unsaved changes and version preview, and a file an
earlier build saved under a `.fig` name is never overwritten by Save (`save_format_tests`,
`save_as_tests`). The writer (`crates/format/src/fig/export.rs`) works from the same public format
notes as the import and inverts its mapping: the zip holds `canvas.fig` (the fig-kiwi header, then a
raw-deflate binary schema and a raw-deflate message), `meta.json`, `thumbnail.png` and
`images/<hash>`; `meta.json` names the file and gives the page colour and the thumbnail size. The
schema written into the file describes only the fields written, under the names, kinds (struct or
message), field ids and enum values the format gives them. The document and each page say they are
visible and opaque (a page also its background opacity): a reader takes what is left out as off, and
draws nothing below a hidden page. The message
lists one node change per layer, parents first, each with its GUID, its parent and a position
string that orders it among its siblings; vector networks and outlines go in blobs, written once
each.

- **Written:** pages (name, background); frames, groups, sections, rectangles (as rounded
  rectangles, with their four radii), ellipses (arcs), polygons, stars, lines, vectors (editable
  network blobs with per-point corner radius, handle mirroring, caps and joins, plus the outline of
  their filled areas), text, boolean groups, slices; the fill outline of frames and shapes and the
  stroke outline of plain strokes (a ring around boxes and whole ellipses, the widened path of other
  centred strokes);
  solid, gradient (handles as the paint transform) and image paints (the image bytes, scale mode,
  rotation, tile scale, crop); strokes (weight, align, join, caps, dashes, miter limit, per-side
  weights); shadows, layer and background blurs (progressive too), noise and texture; corners and
  smoothing, constraints, clip content, masks with their type; auto layout (direction, wrap, gaps, padding,
  alignment, hug, strokes in layout, stacking order, grids with their tracks and cells) and how
  layers sit in it (fill, absolute, min/max sizes); layout grids; export settings; text style
  (font, with styles under their standard names such as "Semi Bold"; size; line height, a share of
  the font size as a raw multiplier; letter spacing, case, decoration, links), alignment, auto resize,
  truncation, paragraph spacing and indent, list types and styled runs with their fills (indexed
  by UTF-16 unit); components (description, boolean, text and instance swap properties and the
  layers bound to them), component sets, instances (their main, property values, what the instance
  itself changes from its main, such as its size, name and paints, as an override addressed by the
  main's id, the fields its layers change, by id path, through nested instances and swaps, and what
  follows from those as derived data: each layer's size, place and outline and the colours its
  variables give in the instance's mode; an instance scaled as a whole says its scale factor);
  variable collections with their modes and variables (colour, number, string and boolean values
  per mode, aliases, descriptions, scopes) as nodes on a hidden page, the variables bound to
  paints, gradient stops, effect and layout grid fields and to layer and text fields (size, gaps,
  padding, radii, stroke weights, opacity, visibility, characters, font and paragraph settings), and
  the modes frames and pages set; shared paint, text, effect and grid styles as nodes on that page
  with the links from the layers that use them (unused styles too); variable-width strokes (each
  point's reach on either side of the path as a share of the stroke weight, half of it for the
  full weight; a stroke wider on one side, on a shape that shows nothing else, as a vector filled
  with the area it paints); prototype
  interactions (clicks, hovers, presses, drags, mouse triggers, delays, keys with modifiers and
  controller buttons; navigate, overlays with their position, outside-click setting and
  background, swap, change to, scroll to, back, close, open link; transitions and easing, custom
  curves included), scrolling and fixed layers, flow starting points.
- **Left out, with a warning each:** variable bindings the format has no field for (component
  properties, arc fields, font weight: written as the values they give), text styles on part of a
  text (written as values), variables' publishing, code syntax and inheritance settings, extended
  collections (written as collections of their own), comments, Dev Mode annotations, statuses,
  resources and code links, plugin data, ruler
  guides, prototype device settings, slot properties and slot content, brush
  strokes (written plain), strokes wider on one side of their path (written as filled outlines, or
  with the width on both sides when the shape has a fill too), pattern fills, WebP export settings (written as PNG),
  image adjustments, glass settings other than frost, springs other than Gentle (written as ease
  out), set-variable, set-variable-mode and conditional actions, scroll-to offsets, text decoration
  looks, variable font axes, text on a path, nested list levels, list spacing, hanging punctuation,
  leading trim, OpenType features, per-layer operations inside boolean groups, grid tracks of fixed
  or content size, different stroke ends on lines. Not written at all: stroke outlines of dashed and
  per-side strokes and of inside or outside strokes on polygons, stars and vectors, glyph outlines
  of text (a reader lays text out with its own fonts), version history and branches.
- **Checked by opening written files in other software:** the example documents and a small test
  file open there, and layers, fills, gradients, strokes, shadows, vectors, components and text draw
  as in FigCraft. A file saved again there and read back keeps the layer tree and properties; two
  differences it showed were fixed (font styles are written under their standard names, and a line
  height that is a share of the font size is written as a raw multiplier, since percent units count
  from the font's own line height); the import now reads per-side stroke weights that leave the
  sides without a border out. Then (2026-10-13), on the example documents and small test files:
  resized and scaled instances keep their size and lay their layers out for it, variables with their modes and colour bindings resolve there (a frame set to another mode shows that mode's colours, also inside instances), colour, text, effect and grid styles are listed, and variable-width strokes that are the same on both sides of the path draw with their width; a nested instance swapped for another variant shows that variant at its size.
  Not working or unchecked there: number, string and boolean bindings and bindings on effects, layout guides and gradient stops come across as the values they give but did not show as bound there (their entries now also name the layer field they set, not yet checked there), and strokes wider on one side of the path are written as filled outlines, no longer editable strokes there (the stroke itself is kept as plugin data and our own import puts it back; not yet checked there); effects such as
  glass and noise, images and prototype interactions are unchecked. Text boxes sized to their content can come out
  a pixel wider or narrower there.
- **Round trips through our own importer:** 60 generated documents (every layer
  kind above, components, nested instances with overrides, a grid inside a component,
  interactions) are exported, read back and compared layer by layer: structure, geometry within
  32-bit float tolerance, paints, text and runs, auto layout, components and prototypes. The written
  file decodes with the generic kiwi decoder and lists every layer after its parent; the encoder
  agrees with the decoder on random messages and refuses values the schema can't hold; junk
  documents never panic (`crates/format/src/fig/export_tests.rs`,
  `crates/engine/src/fig_export_tests.rs`). What other readers need (visible document and pages,
  outlines, text settings, schema numbering, font style names, line height units, mask types) has
  its own tests (`crates/format/src/fig/interop_tests.rs`). Variables, modes, every kind of binding,
  styles, resized, scaled and swapped instances and variable-width strokes are checked field by
  field as written and compared after reading back, files with scrambled values and junk kept strokes never panic
  (`crates/format/src/fig/assets_tests.rs`), and every example document keeps its variables, styles
  and links through `file.export` and `file.open` (`crates/engine/src/fig_export_assets_tests.rs`).

## Fonts

| | Figma | FigCraft |
|---|---|---|
| Local fonts | yes (font helper / desktop app) | yes (system scan) |
| Bundled/shared fonts | Google Fonts in the cloud | optional `CRAFT_FONTS_DIR` build input (storytold/craft-fonts); missing free fonts downloaded from the Google Fonts library on request (Missing fonts ▸ Download, `text.downloadFonts`), into the user's font folder |
| Variable fonts | yes | yes (axes, named instances, per-range axis values) |
| Missing fonts on open | listed, with replacement | listed (Missing fonts window after an open, Text ▸ Missing fonts…, `text.missingFonts`, `figcraft-cli info`); no replacement; text draws in a fallback font |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-17 | minor | Downloading missing fonts (#754): the Missing fonts window's Download button and `text.downloadFonts {families?, confirm: true}` fetch free families (OFL, Apache 2.0, UFL) from the Google Fonts library on GitHub, check each file is a font of the family asked for, save them with their licence under `FigCraft/<family>/` in the user's font folder and draw text in them at once. Nothing is sent without the click or `confirm: true`; `FIGCRAFT_FONT_LIBRARY` names another library or (empty) leaves downloads out. Not in the browser build. |
| 2026-10-17 | minor | Missing fonts (#752): the fonts a file names that aren't installed (text layers, styled ranges, text styles, font family variables in any mode) are listed when the user opens it (once installed fonts are indexed and any import report is closed), from Text ▸ Missing fonts…, by `text.missingFonts` and by `figcraft-cli info`. The row claimed REST import warnings no code produced. |
| 2026-10-17 | minor | The community batches #727 and #728 in the `.fig` row: instance identity and cached library components, overrides by key, text style precedence, grid track sizes, slot content, labels of connectors saved with their path, skip ink, paragraph options, list start numbers, vertical trim, per-point stroke joins, text on a path, image adjustments (also REST), images in the message; budgets from the file's size; the journal keeps no file bytes; the import report. Not read cut to what's left. A `.fig` copy from Save as. Pasted content: layers copied from another design app, best effort (tested only with clipboard HTML made from our own export), and images under Wayland (not run there). All on synthetic files: summary and hours unchanged |
| 2026-10-17 | minor | Hug on import (#732): auto layout frames a `.fig` records no primary-axis sizing for hug on that axis (auto layout sized that axis by its content before it could be fixed) instead of turning fixed, instances that record no auto layout take their main's (they came in without auto layout and fixed, so longer text overflowed them), REST frames whose sizing modes are left out hug (the documented default), and text hugs by its resize mode (auto width both axes, auto height its height; it came in fixed). Numbers unchanged |
| 2026-10-17 | minor | Instance layers' own size and resizing (#731): a `.fig` import keeps the sizes an instance gives its layers and their hug/fill/fixed and min/max as overrides, a REST import reads the resizing fields of `overrides`, and a `.fig` export writes a layer's own size as an override (it went with the derived layout, which a relayout replaces). Numbers unchanged |
| 2026-10-14 | minor | SVG/PDF depth overflow (#208) is checked before bounds and drawing; public checked APIs return the offending root/layer, commands and CLI report it, and no incomplete export replaces an existing file. Tested at depths 255/256/257, with inner roots, hidden branches, masks, raster fallbacks and multiple roots/pages. Numbers unchanged. |
| 2026-10-13 | minor | `.fig` export (gap 1): instances write what they change from their main as an override addressed by the main's id (their size, name and paints were lost elsewhere without it), what follows for their layers as derived data, and a scale factor when scaled as a whole; layers of a nested instance swapped for another component keep their overrides; variable collections, modes, variables and their bindings, the modes frames and pages set, shared styles with their links and variable-width strokes are written, and the import reads all of it back (style definitions and unused styles, variable order and scopes, bindings on paint lists, gradient stops, effects and layout grids, page modes, text field bindings, which a `.fig` import used to drop, and width profiles). Opened `.fig` and file JSON documents link nested instances of variants (and any nested instance under a name of its own) to their main's layers; they stayed unlinked, so a second export lost what they changed. Checked by opening written files in other software: resized and scaled instances keep their size and lay their layers out for it, variables with their modes and colour bindings resolve there (a frame set to another mode shows that mode's colours, also inside instances), colour, text, effect and grid styles are listed, and variable-width strokes that are the same on both sides of the path draw with their width; number, string and boolean bindings and bindings on effects, layout guides and gradient stops come across as the values they give but did not show as bound there (their entries now also name the layer field they set, not yet checked there), and strokes wider on one side of the path are written as filled outlines, no longer editable strokes there (the stroke itself is kept as plugin data and our own import puts it back; not yet checked there). Numbers unchanged |
| 2026-10-12 | minor | `.fig` export made to open elsewhere (gap 1): the document and its pages say they are visible and opaque (left out, nothing was drawn), the schema in the file uses the format's own names, kinds, field ids and enum values, rectangles are rounded rectangles with four radii, frames and shapes carry their fill outline and plain strokes their stroke outline, text carries its layout settings, font styles use their standard names, line heights that are a share of the font size are raw multipliers, masks keep their type (read back too); the import reads per-side stroke weights whose zero sides are left out. Written files were checked by opening them in other software: layers, fills, gradients, strokes, shadows, vectors, components and text draw as in FigCraft; effects such as glass and noise, images and prototype interactions are still unchecked there, and variables are not written. Numbers unchanged |
| 2026-10-12 | minor | Experimental `.fig` export (gap 1): `file.export {format: "fig"}`, File ▸ Export… ▸ .fig (experimental), the CLI and MCP write the whole document (layer tree, geometry, paints with image bytes, strokes, effects, text with styled runs, vector networks, auto layout, constraints, components, sets, instances with their overrides and property values, prototype interactions) with a kiwi encoder that mirrors the decoder and the schema of the fields written; what's left out is a warning. The import gained ellipse arcs, boolean operation types and component set descriptions, and reads durations as whole milliseconds. Verified only by round trips through our own importer. Summary ~59% → ~60%; `.fig` write 15–30 h → 5–15 h (checks against real files) |
| 2026-10-11 | minor | Export and inspect behaviour suite written from the public documentation (139 cases in 19 tests; 47 failed before the fixes) and its fixes: an export covers what the layer paints, so a drop shadow adds room only where it falls, strokes on boxes and ellipses reach their real width (they added up to four times as much), hidden layers take no room and a mask group exports the mask's area; a slice exports everything under it; slashes in layer names export into folders; Copy as PNG copies the whole selection. Added: the export settings' contents only and include bounding box, and for SVG outline text (off writes `<text>`), unique id attributes and simplified strokes (stroke attributes where exact); Inspect colour formats (hex, RGB, CSS, HSL, HSB), CSS in rem, a Blend row, positions measured from the frame around a layer (groups aren't frames), CSS for rotation, blend modes, gradient directions and kinds, text alignment, case and decoration; icons and images in the asset list (`file.exportLayers {assets}`). Summary ~55% → ~56%; SVG options 1–2 h → done |
| 2026-10-11 | minor | `.fig` import's tail (gap 1): vectors open as editable vector networks (points, curved segments, filled areas with their winding, per-point corner radius and handle mirroring, end caps from the end points), falling back to outlines when the network is missing or damaged; sticky notes, shapes with text, connectors, tables, code blocks, slides and similar layers import as the closest frames, text layers and vectors with a warning instead of being dropped; set-variable, set-variable-mode and conditional prototype actions come across. New parsers fuzzed on fixtures generated in code. File formats 55% → 58%, 8–15 h → 5–10 h; import/export breadth 80% → 82%, ready 50% → 54%; feature and area totals and ready for real work (~49%) unchanged; beta ~173–310 h → ~170–305 h; full parity ~321–618 h → ~318–613 h |
| 2026-10-11 | minor | Embedded image colour profiles: placed and opened images are converted once, when decoded, from their own colour space into the file's (sRGB or Display P3): ICC profiles in PNG (`iCCP`), JPG (APP2), WebP and TIFF, matrix/curve and table profiles in RGB or grey (moxcms, already built for the `image` crate), and a PNG's `sRGB`, `gAMA` and `cHRM` chunks; images without a profile, or with one that can't be used (CMYK, damaged, over 4 MB), are taken as sRGB as before. The original file is kept (PDF exports still embed it); pixel exports carry the converted pixels with the file's profile. The image fill's details show the source profile (Unknown — treated as sRGB when there's none) and `image.place` reports it as `imageProfile`. |
| 2026-10-11 | minor | `.fig` import, best effort (gap 1): `.fig` files open (best effort): the zip, its deflate/zstd chunks and the kiwi schema are decoded generically and mapped by field name onto the REST importer, so pages, the layer tree, geometry, paints (styles resolved), strokes, effects, text with styled runs, vectors (as outlines), auto layout (grid included), constraints, components, sets, instances (expanded from their main with overrides, derived layout and property values), styles, variables with modes and bindings, prototyping and image fills from the archive come across; anything else is a warning. Summary ~40% → ~55%; `.fig` read 30–60 h → 8–15 h (write 15–30 h unchanged) |
| 2026-10-11 | minor | Animated GIF fills and TIFF images (gap 12, without video and HEIC): GIF fills keep the original file, show the first frame on the canvas and in exports and play in the presenter and the preview (frames decoded on first play, at most 500 frames and 128 MB; always looping; the window wakes only for a GIF's next frame); TIFF images open, import, place and paste. Paint breadth 85% → 87%, 5–9 h → 4–7 h; gap 12 6–12 h → 5–10 h (video playback and HEIC left, both needing non-Rust decoders); feature breadth (~79%), ready for real work (~47%) and full parity hours unchanged at this rounding |
| 2026-10-10 | major | Created from the 2026-10-10 re-measure |
