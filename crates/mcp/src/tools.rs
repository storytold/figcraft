//! MCP tool definitions (names, descriptions, JSON Schemas) and their implementations on top of a
//! [`Backend`]. Every tool reduces to control-channel methods, so headless and attached sessions
//! behave the same.

use base64::Engine as _;
use figcraft_engine::params::normalize_paint;
use serde_json::{Map, Value, json};

use crate::backend::{Backend, decode_data, page_children};

/// The result of `tools/call`: MCP content blocks plus the `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    pub fn error(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
    }
    /// An image block followed by a JSON text block describing it.
    pub fn image(bytes: &[u8], mime: &str, info: &Value) -> Self {
        Self {
            content: vec![
                json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(bytes), "mimeType": mime}),
                json!({"type": "text", "text": info.to_string()}),
            ],
            is_error: false,
        }
    }
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
    /// The first text block (tests and the CLI).
    pub fn first_text(&self) -> &str {
        self.content.iter().find_map(|c| c.get("text").and_then(Value::as_str)).unwrap_or("")
    }
}

// ---------- schemas ----------

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}
fn int(desc: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": desc})
}
fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}
fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}
fn id_schema(desc: &str) -> Value {
    json!({"description": desc, "anyOf": [{"type": "integer"}, {"type": "string"}]})
}
fn ids_schema(desc: &str) -> Value {
    json!({"type": "array", "description": desc, "items": {"anyOf": [{"type": "integer"}, {"type": "string"}]}})
}
fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
    })
}

/// Tool names, in `tools/list` order.
pub const TOOL_NAMES: &[&str] = &[
    "list_commands",
    "run_command",
    "batch",
    "create_node",
    "set_properties",
    "inspect_document",
    "get_node",
    "select",
    "export",
    "screenshot",
    "open_file",
    "save_file",
    "undo",
    "redo",
    "pointer",
    "key",
    "type_text",
    "inspect_ui",
    "wait_for_changes",
];

/// The tools that take `document`: they act on the file it names instead of the one in front.
const DOCUMENT_TOOLS: &[&str] = &[
    "run_command",
    "batch",
    "create_node",
    "set_properties",
    "inspect_document",
    "get_node",
    "select",
    "export",
    "screenshot",
    "save_file",
    "undo",
    "redo",
];

/// All tools, in `tools/list` order.
pub fn tool_definitions() -> Vec<Value> {
    let mut tools = tools();
    let document = id_schema(
        "The open file to act on: its `id` (from run_command doc.list, or the `document` open_file and inspect_document return) or its path. Default: the file in front. The tab in front stays in front; with an attached app, a screenshot of the window needs the file in front",
    );
    for t in &mut tools {
        if t.get("name").and_then(Value::as_str).is_some_and(|n| DOCUMENT_TOOLS.contains(&n))
            && let Some(props) = t.pointer_mut("/inputSchema/properties").and_then(Value::as_object_mut)
        {
            props.insert("document".into(), document.clone());
        }
    }
    tools
}

