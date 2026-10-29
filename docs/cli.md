# figcraft-cli

FigCraft without a window: inspect, export, convert and render documents, run command scripts, list
commands, serve MCP and measure performance. Build it with
`cargo build --release -p figcraft-cli` (binary `target/release/figcraft-cli`).

`figcraft-cli --help` (or `-h`, or `help`) prints usage on stdout and exits with 0, and so does
`--help` or `-h` after a subcommand (`figcraft-cli mcp --help`). Bad arguments print usage and exit
with code 2; failures exit with 1. The CLI never panics on input.

Put options before `--` to pass a filename beginning with a dash, for example
`figcraft-cli info --json -- --drawing.figcraft`. Every argument after `--` is literal,
including a filename named `--no-system-fonts`.

## Fonts

Text lays out and renders with the same fonts as the desktop app: the fonts built into FigCraft
plus the installed system fonts. The system fonts are indexed once per run, the first time text is
laid out (about 60 ms warm on a Windows machine with ~900 font files; a run without text skips it).
Every subcommand takes `--no-system-fonts` (or set `FIGCRAFT_NO_SYSTEM_FONTS=1`, which the desktop
app honours too) to use only the built-in fonts, for renders that come out the same on every
machine, e.g. in CI.

On Linux and other Unix systems except macOS, user fonts are read from
`$XDG_DATA_HOME/fonts`, or `$HOME/.local/share/fonts` when `XDG_DATA_HOME` is unset,
empty or relative. The legacy `$HOME/.fonts` directory is also scanned.

On macOS the font folders (`/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts` and the
system's downloadable fonts) are read, and so is every other font file Core Text lists as
available: fonts a font manager such as RightFont, FontBase or Typeface activates from its own
library, without copying them into a font folder. Fonts activated while the app runs are found by
`text.rescanFonts`, like newly installed ones.

On macOS, fonts activated through Adobe Fonts are read where Creative Cloud keeps them
(`~/Library/Application Support/Adobe/CoreSync/plugins/livetype`), so they're available while
they're active there. Files without an extension are read when they start like a font file.

The collaboration server is a binary of its own, `figcraft-server` (`cargo build --release -p
figcraft-server`): it keeps files with share links and serves their live sessions, and its
`key`, `files`, `links`, `reports`, `block-ip` and `check` subcommands manage them (a server can
be open to the public: [server.md](server.md#running-a-public-server)). See [server.md](server.md).

## Subcommands

### `info <file> [--json]`

Pages, node counts, top-level frames (id, type, name, size), style/variable/image counts. Repairs
of a damaged file and import warnings go to stderr as `note:` lines (`notes` with `--json`). Fonts the
file names that aren't installed are listed as `missing font` lines (`missingFonts: [{family,
layers}]` with `--json`); their text renders in a fallback font.

Every subcommand that reads a file also takes the public REST API file JSON (the response of
`GET /v1/files/:key` or of its nodes endpoint, written to a `.json` file): it is imported on the fly.
Image fills come from `images/<imageRef>.png` (or `.jpg`, …) next to the JSON when present, else a
grey placeholder. `.fig` files are read the same way (best effort, images from the file itself), and
so are `.sketch` files saved by Sketch 48 or newer;
`info` lists what wasn't imported as `note:` lines.

```text
$ figcraft-cli info app.figcraft
app.figcraft — App (412 nodes, 6 styles, 12 variables, 3 images)
page 1 "Screens": 380 nodes, 4 top-level
  2 FRAME "Home" 393.0×852.0
  …
```

### `export <file> -o <out> [--format png|jpg|svg|webp|pdf|xml|fig|penpot] [--scale N] [--page NAME] [--node ID|NAME] [--bleed LENGTH] [--crop-marks] [--bleed-marks] [--cmyk] [--vd-compat bestEffort|exact|ignore]`

- `<out>` is a directory: one file per top-level frame of the page (default the first page), named
  after the frame (`Home.png`, `Settings.png`…); with no frames, one file per top-level layer.
  The name is kept as it is, except for the characters file systems refuse (`/ \ : * ? " < > |`
  and control characters), which become `_`, and leading or trailing spaces and dots, which are
  dropped. A name is cut at 100 characters. The app's export replaces the same characters (there a
  `/` makes a folder). Two layers with the same name get `Name 2`, `Name 3`…
