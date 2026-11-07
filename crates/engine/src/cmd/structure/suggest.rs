//! Suggest auto layout (#1823): split a frame's layers into rows or columns by the gaps between
//! them, wrap each band that holds several layers in a frame of its own and do the same inside
//! it, so the design gets as many auto layout frames as it needs and keeps its placement.

use figcraft_doc::{AutoLayout, Document, LayoutMode, NodeId, NodeKind, Sizing};
use figcraft_geom::Rect;
use serde_json::{Value, json};

use super::{Wrap, convert, infer_layout, sort_layout_children, wrap};
use crate::params::{id_of, req_targets, string};
use crate::{EngineError, Result, Session, bad};

/// How deep nested band frames go before a frame is laid out as a single list.
const MAX_DEPTH: usize = 16;
/// The most layers analysed in one frame; past this the frame is laid out as a single list.
const MAX_CHILDREN: usize = 1000;
/// Gaps that differ by more than this are reported.
const GAP_TOLERANCE: f64 = 0.5;
/// Edges closer than this count as touching.
const EDGE_EPS: f64 = 1e-6;

/// One row or column: the layers in it and the span it covers along both axes.
struct Band {
    members: Vec<(NodeId, Rect)>,
    /// Start and end along the split axis.
    lo: f64,
    hi: f64,
    /// Start and end along the other axis.
    cross_lo: f64,
    cross_hi: f64,
}

fn span(r: &Rect, vertical: bool) -> (f64, f64, f64, f64) {
    if vertical { (r.y0, r.y1, r.x0, r.x1) } else { (r.x0, r.x1, r.y0, r.y1) }
}

/// Split the layers into bands along one axis: sorted by their start, a layer joins the current
/// band while it starts before the band ends, and opens a new band when there is a gap or it
/// only touches. Layers starting together always share a band.
fn bands(boxes: &[(NodeId, Rect)], vertical: bool) -> Vec<Band> {
    let mut sorted = boxes.to_vec();
    sorted.sort_by(|a, b| span(&a.1, vertical).0.total_cmp(&span(&b.1, vertical).0));
    let mut out: Vec<Band> = Vec::new();
    for item in sorted {
        let (lo, hi, cross_lo, cross_hi) = span(&item.1, vertical);
        match out.last_mut() {
            Some(b) if lo < b.hi - EDGE_EPS || lo - b.lo < EDGE_EPS => {
                b.hi = b.hi.max(hi);
                b.cross_lo = b.cross_lo.min(cross_lo);
                b.cross_hi = b.cross_hi.max(cross_hi);
                b.members.push(item);
            }
            _ => out.push(Band { members: vec![item], lo, hi, cross_lo, cross_hi }),
        }
    }
    out
}

/// The layers a frame lays out, with their boxes in the frame's own space. `None` when a box
/// isn't a finite number.
fn flow_boxes(doc: &Document, frame: NodeId) -> Option<Vec<(NodeId, Rect)>> {
    let mut out = Vec::new();
    for c in figcraft_layout::flow_children(doc, frame) {
        let n = doc.node(c)?;
        let r = figcraft_geom::transform_rect_bbox(n.transform, n.local_rect());
        if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        out.push((c, r));
    }
    Some(out)
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { (v * 1000.0).round() / 1000.0 } else { 0.0 }
}

fn mode_from(direction: &str) -> LayoutMode {
    match direction.to_ascii_lowercase().as_str() {
        "vertical" | "column" => LayoutMode::Vertical,
        "grid" => LayoutMode::Grid,
        _ => LayoutMode::Horizontal,
    }
}

