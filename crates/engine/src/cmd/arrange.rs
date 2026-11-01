//! Arrange: align, distribute, tidy, z-order, flips, rotations and numeric transforms.

use figcraft_doc::{Document, EffectKind, LetterSpacing, LineHeight, Node, NodeId, NodeKind, Sizing, TextAutoResize, TextStyle};
use figcraft_geom::{Affine, Point, Rect};
use serde_json::{Value, json};

use super::edit::{roots, z_sorted};
use super::{CommandSpec, ok};
use crate::params::{boolean, get, num, num_or, req_id, req_num, req_targets, string};
use crate::{Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("align.left", "Align left", "{ids?}", true, |s, p| align(s, p, Al::Left)),
        spec!("align.hcenter", "Align horizontal centers", "{ids?}", true, |s, p| align(s, p, Al::HCenter)),
        spec!("align.right", "Align right", "{ids?}", true, |s, p| align(s, p, Al::Right)),
        spec!("align.top", "Align top", "{ids?}", true, |s, p| align(s, p, Al::Top)),
        spec!("align.vcenter", "Align vertical centers", "{ids?}", true, |s, p| align(s, p, Al::VCenter)),
        spec!("align.bottom", "Align bottom", "{ids?}", true, |s, p| align(s, p, Al::Bottom)),
        spec!("distribute.horizontal", "Distribute horizontal spacing", "{ids?, spacing?}", true, |s, p| distribute(s, p, true)),
        spec!("distribute.vertical", "Distribute vertical spacing", "{ids?, spacing?}", true, |s, p| distribute(s, p, false)),
        spec!(
            "arrange.spacing",
            "Spacing",
            "{ids?, horizontal?, vertical?, fromGesture?} (page units; fromGesture uses the interaction snapshot)",
            true,
            spacing
        ),
        spec!("arrange.tidy", "Tidy up", "{ids?, spacing?}", true, tidy),
        spec!(
            "arrange.reorder",
            "Reorder selection",
            "{ids?, source, target, swap?, fromGesture?} (insert at target's visual slot; no hierarchy changes)",
            true,
            reorder
        ),
        spec!("arrange.packH", "Pack horizontal", "{ids?}", true, |s, p| pack(s, p, true)),
        spec!("arrange.packV", "Pack vertical", "{ids?}", true, |s, p| pack(s, p, false)),
        spec!("arrange.roundToPixels", "Round to pixels", "{ids?}", true, round_px),
        spec!("arrange.front", "Bring to front", "{ids?}", true, |s, p| zorder(s, p, Z::Front)),
        spec!("arrange.forward", "Bring forward", "{ids?}", true, |s, p| zorder(s, p, Z::Forward)),
        spec!("arrange.backward", "Send backward", "{ids?}", true, |s, p| zorder(s, p, Z::Backward)),
        spec!("arrange.back", "Send to back", "{ids?}", true, |s, p| zorder(s, p, Z::Back)),
        spec!("object.flipH", "Flip horizontal", "{ids?} (in vector edit mode without ids: the selected points)", true, |s, p| flip(s, p, true)),
        spec!("object.flipV", "Flip vertical", "{ids?} (in vector edit mode without ids: the selected points)", true, |s, p| flip(s, p, false)),
        spec!("object.rotate180", "Rotate 180°", "{ids?} (in vector edit mode without ids: the selected points)", true, |s, p| rotate_by(
            s, p, 180.0
        )),
        spec!("object.rotateLeft", "Rotate 90° left", "{ids?} (in vector edit mode without ids: the selected points)", true, |s, p| rotate_by(
            s, p, 90.0
        )),
        spec!("object.rotateRight", "Rotate 90° right", "{ids?} (in vector edit mode without ids: the selected points)", true, |s, p| {
            rotate_by(s, p, -90.0)
        }),
        spec!("transform.move", "Move", "{ids?, dx, dy}  (page units)", true, |s, p| {
            let (dx, dy) = (num_or(p, "dx", 0.0), num_or(p, "dy", 0.0));
            let ids = req_targets("transform.move", s, p)?;
            let doc = s.doc_mut()?;
            for i in roots(doc, &ids) {
                move_world(doc, i, dx, dy)?;
            }
            ok()
        }),
        spec!("transform.moveTo", "Move to", "{ids?, x, y} (page coordinates of the selection's top-left)", true, |s, p| {
            let ids = req_targets("transform.moveTo", s, p)?;
            let doc = s.doc_mut()?;
            let ids = roots(doc, &ids);
            let b = doc.bounds_of(&ids).ok_or_else(|| bad("transform.moveTo", "nothing to move"))?;
            let (dx, dy) = (num_or(p, "x", b.x0) - b.x0, num_or(p, "y", b.y0) - b.y0);
            for i in ids {
                move_world(doc, i, dx, dy)?;
            }
            ok()
        }),
        spec!(
            "transform.rotate",
            "Rotate",
            "{ids?, degrees, cx?, cy? (pivot, page coords; default the selection's rotation origin, else its centre)}",
            true,
            |s, p| {
                let deg = req_num("transform.rotate", p, "degrees")?;
                let ids = req_targets("transform.rotate", s, p)?;
                let c = turn_pivot(s, &ids)?.unwrap_or_default();
                let pivot = Point::new(num_or(p, "cx", c.x), num_or(p, "cy", c.y));
                let doc = s.doc_mut()?;
                for i in roots(doc, &ids) {
                    rotate_world(doc, i, pivot, deg)?;
                }
                ok()
            }
        ),
        spec!(
            "transform.rotationOrigin",
            "Move rotation origin",
            "{on? (default: toggles)} — shows the selection's rotation origin to drag (Escape hides it); not saved",
            false,
            |s, p| {
                let on = s.edit_rotation_origin(boolean(p, "on"))?;
                Ok(origin_json(s, on))
            }
        ),
        spec!(
            "transform.setRotationOrigin",
            "Set rotation origin",
            "{x, y (page coords) | reset: true} — the point the selection turns about (default its centre); kept while the selection lasts, not saved",
            false,
            |s, p| {
                let at = num(p, "x").zip(num(p, "y")).map(|(x, y)| Point::new(x, y)).filter(|_| boolean(p, "reset") != Some(true));
                if at.is_none() && boolean(p, "reset") != Some(true) {
                    return Err(bad("transform.setRotationOrigin", "give `x` and `y`, or `reset: true`"));
                }
                s.set_rotation_origin(at)?;
                let on = s.ds()?.origin_handle().is_some();
                Ok(origin_json(s, on))
            }
        ),
        spec!(
            "transform.scale",
            "Scale",
            "{ids?, factor, cx?, cy? (pivot; default top-left)} — scales size, strokes, effects, text (the K tool)",
            true,
            scale_cmd
        ),
        spec!("transform.resize", "Resize", "{ids?, width?, height?, anchor? (tl|t|tr|l|c|r|bl|b|br)}", true, resize_cmd),
    ]
}

