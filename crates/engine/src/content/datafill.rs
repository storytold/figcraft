//! `data.fill`: one frame repeated per row of data (a carousel, a set of posts, cards from a
//! list): each copy gets its row's texts, colours and visibility on the layers named by the row's
//! keys, in one undo step, and optionally each copy exported as a file. Agents make a series with
//! one call instead of a duplicate, a lookup and an edit per layer and an export per copy.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{get, id, id_of, num_or, string};
use crate::{EngineError, Result, Session, bad, spec};

const CMD: &str = "data.fill";
const MAX_ROWS: usize = 500;

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "data.fill",
        "Fill copies with data",
        "{id (the frame to repeat), rows: [{<layer name>: text | {text?, fill? (#rrggbb), visible?, fit? (true or text.fit's {maxLines?, minSize?, orphans?}: size the text to its box)}, name? (the copy's name)}] (every layer with that name in the copy; up to 500 rows), gap? (between copies, placed in a row to the right; default 100), formats? ([{name?, width, height}]: each row once per size, the copy resized so its auto layout reflows and named <name>-<format name>; e.g. feed 1080×1350, story 1080×1920, square 1080×1080), dir? (export each copy there as <name>.<format>), format? (png|jpg|webp|svg|pdf; default png), scale? (default 1)}  one undo step  → {ids (the copies, in row order), missing (keys no layer is named), files? (paths written, one per copy: same file names get a numbered suffix), errors? ([{id, path, error}] for exports that failed; the copies stay)}",
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
    // Refused before any copy or file is made: a rollback afterwards wouldn't take files back.
    if let Some(why) = s.live_read_only(s.ds()?.uid) {
        return Err(EngineError::Disabled(CMD.into(), why.into()));
    }
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
    let formats = formats(p)?;
    let column = formats.iter().flatten().map(|f| f.1).fold(width, f64::max);
    let grid = Grid { src, base, column, gap };
    let mut missing: Vec<String> = Vec::new();
    let ids = s.grouped("Fill copies with data", |s| {
        let mut ids = Vec::with_capacity(rows.len() * formats.len());
        for (i, row) in rows.iter().enumerate() {
            let mut dy = 0.0;
            for fmt in &formats {
                if let Some(made) = copy_row(s, &grid, (i, row), fmt.as_ref(), &mut dy, &mut missing)? {
                    ids.push(made);
                }
            }
        }
        Ok(ids)
    })?;
    let mut out = json!({"ids": ids.iter().map(|(i, _)| i.0).collect::<Vec<_>>(), "missing": missing});
    // Exports after the edits landed: a failed edit writes no files. A failed export keeps the
    // copies (one undo step removes them) and is reported with the files that were written.
    if let Some(dir) = string(p, "dir") {
        export_copies(s, &ids, &dir, (&format, num_or(p, "scale", 1.0)), &mut out);
    }
    Ok(out)
}

/// A size the copies are made in: its name, width and height.
type Format = (String, f64, f64);

/// `formats`: each row once per size, the copy resized so its auto layout reflows (`None`: the
/// frame's own size).
fn formats(p: &Value) -> Result<Vec<Option<Format>>> {
    match get(p, "formats") {
        None | Some(Value::Null) => Ok(vec![None]),
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 12 => a.iter().map(format_of).collect(),
        Some(_) => Err(bad(CMD, "formats: [{name?, width, height}] (1 to 12 sizes)")),
    }
}

/// One `formats` entry, named `<width>x<height>` when it has no name.
fn format_of(f: &Value) -> Result<Option<Format>> {
    let (w, h) = (f.get("width").and_then(Value::as_f64), f.get("height").and_then(Value::as_f64));
    let name = f.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    match (w, h) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && (1.0..=20_000.0).contains(&w) && (1.0..=20_000.0).contains(&h) => {
            Ok(Some((if name.is_empty() { format!("{}x{}", w as i64, h as i64) } else { name }, w, h)))
        }
        _ => Err(bad(CMD, "formats: [{name?, width, height}] with sizes from 1 to 20000")),
    }
}

/// Where the copies go: the frame repeated, its name, the column width and the gap.
struct Grid {
    src: NodeId,
    base: String,
    column: f64,
    gap: f64,
}

