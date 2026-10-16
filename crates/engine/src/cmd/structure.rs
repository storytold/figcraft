//! Structure: groups, frames, masks, boolean groups, flatten, outline stroke, auto layout, pages.

use figcraft_color::{Color, Paint};
use figcraft_doc::*;
use figcraft_geom::{Affine, BezPath, Rect, Shape};
use serde_json::{Value, json};

use super::edit::{roots, z_sorted};
use super::{CommandSpec, ok};
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("object.group", "Group selection", "{ids?, name?}", true, |s, p| wrap(s, p, Wrap::Group)),
        spec!("object.ungroup", "Ungroup selection", "{ids?}", true, ungroup),
        spec!("object.frameSelection", "Frame selection", "{ids?, name?}", true, |s, p| wrap(s, p, Wrap::Frame)),
        spec!("object.removeFrame", "Remove frame", "{ids?}", true, ungroup),
        spec!(
            "object.convertToGroup",
            "Convert to group",
            "{ids?} (frames become groups in place, their layers staying where they are; what only a frame has goes: auto layout, fills, strokes, corner radius, clipping, layout grids, guides and scrolling. Groups are left alone; instances, main components, layers inside instances and empty frames are refused)",
            true,
            |s, p| convert(s, p, "object.convertToGroup", false)
        ),
        spec!(
            "object.convertToFrame",
            "Convert to frame",
            "{ids?} (groups become frames of the same box in place, their layers staying where they are: no fill, not clipping. Frames are left alone; instances, main components and layers inside instances are refused)",
            true,
            |s, p| convert(s, p, "object.convertToFrame", true)
        ),
        spec!(
            "object.wrapInSection",
            "Wrap in new section",
            "{ids?, name?} (layers on the page or in a section; the section wraps them with some room)",
            true,
            |s, p| wrap(s, p, Wrap::Section)
        ),
        spec!(
            "object.resizeToFit",
            "Resize to fit",
            "{ids?} (frames, sections and components shrink or grow to their contents; the contents stay put)",
            true,
            resize_to_fit
        ),
        spec!("object.mask", "Use as mask", "{ids?}", true, mask),
        spec!("object.releaseMask", "Release mask", "{ids?}", true, |s, p| {
            for i in req_targets("object.releaseMask", s, p)? {
                s.doc_mut()?.get_mut(i)?.is_mask = false;
            }
            ok()
        }),
        spec!("bool.union", "Union selection", BOOL_PARAMS, true, |s, p| crate::booleans::apply(s, "bool.union", p, BoolOp::Union)),
        spec!("bool.subtract", "Subtract selection", BOOL_PARAMS, true, |s, p| crate::booleans::apply(s, "bool.subtract", p, BoolOp::Subtract)),
        spec!("bool.intersect", "Intersect selection", BOOL_PARAMS, true, |s, p| crate::booleans::apply(s, "bool.intersect", p, BoolOp::Intersect)),
        spec!("bool.exclude", "Exclude selection", BOOL_PARAMS, true, |s, p| crate::booleans::apply(s, "bool.exclude", p, BoolOp::Exclude)),
        spec!("bool.flatten", "Flatten boolean", "{ids?}", true, flatten),
        spec!("vector.flatten", "Flatten", "{ids?}", true, flatten),
        spec!("vector.outlineStroke", "Outline stroke", "{ids?}", true, outline_stroke),
        spec!("layout.add", "Add auto layout", "{ids?, direction? (horizontal|vertical|grid)}", true, layout_add),
        spec!("layout.remove", "Remove auto layout", "{ids?}", true, |s, p| {
            for i in req_targets("layout.remove", s, p)? {
                // Hugging needs auto layout and the children that filled have nothing to fill: both
                // become fixed (the frame keeps filling its own auto layout parent).
                let doc = s.doc_mut()?;
                doc.get_mut(i)?.layout = None;
                figcraft_layout::coherent_sizing(doc, i);
            }
            ok()
        }),
        spec!("layout.suggest", "Suggest auto layout", "{ids?}", true, layout_add),
        spec!("object.moveToPage", "Move to page", "{ids?, page}", true, move_to_page),
        spec!("object.setThumbnail", "Set as thumbnail", "{id?}", true, |s, p| {
            let id = targets(s, p).first().copied().ok_or_else(|| bad("object.setThumbnail", "select a frame"))?;
            let doc = s.doc_mut()?;
            for n in doc.ids().collect::<Vec<_>>() {
                if let Some(node) = doc.node(n)
                    && node.plugin_data.contains_key("figcraft.thumbnail")
                {
                    doc.get_mut(n)?.plugin_data.remove("figcraft.thumbnail");
                }
            }
            doc.get_mut(id)?.plugin_data.insert("figcraft.thumbnail".into(), "true".into());
            ok()
        }),
        spec!("object.rasterize", "Rasterize selection", "{ids?, scale? (default 2)}", true, rasterize),
        spec!("vector.reverse", "Reverse path direction", "{ids?}", true, |s, p| {
            for i in req_targets("vector.reverse", s, p)? {
                if let NodeKind::Vector { network } = &mut s.doc_mut()?.get_mut(i)?.kind {
                    for seg in &mut network.segments {
                        std::mem::swap(&mut seg.a, &mut seg.b);
                        std::mem::swap(&mut seg.ta, &mut seg.tb);
                    }
                }
            }
            ok()
        }),
        spec!("vector.simplify", "Simplify vector", "{ids?, tolerance?}", true, simplify),
        spec!("vector.offset", "Offset path", "{ids?, offset}", true, offset_path),
        spec!("vector.join", "Join selection", "{ids?} (in vector edit mode: joins the selected points)", true, |s, p| {
            crate::vectoredit::join_in_edit_mode(s, false).unwrap_or_else(|| join(s, p))
        }),
        spec!("vector.smoothJoin", "Smooth join selection", "{ids?} (in vector edit mode: joins the selected points smoothly)", true, |s, p| {
            crate::vectoredit::join_in_edit_mode(s, true).unwrap_or_else(|| join(s, p))
        }),
    ]
}

