# FigJam diagramming parity

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-15 · **Change:** major (CraftJam boards: a whiteboard file kind with shapes with text, connection points, connector labels, sticky notes, mind maps, tables, Mermaid paste, pens, the board's toolbars and palette)

FigJam, Figma's whiteboard, is out of scope as a separate product ([`target-app-parity.md`](target-app-parity.md)):
FigCraft has no `.jam` document kind. What people use it for, though, is often **diagramming**:
flowcharts, user flows, site maps and mind maps ([#470](https://github.com/storytold/figcraft/issues/470)).
So FigCraft has **CraftJam boards**: `.figcraft` files marked as boards (`Document::board`), opened
with a whiteboard interface, whose diagrams are ordinary layers (shapes, texts, connectors, frames)
marked where they need to be (plugin data), so they open, export and sync like anything else. This
page lists every FigJam behaviour from its public documentation and black-box observation and what
CraftJam does about it. Multiplayer and social features (cursor chat, high fives, emotes, stamps,
timers, voting, music) are listed for completeness and are not planned.

Sources are public only: help.figma.com articles, figma.com product pages and blog posts, and public
community forum threads, each cited by key below, plus black-box observation of the running web app
(key OBS: looking at the screen, hovering and selecting, nothing edited). Nothing from the
incumbent's application or web bundle or its network traffic was read; no images were downloaded.
Details the documentation doesn't state are marked "(unverified)". Every icon, colour, font choice and
shape outline in CraftJam is FigCraft's own.

**Status (estimated):** phases 0 to 2 (flowcharts and mind maps) are in, with most of phase 3 (curved
connectors, elbow handles, the shape set, shape swap, tidy up, sections, sticky notes, tables, Mermaid)
and the board's interface. Missing: routing around other layers, dragging labels along their line,
re-parenting mind map topics by dragging, washi tape, templates, and the social features. Remaining
for the diagramming scope: 10 to 20 h.

## CraftJam boards

- **Making one:** New CraftJam board on the home screen, or `file.newBoard`. The tab shows a note.
- **Interface:** no side panels. A dot grid on a light grey page (drawn on the canvas only); the file
  card (main menu, name) at the top left and the zoom at the bottom right; the bottom toolbar with
  select (V), hand (H), marker (M), highlighter (Shift+M), eraser (Shift+Delete), the sticky note pile
  (S; its colour is the next note's), square, ellipse, diamond, triangle, More shapes, connector (X),
  text (T), section (Shift+S), table (Shift+T), mind map and the + menu (mind map, table, sticky note,
  a Mermaid flowchart typed or pasted in).
- **Context toolbar:** dark, floating over the selection (under it when there's no room). Shapes:
  kind (searchable library), fill, border (solid, dashed, none, colour), font look (Simple, Bookish,
  Technical, Scribbled: the first installed of a few families each), size (Small 16, Medium 24, Large
  40, Extra large 64, Huge 96, or any), bold, strikethrough, link, bulleted list, alignment. Sticky
  notes: colour, square or wide, author. Mind map topics: child, sibling, collapse. Table cells: rows
  and columns in and out. Connectors: colour, thin or thick, solid or dashed, label, start and end
  points, line type. Three or more layers: Tidy up.
- **Palette:** 11 strong and 10 light named colours (`engine::board`), plus a free hex colour.
- **Defaults:** shapes pastel blue without a border, slightly rounded; texts 16 px, dark; connectors
  thin, mid grey, an open arrow, rounded elbows.
- Design files keep their own interface; what's on a board (connectors, shapes with text…) works in
  them too.

## What FigCraft has

Status: **done** (exists, with tests), **partial** (exists, with the gaps named), **missing**.

| # | Requirement | Status | FigCraft |
|---|---|---|---|
| 0.1 | Shape with text | **done** | a text that belongs to its shape (`Node::text_host`): inside its box (inset for ellipses, diamonds, stars and library shapes), wrapping, centred; follows it moving, turning and resizing (the font size stays); goes and comes with it (delete, duplicate, Alt-drag, copy, paste); a click on it is a click on the shape. Double-click a shape (or click it with the Text tool) to type; Enter still edits points |
| 0.2 | Connector layer | **done** | `NodeKind::Connector`: two ends, each attached (layer + magnet, or anywhere on its edge) or free; `connector.create`, `connector.detach`, `node.set` keys; MCP `create_node` |
| 0.3 | Basic flowchart shapes | **done** | rectangle, ellipse, diamond, triangle, star and 26 library shapes (`board_shapes`: rounded rectangle, parallelogram both ways, cylinder both ways, document, multiple documents, predefined process, manual input, manual operation, preparation, internal storage, delay, display, off-page reference, or, summing junction, hexagon, octagon, pentagon, plus, arrows, chevron, speech bubble, downward triangle) |
| 0.4 | Straight and elbowed routing that follows moved layers | **done** | re-routed after every edit; elbows straighten when the layers line up; no routing around other layers |
| 0.5 | Ends snap to the four side midpoints | **done** | magnets auto, top, right, bottom, left, centre; Cmd/Ctrl while dragging an end attaches it anywhere on the edge (kept as fractions of the box). Sides are the box's (a triangle's box, not its outline) |
| 0.6 | Line ends per end | **done** | none, line arrow (new connectors), triangle arrow, reversed triangle, circle, diamond |
| 0.7 | Stroke colour, weight, dashes | **done** | the palette; thin (2) / thick (4); solid / dashed; new connectors mid grey and thin (OBS) |
| 0.7a | Rounded elbow bends | **done** | 8 px radius, less on short segments (OBS) |
| 0.8 | Undo, selection, copy and paste, duplicate, delete | **done** | one undo step per edit; copies keep their links, texts and labels |
| 1.1 | Tool and shortcut | **done** | Connector tool (X) |
| 1.2 | Hover handles on shapes | **done** | four dots just outside a selected shape's sides; hovered, a "+" and the outline of the shape a click adds (OBS); a drag draws a connector from that side |
| 1.3 | Quick create (Cmd/Ctrl+Return, "+") | **done** | `connector.next`: a copy of the shape on that side (its text left behind), joined; fans out sideways when the place is taken |
| 1.4 | Connector labels | **done** | a text on the line, halfway, interrupting it (OBS: the board shows behind the words); double-click a connector, the T button or `shape.editText`; not draggable along the line yet |
| 1.5 | Dragging an end to reattach it | **done** | hollow rings on a selected connector's ends; drop on a layer (highlighted; not the other end's, nor a locked one) or free |
| 1.6 | Diagram colour palette | **done** | 21 named colours and a free hex colour, shared by fills, borders and connectors |
| 1.6a | Shapes' default look | **done** | on a board: pastel, no border, slightly rounded (OBS) |
| 1.6b | Floating context toolbar | **done** | see CraftJam boards above |
| 1.7 | Text formatting in shapes | **done** | font look, named sizes or any, bold, strikethrough, link, bulleted list, left / centre / right |
| 1.8 | Align actions and smart guides | **done** | FigCraft's own |
| 1.9 | Shape picker | **done** | More shapes in the toolbar and the context toolbar's shape menu: grouped, searchable, a picture of each |
| 1.10 | Command palette entry points | **done** | every board action is a command (Actions palette, CLI, MCP) |
| 2.1–2.6 | Mind maps | **done** | `mindmap.create`, `addChild`, `addSibling`, `collapse`; Tab or Cmd/Ctrl+Return a child, Return or Cmd/Ctrl+Shift+Return a sibling (also while typing); laid out after every edit (children to the right, stacked, centred); branch colours; topics sized to their words; deleting a topic deletes its branch; no dragging to re-parent |
| 2.7 | Collapse / expand branches | **done** | beyond FigJam: a badge counts what's hidden; a click shows it |
| 3.1 | Curved connectors | partial | one curve leaving and entering the attached sides at right angles; no reshape handles |
| 3.2 | Elbow path handles | **done** | a pill on the middle segment, dragged across; kept while the route keeps its shape |
| 3.3 | Full flowchart shape set | **done** | see 0.3 (no icon-style "advanced" or cloud shape sets) |
| 3.4 | Shape type swap | **done** | `shape.setType`: the same layer as another shape, its fill, border, text and connectors staying |
| 3.5 | Tidy up | **done** | `arrange.tidy` from the context toolbar for three or more layers |
| 3.6 | Sections | **done** | FigCraft's sections (name, colour, hide contents) |
| 3.7 | Sticky notes | **done** | `sticky.create`, `sticky.set`, the Sticky tool (S): ten colours, square or wide, grow with their words, signed by their author, a soft shadow |
| 3.8 | Tables | **done** | `table.create` (Shift+T), insert and delete rows and columns; rows grow with their words; no CSV paste |
| 3.9 | Mermaid paste | **done** | `board.pasteMermaid` and pasting on a board: the flowchart subset, laid out in layers |
| 4.1 | Export | partial | PNG and JPG draw boards as they look (without the dot grid); SVG and PDF leave out the backdrop under connector labels and sticky notes' author names; `.fig` export writes connectors as vectors |
| 4.4 | Marker / highlighter / eraser | **done** | the Pencil with an ink: marker thin or thick, highlighter see-through and darkening, Shift for straight lines; the Eraser rubs out ink only |
| 4.7 | Washi tape, stickers | missing | |
| n/a | `.fig` and REST JSON import | partial | connectors import as connectors; labels, sticky notes and shapes with text import as frames and texts, not as board objects |
| n/a | Routing around other layers | missing | |

Tests: `crates/engine/src/connector_tests.rs` (connectors), `crates/engine/src/shape_text_tests.rs`
(shapes with text, connection points, quick create, boards, the shape library, labels, sticky notes,
mind maps, tables, pens), `crates/engine/src/mermaid_tests.rs` (Mermaid, junk included), the `.fig`
and REST import tests, and the engine's junk-parameter sweep over every command.

## Requirements from the public documentation

Conventions:

- "Cmd/Ctrl" means Cmd on macOS and Ctrl on Windows/Linux. "Return/Enter" likewise.
- "(unverified)" marks a detail that comes from general knowledge, a third-party mirror, a community forum post, or that the official article did not state explicitly.
- Each table row cites its public source by short key. The full URL list is in the "Source keys" table below.
- Plan gating (free vs paid) is noted only where it hints at feature tiering; FigCraft can ignore it.

### Source keys

| Key | Title | URL |
|---|---|---|
| GUIDE | Guide to FigJam | https://help.figma.com/hc/en-us/articles/1500004362321 |
| EXPLORE | Explore FigJam files | https://help.figma.com/hc/en-us/articles/15300412458647 |
| SHAPES | Visualize information using shapes with text | https://help.figma.com/hc/en-us/articles/1500004414382 |
| CONN | Create diagrams and flows with connectors and lines | https://help.figma.com/hc/en-us/articles/1500004414542 |
| MIND | Create mindmaps in FigJam | https://help.figma.com/hc/en-us/articles/18917944627095 |
| STICKY | Sticky notes in FigJam | https://help.figma.com/hc/en-us/articles/1500004414322 |
| COLORS | Apply colors in FigJam | https://help.figma.com/hc/en-us/articles/1500004291341 |
| TEXT | Create text and links in FigJam | https://help.figma.com/hc/en-us/articles/1500004291281 |
| DRAW | Doodle and highlight with drawing tools | https://help.figma.com/hc/en-us/articles/1500004414442 |
| SECT | Organize your FigJam board with sections | https://help.figma.com/hc/en-us/articles/4939765379351 |
| LOCK | Lock and unlock objects in FigJam | https://help.figma.com/hc/en-us/articles/1500004291361 |
| SELECT | Select, move, and order objects in FigJam | https://help.figma.com/hc/en-us/articles/1500004292221 |
| GROUP | Group objects in FigJam | https://help.figma.com/hc/en-us/articles/1500004414962 |
| TABLE | Tables in FigJam | https://help.figma.com/hc/en-us/articles/12583849250199 |
| TABLEBLOG | Raising the (table) stakes on tables in FigJam | https://www.figma.com/blog/tables-in-figjam/ |
| STAMP | Stamps, emotes and high fives | https://help.figma.com/hc/en-us/articles/1500004290981 |
| CODE | Code blocks in FigJam | https://help.figma.com/hc/en-us/articles/4410965151127 |
| LINK | Add link previews in FigJam | https://help.figma.com/hc/en-us/articles/4414079911575 |
| TIMER | Stay on track with the timer in FigJam | https://help.figma.com/hc/en-us/articles/4402269549591 |
| VOTE | Run voting sessions in FigJam | https://help.figma.com/hc/en-us/articles/9359912208663 |
| MEET | Run meetings in FigJam | https://help.figma.com/hc/en-us/articles/8538436879767 |
| OPEN | Invite visitors to an open session | https://help.figma.com/hc/en-us/articles/4410786053911 |
| IMPEXP | Import and export with FigJam | https://help.figma.com/hc/en-us/articles/1500007927941 |
| EXPORT | Export your FigJam board | https://help.figma.com/hc/en-us/articles/4407699832855 |
| CROSS | Work between Figma and FigJam | https://help.figma.com/hc/en-us/articles/1500005916661 |
| A11Y | Use FigJam with a screen reader | https://help.figma.com/hc/en-us/articles/14477051168791 |
| AI | Generate boards and diagrams with FigJam AI | https://help.figma.com/hc/en-us/articles/18706554628119 |
| WIDGET | Use widgets in files | https://help.figma.com/hc/en-us/articles/4410047809431 |
| PLUGIN | Find and install plugins | https://help.figma.com/hc/en-us/articles/360040450413 |
| IPAD | FigJam for iPad | https://help.figma.com/hc/en-us/articles/4502073572247 |
| MMPAGE | FigJam mind map product page | https://www.figma.com/figjam/mind-map/ |
| BLOG22 | Little Big Updates, August 2022 | https://figma.com/blog/little-big-updates-august-2022/ |
| BLOG21 | What's new in Figma, September 2021 | https://www.figma.com/blog/whats-new-in-figma-september-2021/ |
| F-POINTS | Forum: custom connection points on shapes in FigJam | https://forum.figma.com/suggest-a-feature-11/launched-custom-connection-points-on-shapes-in-figjam-36105 |
| F-COLLAPSE | Forum: mind map collapse/hide/show request | https://forum.figma.com/suggest-a-feature-11/mindmap-figjam-collapse-hide-show-26682 |
| F-MERGE | Forum: mind maps cannot share child | https://forum.figma.com/suggest-a-feature-11/figjam-mind-maps-cannot-share-child-18526 |
| F-CURVE | Forum: curved lines update (2025) | https://forum.figma.com/ask-the-community-7/curved-lines-update-44509 |
| F-TIDY | Forum: dynamic drag to reflow (Tidy up is static) | https://forum.figma.com/suggest-a-feature-11/feature-request-dynamic-drag-to-reflow-handle-porting-auto-layout-wrap-to-canvas-49475 |
| OBS | Black-box observation of the running web app (screen only, hover and selection; nothing edited) | 2026-10-15 |
| F-TABLEKEYS | Forum: keyboard shortcuts to add table rows/cols | https://forum.figma.com/suggest-a-feature-11/keyboard-shortcuts-to-add-a-row-or-column-to-figjam-table-25829 |


### 1. Document, boards, pages, canvas

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| File kind | Separate whiteboard file type | FigJam files ("boards") are a distinct file kind from Figma Design files, created from the file browser. Local save format is `.jam` (unverified for current versions; Design uses `.fig`). | GUIDE, general knowledge |
| Regions | Three UI regions | The board (infinite canvas), the file panels at the top (left: file menu, file name, pages; right: spotlight, audio, AI, templates, comments, timer, music, voting, Share), and the main toolbar at the bottom. | EXPLORE, A11Y |
| Pages | Multiple pages per board | Left panel lists pages; add a page via the add control, name it, press Return/Enter. | GUIDE, EXPLORE |
| Canvas | Infinite board | Objects placed anywhere; no artboards/frames are required. | EXPLORE |
| Background | Fixed background | A very light grey board with a subtle dot grid that follows the zoom (OBS; older sources say white). No user-facing background colour setting documented (unverified whether one exists). Dark mode for the board itself: unverified. | EXPLORE |
| Pan | Mouse and trackpad panning | Hold Space and drag; two-finger trackpad scroll pans, following OS scroll settings. Hand tool also pans (H, unverified). | EXPLORE |
| Zoom | Zoom gestures | Cmd/Ctrl+Space+scroll, or pinch on trackpad. Current zoom level is shown (top right in older UI, bottom right zoom controls per A11Y). Standard Figma zoom shortcuts (Shift+1 fit all, Shift+2 zoom to selection, Shift+0 100%, Cmd/Ctrl +/-) are shared with Design (unverified for FigJam). | EXPLORE, A11Y |
| Templates | Template picker and publishing | Right sidebar or the "+" inserts menu opens templates (retros, meetings, diagrams, mind maps, 300+ community templates). Boards can be published as custom templates; a section can be set as the template/file thumbnail. | GUIDE, MEET, SECT, MMPAGE |
| Section templates | Template applied into a section | "Templates" control next to a section's title applies a template inside it. | SECT |
| Thumbnail | File thumbnail | Select a section and choose "Set as thumbnail". | SECT |
| Home / file browser | Create boards from home | Boards are created from the Figma file browser alongside Design, Slides and other file types (general knowledge). | general knowledge |
| Permissions | Edit vs view | Most creation features require "can edit"; spotlight and emotes also work with "can view". | STAMP, MEET |

### 2. Toolbar and tools

FigJam's bottom toolbar (called the "delightful toolbar") groups tools as: Navigation (Hand, Move/Select), Objects (Draw tools, Sticky, Shape, Connections), Tools (Text, Sections, Tables, Stamps and Emotes, Comments, Actions/AI), Inserts ("+": templates, widgets, plugins, sticker packs, component libraries, timer, mind maps, face stamps, code blocks, media and link embeds). Source: EXPLORE.

| Tool | Shortcut | Behaviour detail | Source |
|---|---|---|---|
| Select / Move | V | Default tool. Click, Shift+click, or marquee drag to select; drag to move; resize via bounding box. | SELECT, DRAW |
| Hand | H (unverified) | Pans the board without selecting. Note that EXPLORE also documents "hold H" for high five, so H is context dependent (unverified). | EXPLORE |
| Marker | M | Freehand doodle. Default thin gray stroke. Thin or thick weight; colour via picker; Shift constrains to a straight line. Each doodle is its own object, cannot be contained by a shape or sticky, can be grouped. Tool stays active until another tool or Esc. | DRAW |
| Highlighter | Shift+M | Semi-transparent stroke, thin or thick. Overdrawing or duplicating increases opacity. Highlights stick to objects they touch (stickies, images) and move with them. Shift = straight line. | DRAW |
| Washi tape | W | Click-drag to place decorative tape strip using a preset pattern or an uploaded image. Uploaded patterns are not persisted after the tab closes. Recommended custom image height 224 px. | DRAW |
| Eraser | Shift+Delete (Mac), Shift+Delete or Shift+Backspace (Win) | Click or drag over doodles and highlights to remove them. Only affects marker and highlighter strokes. Stays active until another tool or Esc. | DRAW |
| Stroke editing | n/a | Select stroke with V to change colour or weight after drawing. Straight strokes resize lengthwise only; freehand strokes resize proportionally. | DRAW |
| Sticky note | S | See section 4. | STICKY |
| Shape | R (square), O (ellipse) | Other shapes have no shortcut. See section 3. | SHAPES |
| Connector | X or Shift+L (elbow), L (straight) | Curved has no documented shortcut. See section 5. | CONN |
| Text | T | Single click = point text that grows as you type; click-drag = fixed-width area text that wraps. | GUIDE, TEXT |
| Section | Shift+S | See section 8. | SECT |
| Table | Shift+T | See section 9. | TABLE |
| Stamp | E | Opens the stamp wheel; pick a stamp then click board or object. See section 10. | GUIDE, STAMP, A11Y |
| Emote | via stamp wheel or right-click | Click = burst, click-hold = stream. Esc exits. | GUIDE, STAMP |
| Comment | C | Pin a comment to the board or an object. | GUIDE |
| Cursor chat | / | Type a temporary message attached to your cursor; Esc exits. | GUIDE, EXPLORE |
| High five | hold H next to another cursor | Celebratory gesture between two live cursors. | EXPLORE, BLOG21 |
| Stickers | Alt/Option+2 opens library modal | Stickers tab; stickers and components from published libraries. | GUIDE |
| Media | via "+" More > Media | Images and video. See section 11. | GUIDE, IMPEXP |
| Widgets | via "+" / Widgets tab | See section 11. | WIDGET |
| Code block | via "+" inserts | See section 11. | CODE |
| Quick actions | Cmd/Ctrl+/ or Cmd/Ctrl+P | Command palette: create any object by name ("sticky", "diamond", "create new comment"), run actions ("convert to stickies", "Accessibility settings"). Shows the last three actions; Up/Down cycles; Return repeats. | GUIDE, SHAPES, A11Y |
| Lock | Cmd/Ctrl+Shift+L | Locked objects cannot be moved, edited or have connectors attached; can still be selected, unlocked or duplicated. | LOCK |
| Group | Cmd/Ctrl+G (unverified) | Group objects; doodles can be grouped with others. | GROUP, DRAW |
| Shortcut panel | Help menu > Keyboard shortcuts (Cmd/Ctrl+Shift+? unverified) | Tray at the bottom listing shortcuts. No custom shortcuts. | TEXT, general knowledge |

### 3. Shapes and shape-with-text

#### 3.1 Shape library

| Group | Shapes | Gating | Source |
|---|---|---|---|
| Basic (14) | Square, Ellipse, Diamond, Triangle, Downward-pointing triangle, Rounded rectangle, Pentagon, Octagon, Plus, Left arrow, Right arrow, Chevron, Star, Speech bubble | any plan | SHAPES |
| Flowchart (16) | Right-leaning parallelogram (data / input-output), Left-leaning parallelogram, Cylinder (database), Horizontal cylinder (direct access storage), File, Folder, Document, Multiple documents, Predefined process (rectangle with double side bars), Shield, Trapezoid (manual operation), Manual input, Hexagon (preparation), Internal storage, Or, Summing junction | any plan | SHAPES |
| Advanced (26) | Activity, Archive, Authentication, Chat, Cloud, Computer, Database, Desktop, Email, File, Frontend, Instant, Location, Mobile, Package, Payment, Security, Send, Server, Service, Settings, Storage, Terminal, User, Wallet, Web (icon-style architecture shapes) | paid | SHAPES |
| Third-party sets | AWS, Azure, Google Cloud Platform, Kubernetes icon sets | paid (unverified) | SHAPES |

Notes for a clean implementation of the standard flowchart vocabulary:

- "Process" = Square/Rectangle; "Decision" = Diamond; "Terminator / start-end" = Rounded rectangle (pill); "Data (I/O)" = parallelogram; "Database" = Cylinder; "Document" = Document; "Predefined process" = Predefined process. These mappings are standard flowchart conventions (ISO 5807 style), not FigJam statements.
- FigJam does not list a separate "Delay", "Off-page connector", "Merge" or "Display" shape in the official list (as of research date).

#### 3.2 Shape behaviour

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Placement | Tool click, drag-from-toolbar, quick action | Select Shape tool, hover preview, click to place at default size; or drag shape from toolbar; or Cmd/Ctrl+/ then type name. | SHAPES |
| Shapes sidebar | Persistent picker | "More shapes" opens a sidebar listing all groups plus recently used shapes and line types. | SHAPES |
| Quick create | Cmd/Ctrl+Return on selected shape | Places a new shape of the same type next to the original, connected by a connector. | SHAPES, CONN |
| Duplicate | Cmd/Ctrl+D or Alt-drag | Duplicate keeps the original's text. | SHAPES |
| Resize | Corner vs side | Corner drag scales proportionally; side drag changes one dimension (square to rectangle, circle to oval). | SHAPES |
| Rotate | Snapped rotation | Hover near a corner and drag; rotation is in 15 degree steps only; contents (text) rotate with the shape. | SHAPES |
| Text inside | Every shape is a text container | Double-click or start typing on a selected shape to edit its text (unverified: type-to-edit). Text wraps within the shape width (auto-wrap). | SHAPES |
| Text size | Discrete sizes | Resizing a shape changes the room for text but not the font size. Size is picked from the toolbar: Small, Medium, Large, Extra large, Huge, plus a number field (Small = 16). | SHAPES, OBS |
| Font | Four named styles | Simple, Bookish, Technical and Scribbled (handwritten). | OBS |
| Auto-fit | Shape grows with text | Shape height grows when text overflows (unverified; official article does not state it; stickies explicitly grow). | general knowledge |
| Text formatting | Toolbar | Size, weight (bold), strikethrough, bulleted/numbered lists, links, horizontal alignment (left/center/right). Vertical alignment control: unverified. | SHAPES, TEXT |
| Colour | Palette with fill options | Colour picker on the selection toolbar sets colour and fill style/opacity. Default palette is a fixed set of named colours shared across stickies, shapes, connectors (about ten hues plus light variants, unverified exact list). Organization plans can define custom palettes. Bulk recolour on multi-select. | SHAPES, COLORS |
| Fill styles | Solid / light / outline | Shapes support at least filled and outline variants and an opacity choice (unverified exact set). | SHAPES |
| Stroke | Shape border | Border stroke follows the fill colour (darker tone) in the default style (unverified). | general knowledge |
| Swap type | Change shape kind in place | Selection toolbar lets you swap the shape type while keeping text and connections (unverified; not in the help article). | general knowledge |
| Connection points | Four side midpoints | Connectors snap to the sides of shapes. Forum discussion refers to "four default" connection points (top, right, bottom, left). A selected shape shows them as blue dots just outside the middle of each side; hovering one grows it into a "+" and shows a translucent outline of the shape a click would add on that side (connected); dragging from it draws a connector. | CONN, F-POINTS, OBS |

### 4. Sticky notes

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Create | S, toolbar click, drag from stack, quick action | Hover previews position; click drops sticky; you can type immediately. | STICKY |
| Quick create | Cmd/Ctrl+Return | New blank sticky beside the selected one, text field active. One at a time. | STICKY |
| Duplicate | Cmd/Ctrl+D, Alt-drag | Copy includes text. | STICKY |
| Content | Text only | Size, weight, lists via toolbar; multi-select edits apply to all. | STICKY |
| Colours | Fixed 10-colour palette | White, gray, green, teal, blue, violet, pink, red, orange, yellow. Each user's default sticky colour is randomised when they open a file; changing a sticky's colour also changes the toolbar stack colour for new stickies. Multi-select recolour; picker shows only two colours if the selection has more than two. | STICKY, COLORS |
| Author name | Shown at bottom | On by default, from the account name; toggle per sticky from toolbar/properties. Clearing the text of someone else's sticky replaces the author with you. | STICKY |
| Wide sticky | Two shapes | Square and rectangle (index-card / "wide" sticky). | STICKY |
| Size | Auto height | Height grows with text; width changeable by side drag; corner drag scales proportionally. Duplicated grown stickies re-fit when edited. Stickies cannot be rotated. | STICKY |
| Stacking / bulk | Tidy up and AI sort | No dedicated "stack" object is documented (unverified). Bulk organisation is via Tidy up (grid) and FigJam AI "sort stickies" by topic, colour, author, stamp count or type. | SELECT, AI |
| Table conversion | Table to stickies | Quick action "convert to stickies" on a selected table. | GUIDE |
| Export | CSV | Each sticky becomes a CSV cell; a "Created by" column appears only for stickies showing author name. | EXPORT |
| Connect | Like shapes | Stickies accept bent, curved, straight connectors and snap to their sides. | STICKY |

### 5. Connectors (in depth)

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Types | Elbow (bent), Curved, Straight | Elbow: orthogonal, reshapable, routes around objects. Curved: reshapable, custom path. Straight: direct segment. | CONN |
| Shortcuts | X or Shift+L elbow; L straight | No documented curved shortcut. | CONN |
| Create by drag | Tool drag between objects | Choose connector tool, drag from one object to another. Tool stays active until another tool, V, or Esc (sticky tool mode, since 2022). | CONN, BLOG22 |
| Create from shape handles | Hover handles on selected/hovered object | Hovering a shape or sticky shows connection handles / "+" arrows on its four sides; dragging from one draws a connector; releasing on empty canvas can offer to create a new shape there (unverified details). | general knowledge, SHAPES |
| Quick create ("+") | Click handle or Cmd/Ctrl+Return | Creates a new object of the same kind next to the source, automatically connected. Clicking a side's "+" builds in that direction (unverified for plain shapes; documented for mind maps). | SHAPES, CONN, MIND |
| Duplicate along connector | Repeated quick create | Repeating Cmd/Ctrl+Return keeps adding connected shapes; placement avoids overlap (unverified exact algorithm). | SHAPES |
| Snap to sides | Magnet points | Endpoints snap to the sides of objects (side midpoints), not arbitrary points. | CONN |
| Attach anywhere | Modifier | Hold Cmd/Ctrl while moving an endpoint to place it at any point on the object (no side snapping). | CONN |
| Follow objects | Live re-routing | Attached connectors move and re-route when connected objects move; elbow routes recompute; side choice may switch to the nearest side (unverified for the automatic side switch). | CONN, STICKY |
| Auto-straighten | Alignment assist | When connected objects look centre-aligned, elbow connectors straighten automatically. | BLOG22 |
| Path handles | Manual reshape | Drag a mid handle to reshape a segment; dragging near start/end can add a segment. Manual shape is not fully preserved when objects move (forum complaint). | CONN |
| Endpoint editing | Reattach | Select connector, drag a blue endpoint dot to another object or side. One endpoint at a time. Endpoints may be left unattached (floating) and connected later. | CONN |
| Move whole connector | Detach first | Both endpoints must be detached to drag the line freely. | CONN |
| Locked targets | No attach | Connectors cannot attach to locked objects. | LOCK |
| Line ends | Per end | Start and end caps independently: None, Solid arrow, Line arrow, Triangle, Diamond. | CONN |
| Stroke style | Solid / dashed | Toggle in toolbar. | CONN |
| Stroke weight | Thin / thick | Thin default. | CONN |
| Elbow bends | Rounded | The 90° bends of elbow connectors are rounded with a small radius. Connectors leaving or entering the same point overlap on their shared runs and read as a tree. | OBS |
| Selection | Ends and segment handles | A selected connector shows both ends as hollow circles (white, blue border) and a blue pill in the middle of each segment. | OBS |
| Colour | Palette | Default gray; file remembers last used connector style for new connectors. Bulk edit via Shift-click multi-select. | CONN |
| Labels | Text on line | As observed, a label interrupts the line (the board shows behind the text). Add from connector toolbar (also double-click on line, per Figma video; unverified). Return adds new row. Only bold and strikethrough formatting. Label can be dragged along the path when long enough. Optional label background, off by default, always same colour as the connector. | CONN |
| Curved rendering | Algorithm changed 2025 | Figma intentionally changed curved connector path generation in 2025. | F-CURVE |
| Keyboard creation | Limited | No documented keyboard method to create a connector between two existing objects; keyboard users rely on Cmd/Ctrl+Return quick create. | A11Y |
| Mermaid paste | Text to diagram | Pasting Mermaid.js syntax renders an editable flowchart, system diagram, or ERD. | CONN |

### 6. Diagramming support

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Flowchart workflow | Shape + connector + quick create | Primary flow: place shape (R/O/sidebar), Cmd/Ctrl+Return to chain, connectors snap to sides, labels on lines. | SHAPES, CONN |
| Alignment | Six align actions | Left, horizontal centres, right, top, vertical centres, bottom. Objects align only relative to each other (not to a frame). | SELECT |
| Distribute | Even spacing | Not documented as separate FigJam action (unverified); Tidy up covers it. | SELECT |
| Smart guides | Snapping while dragging | Alignment guides and equal-spacing hints while moving objects, inherited from Figma (unverified for FigJam detail). | general knowledge |
| Tidy up | Grid arrange | Select at least 3 objects, click Tidy up: arranges into uniform rows/columns. Afterwards, drag the dots/lines on the bounding box to adjust horizontal/vertical spacing or reorder while keeping spacing. Static: it does not reflow automatically later. | GUIDE, SELECT, F-TIDY |
| Auto layout of diagrams | Not present for flowcharts | No automatic graph layout for general flowcharts is documented. | (absence in CONN, SHAPES) |
| Diagram templates | Template library | Flowchart, org chart, user flow, architecture, mind map templates in the template picker and Community. | MEET, MMPAGE |
| AI diagrams | Note only | FigJam AI ("Make templates and diagrams" / Actions) generates editable flowcharts, mind maps, Gantt charts, org charts, sequence/state diagrams from a prompt; Mermaid paste; external integrations (Claude, Microsoft Copilot). Out of scope for FigCraft v1. | AI, CONN |

### 7. Mind maps

FigJam has a native mind map object (launched 2022, per community announcement), not only a template.

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Create | Several entry points | Shapes menu > Mind maps then click board; select a text object and click the Mind maps icon in its selection toolbar; quick actions; Inserts menu; FigJam AI prompt. A shape that already contains text cannot become a mind map root, but can be dragged onto an existing map. | MIND |
| Nodes | Text, shape or sticky | Nodes can be text fields, shapes, or stickies; hierarchy of parent/children; same level = siblings. | MIND |
| Add child (mouse) | Directional controls | Select a node, click the directional control on the side you want to grow (one-click branching), then click the new child to type. | MIND, MMPAGE |
| Add child (keyboard) | Cmd/Ctrl+Return | Quick-creates a child node. | MIND |
| Add sibling | Cmd/Ctrl+Shift+Return | Adds a sibling node. | MIND |
| Tab / Enter | Not used for creation | Official docs do not document Tab (child) / Enter (sibling) creation in the XMind style. Tab moves focus between siblings in accessibility navigation. (Tab-to-create: not supported, unverified.) | MIND, A11Y |
| Auto layout | Auto-formatting | Product page advertises "auto-formatting": children are positioned automatically when added (layout algorithm unverified). | MMPAGE |
| Collapse | Not available | Users requested collapse/expand of branches (2024); Figma did not commit. Treat as not supported (unverified whether later shipped). | F-COLLAPSE |
| Merge | Single parent only | A node cannot have two parents (tree, not graph). | F-MERGE |
| Attach existing | Drag to connect | Drag a sticky, shape or text over a node until the connect icon shows, release to attach as child. | MIND |
| Styling | Selection toolbar | Node and line colours, text style, line style and weight, line end points. Branch lines appear to be connectors (curved by default, unverified). | MIND |
| AI expand | Generate ideas | Select node, AI icon, "Generate ideas" adds children. Out of scope. | MIND |

### 8. Sections

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Create | Shift+S, toolbar, context menu | Click = default square section; click-drag = custom size; or select objects then right-click > Create section (wraps them); dragging a section over objects adopts them. | SECT |
| Name | Editable title | Double-click title or use toolbar; Return to commit; title can be removed and re-added. | SECT |
| Colour | Section colour | Set from the colour picker in the section properties menu (light tinted backgrounds, unverified exact palette). | MEET, SECT (via search snippet) |
| Containment | Moves with children | Moving a section moves contained objects. Moving or resizing a section over objects adds them; moving objects in with mouse or arrows adds them. | SECT |
| Hide contents | Toggle visibility | Eye icon / shortcut hides the section's contents without deleting (for reveals in workshops). Shortcut not captured (Cmd/Ctrl+Shift+H, unverified). | SECT |
| Lock | Two modes | "Lock all" (section and contents) or "Lock background only" (section fixed, contents editable). | SECT |
| Delete | Two modes | Delete/Backspace removes section and contents; Cmd+Delete (Ctrl+Backspace) removes only the section, keeping contents. | SECT |
| Nesting | Unverified | Not documented. | (absence) |
| Navigation | Unverified | A section list / jump-to-section navigation is not documented in the article. | (absence) |
| Voting scope | Per section | Voting sessions can be scoped to a section. | VOTE |
| A11Y | Focus scope | Tab cycles siblings within a section; Return selects children; Shift+Return selects parent. | A11Y |

### 9. Tables

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Create | Shift+T then drag | Drag sets the number of rows and columns. Plugin API default is 2x2. | TABLE |
| Cell text | Click into cell | Bold, links, lists, alignment supported. | TABLE |
| Add rows/cols | Hover handles | Hover table edge: blue bar at end appends; blue dot on a border inserts in the middle. No keyboard shortcut (users request Tab to add row). | TABLE, F-TABLEKEYS |
| Resize | Table and tracks | Corner drag resizes table; border drag resizes a row height or column width. | TABLE |
| Colour | Cell/row/column fill | Colour selection for cells/rows/columns (unverified detail). | TABLEBLOG |
| Paste | From spreadsheets | Paste from external spreadsheet creates/fills a table; CSV import creates a table. | TABLE, IMPEXP |
| Convert | To stickies | Quick action "convert to stickies". | GUIDE |
| Export | CSV | Table export to CSV. | EXPORT |

### 10. Social and meeting features (multiplayer)

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Stamps | Persistent reactions | E opens wheel; stamps placed on board or objects; persist; count is sortable by AI; face stamps (avatar) available; not customisable. Keyboard: Tab to object, Cmd/Ctrl+Return places stamp. | STAMP, A11Y, EXPLORE |
| Emotes | Ephemeral reactions | Click for burst, hold for stream; not persisted; usable by viewers. | STAMP, GUIDE |
| Reactions | Covered by stamps/emotes | No separate reaction object documented. | STAMP |
| Cursor chat | Ephemeral cursor bubble | "/" opens; message follows cursor; not logged; Esc clears. Not screen reader supported. | GUIDE, EXPLORE, A11Y |
| High five | Cursor gesture | Hold H next to another collaborator's cursor. | EXPLORE, BLOG21 |
| Timer | Shared countdown | Timer/voting/music panel; one timer per file; anyone can control; max 99:59; time can be added but not reduced while running; at 00:00 timer shakes for everyone. | TIMER |
| Voting | Hidden ballots | Host sets prompt and votes per person; ballots hidden until end; results shown to all; can scope to section; paid teams. | VOTE |
| Music | Shared background track | Preset tracks; everyone hears the same track; per-user volume/mute. | MEET |
| Spotlight | Follow presenter | "Spotlight me" from avatar menu; others get a few seconds to opt out, then follow presenter's viewport. Works for viewers. | MEET |
| Open sessions | Temporary guest access | Up to 24 hours of view/edit access for people without seats; no audio or comments for visitors. | OPEN |
| Audio | Built-in calls | Desktop app and browser; not mobile; not in presentation view. | MEET |
| Comments | Threaded comments | C to place; attach to board or object; mentions with @. | GUIDE, A11Y |
| Mentions in text | @ in text | Typing @ in a text field mentions a person. | GUIDE |
| Presenting | Spotlight and hide sections | Presentation flows rely on spotlight plus hidden sections; separate slide-style presenting of FigJam is not documented (unverified). | MEET, SECT |

### 11. Media, embeds, code, widgets, plugins

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Images | Import | PNG, JPG/JPEG, HEIC, GIF (does not animate on board), TIFF, WEBP; paste, drag-drop, or Media insert. | IMPEXP, LINK |
| Video | Import | MP4, MOV, WEBM. | IMPEXP |
| PDF | Import | PDF pages placed on the board (as images, unverified). | IMPEXP |
| Link previews | URL unfurl | Pasting a URL creates a static preview (title, description, URL, logo, image) or an interactive embed when the site supports oEmbed (play media, pan maps). Private resources show less. Paste-as-plain text with Cmd/Ctrl+Shift+V. | LINK, TEXT |
| Text links | Hyperlinks in text | Shift+Cmd/Ctrl+U adds link; pasting a URL over selected text links it. | TEXT |
| Code blocks | Syntax-highlighted code | Movable code object with language selector; default TypeScript; several languages supported. | CODE |
| Widgets | Interactive objects | Community widgets (e.g. Photobooth, Giphy, Asana, Jira) live on the board as objects with state shared by all collaborators. | WIDGET, GUIDE |
| Plugins | Extensions | Community plugins marked for Design, FigJam or both; run from the inserts menu / quick actions. | PLUGIN |
| Stickers | Library assets | Sticker packs and published component libraries; Alt/Option+2. | GUIDE |

### 12. Text (standalone)

| Feature | Shortcut (Mac / Win) | Source |
|---|---|---|
| Text tool | T | TEXT |
| Bold | Cmd+B / Ctrl+B | TEXT |
| Strikethrough | Cmd+Shift+X / Ctrl+Shift+X | TEXT |
| Bulleted list | Cmd+Shift+8 / Ctrl+Shift+8 | TEXT |
| Numbered list | Cmd+Shift+7 / Ctrl+Shift+7 | TEXT |
| Indent / outdent list | Tab / Shift+Tab | TEXT |
| Align left / centre / right | Opt+Cmd+L / T / R; Alt+Ctrl+L / T / R | TEXT |
| Add link | Shift+Cmd+U / Shift+Ctrl+U | TEXT |
| Paste without link | Cmd+Shift+V / Ctrl+Shift+V | TEXT |
| Accept spelling suggestion | Tab | TEXT |

Behaviour: point text auto-widens; area text wraps at fixed width; text can be converted into a mind map root. Source: GUIDE, TEXT, MIND.

### 13. Interop, import and export

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| FigJam to Design paste | Lossy but structured | Keeps colour, format, size and layout. Stamps become images; stickers/components become instances; marker drawings become vector paths with stroke; text and links become text with links. | CROSS |
| Design to FigJam paste | Objects paste in | Layers paste directly; reported edge case: auto-width text may become fixed width. | CROSS, IMPEXP |
| Cross-device paste | iPad to desktop | Paste banner appears on the other device (same account). | CROSS |
| Board import | Competitor boards | Import from Miro, Mural, Lucid, Google Jamboard. | IMPEXP |
| CSV import | Tables | CSV becomes a table. | IMPEXP |
| Export image | PNG, JPG, PDF | File > Export as; PNG/JPG background style option; JPG/PDF quality option. Export selection: Cmd/Ctrl+Shift+E. | EXPORT |
| Export CSV | Stickies and tables | Each sticky one cell; "Created by" column when author shown. | EXPORT |
| SVG | Not supported directly | Workaround: copy into a Design file and export. | EXPORT |
| Live sync | Not supported | No live link between FigJam and Design content (community requests only). | CROSS |

### 14. Accessibility and keyboard

#### 14.1 Accessibility behaviour

| Area | Requirement | Behaviour detail | Source |
|---|---|---|---|
| Opt-in | Accessibility settings | Quick actions > "Accessibility settings". "Adapt content" removes stickers, connectors and washi tape from tab order; "Include everything on canvas in the focus order" re-adds them. | A11Y |
| Regions | F6 / Shift+F6 cycle (Ctrl+F6 on Windows) | Board; selection properties menu (only if selection); top-left panel; top-right panel; creation tools; bottom-right zoom and help. | A11Y |
| Board navigation | Tree navigation | Tab / Shift+Tab next/previous sibling (same section or board root); Return selects children of a parent; Shift+Return selects the parent; Tab while editing text jumps to the next editable text field. | A11Y |
| Creation | Quick actions | "Create new <type>" places object at viewport centre or centre of selected section; Cmd/Ctrl+Return duplicates-beside. Lines and highlights need a mouse. | A11Y |
| Editing | Keyboard | Arrow keys nudge, Shift+arrow larger nudge; Return enters text edit; Esc exits; Fn+F6 (Mac) / Ctrl+F6 (Win) focuses properties menu. | A11Y |
| Coverage | Supported vs not | Supported: sections, stickies, shapes, tables, text, links, comments, stamps, code blocks, audio, timer, music, stickers, spotlight, voting, connectors, washi tape, widgets. Not supported: emotes, lines, highlights, cursor chat. | A11Y |

#### 14.2 Consolidated FigJam shortcut list

| Action | Shortcut | Confidence / source |
|---|---|---|
| Select / move tool | V | DRAW, CONN |
| Hand tool | H | unverified |
| Marker | M | DRAW |
| Highlighter | Shift+M | DRAW |
| Washi tape | W | DRAW |
| Eraser | Shift+Delete (Shift+Backspace on Win) | DRAW |
| Sticky | S | STICKY |
| Square / rectangle | R | SHAPES |
| Ellipse | O | SHAPES |
| Elbow connector | X or Shift+L | CONN |
| Straight connector | L | CONN |
| Text | T | TEXT |
| Section | Shift+S | SECT |
| Table | Shift+T | TABLE |
| Stamp / emote wheel | E | GUIDE, EXPLORE |
| Comment | C | GUIDE |
| Cursor chat | / | GUIDE, EXPLORE |
| High five | Hold H near another cursor | EXPLORE |
| Quick actions | Cmd/Ctrl+/ or Cmd/Ctrl+P | GUIDE, A11Y |
| Quick create next object | Cmd/Ctrl+Return | SHAPES, STICKY, MIND |
| Mind map sibling | Cmd/Ctrl+Shift+Return | MIND |
| Duplicate | Cmd/Ctrl+D or Alt+drag | SHAPES |
| Lock / unlock | Cmd/Ctrl+Shift+L | LOCK |
| Group | Cmd/Ctrl+G | unverified |
| Ungroup | Cmd/Ctrl+Shift+G | unverified (Figma-wide) |
| Delete | Delete / Backspace | DRAW, SECT |
| Delete section only | Cmd+Delete / Ctrl+Backspace | SECT |
| Hide / show section contents | Cmd/Ctrl+Shift+H | unverified |
| Export selection | Cmd/Ctrl+Shift+E | EXPORT |
| Stickers modal | Alt/Option+2 | GUIDE |
| Pan | Space+drag | EXPLORE |
| Zoom | Cmd/Ctrl+Space+scroll, pinch | EXPLORE |
| Zoom to fit / selection / 100% | Shift+1 / Shift+2 / Shift+0 | unverified (Figma-wide) |
| Undo / redo | Cmd/Ctrl+Z / Cmd/Ctrl+Shift+Z | general knowledge |
| Constrain stroke | Hold Shift while drawing | DRAW |
| Attach connector anywhere | Hold Cmd/Ctrl while dragging endpoint | CONN |
| Region focus | F6 / Shift+F6 (Ctrl+F6 Win) | A11Y |
| Exit tool / edit | Esc | DRAW, A11Y |


### 15. Suggested phasing for a diagramming-first implementation

Target: a solo-user diagramming capability (flowcharts and mind maps) inside FigCraft's regular design file, not a separate `.jam` document kind. Diagram objects are ordinary scene-graph nodes that coexist with frames, vectors and text. Phases are ordered from must-have to later.

#### Explicitly out of scope for the first version

Multiplayer and social features: cursor chat, high five, emotes, stamps (as live reactions), timer, voting, music, spotlight, open sessions, audio calls, presence avatars. Also out of scope: FigJam AI generation, widgets, third-party board import (Miro/Mural/Lucid/Jamboard), advanced/cloud icon shape sets, and plan gating. Comments are deferred unless FigCraft already has them for design files.

#### Phase 0: foundations (must-have, blocks everything)

| # | Requirement | Notes |
|---|---|---|
| 0.1 | "Shape with text" node type | A shape geometry plus an owned, wrapping text body; text rotates with shape; resizing changes the text box, not font size. |
| 0.2 | Connector node type | Persistent object with two endpoints, each either attached (node id + side/magnet or normalized edge position) or free (canvas point). |
| 0.3 | Basic shape set | Rectangle, rounded rectangle (terminator), ellipse, diamond (decision), triangle, parallelogram (I/O), cylinder (database), document, predefined process, hexagon. |
| 0.4 | Connector routing: straight and elbow | Re-route live when attached nodes move; auto-straighten when centres align. |
| 0.5 | Snap to four side magnets | Endpoint snaps to side midpoints; modifier (Cmd/Ctrl) attaches anywhere on the outline. |
| 0.6 | Line ends | None, arrow (solid), arrow (line), triangle, diamond, independent per end. |
| 0.7 | Stroke style and weight | Solid/dashed, thin/thick, colour. |
| 0.8 | Undo/redo and selection integration | Connectors and diagram shapes participate in existing selection, copy/paste, delete, duplicate. Deleting a node leaves its connectors with free endpoints (or deletes them; pick one and document it). |

#### Phase 1: fast flowchart authoring (must-have for usability)

| # | Requirement | Notes |
|---|---|---|
| 1.1 | Tool shortcuts | R, O, L (straight), X or Shift+L (elbow), T, V, Esc; connector tool stays active. |
| 1.2 | Hover handles on shapes | Four side handles; drag from a handle to draw a connector; drop on a node to attach, drop on empty canvas to create a connected copy (or offer a shape picker). |
| 1.3 | Quick create (Cmd/Ctrl+Return and "+" handles) | Create same-type node in a direction, auto-connected, auto-placed with a fixed gap, avoiding overlaps; new node enters text edit. |
| 1.4 | Connector labels | Text on line, draggable along the path, optional background matching connector colour; multi-line. |
| 1.5 | Endpoint editing | Drag endpoint dots to reattach; free-floating endpoints allowed. |
| 1.6 | Diagram colour palette | Fixed named palette (about 10 hues, light and dark variants) applied to shape fill, stroke and connector colour; "remember last used style" per document. |
| 1.7 | Text formatting in shapes | Discrete sizes, bold, strikethrough, lists, horizontal alignment; auto-grow height when text overflows. |
| 1.8 | Align actions and smart guides | Six align commands; reuse FigCraft's existing snapping and equal-spacing guides. |
| 1.9 | Shape picker panel | Basic and flowchart groups with recents; drag to canvas. |
| 1.10 | Command palette entry points | "Create diamond", etc. through FigCraft's quick actions, if it has one. |

#### Phase 2: mind maps (must-have for the stated goal)

| # | Requirement | Notes |
|---|---|---|
| 2.1 | Mind map container / tree model | Root plus children (strict tree, single parent). Nodes may be text, shape or sticky-like. |
| 2.2 | Keyboard creation | Cmd/Ctrl+Return child, Cmd/Ctrl+Shift+Return sibling (FigJam parity). Consider also Tab = child, Enter = sibling as an optional mode common in dedicated mind-map tools; this goes beyond FigJam. |
| 2.3 | Automatic branch layout | Re-layout subtree on insert/delete/text change; left/right/both-side growth by direction clicked. |
| 2.4 | Branch lines | Curved connectors by default, styled per branch (colour, weight, ends). |
| 2.5 | Directional "+" controls | One-click branching on the selected node's sides. |
| 2.6 | Drag to re-parent / attach | Drag an object onto a node to attach it as a child. |
| 2.7 | Collapse / expand branches | Not in FigJam (requested by users). Recommended as a FigCraft improvement in a later sub-step. |

#### Phase 3: richer diagramming

| # | Requirement | Notes |
|---|---|---|
| 3.1 | Curved connector routing | Bezier with reshape handles. |
| 3.2 | Elbow path handles | Manual segment drag, extra segment insertion; keep manual offsets when possible. |
| 3.3 | Full flowchart shape set | Remaining FigJam flowchart shapes: left parallelogram, horizontal cylinder, file, folder, multiple documents, shield, trapezoid, manual input, internal storage, or, summing junction; basic: pentagon, octagon, plus, arrows, chevron, star, speech bubble, downward triangle. |
| 3.4 | Shape type swap | Change shape kind preserving text and connections. |
| 3.5 | Tidy up | Grid arrangement for 3 or more selected objects, with drag handles for spacing and reorder. |
| 3.6 | Sections (diagram grouping) | Named, coloured containers that adopt objects dropped into them; lock all / lock background; delete section only. Map onto FigCraft frames or a dedicated "section" node. |
| 3.7 | Sticky notes | Square and wide variants, 10-colour palette, auto height, no rotation, optional author line (local user name). |
| 3.8 | Tables | Grid object with cell text, add/insert row/column handles, CSV paste. |
| 3.9 | Mermaid paste / import | Parse Mermaid flowchart syntax into editable shapes and connectors (requires an automatic graph layout, for example a layered Sugiyama layout). |

#### Phase 4: polish and output

| # | Requirement | Notes |
|---|---|---|
| 4.1 | Export diagrams | PNG/JPG/PDF of selection or page via FigCraft's existing export; SVG is a FigCraft advantage over FigJam. |
| 4.2 | CSV export of stickies/tables | Low priority. |
| 4.3 | Accessibility | Tree-based keyboard navigation (Tab siblings, Return children, Shift+Return parent), keyboard nudging, quick-action creation. |
| 4.4 | Freehand marker / highlighter / eraser | Useful for annotation, not for diagrams. |
| 4.5 | Code blocks, link previews, images in diagrams | Images already exist in a design tool; link previews need network access and can wait. |
| 4.6 | Diagram templates | Starter flowchart, swimlane, org chart and mind map templates shipped as files. |
| 4.7 | Washi tape, stickers | Decorative; lowest priority. |

#### Later or never (multiplayer, social, AI)

Cursor chat, high five, emotes, stamps as reactions, timer, voting, music, spotlight, open sessions, audio, presence, comments (unless FigCraft adds collaboration), FigJam AI generation, widgets, third-party board import, plan-gated icon shape sets (AWS/Azure/GCP/Kubernetes).

#### Notes on doing it "inside a regular design file"

- FigJam treats diagram objects as first-class board objects; in FigCraft they should be ordinary layers so they can live inside frames, be exported, and be mixed with vector work.
- Copy from FigJam to Design converts stickies/shapes to plain layers and drops connector semantics in some cases; FigCraft can do better by keeping connector attachments as real relationships in the document model.
- Keep connector routing deterministic and recomputed from attachments, storing only user overrides (manual handle offsets), so files stay small and diff-friendly.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | CraftJam boards: the board file kind and interface (toolbars, palette, dot grid), shapes with text, connection points and quick create, the shape library and swap, connector labels, middle-segment handles and ends anywhere on an edge, sticky notes, mind maps, tables, tidy up, Mermaid paste, pens; status table rewritten |
| 2026-10-15 | minor | Connectors get FigJam's default look (mid grey, thin, open line arrow, rounded elbow bends), Thin / Thick and Solid / Dashed in one click, and ends dragged to reattach (1.5 done); details from black-box observation (OBS): background, selection handles, connection points, text sizes and fonts, palette, context toolbar, labels |
| 2026-10-15 | major | Created: the FigJam requirements survey (sections 1 to 15, public sources only) and its status in FigCraft after the first slice, connectors (#470) |
