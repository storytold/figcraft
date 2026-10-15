//! Connectors: attaching, routing, following moved layers, letting go of deleted ones, copies.

use figcraft_doc::{ConnectorLineType, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 100, "height": 60})).unwrap())
}

/// The connector's route in page coordinates.
fn route(s: &Session, c: NodeId) -> Vec<(f64, f64)> {
    let doc = s.doc().unwrap();
    let n = doc.node(c).unwrap();
    let xf = doc.world_transform(c);
    let NodeKind::Connector { route, .. } = &n.kind else { panic!("not a connector") };
    route.iter().map(|p| xf * figcraft_geom::Point::new(p[0], p[1])).map(|p| (p.x, p.y)).collect()
}

fn connect(s: &mut Session, a: NodeId, b: NodeId, line: &str) -> NodeId {
    id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"node": b}, "lineType": line})).unwrap())
}

#[test]
fn attached_ends_meet_the_facing_sides() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "straight");
    assert_eq!(route(&s, c), vec![(100.0, 30.0), (300.0, 30.0)], "right side of a to left side of b");
    let n = s.doc().unwrap().node(c).unwrap().clone();
    assert_eq!((n.width, n.height), (200.0, 0.0), "the box is the route's bounds");
    assert_eq!(n.stroke.cap_end, figcraft_doc::StrokeCap::LineArrow, "connectors end in an open arrow");
    // Below: bottom to top.
    let d = rect(&mut s, 0.0, 300.0);
    let c2 = connect(&mut s, a, d, "straight");
    assert_eq!(route(&s, c2), vec![(50.0, 60.0), (50.0, 300.0)]);
}

#[test]
fn elbowed_routes_bend_at_right_angles_and_straighten_when_aligned() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 100.0);
    let c = connect(&mut s, a, b, "elbowed");
    let r = route(&s, c);
    assert_eq!(r, vec![(100.0, 30.0), (200.0, 30.0), (200.0, 130.0), (300.0, 130.0)]);
    for w in r.windows(2) {
        assert!(w[0].0 == w[1].0 || w[0].1 == w[1].1, "every segment is horizontal or vertical: {r:?}");
    }
    // Line the second box up with the first: the elbow becomes one straight segment.
    s.execute("node.set", &json!({"id": b, "props": {"y": 0}})).unwrap();
    assert_eq!(route(&s, c), vec![(100.0, 30.0), (300.0, 30.0)]);
}

#[test]
fn fixed_magnets_and_curves() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = id(s
        .execute(
            "connector.create",
            &json!({"start": {"node": a, "magnet": "bottom"}, "end": {"node": b, "magnet": "bottom"}, "lineType": "elbowed"}),
        )
        .unwrap());
    // Both ends leave downwards: the route dips below both boxes.
    let r = route(&s, c);
    assert_eq!(r.first(), Some(&(50.0, 60.0)));
    assert_eq!(r.last(), Some(&(350.0, 60.0)));
    assert!(r.iter().any(|p| p.1 > 60.0), "{r:?}");
    s.execute("node.set", &json!({"id": c, "props": {"connectorLineType": "curved"}})).unwrap();
    let r = route(&s, c);
    assert_eq!(r.len(), 4, "a curve is its ends and two control points");
    assert!(r[1].1 > 60.0 && r[2].1 > 60.0, "the curve leaves and enters downwards: {r:?}");
    let g = s.execute("node.get", &json!({"id": c})).unwrap();
    assert_eq!(g["connectorLineType"], "curved");
    assert_eq!(g["connectorStart"]["magnet"], "bottom");
    assert_eq!(g["connectorEnd"]["node"], json!(b.0));
}

#[test]
fn connectors_follow_moved_layers_and_their_parents() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let a = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 60, "parent": f})).unwrap());
    let b = rect(&mut s, 400.0, 10.0);
    let c = connect(&mut s, a, b, "straight");
    assert_eq!(route(&s, c), vec![(110.0, 40.0), (400.0, 40.0)]);
    s.execute("node.set", &json!({"id": b, "props": {"x": 500}})).unwrap();
    assert_eq!(route(&s, c).last(), Some(&(500.0, 40.0)));
    // Moving the frame moves the layer inside it: its end follows, the other stays on its layer.
    s.execute("node.set", &json!({"id": f, "props": {"y": 100}})).unwrap();
    assert_eq!(route(&s, c), vec![(110.0, 140.0), (500.0, 40.0)]);
    s.undo().unwrap();
    assert_eq!(route(&s, c), vec![(110.0, 40.0), (500.0, 40.0)], "undo puts the route back");
}

