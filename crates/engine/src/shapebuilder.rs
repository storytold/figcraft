//! The shape builder: build shapes from the overlapping regions of the selected shapes.
//!
//! The selected shapes' outlines split the plane into **regions** (the faces of their arrangement):
//! connected areas covered by the same shapes. Merging regions makes one new vector from them,
//! styled like the top-most shape covering the first region; removing regions deletes them. Either
//! way every shape covering a picked region loses that area (it becomes a vector, or is deleted
//! when nothing is left), so the rest of the drawing stays exactly where it was.
//!
//! **Strokes and open paths.** A visible stroke takes part as its outlined area (a [`Part`] of its
//! own, above the shape's fill), so stroked lines and open paths split the regions they cross and
//! the band a stroke paints is a region too. When a stroked shape takes part in a merge or a
//! removal, its stroke becomes an outlined vector (filled with the stroke's paints) just above it,
//! so both keep looking the same; open paths, which have no fill, are replaced by that outline. A
//! region merged from a stroke is filled with the stroke's paints.
//!
//! With the Shape builder tool (Shift+M) the canvas highlights the region under the pointer; a
//! click or a drag across regions merges them, and Alt (or the tool's Remove mode) removes them.
//! Each gesture is one command, so one undo step. The arrangement is cached per document version
//! and selection, so hovering doesn't recompute it.

use std::collections::BTreeSet;
use std::sync::Arc;

use figcraft_doc::*;
use figcraft_geom::kurbo::PathEl;
use figcraft_geom::{Affine, BezPath, Point, Rect, Shape};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::cmd::edit::{roots, z_sorted};
use crate::params::*;
use crate::tools::{Cursor, Gesture, PointerEvent, PointerKind, ToolId};
use crate::{EngineError, Result, Session, bad, spec};

/// One input of the arrangement: a shape's fill area, or its stroke's outlined area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Part {
    pub id: NodeId,
    pub stroke: bool,
}

/// One region of the arrangement, in page coordinates.
#[derive(Clone, Debug)]
pub struct Region {
    /// The region's outer contour and holes.
    pub path: BezPath,
    /// The shapes covering it, top-most first.
    pub sources: Vec<NodeId>,
    /// The fills and strokes covering it, top-most first.
    pub parts: Vec<Part>,
    pub bounds: Rect,
    /// The outline flattened to polygons (outer first), for drawing.
    pub polys: Vec<Vec<Point>>,
    /// A point inside the region.
    pub point: Option<Point>,
}

/// The regions formed by a set of shapes, for one version of the document.
#[derive(Clone, Debug)]
pub struct Arrangement {
    doc: Arc<Document>,
    uid: u64,
    ids: Vec<NodeId>,
    /// The shapes taking part, bottom to top.
    pub sources: Vec<NodeId>,
    /// Their fills and strokes, bottom to top (a shape's stroke above its fill).
    pub parts: Vec<Part>,
    pub regions: Vec<Region>,
}

impl Arrangement {
    /// The region under page point `p`.
    pub fn region_at(&self, p: Point) -> Option<usize> {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        self.regions.iter().position(|r| r.bounds.contains(p) && r.path.winding(p) != 0)
    }

    fn is_for(&self, doc: &Arc<Document>, uid: u64, ids: &[NodeId]) -> bool {
        self.uid == uid && Arc::ptr_eq(&self.doc, doc) && self.ids == ids
    }
}

/// Can `id` take part? Visible, unlocked shapes (lines and open paths through their strokes),
/// boolean groups and text, outside instances and boolean groups (whose structure is fixed).
fn usable(doc: &Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let kind_ok = n.kind.is_shape() || matches!(n.kind, NodeKind::BooleanOp { .. } | NodeKind::Text(_));
    kind_ok
        && n.visible
        && !n.locked
        && !doc.ancestors(id).iter().any(|a| doc.node(*a).is_some_and(|x| matches!(x.kind, NodeKind::Instance { .. } | NodeKind::BooleanOp { .. })))
}