#[derive(Clone, Copy)]
enum Al {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

/// The selection's rotation origin as `{editing, moved, x, y}` (the centre when not moved).
fn origin_json(s: &Session, editing: bool) -> Value {
    let Ok(ds) = s.ds() else { return json!({}) };
    let at = ds.origin_point().unwrap_or_default();
    json!({"editing": editing, "moved": ds.moved_origin().is_some(), "x": at.x, "y": at.y})
}

/// Move a node by `(dx, dy)` in page units.
pub fn move_world(doc: &mut Document, id: NodeId, dx: f64, dy: f64) -> Result<()> {
    let n = doc.get(id)?;
    let pw = n.parent.map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
    let d = pw.inverse() * Affine::translate((dx, dy)) * pw;
    let n = doc.get_mut(id)?;
    n.transform = d * n.transform;
    Ok(())
}

pub fn rotate_world(doc: &mut Document, id: NodeId, pivot: Point, deg: f64) -> Result<()> {
    let world = doc.world_transform(id);
    let nw = figcraft_geom::rotate_about(world, pivot, deg);
    doc.set_world_transform(id, nw)?;
    Ok(())
}

/// Whether aligning `ids` would move anything: a layer alone on the page has nothing to align to.
pub fn can_align(doc: &Document, ids: &[NodeId]) -> bool {
    align_target(doc, &roots(doc, ids)).is_some()
}

/// What `roots` align to: one layer to its parent frame, several to their combined bounds.
fn align_target(doc: &Document, roots: &[NodeId]) -> Option<Rect> {
    match roots {
        [] => None,
        [id] => {
            let parent = doc.node(*id)?.parent.filter(|pp| !doc.node(*pp).is_some_and(Node::is_page))?;
            Some(doc.world_bounds(parent).unwrap_or_default())
        }
        _ => Some(doc.bounds_of(roots).unwrap_or_default()),
    }
}

fn align(s: &mut Session, p: &Value, a: Al) -> Result<Value> {
    let ids = req_targets("align", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    let Some(target) = align_target(doc, &ids) else { return ok() };
    for i in ids {
        let Some(b) = doc.world_bounds(i) else { continue };
        let (dx, dy) = match a {
            Al::Left => (target.x0 - b.x0, 0.0),
            Al::HCenter => (target.center().x - b.center().x, 0.0),
            Al::Right => (target.x1 - b.x1, 0.0),
            Al::Top => (0.0, target.y0 - b.y0),
            Al::VCenter => (0.0, target.center().y - b.center().y),
            Al::Bottom => (0.0, target.y1 - b.y1),
        };
        move_world(doc, i, dx, dy)?;
    }
    ok()
}

fn distribute(s: &mut Session, p: &Value, horizontal: bool) -> Result<Value> {
    let ids = req_targets("distribute", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    if ids.len() < 3 && num(p, "spacing").is_none() {
        return ok();
    }
    let mut items: Vec<(NodeId, Rect)> = ids.iter().filter_map(|i| doc.world_bounds(*i).map(|b| (*i, b))).collect();
    items.sort_by(|a, b| if horizontal { a.1.x0.total_cmp(&b.1.x0) } else { a.1.y0.total_cmp(&b.1.y0) });
    let total: f64 = items.iter().map(|(_, b)| if horizontal { b.width() } else { b.height() }).sum();
    let Some(first) = items.first() else { return ok() };
    let end = items.iter().map(|(_, b)| if horizontal { b.x1 } else { b.y1 }).fold(f64::MIN, f64::max);
    let span = if horizontal { end - first.1.x0 } else { end - first.1.y0 };
    let gap = num(p, "spacing").unwrap_or((span - total) / (items.len().saturating_sub(1).max(1)) as f64);
    let mut pos = if horizontal { first.1.x0 } else { first.1.y0 };
    for (i, b) in items.clone() {
        if horizontal {
            move_world(doc, i, pos - b.x0, 0.0)?;
            pos += b.width() + gap;
        } else {
            move_world(doc, i, 0.0, pos - b.y0)?;
            pos += b.height() + gap;
        }
    }
    ok()
}

/// Tidy up: snap the selection into a grid of rows/columns with even spacing.
fn tidy(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("arrange.tidy", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    let mut items: Vec<(NodeId, Rect)> = ids.iter().filter_map(|i| doc.world_bounds(*i).map(|b| (*i, b))).collect();
    if items.len() < 2 {
        return ok();
    }
    // Group into rows by vertical centre overlap.
    items.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0));
    let mut rows: Vec<Vec<(NodeId, Rect)>> = Vec::new();
    for it in items {
        match rows.last_mut() {
            Some(row) if row.iter().any(|(_, b)| it.1.center().y < b.y1 && it.1.center().y > b.y0) => row.push(it),
            _ => rows.push(vec![it]),
        }
    }
    let all: Vec<Rect> = rows.iter().flatten().map(|(_, b)| *b).collect();
    let (Some(origin), maxw, maxh) = (
        all.iter().copied().reduce(|a, b| a.union(b)),
        all.iter().map(Rect::width).fold(0.0, f64::max),
        all.iter().map(Rect::height).fold(0.0, f64::max),
    ) else {
        return ok();
    };
    let gap = num(p, "spacing").unwrap_or_else(|| {
        // Keep the average existing horizontal gap (min 10).
        10f64.max(
            rows.iter()
                .filter(|r| r.len() > 1)
                .map(|r| {
                    (r.iter().map(|(_, b)| b.x1).fold(f64::MIN, f64::max)
                        - r.iter().map(|(_, b)| b.x0).fold(f64::MAX, f64::min)
                        - r.iter().map(|(_, b)| b.width()).sum::<f64>())
                        / (r.len() - 1) as f64
                })
                .fold(0.0, f64::max),
        )
    });
    for (ri, row) in rows.iter_mut().enumerate() {
        row.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
        for (ci, (id, b)) in row.iter().enumerate() {
            let x = origin.x0 + ci as f64 * (maxw + gap);
            let y = origin.y0 + ri as f64 * (maxh + gap);
            move_world(doc, *id, x - b.x0, y - b.y0)?;
        }
    }
    ok()
}

fn pack(s: &mut Session, p: &Value, horizontal: bool) -> Result<Value> {
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.insert("spacing".into(), json!(0.0));
    }
    distribute(s, &q, horizontal)
}

fn round_px(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("arrange.roundToPixels", s, p)?;
    let doc = s.doc_mut()?;
    for i in ids {
        let n = doc.get_mut(i)?;
        let c = n.transform.as_coeffs();
        n.transform = Affine::new([c[0], c[1], c[2], c[3], c[4].round(), c[5].round()]);
        n.width = n.width.round().max(if n.width > 0.0 { 1.0 } else { 0.0 });
        n.height = n.height.round().max(if n.height > 0.0 { 1.0 } else { 0.0 });
    }
    ok()
}

#[derive(Clone, Copy)]
enum Z {
    Front,
    Forward,
    Backward,
    Back,
}

fn zorder(s: &mut Session, p: &Value, z: Z) -> Result<Value> {
    let ids = req_targets("arrange", s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let order: Vec<NodeId> = match z {
        Z::Front | Z::Forward => ids.iter().rev().copied().collect(),
        _ => ids.clone(),
    };
    for i in order {
        let Some(parent) = doc.get(i)?.parent else { continue };
        let len = doc.get(parent)?.children.len();
        let cur = doc.index_in_parent(i).unwrap_or(0);
        let siblings_selected = |k: usize, doc: &Document| doc.node(parent).and_then(|pn| pn.children.get(k)).is_some_and(|c| ids.contains(c));
        let target = match z {
            Z::Front => len.saturating_sub(1),
            Z::Back => 0,
            Z::Forward => {
                if cur + 1 < len && !siblings_selected(cur + 1, doc) {
                    cur + 1
                } else {
                    cur
                }
            }
            Z::Backward => {
                if cur > 0 && !siblings_selected(cur - 1, doc) {
                    cur - 1
                } else {
                    cur
                }
            }
        };
        doc.reorder(i, target)?;
    }
    // Front/Back of several keep their relative order.
    if matches!(z, Z::Front) {
        for i in &ids {
            let parent = doc.get(*i)?.parent;
            if let Some(pp) = parent {
                let len = doc.get(pp)?.children.len();
                doc.reorder(*i, len)?;
            }
        }
    }
    if matches!(z, Z::Back) {
        let mut next: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
        for i in &ids {
            let Some(pp) = doc.get(*i)?.parent else { continue };
            let k = next.entry(pp).or_insert(0);
            doc.reorder(*i, *k)?;
            *k += 1;
        }
    }
    ok()
}

fn flip(s: &mut Session, p: &Value, horizontal: bool) -> Result<Value> {
    // Editing a vector with points selected: flip them in their box.
    let points = crate::vectoredit::transform::transform_in_edit_mode(s, p, |b| {
        if horizontal { b.onto(b.width, 0.0, 0.0, b.height) } else { b.onto(0.0, b.height, b.width, 0.0) }
    });
    if let Some(r) = points {
        return r;
    }
    let ids = req_targets("flip", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    let Some(b) = doc.bounds_of(&ids) else { return ok() };
    let c = b.center();
    let m = if horizontal { Affine::new([-1.0, 0.0, 0.0, 1.0, 2.0 * c.x, 0.0]) } else { Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * c.y]) };
    for i in ids {
        let w = doc.world_transform(i);
        doc.set_world_transform(i, m * w)?;
    }
    ok()
}

fn rotate_by(s: &mut Session, p: &Value, deg: f64) -> Result<Value> {
    if let Some(r) = crate::vectoredit::transform::transform_in_edit_mode(s, p, |b| figcraft_geom::rotate_about(Affine::IDENTITY, b.center(), deg)) {
        return r;
    }
    let ids = req_targets("rotate", s, p)?;
    let Some(pivot) = turn_pivot(s, &ids)? else { return ok() };
    let doc = s.doc_mut()?;
    for i in roots(doc, &ids) {
        rotate_world(doc, i, pivot, deg)?;
    }
    ok()
}

/// Where `ids` turn: the selection's moved rotation origin when they are the selection, else the
/// centre of their bounds.
fn turn_pivot(s: &Session, ids: &[NodeId]) -> Result<Option<Point>> {
    let ds = s.ds()?;
    Ok(ds.moved_origin().filter(|_| ds.selection == ids).or_else(|| ds.doc.bounds_of(&roots(&ds.doc, ids)).map(|b| b.center())))
}

/// Scale a node and everything in it uniformly (the Scale tool): sizes and positions (whatever
/// the children's constraints), strokes, effects, radii, auto layout spacing, min/max sizes,
/// layout grids, ruler guides, tiled images and text sizes.
pub fn scale_node(doc: &mut Document, id: NodeId, k: f64) -> Result<()> {
    let k = if k.is_finite() && k > 0.0 { k } else { return Ok(()) };
    let sub = doc.subtree(id);
    // The layers below `id` scale whatever their constraints, so where they are layers of an
    // instance their new size and place are overrides. The edit's own diff takes geometry that
    // changed with an ancestor's size for what constraints give, so the next rebuild of the
    // instance would put them back at their main's size (#922).
    let in_instance: Vec<bool> = sub.iter().enumerate().map(|(depth, i)| depth > 0 && crate::components::inside_instance(doc, *i)).collect();
    let tiles = |paints: &mut Vec<figcraft_color::Paint>| {
        for p in paints {
            if let figcraft_color::PaintKind::Image(img) = &mut p.kind
                && img.scale_mode == figcraft_color::ImageScaleMode::Tile
            {
                img.scale *= k;
            }
        }
    };
    for (depth, i) in sub.iter().enumerate() {
        let n = doc.get_mut(*i)?;
        let (w0, h0, t0) = (n.width, n.height, n.transform);
        tiles(&mut n.fills);
        tiles(&mut n.strokes);
        let cl = &mut n.child_layout;
        for v in [&mut cl.min_w, &mut cl.max_w, &mut cl.min_h, &mut cl.max_h].into_iter().flatten() {
            *v *= k;
        }
        for g in &mut n.grids {
            g.size *= k;
            g.gutter *= k;
            g.offset *= k;
        }
        for g in &mut n.guides {
            g.offset *= k;
        }
        n.stroke.weight *= k;
        if let Some(sides) = n.stroke.sides.as_mut() {
            for v in sides.iter_mut() {
                *v *= k;
            }
        }
        for d in &mut n.stroke.dashes {
            *d *= k;
        }
        n.corners.radii = n.corners.radii.map(|r| r * k);
        for e in &mut n.effects {
            match &mut e.kind {
                EffectKind::DropShadow(sh) | EffectKind::InnerShadow(sh) => {
                    sh.radius *= k;
                    sh.spread *= k;
                    sh.offset = (sh.offset.0 * k, sh.offset.1 * k);
                }
                EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b) => *b = b.scaled(k),
                EffectKind::Noise(nz) => {
                    nz.size *= k;
                    nz.size_y = nz.size_y.map(|v| v * k);
                }
                EffectKind::Texture(t) => {
                    t.size *= k;
                    t.size_y = t.size_y.map(|v| v * k);
                    t.radius *= k;
                }
                EffectKind::Glass(g) => {
                    g.depth *= k;
                    g.frost *= k;
                }
            }
        }
        if let Some(l) = n.layout.as_mut() {
            l.gap *= k;
            l.cross_gap *= k;
            l.padding = l.padding.map(|x| x * k);
            for t in l.column_tracks.iter_mut().chain(l.row_tracks.iter_mut()).filter(|t| t.sizing == Sizing::Fixed) {
                t.value *= k;
            }
        }
        if let NodeKind::Text(t) = &mut n.kind {
            let scale = |st: &mut TextStyle| {
                st.font_size *= k;
                if let LineHeight::Pixels(v) = &mut st.line_height {
                    *v *= k;
                }
                if let LetterSpacing::Pixels(v) = &mut st.letter_spacing {
                    *v *= k;
                }
                st.set_look(|l| {
                    for m in [&mut l.thickness, &mut l.offset] {
                        if let LineHeight::Pixels(v) = m {
                            *v *= k;
                        }
                    }
                });
            };
            scale(&mut t.style);
            for r in &mut t.runs {
                scale(&mut r.style);
            }
            t.paragraph_spacing *= k;
            t.paragraph_indent *= k;
            t.list_spacing *= k;
            if let Some(tp) = &mut t.path {
                tp.scale(k, k);
            }
        }
        if let NodeKind::Vector { network } = &mut n.kind {
            network.scale(k, k);
            for v in &mut network.vertices {
                v.radius *= k;
            }
        }
        n.width *= k;
        n.height *= k;
        if depth > 0 {
            let c = n.transform.as_coeffs();
            n.transform = Affine::new([c[0], c[1], c[2], c[3], c[4] * k, c[5] * k]);
        }
        if in_instance.get(depth).copied().unwrap_or(false) {
            if n.width != w0 || n.height != h0 {
                n.overrides.insert("size".into());
            }
            if n.transform != t0 {
                n.overrides.insert("position".into());
            }
        }
    }
    Ok(())
}

