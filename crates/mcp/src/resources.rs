//! MCP resources: the document, one layer, one command and the open comments.

use serde_json::{Value, json};

use crate::backend::Backend;

pub const DOC_URI: &str = "figcraft://document";
pub const COMMANDS_URI: &str = "figcraft://commands";
pub const COMMENTS_URI: &str = "figcraft://comments";
pub const NODE_TEMPLATE: &str = "figcraft://node/{id}";
pub const COMMAND_TEMPLATE: &str = "figcraft://command/{id}";

pub enum ReadError {
    NotFound(String),
    Backend(String),
}

pub fn list() -> Value {
    json!({"resources": [
        {"uri": DOC_URI, "name": "document", "title": "Active document", "description": "The active document as JSON (current page's layer tree, pages, styles, selection, history), like inspect_document.", "mimeType": "application/json"},
        {"uri": COMMANDS_URI, "name": "commands", "title": "Command catalog", "description": "Every FigCraft command with label, menu, shortcut and parameter docs.", "mimeType": "application/json"},
        {"uri": COMMENTS_URI, "name": "comments", "title": "Open comments", "description": "Every open comment thread on every page, as JSON (the same objects as comment.list).", "mimeType": "application/json"},
    ]})
}

pub fn templates() -> Value {
    json!({"resourceTemplates": [
        {"uriTemplate": NODE_TEMPLATE, "name": "node", "title": "Layer", "description": "One layer and its children (depth 2) as JSON.", "mimeType": "application/json"},
        {"uriTemplate": COMMAND_TEMPLATE, "name": "command", "title": "Command", "description": "One command's label, menu path, shortcut and parameter docs, e.g. figcraft://command/layout.add.", "mimeType": "application/json"},
    ]})
}

pub fn read(b: &mut dyn Backend, uri: &str) -> Result<Value, ReadError> {
    if uri == DOC_URI {
        return b.call("document.inspect", json!({"depth": 8})).map_err(ReadError::Backend);
    }
    if uri == COMMANDS_URI {
        return b.call("engine.commands", json!({})).map_err(ReadError::Backend);
    }
    if uri == COMMENTS_URI {
        return b.call("engine.execute", json!({"command": "comment.list", "params": {"page": "all", "filter": "open"}})).map_err(ReadError::Backend);
    }
    if let Some(id) = uri.strip_prefix("figcraft://node/") {
        let Some(id) = figcraft_engine::params::id_of(&Value::String(id.to_string())) else { return Err(ReadError::NotFound(uri.into())) };
        return match b.call("node.get", json!({"id": id.0, "depth": 2})) {
            Ok(Value::Null) => Err(ReadError::NotFound(uri.into())),
            Ok(v) => Ok(v),
            Err(e) if e.contains("no such node") => Err(ReadError::NotFound(uri.into())),
            Err(e) => Err(ReadError::Backend(e)),
        };
    }
    if let Some(id) = uri.strip_prefix("figcraft://command/") {
        let all = b.call("engine.commands", json!({})).map_err(ReadError::Backend)?;
        return all
            .as_array()
            .and_then(|a| a.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id)).cloned())
            .ok_or_else(|| ReadError::NotFound(uri.into()));
    }
    Err(ReadError::NotFound(uri.into()))
}
