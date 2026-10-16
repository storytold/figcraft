//! Regions in vector edit mode (`regions.rs`): hover, the hit priority and clearances, selecting
//! and dragging regions, and the hatch's geometry (spacing by zoom, anchoring, clipping).

use figcraft_doc::{NodeId, Region, Segment, VectorNetwork, Vertex, WindingRule};
use figcraft_geom::{Affine, Point, Rect, Vec2};
use serde_json::json;

use super::regions::*;
use super::tests::{ex, net, ve};
use crate::Session;
use crate::tools::{PointerEvent, PointerKind};

/// Four points and four straight segments of a square, clockwise on screen, appended to `n`.
fn square(n: &mut VectorNetwork, x0: f64, y0: f64, size: f64) {
    let v0 = n.vertices.len();
    for (x, y) in [(x0, y0), (x0 + size, y0), (x0 + size, y0 + size), (x0, y0 + size)] {
        n.vertices.push(Vertex::new(x, y));
    }
    for k in 0..4 {
        n.segments.push(Segment::line(v0 + k, v0 + (k + 1) % 4));
    }
}

/// A 200 square (points 0..3, segments 0..3) with an 80 square inside (points 4..7, segments 4..7).
fn nested(regions: Vec<Region>) -> VectorNetwork {
    let mut n = VectorNetwork::default();
    square(&mut n, 0.0, 0.0, 200.0);
    square(&mut n, 60.0, 60.0, 80.0);
    n.regions = regions;
    n
}

fn region(loops: Vec<Vec<usize>>, winding: WindingRule) -> Region {
    Region { loops, winding }
}

/// The 7.1 region: both squares as one region.
fn holed(winding: WindingRule) -> VectorNetwork {
    nested(vec![region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], winding)])
}

/// The 7.2 regions: the outer square, then the inner one.
fn overlapping() -> VectorNetwork {
    nested(vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero), region(vec![vec![4, 5, 6, 7]], WindingRule::NonZero)])
}

/// A filled vector at the page origin with network `n`, in edit mode.
fn editing_vector(s: &mut Session, n: VectorNetwork) -> NodeId {
    let r = ex(s, "vector.create", json!({"points": [[0, 0], [10, 0], [10, 10]], "closed": true}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    let doc = s.doc_mut().unwrap();
    let node = doc.get_mut(id).unwrap();
    node.kind = figcraft_doc::NodeKind::Vector { network: n };
    node.transform = Affine::IDENTITY;
    node.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE)];
    super::refit(doc, id).unwrap();
    ex(s, "vector.edit.begin", json!({"id": id}));
    id
}

fn ev(kind: PointerKind, x: f64, y: f64, zoom: f64, shift: bool) -> PointerEvent {
    PointerEvent { shift, ..PointerEvent::at(kind, x, y, zoom) }
}

fn hover(s: &mut Session, x: f64, y: f64, zoom: f64) -> Option<usize> {
    s.pointer(&ev(PointerKind::Hover, x, y, zoom, false)).unwrap();
    ve(s).region_hover
}

fn press(s: &mut Session, x: f64, y: f64, shift: bool) {
    s.pointer(&ev(PointerKind::Down, x, y, 1.0, shift)).unwrap();
}

fn release(s: &mut Session, x: f64, y: f64, shift: bool) {
    s.pointer(&ev(PointerKind::Up, x, y, 1.0, shift)).unwrap();
}

fn click(s: &mut Session, x: f64, y: f64, shift: bool) {
    press(s, x, y, shift);
    release(s, x, y, shift);
}

fn sorted(mut v: Vec<usize>) -> Vec<usize> {
    v.sort_unstable();
    v.dedup();
    v
}

/// Point `v` of vector `id` on the page.
fn page_point(s: &Session, id: NodeId, v: usize) -> Point {
    s.doc().unwrap().world_transform(id) * net(s, id).vertices[v].point()
}

fn page_points(s: &Session, id: NodeId) -> Vec<Point> {
    (0..net(s, id).vertices.len()).map(|v| page_point(s, id, v)).collect()
}