fn scale_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let k = req_num("transform.scale", p, "factor")?;
    if k <= 0.0 {
        return Err(bad("transform.scale", "`factor` must be positive"));
    }
    let ids = req_targets("transform.scale", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    let b = doc.bounds_of(&ids).unwrap_or_default();
    let pivot = Point::new(num_or(p, "cx", b.x0), num_or(p, "cy", b.y0));
    for i in ids {
        let Some(nb) = doc.world_bounds(i) else { continue };
        scale_node(doc, i, k)?;
        // Keep relative position to the pivot.
        let target = Point::new(pivot.x + (nb.x0 - pivot.x) * k, pivot.y + (nb.y0 - pivot.y) * k);
        if let Some(after) = doc.world_bounds(i) {
            move_world(doc, i, target.x - after.x0, target.y - after.y0)?;
        }
    }
    ok()
}

pub(crate) fn resize_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("transform.resize", s, p)?;
    let (col, row) = string(p, "anchor").and_then(|a| crate::tools::anchor_of(&a)).unwrap_or((0, 0));
    let (ax, ay) = (f64::from(col) / 2.0, f64::from(row) / 2.0);
    let doc = s.doc_mut()?;
    let before = super::styles::bound_before(doc, &ids);
    for i in ids {
        let n = doc.get(i)?;
        let (ow, oh) = (n.width, n.height);
        let (w, h) = (num(p, "width").unwrap_or(ow).max(0.0), num(p, "height").unwrap_or(oh).max(0.0));
        let fixed_local = Point::new(ow * ax, oh * ay);
        let before = n.transform * fixed_local;
        // A section's contents stay where they are on the page, wherever its anchor puts it.
        let kept = if matches!(n.kind, NodeKind::Section { .. }) { super::props::children_worlds(doc, i) } else { Vec::new() };
        super::props::resize_node(doc, i, w, h)?;
        let n = doc.get_mut(i)?;
        let after = n.transform * Point::new(w * ax, h * ay);
        n.transform = Affine::translate(before - after) * n.transform;
        if num(p, "width").is_some() {
            n.child_layout.sizing_h = Sizing::Fixed;
        }
        if num(p, "height").is_some() {
            n.child_layout.sizing_v = Sizing::Fixed;
        }
        if let NodeKind::Text(t) = &mut n.kind {
            if num(p, "height").is_some() {
                t.fix_size(num(p, "width").is_some(), true);
            } else if t.auto_resize != TextAutoResize::Truncate {
                t.auto_resize = TextAutoResize::Height;
            }
        }
        super::props::restore_worlds(doc, &kept)?;
    }
    super::styles::detach_edited(doc, &before);
    ok()
}

