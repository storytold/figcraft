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
  parts calls the plugin back to redraw it from its state, which is kept in the document.
- **More for widgets and panels (API 4).** A widget's texts can be edited in place, a selected
  widget shows a property menu the plugin declares, its parts can have a hover style and a
  tooltip, and it can ask to be called back after a delay. Widgets draw lines, vectors, gradients
  and shadows too, and panels get sliders, pictures, progress bars, dividers and sections.
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
fields (text, number, checkbox, menu, list, colour) and buttons, drawn with FigCraft's own widgets
and named for screen readers. A colour field's swatch opens FigCraft's colour picker (the plugin hears
of the colour when a drag in the picker ends); its hex can also be typed. Pressing a button or
changing a field calls the plugin back with every field's value; what that callback does is one
undo step (none when it changes nothing), and the plugin can update the panel or close it. The
module stays loaded while its panel is open, so the plugin keeps its state between callbacks.
With API 4 a panel also has sliders (a number field beside each takes the keyboard; while one is
dragged the plugin hears of it at most ten times a second, and once more when the drag ends),
pictures, progress bars, dividers and sections that fold the controls after them; a control can
be greyed out, and a field can show a validation message under it.
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

A widget written for API 4 can do more:

- **Text typed in place.** A text the plugin made editable shows an I-beam; a click edits it in
  place with FigCraft's text editor (caret, selection, the input method), all of it selected. What
  you type shows at once, stops at the text's maximum length, and stays on one line unless the
  text takes several. Enter (Escape or a click elsewhere too; in a text of several lines Enter
  starts a new line) commits: the plugin is called back with the text and redraws the widget, as
  one undo step. Leaving the text as it was calls nobody.
- **A property menu.** A selected widget shows the menu its plugin declared as a bar beside it on
  the canvas (above it, or under it when there is no room), and the same items under **Widget** in
  the Design panel, where Tab reaches them: buttons, toggles, dropdowns, a row of colours to pick
  one from, separators and links. Using one calls the plugin back with the item's name and its new
  value, as one undo step. A link opens only when it is clicked, only if it is an `http` or `https`
  address, and its plugin isn't called.
