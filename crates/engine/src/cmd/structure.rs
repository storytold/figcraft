//! Structure: groups, frames, masks, boolean groups, flatten, outline stroke, auto layout, pages.

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    AutoLayout, BoolOp, Corners, Document, LayoutMode, Node, NodeId, NodeKind, Region, RegionPaintSpace, Segment, Sizing, StrokeAlign, StrokeJoin,
    StrokeProps, VectorNetwork, WindingRule, proto,
};
use figcraft_geom::{Affine, BezPath, Rect, Shape};
use serde_json::{Value, json};

use super::edit::{roots, z_sorted};
use super::{CommandSpec, ok};
use crate::params::{f64_of, get, id_of, num_or, req_id, req_index, req_num, req_targets, string, targets};
use crate::{EngineError, Result, Session, bad, spec};

/// The layer structure commands: group, frame, mask, boolean operations, flatten, auto layout
/// add/remove, move to page and vector path edits.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("object.group", "Group selection", "{ids?, name?}", true, |s, p| wrap(s, p, "object.group", Wrap::Group)),
        spec!("object.ungroup", "Ungroup selection", "{ids?}", true, ungroup),
        spec!("object.frameSelection", "Frame selection", "{ids?, name?}", true, |s, p| wrap(s, p, "object.frameSelection", Wrap::Frame)),
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
            "{ids?} (groups become frames of the same box in place, their layers staying where they are: no fill, not clipping; sections become frames that keep their fill and don't clip. Frames are left alone; instances, main components, layers inside instances and sections holding sections are refused)",
            true,
            |s, p| convert(s, p, "object.convertToFrame", true)
        ),
        spec!(
            "object.convertToSection",
            "Convert to section",
            "{ids?} (frames on the page or in a section become sections in place, keeping their fill and layers; what only a frame has goes: auto layout, clipping, layout grids, guides and scrolling. Sections are left alone; nested frames, instances and main components are refused)",
            true,
            convert_to_section
        ),
        spec!(
            "object.wrapInSection",
            "Wrap in new section",
            "{ids?, name?} (layers on the page or in a section; the section wraps them with some room)",
            true,
            |s, p| wrap(s, p, "object.wrapInSection", Wrap::Section)
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
                let n = doc.get_mut(i)?;
                n.layout = None;
                super::props::drop_layout_bindings(n);
                figcraft_layout::coherent_sizing(doc, i);
            }
            ok()
        }),
        spec!("layout.suggest", "Suggest auto layout", "{ids?, direction? (horizontal|vertical|grid)}", true, layout_add),
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
        spec!(
            "vector.fillRule",
            "Set fill rule",
            "{ids? | id?, region? (a filled area of a vector; every area when left out), rule: nonZero|evenOdd}",
            true,
            fill_rule
        ),
        spec!(
            "vector.reverseLoop",
            "Reverse loop",
            "{ids? | id?, region, loop} (a loop of a filled area; its direction flips, which changes how the area fills)",
            true,
            reverse_loop
        ),
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
pub(crate) fn wrap_boolean(s: &mut Session, p: &Value, cmd: &str, op: BoolOp) -> Result<Value> {
    wrap(s, p, cmd, Wrap::Bool(op))
}

