//! Copy, cut and paste in vector edit mode (#1875).
//!
//! With points or segments selected, Copy and Cut take just those: a temporary vector holding the
//! selected segments (and the segments touching a selected point) goes on the ordinary layer
//! clipboard, kept at its place on the page, so a paste outside edit mode is a plain layer paste
//! in place. Cut then removes the selection the way Delete does and stays in edit mode. Paste
//! while editing adds the clipboard's layers to the edited vector's network, in place: a vector
//! with its segments, a shape or boolean with its outline.

use figcraft_doc::{NodeKind, Region, VectorNetwork};

use super::clipboard::{collect_images, store_clipboard};
use super::{Affine, BTreeMap, BTreeSet, Document, EngineError, Node, NodeId, Result, Session, Value, json, num};
use crate::canvas::vectoredit::{self, EditPart};

/// The most segments a vector network takes through a paste.
const MAX_SEGMENTS: usize = 500_000;

/// What the edited layers have selected.
pub(super) struct EditSelection {
    /// Whether any edited layer has a point or a segment selected.
    pub any: bool,
    /// Each edited layer with something to copy, and the sub-network that is.
    pub parts: Vec<(NodeId, VectorNetwork)>,
}

/// The selection of the vector being edited (and the other layers edited with it), or `None` when
/// not in vector edit mode.
pub(super) fn edit_selection(s: &Session) -> Option<EditSelection> {
    let ds = s.ds().ok()?;
    let ve = ds.vector_edit.as_ref()?;
    let mut sel = EditSelection { any: false, parts: Vec::new() };
    for part in ve.parts() {
        if part.vertices.is_empty() && part.segments.is_empty() {
            continue;
        }
        sel.any = true;
        let Some(net) = vectoredit::network(&ds.doc, part.id) else { continue };
        let active = part.id == ve.id;
        // Points selected only through a selected region don't pull in the segments touching
        // them: the region's own segments are the selection.
        let vertices: Vec<usize> = part.vertices.iter().copied().filter(|v| !active || !ve.region_vertices.contains(v)).collect();
        let mut segments = part.segments.clone();
        if active {
            segments.extend(ve.regions.iter().filter_map(|r| net.regions.get(*r)).flat_map(|r| r.loops.iter().flatten().copied()));
        }
        if let Some(sub) = sub_network(net, &vertices, &segments) {
            sel.parts.push((part.id, sub));
        }
    }
    Some(sel)
}

/// The part of `net` that is the segments `segments` and those touching a point of `vertices`,
/// with the points they join; `None` when there are none (a lone point with no segment draws
/// nothing). The regions whose every segment is kept stay regions.
fn sub_network(net: &VectorNetwork, vertices: &[usize], segments: &[usize]) -> Option<VectorNetwork> {
    let picked: BTreeSet<usize> = vertices.iter().copied().collect();
    let mut keep: BTreeSet<usize> = segments.iter().copied().filter(|i| *i < net.segments.len()).collect();
    for (i, sg) in net.segments.iter().enumerate() {
        if picked.contains(&sg.a) || picked.contains(&sg.b) {
            keep.insert(i);
        }
    }
    let used: BTreeSet<usize> = keep.iter().filter_map(|i| net.segments.get(*i)).flat_map(|sg| [sg.a, sg.b]).collect();
    let vmap: BTreeMap<usize, usize> = used.iter().enumerate().map(|(new, old)| (*old, new)).collect();
    let smap: BTreeMap<usize, usize> = keep.iter().enumerate().map(|(new, old)| (*old, new)).collect();
    let mut out = VectorNetwork {
        vertices: used.iter().filter_map(|v| net.vertices.get(*v).cloned()).collect(),
        segments: keep
            .iter()
            .filter_map(|i| net.segments.get(*i))
            .filter_map(|sg| {
                let mut sg = *sg;
                sg.a = *vmap.get(&sg.a)?;
                sg.b = *vmap.get(&sg.b)?;
                Some(sg)
            })
            .collect(),
        regions: Vec::new(),
    };
    if out.segments.is_empty() || out.segments.len() != keep.len() || out.vertices.len() != used.len() {
        return None;
    }
    for r in &net.regions {
        let loops: Option<Vec<Vec<usize>>> = r.loops.iter().map(|l| l.iter().map(|i| smap.get(i).copied()).collect::<Option<Vec<usize>>>()).collect();
        if let Some(loops) = loops.filter(|l| !l.is_empty()) {
            out.regions.push(Region { loops, ..r.clone() });
        }
    }
    Some(out)
}