/// Reorder visual slots without rebuilding or reparenting layers.
fn reorder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "arrange.reorder";
    let ids = req_targets(cmd, s, p)?;
    let source = req_id(cmd, p, "source")?;
    let target = req_id(cmd, p, "target")?;
    let ds = s.ds()?;
    let base = ds.interaction.as_ref().filter(|_| boolean(p, "fromGesture") == Some(true)).map(|i| i.doc.clone()).unwrap_or_else(|| ds.doc.clone());
    let layout =
        crate::selection_reorder::ReorderLayout::new(&base, &ids).ok_or_else(|| bad(cmd, "select a regular row, column or complete aligned grid"))?;
    let positions = layout
        .positions(source, target, boolean(p, "swap") == Some(true))
        .ok_or_else(|| bad(cmd, "source and target must belong to the selection"))?;
    // Validate every delta before editing; absolute targets always come from the gesture's
    // snapshot so crossing several slots and returning to the start does not accumulate edits.
    let doc = s.doc()?;
    let mut moves = Vec::new();
    for (id, at) in positions {
        let now = doc.world_bounds(id).ok_or_else(|| bad(cmd, "layer is no longer available"))?;
        let (dx, dy) = (at.x - now.x0, at.y - now.y0);
        if !dx.is_finite() || !dy.is_finite() {
            return Err(bad(cmd, "non-finite reorder result"));
        }
        if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
            let node = doc.get(id)?;
            let parent = node.parent.map(|p| doc.world_transform(p)).unwrap_or(Affine::IDENTITY);
            let transform = parent.inverse() * Affine::translate((dx, dy)) * parent * node.transform;
            if !transform.as_coeffs().iter().all(|v| v.is_finite()) {
                return Err(bad(cmd, "non-finite local transform"));
            }
            moves.push((id, dx, dy));
        }
    }
    if !moves.is_empty() {
        let doc = s.doc_mut()?;
        for (id, dx, dy) in moves {
            move_world(doc, id, dx, dy)?;
        }
    }
    ok()
}