/// The shapes `ids` stand for (groups contribute their shapes), bottom to top.
pub fn eligible(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, depth: usize, out: &mut Vec<NodeId>) {
        let Some(n) = doc.node(id) else { return };
        if matches!(n.kind, NodeKind::Group) && depth < 64 {
            for c in &n.children {
                walk(doc, *c, depth + 1, out);
            }
        } else if usable(doc, id) {
            out.push(id);
        }
    }
    let ids: Vec<NodeId> = ids.iter().copied().filter(|i| doc.contains(*i)).collect();
    let mut out = Vec::new();
    for id in roots(doc, &ids) {
        walk(doc, id, 0, &mut out);
    }
    let mut out = z_sorted(doc, &out);
    out.dedup();
    out.truncate(figcraft_pathops::MAX_ARRANGEMENT_INPUTS);
    out
}

/// A part's area in page coordinates and whether it fills even-odd: the shape's fill area (closed
/// shapes, boolean groups and text) or its visible stroke's outlined area.
fn world_area(doc: &Document, part: Part) -> Option<(BezPath, bool)> {
    let n = doc.node(part.id)?;
    let (mut p, rule) = if part.stroke {
        figcraft_render::stroke_fill(doc, n).map(|(p, rule)| ((*p).clone(), rule))?
    } else {
        if !(figcraft_doc::geometry::is_closed(n) || matches!(n.kind, NodeKind::BooleanOp { .. } | NodeKind::Text(_))) {
            return None;
        }
        figcraft_render::node_outline(doc, n)?
    };
    p.apply_affine(doc.world_transform(part.id));
    (!p.elements().is_empty() && figcraft_pathops::finite(&p)).then_some((p, rule == figcraft_render::FillRule::EvenOdd))
}

/// Flatten a region outline to polygons for drawing (tolerance relative to its size).
fn polygons(path: &BezPath, bounds: Rect) -> Vec<Vec<Point>> {
    let tol = (bounds.width().max(bounds.height()) * 2e-4).clamp(0.01, 0.5);
    let mut polys: Vec<Vec<Point>> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(last) = polys.last_mut() {
                last.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() > 2);
    polys
}

fn compute(doc: Arc<Document>, uid: u64, ids: &[NodeId]) -> Result<Arrangement> {
    let mut sources = Vec::new();
    let mut parts = Vec::new();
    let mut inputs = Vec::new();
    for id in eligible(&doc, ids) {
        let before = parts.len();
        for part in [Part { id, stroke: false }, Part { id, stroke: true }] {
            if inputs.len() < figcraft_pathops::MAX_ARRANGEMENT_INPUTS
                && let Some(o) = world_area(&doc, part)
            {
                parts.push(part);
                inputs.push(o);
            }
        }
        if parts.len() > before {
            sources.push(id);
        }
    }
    let faces = figcraft_pathops::faces(&inputs).map_err(|e| EngineError::Other(format!("shape builder: {e}")))?;
    let mut regions: Vec<Region> = faces
        .into_iter()
        .filter(|f| f.path.area().abs() > 1e-4)
        .map(|f| {
            let bounds = f.path.bounding_box();
            let covering: Vec<Part> = f.cover.iter().rev().filter_map(|i| parts.get(*i).copied()).collect();
            let mut by: Vec<NodeId> = Vec::new();
            for p in &covering {
                if !by.contains(&p.id) {
                    by.push(p.id);
                }
            }
            Region {
                sources: by,
                parts: covering,
                polys: polygons(&f.path, bounds),
                point: figcraft_doc::vector_faces::interior_point(&f.path),
                bounds,
                path: f.path,
            }
        })
        .collect();
    // Reading order: top to bottom, then left to right.
    regions.sort_by(|a, b| a.bounds.y0.total_cmp(&b.bounds.y0).then(a.bounds.x0.total_cmp(&b.bounds.x0)));
    Ok(Arrangement { doc, uid, ids: ids.to_vec(), sources, parts, regions })
}