/// Row `i` in format `fmt`: duplicate the frame (rows go right; a row's formats stack downwards
/// from `dy`), name the copy and fill its layers. The copy and its name.
fn copy_row(
    s: &mut Session,
    g: &Grid,
    (i, row): (usize, &Value),
    fmt: Option<&Format>,
    dy: &mut f64,
    missing: &mut Vec<String>,
) -> Result<Option<(NodeId, String)>> {
    let dx = (g.column + g.gap) * (i as f64 + 1.0);
    let dup = s.execute("edit.duplicate", &json!({"ids": [g.src], "dx": dx, "dy": *dy}))?;
    let copy = dup["ids"].get(0).and_then(id_of).ok_or_else(|| bad(CMD, "the frame couldn't be duplicated"))?;
    let Some(fields) = row.as_object() else { return Ok(None) };
    let name = fields.get("name").and_then(Value::as_str).map_or_else(|| format!("{} {}", g.base, i + 1), str::to_string);
    let name = match fmt {
        Some((suffix, w, h)) => {
            s.execute("node.set", &json!({"id": copy, "props": {"width": w, "height": h}}))?;
            *dy += h + g.gap;
            format!("{name}-{suffix}")
        }
        None => name,
    };
    s.execute("node.set", &json!({"id": copy, "props": {"name": name}}))?;
    for (key, value) in fields.iter().filter(|(k, _)| k.as_str() != "name") {
        fill_layers(s, copy, (key, value), missing)?;
    }
    Ok(Some((copy, name)))
}

/// Give every layer named `key` in `copy` the row's `value`: its text, fill, visibility and fit.
/// A key no layer has goes on `missing` (once).
fn fill_layers(s: &mut Session, copy: NodeId, (key, value): (&str, &Value), missing: &mut Vec<String>) -> Result<()> {
    let doc = s.doc()?;
    let targets: Vec<_> = doc.subtree(copy).into_iter().filter(|t| *t != copy && doc.node(*t).is_some_and(|n| n.name == key)).collect();
    if targets.is_empty() {
        if !missing.iter().any(|m| m == key) {
            missing.push(key.to_string());
        }
        return Ok(());
    }
    let (text, fill, visible, fit) = match value {
        Value::Object(o) => (
            o.get("text").map(text_of),
            o.get("fill").and_then(Value::as_str),
            o.get("visible").and_then(Value::as_bool),
            o.get("fit").cloned().filter(|f| !matches!(f, Value::Bool(false) | Value::Null)),
        ),
        v => (Some(text_of(v)), None, None, None),
    };
    let mut texts = Vec::new();
    for t in targets {
        let is_text = s.doc()?.node(t).is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Text(_)));
        if let Some(text) = text.as_ref().filter(|_| is_text) {
            s.execute("text.set", &json!({"id": t, "characters": text}))?;
        }
        if is_text {
            texts.push(t.0);
        }
        if let Some(f) = fill {
            s.execute("node.set", &json!({"id": t, "props": {"fills": [f]}}))?;
        }
        if let Some(v) = visible {
            s.execute("node.set", &json!({"id": t, "props": {"visible": v}}))?;
        }
    }
    // `fit: true` or `{maxLines?, minSize?, orphans?}`: size the text to its box.
    if let Some(f) = fit.filter(|_| !texts.is_empty()) {
        let mut params = if f.is_object() { f } else { json!({}) };
        params["ids"] = json!(texts);
        s.execute("text.fit", &params)?;
    }
    Ok(())
}

/// Export each copy into `dir` as `<name>.<format>` at `scale`, adding `files` (and `errors`) to
/// `out`.
fn export_copies(s: &mut Session, ids: &[(NodeId, String)], dir: &str, (format, scale): (&str, f64), out: &mut Value) {
    let (mut files, mut errors, mut used) = (Vec::new(), Vec::new(), Vec::<String>::new());
    for (copy, name) in ids {
        // One file per copy: same or same-looking names get " 2", " 3"… (case-insensitively,
        // as macOS and Windows file systems compare names).
        let base = file_name(name);
        let stem =
            (1..).map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") }).find(|n| !used.contains(&n.to_lowercase())).unwrap_or(base);
        used.push(stem.to_lowercase());
        let path = std::path::Path::new(dir).join(format!("{stem}.{format}")).to_string_lossy().to_string();
        match s.execute("file.export", &json!({"ids": [copy.0], "format": format, "scale": scale, "path": path})) {
            Ok(_) => files.push(path),
            Err(e) => errors.push(json!({"id": copy.0, "path": path, "error": e.to_string()})),
        }
    }
    out["files"] = json!(files);
    if !errors.is_empty() {
        out["errors"] = json!(errors);
    }
}

/// A cell as text: strings as they are, numbers and booleans as written, null as empty.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
