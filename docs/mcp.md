# FigCraft MCP server

FigCraft ships a [Model Context Protocol](https://modelcontextprotocol.io) server so AI agents can
design with it: create and edit layers, run any menu command, inspect the result as JSON and look at
it as a PNG. It speaks newline-delimited JSON-RPC 2.0 over stdio (MCP protocol `2025-06-18`;
`2025-03-26` and `2024-11-05` clients are accepted too).

Incoming lines are limited to **32 MiB before the newline**, including whitespace. The limit is
enforced while reading: an oversized or unterminated frame stops at the first excess byte, returns
JSON-RPC error `-32600` (`message too large`, null id), and closes the stdio session without reading
the rest or dispatching later requests. Lines within the limit keep the same JSON-RPC behavior.

The server lives in `crates/mcp` (`figcraft-mcp`) and is started by the CLI:

| Command | Backend |
|---|---|
| `figcraft-cli mcp` | **Headless**: an in-process engine session with one untitled document. No window; renders with the CPU renderer. |
| `figcraft-cli mcp --attach <port>` | **Attached**: drives a running FigCraft app through its control channel (`figcraft --control <port>`). The user sees every change live. |

`--attach` also takes `host:port`. The control channel only listens on 127.0.0.1.

While an attached agent works, the app shows where, as it shows a person in a live session: its
avatar, in a colour of its own and named as its comments are signed ("Claude (agent)", see
Comments), joins the people row at the top of the right panel, and the canvas outlines the layers
its last edit changed, or that it last selected, with its name. Both go 8 seconds after its last
command, and only the file it works in shows them. This is display only: no undo step, nothing
saved, nothing sent to a live session's other people, and nothing changes in what the agent may
do.

Attached calls are never automatically replayed after a transport failure: the app may have
applied the request before its reply was lost. The error reports that the operation may have
completed; inspect the document before retrying an edit. The next explicit call reconnects.
If that connection cannot be established, the error says the request was not sent.
A reply from the app is read up to **512 MiB before the newline** (the largest window capture is
256 MiB as base64). The backend stops reading a longer reply at the first byte past the limit and
treats it like a lost reply: the connection is closed, the operation may have completed, and the
next call reconnects. For larger exports, give `export` a `path` or lower the scale.
A connection the app closed while it sat idle is noticed before the next request is written and
replaced then, so a pause between calls is not an error.

The app keeps answering while its window is minimised or covered by other windows (see
`docs/control-protocol.md`): edits, inspection (`inspect_ui` too), pointer, key and text input and
renders work as usual (minimised, each request to the app takes up to about 0.1 s). Only a capture
of the window itself (`screenshot` with `window`) needs it on screen: it is then an error at once;
`screenshot` without `window` renders the document instead.

### Confining file access

The roots below belong to the **headless** side: the session `figcraft-cli mcp` runs itself, with
no window. A server attached with `--attach` runs the commands inside the app instead, so the app
confines them — from the flags it was started with, not from anything passed to the CLI.

```sh
figcraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project/out
```

```json
{"mcpServers": {"figcraft": {"command": "/path/to/figcraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"]}}}
```

The two flags (the other Craft apps take the same ones) keep an agent to the folders you give it.
Each may be given more than once, as `--flag DIR` or `--flag=DIR`:

- Every file a tool call reads must lie inside a **read root or a write root**; every file it writes
  inside a **write root**. Anything else is refused with an ordinary tool error the agent can read
  and correct, for example `automation path rejected: /etc/x.png is outside the folders that may be
  written: /work/project/out`. Nothing is written and the server keeps serving.
- Read and write authority are separate. With only `--automation-read-root` nothing can be written
  to disk (`export` and the export commands without a `path` still return the bytes). Without either
  flag nothing changes.
- The roots must be existing folders: a missing one, or a file, stops the server at startup.
  `initialize`'s `instructions` name the folders, so the agent knows where it may work.
- **What is covered:** everything a call does, because the engine reaches the file system through
  one module (`crates/engine/src/automation/file_access.rs`), and `crates/engine/clippy.toml` refuses `std::fs`
  anywhere else in the engine, so a command added later is covered too. That is `open_file` (and the
  `variables` file it may name), `save_file`, `export {path}`, and through `run_command` / `batch`
  every command that names a file: `file.open`, `file.openRecent` (recent files outside the roots
  are never listed, opened, removed or named: `index` counts the files listed), `file.import`,
  `file.placeImage`, `image.place`, `file.save`, `file.saveAs`, `file.revert`, `file.recover`,
  `file.export`, `file.exportSvg`, `file.exportLayers` (and the folders it makes), `file.exportFramesPdf`, the images and variables JSON read next to an imported REST API
  file, `library.add` and the library files open documents link to, `variable.import` /
  `variable.export`, `codeConnect.import` / `codeConnect.export`, `component.playground {path}`,
  `plugin.import {path}`, a development plugin's module read again from disk,
  `plugin.directory.publish {path}` and its assets, `collab.recoverFile {path}`, and the headless
  `ui.render` / `ui.screenshot` with a `path`.
- **How paths are resolved:** a relative path is made absolute against the server's working
  directory, then its links are followed: the real path is what is compared and what is opened. For
  a file that doesn't exist yet, its nearest existing folder is resolved and the rest must be plain
  names, so `..` below a folder that doesn't exist is refused, and so is a link that leads nowhere
  (writing through it would create its target, wherever that is). A path outside the roots gets
  the same refusal whatever exists there, so a refusal doesn't tell what is outside. Folders are
  compared by whole names: `/work2/x` is not inside `/work`. On Windows the case comes from the disk (`C:\WORK\x` is
  inside `C:\work`), `\\?\` and UNC (`\\server\share\…`) paths are resolved like drive-letter ones,
  and new names that are devices (`CON`, `NUL`, `COM1`…) or streams (`a.png:x`) are refused.
- Not confined: the app's own folders, which no tool names (preferences, Data Recovery copies,
  plugins installed from a plugin directory, the system fonts).
- `--attach` with either flag is a usage error: the app runs the commands, so only the app can
  confine them. Start it with the same flags, `figcraft --control 7979 --automation-read-root DIR
  --automation-write-root DIR` (see [Control protocol](control-protocol.md#confining-file-access)),
  then attach as usual. The attached `screenshot` gets its PNGs from the app as data, so no
  temporary file is written anywhere.
- A path is checked, then opened: another program that swaps a folder for a link in between can
  still win that race. The roots keep an agent to the files it was given, not other software on the
  computer; if that matters, run the server as a user that can't change the roots' folders.

### Fonts

The headless server lays text out with the same fonts as the app: the fonts built into FigCraft
plus the installed system fonts, indexed once, the first time text is laid out (a session that
never touches text never pays for it). So text sizes, auto layout and exports match what users see
in the app. For renders that come out the same on every machine (CI), start it with
`figcraft-cli mcp --no-system-fonts` or set `FIGCRAFT_NO_SYSTEM_FONTS=1`: text then uses only the
built-in fonts. Attached, the app's own fonts apply. The browser build has no system fonts.

## Configure a client

Build the CLI first (`cargo build --release -p figcraft-cli`) and use the absolute path to
`target/release/figcraft-cli`.

The Linux AppImage bundles the CLI: `FigCraft.AppImage cli <args>` runs `figcraft-cli <args>`, so
`/path/to/FigCraft.AppImage cli mcp` works wherever `/path/to/figcraft-cli mcp` appears below.

### Claude Code

```sh
# headless
claude mcp add figcraft -- /path/to/figcraft-cli mcp
# drive the running app (start it with: figcraft --control 7979)
claude mcp add figcraft-app -- /path/to/figcraft-cli mcp --attach 7979
```

Or in a project's `.mcp.json`:

```json
{
  "mcpServers": {
    "figcraft": { "command": "/path/to/figcraft-cli", "args": ["mcp"] }
  }
}
```

### Claude Desktop

`claude_desktop_config.json` (Settings > Developer > Edit Config):

```json
{
  "mcpServers": {
    "figcraft": { "command": "/path/to/figcraft-cli", "args": ["mcp"] },
    "figcraft-app": { "command": "/path/to/figcraft-cli", "args": ["mcp", "--attach", "7979"] }
  }
}
```

## Conventions agents must know

- Coordinates are **page units** (1 unit = 1 px at 100% zoom), x right, **y down**.
- `create_node` and `set_properties` take `x`/`y` **relative to the parent frame** (the page when
  there is none, so they are page coordinates then). Groups and boolean groups have no coordinate
  space of their own: a layer in a group is placed in the frame (or on the page) the group is in.
  `inspect_document` reports `x`/`y` the same way, plus `absoluteBounds` in page coordinates.
  In `props`, `fill` and `stroke` also accept `"none"` (either case) or `null` as no paint, the
  same as an empty `fills`/`strokes` list, so an outline-only shape can be made in one call.
- Frames, groups, sections and masks: drawing a frame or section with the `pointer` tool around
  layers puts those fully inside it in it; sections go only on the page or in sections (moves,
  drags, pastes and new sections elsewhere are refused or land on the page); resizing a section
  leaves its contents where they are, and so does resizing a frame with `cmd` held (constraints
  ignored); a layer a mask hides can't be clicked there, and a mask group's box and exports are
  the mask's (plus any layers under the mask).
- Children of an auto-layout frame are positioned by the layout: set `layout`, `gap`, `padding`,
  `mainAlign`, `crossAlign`, `sizingH`/`sizingV` instead of x/y. In one call, `layout` applies
  before the keys that tune it, so `{"layout": {"mode": "vertical"}, "gap": 8}` keeps the gap;
  auto layout keys beside a `layout` of `null` or `"none"` are refused. More auto layout props:
  - `sizingH` / `sizingV`: `fixed`, `hug` (auto layout frames and text) or `fill` (children in an
    auto layout flow). A child can't fill an axis its parent hugs: setting `fill` turns the parent
    `fixed` on that axis, and a parent set to `hug` turns the children that filled it `fixed`. A
    layer moved out of auto layout (or whose frame loses it) keeps its size as `fixed`. Fill
    children share the space like CSS flexbox, within their min/max.
  - `minWidth` / `maxWidth` / `minHeight` / `maxHeight` (px, `null` removes) bound Hug, Fill and
    Fixed sizes; a hugging text wider than its max width wraps.
  - `absolute: true` takes a child out of the flow (it stays where it is and follows its
    `constraints` when the frame resizes; Fill becomes Fixed).
  - `mainAlign` takes `min`, `center`, `max` or an auto gap: `spaceBetween` (no space at the
    edges), `spaceEvenly` (the same space in every gap and at the edges) or `spaceAround` (gaps
    twice the edge space). An auto gap never goes below 0.
  - `padding` never lets a frame get smaller than it: a fixed or hugging frame (or one with a
    smaller max size) sizes up to its padding.
  - `strokesIncluded` (default `true` for new auto layout): the frame's own inside stroke takes up
    space like padding (per side with `strokeSides`); center and outside strokes never do.
  - `reverseZ` (canvas stacking): `true` paints and hits the first child on top, `false` (default)
    the last. The layer order doesn't change.
  - `crossAlign: "baseline"` lines up the first text baselines in horizontal layouts (other layers
    align to the top; nested auto layout frames use their first child's baseline).
  - `wrap` with `crossGap` (alias `rowGap`) for the gap between wrapped rows (horizontal) or
    columns (vertical layouts wrap top to bottom into new columns).
  - `object.convertToGroup` / `object.convertToFrame` turn a frame into a group or back in place
    (layers stay put; a group has no auto layout, fill, stroke, corner radius, clipping or grids).
  - `object.convertToSection` turns a frame on the page or in a section into a section in place, and
    `object.convertToFrame` turns a section back into a frame (keeping its fill, not clipping).
  - `contentAlign` (wrapping layouts): `auto` (default) packs the lines `crossGap` apart at the
    start, centre or end `crossAlign` names (lines of nothing but fill children stretch instead);
    `spaceBetween` puts the free space between the lines (one line, or no free space: like `auto`).
  - Grid (`layout: "grid"`): `gridColumns`, `gridRows` (minimum; rows grow), `columnGap` /
    `rowGap`, track sizes `columnTracks` / `rowTracks` as `[120, "1fr", "auto"]` (px fixed, `fr`
    fill share, `auto` hug) or the `layout.setTrack {axis, index, sizing, value?}` command; on a
    child, `gridColumn` / `gridRow` (0-based anchor cell, `null` = placed automatically) and
    `gridColumnSpan` / `gridRowSpan`, and `gridAlignH` / `gridAlignV` (`min`, `center`, `max`;
    `auto` or `null` uses the frame's alignment) to align it in its cell. `fr` tracks behave like
    the CSS `fr` unit: a track never gets less than its content (a fixed-size child bigger than
    its share keeps it; the other `fr` tracks share the rest). `inspect` reports a grid child's
    resolved `gridCell`.
- Layer order: `node.reparent {ids?, parent, index?}` moves layers into `parent` (keeping their place
  on the canvas, or taking their place in its auto layout flow) and `node.reorder {id, index}` moves
  one within its parent. `index` counts the parent's children bottom first (in auto layout, the
  first in the flow) and is where the (first) moved layer ends up after the move, in the same parent
  or a new one; several layers land together in their stacking order. No `index`, or one past the
  end, puts them on top (last in the flow).
- **Every menu action is a command** (`list_commands`), and most commands act on `ids` when given,
  else on the selection. New layers become the selection.
- Colours are `"#RRGGBB"` or `"#RRGGBBAA"`. Inside fills and strokes, `color` may also be a hex
  string and gradients can be written `{"type": "linearGradient", "stops": ["#7048E8", "#1098AD"]}`;
  the engine reads them as its paint JSON, so `run_command`, `figcraft-cli run` and the control
  channel take them too. Effects passed through the MCP tools may use hex `color` strings as well.
- **Blend modes**: `blend` on a layer (`node.set`) or a paint (`fill.set` / `stroke.set`) is a
  menu label or CSS keyword (`"Normal"`, `"pass through"`, `"multiply"`, `"plusDarker"`, ...;
  `normal` is Normal). Groups and frames start as pass-through: their children blend with what is
  behind them, through clipping too (SVG exports put a clipping frame's clip on what it holds, since
  a clipped group would isolate it); Normal blends the children among themselves first (SVG
  `isolation:isolate`, a PDF isolated group). Plus darker is exact in renders and pixel exports;
  PDF has no such mode (multiply) and SVG writes the CSS `plus-darker` keyword. Inspecting a layer
  reports a paint's `blend` when it isn't normal.
- **Drop shadows** hide behind what their layer covers, its fills (translucent and 0% ones too)
  and strokes, unless the effect has `showBehind: true`; groups, and layers whose pixels a layer
  blur or texture moves, hide them behind their pixels instead. SVG exports cut them out the same
  way, with filter regions as large as the effects reach.
- **Colour profile**: colour values are in the file's profile, sRGB (default) or Display P3
  (`inspect_document` reports `colorProfile`). `file.colorProfile {profile: srgb|displayP3,
  convert?}` changes it: `convert` (default true) converts every colour value so colours keep their
  appearance (a colour calculation's mix keeps its ends exact; its colours in between move
  slightly); `false` keeps the values and reinterprets them. Images are converted from their own colour
  profile into the file's space: an embedded ICC profile (PNG, JPG, WebP, TIFF; RGB or grey), a PNG's
  `sRGB` / `gAMA` / `cHRM` chunks, else sRGB (also for CMYK or damaged profiles); `image.place` reports
  the profile's name as `imageProfile` (`null`: none usable, taken as sRGB). PNG exports are tagged (`sRGB` chunk, or a Display P3 ICC profile plus
  `cICP`), Display P3 JPG and WebP embed the ICC profile, and SVG and PDF export Display P3 files
  converted to sRGB (out-of-gamut colours clip). The app's canvas shows sRGB (P3 colours outside sRGB clip on screen):
  `view.proofSrgb {on?}` (View ▸ Soft-proof sRGB) stripes those pixels so `screenshot {window:
  true}` shows where they are (after the next canvas render), and the colour picker says Outside
  sRGB for such a colour.
- **Image adjustments**: image paints carry `filters` (exposure, contrast, saturation, temperature,
  tint, highlights, shadows; each -1..1). `image.adjust {ids?, target?: fill|stroke, index?,
  preset?, filters?}` applies a built-in preset (`vivid`, `muted`, `warm`, `cool`, `blackAndWhite`,
  `fade`, `reset`) or one saved in the file, with any `filters` values on top.
  `image.savePreset {name, filters?}` saves the current (or given) adjustments in the file,
  `image.deletePreset {name}` removes one and `image.presets` lists both kinds.
  `image.crop {ids?, index?}` (one layer) switches its top visible image fill (or fill `index`) to
  Crop, keeping the picture where it was, and returns `{id, index}`; the app also enters crop mode
  on the canvas. The crop itself is the paint's `crop` transform (`fill.set`).
- Text props (`fontFamily`, `fontStyle`, `fontSize`, `lineHeight`, `letterSpacing`, `textDecoration`,
  `textCase`, `textFills`, `fontVariations`…) apply to the whole layer, or to chars `start`..`end`
  when given (`text.style {id, start, end, …}` does the same). Styled ranges come back as
  `styledRuns`. Variable fonts: `fontVariations` (alias `variationSettings`) sets axis values in
  user units, e.g. `{"wght": 550, "wdth": 85}` (`null` = back to what the style name implies);
  `text.setVariations {variations, replace?}` merges instead. `text.fonts {family, style?, axes: true}`
  lists a font's axes and named instances (named instances are also valid `fontStyle` values).
  `text.missingFonts` lists the families the document names that aren't installed
  (`{families: [{family, layers, source}]}`); their text draws in a fallback font. On the desktop,
  `font.download {family}` installs a family whose `source` isn't null (Google Fonts, Noto) from
  the font store, in the background: pinned, checksum-verified files into the Craft apps' shared
  font folder. It works only while the user has allowed downloading fonts, which only the user can
  do, in the app's Missing fonts window; otherwise it is refused. Call it again for its progress
  (`{state: downloading, done, total}`); once installed, the call fits the open document's text to
  the font (`{state: installed, refitted}`). `font.downloads` lists the downloads and whether
  downloading is allowed (`{allowed, downloads: [{family, state, error?}]}`). Fonts are indexed once
  per process: after installing one, `text.rescanFonts` picks it up (`{added: [families]}`), and
  `text.loadFont {path | data (base64)}` adds a font file without installing it (`{families}`).
  Text boxes keep the size they were measured at: both refit the open document's auto-sizing text
  in the new families (`refitted: [ids]`), and `text.refit {ids?}` does it for any text, e.g. a
  file saved while its fonts were missing.
  OpenType features: `features {"liga": false, "tnum": true}`.
  Decorations: `textDecorationStyle` (`solid`, `dotted`, `wavy`), `textDecorationThickness` and
  `textDecorationOffset` (`auto`, px, or `"10%"` of the font size; the offset runs from the baseline
  down to an underline's top edge), `textDecorationSkipInk` (underlines break around descenders) and
  `textDecorationColor` (a colour, or `auto` for the text's) are character props too.
- **Lists** are per paragraph: `listStyle` (`none`, `bulleted`, `numbered`) and `indentation` (the
  nesting level, 0–8) apply to the paragraphs chars `start`..`end` touch (all of them without a
  range); `get_node` reports one `listStyle` for the layer, or `lineTypes` and `lineIndentations`
  per paragraph, plus `listStartOffsets` when a numbered list starts past 1 (from a `.fig` file;
  1 shows "2."). `text.bulletList`, `text.numberList` and `text.listStyle` act on the selected
  paragraphs while text is edited. Layer props: `listSpacing` (px between list items, in place of
  `paragraphSpacing`), `hangingList` (markers outside the box) and `hangingPunctuation`. While
  editing, `key` Tab / Shift+Tab nest an item or move it out, Enter on an empty item and Backspace
  at an item's start step out of the list, and typing `- `, `* ` or `1. ` at a paragraph's start
  starts one.
- **Text styles** link chars, not just layers: `style.apply {style, ids?, start?, end?}` with a text
  style links chars `start`..`end` (while text is edited, the selected chars; else whole layers), and
  `style.detach {which: "text", start?, end?}` unlinks them (values stay). A layer's `styles.text` is
  the style all its chars share (none when they differ); `get_node` lists the linked chars as
  `textStyleRanges` (`[start, end, style id]`).
  Changing what a text style sets (font, size, line height, letter spacing, decoration, case, axis
  values) on linked chars unlinks them; `style.update` moves every linked char. `style.create
  {type: "text", name}` takes the selected chars while editing; a `value` may use the `node.set`
  spellings (`{"fontSize": 32, "lineHeight": "120%", "letterSpacing": "2%"}`).
- **Styles stay true to their layers**: editing a styled fill, stroke, effect or layout grid list
  (`node.set`, `fill.set`, `fill.add`, a bound variable…) unlinks that style (`styles.fill` etc. go
  `null`), so a later `style.update` doesn't overwrite the edit; `style.apply` replaces variables
  bound to what the style sets; a text style goes on text layers only. In one call, a style link
  (`fillStyle`, `textStyle`…) wins over values it sets; set a deviation in a later call, which
  unlinks it. Names are trimmed per `/` folder part. A `fill` or `stroke` can also bind a variable:
  `node.create {type: "rectangle", props: {fill: {"$var": "Theme/surface"}}}` creates and binds in
  one undo step, and `node.set` takes the same wrapper. `$var` takes an exact variable id first,
  else an unambiguous exact name or `Collection/name` (names can contain `/`); unknown or ambiguous
  names are errors. The wrapper is applied after literal values and style links, and clears that
  paint's style link even when the colour is unchanged. Only `fill` and `stroke` take variables in
  props; use `variable.bind` for other fields. `style.move {styles, before?, folder?}`
  reorders (and with `folder`, renames into it), `style.duplicate {style}` adds "<name> copy" right
  below, `style.renameFolder {folder, name}`.
- **Variables**: `variable.bind` refuses a variable of a type the field doesn't take (a colour on
  `width`); scopes only filter pickers. `variable.delete` / `variable.deleteCollection` unbind the
  layers using them (they keep their last value) and give aliases to them their values. An
  explicit edit that changes a bound field's value (`node.set`, `transform.resize`,
  `object.opacityKeys`, `text.style`, `text.set`, the text size, weight and
  tracking shortcuts, a canvas drag) detaches that field's variable; fields the edit
  left as they were stay bound.
  `variable.addMode {from}` duplicates a mode, `variable.moveMode {collection, mode, to}` reorders
  (index 0 is the default mode, which layers on Auto follow). `variable.update {codeSyntax: {WEB?,
  iOS?, ANDROID?}}` sets code names (`""` clears); generated CSS uses the Web one.
- **Live previews**: `text.preview {command, params}` shows what `style.apply`,
  `library.applyStyle`, `text.setVariations`, `text.style`, `node.set`, `fill.set`, `stroke.set`,
  `selection.setPaint` or `style.update` would do on the canvas without an undo step (in place of an
  earlier preview); `text.endPreview` puts the document back. Any other command ends the preview
  first, so running the command for real commits it as one undo step.
- Every mutation is one undo step (`batch` makes several one step). A failing command leaves the
  document as it was.
- Programmatic calls never open dialogs.
- Verify your work: `inspect_document` / `get_node` for numbers, `screenshot` for pixels.

To find layers without changing the selection, use `run_command` with `node.find`, for example
`{"command": "node.find", "params": {"root": 12, "type": "TEXT", "text": "hello", "limit": 20}}`.
It searches strict descendants of `root`, or the current page without one, in stored child order
(depth-first, parent before children), including hidden and locked layers and instance children.
`name`, `text` (text layers only) and `component` (instances by their main component's name) are
case-insensitive literal substrings; `type` is a canonical layer type, case-insensitive. All
filters must match. The result is `{"ids": [...], "total": N}`: at most `limit` ids (default 100,
0 to 10000), with every match counted in `total`; `limit: 0` counts only. A root on another page
works without switching pages. Unknown roots or types and invalid limits are errors. In a batch,
a later step can use `{"$step": 0, "field": "ids/0"}` to target the first match.

## Tools

| Tool | What it does |
|---|---|
| `list_commands {filter?}` | Every command: id, label, menu, shortcut, undoable, params doc. |
| `run_command {command, params}` | Run any command (e.g. `align.left`, `layout.add`, `component.create`, `instance.place` (drop an instance at a page point into the frame or auto layout slot under it), `component.exposeInstance` / `instance.exposed` (nested instance properties on instances), `component.makeSlot` / `instance.slotInsert` / `instance.resetSlot` (slots, below), `inspect.code` (CSS, HTML + CSS for a whole frame, React, Leptos, Vue, SwiftUI, Jetpack Compose, ArkUI or Flutter for a layer; components take their properties as parameters and instances are calls, see Dev Mode below), `inspect.properties` (layout, auto layout, typography, fills, strokes and effects with their style and variable names; `colorFormat`: hex, rgb, css, hsl or hsb), `inspect.assets` (layers under the selection with export settings, icons and images, each with its `kind`), `inspect.code` takes `unit: rem` and `rootFontSize` for CSS, HTML, React, Leptos and Vue, `style.create`, `variable.bind`, `variable.update` (rename, describe, scope), `variable.set` with `{alias}` or a colour calculation (`{alpha}`, `{mix}`), `variable.renameMode` / `variable.removeMode`, `variable.deleteCollection`, `proto.interaction.add`, `codeConnect.set` (components linked to real code, see Dev Mode below)). |
| `batch {commands: [{command, params}], label?}` | Several commands as one undo step (`command.batch`), which keeps one copy of the document however many steps there are (an `edit.undo` step undoes the batch's edits so far together); all-or-nothing for open documents and their document state. Failed inner commands are removed from the journal; preferences, clipboard, tool state and UI requests are not rolled back. A step can use what an earlier step returned: name it with `"as": "card"` and `{"$ref": "card"}` anywhere in a later step's params becomes the `id` in its result (`{"$step": n}` does the same by zero-based position); `"field"` takes another key or a slash path such as `ids/0` (see Step references below). |
| `create_node {type, x, y, width, height, parent, index, name, text, path, props}` | Create a frame, rectangle, ellipse, polygon, star, line, arrow, text, vector, section, component or slice; returns `{id, node}`. |
| `set_properties {ids, props}` | `node.set` on layers (geometry, fills, strokes, effects, corner radius, auto layout, sizing, constraints, masks (`isMask`, `maskType: alpha\|vector\|luminance`), handoff status (`devStatus: none\|readyForDev\|completed`; marking again clears "Changed"), a section's `sectionContentsHidden`, text…); returns the updated layers. |
| `inspect_document {depth, allPages, fields?, outline?}` | The document as JSON (its `id`, layer tree, styles, selection, history); `fields` selects layer properties, `outline: true` gives a compact tree (below). The tools that act on a document also take `document` (see Naming the document below). |
| `get_node {id, depth, fields?, outline?}` | One layer as JSON; takes the same `fields` and `outline` options; `pluginData` (`{key: value}`, values as strings) when the layer or page has layer data written with `set_properties {props: {pluginData}}`. |
| `select {ids}` | Replace the selection (`[]` deselects). |
| `export {ids, format, scale \| size, path?, bleed?, cropMarks?, bleedMarks?, cmyk?, contentsOnly?, includeBoundingBox?, svgOutlineText?, svgIdAttribute?, svgSimplifyStroke?, vdCompat?}` | png, jpg, svg, webp, pdf, xml (an Android Vector Drawable: `svgIdAttribute` writes `android:name`, `vdCompat` is `bestEffort` (default: approximated with a warning), `exact` (left out with a warning) or `ignore` (no warnings: overlapping layer opacity dropped, even-odd fills kept as `fillType="evenOdd"`, diamond gradients left out), and the result lists `warnings` for what was changed or left out), or `fig`: the whole file as `.fig`, experimental (see Exporting `.fig` files below); `size` is `2x`, `512w` or `512h`. The image covers what the layers paint (shadows towards their offset, strokes, unclipped children, a mask group's mask; hidden layers take no room; a slice is its area with everything under it, or only the frame or group it sits in while `contentsOnly` is on); `contentsOnly: false` also draws every other layer showing in that area, `includeBoundingBox` exports the layers' own boxes; SVG: `svgOutlineText: false` keeps `<text>`, `svgIdAttribute` writes unique ids from layer names, `svgSimplifyStroke: false` writes every stroke as an outline. The same options go in export settings (`export.add` / `export.update`). PDF: frames one page each (sized in points), anything else together on one page; print options: `bleed` (points or a length like `"3mm"`, `"0.125in"`), crop and bleed marks, CMYK colour (see Exports below), also taken by `file.export`, `file.exportLayers`, `file.exportFramesPdf` and stored per export setting by `export.add` / `export.update`; `run_command` `file.exportFramesPdf {page?, path?}` writes every top-level frame of a page. Without `path`, images come back as MCP image content, SVG as text, XML as text followed by `{format, bytes, warnings}`. Layers' own export settings: `run_command` with `export.add` / `export.update {index, size?, suffix?, format?}` / `export.remove` and `file.exportLayers {ids?, dir?, assets?}` (every layer once per setting, named `<layer><suffix>.<format>`, a slash in a layer name making a folder; `assets: true` exports the Inspect tab's assets instead: icons as SVG, images as PNG). |
| `screenshot {ids?, scale?, maxSize?, window?}` | PNG of the page (with background) or of layers, scaled to fit `maxSize` (default 1600). `window: true` captures the app window in front when attached, a window id (see Windows below) that window. |
| `open_file {path, variables?, reload?}` / `save_file {path?}` | Open `.figcraft`, `.json` (FigCraft JSON or REST API file JSON with its variables, see below), `.fig` or FigJam `.jam` (best effort, see below), `.sketch` (Sketch 48 and newer, best effort), `.svg` (see Importing SVG below) or an image; save as `.figcraft`. `open_file` returns `opened` (for a file opened anew, a `summary` of `name`, `pages`, `layers` and `images`, and `warnings` or `repairs`; a file that was already open gives its `index`, with `stale: true` when the file changed on disk since that tab read or saved it; `reload: true` reads it again into the tab, refused while the tab has unsaved changes) and the document's `document` outline. |
| `undo` / `redo` | Step through history. |
| `pointer {kind, x, y, to?, steps?, tool?, shift, alt, cmd, pressure?, pressureTo?}` | Mouse gestures in page coordinates with the current tool: `click`, `double`, `drag`, `down`, `move`, `up`, `hover`. A dragged layer drops into the frame or auto layout slot under the pointer, except one inside a group, a boolean group or an instance (outside its slots), which only moves. `pressure` (0–1) is pen pressure; a drag goes from `pressure` to `pressureTo` (see Brushes and pen pressure). |
| `key {key, shift, alt, cmd}` | A shortcut or key as if typed on the canvas (every default shortcut, tool keys, Escape, Delete, arrows, number keys for opacity, arrows that pan the view when nothing is selected; while editing text, the text shortcuts and then text editing keys). Keys are names like `A`, `0`, `Enter`, `ArrowLeft`, `=`, `/`, or egui's (`Plus`, `Questionmark`); `cmd` is Cmd on macOS and Ctrl elsewhere. |
| `type_text {text, id?}` | Type into the text being edited, or replace the contents of text layer `id`. |
| `inspect_ui {}` | Tool, view and status (and panels, timings and the app's `windows` when attached). |
| `wait_for_changes {after?, timeoutMs?, topics?, limit?}` | Wait for new comment events in the attached app; returns pending events at once, else waits up to `timeoutMs` (default 20 s, max 25 s). Pass the returned `cursor` back to keep watching. Headless returns immediately. |

### Compact layer trees

`inspect_document` and `get_node` accept `fields`, an array of exact, case-sensitive top-level node
keys, applied to every layer and page. For example, `fields: ["name", "type", "width", "height"]`
leaves out paints and text properties; `fields: []` keeps only ids and structure. `id`, `children`,
depth-boundary `childCount` and the transport's `truncated` marker are always kept (`truncated` is
not a selectable field). An unknown key is an error that lists the valid keys; a `fields` that is
not an array of strings, or an `outline` that is not a boolean, is an error too. Document metadata
stays unchanged.

`outline: true` selects `id`, `name`, `type`, `characters`, `mainComponentName` and `visible`, plus
structure. `characters` is cut after 120 characters (Unicode scalars), with ASCII `...` appended
only when longer. Selecting `visible`, explicitly or through outline, reports a boolean even for
visible layers and pages;
without these options the output stays sparse. `outline: true` cannot be combined with `fields`;
`outline: false` can. Both options keep the existing depth and response-size limits.

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"inspect_document","arguments":{"depth":6,"outline":true}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_node","arguments":{"id":3,"depth":2,"fields":["name","width","height"]}}}
```

### Starter templates

An agent can start from a template instead of an empty page. `run_command {command: "template.list"}`
returns the starter templates, each `{id, name, description, kind, size}` (`kind`: `app`, `website`,
`kit` or `media`): `mobile-app`, `landing-page`, `dashboard`, `design-system`, `wireframe-kit`,
`social-posts`, `slides` and `icon-set`. `run_command {command: "file.newFromTemplate", params:
{template, name?}}` builds one as a new, unsaved document, makes it the active one and returns
`{index, template}`; open documents are left alone. The templates are built in code from the same
commands an agent uses (frames with auto layout, components and instances, colour and text styles,
variables with Light and Dark modes, prototype links), so everything in them is editable; their
content is synthetic. Building one takes from about 10 ms to about 190 ms. Without `template` the
command returns `{templates}` (in the app it also shows the home screen's Templates tab).
`help.gettingStarted {show?}` shows (or with `show: false` closes) the home screen's Getting started
tips; headless, it only sets the preference.

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"file.newFromTemplate","params":{"template":"dashboard"}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"export","arguments":{"format":"png","path":"dashboard.png"}}}
```

### Filling copies with data

`data.fill {id, rows, gap?, formats?, dir?, format?, scale?}` repeats a frame once per row (a carousel, a set
of posts, a card per item) in one undo step: each key of a row names layers in the copy, and a
string (or number) sets their text, `{text?, fill?, visible?}` sets more; `name` names the copy
(default "<frame> <n>"). Copies go in a row to the right, `gap` apart (default 100). With `dir`, each
copy is exported there as `<name>.<format>` (png by default) after the edits land. Returns `{ids,
missing (keys no layer is named), files?, errors?}`. `errors` lists the exports that failed as
`[{id, path, error}]`; the copies stay, and the files that were written are in `files`.

Copy that changes length per row can size itself: `{"Headline": {"text": "…", "fit": {"maxLines": 2}}}` runs
`text.fit` on those layers after the text is set.

`formats: [{name?, width, height}]` makes every row once per size: the copy is resized so its auto layout
reflows, named `<name>-<format name>` (default `<width>x<height>`), and placed under the row's previous
format. Text `fit` runs after the resize. Use case: one call turns a 1080×1350 post into feed, story
(1080×1920) and square (1080×1080) files for each slide of a carousel; the frame must be auto layout (fixed-
position children keep their place).

`text.fit {ids?, maxLines?, minSize?, orphans?}` gives a text layer with a fixed width (auto height or fixed size)
the largest font size, down to `minSize` (default 60% of the current), at which it stays within `maxLines` lines
and its box's height. Every style run scales by the same factor. A one-word last line (an orphan) is avoided by
shrinking up to 8% more, else by keeping the last two words together with a no-break space. A one-word paragraph after a hard line break is not an orphan. Text never
grows, text that already fits keeps its exact size, and sizes round to a tenth without going below `minSize`.
Returns `{fitted: [{id, fontSize, lines, fits, orphan}]}`. Use case: a carousel slide "Lighthouse 100. Loads in under
1s." at 96px broke into three lines with "1s." alone; with `fit: {maxLines: 3}` it is two balanced lines.

```json
{"command": "data.fill", "params": {"id": 80, "dir": "out/carousel", "scale": 1, "rows": [
  {"name": "01-cover", "Headline": "Your website and AI chat assistant in 7 days", "Badge": {"visible": true}},
  {"name": "02-proof", "Headline": "Lighthouse 100, loads in under a second", "Badge": {"visible": false}}
]}}
```

### Boards

A board is a file of its own kind for thinking together: `file.newBoard {name?}` opens one (its
page is light; `inspect_document` says `kind: "board"`). Board objects are ordinary layers with a
`board` entry `get_node` shows (`{type: sticky|shape|connector|label|stamp|ink, …}`), so the usual
commands move, resize, recolour, copy, delete and undo them. On a design file the board commands
refuse; on a board `proto.*`, `dev.*`, `view.prototype` and `view.devMode` refuse. Each of these is
one undo step and returns the ids it made:

- `board.sticky {x, y, text?, color?, author?, parent?}` → `{id, text}`: a sticky note (a square
  frame whose height grows with its note; its text size shrinks as the note gets longer). `color`
  is a board colour (`yellow orange red pink violet blue green gray`) or a hex colour; `author`
  defaults to your name (`pref.userName`, or the one a live session gave you; `false` for none).
- `board.nextSticky {id?, side?: right|below|left|above, edit?}` → `{id, text}`: a sticky of the
  same colour beside the selected (or edited) one; with `edit: true` the app goes on typing in it
  (`Shift+Cmd/Ctrl+Enter` in the app does both). On a shape it does what `board.addConnected` does:
  the next shape, joined to it by a connector (`{id, text, connector}`).
- `board.addConnected {id?, side?: right|below|left|above, edit?}` → `{id, text, connector}`: the
  next sticky or shape of the same kind, size and colour on that side of the selected (or edited)
  one, 80 units away, joined to it by a connector the way the Connector tool makes one; one undo
  step. With `edit: true` the app goes on typing in it (the "+" beside a selected sticky or shape
  in the app does both).
- `board.shape {shape, x, y, width?, height?, text?, color?, parent?}` → `{id, text}`: a shape with centred,
  wrapping text: `rectangle roundedRectangle ellipse diamond triangle speechBubble chevron`.
- `board.connector {from, to, fromSide?, toSide?, style?: straight|elbow|curved, startCap?, endCap?, label?}`
  → `{id, label?}`: `from`/`to` are layer ids or `{x, y}` page points; sides are `auto` (the side
  facing the other end) or `top right bottom left`; caps are stroke caps (default an arrow at the
  end). An elbow goes around the objects it joins when the short way would cross them; a curved
  one leaves and arrives along the sides. The connector re-routes itself whenever what it joins
  moves or resizes, also when someone else in a live session moves it; a deleted end stays where it
  was as a point. `board.label {id, text}` sets its label (empty removes it).
- `board.reattach {id?, end: from|to, target, side?}` → `{id}`: moves one end of a connector
  (default the selected one; its label stands for it) onto `target`, a layer id or an `{x, y}`
  page point where the end stays free. Refused when both ends would be on one layer or the target
  is a connector. Dragging a selected connector's end handle in the app does the same.
- `board.stamp {x, y, stamp: thumbsUp|heart|plusOne|star|question|check}` → `{id}`: a round badge
  centred there, with your name on it for hover.
- `board.ink {path | points, highlighter?, color?, select? (default true)}` → `{id}`: a marker (or
  wide, see-through highlighter) stroke.
- `board.setColor {ids?, color}`: stickies and shapes take it as their fill, connectors and ink as
  their line. `board.setStyle {ids?, shape?, fontSize?, style?: straight|elbow|curved, startCap?,
  endCap?}`: a shape's outline and text size, a connector's routing and arrowheads.
- `board.templates` → `[{id, name, description, width, height}]` lists the built-in templates:
  `retrospective brainstorm weeklyPlanner flowChart kanban`. `board.template {name, x?, y?}` →
  `{name, ids}` places one (its top-left at `x`, `y`; centred in the view without them) as ordinary
  sections, stickies, shapes, connectors and text, selected, in one undo step.
- `board.table {rows?, columns?, cells?, x?, y?}` → `{id, rows, columns, cells}`: a table (default
  3 × 3, at most 50 × 20; `cells` is `[["Name", "Role"], …]` row by row and sets the size when
  `rows`/`columns` are left out). It is an auto layout frame of row frames holding cell frames, each
  with one text layer; `cells` in the answer holds those text layers' ids row by row, so
  `node.set {id, props: {characters}}` changes a cell. A cell's row grows with its text. `board.tableEdit {id?, action: addRow|addColumn|removeRow|removeColumn,
  index?}` → the same answer: `index` is where the new row or column goes, or which one goes
  (default the last); `id` may be the table or any layer in it.
- Tools for `pointer`: `tool.sticky {color?}`, `tool.shape {shape?}`, `tool.connector {style?: straight|elbow|curved}`
  (drag from one object to another), `tool.marker {color?}`, `tool.highlighter {color?}`,
  `tool.eraser` (drag over strokes), `tool.stamp {stamp?}`; Text and Section are the usual tools.

```json
{"command": "file.newBoard", "params": {"name": "Retro"}}
{"command": "board.sticky", "params": {"x": 0, "y": 0, "text": "Went well", "color": "green"}}
{"command": "board.sticky", "params": {"x": 240, "y": 0, "text": "To improve", "color": "pink"}}
{"command": "board.connector", "params": {"from": 2, "to": 4, "style": "elbow", "label": "leads to"}}
{"command": "board.stamp", "params": {"x": 200, "y": 0, "stamp": "plusOne"}}
```

`from` and `to` are the `id`s the two `board.sticky` calls returned (here `{"id": 2, "text": 3}` and
`{"id": 4, "text": 5}` in a new board without an author name; a sticky with its author's name takes
one more id). In a `batch`, step references (`{"from": {"$step": 1}, "to": {"$step": 2}}`, the `id`s steps 1 and 2 returned) avoid counting.

Running a session on a board: a countdown and votes. Both are part of the board, so everyone in a
live session sees the same, and neither is an undo step (undo leaves them as they are). On a
design file they refuse.

- `board.timer {action: start|pause|resume|add|stop, minutes?}` → `{state, left, text, ends}`: one
  countdown per board. `start` takes `minutes` (default 5) and `add` adds them (default 1), from
  one second up to 1440 minutes in all; `pause` keeps the time left, `resume` counts it down again.
  The timer keeps its end time (milliseconds since 1970) or, paused, the time left, and each
  client counts down on its own clock. `board.timerState {}` reads it: `state` is `off`,
  `running`, `paused` or `done` (it ran out: 0:00 until someone stops it), `left` milliseconds,
  `text` as `m:ss` (or `h:mm:ss`).
- `board.vote {action: start|cast|retract|end|clear, …}` → the vote as `board.voteResults` gives
  it. One vote at a time: `start {votes? (per person, 1–99, default 3), on?: all|stickies|selection
  (default all), ids? (with selection; default the selected layers)}` (a vote that ended may be
  replaced; one still running is refused), `cast {id}` puts one of your votes on an object (several
  on the same object are fine; once yours are used up it refuses), `retract {id}` takes one back,
  `end` shows everyone's votes, `clear` removes the vote and its results. `voter` (any action)
  votes as someone else, for scripts and tests; by default you are your live-session client, or
  your name (`pref.userName`) when the file isn't shared. In the app a click on an object votes
  while a vote runs, and a click on your badge (or an Alt-click) takes the vote back.
- `board.voteResults {voter?}` → `{state: off|running|ended, votesPerPerson, on, by, left, mine:
  [{id, votes}], voters, results?}`: while it runs you see your own votes only (`mine`, `left`) and
  how many have voted; once it ends `results` lists `[{id, name, votes, rank}]`, most votes first
  (ties share a rank), with `name` the sticky's text or the layer's name. Votes on deleted objects
  don't count.

```json
{"command": "board.timer", "params": {"action": "start", "minutes": 10}}
{"command": "board.vote", "params": {"action": "start", "votes": 2, "on": "stickies"}}
{"command": "board.vote", "params": {"action": "cast", "id": 2}}
{"command": "board.vote", "params": {"action": "end"}}
{"command": "board.voteResults", "params": {}}
```

### Sections, handoff and arrow keys

`run_command` with `object.wrapInSection {ids?, name?}` (Wrap in new section: layers on the page or
in a section, with some room around them), `section.toggleContents {ids?, hidden?}` (a section
hiding its contents shows only its name and outline; its layers aren't drawn or hit, and the layers
panel dims them), `object.resizeToFit`. A frame or section marked `devStatus: readyForDev` turns
"Changed" (`devChanged: true` in `get_node`) when a layer in it is edited, added or removed;
`dev.resolveChange {ids?, reason?}` returns it to Ready for dev and keeps the reason (`devNote`).
Frames in sections are top-level frames (flows can start there; `proto.flow.add`); a "Navigate to"
whose destination is a section shows the frame of it shown last in the presentation, else its
first.

`transform.nudge {dx, dy}` (arrow keys) moves auto layout children one place instead of nudging:
along the flow, to the previous/next row of a wrapping layout, to the neighbouring grid cell, and,
at the end of a flow or across a row's axis, into the neighbouring nested auto layout frame when the
frame around flows that way (a list of rows: Down moves an item into the row below).

### Keyboard shortcuts

The default shortcuts live in one table (`figcraft_engine::keymap`), grouped like the public shortcut
reference (Essential, Tools, View, Zoom, Text, Shape, Selection, Edit, Transform, Arrange,
Components, Prototype) with the macOS keys and, where they differ, the Windows/Linux keys (for
example Use as mask is ⌃⌘M on macOS and Ctrl+Alt+M elsewhere; Bring to front ⌥⌘] and
Ctrl+Shift+]). `list_commands` reports each command's first key for the platform the server runs
on; `key` resolves keys the way the canvas does. Commands behind keys:

- `object.opacityKeys {digits, ids?}`: the opacity number keys. One digit is tens of percent (`"0"`
  is 100%); two digits are the exact percentage (`"45"` is 45%, `"05"` is 5%), what typing two
  digits in quick succession does on the canvas.
- `view.zoomAt {x, y, out?, width?, height?}`: the zoom tool (hold Z on the canvas; Alt zooms out).
  A point zooms one step in (or out) keeping that page point in place; an area fills the view.
- `view.designPrototype` (`Shift+E`), `view.devMode` (`Shift+D`) and `view.leftSidebar`
  (`Shift+Cmd+\`) switch the right panel's tab or hide the left sidebar in the app (no-ops headless).
- `doc.nextTab` / `doc.previousTab` (`Ctrl+Tab` / `Ctrl+Shift+Tab`): the next or previous open file.
- Board tools (boards only): `tool.sticky` (`Q`), `tool.shape` (`B`), `tool.connector` (`X`),
  `tool.marker` (`M`), `tool.highlighter` (`U`), `tool.eraser` (`E`), `tool.stamp` (`W`), and
  `board.nextSticky` (`Shift+Cmd/Ctrl+Enter`, also while typing in a sticky). Keys no design-file
  shortcut uses; on a design file they refuse.
- `view.pan {dx, dy, big?}`: the arrow keys with nothing selected (`key ArrowRight` does the same)
  pan the view by 20 screen pixels a step, 200 with `big` (Shift).
- `view.multiplayerCursors {on?}` (`Alt+Cmd+\`): show or hide others' cursors in a live session.
- `view.outlinesHiddenLayers {on?}` and `view.outlinesObjectBounds {on?}`: what outline mode
  (`view.outlines`, `view.outlinesSelection`) draws besides the shapes: hidden layers, and the box
  of every layer whose outline isn't its box (groups, text, ellipses, vectors). What they draw can
  be clicked (`pointer`) in outline mode. View settings of the file: no undo step.
- `view.panelWidths {left?, right?}`: the app's side panel widths in pixels (kept within their
  limits; without either, both go back to the default). A no-op headless.
- `component.hideWhenPublishing {ids?, on?}`: keep components or sets (those `ids`, or the layers
  given, are or are in; default the selection) out of the library the file publishes, or let them
  back in (`on: false`; without `on` it toggles). The sign of it is the name: hiding puts a `.`
  before it, showing takes the leading `.` and `_` away. Returns `{ids, hidden}`; one undo step.
- New files and pages: `file.new` gives its page the colour `#F5F5F5`. Only the app with its dark
  theme showing makes it `#1E1E1E`; headless sessions (the CLI, this server) have no theme.
  `page.add` gives the new page the current page's colour, wherever it runs. Text made with `text.create` or `node.create` is black whatever the page here; in the
  app (the control channel, `--attach`) it is white on a dark page unless `props` name a `fill`,
  as the Text tool (`pointer` with `tool.text`) makes it everywhere.
