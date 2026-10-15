//! CraftJam boards' edits: changing a shape's kind in place (its text and connectors stay).

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::*;
use crate::{Result, Session, bad, spec};

/// The shape kinds `shape.setType` turns layers into.
pub const SHAPE_TYPES: &str = "rectangle|ellipse|diamond|triangle|star";

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "shape.setType",
        "Change shape",
        "{type: rectangle|ellipse|diamond|triangle|star, ids?}  the same layer as another shape: its fill, border, text and connectors stay",
        true,
        set_type
    )]
}

/// The kind `name` names (`None` for an unknown name).
pub fn kind_of(name: &str) -> Option<NodeKind> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "rectangle" | "square" => NodeKind::Rectangle,
        "ellipse" | "circle" => NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
        "diamond" => NodeKind::Polygon { count: 4 },
        "triangle" => NodeKind::Polygon { count: 3 },
        "star" => NodeKind::Star { count: 5, inner_ratio: 0.38 },
        _ => return None,
    })
}

fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "shape.setType";
    let name = req_string(cmd, p, "type")?;
    let kind = kind_of(&name).ok_or_else(|| bad(cmd, format!("`type` is {SHAPE_TYPES}")))?;
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut done: Vec<NodeId> = Vec::new();
    for i in ids {
        let i = figcraft_doc::shape_text::click_target(doc, i);
        let n = doc.get_mut(i)?;
        if !figcraft_doc::shape_text::can_host(&n.kind) || matches!(n.kind, NodeKind::Vector { .. }) {
            continue;
        }
        n.kind = kind.clone();
        done.push(i);
    }
    if done.is_empty() {
        return Err(bad(cmd, "nothing selected is a shape"));
    }
    Ok(json!({"ids": done}))
}