/// Wrap the selection in a new container at the topmost selected layer's place. Errors name `cmd`,
/// the command that asked.
fn wrap(s: &mut Session, p: &Value, cmd: &str, w: Wrap) -> Result<Value> {
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let top = *ids.last().ok_or_else(|| bad(cmd, "nothing selected"))?;
    let parent = doc.get(top)?.parent.ok_or_else(|| bad(cmd, "can't group a page"))?;
    // Sections sit on the page or in sections, never in frames or groups.
    let board = |doc: &Document, i: NodeId| {
        doc.node(i).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || matches!(p.kind, NodeKind::Section { .. }))
    };
    if matches!(w, Wrap::Section) && !ids.iter().all(|i| board(doc, *i)) {
        return Err(bad(cmd, "only layers on the page or in a section can be wrapped in a section"));
    }
    if !matches!(w, Wrap::Section) && ids.iter().any(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))) {
        return Err(bad(cmd, "sections can't go in groups or frames"));
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
            // A frame can't hold sections.
            (NodeKind::Section { .. }, true)
                if n.children.iter().any(|c| doc.node(*c).is_some_and(|c| matches!(c.kind, NodeKind::Section { .. }))) =>
            {
                return refuse("holds sections, which can't go inside a frame: convert those first");
            }
            (NodeKind::Section { .. }, true) => todo.push(i),
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
            // The box as it is and nothing clipped, so the look stays: a group has nothing painted, a
            // section keeps its fill.
            let n = doc.get_mut(*i)?;
            if matches!(n.kind, NodeKind::Group) {
                n.fills.clear();
                n.strokes.clear();
            }
            n.kind = NodeKind::Frame;
            n.clip = false;
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

/// Frames on the page or in a section become sections in place (sections only sit there).
fn convert_to_section(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "object.convertToSection";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let mut todo = Vec::new();
    for i in ids {
        let n = doc.get(i)?;
        let refuse = |why: &str| Err(bad(cmd, format!("\"{}\" {why}", n.name)));
        let on_board = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || matches!(p.kind, NodeKind::Section { .. }));
        match &n.kind {
            NodeKind::Section { .. } => {}
            NodeKind::Instance { .. } => return refuse("is an instance: its layers come from its main component (detach it first)"),
            NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => return refuse("is a main component, which stays a frame"),
            NodeKind::Frame if !on_board => return refuse("is inside a frame or group: only frames on the page or in a section become sections"),
            NodeKind::Frame => todo.push(i),
            _ => return refuse("is not a frame"),
        }
    }
    for i in &todo {
        let n = doc.get_mut(*i)?;
        n.kind = NodeKind::Section { contents_hidden: false };
        (n.layout, n.clip, n.overflow) = (None, false, proto::Overflow::None);
        n.grids.clear();
        n.guides.clear();
        n.styles.grid = None;
        super::props::drop_layout_bindings(n);
        let kids = n.children.clone();
        for k in kids {
            // Out of the flow there is nothing to fill or to be absolute against.
            doc.get_mut(k)?.child_layout.absolute = false;
        }
        figcraft_layout::coherent_sizing(doc, *i);
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
    let r = wrap(s, &json!({"ids": ids, "name": "Mask group"}), "object.mask", Wrap::Group)?;
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
    let first = ids.first().and_then(|i| doc.node(*i)).cloned();
    let fallback_fills = first
        .as_ref()
        .filter(|node| matches!(node.kind, NodeKind::Group | NodeKind::Line) && node.fills.is_empty())
        .map(|_| vec![Paint::solid(Color::DEFAULT_SHAPE)]);
    let output_fills = first
        .as_ref()
        .map(|node| {
            if matches!(node.kind, NodeKind::Group | NodeKind::Line) && node.fills.is_empty() {
                fallback_fills.clone().unwrap_or_default()
            } else {
                node.fills.clone()
            }
        })
        .unwrap_or_default();
    let mut net = VectorNetwork::default();
    for id in &ids {
        collect_flatten_parts(doc, *id, pw, fallback_fills.as_deref(), &output_fills, &mut net, 0)?;
    }
    let Some(bb) = net.bounds() else {
        return Err(EngineError::Disabled("vector.flatten".into(), "nothing to flatten".into()));
    };
    net.translate(-bb.x0, -bb.y0);
    let vid = doc.alloc_id();
    let name = first.as_ref().map(|f| f.name.clone()).unwrap_or_else(|| "Vector".into());
    let mut v = Node::new(vid, NodeKind::Vector { network: net }, name, bb.width(), bb.height());
    v.transform = Affine::translate((bb.x0, bb.y0));
    if let Some(f) = first {
        v.fills = output_fills;
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

fn collect_flatten_parts(
    doc: &Document,
    id: NodeId,
    parent_world: Affine,
    fallback_fills: Option<&[Paint]>,
    output_fills: &[Paint],
    net: &mut VectorNetwork,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        return Err(bad("vector.flatten", "group nesting exceeds 64"));
    }
    let Some(node) = doc.node(id) else { return Ok(()) };
    if matches!(node.kind, NodeKind::Group | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. }) && !node.children.is_empty() {
        // Match subtree_outline: a nonempty frame contributes its own visible fill, then
        // visible descendants in stacking order. Child clipping remains the container's outline rule.
        if node.kind.is_frame_like()
            && node.fills.iter().any(|paint| paint.visible)
            && let Some(path) = figcraft_doc::geometry::shape_path(node)
        {
            let frame = VectorNetwork::from_path(&path);
            let into_parent = parent_world.inverse() * doc.world_transform(id);
            append_flatten_part(node, fallback_fills, output_fills, into_parent, frame, net);
        }
        for child in &node.children {
            if !doc.node(*child).is_some_and(|child| child.visible) {
                continue;
            }
            collect_flatten_parts(doc, *child, parent_world, fallback_fills, output_fills, net, depth + 1)?;
        }
        return Ok(());
    }
    let part = match &node.kind {
        NodeKind::Vector { network } => {
            let mut part = network.clone();
            if !part.regions.is_empty() && part.regions.iter().all(|region| region.fills.is_none() && region.paint_space.is_none()) {
                let winding = if part.regions.iter().any(|region| region.winding == WindingRule::EvenOdd) {
                    WindingRule::EvenOdd
                } else {
                    WindingRule::NonZero
                };
                let loops = std::mem::take(&mut part.regions).into_iter().flat_map(|region| region.loops).collect();
                part.regions = vec![Region { loops, winding, ..Default::default() }];
            }
            part
        }
        _ => {
            let Some(part) = outline_part(doc, node)? else { return Ok(()) };
            part
        }
    };
    let into_parent = parent_world.inverse() * doc.world_transform(id);
    append_flatten_part(node, fallback_fills, output_fills, into_parent, part, net);
    Ok(())
}

/// What a layer other than a vector or a container flattens into: its outline (`None`: it has
/// none). A boolean group has no outline only past the path operations' caps; flattening it would
/// drop it, so that is refused.
fn outline_part(doc: &Document, node: &Node) -> Result<Option<VectorNetwork>> {
    let Some((path, rule)) = figcraft_render::node_outline(doc, node) else {
        if matches!(node.kind, NodeKind::BooleanOp { .. }) {
            return Err(bad("vector.flatten", figcraft_pathops::PathOpsError::TooComplex.to_string()));
        }
        return Ok(None);
    };
    let mut part = VectorNetwork::from_path(&path);
    for region in &mut part.regions {
        if rule == figcraft_render::FillRule::EvenOdd {
            region.winding = WindingRule::EvenOdd;
        }
    }
    Ok(Some(part))
}

fn append_flatten_part(
    node: &Node,
    fallback_fills: Option<&[Paint]>,
    output_fills: &[Paint],
    into_parent: Affine,
    mut part: VectorNetwork,
    net: &mut VectorNetwork,
) {
    for region in &mut part.regions {
        let (width, height, paint_transform) = region.paint_frame(node.width, node.height);
        if region.fills.is_none() {
            let effective = if node.fills.is_empty() { fallback_fills.unwrap_or(&node.fills) } else { &node.fills };
            if effective != output_fills {
                region.fills = Some(effective.to_vec());
            }
        }
        region.paint_space = Some(RegionPaintSpace { width, height, transform: paint_transform });
    }
    let vertices: Vec<usize> = (0..part.vertices.len()).collect();
    crate::vectoredit::ops::transform_vertices(&mut part, &vertices, into_parent);
    let (nv, ns) = (net.vertices.len(), net.segments.len());
    net.vertices.extend(part.vertices);
    net.segments.extend(part.segments.into_iter().map(|segment| Segment { a: segment.a + nv, b: segment.b + nv, ..segment }));
    net.regions.extend(
        part.regions.into_iter().map(|region| Region {
            loops: region.loops.into_iter().map(|lp| lp.into_iter().map(|segment| segment + ns).collect()).collect(),
            ..region
        }),
    );
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
        let Some((area, rule)) = (match &n.kind {
            NodeKind::Text(_) => figcraft_render::node_outline(doc, &n)
                .map(|(p, _)| (figcraft_render::draw::stroke_area(&n, &p, closed), figcraft_render::FillRule::Winding)),
            _ => figcraft_render::stroke_fill(doc, &n),
        }) else {
            continue;
        };
        let mut path = (*area).clone();
        // Simplify non-zero strokes into even-odd contours. Inside/outside rings already use
        // even-odd: interpreting their same-direction contours as non-zero would fill the hole.
        if rule == figcraft_render::FillRule::Winding
            && let Ok(u) = figcraft_pathops::simplify(&path)
        {
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
        let region_fill = matches!(&n.kind, NodeKind::Vector { network } if network.regions.iter().any(|r| r.fills.as_ref().is_some_and(|fills| fills.iter().any(|f| f.visible))));
        if (n.fills.iter().any(|f| f.visible) || region_fill) && closed {
            // Keep the fill as its own layer under the outline, grouped.
            doc.insert(v, parent, idx.map(|x| x + 1))?;
            let fill_id = i;
            {
                let f = doc.get_mut(fill_id)?;
                f.strokes.clear();
            }
            let r = wrap(s, &json!({"ids": [fill_id, vid], "name": n.name}), "vector.outlineStroke", Wrap::Group)?;
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

/// Infer direction, gap and padding from visible, non-absolute children, matching the layout flow.
fn infer_layout(doc: &Document, frame: NodeId) -> AutoLayout {
    let Some(f) = doc.node(frame) else { return AutoLayout::default() };
    let boxes: Vec<Rect> = figcraft_layout::flow_children(doc, frame)
        .iter()
        .filter_map(|c| doc.node(*c))
        .map(|c| figcraft_geom::transform_rect_bbox(c.transform, c.local_rect()))
        .collect();
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

/// Sort unmasked spans by position along the layout axis. Masks stay at their original indices;
/// each visible mask's following content keeps its paint order until the next mask, hidden or not.
fn sort_layout_children(doc: &Document, kids: &mut [NodeId], vertical: bool) {
    let mut start = 0;
    let mut masked = false;
    for end in 0..=kids.len() {
        let mask = kids.get(end).and_then(|id| doc.node(*id)).filter(|n| n.is_mask);
        if end != kids.len() && mask.is_none() {
            continue;
        }
        if !masked && let Some(span) = kids.get_mut(start..end) {
            span.sort_by(|a, b| {
                let pa = doc.node(*a).map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect())).unwrap_or_default();
                let pb = doc.node(*b).map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect())).unwrap_or_default();
                if vertical { pa.y0.total_cmp(&pb.y0) } else { pa.x0.total_cmp(&pb.x0) }
            });
        }
        start = end + 1;
        masked = mask.is_some_and(|n| n.visible);
    }
}

/// Add inferred auto layout to existing frames or one group converted in place; wrap other
/// selections. Preserve mask boundaries and a converted group's Fill sizing in its parent.
fn layout_add(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("layout.add", s, p)?;
    let dir = string(p, "direction");
    let doc = s.doc()?;
    let all_frames = ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && n.layout.is_none()));
    let single_group = ids.len() == 1 && ids.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Group));
    let in_place = all_frames || single_group;
    let targets = if in_place {
        if single_group {
            // Layout must arrange the group's children directly. Wrapping the group would
            // leave them in a single fixed-size layout item and add an unnecessary layer.
            convert(s, &json!({"ids": ids}), "layout.add", true)?;
        }
        ids
    } else {
        // Wrap non-frames in a new frame without a fill, then add auto layout to it.
        let r = wrap(s, &json!({"ids": ids}), "layout.add", Wrap::Frame)?;
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
        let mut kids = doc.get(*f)?.children.clone();
        sort_layout_children(doc, &mut kids, al.mode == LayoutMode::Vertical);
        let empty = doc.node(*f).is_some_and(|n| n.children.iter().filter_map(|c| doc.node(*c)).all(|c| !c.visible || c.child_layout.absolute));
        let n = doc.get_mut(*f)?;
        n.children = kids;
        n.layout = Some(al);
        if !in_place || !empty {
            // A frame wrapped around the selection hugs it; an existing frame with no flow
            // children keeps its size instead of collapsing to its padding.
            if !single_group || n.child_layout.sizing_h != Sizing::Fill {
                n.child_layout.sizing_h = Sizing::Hug;
            }
            if !single_group || n.child_layout.sizing_v != Sizing::Fill {
                n.child_layout.sizing_v = Sizing::Hug;
            }
        }
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
            let source = network.clone();
            let mut net = VectorNetwork::default();
            let legacy = source.regions.iter().all(|region| region.fills.is_none() && region.paint_space.is_none());
            if legacy && !source.regions.is_empty() {
                let winding = if source.regions.iter().any(|region| region.winding == WindingRule::EvenOdd) {
                    WindingRule::EvenOdd
                } else {
                    WindingRule::NonZero
                };
                let path = source.fill_path();
                let path = figcraft_geom::kurbo::simplify::simplify_bezpath(path.iter(), tol, &Default::default());
                let (path, output_winding) = match figcraft_pathops::simplify_with_rule(&path, winding == WindingRule::EvenOdd) {
                    Ok(path) => (path, WindingRule::EvenOdd),
                    Err(_) => (path, winding),
                };
                let combined = Region { winding: output_winding, ..Default::default() };
                append_region_path(&mut net, &path, &combined, output_winding);
            } else {
                for (index, region) in source.regions.iter().enumerate() {
                    let Some(path) = source.region_path(index) else { continue };
                    let path = figcraft_geom::kurbo::simplify::simplify_bezpath(path.iter(), tol, &Default::default());
                    let path = figcraft_pathops::simplify_with_rule(&path, region.winding == WindingRule::EvenOdd).unwrap_or(path);
                    append_region_path(&mut net, &path, region, region.winding);
                }
            }
            if !source.regions.is_empty() {
                let mut used = vec![false; source.segments.len()];
                for segment in source.regions.iter().flat_map(|region| region.loops.iter().flatten()) {
                    if let Some(used) = used.get_mut(*segment) {
                        *used = true;
                    }
                }
                let mut open = source.clone();
                open.segments = source.segments.iter().enumerate().filter_map(|(index, segment)| (!used[index]).then_some(*segment)).collect();
                open.regions.clear();
                if !open.segments.is_empty() {
                    let path = open.stroke_path();
                    let simplified = figcraft_geom::kurbo::simplify::simplify_bezpath(path.iter(), tol, &Default::default());
                    let mut loose = VectorNetwork::from_path(&simplified);
                    loose.regions.clear();
                    append_network(&mut net, loose);
                }
            }
            if source.regions.is_empty() {
                let path = source.stroke_path();
                let simplified = figcraft_geom::kurbo::simplify::simplify_bezpath(path.iter(), tol, &Default::default());
                net = VectorNetwork::from_path(&simplified);
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
        // A vector can't hold children: leave containers (frames, groups, instances) alone.
        if !n.children.is_empty() {
            continue;
        }
        if d == 0.0 {
            continue;
        }
        let props = StrokeProps { weight: d.abs() * 2.0, join: StrokeJoin::Round, align: StrokeAlign::Center, ..Default::default() };
        let NodeKind::Vector { network: source } = &n.kind else {
            let Some((path, _)) = figcraft_render::node_outline(doc, &n) else { continue };
            let ring = figcraft_pathops::stroke_center(&path, d.abs() * 2.0, &props);
            let result = if d >= 0.0 {
                figcraft_pathops::boolean(&[path, ring], BoolOp::Union)
            } else {
                figcraft_pathops::boolean(&[path, ring], BoolOp::Subtract)
            }
            .map_err(|error| bad("vector.offset", error.to_string()))?;
            let mut result = result;
            let bounds = result.bounding_box();
            result.apply_affine(Affine::translate((-bounds.x0, -bounds.y0)));
            let node = doc.get_mut(i)?;
            node.kind = NodeKind::Vector { network: VectorNetwork::from_path(&result) };
            node.transform *= Affine::translate((bounds.x0, bounds.y0));
            node.width = bounds.width();
            node.height = bounds.height();
            continue;
        };
        let mut net = VectorNetwork::default();
        let legacy = source.regions.iter().all(|region| region.fills.is_none() && region.paint_space.is_none());
        if source.regions.is_empty() {
            let Some((path, _)) = figcraft_render::node_outline(doc, &n) else { continue };
            let ring = figcraft_pathops::stroke_center(&path, d.abs() * 2.0, &props);
            let mut res = if d >= 0.0 {
                figcraft_pathops::boolean(&[path.clone(), ring], BoolOp::Union)
            } else {
                figcraft_pathops::boolean(&[path.clone(), ring], BoolOp::Subtract)
            }
            .map_err(|error| bad("vector.offset", error.to_string()))?;
            let bb = res.bounding_box();
            res.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
            let nm = doc.get_mut(i)?;
            nm.kind = NodeKind::Vector { network: VectorNetwork::from_path(&res) };
            nm.transform *= Affine::translate((bb.x0, bb.y0));
            nm.width = bb.width();
            nm.height = bb.height();
            continue;
        }
        if legacy {
            let path = source.fill_path();
            let even_odd = source.regions.iter().any(|region| region.winding == WindingRule::EvenOdd);
            let path = figcraft_pathops::simplify_with_rule(&path, even_odd).map_err(|error| bad("vector.offset", error.to_string()))?;
            let ring = figcraft_pathops::stroke_center(&path, d.abs() * 2.0, &props);
            let result = if d >= 0.0 {
                figcraft_pathops::boolean(&[path.clone(), ring], BoolOp::Union)
            } else {
                figcraft_pathops::boolean(&[path, ring], BoolOp::Subtract)
            }
            .map_err(|error| bad("vector.offset", error.to_string()))?;
            let mut part = VectorNetwork::from_path(&result);
            let (paint_width, paint_height, paint_transform) = source.regions[0].paint_frame(n.width, n.height);
            for region in &mut part.regions {
                region.winding = WindingRule::EvenOdd;
                region.fills = source.regions[0].fills.clone();
                region.paint_space = Some(RegionPaintSpace { width: paint_width, height: paint_height, transform: paint_transform });
            }
            append_network(&mut net, part);
        } else {
            for (index, region) in source.regions.iter().enumerate() {
                let Some(path) = source.region_path(index) else { continue };
                let mut paint_region = region.clone();
                let (paint_width, paint_height, paint_transform) = region.paint_frame(n.width, n.height);
                paint_region.paint_space = Some(RegionPaintSpace { width: paint_width, height: paint_height, transform: paint_transform });
                let path = figcraft_pathops::simplify_with_rule(&path, region.winding == WindingRule::EvenOdd)
                    .map_err(|error| bad("vector.offset", error.to_string()))?;
                let ring = figcraft_pathops::stroke_center(&path, d.abs() * 2.0, &props);
                let res = if d >= 0.0 {
                    figcraft_pathops::boolean(&[path.clone(), ring], BoolOp::Union)
                } else {
                    figcraft_pathops::boolean(&[path.clone(), ring], BoolOp::Subtract)
                }
                .map_err(|error| bad("vector.offset", error.to_string()))?;
                // A collapsed negative offset removes this region, while other source regions stay.
                if res.elements().is_empty() {
                    continue;
                }
                append_region_path(&mut net, &res, &paint_region, WindingRule::EvenOdd);
            }
        }
        let Some(bb) = net.bounds() else {
            let nm = doc.get_mut(i)?;
            nm.kind = NodeKind::Vector { network: net };
            nm.width = 0.0;
            nm.height = 0.0;
            continue;
        };
        net.translate(-bb.x0, -bb.y0);
        let nm = doc.get_mut(i)?;
        nm.kind = NodeKind::Vector { network: net };
        nm.transform *= Affine::translate((bb.x0, bb.y0));
        nm.width = bb.width();
        nm.height = bb.height();
    }
    ok()
}

