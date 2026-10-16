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
];

/// All tools, in `tools/list` order.
pub fn tool_definitions() -> Vec<Value> {
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
            "Run several commands in order as ONE undo step (engine `command.batch`). If any command fails, everything is rolled back and the error names the failing step. Returns an array of the results in order. A later step can use a layer an earlier step created: anywhere in its params, {\"$step\": n} stands for the `id` in step n's result (zero-based; {\"$step\": n, \"field\": \"…\"} picks another field). Example: [{\"command\":\"node.create\",\"params\":{\"type\":\"frame\"}}, {\"command\":\"node.create\",\"params\":{\"type\":\"text\",\"text\":\"Hi\",\"parent\":{\"$step\":0}}}] builds a frame and a text inside it in one call.",
            obj(
                json!({
                    "commands": {"type": "array", "minItems": 1, "description": "Steps to run", "items": obj(json!({"command": string("Command id"), "params": {"type": "object"}}), &["command"])},
                    "label": string("Undo history label (default \"Batch\")"),
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
                    "width": num("Width (default 100; text hugs its content unless width is set, which makes it wrap)"),
                    "height": num("Height (default 100; ignored for lines/arrows and auto-height text)"),
                    "parent": id_schema("Parent frame/group/component id (default: the current page)"),
                    "index": int("Z-index among the parent's children (default: on top)"),
                    "name": string("Layer name"),
                    "text": string("Characters, for type text"),
                    "path": string("SVG path data, for type vector (e.g. \"M0 0 L100 0 L50 80 Z\")"),
                    "props": {"type": "object", "description": "Any set_properties keys applied at creation, e.g. {\"fill\":\"#3366FF\",\"cornerRadius\":12,\"fontSize\":24,\"layout\":\"vertical\",\"gap\":8,\"padding\":16}"},
                }),
                &["type"],
            ),
            false,
        ),
        tool(
            "set_properties",
            "Set properties",
            "Set properties on one or more layers as one undo step (engine `node.set`). Keys: name, visible, locked, opacity (0-1), blend, x, y (relative to the parent frame; groups don't count), width, height, rotation (degrees), fill (colour) / fills (array of paints: \"#hex\" or {\"type\":\"solid\",\"color\":\"#hex\"} or {\"type\":\"linearGradient\",\"stops\":[\"#hex\",\"#hex\"]} or stops [{\"position\":0..1,\"color\":\"#hex\"}], optional from/to handles in the unit box; radialGradient too), stroke / strokes, strokeWeight, strokeAlign (inside|center|outside), cornerRadius (n or [tl,tr,br,bl]), effects (array, e.g. {\"type\":\"dropShadow\",\"color\":\"#00000040\",\"offset\":[0,4],\"radius\":8}), clip, layout (null|horizontal|vertical|grid), gap, padding (n or [t,r,b,l]), mainAlign (min|center|max|spaceBetween|spaceAround|spaceEvenly), crossAlign, wrap, contentAlign (auto|spaceBetween: wrapped lines), sizingH / sizingV (fixed|hug|fill), constraints {horizontal, vertical}, characters, fontFamily, fontStyle, fontSize, lineHeight, letterSpacing, textAlign, textAutoResize. Run list_commands with filter \"node.set\" for the full list. Unknown keys are an error. Returns the updated layers.",
            obj(
                json!({"ids": ids_schema("Layer ids (default: the selection)"), "props": {"type": "object", "description": "Property values to set"}}),
                &["props"],
            ),
            false,
        ),
        tool(
            "inspect_document",
            "Inspect document",
            "The active document as JSON: name, path, dirty flag, current page, selection, the current page's layer tree (id, name, type, x/y relative to the parent frame (groups don't count), width, height, absoluteBounds in page coordinates, fills, strokes, effects, cornerRadius, layout, sizing, text properties, component links, overrides, bound variables), other pages summarised, styles, variable count and the undo history. Use it to verify every change.",
            obj(
                json!({"depth": int("Tree depth for the current page (default 4)"), "allPages": boolean("Expand every page, not only the current one")}),
                &[],
            ),
            true,
        ),
        tool(
            "get_node",
            "Get layer",
            "One layer as JSON (same fields as inspect_document) with its children to `depth`.",
            obj(json!({"id": id_schema("Layer id"), "depth": int("Child depth (default 1)")}), &["id"]),
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
            "Export layers (default: the selection, else every top-level layer of the current page) as png, jpg, svg, webp or pdf at `scale`, or the whole file as `fig` (experimental: written from public format knowledge, with `warnings` for what was left out; other apps may not open every file) (PDF: with `bleed`, `cropMarks`, `bleedMarks` and `cmyk` for print; options `contentsOnly`, `includeBoundingBox`, and for SVG `svgOutlineText`, `svgIdAttribute`, `svgSimplifyStroke`). With `path` the file is written there (absolute path recommended) and the result says where. Without `path`: png/jpg/webp come back as an image content block, svg as text, other formats as base64 `data`.",
            obj(
                json!({
                    "ids": ids_schema("Layers to export (several are composited into one image)"),
                    "format": {"type": "string", "enum": ["png", "jpg", "svg", "webp", "pdf", "fig"], "description": "Format (default png, or from the path's extension)"},
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
            "Open a .figcraft or .json document (FigCraft or REST API file JSON), a .fig file (best effort, with `warnings` for what wasn't imported), an .svg (becomes vectors) or an image (becomes an image layer) as the active document. Returns the new document's summary.",
            obj(
                json!({
                    "path": string("File path"),
                    "variables": {"description": "REST API file JSON only: the local variables JSON (`GET /v1/files/:key/variables/local`) as a path or the JSON object. Default: `<name>.variables.json` or `variables.json` next to the file."},
                }),
                &["path"],
            ),
            false,
        ),
        tool(
            "save_file",
            "Save file",
            "Save the active document as a .figcraft file. `path` is required the first time (an untitled document has no path); later saves reuse it.",
            obj(json!({"path": string("Output .figcraft path")}), &[]),
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
                json!({"key": string("Key name: a letter/digit, Escape, Delete, Backspace, Enter, ArrowLeft…"), "shift": boolean("Shift"), "alt": boolean("Alt/Option"), "cmd": boolean("Cmd/Ctrl")}),
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
    ]
}

// ---------- helpers ----------

fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
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

/// Run one tool. Every failure becomes an `isError` result (never a panic).
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> ToolResult {
    let a = match args_obj(args) {
        Ok(a) => a,
        Err(e) => return ToolResult::error(e),
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
        steps.push(json!({"command": cmd, "params": params}));
    }
    let label = s_arg(a, "label")?.unwrap_or("Batch");
    exec(b, "command.batch", json!({"label": label, "commands": steps})).map(|v| ToolResult::json(&v))
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
    exec(b, "node.set", params)?;
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
    Ok(ToolResult::json(&json!({"ok": true, "nodes": nodes})))
}

fn inspect_document(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let depth = n_arg(a, "depth")?.unwrap_or(4.0).clamp(0.0, 64.0) as u64;
    let all = match a.get("allPages") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(x)) => *x,
        Some(_) => return Err("`allPages` must be a boolean".into()),
    };
    b.call("document.inspect", json!({"depth": depth, "allPages": all})).map(|v| ToolResult::json(&v))
}

fn get_node(b: &mut dyn Backend, a: &Map<String, Value>) -> Result<ToolResult, String> {
    let id = id_arg(a.get("id").ok_or("`id` is required")?)?;
    let depth = n_arg(a, "depth")?.unwrap_or(1.0).clamp(0.0, 64.0) as u64;
    let v = b.call("node.get", json!({"id": id, "depth": depth}))?;
    if v.is_null() {
        return Err(format!("no such layer {id}"));
    }
    Ok(ToolResult::json(&v))
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
        if !["png", "jpg", "jpeg", "svg", "webp", "pdf", "fig"].contains(&f.as_str()) {
            return Err(format!("unknown format `{f}` (png|jpg|svg|webp|pdf|fig)"));
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
    for k in
        ["bleed", "cropMarks", "bleedMarks", "cmyk", "contentsOnly", "includeBoundingBox", "svgOutlineText", "svgIdAttribute", "svgSimplifyStroke"]
    {
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
