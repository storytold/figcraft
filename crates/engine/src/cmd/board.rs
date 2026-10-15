//! CraftJam boards' edits: changing a shape's kind in place (its text and connectors stay).

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::*;
use crate::{Result, Session, bad, spec};

/// The shape kinds `shape.setType` turns layers into.
pub const SHAPE_TYPES: &str = "rectangle|ellipse|diamond|triangle|star";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "shape.setType",
            "Change shape",
            "{type: rectangle|ellipse|diamond|triangle|star or a library shape (cylinder, document, parallelogram, predefinedProcess…), ids?}  the same layer as another shape: its fill, border, text and connectors stay",
            true,
            set_type
        ),
        spec!(
            "shape.editText",
            "Edit text",
            "{id? (default the one selected shape or connector)}  types inside a shape, or on a connector's line (its label); the text is made when there's none",
            false,
            |s, p| {
                let cmd = "shape.editText";
                let doc = s.doc()?;
                let id = match id(p, "id") {
                    Some(i) => i,
                    None => match s.selection().as_slice() {
                        [one] => figcraft_doc::shape_text::click_target(doc, *one),
                        _ => return Err(bad(cmd, "select one shape or connector")),
                    },
                };
                crate::textedit::edit_shape_text(s, id, None)?;
                Ok(json!({"text": s.ds()?.text_edit.as_ref().map(|t| t.id)}))
            }
        ),
    ]
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
    let kind = kind_of(&name);
    let preset = crate::board_shapes::preset(&name);
    if kind.is_none() && preset.is_none() {
        return Err(bad(cmd, format!("`type` is {SHAPE_TYPES} or a library shape (cylinder, document, parallelogram…)")));
    }
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut done: Vec<NodeId> = Vec::new();
    for i in ids {
        let i = figcraft_doc::shape_text::click_target(doc, i);
        let n = doc.get_mut(i)?;
        // Shapes, and vectors only when they are library shapes (not drawn paths).
        let library = crate::board_shapes::preset_of(n).is_some();
        if !figcraft_doc::shape_text::can_host(&n.kind) || (matches!(n.kind, NodeKind::Vector { .. }) && !library) {
            continue;
        }
        match (&kind, preset) {
            (Some(k), _) => {
                n.kind = k.clone();
                n.plugin_data.remove(crate::board_shapes::KEY);
            }
            (None, Some(pr)) => {
                crate::board_shapes::apply(n, pr);
            }
            (None, None) => continue,
        }
        done.push(i);
    }
    if done.is_empty() {
        return Err(bad(cmd, "nothing selected is a shape"));
    }
    Ok(json!({"ids": done}))
}