/// The arrangement of `ids` (default: the selection), from the cache when the document and the
/// ids haven't changed.
pub fn arrangement(s: &mut Session, ids: &[NodeId]) -> Result<Arc<Arrangement>> {
    let ds = s.ds()?;
    let (doc, uid) = (ds.doc.clone(), ds.uid);
    if let Some(a) = s.tool.shape_builder.as_ref().filter(|a| a.is_for(&doc, uid, ids)) {
        return Ok(a.clone());
    }
    let a = Arc::new(compute(doc, uid, ids)?);
    s.tool.shape_builder = Some(a.clone());
    Ok(a)
}

/// The cached arrangement of the selection, if it is up to date (for drawing).
pub fn current(s: &Session) -> Option<Arc<Arrangement>> {
    let ds = s.ds().ok()?;
    s.tool.shape_builder.as_ref().filter(|a| a.is_for(&ds.doc, ds.uid, &ds.selection)).cloned()
}

/// Region indices from `regions: [i, ...]` and `points: [[x, y], ...]`, first pick first.
fn picked(p: &Value, a: &Arrangement) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut add = |i: usize| {
        if i < a.regions.len() && !out.contains(&i) {
            out.push(i);
        }
    };
    if let Some(Value::Array(v)) = get(p, "regions") {
        for i in v.iter().filter_map(f64_of).filter(|x| *x >= 0.0 && x.fract() == 0.0 && *x < 1e9) {
            add(i as usize);
        }
    }
    if let Some(Value::Array(v)) = get(p, "points") {
        for q in v.iter().take(10_000) {
            if let (Some(x), Some(y)) = (q.get(0).and_then(f64_of), q.get(1).and_then(f64_of))
                && let Some(i) = a.region_at(Point::new(x, y))
            {
                add(i);
            }
        }
    }
    out
}

/// A vector network from a page-space path, mapped through `to_local`.
fn network_from(path: &BezPath, to_local: Affine) -> Option<VectorNetwork> {
    let mut local = path.clone();
    local.apply_affine(to_local);
    if !figcraft_pathops::finite(&local) {
        return None;
    }
    let mut net = VectorNetwork::from_path(&local);
    for r in &mut net.regions {
        r.winding = WindingRule::EvenOdd;
    }
    Some(net)
}

/// Replace shape `id`'s geometry with the page-space area `area`, keeping its transform and style.
fn reshape(doc: &mut Document, id: NodeId, area: &BezPath) -> Result<()> {
    let w = doc.world_transform(id);
    if !(w.determinant().is_finite() && w.determinant().abs() > 1e-12) {
        return Err(EngineError::Other("can't reshape a layer scaled to nothing".into()));
    }
    let net = network_from(area, w.inverse()).ok_or_else(|| EngineError::Other("invalid geometry".into()))?;
    // A boolean group's operands go with it.
    if matches!(doc.get(id)?.kind, NodeKind::BooleanOp { .. }) {
        for c in doc.get(id)?.children.clone() {
            doc.remove(c)?;
        }
    }
    let n = doc.get_mut(id)?;
    n.kind = NodeKind::Vector { network: net };
    n.children.clear();
    n.corners = Corners::default();
    crate::vectoredit::refit(doc, id)
}

