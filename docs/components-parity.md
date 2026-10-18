# Components, variants, styles and variables parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-18 · **Change:** minor (a bound colour variable's alpha applies once (#802); variables import reads `variables` outside `meta` and warns when it imports none (#790); variant properties reorder by drag; library files saved right after a read are read again; Hide when publishing note corrected; size and resizing overrides of layers inside instances (#731); variables export in the browser as a download; core editor leftovers: component playground, variables inside styles; layers and assets panels behaviour suite and fixes; extended collections tail; styles and variables behaviour suite and fixes; components behaviour suite and fixes; team libraries on the server, library usage and swapping a library; extended collections, variables import/export, gradient stops and variant properties bound to variables) · **Target:** Figma Design (desktop app 126.10.7)

Design systems are why teams adopt Figma. **Components: breadth ~96%, ready ~72%. Styles and
variables: breadth ~95%, ready ~76% (estimated).** QA'd with a synthetic design system of ~300
components used 2,000 times (`crates/engine/src/synthetic_ds.rs`, `components_scale_tests.rs`);
not with real libraries. Remaining: 4–10 h here, plus team libraries' tail (4–8 h, partly after
accounts).

## Components and variants

| Feature | Status | Notes |
|---|---|---|
| Create component, instances, detach, reset, go to main, restore main, push changes | done | push from a nested layer goes to the outer main |
| Per-layer overrides with reset; kept across main edits, nested swaps, copy/paste | done | main edits match a full rebuild exactly at 2,000 instances |
| What an instance overrides: fills, strokes, effects, visibility, layer names, text content and every text property (resizing included), auto layout padding, gap and alignment, layout grids, export settings, prototype interactions, nested swaps and property values; the size and resizing (hug, fill, fixed, min and max) of layers inside it | done | each kept when the main changes that property, reset per group; opacity and corner radius too (documentation silent); size and resizing are kept through relayout, undo, save, `.fig` and REST import and detach, and stay with the instance on push |
| What it doesn't: position, rotation, constraints and absolute or grid placement of layers inside it, its auto layout direction and other settings, its structure | done | refused with a message, the fields greyed out; the instance itself resizes and its layers follow the main's constraints |
| Component sets / variants, add variant, switch | done | |
| Switching variants keeps overrides where names match and both variants had the same value; text layers keep theirs | done | |
| Default variant: the top-left one (new instances, Assets thumbnail, generated code) | done | |
| Variant property order (names rewritten), duplicate combinations reported | done | drag a property in the Properties section to reorder it, one undo step (#679), or `component.editProperty {index}` |
| Properties: boolean, text, instance swap (preferred values), variant | done | |
| Slots (preferred components, fill menu, kept across updates) | done | Figma 2025 feature |
| Exposed nested instances | done | hidden ones hide their properties |
| Descriptions | done | |
| Instance swap: panel, Alt-drop from Assets | done | the panel keeps overrides on same-named layers, an Alt-drop from Assets only text layers' |
| Assets panel: folders by page, frame and slash names (fold away), thumbnails, search, drag into frames and auto layout slots | done | virtualized; `component.list` gives each one's `folder` |
| Hidden from publishing: a leading `.` or `_` in the name | done | the name decides both ways (adding the prefix takes a published component out, removing it brings it back); Hide when publishing in the Assets panel's context menu and Libraries ▸ Hide when publishing (see [ui-parity](ui-parity.md)) |
| Instances pasted into other files follow their mains | done | missing main restorable |
| Component playground (try properties without placing) | partial | variant, boolean and text properties in the Inspect tab and `component.playground`; instance swaps and slots have no control there yet (0.5–1 h) |
| Real-world libraries QA | **not checked** | [gaps](gaps.md) #3 |

The behaviour of instances, overrides, variants and swaps is checked against the public
documentation by `crates/engine/src/components_behaviour_tests.rs` (one table row per documented
rule, with its derivation). Where the documentation is silent, the suite pins today's behaviour:
layer opacity and corner radius are overridable; rotation of a layer inside an instance is refused
with its position; boolean and text property values stay across variants; a missing combination
picks the variant matching the most values; reset all keeps the instance's size; an instance's own
min / max width and height stay its own. An instance's own export settings and interactions are
its own (the main's later changes to them don't reach existing instances, so connections people
added are never lost). Nested instance overrides: `nested_override_tests.rs`; accepting library
updates with overrides kept: `library_tests.rs`, `components_scale_tests.rs`.

## Libraries

| Feature | Status | Notes |
|---|---|---|
| Publish a file's components, sets, styles and variables (per-asset versions) | done | as local library files |
| Enable a library in a file; read-only linked copies | done | |
| Library updates available; overrides kept | done | a library file saved again within two seconds of being read, at the same length, is read again (its time stamp can't prove it unchanged; #652) |
| Team libraries: publish to a `figcraft-server` under the file, enable by a share link (a view link reads it), background update checks | done | [server.md](server.md#team-libraries); no team-wide library list or accounts ([gaps](gaps.md) #10) |
| Library analytics | partial | instances per component and the files using them, as apps report them; nothing over time, no detach counts |
| Swap library (instances, styles and variables to another library's of the same names, overrides kept) | done | what has no match stays and is listed; one undo step |

## Styles and variables

| Feature | Status | Notes |
|---|---|---|
| Paint, text, effect, grid styles; style picker; text styles on character ranges | done | behaviour suite: `styles_behaviour_tests.rs` |
| Editing a style updates its layers; detach and delete keep values; editing a styled property unlinks that style | done | an edited fill/stroke/effect/grid list or text chars unlink; applying a style replaces variables bound to what it sets; text styles on text layers only |
| Organising styles: folders by `/` names, order, duplicate, rename folder, Local styles list | done | `style.move` (into a folder renames), `style.duplicate` (right below), `style.renameFolder`; the page's Local styles list by type and folder (drag to reorder, right-click) |
| Variables inside styles (paint, effect, grid and text styles; each user resolves them in its own modes) | done | `style.bind`, `style_vars_tests`; the editor binds paint colours, shadow colours, blur radii and grid sizes, the rest through the command |
| Variables: colour, number, string, boolean; collections, modes, aliases, scopes, groups | done | behaviour suite: `variables_behaviour_tests.rs`; aliases same type, no loops (also on create); scopes filter pickers only |
| Modes: add, duplicate, rename, delete, set as default / reorder | done | layers on Auto follow a new default; explicit modes stay |
| Deleting a bound variable or its collection | done | layers keep their last value and are unbound; aliases to it take its values |
| Code syntax per platform (Web, iOS, Android) | done | generated CSS uses the Web name; in the variables JSON; SwiftUI/Compose code doesn't name variables yet |
| Bindings: fills, strokes, effects, opacity, padding, gaps, sizes, radius, font size/weight, line height, letter spacing, paragraph spacing/indent, text, visibility, layout grids, instance properties | done | type-checked per field; number variables fill text content; a colour variable bound to a solid paint brings its whole colour, alpha included, once (the paint's opacity restarts at 100% on binding, an opacity set while bound multiplies on top; unbinding moves the alpha back into the opacity) |
| Explicit modes per frame, library collections' modes | done | |
| Gradient stops and variant properties bound to variables | done | a stop's binding follows it when earlier stops are removed |
| Extended collections | done | library collections, including a library's own extended collections (chains through libraries); overrides per mode with reset; follow library updates, variables new to the library collection being an update of their own; modes inherited. Checked with synthetic libraries only ([gaps](gaps.md) #14) |
| Variables import/export (JSON) | done | the REST local variables shape (round-trips; merges by collection, mode and variable name; `variableCollections` and `variables` read under `meta` or at the top level, a warning when no variables are found or fewer than the collections list); CSS custom properties export; in the browser the export arrives as a download (import there still needs a file picker) |
| Expressions in prototype set-variable actions | done | see [prototyping-parity](prototyping-parity.md) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | A bound colour variable's alpha applies once (#802): a `#RRGGBBAA` paint keeps its alpha in the paint opacity, and binding a variable with alpha left that opacity in place, so a 50% fill bound to a 50% variable drew at 25%. Binding (`variable.bind`, `node.set` `boundVariables`, `style.bind`) now restarts the paint's opacity at 100% and unbinding (also deleting the variable) moves the colour's alpha back into the opacity; `.fig` and REST imports take the variable's alpha back out of a bound paint's opacity, so `.fig` round trips no longer square it. Colour-picker edits of a colour variable or a plugin colour field, generated CSS strokes and the document colours list counted the paint opacity twice and now count it once (`bound_alpha_tests`). Numbers unchanged |
| 2026-10-18 | minor | Variables import (#790): a `variables` map outside `meta` (or `meta.variables` next to top-level collections) is read, and an import that finds no variables, or fewer than the collections' `variableIds` list, says so in `warnings` instead of succeeding silently |
| 2026-10-17 | minor | Variant properties reorder by dragging them in the Properties section, one undo step (#679; was "no drag in the panel yet"); a library file saved again right after it was read is read again, so a quick republish isn't missed (#652); the Hidden from publishing row no longer says the Hide when publishing item is missing (it landed 2026-10-12). Numbers unchanged |
| 2026-10-17 | minor | Size and resizing overrides of layers inside instances (#731): a layer in an instance can hug, fill or keep a size of its own (group `resizing`: hug/fill/fixed with min and max; `size`), from the Layout panel, the canvas handles and `node.set`; they were refused, and imported ones were dropped at the first relayout. Kept through main edits of other properties, rebuilds, undo, save, `.fig` and REST import (sizes a `.fig` instance gives its layers and the REST `overrides` fields), `.fig` export and detach; reset per group and with Reset all changes (`instance_resizing_tests`). The row that listed them as not overridable was wrong. Numbers unchanged. |
| 2026-10-14 | minor | Variables export in the browser: Export… ▸ Variables JSON / CSS custom properties did nothing in the web build, which has no save dialogs; it now hands the file over as a download (`Services::downloads` / `download`, `variables_export_ui_tests`). Import in the browser still needs a file picker, as do the other exports there. Numbers unchanged. |
| 2026-10-13 | minor | Component playground (M3): with an instance, a component or a component set selected, the Inspect tab shows a Playground: a preview of the component and a control for each variant, boolean and text property. What is tried there changes the preview and the code in the Code section, never the file (no undo step, the canvas instance stays as it is); Reset goes back to the component's or the instance's own values. `component.playground {id?, properties?, language?, path?}` gives the same to scripts: the properties with their values, the code, and a PNG on request (`playground_tests`). Left: instance-swap and slot properties in the playground (the command takes an instance swap; the panel has no control for it), a code connect template's snippet with the tried values. Numbers unchanged. |
| 2026-10-13 | minor | Variables inside styles (M3): a shared style can bind variables to fields of its value (`style.bind {style, field, variable?}`: a paint style's colours and gradient stops, an effect style's colour, radius, spread and offsets, a grid style's size, count, gutter and offset, a text style's font size, line height, letter spacing, family and style). Every layer using the style shows those fields resolved in its own modes and stays linked when a variable or a mode changes; the style keeps them resolved in the default modes. A style made from a layer keeps that layer's bindings; a new value drops bindings to what is gone. The style editor binds from its property rows (paint colour, shadow colour, blur radius, grid size) and detaches from the variable's chip; the style picker marks styles that use variables. `.fig` export and import carry the bindings on the style, REST layers hand theirs to the style they use. Left: binding text style fields and the other effect and grid fields from the editor (commands only), library variables in the picker, a style's library copy. Numbers unchanged. |
| 2026-10-12 | minor | Layers and Assets panels behaviour suite: Assets folders by page, frame and slash names; a leading `.` or `_` keeps a component out of the library even after it was published, and removing it brings the component back |
| 2026-10-12 | minor | Extended collections' tail (gap 14): variables a library adds to a collection this file extends now count as a library update of their own (listed by name without a copy yet) and arrive inheriting when accepted, instead of waiting for the library's next other update; a library collection that is itself an extended collection can be extended again, values flowing down the chain as each file accepts updates with every level's overrides kept (checked end to end). Styles & variables breadth 94% → 95%, ready unchanged (76%), 3–5 h → 2–4 h; gap 14 1–2 h → 0–1 h (real-library checks left); feature breadth (~82%) and ready for real work (~51%) unchanged at this rounding; full parity ~305–594 h → ~304–593 h |
| 2026-10-11 | minor | Styles and variables behaviour suite from the public documentation (18 tests) and its fixes (styled properties edited away unlink, deleted variables unbind with values kept, type-checked bindings, alias checks on create, text styles on text only); style folders, order and duplicates with a Local styles list; mode duplicate and default; code syntax. Found missing: variables inside styles. Styles and variables breadth ~92% → ~94%, ready ~72% → ~76%; remaining here 7–14 h → 9–17 h |
| 2026-10-11 | minor | Behaviour suite from the public documentation and its fixes: overrides of padding, gap, alignment, layout grids, export settings, interactions and every text property kept (they were lost when the main changed); geometry, constraints and resizing of layers inside instances refused; variant switches keep only overrides both variants agreed on (text layers keep theirs); Alt-drops from Assets keep only text overrides; top-left default variant; hidden exposed instances hide their properties; push from nested layers goes to the outer main; variant property reordering and conflict warnings. Components ~95%/~68% → ~96%/~72%; remaining here 7–14 h → 5–11 h |
| 2026-10-11 | minor | Team libraries on `figcraft-server` (publish under the file, enable by a share link, updates with overrides kept), usage counts per component and file, and swapping a library by names (gap 10) |
| 2026-10-11 | minor | Extended collections, variables import/export (JSON, CSS) and gradient stop and variant property bindings landed: styles and variables breadth ~85% → ~92%, ready ~70% → ~72%; remaining here 10–20 h → 7–14 h |
| 2026-10-10 | major | Created from the 2026-10-10 re-measure |