- **Hover styles and tooltips.** A part can change its fill, stroke or opacity while the pointer
  is over it, and show a tooltip. FigCraft does that itself: the plugin isn't called for pointer
  moves, and the look is never part of the document (it isn't saved, synced or undone).
- **Timers.** A callback can ask for the widget to be called back after 100 ms to an hour. A
  widget has one timer at most; asking again replaces it. Timers are kept with the open file, not
  in it: they run while the file is open and in front (one that came due meanwhile fires when you
  come back to it), wait while you drag or type and while a prototype is presented, and are gone
  once the file is closed or the widget is deleted. A run of timer callbacks is one undo step,
  and undoing it, or the step that started the timer, stops the timer. The window only wakes
  when the next timer is due.

**Timers in a live session** run for one person: whoever's click (or typing, or menu choice)
started the timer last. Starting a timer stamps the widget's layer with a mark only that
person's app knows, and a timer fires only while its mark is still on the layer; when someone
else starts the widget's timer, their mark replaces it and the earlier timer stops without
calling anyone. So two people's timers never take turns rewriting a widget, everyone else sees
the timer's updates arrive like any other edit, and when the person whose timer it is leaves, it
stops until someone starts it again. A file you may only view runs no callbacks at all.

In a presented prototype (the presenter or the preview pane) clicks on a widget's clickable parts
call its plugin back the same way, with the same limits, but on the prototype's own copy of the
file: the widget changes in the prototype only, the file and its undo history stay as they are,
and restarting the prototype starts the widget over. An interaction on the clicked layer wins over
the widget. Hover styles and tooltips show there too; a click on an editable text opens a field
over it (one line: Enter or a click elsewhere commits, Escape leaves the text as it was); and
timers run on the player's clock, for the prototype's copy only. While a prototype is presented
full screen, the file's own timers wait.

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
{"command": "widget.menu", "params": {"id": 12}}
{"command": "widget.menuAction", "params": {"id": 12, "item": "step", "value": "5"}}
{"command": "plugin.import", "params": {"path": "target/plugins/dist/focus-list"}}
{"command": "widget.insert", "params": {"plugin": "org.figcraft.examples.focus-list", "x": 0, "y": 200}}
{"command": "widget.setText", "params": {"input": "title", "text": "Release day"}}
{"command": "widget.click", "params": {"target": "focus"}}
{"command": "widget.fireTimer", "params": {}}
```

A widget command without an `id` acts on the selected widget (`widget.insert` selects the one it
makes).

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
| `widget.insert` | `{plugin, x?, y?, fuel?}` | Puts widget plugin `plugin`'s widget on the current page, its top-left at `x`, `y` (default: centred in the view), and selects it; the plugin draws it. One undo step. Returns `{id, plugin, state, targets, inputs, menu, timer, output, logs, calls, fuel}`: `targets` are the names of its clickable parts, `inputs` its editable texts (`[{name, text, maxLength, multiline}]`), `menu` its property menu (as `widget.menu` lists it), `timer` the milliseconds until its pending timer is due (null: none). |
| `widget.click` | `{id?, target, fuel?}` | Clicks part `target` of widget `id` (or of the widget a layer `id` is in; without `id`, the selected one), as a click on the canvas would: the plugin is called back and redraws it. One undo step; returns like `widget.insert`. |
| `widget.setText` | `{id?, input, text, fuel?}` | API 4. Commits `text` to the editable text named `input`, as typing it in place and pressing Enter does: cut to its maximum length, on one line unless it takes several, control characters dropped. The plugin is called back with it; one undo step; returns like `widget.insert`. |
| `widget.menu` | `{id?}` | API 4. `{id, plugin, items: [{type, name, label, value, options, url}]}`: the widget's property menu (`type`: `action`, `toggle`, `dropdown`, `color`, `separator` or `link`). |
| `widget.menuAction` | `{id?, item, value?, fuel?}` | API 4. Uses menu item `item`: an action takes no `value`; a toggle `true` or `false` (without one it flips); a dropdown or a colour one of its `options`. The plugin is called back; one undo step; returns like `widget.insert`. A link isn't opened and its plugin isn't called: `{id, item, url}`. |
| `widget.fireTimer` | `{id?, fuel?}` | API 4. Fires the widget's pending timer now, for scripts and tests (the app fires it when it is due; a headless session has no clock for it): the plugin is called back. A run of timer callbacks is one undo step. Refused when no timer is pending. |
| `widget.list` | `{}` | `{widgets: [{id, plugin, name, page, state, targets, inputs, menu, timer}]}`: every widget in the file. |
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
`~/.config/FigCraft` elsewhere), each in a folder of its own named by a hash of its id, with its
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
| `api` | The plugin API version the plugin was written for: `1` (the default), `2` (panels and settings), `3` (widgets) or `4` (text input, property menus, hover, timers and richer drawing in widgets; sliders, pictures and sections in panels). A manifest asking for a newer one than FigCraft runs is refused, so a plugin that needs panels or widgets fails clearly on an older FigCraft. What a version added is read only from plugins that ask for it: an older plugin's descriptions are read as its version read them, so it behaves as it always did. |
| `widget` | API 3: `true` when the plugin draws a widget (it's listed in Plugins ▸ Widgets). |
| `permissions.access` | `read` (the default): commands that leave the document as it is; such plugins also run from Dev Mode's Inspect tab. `write`: edits too. |
| `permissions.commands` | Optional allowlist: exact command ids or prefixes ending in `*` (`node.*`). Empty: any command the access allows. |
| `commands` | The menu commands, each `{id, label, params?}`. Without any, the plugin has one, `run`, unless it's a widget (a widget needs no command). |
| `params[]` | `{name, label?, type, default?, options?, min?, max?, step?}`; `type` is `text`, `number` (clamped to `min`/`max`), `boolean`, `select` (one of `options`, a menu), `list` (one of `options`, shown as a list), `color` (hex, `#RRGGBB` or `#RRGGBBAA`) or, with API 4, `slider` (a number from `min` to `max`, 0 to 100 when not given, in `step`s). Panels also take `button`, `note`, `image`, `progress`, `divider`, `section` and `grid` (a command's parameters never do). A default that doesn't fit is dropped. |

## What a plugin can't do

Whatever its manifest says, FigCraft refuses these calls (the plugin gets the refusal as an error
it can handle):

- files and the app: `file.*`, `app.*`, `pref.*`, `help.*`, `library.add`, `libraries.*`,
  `image.place` with a `path` (pass the image as base64 `data` instead), `view.theme*`;
- other documents and the version history: `doc.*` and `version.*` (except `doc.list` and
  `version.list`);
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
16 MB. A panel spec is at most 64 KB with 64 controls; settings are at most 64 KB per plugin (keys
and JSON values), keys 1–128 characters. A widget callback gets the same limits; its description is
at most 4 MB with 256 layers nested 16 deep, its state at most 16 KB (keys 1–128 characters), and
its images 1 MB each, 2 MB together and 4096 pixels on a side. With API 4: a text or a typed text
is at most 1,000 characters (`maxLength` lowers it), a vector's path data 8 KB with every
coordinate within ±4,000 and 64 vectors in a widget, a gradient has 2–8 stops, a tooltip is at
most 200 characters, a property menu has at most 16 items with 32 options each and links of at
most 2,048 characters, a timer runs 100 ms to an hour with one pending per widget, and a panel is
at most 1 MB with its pictures 512 KB together (a picture is shown up to 2,048 pixels on a side).
Every one of those callbacks (typed text, a menu item, a timer) is metered and capped like a click.

Copying a byte across the guest-memory boundary costs one additional fuel, reserved before the
allocation or copy. This also applies when the configured per-call overhead is zero. A reply
read is charged for the bytes actually returned, not the requested buffer size. `calls` counts
every ABI import, including calls which only query the reply length.

## ABI (version 4)

Version 2 adds panels and settings to version 1, version 3 adds widgets, and version 4 adds one
host function (`widget_timer`), more events for widgets and more in the panel spec and the widget
description; a version 1, 2 or 3 module runs unchanged and gets the input it always got. Strings are UTF-8 `(ptr, len)` pairs in the plugin's linear memory; all values
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

With API 4, `event` is also one of these, for a text typed in place, an item of the property
menu (`value`: a toggle's new state, a dropdown's or colour's choice, null for an action) and a
timer that is due:

```json
{"input": "title", "text": "Release day"}
{"menu": "step", "value": "5"}
{"timer": true}
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
| `ui_show` | `(ptr, len) -> status` | API 2. Shows the plugin's panel, or replaces the one showing (the panel spec below, as JSON). `1`: the spec is invalid, the reply says why. |
| `ui_close` | `()` | API 2. Closes the panel when the call returns. |
| `setting_get` | `(key_ptr, key_len) -> status` | API 2. `0`: the reply holds the setting's JSON. `1`: there is none. |
| `setting_set` | `(key_ptr, key_len, value_ptr, value_len) -> status` | API 2. Stores JSON under the key, for this plugin only, kept with the preferences. `1`: refused (not JSON, a bad key, over 64 KB), the reply says why. |
| `setting_delete` | `(key_ptr, key_len)` | API 2. Forgets a setting. |
| `widget_render` | `(ptr, len) -> status` | API 3. Draws the widget being inserted or clicked (the widget description below, as JSON). `1`: the description is invalid, the reply says why. A widget call that doesn't draw keeps the widget as it was (inserting one must draw it). |
| `widget_timer` | `(ms) -> status` | API 4. Asks for the widget being called back to be called again in `ms` milliseconds (100 to 3,600,000), replacing its pending timer; `0` stops it. `1`: refused (out of range, or the manifest's `api` is below 4), the reply says why. The timer starts when the callback succeeds. |

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
`number`, `boolean`, `select`, `list`, `color`) plus `button` and `note` (text in `label`, up to
1,000 characters, no `name` needed), in order, each `name` unique. A field shows its `value`; a
field without one keeps what the user entered before the panel was updated, so a plugin can update
a note without resetting the other fields. Buttons next to each other share a row; the first button
is the primary one. FigCraft checks the whole spec (sizes, kinds, options, bounds) and drops values
that don't fit their field.

API 4 adds these controls and fields:

```json
{"title": "Dot grid", "controls": [
  {"type": "text", "name": "name", "label": "Name", "value": "", "error": "Give the grid a name"},
  {"type": "slider", "name": "columns", "label": "Dots per row", "min": 2, "max": 12, "step": 1, "value": 6},
  {"type": "section", "label": "Spacing", "value": false},
  {"type": "slider", "name": "gap", "label": "Gap", "min": 0, "max": 32, "step": 1, "value": 8},
  {"type": "divider"},
  {"type": "image", "label": "A preview of the dot grid", "data": "Qk02bAAA…", "height": 120, "fit": "fit"},
  {"type": "grid", "name": "shape", "label": "Shape", "columns": 4, "value": "circle", "items": [
    {"value": "circle", "path": "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Z", "label": "Circle", "color": "#1E1E1E"},
    {"value": "plus", "path": "M12 5v14M5 12h14", "label": "Plus", "color": "#1E1E1E", "stroke": 2.0},
    {"value": "photo", "data": "iVBORw0KGgo…", "label": "A photo"}
  ]},
  {"type": "progress", "label": "36 of 144 dots", "value": 0.25},
  {"type": "button", "name": "generate", "label": "Generate", "disabled": true}
]}
```

- `slider`: a field holding a number from `min` to `max` (0 to 100 when not given) on a `step`
  counted from `min` (`number` takes `step` too).
- `image`: a picture from its file's bytes as base64 in `data` (PNG, JPEG, BMP…, usually carried
  in the module), `height` high (16–400, 120 when not given), `fit` in the panel's width (`fit`
  shows all of it, `fill` stretches it, `crop` and `tile` cover the box); `label` names it for
  screen readers.
- `grid`: clickable cells, `columns` across (6 when not given, 1–12). Each item has a `value`
  (unique, the picked value) and shows SVG path data (`path`, painted in `color`, `#1E1E1E` when
  not given: filled, or stroked `stroke` wide) or a picture (`data`, as an image control's);
  `label` names it for screen readers and tooltips. A click calls the plugin back with the picked
  value, like a button; the picked cell draws selected. 1–256 cells, each path 16 KB at most, and
  a cell's picture counts toward the images' 512 KB together.
- `progress`: a bar filled to `value` (0–1), with `label` above it.
- `divider`: a line across the panel.
- `section`: a heading (`label`) that folds the controls after it, up to the next section or
  divider; `value` says whether it starts open (true when not given). Folding is the user's and
  calls nobody.
- `disabled: true` on any control greys it out: it can't be used (and `plugin.event` refuses it).
- `error` on a field shows that message under it (200 characters at most).

None of these needs a `name` except `slider`. An API 4 panel may be 1 MB.

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
(`frame`, `text`, `rectangle`, `ellipse`, `image`) and may have a `name`, a `target` (clicking it
calls the plugin back with that name: letters, digits, `.`, `-`, `_`), `x` and `y` (in its frame,
ignored in auto layout), `width` and `height`, `fill` and `stroke` (hex colours), `strokeWeight`,
`radius` (frames, rectangles, images) and `opacity`. A frame may have `layout` (`horizontal` or
`vertical`), `gap`, `padding`, `justify` and `align` (`min`, `center`, `max`: along and across the
layout) and `children`;
with auto layout it hugs its contents on an axis without a size. A text has `text`, `size`,
`color` and `bold` (with a `width` it wraps). An image has `data`, the image's bytes (PNG, JPEG…)
as base64, usually carried in the module itself; without a size it takes the image's.
FigCraft checks the whole description (kinds, sizes, numbers, colours, lengths, counts, depth,
image sizes) and refuses it if anything is off.

API 4 adds:

```json
{"state": {"title": "Launch checklist"},
 "menu": [
  {"type": "action", "name": "clear", "label": "Clear ticked"},
  {"type": "toggle", "name": "pinned", "label": "Pinned", "value": false},
  {"type": "dropdown", "name": "length", "label": "Countdown", "options": ["30 seconds", "1 minute"], "value": "30 seconds"},
  {"type": "color", "name": "paper", "label": "Paper", "options": ["#FFF3B0", "#D9F2E3"], "value": "#FFF3B0"},
  {"type": "separator"},
  {"type": "link", "name": "help", "label": "About FigCraft", "url": "https://getartcraft.com/apps/figcraft"}],
 "root": {"type": "frame", "layout": "vertical", "width": 272, "gap": 10, "padding": [16, 16, 20, 16], "radius": [2, 2, 14, 2],
  "gradient": {"type": "linear", "angle": 90, "stops": [{"at": 0, "color": "#FFF7C7"}, {"at": 1, "color": "#FFF3B0"}]},
  "shadow": {"x": 0, "y": 4, "blur": 12, "color": "#00000030"}, "children": [
  {"type": "text", "text": "Launch checklist", "input": "title", "maxLength": 60, "width": 240, "size": 16, "weight": 700,
   "textAlign": "left", "maxLines": 2, "tooltip": "Click to rename"},
  {"type": "line", "width": 240, "stroke": "#00000022", "fillWidth": true},
  {"type": "frame", "target": "toggle.0", "layout": "horizontal", "justify": "center", "width": 16, "height": 16, "radius": 4, "fill": "#2B2B2B",
   "hover": {"opacity": 0.8}, "children": [{"type": "vector", "path": "M2 6 L5 9 L10 3", "stroke": "#FFFFFF", "strokeWeight": 2}]},
  {"type": "image", "data": "iVBORw0KGgo…", "width": 24, "height": 24, "fit": "fit"}
]}}
```

- **Layers.** `line` is a horizontal line `width` long (black and 1 thick unless it has a
  `stroke`). `vector` is a shape from SVG path data in `path` (`M L H V C S Q T A Z`, absolute or
  relative); its box is the path's, and a `width` or `height` scales it.
- **Auto layout.** `padding` is one number or four (`[top, right, bottom, left]`), `radius` one
  or four (`[top-left, top-right, bottom-right, bottom-left]`). `fillWidth` / `fillHeight` on a
  child take the room that's left on that axis of its auto layout frame. That frame needs a size
  on the axis (its own, or because it fills too): a frame can't hug what fills it, and the widget
  itself has nothing to fill.
- **Looks.** `gradient` replaces `fill` on frames, rectangles, ellipses and vectors: `linear`
  along `angle` in degrees (0 runs left to right, 90 top to bottom, the default) or `radial` from
  the centre, with 2–8 `stops` (`at` 0–1, `color` hex). `shadow` is a drop shadow (`x`, `y`,
  `blur`, `spread`, `color`).
- **Text.** `weight` (100–900; it overrides `bold`), `textAlign` (`left`, `center`, `right`),
  and with a `width`, `maxLines`: the text keeps a fixed box and what doesn't fit is cut with an
  ellipsis.
- **Images.** `fit` is `fill` (the default), `fit`, `crop` or `tile`.
- **Editable text.** `input` names a text the user can edit in place; the plugin gets
  `{"input": name, "text": …}` when it's committed. `maxLength` (1–1,000) caps it and `multiline`
  lets it take several lines. Each name is used once. The plugin decides what the widget shows
  afterwards: a callback that draws nothing leaves the text as it was.
- **Hover and tooltip.** `hover` (`fill`, `stroke`, `opacity`) is what changes while the pointer
  is over the layer; `tooltip` (200 characters at most) shows when it rests there.
- **The property menu.** `menu` beside `root` lists the items (16 at most): `action` (a button),
  `toggle` (`value`: true or false), `dropdown` (`value`: one of `options`), `color` (`value`:
  one of the hex colours in `options`), `separator` and `link` (`url`, http or https). Every item
  but a separator has a `name`, used once, and a `label`. Left out, the widget keeps the menu it
  has; an empty list removes it. FigCraft remembers a toggle's, dropdown's or colour's new value
  when it's chosen, so a callback needn't hand the menu back.

A plugin written for API 3 gets none of this: the fields are ignored as unknown fields always
were, and the new kinds and the four-number forms are refused as they always were.

## Writing a plugin in Rust

`plugins/` in the repository is a workspace of its own: `plugins/sdk` (the crate
`figcraft-plugin-sdk`, `no_std` with `alloc`) wraps the ABI with serde_json, and
`plugins/examples` has five plugins:

- **Tidy names** (`tidy-names`): cleans up the selected layers' names (extra spaces, the "copy" left
  by duplicating) and optionally their letter case, inside the selection too. Two commands: one
  with a form (case, include layers inside), and a quick one without.
- **Swatch grid** (`swatch-grid`, API 2): a grid of colour swatches, centred in the view, named after
  their colours and optionally labelled. **Palettes…** opens a panel with a list of built-in
  palettes (picking one shows its colours), columns, labels and their colour, and a Generate
  button; the palette
  picked last is remembered as a setting for next time. **From hex colours…** takes a palette of
  hex colours in the parameters form.
- **Counter** (`counter`, API 4): a widget, a count between − and + buttons; each click changes the
  count kept in the widget's state. Its property menu resets the count and picks the step of a
  click and the count's colour, and its buttons have a hover style and a tooltip. It only reads
  the document.
- **Focus list** (`focus-list`, API 4): a sticky checklist widget laid out with auto layout. Its
  title and items are edited in place, a last line adds an item, each item has a box to tick (a
  vector tick), and a button starts a focus countdown that a timer ticks down every second, with
  a progress line. The menu clears ticked items, sets the countdown's length and picks the note's
  colour.
- **Dot grid** (`dot-grid`, API 4): a panel with sliders for the dots per row, their size and the
  gap between them (in a folding section), a colour, a preview picture that follows them (drawn
  pixel by pixel in the plugin: no image file) and a progress bar. Without a name the field says
  so and Generate is greyed out; Generate puts the grid in the middle of the view as one group.

`cargo xtask plugins` (part of `cargo xtask wasm` and `cargo xtask ci`; it needs the
`wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`) checks, tests and
builds them, leaves each example's manifest and module side by side in
`target/plugins/dist/<example>/`, ready to import (it prints each module's size: 75–92 KB), and
then runs every built example in the engine as a script would, so a module that stops working
fails the build.

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
`"api": 4`: `input.widget.event` says what happened (`WidgetEvent::Insert`, `Click(target)`,
`Text { input, text }`, `Menu { name, value }` or `Timer`), `set_timer(ms)` and `cancel_timer()`
manage the widget's timer, and the `ui` module builds descriptions without spelling the JSON out:

```rust
use figcraft_plugin_sdk::ui::{Node, menu_action, menu_toggle, render};
use figcraft_plugin_sdk::{Input, Result, Value, WidgetEvent, entry, json, set_timer};

fn run(input: Input) -> Result<Value> {
    let w = input.widget.ok_or("Insert me from Plugins ▸ Widgets")?;
    let mut title = w.state["title"].as_str().unwrap_or("Untitled").to_string();
    match &w.event {
        WidgetEvent::Text { input, text } if input == "title" && !text.is_empty() => title = text.clone(),
        WidgetEvent::Click(target) if target == "remind" => set_timer(60_000)?,
        _ => {}
    }
    let root = Node::column()
        .width(240.0)
        .gap(8.0)
        .padding(12.0)
        .radius(12.0)
        .fill("#FFFFFF")
        .shadow(0.0, 2.0, 8.0, "#00000033")
        .child(Node::text(&title).input("title", 60).width(216.0).weight(600.0).tooltip("Click to rename"))
        .child(Node::line(216.0).stroke("#E6E6E6", 1.0).fill_width())
        .child(Node::row().target("remind").padding(8.0).radius(8.0).fill("#F0F0F0").hover(Some("#E0E0E0"), None, None)
            .child(Node::text("Remind me in a minute")));
    render(&json!({"title": title}), root, Some(vec![menu_action("clear", "Clear"), menu_toggle("pinned", "Pinned", false)]))?;
    Ok(Value::Null)
}

entry!(run);
```

`Node::set(key, value)` reaches any property the builders don't name, and plain `json!` still
works everywhere. For panels, `ui::slider`, `ui::image` (from a file's bytes; `ui::base64` encodes
them), `ui::progress`, `ui::divider`, `ui::section`, `ui::disabled` and `ui::invalid` make the
controls API 4 added. Outside WebAssembly every host call fails, so the plugin's own logic can be
unit-tested with `cargo test`.

Any language that compiles to WebAssembly works the same way: export `memory` and `figcraft_run`,
import what you need from `figcraft`.

## Not yet

- Distribution: directories live on your own `figcraft-server`; there is no public catalogue,
  no publisher accounts or signatures (the server's key is the publisher), no ratings, reviews or
  install counts, and updates are found when you browse, not in the background. The browser build
  can't use a directory yet.
- Plugins written for other apps (JavaScript plugins) don't run.
- Widgets: they can't open a panel, their layers can't be components or use styles or variables,
  an image is bytes in the description (the module carries it; a widget can't name a file from
  the plugin's `assets/` folder), there are no pointer events beyond clicks and FigCraft's own
  hover styles (no drag, no callback on hover), and a timer doesn't run while its file is closed
  or behind another tab. In a presented prototype, typed text is one line and can't be emptied.
- Panels: no free layout or tabs inside a panel (sections fold; controls stack), and a slider has
  no tick marks or labels of its own.
- The property menu bar on the canvas is used with the pointer; from the keyboard its items are
  in the Design panel.
- Data in the document: plugins keep data on layers (pages included) with `node.set {props:
  {pluginData: {key: value}}}`; there is no separate per-document store.
