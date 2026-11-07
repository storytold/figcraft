//! Vector edit mode through the session: tools, pointer gestures, keys and undo.

use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::tools::{PointerEvent, PointerKind};

pub(super) fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"))
}

pub(super) fn net(s: &Session, id: NodeId) -> VectorNetwork {
    network(s.doc().unwrap(), id).unwrap().clone()
}

pub(super) fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::at(kind, x, y, 1.0)
}

pub(super) fn click(s: &mut Session, x: f64, y: f64) {
    s.pointer(&ev(PointerKind::Down, x, y)).unwrap();
    s.pointer(&ev(PointerKind::Up, x, y)).unwrap();
}

pub(super) fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&ev(PointerKind::Down, from.0, from.1)).unwrap();
    s.pointer(&ev(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, to.0, to.1)).unwrap();
    s.pointer(&ev(PointerKind::Up, to.0, to.1)).unwrap();
}

/// A 100×100 rectangle at the origin, in edit mode.
pub(super) fn rect(s: &mut Session) -> NodeId {
    let r = ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(s, "vector.edit.begin", json!({"id": id}));
    id
}

pub(super) fn ve(s: &Session) -> VectorEdit {
    s.ds().unwrap().vector_edit.clone().unwrap()
}

pub(super) fn vertex_at(n: &VectorNetwork, x: f64, y: f64) -> usize {
    n.vertices.iter().position(|v| (v.x - x).abs() < 1e-6 && (v.y - y).abs() < 1e-6).unwrap()
}

pub(super) fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

#[test]
fn pen_extends_branches_and_closes_paths() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    // Click the end point to continue from it, then add a point below and close onto the start.
    click(&mut s, 100.0, 0.0);
    assert!(ve(&s).anchor.is_some());
    click(&mut s, 100.0, 100.0);
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (3, 2));
    click(&mut s, 0.0, 0.0);
    let n = net(&s, id);
    assert_eq!(n.segments.len(), 3);
    assert_eq!(n.regions.len(), 1, "closing the path makes a fill region");
    assert!(ve(&s).anchor.is_none(), "closing ends the path");
    // Branch from an existing corner.
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 150.0);
    let n = net(&s, id);
    let corner = vertex_at(&n, 100.0, 100.0);
    assert_eq!(n.incident(corner).len(), 3, "a vector network branch");
    // Escape ends the path but stays in edit mode; the next Escape leaves.
    s.escape().unwrap();
    assert!(ve(&s).anchor.is_none());
    s.escape().unwrap();
    assert!(s.ds().unwrap().vector_edit.is_none());
}

#[test]
fn pen_drag_makes_a_smooth_point() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    click(&mut s, 100.0, 0.0);
    drag(&mut s, (150.0, 50.0), (150.0, 80.0));
    let n = net(&s, id);
    let v = n.vertices.len() - 1;
    assert_eq!(n.mirroring(v), HandleMirroring::AngleAndLength);
    let (seg, start) = n.incident(v)[0];
    let h = n.handle(seg, start).unwrap();
    assert!(h.0.abs() < 1e-6 && (h.1 + 30.0).abs() < 1e-6, "incoming handle mirrors the drag: {h:?}");
    assert_eq!(ve(&s).pen_out, Some((0.0, 30.0)));
    // The next segment starts with the dragged-out handle.
    click(&mut s, 250.0, 50.0);
    let n = net(&s, id);
    let last = *n.segments.last().unwrap();
    assert_eq!(last.ta, (0.0, 30.0));
    // One undo step per click.
    assert!(s.undo().unwrap());
    assert_eq!(net(&s, id).vertices.len(), n.vertices.len() - 1);
}

