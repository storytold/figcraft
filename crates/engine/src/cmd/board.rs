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
            "mindmap.create",
            "Mind map",
            "{x?, y? (its centre, page coordinates; default the middle of the view), text? (default \"Central topic\"), edit? (start typing in it)} → {id}  a mind map's central topic",
            true,
            mind_create
        ),
        spec!(
            "mindmap.addChild",
            "Add child topic",
            "{id? (default the selected topic), edit?} → {id}  a topic branching from it (Tab, Cmd/Ctrl+Return)",
            true,
            |s, p| mind_add(s, p, true)
        ),
        spec!(
            "mindmap.addSibling",
            "Add sibling topic",
            "{id? (default the selected topic), edit?} → {id}  a topic after it (Return, Cmd/Ctrl+Shift+Return; a child of the central topic)",
            true,
            |s, p| mind_add(s, p, false)
        ),
        spec!(
            "mindmap.collapse",
            "Collapse branch",
            "{id? (default the selected topic), collapsed? (default: toggle)}  hides or shows what branches off it",
            true,
            mind_collapse
        ),
        spec!(
            "board.pasteMermaid",
            "Paste Mermaid",
            "{text (a Mermaid flowchart: graph TD / flowchart LR …), x?, y? (its centre; default the middle of the view)} → {ids, connectors}  shapes with text and connectors, laid out in layers",
            true,
            paste_mermaid
        ),
        spec!(
            "table.create",
            "Table",
            "{rows? (default 3), cols? (default 3), x?, y? (its centre, page coordinates; default the middle of the view), parent?} → {id}  cells in rows and columns, each a rectangle with text",
            true,
            table_create
        ),
        spec!(
            "table.insertRow",
            "Insert row",
            "{id? (a cell; default the selected one), above?}  a row below (or above) the cell's",
            true,
            |s, p| table_edit(s, p, "table.insertRow")
        ),
        spec!(
            "table.insertColumn",
            "Insert column",
            "{id? (a cell; default the selected one), left?}  a column right (or left) of the cell's",
            true,
            |s, p| table_edit(s, p, "table.insertColumn")
        ),
        spec!("table.deleteRow", "Delete row", "{id? (a cell; default the selected one)}", true, |s, p| table_edit(s, p, "table.deleteRow")),
        spec!("table.deleteColumn", "Delete column", "{id? (a cell; default the selected one)}", true, |s, p| table_edit(s, p, "table.deleteColumn")),
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

fn mind_create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "mindmap.create";
    let page = s.page()?;
    let centre = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => figcraft_geom::Point::new(x, y),
        _ => {
            let v = &s.ds()?.view;
            figcraft_geom::Point::new(v.cx, v.cy)
        }
    };
    if !(centre.x.is_finite() && centre.y.is_finite()) || centre.x.abs() > 1e7 || centre.y.abs() > 1e7 {
        return Err(bad(cmd, "the mind map's place is out of range"));
    }
    let text = string(p, "text").unwrap_or_else(|| "Central topic".into());
    let doc = s.doc_mut()?;
    let id = doc.alloc_id();
    let mut root = crate::mindmap::new_topic(id, None, 0, crate::mindmap::BRANCH_COLORS[0]);
    root.transform = figcraft_geom::Affine::translate((centre.x - root.width / 2.0, centre.y - root.height / 2.0));
    doc.insert(root, page, None)?;
    s.set_selection(vec![id]);
    crate::textedit::edit_shape_text(s, id, None)?;
    crate::textedit::insert(s, &text)?;
    if boolean(p, "edit").unwrap_or(false) {
        crate::textedit::key(s, "SelectAll", false, false, false)?;
    } else {
        crate::textedit::end(s)?;
    }
    Ok(json!({"id": id}))
}

/// The topic a mind map command works on: `id`, else the one selected (or the one whose words
/// are being typed).
fn topic_target(cmd: &str, s: &Session, p: &Value) -> Result<NodeId> {
    let doc = s.doc()?;
    let id = match id(p, "id") {
        Some(i) => i,
        None => {
            let editing = s.ds()?.text_edit.as_ref().map(|t| t.id);
            match (editing, s.selection().as_slice()) {
                (Some(t), _) => shape_text::click_target(doc, t),
                (None, [one]) => shape_text::click_target(doc, *one),
                _ => return Err(bad(cmd, "select one mind map topic")),
            }
        }
    };
    if !doc.get(id).is_ok_and(crate::mindmap::is_topic) {
        return Err(bad(cmd, "that's not a mind map topic"));
    }
    Ok(id)
}