#[test]
fn deleting_an_attached_layer_leaves_the_end_free_where_it_was() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "straight");
    s.execute("edit.delete", &json!({"ids": [b]})).unwrap();
    let doc = s.doc().unwrap();
    let NodeKind::Connector { end, .. } = &doc.node(c).unwrap().kind else { panic!() };
    assert_eq!((end.node, end.x, end.y), (None, 300.0, 30.0));
    assert_eq!(route(&s, c), vec![(100.0, 30.0), (300.0, 30.0)]);
    // The free end stays put when the other layer moves.
    s.execute("node.set", &json!({"id": a, "props": {"y": 100}})).unwrap();
    assert_eq!(route(&s, c).last(), Some(&(300.0, 30.0)));
}

#[test]
fn free_connectors_move_as_a_whole_and_detach_keeps_the_line() {
    let mut s = Session::new();
    let c = id(s.execute("connector.create", &json!({"start": {"x": 0, "y": 0}, "end": {"x": 100, "y": 50}, "lineType": "straight"})).unwrap());
    s.execute("node.set", &json!({"id": c, "props": {"x": 20, "y": 30}})).unwrap();
    assert_eq!(route(&s, c), vec![(20.0, 30.0), (120.0, 80.0)], "both free ends move with the connector");
    let a = rect(&mut s, 300.0, 300.0);
    let b = rect(&mut s, 600.0, 300.0);
    let c2 = connect(&mut s, a, b, "straight");
    let before = route(&s, c2);
    s.execute("connector.detach", &json!({"ids": [c2]})).unwrap();
    s.execute("node.set", &json!({"id": b, "props": {"y": 500}})).unwrap();
    assert_eq!(route(&s, c2), before, "a detached connector no longer follows");
}

#[test]
fn copies_attach_to_copied_layers_only() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "elbowed");
    // All three duplicated: the copy joins the copies.
    let r = s.execute("edit.duplicate", &json!({"ids": [a, b, c], "dx": 0, "dy": 200})).unwrap();
    let doc = s.doc().unwrap();
    let copies: Vec<NodeId> = s.selection();
    let cc = *copies.iter().find(|i| matches!(doc.node(**i).unwrap().kind, NodeKind::Connector { .. })).unwrap_or_else(|| panic!("{r}"));
    let NodeKind::Connector { start, end, line_type, .. } = &doc.node(cc).unwrap().kind else { panic!() };
    assert!(copies.contains(&start.node.unwrap()) && copies.contains(&end.node.unwrap()));
    assert_eq!(*line_type, ConnectorLineType::Elbowed);
    assert_eq!(route(&s, cc), vec![(100.0, 230.0), (300.0, 230.0)]);
    // The connector duplicated alone keeps its line but lets go of the originals.
    s.execute("edit.duplicate", &json!({"ids": [c], "dx": 0, "dy": 400})).unwrap();
    let alone = s.selection()[0];
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(alone).unwrap().kind else { panic!() };
    assert_eq!((start.node, end.node), (None, None));
}

#[test]
fn bad_connector_params_are_refused() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let page = s.page().unwrap();
    for p in [
        json!({}),
        json!({"start": {"node": a}}),
        json!({"start": {"node": a}, "end": {"node": a}}),
        json!({"start": {"node": page}, "end": {"x": 1, "y": 1}}),
        json!({"start": {"node": 999_999}, "end": {"x": 1, "y": 1}}),
        json!({"start": {"x": 1e12, "y": 0}, "end": {"x": 1, "y": 1}}),
        json!({"start": {"node": a, "magnet": "sideways"}, "end": {"x": 1, "y": 1}}),
        json!({"start": {"node": a}, "end": {"x": 1, "y": 1}, "lineType": "zigzag"}),
    ] {
        assert!(s.execute("connector.create", &p).is_err(), "{p}");
    }
    let c = id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"x": 400, "y": 0}})).unwrap());
    assert!(s.execute("node.set", &json!({"id": a, "props": {"connectorLineType": "straight"}})).is_err(), "only connectors take it");
    assert!(s.execute("node.set", &json!({"id": c, "props": {"connectorEnd": {"node": a}}})).is_err(), "not both ends on one layer");
    let other = s.execute("node.create", &json!({"type": "connector", "start": {"node": c}, "end": {"x": 0, "y": 0}}));
    assert!(other.is_err(), "connectors don't attach to connectors");
}