- `<out>` has an image extension: one file with the given `--node`, or the whole page composited.
- `--node` takes a layer id (`12` or `0:12`) or a layer name.
- `--format fig` (or an `<out>` ending in `.fig`) writes the whole file as `.fig`, experimental:
  written from public format notes, including variables with their modes and bindings and shared
  styles with their layer links. Checked by opening it again in FigCraft and, on the example
  documents and small test files, in other software; other apps may not open every file. What was
  left out (comments, Dev Mode data, some bindings and settings…) is printed as `note:` lines;
  `docs/file-format-parity.md` lists what was checked and what is unchecked.
- `--format xml` (or an `<out>` ending in `.xml`) writes an Android Vector Drawable, e.g.
  `figcraft-cli export app.figcraft --node Icon -o ic_icon.xml`. What the format can't draw is
  printed as `note:` lines; `--vd-compat` picks how it's handled: `bestEffort` (default:
  approximated), `exact` (left out) or `ignore` (no notes: overlapping layer opacity is dropped,
  even-odd fills keep `fillType="evenOdd"`, diamond gradients are left out). See
  `docs/file-format-parity.md`.
- `--format penpot` (or an `<out>` ending in `.penpot`) writes the whole file as a Penpot 2.x
  archive (`manifest.json` plus one JSON per file, page and shape) that Penpot imports into a team:
  pages, boards, groups, rectangles, ellipses, paths, booleans and text, fills, strokes, shadows and
  blurs, flex and grid layout, components and variants, design tokens and prototype interactions.
  Layers opened from a `.penpot` keep their Penpot ids, and unchanged text, layouts, interactions
  and tokens are written back from the original. What Penpot cannot represent is printed as
  `note:` lines.
- The format comes from `--format`, else `<out>`'s extension, else png. `--scale` (default 1, max 64)
  applies to raster formats.
- SVG/PDF/XML exports with visible descendants more than 256 levels below an exported root fail with
  an error instead of writing an incomplete file. Select an inner layer with `--node` or reduce
  nesting. An existing destination file is kept on this failure.
- A `.pdf` `<out>` without `--node` holds the page's top-level frames, one PDF page each in reading
  order, each the frame's size in points (a page without frames goes on one page). Shapes, text
  and gradients stay vector, text is real (selectable, with subset fonts) and its links clickable;
  shadows and blurs are images.
- PDF print options: `--bleed` (points, or a length like `3mm` or `0.125in`, up to 2 in) grows each
  page by the bleed, which shows what lies past the frame's edge (its fills reach out to it, its
  children are clipped there instead); `--crop-marks` and `--bleed-marks` draw hairline marks
  outside the bleed; the page boxes are set (MediaBox the sheet, BleedBox, TrimBox the frame).
  `--cmyk` writes CMYK colour, images included, converted from sRGB with the naive formula
  `K = 1 − max(R, G, B)`, `C = (1 − R − K) / (1 − K)` (M and Y alike): DeviceCMYK without an
  output intent (no CMYK profile ships with FigCraft), so proof against your printer's profile.

```sh
figcraft-cli export app.figcraft -o exports/ --scale 2
figcraft-cli export app.figcraft -o hero.svg --node "Hero"
figcraft-cli export app.figcraft -o screens.pdf --page "Screens"
figcraft-cli export app.figcraft -o poster.pdf --node "Poster" --bleed 3mm --crop-marks --cmyk
figcraft-cli export app.figcraft -o handoff.fig
```

### `convert <in> <out>`