#[test]
fn bend_tool_smooths_and_sharpens_points() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "bend"}));
    drag(&mut s, (100.0, 0.0), (130.0, 0.0));
    let n = net(&s, id);
    let v = vertex_at(&n, 100.0, 0.0);
    assert_eq!(n.mirroring(v), HandleMirroring::Angle);
    assert!(n.incident(v).iter().all(|(s, st)| n.handle(*s, *st) != Some((0.0, 0.0))));
    click(&mut s, 100.0, 0.0);
    let n = net(&s, id);
    let v = vertex_at(&n, 100.0, 0.0);
    assert!(n.incident(v).iter().all(|(s, st)| n.handle(*s, *st) == Some((0.0, 0.0))), "a click sharpens it");
    // Clicked again, the sharp corner curves: handles along the line between its neighbours
    // (0,0) and (100,100), a third of the way to each (the box refits around the bulge, so the
    // point is found by its index).
    click(&mut s, 100.0, 0.0);
    let n = net(&s, id);
    assert_eq!(n.mirroring(v), HandleMirroring::Angle);
    for (seg, st) in n.incident(v) {
        let h = n.handle(seg, st).unwrap();
        assert!((h.0.abs() - 100.0 / 3.0 / 2f64.sqrt()).abs() < 1e-6 && (h.0 - h.1).abs() < 1e-6, "along the neighbours' line: {h:?}");
    }
    // ⌘-drag on a segment with the Move tool bends it.
    ex(&mut s, "vector.edit.tool", json!({"tool": "move"}));
    let mut down = ev(PointerKind::Down, 0.0, 50.0);
    down.cmd = true;
    s.pointer(&down).unwrap();
    s.pointer(&ev(PointerKind::Move, -40.0, 50.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, -40.0, 50.0)).unwrap();
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert!(b.x0 < -25.0, "bent outwards: {b:?}");
}

/// #1875: a line whose middle point was moved off the line, clicked with the Bend tool, curves
/// there ("Click on a point or path where you want to add a curve", Edit vector layers): the
/// handles run along the line between the neighbours, a third of the way to each, in one undo
/// step; an endpoint is left alone; a click on the curved point makes it sharp again.
#[test]
fn a_bend_click_curves_a_sharp_point_between_its_neighbours() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [50, 30], [100, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "bend"}));
    let (v, end) = (vertex_at(&net(&s, id), 50.0, 30.0), vertex_at(&net(&s, id), 0.0, 0.0));
    let before = s.undo_depth().0;
    click(&mut s, 50.0, 30.0);
    let n = net(&s, id);
    let reach = (50f64.powi(2) + 30f64.powi(2)).sqrt() / 3.0;
    for (seg, st) in n.incident(v) {
        let h = n.handle(seg, st).unwrap();
        assert!((h.0.abs() - reach).abs() < 1e-6 && h.1.abs() < 1e-6, "along the neighbours' line, a third of the way: {h:?}");
    }
    assert_eq!(n.mirroring(v), HandleMirroring::Angle);
    assert_eq!(s.undo_depth().0, before + 1, "one undo step");
    // An endpoint has nothing to curve: the click changes nothing and leaves no undo step.
    click(&mut s, 0.0, 0.0);
    let n = net(&s, id);
    assert!(n.incident(end).iter().all(|(sg, st)| n.handle(*sg, *st) == Some((0.0, 0.0))));
    assert_eq!(s.undo_depth().0, before + 1);
    // The curved point clicked again is sharp again.
    click(&mut s, 50.0, 30.0);
    let n = net(&s, id);
    assert!(n.incident(v).iter().all(|(sg, st)| n.handle(*sg, *st) == Some((0.0, 0.0))));
}

/// Between a handle's end and a point both within reach, the nearer on the page wins, also under a
/// non-uniform scale (10× wide here), where local distances would pick the point.
#[test]
fn a_handle_and_a_point_are_compared_on_the_page() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [5, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    {
        let node = s.doc_mut().unwrap().get_mut(id).unwrap();
        let mut n = VectorNetwork::default();
        n.vertices.push(figcraft_doc::Vertex::new(0.0, 0.0));
        n.vertices.push(figcraft_doc::Vertex::new(5.0, 0.0));
        let mut seg = figcraft_doc::Segment::line(0, 1);
        seg.ta = (0.5, -0.9);
        n.segments.push(seg);
        node.kind = figcraft_doc::NodeKind::Vector { network: n };
        node.transform = figcraft_geom::Affine::scale_non_uniform(10.0, 1.0);
    }
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [0]}));
    let doc = s.doc().unwrap().clone();
    // On the page the point is at (0, 0) and its handle's end at (5, -0.9): from (4, 0) the
    // handle's end is 1.3 away and the point 4, though locally the point is the nearer.
    assert_eq!(hit(&doc, &ve(&s), pt(4.0, 0.0), 6.0), Some(VHit::Handle { seg: 0, start: true }));
    assert_eq!(hit(&doc, &ve(&s), pt(0.0, 0.0), 6.0), Some(VHit::Vertex { index: 0 }));
}

