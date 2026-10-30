# GPU canvas rasterizer (vello)

Plan for moving the canvas from CPU rasterisation (tiny-skia) to a GPU rasterizer, and the
tests that gate it. Exports, the CLI, MCP, tests, the browser build and machines without an
adapter stay on the CPU renderer: `figcraft-render` remains the reference implementation and the
fallback, not a second product.

## Why

Measured on the owner's machine (Intel Arc A750, release build, control channel, 2026-10-24):

| canvas frame | before the layer cache | after the layer cache |
|---|---|---|
| drag a glass pane, 2400 × 1600 | 95–107 ms | 10–13 ms |
| drag a shadowed card under it | ~95 ms | 15–20 ms |
| same scene without glass | ~65 ms | ~1 ms |
| drag a plain rectangle, 2320 × 1440 | 3.6–4.8 ms | 3.6–4.8 ms |
| drag a layer-blurred rectangle | 6.7–7.2 ms | 6.7–7.2 ms |

The last two rows are the ceiling of the CPU pipeline: a plain rectangle and a blurred one cost
almost the same because the frame is dominated by work that does not depend on what is dragged —
rasterising the whole 2320 × 1440 view, converting it to an `egui::ColorImage`, and uploading the
whole texture (~13 MB) every frame. No amount of effect tuning removes that fixed cost. A GPU
rasterizer does: the scene is drawn straight into a texture the UI samples, with no per-frame
pixels crossing the bus.

## Decision

- **vello 0.11** (`wgpu ^30`, `peniko 0.6`, `kurbo 0.13`): the same wgpu major version as
  eframe 0.36 / egui-wgpu 0.36, so it renders with the window's own `Device`/`Queue` and its
  output can be shown by egui without a read-back. `vello_hybrid 0.2` is on wgpu 29 and is not an
  option. `vello_cpu` is a CPU renderer and does not help.
- The GPU path lives in `figcraft-gpu` (L4). It draws a page region into an `Rgba8Unorm` texture
  (`STORAGE_BINDING | TEXTURE_BINDING | COPY_SRC`) sized to the view's device pixels.
- The canvas shows that texture through `egui_wgpu` (`register_native_texture`), not through a
  `ColorImage`. Overlays (selection, handles, snap guides) stay egui shapes on top.
- **All-or-nothing fallback.** For a page with something the GPU path cannot draw *identically*
  (a feature still in a later phase), the canvas renders that frame on the CPU as today. A
  half-GPU frame is never shown. The predicate is one function over the page subtree
  (`figcraft_gpu::scene::supported`) plus `scene::plan` for backdrop effects, extended phase by
  phase with a test either way.
- Determinism: the GPU texture is read back only for screenshots, exports and tests. The parity
  suite compares GPU and CPU pixel by pixel with a stated tolerance.

Invariants:

- Same `View`, same device-pixel snapping, same LOD decisions as the CPU path.
- Nothing below L6 depends on egui; `figcraft-gpu` depends on wgpu only. wasm stays CPU
  (`vello`'s wasm targets are WebGL2-only and compute is unavailable there).
- Every phase lands with `cargo xtask ci` green, layered (`cargo xtask layers`), and the canvas
  visibly identical in a screenshot comparison.

## Phases

### P0 — feasibility (dependency, device, handoff, read-back)

- Add `vello = { version = "0.11", default-features = false, features = ["wgpu"] }` to
  `figcraft-gpu`; it must resolve against the workspace's wgpu 30.
- `Gpu::canvas()` builds a `CanvasGpu` on the window's device and queue (once, next to the effects).
- Prove the texture can be shown by egui-wgpu without a copy and read back for screenshots.

Exit: a test renders a fixed scene through vello on the machine's adapter, reads the texture back,
and the app builds and runs unchanged (the CPU canvas is still the default in this phase).

### P1 — scene builder and parity

- `figcraft_gpu::scene`: document page + `View` → `vello::Scene`, reusing
  `figcraft_render::node_outline` (shapes, vectors, booleans, text as glyph outlines) and the
  document's paints.
- Covered in P1: rectangles (per-corner radii, corner smoothing), ellipses/arcs, polygons, stars,
  lines, vectors and booleans (via `node_outline`), text in one style (glyph outlines), solid and
  linear/radial/angular gradients, node opacity, node blend modes, clipping frames, hidden layers.
- Not covered (the page falls back to the CPU): effects that are not a layer blur, a drop shadow or
  a backdrop effect (inner shadows, grain, a node with several effects), a shadow cast by anything
  that isn't one rounded rectangle (a group, an ellipse, a non-uniform radius), image and pattern
  fills, diamond and skewed linear gradients, per-paint blend modes,
  `PlusDarker`, masks, text with a stroke, arrow/circle caps, text with styled runs or colour
  glyphs, and the editor's own options (outline mode, `skip`, `solo`) which come with the canvas
  integration.

Exit: parity tests (below) pass for the P1 feature list; a drag benchmark shows the frame cost
at 2320 × 1440 well under the CPU's fixed 3.6–4.8 ms.

### P2 — canvas integration

- The render thread owns the `CanvasGpu` (the window's device and queue) and draws each job it can
  into one of two textures; the UI thread registers that texture with `egui_wgpu` (once per buffer
  and generation) and paints it in the canvas' rect, with the usual re-projection. No pixels cross
  the bus, and the canvas never uploads an image for a GPU frame.
- Double buffering: the thread draws into one buffer while the window samples the other, so a
  frame on screen is never half-overwritten. A resize (a zoom) reallocates both and bumps a
  generation; the UI frees the textures it registered for the old one.
- The GPU path is chosen per frame: pages in the builder's feature set go to the GPU, everything
  else to the CPU as before, so nothing that worked changes. Outline mode, outlines for the
  selection, skipped layers, the focus view, soft proof and non-sRGB documents stay on the CPU.
- `ui.inspect` reports the choice and the reason for a fallback (`canvas.rasterizer`,
  `canvas.gpuRefusal`), so a scene that stays on the CPU can be explained without a debugger.
  Preferences ▸ **Draw the canvas on the GPU** (`pref.gpuCanvas`, off by default) turns the GPU
  canvas on for a machine; the CPU renderer is the reference and the fallback.

Exit: dragging any page in the P1 feature set is GPU-rendered in the real app; screenshots match
the CPU render within the parity tolerance; toggling the setting changes nothing visible.

### P3 — effects on the GPU

- Landed: **layer blur**. A node with one non-progressive layer blur is drawn into its own
  offscreen texture, blurred by the same three box passes as the CPU renderer (on the GPU), and
  composited with the node's opacity and blend mode; the pixels never leave the device. Parity
  (`the_gpu_layer_blur_matches_the_cpu_render`): mean channel difference 0.047/255, worst 33 on a
  blurred antialiased edge.
- A box pass is **O(1) a pixel whatever the radius**, in **one dispatch** (since 2026-10-26): a
  workgroup takes 256 pixels of a line with the halo their windows read (up to 256 pixels a side),
  scans the halo in shared memory, and writes the averages straight into the next texture
  (`blur.wgsl`'s `box`) — no running-sums texture between passes. A radius past the halo (a blur
  wider than ~500 document pixels at 100%) falls back to `scan` + `fs_box`: the running sum along
  each line, and the window's average as the difference of two of them. Before, each pixel read its
  whole window, so a 0 → 300 progressive blur read some 5 × 10⁸ texels a frame and a weaker GPU
  dropped to a crawl; now the radius costs nothing (numbers under P7). A device without compute
  falls back to reading the window directly.
- Landed: **drop shadows** on rounded rectangles and frames (the common card): the shadow is vello's
  blurred rounded rect — the card's own outline, offset and blurred — drawn under the card, which
  hides it exactly where the CPU masks it. No offscreen layer, so a page of shadowed cards costs
  almost nothing. Parity (`the_gpu_drop_shadow_matches_the_cpu_render`): mean 0.044/255, worst 31
  (vello's Gaussian against the CPU's three-box approximation).
- Landed: **shadows of any number and shape the card itself is**: a shadow that *spreads* grows the
  rounded rect (and its radius) by the spread — the CPU's mitered silhouette dilation; one that
  shows behind is hidden by an opaque card either way; one with a blend mode of its own composites
  as a layer; the card may have any number of shadows, a gradient fill (its stops opaque), an
  opaque stroke (which grows the silhouette by its outer half — inside, centred or outside), rotated
  corners, and children in a clipping frame. `the_gpu_draws_the_shadow_variations` covers all of
  them: mean 0.558/255, worst 59 on the softest shadow edge, 1.5% of pixels off by more than 8 —
  the Gaussian-against-three-boxes difference, largest where two large shadows overlap. A
  **translucent card** (one fill alpha, so one shadow strength) casts a shadow as faint as it paints
  and clipped away where the card covers, as the CPU's cover mask hides it — except when the shadow
  spreads, which the CPU casts from the paints' outline, opaque, so the shadow is full strength.
  Still on the CPU: a non-uniform radius, an ellipse, a group or text with a shadow, a translucent
  fill beside a stroke or fading across the shape, and a shadow alongside another effect (the CPU's
  order matters).
- Landed: **inside, outside and per-side strokes, dashes, width profiles and brush strokes**, as
  the renderer's own band (`figcraft_render::stroke_fill`) filled — vello's native stroke draws the
  centred, plain case. Text with a stroke still falls back.
- A **linear gradient whose width handle isn't square to its axis** (only imported files skew one)
  stays on the CPU: vello's linear gradient is a projection onto its axis, the CPU inverts the full
  handle frame, and the two disagree on such a gradient's bands.
- Landed: **background blur**: the page under the node is drawn into a view-sized texture (the
  node and everything after it left out — the renderer's `backdrop::above`), blurred by the GPU's
  box passes, shown clipped to the node's shape, and the node's content goes over it. Parity mean
  0.049/255, worst 25.
- Landed: **glass**, from the same backdrop: the padded region around the pane is cropped into a
  texture, blurred by the pane's frost and handed to the existing glass shader (the CPU still
  computes the cached distance field and normals — `figcraft_render::glass::pane`), into a texture
  drawn under the pane's content. Parity mean 0.061/255, worst 6, no pixel differs by more than 8
  (`the_gpu_glass_matches_the_cpu_canvas`).
- The single-scene route for a backdrop node (draw the page under it, blur, draw the node over it)
  came first and is still the builder's own path and the parity reference
  (`the_gpu_glass_matches_the_cpu_canvas`, `the_gpu_background_blur_matches_the_cpu_render`); the
  canvas takes the segmented route for every page that has a backdrop node.
- Landed: **the accumulated-backdrop renderer.** A page with a backdrop node is drawn in segments:
  the layers before such a node go into a segment texture, the segment is laid over an
  accumulation texture, the effect is made from the accumulation (a blur of the node's padded
  layer, or a crop + blur + the glass shader) and the node's content goes over it. The page under
  two panes is drawn once, so panes no longer compound.
- **Everything that reads its backdrop now stays on the GPU**, as many panes and blurs as the page
  has, nested in containers too (P10): `scene::plan` refuses nothing for a backdrop effect any more;
  a container with one nested inside it gets a step of its own (P11) and the builder re-renders what
  lies under the node.
- Parity: glass mean 0.061/255, worst 6, no pixel differs by more than 8
  (`the_segmented_page_matches_the_cpu_with_one_glass_pane` — the same numbers as the single-scene
  path); background blur mean 0.049/255, worst 25
  (`the_segmented_page_matches_the_cpu_with_one_background_blur`). The heavy page (30 shadowed
  cards, 3 panes, a background blur and a layer blur) matching to mean 0.725/255, worst 40, all of
  them on antialiased card edges (`the_gpu_page_matches_with_several_backdrop_layers`).
- *Timing* (`segmented_timings`, release, pipelined as the app draws): one pane 904 × 860 CPU
  8.3 ms / GPU 5.7 ms, 2400 × 1600 CPU 14.3 / GPU 3.3; one background blur 4.3 / 3.8 and 11.9 /
  3.2; the heavy page 48.6 / 8.8 and 53.6 / 6.2 — under the 8 ms budget at 2400 × 1600.
- *Timing* (`shadow_timings`, release): 30 cards with radius-12 shadows **0.84–0.90 ms** a frame on
  the GPU (CPU 21–29 ms); with a spread of 6, **0.69–0.91 ms** (CPU 28–34 ms) — a spread shadow is
  still one rounded rect, not a layer.
- In the app (release, control channel, canvas 904 × 860, the same page): the GPU holds
  `canvas.rasterizer = gpu` and drags it in 12–19 ms a frame where the CPU's *full* render takes
  30–34 ms; a single-layer drag was 4.3–4.5 ms on the CPU's retained layers against 12–19 ms on the
  GPU, which re-encoded the page every frame. P11 has since kept the page's steps between frames
  (see P11 below): a single-layer drag on the corpus pages is now 0.9–10.9 ms, at or under the
  CPU's retained renderer on the content-dominated pages and still 2–3× it on backdrop pages.
  Its canvas *latency* was already lower in the app (17.8 ms against the CPU's 41 ms in the same
  session), because no pixels cross the bus.
- Then: inner shadows, grain, spread and translucent shadows, and drop shadows cast from blurred
  layers — all landed under P6/P7.

Exit: the 30-shadow + 3-glass + 2-blur page renders entirely on the GPU, and its drag frame at
2400 × 1600 stays at or under the 10–13 ms the CPU path reaches with the layer cache, at
full quality — the target is a single frame budget (≤ 8 ms p95) at that size. **Met** in isolation:
6.2 ms at 2400 × 1600 (`segmented_timings`; the machine's GPU throughput moves 2–3× with what else
is running, so compare runs made together, not across sessions).

#### Straight vs premultiplied alpha (what broke the first attempt)

A first implementation matched the CPU inside every effect (mean 0.06–0.11/255) but failed at
their borders: a pane's rim showed the page background instead of the stripe under it, and a
background blur's edge showed blur where the CPU shows none (worst 123–223 on 250–1600 pixels).

The cause was neither the compose pass nor the image's placement: **vello's targets hold straight
(separated) alpha** — its fine pass stores `rgb / alpha` (`vello_shaders`' `fine.wgsl`, and
`Renderer::register_texture` says its textures are read as unpremultiplied) — while the
accumulation, the box blurs, the glass shader (its `min(rgb, alpha)` clamp) and egui all work on
premultiplied pixels, as tiny-skia's pixmaps do. A segment's antialiased rim therefore held
`(255, 255, 255, 0)`, and a premultiplied-over blit added all of that white to the page.

The canvas now premultiplies once, where vello's output meets the premultiplied side: the compose
pass premultiplies each segment as it lays it over the accumulation, and `render_layer`
premultiplies an effect layer before a blur, the glass shader, or vello's own image brush reads it.
The same conversion removed the differences the layer-blur parity test used to have on its
antialiased edges (mean 0.047 → 0.011/255, worst 33 → 7).

#### Why the segments read only the node's padded layer

The CPU blurs `back`, the node's layer grown by `pad` each way — not the whole view. Cropping to
that rectangle first and blurring the crop is what the segmented canvas does now (`blur_rect`),
which is both the same arithmetic and much cheaper: the background-blur page at 2400 × 1600 went
from 11.6 to 3.2 ms a frame.

### P4 — opt-in, cleanup

- **Landed.** The GPU canvas is off by default: Preferences ▸ Draw the canvas on the GPU
  (`pref.gpuCanvas`) turns it on, a page outside the feature set stays on the CPU for that frame,
  and the fallback, wasm and export policy are written here and in `docs/architecture.md`;
  `docs/hardware-parity.md`, `docs/gaps.md` and `ROADMAP.md` carry the numbers.
- **It can't panic on a texture it can't make.** Every texture is checked against the device's
  `max_texture_dimension_2d` first (`canvas::fits`); a device whose limit is under 8,192 px — the
  cap the scene keeps effect layers to — leaves the whole canvas on the CPU, and a layer, shadow,
  blur or glass texture the device can't hold refuses that frame (`canvas.gpuRefusal` says so).
- **The CPU renderer stays stateless in the browser.** The retained layer cache (`render::layers`)
  is a desktop canvas optimisation; the web build renders with `doc_id = 0`, keeping nothing.
- **An image that arrives after a frame was drawn redraws it.** Both the kept layers and the kept
  frame hash the document's image store (`layers::images_id`), so a collaborative image entry or an
  import finishing is never served from stale pixels.
- **Nothing CPU-only was removed, on purpose.** The CPU renderer is still the oracle for the parity
  suite, the source of every export, the browser's canvas, the fallback for refused pages and the
  reference for what the GPU's own retention should cost (it keeps effect layers and repaints only
  damaged pixels; the GPU keeps steps, see P11). The single-scene backdrop route in
  `scene::build_with` is kept as the parity reference for each effect, though the canvas always
  segments a page that has a backdrop node.
- What is left is the full-coverage program below (P5–P11), also listed in `docs/gaps.md` (gap 7).

## Full GPU coverage (P5–P11)

Everything the CPU canvas draws, the GPU canvas should draw too — exports, the parity suite, the
browser and machines without an adapter keep the CPU renderer, but no *page* should fall back
because of what it contains. In the order the work pays off:

### P5 — paints: images, patterns, diamond gradients, per-paint blends

Any file with a photo, a logo or a pattern currently re-renders the whole page on the CPU. The
renderer already decodes, filters (brightness, contrast, saturation, exposure, blur) and caches
every image pixmap; the GPU only has to draw it.

**Landed.** A paint that isn't a solid or a linear/radial/angular gradient is handed to
`figcraft_render::paint::prepare_at`, which returns the renderer's own prepared pixmap (decoded and
filtered image, a pattern's cell, a diamond gradient's 256 × 256 tile), its extend mode, filter
quality, opacity and image-pixel → local transform. The builder uploads that pixmap once (vello's
image atlas keeps it while the same `Blob` is asked for; the `Arc` is cached, so a photo's pixmap —
already cached by the renderer — is the same one every frame) and draws it as vello's image brush:

- **Images**: Fill, Fit, Crop, Tile, rotation and crop, the renderer's padded 1-px border for Fit and
  Crop (so `Pad` spreads transparency, not edge pixels), `Extend::Repeat` for tiles. Filtered and
  translucent images work: the filters are in the pixmap, the paint's opacity is the sampler's
  `alpha`.
- **Pattern fills**: the renderer's cell pixmap, `Extend::Repeat`, bilinear.
- **Diamond gradients**: the renderer's tile through its handle frame, `Extend::Pad`.
- **Per-paint blend modes**: a fill or stroke with its own blend gets a vello layer of its own, as
  the node's blend already did. `PlusDarker` still falls back (P8).

Resampling is the one difference: vello's image sampling is nearest (`ImageQuality::Low`) or
bilinear (`Medium`) — there is no bicubic — so a photo drawn much smaller than its pixels samples
four texels where tiny-skia's bicubic samples sixteen. Measured on a page of 1:1, fitted (2×
downscaled), cropped, tiled, rotated and filtered photos, a diamond gradient, a pattern fill and
multiplied/screened paints: mean 0.066/255, worst 10 on one channel, 5 of 450,000 pixels off by
more than 8 (`the_gpu_draws_the_paint_variations`).
- Animated GIFs (the only animation the renderer plays) stay on the CPU: the canvas refuses a page
  whose images animate, since a frame changes under a document that did not.