#[test]
fn an_empty_network_has_nothing_to_hover_and_presses_go_to_empty_space() {
    // 19.
    let mut s = Session::new();
    let id = editing_vector(&mut s, nested(vec![]));
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), None);
    click(&mut s, 100.0, 100.0, false);
    assert!(ve(&s).regions.is_empty());
    assert_eq!(ve(&s).id, id, "still editing (the press was inside the layer)");
    assert!(region_hit(s.doc().unwrap(), &ve(&s), Point::new(100.0, 100.0), 1.0).is_none());
}

#[test]
fn only_filled_areas_are_regions() {
    let mut s = Session::new();
    let id = editing_vector(&mut s, holed(WindingRule::NonZero));
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), Some(0));
    s.doc_mut().unwrap().get_mut(id).unwrap().fills.clear();
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), None, "no fill: nothing to hatch");
    click(&mut s, 100.0, 100.0, false);
    assert!(ve(&s).regions.is_empty());
}

#[test]
fn hover_and_press_select_a_region_with_all_its_points() {
    // 20.
    let mut s = Session::new();
    let id = editing_vector(&mut s, holed(WindingRule::NonZero));
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), Some(0));
    assert_eq!(s.tool.overlay.vector_hover, None);
    press(&mut s, 100.0, 100.0, false);
    let e = ve(&s);
    assert_eq!(e.regions, vec![0]);
    assert_eq!(sorted(e.vertices.clone()), (0..8).collect::<Vec<_>>());
    assert!(s.tool.busy(), "a points drag started");
    assert_eq!(e.region_hover, None, "no hover hatch while the button is down");
    release(&mut s, 100.0, 100.0, false);
    // A click without a drag leaves the region selected and stays in edit mode; nothing moved.
    assert_eq!(ve(&s).regions, vec![0]);
    assert_eq!(ve(&s).id, id);
    assert_eq!(page_point(&s, id, 0), Point::new(0.0, 0.0));
    // The region's points are drawn unselected (no handles either), but they are selected.
    let o = super::overlay(s.doc().unwrap(), &ve(&s)).unwrap();
    assert!(o.selected_vertices.is_empty(), "{:?}", o.selected_vertices);
    // An arrow key moves them all.
    assert!(super::key(&mut s, "ArrowRight", false, false, false).unwrap());
    let page = page_points(&s, id);
    assert_eq!(page[0], Point::new(1.0, 0.0));
    assert_eq!(page[6], Point::new(141.0, 140.0));
    assert_eq!(ve(&s).regions, vec![0], "moving points keeps the region selected");
}

#[test]
fn clearances_around_points_and_segments_are_in_screen_pixels() {
    // 21-23, at 100% and 400%: the hatch goes 3.5 px from a point and 2 px from a segment.
    for zoom in [1.0, 4.0] {
        let mut s = Session::new();
        editing_vector(&mut s, holed(WindingRule::NonZero));
        let px = 1.0 / zoom;
        // Up and left of point 4 at (60, 60): inside the region.
        let near = |k: f64| (60.0 - k * px / std::f64::consts::SQRT_2, 60.0 - k * px / std::f64::consts::SQRT_2);
        for (k, want) in [(3.0, None), (3.4, None), (3.6, Some(0)), (5.0, Some(0)), (20.0, Some(0))] {
            let (x, y) = near(k);
            assert_eq!(hover(&mut s, x, y, zoom), want, "{k} px from a point at {zoom}x");
        }
        // Below the outer square's top edge (y = 0), away from its points.
        for (k, want) in [(1.5, None), (1.9, None), (2.1, Some(0)), (2.6, Some(0)), (10.0, Some(0))] {
            assert_eq!(hover(&mut s, 100.0, k * px, zoom), want, "{k} px from a segment at {zoom}x");
        }
        assert_eq!(hover(&mut s, 100.0, -10.0 * px, zoom), None, "outside");
    }
}

