# Interaction measurements

> **Last reviewed:** 2026-11-08 · **Last updated:** 2026-11-08 · **Change:** minor (first record of the interactions measured against Figma, case by case) · **Target:** Figma Design web app, measured 2026-11-08

On 2026-11-08 a set of interactions was measured in Figma's web app and compared with FigCraft's: modifier keys on move, resize and rotate, arrow keys, zoom steps, auto layout Fill and Hug, instances, components, number field scrubbing, sidebar widths, auto-pan at the viewport edge, copy and paste in vector edit mode, duplicate and paste placement, and text-editing keys. Every measurement used synthetic files, driving the UI and reading the Design and Layers panels (field values, dropdown labels, bounding boxes), or counting pixels on screen where a panel could not show the number. No incumbent screenshots are committed. This complements [ui-parity.md](ui-parity.md) and works on gap 4 in [gaps.md](gaps.md), "Interaction fidelity not compared tool by tool". It is not a full pass: each section lists what was covered, and the open questions at the end list what could not be measured.

Status values: **matches** (FigCraft already behaved as measured), **fixed in #NNNN (open)** (a mismatch found, with the pull request that fixes it, not merged yet), **not measured** (with the reason), **inconclusive**. Error margins are given where the measurement had one. Trust the shapes of the rules more than the last digit of a number.

## Move, resize and rotate modifiers

Synthetic pointer sequences on the canvas with modifier flags set. Resize handles were pressed 4 px inside the edge, so absolute resize numbers are about 8 px short per side; the cases were read relatively.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Shift while moving | locks to the dominant axis (drags of +100,+30 and +100,+90 both gave dy 0); no 45 degree snap | the smaller of dx and dy is zeroed | matches |
| Alt while moving | a copy stays at the original place and the selection moves (layers 1 to 2) | duplicates on drag, re-synced with the Alt key | matches |
| Ctrl while moving | inconclusive: with Ctrl from the start the layer did not move, with Ctrl late in the drag it still snapped; a synthetic Ctrl does not seem to reach Figma as the modifier | Ctrl or Cmd turns snapping off | not measured (synthetic Ctrl is not delivered as a modifier) |
| Shift while resizing | corner (+50,+10) on a 100 by 50 layer keeps the ratio; right edge (+50,0) scales the height too, about the vertical centre | corner by the dominant factor about the opposite corner; edge about the opposite edge's middle | matches |
| Alt while resizing | a right edge grows symmetrically, the centre kept | resizes from the centre | matches |
| Shift and Alt on a corner | proportional, centre kept | both | matches |
| Modifier changed mid-drag | Shift on the second half locks; Shift on then off unlocks again; read fresh on every move | modifiers read on each move | matches |
| Shift while rotating | snaps the absolute angle to multiples of 15 | snapped the change in angle | fixed in #2096 (open) |
| Space while drawing or marquee | not measurable through the browser | not compared | not measured (the browser does not deliver it) |

## Nudge and arrow pan

Real keys; pan distances read from screenshots, plus or minus 3 px.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Arrow keys on a selection | nudge 1 document px, 10 with Shift, at any zoom | same | matches |
| Arrow keys with nothing selected | pan the view: Right about 62 to 64 px (two trials), Shift+Right about 320 px (5 times) | 20 and 200 px (`PAN_STEP_PX`, 10 times) | fixed in #2106 (open) |

## Zoom steps and fit

Real keys. The canvas was 1389 by 903 CSS px with the floating bottom toolbar over it.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Zoom in from 100% | 200, 400, 800, 1600, 3200, 6400, 12800, 25600 (maximum) | same steps (`STEPS` in `cmd/view.rs`) | matches |
| Zoom out from 100% | 50, 25, 13, 6, 3, 2 (minimum) | same | matches |
| Zoom from a typed 150% | in gives 200, out gives 100 (snaps to the list) | same | matches |
| Zoom to fit (Shift+1) | a 100 by 100 layer at 793% (vertical margin about 55 px); a 1000 by 200 layer at 134% (horizontal margin about 24 px each side); not a fixed 64 px margin, and not separated from the toolbar overlay | not changed | inconclusive (the toolbar overlay mixes into the margins) |
| Zoom to selection (Shift+2) | a 10 by 10 layer at 7630%, about 39 px above and about 100 px below (toolbar) | not changed | inconclusive (same overlay) |