Tests: parity per scale mode, per filter, with rotation and crop; a pattern tile; a diamond
gradient; a multiplied fill (`the_gpu_draws_the_paint_variations`). Exit: a page of photos, logos
and pattern fills draws on the GPU within the page tolerance. **Met.**


### P6 — any-outline shadows, inner shadows

A shadow or an inner shadow cast by anything that is not one rounded rectangle still sent the page
to the CPU: an ellipse, a star or vector, text, a card with per-corner radii, a group. The CPU draws
these from the layer's pixels (or the paints' outline) with the box blur; on the GPU they need one
offscreen layer per such node:

**Landed.** The builder classifies a node's effects: drop shadows it can draw straight into the
scene stay the fast path (P3); every other drop shadow — and every inner shadow — is one layer the
GPU renders, blurs with the same three box passes as the CPU, and tints (`shadow.wgsl`, in
`crates/gpu/src/shadow.rs`):

- **Silhouette** (spread, and the node's paints are one outline: ellipse, star, polygon, vector,
  boolean, text, per-corner rect): the outline in white, grown by the spread (a mitred band of twice
  the spread, as the renderer's `spread_alpha`) or shrunk (the band cut out with a `DestOut` layer),
  offset by the shadow's device offset; blurred, tinted, drawn under the node and clipped outside the
  paints' cover — the CPU's `mask_by(cover, inverted)`.
- **Pixels** (a group, a frame whose children reach past it, a spreading shadow with no single
  outline, a translucent fill beside a stroke, a fill whose alpha varies): the node's own content
  into a layer, its alpha grown or shrunk by `whole_px(spread)` with a square kernel (`fx::morph`,
  as two passes of a max/min filter), offset by whole pixels, blurred and tinted. The shadow is
  drawn under the content, which hides it exactly where the content does — the CPU masks it by the
  layer's alpha, and `content over shadow·(1−a)` is the same arithmetic.
- **Inner shadow**: the shape at the shadow's offset in a buffer padded by its reach, shrunk by the
  spread (a `DestOut` cut plus the band painted back), inverted (white outside the shape), blurred,
  tinted, and drawn clipped to the shape — over the fills, under the children and strokes, where the
  CPU puts it.
- A shadow keeps its own blend mode (a layer), its `showBehind` (no cover clip) and the node's
  opacity (baked into the tint's colour alpha), and is cut to the node's paint bounds, as the CPU
  crops it to the layer. The layer rectangle is the renderer's own (`figcraft_render::layer_bounds`:
  the paint bounds *without* the effects' margin), because that crop is visible: a frame whose child
  reaches past it casts a shadow that stops at the child, exactly as the CPU's does.
- **Kept between frames.** A shadow is a function of the node, its subtree, the view and the zoom —
  and a node is a shared `Arc`, so an edit is a new address — so the canvas keys its pixels on
  exactly that (`(u64, u64)`, the renderer's `layers::subtree` idea, plus the document's page ids and
  node count so another file can't match) and keeps up to 64 MB of them. A drag then rebuilds one
  shadow and draws the rest as they are: the second frame of `the_gpu_draws_shadows_of_any_shape`
  is pixel-identical to the first with 10 of its 10 shadows kept, and editing one node adds exactly
  one new shadow.

Still on the CPU at this point: a group's inner shadow (cast from its pixels, not one outline),
which P7's layer path now draws, and a node with a shadow *and* another effect (the CPU's order).

Tests: parity for an ellipse, a star, a polygon, a per-corner card, text, a group and a frame with
overflowing children; each with and without spread; an inner shadow on a card, on a per-corner card
and on an ellipse (`the_gpu_draws_shadows_of_any_shape`): mean 0.066/255, worst 50 on antialiased
shape edges, 0.28% of pixels off by more than 8.
Timing (`shadow_layer_timings`, release, 30 cards): from 1.2 ms to 2.4 ms a frame at 904 × 860 and
2400 × 1600 once the shadows are kept, against the CPU's 17–43 ms. The *first* frame makes all
thirty — one vello layer, a blur and a tint each — and costs 90–170 ms, which is the one-off the
cache pays for; fusing those passes into one submission is P11 work.
Exit: no shadow of any kind falls back — **met**: every drop shadow, every inner shadow on a shape
and (since P7) a group's inner shadow draw on the GPU.


### P7 — effect combinations, grain, progressive blur

The CPU applies a node's effects in list order: inner shadows, then the layer blur, grain and
texture, then drop shadows, over the backdrop read. A node with two kinds used to fall back whole.

**Landed.** A node whose effects are any mix of inner shadows, a layer blur, grain, texture and
drop shadows is drawn as one layer, in that order (`Builder::layer_pipeline`):

- The content (with its inner shadows, drawn in `content`) goes into a layer of its own.
- Each **layer blur** in the list is applied in order: a uniform one is the GPU's box blur, a
  **progressive** one is the renderer's own levels — `figcraft_render::blur::levels` of uniform
  blurs, mixed per pixel by `mix.wgsl` from the ramp (`bracket`'s two levels at that pixel) — so a
  ramp of any steepness costs at most eight blurs and one pass. Each level is blurred **only in the
  band that uses it** (the ramp's deviation between its neighbours', plus the blur's reach), as the
  renderer crops its own: a 0 → 300 ramp on a 200 × 300 card blurs 1.5 M pixels across all eight
  levels instead of 10.6 M, and the blend reads each level at its own top-left
  (`mix.wgsl`'s offsets; `a_progressive_level_blurs_only_its_band`).
- **Grain** is the renderer's own noise: the layer's pixels are read back once, `grain::grains`
  works them out on the CPU (the grains are a hash of the cell and the node, so they land exactly
  where an export's do), and the pass draws them over the layer in the noise's blend mode.
- **Texture** is the renderer's own scatter: the layer's pixels are read back once and
  `grain::texture` moves each grain's pixel by its hashed jitter, so the rough edge lands where an
  export's does; the result replaces the layer.
- The **drop shadows** are cast from that result — the outline grown by the spread, or the layer's
  pixels (`shadow_of_layer`) — and drawn under it, then the layer over them, at the node's opacity
  and blend mode.
- A **group's** inner shadow is cast from the layer's own pixels — the renderer's
  `inner_shadow_of` on a read-back of the layer, drawn over it — before the layer effects, as the
  CPU does.
- Everything is keyed on the node's subtree, the view and the zoom: a drag rebuilds only the layers
  of the nodes that moved.

Still on the CPU: a backdrop effect beside another effect (its pixels would have to be read from
the page and then fed through this layer).

Tests: parity for a blur and a shadow, an inner shadow and a shadow, a blur with grain and a shadow,
a group's inner shadow, and a progressive blur with a shadow, beside the P6 shapes
(`the_gpu_draws_shadows_of_any_shape`):
mean 0.130/255, worst 50 on antialiased shape edges, 0.47% of pixels off by more than 8 — the
progressive card's outermost pixel row is the one place the ramp and the blur disagree. Texture has
its own page (`the_gpu_draws_the_texture_effect`: a rough card, one clipped to its shape, and one
in a list with a blur and a shadow): mean 0.126/255, worst 92, 0.6% of pixels off by more than 8,
all on the cards' antialiased edges (vello's coverage against tiny-skia's, scattered with the
pixels).
Exit: any single node's effect list draws on the GPU — met apart from the one exclusion above.

Cost of a progressive blur (the box passes are O(1), see P3; `progressive_drag_timings`, release,
medians of ten frames dragging a 200 × 300 card a pixel a frame, the CPU is a full render of the
same doc):

| ramp | canvas | CPU full | GPU drag |
|---|---|---|---|
| 0 → 300 | 904 × 860 | 218 ms | 0.57 ms |
| | 2400 × 1600 | 252 ms | 0.83 ms |
| 0 → 600 | 904 × 860 | 686 ms | 0.79 ms |
| | 2400 × 1600 | 1042 ms | 1.07 ms |

Doubling the ramp barely moves the GPU frame (the layer's own pixels grow, not the window's taps);
the CPU's four-box-per-level cost doubles. `the_gpu_draws_a_progressive_blur` checks the 0 → 300
case (a 1100 × 1200 layer, each level blurred in its band) against the CPU: mean 0.135/255, worst
30, 212 pixels off by more than 8. A progressive blur whose layer is past 16 M pixels (0 → 1250 on
that card past about 260% zoom) stays on the CPU: with the levels cropped to their bands the device
holds the layer, the crops and the mix output at once — 64 MB for a full-size texture, some 300 MB
together, which a driver can refuse outright.

### P8 — masks and plus-darker

**Landed (masks).** A mask and the siblings it clips — its paint run, from the mask up to the next
one, as `Document::paint_runs_of` gives it — are drawn in a layer of their own:

- The run's bounds (`layer_bounds` a node at a time, unioned and clipped to the view) set the layer's
  rectangle; the mask and the run are built for a view whose origin is that layer's corner, so
  anything inside them (an effect, a nested mask, text) lands on the layer's own grid.
- The mask's **coverage** is its own paint (alpha), its outline filled solid (vector), or — for a
  **luminance** mask — its paint's luma × alpha through a small `fs_luma` pass (tiny-skia's
  `ceil(255 × luma × alpha) / 255`).
- An alpha or vector mask's pass is vello's own `Compose::SrcIn`: the coverage drawn first, the run
  over it in a `SrcIn` layer — `source × destination's alpha`, which is the CPU's mask multiply. No
  new shader; a luminance mask takes the one pass above.
- The result is keyed on the mask's subtree, the view and the zoom, and kept between frames.

Tests: `the_gpu_draws_masks` — an ellipse mask over a card inside a frame, a vector mask in a group,
a mask inside a group, and a luminance mask over a card: mean 0.052/255, worst 57 on the mask's
edges (vello's coverage against tiny-skia's mask sampling), 0.18% of pixels off by more than 8; the
luminance mask's bright and black ends match the CPU within 2/255.

**Landed (plus-darker).** vello has no plus-darker blend and the CPU's compositing is W3C's
general formula (`max(0, backdrop + source − 1)` per premultiplied channel), so a page-level node
whose blend is plus-darker is a **step of its own** in the plan:

- The step's pixels are built with its blend left off (`scene::draw_plus_into`), so vello's scene
  holds plain content; the canvas composites it in a pass of its own (`plus.wgsl`,
  `CanvasGpu::plus_darker`).
- A render pass can't read the texture it writes, so the page is first copied into a scratch
  texture (`under`, `copy_texture_to_texture`) and the pass reads that, writing straight-alpha
  `max(0, …)` back over the page.
- The copy is why the step is never merged with its neighbours: it needs the page as it stands, and
  the step's own scene (like any other) is kept between frames.

Tests: `the_gpu_draws_a_plus_darker_layer` — six stripes under a translucent plus-darker card with
a shadow, drawn by both renderers and compared through the segmented path: mean 0.004/255, worst 6,
no pixel off by more than 8.

**Still on the CPU**: a **paint's** plus-darker blend (a fill or stroke, not a layer — vello can't
compose it and the pass works a layer at a time), a plus-darker layer *under* a backdrop effect (it
would be part of the accumulated page rather than a step), and the editor's mask outlines (an egui
overlay).
Exit: masked and plus-darker pages draw on the GPU — met apart from those two.

### P9 — the text tail

Text in one style already matched byte for byte. What used to fall back: runs with their own fills
and colours, colour glyphs (emoji), and text with a stroke.

**Landed.** Text is one outline per run: the layout's glyphs are grouped by their style index (the
decorations with them), each run is filled with its own `fills` or, without any, the node's, its
colour glyphs draw their own ink (a COLR layer's outline and colour, or the embedded bitmap as a
vello image — kept per layout bitmap, so it is handed over once), the inner shadows go over the
fills, and the strokes are the renderer's own bands: `stroke_area` and `stroke_rule` around each
run's outline, so inside and outside strokes clip to the glyphs.

`the_gpu_draws_styled_text`: a paragraph with a coloured run and an outlined (stroked) heading —
**pixel-identical** to the CPU (worst 0, mean 0.000, 0 pixels off by more than 8).
Exit: no text falls back — met.

### P10 — structure: nested backdrops, outline mode

**Landed (nested backdrops).** A backdrop node inside a container (a glass pane in a group, a
blurred card in a card) used to make `scene::plan` refuse the page. It doesn't need a segment
boundary *inside* a layer: the builder already re-renders everything that lies under such a node
(`Builder::backdrop_of`, over `figcraft_render::backdrop::above`) and blurs that — the page's
content up to the node, its ancestors' earlier children included. That is one extra page render per
nested backdrop each frame (the price of not segmenting inside a layer), and it is exact. P11 gives
the top-level container such a node lives in a step of its own, so the layers before it are kept
between frames; the render under the node itself still repeats.

`the_gpu_draws_nested_backdrops`: stripes, a group with a card and a glass pane on it, a frame with
a blurred card inside — **pixel-identical** to the CPU (worst 0, mean 0.000).

**Deliberately not ported**: the editor's **outline modes** (View ▸ Outlines, a selection's
outlines, hidden-layer outlines), **skipped/soloed layers**, **soft proof** and **Display P3**
documents. `render_worker::gpu_job` keeps them on the CPU: they are editor views of the document,
not content, and the colour-managed ones need a colour-managed surface (the canvas is sRGB).