- `transform.rotationOrigin {on?}` (`Alt+R`): show or hide the selection's rotation origin handle
  (toggles without `on`); `transform.setRotationOrigin {x, y}` moves the origin (page coordinates),
  `{reset: true}` puts it back at the centre. Both return `{editing, moved, x, y}`. The corner
  rotation handles, a `rotation` set through `node.set` on the selection, Rotate 90°/180° and
  `transform.rotate` without `cx`/`cy` turn about it. It belongs to the selection, follows the layers
  as they move and turn, and is dropped when the selection changes; it isn't saved.
- `tool.place {tool?, x?, y?}`: what Enter does with a shape or frame tool chosen (`key Enter` does
  the same): makes what a click with the tool makes (100×100; lines 100 long; sections 400×300)
  centred on `x`, `y` (default the middle of the view), selects it and returns `{ids}`. One undo step.
- `text.listIndent` / `text.listOutdent` (`Cmd+]` / `Cmd+[` while editing text): the selected list
  items a level deeper or out, like Tab and Shift+Tab.
- `view.nextRegion` / `view.previousRegion` (`F6` / `Shift+F6`, live while editing text too): move
  the keyboard between the app's regions (toolbar, layers, canvas, properties). In the app, Tab and
  Shift+Tab then move between the region's controls, Enter or Space activate one, the arrows move
  along the toolbar and through the layer rows, and Escape returns to the canvas (no-ops headless).
