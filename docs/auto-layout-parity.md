# Auto layout and constraints parity

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-29 · **Change:** minor (Layout sidebar: Add auto layout (+) is available for groups and converts a single group to a frame in place, with mask order, flow-only inference and Fill sizing preserved; adding auto layout to an empty frame keeps its size (#369); `node.set` applies auto layout keys only to frames, with warnings (#603); core editor leftovers: auto gap handles, wrapped-line alignment; behaviour suite written from the public documentation, every rule's status; fixes and the missing settings it found) · **Target:** Figma Design (desktop app 126.10.7)

Auto layout is the most used layout feature in Figma and the one where small differences (a
hugging frame one pixel off, text wrapping at another width) break real files. **Breadth ~98%,
ready ~74% (estimated)**: every rule of the public documentation is a test case with the sizes and
positions it implies (`crates/layout/src/behaviour_tests.rs`, 64 cases, and
`crates/engine/src/al_behaviour_tests.rs` through commands), and all pass; nothing yet against real
Figma files. Remaining: 3–6 h.

## Adding auto layout to a group

With one group selected, Add auto layout converts that group into a frame in place, retaining its
id, name, parent and direct children. The Layout sidebar’s +, context menu, Shift+A and API calls
all dispatch `layout.add`, so they share the conversion and a single undo step restores the group.
The layout settings are inferred from visible, non-absolute children, matching the layout engine’s
flow. Hidden and absolute children therefore do not distort direction, gap or padding. A converted
group keeps Fill sizing on either axis in its auto layout parent. Sorting by position preserves
mask boundaries and the paint order within masked runs. The sidebar offers Add only when every
selected container has no auto layout, including mixed selections.

This follows the Figma behavior reported by the contributor and the sidebar workflow users know from
similar design tools: start with a group and make its children participate in auto layout.
Wrapping the group would instead make the entire group one layout item, leaving its children
outside the new frame’s layout flow and adding an unnecessary level to the layer tree. Converting
the selected container makes later changes to a child’s size reflow its siblings directly.
Selecting several groups still wraps that selection in a shared auto layout frame.

## Rules from the documentation

Status: **pass** (a case in the suite holds), **fixed** (the suite found a divergence, now fixed),
**added** (the setting was missing and now exists as a `node.set` prop and a Design panel control),
**silent** (the documentation doesn't say; the case pins the current behaviour).

| Rule | Status | Evidence / notes |
|---|---|---|
| Hug: the frame sizes to its children plus padding (the 40 px text + 10 px padding = 60 px example) | pass | `sizing_rules` |
| Fixed: the frame keeps its size whatever its children need | pass | `sizing_rules` |
| Fill: children share the space left after padding, gaps and fixed siblings; across the flow they stretch | pass | `sizing_rules` |
| Fill on a child turns its hugging parent Fixed on that axis | pass | `al_behaviour_tests::fill_on_a_child_turns_the_hugging_parent_fixed` |
| Hug only on auto layout frames (and text); Fill only in an auto layout flow, never on a top-level frame | pass | `al_behaviour_tests::hug_and_fill_only_where_they_apply` |
| Min/max width and height bound Hug, Fill and Fixed sizes, on frames and children | pass | `sizing_rules`; a fill child at its max leaves the rest to the other fill children (silent on redistribution: CSS flexbox) |
| A hugging text wider than its max width wraps at the max | pass | `sizing_rules` |
| Text filling a width wraps and its hugging frame grows with it | pass | `sizing_rules` |
| A hugging frame follows a child that grows, through nested levels, in one edit and one undo step | pass | `al_behaviour_tests::a_hugging_frame_follows_its_children_through_nested_levels` |
| Padding per side (uniform, vertical/horizontal, individual) | pass | `padding_and_stroke_rules` |
| A frame is never smaller than its padding (plus an included inside stroke), even when Fixed, hugging or given a smaller max | **fixed** | was free to shrink below its padding |
| Strokes included: only the frame's own **inside** stroke takes up space; center and outside strokes never do | **fixed** | every stroke alignment counted before; per-side inside weights count per side (silent on individual weights) |
| New auto layout includes inside strokes by default | **fixed** | the default was excluded |
| Strokes included / excluded setting in the Design panel | **added** | `strokesIncluded` existed as a prop only |
| Gap: fixed distance; negative gaps overlap | pass | `gap_rules` |
| Auto gap between: no space at the edges; a single child at the start | pass | `gap_rules` |
| Auto gap evenly (1:1 with the edges) and around (gaps twice the edges) | **added** | `mainAlign: spaceEvenly / spaceAround`, Design panel gap menu |
| An auto gap stops at 0 when children don't fit (no overlap) | **fixed** | went negative before |
| Alignment 3×3; with an auto gap only the cross alignment; a hugging frame shows no difference | pass | `alignment_rules` |
| Text baseline alignment, texts of different sizes | pass | `alignment_rules` |
| Baseline alignment of a layer without text (an icon next to text) | silent | it keeps to the top of the line |
| Horizontal wrap: a new row when the width runs out; gap between rows | pass | `wrap_rules` |
| Vertical wrap: a new column when the height runs out; not masonry; gap between columns | **added** | layout, Design panel wrap toggle, column gap field, on-canvas gap handles, drag reorder and arrow keys by column |
| Wrapped rows/columns as a block: packed at the start, centre or end the cross alignment names; stretched when every child fills across the flow; or the free space between them (one line or no free space: like packed) | **added** | `content_align_tests` (13 cases), `al_content_align_tests`; `contentAlign`, the Row/Column gap mode in the Design panel, REST and `.fig` both ways, `align-content` in CSS |
| Ignore auto layout: out of the flow, kept in the frame; constraints apply only then | pass | `absolute_position_rules` |
| Canvas stacking: last on top by default, first on top on request; the layer order stays | **fixed** + **added** | `reverseZ` was stored but never drawn: now the canvas, PNG, SVG and PDF exports and hit testing paint in that order; Design panel control |
| Grid: `fr` tracks share by fraction; fixed tracks keep their size; hug tracks fit their content | pass | `grid_rules` |
| Grid `fr` tracks behave like the CSS `fr` unit: a track never gets less than its content | **fixed** | was `minmax(0, 1fr)`: a fixed-size child taller than its share overflowed into the next row |
| Grid placement left to right, top to bottom; rows grow; anchors; spans; gaps; padding | pass | `grid_rules` |
| Grid children align independently in their cells | **added** | `gridAlignH` / `gridAlignV` (Auto follows the frame), Design panel cell alignment |
| A hugging grid with `fr` tracks | silent | the file format calls it invalid; `fr` tracks hug their content |
| A hugging frame with an auto gap | silent | it hugs with the gap value, then shares the (no) extra space |
| One layout pass settles every case | pass | every case is laid out twice and must not move |

## Features

| Feature | Status | Evidence / notes |
|---|---|---|
| Horizontal, vertical, wrap (both directions) | done | `crates/layout` |
| Grid auto layout: fixed / hug / `fr` tracks, cells, spans, gaps, cell alignment | done | `fr` like CSS `fr` |
| Gap, auto gap (between / evenly / around), padding (per side), alignment 3×3 | done | |
| Text baseline alignment | done | |
| Hug / Fill / Fixed sizing with coherent rules (Fill turns a hugging parent Fixed) | done | `al_nesting_tests.rs`, `al_behaviour_tests.rs` |
| Min/max width and height, redistribution | done | hugging text wraps at its max |
| Nested hug/fill settling in one pass (three levels, wrapping fill text, grid rows) | done | |
| Absolute position (ignore auto layout) with constraints | done | |
| Strokes included in layout (inside strokes), canvas stacking | done | |
| On-canvas padding and gap handles, incl. wrap row/column gap and grid gaps | done | an automatic gap's handle shows the distance it has; dragging it fixes the gap from there (silent in the documentation) |
| Drag to reorder (wrap-aware, by row or column), into grid cells, across nested frames | done | `al_drag_tests.rs` |
| Arrow keys move children one place (flows, rows, columns, grid cells, into neighbours) | done | `al_keys_tests.rs` |
| Constraints: left, right, left and right, centre, scale; top/bottom likewise | done | |
| Layout grids: columns, rows, grid; stretch/left/centre/right; variables bound | done | |
| Suggest auto layout / "Add auto layout" on a selection | done | the Layout sidebar exposes Add auto layout (+) for groups and frames using the same `layout.add` command as the context menu; a single group becomes a frame in place, keeping its id, name and direct children; adding auto layout to an empty frame keeps its size, Fixed (it collapsed to 20×20); a frame with content still hugs it, as does a new frame wrapped around a selection (#369). `node.set` with auto layout keys on a mix of layers applies them to the frames and warns about the rest instead of failing the whole call (#603) |
| Real Figma files laid out identically | **not checked** | needs `.fig` or REST import of real files ([gaps](gaps.md) #1, #3) |
| Performance: re-flow only where sizes or places changed | done | measured in `cargo xtask bench` (components at scale) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-29 | minor | M2 review fixes: conversion preserves mask runs and Fill sizing per axis; layout inference uses visible, non-absolute flow children; the sidebar hides Add for mixed selections containing existing auto layout. Regression coverage includes rendered mask pixels, visible-child positions, parent layout stability and the sidebar accessibility tree. Numbers unchanged |
| 2026-10-29 | minor | M2: the Layout sidebar exposes Add auto layout (+) for groups. `layout.add` converts one selected group into a frame in place rather than wrapping it: same id, name and direct children; the layout arranges those children, and one undo restores the group. Sidebar, menu, shortcut and API calls all use the corrected command. This follows the contributor’s observed Figma behavior and the workflow familiar to users of similar design tools. Tests cover direct-child reflow, stable hierarchy, undo/redo and wrapping a multi-group selection. Numbers unchanged |
| 2026-10-17 | minor | Community batch #753: adding auto layout to an empty frame keeps its size, Fixed, while a frame with content hugs it (#369); `node.set` applies auto layout keys to the frames among its targets and warns about the others, and a font style the family lacks warns with the nearest face (#603). Checked over MCP. Numbers unchanged |
| 2026-10-13 | minor | Auto layout, on-canvas handles of automatic gaps (M2): the handle between spread rows or columns (and between items with an auto gap) shows the distance the layout gives the gap instead of the stored value, and dragging it makes the gap a fixed one from there, as typing a gap in the panel does; one undo step (`gap_handle_tests`). Before, the drag changed a value that had no effect. The documentation is silent on dragging an automatic gap. Numbers unchanged. |
| 2026-10-13 | minor | Auto layout, wrapped-line alignment (M2): a wrapping layout's lines (rows of a horizontal one, columns of a vertical one) are packed where the cross alignment says (start, centre, end; before, they always sat at the start), stretch when every child fills across the flow, or take the free space between them (`contentAlign: spaceBetween`, the Design panel's Row/Column gap mode, the alignment pad lighting the whole cross axis). Read from REST JSON and `.fig`, written to `.fig`, `align-content` in CSS, an instance override with the other alignment settings. 13 hand-computed layout cases and command tests. Numbers unchanged. |
| 2026-10-11 | minor | Behaviour suite from the public documentation (64 layout cases, 8 command tests). Fixed: padding floor, inside-only included strokes (and included by default), auto gaps stop at 0, grid `fr` like CSS `fr`, canvas stacking actually drawn and hit. Added: auto gaps evenly/around, vertical wrap, grid cell alignment, strokes and canvas stacking controls. Breadth 95% → 98%, ready 68% → 74%, remaining 6–12 h → 3–6 h |
| 2026-10-10 | major | Created from the 2026-10-10 re-measure |
