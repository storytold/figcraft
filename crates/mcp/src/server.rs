//! JSON-RPC 2.0 framing and the MCP lifecycle / tools / resources / prompts methods.

use std::io::{BufRead, Read, Write};
use std::sync::Arc;

use figcraft_engine::file_access::{Roots, confined};

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::prompts;
use crate::resources;
use crate::tools::{call_tool, tool_definitions};

/// The MCP revision we implement.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Revisions we also accept from clients.
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const RESOURCE_NOT_FOUND: i64 = -32002;

/// The longest line we accept (32 MiB); longer lines are rejected without parsing.
const MAX_LINE: usize = 32 * 1024 * 1024;

/// The `instructions` string sent on initialize.
pub const INSTRUCTIONS: &str = "FigCraft is an interface design tool (Figma-class): pages hold frames, \
groups, shapes, text, components and instances, with auto layout, styles, variables and prototyping. \
Coordinates are page units (1 unit = 1 px at 100% zoom), x right and y DOWN. create_node and set_properties \
take x/y RELATIVE TO THE PARENT (the page when there is no parent, so page coordinates); inspect_document \
reports both x/y relative to the parent frame (groups don't count) and absoluteBounds in page coordinates. Children of an auto-layout frame are \
positioned by the layout (set layout, gap, padding, mainAlign, crossAlign, sizingH/sizingV instead). \
Every menu item, panel action and shortcut is a command: find ids and params with list_commands (use a filter) \
and run them with run_command, or several as one undo step with batch. Commands act on `ids` when given, else \
on the selection; new layers become the selection. Colours are \"#RRGGBB\" or \"#RRGGBBAA\". Verify your work: \
inspect_document / get_node return the layer tree as JSON (check sizes, fills, layout, overrides), and \
screenshot returns a PNG of the page or of given layers to look at. undo/redo step through history. \
Comments: comment.list returns threads (text, anchor node id, frame, page, x/y, replies); fix open ones, \
reply with what changed, then resolve (a reply reopens a thread). To react to new feedback, call \
wait_for_changes {after, timeoutMs} and pass the returned cursor back. \
Resources: figcraft://document, figcraft://node/{id}, figcraft://command/{id}, figcraft://comments. \
prompts/list has workflows, including fix_comments.";

/// An MCP server bound to one backend.
pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
    /// The automation roots every request's file access is confined to (`--automation-read-root`,
    /// `--automation-write-root`), if any.
    roots: Option<Arc<Roots>>,
    /// The client's name from `initialize` (`clientInfo.title`, else `.name`; empty until then):
    /// every command runs as this AI agent, so its comments are signed with it, not the user's name.
    agent: String,
}

/// The backend as the MCP client's AI agent: `engine.execute` carries its name (`agent`).
struct AsAgent<'a> {
    inner: &'a mut dyn Backend,
    agent: &'a str,
}