- `object.rename {ids?}` without a `name` (`Cmd+R`, or `F2`): rename the layer inline in the app's
  layers panel, also from a layer row that has the keyboard; when the rename ends the row gets the
  keyboard back (Escape cancels it there). With several layers it opens the Rename layers dialog.
  Headless it changes nothing and leaves no undo step.
- `object.batchRename {ids?, find?, regex?, replace?, name?, start?}`: rename several layers in one
  step. `find` (plain text, or a regular expression with `regex: true`) has each match replaced by
  `replace`; `name` renames the whole name. Both take codes: `$&` the current name (the match when
  matching), `` $` `` and `$'` the text before and after the match, `$1`…`$9` groups, `$n` a number
  counting up from `start` (default 1), `$N` one counting down to it, `$nnn` padded to three digits,
  `$$` a dollar sign. Layers are numbered in stacking order, bottom first. Returns `{names}`.
- Pages: `page.add {name?}` (a name starting with `-` adds a divider: an empty page shown as a line
  in the pages list, never opened and holding no layers), `page.list` (each page's `divider`),
  `page.rename`, `page.move {id, index}`, `page.duplicate` ("Copy of …", after its source, with its colour,
  flows (starting at the copied frames), prototype settings and guides),
  `page.delete` (the last page that isn't a divider stays).
- `component.list`: each component's `folder` (the Assets panel's folders: its page, the frames
  holding it, then its name's slash parts) and `label` (the name on its tile).
- `view.contextMenu` (`Shift+F10`): the selection's context menu, at its layer row when the layers
  panel has the keyboard (after F6 or a click on a row), else on the canvas below the selection; the
  menu takes the keyboard and gives it back when it closes (no-op headless).
- `select.area {x, y, width, height, add?, deep?}`: select what a marquee from `x, y` over that box
  (page units) would: top-level layers, a frame only when the box covers it, the children of the
  top-level frame the box starts in; `add` toggles like a Shift marquee, `deep` takes nested layers
  like Cmd/Ctrl. Returns `{selection}`. Without a box (`F8` in the app) it draws one for the
  keyboard around the selection (or in the middle of the view): the arrows move it, Shift+arrows
  move its far corner (10 screen pixels a press), Enter selects (Shift+Enter adds, Cmd/Ctrl+Enter
  nested layers), Escape or a click drops it.
- `pref.reduceMotion {on?}`: reduced motion (the interface's own animations off; the system's
  accessibility setting turns it on too). Prototype transitions still play.
- `pref.uiScale {scale?}`: how large the whole interface is drawn, relative to the system's display
  scale: 1 (the default) is the system's own size on every screen, 1.25 means 125 %, not 125; from
  0.75 to 3 (outside the range it stops at the nearest end; `"auto"` is 1). Omit `scale` to read it.
  The canvas zoom is separate. A window too small for a size above 100 % draws a smaller one until
  it is larger; the choice is kept. In the browser the interface always follows the browser's zoom.
- `pref.autoSave {on?}`: files that have a path save themselves a few seconds after they change
  (off by default; the desktop app's timer does it, so scripts still call `file.save`).
- `pref.startOnHome {on?}`: the desktop app starts on the Home screen instead of in a new file
  (off by default; files named on the command line still open, and `--control` starts in a new
  file anyway, so control callers have a document).
- `pref.language {language: system|en|bs|cs|ru|es|es-419|fr|de|pt-BR|ja|ka|ko|sv|vi|zh-Hans|…}`: the app's interface language, applied live ("system":
  the OS or browser language; a language without a catalog shows English). Command ids, labels and
  every result stay English.

### Editing vectors

`vector.create {path | points}` makes the layer's box from the geometry; `width`/`height` scale
the geometry to the size asked for (with both, each side independently; with one, the other keeps
its proportion). A path or point set that draws nothing is refused instead of making an invisible
0×0 layer, as is a size on an axis the geometry has none of (except 0, such as a horizontal line's
height) and a size of 0 or less on an axis it has, which would flatten it.

`node.set {path}` (or `set_properties`) replaces a vector's geometry with new SVG path data and
keeps the layer: its id, name, place in the layer list, paints, effects, styles, variable bindings
and constraints. The path is read as `vector.create` reads it, and `width`/`height` in the same call
scale it the same way. The path's own box goes at the layer's current x/y (rotation and flips
stay); with `x`/`y` in the same call it goes at `x` + its left edge and `y` + its top, as
`vector.create` places it. Points' own settings (per-point corner radius, caps, joins) go with the
old geometry. On several layers, layers that aren't vectors are skipped with a warning; on a
single one, or when none is a vector, the call is refused (convert shapes with
`vector.edit.begin` or Flatten first). A layer inside an instance is refused too: its geometry
comes from its main component, so edit that or detach the instance. Undo brings the old path back.

Vector edit mode is driven with `run_command` (or `pointer`/`key` for gestures):
`vector.edit.begin {id? | ids?}` (default the selection; converts shapes to vectors; several layers
are edited at once, the first active; `key` Enter with shapes selected does the same),
`vector.edit.state` (page-space overlay of the active layer, its local network, each point's handle
mirroring, the width points, `layers` (every edited layer, the active first), `others` (the other
layers' selected points and segments) and `handles` (the selected Bezier handles as
`[segment, start]`)), `vector.edit.tool {tool: move|pen|bend|paint|lasso|cut|eraser|width}`,
`vector.edit.select {vertices?, segments?, handles?: [[segment, start], ...], id? (an edited layer,
made active), add?}`, `vector.edit.selectAll` (every edited layer's points), `vector.edit.moveVertices`,
`vector.edit.nudge`, `vector.edit.position {x?, y?}`, `vector.edit.setHandle`, `vector.edit.mirroring
{mode: none|angle|angleAndLength}`, `vector.edit.smooth {smooth?}`, `vector.edit.bend`,
`vector.edit.addVertex`, `vector.edit.connect {a, b}`, `vector.edit.pen {x, y, dx?, dy?}` (continues
from the pen's open end; clicking a point connects or branches), `vector.edit.penEnd`,
`vector.edit.paint {x, y} | {points}` (paint bucket: fills or unfills the closed area under a page
point; with `points`, the areas under the rest go the way the first went, like a drag),
`vector.edit.cut {segment, t} | {vertex} | {from: [x, y], to: [x, y]}` (the last cuts every path the
page line crosses; parts that come apart move to layers of their own above, listed in `layers`),
`vector.edit.erase {points, radius?}` (the eraser),
`vector.edit.lasso {points}`, `vector.edit.delete`,
`vector.deleteHeal`, `vector.join` / `vector.smoothJoin` (join the selected points while editing),
`vector.edit.cornerRadius` (rounds the selected sharp points, next to straight or curved segments: a
curve is cut where the arc meets it and keeps the rest of its shape; drawn and exported;
`node.set {cornerRadius}` on a vector rounds every point), `vector.edit.strokeCap {cap, vertices?}`
and `vector.edit.strokeJoin {join, vertices?}` (the stroke's cap where it ends at these points and
its join where it turns there, `null` for the layer's; drawn, exported and outlined),
`vector.edit.moveHandles {dx, dy, handles?}` (moves the selected handles by a page offset),
`vector.edit.end`. Point indices refer to `network.vertices` in `vector.edit.state`; local
coordinates are the layer's own.

Several layers at once: with more than one layer in edit mode, a press on another layer's point,
handle or segment makes it the active layer; Shift-click and marquee or lasso select points in any of
them; dragging a selected point, the arrows, Delete, `vector.edit.nudge`, `vector.edit.moveVertices`,
`vector.edit.delete`, `vector.deleteHeal`, `vector.edit.mirroring`, `vector.edit.smooth`,
`vector.edit.cornerRadius`, `vector.edit.strokeCap` and `vector.edit.strokeJoin` reach every layer's
selected points in one undo step (a `vertices` list names points of the active layer only). The pen,
bend, paint bucket, cut, eraser, join, position and the transform box work in the active layer; the
box hides while points are selected in more than one layer. Shift-click Bezier handles to select
several; dragging one drags them all.

The Pen tool outside edit mode: clicking the path's first point closes it; clicking another of its
own points (not the last) connects there and ends the path; with an open vector selected, a click on
one of its end points continues that path in edit mode.
`vector.deleteHeal` fits the healed curve through where the deleted point was, keeping the
neighbours' tangents. With `pointer`, the Paint bucket and Cut tools also drag (across areas to
paint, across paths to divide), one undo step each; `shift` on a selected point at a corner of the
points' box takes it out of the selection on a click and scales in proportion on a drag.

The selected points' transform box: `vector.edit.bounds` (`{x, y, width, height, corners,
points}`: the box in the vector's axes, `x`/`y` in the parent's coordinates like the Position
fields), `vector.edit.resize {width?, height?, center?}` (keeps the top-left unless `center`),
`vector.edit.rotate {angle, cx?, cy?}` (degrees counter-clockwise about the box centre or a page
pivot), `vector.edit.transform {matrix: [a, b, c, d, e, f], vertices?}` (any page-space affine).
While points are selected, `object.flipH` / `object.flipV` and the 90°/180° rotations act on them.
On the canvas (`pointer`), dragging points, handles and pen clicks snap to other points and segment
midpoints (this vector's and other vectors'), the usual layer targets and the pixel grid; `cmd`
held while dragging turns the object snapping off.

Variable width strokes: `stroke.widthProfile {ids?, preset?: uniform|wedge|taper|eye|rounded|pinch,
points?: [[t, scale], ...]}` (also `node.set {widthProfile}`; `t` runs 0–1 along each subpath by
length, `scale` multiplies the stroke weight, widths in between are interpolated smoothly; no
preset or points = uniform). While editing, the Variable width tool (`width`, ⇧W) drags width
points: `vector.edit.widthPoint {t, width? (px) | scale?}` adds one, `{index, t?, width? | scale?}`
moves or resizes it, `{action: "remove", index}` deletes it. Rendering, PNG/SVG/PDF export and
`vector.outlineStroke` follow the profile.

One-sided widths: a width point can be wider on one side of the path than the other (left and
right looking along the path, y down). Points are `[t, left, right]` (or `{t, left, right}`) where
each side's scale is what `scale` would be with both sides that wide, so `[t, s]` equals
`[t, s, s]`; `get_node` reports one-sided points that way. `vector.edit.widthPoint {index, side:
left|right, width? | scale?}` changes one side (`width`: the distance from the centre line to that
edge; with `t` it adds a point keeping the other side as it is); a width without `side` makes the
point symmetric again. With `pointer`, `alt: true` while dragging a handle (or the path) changes
that side only; `vector.edit.state` lists `widthPoints` with both handle positions (`sides`:
right, left) and `oneSided`.

### Prototypes

Interactions are `proto.interaction.add` / `set` / `remove` / `clear` (a navigate action can keep
the scroll position with `preserveScroll: true`; `{type: "scrollTo", dest, offset?: [x, y]}` stops
the layer that far from the top-left of the innermost frame scrolling it, e.g. below a fixed
header; `delayMs` makes a mouse enter, leave, down or up trigger wait before it acts;
`{type: "openOverlay", dest, position: "manual", offset?: [x, y]}` puts the overlay that far from
the top-left of the layer that opens it, and right below that layer without an offset;
`anchor: "triggerInside" | "triggerOutside"` measures any other position on the layer that opens
the overlay instead of the screen, so `{position: "bottomLeft", anchor: "triggerOutside"}` opens a
dropdown right under its button, left edges lined up; Figma has no such setting, and `.fig` export
writes it as a manual offset). A key trigger takes a key or a combination (`{type: "keyPress", key: "Shift+K"}`; Ctrl,
Alt, Shift and Cmd, all held and no others) or a controller button (`key: "Gamepad A"`; `Gamepad`
followed by `A`, `B`, `X`, `Y`, `LB`, `RB`, `LT`, `RT`, `View`, `Menu`, `LS`, `RS`, `Up`, `Down`,
`Left`, `Right` or `Home`: the standard layout, face buttons by position), which the player reads
from controllers on Windows and in the browser and which the control channel presses with
`ui.input {events: [{gamepad: "A"}]}`. Widgets react to clicks in the player, on its own copy of
the file (see [plugins.md](plugins.md)). In the player, an "on drag" trigger with a move, push
or slide animation starts only for a drag that way, and the animation follows the pointer until it
lets go; spring easings play until they settle (their duration follows from the spring, not
`durationMs`); an overlay's after-delay timer starts when it opens; instances of a component play
the interactions on the variant they show unless they have their own for that trigger; fixed
layers draw above the content scrolling under them. Variable actions: `{type: "setVariable",
variable, value}` sets a value, or with `expr` an expression computed when it runs;
`{type: "setVariableMode", collection, mode}` switches a collection's mode for the prototype (ids or
names; modes chosen on layers inside the screen still win); `{type: "conditional", blocks:
[{condition?, actions}]}` runs the first block whose condition holds (a block without one is the
"else"). Expressions are `{value: {type: number|string|boolean|color|alias, value}}` (`alias` reads
a variable) or `{op: {op, args}}` with `add` (text joins when a string is involved), `subtract`,
`multiply`, `divide`, `equals`, `notEqual`, `less`, `lessOrEqual`, `greater`, `greaterOrEqual`,
`and`, `or`, `not` and `negate`; `{inMode: {variable, mode}}` reads a variable's value in one mode
of its collection whichever mode is chosen, and `{mode: {collection}}` gives the name of the mode a
collection is in where the interaction runs (as text; it follows set variable mode actions).
Variables, collections and modes may be given by name; they are stored by id.
`proto.evaluate {expression | expr, id?}` computes an expression without changing anything, with
variables in the modes where layer `id`'s interaction would run (default: the default modes) →
`{value, type, text}`; it fails with what's wrong, like the interaction commands. Expressions can also
be written as text: `expression: "count + 1"`
on a set variable action, or `expression: "count >= 3 and not done"` on a conditional block, in
place of `expr` / `condition` (variables by name, `{name}` for any name; numbers, `"text"`,
`true` / `false`, `#RRGGBB`; `or` / `||`, `and` / `&&`, `not` / `!`, `==` (or `=`), `!=`, `<`,
`<=`, `>`, `>=`, `+` (joins text), `-`, `*`, `/` and brackets; `accent[Dark]` for a variable's
value in a mode, `mode(Theme) == "Dark"` for a collection's current mode, names with brackets or
spaces at their ends in quotes). Text expressions are checked: a
condition must give true or false and a set variable's expression a value the variable can hold,
otherwise the command fails with what's wrong (`Something is missing at the end`, `No variable
\`x\``, `Can't compare a number with text`). Single actions, also inside conditionals, are edited
with `proto.action.add {id?, index, path?, action}` (→ `{path}`), `proto.action.set {index, path,
action}`, `proto.action.remove {index, path}` and `proto.action.move {index, path, to}`, where
`index` is the interaction and `path` the action: `[i]` the interaction's action `i`, `[i, b, j]`
action `j` of block `b` of the conditional action `i`, and so on (for `add`, where it goes:
`[i, b, n]` appends to a block with `n` actions; default: after the last action). A conditional's
blocks: `proto.conditional.addBlock {index, path, else?, expression? | condition?}` adds an else if
(before the else; by default the first boolean variable equal to its value) or with `else: true`
the else, `proto.conditional.setCondition {index, path, block, expression | condition}` and
`proto.conditional.removeBlock {index, path, block}` (removing the if when only an else is left
removes the whole conditional). Every one is an undo step and applies to all `ids` given (layers
with the same interactions). Flows `proto.flow.add` / `rename` / `remove`,
and the device, its rotation and the background `proto.settings {device?, background?,
rotation?, color?, width?, height?, responsive?}` (`None`, `iPhone 16`, `iPhone 16 Pro Max`,
`Android Compact`, `iPad mini`, `MacBook Air`, `Desktop` or `Custom size`; `rotation` `portrait` or
`landscape`, which turns phones and tablets; `color` one of the device's colours by name, e.g.
`Black`, `White`, `Pink`, `Teal`, `Blue` for the iPhone 16 (an error lists them), `""` for the
default; `width` / `height` set the custom size's screen, 1–10000 points, and pick `Custom size`,
which is a bare screen without a body). On a device the presented frame is sized to the device's
screen: a frame bigger than the screen scrolls through its whole size whatever its `overflow`, content
past the frame's own edges scrolls only as its `overflow` allows (else it's clipped), and `fixed`
layers stay on screen (pinned to the edges their constraints name). With `responsive: true` (saved
in the file) the presented frame is instead resized to the device's screen, or without a device to
the presenter's window at 100%, and its constraints and auto layout respond as when it is resized
in the editor (an axis it hugs keeps hugging its content); the file's frames don't change. `proto.present {start?}` plays the prototype full-window and `proto.preview
{start?}` in a resizable pane over the canvas that follows edits live; `proto.closePreview` closes
it. `proto.presentOptions {scale?, deviceFrame?, hotspotHints?, inspect?}` sets how the
full-window presenter sizes the prototype (`fit` scales it down to fit, `fitWidth` fills the width,
`fill` covers the window, `actual` shows it at 100%; what overflows pans), whether it draws the
device, whether a click where nothing reacts briefly flashes the screen's hotspots (the layers with
interactions, on by default; also in the presenter's Options menu) and whether its Inspect panel
shows the size, colours and typography of the layer under the pointer (also the Inspect button in
its bar); the options are kept for later presentations (in the presenter Z cycles them and R
restarts). An Open link to an `http` or `https` address first shows a notice that the link leaves
the prototype, with Open link, Cancel and "Don't show this again"; `proto.linkNotice {show?}`
turns the notice on (the default) or off, kept with the desktop app's preferences (the web build
keeps it until the page reloads). The player's state is
in `inspect_ui` (`presenter`, `preview`: screen, scroll, `scale`, `zoom`, `pan`, the screen's
`hotspots` and whether they are flashing, the `inspected` layer) when attached, and
`ui.set {presentScroll: [x, y]}` scrolls its screen (see [control-protocol.md](control-protocol.md)).
The presenter's Options menu offers Responsive next to the size options (without a device) or
Responsive / Fixed size (on one), the same `proto.settings {responsive}`; its flow picker lists each
flow with its description.