#[test]
fn between_the_clearance_and_the_hit_radius_the_region_wins() {
    let mut s = Session::new();
    editing_vector(&mut s, holed(WindingRule::NonZero));
    // 5 px from point 4: inside its 6 px hit radius, outside the 3.5 px clearance.
    let (x, y) = (60.0 - 5.0 / std::f64::consts::SQRT_2, 60.0 - 5.0 / std::f64::consts::SQRT_2);
    assert_eq!(hover(&mut s, x, y, 1.0), Some(0));
    assert_eq!(s.tool.overlay.vector_hover, None, "the point isn't highlighted");
    click(&mut s, x, y, false);
    assert_eq!(ve(&s).regions, vec![0]);
    // 2 px from it: the point.
    click(&mut s, 58.0, 60.0, false);
    assert!(ve(&s).regions.is_empty());
    assert_eq!(ve(&s).vertices, vec![4]);
}

#[test]
fn a_handle_inside_a_region_wins() {
    // 24.
    let mut s = Session::new();
    let mut n = holed(WindingRule::NonZero);
    n.segments[0].ta = (50.0, 50.0);
    let id = editing_vector(&mut s, n);
    ex(&mut s, "vector.edit.select", json!({"vertices": [0]}));
    let w = s.doc().unwrap().world_transform(id);
    let local = net(&s, id).vertices[0].point() + Vec2::new(50.0, 50.0);
    let h = w * local;
    assert!(net(&s, id).region_at(local).is_some(), "the handle lies inside the region");
    assert_eq!(hover(&mut s, h.x, h.y, 1.0), None);
    click(&mut s, h.x, h.y, false);
    assert_eq!(ve(&s).handles, vec![(0, true)]);
    assert!(ve(&s).regions.is_empty());
}

#[test]
fn selecting_regions_follows_shift_and_presses_elsewhere() {
    let mut s = Session::new();
    let id = editing_vector(&mut s, overlapping());
    // 25. Region, then empty space (inside the layer's bounds: edit mode stays).
    click(&mut s, 20.0, 100.0, false);
    assert_eq!(ve(&s).regions, vec![0]);
    click(&mut s, 210.0, 100.0, false);
    assert!(ve(&s).regions.is_empty());
    assert!(ve(&s).vertices.is_empty(), "points selected through the region go with it");
    assert_eq!(ve(&s).id, id, "still in edit mode");
    // 26. Region, then a point.
    click(&mut s, 20.0, 100.0, false);
    click(&mut s, 200.0, 200.0, false);
    assert!(ve(&s).regions.is_empty());
    assert_eq!(ve(&s).vertices, vec![2]);
    // 27. Shift adds the inner region (the later one wins in the middle).
    click(&mut s, 20.0, 100.0, false);
    click(&mut s, 100.0, 100.0, true);
    assert_eq!(sorted(ve(&s).regions.clone()), vec![0, 1]);
    assert_eq!(sorted(ve(&s).vertices.clone()), (0..8).collect::<Vec<_>>());
    // 29. Shift on a selected region takes it out with its own points; no drag starts.
    press(&mut s, 100.0, 100.0, true);
    assert!(!s.tool.busy());
    release(&mut s, 100.0, 100.0, true);
    assert_eq!(ve(&s).regions, vec![0]);
    assert_eq!(sorted(ve(&s).vertices.clone()), vec![0, 1, 2, 3]);
    // 28. A point selected on its own stays when a region is added, and when it is taken out.
    click(&mut s, 200.0, 200.0, false);
    click(&mut s, 100.0, 100.0, true);
    assert_eq!(ve(&s).regions, vec![1]);
    assert_eq!(sorted(ve(&s).vertices.clone()), vec![2, 4, 5, 6, 7]);
    let o = super::overlay(s.doc().unwrap(), &ve(&s)).unwrap();
    assert_eq!(o.selected_vertices, vec![2], "only the point selected on its own looks selected");
    click(&mut s, 100.0, 100.0, true);
    assert!(ve(&s).regions.is_empty());
    assert_eq!(ve(&s).vertices, vec![2]);
    // Points shared by two selected regions stay while one of them does.
    let mut s = Session::new();
    let ring = region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], WindingRule::EvenOdd);
    editing_vector(&mut s, nested(vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero), ring]));
    ex(&mut s, "vector.edit.select", json!({"regions": [0, 1]}));
    assert_eq!(sorted(ve(&s).regions.clone()), vec![0, 1]);
    click(&mut s, 20.0, 100.0, true); // the ring (region 1) is on top there
    assert_eq!(ve(&s).regions, vec![0]);
    assert_eq!(sorted(ve(&s).vertices.clone()), vec![0, 1, 2, 3]);
}