Exit: the fallback predicate answers `Ok` for every page the canvas can be asked to draw with a
device, apart from the editor's own modes — met apart from a paint's plus-darker and a plus-darker
layer under a backdrop effect (P8).

### P11 — performance: cross-frame retention

Coverage first, then speed. **Landed (2026-10-25), with the damage-tracking part left open.**

A segmented page no longer re-encodes itself whole every frame:

- Each ordinary `Step::Nodes` is drawn into a canvas-sized step segment kept between frames, keyed
  on its subtrees, the view and the zoom (`scene::step_key`, as `figcraft_render::layers` keys its
  kept layers). A step whose key still matches is laid over the page again instead of being drawn
  again. The segments are bounded: as many canvas-sized textures as `STEP_BYTES` (128 MB) affords
  (`CanvasGpu::step_slots`); a page with more steps than that draws the rest into the shared one.
- Each page-level node's **encoded scene** is kept too (the same key), so a drag re-encodes only the
  layers it changed: `build_with` and `draw_into` draw a node at a time, reuse its kept scene, and
  keep the scene they encoded (`CanvasGpu::keep_node_scene`, 2048 scenes, least recently used first).
  A node whose pixels are not its own (a backdrop effect, its own or nested) is never kept.
- A step whose pixels are not its own — a container with a backdrop effect nested inside it — is
  never kept (`scene::step_is_self_contained`): it reads the page under it, which may have changed.
- A top-level container with a backdrop effect nested in it gets a segment of its own
  (`scene::plan`), so on a segmented page the layers before it are kept and only its own re-render
  (the page under it, then the glass or blur) repeats. On a page that is not segmented otherwise
  (all its backdrop effects nested), the single-scene builder draws it whole, as in P10.