fn mind_add(s: &mut Session, p: &Value, child: bool) -> Result<Value> {
    let cmd = if child { "mindmap.addChild" } else { "mindmap.addSibling" };
    let at = topic_target(cmd, s, p)?;
    if s.ds()?.text_edit.is_some() {
        crate::textedit::end(s)?;
    }
    // A collapsed topic opens to show its new child.
    if child && let Ok(n) = s.doc_mut()?.get_mut(at) {
        n.plugin_data.remove(crate::mindmap::COLLAPSED);
    }
    let doc = s.doc_mut()?;
    let new = if child { crate::mindmap::add_child(doc, at) } else { crate::mindmap::add_sibling(doc, at) };
    let new = new.ok_or_else(|| bad(cmd, "can't add a topic there"))?;
    s.set_selection(vec![new]);
    if boolean(p, "edit").unwrap_or(false) {
        crate::textedit::edit_shape_text(s, new, None)?;
    }
    Ok(json!({"id": new}))
}

fn mind_collapse(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "mindmap.collapse";
    let at = topic_target(cmd, s, p)?;
    let n = s.doc_mut()?.get_mut(at)?;
    let collapse = boolean(p, "collapsed").unwrap_or(!crate::mindmap::is_collapsed(n));
    if collapse {
        n.plugin_data.insert(crate::mindmap::COLLAPSED.into(), String::new());
    } else {
        n.plugin_data.remove(crate::mindmap::COLLAPSED);
    }
    Ok(json!({"id": at, "collapsed": collapse}))
}

fn paste_mermaid(s: &mut Session, p: &Value) -> Result<Value> {
    use figcraft_doc::{ConnectorEnd, Magnet, Node};
    let cmd = "board.pasteMermaid";
    let text = req_string(cmd, p, "text")?;
    let chart = crate::mermaid::parse(&text).ok_or_else(|| bad(cmd, "not a Mermaid flowchart (or more than 500 nodes)"))?;
    let at = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => figcraft_geom::Point::new(x, y),
        _ => {
            let v = &s.ds()?.view;
            figcraft_geom::Point::new(v.cx, v.cy)
        }
    };
    if !(at.x.is_finite() && at.y.is_finite()) || at.x.abs() > 1e7 || at.y.abs() > 1e7 {
        return Err(bad(cmd, "the chart's place is out of range"));
    }
    let places = crate::mermaid::layout(&chart);
    // Centred on `at`.
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for nd in &chart.nodes {
        let (_, w, h) = nd.shape.board();
        if let Some((cx, cy)) = places.get(&nd.id) {
            (x0, y0, x1, y1) = (x0.min(cx - w / 2.0), y0.min(cy - h / 2.0), x1.max(cx + w / 2.0), y1.max(cy + h / 2.0));
        }
    }
    let shift = (at.x - (x0 + x1) / 2.0, at.y - (y0 + y1) / 2.0);
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let mut ids: std::collections::BTreeMap<String, NodeId> = std::collections::BTreeMap::new();
    let mut made = Vec::new();
    for nd in &chart.nodes {
        let (ty, w, h) = nd.shape.board();
        let Some((cx, cy)) = places.get(&nd.id).copied() else { continue };
        let id = doc.alloc_id();
        let mut n = Node::new(id, NodeKind::Rectangle, nd.text.chars().take(60).collect::<String>(), w, h);
        if doc.board {
            crate::board::dress(&mut n);
        }
        match (kind_of(ty), crate::board_shapes::preset(ty)) {
            (Some(k), _) => n.kind = k,
            (None, Some(pr)) => {
                crate::board_shapes::apply(&mut n, pr);
            }
            _ => {}
        }
        n.transform = figcraft_geom::Affine::translate((cx - w / 2.0 + shift.0, cy - h / 2.0 + shift.1));
        doc.insert(n, page, None)?;
        if !nd.text.is_empty() {
            crate::textedit::add_shape_text(doc, id, &nd.text)?;
        }
        ids.insert(nd.id.clone(), id);
        made.push(id);
    }
    // Links leave and enter along the flow (down: bottom to top); links back against it go round
    // the side, clear of the shapes between.
    use crate::mermaid::Direction;
    let (fwd, back) = match chart.direction {
        Direction::Down => ((Magnet::Bottom, Magnet::Top), (Magnet::Right, Magnet::Right)),
        Direction::Up => ((Magnet::Top, Magnet::Bottom), (Magnet::Right, Magnet::Right)),
        Direction::Right => ((Magnet::Right, Magnet::Left), (Magnet::Bottom, Magnet::Bottom)),
        Direction::Left => ((Magnet::Left, Magnet::Right), (Magnet::Bottom, Magnet::Bottom)),
    };
    let along = |id: &str| {
        places.get(id).map(|(x, y)| match chart.direction {
            Direction::Down => *y,
            Direction::Up => -*y,
            Direction::Right => *x,
            Direction::Left => -*x,
        })
    };
    let mut links = Vec::new();
    for e in &chart.edges {
        let (Some(a), Some(b)) = (ids.get(&e.from), ids.get(&e.to)) else { continue };
        let (m0, m1) = match (along(&e.from), along(&e.to)) {
            (Some(x), Some(y)) if y > x + 1.0 => fwd,
            (Some(x), Some(y)) if y < x - 1.0 => back,
            // Side by side in a layer.
            _ => (Magnet::Auto, Magnet::Auto),
        };
        let cid = doc.alloc_id();
        let mut c = Node::new_connector(cid, "Connector", ConnectorEnd::attached(*a, m0), ConnectorEnd::attached(*b, m1));
        if e.dashed {
            c.stroke.dashes = vec![8.0, 6.0];
        }
        if e.thick {
            c.stroke.weight = figcraft_doc::connector::THICK;
        }
        if !e.arrow {
            c.stroke.cap_end = figcraft_doc::StrokeCap::None;
        }
        doc.insert(c, page, None)?;
        figcraft_doc::connector::reroute(doc, cid);
        if !e.label.is_empty() {
            crate::textedit::add_shape_text(doc, cid, &e.label)?;
        }
        links.push(cid);
    }
    s.set_selection(made.clone());
    Ok(json!({"ids": made, "connectors": links}))
}