/// A new vector for the page-space area `area`, styled like part `style` (a stroke part: filled
/// with the stroke's paints), just above its shape.
fn create_merged(doc: &mut Document, style: Part, area: &BezPath) -> Result<NodeId> {
    let mut src = doc.get(style.id)?.clone();
    if style.stroke {
        src.fills = std::mem::take(&mut src.strokes);
        src.stroke = StrokeProps::default();
        src.styles.fill = src.styles.stroke.take();
    }
    let style = style.id;
    let parent = src.parent.ok_or_else(|| EngineError::Other("no parent".into()))?;
    let pw = doc.world_transform(parent);
    if !(pw.determinant().is_finite() && pw.determinant().abs() > 1e-12) {
        return Err(EngineError::Other("can't place a shape in a container scaled to nothing".into()));
    }
    let mut net = network_from(area, pw.inverse()).ok_or_else(|| EngineError::Other("invalid geometry".into()))?;
    let bb = net.bounds().ok_or_else(|| EngineError::Other("empty region".into()))?;
    net.translate(-bb.x0, -bb.y0);
    let id = doc.alloc_id();
    let mut v = Node::new(id, NodeKind::Vector { network: net }, src.name.clone(), bb.width(), bb.height());
    v.transform = Affine::translate((bb.x0, bb.y0));
    v.fills = src.fills;
    v.strokes = src.strokes;
    v.stroke = src.stroke;
    v.effects = src.effects;
    v.opacity = src.opacity;
    v.blend = if matches!(src.blend, figcraft_color::BlendMode::PassThrough) { figcraft_color::BlendMode::Normal } else { src.blend };
    // In an auto layout frame the new shape stays over its region instead of joining the flow.
    v.child_layout.absolute = doc.get(parent)?.layout.is_some();
    let index = doc.index_in_parent(style).map(|i| i + 1);
    doc.insert(v, parent, index)?;
    Ok(id)
}