fn tools() -> Vec<Value> {
    let empty = || obj(json!({}), &[]);
    vec![
        tool(
            "list_commands",
            "List commands",
            "List FigCraft commands: id, label, menu path, shortcut, whether it is undoable, and a parameter description. Every menu item, panel action and shortcut is a command; run one with run_command (or several as one undo step with batch). Most commands act on `ids` when given, else on the selection. Use `filter` to narrow (e.g. \"align\", \"layout\", \"variable\", \"style\", \"component\", \"proto\").",
            obj(json!({"filter": string("Case-insensitive substring matched against id, label, menu path and parameter docs")}), &[]),
            true,
        ),
        tool(
            "run_command",
            "Run command",
            "Execute one FigCraft command by id with JSON params (ids and params from list_commands). Mutating commands are one undo step each; a failing command leaves the document unchanged. Examples: {\"command\":\"align.left\",\"params\":{\"ids\":[3,4]}}, {\"command\":\"layout.add\",\"params\":{\"ids\":[2],\"direction\":\"vertical\"}}, {\"command\":\"style.create\",\"params\":{\"type\":\"paint\",\"name\":\"Brand/Primary\",\"from\":5}}. Returns the command's result (creation commands return {\"id\"}). Programmatic calls never open dialogs.",
            obj(
                json!({"command": string("Command id, e.g. node.create, align.left, component.create, file.export"), "params": {"type": "object", "description": "Command parameters (see list_commands)"}}),
                &["command"],
            ),
            false,
        ),
        tool(
            "batch",
            "Run commands as one step",
            "Run several commands in order as ONE undo step (engine `command.batch`). By default (onError: \"abort\"), if any command fails, everything is rolled back and the error names the failing step; success returns an array of the results in order. With onError: \"continue\", a failed step is rolled back on its own, the later steps still run, the call is not an error, and the array instead holds one {step, command, ok, result} or {step, command, ok, error} object per step; a reference to a failed step fails that step too. A later step can use what an earlier step returned: name a step with \"as\": \"card\", then anywhere in a later step's params {\"$ref\": \"card\"} stands for the `id` in its result ({\"$step\": n} does the same by zero-based position). Add \"field\" for another part of the result: a key or a slash path, e.g. {\"$ref\": \"copy\", \"field\": \"ids/0\"} for the first id edit.duplicate returned. Any object with a `$ref` or `$step` key is a reference. Example: [{\"command\":\"node.create\",\"as\":\"card\",\"params\":{\"type\":\"frame\"}}, {\"command\":\"node.create\",\"params\":{\"type\":\"text\",\"text\":\"Hi\",\"parent\":{\"$ref\":\"card\"}}}] builds a frame and a text inside it in one call.",
            obj(
                json!({
                    "commands": {"type": "array", "minItems": 1, "description": "Steps to run", "items": obj(json!({"command": string("Command id"), "params": {"type": "object"}, "as": string("Name for this step, unique in the batch; later steps use its result with {\"$ref\": name}")}), &["command"])},
                    "label": string("Undo history label (default \"Batch\")"),
                    "onError": {"type": "string", "enum": ["abort", "continue"], "description": "abort (default) rolls back the whole batch on the first failure; continue rolls back only the failed step, runs the rest and returns one {step, command, ok, result|error} object per step"},
                }),
                &["commands"],
            ),
            false,
        ),
        tool(
            "create_node",
            "Create layer",
            "Create a layer (engine `node.create`) and return its id and inspected properties. Coordinates are page units, y down; x/y are relative to the parent frame (the page when there is none, so then they are page coordinates; groups don't count). Inside an auto-layout frame the position is decided by the layout. The new layer becomes the selection.",
            obj(
                json!({
                    "type": {"type": "string", "enum": ["frame", "rectangle", "ellipse", "polygon", "star", "line", "arrow", "text", "vector", "section", "component", "slice"], "description": "Layer type"},
                    "x": num("Left, relative to the parent frame (default 0)"),
                    "y": num("Top, relative to the parent frame (default 0)"),
                    "width": num("Width (default 100; text hugs its content unless width is set, which makes it wrap; for a vector with path, width/height scale the path, one alone keeping its proportion, and without them the path's own box is used)"),
                    "height": num("Height (default 100; ignored for lines/arrows and auto-height text)"),
                    "parent": id_schema("Parent frame/group/component id (default: the current page)"),
                    "index": int("Z-index among the parent's children (default: on top)"),
                    "name": string("Layer name"),
                    "text": string("Characters, for type text"),
                    "path": string("SVG path data, for type vector (e.g. \"M0 0 L100 0 L50 80 Z\")"),
                    "points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "For type vector, instead of path: the points [[x, y], ...]"},
                    "closed": boolean("For type vector: close the path"),
                    "select": boolean("Select the new layer (default true)"),
                    "props": {"type": "object", "description": "Any set_properties keys applied at creation, e.g. {\"fill\":\"#3366FF\",\"cornerRadius\":12,\"fontSize\":24,\"layout\":\"vertical\",\"gap\":8,\"padding\":16}"},
                }),
                &["type"],
            ),
            false,
        ),
        tool(
            "set_properties",
            "Set properties",
            "Set properties on one or more layers as one undo step (engine `node.set`). Keys: name, visible, locked, opacity (0-1), blend, x, y (relative to the parent frame; groups don't count), width, height, rotation (degrees), fill (colour) / fills (array of paints: \"#hex\" or {\"type\":\"solid\",\"color\":\"#hex\"} or {\"type\":\"linearGradient\",\"stops\":[\"#hex\",\"#hex\"]} or stops [{\"position\":0..1,\"color\":\"#hex\"}], optional from/to handles in the unit box; radialGradient too), stroke / strokes, strokeWeight, strokeAlign (inside|center|outside), cornerRadius (n or [tl,tr,br,bl]), effects (array, e.g. {\"type\":\"dropShadow\",\"color\":\"#00000040\",\"offset\":[0,4],\"radius\":8}), clip, layout (null|horizontal|vertical|grid), gap, padding (n or [t,r,b,l]), mainAlign (min|center|max|spaceBetween|spaceAround|spaceEvenly), crossAlign, wrap, contentAlign (auto|spaceBetween: wrapped lines), sizingH / sizingV (fixed|hug|fill), constraints {horizontal, vertical}, characters, fontFamily, fontStyle, fontSize, lineHeight, letterSpacing, textAlign, textAutoResize. Auto layout keys (gap, padding, mainAlign, crossAlign, wrap, …) apply to frames, components and instances only: other layers among the ids are skipped and named in `warnings`, and with none that can take them the call is an error. A fontStyle draws in the family's nearest face; `warnings` says when that face has another weight or slant than the name asks for. Run list_commands with filter \"node.set\" for the full list. Unknown keys are an error. Returns the updated layers.",
            obj(
                json!({"ids": ids_schema("Layer ids (default: the selection)"), "props": {"type": "object", "description": "Property values to set"}}),
                &["props"],
            ),
            false,
        ),
        tool(
            "inspect_document",
            "Inspect document",
            "The active document as JSON: name, path, dirty flag, current page, selection, the current page's layer tree (id, name, type, x/y relative to the parent frame (groups don't count), width, height, absoluteBounds in page coordinates, fills, strokes, effects, cornerRadius, layout, sizing, text properties, component links, overrides, bound variables), other pages summarised, styles, variable count and the undo history. Use it to verify every change. Each level of `depth` multiplies the response, and `allPages` multiplies it again: on a large document the tree is reported at a shallower depth than asked for, with a `truncated` object saying so. Use get_node to read one subtree deeply instead.",
            obj(
                json!({
                    "depth": int("Tree depth for the current page (default 4)"),
                    "allPages": boolean("Expand every page, not only the current one"),
                    "maxBytes": int(
                        "Largest response to send, in bytes of the whole JSON-RPC message (default 8388608, which fits the usual 10 MiB client buffer); 1024 of it is kept for the envelope and the request id"
                    ),
                }),
                &[],
            ),
            true,
        ),
        tool(
            "get_node",
            "Get layer",
            "One layer as JSON (same fields as inspect_document) with its children to `depth`. A subtree too large for one message is reported at a shallower depth, or with a window of the children, and a `truncated` object says so and gives the `offset` to ask for next.",
            obj(
                json!({
                    "id": id_schema("Layer id"),
                    "depth": int("Child depth (default 1)"),
                    "offset": int("First child to report (default 0), for reading a node with more children than fit in one message"),
                    "maxBytes": int(
                        "Largest response to send, in bytes of the whole JSON-RPC message (default 8388608, which fits the usual 10 MiB client buffer); 1024 of it is kept for the envelope and the request id"
                    ),
                }),
                &["id"],
            ),
            true,
        ),
        tool(
            "select",
            "Select layers",
            "Replace the selection with `ids` (switching to their page). An empty array deselects everything. Most commands act on the selection when no `ids` are passed.",
            obj(json!({"ids": ids_schema("Layer ids to select")}), &["ids"]),
            false,
        ),
        tool(
            "export",
            "Export",
            "Export layers (default: the selection, else every top-level layer of the current page) as png, jpg, svg, webp, pdf or xml (an Android Vector Drawable, with `warnings` for what it changed or left out, per `vdCompat`) at `scale`, or the whole file as `fig` (experimental: written from public format knowledge, with `warnings` for what was left out; other apps may not open every file) (PDF: with `bleed`, `cropMarks`, `bleedMarks` and `cmyk` for print; options `contentsOnly`, `includeBoundingBox`, and for SVG `svgOutlineText`, `svgIdAttribute`, `svgSimplifyStroke`; for XML `svgIdAttribute` and `vdCompat`). With `path` the file is written there (absolute path recommended) and the result says where. Without `path`: png/jpg/webp come back as an image content block, svg as text, xml as text followed by `{format, warnings}`, other formats as base64 `data`.",
            obj(
                json!({
                    "ids": ids_schema("Layers to export (several are composited into one image)"),
                    "format": {"type": "string", "enum": ["png", "jpg", "svg", "webp", "pdf", "xml", "fig"], "description": "Format (default png, or from the path's extension)"},
                    "scale": num("Pixel scale for raster formats (default 1, max 64)"),
                    "size": string("Instead of `scale`: `2x`, or a width or height in pixels like `512w` / `512h`"),
                    "path": string("Output file path"),
                    "bleed": {"type": ["number", "string"], "description": "PDF: bleed around each page, in points or as a length like `3mm` or `0.125in`"},
                    "cropMarks": boolean("PDF: crop marks"),
                    "bleedMarks": boolean("PDF: bleed marks"),
                    "cmyk": boolean("PDF: CMYK colour instead of RGB"),
                    "contentsOnly": boolean("Only the layers themselves (default true); false also draws every other layer that shows in their area"),
                    "includeBoundingBox": boolean("Export the layers' own boxes, cutting off shadows and outside strokes (default false)"),
                    "svgOutlineText": boolean("SVG: text as outlines (default true); false keeps `<text>` elements"),
                    "svgIdAttribute": boolean("SVG: layer names as unique `id` attributes (default false)"),
                    "svgSimplifyStroke": boolean("SVG: strokes as `stroke` attributes where exact (default true); false writes outlines"),
                    "vdCompat": {"type": "string", "enum": ["bestEffort", "exact", "ignore"], "description": "XML: what a Vector Drawable can't draw exactly is approximated with a warning (bestEffort, default), left out with a warning (exact), or written as far as it maps without warnings (ignore: overlapping layer opacity dropped, even-odd fills kept as fillType=evenOdd, diamond gradients left out)"},
                }),
                &[],
            ),
            true,
        ),
        tool(
            "screenshot",
            "Screenshot",
            "Look at the design: renders the given layers (or the whole current page, with its background) to PNG and returns it as image content, plus its size and the scale used. The image is scaled down to fit `maxSize` pixels. With `window: true` (or a window id from `inspect_ui`'s `windows`) and an attached app, captures that app window including the UI instead.",
            obj(
                json!({
                    "ids": ids_schema("Layers to render (default: the whole current page)"),
                    "scale": num("Requested scale (default 1; reduced to fit maxSize)"),
                    "maxSize": int("Largest side in pixels (default 1600)"),
                    "window": {"description": "Capture the app window: true for the one in front, or a window id (attached app only; headless renders the page)", "anyOf": [{"type": "boolean"}, {"type": "integer"}]},
                }),
                &[],
            ),
            true,
        ),
        tool(
            "open_file",
            "Open file",
            "Open a .figcraft or .json document (FigCraft or REST API file JSON), a .fig or .sketch file (best effort, with `warnings` for what wasn't imported; .sketch from Sketch 48 or newer), an .svg (becomes vectors) or an image (becomes an image layer) as the active document. Returns the new document's summary.",
            obj(
                json!({
                    "path": string("File path"),
                    "variables": {"description": "REST API file JSON only: the local variables JSON (`GET /v1/files/:key/variables/local`) as a path or the JSON object. Default: `<name>.variables.json` or `variables.json` next to the file."},
                    "reload": boolean("A file that is already open: read it again from disk into its tab (refused while the tab has unsaved changes). Without it, the result says `stale: true` when the file changed on disk since the tab read or saved it"),
                }),
                &["path"],
            ),
            false,
        ),
        tool(
            "save_file",
            "Save file",
            "Save the active document as a .figcraft file. `path` is required the first time (an untitled document has no path); later saves reuse it. A `path` ending in .fig writes an experimental .fig export copy instead and leaves the document's own path and unsaved changes alone.",
            obj(json!({"path": string("Output .figcraft path, or a .fig path for an experimental export copy")}), &[]),
            false,
        ),
        tool("undo", "Undo", "Undo the last change (one command, batch or gesture). Returns {undone: bool}.", empty(), false),
        tool("redo", "Redo", "Redo the last undone change. Returns {redone: bool}.", empty(), false),
        tool(
            "pointer",
            "Pointer gesture",
            "Drive the canvas with the mouse in page coordinates, exactly as a user would with the current tool: click to select, drag to move/resize/marquee, or draw with a shape tool (set `tool` first, e.g. {\"tool\":\"rectangle\",\"kind\":\"drag\",\"x\":0,\"y\":0,\"to\":[200,100]}). `double` on a text layer starts editing it. Returns the selection and tool.",
            obj(
                json!({
                    "kind": {"type": "string", "enum": ["click", "double", "drag", "down", "move", "up", "hover"], "description": "Gesture (default click)"},
                    "x": num("Page x"),
                    "y": num("Page y"),
                    "to": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "Drag end [x, y] (page)"},
                    "steps": int("Intermediate move events for drag (default 8)"),
                    "tool": {"type": "string", "description": "Tool to choose first: move|scale|hand|frame|section|slice|rectangle|line|arrow|ellipse|polygon|star|pen|pencil|text|comment"},
                    "shift": boolean("Shift held"),
                    "alt": boolean("Alt/Option held"),
                    "cmd": boolean("Cmd (macOS) / Ctrl held"),
                    "ctrl": boolean("Ctrl held (the Control key on macOS)"),
                    "pressure": num("Pen pressure 0-1 (the Pencil records it as the stroke's width profile; a drag starts with it)"),
                    "pressureTo": num("Pen pressure 0-1 at a drag's end (interpolated along it)"),
                }),
                &["x", "y"],
            ),
            false,
        ),
        tool(
            "key",
            "Press key",
            "Press a key or shortcut as if typed on the canvas: a menu shortcut (e.g. {\"key\":\"G\",\"cmd\":true} groups, {\"key\":\"A\",\"alt\":true,\"shift\":false} …), a tool key (R, O, T, F, V…), Escape, Delete, Enter, arrows (nudge; shift = big nudge). While editing text, keys go to the text editor (Backspace, Delete, Left, Right, Home, End, Enter, Escape, SelectAll).",
            obj(
                json!({"key": string("Key name: a letter/digit, Escape, Delete, Backspace, Enter, ArrowLeft…"), "shift": boolean("Shift"), "alt": boolean("Alt/Option"), "cmd": boolean("Cmd/Ctrl"), "ctrl": boolean("Ctrl (the Control key on macOS)")}),
                &["key"],
            ),
            false,
        ),
        tool(
            "type_text",
            "Type text",
            "Type into a text layer. With `id`, starts editing that layer with all of its text selected (so the typed text replaces it), types, then stops editing. Without `id`, types at the caret of the text currently being edited (after a double-click with pointer, or after drawing with the text tool).",
            obj(json!({"text": string("Text to type (\\n for a new line)"), "id": id_schema("Text layer to replace the contents of")}), &["text"]),
            false,
        ),
        tool(
            "inspect_ui",
            "Inspect UI",
            "UI state: current tool, view (zoom, centre), status, panels, timings and the app's windows (`windows`: id, title, files) when attached to the app; headless reports the session status.",
            empty(),
            true,
        ),
        tool(
            "wait_for_changes",
            "Wait for changes",
            "Wait for new comment events in the attached FigCraft app. Returns pending events immediately; otherwise waits up to `timeoutMs` (default 20000, max 25000). Call it again with the returned `cursor` to keep watching. Headless returns immediately (no other writer). Events carry a sequence number, kind, document and a small payload (for comments, the thread id).",
            obj(
                json!({
                    "after": int("Return events after this cursor (default 0)"),
                    "timeoutMs": int("Longest wait in milliseconds (default 20000, max 25000)"),
                    "topics": {"type": "array", "items": {"type": "string", "enum": ["comments", "document", "all"]}, "description": "Event topics (default [\"comments\"]; document events are not emitted yet)"},
                    "limit": int("Most events to return (default 100, max 1000)")
                }),
                &[],
            ),
            true,
        ),
    ]
}