/// The auto layout that places `bands` where they are: the gap between them, the room before the
/// first and after the last, and on the other axis the smallest offset from each edge.
fn layout_for(bands: &[Band], vertical: bool, frame_w: f64, frame_h: f64, name: &str, warnings: &mut Vec<String>) -> AutoLayout {
    let gaps: Vec<f64> = bands.windows(2).filter_map(|w| Some((w.get(1)?.lo - w.first()?.hi).max(0.0))).collect();
    let smallest = gaps.iter().copied().fold(f64::INFINITY, f64::min);
    let largest = gaps.iter().copied().fold(0.0, f64::max);
    if largest - smallest > GAP_TOLERANCE {
        let what = if vertical { "rows" } else { "columns" };
        warnings.push(format!(
            "\"{name}\": the gaps between the {what} differ ({} to {}), so the smallest is used and later {what} move",
            finite_or_zero(smallest),
            finite_or_zero(largest)
        ));
    }
    let (main_size, cross_size) = if vertical { (frame_h, frame_w) } else { (frame_w, frame_h) };
    let main_start = bands.first().map_or(0.0, |b| b.lo).max(0.0);
    let main_end = (main_size - bands.last().map_or(0.0, |b| b.hi)).max(0.0);
    let cross_start = bands.iter().map(|b| b.cross_lo).fold(f64::INFINITY, f64::min).max(0.0);
    let cross_end = bands.iter().map(|b| cross_size - b.cross_hi).fold(f64::INFINITY, f64::min).max(0.0);
    // Every band is lined up at the start across the flow, so one that starts further in (a
    // centred title above a card) moves to the smallest offset.
    let widest_start = bands.iter().map(|b| b.cross_lo).fold(f64::NEG_INFINITY, f64::max);
    let smallest_start = bands.iter().map(|b| b.cross_lo).fold(f64::INFINITY, f64::min);
    if widest_start - smallest_start > GAP_TOLERANCE {
        let what = if vertical { "rows" } else { "columns" };
        warnings.push(format!(
            "\"{name}\": the {what} start at different offsets ({} to {}), so they are lined up at the smallest and some move",
            finite_or_zero(smallest_start),
            finite_or_zero(widest_start)
        ));
    }
    let [ms, me, cs, ce] = [main_start, main_end, cross_start, cross_end].map(finite_or_zero);
    AutoLayout {
        mode: if vertical { LayoutMode::Vertical } else { LayoutMode::Horizontal },
        gap: finite_or_zero(smallest),
        // Top, right, bottom, left.
        padding: if vertical { [ms, ce, me, cs] } else { [cs, me, ce, ms] },
        ..Default::default()
    }
}

/// Lay a frame out as one list the way "Add auto layout" does: the direction is inferred, or
/// `direction` forces it.
fn flat_layout(s: &mut Session, frame: NodeId, direction: Option<&str>) -> Result<()> {
    let doc = s.doc_mut()?;
    let mut al = infer_layout(doc, frame);
    if let Some(d) = direction {
        al.mode = mode_from(d);
    }
    let mut kids = doc.get(frame)?.children.clone();
    sort_layout_children(doc, &mut kids, al.mode == LayoutMode::Vertical);
    let n = doc.get_mut(frame)?;
    n.children = kids;
    n.layout = Some(al);
    Ok(())
}