#[test]
fn connectors_survive_save_and_load() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 200.0);
    let c = connect(&mut s, a, b, "curved");
    let doc = s.doc().unwrap().clone();
    let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!(back.node(c), doc.node(c));
}

#[test]
fn svg_export_draws_the_route_with_its_arrow() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "straight");
    let doc = s.doc().unwrap();
    let svg = figcraft_svg::export_svg(doc, &[c], &Default::default()).unwrap();
    assert!(svg.contains("<path"), "{svg}");
}

#[test]
fn pasted_connectors_join_the_pasted_layers() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "straight");
    s.execute("edit.copy", &json!({"ids": [a, b, c]})).unwrap();
    s.execute("edit.paste", &json!({"x": 0, "y": 500})).unwrap();
    let pasted = s.selection();
    let doc = s.doc().unwrap();
    let pc = *pasted.iter().find(|i| matches!(doc.node(**i).unwrap().kind, NodeKind::Connector { .. })).unwrap();
    let NodeKind::Connector { start, end, .. } = &doc.node(pc).unwrap().kind else { panic!() };
    assert!(pasted.contains(&start.node.unwrap()) && pasted.contains(&end.node.unwrap()));
    assert_eq!(route(&s, pc), vec![(100.0, 530.0), (300.0, 530.0)]);
}

#[test]
fn connectors_inside_components_join_each_instance_s_own_layers() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 500, "height": 100})).unwrap());
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 20, "width": 100, "height": 60, "parent": f})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 300, "y": 20, "width": 100, "height": 60, "parent": f})).unwrap());
    let c = id(s.execute("connector.create", &json!({"start": {"node": a}, "end": {"node": b}, "lineType": "straight", "parent": f})).unwrap());
    let comp = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(s.execute("instance.create", &json!({"component": comp, "x": 0, "y": 300})).unwrap());
    let doc = s.doc().unwrap();
    let inside = doc.subtree(inst);
    let copy = *inside.iter().find(|i| doc.node(**i).unwrap().source == Some(c)).unwrap();
    let NodeKind::Connector { start, end, .. } = &doc.node(copy).unwrap().kind else { panic!() };
    assert!(inside.contains(&start.node.unwrap()) && inside.contains(&end.node.unwrap()), "the copy joins the instance's layers");
    assert_eq!(route(&s, copy), vec![(100.0, 350.0), (300.0, 350.0)]);
}

#[test]
fn fig_export_writes_connectors_as_the_lines_they_draw() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 100.0);
    let c = connect(&mut s, a, b, "elbowed");
    let doc = s.doc().unwrap();
    let (bytes, warnings) = figcraft_format::fig::export(doc, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("connectors were written as vectors")), "{warnings:?}");
    let (back, _) = figcraft_format::fig::import(&bytes, "Back").unwrap();
    let name = &doc.node(c).unwrap().name;
    let v = back.nodes().find(|n| &n.name == name).unwrap();
    assert!(matches!(v.kind, NodeKind::Vector { .. }), "{:?}", v.kind);
    assert_eq!(back.world_bounds(v.id).map(|r| (r.x0, r.y0, r.x1, r.y1)), doc.world_bounds(c).map(|r| (r.x0, r.y0, r.x1, r.y1)));
}

#[test]
fn the_connector_tool_joins_the_layers_dragged_between() {
    use crate::tools::{PointerEvent, PointerKind, ToolId};
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    s.execute("tool.connector", &json!({})).unwrap();
    assert_eq!(s.tool.tool, ToolId::Connector);
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 30.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 200.0, 40.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 350.0, 30.0, 1.0)).unwrap();
    assert_eq!(s.tool.overlay.drop_target, Some(b), "the layer it would attach to is highlighted");
    s.pointer(&PointerEvent::at(PointerKind::Up, 350.0, 30.0, 1.0)).unwrap();
    let c = s.selection()[0];
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!((start.node, end.node), (Some(a), Some(b)));
    assert_eq!(route(&s, c), vec![(100.0, 30.0), (300.0, 30.0)]);
    // One undo step removes it.
    s.undo().unwrap();
    assert!(!s.doc().unwrap().contains(c));
    // Over empty canvas the ends are free.
    s.execute("tool.connector", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 500.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 200.0, 600.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 200.0, 600.0, 1.0)).unwrap();
    let c = s.selection()[0];
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!((start.node, end.node, end.x, end.y), (None, None, 200.0, 600.0));
}