// ---------- helpers ----------

/// A backend whose every call names `document` (a tool's `document` argument).
struct InDocument<'a> {
    inner: &'a mut dyn Backend,
    document: Value,
}

impl Backend for InDocument<'_> {
    fn call(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if let Some(o) = params.as_object_mut() {
            o.insert("document".into(), self.document.clone());
        }
        self.inner.call(method, params)
    }

    fn has_ui(&self) -> bool {
        self.inner.has_ui()
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }
}

fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
}

fn wait_for_changes(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    b.call("events.poll", Value::Object(a.clone())).map(|v| ToolResult::json(&v))
}

fn args_obj(args: &Value) -> Result<Map<String, Value>, String> {
    match args {
        Value::Object(o) => Ok(o.clone()),
        Value::Null => Ok(Map::new()),
        _ => Err("`arguments` must be an object".into()),
    }
}

fn s_arg<'a>(a: &'a Map<String, Value>, k: &str) -> Result<Option<&'a str>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{k}` must be a string")),
    }
}

fn req_s<'a>(a: &'a Map<String, Value>, k: &str) -> Result<&'a str, String> {
    s_arg(a, k)?.ok_or_else(|| format!("`{k}` is required"))
}

fn n_arg(a: &Map<String, Value>, k: &str) -> Result<Option<f64>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => figcraft_engine::params::f64_of(v).map(Some).ok_or_else(|| format!("`{k}` must be a finite number")),
    }
}