/// A handle whose end lies exactly on another point (snapped there) can still be taken there: on a
/// tie the handle wins, as handles always did.
#[test]
fn a_handle_on_another_point_is_still_the_handles() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [10, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    {
        let node = s.doc_mut().unwrap().get_mut(id).unwrap();
        let mut n = VectorNetwork::default();
        n.vertices.push(figcraft_doc::Vertex::new(0.0, 0.0));
        n.vertices.push(figcraft_doc::Vertex::new(10.0, 0.0));
        let mut seg = figcraft_doc::Segment::line(0, 1);
        seg.ta = (10.0, 0.0);
        n.segments.push(seg);
        node.kind = figcraft_doc::NodeKind::Vector { network: n };
        node.transform = figcraft_geom::Affine::IDENTITY;
    }
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [0]}));
    let doc = s.doc().unwrap().clone();
    // Where the points are on the page now (entering edit mode may refit the layer's box).
    let n = net(&s, id);
    let w = doc.world_transform(id);
    let (p0, p1) = (w * n.vertices[0].point(), w * n.vertices[1].point());
    // Point 0's handle ends on point 1: there the handle; on point 0, the point.
    assert_eq!(hit(&doc, &ve(&s), p1, 6.0), Some(VHit::Handle { seg: 0, start: true }));
    assert_eq!(hit(&doc, &ve(&s), p0, 6.0), Some(VHit::Vertex { index: 0 }));
}

/// A short path's handles end a few pixels from their point: a click on the point is still on the
/// point (the nearer of the two wins), so the Bend tool can sharpen it again, while a click on the
/// handle's end still takes the handle.
#[test]
fn short_handles_leave_their_point_clickable() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [6, 10], [12, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "bend"}));
    let v = vertex_at(&net(&s, id), 6.0, 10.0);
    click(&mut s, 6.0, 10.0);
    let n = net(&s, id);
    let curved = |n: &VectorNetwork| n.incident(v).iter().all(|(sg, st)| n.handle(*sg, *st).is_some_and(|h| h != (0.0, 0.0)));
    assert!(curved(&n));
    let reach = (6f64.powi(2) + 10f64.powi(2)).sqrt() / 3.0;
    assert!(reach < 6.0, "the handles end within the 6 px hit tolerance: {reach}");
    // A click on the handle's end takes the handle (nothing changes); one on the point sharpens it.
    let (sg, st) = n.incident(v)[0];
    let h = n.handle(sg, st).unwrap();
    click(&mut s, 6.0 + h.0, 10.0 + h.1);
    assert!(curved(&net(&s, id)), "the handle's end is the handle's");
    click(&mut s, 6.0, 10.0);
    let n = net(&s, id);
    assert!(n.incident(v).iter().all(|(sg, st)| n.handle(*sg, *st) == Some((0.0, 0.0))), "the point is the point's: sharp again");
}

#[test]
fn paint_bucket_fills_and_unfills_areas() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.connect", json!({"a": 0, "b": 2}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "paint"}));
    assert!(net(&s, id).fill_contains(pt(75.0, 25.0)));
    // Hovering highlights the area; clicking unfills just that triangle.
    s.pointer(&ev(PointerKind::Hover, 75.0, 25.0)).unwrap();
    assert!(!s.tool.overlay.face.is_empty());
    click(&mut s, 75.0, 25.0);
    let n = net(&s, id);
    assert!(!n.fill_contains(pt(75.0, 25.0)));
    assert!(n.fill_contains(pt(25.0, 75.0)));
    click(&mut s, 75.0, 25.0);
    assert!(net(&s, id).fill_contains(pt(75.0, 25.0)));
    s.undo().unwrap();
    assert!(!net(&s, id).fill_contains(pt(75.0, 25.0)));
    // A vector without fills gets one when painted.
    ex(&mut s, "node.set", json!({"id": id, "props": {"fills": []}}));
    let r = ex(&mut s, "vector.edit.paint", json!({"x": 75, "y": 25}));
    assert_eq!(r["filled"], json!(true));
    assert!(!s.doc().unwrap().node(id).unwrap().fills.is_empty());
    assert!(s.execute("vector.edit.paint", &json!({"x": 500, "y": 500})).is_err(), "outside any area");
}

