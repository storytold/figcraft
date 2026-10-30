# FigCraft architecture

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-18 · **Change:** minor (the `vectordrawable` crate, L4: Android Vector Drawable export; earlier: the shared hub in `collab-net` and the `figcraft-server` app) · **Target:** Figma Design

How FigCraft is built today. Engine first, a thin swappable UI, everything a command; a Figma-shaped
data model (a scene graph of frames with auto layout, components and variables). ~108,800 lines of
Rust (2026-10-10).

## Crates and layers

`cargo xtask layers` enforces the layering (`xtask/src/layers.rs`): a crate may depend only on
lower layers. Nothing below L6 depends on egui, eframe, rfd or winit, so the UI crate can be
replaced.

| Layer | Crate | Lines | Role |
|---|---|---|---|
| L0 | `figcraft-geom` | ~0.5k | Affine transforms, shape path builders (rounded and smoothed corners, polygons, stars, arcs) on kurbo |
| L0 | `figcraft-color` | ~1.3k | Colours (HSB/HSL, LCH/OKLCH, hex/CSS, Display P3), paints (solid, gradients, image), blend modes |
| L1 | `figcraft-doc` | ~8.5k | Document model: pages, node tree in a persistent map (`imbl`, stable ids), properties, components, styles, variables, prototype data, comments, versions. Serde everywhere |
| L2 | `figcraft-layout` | ~1.8k | Auto layout (horizontal, vertical, wrap, grid; hug/fill/fixed; min/max), constraints, layout grid geometry |
| L2 | `figcraft-text` | ~3.1k | Font database (optional embedded craft-fonts + system scan), shaping (harfrust), bidi, line layout, glyph outlines (skrifa), variable fonts |
| L2 | `figcraft-collab` | ~3.4k | Real-time collaboration core (no networking): synced object/property model, change sets, fractional positions, server ordering and validation, client reconciliation, wire messages |
| L2 | `figcraft-plugin` | ~1.0k | Plugin manifest and the WebAssembly sandbox (wasmi, metered, memory cap, no file system, network or clock) |
| L2 | `figcraft-pathops` | ~1.1k | Booleans (linesweeper), outline stroke with alignment, arrowheads |
| L3 | `figcraft-render` | ~5.4k | Scene → pixels on tiny-skia (CPU): isolated layers for opacity/blend/effects, clips, masks, shadows, blurs (uniform, progressive, background), noise, texture, glass, text; banded in parallel; retained rendering of only what changed; levels of detail when zoomed out |
| L3 | `figcraft-format` | ~5.2k | Native `.figcraft` (zip of `document.json` + images; streamed JSON), local library files, versions, REST API file JSON import (`rest/`) |
| L4 | `figcraft-svg` | ~3.7k | SVG export and import (usvg) |
| L4 | `figcraft-pdf` | ~2.3k | PDF export (krilla) with print options |
| L4 | `figcraft-gpu` | ~2.1k | The window's GPU: effects (the same box-blur the CPU renderer uses, glass) through wgpu, plugged into `figcraft-render`'s accelerator, and the GPU canvas — document → vello scene → texture, with effect layers drawn and blurred on the device, and pages with glass or a background blur drawn in accumulated segments ([`gpu-rasterizer.md`](gpu-rasterizer.md)); exports, tests and machines without a device keep the CPU |
| L4 | `figcraft-vectordrawable` | ~1.2k | Android Vector Drawable (XML) export: baked path data, gradients through `aapt:attr`, clip groups, compatibility modes and warnings |
| L5 | `figcraft-engine` | ~39.3k | Sessions, undo history (`Arc` snapshots), the command registry (`cmd/*`, `spec!`), pointer tools and vector edit mode, snapping and measuring, components sync, libraries, prototype runtime, Dev Mode code generation, the menu catalog (`catalog.rs`) and the keymap (`keymap.rs`), a panic guard |
| L6 | `figcraft-ui-egui` | ~31.9k | The UI3-style frontend: canvas (render worker), toolbar, layers, Design/Prototype/Inspect panels, colour picker, menus, dialogs, presenter, comments, accessibility (AccessKit), the control channel server |
| L6 | `figcraft-mcp` | ~2.4k | MCP server (JSON-RPC over stdio), headless or bridged to a running app |
| L6 | `figcraft-collab-net` | ~3.0k | Live sessions over the network: the WebSocket transport (tungstenite, a reader and a writer thread per connection), the hub that runs a collaboration core server per open file for the session host the app embeds and for `figcraft-server` (behind a `Backend`), the joining and managing clients (publishing), invite links, presence relay and limits; the browser client; the only crate allowed networking dependencies |
| apps | `figcraft`, `figcraft-cli`, `figcraft-web`, `figcraft-server` | ~3.2k | Desktop app (eframe + wgpu), command line, browser build (WASM, WebGPU/WebGL2), collaboration server (files, share links, persistence; [`server.md`](server.md)) |
| plugins | `plugins/` (own workspace) | | Rust plugin SDK and two example plugins, built for wasm32 by `cargo xtask plugins` ([`plugins.md`](plugins.md)) |
| tools | `xtask` | | `ci`, `layers`, `assets`, `parity`, `version`, `icon`, `wasm`, `plugins`, `bench` |

## Data model