#[test]
fn a_click_in_a_hole_clears_the_regions_and_stays_in_edit_mode() {
    let mut s = Session::new();
    let id = editing_vector(&mut s, holed(WindingRule::EvenOdd));
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), None, "a hole is not the region");
    click(&mut s, 20.0, 100.0, false);
    assert_eq!(ve(&s).regions, vec![0]);
    click(&mut s, 100.0, 100.0, false);
    assert!(ve(&s).regions.is_empty());
    assert!(ve(&s).vertices.is_empty());
    assert_eq!(ve(&s).id, id);
}

#[test]
fn a_press_on_a_selected_region_keeps_and_drags_the_whole_selection() {
    // 30.
    let mut s = Session::new();
    editing_vector(&mut s, overlapping());
    click(&mut s, 20.0, 100.0, false);
    click(&mut s, 100.0, 100.0, true);
    press(&mut s, 100.0, 100.0, false);
    assert_eq!(sorted(ve(&s).regions.clone()), vec![0, 1]);
    assert!(s.tool.busy());
    release(&mut s, 100.0, 100.0, false);
    assert_eq!(sorted(ve(&s).regions.clone()), vec![0, 1]);
    // 32. A region plus a separate point: a plain drag inside the region moves both.
    let mut s = Session::new();
    let mut n = overlapping();
    n.vertices.push(Vertex::new(300.0, 300.0));
    n.vertices.push(Vertex::new(320.0, 300.0));
    n.segments.push(Segment::line(8, 9));
    let id = editing_vector(&mut s, n);
    ex(&mut s, "vector.edit.select", json!({"vertices": [9]}));
    click(&mut s, 100.0, 100.0, true);
    assert_eq!(ve(&s).regions, vec![1]);
    let before = page_points(&s, id);
    press(&mut s, 100.0, 100.0, false);
    s.pointer(&ev(PointerKind::Move, 107.0, 112.0, 1.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 115.0, 125.0, 1.0, false)).unwrap();
    release(&mut s, 115.0, 125.0, false);
    let after = page_points(&s, id);
    for v in [4, 5, 6, 7, 9] {
        assert_eq!(after[v] - before[v], Vec2::new(15.0, 25.0), "point {v}");
    }
    for v in [0, 1, 2, 3, 8] {
        assert_eq!(after[v], before[v], "point {v} isn't selected");
    }
    assert_eq!(ve(&s).regions, vec![1]);
    assert_eq!(sorted(ve(&s).vertices.clone()), vec![4, 5, 6, 7, 9]);
}