#[test]
fn lasso_marquee_and_segment_selection() {
    let mut s = Session::new();
    let _id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "lasso"}));
    s.pointer(&ev(PointerKind::Down, -10.0, -10.0)).unwrap();
    for (x, y) in [(110.0, -10.0), (110.0, 10.0), (-10.0, 10.0)] {
        s.pointer(&ev(PointerKind::Move, x, y)).unwrap();
    }
    assert!(!s.tool.overlay.lasso.is_empty());
    s.pointer(&ev(PointerKind::Up, -10.0, 10.0)).unwrap();
    assert_eq!(ve(&s).vertices.len(), 2, "the top two points");
    assert!(s.tool.overlay.lasso.is_empty());
    // Move tool: clicking a segment selects it; shift adds another.
    ex(&mut s, "vector.edit.tool", json!({"tool": "move"}));
    click(&mut s, 30.0, 100.0);
    assert_eq!(ve(&s).segments.len(), 1);
    assert!(ve(&s).vertices.is_empty());
    let mut sh = ev(PointerKind::Down, 0.0, 50.0);
    sh.shift = true;
    s.pointer(&sh).unwrap();
    s.pointer(&ev(PointerKind::Up, 0.0, 50.0)).unwrap();
    assert_eq!(ve(&s).segments.len(), 2);
    ex(&mut s, "vector.edit.selectAll", json!({}));
    assert_eq!(ve(&s).vertices.len(), 4);
}

#[test]
fn alt_click_adds_a_point_that_drags_in_the_same_gesture() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let mut down = ev(PointerKind::Down, 50.0, 0.0);
    down.alt = true;
    s.pointer(&down).unwrap();
    s.pointer(&ev(PointerKind::Move, 50.0, -20.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, 50.0, -20.0)).unwrap();
    assert_eq!(net(&s, id).vertices.len(), 5);
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!(b.y0, -20.0, "the new point was dragged up");
    s.undo().unwrap();
    assert_eq!(net(&s, id).vertices.len(), 4, "add + drag is one step");
}

#[test]
fn cut_tool_and_keys() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "cut"}));
    click(&mut s, 50.0, 0.0);
    assert_eq!(net(&s, id).vertices.len(), 6);
    assert_eq!(ve(&s).vertices.len(), 2);
    // Keys: tool letters, arrows nudge, ⌘A selects all, Delete removes, Enter leaves.
    assert!(key(&mut s, "V", false, false, false).unwrap());
    assert_eq!(ve(&s).tool, VTool::Move);
    assert!(key(&mut s, "Q", false, false, false).unwrap());
    assert_eq!(ve(&s).tool, VTool::Lasso);
    assert!(key(&mut s, "B", true, false, false).unwrap());
    assert_eq!(ve(&s).tool, VTool::Paint);
    let v = vertex_at(&net(&s, id), 100.0, 100.0);
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    assert!(key(&mut s, "Right", false, false, false).unwrap());
    let w = s.doc().unwrap().world_transform(id);
    assert_eq!((w * net(&s, id).vertices[v].point()).x, 101.0);
    assert!(key(&mut s, "Down", true, false, false).unwrap());
    let w = s.doc().unwrap().world_transform(id);
    assert_eq!((w * net(&s, id).vertices[v].point()).y, 110.0);
    assert!(key(&mut s, "A", false, false, true).unwrap());
    assert_eq!(ve(&s).vertices.len(), 6);
    assert!(!key(&mut s, "Z", false, false, true).unwrap(), "undo falls through to shortcuts");
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    assert!(key(&mut s, "Backspace", false, false, false).unwrap());
    assert_eq!(net(&s, id).vertices.len(), 5);
    assert!(key(&mut s, "Enter", false, false, false).unwrap());
    assert!(s.ds().unwrap().vector_edit.is_none());
    assert!(!key(&mut s, "V", false, false, false).unwrap(), "keys pass through outside edit mode");
}