### Local libraries

A `.figcraft` file can be published as a library and used from other files (library files here;
libraries kept on a `figcraft-server` for a team: [Team libraries](#team-libraries); the browser
build has no file system, so there the commands that read library files return an error).

- **Publish** (in the library file): `library.publish {description?, include?, exclude?}` publishes
  the file's components, component sets, styles and variable collections as a new version (by
  default everything except names starting with `.` or `_`, or what the last publish offered;
  `include`/`exclude` take component ids and style or collection ids or names). A collection comes
  with its variables (values per mode, aliases, scopes), except those hidden from publishing
  (`variable.update {variable, hiddenFromPublishing: true}`). Components, styles and variables that
  published assets use (a component's bound variables, a variable's alias targets) come along,
  unoffered. Each asset keeps the version it last changed in; publishing with no changes is refused.
  Save the file (`file.save`) so other files see the new version.
- **Enable**: `library.add {path}` / `library.remove {id}` (id, name or path) / `library.list
  {assets?}` (versions, pending updates, errors; with `assets: true` each library's components,
  styles, variable collections (with their modes) and variables, and what this file can publish). Libraries are read read-only with size limits, and read
  again when the file changes.
- **Use**: `library.place {library, component, x?, y?}` (a component, set or variant by id or name;
  page point, default the view's centre) copies the component, with the components and styles it
  uses, into the file as a read-only main component and places an instance; `library.import
  {library, component}` only copies it (then `instance.create`, `instance.swap`…);
  `library.swapAt {library, component, x, y}` copies it and swaps the instance at that page point to
  it (as `instance.swapAt`, below).
  `library.applyStyle {library, style, ids?, target?}` copies a style in (linked, read-only) and
  applies it. Instances of library components behave like any instance: overrides, properties,
  variants, swap, reset, detach. Library components keep their variable bindings and explicit modes
  (bound to copies of the library's variables).
- **Library variables**: `library.bindVariable {library, variable, field, ids?}` copies a published
  variable in (linked, read-only, with the variables it aliases and its collection) and binds it
  like `variable.bind`; `library.aliasVariable {library, alias, variable, mode?}` makes a local
  variable's value an alias of a library variable; `library.setVariableMode {library, collection,
  mode, ids?}` sets a library collection's mode on frames (the page when no ids are given; the
Design panel's Variable modes section lists enabled libraries' collections under the library's
name and sets them this way);
  `library.importVariable {library, variable}` only copies it (then `variable.bind`, `variable.set
  {value: {alias}}`). The copies render without the library file. Editing the copies (library
  components, styles, variables or collections) is refused; edit the library file and publish.
- **Extended collections**: `library.extendCollection {library, collection, name?}` → `{id}` makes
  a collection of this file (default name "<name> (extended)") with the library collection's modes
  and one inheriting variable per variable of it. `variable.set` on an inheriting variable
  overrides its value in that mode (the variable lists it in `overrides`);
  `variable.resetOverride {variable, mode?}` goes back to the inherited value (every mode when
  `mode` is left out). Variables of its own come from `variable.create`. Modes, and the names of
  inheriting variables, come from the library collection (renaming, adding or deleting them is
  refused); accepting library updates brings new values (overrides stay), renames and new
  variables. A variable the library adds to the extended collection is an update of its own
  (`library.updates` lists it without an `id`, it has no copy yet). A library collection that is
  itself an extended collection can be extended again: the chain follows as each file accepts
  its library's updates. The Variables editor offers Extend collection on a library collection and marks
  overridden values, with a reset button.
- **Update**: `library.updates` lists the components, styles, variable collections and variables
  with a newer published version; `library.acceptUpdates {ids? (library ids)}` pulls them in,
  keeping instance overrides; bound layers, aliases and explicit modes follow the updated values.
- **Paste between files**: instances copied from another open file (`edit.copy` there, `edit.paste`
  here) follow their mains: library components join (or bring along) this file's copies of the
  same library, components the other file publishes as a library are used from it (and that
  library is enabled when the other file is saved), and other mains stay behind: the instance shows
  a missing main (`component.restoreMain` makes it a main here).

### Team libraries

A file kept on a `figcraft-server` ([`server.md`](server.md#team-libraries)) can hold a library that
other files use through one of its share links. Each call waits for the server (a few seconds at
most when it can't be reached).

- `library.publishToServer {server?, key?, description?, include?, exclude?}` publishes the active
  document's library as the next version of the one the server keeps under the file (selection and
  per-asset versions as `library.publish`, following the server's version): `server` is an edit
  link of the file, or omit it while the document is in a live session on a server with edit
  access; with the server's address and key (`ws://host:47900/?key=…`, or `key`) the document is
  kept there as a new file first and the result has its `file`, `edit` and `view` links. A view
  link is refused. The document itself doesn't change (nothing to undo).
- `library.add {path: <share link>}` enables it (a view link is enough); then `library.place`,
  `library.applyStyle`, `library.bindVariable`… work as for library files. `library.list` and
  `library.updates` ask the server for a newer version first (`"server": true` in the listing);
  `library.acceptUpdates` takes it, overrides kept. The app's own background checks ask only about
  team libraries added (`library.add`) or checked (`library.check {library?}`) on this computer,
  so opening a file that names another one contacts nothing (#1097).
- `library.usage {library?}` → `{id, name, instances, files: [{name, updated, instances}],
  components: [{id, name, instances, files}]}`: what the files using it reported (each fetch
  reports the asking document's name and its instances per component). `library` is an enabled
  team library; omit it for the active document's own library on its server.
- `library.swap {from, to}` (any two enabled libraries, files or team) re-points every instance,
  style and variable copied from `from` to the published asset of the same name in `to` (variants
  by name in their set, styles by name and type, variables by collection, name and type, modes by
  name), keeping overrides; one undo step → `{components, styles, variables, unmatched: [names
  still from `from`, including this file's collections that extend one of its collections],
  removed (nothing of `from` was left, so it was removed)}`.

### Overrides and variants

- **What an instance overrides** (`node.set` / `text.set` on its layers; `get_node` lists
  `overrides`): fills, strokes, effects, opacity, visibility, layer names, corner radius, text
  content and text properties (font, size, spacing, resizing, lists…), auto layout padding, gap and
  alignment, layout grids, export settings and prototype interactions; nested instances swap
  (`component`) and take property values (`props`). Each stays when the main changes that property
  and goes with `component.resetOverride {id, group}` or `component.resetInstance`.
- **What it doesn't**: moving, rotating, resizing or re-constraining a layer inside an instance,
  its resizing (hug / fill / min / max), the auto layout direction and other auto layout settings,
  and adding, removing or reordering layers (outside slots) are refused (nothing changes); resize
  the instance itself (its layers follow the main's constraints) or edit the main.
- **Variants**: `instance.setProperties {ids?, properties}` switching variants keeps an override
  where the layer's name path matches and both variants had the same value for that property;
  text layers keep all theirs. The default variant (`instance.create` with a set, the Assets
  thumbnail) is the one in the set's top-left corner; `component.editProperty {name, default}`
  moves the chosen variant there, `{name, index}` reorders variant properties (the variants'
  names follow). `component.properties` on a set reports `conflicts`: variants sharing one
  combination of values.
- **Boolean properties**: on the main component, a boolean property's value is the visibility of
  the layers bound to it. `component.addProperty {type: "boolean", default, layer}`,
  `component.bindProperty {field: "visible"}` and `component.editProperty {name, default}` show or
  hide the main's bound layers to match the default (without `default`, `addProperty` takes the
  bound layer's visibility). Showing or hiding a bound layer on the main sets the default, and the
  property's other bound layers (in every variant of a set) follow. New instances start from the
  default; existing instances keep the values they have. Files open as saved: a main whose bound
  layer disagrees with the default stays that way until one of them is edited.
- `component.pushChanges {id?}` copies the instance's overrides into its main (from a layer of a
  nested instance too: always into the outer instance's main).

### Swapping instances

- `instance.swap {ids?, component}` shows another component in the instances (overrides on layers
  with matching names stay; property values with the same names too). Nested instances can be
  swapped on their own (an override of the instance they're in); a swapped nested instance keeps
  what its outer main overrode on matching layers as its own, so later edits of those in the main
  don't reach it.
- `instance.swapAt {component, x, y}` swaps the instance at a page point: of the instances under
  it (innermost first), the first showing a variant of the same component set, else the innermost
  (never one inside the dropped component); only text layers keep their overrides. The Assets
  panel does this when a component is dropped onto an instance with Alt (Option) held,
  highlighting that instance while dragging.
- Instance swap properties offer **preferred values**: `component.addProperty {type:
  "instanceSwap", default, preferred? [component or set ids]}` and `component.editProperty {name,
  preferred}`; `component.properties` lists them, and the Design panel's swap menus show them first.

### Slots

A slot is a frame inside a main component whose instances take any content of their own (add,
remove and reorder layers there, unlike the rest of an instance). It is a component property of type
`slot` (`component.properties` lists it: on a main with its `layers`, on an instance with
`value: {layer, edited, layers}`).

- **Define** (on the main): `component.makeSlot {id (a frame inside a main component), name?,
  preferred? [component or set ids]}` (the property is named after the frame by default; a new
  property goes last, as `component.properties` lists them in the order they were made, a rename
  keeping the place);
  `component.editProperty {name, preferred}`; `component.removeSlot {id}` (instances lose their own
  content in it). `component.addProperty {type: "slot", layer}` and `component.bindProperty {field:
  "slot"}` work too (one frame per component for a slot).
- **Fill** (on an instance): `instance.slotInsert {id (the slot layer, or the instance with `slot`
  (property name)), component, index?, replace?}` places an instance of a component in it; any
  structure command (`shape.*`, `node.reparent`, `edit.paste` with the slot selected, `edit.delete`,
  `node.reorder`, `instance.place` or a drag onto it) works inside it. The main's default content
  shows (and follows the main) until the slot is first edited; then the slot's layers are the
  instance's own (override group `slot`): kept across main updates, variant switches, copy/paste and
  detach, and instances placed there stand alone (their own overrides, properties and detach).
- **Reset**: `instance.resetSlot {id}` (or `component.resetOverride {id, group: "slot"}`, or
  `component.resetInstance`) brings back the main's default content.

`get_node` shows `"slot": "<property>"` on slot layers.

### Boolean groups

`bool.union` / `bool.subtract` / `bool.intersect` / `bool.exclude {ids?}` wrap layers in a new
boolean group, except: with one boolean group selected they set its operation (every operand follows
it again), and with layers inside boolean groups they set those operands' own operation. Each
operand above the bottom one combines with the result of the operands below it by its own
operation, or the group's (`node.set {operandOperation: union|subtract|intersect|exclude | null}`
on any operand, or `booleanOperation` on an operand that isn't itself a boolean group; on a group
`booleanOperation` is the group's). `get_node` shows `operandOperation` on operands that have one.
Operands stay editable layers: `select.children` (Enter) or a double-click selects them, `cmd: true`
clicks reach them directly, and moving, resizing or editing an operand's points (`vector.edit.*`)
updates the group's result on every pointer move, one undo step per gesture; a dragged operand
stays in its group. Clicks and `select.layersAt` hit a boolean group where its result is (a click in
a subtracted hole reaches what's behind it); while the group or one of its operands is selected, its
operands are hit instead, so each can be picked. `bool.flatten` bakes the result. Only the operands from the edited one up are
recombined during a drag (`figcraft_render::boolean`; timed by `cargo xtask bench`).

### Shape builder

The selected shapes' outlines split the canvas into regions (areas covered by the same shapes).
`shapeBuilder.regions {ids?}` lists them: an SVG `path` in page coordinates, the `sources` covering
each one (top-most first), the `parts` covering it (`{id, stroke}`: a shape's fill, or its stroke's
outlined area; top-most first), `bounds` and a `point` inside. Visible strokes take part as their
outlined areas, so stroked lines and open paths split the regions they cross and a stroke's band is a
region of its own. `shapeBuilder.merge {ids?, regions?, points?}` makes one vector from the regions
(by index, or the regions under page points), styled like the top-most fill or stroke over the first
one (a stroke's region is filled with the stroke's paints); `shapeBuilder.remove {ids?, regions?,
points?}` deletes them. Either way every shape covering a picked region loses that area (it becomes
a vector, or goes when nothing is left); the strokes of the shapes involved become outlined vectors
just above them (`outlines` in the result; an open path is replaced by its outline), and each call
is one undo step. The tool is `tool.shapeBuilder` (Shift+M):
click or drag across regions with `pointer` to merge, add `alt` to remove; `shapeBuilder.mode
{mode: merge|remove}` swaps what a plain click does.

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"shapeBuilder.regions","params":{"ids":[2,3]}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"shapeBuilder.merge","params":{"ids":[2,3],"points":[[120,80],[160,80]]}}}}
```

### Text on a path and pattern fills

- **Text on a path**: `text.onPath {text?, path?, start?, flip?}` (default: the selected text layer
  and the other selected layer) puts a text layer on a vector's or shape's path: the text takes the
  path layer's place, transform and box, and the path layer is removed (`edit.undo` brings it back).
  The text also takes the path layer's effects when it has any, and its fills when one of them
  shows, each with its style and variable bindings; otherwise the text keeps its own.
  The baseline follows the path from `start` (0..1 of its length); the text's horizontal alignment
  places it there (left starts, centre centres, right ends at it); `flip` runs it on the other side
  the other way. Glyphs past an open path's end are hidden; a closed path (ellipse, rectangle) wraps
  them around; only the first subpath is followed. `text.pathOptions {start?, flip?, ids?}` changes
  them, `text.removeFromPath {ids?}` makes it plain auto-width text again. Resizing the layer
  scales the path. The text stays editable (`text.edit.*`, `type_text`); `get_node` shows
  `textPath {d (SVG path data in the layer's space), start, flip}`. Exports draw it as outlines
  (PDF too: not selectable); underline and strikethrough aren't drawn on a path. With the tool
  `tool.textOnPath`, a `pointer` press within 6 screen pixels of a shape's or vector's first
  subpath puts a new, empty text layer on it, starting at the press, and starts typing: the text
  also takes the path layer's constraints and effects, and its fills when one of them shows. Ending
  the edit with nothing typed puts the path layer back with no undo step. Locked layers and layers
  inside instances are skipped.
- **Pattern fills**: `fill.pattern {source (layer id), ids?, index?, tile? (grid|horizontalHex|
  verticalHex), scale?, spacing? [x, y], align? (start|center|end), opacity?}` makes the fill at
  `index` (default the top one) the source layer repeated, at `scale` × its own size with `spacing`
  between tiles; hex tiles shift every other row (column) by half a tile; `align` puts a tile in the
  box's top-left corner, centre or bottom-right corner. The fill is `{type: "pattern", source,
  tile, scale, spacing, align}` and can also be set with `fill.set {paint}`. It updates as the
  source changes; a layer can't tile itself or a layer containing it (a pattern nested more than
  two deep draws nothing). Copies made together with their source tile the copied source. Exports:
  SVG `<pattern>` and PDF tiling patterns of the source's vectors (a source with effects as an image
  of the tile); the canvas draws one tile at the zoom it shows (at most 2048 px a side) and repeats it.

### Importing SVG

`open_file {path}` opens an SVG as a new document; `run_command {command: "file.import", params:
{path | data + name, x?, y?, center?}}` adds it to the current page as a frame (top-left at `x, y`,
or centred there with `center: true`, as pasting does). It comes in editable: groups named from
their `id`, transforms baked into the layers, `<rect>`/`<circle>`/`<ellipse>`/`<line>` (and paths
tracing those outlines) as rectangles with their corner radius, ellipses and lines, everything else
as vectors; strokes with their width, dashes, caps, joins and miter limit; linear and radial
gradients with their exact handles (`gradientTransform`, `reflect`/`repeat` unrolled, a focal
point as the centre); patterns as image fills (fill, fit, crop or tile); clip paths as vector masks
and masks as luminance or alpha masks; shadow and blur filters (`feDropShadow`, the offset / blur /
flood / composite chains, colour-matrix shadows, inner shadows, `feMorphology` spread) as drop
shadows, inner shadows and layer blurs; opacity and `mix-blend-mode`; text as text layers (fonts
the SVG names when installed, anchors, letter spacing, styled `<tspan>` runs, stacked lines as one
layer; text on one `<textPath>` as text on a path, with its start offset and anchor); images as
image fills, and SVG images imported in place. CSS, `<use>`/`<symbol>`, units
and `viewBox`/`preserveAspectRatio` are resolved. The result's `warnings` list what was left out
(filters that aren't shadows or blurs, rotated repeating patterns, a clip path's own clip path).
Linked files are never read, only data URIs; documents nested more than 512 elements deep are
refused.

### Importing `.fig` files

`open_file {path}` (a new, untitled document; the file is never saved over) and `file.import`
(pages appended) read `.fig` files, best effort. Each file carries the schema its data is encoded
with, so the data is decoded generically and mapped by field name onto the REST import below: the
same layers, paints, text, auto layout, components, styles, variables and prototyping come across,
with image fills from the file's own `images/` folder. Instances are rebuilt from their main
component with the overrides and property values the file records. Vectors come in as editable
vector networks (their outlines when the network is missing or damaged); sticky notes, shapes with
text, connectors, tables and similar layers come in as the closest frames, text layers and
vectors; set-variable, set-variable-mode and conditional actions come across; video fills are
skipped. The result's `opened.warnings` list what was approximated or skipped.

### Exporting `.fig` files (experimental)

`export {format: "fig", path?}` (or `run_command` `file.export {format: "fig"}`, or any `path`
ending in `.fig`) writes the whole document as a `.fig` file, from the same public format notes the
import uses: the layer tree, geometry, paints with image bytes, strokes, effects, text with styled
runs, vector networks, auto layout, constraints, components, sets, instances with their overrides
and property values, variable collections with their modes, variables and bindings, shared styles
with their layer links, and prototype interactions. `warnings` lists what was left out (comments,
Dev Mode data, some bindings and settings and the like). Round trips through FigCraft's own
importer are tested, and the example documents and small test files were opened in other software
(`docs/file-format-parity.md` says what was checked); other apps may not open every file. Without
`path` the file comes back as base64 `data`. Check one by opening it again:

```text
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"export","arguments":{"format":"fig","path":"/tmp/handoff.fig"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open_file","arguments":{"path":"/tmp/handoff.fig"}}}
```

### Importing REST API file JSON

The JSON returned by the public REST API's file endpoint (`GET /v1/files/:key`, best with
`geometry=paths` so vectors carry their outlines) and its nodes endpoint opens like any document:
`open_file {path}` (a new, untitled document; the JSON is never saved over) or
`run_command {command: "file.import", params: {path}}` (pages appended to the current document,
with their flows, prototype settings and variables).
The result's `warnings` list what was skipped or approximated (unsupported layer, paint and effect
types, video triggers, controller triggers on more than one button, expression functions other than a variable mode lookup over a
variable and one of its modes, bindings to library
variables, instances of components outside the file, which come in as frames). Pages, the layer
tree, geometry, paints, strokes, effects, corners, constraints, auto layout, layout grids, exports,
text with styled runs, vectors, components, component sets, instances and component properties are
mapped, and so is prototyping: `interactions` (on click, hover, press, drag, after delay, mouse
enter/leave/down/up and key triggers, controller triggers as controller buttons; navigate, overlays, back, scroll to, change to, open link,
set variable (values, variable references and expressions), set variable mode and conditional
(if / else if / else over expressions) actions; dissolve, smart animate, move, push and slide transitions with their easing,
custom bezier and spring curves included), flow starting points, scroll overflow and fixed/sticky
layers, and the prototype device. Image fills keep their `imageRef` and show a grey
placeholder unless the image files sit in an `images` folder next to the JSON, named by `imageRef`
(`images/<imageRef>.png`); nothing is fetched from the network.

Variables come from the local variables response (`GET /v1/files/:key/variables/local`):
collections with their modes (the default mode first), values per mode, aliases and scopes. Pass it
as `variables` (a path, or the JSON object or text) to `open_file`, `file.open` or `file.import`,
put it next to the file as `<name>.variables.json` or `variables.json`, or embed its `meta` in the
file JSON. Layers' `boundVariables` (sizes, gaps and padding, corner radii, opacity, stroke weight and
per-side stroke weights, visibility, text, font properties, single solid fills and strokes) and `explicitVariableModes` then
link to them, so switching a mode with `variable.setMode` restyles the layers (`mode: null` sets a
frame back to Auto, inheriting its parent's mode).

The same JSON carries variables between FigCraft files: `variable.export {collections?, format?,
path?}` writes this file's collections (default: all but library copies) as a local variables
response (`format: "json"`, the default; colour calculations as the colour they show in each mode,
other collections in their default modes) or as CSS custom properties (`format: "css"`: the default
mode on `:root`, each other mode on `[data-<collection>="<mode>"]`, aliases as `var(--name)`, colour
calculations as the colour they show in that mode (other collections in their default modes), numbers
in px unless the layers bound to them, their scopes or the variables aliasing them make them an
opacity or a font weight, so `border-radius: var(--radius)` reads `12px`) or as
Dart constants for Flutter (`format: "dart"`: an `abstract final class` per collection and mode, the
default mode's named after the collection and the others after the collection and the mode, with a
`static const` `Color`, `double`, `String` or `bool` per variable; names become camelCase Dart
identifiers kept apart, aliases are resolved to their value in that mode), and
returns `{text}` (or writes `path` and returns `{path}`). `variable.import {variables}` (an object,
JSON text or a path to a regular file of at most 256 MiB;
`variableCollections` and `variables` under `meta` or at the top level) reads
one back: a collection joins this file's collection of the same name
(values matched by mode name, new modes and variables added, a same-named variable of another type
skipped with a warning; library copies and extended collections are never targets), others are
added; aliases are checked for loops; it returns `{collections, variables, updated, warnings}`,
with a warning when the JSON holds no variables (`no variables found: expected meta.variables …`)
or fewer than its collections list in `variableIds`. The
Variables editor's Import… and Export… buttons do the same through file dialogs.

Strokes take colour styles and variables like fills: `style.apply {style, target: "stroke"}` links
a colour style to the strokes (`styles.stroke`; edits of the style follow, `style.detach {which:
"stroke"}` keeps the colour), and `variable.bind` binds `strokeWeight` and each side's weight on its
own (`strokeTopWeight`, `strokeRightWeight`, `strokeBottomWeight`, `strokeLeftWeight`; binding or
setting one side with `node.set` turns on individual strokes, the other sides at the weight) to
number variables scoped for stroke weights.

`variable.bind` also binds gradient stops (`fills/<i>/stops/<j>/color`, `strokes/<i>/stops/<j>/color`;
the binding follows its stop when an earlier stop is removed and its paint when the list is
reordered) and variant properties of instances (`componentProperties/<property>` with a string
variable: the instance shows the variant its value names; a value naming no variant changes
nothing). The colour picker binds the selected stop; the Design panel's variant dropdowns take a
string variable.

### Comments

Comment threads live in the file, pinned to a page point or to a layer (`anchor`: a node id, or
`true` for the innermost frame under the point) so they move with it; when the layer is deleted the
pin stays where it was, on the nearest surviving parent. Driven with `run_command`:
`comment.add {x, y, text, page?, anchor?}` (returns `{id, link}`), `comment.reply {id, text}`,
`comment.edit {id, text, reply?}` and `comment.delete {id, reply?}` (a message is the thread's
first one, or `reply`: an index into its replies; deleting the first message deletes the thread),
`comment.react {id, emoji, reply?, on?}` (toggles your reaction), `comment.resolve {id, resolved?}`,
`comment.move {id, x, y, anchor?}` (pins it to the frame under the new point unless `anchor: false`).
An agent comments under its own name, never the user's: every command an MCP client runs acts as
that client's name from `initialize` (`clientInfo.title`, else `clientInfo.name`) with " (agent)"
after it, e.g. "Claude (agent)", or "AI agent" when the client gave none. Its messages, reactions
and read state are its own, it can edit and delete only its own messages, and its messages carry
`"agent": true` in `comment.list` (the app shows an **Agent** label by the name); the user's name in
Preferences is unchanged. It is set once per connection, so no command needs `author`; an `author`
param still wins. In the app, comments act as the name from Preferences (`pref.userName {name}`,
default "You"); only a message's author can edit or delete it. `@Name` in a message mentions
someone (`comment.people` lists the names used in the file: everyone who wrote or reacted, and
every name a message @mentions, once each in any case; and, as `you`, who is acting).

Read state is kept per name: a new message is unread for everyone but its author;
`comment.markRead {id? | link?, read?}` marks one thread (or, without `id`, all of them) read or
unread (not an undo step). `comment.list {page? | "all", filter? (all|open|resolved|mine|unread),
sort? (date|page), query?, frame?, mentions?}` returns the threads with their position, `frame`/`frameName`,
`pageName`, `unread`, `link` and replies (with `edited` times and `reactions`). Each message, the
first and every reply, has `mentions`: the names it @mentions by the rule the app highlights them
with, `{name, known, you}` each. `known` means the file's people (`comment.people`, or you) have
the name, spelt as they spell it; `you` means it names you (or `author`). `@Name` counts at the
start of the text or after a character that isn't a letter or digit, so `a@b.com` mentions nobody;
an unknown name is the letters and digits after the `@`. `mentions: "<name>"` (any case; a leading
`@` is fine) lists only the threads where a message, the first or a reply, mentions that name as
those `mentions` report it, combined with the other filters: `{"mentions": "Claude", "filter":
"unread"}` is the unread threads that tag Claude. `@Ada Lovelace` mentions "Ada Lovelace", not
"Ada", when the file knows Ada Lovelace; a name nobody in the file has can still be several words
(`"Zoe Smith"` finds `@Zoe Smith`). Links are
`figcraft://comment/<id>`: `comment.link {id}` returns one, `comment.open {id | link, view?}` goes
to its page, zooms to its frame (`fit`, the default; `center` keeps the zoom; `keep` doesn't move)
and opens it in the app, and `comment.next {id?, dir?, …list params}` (`mentions` too) steps to the
next or previous thread of a list the same way. Both return the thread as `comment.list` lists it,
with each message's `mentions`. `comment.zoom {ids}` zooms in on a group of pins. Comment ids made in
a live session are large numbers (their creator's id space): pass them back as given.

An agent can read every open thread as one resource (`figcraft://comments`) or call
`comment.list`; `comment.prompt {ids?, page? (default "all"), filter? (default open), sort?,
query?, includeResolved?}` returns `{count, truncated, text, comments}`, where `text` is a Markdown
task brief with the document, each thread's page, frame, anchor, point, messages, replies and link,
and the instructions to fix, reply and resolve. `comments` is bounded metadata for at most 200 of
the selected threads (id, page, frame, anchor, status, link, author, preview, reply count and
`mentions`: the names the whole thread @mentions, as `comment.list` reports them per message, at most
32 with names cut to 80 characters), not the full threads: use `comment.list` for those. The Comments panel's **Copy for AI** action uses
it. The `fix_comments` prompt runs the review loop: inspect the anchored layer, make the
smallest change (one `batch` per thread), reply with what changed, verify with a screenshot, then
resolve. Reply before resolving: `comment.reply` reopens a thread, so resolving first and replying
after leaves it open again. `auto_resolve: "no"` makes the agent reply and leave the thread for the
user. Comments are the feedback queue for any MCP client.

While attached, `wait_for_changes` long-polls the app's event log: `comment.added`, `comment.replied`,
`comment.edited`, `comment.reacted`, `comment.resolved`, `comment.moved` and `comment.deleted` carry
`{command, commentId, reply}`. Read the thread with `comment.list` after an event. A `batch` that
rolls back reports none of its steps' events. The log keeps the newest 1024 events; `reset: true`
means the cursor is ahead (the app restarted), and `dropped` counts events older than the retained
window. Headless returns immediately; the control method behind it is `events.poll` (see
[control-protocol.md](control-protocol.md)).

### Version history

Named snapshots of the document, kept in the `.figcraft` file (they persist once the file is saved;
a new, renamed or deleted version marks the file as changed). Driven with `run_command`:
`file.saveVersion {name?, description?}` (returns `{id, index, name, description, time}`; untitled
versions are named "Version N"), `version.list` (oldest first; `time` is Unix seconds),
`version.restore {id | index}` (replaces the design with the version as one undo step; the file
name and the comments stay, comments on layers the version lacks staying where they were, and the
design it replaces is first kept as an autosave unless a version of it was just saved or restored),
`version.rename {id | index, name, description?}` (a titled autosave becomes a named version),
`version.delete {id | index}` and
`version.duplicate {id | index}` (opens the version as a new document tab).
`version.preview {id | index}` shows a version in place of the document, read-only: edits, undo and
gestures are refused until `version.restore` keeps it or `version.exitPreview` goes back. Saving,
closing or saving a version ends the preview first, and `version.list` marks the shown version
`previewing`. Indexes are validated
(a negative or out-of-range index is an error). A file keeps at most 100 versions.
`version.autosave` saves an automatic checkpoint (`auto: true`, named "Autosave") when the document
has unsaved changes since its last version; the app runs it every 30 minutes for each open document,
and a file keeps the newest 20 autosaves (user versions are never dropped for them). Merging a
branch adds the main file as it was (an autosave, when it changed since its last version) and then
`Merged “<branch>”`. `file.versionHistory` returns the same list and opens the panel in the app.
Versions stay with the person who saved them: in a live session they aren't shared.

### Branches

A branch is a copy of the file to try changes in, reviewed against the main file and merged back.
Branches are kept in the main `.figcraft` file (saving either tab saves the main file with its
branches). Driven with `run_command`:

- `branch.create {name?}` copies the file into a new branch tab and returns `{id, name, index}`.
  The branch remembers the file as it is now (its base) and makes new ids in its own id space, so
  layers, styles and comments made on both sides never clash. Its history starts with a `Branch
  created` version.
- `branch.list` lists the branches of the file shown (or of the file a branch tab belongs to):
  `[{id, name, time, state: active|merged|archived, index?}]` (`index`: its open tab).
- `branch.open {id}` opens a branch in a tab (or switches to it). Edit it like any document.
- `branch.review {id?}` (default: the branch tab shown) returns what changed on each side since the
  base and the conflicts: `{branch: {layers, document, truncated}, main: {…}, conflicts: [{key, id?,
  name, property, branch, main, deletedBy?}]}`. `layers` are the changed layers with their property
  changes formatted like Inspect (`fill #2563EB → #DC2626`); `document` lists styles, variables,
  comments and file settings. A conflict is the same property changed on both sides to different
  values (key `<layer id>/<property>`, e.g. `4/fills`; fills, strokes, effects, layout grids,
  export settings, guides, prototype interactions, annotations and Dev Mode resources compare item
  by item, so changing different items of one list on each side, or adding items at its end on one
  side, merges without a conflict; other lists, such as code links, compare whole; a layer's parent
  and position are one property, `parent`), a layer deleted on one side while the other edited it or what's inside it
  (`<layer id>/layer`, with `deletedBy`), or a document-level entry or setting changed on both sides
  (`style/<id>`, `variable/<id>`, `comment/<id>`, `doc/<property>`…).
- `branch.merge {id?, resolutions?}` merges the branch into the main file as one undo step,
  archives it (`state: merged`), closes its tab and shows the main file; the main file's version
  history gets a version for the merge (`Merged “<name>”`, after an autosave of main as it was when it had changed). `resolutions` maps every
  conflict key to `"branch"` or `"main"`; the merge is refused while a conflict has none, so
  neither side's work is dropped without a choice.
- `branch.updateFromMain {id?, resolutions?}` brings the main file's changes since the base into the
  branch the same way (the new base is the main file as it is now) and adds an `Updated from main`
  version to the branch's history. It is not undoable: the branch tab's undo history starts over.
- `branch.archive {id?}` sets a branch aside without merging (it stays in the file and can still be
  opened, but not merged).

Each branch keeps its own version history: on a branch tab, `file.saveVersion`, `version.autosave`
and the `version.*` commands (list, preview, restore, rename, delete, duplicate) act on the
branch's versions, which are saved in the file with the branch and stay with it when it is merged
or archived.

In a live session branches work the same way and stay with the person who made them (others
don't see them). A branch made in the session makes its ids in your own client space, far above
the ids the main file makes, so the session server takes them. Everyone's edits on main while the
branch is open are main's changes in the review (against the branch's own base), and conflicts
with them are resolved as offline. `branch.merge` is one edit of the session document: everyone
gets it, and it is one step of your own undo history. It is refused on a view link, and for a
branch made outside the session (or in an earlier one) whose new layers, styles, variables or
comments lie in another id space, which the server would refuse; merge those once the file has
left the session. While a session is still starting, branch commands that need your client id
wait for it.

### Collaboration changes

`collab.diff {against? (undo: the last undoable action, the default | saved: since the last
save)}` returns `{changes, count, bytes}`: the change batch a live session would send for that edit
(per-property `create` / `set` / `delete` and document-level changes, see
`docs/collab-protocol.md`) and its binary size. It changes nothing. In a live session, where undo
is per user (`edit.undo` / `edit.redo` take back only your own steps, see
`docs/collab-protocol.md`), only `saved` applies.

### Windows

The attached desktop app can show files in several windows, each with its own tabs; the files,
preferences, libraries and clipboard are shared. `run_command` with `window.new {path?, index?}`
opens one (a new file, the file at `path`, or open file `index` moved out of its window) and
`window.focus {id}` brings one to the front; `inspect_ui` lists them as `windows` (`id`, `main`,
`title`, `files`, `active`, `focused`). Tools act on the window in front (the last one opened, focused or clicked):
focus a window before working on its files, and pass its id to `screenshot {window}` to see it.
Headless (one session, no windows) and in the browser the two commands do nothing.

### Naming the document

Tools act on the file in front, so with an attached app a tab the user clicks mid-task would
change where the next edit lands. `run_command`, `batch`, `create_node`, `set_properties`,
`inspect_document`, `get_node`, `select`, `export`, `screenshot`, `save_file`, `undo` and `redo`
take an optional `document`: the file's `id` (in `run_command {command: "doc.list"}`, and the `id`
of the `document` outline `open_file` and `inspect_document` return) or the path of an open file.
The call acts on that file and the tab in front stays in front, unless the command itself opens or
switches to another file (`file.open`, `file.new`, `doc.switch`), as it would without `document`. A
file that isn't open is an error. `doc.list` gives an agent each open file's `id`, name and state, not`nwhere it is on disk (#1117). `screenshot {window}` captures what the window shows, so it
needs the file in front. Over the control channel, `engine.execute`, `document.inspect`,
`node.get` and `ui.render` take the same `document`; other methods refuse one naming a file behind
the tab in front. Headless sessions take it too.

### Live sessions

People (and agents) edit one file together and see each other's cursors, selections and
viewports. Both the headless session and the attached app can host and join (the web build can
join but not host); each call takes in what happened since the last one and sends your edits (a
joined file opens as a new document).

Sessions are per document: every open document has its own session or none, and several run at
once (host one document, join another, publish a third). They keep syncing while another document
is active; there your presence is empty (no cursor, selection or view). An attached agent's own
presence in the app (above) is never part of a live session's presence. The `collab.*` commands
and `collab.peers` are about the **active document's** session (`doc.switch {index}` changes which
one that is), and `sessions` in `collab.peers` lists them all. Closing a document ends its session
only: a host stops hosting it, a guest leaves it.

- `collab.share {via?, access?, server?, name?, port?, bind?}` shares the active document with a
  **join code**, the short way to do what `collab.publish` and `collab.host` do. `via: "online"`
  (the default) publishes it to the server in Preferences (`pref.publishServer {url?}`: the
  FigCraft server unless changed; `{url: ""}` turns online sharing off, `{}` goes back to the
  built-in one; `server` names another for this call, with `?key=…` when it needs its key) and
  returns `status: "connecting"`; once `collab.peers` says `"live"`, `code` is the code to give
  others (`K7M-2QX-9TD`) and `share` is `{via, access, shared, editCode, viewCode, commentCode, editLink,
  viewLink, commentLink}`: links and codes made for others (`published.edit` / `published.view` and `link` are
  your own way in; keep them). `access: "view"` (or calling again with it while shared) makes
  `code` the view-only one, and `access: "comment"` the one for viewing and commenting (whoever
  joins with it adds, replies to, reacts to and resolves comments but can't change the design;
  `commentCode` is null from a server older than comment links, and asking for it then fails). A server older than join codes leaves `code` null: give
  `share.editLink` / `share.viewLink` instead. `via: "local"`
  hosts the document for people on the same network and returns at once with a longer `code`
  (20 symbols, carrying this machine's IPv4 address, the port and the session's secret; `bind:
  "local"` keeps it to this machine); everyone who joins edits. **Calling it publishes the
  document to a server other people run: do it only when asked to share.** `collab.stop {}` stops
  the sharing. Local: the session ends for everyone (`{ended: true}`). Online: `{ended: false,
  shared: false}`: the others' links and codes are revoked and they are disconnected at once (with
  their copy), while you stay in the file, which stays on the server, open only to you
  (`share.shared` is false, `code` null); `collab.share {}` then shares the same file again with
  new codes, `collab.leave {}` leaves it there, and `collab.manageFile {file, action: "remove"}`
  takes it off the server (the file is recorded with your online files: see `collab.files`
  below).
- `collab.host {port?, bind?}` starts hosting the active document (a session that document was in
  ends; other documents' sessions go on): `bind` is `"local"` (this
  machine, the default), `"lan"` (the local network) or an interface address; the port is 47900,
  or any free one when it's taken, as it is for a second hosted document (`port` picks one, 0
  any). Returns the session as
  `collab.peers` does, with `link` (`figcraft://join?host=…&file=…&token=…`) and `url`
  (`ws://…/?file=…&token=…`): both carry the session's secret, so share them only with people you
  invite. `collab.stop {}` ends the active document's session.
- `collab.join {code}` or `collab.join {link}` (a join code of either kind: spaces, dashes and
  case don't matter, `O` reads as `0` and `I` or `L` as `1`; or a link in any form: an app's
  invite, a `figcraft-server` share link, or the `ws(s)://…?file=…&token=…` URL; either parameter
  takes either) returns at once with the new session (`session`, `status:
  "connecting"`) and touches no other document's session. Its file opens as a new document, which
  becomes the active one: poll
  `collab.peers` until `status` is `"live"`. Until then the join shows on the document it was
  started from when that one isn't in a session itself, and in `sessions` either way, where it
  becomes `"failed"` with a `reason` when it can't be joined ("No session with that code", "That
  code has expired or its sharing was stopped", "Couldn't reach the server (…)", too many wrong
  codes from your address, a wrong token, a full
  session, a revoked link, an unreachable host, a `wss://` server whose certificate isn't trusted:
  see `FIGCRAFT_CA_FILE` in [`server.md`](server.md#behind-a-reverse-proxy-tls)). `collab.leave {}`
  leaves the active document's session (or ends a join started from it that is still on its way
  or failed), and `collab.leave {session}` the session with that id (from `sessions`), whichever
  document is active; the synced document stays open as your copy.
- `collab.publish {server, name?, key?}` puts the active document on a `figcraft-server`
  ([`server.md`](server.md)) and syncs it from there (as that document's session; others go on): `server` is the address
  `figcraft-server key` prints (`ws://host:47900/?key=…`; or give `key` separately), `name` the
  file's name (default: the document's). It returns at once with `status: "connecting"`; the
  document is read-only until the upload is done, then `collab.peers` has `published: {server,
  file, name, edit, view}` with the file's first edit link and view link (they carry their
  tokens: share them like passwords). Someone joining with the view link gets `access: "view"`:
  every edit on the synced document is refused with "View only: …" (the server refuses them too).
  With a comment link (`access: "comment"`) the comment commands work and sync, and every other edit
  is refused with "Can comment: …".
  On a server open to the public ([`server.md`](server.md#running-a-public-server)) `server` needs no
  key (`wss://figcraft.example.com/`): `published` then also has `owner` (the owner token, shown this
  once: keep it) and `expires` (when the server deletes the file unless someone opens it, seconds
  since 1970), and `hosting` says who runs the server. A server that isn't open answers that it
  takes files only with its key (`reason` in `collab.peers`).
- `collab.serverInfo {server}` → `{open, operator, terms, idleDays, maxFileMb, expires}`: what a
  server says about its hosting, before publishing (`open`: no key needed; `idleDays`: how long
  files published without the key are kept unopened, null when they are kept). A server older
  than open hosting is reported as taking files only with its key.
- `collab.files {refresh?}` → `{files: [{server, host, file, name, created, opened, shared, code,
  viewCode, state, expires, session, document, hasOwnerToken}], problems: [{server, host, state,
  reason}], max}`: **your online files**, the files this session (or, attached, the app with its
  saved preferences) put on a server without the server's key, most recently opened first. `file`
  is the id the other commands take; `shared` says whether others get in now, with `code` /
  `viewCode` to hand out; `session` and `document` name the session and open document a file is
  in, or are null. Without `refresh` nothing is asked of any server and `state` is what was last
  heard (`"unknown"` until asked). `refresh: true` asks each server once what became of its files:
  `"online"`, or `"gone"` (removed, or expired unopened); a name changed elsewhere and links
  revoked elsewhere are taken over. A server that can't be reached, or is older than the request,
  is listed in `problems` (`state: "offline"` with the `reason`, or `"unsupported"`) and its
  files stay as they were. **Owner tokens and your own links are never in the result**
  (`hasOwnerToken: true` stands for the token); they are kept with the preferences. The headless
  session keeps its list only while it runs: note `published.owner` from `collab.peers` if a file
  must be managed after it ends.
- `collab.openFile {file, server?}` joins one of your online files again as its owner →
  `{session, status: "connecting", document, alreadyOpen, file, server, name}`; it opens as a new
  document (poll `collab.peers` until `"live"`), where `collab.stop {}` and `collab.share {}` stop
  and start its sharing. A file that is open already becomes the active document instead
  (`alreadyOpen: true`, `document` its index). `server` is needed only when files on several
  servers share an id. A file whose `state` is `"gone"` is refused.
- `collab.forgetFile {file, server?}` → `{forgotten, file, server, onServer}` drops a file from
  the list **on this computer only**: the file stays on its server (`onServer`) until it expires,
  and can't be managed from here again. To take it off the server use `collab.manageFile {file,
  action: "remove"}` instead.
- `collab.recoverFile {path, file, server?, session, client, seq}` is what the desktop app's crash
  recovery runs for the copy of an online file (the last three come from the note beside the
  copy): it joins the file as its owner and keeps the copy until the file arrives, then discards
  it (the server has everything), applies its unconfirmed changes on top as one synced edit, or
  opens it beside the file as "<name> (recovered copy)"; a file that can't be joined opens the
  copy as an unsaved document. Returns `{session, status: "connecting"}`.
- `collab.manageFile {file, action, server?, owner?, …}` manages a file published without the key,
  with its owner token. `server` and `owner` default to what is recorded for one of your online
  files (`collab.files`); give both for a file that isn't recorded here. `links` → `{links: [{id,
  access, label, created}]}`; `createLink {access?: view|edit, label?}` → `{id, access, link}` (a
  new share link, shown once); `revoke {link}` (whoever joined with it is disconnected); `share`
  → `{shared: true, code, viewCode, editLink, viewLink}` (new links and join codes for others; the
  same ones when it is shared already) and `unshare` → `{shared: false, …}` (the recorded links
  and codes are revoked and whoever joined with them is disconnected; the file stays yours on the
  server); `rename {name}` (its name on the server and in your list) and `remove` (the file is
  deleted from the server, whoever is in it is disconnected, it leaves your list and a session
  open on it here ends, its document staying open as a copy) → `{done}`. A wrong token, or
  another file's, is refused.
- `collab.report {reason}` (at most 500 characters) reports the file you joined on a server to its
  operator → `{sent}`; the session's message says when the server kept it, or why not (ten an hour
  per address). Servers older than this version don't take reports (`hosting` is null).
- `collab.peers {}` → the active document's session: `{available, canHost (false in the web
  build), session (its id; null when the document isn't in a session: then `status` is `"off"`
  and the rest is null or empty), status
  (off|connecting|live|reconnecting|failed), reason?,
  attempt?, role (host|guest), me, name, color, fileName, pages, link, url, address, network,
  access (edit|view|comment), readOnly (the active document can't be edited now: view only,
  comment only, or being published), published (a file you published: {server, file, name, edit, view, owner, expires}),
  code (the join code to give others when the document was shared with `collab.share`, else
  null), share ({via: online|local, access, shared, editCode, viewCode, commentCode, editLink, viewLink,
  commentLink}, or
  null; `shared: false`: your file on a server, not shared now),
  hosting (what the server of the file you're in says: {open, operator, terms, idleDays, maxFileMb,
  expires}, or null),
  follow, signals (the host passes cursor chat and spotlights on), chat (your cursor chat),
  spotlight (who spotlights, or null), spotlighting (you do), syncing (the document arrived),
  document (the index of the synced open document),
  editing (the text layer you are typing in: {node, layer, caret, selection}, or null), peers: [{id,
  name, color, editing (the text layer they are typing in, same shape, or null), chat (their open
  cursor chat message, or null), presence: {cursor:
  [page, x, y], selection: [ids], viewport: [page, x, y, w, h], editing_text: {node, caret,
  selection?}}}], sessions: [{session, document (the index of its open document; null for a join
  on its way), status, reason, role, fileName, link, peers (how many others are in it)}]}`.
  `sessions` is every session of the app, whichever document is active. Carets and selections
  count characters; someone's typing syncs when they
  finish, so their caret may be past the end of your copy until then. Starting `text.edit.begin`
  in a layer someone else is typing in sets the message "<name> is editing this text too: the
  version finished last is kept" (both edits sync as whole values when each person finishes; the
  last one wins).
- `collab.cursor {x, y}` puts your cursor at a page position (omit both to hide it): an agent's
  pointer. Your selection, page and view go out by themselves when they change.
- `collab.follow {client}` follows that person's viewport; `collab.follow {}` stops (so does any
  pan or zoom of your own, or Escape).
- `collab.removePerson {client}` removes that person from the session: they are told the session
  is over for them (they keep their copy and don't reconnect), and their address can't join the
  file again while the session runs. Only the person hosting, or the owner of a
  file on a server (with its owner token), can; a server older than this request says it can't.
  Returns `{removed, name}`.
- `collab.cursorChat {text}` shows `text` next to your cursor for everyone in the session (put the
  cursor somewhere first with `collab.cursor`): at most 100 characters, control characters become
  spaces, `""` closes it. Others see it fade 5 seconds after its last change, so send it again to
  keep it up. Without `text` it opens the chat box in the app (`/` on the canvas), which a
  programmatic call never does: it returns `{open}`. Others' messages are each peer's `chat` in
  `collab.peers`.
- `collab.spotlight {on?}` (no `on`: switch) asks everyone in the session to follow your view and
  page; people who join later follow too, and anyone who pans or zooms stops following (the
  spotlight goes on; they can follow again). Someone else's spotlight takes over yours. Returns
  `{spotlighting}`. Both commands need a live session whose host is this version or newer (`signals`
  in `collab.peers`); otherwise they say why.
- Styles, variables, collections, modes and comments you create on the session's document take
  ids from your own id space (`S:<n>`, `VariableID:<n>`, … with large numbers), so two people
  creating them at once never overwrite each other's; outside a session ids look as before (`S:1`,
  `VariableID:1`…).
- On the session's document every command (or `batch`, or gesture) is one step of your own undo
  history: `edit.undo` / `edit.redo` take back only your steps and leave what others changed since.
  An edit the host refuses (someone deleted the layer) is rolled back and the reason is the
  session's message. While the connection is reconnecting, editing goes on; the edits go out when
  it is back (the same client, catching up with what it missed).
- `collab.registerLinks {on?}` → `{on, registered}` makes the desktop app the opener of
  `figcraft://` invite links, or stops (no `on`: switch). Only where the app registers them itself
  (Windows, per user); elsewhere it reports the command unavailable. It also sets the
  Preferences checkbox. Starting the app with an invite link (`figcraft "figcraft://join?…"`)
  joins that session.

### Plugins

Sandboxed WebAssembly plugins act through the same commands (see [`plugins.md`](plugins.md) for the
manifest, the ABI and the Rust SDK). Driven with `run_command`: `plugin.import {path}` (a
`figcraft-plugin.json` or its folder) or `plugin.import {manifest, wasm}` (the manifest as an
object or JSON text and the module as base64, for this session only), `plugin.list`,
`plugin.remove {id}`, and `plugin.run {id, command?, params?, fuel?}`, which returns
`{plugin, command, output, logs, calls, fuel, panel}`. A run is one undo step and is rolled back when it
fails; a plugin only reads the document unless its manifest asks for `write`, may be limited to
some commands, and can never touch files, other documents, the undo history, dialogs or other
plugins. A headless MCP session starts with no plugins: import them first. `plugins.runLast` runs
the last plugin command again.

A plugin that shows a panel (API 2) can be used headless too: `plugin.ui {id?}` returns plugin
`id`'s open panel, or without `id` the one opened last (`{plugin, command, title, controls,
values}`, or null; several plugins can have panels open at once), `plugin.event {id, control, value?}`
presses a button or sets a field as a click would (the plugin is called back; one undo step;
returns like `plugin.run`, with the updated `panel`), and `plugin.close {id}` ends the session.
Plugin settings are kept per plugin with the preferences (in a headless session, for the session).

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"plugin.run","params":{"id":"org.figcraft.examples.swatch-grid","command":"panel"}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"plugin.event","params":{"id":"org.figcraft.examples.swatch-grid","control":"palette","value":"Forest"}}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"plugin.event","params":{"id":"org.figcraft.examples.swatch-grid","control":"generate"}}}}
```

Widgets (API 3 and 4, layers on the canvas a plugin draws) are driven the same way: `widget.insert
{plugin, x?, y?}` puts one on the page (centred in the view without a position) and returns `{id,
plugin, state, targets, output, logs, calls, fuel}`; `widget.click {id, target}` clicks one of its
`targets` as a click on the canvas would (the plugin redraws it; one undo step; returns the same);
`widget.list` lists every widget in the file with its plugin, page, state and targets. With plugin
API 4 a widget can have editable text fields and a property menu (both listed with it, as `fields`
and `menu`): `widget.setText {id, field, value}` sets a field as typing in it would (the value,
at most 1,000 characters, goes into the state under the field's name and the plugin redraws; one
undo step), and `widget.menu {id, item, value?}` uses a menu item (a toggle's `value` is `true` or
`false`, by default the opposite of now; a colour's is one of its options). A widget's state is
layer data, so it is saved, undone and synced with the file.

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"widget.insert","params":{"plugin":"org.figcraft.examples.counter"}}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"widget.click","params":{"id":12,"target":"increment"}}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"widget.list","params":{}}}}
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"widget.setText","params":{"id":12,"field":"label","value":"Votes"}}}}
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"widget.menu","params":{"id":12,"item":"big"}}}}
```

Plugins from a `figcraft-server`'s plugin directory ([`plugins.md`](plugins.md#plugin-directories)):
`plugin.directory.list {server?, query?}` lists the latest version of each plugin with the
permissions it asks for (`access`, `commands`), its size and SHA-256, and `installed` / `update`;
`plugin.directory.install {id, version?, confirm: true, sha256?}` downloads it, checks the SHA-256 and
installs it for the session (an agent must pass `confirm: true`, after showing its user the
permissions: without it nothing is downloaded); `plugin.directory.publish {path | plugin, server}` and
`plugin.directory.unpublish {id, version?, server}` need the server's key in `server`
(`ws://host:port/?key=…`).