#[test]
fn dragging_a_region_moves_its_points_in_one_undo_step_and_cancels() {
    let mut s = Session::new();
    let id = editing_vector(&mut s, overlapping());
    let before = page_points(&s, id);
    // 31.
    press(&mut s, 20.0, 100.0, false);
    s.pointer(&ev(PointerKind::Move, 27.0, 112.0, 1.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 35.0, 125.0, 1.0, false)).unwrap();
    // 35. No hover hatch during the drag; the selected region stays hatched and follows live.
    assert_eq!(hatched_regions(&ve(&s), s.tool.busy()), vec![0]);
    let mut hovered = ve(&s);
    hovered.region_hover = Some(1);
    assert_eq!(hatched_regions(&hovered, true), vec![0], "a hovered region isn't hatched while dragging");
    assert_eq!(hatched_regions(&hovered, false), vec![0, 1]);
    let live = s.vector_region_hatch(Affine::IDENTITY, None);
    assert!(live.iter().all(|[a, b]| a.x >= 15.0 - 1e-6 && b.x >= 15.0 - 1e-6 && a.y >= 25.0 - 1e-6 && b.y >= 25.0 - 1e-6));
    release(&mut s, 35.0, 125.0, false);
    let after = page_points(&s, id);
    for v in 0..4 {
        assert_eq!(after[v] - before[v], Vec2::new(15.0, 25.0));
    }
    for v in 4..8 {
        assert_eq!(after[v], before[v]);
    }
    assert_eq!(ve(&s).regions, vec![0]);
    // 34. One undo step.
    s.undo().unwrap();
    assert_eq!(page_points(&s, id), before);
    // 33. Escape mid-drag puts the points back and keeps the selection.
    press(&mut s, 20.0, 100.0, false);
    s.pointer(&ev(PointerKind::Move, 40.0, 100.0, 1.0, false)).unwrap();
    assert_ne!(page_points(&s, id), before);
    s.escape().unwrap();
    assert_eq!(page_points(&s, id), before);
    assert_eq!(ve(&s).regions, vec![0]);
}

#[test]
fn a_topology_change_clears_the_regions() {
    let mut s = Session::new();
    editing_vector(&mut s, overlapping());
    click(&mut s, 20.0, 100.0, false);
    hover(&mut s, 100.0, 100.0, 1.0);
    assert_eq!(ve(&s).regions, vec![0]);
    // Moving points keeps them.
    ex(&mut s, "vector.edit.nudge", json!({"dx": 3, "dy": 0}));
    assert_eq!(ve(&s).regions, vec![0]);
    assert_eq!(ve(&s).region_hover, Some(1));
    // Connecting two points adds a segment: indexes may no longer hold.
    ex(&mut s, "vector.edit.connect", json!({"a": 0, "b": 4}));
    assert!(ve(&s).regions.is_empty());
    assert_eq!(ve(&s).region_hover, None);
    // The pointer leaving the canvas drops the hover.
    let mut s = Session::new();
    editing_vector(&mut s, overlapping());
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), Some(1));
    s.vector_pointer_left();
    assert_eq!(ve(&s).region_hover, None);
}

/// The expected hatch gaps (screen px) by zoom.
const GAPS: [(f64, f64); 7] = [(0.5, 7.9), (1.0, 7.9), (1.4, 11.0), (1.45, 5.7), (1.5, 5.9), (2.0, 7.9), (4.0, 7.9)];

/// Perpendicular offsets of hatch segments from `o`, distinct and sorted.
fn offsets(lines: &[[Point; 2]], o: Point) -> Vec<f64> {
    let n = Vec2::new(-HATCH_DIRECTION.y, HATCH_DIRECTION.x);
    let mut v: Vec<f64> = lines.iter().map(|[a, b]| (a.midpoint(*b) - o).dot(n)).collect();
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    v
}

fn view(zoom: f64, tx: f64, ty: f64) -> Affine {
    Affine::new([zoom, 0.0, 0.0, zoom, tx, ty])
}

fn on_pattern(x: f64, spacing: f64) -> bool {
    ((x / spacing) - (x / spacing).round()).abs() < 1e-6
}