/// Merge (`merge`) or remove the picked regions. See the module docs for the rule.
fn apply(s: &mut Session, cmd: &str, p: &Value, merge: bool) -> Result<Value> {
    let ids = targets(s, p);
    let a = arrangement(s, &ids)?;
    let pick = picked(p, &a);
    if pick.is_empty() {
        return Err(bad(cmd, "no region picked: pass `regions` (indexes from shapeBuilder.regions) or `points` inside regions"));
    }
    let regions: Vec<&Region> = pick.iter().filter_map(|i| a.regions.get(*i)).collect();
    let touched_parts: BTreeSet<Part> = regions.iter().flat_map(|r| r.parts.iter().copied()).collect();
    let touched: BTreeSet<NodeId> = touched_parts.iter().map(|p| p.id).collect();
    // Merging every region of a single shape would only replace it with a copy of itself.
    let whole_shape =
        touched.len() == 1 && a.regions.iter().enumerate().all(|(i, r)| pick.contains(&i) || !r.sources.iter().any(|x| touched.contains(x)));
    if merge && whole_shape {
        return Ok(json!({"id": null, "changed": [], "removed": [], "outlines": [], "regions": pick}));
    }
    let doc = s.doc_mut()?;
    let created = if merge {
        let area = figcraft_pathops::union_faces(regions.iter().map(|r| &r.path)).map_err(|e| EngineError::Other(e.to_string()))?;
        let style = regions.first().and_then(|r| r.parts.first().copied()).ok_or_else(|| bad(cmd, "the region has no shape"))?;
        Some(create_merged(doc, style, &area)?)
    } else {
        None
    };
    // What part `part` keeps: the regions it covers that weren't picked, as one area (`None`:
    // nothing).
    let kept = |part: Part| -> Result<Option<BezPath>> {
        let keep: Vec<&BezPath> =
            a.regions.iter().enumerate().filter(|(i, r)| !pick.contains(i) && r.parts.contains(&part)).map(|(_, r)| &r.path).collect();
        if keep.is_empty() {
            return Ok(None);
        }
        let area = figcraft_pathops::union_faces(keep).map_err(|e| EngineError::Other(e.to_string()))?;
        Ok((!area.elements().is_empty()).then_some(area))
    };
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut outlined: Vec<(NodeId, NodeId)> = Vec::new();
    for src in a.sources.iter().copied().filter(|i| touched.contains(i)) {
        let (fill, stroke) = (Part { id: src, stroke: false }, Part { id: src, stroke: true });
        // The stroke becomes an outline of its own (what's left of it), the shape loses it.
        if a.parts.contains(&stroke) {
            if let Some(area) = kept(stroke)? {
                outlined.push((src, create_merged(doc, stroke, &area)?));
            }
            let n = doc.get_mut(src)?;
            n.strokes.clear();
            n.styles.stroke = None;
        }
        let left = if !a.parts.contains(&fill) {
            None
        } else if touched_parts.contains(&fill) {
            kept(fill)?
        } else {
            // The fill wasn't picked: the shape stays as it is.
            changed.push(src);
            continue;
        };
        match left {
            Some(area) => {
                reshape(doc, src, &area)?;
                changed.push(src);
            }
            None => {
                doc.remove(src)?;
                removed.push(src);
            }
        }
    }
    // The selection stays (minus what went, plus the outlines of selected strokes) and gains the
    // new shape unless a selected group already holds it, so the tool keeps working on the same
    // shapes.
    let mut sel: Vec<NodeId> = ids.iter().copied().filter(|i| !removed.contains(i)).collect();
    for (src, o) in &outlined {
        if ids.contains(src) {
            sel.push(*o);
        }
    }
    if let Some(c) = created
        && !doc.ancestors(c).iter().any(|x| sel.contains(x))
    {
        sel.push(c);
    }
    s.set_selection(sel);
    let outlines: Vec<NodeId> = outlined.iter().map(|(_, o)| *o).collect();
    Ok(json!({"id": created, "changed": changed, "removed": removed, "outlines": outlines, "regions": pick}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "shapeBuilder.regions",
            "Shape builder regions",
            "{ids?} → {sources (bottom to top), regions: [{index, path (SVG, page), sources (top-most first), parts: [{id, stroke}] (the fills and outlined strokes covering it, top-most first), bounds [x, y, w, h], point [x, y] inside}]} — visible strokes take part as their outlined areas, so stroked lines and open paths split regions",
            false,
            |s, p| {
                let ids = targets(s, p);
                let a = arrangement(s, &ids)?;
                let regions: Vec<Value> = a
                    .regions
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        json!({
                            "index": i,
                            "path": r.path.to_svg(),
                            "sources": r.sources,
                            "parts": r.parts,
                            "bounds": [r.bounds.x0, r.bounds.y0, r.bounds.width(), r.bounds.height()],
                            "point": r.point.map(|q| [q.x, q.y]),
                        })
                    })
                    .collect();
                Ok(json!({"sources": a.sources, "regions": regions}))
            }
        ),
        spec!(
            "shapeBuilder.merge",
            "Merge regions",
            "{ids?, regions?: [i, ...], points?: [[x, y], ...] (page)} → {id, changed, removed, outlines, regions} — one new shape from the regions, styled like the top-most fill or stroke over the first; the shapes lose those areas (strokes of the shapes involved become outlined vectors: `outlines`)",
            true,
            |s, p| apply(s, "shapeBuilder.merge", p, true)
        ),
        spec!(
            "shapeBuilder.remove",
            "Remove regions",
            "{ids?, regions?: [i, ...], points?: [[x, y], ...] (page)} → {changed, removed, outlines, regions} — the shapes (and strokes) covering the regions lose them",
            true,
            |s, p| apply(s, "shapeBuilder.remove", p, false)
        ),
        spec!("shapeBuilder.mode", "Shape builder mode", "{mode: merge|remove} (what a click does; Alt does the other)", false, |s, p| {
            let remove = match string(p, "mode").as_deref() {
                Some("merge") => false,
                Some("remove") => true,
                _ => return Err(bad("shapeBuilder.mode", "mode: merge or remove")),
            };
            s.tool.builder_remove = remove;
            s.tool.overlay.builder_remove = remove;
            if let Ok(ds) = s.ds_mut() {
                ds.revision += 1;
            }
            Ok(json!({"mode": if remove { "remove" } else { "merge" }}))
        }),
    ]
}

/// Keys while the Shape builder tool is active: Enter finishes (Escape goes through
/// [`Session::escape`]).
pub fn key(s: &mut Session, name: &str) -> Result<bool> {
    if s.tool.tool != ToolId::ShapeBuilder || !matches!(name, "Enter" | "Return") {
        return Ok(false);
    }
    s.set_tool(ToolId::Move);
    Ok(true)
}