```json
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"plugin.directory.list","params":{"server":"ws://127.0.0.1:47900/"}}}}
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"plugin.directory.install","params":{"id":"org.figcraft.examples.swatch-grid","confirm":true}}}}
```

### Dev Mode

- **Pages as React + Tailwind:** `inspect.code {id, language: "tailwind"}` writes a layer tree as a
  React function component styled with Tailwind CSS v4 utilities and no stylesheet. Auto layout
  becomes `flex`/`flex-col`/`gap-*`/`p-*` on the 4px spacing scale, fixed-width auto layout frames
  and text become fluid (`w-full max-w-[960px]`; as wide as the frame: `w-full`) and a frame's
  fixed height a minimum, per-side strokes become `border-b` and the like, and instances are
  inlined with their overrides (component code is `language: react`), except instances whose component
  is connected to code (`codeConnect.set`/`codeConnect.import`, React): those are their snippet, e.g.
  `<Button variant="outline">{"See examples"}</Button>`, with each import once at the top. A design
  system mapped to shadcn/ui this way exports pages that use the project's own components. Bound variables become theme
  utilities (`bg-accent`, `rounded-card`, `text-muted`) that `variable.export {format: "tailwind"}`
  declares: `@import "tailwindcss";` and an `@theme static` block (every token is emitted, also those only read through `var()`) named by namespace (`--color-*`,
  `--radius-*`, `--text-*`, `--spacing-*`, `--font-*`; a number's namespace and unit come from what
  layers bind it to), other modes on `[data-<collection>="<mode>"]`. With `mobile: <frame id>` (the
  same layer names, mobile layout) the code is mobile first: the mobile frame's classes, the
  desktop frame's that differ behind `lg:`, and layers only on desktop `hidden` plus `lg:flex`
  (auto layout frames) or `lg:block` (other layers).