#[test]
fn new_connectors_have_the_whiteboard_look() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 100.0);
    let c = connect(&mut s, a, b, "elbowed");
    let n = s.doc().unwrap().node(c).unwrap().clone();
    assert_eq!(n.strokes, vec![figcraft_color::Paint::solid(figcraft_doc::connector::DEFAULT_COLOR)], "mid grey, not black");
    assert_eq!(n.stroke.weight, figcraft_doc::connector::THIN);
    assert_eq!(n.stroke.cap_end, figcraft_doc::StrokeCap::LineArrow, "an open arrow, not a filled triangle");
    // The elbow's two bends are rounded: one curve each, between straight runs.
    let NodeKind::Connector { line_type, route, .. } = &n.kind else { panic!() };
    let curves = |p: &figcraft_geom::BezPath| p.elements().iter().filter(|e| matches!(e, figcraft_geom::kurbo::PathEl::CurveTo(..))).count();
    assert_eq!(curves(&figcraft_doc::connector::path(*line_type, route)), 2);
    // Lined up, the elbow is straight: nothing to round.
    s.execute("node.set", &json!({"ids": [b], "props": {"y": 0}})).unwrap();
    let NodeKind::Connector { line_type, route, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!(curves(&figcraft_doc::connector::path(*line_type, route)), 0);
}

#[test]
fn dragging_a_selected_connectors_end_attaches_it_elsewhere() {
    use crate::tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let d = rect(&mut s, 0.0, 300.0);
    let c = connect(&mut s, a, b, "straight");
    s.set_selection(vec![c]);
    // A single connector has no resize handles: its box follows the route.
    assert!(crate::tools::handle_at(s.doc().unwrap(), &[c], figcraft_geom::Point::new(100.0, 30.0), 1.0).is_none());
    // Its end (on b's left side) dragged onto d.
    s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, 30.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 40.0, 320.0, 1.0)).unwrap();
    assert_eq!(s.tool.overlay.drop_target, Some(d), "the layer it would attach to is highlighted");
    s.pointer(&PointerEvent::at(PointerKind::Up, 40.0, 320.0, 1.0)).unwrap();
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!((start.node, end.node), (Some(a), Some(d)));
    assert_eq!(route(&s, c), vec![(50.0, 60.0), (50.0, 300.0)], "bottom of a to top of d");
    // One undo step puts it back on b.
    s.undo().unwrap();
    let NodeKind::Connector { end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!(end.node, Some(b));
    // Not onto the layer the other end is on; over empty canvas the end is let go, free.
    s.set_selection(vec![c]);
    s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, 30.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 50.0, 30.0, 1.0)).unwrap();
    assert_eq!(s.tool.overlay.drop_target, None);
    s.pointer(&PointerEvent::at(PointerKind::Move, 600.0, 500.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 600.0, 500.0, 1.0)).unwrap();
    let NodeKind::Connector { start, end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!((start.node, end.node, end.x, end.y), (Some(a), None, 600.0, 500.0));
    // A locked connector's ends don't move.
    s.execute("node.set", &json!({"ids": [c], "props": {"locked": true}})).unwrap();
    assert!(crate::tools::connector_end_at(s.doc().unwrap(), &[c], figcraft_geom::Point::new(600.0, 500.0), 8.0).is_none());
}

#[test]
fn a_free_end_set_by_node_set_stays_where_it_is_put() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    let b = rect(&mut s, 300.0, 0.0);
    let c = connect(&mut s, a, b, "straight");
    s.execute("node.set", &json!({"ids": [c], "props": {"connectorEnd": {"x": 600, "y": 500}}})).unwrap();
    let NodeKind::Connector { end, .. } = &s.doc().unwrap().node(c).unwrap().kind else { panic!() };
    assert_eq!((end.node, end.x, end.y), (None, 600.0, 500.0));
    assert_eq!(route(&s, c).last().copied(), Some((600.0, 500.0)));
}