/// Set the distances between smart-selection tracks without rebuilding/reparenting the layers.
fn spacing(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("arrange.spacing", s, p)?;
    // A scrub is absolute from the interaction snapshot, even if tracks overlap or cross.
    let ds = s.ds()?;
    let base = ds.interaction.as_ref().filter(|_| boolean(p, "fromGesture") == Some(true)).map(|i| i.doc.clone()).unwrap_or_else(|| ds.doc.clone());
    let layout = crate::selection_spacing::selection_spacing(&base, &ids);
    let mut offsets = std::collections::BTreeMap::<NodeId, (Option<f64>, Option<f64>)>::new();
    let mut any = false;
    for (key, axis, vertical) in [("horizontal", &layout.horizontal, false), ("vertical", &layout.vertical, true)] {
        if get(p, key).is_none() {
            continue;
        }
        let value = req_num("arrange.spacing", p, key)?;
        if value.abs() > 1_000_000.0 {
            return Err(bad("arrange.spacing", "spacing out of range"));
        }
        let axis = axis.as_ref().ok_or_else(|| bad("arrange.spacing", "selection has no spacing on this axis"))?;
        any = true;
        for (id, delta) in axis.offsets(value) {
            if !delta.is_finite() {
                return Err(bad("arrange.spacing", "non-finite spacing result"));
            }
            let pair = offsets.entry(id).or_default();
            if vertical {
                pair.1 = Some(delta);
            } else {
                pair.0 = Some(delta);
            }
        }
    }
    if !any {
        return Err(bad("arrange.spacing", "pass horizontal or vertical spacing"));
    }
    let doc = s.doc_mut()?;
    let mut moves = Vec::new();
    for (id, (dx, dy)) in offsets {
        let (Some(before), Some(now)) = (base.world_bounds(id), doc.world_bounds(id)) else { continue };
        let (dx, dy) = (dx.map_or(0.0, |d| before.x0 + d - now.x0), dy.map_or(0.0, |d| before.y0 + d - now.y0));
        if !dx.is_finite() || !dy.is_finite() {
            return Err(bad("arrange.spacing", "non-finite spacing result"));
        }
        moves.push((id, dx, dy));
    }
    for (id, dx, dy) in moves {
        move_world(doc, id, dx, dy)?;
    }
    ok()
}