fn id_arg(v: &Value) -> Result<u64, String> {
    figcraft_engine::params::id_of(v).map(|i| i.0).ok_or_else(|| format!("{v} is not a layer id"))
}

fn ids_arg(a: &Map<String, Value>, k: &str) -> Result<Option<Vec<u64>>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => arr.iter().map(id_arg).collect::<Result<Vec<_>, _>>().map(Some),
        Some(v) => id_arg(v).map(|i| Some(vec![i])),
    }
}

fn png_info(bytes: &[u8]) -> (u32, u32) {
    crate::png_size(bytes).unwrap_or((0, 0))
}

/// Union of the layers' absolute bounds (page coordinates).
fn bounds_of(b: &mut dyn Backend, ids: &[u64]) -> Option<(f64, f64, f64, f64)> {
    let mut acc: Option<(f64, f64, f64, f64)> = None;
    for id in ids.iter().take(500) {
        let Ok(n) = b.call("node.get", json!({"id": id, "depth": 0})) else { continue };
        let Some(ab) = n.get("absoluteBounds") else { continue };
        let g = |k: &str| ab.get(k).and_then(Value::as_f64);
        let (Some(x), Some(y), Some(w), Some(h)) = (g("x"), g("y"), g("width"), g("height")) else { continue };
        acc = Some(match acc {
            None => (x, y, x + w, y + h),
            Some((a, b2, c, d)) => (a.min(x), b2.min(y), c.max(x + w), d.max(y + h)),
        });
    }
    acc
}

/// Make paint/effect JSON friendlier than the engine's strict serde (see
/// [`figcraft_engine::params::normalize_paint`]). The engine reads paints this way itself; effects
/// still need it here.
pub fn normalize_props(props: &mut Map<String, Value>) {
    for k in ["fills", "strokes", "effects"] {
        if let Some(v) = props.get_mut(k) {
            normalize_paint(v);
        }
    }
}

/// [`normalize_props`] for command params: `props`, flat node.set keys, `paint`, `effect`.
fn friendly_params(p: &mut Value) {
    let Some(o) = p.as_object_mut() else { return };
    if let Some(Value::Object(props)) = o.get_mut("props") {
        normalize_props(props);
    }
    normalize_props(o);
    for k in ["paint", "effect"] {
        if let Some(v) = o.get_mut(k) {
            normalize_paint(v);
        }
    }
}

// ---------- dispatch ----------

/// Refuse argument names `schema` doesn't list, at the top and in the objects it describes inside
/// (a batch's steps); objects it leaves open (`params`) take any. `path` leads the names in the
/// error (`commands[0].`).
fn refuse_unknown_args(schema: &Value, args: &Value, path: &str) -> Result<(), String> {
    if let (Some(props), Some(given)) = (schema.get("properties").and_then(Value::as_object), args.as_object()) {
        let unknown: Vec<String> = given.keys().filter(|k| !props.contains_key(*k)).map(|k| format!("`{path}{k}`")).collect();
        if !unknown.is_empty() {
            let s = if unknown.len() == 1 { "" } else { "s" };
            let takes: Vec<&str> = props.keys().map(String::as_str).collect();
            let takes = if takes.is_empty() { "none".to_string() } else { takes.join(", ") };
            return Err(format!("unknown argument{s} {} (it takes {takes})", unknown.join(", ")));
        }
        for (k, v) in given {
            if let Some(inner) = props.get(k) {
                refuse_unknown_args(inner, v, &format!("{path}{k}."))?;
            }
        }
    }
    if let (Some(items), Some(list)) = (schema.get("items"), args.as_array()) {
        for (i, v) in list.iter().enumerate() {
            refuse_unknown_args(items, v, &format!("{}[{i}].", path.trim_end_matches('.')))?;
        }
    }
    Ok(())
}

