# FigCraft plugins

A FigCraft plugin is a WebAssembly module with a small JSON manifest. It acts on the document only
through FigCraft's commands, the same ones the menus, the CLI, the control channel and the MCP
server run (`figcraft-cli commands` lists them), so a plugin can do what a script can do, within the
permissions its manifest asks for, and nothing more.

- **Sandboxed.** Plugins run in [wasmi](https://github.com/wasmi-labs/wasmi), a pure-Rust
  WebAssembly interpreter (no JIT), in the desktop app and in the browser build alike. There is no
  WASI: no file system, network, clock or randomness. A run has an instruction budget (fuel) and a
  memory cap, so an endless loop or a runaway allocation fails that run, not the app.
- **One undo step.** Everything a run does is one step in the undo history, named after the plugin;
  so is everything one callback from its panel does. When a run or a callback fails (the plugin
  returns an error, traps, panics or runs out of fuel or memory), the document, selection and page
  are put back as they were.
- **Panels and settings (API 2).** A plugin can show a small panel of its own while it runs: FigCraft
  draws its fields and buttons with its own widgets and calls the plugin back when one is used. A
  plugin can also keep settings from one run to the next.
- **Widgets (API 3).** A plugin can put a layer of its own on the canvas: it describes the layer
  (frames with auto layout, text, shapes, images), FigCraft builds it, and a click on one of its
  parts calls the plugin back to redraw it from its state, which is kept in the document. Since
  API 4 a widget can also have editable text fields and a small property menu, since API 5 it
  can open its plugin's panel, whose buttons and fields redraw it, and since API 6 it can draw
  lines and SVG pictures.
- **Permissions.** The manifest says whether the plugin only reads the document or may edit it, and
  can narrow the commands it may call. FigCraft enforces both.
- **Distribution.** A team's own `figcraft-server` can keep a plugin directory: plugins are
  published to it with the server's key, browsed and installed from Plugins ▸ Manage plugins, and
  updated when a newer version is published (see [Plugin directories](#plugin-directories)). No
  third-party service is involved.

## Logging limits

Logs have separate limits from the plugin's WebAssembly memory and general message limits. By
default, each run or panel/widget callback may log at most:

| Limit | Default |
|---|---|
| UTF-8 bytes per log line | 8 KiB |
| Total UTF-8 log bytes | 1 MiB |
| Log lines (including empty lines) | 1,000 |

The sandbox checks these limits before copying a line from plugin memory or forwarding it to the
application logger. Accepted lines are kept in the run's `logs` result and forwarded once; the
same byte and count budgets apply to both. The byte budget counts the strings' contents, excluding
their container overhead. The general message limit also applies to log lines.

Exceeding a log limit fails the run with a limit error and rolls back its document edits, selection
and page, just like other sandbox failures. Earlier accepted lines may already have reached the
application log. Each callback starts with fresh budgets. Keep log messages short and summarize
large data instead of logging it in full.

## Using plugins

- **Plugins ▸ Development ▸ Import plugin from manifest…** picks a `figcraft-plugin.json`. The
  plugin is kept in the preferences and its module is read again on every run, so a rebuilt
  development plugin runs its new code without importing it again.
- The desktop app saves the preferences the moment a plugin is imported, installed, updated or
  removed, or changes its settings (a temporary file renamed over the old one), so a crash or a
  killed app never loses them; it doesn't wait until it quits.
- The installed plugins head the **Plugins** menu: a plugin with one command is an item, one with
  several a submenu. A command with parameters (an ellipsis after its name) first opens a small form
  with a field per parameter, drawn with FigCraft's own widgets; **Run** runs it.
- **Plugins ▸ Run last plugin** (⌥⌘P on macOS, Ctrl+Alt+Shift+P on Windows and Linux, where
  Ctrl+Alt+P is Pixel preview) runs the last plugin command again with the same parameters.
- **Plugins ▸ Manage plugins…** has two tabs. **Installed** lists the installed plugins (version,
  access, where each comes from: its manifest path or the server it was installed from) with a
  Remove button for each, and an Import button. **Browse** shows a server's plugin directory, with
  Install, Update and Remove (see [Plugin directories](#plugin-directories)).
- **Plugins ▸ Widgets** lists the installed widget plugins; picking one puts its widget in the
  middle of the view.

A message the plugin shows (`notify`) appears as a toast; so does a text result.

**Plugin panels.** A plugin written for API 2 can show a panel while it runs: a floating window,
first placed beside the properties panel (drag its title bar to move it), with the plugin's notes,
pictures, fields (text, number, slider, checkbox, menu, list, colour) and buttons, drawn with FigCraft's own widgets
and named for screen readers. A colour field's swatch opens FigCraft's colour picker (the plugin hears
of the colour when a drag in the picker ends); its hex can also be typed. Pressing a button or
changing a field calls the plugin back with every field's value; what that callback does is one
undo step (none when it changes nothing), and the plugin can update the panel or close it. The
module stays loaded while its panel is open, so the plugin keeps its state between callbacks.
Closing the window ends the plugin's session. Each plugin has at most one panel, and the panels of
different plugins open side by side (leftwards from the first, three to a row);
running a plugin again replaces its own panel. An open panel costs nothing while nobody uses it (no
frames are drawn).

**Widgets.** A widget is a frame on the canvas that belongs to its plugin. It moves, copies,
deletes, exports and undoes like any layer, as one layer: the layers inside it are locked. Clicking
one of its parts that the plugin made clickable (with the Move tool; the pointer turns into a hand
over them) calls the plugin back, which redraws the widget; that click is one undo step. Dragging
moves the widget instead, and Shift- or Cmd-clicks only select. Each widget keeps its plugin's id
and its state (a small JSON object) in its own layer data, so a saved file, a copy, an undo step or
a live session carries the widget as it is, and a copy is a widget of its own. A widget whose
plugin isn't installed stays as it was drawn; clicking it says the plugin is missing.

**Panels from widgets (API 5).** As a Figma widget's click handler can show a plugin window, a
widget's callback (a click, a field edit or a menu item) can show its plugin's panel with `ui_show`.
The module then stays loaded while the panel is open, like any panel's, and each of the panel's
callbacks gets the widget too (`widget`: its id and current state, no `event` of its own), so the
plugin can redraw the widget with `widget_render`: the callback and the redraw are one undo step. A
widget removed meanwhile isn't redrawn (the callback fails and rolls back; the panel stays open), and
`ui_close` from either side closes the panel. A plugin written for an older API that calls `ui_show`
from a widget still opens nothing, as before. In a presented prototype, where widgets react on the
prototype's own copy, no panel opens.

**Fields and menus (API 4).** A text layer of a widget can be an editable field: double-click it
with the Move tool and it is edited like any text (all of it selected first). When editing ends
(Escape, or a click elsewhere), the typing is put back and the new value goes to the widget
instead: it is cleaned (line breaks kept, tabs become spaces, other control characters dropped),
refused when longer than 1,000 characters, written into the widget's state under the field's
name and into the field's text, and the plugin is called back to redraw, all as one undo step
(none when the text didn't change). The value lives in the state, so it syncs and saves like the
rest of it. A widget can also carry a property menu: while it is the only layer selected, a
small bar over it shows its actions and toggles as buttons and its colour choices as swatches;
using one calls the plugin back, one undo step. The labels are the plugin's own text.

In a presented prototype (the presenter or the preview pane) clicks on a widget's clickable parts
call its plugin back the same way, with the same limits, but on the prototype's own copy of the
file: the widget changes in the prototype only, the file and its undo history stay as they are,
and restarting the prototype starts the widget over. An interaction on the clicked layer wins over
the widget.

If a callback fails, it is rolled back and its message shown; a plugin that returned an error
keeps its panel, one that crashed or ran out of fuel or memory loses it.

From scripts, the CLI and MCP the same commands apply, panels included:

```jsonl
{"command": "plugin.import", "params": {"path": "plugins/tidy-names"}}
{"command": "select.all", "params": {}}
{"command": "plugin.run", "params": {"id": "org.figcraft.examples.tidy-names", "command": "tidy", "params": {"case": "Title Case"}}}
{"command": "plugin.import", "params": {"path": "target/plugins/dist/swatch-grid"}}
{"command": "plugin.run", "params": {"id": "org.figcraft.examples.swatch-grid", "command": "panel"}}
{"command": "plugin.event", "params": {"id": "org.figcraft.examples.swatch-grid", "control": "palette", "value": "Ocean"}}
{"command": "plugin.event", "params": {"id": "org.figcraft.examples.swatch-grid", "control": "generate"}}
{"command": "plugin.close", "params": {"id": "org.figcraft.examples.swatch-grid"}}
{"command": "plugin.import", "params": {"path": "target/plugins/dist/counter"}}
{"command": "widget.insert", "params": {"plugin": "org.figcraft.examples.counter", "x": 0, "y": 0}}
{"command": "widget.list", "params": {}}
{"command": "widget.click", "params": {"id": 12, "target": "increment"}}
{"command": "widget.setText", "params": {"id": 12, "field": "label", "value": "Votes"}}
{"command": "widget.menu", "params": {"id": 12, "item": "color", "value": "#14AE5C"}}
```

`figcraft-cli run script.jsonl --in design.figcraft --out tidy.figcraft` runs that headless; over
MCP, `run_command` takes the same ids. A session started by the CLI or the MCP server starts with no
plugins: import them in the script.

| Command | Params | Does |
|---|---|---|
| `plugin.list` | `{}` | `{plugins: [{id, name, version, description, main, api, permissions, commands, path, loaded}], last: {id, command}}` |
| `plugin.import` | `{path}` or `{manifest, wasm}` | `path` is a `figcraft-plugin.json` or its folder. Without files (the browser, tests), pass the manifest (an object or JSON text) and the module as base64 `wasm`; such a plugin lasts for the session only. A plugin with the same id is replaced. The module is checked first. |
| `plugin.remove` | `{id}` | Uninstalls it. |
| `plugin.run` | `{id, command?, params?, fuel?}` | Runs a command (default: the first) with its parameters checked against the manifest and the missing ones filled with their defaults; `fuel` lowers the instruction budget (for the panel's callbacks too). Returns `{plugin, command, output, logs, calls, fuel, panel}`; `output` is the plugin's result, parsed as JSON when it is JSON; `panel` is the plugin's open panel (as `plugin.ui` returns it) or null. |
| `plugin.ui` | `{id?}` | Plugin `id`'s open panel: `{plugin, command, title, controls, values}` (`values`: every field's current value by name), or null. Without `id`, the panel opened last. |
| `plugin.event` | `{id, control, value?}` | Uses a control of plugin `id`'s open panel, as a click would: a button (no `value`), or a field set to `value` (checked like a parameter: a number is clamped, a list or menu value must be one of its options, a colour must be hex). The plugin is called back; returns like `plugin.run`. |
| `plugin.close` | `{id}` | Closes plugin `id`'s panel and ends its session. `{closed}` says whether one was open. |
| `plugins.runLast` | `{}` | Runs the last plugin command again. |
| `plugins.manage` | `{}` | Opens the Plugins window; returns the list like `plugin.list`. |
| `widget.insert` | `{plugin, x?, y?, fuel?}` | Puts widget plugin `plugin`'s widget on the current page, its top-left at `x`, `y` (default: centred in the view), and selects it; the plugin draws it. One undo step. Returns `{id, plugin, state, targets, fields, menu, output, logs, calls, fuel}`: `targets` are the names of its clickable parts, `fields` of its editable fields, `menu` its property menu items. |
| `widget.click` | `{id, target, fuel?}` | Clicks part `target` of widget `id` (or of the widget a layer `id` is in), as a click on the canvas would: the plugin is called back and redraws it. One undo step; returns like `widget.insert`. |
| `widget.setText` | `{id, field, value, fuel?}` | Sets editable field `field` of widget `id` (or of the widget a layer `id` is in) to the text `value`, as editing it on the canvas would: the value (at most 1,000 characters, cleaned) goes into the state under the field's name and into the field's text, and the plugin is called back. One undo step; returns like `widget.insert`. |
| `widget.menu` | `{id, item, value?, fuel?}` | Uses item `item` of widget `id`'s property menu, as its bar would: an action takes no `value`; a toggle takes `true` or `false` (default: the opposite of now); a colour takes one of its options (hex, any case). The plugin is called back. One undo step; returns like `widget.insert`. |
| `widget.hover` | `{ids?}` | Shows the hover styles of widget layers `ids`, as the canvas does while the pointer is over them: a live preview with no undo step, which the next command (or no `ids`) ends. Returns `{hovering}`. |
| `widget.list` | `{}` | `{widgets: [{id, plugin, name, page, state, targets, fields, menu}]}`: every widget in the file. |
| `plugin.directory.list` | `{server?, query?}` | The plugin directory at `server` (`ws://host:port/`; default: the one browsed last): `{server, plugins: [{id, name, version, description, author, api, access, commands, widget, bytes, sha256, published, versions, installed, update}]}`, the latest version of each plugin whose id, name, description or author contains `query`. `installed` is the installed version (or null), `update` whether the listed one is newer. |
| `plugin.directory.install` | `{id, version?, confirm, server?, sha256?}` | Downloads version `version` (default: the latest) and installs it, or updates the installed one. `confirm` must be `true` (installing runs the plugin's code: show its permissions first); without it nothing is downloaded. The download must match the SHA-256 the directory lists (and `sha256`, the one shown, when given). Returns `{installed, name, version, permissions, sha256, path, source, replaced}`. |
| `plugin.directory.publish` | `{path \| plugin, server}` | Publishes a plugin folder (or its `figcraft-plugin.json`), or an installed plugin, as a new version. `server` carries the server's key: `ws://host:port/?key=…`. Returns `{published, server}`. |
| `plugin.directory.unpublish` | `{id, version?, server}` | Removes one version (default: every version) from the directory; needs the key. Returns `{id, removed}`. |

## Plugin directories

A `figcraft-server` ([`server.md`](server.md#plugin-directory)) can keep a directory of plugins for a
team. Anyone who can reach the server may browse it and install from it; only whoever has the
server's key may publish to it or unpublish from it. There are no accounts: the server's operator
decides who gets the key and moderates what's there.

**Installing.** In **Plugins ▸ Manage plugins… ▸ Browse**, type the server's address
(`ws://host:47900/`, or `wss://…` behind TLS) and press **Browse**: nothing is fetched before that.
The list shows each plugin's latest version, author (as the publisher typed it), size and the
access it asks for; the search filters it. **Install** opens a confirmation that lists what the
plugin asks permission to do (read or edit the document, the commands it may call, whether it draws
a widget), the server it comes from and its SHA-256; only its **Install** button downloads and
installs it. The app never waits for the server: while the listing or the download is on its way,
Browse shows "Fetching from the server…" and the confirmation "Installing…", each with **Cancel**,
and the rest of the app keeps working (closing the window cancels too). The window is drawn again
only when the answer arrives. Scripts, the CLI, the control channel and the MCP server get their
answer from the same commands as before, waiting for it. The download must match the SHA-256 the directory lists, and the bundle is then
checked like an imported plugin (manifest, plugin API version, the module compiled but not run,
asset names) before anything is written. A plugin installed this way runs in the same sandbox,
with the same limits and its manifest's permissions, as any other.

The desktop app keeps such plugins per user, beside its settings (`Plugins/` in FigCraft's settings
folder: `%APPDATA%\FigCraft` on Windows, `~/Library/Application Support/FigCraft` on macOS,
`~/.config/FigCraft` elsewhere; `config/` beside the executable in a portable copy), each in a folder of its own named by a hash of its id, with its
module and its assets. The CLI and the MCP server install for the session only.

**Updating.** When the directory has a newer version than the one installed, the plugin's row
offers **Update**, with the same confirmation (the permissions may have changed). Browsing again is
the update check; nothing checks in the background. **Remove** uninstalls it and deletes its folder.

**Publishing.** A plugin folder holds `figcraft-plugin.json`, the module its `main` names and, if
you like, an `assets/` folder of up to 32 plain files (an icon, a licence) that are installed with
it. Publish it with the server's key:

```sh
FIGCRAFT_SERVER_KEY=<key> figcraft-cli plugin publish target/plugins/dist/counter --server ws://host:47900/
```

(the key may also be in the address, `?key=…`, but the environment keeps it out of your shell
history), or `plugin.directory.publish {path, server}` from a script or MCP. The server checks the
whole bundle before keeping it, refuses a version that isn't newer than the one it has, and lists
the new version at once. `plugin.directory.unpublish` (or the operator's `figcraft-server plugins
remove`) takes a version, or the whole plugin, down; installed copies keep working.

**In the browser** the directory isn't available yet: the browser build's network link has no
plugin directory requests (`plugin.directory.*` says so). Import plugins with `plugin.import
{manifest, wasm}` there.

## The manifest: `figcraft-plugin.json`

```json
{
  "id": "org.figcraft.examples.swatch-grid",
  "name": "Swatch grid",
  "version": "1.0.0",
  "description": "Makes a grid of colour swatches from a palette of hex colours.",
  "main": "swatch_grid.wasm",
  "api": 1,
  "permissions": {
    "access": "write",
    "commands": ["view.get", "shape.rectangle", "text.create", "select.set"]
  },
  "commands": [
    {
      "id": "grid",
      "label": "Swatch grid…",
      "params": [
        { "name": "palette", "label": "Palette", "type": "text", "default": "#FF5A5F, #00A699" },
        { "name": "columns", "label": "Columns", "type": "number", "default": 3, "min": 1, "max": 24 },
        { "name": "labels", "label": "Label each swatch", "type": "boolean", "default": true }
      ]
    }
  ]
}
```

| Field | |
|---|---|
| `id` | Required. 1–128 letters, digits, `.`, `-`, `_`; a reverse domain name is a good choice. |
| `name` | Required. Shown in the Plugins menu (64 characters at most). |
| `version`, `description` | Optional text. A plugin published to a [directory](#plugin-directories) needs a `version` of one to four numbers (`1.2.0`), each version newer than the last. |
| `author` | Optional: who made it, as you type it (64 characters at most); plugin directories show it. |
| `main` | Required. The module, a `.wasm` path relative to the manifest, inside its folder. |
| `api` | The plugin API version the plugin was written for: `1` (the default), `2` (panels and settings), `3` (widgets), `4` (saving files, widget fields and property menus) or `5` (panels from widgets). A manifest asking for a newer one than FigCraft runs is refused, so a plugin that needs panels, widgets or `saveFile` fails clearly on an older FigCraft. |
| `permissions.saveFile` | API 4: `true` lets the plugin offer a file to save (an exporter's output). FigCraft asks the user where it goes with its own Save dialog (a download in the browser); the plugin never gets a path. Manage plugins lists it ("can save files"). A plugin directory's entries don't carry it (their wire format stays as it is), so installing any API 4 plugin from one says it may offer files to save. |
| `widget` | API 3: `true` when the plugin draws a widget (it's listed in Plugins ▸ Widgets). |
| `permissions.access` | `read` (the default): commands that leave the document as it is; such plugins also run from Dev Mode's Inspect tab. `write`: edits too. |
| `permissions.commands` | Optional allowlist: exact command ids or prefixes ending in `*` (`node.*`). Empty: any command the access allows. |
| `commands` | The menu commands, each `{id, label, params?}`. Without any, the plugin has one, `run`, unless it's a widget (a widget needs no command). |
| `params[]` | `{name, label?, type, default?, options?, min?, max?}`; `type` is `text`, `number` (clamped to `min`/`max`), `boolean`, `select` (one of `options`, a menu), `list` (one of `options`, shown as a list), `slider` (a number between `min` and `max`, both required, shown as a slider with its value beside it) or `color` (hex, `#RRGGBB` or `#RRGGBBAA`). A default that doesn't fit is dropped. |

## What a plugin can't do

Whatever its manifest says, FigCraft refuses these calls (the plugin gets the refusal as an error
it can handle):

- read or write files: a plugin never gets a path or the disk. With `permissions.saveFile` it can
  only hand FigCraft a file (one per run, 16 MB at most, a plain file name) for the user to save
  through FigCraft's own dialog; the user can cancel it;
- files and the app: `file.*`, `app.*`, `pref.*`, `help.*`, `library.add`, `libraries.*`,
  `image.place` with a `path` (pass the image as base64 `data` instead), `variable.export`,
  `codeConnect.export` and `component.playground` with a `path` (leave it out to get the result
  back), `codeConnect.import` with a `path` and `variable.import` with a file path instead of JSON,
  `window.*` (new windows open files), `view.theme*`;
- live sessions and online files: `collab.*`;
- other documents and the version history: `doc.*` and `version.*` (except `doc.list`, which gives a
  plugin the open files' names but not their paths, and `version.list`);
- the undo history and nesting: `edit.undo`, `edit.redo`, `edit.repeat`, `command.batch` (a run is
  already one undo step), `plugin.*` / `plugins.*` (no plugin runs another) and `widget.*`;
- writing layer data whose key starts with `figcraft.widget` (a widget's own data: no plugin can
  forge a widget or change another one's state);
- commands that open a dialog or panel (for example `object.rename` without a `name`, which would
  start renaming in the layers panel): the call is undone and refused;
- for a `read` plugin, any command that changes the document (checked before the call for commands
  that make an undo step, and after it for the rest: a change is undone and refused).

Limits per run, and again per panel callback: 1,000,000,000 fuel (about a second or two of plain
WebAssembly in a debug build, far less in a release build), 64 MB of memory, 100,000 host calls
(each also costs 10,000 fuel, including `reply_len`, `reply_read` and `set_output`), 16 MB per string either way, 1,000 log lines, and modules up to
16 MB. A panel spec is at most 1 MB with 64 controls, each image at most 256 KB; settings are at most 64 KB per plugin (keys
and JSON values), keys 1–128 characters. A widget callback gets the same limits; its description is
at most 4 MB with 256 layers nested 16 deep, its state at most 16 KB (keys 1–128 characters), and
its images 1 MB each, 2 MB together and 4096 pixels on a side. A property menu has at most 8
items (labels up to 32 characters), a colour item 1–8 colours; a field's value is at most 1,000
characters.

Copying a byte across the guest-memory boundary costs one additional fuel, reserved before the
allocation or copy. This also applies when the configured per-call overhead is zero. A reply
read is charged for the bytes actually returned, not the requested buffer size. `calls` counts
every ABI import, including calls which only query the reply length.

## ABI (version 6)

Version 2 adds panels and settings to version 1, version 3 adds widgets, version 4 adds
`save_file` and widget fields and property menus (new keys in the widget description and new
widget events), version 5 lets a widget open its plugin's panel (no new imports), and version 6 adds lines and SVG to the widget description; a module written for an older version runs unchanged and gets the input it always
got. A widget description with a field or a menu from a plugin whose manifest says `3` is refused, as
is one with a line or an SVG from a manifest that says less than `6`,
and a FigCraft that runs an older API refuses a manifest that asks for `4` when it's imported. Strings are UTF-8 `(ptr, len)` pairs in the plugin's linear memory; all values
are `i32`.

The module **exports** `memory` and `figcraft_run(input_len) -> status`. When `figcraft_run` starts,
the reply buffer holds the input JSON, `input_len` bytes:

```json
{"api": 1, "command": "grid", "params": {"palette": "#FF5A5F", "columns": 3, "labels": true}, "selection": [12, 15]}
```

`api` is the manifest's. When the plugin's panel calls it back, `figcraft_run` runs again in the
same instance (its memory as the last call left it) with the event and every field's value added:

```json
{"api": 2, "command": "panel", "params": {}, "selection": [], "event": {"control": "generate", "value": null}, "values": {"palette": "Ocean", "columns": 3, "labels": true}}
```

When its widget is inserted or clicked, `figcraft_run` runs in a fresh instance (nothing is kept
from one call to the next: a widget's state is all it remembers, so the same state and click
always give the same widget) with `widget` added: the widget's layer id, its state (an empty object
for a new widget) and what was clicked (`event` is null when it's inserted):

```json
{"api": 3, "command": "widget", "params": {}, "selection": [], "widget": {"id": 12, "state": {"count": 2}, "event": {"target": "increment"}}}
```

With API 4 the event can also be an edited field (its value is already in `state` under the
field's name) or a property menu item (`value`: null for an action, the new `true`/`false` for a
toggle, the colour picked for a colour item):

```json
{"api": 4, "command": "widget", "params": {}, "selection": [], "widget": {"id": 12, "state": {"count": 2, "label": "Votes"}, "event": {"field": "label", "value": "Votes"}}}
{"api": 4, "command": "widget", "params": {}, "selection": [], "widget": {"id": 12, "state": {"count": 2}, "event": {"menu": "color", "value": "#14AE5C"}}}
```

With API 5, a callback of a panel a widget opened has the panel's `event` and `values` and the
widget's id and state:

```json
{"api": 5, "command": "widget", "params": {}, "selection": [], "event": {"control": "set", "value": null}, "values": {"count": 42}, "widget": {"id": 12, "state": {"count": 2}}}
```

It returns `0` for success; anything else fails the run (and rolls it back), with the text passed
to `set_output` as the error message.

The module may **import** these functions from the module `figcraft` (anything else, WASI
included, is refused when the plugin is imported):

| Import | Signature | Does |
|---|---|---|
| `call` | `(cmd_ptr, cmd_len, params_ptr, params_len) -> status` | Runs a command with JSON params (empty: `{}`). `0`: done, the reply holds its result JSON. `1`: refused or failed, the reply holds the message. |
| `selection` | `() -> len` | The reply becomes the selected layer ids, a JSON array. |
| `reply_len` | `() -> len` | The reply's length in bytes. |
| `reply_read` | `(ptr, len) -> copied` | Copies up to `len` bytes of the reply to `ptr`. |
| `log` | `(ptr, len)` | A line of the run's log (returned in `logs`, and written to FigCraft's log). |
| `notify` | `(ptr, len)` | A short message for the user (a toast in the app). |
| `set_output` | `(ptr, len)` | The run's result (JSON or text), or its error message when it fails. |
| `ui_show` | `(ptr, len) -> status` | API 2. Shows the plugin's panel, or replaces the one showing (the panel spec below, as JSON); from a widget's callback, API 5. `1`: the spec is invalid, the reply says why. |
| `ui_close` | `()` | API 2. Closes the panel when the call returns. |
| `setting_get` | `(key_ptr, key_len) -> status` | API 2. `0`: the reply holds the setting's JSON. `1`: there is none. |
| `setting_set` | `(key_ptr, key_len, value_ptr, value_len) -> status` | API 2. Stores JSON under the key, for this plugin only, kept with the preferences. `1`: refused (not JSON, a bad key, over 64 KB), the reply says why. |
| `setting_delete` | `(key_ptr, key_len)` | API 2. Forgets a setting. |
| `save_file` | `(name_ptr, name_len, mime_ptr, mime_len, data_ptr, data_len) -> status` | API 4, needs `permissions.saveFile`. Offers a file to save; FigCraft asks the user where it goes once the run succeeds. `1`: refused (no permission, a bad file name, over 16 MB, a second file in one run), the reply says why. |
| `widget_render` | `(ptr, len) -> status` | API 3. Draws the widget being inserted or clicked, or (API 5) the widget whose panel called back (the widget description below, as JSON). `1`: the description is invalid, the reply says why. A widget call that doesn't draw keeps the widget as it was (inserting one must draw it). |

A pointer or length outside the plugin's memory traps, which fails the run. Host functions can only
be called while `figcraft_run` runs (not from a start function). Settings are forgotten when the
plugin is removed.

### Panel spec

```json
{"title": "Swatch grid", "controls": [
  {"type": "list", "name": "palette", "label": "Palette", "options": ["Sunset", "Ocean"], "value": "Ocean"},
  {"type": "note", "label": "#0A2463, #1E5AA8, #3E92CC"},
  {"type": "number", "name": "columns", "label": "Columns", "min": 1, "max": 24, "value": 3},
  {"type": "boolean", "name": "labels", "label": "Label each swatch", "value": true},
  {"type": "color", "name": "accent", "label": "Accent", "value": "#FF5A5F"},
  {"type": "button", "name": "generate", "label": "Generate"},
  {"type": "button", "name": "close", "label": "Close"}
]}
```

`title` defaults to the plugin's name. Controls are the manifest's parameter types (`text`,
`number`, `boolean`, `select`, `list`, `slider`, `color`) plus `button`, `note` (text in `label`, up to
1,000 characters, no `name` needed) and `image` (`data`: a PNG, JPEG, WebP, GIF, BMP or TIFF file as
base64, at most 256 KB and 4,096 pixels on a side; `label` is what screen readers say, and what
shows if the picture can't be; no `name` needed), in order, each `name` unique. A field shows its `value`; a
field without one keeps what the user entered before the panel was updated, so a plugin can update
a note without resetting the other fields. Buttons next to each other share a row; the first button
is the primary one. FigCraft checks the whole spec (sizes, kinds, options, bounds) and drops values
that don't fit their field.

### Widget description

```json
{"state": {"count": 2}, "root": {"type": "frame", "layout": "horizontal", "gap": 12, "padding": 12, "radius": 12, "fill": "#FFFFFF", "stroke": "#D9D9D9", "children": [
  {"type": "frame", "target": "decrement", "layout": "horizontal", "justify": "center", "width": 32, "height": 32, "radius": 8, "fill": "#F0F0F0",
   "children": [{"type": "text", "text": "−", "size": 16, "bold": true}]},
  {"type": "text", "name": "Count", "text": "2", "size": 24, "bold": true, "color": "#1E1E1E"},
  {"type": "image", "data": "iVBORw0KGgo…", "width": 24, "height": 24}
]}}
```

`state` is the widget's new state, a JSON object (left out: it keeps the one it has). `root` is the
widget itself, a frame; FigCraft replaces everything inside it with the layers described and keeps
the widget where it is (and its size, unless the description gives one). Every layer has a `type`
(`frame`, `text`, `rectangle`, `ellipse`, `image`, and with API 6 `line` and `svg`) and may have a `name`, a `target` (clicking it
calls the plugin back with that name: letters, digits, `.`, `-`, `_`), `x` and `y` (in its frame,
ignored in auto layout), `width` and `height`, `fill` and `stroke` (hex colours), `strokeWeight`,
`radius` (frames, rectangles, images) and `opacity`. A frame may have `layout` (`horizontal` or
`vertical`), `gap`, `padding`, `justify` and `align` (`min`, `center`, `max`: along and across the
layout) and `children`;
with auto layout it hugs its contents on an axis without a size. A text has `text`, `size`,
`color` and `bold` (with a `width` it wraps), and with API 4 may have a `field` (a name like a
target's): the text is then an editable field showing its current value. An image has `data`, the image's bytes (PNG, JPEG…)
as base64, usually carried in the module itself; without a size it takes the image's.
With API 6, a line is `width` long (100 without one), with `stroke` (black without one) and
`strokeWeight` (1); it has no height, so a click needs a clickable frame around it. An SVG has `src`,
the picture as SVG text (at most 1 MB, counted with the images), which comes in as layers of a frame
of the picture's size and takes neither `fill` nor `stroke`. A `width` or `height` scales it to fit,
keeping its proportions, and its layers count towards the widget's 256. Both may have `rotation`,
degrees from -180 to 180 around the top-left corner, positive counter-clockwise. A line's `hover`
takes no `fill` and an SVG's only `opacity`.
Any layer, the widget itself too, may have a `hover`: how it looks while the pointer is over
it on the canvas, with `fill` (a text: `color`), `stroke` and `opacity` replacing its own,
for example `"hover": {"fill": "#E0E0E0"}` on a button. It is shown as a live preview, never
saved or sent to a live session, and needs no new API (a FigCraft that doesn't know it draws the
layer as it is).
FigCraft checks the whole description (kinds, sizes, numbers, colours, lengths, counts, depth,
image sizes) and refuses it if anything is off.

With API 4, `menu` beside `root` is the widget's property menu (left out: none), for example:

```json
"menu": [{"item": "reset", "label": "Reset"},
         {"item": "big", "label": "Steps of 10", "type": "toggle", "checked": true},
         {"item": "color", "label": "Colour", "type": "color", "options": ["#1E1E1E", "#0D99FF"], "value": "#0D99FF"}]
```

Each item has an `item` name (like a target's, each once), a `label` (default: the name) and a
`type`: `action` (the default), `toggle` (with `checked`) or `color` (with `options`, 1–8 hex
colours, and optionally `value`, one of them: the current one). Every redraw replaces the menu.

## Writing a plugin in Rust

`plugins/` in the repository is a workspace of its own: `plugins/sdk` (the crate
`figcraft-plugin-sdk`, `no_std` with `alloc`) wraps the ABI with serde_json, and
`plugins/examples` has four plugins:

- **Tidy names** (`tidy-names`): cleans up the selected layers' names (extra spaces, the "copy" left
  by duplicating) and optionally their letter case, inside the selection too. Two commands: one
  with a form (case, include layers inside), and a quick one without.
- **Swatch grid** (`swatch-grid`, API 2): a grid of colour swatches, centred in the view, named after
  their colours and optionally labelled. **Palettes…** opens a panel with a list of built-in
  palettes (picking one shows its colours), columns, labels and their colour, and a Generate
  button; the palette
  picked last is remembered as a setting for next time. **From hex colours…** takes a palette of
  hex colours in the parameters form.
- **Counter** (`counter`, API 5): a widget, a count between − and + buttons under a label; each
  click changes the count kept in the widget's state, a double-click on the label edits it, and its
  menu resets the count, counts in tens or colours it; its **Set count…** opens a panel whose Set
  button redraws the widget with the count entered. It only reads the document.
- **Export names** (`export-names`, API 4): saves the names of the selected layers (optionally the
  layers inside them too) as `layer-names.txt`. It reads the document and offers the file with
  `save_file`; FigCraft asks where it goes.

`cargo xtask plugins` (part of `cargo xtask wasm` and `cargo xtask ci`; it needs the
`wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`) checks, tests and
builds them, and leaves each example's manifest and module side by side in
`target/plugins/dist/<example>/`, ready to import. With `CARGO_TARGET_DIR` set, that location is
`$CARGO_TARGET_DIR/plugins/dist/<example>/`. Plugin compilation and host tests use the caller's
same Cargo target directory; wasm modules are built under its `wasm32-unknown-unknown/release/`
directory. The plugin workspace keeps its own lockfile and release profile.

A plugin of your own:

```toml
# Cargo.toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
figcraft-plugin-sdk = { path = "../figcraft/plugins/sdk" }

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
strip = true
```

```rust
// src/lib.rs
use figcraft_plugin_sdk::{Input, Result, Value, call, entry, json, notify};

fn run(input: Input) -> Result<Value> {
    if input.selection.is_empty() {
        return Err("Select some layers first".into());
    }
    for id in &input.selection {
        let layer = call("node.get", &json!({"id": id}))?;
        let name = layer["name"].as_str().unwrap_or_default().to_uppercase();
        call("object.rename", &json!({"ids": [id], "name": name}))?;
    }
    notify(&format!("Renamed {} layers", input.selection.len()));
    Ok(json!({"renamed": input.selection.len()}))
}

entry!(run);
```

Build it with `cargo build --release --target wasm32-unknown-unknown`, put
`target/wasm32-unknown-unknown/release/my_plugin.wasm` next to a `figcraft-plugin.json` whose `main`
names it, and import the manifest. The SDK gives `call`, `selection`, `log`, `notify` and the
`Input` (`command`, `params` with `str`/`f64`/`bool` helpers, `selection`, and `event` when the
panel calls back: `control`, `value` and every field's `values`); returning `Err` fails the run and
rolls it back. With `"api": 2` in the manifest: `show_ui(&json!({...}))` and `close_ui()` for the
panel, `setting(key)`, `set_setting(key, &value)` and `delete_setting(key)` for settings. With
`"api": 3` and `"widget": true`: `input.widget` (`id`, `state`, `target`) when the widget is
inserted or clicked, and `render_widget(&json!({"state": ..., "root": ...}))` to draw it. With
`"api": 4`, `input.widget` also has `field` or `menu` and the `value` that came with it, and the
description may have fields and a `menu`; with `"permissions": {"saveFile": true}` too,
`save_file(name, mime, &bytes)` offers a file; FigCraft asks the user where it goes and the plugin
never learns a path. Outside
WebAssembly every host call fails, so the plugin's own logic can be unit-tested with `cargo test`.

Any language that compiles to WebAssembly works the same way: export `memory` and `figcraft_run`,
import what you need from `figcraft`.

## Not yet

- Distribution: directories live on your own `figcraft-server`; there is no public catalogue,
  no publisher accounts or signatures (the server's key is the publisher), no ratings, reviews or
  install counts, and updates are found when you browse, not in the background. The browser build
  can't use a directory yet.
- Plugins written for other apps (JavaScript plugins) don't run.
- Widgets: clicks, field edits and menu items are their only events (no hover or timers), a field
  is plain text (no placeholder, length or single-line setting), and their layers are frames, text, rectangles, ellipses and images only (no
  vectors, components or variables).
- Panels: no free layout inside a panel; images can't be clicked.
- Data in the document: plugins keep data on layers (pages included) with `node.set {props:
  {pluginData: {key: value}}}` (a `null` value removes a key) and read it back with `node.get`, which
  reports `pluginData: {key: value}` (values as strings) when a layer has any; there is no separate
  per-document store.