fn append_region_path(net: &mut VectorNetwork, path: &BezPath, source: &Region, winding: WindingRule) {
    let mut part = VectorNetwork::from_path(path);
    for region in &mut part.regions {
        region.winding = winding;
        region.fills = source.fills.clone();
        region.paint_space = source.paint_space;
    }
    append_network(net, part);
}

fn append_network(net: &mut VectorNetwork, part: VectorNetwork) {
    let (nv, ns) = (net.vertices.len(), net.segments.len());
    net.vertices.extend(part.vertices);
    net.segments.extend(part.segments.into_iter().map(|s| Segment { a: s.a + nv, b: s.b + nv, ..s }));
    net.regions.extend(
        part.regions.into_iter().map(|r| Region { loops: r.loops.into_iter().map(|l| l.into_iter().map(|s| s + ns).collect()).collect(), ..r }),
    );
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
                network.regions.push(Region { loops: vec![all], winding: WindingRule::NonZero, fills: None, paint_space: None });
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

/// The vector network of `id`, when it is a vector layer.
fn network_mut(doc: &mut Document, id: NodeId) -> Option<&mut VectorNetwork> {
    match &mut doc.get_mut(id).ok()?.kind {
        NodeKind::Vector { network } => Some(network),
        _ => None,
    }
}

/// The id a fill rule is written as.
fn rule_id(rule: WindingRule) -> &'static str {
    match rule {
        WindingRule::NonZero => "nonZero",
        WindingRule::EvenOdd => "evenOdd",
    }
}