A `Document` holds pages, a persistent map of nodes (frames, groups, sections, rectangles,
ellipses, polygons, stars, lines, vector networks, text with styled runs, boolean groups,
components, component sets, instances, slices), styles, variable collections, comments, versions
and images by hash. Instances keep their main's id and per-layer overrides; the engine keeps their
materialised children in sync when a main changes (directly for value, size and add/remove edits,
by a rebuild otherwise). Snapshots are cheap (`Arc` and structural sharing), which gives unlimited
undo with one step per gesture.

## Commands: one path for every surface

Every user-visible behaviour is a command in `crates/engine/src/cmd/*` (455 ids after #101)
with an id, label, parameter docs and undo flag. The menus (generated from the catalog), keyboard
shortcuts, Actions palette, CLI (`figcraft-cli run`), JSON control channel (`--control <port>`) and
MCP server all dispatch the same ids, so agents can do anything a person can; WebAssembly plugins call them too, within their manifest's permissions. Commands never open
dialogs themselves; they request UI, and programmatic callers drop those requests.
`cargo xtask parity` measures the catalog's coverage ([`parity-checklist.md`](parity-checklist.md)).

## Rendering

The desktop canvas draws on the GPU, when Preferences ▸ Draw the canvas on the GPU is on and the
page is in the GPU rasterizer's feature set
([`gpu-rasterizer.md`](gpu-rasterizer.md)): the render thread turns the page into a vello scene and
draws it into one of two textures, and the window paints that texture, so no pixels cross the bus
and nothing is uploaded per frame. A node with a single layer blur is drawn into an offscreen
texture, blurred by the GPU's own box passes (the CPU blur's radii) and composited there too; a
card's drop shadow (spread, several of them, any blend) is vello's blurred rounded rect of the
card's own silhouette, drawn under the card, and the card's strokes are the renderer's own band
filled; and a page with a backdrop effect (glass, background blur) is drawn in segments: the layers
before such a node accumulate in a texture, the node's effect is made from that texture (a crop and a blur, or the
CPU's pane geometry through the GPU's glass shader) and its content goes over it, so the page under
two panes is drawn once. vello's textures hold straight alpha, so a segment is premultiplied as it
is laid over the accumulation and an effect layer before a blur or the glass shader reads it.
Everything else — a paint's own plus-darker, a plus-darker layer under a backdrop effect, an
animated image, a progressive blur whose layer is past 16 M pixels, a skewed linear-gradient
handle (imported files only) — stays on the CPU renderer and is uploaded as before, as do the
editor's own views (outline modes, skipped layers, the focus view, soft proof, Display P3
documents); the choice is made per frame and reported by `ui.inspect`. The
CPU renderer is the single source of pixels for exports and the browser. The desktop canvas
renders on a worker thread at the device pixel ratio and keeps its last image, re-rendering only
the region a pan uncovers or an edit touches; overlays (selection, handles, guides, labels) are
drawn by egui on top. A seeded random walk checks the retained canvas against full renders.
Within that region, isolated layers (opacity, blend modes, effects) are rasterised on the page's
pixel grid and kept between frames (`figcraft_render::layers`), so dragging a few layers across a
page full of shadows, blurs and glass re-composites the rest instead of drawing it again; only the
GPU canvas's fallback keeps them (`Options::doc_id`), so with the preference off the canvas is the
stateless renderer it was; layers that read the backdrop (glass, background blur) or start from it
(masks) are always drawn. The window's GPU draws raster effects only inside a canvas frame
(`Options::accelerate`): exports, thumbnails and the CLI use the CPU's. SVG and
PDF export walk the same scene and embed rasters where those formats can't express an effect.

## File I/O

`.figcraft` is the native format (atomic saves, autosave checkpoints; the desktop app keeps
recovery copies of unsaved documents that reopen after a crash or a forced quit). Imports: REST API
file JSON with variables and images, SVG, raster images, clipboard
content. Exports: PNG, JPG, WebP, SVG, PDF, code. There is no `.fig` reader yet
([`file-format-parity.md`](file-format-parity.md)).

## Robustness

Workspace lints deny `unwrap`, `expect`, `panic!` and friends in shipped code; `unsafe_code` is
forbidden. `figcraft_engine::guard` turns a panicking command into an error and rolls the document
back, and a panic in a `Session::grouped` closure outside any command rolls the group back like a
failure before the panic continues; the desktop app also guards each UI frame. The browser build can't catch a panic (wasm32 aborts), so there a panic stops the page and its panic hook keeps the files with unsaved changes in local storage for the next page. Junk-parameter sweeps run every command with bad
input.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | Added `vectordrawable` (L4), the Android Vector Drawable (XML) writer |
| 2026-10-11 | minor | `collab-net` hosts through one hub for the app's sessions and the standalone server; added the `figcraft-server` app |
| 2026-10-10 | minor | Merged main after #101–#103: WASM plugins (plugins area 0% → 35% breadth, 20% ready; ecosystem 5% → 20%) and the multiplayer core without networking (multiplayer 5% → 15% breadth, 5% ready; collaboration 8% → 12%); menu breadth 97.5% → 98.5% (200/203); ready for real work ~46% → ~47%; full parity 420–790 h → 390–745 h; beta 210–380 h → 200–365 h. Fixed the stated weight sums (areas 1–16 sum to 94, total 113; percentages were already computed with those) |
| 2026-10-10 | major | Created from the code as of PR #99 |
