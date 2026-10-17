//! Commands that make the document easier to build and check from an agent (MCP, the control
//! channel, scripts): a whole layer tree in one step, a compact outline, text measurement and a
//! layout lint that finds what otherwise takes a screenshot to see.

use std::collections::BTreeMap;

use figcraft_doc::*;
use serde_json::{Map, Value, json};

use super::CommandSpec;
use crate::params::*;
use crate::{Result, Session, bad, spec};

/// Deepest tree `node.createTree` builds, and the most layers it creates in one call.
const TREE_MAX_DEPTH: usize = 32;
const TREE_MAX_NODES: usize = 5000;
/// Lines `inspect.outline` lists before it says how many it left out.
const OUTLINE_MAX_LINES: usize = 2000;
/// Texts compared pairwise per top-level frame by the overlap rule.
// ponytail: O(n²) per frame, capped; a sweep line if frames with thousands of texts matter.
const LINT_MAX_TEXTS: usize = 1500;
/// Distances under this (page units) are rounding, not defects.
const SLACK: f64 = 0.5;
/// Deepest nesting the lint walks (documents are far shallower; this only bounds the recursion).
const LINT_MAX_DEPTH: usize = 256;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "node.createTree",
            "Create layer tree",
            "{tree: {type, name?, x?, y?, width?, height?, text?, path?, props?, children?: [same]}, parent? (id; default: the page), index?, x?, y?} → {id, ids: {\"Name/Child\": id}} (one undo step; keys of node.create)",
            false,
            create_tree
        ),
        spec!(
            "inspect.outline",
            "Layer outline",
            "{id? (default: the current page), depth? (default 8)} → {outline (one line per layer: id TYPE “name” x,y w×h, auto layout, text), count, truncated}",
            false,
            outline
        ),
        spec!(
            "inspect.lint",
            "Check layout",
            "{id? (default: the current page)} → {issues: [{rule: textOverflow|outsideParent|clipped|textOverlap, id, name, message, by?, other?}], checked}",
            false,
            lint
        ),
        spec!(
            "text.measure",
            "Measure text",
            "{text, width? (wrap at this width; default: one line per paragraph), fontFamily?, fontStyle?, fontSize?, lineHeight?, letterSpacing?, textCase?} → {width, height, lines}",
            false,
            measure
        ),
    ]
}

// ---------- node.createTree ----------

fn create_tree(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "node.createTree";
    let tree = get(p, "tree").filter(|t| t.is_object()).ok_or_else(|| bad(cmd, "`tree` must be a layer: {type, children?, …}"))?.clone();
    let mut root_params = tree.clone();
    for k in ["parent", "index", "x", "y"] {
        if let Some(v) = get(p, k)
            && let Some(o) = root_params.as_object_mut()
        {
            o.entry(k.to_string()).or_insert_with(|| v.clone());
        }
    }
    let mut ids = Map::new();
    let mut count = 0;
    let root = s.grouped("Create layers", |s| build(s, &root_params, "tree", "", 0, &mut count, &mut ids))?;
    s.set_selection(vec![root]);
    Ok(json!({"id": root, "ids": ids}))
}

/// Create `node` and its children; `path` names it in errors, `prefix` in the `ids` map.
fn build(s: &mut Session, node: &Value, path: &str, prefix: &str, depth: usize, count: &mut usize, ids: &mut Map<String, Value>) -> Result<NodeId> {
    let cmd = "node.createTree";
    if depth > TREE_MAX_DEPTH {
        return Err(bad(cmd, format!("{path}: the tree is deeper than {TREE_MAX_DEPTH} levels")));
    }
    *count += 1;
    if *count > TREE_MAX_NODES {
        return Err(bad(cmd, format!("{path}: more than {TREE_MAX_NODES} layers in one tree")));
    }
    let Some(o) = node.as_object() else { return Err(bad(cmd, format!("{path} must be an object"))) };
    let mut params = o.clone();
    let children = params.remove("children");
    params.insert("select".into(), json!(false));
    let id = s.execute("node.create", &Value::Object(params)).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    let id = id_of(&id["id"]).ok_or_else(|| bad(cmd, format!("{path}: node.create returned no id")))?;
    let name = s.doc()?.node(id).map(|n| n.name.clone()).unwrap_or_default();
    // Repeated names get #2, #3… so every layer has its own key.
    let base = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
    let mut key = base.clone();
    let mut n = 2;
    while ids.contains_key(&key) {
        key = format!("{base} #{n}");
        n += 1;
    }
    ids.insert(key.clone(), json!(id));
    match children {
        None | Some(Value::Null) => {}
        Some(Value::Array(kids)) => {
            for (i, kid) in kids.iter().enumerate() {
                let mut kid = kid.clone();
                if let Some(k) = kid.as_object_mut() {
                    k.insert("parent".into(), json!(id));
                }
                build(s, &kid, &format!("{path}.children[{i}]"), &key, depth + 1, count, ids)?;
            }
        }
        Some(_) => return Err(bad(cmd, format!("{path}.children must be an array"))),
    }
    Ok(id)
}