/// `region` as an index: absent (or null) means every area.
fn area_index(cmd: &str, p: &Value) -> Result<Option<usize>> {
    match get(p, "region") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let k = f64_of(v).ok_or_else(|| bad(cmd, "`region` must be a number"))?;
            if !(0.0..1e9).contains(&k) || k.fract() != 0.0 {
                return Err(bad(cmd, "`region` out of range"));
            }
            Ok(Some(k as usize))
        }
    }
}

/// Segments another one- or two-segment loop lists: a short loop takes its direction from the ends
/// its segments are stored with, so re-storing one of those would turn that loop round too. A loop
/// of three or more segments is left out: its ring is walked in list order, which the stored ends
/// don't decide.
fn short_loop_segments(net: &VectorNetwork, skip: (usize, usize)) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    for (r, region) in net.regions.iter().enumerate() {
        for (l, lp) in region.loops.iter().enumerate() {
            if (r, l) != skip && lp.len() <= 2 {
                out.extend(lp.iter().copied());
            }
        }
    }
    out
}

fn fill_rule(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("vector.fillRule", s, p)?;
    let rule = match string(p, "rule").as_deref().map(str::trim) {
        Some("evenOdd" | "evenodd" | "EVENODD") => WindingRule::EvenOdd,
        Some("nonZero" | "nonzero" | "NONZERO") => WindingRule::NonZero,
        _ => return Err(bad("vector.fillRule", "`rule` is nonZero or evenOdd")),
    };
    let only = area_index("vector.fillRule", p)?;
    let doc = s.doc_mut()?;
    let mut count = 0usize;
    for id in ids {
        let Some(net) = network_mut(doc, id) else { continue };
        match only {
            Some(r) => {
                if let Some(region) = net.regions.get_mut(r) {
                    region.winding = rule;
                    count += 1;
                }
            }
            None => {
                for region in &mut net.regions {
                    region.winding = rule;
                    count += 1;
                }
            }
        }
    }
    if count == 0 {
        return Err(EngineError::Disabled("vector.fillRule".into(), "no filled area to set (select a vector)".into()));
    }
    Ok(json!({"regions": count, "rule": rule_id(rule)}))
}