/// Run one tool. Every failure becomes an `isError` result (never a panic).
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> ToolResult {
    let mut a = match args_obj(args) {
        Ok(a) => a,
        Err(e) => return ToolResult::error(e),
    };
    // Built once: the catalogue is the same on every call.
    static TOOLS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(tool_definitions);
    if let Some(schema) = TOOLS.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(name)).and_then(|t| t.get("inputSchema"))
        && let Err(e) = refuse_unknown_args(schema, &Value::Object(a.clone()), "")
    {
        return ToolResult::error(e);
    }
    let mut targeted;
    let b: &mut dyn Backend = match a.remove("document").filter(|d| !d.is_null()) {
        Some(document) => {
            targeted = InDocument { inner: b, document };
            &mut targeted
        }
        None => b,
    };
    let r = match name {
        "list_commands" => list_commands(b, &a),
        "run_command" => run_command(b, &a),
        "batch" => batch(b, &a),
        "create_node" => create_node(b, &a),
        "set_properties" => set_properties(b, &a),
        "inspect_document" => inspect_document(b, &a),
        "get_node" => get_node(b, &a),
        "select" => select(b, &a),
        "export" => export(b, &a),
        "screenshot" => screenshot(b, &a),
        "open_file" => open_file(b, &a),
        "save_file" => save_file(b, &a),
        "undo" => exec(b, "edit.undo", json!({})).map(|v| ToolResult::json(&v)),
        "redo" => exec(b, "edit.redo", json!({})).map(|v| ToolResult::json(&v)),
        "pointer" => pointer(b, &a),
        "key" => key(b, &a),
        "type_text" => type_text(b, &a),
        "inspect_ui" => b.call("ui.inspect", json!({})).map(|v| ToolResult::json(&v)),
        "wait_for_changes" => wait_for_changes(b, &a),
        other => Err(format!("unknown tool `{other}` (tools/list has: {})", TOOL_NAMES.join(", "))),
    };
    r.unwrap_or_else(ToolResult::error)
}

fn list_commands(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let filter = s_arg(a, "filter")?.map(str::to_lowercase).unwrap_or_default();
    let all = b.call("engine.commands", json!({}))?;
    let list: Vec<Value> = all
        .as_array()
        .map(|v| {
            v.iter()
                .filter(|c| {
                    filter.is_empty()
                        || ["id", "label", "menu", "params"]
                            .iter()
                            .any(|k| c.get(*k).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&filter)))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if list.is_empty() {
        return Ok(ToolResult::text(format!("no command matches `{filter}`")));
    }
    // One compact line per command keeps this readable for agents.
    let mut out = format!("{} commands (id — label [menu] (shortcut) params; * = undoable):\n", list.len());
    for c in &list {
        let g = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("");
        out.push_str(g("id"));
        if c.get("undoable").and_then(Value::as_bool) == Some(true) {
            out.push('*');
        }
        out.push_str(" — ");
        out.push_str(g("label"));
        if !g("menu").is_empty() {
            out.push_str(&format!(" [{}]", g("menu")));
        }
        if !g("shortcut").is_empty() {
            out.push_str(&format!(" ({})", g("shortcut")));
        }
        out.push(' ');
        out.push_str(g("params"));
        out.push('\n');
    }
    Ok(ToolResult::text(out))
}

fn run_command(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let cmd = req_s(a, "command")?;
    let params = match a.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err("`params` must be an object".into()),
    };
    let mut params = params;
    friendly_params(&mut params);
    exec(b, cmd, params).map(|v| ToolResult::json(&v))
}

fn batch(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let Some(Value::Array(cmds)) = a.get("commands") else { return Err("`commands` must be an array of {command, params}".into()) };
    if cmds.is_empty() {
        return Err("`commands` is empty".into());
    }
    let mut steps = Vec::with_capacity(cmds.len());
    for (i, c) in cmds.iter().enumerate() {
        let Some(cmd) = c.get("command").and_then(Value::as_str) else { return Err(format!("step {i}: `command` must be a string")) };
        let params = match c.get("params") {
            None | Some(Value::Null) => json!({}),
            Some(v @ Value::Object(_)) => v.clone(),
            Some(_) => return Err(format!("step {i} ({cmd}): `params` must be an object")),
        };
        let mut params = params;
        friendly_params(&mut params);
        let mut step = json!({"command": cmd, "params": params});
        if let (Some(name), Some(o)) = (c.get("as"), step.as_object_mut()) {
            o.insert("as".into(), name.clone());
        }
        steps.push(step);
    }
    let label = s_arg(a, "label")?.unwrap_or("Batch");
    let mut params = json!({"label": label, "commands": steps});
    if let Some(mode) = a.get("onError") {
        params["onError"] = mode.clone();
    }
    exec(b, "command.batch", params).map(|v| ToolResult::json(&v))
}

fn create_node(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    req_s(a, "type")?;
    for k in ["x", "y", "width", "height"] {
        n_arg(a, k)?;
    }
    if let Some(p) = a.get("props")
        && !p.is_object()
        && !p.is_null()
    {
        return Err("`props` must be an object".into());
    }
    let mut params = Map::new();
    for k in ["type", "x", "y", "width", "height", "parent", "index", "name", "text", "path", "points", "closed", "props", "select"] {
        if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
            params.insert(k.into(), v.clone());
        }
    }
    if let Some(Value::Object(p)) = params.get_mut("props") {
        normalize_props(p);
    }
    let r = exec(b, "node.create", Value::Object(params))?;
    let id = r.get("id").cloned().ok_or("node.create returned no id")?;
    let node = b.call("node.get", json!({"id": id, "depth": 0}))?;
    Ok(ToolResult::json(&json!({"id": id, "node": node})))
}

fn set_properties(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let props = match a.get("props") {
        Some(Value::Object(p)) if !p.is_empty() => p.clone(),
        _ => return Err("`props` must be a non-empty object".into()),
    };
    let mut props = props;
    normalize_props(&mut props);
    let mut params = json!({"props": props});
    let ids = ids_arg(a, "ids")?;
    if let Some(ids) = &ids {
        if ids.is_empty() {
            return Err("`ids` is empty".into());
        }
        params["ids"] = json!(ids);
    }
    let warnings = exec(b, "node.set", params)?.get("warnings").cloned();
    let ids = match ids {
        Some(i) => i,
        None => b
            .call("document.inspect", json!({"depth": 0}))?
            .get("selection")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default(),
    };
    let nodes: Vec<Value> = ids.iter().take(20).filter_map(|i| b.call("node.get", json!({"id": i, "depth": 0})).ok()).collect();
    let mut out = json!({"ok": true, "nodes": nodes});
    if let Some(w) = warnings {
        out["warnings"] = w;
    }
    Ok(ToolResult::json(&out))
}

/// What one tool result may take on the wire. MCP stdio is newline-delimited JSON, so a result is
/// a single message and has to fit the client's read buffer whole -- there is no streaming of part
/// of one. The TypeScript SDK's stdio transport defaults to 10 MiB
/// (`STDIO_DEFAULT_MAX_BUFFER_SIZE`) and hosts rarely raise it; a document whose tree went past
/// that killed the server, because the client stopped reading, the pipe broke, and every later
/// call failed (#912). The 2 MiB left over covers the JSON-RPC envelope.
const MAX_RESPONSE: usize = 8 << 20;