/// Turn the frame's layers into bands and give the frame, and every band frame made on the way,
/// the auto layout that keeps them where they are. Returns whether the frame got auto layout.
fn split(s: &mut Session, frame: NodeId, depth: usize, direction: Option<&str>, warnings: &mut Vec<String>) -> Result<bool> {
    let doc = s.doc()?;
    let name = doc.get(frame)?.name.clone();
    let (frame_w, frame_h) = (doc.get(frame)?.width, doc.get(frame)?.height);
    let any_mask = doc.get(frame)?.children.iter().filter_map(|c| doc.node(*c)).any(|c| c.is_mask);
    let Some(boxes) = flow_boxes(doc, frame) else {
        warnings.push(format!("\"{name}\": a layer has no usable position, so no auto layout was added"));
        return Ok(false);
    };
    if boxes.len() < 2 {
        flat_layout(s, frame, direction)?;
        return Ok(true);
    }
    if depth >= MAX_DEPTH || boxes.len() > MAX_CHILDREN || any_mask {
        let why = if any_mask {
            "it has a mask"
        } else if depth >= MAX_DEPTH {
            "the frames are nested too deep"
        } else {
            "it has too many layers to analyse"
        };
        warnings.push(format!("\"{name}\": {why}, so its layers are laid out in one list"));
        flat_layout(s, frame, direction)?;
        return Ok(true);
    }
    // The direction asked for decides the axis of the top level; nested frames follow the gaps.
    let forced = direction.map(mode_from);
    let (vertical, found) = match forced {
        Some(LayoutMode::Vertical) => (true, bands(&boxes, true)),
        Some(LayoutMode::Horizontal) => (false, bands(&boxes, false)),
        Some(_) => (true, Vec::new()),
        None => {
            let rows = bands(&boxes, true);
            if rows.len() >= 2 { (true, rows) } else { (false, bands(&boxes, false)) }
        }
    };
    if found.len() < 2 {
        if forced.is_some() {
            if forced != Some(LayoutMode::Grid) {
                warnings.push(format!("\"{name}\": its layers don't separate along the direction asked for, so they are laid out in one list"));
            }
            flat_layout(s, frame, direction)?;
            return Ok(true);
        }
        warnings.push(format!("\"{name}\": its layers overlap along both axes, so no auto layout was added"));
        return Ok(false);
    }
    let al = layout_for(&found, vertical, frame_w, frame_h, &name, warnings);
    // Band frames first, while the layers are still where they were.
    let mut made = Vec::new();
    for band in &found {
        if band.members.len() < 2 {
            continue;
        }
        let ids: Vec<NodeId> = band.members.iter().map(|(id, _)| *id).collect();
        let r = wrap(s, &json!({"ids": ids}), "layout.suggest", Wrap::Frame)?;
        let fid = id_of(&r["id"]).ok_or_else(|| EngineError::Other("wrap failed".into()))?;
        let f = s.doc_mut()?.get_mut(fid)?;
        f.fills.clear();
        f.clip = false;
        made.push(fid);
    }
    let doc = s.doc_mut()?;
    let mut kids = doc.get(frame)?.children.clone();
    sort_layout_children(doc, &mut kids, vertical);
    let n = doc.get_mut(frame)?;
    n.children = kids;
    n.layout = Some(al);
    for fid in made {
        if split(s, fid, depth + 1, None, warnings)? {
            // A band frame that got auto layout is as big as what is in it (one that did not stays Fixed).
            let f = s.doc_mut()?.get_mut(fid)?;
            f.child_layout.sizing_h = Sizing::Hug;
            f.child_layout.sizing_v = Sizing::Hug;
        }
    }
    Ok(true)
}

/// Add auto layout to the frames, group or layers asked for the way "Add auto layout" finds its
/// targets, then split each into nested rows and columns by the gaps between its layers.
pub(super) fn layout_suggest(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("layout.suggest", s, p)?;
    let direction = string(p, "direction");
    let doc = s.doc()?;
    // An instance's layers come from its main component: nothing is added to them. Checked
    // before anything changes, with the words Convert to group uses.
    for i in &ids {
        let n = doc.get(*i)?;
        if matches!(n.kind, NodeKind::Instance { .. }) {
            return Err(bad("layout.suggest", format!("\"{}\" is an instance: its layers come from its main component (detach it first)", n.name)));
        }
        if crate::components::owning_instance(doc, *i).is_some() {
            return Err(bad("layout.suggest", format!("\"{}\" is inside an instance: edit its main component, or detach the instance", n.name)));
        }
    }
    let all_frames = ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && n.layout.is_none()));
    let single_group = ids.len() == 1 && ids.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Group));
    // A frame with auto layout already is wrapped, like a layer, and so are several layers.
    let (targets, own_size) = if all_frames {
        (ids, true)
    } else if single_group {
        // The group's layers are laid out directly, not inside a wrapper.
        convert(s, &json!({"ids": ids}), "layout.suggest", true)?;
        (ids, false)
    } else {
        let r = wrap(s, &json!({"ids": ids}), "layout.suggest", Wrap::Frame)?;
        let fid = id_of(&r["id"]).ok_or_else(|| EngineError::Other("wrap failed".into()))?;
        let f = s.doc_mut()?.get_mut(fid)?;
        f.fills.clear();
        f.clip = false;
        (vec![fid], false)
    };
    let mut warnings = Vec::new();
    for f in &targets {
        // A frame keeps its size; a wrapper or a converted group has none of its own and hugs.
        let got_layout = split(s, *f, 0, direction.as_deref(), &mut warnings)?;
        if got_layout && !own_size {
            let n = s.doc_mut()?.get_mut(*f)?;
            if !single_group || n.child_layout.sizing_h != Sizing::Fill {
                n.child_layout.sizing_h = Sizing::Hug;
            }
            if !single_group || n.child_layout.sizing_v != Sizing::Fill {
                n.child_layout.sizing_v = Sizing::Hug;
            }
        }
    }
    s.set_selection(targets.clone());
    let mut out = json!({"ids": targets});
    if !warnings.is_empty() {
        out["warnings"] = json!(warnings);
    }
    Ok(out)
}