#[test]
fn delete_vs_delete_and_heal() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.select", json!({"vertices": [1]}));
    ex(&mut s, "vector.edit.delete", json!({}));
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (3, 2), "the path opens");
    s.undo().unwrap();
    ex(&mut s, "vector.edit.select", json!({"vertices": [1]}));
    ex(&mut s, "vector.deleteHeal", json!({}));
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len(), n.regions.len()), (3, 3, 1), "healed and closed");
    // Deleting a selected segment keeps its points.
    s.undo().unwrap();
    ex(&mut s, "vector.edit.select", json!({"segments": [0]}));
    ex(&mut s, "vector.edit.delete", json!({}));
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (4, 3));
}

#[test]
fn mirroring_position_and_join_commands() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.select", json!({"vertices": [1]}));
    ex(&mut s, "vector.edit.smooth", json!({}));
    ex(&mut s, "vector.edit.mirroring", json!({"mode": "angleAndLength"}));
    let st = ex(&mut s, "vector.edit.state", json!({}));
    assert_eq!(st["mirroring"][1], json!("angleAndLength"));
    assert_eq!(st["tool"], json!("move"));
    assert!(s.execute("vector.edit.mirroring", &json!({"mode": "sideways"})).is_err());
    // Position: move the selected point to x = -50 in parent coordinates.
    ex(&mut s, "vector.edit.select", json!({"vertices": [0]}));
    ex(&mut s, "vector.edit.position", json!({"x": -50}));
    let p = selection_position(s.doc().unwrap(), &ve(&s)).unwrap();
    assert!((p.x + 50.0).abs() < 1e-6, "{p:?}");
    // Join: cut a corner open, then join its two ends back (they overlap: merged).
    ex(&mut s, "vector.edit.cut", json!({"vertex": 2}));
    let before = net(&s, id).vertices.len();
    ex(&mut s, "vector.join", json!({}));
    assert_eq!(net(&s, id).vertices.len(), before - 1);
    // Outside edit mode, vector.join keeps its layer behaviour.
    ex(&mut s, "vector.edit.end", json!({}));
    assert!(s.execute("vector.join", &json!({"ids": [id]})).is_ok());
}

#[test]
fn a_point_dropped_on_another_merges_into_it() {
    // #401: an open path's end dragged onto its start closes the path at that point.
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0], [100, 100], [0, 100]]}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    drag(&mut s, (0.0, 100.0), (2.0, 3.0));
    let n = net(&s, id);
    assert_eq!(n.vertices.len(), 3, "the dropped point merged");
    assert!((0..3).all(|v| n.incident(v).len() == 2), "a closed path: {n:?}");
    assert_eq!(ve(&s).vertices, vec![vertex_at(&n, 0.0, 0.0)], "the point it merged into stays selected");
    s.undo().unwrap();
    let n = net(&s, id);
    assert_eq!(n.vertices.len(), 4, "one undo step takes back the move and the merge");
    vertex_at(&n, 0.0, 100.0);
    // A drop away from every point only moves it.
    ex(&mut s, "vector.edit.select", json!({"vertices": []}));
    drag(&mut s, (0.0, 100.0), (0.0, 50.0));
    assert_eq!(net(&s, id).vertices.len(), 4);
    // A 2-point line's end dropped on its other end only moves: no segment would be left.
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]]}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    drag(&mut s, (100.0, 0.0), (2.0, 0.0));
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (2, 1), "still a line: {n:?}");
}