/// The clipboard's layer for edited vector `id` holding `sub`: it keeps the layer's look (fills,
/// strokes, effects, opacity, blend) and its place on the page. A variable width profile runs
/// along the whole path, so it stays only when every segment is copied.
fn clip_node(doc: &Document, id: NodeId, mut sub: VectorNetwork) -> Option<Node> {
    let mut n = doc.node(id)?.clone();
    let whole = vectoredit::network(doc, id).is_some_and(|all| all.segments.len() == sub.segments.len());
    let b = sub.bounds().filter(|b| b.x0.is_finite() && b.y0.is_finite() && b.width().is_finite() && b.height().is_finite())?;
    sub.translate(-b.x0, -b.y0);
    n.transform = doc.world_transform(id) * Affine::translate((b.x0, b.y0));
    n.width = b.width();
    n.height = b.height();
    n.kind = NodeKind::Vector { network: sub };
    n.parent = None;
    n.children.clear();
    n.name = "Vector".into();
    if !whole {
        n.stroke.width_profile = None;
    }
    Some(n)
}

/// Put the selection's sub-networks on the layer clipboard. Returns how many segments went.
fn copy_parts(s: &mut Session, cmd: &str, sel: &EditSelection) -> Result<usize> {
    let doc = s.doc()?;
    let mut nodes = Vec::new();
    let mut images = BTreeMap::new();
    let mut bounds: Option<figcraft_geom::Rect> = None;
    let mut segments = 0;
    for (id, sub) in &sel.parts {
        segments += sub.segments.len();
        let Some(n) = clip_node(doc, *id, sub.clone()) else { continue };
        let r = figcraft_geom::transform_rect_bbox(n.transform, n.local_rect());
        bounds = Some(bounds.map_or(r, |b| b.union(r)));
        collect_images(doc, std::slice::from_ref(&n), &mut images);
        nodes.push((*id, vec![n]));
    }
    if nodes.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "the selection has nothing to copy".into()));
    }
    store_clipboard(s, cmd, nodes, images, bounds)?;
    Ok(segments)
}

/// Copy the selected segments (see [`edit_selection`]).
pub(super) fn copy_selection(s: &mut Session, cmd: &str, sel: EditSelection) -> Result<Value> {
    let segments = copy_parts(s, cmd, &sel)?;
    Ok(json!({"copied": s.clipboard.nodes.len(), "segments": segments}))
}

/// Cut: copy the selected segments, then remove the selection as Delete does (the path splits in
/// the same layer and edit mode stays on). A selection that has no segment to copy (lone points
/// with nothing attached) is removed and the clipboard is left as it was.
pub(super) fn cut_selection(s: &mut Session, sel: EditSelection) -> Result<Value> {
    let segments = if sel.parts.is_empty() { 0 } else { copy_parts(s, "edit.cut", &sel)? };
    let parts: Vec<EditPart> = vectoredit::editing(s)?.parts();
    vectoredit::delete_selection(s, parts, false)?;
    Ok(json!({"cut": sel.parts.len(), "segments": segments}))
}

/// The network of the outline of copied layer `n` in its own coordinates: a vector's segments, a
/// shape's or boolean's outline. `None` for the layers with no outline to add (text, images,
/// frames, groups, instances, …). `doc` is the file the layer was copied from.
fn outline_network(doc: &Document, n: &Node) -> Option<VectorNetwork> {
    let net = match &n.kind {
        NodeKind::Vector { network } => network.clone(),
        k if k.is_shape() || matches!(k, NodeKind::BooleanOp { .. }) => {
            let (path, rule) = figcraft_render::node_outline(doc, n)?;
            let mut net = VectorNetwork::from_path(&path);
            if matches!(n.kind, NodeKind::Line) {
                net.regions.clear();
            }
            if rule == figcraft_render::FillRule::EvenOdd {
                for r in &mut net.regions {
                    r.winding = figcraft_doc::WindingRule::EvenOdd;
                }
            }
            net
        }
        _ => return None,
    };
    (!net.segments.is_empty()).then_some(net)
}