/// Room kept for the `truncated` object, which is added after the size is measured and would
/// otherwise be able to push a response that only just fitted back over the limit.
const MARKER_RESERVE: usize = 1 << 10;

/// Room kept for everything the text block is wrapped in: the content object and `isError`, the
/// JSON-RPC envelope (`jsonrpc`, `id`, `result`) and the newline the transport writes after it.
/// About ninety bytes of that is fixed and the rest is the request id, so `maxBytes` holds for
/// any id up to roughly 900 bytes -- which is every id in practice, a number or a UUID. An id
/// larger than that is the one case where the message can pass the caller's figure, and sizing
/// the real envelope here would mean handing the transport's request id to every tool.
const ENVELOPE_RESERVE: usize = 1 << 10;

/// What the caller says it can read, as the bytes its *tree* may take: the figure is for the whole
/// message, so the envelope's share comes off it here.
fn budget(a: &Map<String, Value>) -> Result<(usize, usize), String> {
    // A caller that knows its own buffer can say so; 1 KiB is below the shallowest tree, so the
    // floor only keeps the arithmetic sane.
    let max_bytes = match n_arg(a, "maxBytes")? {
        Some(v) if v < 1024.0 => return Err("`maxBytes` must be at least 1024".into()),
        Some(v) => v.min(usize::MAX as f64) as usize,
        None => MAX_RESPONSE,
    };
    Ok((max_bytes, max_bytes.saturating_sub(ENVELOPE_RESERVE)))
}

/// What a value's text block would take on the wire. It is sent as a text block, so it is measured
/// pretty-printed *and* escaped -- escaping adds about a tenth.
fn wire_size(value: &Value) -> usize {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    serde_json::to_string(&text).map_or(text.len(), |s| s.len())
}

/// The document at `depth`, and what its text block would take on the wire. The tree is sent as a
/// text block, so it is measured pretty-printed *and* escaped -- escaping adds about a tenth.
fn inspect_at(b: &mut dyn Backend, depth: u64, all: bool) -> Result<(Value, usize), String> {
    let value = b.call("document.inspect", json!({"depth": depth, "allPages": all}))?;
    let wire = wire_size(&value);
    Ok((value, wire))
}

/// One layer's subtree at `depth`, and what its text block would take on the wire.
fn node_at(b: &mut dyn Backend, id: u64, depth: u64) -> Result<(Value, usize), String> {
    let value = b.call("node.get", json!({"id": id, "depth": depth}))?;
    let wire = wire_size(&value);
    Ok((value, wire))
}

fn inspect_document(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let depth = n_arg(a, "depth")?.unwrap_or(4.0).clamp(0.0, 64.0) as u64;
    let all = match a.get("allPages") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(x)) => *x,
        Some(_) => return Err("`allPages` must be a boolean".into()),
    };
    let (max_bytes, budget) = budget(a)?;

    let (value, wire) = inspect_at(b, depth, all)?;
    if wire <= budget {
        return Ok(ToolResult::json(&value));
    }

    let (used, mut value, used_wire) = if depth == 0 {
        // Depth 0 is the shallowest tree there is, so there is nothing smaller to send: it goes
        // out over the limit with the marker saying so, the same as a bisection that reaches
        // depth 0. An error would leave the caller with nothing at all.
        (0, value, wire)
    } else {
        // The deepest tree that fits, by bisection: a deeper tree holds every node of a shallower
        // one, so the size does not fall as the depth grows. Bisecting keeps this to about six
        // more walks of the document rather than one per level.
        let (mut lo, mut hi) = (0u64, depth - 1);
        let mut best: Option<(u64, Value, usize)> = None;
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let (v, w) = inspect_at(b, mid, all)?;
            if w + MARKER_RESERVE <= budget {
                best = Some((mid, v, w));
                if mid == hi {
                    break;
                }
                lo = mid + 1;
            } else if mid == 0 {
                best = Some((0, v, w));
                break;
            } else {
                hi = mid - 1;
            }
        }
        match best {
            Some(found) => found,
            None => return Ok(ToolResult::json(&value)),
        }
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "truncated".into(),
            json!({
                "requestedDepth": depth,
                "depth": used,
                "requestedDepthBytes": wire,
                "bytes": used_wire,
                "maxBytes": max_bytes,
                "reason": format!(
                    "the tree at depth {depth} would be {wire} bytes on the wire, over the {max_bytes}-byte limit a client can read in one message; it is reported at depth {used} instead"
                ),
                "hint": "get_node({id, depth}) walks one subtree at a time, within the same limit: a node with more children than fit answers with a window of them and the offset to ask for next",
            }),
        );
    }
    Ok(ToolResult::json(&value))
}

/// One layer's subtree, kept inside what the caller can read.
///
/// `inspect_document` sends a shallower tree when the document's is too large and points here for
/// the rest, so this tool is the way out of that limit and must not be a way past it: a frame with
/// fifteen thousand children answers with 10.5 MiB at depth 1, over the buffer the report in #912
/// is about, and no depth makes that smaller because the width is at depth 1. So the subtree is
/// cut by depth first, as the document's is, and then by a window over the children.
fn get_node(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let id = id_arg(a.get("id").ok_or("`id` is required")?)?;
    let depth = n_arg(a, "depth")?.unwrap_or(1.0).clamp(0.0, 64.0) as u64;
    let offset = n_arg(a, "offset")?.unwrap_or(0.0).max(0.0).min(u32::MAX as f64) as usize;
    let (max_bytes, budget) = budget(a)?;

    let (value, wire) = node_at(b, id, depth)?;
    if value.is_null() {
        return Err(format!("no such layer {id}"));
    }
    if wire <= budget && offset == 0 {
        return Ok(ToolResult::json(&value));
    }

    // The deepest subtree that fits, by bisection, as the document's tree is cut. A node asked for
    // at a depth of its own children only (1) cannot be made shallower without losing them all,
    // so that is where the window takes over.
    let (mut used, mut value, mut used_wire) = (depth, value, wire);
    if depth > 1 && offset == 0 {
        let (mut lo, mut hi) = (1u64, depth - 1);
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let (v, w) = node_at(b, id, mid)?;
            if w + MARKER_RESERVE <= budget {
                (used, value, used_wire) = (mid, v, w);
                if mid == hi {
                    break;
                }
                lo = mid + 1;
            } else if mid == 1 {
                (used, value, used_wire) = (1, v, w);
                break;
            } else {
                hi = mid - 1;
            }
        }
        if used_wire + MARKER_RESERVE <= budget {
            return Ok(ToolResult::json(&with_marker(value, Cut { asked: depth, used, asked_wire: wire, used_wire }, max_bytes, true, None)));
        }
    }

    // Still over, or a window was asked for: the children from `offset` that fit.
    let total = value.get("children").and_then(Value::as_array).map_or(0, Vec::len);
    let window = children_window(&mut value, offset, budget.saturating_sub(MARKER_RESERVE));
    let shown = value.get("children").and_then(Value::as_array).map_or(0, Vec::len);
    // A child too large to fit on its own is passed over (a page of just the marker), so paging
    // moves on and every answer stays within maxBytes; `get_node` on the child reads it.
    let after = offset + shown.max(1);
    let next = (after < total).then_some(after);
    let used_wire = wire_size(&value);
    if used_wire + MARKER_RESERVE > budget {
        // Even with every child left out, the layer's own fields (a very long name, say) pass the
        // limit: an answer would be larger than the caller can read, so there is none.
        return Err(format!(
            "layer {id} takes {used_wire} bytes on the wire without its children, more than fit in maxBytes {max_bytes}; ask again with a larger `maxBytes`"
        ));
    }
    // Children left out after the window were cut by size; a window that only `offset` moved, of a
    // subtree within the limit, is paging.
    let over = wire > budget || next.is_some();
    let mut out = with_marker(
        value,
        Cut { asked: depth, used, asked_wire: wire, used_wire },
        max_bytes,
        over,
        Some(json!({
            "from": offset,
            "shown": shown,
            "total": total,
            "nextOffset": next,
        })),
    );
    if window && shown == total && offset == 0 && used == depth {
        // Nothing was left out after all (an offset of zero that fitted whole).
        out.as_object_mut().map(|o| o.remove("truncated"));
    }
    Ok(ToolResult::json(&out))
}