#[test]
fn the_hatch_spacing_follows_octave_folding() {
    for (zoom, gap) in GAPS {
        let s = hatch_spacing(zoom).unwrap();
        assert!((s - gap).abs() <= 0.3, "{zoom}: {s} vs {gap}");
    }
    // Every power of two gives the base gap; the step is at √2 · 2ⁿ.
    for k in -6..=8 {
        assert!((hatch_spacing(2f64.powi(k)).unwrap() - HATCH_GAP).abs() < 1e-9);
    }
    assert!(hatch_spacing(1.41).unwrap() > 11.0 && hatch_spacing(1.42).unwrap() < 5.7);
    // 36, by that rule: the drawn lines are that far apart at each zoom.
    let mut s = Session::new();
    let id = editing_vector(&mut s, holed(WindingRule::NonZero));
    ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
    for zoom in [0.25, 0.5, 1.0, 1.4, 1.45, 3.0, 16.0] {
        let v = view(zoom, 37.0, -12.0);
        let o = v * s.doc().unwrap().world_transform(id) * Point::ZERO;
        let off = offsets(&s.vector_region_hatch(v, None), o);
        assert!(off.len() > 4, "{zoom}");
        let want = hatch_spacing(zoom).unwrap();
        for pair in off.windows(2) {
            assert!(((pair[1] - pair[0]) - want).abs() < 1e-6, "{zoom}: {pair:?}, want {want}");
        }
        assert!(off.iter().all(|x| on_pattern(*x, want)), "lines sit on whole multiples of the spacing from the anchor");
    }
}

#[test]
fn the_hatch_runs_bottom_left_to_top_right() {
    let mut s = Session::new();
    editing_vector(&mut s, holed(WindingRule::NonZero));
    ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
    let lines = s.vector_region_hatch(Affine::IDENTITY, None);
    assert!(!lines.is_empty());
    for [a, b] in lines {
        let d = b - a;
        assert!(d.x * d.y < 0.0, "a '/' line on screen (y down): {d:?}");
        assert!((d.x.abs() - d.y.abs()).abs() < 1e-6, "45°");
    }
}

#[test]
fn the_hatch_stays_on_the_shape_when_panning_and_moving_it() {
    let mut s = Session::new();
    let id = editing_vector(&mut s, holed(WindingRule::NonZero));
    ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
    let zoom = 1.5;
    let o = |s: &Session, v: Affine| v * s.doc().unwrap().world_transform(id) * Point::ZERO;
    // 37. Pan by (dx, dy): the same offsets from the anchor, every line moved by (dx, dy).
    let (v0, v1) = (view(zoom, 10.0, 20.0), view(zoom, 10.0 + 33.3, 20.0 - 7.7));
    let (a, b) = (s.vector_region_hatch(v0, None), s.vector_region_hatch(v1, None));
    assert_eq!(offsets(&a, o(&s, v0)).len(), offsets(&b, o(&s, v1)).len());
    for (x, y) in offsets(&a, o(&s, v0)).iter().zip(offsets(&b, o(&s, v1))) {
        assert!((x - y).abs() < 1e-6);
    }
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert!(((y[0] - x[0]) - Vec2::new(33.3, -7.7)).hypot() < 1e-6);
    }
    // 38. Moving the node moves the pattern by the offset times the zoom.
    {
        let node = s.doc_mut().unwrap().get_mut(id).unwrap();
        node.transform = Affine::translate((12.0, -4.0)) * node.transform;
    }
    let c = s.vector_region_hatch(v0, None);
    assert_eq!(a.len(), c.len());
    for (x, y) in a.iter().zip(&c) {
        assert!(((y[0] - x[0]) - Vec2::new(12.0 * zoom, -4.0 * zoom)).hypot() < 1e-6);
    }
}

#[test]
fn hatch_lines_cover_any_box() {
    // 39.
    let n = Vec2::new(-HATCH_DIRECTION.y, HATCH_DIRECTION.x);
    for (b, o, sp) in [
        (Rect::new(0.0, 0.0, 100.0, 50.0), Point::new(0.0, 0.0), 8.0),
        (Rect::new(-317.0, 41.0, -300.0, 900.0), Point::new(13.5, -2.25), 5.7),
        (Rect::new(1e5, 1e5, 1e5 + 3.0, 1e5 + 2.0), Point::new(-40.0, 7.0), 11.0),
    ] {
        let lines = hatch_lines(b, o, HATCH_DIRECTION, sp);
        let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
        let proj: Vec<f64> = corners.iter().map(|c| (*c - o).dot(n)).collect();
        let lo = proj.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = proj.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let (first, last) = (lines.first().unwrap(), lines.last().unwrap());
        assert!(first.k as f64 * sp <= lo && last.k as f64 * sp >= hi);
        for l in &lines {
            assert!(((l.a - o).dot(n) - l.k as f64 * sp).abs() < 1e-6, "on its line");
            assert!((l.b - l.a).hypot() >= b.width().hypot(b.height()), "across the whole box");
        }
    }
    assert!(hatch_lines(Rect::new(0.0, 0.0, 10.0, 10.0), Point::ZERO, HATCH_DIRECTION, 0.0).is_empty());
    assert!(hatch_lines(Rect::new(0.0, 0.0, f64::NAN, 10.0), Point::ZERO, HATCH_DIRECTION, 8.0).is_empty());
}