impl Session {
    /// The Shape builder tool: hover highlights a region, click/drag collects regions, release
    /// merges (or removes, with Alt or in Remove mode) them as one undo step. Clicking outside
    /// every region selects the layer there (Shift adds), so shapes can be picked without
    /// switching tools.
    pub(crate) fn shape_builder_pointer(&mut self, ev: &PointerEvent) -> Result<()> {
        let sel = self.selection();
        // An arrangement that can't be built (bad geometry) just shows no regions.
        let arr = arrangement(self, &sel).ok();
        let at = |p: Point| arr.as_ref().and_then(|a| a.region_at(p));
        let remove = ev.alt != self.tool.builder_remove;
        self.tool.overlay.cursor = Cursor::Crosshair;
        let before = (self.tool.overlay.builder_regions.clone(), self.tool.overlay.builder_remove);
        match ev.kind {
            // While a gesture runs, the collected regions stay highlighted.
            PointerKind::Hover if !matches!(self.tool.gesture, Gesture::ShapeBuild { .. }) => {
                self.tool.overlay.builder_regions = at(ev.p()).into_iter().collect();
            }
            PointerKind::Hover => {}
            PointerKind::Down => {
                let picked: Vec<usize> = at(ev.p()).into_iter().collect();
                self.tool.overlay.builder_regions = picked.clone();
                self.tool.gesture = Gesture::ShapeBuild { picked, remove, start: ev.p(), last: ev.p() };
            }
            PointerKind::Move => {
                let Gesture::ShapeBuild { picked, last, .. } = &mut self.tool.gesture else { return Ok(()) };
                // Sample the stroke every few screen pixels so fast drags don't skip regions.
                let (from, to) = (*last, ev.p());
                let steps = ((to - from).hypot() * ev.zoom.max(1e-6) / 3.0).ceil().clamp(1.0, 500.0) as usize;
                for k in 1..=steps {
                    if let Some(i) = at(from.lerp(to, k as f64 / steps as f64))
                        && !picked.contains(&i)
                    {
                        picked.push(i);
                    }
                }
                *last = to;
                self.tool.overlay.builder_regions = picked.clone();
            }
            // A double click arrives instead of the second click's release.
            PointerKind::Up | PointerKind::DoubleClick => {
                let Gesture::ShapeBuild { picked, remove, start, .. } = std::mem::take(&mut self.tool.gesture) else { return Ok(()) };
                self.tool.overlay.builder_regions.clear();
                if picked.is_empty() {
                    if (ev.p() - start).hypot() * ev.zoom < 3.0 {
                        self.builder_select(ev);
                    }
                } else {
                    let cmd = if remove { "shapeBuilder.remove" } else { "shapeBuilder.merge" };
                    self.execute(cmd, &json!({"ids": sel, "regions": picked}))?;
                }
            }
        }
        // A gesture keeps the mode it started with.
        self.tool.overlay.builder_remove = match &self.tool.gesture {
            Gesture::ShapeBuild { remove, .. } => *remove,
            _ => remove,
        };
        if before != (self.tool.overlay.builder_regions.clone(), self.tool.overlay.builder_remove)
            && let Ok(ds) = self.ds_mut()
        {
            ds.revision += 1;
        }
        Ok(())
    }

    /// A click outside the regions: select the layer under the pointer (Shift toggles it).
    fn builder_select(&mut self, ev: &PointerEvent) {
        let Ok(ds) = self.ds() else { return };
        let (doc, page, mut sel) = (ds.doc.clone(), ds.page, ds.selection.clone());
        let Some(leaf) = self.hit(&doc, page, ev.p(), 2.0 / ev.zoom.max(1e-6)) else { return };
        let t = figcraft_doc::geometry::select_target(&doc, leaf, &sel, ev.cmd);
        if ev.shift {
            if let Some(i) = sel.iter().position(|x| *x == t) {
                sel.remove(i);
            } else {
                sel.push(t);
            }
        } else {
            sel = vec![t];
        }
        self.set_selection(sel);
    }
}