- **Component code:** `inspect.code {id, language: react|leptos|vue|swiftui|compose|arkui|flutter}` on a component,
  a component set or a variant writes the component as a reusable unit: React (TSX) with a props
  interface, Leptos with a `#[component]` function (`#[prop]` defaults, `view!` markup), Vue as a
  single-file component (`<template>`, a `<script setup>` with `defineProps` and its defaults, a
  `<style scoped>`; a slot layer is Vue's own `<slot>`, so it takes no prop), SwiftUI
  with a `View` struct, Compose with a `@Composable` function, ArkUI with a `@Component` ArkTS
  struct (`@Prop` parameters, `@BuilderParam` for swaps and slots), Flutter with a `StatelessWidget`. Its component properties are the
  parameters, with their defaults: variant properties become string unions (React, Vue) or enums,
  booleans become flags around the layers they show, text properties strings, instance swaps views
  (`React.ReactNode`, Leptos `ViewFn`, `AnyView`, a composable lambda, an ArkUI builder, a `Widget`) and slots
  optional content.
  Every variant of a set shares one definition; in React, Vue and Leptos the other variants'
  differences become CSS rules on `data-*` attributes. A Vue template holds no arbitrary
  expressions, so what React writes as a braced expression has its own spelling there: text is
  `{{ prop }}`, a boolean property's layer carries `v-if="prop"`, and a binding is
  `:prop="value"`. On an instance (or a layer tree containing
  instances) each instance is a call passing only the values it changes, e.g. `<Button
  state="Hover" label="Buy" />`, `<Button state=ButtonState::Hover label="Buy" />`,
  `<Button state="Hover" />`, `Button(state:
  .hover, label: "Buy")`, `Button(state = ButtonState.Hover, label = "Buy")`, `Button({ state:
  ButtonState.Hover, label: "Buy" })`, `Button(state: ButtonState.hover, label: "Buy")`. Names are escaped and
  unique (keywords get a suffix, `default` → `defaultValue`, Rust's `type` → `type_`; Leptos names
  are snake_case, Dart enum members lowerCamel; names that would shadow framework types get one too, `Text` → `TextComponent`). `inspect.componentCode
  {id?, language?}` returns the parts: `{component, name, set, properties: [{name, param, type,
  default, value? (instances)}], definition, usage? (instances)}`.
- **Code connect:** link a component (a variant links its set) to the code component it is built
  as, per language (`react`, `leptos`, `vue`, `swiftui`, `compose`, `arkui`, `flutter`): `codeConnect.set {id?, language?, importPath?,
  name?, template?}` (undoable; fields left out keep their value; the defaults are the generated
  name and a call passing every property) → `{component, language, code, unknown}`. Templates are
  text, never executed: `{property}` (its name or parameter name) is the instance's value escaped
  for the language (React and Leptos keep the markup's braces, so `size={size}` gives
  `size="Large"` and `disabled={disabled}` gives `disabled={true}`; a Vue binding takes an
  expression, so `:size={size}` gives `:size="&quot;Large&quot;"` and `:disabled={disabled}` gives
  `:disabled="true"` — the string keeps its JS quotes as entities, and a static attribute
  (`size={size}`) takes plain text; a
  component swap is the component itself, `:icon={icon}` giving `:icon="Icon"`; a string that needs
  escapes keeps them inside the value, where the attribute's own quotes become `&quot;`; Leptos
  imports with `use
  <importPath>::<name>;`), `{property ? "a" : "b"}` inserts one of two
  strings, any other `{…}` stays as written (`unknown` lists placeholder names that aren't
  properties). Then `inspect.code` on an instance or the component returns the import line and the
  snippet instead of generated code (`connected: true`), connected instances inside a generated
  layer tree (also in a component's definition, in the content an instance puts in its slots, and
  the components a connected template's instance swaps place) are their snippet with their import
  lines once each at the top of the code, per language, and `inspect.componentCode` returns `connected: {importPath, name,
  template, import, code}` with the snippet as `usage`. Example: template `<Button
  variant="primary" icon="{Has icon ? "cart" : "none"}">{label}</Button>` with import path
  `@acme/ui` gives `import { Button } from "@acme/ui";` then `<Button variant="primary" icon="none">{"Buy
  now"}</Button>`. `codeConnect.remove {id?, language?}` (no language: all). `codeConnect.export
  {path?}` → `{text, links}` (or writes the file): `{version: 1, links: [{component, language,
  importPath, name, template}]}`, sorted, meant to live in the code repository as
  `figcraft.connect.json`; `codeConnect.import {json | path}` → `{linked, warnings}` links each
  entry to every component or set with that name (one undo step; at most 5,000 links and 4 MB;
  templates up to 4,000 characters, names 100, import paths 300; invalid entries are skipped with a
  warning). `get_node` lists a component's links as `codeLinks`. In the app, the Inspect tab's
  Connect to code section edits them in the Code section's language, with a live preview and a
  Connected badge. An editor extension isn't needed: agents and editors read the same snippets
  through this server. Plugins whose manifest asks only for `read` also run from the Inspect tab,
  with their panels (`plugin.ui`); their callbacks can only read.
- **Dev resources:** links for developers (docs, code, tickets) on any layer, each a URL and a
  name. `devResource.add {id?, url, name?}` → `{id, index}` (no name: the URL without its scheme,
  `www.` and a trailing slash; no page title is looked up), `devResource.edit {id?, index, url?,
  name?}`, `devResource.remove {id?, index?}` (no index: all of the layer's own), all undoable, and
  `devResource.list {id?}` → `[{name, url, index? (its own), from? (the layer it comes from)}]`: an
  instance lists its own, then its main component's and that component's set's (read-only); a
  layer inside an instance shows its main layer's and takes none of its own. URLs must be `http://`
  or `https://` with a host, without spaces or control characters, at most 2,048 characters; names
  are clipped to 200 characters; a layer holds at most 32 links. Links are data: never fetched or
  opened by the engine (the app opens one when it is clicked). `get_node` and `inspect_document`
  list them as `devResources`; the Inspect tab lists, adds, edits and removes them.
- **Measurements kept on the canvas:** distances a developer leaves on the canvas for everyone.
  `devMode.addMeasurement {from? (default the first selected layer), to?, side?}` → `{from, index,
  lines}` places one between `from` and `to` (a layer on the same page), along one side of `from`
  (`side`: `top`, `right`, `bottom` or `left`: that side's length) or, with neither, to the layer
  around `from`; `devMode.removeMeasurement {from?, index?}` → `{removed}` (no index: every one
  starting from the layer); both undoable. `devMode.measurements {page?, id?}` → `[{from, fromName,
  index, to?, toName?, side?, labels, lines: [{x0, y0, x1, y1, label, dashed}]}]` lists those on a
  page (default the current one; `id`: only those from or to that layer), with their lines in page
  space measured from the layers' current bounds, so they follow the layers. A measurement is
  stored on the layer it starts from (`get_node` lists them as `measurements: [{to} | {side}]`),
  saved with the file and synced; the same two layers are measured once (either way round), a
  layer starts at most 64. One whose other layer was deleted or moved to another page isn't listed
  or drawn (an undo brings it back); a copy keeps the side measurements and those to layers copied
  with it. In the app, Dev Mode draws them with the hover redlines, and the Inspect tab lists them
  (with nothing selected, every one on the page), selects a measurement's layer on a click, removes
  one with −, and places new ones from its Add measurement menu.
- **Annotations:** notes for developers on any layer, each with optional pinned properties that
  Dev Mode shows with their current values (and the style or variable behind them).
  `annotation.add {id?, label?, properties?}` (a note, pinned properties or both; returns `{id,
  index}`), `annotation.edit {id?, index, label?, properties?}` (`properties` replaces the list),
  `annotation.remove {id?, index?}` (no index: all of the layer's), all undoable, and
  `annotation.list {ids? | id? | page? ("all")}` → `{annotations: [{id, name, index, label,
  properties: [{property, label, value, token?, color?}]}]}` (`value` is null when the layer doesn't
  have that property). Pinnable ids: `width`, `height`, `top`, `left`, `rotation`, `opacity`,
  `cornerRadius`, `direction`, `gap`, `padding`, `alignment`, `fontFamily`, `fontStyle`,
  `fontSize`, `lineHeight`, `letterSpacing`, `textAlign`, `fill`, `stroke`, `strokeWeight`,
  `effects`. A layer holds at most 64 annotations; notes are capped at 5,000 characters. They are
  saved with the file (`get_node` lists them as `annotations`); instances don't inherit their
  main's annotations and keep their own across main edits. In the app the Inspect tab draws them
  as callouts beside their layers and edits them in its Annotations section.
- **Annotation categories:** every annotation can have a category, drawn as the colour of its
  callout: the defaults are Development, Interaction, Accessibility and Content, and a file can add
  its own. `annotation.add` / `annotation.edit` take `category` (an id or name; `null` clears it)
  and `annotation.list` returns `category: {id, name, color}` and filters with `{category: id |
  name | "none"}`. `annotation.categories {}` → `{categories: [{id, name, color, count}]}`;
  undoable `annotation.addCategory {name, color?}` → `{id}` (a new one takes the first unused
  colour), `annotation.editCategory {id, name?, color?}` (rename, recolour; ids stay) and
  `annotation.removeCategory {id}` (its annotations stay, uncategorised). Names are unique (ignoring
  case), up to 64 characters; at most 64 categories. Files keep the list only once it differs from
  the defaults (old files open with the defaults). In the app, with nothing selected, the Inspect
  tab filters the callouts by category and edits the categories (colour, name, delete, add); each
  note has a category menu.
- **Ready for dev:** frames at the top of a page or section, and sections, carry a handoff status
  (`node.set {devStatus: none|readyForDev|completed}`; other layers are refused a status).
  `inspect.readyForDev {status?: readyForDev|changed|completed|any, page?}` lists them across pages
  (or on one page) in page and layer order: `{items: [{id, name, page, status, changed}]}`
  (`changed`: ready for dev and edited since). The Inspect tab sets the status of the selected
  frames and, with nothing selected, lists them with a status filter (Ready for dev, Changed,
  Completed, Any status; changed ones carry a Changed badge; click one to go to it, or its focus
  button to open it in the focus view). In the app's Dev Mode the canvas only inspects: presses
  select (and pan, and comment), never move, resize, draw or edit text and vectors, and the menus
  and shortcuts refuse commands that edit the design (Dev Mode's own annotations, statuses, dev
  resources, measurements, code connections and comments still work); choosing a drawing tool in
  the toolbar goes back to the design, and the layers panel only selects and expands. Commands sent over MCP or the control channel aren't affected.
- **Compare changes:** when a layer marked Ready for dev first changes (it turns "Changed"), the
  file keeps a copy of it and everything in it as it was just before that edit (saved in the
  `.figcraft` file only while there is a change; resolving the change, marking it again, another
  status or deleting it drops the copy, so the next change compares with that state).
  `dev.compare {id?, version?, show?}` → `{id, name, changed, snapshot, added, removed, modified,
  truncated, version: {id, name, time} | null, changes: [{id, name, type, change: added|removed|changed, depth, layers, properties?: [{section,
  property, before, after, text}]}]}`: layers matched by id, added and removed ones listed as a
  whole (`layers` counts what's inside), changed ones with their properties formatted like Inspect
  (`width 120 → 136`, `fill #FF0000 → #E03030`, `text “Welcome” → “Hello”`, `parent Badge → Screen`,
  `order 3 → 1`, `component Card → Card 2`, auto layout, typography, strokes, effects, corner radius,
  opacity, visibility, name), nested layers and instance contents included. `snapshot: false` means
  the frame changed before copies were kept (an older file). `show: true` opens the compare view in
  the app: before and now side by side (or overlaid with a slider), changed layers outlined (added
  green, changed orange, removed red), the list of changes and Resolve change. With `version` (an id
  from `version.list`) any top-level frame or section, ready for dev or not, is compared with that
  saved version instead (a layer the version doesn't have was added as a whole); the compare view
  picks it at the bottom (Compare with: When marked ready, or a version; a frame that isn't ready
  for dev compares with the newest version), and `ui.set {devCompareVersion}` sets it.
- **Focus view:** `dev.focus {id? | direction: next|previous}` → `{id, name, page, index, count}`
  shows one top-level frame or section alone on the canvas (fitted, selected, other layers neither
  drawn nor hit), switching page when needed; `direction` steps through the ready-for-dev frames
  across pages, wrapping around. `dev.unfocus {}` ends it, as do Escape (once the frame or nothing
  is selected), leaving the page or deleting the frame. The app shows a bar with the frame's name
  and status, Compare changes, previous / next and exit; `inspect_ui` reports `devFocus`.

### Effects

`component.playground {id?, properties? {name: value}, language?, path?, scale?}` tries a component
with other variant, boolean, text (and instance-swap) property values on a copy and returns its
properties with their values, its code and, with `path`, a PNG of it. The document doesn't change.

`style.bind {style, field, variable?}` binds a variable inside a shared style (no `variable`:
unbinds): `fill` or `fills/<i>/color` and `fills/<i>/stops/<j>/color` on a paint style,
`effects/<i>/<color|radius|spread|offsetX|offsetY|startRadius>` on an effect style,
`layoutGrids/<i>/<size|count|gutter|offset>` on a grid style, `fontSize`, `lineHeight`,
`letterSpacing`, `fontFamily`, `fontStyle` on a text style. Each layer using the style shows the
field in its own modes and stays linked; `style.list` gives a style's `bound` map.

`effect.add {ids?, type? | effect?}` adds an effect (`dropShadow`, `innerShadow`, `layerBlur`,
`backgroundBlur`, `noise`, `texture`, `glass`) with its defaults, or the full `effect` object;
`effect.set {ids?, index, effect}` replaces one. A layer paints its drop shadows first, then its
fills, inner shadows, children and strokes (strokes draw over inner shadows), with layer blur,
noise and texture over the result. A group has no outline of its own: its inner shadows are cast
from around what its layers draw together, over them, and its background blur and glass show
through the outlines of its layers.

- **Blurs** (`layerBlur`, `backgroundBlur`) are uniform (`{"radius": 8}`) or progressive:
  `{"type": "layerBlur", "blurType": "progressive", "startRadius": 0, "radius": 24, "startOffset":
  [0.5, 0], "endOffset": [0.5, 1]}` ramps the radius linearly from `startRadius` at `startOffset` to
  `radius` at `endOffset`, points in the layer's unit box (0–1 across and down, like a gradient's
  handles; before the start and past the end the radius stays). Radii above 10,000 draw as
  10,000. In the app, the Design panel's blur row picks Uniform or Progressive, and while a
  progressive blur's settings are open its start and end points are handles on the canvas.
  Generated CSS leaves progressive blurs out (CSS blurs are uniform); Inspect lists the ramp.
- **Exports:** PNG, PDF and SVG draw the same pixels as the canvas. PDF and SVG keep layers vector
  and embed images for what vectors can't show: background blur and glass (from what lies under
  the layer: on the page in PDF, among the exported layers in SVG), layer blur, noise and texture
  in PDF; noise grains (an image over the vector layer), texture and progressive layer blur (the
  layer whole) in SVG, where uniform layer blur and shadows stay SVG filters.
- **PDF for print:** a bleed grows each page by that much on every side and shows what lies past
  the frame there (the frame's fills reach out to the bleed edge with gradients and images in
  place, its children are clipped to the bleed instead of the frame, nothing is drawn past it);
  a page of one frame is trimmed to the frame's box (its shadows go into the bleed). Crop marks
  (at the trim's corners) and bleed marks (at the bleed's) are 0.25 pt hairlines 18 pt long,
  starting 3 pt outside the bleed, in registration colour. MediaBox is the sheet, BleedBox the
  trim grown by the bleed, TrimBox the frame. CMYK converts every colour (fills, strokes, text,
  gradient stops, images and rasterised effects) from sRGB with `K = 1 − max(R, G, B)`,
  `C = (1 − R − K) / (1 − K)` (M and Y alike), deterministic and profile-free: the file is
  DeviceCMYK without an output intent, because embedding one needs a CMYK ICC profile and none
  ships with FigCraft. Soft masks stay RGB (they aren't printed) and transparency groups blend in
  RGB, as the PDF writer declares them.

### Brushes and pen pressure

- `stroke.brush {ids?, preset, spacing?, sizeJitter?, rotationJitter?, scatter?, seed?}` draws a
  layer's stroke with a brush (`preset: null` back to a plain stroke; a layer without a stroke gets
  a black one); `node.set {brush}` takes the same (a preset id, an object or `null`) and `get_node`
  shows `brush`. Stretch brushes bend one shape along each subpath: `taper`, `ink`, `ribbon`,
  `bristle`. Scatter brushes drop copies along it every `spacing` stroke weights (default 1.5; at
  most 5,000 copies a path): `dots`, `stars`, `confetti`, `leaves`, varied by `sizeJitter` (0–1),
  `rotationJitter` (0–1, up to half a turn) and `scatter` (0–4 weights off the path), repeatably
  from `seed`. The stroke weight sets the brush's size and a width profile scales it along the
  path. Brushes draw on the canvas, in PNG, SVG and PDF (as filled outlines) and `vector.outlineStroke`
  flattens them; dashes don't apply to brushes.
- **Pen pressure**: the Pencil records pressure into the stroke's width profile (up to 32 points by
  length; medium pressure, 0.5, draws the stroke weight, from a tenth of it to twice it); without
  pressure (a mouse) the stroke stays uniform. `pointer` takes `pressure` and, for drags,
  `pressureTo`. `tool.pencilBrush {preset?, …}` sets the brush new Pencil strokes get (`null`
  clears it; no `preset` reads it). The desktop app reads pressure from pen input on Windows; in the
  browser from stylus touch events that report a force; macOS and Linux tablets report none.

### Selection colors

`selection.colors {ids?}` returns `{colors: [{paint, style, variable, targets}]}` across the
selection and its descendants, deduplicating overlapping roots. Only visible solid and gradient
paints are listed; masks and boolean operands are omitted. Styles and variables distinguish
otherwise equal paints. Each target is `{id, fills, index, range}`; `range` is null for a node
paint or `[start, end]` in character offsets for an effective text-range fill.

Pass the returned target list to:

- `selection.setPaint {targets, paint}` to replace only those slots (same paint syntax as `fill.set`).
- `selection.selectColor {targets}` to select their distinct owning layers, without a document undo step.
- `selection.applyColorStyle {targets, style}` to apply a document paint style. As with `style.apply`,
  this replaces the matching node's entire fill/stroke list and links the style. Text ranges copy
  resolved paints and keep their typography, without a paint-style link.
- `selection.bindColorVariable {targets, variable}` to bind a document color variable (id or name)
  to those solid fill and stroke slots, as `variable.bind` does with `fills/<i>/color` or
  `strokes/<i>/color`. A color style on the list and an earlier binding of the slot are removed.
  Text ranges with fills of their own take the variable's value in the layer's modes, without a
  binding. Gradients and other variable types are refused; scopes only filter what the Selection
  colors menu offers, as for `variable.bind`.
- `selection.detachColor {targets}` to unlink the color style and the color variable of those
  slots, keeping their values.

Keep the targets fixed throughout a live interaction. A color becoming equal to another color
must not add slots to the gesture. Invalid or missing targets fail atomically. Direct paint edits
detach bindings on changed paint values (opacity alone keeps the color binding); a text-range edit preserves the node bindings other
characters still inherit. Each mutating command is one undo step (an interaction can group them).

### Paint, effect and layout grid lists

A layer's fills, strokes, effects and layout grids are lists kept **bottom first**: index 0 paints
first, the last one is on top (the Design panel lists them the other way round, topmost first).

- **Reorder:** `fill.move`, `stroke.move`, `effect.move` and `grid.move {ids?, from, to}` move one
  item (one undo step for every layer given; out-of-range or equal indices change nothing, negative
  or fractional ones are refused). The item keeps everything (an image fill its image, scale mode
  and adjustments, a gradient its stops and handles), and a paint, effect or grid style linked to
  the list is unlinked (the list no longer matches it). In the app, drag a row by its grip (the
  empty strip left of its fields); a line shows where it lands.
- **Variable bindings follow their items** when a list is reordered or an item removed
  (`fill.remove`, `stroke.remove`, `effect.remove`, `grid.remove`); a removed item's bindings go
  with it. Binding fields (`variable.bind {field}`): `fill` / `stroke` (the first paint's colour) or
  `fills/<i>/color` / `strokes/<i>/color`, `effects/<i>/<offsetX|offsetY|radius|startRadius|spread|color>`,
  `layoutGrids/<i>/<size|count|gutter|offset>` (number variables; count is kept to 1–1000, size and
  gutter are never negative). Moving a bound paint rewrites its key (`fill` becomes `fills/2/color`).
- **Instance properties:** `variable.bind {ids: [instance], field: "componentProperties/<name>",
  variable}` drives a boolean property from a boolean variable and a text property from a string
  variable (other types are refused); the value follows the variable's modes like any binding, the
  binding follows renames of the property, and deleting the property, resetting the instance or
  detaching it drops it. The instance's properties panel offers the same with its variable button.

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"fill.move","params":{"ids":[2],"from":0,"to":2}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"variable.bind","params":{"ids":[5],"field":"layoutGrids/0/gutter","variable":"gutter"}}}}
```

### Ruler guides

`guide.add {vertical, offset, frame? | page?}` (returns `{index}`), `guide.move {index, offset, …}`,
`guide.remove {index, …}` and `guide.clear {…}` edit the guides of the current page, another
`page`, or a top-level `frame` (frames, components and instances on the page or in a section;
nested frames are refused). A frame's guides are in its own coordinates: they move with the frame,
span only it, are copied with it and snap the layers inside it. A frame moved into another frame
keeps its guides, but they stop showing, snapping and measuring until it's top-level again. On a
rotated frame they run along its own axes, and the layers inside it snap to them (and to their
siblings and the frame's edges) along those axes, keeping whole pixels in the frame's coordinates.

`guide.select {index, frame? | page?}` selects a guide (and deselects the layers; `{}` deselects
it), as clicking one does in the app; `guide.remove {}` without an index removes the selected one
(the context menu's Remove guide), and removing another guide keeps the selected one selected.
While a guide is selected and no layer is, Delete removes it,
the arrows across it nudge it (Shift: the big nudge) and Escape deselects it. In the app, a guide
dragged out of a ruler and dropped on a frame becomes the guide of the top-level frame there, along
the frame's axis nearer the ruler's; Alt-dragging a guide drops a copy, and holding Alt while
dragging one shows its distances to the nearest edges inside the frame it crosses (the frame
selected before the drag, else its own).

Layout grids: a columns or rows grid with a fixed width (left, centre, right) and a `count` of 0 is
Auto, as many as fit (a stretched grid shows at least one); grids don't show or snap on a frame
turned off 0°.

### Measuring, the Scale tool, arc handles and selecting

- **Measure** (what Alt/Option-hover shows): `measure.distances {ids? (default the selection), to?
  (a layer; default the selection's parent)}` returns `{lines: [{x0, y0, x1, y1, label, dashed}]}`
  in page coordinates, between bounding boxes (several layers by their union, rotated ones by
  their bounds): one box inside the other measures to each edge, boxes apart measure the gaps
  (`dashed` lines extend a gap line to a box it passes beside), overlapping boxes measure between
  matching edges. `measure.padding {id? (an auto layout frame or a child)}` returns `{frame, lines}`:
  the frame's padding and the gaps between neighbouring children (what Shift+Alt-hover shows).
  With `pointer`, `alt: true` hovers measure (add `cmd` for nested layers, `shift` for padding) and
  `alt` drags show live distances to the neighbours; `inspect_ui` reports them under
  `tool.overlay` (`guides`, and `measure_box` for the box measured to).
- **Duplicating at a spacing**: a copy Alt-dragged off a layer with `pointer` snaps to that layer
  too (its edges, centres and equal spacing). `edit.duplicate {ids?, dx?, dy?}` without `dx`/`dy`
  repeats the last duplicate's page offset while its copies are what it duplicates (and still the
  selection), measured when it runs: Alt-drag or duplicate and move a copy, then each
  `edit.duplicate` steps out the next one as far again. Copies placed by an auto layout set no
  step; explicit `dx`/`dy` always win.
- **Scale tool** (`tool.scale`, K): dragging a selection handle with `pointer` scales everything by
  one factor (sizes and positions whatever the constraints, strokes, effects, corner radii, auto
  layout spacing, min/max sizes, layout grids, guides, tiled images, text), one undo step per drag,
  about the anchor point `tool.scaleAnchor {anchor?: tl|t|tr|l|c|r|bl|b|br}` (or, when the anchor
  lies on the side being dragged, the opposite handle; `alt: true` scales from the centre), with the
  factor and pivot in `tool.overlay.scale` while dragging. Without
  dragging, `transform.scale {ids?, factor, cx?, cy?}` does the same numerically.
- **Arc handles**: with the Move tool, a single selected ellipse (at least 40 px on screen, unlocked)
  shows its arc handles, listed by `inspect_ui` under `tool.arcHandles` (`{id, sweep: [x, y],
  start, ratio}` in page coordinates; `start` once there is an arc, `ratio` once there is an arc or
  a ring). Drag them with `pointer`. **Sweep** starts inside the right edge. Dragged round the
  ellipse, it opens an arc: positive up from the right edge, negative down. It closes again back at
  the start. **Start** turns the arc and keeps its sweep. **Ratio** (the centre, or the inner edge
  halfway round a ring) drags out to make a ring, and into the gap to show the other part.
  `shift: true` snaps angles to 15° steps and the ratio to 5% steps. Each drag is one undo step
  and writes `node.set`'s `arcStart` / `arcSweep` (degrees) and `arcInner` (0–0.99). The Design
  panel's Arc fields (Start °, Sweep %, Ratio %) write the same keys; number variables bound to
  `arcSweep` and `arcInner` (`variable.bind`) hold percentages, as the fields show them, and ones
  bound to `arcStart` degrees.
  `tool.overlay.arc` holds the readout while dragging (`{handle, label: "Sweep 75%", at}`), and
  `tool.overlay.arc_hover` holds the handle under the pointer.
- **Corner radius and star ratio handles**: a selected rectangle or frame (at least 48 px on screen)
  has a handle inside each corner, a polygon or star one inside its top point, and a star a ratio
  handle on its first inner point; `inspect_ui` lists them under `tool.shapeHandles`
  (`{handle: {kind: radius, corner} | {kind: ratio}, at}`). Dragging a radius handle along its
  corner's bisector sets `cornerRadius` (stopping where the corners meet); `alt: true` sets only that
  corner of a rectangle or frame (`topLeftRadius`…). Dragging the ratio handle sets `innerRadius`.
  One undo step per drag; `tool.overlay.shape` holds the readout (`{handle, value, at}`).
  `node.set {pointCount}` keeps stars to 3–60 points; `miterAngle` (degrees, the Design panel's
  Miter angle) sets the miter limit (`limit = 1 / sin(angle / 2)`); `get_node` reports a join
  other than miter (`strokeJoin`) or a miter angle other than the default ~28.96° (`miterAngle`).
- **Select**: `select.layersAt {x, y}` lists every layer under a page point, front-most first with its
  depth (the context menu's Select layer); `select.set`, `select.children` / `select.parent` (Enter
  / Shift+Enter), `select.next` / `select.previous` (Tab / Shift+Tab). With `pointer`, `cmd: true`
  clicks select the nested layer, `cmd` drags from empty canvas select the nested layers they touch,
  and `shift` drags toggle what they touch. A press on the background of a top-level frame with
  layers in it (the frame not selected) starts a marquee inside it, as on the canvas; an empty frame
  is picked and dragged. Locked layers (selectable with `select.set`) get no handles and stay put
  when the rest of the selection is dragged. `select.all` / `select.inverse` skip hidden and locked
  layers.
- **Transform gestures**: with `pointer`, `shift` keeps proportions, `alt` resizes from the centre
  (both together too), dragging past the opposite edge flips (several layers mirror and flip);
  groups and several layers scale each layer along its own axes, keeping its rotation and flip;
  `cmd` while resizing a frame ignores its children's constraints (they stay where they are) and,
  as in every drag, turns snapping off. Moves snap to edges, centres and equal spacing (the gaps
  show in `tool.overlay.guides` with labels); with the pixel grid on, moves and resizes land on
  whole pixels.

Tool failures come back as results with `isError: true` and a message; protocol problems (unknown
tool, malformed JSON, wrong parameter types) are JSON-RPC errors. Nothing crashes the server.
A name a tool or command doesn't take (`run_command {args}` for `params`, a step key other than
`command`, `params` and `as`, or a parameter its `list_commands` entry doesn't list, such as
`comment.list {filterr}`) is an error that lists the names it does take, never ignored; `ids` and
`id` are taken by every command.

## Resources

| URI | Content |
|---|---|
| `figcraft://document` | The active document (like `inspect_document` at depth 8). |
| `figcraft://commands` | The command catalog. |
| `figcraft://comments` | Every open comment thread on every page, as JSON. |
| `figcraft://node/{id}` (template) | One layer with children to depth 2. |
| `figcraft://command/{id}` (template) | One command's docs, e.g. `figcraft://command/layout.add`. |

## Prompts

| Prompt | Arguments |
|---|---|
| `mobile_screen` | `screen` (required), `device` (default "iPhone 16"), `brand_color` |
| `design_system` | `name`, `primary` |
| `landing_page` | `product` (required), `width` (default 1440), `accent` |
| `prototype_flow` | `screens` (comma-separated), `transition` |
| `fix_comments` | `page` (default "all"), `auto_resolve` (default "yes"), `author` (default "AI") |
| `watch_comments` | `minutes` (default 5) |

Each returns a step-by-step plan naming the exact tools and commands to use and how to verify.

## Examples

Create a card with auto layout and check it:

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_node","arguments":{
  "type":"frame","name":"Card","x":100,"y":100,"width":320,
  "props":{"fill":"#FFFFFF","cornerRadius":16,"layout":"vertical","padding":24,"gap":12,"sizingV":"hug",
           "effects":[{"type":"dropShadow","color":"#00000026","offset":[0,4],"radius":16}]}}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_node","arguments":{
  "type":"text","parent":2,"text":"Ada Lovelace","props":{"fontSize":24,"fontStyle":"Bold"}}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_node","arguments":{"id":2,"depth":2}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"screenshot","arguments":{"ids":[2]}}}