#[test]
fn the_hatch_is_clipped_by_the_regions_fill_rule() {
    // 40.
    let samples =
        |lines: &[[Point; 2]]| -> Vec<Point> { lines.iter().flat_map(|[a, b]| (0..=10).map(|k| a.lerp(*b, f64::from(k) / 10.0))).collect() };
    let inner = Rect::new(60.5, 60.5, 139.5, 139.5);
    let outer = Rect::new(-0.5, -0.5, 200.5, 200.5);
    for (rule, hole) in [(WindingRule::EvenOdd, true), (WindingRule::NonZero, false)] {
        let mut s = Session::new();
        editing_vector(&mut s, holed(rule));
        ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
        let pts = samples(&s.vector_region_hatch(Affine::IDENTITY, None));
        assert!(!pts.is_empty());
        assert_eq!(pts.iter().any(|p| inner.contains(*p)), !hole, "{rule:?}");
        assert!(pts.iter().all(|p| outer.contains(*p)), "{rule:?}: nothing outside the region");
    }
}

#[test]
fn hovered_and_selected_is_hatched_once_and_regions_share_one_pattern() {
    // 41.
    let mut s = Session::new();
    let id = editing_vector(&mut s, overlapping());
    ex(&mut s, "vector.edit.select", json!({"regions": [1]}));
    let one = s.vector_region_hatch(Affine::IDENTITY, None).len();
    assert_eq!(hover(&mut s, 100.0, 100.0, 1.0), Some(1));
    assert_eq!(s.vector_region_hatch(Affine::IDENTITY, None).len(), one);
    // Several selected regions: one continuous pattern (the same phase).
    ex(&mut s, "vector.edit.select", json!({"regions": [0, 1]}));
    let v = view(1.45, 3.3, 9.1);
    let o = v * s.doc().unwrap().world_transform(id) * Point::ZERO;
    let sp = hatch_spacing(1.45).unwrap();
    assert!(offsets(&s.vector_region_hatch(v, None), o).iter().all(|x| on_pattern(*x, sp)));
    // 42. Nothing selected or hovered: nothing drawn.
    ex(&mut s, "vector.edit.select", json!({}));
    s.vector_pointer_left();
    assert!(s.vector_region_hatch(Affine::IDENTITY, None).is_empty());
    // 43. A zero or not-a-number zoom: nothing drawn, no panic.
    ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
    for z in [0.0, f64::NAN, f64::INFINITY] {
        assert!(s.vector_region_hatch(view(z, 0.0, 0.0), None).is_empty());
        assert!(hatch_spacing(z).is_none());
        assert!(region_hit(s.doc().unwrap(), &ve(&s), Point::new(100.0, 100.0), z).is_none());
    }
    // Clipped to what shows: only lines crossing it, on the same pattern.
    let all = offsets(&s.vector_region_hatch(Affine::IDENTITY, None), Point::ZERO);
    let part = offsets(&s.vector_region_hatch(Affine::IDENTITY, Some(Rect::new(0.0, 0.0, 50.0, 50.0))), Point::ZERO);
    assert!(!part.is_empty() && part.len() < all.len());
    assert!(part.iter().all(|x| all.iter().any(|y| (x - y).abs() < 1e-6)));
}