enum Wrap {
    Group,
    Frame,
    Section,
    Bool(BoolOp),
}

/// Room a section made around layers leaves on each side.
const SECTION_PAD: f64 = 40.0;

/// What the boolean commands do with what they're given.
const BOOL_PARAMS: &str = "{ids?} — layers inside boolean groups: sets their own operation (how each combines with the operands below it); one boolean group: sets its operation (every operand follows it); otherwise wraps the layers in a new boolean group";

/// Wrap the selection in a new boolean group doing `op`.
pub(crate) fn wrap_boolean(s: &mut Session, p: &Value, op: BoolOp) -> Result<Value> {
    wrap(s, p, Wrap::Bool(op))
}

/// Wrap the selection in a new container at the topmost selected layer's place.
fn wrap(s: &mut Session, p: &Value, w: Wrap) -> Result<Value> {
    let ids = req_targets("object.group", s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let top = *ids.last().ok_or_else(|| bad("object.group", "nothing selected"))?;
    let parent = doc.get(top)?.parent.ok_or_else(|| bad("object.group", "can't group a page"))?;
    // Sections sit on the page or in sections, never in frames or groups.
    let board = |doc: &Document, i: NodeId| {
        doc.node(i).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || matches!(p.kind, NodeKind::Section { .. }))
    };
    if matches!(w, Wrap::Section) && !ids.iter().all(|i| board(doc, *i)) {
        return Err(bad("object.wrapInSection", "only layers on the page or in a section can be wrapped in a section"));
    }
    if !matches!(w, Wrap::Section) && ids.iter().any(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))) {
        return Err(bad("object.group", "sections can't go in groups or frames"));
    }
    let index = doc.index_in_parent(top).map(|i| i + 1);
    let mut bounds = doc.bounds_of(&ids).unwrap_or_default();
    if matches!(w, Wrap::Section) {
        bounds = bounds.inflate(SECTION_PAD, SECTION_PAD);
    }
    let pw = doc.world_transform(parent);
    let local = figcraft_geom::transform_rect_bbox(pw.inverse(), bounds);
    let (kind, base) = match &w {
        Wrap::Group => (NodeKind::Group, "Group"),
        Wrap::Frame => (NodeKind::Frame, "Frame"),
        Wrap::Section => (NodeKind::Section { contents_hidden: false }, "Section"),
        Wrap::Bool(op) => (NodeKind::BooleanOp { op: *op }, op.label()),
    };
    let name = string(p, "name").unwrap_or_else(|| if matches!(w, Wrap::Bool(_)) { base.to_string() } else { doc.next_name(base) });
    let gid = doc.alloc_id();
    let mut g = match w {
        Wrap::Section => Node::new_section(gid, name, local.width(), local.height()),
        _ => Node::new(gid, kind, name, local.width(), local.height()),
    };
    g.transform = Affine::translate((local.x0, local.y0));
    if let Wrap::Bool(op) = w {
        // A boolean group looks like its top layer; a subtraction like the bottom one it cuts.
        let source = if op == BoolOp::Subtract { ids.first() } else { ids.last() };
        if let Some(first) = source.and_then(|i| doc.node(*i)) {
            g.fills = if first.fills.is_empty() { vec![Paint::solid(Color::DEFAULT_SHAPE)] } else { first.fills.clone() };
            g.strokes = first.strokes.clone();
            g.stroke = first.stroke.clone();
            g.effects = first.effects.clone();
        }
        g.blend = figcraft_color::BlendMode::Normal;
    }
    if let Wrap::Frame = w {
        g.clip = true;
    }
    // Insert the container where the topmost layer is.
    doc.insert(g, parent, index)?;
    for i in &ids {
        doc.reparent(*i, gid, None)?;
    }
    if !matches!(w, Wrap::Frame | Wrap::Section) {
        let _ = doc.fit_to_children(gid);
    }
    s.set_selection(vec![gid]);
    Ok(json!({"id": gid}))
}

