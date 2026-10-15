//! CraftJam boards' edits: changing a shape's kind in place (its text and connectors stay).

use figcraft_color::Color;
use figcraft_doc::{NodeId, NodeKind, shape_text};
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
            "sticky.create",
            "Sticky note",
            "{x?, y? (its centre, page coordinates; default the middle of the view), color? (a sticky colour: yellow, orange, red, pink, violet, blue, teal, green, gray, white; or #hex), wide?, text?, author? (default your name; \"\" for none), parent?} → {id, text}",
            true,
            sticky_create
        ),
        spec!("sticky.set", "Sticky note options", "{ids?, wide?, author? (true: your name, false: none, or a name), color?}", true, sticky_set),
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

/// A sticky colour by name (`yellow`…) or as a CSS colour.
fn sticky_color(v: &str) -> Option<Color> {
    let k = v.trim().to_ascii_lowercase();
    crate::board::STICKY_COLORS.iter().find(|s| s.name.eq_ignore_ascii_case(&k)).map(|s| s.color).or_else(|| Color::parse_css(v.trim()))
}

fn sticky_create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sticky.create";
    let color = match string(p, "color") {
        Some(c) => sticky_color(&c).ok_or_else(|| bad(cmd, "`color` is a sticky colour or #hex"))?,
        None => crate::board::STICKY_COLORS.get(s.tool.sticky_color).or(crate::board::STICKY_COLORS.first()).map(|c| c.color).unwrap_or(Color::WHITE),
    };
    let wide = boolean(p, "wide").unwrap_or(false);
    let author = string(p, "author").unwrap_or_else(|| s.prefs.user_name.clone());
    let page = s.page()?;
    let parent = id(p, "parent").unwrap_or(page);
    let centre = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => figcraft_geom::Point::new(x, y),
        _ => {
            let v = &s.ds()?.view;
            figcraft_geom::Point::new(v.cx, v.cy)
        }
    };
    if !(centre.x.is_finite() && centre.y.is_finite()) || centre.x.abs() > 1e7 || centre.y.abs() > 1e7 {
        return Err(bad(cmd, "the note's place is out of range"));
    }
    let w = if wide { 2.0 * crate::board::STICKY_SIZE } else { crate::board::STICKY_SIZE };
    let doc = s.doc_mut()?;
    if !doc.get(parent)?.kind.is_container() {
        return Err(bad(cmd, format!("{parent} can't contain layers")));
    }
    let space = doc.world_transform(parent).inverse();
    let at = space * figcraft_geom::Point::new(centre.x - w / 2.0, centre.y - crate::board::STICKY_SIZE / 2.0);
    let nid = doc.alloc_id();
    doc.insert(crate::board::new_sticky(nid, at.x, at.y, color, wide, &author), parent, None)?;
    s.set_selection(vec![nid]);
    let mut text = None;
    if let Some(t) = string(p, "text") {
        crate::textedit::edit_shape_text(s, nid, None)?;
        crate::textedit::insert(s, &t)?;
        text = s.ds()?.text_edit.as_ref().map(|t| t.id);
        crate::textedit::end(s)?;
    }
    Ok(json!({"id": nid, "text": text}))
}

fn sticky_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sticky.set";
    let ids = req_targets(cmd, s, p)?;
    let me = s.prefs.user_name.clone();
    let color = match string(p, "color") {
        Some(c) => Some(sticky_color(&c).ok_or_else(|| bad(cmd, "`color` is a sticky colour or #hex"))?),
        None => None,
    };
    let doc = s.doc_mut()?;
    let mut done: Vec<NodeId> = Vec::new();
    for i in ids {
        let i = shape_text::click_target(doc, i);
        let n = doc.get_mut(i)?;
        if !shape_text::is_sticky(n) {
            continue;
        }
        if let Some(w) = boolean(p, "wide") {
            n.width = if w { 2.0 * crate::board::STICKY_SIZE } else { crate::board::STICKY_SIZE };
        }
        match p.get("author") {
            Some(Value::Bool(true)) => {
                n.plugin_data.insert(shape_text::STICKY.into(), me.clone());
            }
            Some(Value::Bool(false)) => {
                n.plugin_data.insert(shape_text::STICKY.into(), String::new());
            }
            Some(Value::String(a)) => {
                n.plugin_data.insert(shape_text::STICKY.into(), a.trim().chars().take(100).collect());
            }
            _ => {}
        }
        if let Some(c) = color {
            n.fills = vec![figcraft_color::Paint::solid(c)];
        }
        done.push(i);
    }
    if done.is_empty() {
        return Err(bad(cmd, "nothing selected is a sticky note"));
    }
    Ok(json!({"ids": done}))
}