// ---------- inspect.outline ----------

fn root_of(s: &Session, p: &Value, cmd: &str) -> Result<NodeId> {
    match get(p, "id") {
        Some(v) => {
            let id = id_of(v).ok_or_else(|| bad(cmd, "`id` must be a layer id"))?;
            s.doc()?.get(id)?;
            Ok(id)
        }
        None => s.page(),
    }
}

fn outline(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "inspect.outline";
    let root = root_of(s, p, cmd)?;
    let depth = num(p, "depth").filter(|d| d.is_finite() && *d >= 0.0).map_or(8, |d| d.min(64.0) as usize);
    let doc = s.doc()?;
    let mut lines = Vec::new();
    let mut total = 0;
    outline_into(doc, root, 0, depth, &mut lines, &mut total);
    let truncated = total > lines.len();
    if truncated {
        lines.push(format!("… {} more layers (pass `id` or a smaller `depth`)", total - lines.len()));
    }
    Ok(json!({"outline": lines.join("\n"), "count": total, "truncated": truncated}))
}

fn outline_into(doc: &Document, id: NodeId, level: usize, depth: usize, lines: &mut Vec<String>, total: &mut usize) {
    let Some(n) = doc.node(id) else { return };
    *total += 1;
    if lines.len() < OUTLINE_MAX_LINES {
        lines.push(format!("{}{}", "  ".repeat(level), outline_line(doc, n)));
    }
    if level < depth {
        for c in &n.children {
            outline_into(doc, *c, level + 1, depth, lines, total);
        }
    } else if !n.children.is_empty() && lines.len() < OUTLINE_MAX_LINES {
        lines.push(format!("{}… {} children", "  ".repeat(level + 1), n.children.len()));
    }
}

fn outline_line(doc: &Document, n: &Node) -> String {
    let mut line = format!("{} {} “{}”", n.id, n.kind.type_name(), n.name);
    if !n.is_page() {
        let c = doc.frame_relative(n.id).as_coeffs();
        line.push_str(&format!(" {},{} {}×{}", fmt(c[4]), fmt(c[5]), fmt(n.width), fmt(n.height)));
    }
    if let Some(l) = &n.layout {
        let mode = match l.mode {
            LayoutMode::Horizontal => "horizontal",
            LayoutMode::Vertical => "vertical",
            LayoutMode::Grid => "grid",
        };
        line.push_str(&format!(" [{mode} gap {}]", fmt(l.gap)));
    }
    if let NodeKind::Text(t) = &n.kind {
        let chars: String = t.characters.chars().take(40).collect();
        let more = if t.characters.chars().count() > 40 { "…" } else { "" };
        line.push_str(&format!(" text “{}{more}”", chars.replace('\n', "⏎")));
    }
    if !n.visible {
        line.push_str(" hidden");
    }
    line
}

fn round2(v: f64) -> f64 {
    if v.is_finite() { (v * 100.0).round() / 100.0 } else { 0.0 }
}

fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

// ---------- inspect.lint ----------

fn lint(s: &mut Session, p: &Value) -> Result<Value> {
    let root = root_of(s, p, "inspect.lint")?;
    let doc = s.doc()?;
    let mut issues = Vec::new();
    let mut checked = 0;
    lint_into(doc, root, 0, &mut issues, &mut checked);
    // Overlapping texts, within each top-level frame (or the subtree asked for).
    let scopes: Vec<NodeId> = match doc.node(root) {
        Some(n) if n.is_page() => n.children.clone(),
        _ => vec![root],
    };
    for scope in scopes {
        let texts: Vec<(NodeId, figcraft_geom::Rect)> = doc
            .subtree(scope)
            .into_iter()
            .filter(|i| visible_chain(doc, *i))
            .filter_map(|i| doc.node(i).filter(|n| matches!(n.kind, NodeKind::Text(_))).and_then(|_| doc.world_bounds(i)).map(|b| (i, b)))
            .take(LINT_MAX_TEXTS)
            .collect();
        for (a, (ia, ra)) in texts.iter().enumerate() {
            for (ib, rb) in texts.iter().skip(a + 1) {
                let (w, h) = (ra.x1.min(rb.x1) - ra.x0.max(rb.x0), ra.y1.min(rb.y1) - ra.y0.max(rb.y0));
                if w > SLACK && h > SLACK {
                    issues.push(json!({
                        "rule": "textOverlap", "id": ia, "name": name(doc, *ia), "other": ib,
                        "message": format!("text {} “{}” overlaps text {} “{}” by {}×{}", ia, name(doc, *ia), ib, name(doc, *ib), fmt(w), fmt(h)),
                    }));
                }
            }
        }
    }
    Ok(json!({"issues": issues, "checked": checked}))
}