/// Turn frames into groups or groups into frames in place (same id, name, layers, effects,
/// opacity, exports and interactions), the layers inside staying where they are on the page.
/// Nothing changes unless every layer can convert.
fn convert(s: &mut Session, p: &Value, cmd: &'static str, to_frame: bool) -> Result<Value> {
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut todo = Vec::new();
    for i in ids {
        let n = doc.get(i)?;
        let refuse = |why: &str| Err(bad(cmd, format!("\"{}\" {why}", n.name)));
        match (&n.kind, to_frame) {
            (NodeKind::Instance { .. }, _) => return refuse("is an instance: its layers come from its main component (detach it first)"),
            (NodeKind::Component { .. }, _) => return refuse("is a main component, which stays a frame"),
            (NodeKind::Frame | NodeKind::Group, _) if crate::components::owning_instance(doc, i).is_some() => {
                return refuse("is inside an instance: edit its main component, or detach the instance");
            }
            (NodeKind::Frame, false) if n.children.is_empty() => return refuse("is empty: a group needs layers"),
            (NodeKind::Frame, false) | (NodeKind::Group, true) => todo.push(i),
            (NodeKind::Frame | NodeKind::Group, _) => {}
            _ => return refuse("is neither a frame nor a group"),
        }
    }
    for i in &todo {
        if to_frame {
            // The group's box as it is; nothing painted or clipped, so the look stays.
            let n = doc.get_mut(*i)?;
            n.kind = NodeKind::Frame;
            n.clip = false;
            n.fills.clear();
            n.strokes.clear();
            continue;
        }
        let n = doc.get_mut(*i)?;
        n.kind = NodeKind::Group;
        (n.layout, n.clip, n.overflow) = (None, false, proto::Overflow::None);
        n.fills.clear();
        n.strokes.clear();
        n.grids.clear();
        n.guides.clear();
        n.corners = Corners::default();
        (n.styles.fill, n.styles.stroke, n.styles.grid) = (None, None, None);
        // Variables bound to what went would bring it back.
        n.bound.retain(|k, _| matches!(k.as_str(), "opacity" | "visible"));
        let kids = n.children.clone();
        for k in kids {
            // Out of the flow there is nothing to fill or to be absolute against.
            doc.get_mut(k)?.child_layout.absolute = false;
        }
        figcraft_layout::coherent_sizing(doc, *i);
        doc.fit_to_children(*i)?;
    }
    Ok(json!({"ids": todo}))
}