/// Keep the children from `offset` that fit in `budget`, and say whether any were left out. The
/// window is found by bisection: a node with thousands of children would otherwise be serialised
/// once per child.
fn children_window(value: &mut Value, offset: usize, budget: usize) -> bool {
    let Some(all) = value.get("children").and_then(Value::as_array).cloned() else { return false };
    let rest: Vec<Value> = all.iter().skip(offset).cloned().collect();
    let fits = |value: &mut Value, n: usize| {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("children".into(), Value::Array(rest.iter().take(n).cloned().collect()));
        }
        wire_size(value) <= budget
    };
    if fits(value, rest.len()) {
        return rest.len() != all.len();
    }
    // The largest window that fits, between none and all of the rest.
    let (mut lo, mut hi) = (0usize, rest.len());
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(value, mid) { lo = mid } else { hi = mid - 1 }
    }
    fits(value, lo);
    true
}

/// How a subtree was cut: the depth asked for and the depth reported, with their sizes on the wire.
struct Cut {
    asked: u64,
    used: u64,
    asked_wire: usize,
    used_wire: usize,
}

/// `value` with the `truncated` object saying how it was cut. `over`: the subtree asked for passed
/// the limit (else the window is only the `offset` asked for). `children`: the window, when the
/// children were cut as well as (or instead of) the depth.
fn with_marker(mut value: Value, cut: Cut, max_bytes: usize, over: bool, children: Option<Value>) -> Value {
    let Cut { asked, used, asked_wire, used_wire } = cut;
    if let Some(obj) = value.as_object_mut() {
        let mut marker = json!({
            "requestedDepth": asked,
            "depth": used,
            "requestedDepthBytes": asked_wire,
            "bytes": used_wire,
            "maxBytes": max_bytes,
        });
        match &children {
            Some(window) => {
                marker["children"] = window.clone();
                let from = window.get("from").and_then(Value::as_u64).unwrap_or(0);
                marker["reason"] = json!(if over {
                    format!(
                        "the subtree at depth {asked} would be {asked_wire} bytes on the wire, over the {max_bytes}-byte limit a client can read in one message; its children are reported from {from} instead"
                    )
                } else {
                    format!(
                        "its children are reported from {from}, the `offset` asked for; the subtree at depth {asked} ({asked_wire} bytes) is within the {max_bytes}-byte limit"
                    )
                });
                marker["hint"] = json!("ask again with the `offset` in `children.nextOffset` for the rest, or get_node({id, depth}) on one child");
            }
            None => {
                marker["reason"] = json!(format!(
                    "the subtree at depth {asked} would be {asked_wire} bytes on the wire, over the {max_bytes}-byte limit a client can read in one message; it is reported at depth {used} instead"
                ));
                marker["hint"] = json!("get_node({id, depth}) on a child reads deeper, one subtree at a time");
            }
        }
        obj.insert("truncated".into(), marker);
    }
    value
}

fn select(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let ids = ids_arg(a, "ids")?.ok_or("`ids` is required (an empty array deselects)")?;
    if ids.is_empty() {
        exec(b, "select.none", json!({}))?;
    } else {
        exec(b, "select.set", json!({"ids": ids}))?;
    }
    let sel = b.call("document.inspect", json!({"depth": 0}))?.get("selection").cloned().unwrap_or(Value::Null);
    Ok(ToolResult::json(&json!({"selection": sel})))
}

fn export(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let mut params = Map::new();
    if let Some(ids) = ids_arg(a, "ids")? {
        params.insert("ids".into(), json!(ids));
    }
    let format = s_arg(a, "format")?.map(str::to_ascii_lowercase);
    if let Some(f) = &format {
        if !["png", "jpg", "jpeg", "svg", "webp", "pdf", "xml", "fig"].contains(&f.as_str()) {
            return Err(format!("unknown format `{f}` (png|jpg|svg|webp|pdf|xml|fig)"));
        }
        params.insert("format".into(), json!(f));
    }
    if let Some(s) = n_arg(a, "scale")? {
        if s <= 0.0 {
            return Err("`scale` must be positive".into());
        }
        params.insert("scale".into(), json!(s));
    }
    if let Some(s) = s_arg(a, "size")? {
        params.insert("size".into(), json!(s));
    }
    // PDF print options and the export options, checked by the command.
    for k in [
        "bleed",
        "cropMarks",
        "bleedMarks",
        "cmyk",
        "contentsOnly",
        "includeBoundingBox",
        "svgOutlineText",
        "svgIdAttribute",
        "svgSimplifyStroke",
        "vdCompat",
    ] {
        if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
            params.insert(k.into(), v.clone());
        }
    }
    if let Some(p) = s_arg(a, "path")? {
        params.insert("path".into(), json!(p));
        return exec(b, "file.export", Value::Object(params)).map(|v| ToolResult::json(&v));
    }
    let r = exec(b, "file.export", Value::Object(params))?;
    let fmt = r.get("format").and_then(Value::as_str).unwrap_or("png").to_string();
    let bytes = decode_data(&r)?;
    match fmt.as_str() {
        "png" => {
            let (w, h) = png_info(&bytes);
            Ok(ToolResult::image(&bytes, "image/png", &json!({"format": "png", "width": w, "height": h, "bytes": bytes.len()})))
        }
        "jpg" | "jpeg" => Ok(ToolResult::image(&bytes, "image/jpeg", &json!({"format": "jpg", "bytes": bytes.len()}))),
        "webp" => Ok(ToolResult::image(&bytes, "image/webp", &json!({"format": "webp", "bytes": bytes.len()}))),
        "svg" => Ok(ToolResult::text(String::from_utf8_lossy(&bytes).into_owned())),
        "xml" => {
            let info = json!({"format": "xml", "bytes": bytes.len(), "warnings": r.get("warnings").cloned().unwrap_or_else(|| json!([]))});
            let mut out = ToolResult::text(String::from_utf8_lossy(&bytes).into_owned());
            out.content.extend(ToolResult::json(&info).content);
            Ok(out)
        }
        other => {
            let mut out = json!({"format": other, "bytes": bytes.len(), "data": base64::engine::general_purpose::STANDARD.encode(&bytes)});
            if let Some(w) = r.get("warnings") {
                out["warnings"] = w.clone();
            }
            Ok(ToolResult::json(&out))
        }
    }
}