```

A component with an instance whose text is overridden (ids are the ones earlier calls returned; read
the instance's text layer id with `get_node`):

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"component.create","params":{"ids":[2]}}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"instance.create","params":{"component":2,"x":500,"y":100}}}}
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"type_text","arguments":{"id":7,"text":"Grace Hopper"}}}
```

Smart-selection spacing (movable siblings in rows, columns or aligned grids):

```json
{"jsonrpc":"2.0","id":"spacing","method":"tools/call","params":{"name":"run_command","arguments":{"command":"arrange.spacing","params":{"ids":[3,4,5],"horizontal":24}}}}
```

`horizontal` and `vertical` are optional page-unit gaps (at least one is required, magnitude at
most 1,000,000). The first column/row stays fixed; subsequent tracks move without changing sizes,
parents or offsets inside each track. Nonuniform gaps become uniform. Normal auto-layout flow
children, locked/hidden layers and instance internals are excluded. Interactive callers may pass
`fromGesture: true` inside an interaction to keep using its original tracks during a scrub.

Align and distribute as one undo step:

```json
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"batch","arguments":{"label":"Tidy","commands":[
  {"command":"align.top","params":{"ids":[3,4,5]}},
  {"command":"distribute.horizontal","params":{"ids":[3,4,5]}}]}}}
```