fn ungroup(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("object.ungroup", s, p)?;
    let doc = s.doc_mut()?;
    let mut out = Vec::new();
    for g in ids {
        let n = doc.get(g)?;
        if !(matches!(n.kind, NodeKind::Group | NodeKind::BooleanOp { .. } | NodeKind::Frame | NodeKind::Section { .. })) {
            continue;
        }
        let Some(parent) = n.parent else { continue };
        let idx = doc.index_in_parent(g).unwrap_or(0);
        let kids = n.children.clone();
        // Ungrouping a group keeps its opacity on the children (multiplied).
        let op = n.opacity;
        for (k, c) in kids.iter().enumerate() {
            doc.reparent(*c, parent, Some(idx + k))?;
            // Released operands no longer combine with anything.
            if doc.get(*c)?.bool_op.is_some() {
                doc.get_mut(*c)?.bool_op = None;
            }
            if op < 1.0 {
                let cn = doc.get_mut(*c)?;
                cn.opacity *= op;
            }
            out.push(*c);
        }
        doc.remove(g)?;
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

fn mask(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("object.mask", s, p)?;
    if ids.len() == 1 {
        let doc = s.doc_mut()?;
        let id = ids.first().copied().unwrap_or(NodeId(0));
        let n = doc.get_mut(id)?;
        n.is_mask = !n.is_mask;
        return ok();
    }
    // Group, then the bottom-most child is the mask.
    let r = wrap(s, &json!({"ids": ids, "name": "Mask group"}), Wrap::Group)?;
    let gid = id_of(&r["id"]).ok_or_else(|| EngineError::Other("group failed".into()))?;
    let doc = s.doc_mut()?;
    if let Some(first) = doc.get(gid)?.children.first().copied() {
        doc.get_mut(first)?.is_mask = true;
    }
    Ok(r)
}

/// The outline of a node in its parent's space (shapes, text, booleans, groups).
pub fn outline_in_parent(doc: &Document, id: NodeId) -> Option<BezPath> {
    let n = doc.node(id)?;
    let (mut p, _) = figcraft_render::subtree_outline(doc, n, 0)?;
    p.apply_affine(n.transform);
    Some(p)
}

fn flatten(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("vector.flatten", s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let top = *ids.last().ok_or_else(|| bad("vector.flatten", "nothing selected"))?;
    let parent = doc.get(top)?.parent.ok_or_else(|| bad("vector.flatten", "can't flatten a page"))?;
    let index = doc.index_in_parent(top).map(|i| i + 1);
    let pw = doc.world_transform(parent);
    let mut path = BezPath::new();
    let mut even = false;
    for i in &ids {
        let Some(n) = doc.node(*i) else { continue };
        // Booleans flatten to their result; strokes don't get outlined.
        let Some((mut op, rule)) = (match &n.kind {
            NodeKind::Group | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. } => figcraft_render::subtree_outline(doc, n, 0),
            _ => figcraft_render::node_outline(doc, n),
        }) else {
            continue;
        };
        even |= rule == figcraft_render::FillRule::EvenOdd;
        // Into the common parent's space.
        let w = doc.world_transform(*i);
        op.apply_affine(pw.inverse() * w);
        path.extend(op.elements().iter().copied());
    }
    if path.elements().is_empty() {
        return Err(EngineError::Disabled("vector.flatten".into(), "nothing to flatten".into()));
    }
    let first = ids.first().and_then(|i| doc.node(*i)).cloned();
    let bb = path.bounding_box();
    path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
    let mut net = VectorNetwork::from_path(&path);
    for r in &mut net.regions {
        r.winding = if even { WindingRule::EvenOdd } else { WindingRule::NonZero };
    }
    let vid = doc.alloc_id();
    let name = first.as_ref().map(|f| f.name.clone()).unwrap_or_else(|| "Vector".into());
    let mut v = Node::new(vid, NodeKind::Vector { network: net }, name, bb.width(), bb.height());
    v.transform = Affine::translate((bb.x0, bb.y0));
    if let Some(f) = first {
        v.fills = if matches!(f.kind, NodeKind::Group | NodeKind::Line) && f.fills.is_empty() {
            vec![Paint::solid(Color::DEFAULT_SHAPE)]
        } else {
            f.fills.clone()
        };
        v.strokes = f.strokes.clone();
        v.stroke = StrokeProps { align: StrokeAlign::Center, ..f.stroke.clone() };
        v.effects = f.effects.clone();
        v.opacity = f.opacity;
        v.blend = if matches!(f.blend, figcraft_color::BlendMode::PassThrough) { figcraft_color::BlendMode::Normal } else { f.blend };
    }
    doc.insert(v, parent, index)?;
    for i in &ids {
        doc.remove(*i)?;
    }
    s.set_selection(vec![vid]);
    Ok(json!({"id": vid}))
}

fn outline_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("vector.outlineStroke", s, p)?;
    let mut out = Vec::new();
    for i in ids {
        let doc = s.doc_mut()?;
        let n = doc.get(i)?.clone();
        if !n.has_visible_stroke() {
            continue;
        }
        let closed = figcraft_doc::geometry::is_closed(&n);
        let Some(geom) = (match &n.kind {
            NodeKind::Text(_) | NodeKind::BooleanOp { .. } => figcraft_render::node_outline(doc, &n).map(|(p, _)| p),
            _ => figcraft_doc::geometry::stroke_geometry(&n),
        }) else {
            continue;
        };
        let area = figcraft_render::draw::stroke_area(&n, &geom, closed);
        let mut path = (*area).clone();
        // Union the pieces so the outline is one clean shape.
        if let Ok(u) = figcraft_pathops::simplify(&path) {
            path = u;
        }
        let bb = path.bounding_box();
        path.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
        let mut net = VectorNetwork::from_path(&path);
        for r in &mut net.regions {
            r.winding = WindingRule::EvenOdd;
        }
        let parent = n.parent.ok_or_else(|| bad("vector.outlineStroke", "page"))?;
        let idx = doc.index_in_parent(i);
        let vid = doc.alloc_id();
        let mut v = Node::new(vid, NodeKind::Vector { network: net }, n.name.clone(), bb.width(), bb.height());
        v.transform = n.transform * Affine::translate((bb.x0, bb.y0));
        // The stroke's paints (and its colour style) fill the outline.
        v.fills = n.strokes.clone();
        v.strokes.clear();
        v.styles.fill = n.styles.stroke;
        if n.fills.iter().any(|f| f.visible) && closed {
            // Keep the fill as its own layer under the outline, grouped.
            doc.insert(v, parent, idx.map(|x| x + 1))?;
            let fill_id = i;
            {
                let f = doc.get_mut(fill_id)?;
                f.strokes.clear();
            }
            let r = wrap(s, &json!({"ids": [fill_id, vid], "name": n.name}), Wrap::Group)?;
            if let Some(g) = id_of(&r["id"]) {
                out.push(g);
            }
            continue;
        }
        // Standing in for the layer, the outline keeps how it is drawn and placed.
        (v.opacity, v.blend, v.effects, v.styles.effect) = (n.opacity, n.blend, n.effects.clone(), n.styles.effect);
        (v.constraints, v.child_layout, v.visible, v.locked) = (n.constraints, n.child_layout.clone(), n.visible, n.locked);
        doc.insert(v, parent, idx.map(|x| x + 1))?;
        doc.remove(i)?;
        out.push(vid);
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

/// Infer direction, gap and padding from where children are (Figma's "Add auto layout").
fn infer_layout(doc: &Document, frame: NodeId) -> AutoLayout {
    let Some(f) = doc.node(frame) else { return AutoLayout::default() };
    let boxes: Vec<Rect> =
        f.children.iter().filter_map(|c| doc.node(*c)).map(|c| figcraft_geom::transform_rect_bbox(c.transform, c.local_rect())).collect();
    if boxes.is_empty() {
        return AutoLayout { padding: [10.0; 4], gap: 10.0, ..Default::default() };
    }
    let span_x = boxes.iter().map(|b| b.x1).fold(f64::MIN, f64::max) - boxes.iter().map(|b| b.x0).fold(f64::MAX, f64::min);
    let span_y = boxes.iter().map(|b| b.y1).fold(f64::MIN, f64::max) - boxes.iter().map(|b| b.y0).fold(f64::MAX, f64::min);
    let vertical = span_y > span_x;
    let mut sorted = boxes.clone();
    sorted.sort_by(|a, b| if vertical { a.y0.total_cmp(&b.y0) } else { a.x0.total_cmp(&b.x0) });
    let gaps: Vec<f64> =
        sorted.windows(2).filter_map(|w| Some(if vertical { w.get(1)?.y0 - w.first()?.y1 } else { w.get(1)?.x0 - w.first()?.x1 })).collect();
    let gap = if gaps.is_empty() { 10.0 } else { (gaps.iter().sum::<f64>() / gaps.len() as f64).max(0.0).round() };
    let all = boxes.iter().copied().reduce(|a, b| a.union(b)).unwrap_or_default();
    let pad = [all.y0.max(0.0), (f.width - all.x1).max(0.0), (f.height - all.y1).max(0.0), all.x0.max(0.0)].map(f64::round);
    AutoLayout { mode: if vertical { LayoutMode::Vertical } else { LayoutMode::Horizontal }, gap, padding: pad, ..Default::default() }
}

fn layout_add(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("layout.add", s, p)?;
    let dir = string(p, "direction");
    let doc = s.doc()?;
    let all_frames = ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && n.layout.is_none()));
    let targets = if all_frames {
        ids
    } else {
        // Wrap non-frames in a new frame without a fill, then add auto layout to it.
        let r = wrap(s, &json!({"ids": ids}), Wrap::Frame)?;
        let fid = id_of(&r["id"]).ok_or_else(|| EngineError::Other("wrap failed".into()))?;
        let f = s.doc_mut()?.get_mut(fid)?;
        f.fills.clear();
        f.clip = false;
        vec![fid]
    };
    let doc = s.doc_mut()?;
    for f in &targets {
        let mut al = infer_layout(doc, *f);
        if let Some(d) = &dir {
            al.mode = match d.to_ascii_lowercase().as_str() {
                "vertical" | "column" => LayoutMode::Vertical,
                "grid" => LayoutMode::Grid,
                _ => LayoutMode::Horizontal,
            };
        }
        // Children in reading order along the axis.
        let mut kids = doc.get(*f)?.children.clone();
        let vertical = al.mode == LayoutMode::Vertical;
        kids.sort_by(|a, b| {
            let pa = doc.node(*a).map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect())).unwrap_or_default();
            let pb = doc.node(*b).map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect())).unwrap_or_default();
            if vertical { pa.y0.total_cmp(&pb.y0) } else { pa.x0.total_cmp(&pb.x0) }
        });
        let n = doc.get_mut(*f)?;
        n.children = kids;
        n.layout = Some(al);
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
    }
    s.set_selection(targets.clone());
    Ok(json!({"ids": targets}))
}