Tests: `the_gpu_keeps_an_unchanged_step_between_frames` (a kept segment draws the same pixels; an
edit gives the step a new key and the frame matches one from a canvas that never drew it),
`the_gpu_keeps_a_step_while_a_later_backdrop_moves`, `a_step_that_reads_the_page_is_drawn_every_frame`
(a nested pane's step is drawn again when the stripes under it change),
`a_drag_reuses_the_encoded_scenes_of_the_layers_it_did_not_move` (twelve scenes on the first frame,
eleven reused and one encoded when a card moves, and the frame matches a fresh canvas's),
`a_container_with_a_backdrop_effect_is_not_kept`, and the *timing* `segmented_drag_timings`.

Measured (`segmented_drag_timings`, release, medians of ten frames with a wait after each, one card
dragged a pixel a frame; the CPU is `figcraft_render::Retained` — damaged pixels and kept effect
layers — on the same frames; owner's machine, 2026-10-25, three runs shown as ranges because the
GPU's throughput moves 2–3× with what else is running; the step cache alone — per-node scene
caching, which came after, is measured below):

| page | canvas | CPU retained | GPU drag | GPU, nothing changed |
|---|---|---|---|---|
| plain (30 shadowed cards) | 904 × 860 | 2.7–3.3 ms | 1.1–1.4 ms | 1.0–1.3 ms |
| | 2400 × 1600 | 3.1–4.8 | 1.1–1.5 | 1.2–1.3 |
| layer blur | 904 × 860 | 0.2–0.3 | 0.9–1.1 | 0.8–1.1 |
| | 2400 × 1600 | 0.2–0.3 | 1.0–1.4 | 1.0–1.3 |
| one glass pane | 904 × 860 | 2.2–2.6 | 1.8–6.5 | 2.8–3.8 |
| | 2400 × 1600 | 2.3–2.5 | 5.3–6.6 | 2.1–2.3 |
| heavy (30 shadows, 3 panes, 2 blurs) | 904 × 860 | 10.1–10.3 | 6.2–8.6 | 6.2–8.4 |
| | 2400 × 1600 | 9.8–10.2 | 5.7–9.3 | 6.1–6.5 |
| nested backdrops | 904 × 860 | 3.0–3.4 | 5.1–7.1 | 4.6–6.7 |
| | 2400 × 1600 | 2.8–3.3 | 10.6–10.9 | 10.2–10.9 |

The content-dominated pages are at or under the CPU's retained renderer, and every frame is inside
a 60 Hz budget (16.7 ms) at 2400 × 1600. A drag *on a backdrop page* is still 2–3× the CPU's: the
GPU assembles the whole page every frame (the background fill, a compose per step, the effect
passes, the present copy) where the CPU repaints only the damaged rectangle. **The exit is not met
on backdrop pages.** Closing it needs damage-tracked assembly: keep the last frame's page, clear
only the damage rectangle, compose the steps that meet it and redo only the effects whose pane it
touches (4–6 h).

In the app (release, control channel, eight stripes with a glass pane over them at 904 × 860):
`canvas.rasterizer` stays `gpu` with no refusal, twelve one-pixel edits of a stripe render in
2.4–3.3 ms a frame after the first four (11–16 ms: the pipelines and the first frames), and the
screenshot shows the pane over the stripes.

With each node's encoded scene kept as well, a frame with *nothing* changed costs 0.65–1.35 ms on
the plain page, 1.3–2.5 ms on the glass page and 4.6–5.6 ms on the heavy page (runs on 2026-10-26;
a drag stays inside the table's ranges — the run-to-run variance there is larger than the saving).
The first frame encodes every node; after that each node that changes is encoded again and the rest
are appended from the cache.

### What is left on the CPU

Content, after P5–P10: a **paint's plus-darker blend** and a **plus-darker layer under a backdrop
effect**. Each is a pass of its own, named where it is planned above (P8). A content file with one
of those still re-renders whole on the CPU.

Performance, after P11: **damage-tracked assembly** on backdrop pages (4–6 h) — the GPU assembles a
segmented page every frame; the CPU repaints only the damaged rectangle, so a drag there is 2–3×
faster.

Deliberate exceptions, documented rather than ported: exports (`svg`, `pdf`, `png`/`jpg`), the parity
suite and the browser keep the CPU renderer; the editor's outline modes, skipped layers, soft proof
and Display P3 (P10); a machine whose adapter cannot run vello or whose textures are under 8,192 px,
and a frame with a layer the device's textures can't hold (the canvas refuses and says why in
`ui.inspect`).

## Test matrix

Run in `cargo xtask ci` unless marked *timing* (release, `--ignored --nocapture`).

| # | test | what it proves |
|---|---|---|
| 1 | `the_gpu_canvas_matches_the_cpu_render` | pixel parity per feature: shapes, radii, gradients, text, opacity, blend, strokes |
| 1b | `the_gpu_text_matches_the_cpu_render` | text is pixel-identical (measured: 0 differing bytes) |
| 1c | `the_gpu_layer_blur_matches_the_cpu_render`, `the_gpu_drop_shadow_matches_the_cpu_render`, `the_gpu_draws_the_shadow_variations`, `the_gpu_draws_a_progressive_blur`, `the_gpu_draws_a_sideways_progressive_blur`, `a_dragged_card_keeps_its_blur`, `a_whole_pixel_move_keeps_the_layer_key` | effects: mean 0.011, 0.044 and 0.512/255 (the last with spread, two shadows, a stroke, a gradient card, a clipped frame and a rotation); a 0 → 300 progressive blur over a 1100 × 1200 layer, down the card and across it (bands of crops): mean 0.135/255, worst 30; a dragged card: the blur kept on every 1 px move (7/7) and the moved frame matched at mean 0.136/255 |
| 1d | `the_segmented_page_matches_the_cpu_with_one_glass_pane`, `..._one_background_blur`, `the_gpu_page_matches_with_several_backdrop_layers` | backdrop effects in the accumulated renderer: mean 0.061, 0.049 and 0.725/255 |
| 1e | `the_gpu_draws_the_paint_variations` | paints: a photo 1:1, fitted, cropped, tiled, rotated, filtered and translucent, a pattern fill, a diamond gradient, multiplied and screened paints (mean 0.066/255, worst 10) |
| 1f | `the_gpu_draws_shadows_of_any_shape`, `the_gpu_draws_the_texture_effect` | shadows of any shape: an ellipse, a star, a polygon, a per-corner card, text, a group, a frame with an overflowing child, inner shadows on shapes and a group's — with and without spread, and kept between frames (mean 0.130/255, worst 50); texture: a rough card, one clipped to its shape, one in a list with a blur and a shadow (mean 0.126/255, worst 92 on antialiased edges) |
| 1g | `the_gpu_draws_styled_text`, `the_gpu_draws_masks`, `the_gpu_draws_a_plus_darker_layer`, `the_gpu_draws_nested_backdrops` | styled runs and stroked text (pixel-identical), alpha, vector and luminance masks (mean 0.052/255; a luminance mask's ends within 2/255), a plus-darker layer (mean 0.004/255, worst 6), nested backdrops (pixel-identical) |
| 1h | `the_gpu_keeps_an_unchanged_step_between_frames`, `the_gpu_keeps_a_step_while_a_later_backdrop_moves`, `a_step_that_reads_the_page_is_drawn_every_frame`, `a_drag_reuses_the_encoded_scenes_of_the_layers_it_did_not_move`, `a_container_with_a_backdrop_effect_is_not_kept` | a kept step's or node's pixels, an edit giving it a new key, a step that reads the page never being kept, and a drag re-encoding only the layers it moved (the frame matches a canvas that drew it fresh) |
| 2 | `unsupported_pages_fall_back` | the predicate rejects exactly the features not yet ported |
| 3 | `the_gpu_canvas_reads_back` | screenshot/export path: texture → `Pixmap`, premultiplied, same size |
| 4 | *timing* `canvas_frame_timings`, `segmented_timings` | CPU vs GPU frame at 2320 × 1440 (plain and heavy) and 2400 × 1600 (backdrop pages) |
| 5 | *timing* `heavy_scene_timings` (one effect group at a time, GPU against CPU), `heavy_scene_gpu_timings`, `layer_blur_timings`, `shadow_timings`, `segmented_drag_timings`, `progressive_drag_timings` | each effect's own cost stays honest; a single-layer drag against the CPU's retained renderer on the corpus pages; and a progressive blur's cost staying flat as its radius grows |
| 6 | `cargo xtask wasm` | the browser build still compiles and stays CPU |
| 7 | screenshot comparison | the real app looks the same with the GPU canvas on. Measured on the 34-layer page at 34.6%: mean 0.9/255, differences on shape and shadow edges only — the CPU path snaps the view outwards to whole device pixels while the GPU draws the requested view, so content can sit a pixel apart |

Parity tolerance (what the suite enforces, measured 2026-10-24 on the owner's machine): mean
absolute channel difference ≤ 0.5/255; at most 0.5% of pixels may differ by more than 8 in a
channel; no channel may differ by more than 96 (a half-covered antialiased edge pixel). Text may
differ in hinting but not in position or size. Anything a pass cannot meet is not enabled; it
stays on the fallback list.

## Risks

- **Text.** vello's text stack (`skrifa 0.44`) differs from the workspace's (`0.47`). We do not
  use vello's text: glyphs come from `figcraft_text`'s outlines, so layout, metrics and coverage
  stay ours.
- **Colour.** vello blends in its own precision; Display P3 documents need a shading colour space
  decision before P2 (canvas shows sRGB today; the CPU path converts after rasterising).
- **Blend modes and masks.** vello has layers, blend modes and `SrcIn`; alpha, vector and luminance
  masks are checked in the parity suite, and plus-darker is a pass of its own (`plus.wgsl`) fed
  through a copy of the page.
- **Images.** vello images live in an atlas owned by the `Renderer`; the CPU path caches decoded
  pixmaps. The GPU path registers textures once per image and reuses them, or stays CPU for
  animated GIFs.
- **Start-up cost.** vello builds shader pipelines at first use (~100 ms); build the renderer
  once, when the first GPU frame is requested, and never on a resize.
- **Driver variance.** The parity suite runs on the machine's adapter and skips without one; CI
  without a GPU stays green on the CPU path.

## Status

- 2026-10-24: **P0 and P1 landed** in `crates/gpu` (`scene.rs`, `canvas.rs`, `canvas_tests.rs`),
  with vello 0.11 on the workspace's wgpu 30.0.1 and the same `kurbo`/`peniko` versions the
  renderer uses.
  - Parity, 600 × 300 page (solids, per-corner radii, linear gradient, ellipse, a 50%-opacity
    layer, a multiply layer, a centre stroke): **mean channel difference 0.072/255**, 600 of
    180,000 pixels (0.33%, all on antialiased edges) differ by more than 8, worst single channel
    67 at a half-covered ellipse edge. Both images look identical.
  - Frame cost, release, Intel Arc A750, whole canvas redrawn every frame, gradients and corner
    radii included:

    | canvas | scene | CPU `render_view` | GPU frame (draw + wait) |
    |---|---|---|---|
    | 2320 × 1440 | one card | 5.5–5.7 ms | 0.76–0.86 ms |
    | 2320 × 1440 | 360 cards | 7.5–8.2 ms | 1.4–1.6 ms |
    | 1160 × 720 | one card | 1.5–1.9 ms | 1.1–1.3 ms |
    | 1160 × 720 | 360 cards | 2.3–2.8 ms | 1.45–1.5 ms |

    The GPU frame is nearly flat in pixel count and scene size (~1 ms of it is vello's per-frame
    encode and the submit/wait round trip); the CPU cost scales with both. Pipelined submissions
    (no wait between frames, as the app draws) measured 0.8–2.2 ms a frame.
  - Read-back (screenshots, exports, tests) costs 1.5 ms at 1160 × 720 and 6–9 ms at 2320 × 1440:
    it stays off the canvas path.
  - `cargo xtask layers` passes; the crate is desktop-only (the web app doesn't build it), so the
    browser stays on the CPU renderer.
- 2026-10-24: **P2 landed** — the canvas draws on the GPU for pages in the P1 feature set and
  falls back to the CPU for everything else, in the real app:
  - 60 plain rectangles at a 1200 × 900 canvas (the app's window): **1.1–1.7 ms a frame** on the
    GPU (steady state, measured over 20 drag steps), **5.4–6.7 ms** on the CPU (one drop shadow
    on one layer puts the page back on the CPU, as designed). Occasional 6–8 ms frames after an
    idle gap are the GPU waking up, not scene cost.
  - Zooming (new texture sizes, re-registration), selection overlays and `ui.screenshot` verified
    in the running app; 645 UI tests, the GPU tests and clippy pass.
- 2026-10-24: **P3 slice 1 landed** — layer blur on the GPU. A page whose only effect is one layer
  blur now stays on the GPU: its layer is drawn into an offscreen texture, blurred by the same
  box passes as the CPU, and composited, all on the device.
  - In the app, a 36-rectangle page with one blurred card (radius 20) holds 2.5–4.9 ms a frame
    over 20 drag steps (median ~3.5 ms, occasional GPU wake-ups) where the same page with the
    blur plus a shadow — which still falls back — takes 6.6 ms on the CPU.
  - In isolation (release, 1200 × 900): 1.31 ms a frame against the CPU's 3.33 ms
    (`layer_blur_timings`). The gap to the in-app number is the window's own rendering and the
    control-channel round trips between steps.
  - Shadows, background blur, glass, inner shadows and grain still put the page on the CPU, and a
    node with a blur *and* another effect does too (the CPU applies them in order).
- 2026-10-24: **P3 slice 2 landed** — drop shadows on rounded, opaque cards, straight in the
  scene: no offscreen layer and no blur pass, so a page of 30 shadowed cards is drawn in one
  frame.
  - Release, 30 cards with radius-12 shadows: **0.74–0.82 ms a frame** on the GPU against 24–26 ms
    for a full CPU render (`shadow_timings`); at 2320 × 1440 as well as 904 × 860, since the cost
    is the scene, not the pixels. In the app, dragging a card on that page holds 1.2–2.2 ms a
    frame (occasional GPU wake-ups spike to 7–10 ms, as on plain pages).
  - Parity: mean 0.044/255, worst 31 (the soft edge), against the CPU's three-box blur.
- 2026-10-24: text proved pixel-identical between the two rasterizers (same glyph outlines from
  the renderer's layout: `the_gpu_text_matches_the_cpu_render`, 0 differing bytes at 1×).
- 2026-10-24: **P3 slice 3 landed** — backdrop effects. The page under a node is drawn into a
  texture (`backdrop::above` lists what to leave out) and read there:
  - **Background blur** on the GPU: blurred, shown through the node's shape, content over it.
    Parity mean 0.049/255, worst 25; 1.84 ms pipelined against the CPU's 5.07 ms at 904 × 860.
    Pages may have one and stay on the GPU.
  - **Glass** on the GPU: the padded backdrop is cropped, blurred by the pane's frost and run
    through the existing glass shader, from the CPU's cached distance field and normals
    (`figcraft_render::glass::pane` split out of `glass()`). Parity mean 0.061/255, worst 6, no
    pixel off by more than 8. Alone, a pane cost about what the CPU cost (4–11 ms against 5–9 ms),
    so pages with a pane stayed on the CPU until the accumulated renderer landed.
- 2026-10-24: **P3 slice 4 landed — the accumulated-backdrop renderer**, and with it glass and every
  backdrop effect stay on the GPU, however many the page has:
  - A page with a backdrop node is drawn in segments: the layers before it into a segment texture,
    laid over an accumulation, the effect made from the accumulation, its content over it. The page
    under two panes is drawn once.
  - Parity (release tests): glass mean 0.061/255 worst 6 with no pixel off by more than 8;
    background blur mean 0.049/255 worst 25; the heavy 30-card + 3-pane + background-blur +
    layer-blur page mean 0.725/255 worst 40, all on card edges.
  - Release, pipelined: heavy page 8.8 ms at 904 × 860 and **6.2 ms at 2400 × 1600** against the
    CPU's 48.6 and 53.6 (`segmented_timings`); one pane 5.7 / 3.3 ms, one background blur 3.8 /
    3.2 ms.
  - The first attempt failed at effect borders because vello's targets hold straight alpha; see
    "Straight vs premultiplied alpha" above.
- 2026-10-24: **P3 slice 5 landed — wider drop shadows, and the strokes that cast them.** A card's
  shadow is its own rounded rect whatever the card is painted with: spread, any number of shadows,
  `showBehind`, a shadow blend of its own, a gradient fill with opaque stops, a translucent fill
  (one alpha: the shadow is as faint as it paints and clipped away where the card covers), an
  opaque stroke (inside, centred or outside — the silhouette grows by its outer half), rotation,
  and children in a clipping frame. Strokes themselves are no longer limited to plain centred ones: inside, outside,
  per-side, dashed, profiled and brush bands are filled from the renderer's own outline. A linear
  gradient whose width handle isn't square to its axis (only imported files skew one) now stays on
  the CPU, where vello's projection and the CPU's handle frame agree.
  - `the_gpu_draws_the_shadow_variations`: mean 0.558/255, worst 59, 1.5% of pixels off by more
    than 8 — the Gaussian/three-box difference on the soft edges, not a shape error.
  - 30 spread shadows: **0.69–0.91 ms** a frame against the CPU's 28–34 ms (`shadow_timings`).
- 2026-10-24: **the full-coverage program (P5–P11) is written down** in *Full GPU coverage*, in the
  order the work pays off: paints (images, patterns, diamond gradients, per-paint blends), then any-outline and
  inner shadows, effect combinations with grain and progressive blur, masks and plus-darker, the
  text tail, nested backdrops and outline mode, and cross-frame retention. Each phase carries its
  own tests and exit criterion; the deliberately non-ported set (exports, the browser, soft proof
  and Display P3) is named at the end.
- 2026-10-25: **P5 landed — paints.** Images (every scale mode, rotation, crop, filters, opacity),
  pattern fills, diamond gradients and per-paint blend modes draw on the GPU from the renderer's own
  prepared pixmaps; only skew-free linear gradients still use vello's own gradient brush. A page
  with a photo, a logo or a pattern no longer re-renders on the CPU.
  - `the_gpu_draws_the_paint_variations`: mean 0.066/255, worst 10, 5 of 450,000 pixels off by more
    than 8 — the bilinear (vello) against bicubic (tiny-skia) difference on a 2× downscaled photo.
  - Animated GIF pages stay on the CPU (the canvas refuses them in `present`, before any work).
  - `PlusDarker` paints and skewed linear gradients still fall back, with a test each.
- 2026-10-25: **P6 landed — shadows of any shape, and inner shadows.** A drop shadow the rounded-rect
  fast path can't draw is now a layer the GPU renders, blurs (the CPU's own three box passes) and
  tints: a node's silhouette grown or shrunk by the spread, or its own pixels grown by `fx::morph`.
  Inner shadows are the shape's outside at the offset in a padded buffer, cut out with a `DestOut`
  layer, blurred and clipped to the shape, between the fills and the children.
  - `the_gpu_draws_shadows_of_any_shape`: an ellipse, a star, a polygon, a per-corner card, text, a
    group, a frame with an overflowing child, and inner shadows on a card, a per-corner card and an
    ellipse — mean 0.066/255, worst 50 on antialiased edges, 0.28% of pixels off by more than 8.
  - The layer rectangle is `figcraft_render::layer_bounds` (the renderer's own, without the effects'
    margin): the crop is visible on a frame whose child reaches past it, and the GPU now stops the
    shadow where the CPU does.
  - **Kept between frames**: keyed on the node's `Arc` subtree, the view and the zoom (plus the
    document's ids), so a drag rebuilds one shadow and redraws the rest. 30 shadow-layer cards:
    **1.2–2.4 ms** a frame where the CPU takes 17–43; the first frame (all thirty, and the new
    pipelines compiled) 90–170 ms.
- 2026-10-25: **P7 landed — effect combinations, grain, progressive blur.** A node's effects are one
  layer in the CPU's order: content and inner shadows → each layer blur (uniform, or progressive as
  the renderer's own blur levels mixed by `mix.wgsl`) → grain (the renderer's noise from the layer's
  pixels, read back once) → the drop shadows cast from that result → the layer over them.
  - `the_gpu_draws_shadows_of_any_shape` also covers a blur and a shadow, an inner shadow and a
    shadow, a blur with grain and a shadow, and a progressive blur with a shadow: mean 0.130/255,
    worst 50, 0.47% of pixels off by more than 8 (the progressive card's outermost row).
  - Still on the CPU: texture, a group's inner shadow, and a backdrop effect beside another effect
    (named in P7 and P10).
- 2026-10-25: **P9 landed — the text tail.** A text node is one outline per run, filled with the
  run's own paints, its colour glyphs' ink drawn where they are (COLR paths, or the embedded bitmap
  as an image), its inner shadows over the fills, and its strokes from the renderer's glyph bands
  (inside and outside clip to the glyphs).
  - `the_gpu_draws_styled_text`: a coloured run and a stroked heading — pixel-identical to the CPU.
  - No text falls back any more; what is left is `Unsupported::Mask` (a text node masked, P8), and
    soft proof / Display P3 (P10).
- 2026-10-25: **P8 landed — masks.** A mask and the siblings it clips are one layer: the mask's
  coverage (its paint, or its outline for a vector mask) and the run, cut by vello's own
  `Compose::SrcIn` (`source × destination's alpha` — the CPU's mask multiply, no new shader). The
  run's bounds set the layer, and the sub-scenes are built for a view whose origin is that layer's
  corner, so effects, text and nested masks inside land on its grid.
  - `the_gpu_draws_masks`: an ellipse mask over a card in a frame, a vector mask in a group, a mask
    in a group — mean 0.052/255, worst 57 on mask edges, 0.18% of pixels off by more than 8.
  - **Luminance masks** draw on the GPU too: a `fs_luma` pass multiplies the run by the mask's
    `ceil(255 × luma × alpha) / 255` (tiny-skia's own formula), kept like any other masked layer.
    `the_gpu_draws_masks` now covers one (a white → black gradient mask over a card): its bright
    and black ends match the CPU within 2/255.
  - `PlusDarker` stays on the CPU.
- 2026-10-25: **P10 landed — nested backdrops.** A glass pane in a group or a blurred card in a
  card no longer refuses the page: no segment boundary is needed, the builder re-renders what lies
  under the node and blurs that (one extra page render per nested backdrop a frame, and exact).
  - `the_gpu_draws_nested_backdrops` — stripes, a group with a card and a glass pane, a frame with a
    blurred card inside: pixel-identical to the CPU.
  - The editor's outline modes, skipped layers, soft proof and Display P3 stay on the CPU by design
    (`render_worker::gpu_job`), as does `PlusDarker`; texture and a group's inner shadow are the
    last two content cases left (P7).
- 2026-10-25: **P11 landed — cross-frame retention for segmented pages, and nested backdrops get a
  segment of their own.** Each ordinary step is drawn once and kept in a canvas-sized texture under
  the key of its subtrees, the view and the zoom; unchanged steps are laid over the page again
  without being drawn again. Each page-level node's **encoded scene** is kept too, so a drag
  re-encodes only the layers it changed (eleven of twelve scenes reused when one card moves); a
  step or node that reads the page is never kept. On a segmented page a container with a nested
  backdrop forms a step of its own. A single-layer drag on the corpus pages is 0.9–10.9 ms against
  the CPU's retained renderer's 0.2–10.3: at or under it on the plain, shadow and effect-heavy
  pages, still 2–3× on backdrop pages, which the GPU re-assembles whole each frame
  (`segmented_drag_timings`). Damage-tracked assembly is what is left of P11 (4–6 h).
  - `the_gpu_keeps_an_unchanged_step_between_frames`, `the_gpu_keeps_a_step_while_a_later_backdrop_moves`,
    `a_step_that_reads_the_page_is_drawn_every_frame`,
    `a_drag_reuses_the_encoded_scenes_of_the_layers_it_did_not_move`,
    `a_container_with_a_backdrop_effect_is_not_kept`: a kept step or scene draws the same pixels, an
    edit gives it a new key, and what reads the page is redrawn when what lies under it changes.
- 2026-10-26: **plus-darker landed.** A page-level node whose blend is plus-darker is a step of its
  own: its pixels are built with the blend left off (`scene::draw_plus_into`) and the canvas
  composites them over a copy of the page with `plus.wgsl` (W3C's `max(0, backdrop + source − 1)`),
  straight alpha. `the_gpu_draws_a_plus_darker_layer`: mean 0.004/255, worst 6, no pixel off by
  more than 8. A **paint's** plus-darker and a plus-darker layer under a backdrop effect still
  fall back.
- 2026-10-26: **a group's inner shadow landed.** A group has no outline of its own, so its inner
  shadows are cast from its layer's pixels: the renderer's `inner_shadow_of` on a read-back of the
  layer, drawn over it before the layer effects (the layer path P7 built). The group in
  `the_gpu_draws_shadows_of_any_shape` matches the CPU at mean 0.001/255, worst 1.
- 2026-10-26: **in the app, a blurred card's drag frame is 1.3 ms.** Re-measured through the
  control channel on the release app (`ui.pointer`, a real drag of the 0 → 300 card at 100%):
  `canvas.renderMs` 1.29 ms a frame, 60.5 ms for the first frame after the effect is added
  (pipelines, the layer's first upload), 6–8 ms a control round trip. **The hitches were texture
  churn**: a progressive blur made and dropped a dozen full-size textures a frame (its levels, the
  content layer, the mixed result), and frames alternated between 2 ms and 100–370 ms — the stall
  was inside `build_with`, in the allocations, not in the passes. Blur scratch textures and layer
  textures now come from pools a frame gives back (128 MB each, the oldest evicted: clearing a
  pool on overflow put the churn straight back), so a steady drag frame is 0.5 ms of building,
  0.3 ms of rendering and 0.5 ms of wait; only the first frame of a new canvas pays its pipelines
  (110–170 ms). The progressive blur's device-memory ceiling is 16 M pixels (2026-10-28: it was
  6 M, which put the same card on the CPU from 200% zoom; see the entry below), so the same card
  stays on the GPU to 244% zoom.
- 2026-10-26: **the box passes are O(1) a pixel, in one dispatch.** A 200 × 300 card with a
  progressive blur from 0 to 300 (the radius the acceptance pass found laggy) dragged at
  2400 × 1600 costs 0.83 ms a frame where a full CPU render takes 252; 0 → 600 costs 1.07 ms
  against 1042 — the radius no longer costs the GPU anything (`progressive_drag_timings`). Each
  pass is one compute dispatch: a workgroup scans 256 pixels of a line and their halo (up to 256
  a side) in shared memory and writes the averages straight out (`blur.wgsl`'s `box`); a radius
  past the halo uses the running-sum `scan` + `fs_box` pair, and a device without compute keeps
  the window-reading pass. `the_gpu_draws_a_progressive_blur` (a 1100 × 1200 layer):
  mean 0.135/255, worst 30 — the same pixels as the strip version, which is the point: the
  speedups below changed no output. `the_gpu_draws_a_sideways_progressive_blur` covers the ramp
  across the card, where the levels' bands are columns and the crops start differently along x. A
  progressive blur whose layer is past 16 M pixels stays on the CPU (see the entry above).
- 2026-10-28: **a drag is silky at any zoom (100% … 2000%).** At 600% a full frame was 25–35 ms
  (the card's layer is clipped to the window and made again per frame, and its blur plus the page
  render did not fit in a frame), so the pointer outran it. Three changes:
  - **Interaction frames are previews, drawn into the canvas's own texture.** `present` takes the
    window's device size separately from the view: half of it each way for a preview (a quarter of
    the pixels), drawn into the target's top-left corner — the target is not reallocated, no
    texture is registered again, and `Frame.uv` says which part of the texture the frame fills
    (the UI samples it). The worker draws a preview when a full frame of *this* zoom took more than
    13 ms (`Renders::gpu_wants_preview`, which also requires the same uid/page/scale, so zooming
    out is drawn full and sharp again); the UI asks for the sharp frame with `Job::refine` once
    nothing has been asked for for 60 ms and the pointer is up (`GPU_REFINE_AFTER_MS`), and once
    per view (`CanvasState::refined`). While a drag goes on no full frame is drawn at all: a full
    render between two slow steps used to cost the drag its next preview.
  - **Each progressive level is blurred at its own resolution** (`canvas::level_scale`): a level
    whose box radius is past the one-dispatch pass's halo (256 px) takes running sums — a dozen
    dispatches — so its sigma is what costs, not its pixels: at 900 px it is blurred at a quarter,
    at 1800 px at an eighth, while a narrow level keeps the layer's resolution (the ramp's sharp
    end). The chain at 600% is 2.5–4 ms on the device (was 10–16), and the sharp frame 15–22 ms
    (was 40).
  - **vello was handed a new image every frame.** Each layer registered a fresh blank `ImageData`
    (a new blob id), so vello's image atlas added a slot a frame, grew to 8192 × 8192 and was
    reallocated *twice a frame* (a layer's solid content has no late-bound resource, so it reports
    a 1 × 1 atlas; the page with the layer's image the big one) — 40 ms of driver stalls a frame
    after a few seconds of dragging. Now a texture keeps the image it was first registered with
    (its atlas slot with it) and its pixels are marked dirty (`register_layer`), an evicted kept
    layer gives its texture back to the pool (`keep_shadow`), a scene that draws a layer image is
    not kept as a scene (`scene::has_effects`: the canvas reuses those images), and a layer or
    segment scene with no late-bound resource gets a transparent gradient (`canvas::steady_atlas`)
    so its atlas size matches the page render's. The atlas sits at 4096 × 4096 `Reused`, one
    upload a frame.
  Sustained 400-move drags (control channel, release app, maximized window, 2560 × 1440) at
  600%: 67, 70 and 72 canvas frames a second one after another (before, a long drag fell from 60 to
  15–18 fps); 2000%: 76 fps; 1200%: 75; 200%: 94; 100%: 160. The frame at 600% is `build 8.8–9.8,
  render 1.8–1.9 ms` (a preview, with the log's device wait) and the sharp one `build 12.8,
  render 2.6 ms`. Tests: `a_smaller_frame_fills_the_canvas_corner` (a preview is the same pixels as
  a canvas of its own size, and the target is not reallocated), `wide_blur_levels_are_blurred_smaller_than_narrow_ones`,
  `a_scene_that_draws_a_layer_image_is_not_kept`, `a_preview_comes_only_after_a_slow_full_frame_of_the_same_zoom`,
  `a_preview_on_screen_asks_for_the_sharp_frame_once`, `the_canvas_shows_images_of_its_own_page_and_never_a_coarser_one_of_its_view`
  (a preview of newer content does go over a refined frame of older content — a drag — while a
  preview of the same content does not).
- 2026-10-28: **the card drags smoothly at any zoom, 200% included.** A real cursor over the
  maximized window at 200% exposed two more places where a zoom other than 100% was different:
  - **The blur's device-memory ceiling (6 M pixels) put 200% on the CPU.** The card's layer is
    its paint bounds grown by the effect's margin — 1400 × 1466 device px at 100%, 2800 × 2930
    (8.2 M) at 200% — so past 100% the progressive blur refused, the whole frame left the GPU
    canvas, and the CPU redrew the blurred card every frame: 818 ms a frame (`renderMs`), 26–34
    frames in a four-second drag, the card left behind at its old place. The ceiling is 16 M
    pixels now: the levels are cropped to their bands, so the device holds the layer, the crops
    and the mix output (some 300 MB) rather than eight full-size levels. At 200% the canvas is
    back on the GPU: `render` 4.4–6.4 ms a frame, one job a pointer move, and the card within
    0–2 px of the cursor (median) at 100%, 200% and 244% (12.2 M pixels, 6.2 ms a frame).
  - **The canvas's job key was the document's address, and a drag reuses it (ABA).** Measured
    with the log on: 200 of 231 frame decisions found the "same" document and kept the texture,
    and only the 31 frames where the allocator happened to move the document were drawn — a blur
    drag at 200% froze between those. `JobKey.doc` is the document's uid and its **revision** by
    value now (the revision is bumped by every command, which is what a drag is), with the IME
    composition alongside it while one is typed. Regression test:
    `a_moved_document_asks_for_a_new_frame`.
  - **The exact corner is for layers the frame *makes*.** A progressive layer drawn from its exact
    corner and composited on a whole pixel lands up to half a device pixel off the vector's place,
    which is invisible on a blur but shows as a halo where a crisp layer meets a shadow's cut-out
    (`the_gpu_draws_shadows_of_any_shape` went from worst 50 to 135). Now only a layer the frame
    makes — a blur, noise or texture, whose pixels are laid down anyway — is drawn from its exact
    corner and cached across subpixel moves; a group, a card under a shadow and an inner shadow
    stay on the renderer's floored grid and draw as exactly as the CPU does. Parity is back to
    worst 50 / mean 0.142 with every GPU test green, and `a_dragged_card_keeps_its_blur` keeps the
    layer 7 of 7 at 100%, 86% and 244%.
  The screen captures above were misleading until they were made against a white card on the
  dark page: a black card on the dark background has no edge to follow, and every frame "measured"
  the background.
- 2026-10-28: **effect layers are clipped to the view.** At 600% the 0 → 300 card's layer is
  6600 × 7200 device pixels (47 M) — past the 16 M blur budget, so the whole frame went back to
  the CPU, and the allocations it made saturated the device (a maximized window then failed
  `Surface::configure`, "failed to wait for GPU to come idle"). The renderer clips its own layers
  to the view (`int_rect` against the pixmap), and now so does the canvas: a layer is its paint
  bounds ∩ the view (plus a pixel of slack), a backdrop blur's crop likewise. At 600% the layer
  is 2281 × 1625 — the window — and the canvas stays on the GPU (`gpuRefusal` none). Test:
  `a_layer_bigger_than_the_view_is_cropped_to_it`. What is left at that zoom is that the layer's
  near edge is the window's, so the layer is made again for every frame the card moves (its
  content moves inside the crop): the frame is a window-sized progressive blur — measured with the
  present log, `progressive (2281, 1625)` 16–31 ms on the device a rebuild, `build` 27–81 ms and
  `render` 9–20 ms, against 4–5 ms for a frame that keeps it; a real cursor at 600% leaves the card
  about 90 ms behind (it was 818 ms a frame on the CPU, when the frame fell back at all).
- 2026-10-28: **a large layer's blur levels are computed at half resolution.** The levels are the
  blur: the detail a box pass averages away is the detail a smaller texture would have had, so past
  2 M pixels (and at a quarter past 8 M) they are blurred from a bilinear downscale of the layer —
  a quarter of the work each way — and the blend reads them upscaled, while the layer itself, which
  the ramp's sharp end shows, stays at full resolution. `the_gpu_draws_a_big_progressive_blur_at_half_resolution`
  (a 1461 × 1561 layer, levels at half) is worst 28 / mean 0.041 against the CPU, as good as the
  full-resolution test's 0.135; `a_downscale_halves_the_layer_the_right_way_up` pins the downscale
  itself (a texel is the average of its four, the right way up, premultiplied). At 600% a rebuild
  is now 9–15 ms on the device (was 16–31) and 19–53 ms of building (was 27–81).
- 2026-10-28: **a blur's box passes are one compute pass.** A pass a box pass was most of a
  window-sized blur's frame: `encoder.finish` (recording 48 dispatches as 48 passes, with their
  bind groups) took 5–28 ms at 600%, where the dispatches themselves are cheap. Every pass of a
  level now goes into one compute pass (each dispatch is its own usage scope, so wgpu keeps their
  reads and writes in order), and `finish` is 6–15 ms. The blend's output texture comes from the
  layer pool too (a fresh window-sized texture a frame was 5–15 ms of driver work): at 600% a
  rebuild's `build` is 19–23 ms (was 36–53) and its `finish` 0.7–3 ms. What is left is the canvas
  render itself (13–18 ms for 2280 × 1624) and the page's own build.
- 2026-10-26: **a real drag no longer re-blurs the card (and no longer freezes it).** With a real
  mouse (the OS cursor moving at 714 px/s over a maximized 2560 × 1440 window, canvas 2280 × 1624
  device px at 100%) a drag rebuilt the blurred layer on **every** frame — 73 rebuilds in a 0.5 s
  drag, ~35–50 ms a frame — and, worse, the step cache could serve a **stale** scene: the keys were
  node *addresses*, and a drag frees the previous document, so the allocator handed the next node
  the same address (ABA) and the card froze at an earlier position. After the fix the same drag
  rebuilds the blur **zero** times and every frame is `plan 0.0, build 0.1, render 6.5–8.3 ms`;
  screen captures put the card within 7 px of the cursor (~10 ms behind). Two changes:
  - **Keys are values, not addresses** (`scene::node_hash`): a node and everything under it
    serialized into the hasher. `step_key` keeps the node's own placement (a step's pixels move
    with it); `content_hash` leaves it out (a layer's *texture* does not depend on where the node
    sits).
  - **The layer key is the layer-local transform** (`layer_xf`, from the layer's **exact**
    device corner): a layer's content is drawn in the layer's own space, so moving the node by a
    whole device pixel *or part of one* leaves the content where it is and moves the layer — the
    kept blurred layer is drawn at its new place (its corner rounded to a pixel). Before, the
    corner was floored, which put the corner's fraction into the content, so only a drag that was
    a whole number of device pixels (that is, 100% zoom) kept the layer; at 86% or 244% every
    frame re-blurred it. Tests: `a_whole_pixel_move_keeps_the_layer_key` (the same key for 0.5,
    1, 2 and 3.7 px moves at 100%, 86% and 244%; a repaint or a resize changes it) and
    `a_dragged_card_keeps_its_blur` (seven moves at 100%, 86% and 244% find the kept layer 7/7,
    and each moved frame matches the CPU within 3/255 on average).
  The earlier scripted measurements missed this: they walked the same positions back and forth, so
  the cache hit by luck. Every number below that came from `ui.pointer` was re-checked with a real
  drag; the drag path is now measured with one.
- 2026-10-26: **a dragged blurred card reaches 50–60 fps in the app.** The acceptance case
  (200 × 300 card, progressive 0 → 300, 100% zoom, release app driven through the control channel)
  went from **8 fps** to **116–141 canvas frames in about 2.3 s** while the pointer moved 154 pixels
  a second (the render thread skips to the newest job, so this is what the window shows). (Its
  "settled drag rebuilds nothing" reading — 133 frames, 0 rebuilds — was an artefact of the script
  repeating the same positions: a real drag rebuilt the layer every frame until the layer key was
  made translation-free, see the entry above.) A rebuilding frame is now 24–60 ms of building plus
  5–25 ms of drawing (it
  was 76–124 ms, and 250–370 ms before the pools), of which the blur's encode is 0.4 ms and the
  device 8–18 ms; a frame that keeps the layer is 3–7 ms. Three changes, in order of what they
  bought:
  - **Only the pixels a level is used in are blurred** (`canvas::level_crop`): the ramp's bands,
    plus 3σ + 3 of reach, cropped per level. 1.5 M pixels of crops against 10.6 M before; the
    parity above is unchanged, and `a_progressive_level_blurs_only_its_band` pins the crop.
  - **A box pass is one dispatch** (the halo in shared memory), not a scan pass plus an average
    pass: 48 dispatches a frame instead of 96 plus a 21 MB running-sums texture read and written
    twice a pass. Device time for the whole progressive blur fell from ~50 ms to 12–18 ms, which
    is what a whole canvas frame now costs.
  - **One parameters buffer with dynamic offsets** for a level's six passes, and one running-sums
    texture shared by all eight levels, instead of a buffer an allocation and a texture a level.
  What is left of the frame is not the blur: `queue.submit` is under a millisecond, and the
  remainder is vello drawing the 1.1 M-pixel canvas plus the driver's command recording.
- 2026-10-26: **texture landed.** A texture effect is the renderer's own pixel scatter: the layer's
  pixels are read back once, `grain::texture` moves each grain's pixel by its hashed jitter, and the
  result replaces the layer before the drop shadows are cast from it.
  `the_gpu_draws_the_texture_effect` (a rough card, one clipped to its shape, one in a list with a
  blur and a shadow): mean 0.126/255, worst 92, 0.6% of pixels off by more than 8, all on the cards'
  antialiased edges. A paint's plus-darker and a plus-darker layer under a backdrop effect are what
  content still leaves on the CPU.
- 2026-10-29: **the canvas is opt-in, and four review points are closed.** Preferences ▸ Draw the
  canvas on the GPU (`pref.gpuCanvas`, off by default; `Jobs` carry it so toggling asks for a fresh
  frame) replaces the old on-by-default behaviour and `FIGCRAFT_GPU_CANVAS`. Textures are checked
  against `max_texture_dimension_2d` before they are made (`canvas::fits`), so a device under the
  scene's 8,192 px layer cap, or a layer, shadow, blur or glass pass the device can't hold, leaves
  the frame to the CPU instead of panicking. Kept layers and the kept CPU frame hash the document's
  image store (`layers::images_id`), so an image that arrives after a frame was drawn redraws it;
  the web build renders with `doc_id = 0`, so the CPU renderer stays stateless there. Tests:
  `an_image_that_arrives_draws_the_layer_again`, `the_gpu_canvas_runs_only_when_the_preference_is_on`.
- 2026-10-30: **the effects are the canvas's, and the CPU canvas goes back to stateless.** The
  window's GPU could draw the renderer's raster effects (blur, glass, shadows) for every render,
  exports included, because `figcraft_gpu::install` gave the renderer an accelerator at launch.
  `Options::accelerate` now follows each render down to every effect call (`fx::blur`, `blur::apply`,
  `shadow::drop_shadow`/`inner_shadow`/`inner_shadow_of`, `EffectFrame`) and only a canvas render with
  the preference on sets it, so an export, a thumbnail or the CLI draws them on the CPU. With the
  preference off the CPU canvas keeps no layers between frames either (`doc_id` 0): the layers a
  frame keeps are the GPU canvas's fallback's, and everything else is the stateless renderer it was.
  Tests: `an_export_never_uses_the_canvas_accelerator`,
  `the_cpu_canvas_keeps_layers_and_accelerates_only_for_the_gpu_preference`.