By extension. Inputs: `.figcraft`, `.json` (FigCraft or REST API file JSON), `.fig` (best effort), `.sketch` (Sketch 48 or newer, best effort), `.svg` (an editable frame: groups,
shapes, text, gradients, masks and effects; `info` lists what it left out as `note:` lines), `.png/.jpg/.webp/.gif/.tif/.bmp`
(an image layer). Outputs: `.figcraft`, `.json` (FigCraft JSON, pretty-printed), `.svg`,
`.png/.jpg/.webp` (the current page rendered at 1x with its background), `.pdf` (the current page's
frames, one page each), `.fig` (the whole file, experimental, as `export --format fig`), `.penpot`
(the whole file as a Penpot 2.x archive, as `export --format penpot`).

### `render <file> -o <out.png> [--scale N] [--page NAME]`

The whole page (default the first) with its background, as png, jpg or webp. Large pages are scaled
down to fit 16384 px.

### `run <script.json|script.jsonl> [--in <file>] [--out <file>]`

Runs commands in a headless session (a new document, or `--in`), printing one JSON line per step:

```text
{"command":"frame.create","ok":true,"result":{"id":2},"step":0}
```

The whole script is **one undo step**, and it is **all-or-nothing**, exactly like running the same
commands through `command.batch` (the MCP tool `batch`, the control channel, plugins). A step key
other than `command`, `params` and `as`, or a parameter the command's doc (`figcraft-cli commands`)
doesn't list, fails the script with an error naming it (`ids` and `id` are always taken). A step named
with `"as"` can be used by later steps: `{"$ref": "name"}` in a step's params is the `id` in that
step's result (`{"$step": n}` does the same by zero-based position), and `"field"` takes another key
or a slash path such as `ids/0`. So a step can target a layer an earlier step of the same script
created ([Step references](mcp.md#step-references) has the rules):

```json
[
  {"command": "node.create", "as": "card", "params": {"type": "frame", "name": "Card", "width": 200, "height": 100}},
  {"command": "node.create", "params": {"type": "text", "parent": {"$ref": "card"}, "text": "Hi"}},
  {"command": "edit.duplicate", "as": "copy", "params": {"ids": [{"$ref": "card"}]}},
  {"command": "node.set", "params": {"ids": [{"$ref": "copy", "field": "ids/0"}], "props": {"name": "Card copy"}}}
]
```

The result lines are printed once the script is through, so none of them reports work a later step
took back.

To keep going past a failed command, make a step a `command.batch` with `"onError": "continue"`
([MCP server](mcp.md) has the rules). The script itself stays all-or-nothing: if a
later top-level step fails, that batch's edits are rolled back with everything else.

A failing step prints one line with `"ok": false`, the error and `"rolledBack": true`, writes
nothing (`--out` is not saved, and the session's documents go back to how the script found them),
and exits with code 1:

```text
{"step":1,"command":"node.set","ok":false,"error":"no such node 99999","rolledBack":true}
```

`--out` saves the result by extension like `convert`.

Script formats: a JSON array of `{"command": id, "params": {...}}`, an object
`{"commands": [...]}`, or JSON lines (one step per line; blank lines and `//` lines are skipped).

```json
[
  {"command": "frame.create", "params": {"preset": "iPhone 16", "x": 0, "y": 0, "name": "Home"}},
  {"command": "node.create", "params": {"type": "text", "parent": 2, "x": 24, "y": 64, "text": "Welcome", "props": {"fontSize": 32}}},
  {"command": "layout.add", "params": {"ids": [2], "direction": "vertical"}}
]
```

Ids are assigned in order (a new document's first page is 1, so the first layer is 2), which lets a
script refer to layers it created.

A script can start from a starter template instead of an empty document: `template.list` names
them and `file.newFromTemplate` opens one as the document the following steps (and `--out`) act on.

```sh
echo '[{"command": "file.newFromTemplate", "params": {"template": "dashboard"}}]' > start.json
figcraft-cli run start.json --out dashboard.figcraft
figcraft-cli export dashboard.figcraft -o dashboard.png
```

### Confining a run: `--automation-read-root DIR` / `--automation-write-root DIR`

`info`, `export`, `convert`, `render` and `run` take these two flags, with the same meaning they
have on [`mcp`](#mcp---attach-port---automation-read-root-dir---automation-write-root-dir): each is
repeatable, each folder must exist, and together they confine every file that subcommand touches.
Reads are allowed inside a read or write root, writes only inside a write root, and everything else
is refused — so a run given only a read root cannot write at all. Without either flag a subcommand
reads and writes as the user who started it.

```sh
# Renders in/doc.figcraft to out/ok.png and can touch nothing else.
figcraft-cli render in/doc.figcraft -o out/ok.png \
  --automation-read-root in --automation-write-root out

figcraft-cli render in/doc.figcraft -o ./escaped.png \
  --automation-read-root in --automation-write-root out
# error: automation path rejected: . is outside the folders that may be written: …/out
```

This is for driving the CLI from a script or an agent that should not reach the rest of the disk;
the flags leave the output identical when nothing strays outside the roots.

### `commands [--json] [--filter TEXT]`

Every command with its label, shortcut, undoable flag (`*`) and parameter docs; `--json` for
machines.

### `mcp [--attach PORT] [--automation-read-root DIR]… [--automation-write-root DIR]…`

Serve MCP on stdio: headless by default, or attached to a running app started with
`figcraft --control PORT`. See [mcp.md](mcp.md). The automation roots (each repeatable; existing
folders) confine the files the agent's calls read (inside a read or write root) and write (inside a
write root); everything else is refused ([mcp.md › Confining file access](mcp.md#confining-file-access)).
They apply to the headless session only, and that is why `--attach` refuses them: an attached server
is confined by the app it attaches to, from the flags that app was started with. The headless
session can host, join and publish
live sessions (`collab.*`: without a key on a server open to the public, with `collab.serverInfo`,
`collab.manageFile` for the owner token and `collab.report`) and use plugin directories
(`plugin.directory.*`); the one-shot `run` can't (no network link).

### `plugin publish <folder> --server ws://host:port/[?key=KEY]`

Publishes a plugin folder (`figcraft-plugin.json`, the module its `main` names, an optional `assets/`
folder) to a `figcraft-server`'s plugin directory as a new version, and prints its name, version,
size and SHA-256. The server's key comes from the address or, better, from the
`FIGCRAFT_SERVER_KEY` environment variable (it then stays out of the shell history and process
lists); it is never printed. The bundle is checked before it is sent and again by the server. See
[plugins.md](plugins.md#plugin-directories).

### `perf [--layers N[,N...]] [--instances N[,N...]]`

Builds synthetic documents of each size (default `1000,10000` layers: screens with an auto layout
header, a card group, button instances and a grid of shapes and text) and prints a Markdown table of
the hot paths: build + normalise, save/load, 1440×900 renders at 100% and zoomed to fit, hit testing
and hover, single-layer edits through the engine (fill, move, text in auto layout, a main component
with all its instances), undo/redo, a drag preview frame, `normalize` after a one-node edit,
`document.inspect` and select-all + move. Use a release build for meaningful numbers;
`cargo xtask bench` runs it at 1k/10k/50k layers together with the UI frame benchmarks.

`--layers` picks the document sizes (each value is clamped to 10 to 2,000,000 layers). `--instances`
picks the component workloads (default `2000`): for each count the command builds a synthetic design
system (about 300 components, variables and styles, nested four deep) with that many instances that
carry overrides at every level, in one file and as a library in another. It then prints a second
table with the build time and edits of a deep main component (icon colour, glyph size, an added
layer) and of the 72 button mains, where all copies follow. Zeros are dropped and counts above
100,000 are capped to 100,000; `--instances 0` skips this table.