fn move_to_page(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("object.moveToPage", s, p)?;
    let page = req_id("object.moveToPage", p, "page")?;
    let doc = s.doc_mut()?;
    if !doc.node(page).is_some_and(Node::is_page) {
        return Err(bad("object.moveToPage", format!("{page} is not a page")));
    }
    if doc.is_divider(page) {
        return Err(bad("object.moveToPage", "a divider holds no layers"));
    }
    for i in z_sorted(doc, &roots(doc, &ids)) {
        doc.reparent(i, page, None)?;
    }
    s.set_selection(vec![]);
    ok()
}

fn rasterize(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("object.rasterize", s, p)?;
    let scale = num_or(p, "scale", 2.0);
    let mut out = Vec::new();
    for i in ids {
        let doc = s.doc()?;
        let Some(pix) = figcraft_render::render_node(doc, i, scale) else { continue };
        let png = figcraft_render::encode_png(&pix).map_err(EngineError::Other)?;
        let b = figcraft_render::export_bounds(doc, i).unwrap_or_default();
        let parent = doc.get(i)?.parent.ok_or_else(|| bad("object.rasterize", "page"))?;
        let idx = doc.index_in_parent(i);
        let pw = doc.world_transform(parent);
        let name = doc.get(i)?.name.clone();
        let docm = s.doc_mut()?;
        let (hash, _, _) = figcraft_format::add_image(docm, png).map_err(|e| EngineError::Other(e.to_string()))?;
        let rid = docm.alloc_id();
        let mut r = Node::new(rid, NodeKind::Rectangle, name, b.width(), b.height());
        r.transform = pw.inverse() * Affine::translate((b.x0, b.y0));
        r.fills = vec![Paint::image(hash, figcraft_color::ImageScaleMode::Fill)];
        docm.insert(r, parent, idx.map(|x| x + 1))?;
        docm.remove(i)?;
        out.push(rid);
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

fn simplify(s: &mut Session, p: &Value) -> Result<Value> {
    let tol = num_or(p, "tolerance", 0.5).max(0.01);
    for i in req_targets("vector.simplify", s, p)? {
        let doc = s.doc_mut()?;
        let n = doc.get_mut(i)?;
        if let NodeKind::Vector { network } = &mut n.kind {
            let fill = network.fill_path();
            let src = if fill.elements().is_empty() { network.stroke_path() } else { fill };
            let simplified = figcraft_geom::kurbo::simplify::simplify_bezpath(src.iter(), tol, &Default::default());
            let regions = !network.regions.is_empty();
            let mut net = VectorNetwork::from_path(&simplified);
            if !regions {
                net.regions.clear();
            }
            *network = net;
        }
    }
    ok()
}

fn offset_path(s: &mut Session, p: &Value) -> Result<Value> {
    let d = req_num("vector.offset", p, "offset")?;
    for i in req_targets("vector.offset", s, p)? {
        let doc = s.doc_mut()?;
        let n = doc.get(i)?.clone();
        let Some((path, _)) = figcraft_render::node_outline(doc, &n) else { continue };
        let props = StrokeProps { weight: d.abs() * 2.0, join: StrokeJoin::Round, align: StrokeAlign::Center, ..Default::default() };
        let ring = figcraft_pathops::stroke_center(&path, d.abs() * 2.0, &props);
        let res = if d >= 0.0 {
            figcraft_pathops::boolean(&[path, ring], BoolOp::Union)
        } else {
            figcraft_pathops::boolean(&[path, ring], BoolOp::Subtract)
        };
        let Ok(mut res) = res else { continue };
        let bb = res.bounding_box();
        res.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
        let nm = doc.get_mut(i)?;
        let mut net = VectorNetwork::from_path(&res);
        for r in &mut net.regions {
            r.winding = WindingRule::EvenOdd;
        }
        nm.kind = NodeKind::Vector { network: net };
        nm.transform *= Affine::translate((bb.x0, bb.y0));
        nm.width = bb.width();
        nm.height = bb.height();
    }
    ok()
}

/// Join open vectors whose end points touch into one vector.
fn join(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("vector.join", s, p)?;
    if ids.len() < 2 {
        // Close a single open path.
        if let Some(i) = ids.first() {
            let n = s.doc_mut()?.get_mut(*i)?;
            if let NodeKind::Vector { network } = &mut n.kind
                && network.regions.is_empty()
                && network.vertices.len() > 2
            {
                let last = network.vertices.len() - 1;
                network.segments.push(Segment::line(last, 0));
                let all: Vec<usize> = (0..network.segments.len()).collect();
                network.regions.push(Region { loops: vec![all], winding: WindingRule::NonZero });
            }
        }
        return ok();
    }
    flatten(s, &json!({"ids": ids}))
}

/// Fit containers to their visible children's bounds, keeping the children where they are on the
/// page (auto layout frames hug instead); rotated containers fit in their own axes.
fn resize_to_fit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "object.resizeToFit";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut fitted = Vec::new();
    for id in ids {
        let Some(n) = doc.node(id).cloned() else { continue };
        let container = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. });
        if !container || n.children.is_empty() || matches!(n.kind, NodeKind::Instance { .. }) {
            continue;
        }
        // Auto layout frames fit by hugging their contents.
        if n.layout.is_some() {
            let node = doc.get_mut(id)?;
            node.child_layout.sizing_h = Sizing::Hug;
            node.child_layout.sizing_v = Sizing::Hug;
            fitted.push(id);
            continue;
        }
        // Children's bounds in the container's own coordinates.
        let mut b: Option<figcraft_geom::Rect> = None;
        for c in n.children.iter().filter_map(|c| doc.node(*c)).filter(|c| c.visible) {
            let r = figcraft_geom::transform_rect_bbox(c.transform, c.local_rect());
            if r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite() {
                b = Some(b.map_or(r, |u| u.union(r)));
            }
        }
        let Some(b) = b else { continue };
        let (w, h) = (b.width().max(0.01), b.height().max(0.01));
        // Move the container by the offset, and its children back by it.
        let shift = Affine::translate((b.x0, b.y0));
        let node = doc.get_mut(id)?;
        node.transform *= shift;
        node.width = w;
        node.height = h;
        for c in &n.children {
            if let Ok(cn) = doc.get_mut(*c) {
                cn.transform = shift.inverse() * cn.transform;
            }
        }
        fitted.push(id);
    }
    Ok(json!({"ids": fitted}))
}