#[test]
fn deleting_every_point_takes_the_layer_with_the_edit_mode() {
    // Delete: no point is left at all.
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.selectAll", json!({}));
    ex(&mut s, "vector.edit.delete", json!({}));
    assert!(net(&s, id).vertices.is_empty() && net(&s, id).segments.is_empty(), "no point left");
    assert!(s.doc().unwrap().node(id).is_some(), "the layer stays while editing");
    ex(&mut s, "vector.edit.end", json!({}));
    assert!(s.doc().unwrap().node(id).is_none(), "it goes with edit mode instead of leaving an empty box");
    assert!(s.selection().is_empty());
    // The step that deleted the last points brings the shape back.
    s.undo().unwrap();
    assert_eq!(net(&s, id).vertices.len(), 4);
    // Shift+Delete (heal): the last point is left on its own with no segment, drawing nothing either.
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.selectAll", json!({}));
    ex(&mut s, "vector.deleteHeal", json!({}));
    assert!(net(&s, id).segments.is_empty(), "nothing draws: {:?}", net(&s, id));
    ex(&mut s, "vector.edit.end", json!({}));
    assert!(s.doc().unwrap().node(id).is_none(), "an unconnected point left behind doesn't keep the layer");
    // Escape ends edit mode the same way, for every layer edited along that is left empty.
    let mut s = Session::new();
    let a = crate::params::id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}))["id"]).unwrap();
    let b = crate::params::id_of(&ex(&mut s, "shape.rectangle", json!({"x": 200, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    ex(&mut s, "vector.edit.selectAll", json!({}));
    ex(&mut s, "vector.edit.delete", json!({}));
    s.escape().unwrap();
    assert!(s.ds().unwrap().vector_edit.is_none());
    assert!(s.doc().unwrap().node(a).is_none() && s.doc().unwrap().node(b).is_none());
}

#[test]
fn undo_back_past_the_conversion_leaves_edit_mode() {
    let mut s = Session::new();
    let id = rect(&mut s);
    assert!(s.ds().unwrap().vector_edit.is_some());
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(id).unwrap().kind.type_name(), "RECTANGLE");
    assert!(s.ds().unwrap().vector_edit.is_none());
    // Choosing a drawing tool leaves edit mode too.
    let _ = rect(&mut s);
    s.set_tool(crate::tools::ToolId::Rectangle);
    assert!(s.ds().unwrap().vector_edit.is_none());
}

#[test]
fn every_command_survives_junk_params_while_editing() {
    let junk = [
        json!({}),
        json!(null),
        json!({"vertices": [-1, 1e12, "x"], "segments": [99999], "segment": -5, "t": "NaN", "x": 1e308, "y": -1e308}),
        json!({"segment": 0, "t": 2, "x": 5, "y": 5, "dx": 1e300, "dy": 1, "a": 0, "b": 0, "vertex": 99}),
        json!({"points": [[0, 0], [1], "x", [5, 5], [0, 9]], "mode": 7, "tool": "nope", "radius": -3}),
    ];
    for spec in crate::all_commands().iter().filter(|c| c.id.starts_with("vector.")) {
        for j in &junk {
            let mut s = Session::new();
            let _ = rect(&mut s);
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(spec.id, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            // The network stays sane.
            if let Some(ve) = s.ds().unwrap().vector_edit.clone()
                && let Some(n) = network(s.doc().unwrap(), ve.id)
            {
                assert!(n.segments.iter().all(|x| x.a < n.vertices.len() && x.b < n.vertices.len()), "{} broke the network", spec.id);
            }
        }
    }
}

#[test]
fn eraser_strokes_cut_paths_in_one_step() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "eraser"}));
    s.pointer(&ev(PointerKind::Hover, 50.0, 0.0)).unwrap();
    assert!(s.tool.overlay.brush.is_some(), "the brush shows");
    // A stroke across the top edge.
    drag(&mut s, (50.0, -5.0), (50.0, 5.0));
    let n = net(&s, id);
    assert_eq!(n.segments.len(), 5, "the top edge is split into two with a gap");
    assert!(n.vertices.iter().all(|v| (v.point() - pt(50.0, 0.0)).hypot() >= 7.9), "nothing left under the brush");
    s.undo().unwrap();
    assert_eq!(net(&s, id).segments.len(), 4);
    assert_eq!(net(&s, id).vertices.len(), 4, "one undo step restores the square");
    // The command does the same with an explicit radius.
    let r = ex(&mut s, "vector.edit.erase", json!({"points": [[100, 50]], "radius": 5}));
    assert_eq!(r["changed"], json!(true));
    assert!(key(&mut s, "E", true, false, false).unwrap());
    assert_eq!(ve(&s).tool, VTool::Eraser);
}

