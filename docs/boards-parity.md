# Boards parity

> **Last reviewed:** 2026-10-24 · **Last updated:** 2026-10-30 · **Change:** minor (a FigJam `.jam` file opens as a board (#1342); board grid settings: dots, lines, graph paper or none, spacing, size and contrast; the connector tool shows the connector it will make while dragging, ends snapping to attach points (#1387); templates and tables; facilitation: a shared timer and voting sessions; the dot grid, dragging a connector's end to re-attach it, curved connectors, elbows that go around what they join) · **Target:** the incumbent's whiteboard product (behaviour from its public help centre)

**Breadth ~47%, ready ~29% (estimated, erring low).** A board is a FigCraft file of its own kind
(`file.newBoard`) with a dot grid on the page, sticky notes, shapes with text, connectors that
follow what they join (straight, elbow or curved, their ends dragged onto other objects), marker,
highlighter and eraser, stamps, text and sections, five built-in templates and tables, edited
together in the existing live sessions, with a shared countdown and voting sessions to run a
workshop. Music, board widgets, mind maps and the rest of the tail are missing. Remaining:
~20–44 h. Tracked as area 22 of
[`target-app-parity.md`](target-app-parity.md) (not in its weighted sum: the target there is the
design product). The work item is gap 24 in [`gaps.md`](gaps.md).

## How boards are built

Board objects are ordinary layers with a small `board` field on the node (`figcraft_doc::BoardItem`),
so rendering, hit testing, undo, saving, SVG/PNG/PDF export, copy and paste and live sessions work
on them unchanged. The file kind (`Document.kind`) is saved in `.figcraft` and reaches people who
join a live session in their snapshot. What follows from the metadata (a sticky's text size, a
shape's text room, connector routes and their labels) is recomputed in the engine's normalize pass,
deterministically from the document, so local edits, undo and edits from others in a live session
all end with the same geometry.

## Features

| Feature | Status | Notes |
|---|---|---|
| Board files: File ▸ New board, the home screen's New board card, a light page | done | `file.newBoard`; the kind shows in `inspect_document`, tabs and file rows (a note mark; design files a pen nib) |
| Board toolbar at the bottom | done | Move/Hand, Marker/Highlighter/Eraser, Sticky (colour flyout), Shapes (flyout), Connector, Text, Section, Stamp (flyout), Comment, Actions, then the templates and the Design panel's buttons — on the same 32 pt grid as a design file's (no Resources button, as there) |
| Design panel hidden by default; a contextual bar above the selection | done | colour, shape, text size (S/M/L/XL), connector straight/elbow/curved and its arrow; the Design panel opens with its button or View ▸ Design panel |
| Prototyping and Dev Mode refuse on boards | done | `proto.*`, `dev.*`, `view.prototype`, `view.devMode` say why |
| Sticky notes | done | a square frame (200 wide, growing taller through auto layout), 8 soft colours, a soft shadow, the note's text size shrinking as it grows (24 → 11, by characters and lines), the author's name at the bottom; a click places one and starts typing; Enter or a double-click edits; Shift+Cmd/Ctrl+Enter adds the next one beside it |
| Sticky options: hide the author, change size presets, sticky stacks | partial | `board.sticky {author: false}`; no size presets or stacks |
| Shapes with text | done | rectangle, rounded rectangle, ellipse, diamond, triangle, speech bubble, chevron; text centred and wrapping inside the outline; shape type and text size from the contextual bar |
| Shape auto-fit to its text, more shapes (cylinder, cross, star…), shape libraries | missing | |
| Connectors | done | from a layer's side (the facing one, or one named) to another layer or a free point, straight, elbow or curved, arrowheads (the stroke's caps), a label in the middle of the route by length (double-click to add); while one is dragged out it shows as it will be made (route, line and arrowhead), the object under the pointer outlined with its four attach points (side middles), the one the end goes to ringed, and within 10 px of one the end keeps that side; re-routed when either end moves or resizes; a deleted end stays where it was |
| Curved connectors, elbows around objects, dragging an end to re-attach | done | `board.setStyle {style: curved}` (and the contextual bar): one curve leaving and arriving along the sides; an elbow whose short way would cross what it joins goes around their boxes, 24 units clear; a selected connector shows its two ends as handles, and dragging one onto an object attaches it there (the object is outlined while dragging), onto empty canvas leaves it free (`board.reattach {id, end, target, side?}`, one undo step) |
| Connector waypoints placed by hand | missing | routes are computed from the ends; no bend points of your own |
| Quick-create handles (drag out a connected copy) | missing | |
| Marker and highlighter | done | the Pencil's stroke fitting with board presets: marker opaque, highlighter wide, see-through and multiplying |
| Eraser | done | drag over marker and highlighter strokes to remove them, one undo step |
| Pen colours and widths in the toolbar | partial | `tool.marker {color}`, `tool.highlighter {color}`; colour changes on a selected stroke from the contextual bar |
| Stamps | done | thumbs up, heart, plus one, star, question, check: round badges drawn as vectors, the placer's name on hover |
| Stamps sticking to the object they land on, the stamp wheel, emotes, stickers | missing | |
| Text and sections | done | the existing Text tool and sections |
| Live sessions: cursors, selections, cursor chat, spotlight | done | the existing sessions; board objects, connectors following moved objects, colours and stamps sync both ways (checked with two app instances) |
| Timer | done | a pill at the top right of the board: presets (1, 3, 5, 10, 15 min) or any whole minutes, pause, resume, a minute more, stop; red at 0:00 until stopped; no sound. `board.timer {action, minutes?}`, `board.timerState`. Synced as its end time (or the time left while paused), so every client counts down on its own clock: nothing ticks over the network. Not an undo step |
| Timer music, timer sounds | missing | |
| Voting sessions | done | one at a time: votes per person (1–99), on every object, on sticky notes or on the selection; a click on an object places a vote (several on one object allowed), a click on your badge or an Alt-click takes it back; your own votes show as badges while it runs, everyone's once it ends, with a ranked list (ties share a rank) that selects an object from its row; Clear results. `board.vote {action: start\|cast\|retract\|end\|clear}`, `board.voteResults`. Ballots sync per voter (keyed by the live-session client id, or your name offline), so people voting at once never overwrite each other. Not undo steps |
| Voting: only the facilitator ends or clears it, who voted for what | missing | anyone who can edit the board can end or clear a vote; results are counts only |
| Board widgets | missing | design-file widgets exist (`crates/engine/src/extensions/widgets.rs`), not wired to boards |
| Templates (retrospective, brainstorm, flow chart…) | partial | five built-in templates drawn up for FigCraft (retrospective, brainstorm, weekly planner, flow chart, kanban board), placed from the toolbar's templates picker (each with a picture of itself) or `board.template {name, x?, y?}` as ordinary sections, stickies, shapes, connectors and text in one undo step; `board.templates` lists them. No community templates, no saving your own |
| Tables | partial | `board.table {rows, columns, cells?, x?, y?}` (at most 50 × 20): an auto layout frame of rows of cells with text, a tinted header row and lines between rows and columns; a cell's row grows with its text; a double-click types in the cell under the pointer; add or remove a row or column from the bar over the selection or `board.tableEdit {id, action, index?}`; undoable and synced in live sessions. No column resizing by dragging, no cell colours, no Tab between cells, no CSV paste |
| Mind maps | missing | |
| Washi tape, code blocks, image cropping on boards | missing | images can be placed as in design files |
| Audio and huddles | missing | |
| AI features (summaries, sorting stickies, generating boards) | missing | |
| Grid on the page | done | the canvas render draws it after the page colour and before the layers (`figcraft_render::Dots`), so objects cover it. Dots by default, or lines, graph paper (a strong line every 2–12 cells over fainter ones, like Japanese section paper) or none; spacing (4–512), mark size and contrast are settings kept with the preferences (`board.grid`; Preferences ▸ Board grid). Zoomed out it thins out (dots and lines by halves, graph paper down to its strong lines) until the marks are at least 14 points apart (10 for lines), the marks the next step drops fading out first; dots stay about a point and a half across on screen (1 to 3 points), scaled by the size setting. Dark marks on a light page, light ones on a dark page, from the theme's ink; written straight into the image, no allocation per mark, a crossing of two lines blended once; never in exports. FigCraft's own settings: the incumbent's board grid has none |
| `.jam` import and export | **partial**: a `.jam` opens as a board, best effort (#1342): its stickies and shapes with text come in as frames with their generated layers and its connectors as vectors, not as native board objects; no export | see [`file-format-parity.md`](file-format-parity.md) |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | minor | M12, #1342: a FigJam `.jam` file opens as a board, our whiteboard file kind, instead of a design file: the `.fig` reader marks a canvas with FigJam's `fig-jam.` header as `FileKind::Board`, so the file opens with the board toolbar, the dot grid and the board panels, and saves as a board. A `.fig` stays a design file, and `file.import` of a `.jam` adds its pages to the open file without changing its kind. Its stickies and shapes with text are still frames with their generated layers and its connectors vectors, not native board objects. Tests: `board_import_tests.rs` (the kind, `.jam` against `.fig`), `jam_open_tests.rs` (open as a board, import into a design file). |
| 2026-10-29 | minor | M12: board grid settings (`board.grid`): dots, lines, graph paper or none, with spacing, size and contrast, in Preferences; the default grid looks as before. Numbers unchanged (beyond the incumbent's behaviour) |
| 2026-10-29 | minor | M12, #1387 (connector tool): while a connector is dragged out with the connector tool, the canvas shows the connector itself as it will be made (its straight, elbow or curved route, line, weight and arrowhead, built by the same code as `board.connector`), not a thin line; the object under the pointer is outlined with its four attach points (the middles of its sides), the one the end goes to ringed; within 10 px of one the end keeps that side (`fromSide`/`toSide`, and `side` when re-attaching). Hovering with the tool shows the attach points too. Escape drops the preview; releasing makes the connector in one undo step. Tests: `board_connector_tests`. Numbers unchanged |
| 2026-10-24 | minor | M12: the dot grid on board pages (a render pass under the layers), dragging a selected connector's end onto another object or empty canvas (`board.reattach`), curved connectors, and elbows that go around the objects they join. Breadth ~35% → ~37%, ready ~20% → ~21%, ~30–60 h → ~28–56 h |
| 2026-10-24 | minor | Facilitation (M12): a shared timer and voting sessions. Breadth ~35% → ~40%, ready ~20% → ~24%, ~30–60 h → ~26–54 h |
| 2026-10-24 | minor | Templates (retrospective, brainstorm, weekly planner, flow chart, kanban board) from a toolbar picker or `board.template`, and tables (`board.table`, `board.tableEdit`): breadth ~35% → ~40%, ready ~20% → ~24%, remaining 30–60 h → 26–54 h |
| 2026-10-24 | major | Created with board files (M12): sticky notes, shapes with text, connectors, ink and eraser, stamps, the board toolbar and contextual bar, live-session sync |