/// One loop of a vector, worked out but not yet written back: its segments in the new order and,
/// for a loop of one or two segments, the one to store the other way.
struct LoopFlip {
    id: NodeId,
    turned: Vec<usize>,
    store: Option<usize>,
}

/// Turn one loop of a filled area round: its segments are walked the other way, so the area it
/// bounds counts the other way in the winding (which turns a hole into a filled part and back).
///
/// How a loop is walked depends on how many segments it has:
/// - three or more: the order they are listed in, each segment turned to continue from the point
///   reached. Reversing the list is therefore enough, and the stored ends are left alone (a shared
///   segment's ends belong to its neighbour too);
/// - two: the same ring either way round, so one of the two has to be stored the other way instead;
/// - one: only its stored ends say which way it runs, so they are swapped.
///
/// A short loop can only turn round by re-storing a segment, which is refused when another short
/// loop lists that segment too: it would turn that area round as well.
fn reverse_loop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "vector.reverseLoop";
    let ids = req_targets(cmd, s, p)?;
    let region = req_index(cmd, p, "region")?;
    let index = req_index(cmd, p, "loop")?;
    let doc = s.doc_mut()?;
    // Work every loop out before touching the document: one that can't turn round refuses the call
    // and the others stay as they were.
    let mut flips: Vec<LoopFlip> = Vec::new();
    for id in ids {
        let Some(NodeKind::Vector { network }) = doc.node(id).map(|n| &n.kind) else { continue };
        let Some(loop_) = network.regions.get(region).and_then(|r| r.loops.get(index)) else { continue };
        if loop_.is_empty() {
            continue;
        }
        let mut store = None;
        if loop_.len() <= 2 {
            let busy = short_loop_segments(network, (region, index));
            let Some(pick) = loop_.iter().copied().filter(|s| !busy.contains(s)).min() else {
                return Err(EngineError::Disabled(
                    cmd.into(),
                    "the loop shares its segments with another one- or two-segment loop, so turning it round would turn that one round too".into(),
                ));
            };
            store = Some(pick);
        }
        flips.push(LoopFlip { id, turned: loop_.iter().rev().copied().collect(), store });
    }
    if flips.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "no such loop (select a vector with filled areas)".into()));
    }
    for LoopFlip { id, turned, store } in flips {
        let Some(net) = network_mut(doc, id) else { continue };
        if let Some(seg) = store.and_then(|i| net.segments.get_mut(i)) {
            std::mem::swap(&mut seg.a, &mut seg.b);
            std::mem::swap(&mut seg.ta, &mut seg.tb);
        }
        if let Some(l) = net.regions.get_mut(region).and_then(|r| r.loops.get_mut(index)) {
            *l = turned;
        }
    }
    Ok(json!({"reversed": true}))
}