/// `net` moved by `m` (points, handles, the regions' paint frames and the points' radii).
fn placed(mut net: VectorNetwork, m: Affine) -> Option<VectorNetwork> {
    let c = m.as_coeffs();
    if !c.iter().all(|x| x.is_finite()) {
        return None;
    }
    let linear = Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0]);
    let scale = m.determinant().abs().sqrt();
    for v in &mut net.vertices {
        let p = m * v.point();
        (v.x, v.y) = (p.x, p.y);
        v.radius *= scale;
    }
    for sg in &mut net.segments {
        let (ta, tb) = (linear * figcraft_geom::Point::new(sg.ta.0, sg.ta.1), linear * figcraft_geom::Point::new(sg.tb.0, sg.tb.1));
        sg.ta = (ta.x, ta.y);
        sg.tb = (tb.x, tb.y);
    }
    for r in &mut net.regions {
        if let Some(frame) = r.paint_space.as_mut() {
            frame.transform = m * frame.transform;
        }
    }
    let finite = net.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite())
        && net.segments.iter().all(|sg| [sg.ta.0, sg.ta.1, sg.tb.0, sg.tb.1].iter().all(|x| x.is_finite()));
    finite.then_some(net)
}

/// Paste while editing a vector: the clipboard's layers join its network in place (with `x`, `y`:
/// the copied top-left moved there), their points selected. Layers with no outline are skipped
/// (named in `skipped`); when none has one, nothing is pasted.
pub(super) fn paste_into_vector(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.paste";
    let clip = s.clipboard.clone();
    if clip.nodes.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "the clipboard is empty".into()));
    }
    let target = vectoredit::editing(s)?.id;
    let shift = match (num(p, "x"), num(p, "y"), clip.bounds) {
        (Some(x), Some(y), Some(b)) => Affine::translate((x - b.x0, y - b.y0)),
        _ => Affine::IDENTITY,
    };
    let live = s.doc()?;
    let source: &Document = clip.source.as_deref().unwrap_or(live);
    let to_local = live.world_transform(target).inverse() * shift;
    let mut pieces = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (_, nodes) in &clip.nodes {
        let Some(root) = nodes.first() else { continue };
        match outline_network(source, root).and_then(|net| placed(net, to_local * root.transform)) {
            Some(net) => pieces.push(net),
            None => skipped.push(root.name.clone()),
        }
    }
    if pieces.is_empty() {
        return Err(EngineError::Disabled(
            CMD.into(),
            "none of the copied layers has an outline to add to the vector (text, frames and groups have none)".into(),
        ));
    }
    let have = vectoredit::network(live, target).map_or(0, |n| n.segments.len());
    if have.saturating_add(pieces.iter().map(|n| n.segments.len()).sum::<usize>()) > MAX_SEGMENTS {
        return Err(EngineError::Disabled(CMD.into(), format!("a vector takes at most {MAX_SEGMENTS} segments")));
    }
    let mut pasted: Vec<usize> = Vec::new();
    vectoredit::edit(s.doc_mut()?, target, |net| {
        for piece in pieces {
            let (vo, so) = (net.vertices.len(), net.segments.len());
            pasted.extend(vo..vo + piece.vertices.len());
            net.vertices.extend(piece.vertices);
            net.segments.extend(piece.segments.into_iter().map(|mut sg| {
                sg.a += vo;
                sg.b += vo;
                sg
            }));
            net.regions.extend(piece.regions.into_iter().map(|mut r| {
                for i in r.loops.iter_mut().flatten() {
                    *i += so;
                }
                r
            }));
        }
    })?;
    let selected = pasted.clone();
    vectoredit::set_state(s, |ve| {
        ve.clear_selection();
        ve.clear_regions();
        ve.vertices = selected;
        ve.anchor = None;
        ve.pen_out = None;
        ve.width_point = None;
    })?;
    let mut out = json!({"ids": [], "edited": target, "vertices": pasted.len()});
    if !skipped.is_empty() {
        out["skipped"] = json!(skipped);
    }
    Ok(out)
}