fn screenshot(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let id = a.get("window").and_then(Value::as_u64);
    if (id.is_some() || matches!(a.get("window"), Some(Value::Bool(true)))) && b.has_ui() {
        let bytes = match id {
            Some(id) => b.window_png(id)?,
            None => b.screenshot_png()?,
        };
        let (w, h) = png_info(&bytes);
        return Ok(ToolResult::image(&bytes, "image/png", &json!({"source": "window", "width": w, "height": h})));
    }
    let max = n_arg(a, "maxSize")?.unwrap_or(1600.0).clamp(16.0, 8192.0);
    let req = n_arg(a, "scale")?.unwrap_or(1.0);
    if req <= 0.0 {
        return Err("`scale` must be positive".into());
    }
    let ids = ids_arg(a, "ids")?.unwrap_or_default();
    let measured = if ids.is_empty() { page_children(b)? } else { ids.clone() };
    let mut scale = req;
    if let Some((x0, y0, x1, y1)) = bounds_of(b, &measured) {
        let side = (x1 - x0).max(y1 - y0).max(1e-6);
        scale = scale.min(max / side);
    }
    let mut bytes = b.render_png(&ids, scale)?;
    let (mut w, mut h) = png_info(&bytes);
    // Layer bounds leave out effects (shadows) and overflow of unclipped frames, but the render
    // includes them: shrink to the measured image until it fits `maxSize` (rounding can add a pixel).
    for _ in 0..4 {
        let side = f64::from(w.max(h));
        if side <= max {
            break;
        }
        scale *= max / side;
        bytes = b.render_png(&ids, scale)?;
        (w, h) = png_info(&bytes);
    }
    Ok(ToolResult::image(
        &bytes,
        "image/png",
        &json!({"source": if ids.is_empty() { "page" } else { "layers" }, "ids": ids, "scale": scale, "width": w, "height": h}),
    ))
}

fn open_file(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let path = req_s(a, "path")?;
    let mut params = json!({"path": path});
    match a.get("variables") {
        None | Some(Value::Null) => {}
        Some(v @ (Value::String(_) | Value::Object(_))) => params["variables"] = v.clone(),
        Some(_) => return Err("`variables` must be a path or the local variables JSON object".into()),
    }
    if let Some(reload) = a.get("reload").filter(|v| !v.is_null()) {
        params["reload"] = reload.clone();
    }
    let r = exec(b, "file.open", params)?;
    let doc = b.call("document.inspect", json!({"depth": 1}))?;
    Ok(ToolResult::json(&json!({"opened": r, "document": doc})))
}

fn save_file(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let params = match s_arg(a, "path")? {
        Some(p) => json!({"path": p}),
        None => json!({}),
    };
    exec(b, "file.save", params).map(|v| ToolResult::json(&v))
}

fn mods(a: &Map<String, Value>, out: &mut Map<String, Value>) -> Result<(), String> {
    for k in ["shift", "alt", "cmd", "ctrl"] {
        match a.get(k) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(x)) => {
                out.insert(k.into(), json!(x));
            }
            Some(_) => return Err(format!("`{k}` must be a boolean")),
        }
    }
    Ok(())
}

fn pointer(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let x = n_arg(a, "x")?.ok_or("`x` is required")?;
    let y = n_arg(a, "y")?.ok_or("`y` is required")?;
    let kind = s_arg(a, "kind")?.unwrap_or("click");
    if let Some(t) = s_arg(a, "tool")? {
        exec(b, "tool.set", json!({"tool": t}))?;
    }
    let mut p = Map::new();
    p.insert("kind".into(), json!(kind));
    p.insert("x".into(), json!(x));
    p.insert("y".into(), json!(y));
    if let Some(to) = a.get("to").filter(|v| !v.is_null()) {
        let arr =
            to.as_array().filter(|t| t.len() == 2 && t.iter().all(|v| figcraft_engine::params::f64_of(v).is_some())).ok_or("`to` must be [x, y]")?;
        p.insert("to".into(), Value::Array(arr.clone()));
    }
    if let Some(s) = n_arg(a, "steps")? {
        p.insert("steps".into(), json!(s.clamp(1.0, 200.0) as u64));
    }
    for k in ["pressure", "pressureTo"] {
        if let Some(v) = n_arg(a, k)? {
            p.insert(k.into(), json!(v.clamp(0.0, 1.0)));
        }
    }
    mods(a, &mut p)?;
    b.call("ui.pointer", Value::Object(p)).map(|v| ToolResult::json(&v))
}

fn key(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let k = req_s(a, "key")?;
    let mut p = Map::new();
    p.insert("key".into(), json!(k));
    mods(a, &mut p)?;
    b.call("ui.key", Value::Object(p)).map(|v| ToolResult::json(&v))
}

fn type_text(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let text = req_s(a, "text")?;
    match a.get("id").filter(|v| !v.is_null()) {
        Some(id) => {
            let id = id_arg(id)?;
            exec(b, "text.edit.begin", json!({"id": id}))?;
            let typed = b.call("ui.type", json!({"text": text}));
            // Always leave edit mode, even when typing failed.
            let ended = exec(b, "text.edit.end", json!({}));
            typed?;
            ended?;
            let node = b.call("node.get", json!({"id": id, "depth": 0}))?;
            Ok(ToolResult::json(&json!({"ok": true, "node": node})))
        }
        None => b.call("ui.type", json!({"text": text})).map(|v| ToolResult::json(&v)),
    }
}
