//! `data.fill`: one frame repeated per row of data (a carousel, a set of posts, cards from a
//! list): each copy gets its row's texts, colours and visibility on the layers named by the row's
//! keys, in one undo step, and optionally each copy exported as a file. Agents make a series with
//! one call instead of a duplicate, a lookup and an edit per layer and an export per copy.

use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::*;
use crate::{Result, Session, bad, spec};

const CMD: &str = "data.fill";
const MAX_ROWS: usize = 500;

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "data.fill",
        "Fill copies with data",
        "{id (the frame to repeat), rows: [{<layer name>: text | {text?, fill? (#rrggbb), visible?}, name? (the copy's name)}] (every layer with that name in the copy; up to 500 rows), gap? (between copies, placed in a row to the right; default 100), dir? (export each copy there as <name>.<format>), format? (png|jpg|webp|svg|pdf; default png), scale? (default 1)}  one undo step  → {ids (the copies, in row order), missing (keys no layer is named), files? (paths written)}",
        false,
        fill
    )]
}

/// A file name from a layer name: letters, digits, spaces, `-` and `_` kept, the rest `-`.
fn file_name(name: &str) -> String {
    let s: String = name.chars().take(80).map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '-' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "copy".into() } else { s }
}

fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let src = id(p, "id").ok_or_else(|| bad(CMD, "`id` (the frame to repeat) required"))?;
    let rows = match get(p, "rows") {
        Some(Value::Array(a)) if a.iter().all(Value::is_object) && a.len() <= MAX_ROWS => a.clone(),
        _ => return Err(bad(CMD, "rows: an array of objects (up to 500)")),
    };
    let gap = num_or(p, "gap", 100.0);
    if !gap.is_finite() {
        return Err(bad(CMD, "gap: a number"));
    }
    let format = string(p, "format").unwrap_or_else(|| "png".into()).to_ascii_lowercase();
    if !["png", "jpg", "webp", "svg", "pdf"].contains(&format.as_str()) {
        return Err(bad(CMD, "format: png|jpg|webp|svg|pdf"));
    }
    let (width, base) = {
        let doc = s.doc()?;
        let n = doc.node(src).ok_or_else(|| bad(CMD, "no such layer"))?;
        (n.width, n.name.clone())
    };
    let mut missing: Vec<String> = Vec::new();
    let ids = s.grouped("Fill copies with data", |s| {
        let mut ids = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let dx = (width + gap) * (i as f64 + 1.0);
            let dup = s.execute("edit.duplicate", &json!({"ids": [src], "dx": dx, "dy": 0}))?;
            let copy = dup["ids"].get(0).and_then(id_of).ok_or_else(|| bad(CMD, "the frame couldn't be duplicated"))?;
            let Some(fields) = row.as_object() else { continue };
            let name = fields.get("name").and_then(Value::as_str).map_or_else(|| format!("{base} {}", i + 1), str::to_string);
            s.execute("node.set", &json!({"id": copy, "props": {"name": name}}))?;
            for (key, value) in fields.iter().filter(|(k, _)| k.as_str() != "name") {
                let doc = s.doc()?;
                let targets: Vec<_> = doc.subtree(copy).into_iter().filter(|t| *t != copy && doc.node(*t).is_some_and(|n| n.name == *key)).collect();
                if targets.is_empty() {
                    if !missing.contains(key) {
                        missing.push(key.clone());
                    }
                    continue;
                }
                let (text, fill, visible) = match value {
                    Value::Object(o) => {
                        (o.get("text").map(text_of), o.get("fill").and_then(Value::as_str), o.get("visible").and_then(Value::as_bool))
                    }
                    v => (Some(text_of(v)), None, None),
                };
                for t in targets {
                    let is_text = s.doc()?.node(t).is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Text(_)));
                    if let Some(text) = text.as_ref().filter(|_| is_text) {
                        s.execute("text.set", &json!({"id": t, "characters": text}))?;
                    }
                    if let Some(f) = fill {
                        s.execute("node.set", &json!({"id": t, "props": {"fills": [f]}}))?;
                    }
                    if let Some(v) = visible {
                        s.execute("node.set", &json!({"id": t, "props": {"visible": v}}))?;
                    }
                }
            }
            ids.push((copy, name));
        }
        Ok(ids)
    })?;
    let mut out = json!({"ids": ids.iter().map(|(i, _)| i.0).collect::<Vec<_>>(), "missing": missing});
    // Exports after the edits landed: a failed edit writes no files.
    if let Some(dir) = string(p, "dir") {
        let mut files = Vec::new();
        for (copy, name) in &ids {
            let path = std::path::Path::new(&dir).join(format!("{}.{format}", file_name(name)));
            let path = path.to_string_lossy().to_string();
            s.execute("file.export", &json!({"ids": [copy.0], "format": format, "scale": num_or(p, "scale", 1.0), "path": path}))?;
            files.push(path);
        }
        out["files"] = json!(files);
    }
    Ok(out)
}

/// A cell as text: strings as they are, numbers and booleans as written, null as empty.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