## Snapping and menus

Measured earlier the same day; recorded here by pull request only.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Snap threshold | a layer snaps about 4.5 screen px from a guide, at 100% and 200% | snapped at 5 screen px | fixed in #2086 (open) |
| Submenu opening | a submenu opens about 11 ms after its row is hovered; no safe triangle seen | opened after 0.15 s | fixed in #2093 (open) |

## Auto layout: Fill and Hug

Values read from the zoomed Design panel's fields and dropdowns, in horizontal rows hugging height unless noted.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Cross axis: a child set to Fill in a hugging parent | the child stretches to the tallest sibling; the parent stays Hug (child 40 Fill, parent 40 Hug; sibling typed to 60 gives 60 Fill and 60 Hug); no notice | turned the hugging parent Fixed | fixed in #2099 (open) |
| Cross axis: every child set to Fill | when the last non-Fill child is set, toast "Height set to Fixed for Frame 1" and the parent turns Fixed at its current size; typing 80 on the parent gives 80 Fixed with both children 80 Fill | the parent turned Fixed and the child kept its own size | fixed in #2099 (open) |
| Vertical stack hugging width | C (20 by 10) set to W Fill gives C 100 and the parent 100 Hug; D typed to 150 gives C 150, parent 150 Hug; no toast | the parent turned Fixed and the child kept its own size | fixed in #2099 (open) |
| Padding with a stretched child | padding 10 top and bottom, parent H 60 Hug; child H Fill gives 40 (the content box), parent stays 60 Hug | the parent turned Fixed and the child kept its own size | fixed in #2099 (open) |
| Min height on a stretched child | min height 50 gives child 50 Fill and parent 50 Hug (the stretched child's minimum counts toward the hug size) | the parent turned Fixed and the child kept its own size | fixed in #2099 (open) |
| Text sibling | rect H Fill beside two lines of text (Inter 24, line height 32): text 64 Hug, rect 64 Fill, parent 64 Hug; a third line gives 96, 96, 96 Hug | the parent turned Fixed and the child kept its own size | fixed in #2099 (open) |
| Main axis: a Fill child in a hugging parent | the parent turns Fixed (toast "Width set to Fixed for Frame 1", parent 197 Fixed) | the parent becomes fixed | matches |
| Wrap on, Fill height on a child | "Fill container" for height is greyed out on a child of a wrapping row (checked only with the parent hugging height) | not compared | not measured (only the hugging case was checked) |
| Setting a parent to Hug while a child fills along it | toast "Width set to Fixed for <child>"; FigCraft demotes the child silently | no notice | not measured as a fix (possible small follow-up: the same notice) |

## Instances

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Main component width and height follow its content | main: horizontal auto layout, W Hug, H Hug (135 by 49); a pasted instance reads the same | instance root sizing did not follow its main | fixed in #2099 (open) |
| A child set to Fill along the main axis | toast "Width set to Fixed for Frame 1"; main W Fixed 135, H Hug; the plain instance turns W Fixed 135 too, H Hug | the instance's own resizing never followed its main | fixed in #2099 (open) |
| Instance with its width overridden to 200 | stays W 200 Fixed through the main's change, H keeps following (Hug) | the instance's own resizing never followed its main | fixed in #2099 (open) |
| Main set back to Hug contents | toast "Width set to Fixed for Rectangle 1"; main 135 Hug; the plain instance back to 135 Hug; the overridden one stays 200 Fixed | the instance's own resizing never followed its main | fixed in #2099 (open) |
| Undo after the Fill change | unreliable in the web app (reverted the child, left the main Fixed) | not compared | inconclusive (not used as evidence) |

## Components

Third entry of the sidebar header's component dropdown, several layers selected.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Create component set on frames named "Primary", "Secondary", "Ghost" | one set named "Component 1"; variants show their bare names; one property "Property 1" with the names as values, in canvas order left to right | no Create component set | fixed in #2095 (open) |
| Frames named "Size=Small", "Size=Large", "Size=Medium" | set "Component 1"; property "Size", values parsed from the names (Layers shows "Small", "Large", "Medium") | no Create component set | fixed in #2095 (open) |
| Three frames all named "Card" | set named "Card"; property "Property 1", every variant "Default"; the panel warns that some variants share property values, with "Select conflicting variants" | no Create component set | fixed in #2095 (open) |
| Layout of the new set | no auto layout; the frames stay where they were; the set (purple dashed frame, stroke #8A38F5 inside 1 px, corner radius 5) wraps them with 20 px on every side; the set is selected afterwards; one undo reverts everything | no Create component set | fixed in #2095 (open) |
| Wrap style, caret in one paragraph | Balance re-wraps only that paragraph; with the layer selected the control shows "Mixed" | a wrap style per layer only | fixed in #2094 (open) |
| Wrap style set on the whole layer | Pretty overwrites the paragraph's Balance; editing again, both paragraphs show Pretty | a wrap style per layer only | fixed in #2094 (open) |
| Text style carrying the wrap style | not checked | not compared | not measured (not checked) |

## Number field scrubbing

Real drags on the field's label; values read from the input.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| X from 100 | a 12.2 CSS px drag right gives 106; a 61.2 CSS px drag gives 131; about 0.5 per CSS px, whole numbers | 1 per point, with a speed factor by height | fixed in #2102 (open) |
| Rotation | a 12.2 CSS px drag right on its icon gave -6 degrees (one clean run; direction to confirm) | 1 per point | fixed in #2102 (open) |
| Opacity from 100% | 12.2 CSS px left gives 95% (about 0.4 per px); clamped at 100 going right | 1 per point | fixed in #2102 (open) |
| Shift and Alt rates | synthetic events did not reach the label's handlers | ten times with Shift, a tenth with Alt | not measured (not deliverable through the browser) |
| Pointer lock | not measured | not compared | not measured |

## Sidebars

Read with `getBoundingClientRect` at a 1920 CSS px viewport, device pixel ratio 1.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Left sidebar | default 240, minimum 240, maximum about 450 (451 with the border) | 200 to 480 | fixed in #2103 (open) |
| Right sidebar | default 240 (241 with the border), minimum 240, maximum 500 | 240 to 480 | fixed in #2103 (open) |
| Edge | a double-click resets to the default; the hit zone is a couple of px wide | double-click resets | matches |

## Auto-pan at the viewport edge

Synthetic held drags at 100% zoom, pan speeds read from timed holds (0.5, 1.0, 1.5 and 3.0 s, linear in time).

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Moving a layer with the pointer near an edge | 40, 25 and 15 px inside: no pan; 12 px inside about 120 px/s; 8 px about 275 px/s; 2 px 520 to 530 px/s; 30 px outside about 600 px/s | no auto-pan for any drag | fixed in #2104 (open) |
| Fit of the speed | speed = clamp(40 x (15 - d), 0, 600) screen px per second, d = distance inside the edge (negative outside); zone about 14 to 15 px; cap about 600 px/s; no acceleration | n/a | fixed in #2104 (open) |
| Marquee, drawing a rectangle, dragging a prototype connection | the same pan (bottom, left and right edges tested) | none | fixed in #2104 (open) |
| Resize, rotate and other gestures | not measured | not compared | not measured |
| Speed at other zooms | not measured (taken as screen px per second) | not compared | not measured |

## Vector edit clipboard

Real keys and clicks, an open pen path A-B-C-D (three segments), vector edit mode entered on it.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Copy a segment, leave edit mode, paste | a new layer holding only that segment, in place, sized to it | copied the whole layer | fixed in #2105 (open) |
| Copy one point, paste | a new layer holding the segments touching the point, in place | copied the whole layer | fixed in #2105 (open) |
| Cut a segment | removed from the network; the path splits into two subpaths in the same layer; edit mode stays; pasting outside gives a new layer with the cut segment, in place | deleted the segment without copying it | fixed in #2105 (open) |
| Paste while editing the same vector | no new layer; a duplicate of the segment is added to the network in place; the pasted points are selected | did nothing | fixed in #2105 (open) |
| Paste a copied rectangle while editing a vector | the rectangle's outline joins the network in place (four corners selected); no new layer; the bounds grow to include it | did nothing | fixed in #2105 (open) |

A click on a segment's middle can add a point there in Figma's edit mode (seen once); it did not change the rules above.

## Duplicate and paste

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Ctrl+D | a copy in place, selected, just above | same | matches |
| Move the copy +100, Ctrl+D | repeats +100 (smart duplicate); a reselect forgets the step | same | matches |
| Alt-drag +100, Ctrl+D | repeats (+200 in total) | same | matches |
| Paste with the original in view | in place on it, selected, just above | same | matches |
| Paste with the original off screen | at the view's centre | same | matches |
| Paste with a frame selected (far away) | inside the frame, at its centre | same | matches |
| Tests for the three paste placement rules | n/a | pinned by tests | fixed in #2108 (open) |
| Ctrl+Shift+V (paste over) | not measured (chord blocked or slow) | paste over at the selection's top-left | not measured (chord blocked or slow) |

## Selection keys

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Escape with a nested layer selected | one Escape selects nothing, whether the layer was selected on the canvas, from the Layers panel or by Tab | selected the parent first | fixed in #2107 (open) |

## Text-editing keys

Defaults for a new text: Inter Regular 12, line height Auto, letter spacing 0%.

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Ctrl+Shift+> and < | font size +1 and -1 (12 to 18 stepped) | step 1 | matches |
| Alt+Shift+> and < | line height +1 and -1 px; from Auto at 12 it starts about 15 (not exact) | from the resolved height, 1 px | matches |
| Alt+> and < | letter spacing +1% and -1% | 1% | matches |
| Ctrl+B, I, U on a selected layer | toggles bold, italic, underline | same | matches |
| Double-click a word while editing | selects the word | same | matches |
| Triple-click | selects the line (paragraph) | same | matches |
| Escape while editing | leaves editing, the layer stays selected | same | matches |
| Ctrl+A while editing | selects all the text | same | matches |

## Open questions

| Behaviour | Figma (measured) | FigCraft | Status |
|---|---|---|---|
| Add auto layout (Shift+A) defaults | two 40 by 40 layers side by side: horizontal, gap "Auto", padding 0, W Fixed (140), H Hug (40); the same with a Y offset of 30 drops the offset; stacked: vertical, gap 60 (numeric), W Fixed (40), H Hug (140); a single 40 by 40 layer: new frame, W and H Hug (60), padding 10, gap 10; an existing 300 by 200 frame with two layers: W stays Fixed (300), H becomes Hug (149), gap "Auto", padding 49 | Hug on both axes for a wrapped selection and an inferred gap | inconclusive (inconsistent between cases in Figma itself: the gap is "Auto" in one case and numeric in another, the cross axis Fixed in the vertical case; the inference has more inputs than measured) |
| Zoom to fit margins | see the zoom section | not changed | inconclusive (the toolbar overlay) |
| Space while drawing or marquee | not measurable | not compared | not measured (the browser does not deliver it) |
| Ctrl while moving | not measurable | Ctrl or Cmd turns snapping off | not measured (synthetic Ctrl is not delivered as a modifier) |
| Shift and Alt scrub rates | not measurable | ten times and a tenth | not measured (events did not reach the label) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-08 | minor | Created from the 2026-11-08 black-box comparison of FigCraft's interactions with Figma's, with synthetic files and no committed incumbent screenshots |