#[test]
fn a_segment_shows_its_midpoint_and_a_press_adds_a_point_there() {
    let mut s = Session::new();
    let id = rect(&mut s);
    // Near the top edge but off its midpoint: the hint shows where the point would go, but the
    // pointer is not on it, so the arrow stays.
    s.pointer(&ev(PointerKind::Hover, 30.0, 0.0)).unwrap();
    let add = s.tool.overlay.vector_add.expect("the segment's midpoint");
    assert!(add.midpoint && !add.on, "{add:?}");
    assert!((add.at[0] - 50.0).abs() < 1e-9 && add.at[1].abs() < 1e-9, "{add:?}");
    assert_eq!(s.tool.overlay.cursor, crate::tools::Cursor::Default);
    // On the midpoint: the pen cursor, and a press adds a point there and selects it.
    s.pointer(&ev(PointerKind::Hover, 50.0, 0.0)).unwrap();
    assert!(s.tool.overlay.vector_add.is_some_and(|a| a.on), "the pointer is on the midpoint");
    assert_eq!(s.tool.overlay.cursor, crate::tools::Cursor::Pen(crate::tools::PenCursor::Add));
    click(&mut s, 50.0, 0.0);
    let n = net(&s, id);
    assert_eq!(n.vertices.len(), 5, "a point was added: {n:?}");
    assert_eq!(ve(&s).vertices, vec![vertex_at(&n, 50.0, 0.0)], "the new point is selected");
}

#[test]
fn adding_a_midpoint_keeps_new_midpoints_coming_on_each_half() {
    let mut s = Session::new();
    let id = rect(&mut s);
    click(&mut s, 50.0, 0.0);
    assert_eq!(net(&s, id).vertices.len(), 5);
    // The left half has its own midpoint, shown and added the same way.
    s.pointer(&ev(PointerKind::Hover, 25.0, 0.0)).unwrap();
    let add = s.tool.overlay.vector_add.expect("the new half's midpoint");
    assert!(add.midpoint && add.on && (add.at[0] - 25.0).abs() < 1e-9, "{add:?}");
    click(&mut s, 25.0, 0.0);
    let n = net(&s, id);
    assert_eq!(n.vertices.len(), 6, "{n:?}");
    assert_eq!(ve(&s).vertices, vec![vertex_at(&n, 25.0, 0.0)]);
    // And on it goes: a quarter of the way along.
    s.pointer(&ev(PointerKind::Hover, 12.5, 0.0)).unwrap();
    let add = s.tool.overlay.vector_add.expect("another midpoint");
    assert!((add.at[0] - 12.5).abs() < 1e-9, "{add:?}");
}

#[test]
fn a_press_away_from_the_midpoint_selects_the_segment() {
    let mut s = Session::new();
    let id = rect(&mut s);
    click(&mut s, 30.0, 0.0);
    assert_eq!(net(&s, id).vertices.len(), 4, "no point is added off the midpoint");
    assert_eq!(ve(&s).segments.len(), 1, "the segment is selected instead");
}

#[test]
fn the_pen_shows_its_ghost_anchor_on_the_segment_under_it() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    s.pointer(&ev(PointerKind::Hover, 80.0, 0.0)).unwrap();
    let add = s.tool.overlay.vector_add.expect("the ghost anchor");
    assert!(!add.midpoint && add.on, "{add:?}");
    assert!((add.at[0] - 80.0).abs() < 1e-3 && add.at[1].abs() < 1e-3, "{add:?}");
    click(&mut s, 80.0, 0.0);
    assert_eq!(net(&s, id).vertices.len(), 5, "the press added the point");
    assert_eq!(ve(&s).anchor, None, "the click ended the pen's path");
}

/// A rectangle's corner cut off in the editor (a point added on each edge next to it, the corner
/// deleted with Shift+Delete so the two join) and the shape rounded: the fill, which the fill and
/// the selection outline draw, traces the stroke's outline, cut at both ends of the diagonal (#1951).
#[test]
fn a_rounded_chamfer_cut_in_the_editor_fills_along_its_arcs() {
    for top_first in [true, false] {
        let mut s = Session::new();
        let id = rect(&mut s);
        // Splitting keeps the split edge's index, so the other edge's stays: top 0, right 1.
        let (first, second) = if top_first { (0, 1) } else { (1, 0) };
        ex(&mut s, "vector.edit.addVertex", json!({"segment": first, "t": 0.5}));
        ex(&mut s, "vector.edit.addVertex", json!({"segment": second, "t": 0.5}));
        let corner = vertex_at(&net(&s, id), 100.0, 0.0);
        ex(&mut s, "vector.edit.select", json!({"vertices": [corner]}));
        ex(&mut s, "vector.deleteHeal", json!({}));
        let n = net(&s, id);
        assert_eq!((n.vertices.len(), n.segments.len(), n.regions.len()), (5, 5, 1), "top first {top_first}: a pentagon");
        ex(&mut s, "vector.edit.cornerRadius", json!({"radius": 43, "vertices": [0, 1, 2, 3, 4]}));
        fill_traces_stroke(&s, id, &format!("top first {top_first}"));
        // A region with a fill of its own is drawn through the region path and the planar normaliser.
        if let NodeKind::Vector { network } = &mut s.doc_mut().unwrap().get_mut(id).unwrap().kind {
            network.regions[0].fills = Some(vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)]);
        }
        fill_traces_stroke(&s, id, &format!("top first {top_first}, region fill"));
    }
}

