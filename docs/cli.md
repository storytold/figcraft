# figcraft-cli

FigCraft without a window: inspect, export, convert and render documents, run command scripts, list
commands, serve MCP and measure performance. Build it with
`cargo build --release -p figcraft-cli` (binary `target/release/figcraft-cli`).

Bad arguments print usage and exit with code 2; failures exit with 1. The CLI never panics on input.

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

The collaboration server is a binary of its own, `figcraft-server` (`cargo build --release -p
figcraft-server`): it keeps files with share links and serves their live sessions, and its
`key`, `files`, `links`, `reports`, `block-ip` and `check` subcommands manage them (a server can
be open to the public: [server.md](server.md#running-a-public-server)). See [server.md](server.md).

## Subcommands

### `info <file> [--json]`

Pages, node counts, top-level frames (id, type, name, size), style/variable/image counts. Repairs
of a damaged file and import warnings go to stderr as `note:` lines (`notes` with `--json`).

Every subcommand that reads a file also takes the public REST API file JSON (the response of
`GET /v1/files/:key` or of its nodes endpoint, written to a `.json` file): it is imported on the fly.
Image fills come from `images/<imageRef>.png` (or `.jpg`, …) next to the JSON when present, else a
grey placeholder. `.fig` files are read the same way (best effort, images from the file itself);
`info` lists what wasn't imported as `note:` lines.

```text
$ figcraft-cli info app.figcraft
app.figcraft — App (412 nodes, 6 styles, 12 variables, 3 images)
page 1 "Screens": 380 nodes, 4 top-level
  2 FRAME "Home" 393.0×852.0
  …
```

### `export <file> -o <out> [--format png|jpg|svg|webp|pdf|fig] [--scale N] [--page NAME] [--node ID|NAME] [--bleed LENGTH] [--crop-marks] [--bleed-marks] [--cmyk]`

- `<out>` is a directory: one file per top-level frame of the page (default the first page), named
  after the frame (`Home.png`, `Settings.png`…); with no frames, one file per top-level layer.
- `<out>` has an image extension: one file with the given `--node`, or the whole page composited.
- `--node` takes a layer id (`12` or `0:12`) or a layer name.
- `--format fig` (or an `<out>` ending in `.fig`) writes the whole file as `.fig`, experimental:
  written from public format notes, including variables with their modes and bindings and shared
  styles with their layer links. Checked by opening it again in FigCraft and, on the example
  documents and small test files, in other software; other apps may not open every file. What was
  left out (comments, Dev Mode data, some bindings and settings…) is printed as `note:` lines;
  `docs/file-format-parity.md` lists what was checked and what is unchecked.
- The format comes from `--format`, else `<out>`'s extension, else png. `--scale` (default 1, max 64)
  applies to raster formats.
- SVG/PDF exports with visible descendants more than 256 levels below an exported root fail with
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

By extension. Inputs: `.figcraft`, `.json` (FigCraft or REST API file JSON), `.fig` (best effort), `.svg` (an editable frame: groups,
shapes, text, gradients, masks and effects; `info` lists what it left out as `note:` lines), `.png/.jpg/.webp/.gif/.tif/.bmp`
(an image layer). Outputs: `.figcraft`, `.json` (FigCraft JSON, pretty-printed), `.svg`,
`.png/.jpg/.webp` (the current page rendered at 1x with its background), `.pdf` (the current page's
frames, one page each), `.fig` (the whole file, experimental, as `export --format fig`).

### `render <file> -o <out.png> [--scale N] [--page NAME]`

The whole page (default the first) with its background, as png, jpg or webp. Large pages are scaled
down to fit 16384 px.

### `run <script.json|script.jsonl> [--in <file>] [--out <file>]`

Runs commands in a headless session (a new document, or `--in`), printing one JSON line per step:

```text
{"command":"frame.create","ok":true,"result":{"id":2},"step":0}
```

A failing step prints `"ok": false` with the error and exits with code 1 (nothing is saved).
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

### `commands [--json] [--filter TEXT]`

Every command with its label, shortcut, undoable flag (`*`) and parameter docs; `--json` for
machines.

### `mcp [--attach PORT]`

Serve MCP on stdio: headless by default, or attached to a running app started with
`figcraft --control PORT`. See [mcp.md](mcp.md). The headless session can host, join and publish
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