fn visible_chain(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| n.visible) && doc.ancestors(id).iter().all(|a| doc.node(*a).is_none_or(|n| n.visible))
}

fn name(doc: &Document, id: NodeId) -> String {
    doc.node(id).map(|n| n.name.clone()).unwrap_or_default()
}

fn lint_into(doc: &Document, id: NodeId, level: usize, issues: &mut Vec<Value>, checked: &mut usize) {
    let Some(n) = doc.node(id) else { return };
    if !n.visible || level > LINT_MAX_DEPTH {
        return;
    }
    *checked += 1;
    // Text that doesn't fit its own box: cut off, or drawn past it.
    if let NodeKind::Text(t) = &n.kind
        && t.path.is_none()
        && let Some((l, _)) = figcraft_text::layout_node(n)
    {
        let over_h = l.height - n.height;
        let over_w = l.width - n.width;
        if l.truncated || over_h > SLACK || (t.auto_resize != TextAutoResize::WidthAndHeight && over_w > SLACK) {
            let by = over_h.max(over_w).max(0.0);
            issues.push(json!({
                "rule": "textOverflow", "id": id, "name": n.name, "by": round2(by),
                "message": format!("text {} “{}” needs {}×{} but its box is {}×{}{}", id, n.name, fmt(l.width), fmt(l.height), fmt(n.width), fmt(n.height),
                    if l.truncated { " (truncated)" } else { "" }),
            }));
        }
    }
    // Children that reach past a frame: cut off when it clips, drawn outside it when it doesn't.
    if n.kind.is_frame_like()
        && let Some(fb) = doc.world_bounds(id)
    {
        for c in &n.children {
            let Some(cn) = doc.node(*c) else { continue };
            if !cn.visible || cn.child_layout.absolute {
                continue;
            }
            let Some(cb) = doc.world_bounds(*c) else { continue };
            let by = (fb.x0 - cb.x0).max(cb.x1 - fb.x1).max(fb.y0 - cb.y0).max(cb.y1 - fb.y1);
            if by > SLACK {
                let rule = if n.clip { "clipped" } else { "outsideParent" };
                let what = if n.clip { "is cut off by" } else { "reaches outside" };
                issues.push(json!({
                    "rule": rule, "id": c, "name": cn.name, "other": id, "by": round2(by),
                    "message": format!("{} {} “{}” {what} {} {} “{}” by {}", cn.kind.type_name(), c, cn.name, n.kind.type_name(), id, n.name, fmt(by)),
                }));
            }
        }
    }
    for c in &n.children {
        lint_into(doc, *c, level + 1, issues, checked);
    }
}

// ---------- text.measure ----------

fn measure(_s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "text.measure";
    let text = string(p, "text").ok_or_else(|| bad(cmd, "`text` is required"))?;
    let width = match num(p, "width") {
        Some(w) if w.is_finite() && w > 0.0 => Some(w.min(1e6)),
        Some(_) => return Err(bad(cmd, "`width` must be a positive number")),
        None => None,
    };
    let mut t = TextData::new(text);
    // Measured as a text layer would be laid out: wrapping at a fixed width, else hugging its words.
    t.auto_resize = if width.is_some() { TextAutoResize::Height } else { TextAutoResize::WidthAndHeight };
    let len = t.char_count();
    let style_keys: BTreeMap<&str, &Value> = ["fontFamily", "fontStyle", "fontSize", "lineHeight", "letterSpacing", "textCase"]
        .into_iter()
        .filter_map(|k| get(p, k).map(|v| (k, v)))
        .collect();
    // The family first, so a style is checked against the family it will have.
    for k in ["fontFamily", "fontStyle", "fontSize", "lineHeight", "letterSpacing", "textCase"] {
        if let Some(v) = style_keys.get(k) {
            super::props::apply_text_style(cmd, &mut t, 0, len, k, v)?;
        }
    }
    let l = figcraft_text::layout_cached(&t, width, None);
    Ok(json!({"width": round2(l.width), "height": round2(l.height), "lines": l.lines.len()}))
}