fn table_create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "table.create";
    let rows = num(p, "rows").filter(|n| n.is_finite()).map_or(3, |n| n.round().clamp(1.0, crate::table::MAX as f64) as usize);
    let cols = num(p, "cols").filter(|n| n.is_finite()).map_or(3, |n| n.round().clamp(1.0, crate::table::MAX as f64) as usize);
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
        return Err(bad(cmd, "the table's place is out of range"));
    }
    let doc = s.doc_mut()?;
    if !doc.get(parent)?.kind.is_container() {
        return Err(bad(cmd, format!("{parent} can't contain layers")));
    }
    let (w, h) = (crate::table::COL_W * cols as f64, crate::table::ROW_H * rows as f64);
    let at = doc.world_transform(parent).inverse() * figcraft_geom::Point::new(centre.x - w / 2.0, centre.y - h / 2.0);
    let id = doc.alloc_id();
    doc.insert(crate::table::new_table(id, at.x, at.y, cols), parent, None)?;
    crate::table::fill(doc, id, rows);
    s.set_selection(vec![id]);
    Ok(json!({"id": id}))
}

fn table_edit(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let doc = s.doc()?;
    let cell = match id(p, "id") {
        Some(i) => i,
        None => match s.selection().as_slice() {
            [one] => shape_text::click_target(doc, *one),
            _ => return Err(bad(cmd, "select one cell")),
        },
    };
    let n = doc.get(cell)?;
    let ((r, c), table) = match (crate::table::cell_of(n), n.parent) {
        (Some(rc), Some(t)) if doc.node(t).is_some_and(crate::table::is_table) => (rc, t),
        _ => return Err(bad(cmd, "that's not a table's cell")),
    };
    let doc = s.doc_mut()?;
    match cmd {
        "table.insertRow" => crate::table::insert_row(doc, table, if boolean(p, "above").unwrap_or(false) { r } else { r + 1 }),
        "table.insertColumn" => crate::table::insert_column(doc, table, if boolean(p, "left").unwrap_or(false) { c } else { c + 1 }),
        "table.deleteRow" => crate::table::delete_row(doc, table, r),
        _ => crate::table::delete_column(doc, table, c),
    }
    // The cell stays selected, or its table when the cell went.
    let keep = if doc.contains(cell) { cell } else { table };
    s.set_selection(vec![keep]);
    Ok(json!({"table": table}))
}