By default (`"onError": "abort"`) the first failure rolls back the whole batch, and success
returns the raw results in order. With `"onError": "continue"` next to `commands`, a failed step's
document edits are rolled back on their own, the later steps still run, and the result is instead
one object per step:

```json
[
  {"step": 0, "command": "text.set", "ok": true, "result": {"ok": true}},
  {"step": 1, "command": "text.set", "ok": false, "error": "invalid parameters for `text.set`: step 1: node 2 is not text"},
  {"step": 2, "command": "page.rename", "ok": true, "result": {"ok": true}}
]
```

The tool call is not an error (`isError` is false) even when some steps fail. Successful document
edits form one undo step; if every step fails, none is added. Unknown commands, bad params and bad
references fail only their step, but a malformed `commands` list, an invalid `onError` or two steps
with the same `as` still reject the whole batch before any step runs. A step that has to wait for
background work makes the whole call wait, as without `onError`. A nested batch has its own
`onError`.

### Step references

A step in a `batch` can use what an earlier step returned. Give the step a name with `"as"` (next to
`command` and `params`), and `{"$ref": "name"}` anywhere in a later step's params (a value, an item
in `ids`, a nested field) is replaced by the `id` in that step's result before the later step runs.
This builds a nested tree in one call and one undo step, without guessing ids or counting steps:

```json
{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"batch","arguments":{"label":"Card","commands":[
  {"command":"node.create","as":"card","params":{"type":"frame","name":"Card","props":{"layout":"vertical","gap":12}}},
  {"command":"node.create","params":{"type":"text","parent":{"$ref":"card"},"text":"Ada Lovelace"}},
  {"command":"node.create","as":"button","params":{"type":"frame","parent":{"$ref":"card"},"name":"Button"}},
  {"command":"node.create","params":{"type":"text","parent":{"$ref":"button"},"text":"Follow"}}]}}}
```

`{"$step": n}` refers to a step by its zero-based position instead, and works in the same places;
the two can be mixed. Names are any non-empty string and must be unique within the batch.

`"field"` takes something other than the `id`: a key of the result (`{"$ref": "card", "field":
"name"}`) or a path into it, with `/` between the parts and list items by zero-based index, the way
`variable.bind` names `fills/0/color`. `edit.duplicate` returns `{"ids": [...]}`, so its first copy is
`ids/0`:

```json
{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"batch","arguments":{"label":"Copy","commands":[
  {"command":"edit.duplicate","as":"copy","params":{"ids":[5]}},
  {"command":"node.set","params":{"ids":[{"$ref":"copy","field":"ids/0"}],"props":{"name":"Card copy"}}}]}}}
```

To edit inside a copy, pass `"map": true`: `edit.duplicate` then also returns `map`, each original
layer's id (as a string, since JSON keys are strings), descendants included, paired with its copy's
id, e.g. `{"ids": [8], "map": {"5": 8, "6": 9, "7": 10}}`. Nested frames, groups and instances (and
their sublayers) are in it; a main component's copy is an instance of it, so its sublayers pair with
the instance's. So `"field": "map/6"` is the copy of layer 6:

```json
{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"batch","arguments":{"label":"Copy","commands":[
  {"command":"edit.duplicate","as":"copy","params":{"ids":[5],"map":true}},
  {"command":"node.set","params":{"ids":[{"$ref":"copy","field":"map/6"}],"props":{"text":"Copy title"}}}]}}}
```

Without `map` the result is `{"ids": [...]}` as before.

A path may start with `/`, as in JSON Pointer (`/ids/0`). A key that itself holds `/` is matched as
written before it is read as a path. A path ends at whatever value it reaches, so `"field": "ids"` is
the whole list.

These fail the whole batch, which is rolled back, with an error naming the step: two steps with the
same `as` (before any step runs); a `$ref` to a name no step has (the error lists the names there
are) or to the same or a later step; a `$step` to the same or a later step, or that is negative or
not an integer; a `field` the result doesn't have (the error names the full path and, for a list,
how many items it has); any key next to `$ref` or `$step` other than `field`. With `"onError":
"continue"` all but the duplicate names fail only their step, and so does a reference to a step that
failed (the error names both). Steps keep their positions, and references read the raw results. Only
an object holding
a `$ref` or `$step` key is a reference: text such as `"$ref card"` is left as written. Both keys are
reserved in step params: an object holding either anywhere (in plugin data or a variable value too)
is read as a reference. A step that is itself a `command.batch` is passed on as written, so the
references in its steps are to its own steps and names, not the outer batch's; the outer batch can
reach into its result list with a path (`{"$ref": "inner", "field": "1/id"}`).

The end-to-end tests in `crates/mcp/tests/it/agent_tasks.rs` are worked examples of realistic tasks
(auto layout, a card built in one batch with step references, components and overrides, gradients
and shadows, paint styles, Light/Dark variables, align/distribute/group, boolean subtract + flatten,
export, undo/redo, drawing with tools).