impl Backend for AsAgent<'_> {
    fn call(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if method == "engine.execute"
            && let Some(p) = params.as_object_mut()
        {
            p.insert("agent".into(), json!(self.agent));
        }
        self.inner.call(method, params)
    }

    fn has_ui(&self) -> bool {
        self.inner.has_ui()
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }

    fn render_png(&mut self, ids: &[u64], scale: f64) -> Result<Vec<u8>, String> {
        self.inner.render_png(ids, scale)
    }

    fn screenshot_png(&mut self) -> Result<Vec<u8>, String> {
        self.inner.screenshot_png()
    }

    fn window_png(&mut self, window: u64) -> Result<Vec<u8>, String> {
        self.inner.window_png(window)
    }
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// The name an `initialize` request gives its client (`clientInfo.title`, else `.name`; empty
/// without one). The engine keeps 64 characters of it; this only bounds what each call carries.
fn client_name(params: &Value) -> String {
    let name = |k: &str| params.get("clientInfo").and_then(|i| i.get(k)).and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
    name("title").or_else(|| name("name")).unwrap_or_default().chars().take(256).collect()
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(ToString::to_string).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, initialized: false, roots: None, agent: String::new() }
    }

    /// Confine the files every request reads and writes to `roots` (see
    /// [`figcraft_engine::file_access`]).
    pub fn with_roots(mut self, roots: Option<Roots>) -> Self {
        self.roots = roots.map(Arc::new);
        self
    }

    /// A server over a fresh headless session.
    pub fn headless() -> Self {
        Self::new(Box::new(crate::Headless::new()))
    }

    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. stdout is the protocol stream; logs
    /// go to stderr. An oversized frame gets an error and closes the stream without draining it.
    pub fn serve(&mut self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        let mut line = Vec::new();
        loop {
            line.clear();
            if (&mut input).take((MAX_LINE + 1) as u64).read_until(b'\n', &mut line)? == 0 {
                break;
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.len() > MAX_LINE {
                writeln!(output, "{}", error(Value::Null, INVALID_REQUEST, "message too large"))?;
                output.flush()?;
                break;
            }
            let text = String::from_utf8_lossy(&line);
            if let Some(reply) = self.handle_line(&text) {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one line; returns the reply line (None for notifications and blank lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        if line.len() > MAX_LINE {
            return Some(error(Value::Null, INVALID_REQUEST, "message too large").to_string());
        }
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                // Batches were removed in 2025-06-18; still answer older clients sensibly.
                if batch.is_empty() {
                    Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
                } else {
                    let replies: Vec<Value> = batch.into_iter().take(1000).filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one JSON-RPC message; `None` for notifications and responses.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else { return Some(error(Value::Null, INVALID_REQUEST, "message must be a JSON object")) };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a server→client request (we send none): ignore.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.filter(|i| i.is_string() || i.is_number()).unwrap_or(Value::Null), INVALID_REQUEST, "missing `method` (a string)"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            self.notification(method);
            return None;
        };
        if !(id.is_string() || id.is_number()) {
            return Some(error(Value::Null, INVALID_REQUEST, "`id` must be a string or number"));
        }
        if !(params.is_object() || params.is_null()) {
            return Some(error(id, INVALID_PARAMS, "`params` must be an object"));
        }
        // A bug that panics fails this request only; the server keeps serving.
        let roots = self.roots.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| confined(roots.as_ref(), || self.request(method, &params)))).unwrap_or_else(
            |p| Err((INTERNAL_ERROR, format!("internal error in `{method}`: {} (please report this bug)", panic_message(p.as_ref())))),
        );
        Some(match r {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    /// What `initialize` says about the automation roots.
    fn roots_note(&self) -> String {
        let Some(r) = &self.roots else { return String::new() };
        let j = r.to_json();
        format!(" Files: only these folders may be read: {}; written: {} (any other path is refused).", j["read"], j["write"])
    }

    fn notification(&mut self, method: &str) {
        match method {
            "notifications/initialized" => self.initialized = true,
            "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {}
            other => log::debug!("ignoring notification {other}"),
        }
    }

    /// The backend, acting as the client's agent.
    fn as_agent(&mut self) -> AsAgent<'_> {
        AsAgent { inner: self.backend.as_mut(), agent: &self.agent }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                self.agent = client_name(params);
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {
                        "tools": {"listChanged": false},
                        "resources": {"subscribe": false, "listChanged": false},
                        "prompts": {"listChanged": false},
                    },
                    "serverInfo": {"name": "figcraft", "title": "FigCraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{INSTRUCTIONS} Backend: {}.{}", self.backend.describe(), self.roots_note()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name` (a string)".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                if !(args.is_object() || args.is_null()) {
                    return Err((INVALID_PARAMS, "`arguments` must be an object".into()));
                }
                if !crate::tools::TOOL_NAMES.contains(&name) {
                    return Err((INVALID_PARAMS, format!("unknown tool `{name}`")));
                }
                Ok(call_tool(&mut self.as_agent(), name, &args).to_value())
            }
            "resources/list" => Ok(resources::list()),
            "resources/templates/list" => Ok(resources::templates()),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing `uri` (a string)".to_string()))?;
                let v = resources::read(&mut self.as_agent(), uri).map_err(|e| match e {
                    resources::ReadError::NotFound(uri) => (RESOURCE_NOT_FOUND, format!("resource not found: {uri}")),
                    resources::ReadError::Backend(why) => (INTERNAL_ERROR, why),
                })?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}))
            }
            "prompts/list" => Ok(prompts::list()),
            "prompts/get" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing prompt `name` (a string)".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                prompts::get(name, &args).map_err(|e| (INVALID_PARAMS, e))
            }
            "logging/setLevel" => Ok(json!({})),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }
}