/// The chamfer made other ways: the corner deleted plainly, which opens the path between the two
/// new points, and the two connected again; or the pentagon drawn with the pen (#1951).
#[test]
fn a_rounded_chamfer_made_other_ways_fills_along_its_arcs() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.addVertex", json!({"segment": 0, "t": 0.5}));
    ex(&mut s, "vector.edit.addVertex", json!({"segment": 1, "t": 0.5}));
    let corner = vertex_at(&net(&s, id), 100.0, 0.0);
    ex(&mut s, "vector.edit.select", json!({"vertices": [corner]}));
    ex(&mut s, "vector.edit.delete", json!({}));
    let n = net(&s, id);
    assert_eq!((n.segments.len(), n.regions[0].loops[0].len()), (4, 4), "the path opened: {:?}", n.regions);
    let (top, right) = (vertex_at(&n, 50.0, 0.0), vertex_at(&n, 100.0, 50.0));
    ex(&mut s, "vector.edit.connect", json!({"a": top, "b": right}));
    assert_eq!(net(&s, id).segments.len(), 5, "connected again");
    ex(&mut s, "vector.edit.cornerRadius", json!({"radius": 43, "vertices": [0, 1, 2, 3, 4]}));
    fill_traces_stroke(&s, id, "deleted and connected");
    // Drawn with the pen: continue from the end of an open two-point path and close on its start.
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[0, 0], [50, 0]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    click(&mut s, 50.0, 0.0);
    for (x, y) in [(100.0, 50.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)] {
        click(&mut s, x, y);
    }
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len(), n.regions.len()), (5, 5, 1), "closed by the pen");
    ex(&mut s, "vector.edit.cornerRadius", json!({"radius": 43, "vertices": [0, 1, 2, 3, 4]}));
    fill_traces_stroke(&s, id, "drawn with the pen");
}

/// The rounded chamfered rectangle `id` (five points on the origin's 100×100 box, the top-right
/// corner cut off between (50, 0) and (100, 50)): one region with every segment in its loop, and a
/// fill that keeps off every sharp point and traces the same outline as the stroke.
fn fill_traces_stroke(s: &Session, id: NodeId, why: &str) {
    use figcraft_geom::BezPath;
    use figcraft_geom::kurbo::{ParamCurveArclen, ParamCurveNearest};
    let n = net(s, id);
    assert_eq!(n.regions.len(), 1, "{why}: one region, {:?}", n.regions);
    assert_eq!(n.regions[0].loops[0].len(), n.segments.len(), "{why}: every segment in the loop, {:?}", n.regions[0].loops);
    let doc = s.doc().unwrap();
    let (fill, _) = figcraft_render::node_outline(doc, doc.node(id).unwrap()).unwrap();
    let stroke = n.stroke_path();
    let keeps_off = |path: &BezPath, p: Point| path.segments().map(|s| s.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
    let length = |path: &BezPath| path.segments().map(|s| s.arclen(1e-9)).sum::<f64>();
    for p in [pt(50.0, 0.0), pt(100.0, 50.0), pt(0.0, 0.0), pt(100.0, 100.0), pt(0.0, 100.0)] {
        assert!(keeps_off(&stroke, p) > 2.0, "{why}: the stroke is cut at {p:?} ({:.2} from it)", keeps_off(&stroke, p));
        assert!(keeps_off(&fill, p) > 2.0, "{why}: the fill runs into {p:?} ({:.2} from it)", keeps_off(&fill, p));
    }
    assert!((length(&fill) - length(&stroke)).abs() < 1e-3, "{why}: fill {} and stroke {} trace the same outline", length(&fill), length(&stroke));
}
