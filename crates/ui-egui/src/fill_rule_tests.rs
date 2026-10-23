//! The Fill section's fill-rule panel: the button beside Styles and Add fill opens it, its preview
//! switches an area's rule and turns a loop round, and the X closes it (so does picking another
//! layer). The rules themselves are the engine's (`vector.fillRule`, `vector.reverseLoop`); this
//! checks the panel drives them from where the pointer is.

use figcraft_doc::{NodeId, NodeKind, VectorNetwork, WindingRule};
use figcraft_geom::{BezPath, Point};
use serde_json::json;

use crate::fill_rule::{self, Panel};

/// A 100 × 100 square with a 25…75 hole, both loops in one area and running the same way (so
/// non-zero fills the hole too).
fn ring() -> VectorNetwork {
    let mut path = BezPath::new();
    for (x0, y0, x1, y1) in [(0.0, 0.0, 100.0, 100.0), (25.0, 25.0, 75.0, 75.0)] {
        path.move_to((x0, y0));
        path.line_to((x1, y0));
        path.line_to((x1, y1));
        path.line_to((x0, y1));
        path.close_path();
    }
    let mut net = VectorNetwork::from_path(&path);
    net.regions[0].winding = WindingRule::NonZero;
    net
}

/// A selected vector layer holding [`ring`].
fn ringed(h: &mut crate::live_tests::Harness) -> NodeId {
    let id = NodeId(h.app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    h.app.session.execute("vector.edit.begin", &json!({"id": id})).unwrap();
    h.app.session.execute("vector.edit.end", &json!({})).unwrap();
    {
        let doc = h.app.session.doc_mut().unwrap();
        match &mut doc.get_mut(id).unwrap().kind {
            NodeKind::Vector { network } => *network = ring(),
            other => panic!("not a vector: {other:?}"),
        }
    }
    h.app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    id
}

fn network(h: &crate::live_tests::Harness, id: NodeId) -> VectorNetwork {
    match &h.app.session.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Vector { network } => network.clone(),
        other => panic!("not a vector: {other:?}"),
    }
}

/// The preview as the panel drew it, with the panel open. A few frames: the panel takes the
/// window's content rect the frame after it opens.
fn preview(h: &mut crate::live_tests::Harness) -> egui::Rect {
    h.frames(4);
    let p = h.app.ui.fill_rule.as_ref().expect("the panel shows");
    assert!(p.preview.width() > 100.0 && p.preview.height() > 100.0, "the preview is laid out: {:?}", p.preview);
    p.preview
}

#[test]
fn the_button_opens_the_panel_and_the_x_closes_it() {
    let (mut h, _) = crate::live_tests::Harness::new();
    let id = ringed(&mut h);
    assert!(fill_rule::toggle(&mut h.app, 260.0), "a vector layer opens it");
    assert_eq!(h.app.ui.fill_rule.as_ref().map(|p| p.id), Some(id), "opened for the vector");
    h.frames(2);
    assert!(h.app.ui.fill_rule.is_some(), "it stays open");
    // The same button again closes it.
    assert!(fill_rule::toggle(&mut h.app, 260.0));
    assert!(h.app.ui.fill_rule.is_none(), "the button toggles it shut");
    // The X, at the right of the title.
    assert!(fill_rule::toggle(&mut h.app, 260.0));
    let r = preview(&mut h);
    h.click(egui::pos2(r.max.x - 12.0, r.min.y - 20.0));
    assert!(h.app.ui.fill_rule.is_none(), "the X closes it");
    // A rectangle has no areas to set a rule on: no panel.
    let rect = NodeId(h.app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    h.app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    assert!(!fill_rule::toggle(&mut h.app, 260.0), "nothing to show for a rectangle");
    assert!(h.app.ui.fill_rule.is_none());
}

#[test]
fn a_click_switches_the_area_under_it_and_a_click_on_a_loop_turns_it_round() {
    let (mut h, _) = crate::live_tests::Harness::new();
    let id = ringed(&mut h);
    assert!(fill_rule::toggle(&mut h.app, 260.0));
    let r = preview(&mut h);
    // The band between the square's edge and its hole: a click switches the area's rule.
    let at = fill_rule::preview_point(&ring(), r, Point::new(10.0, 50.0)).expect("the preview maps it");
    assert!(r.contains(at), "{at:?} is in the preview {r:?}");
    h.click(at);
    h.frames(1);
    assert_eq!(network(&h, id).regions[0].winding, WindingRule::EvenOdd, "the area switched to even-odd");
    assert!(h.app.ui.fill_rule.is_some(), "the panel stays open");
    // The hole's own loop: a click turns it round (and the area's rule stays as it is).
    let before = network(&h, id).regions[0].loops.clone();
    let at = fill_rule::preview_point(&ring(), r, Point::new(25.0, 50.0)).expect("the preview maps it");
    h.click(at);
    h.frames(1);
    let after = network(&h, id);
    assert_ne!(after.regions[0].loops, before, "the loop under the pointer turned round");
    assert_eq!(after.regions[0].loops.len(), 2, "and it is still two loops");
    assert_eq!(after.regions[0].winding, WindingRule::EvenOdd, "clicking a loop leaves the rule alone");
    // Undo puts the loop back.
    h.app.session.undo().unwrap();
    assert_eq!(network(&h, id).regions[0].loops, before);
}

#[test]
fn the_preview_hatches_what_the_rule_fills() {
    let net = ring();
    let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(280.0, 190.0));
    let at = |q: (f64, f64)| fill_rule::preview_point(&net, r, Point::new(q.0, q.1)).expect("mapped");
    let (a, b) = (at((25.0, 25.0)), at((75.0, 75.0)));
    let hole = egui::Rect::from_min_max(egui::pos2(a.x.min(b.x), a.y.min(b.y)), egui::pos2(a.x.max(b.x), a.y.max(b.y)));
    let inside = |segs: &[[egui::Pos2; 2]], area: egui::Rect| segs.iter().filter(|[p, q]| area.contains(*p + (*q - *p) * 0.5)).count();
    // Non-zero with both loops running the same way fills the hole, so the hatch covers it.
    let nonzero = fill_rule::area_hatch(&net, 0, r);
    assert!(nonzero.len() > 20, "the square is hatched: {} segments", nonzero.len());
    assert!(inside(&nonzero, hole) > 0, "non-zero fills the hole too");
    for [p, q] in &nonzero {
        let mid = *p + (*q - *p) * 0.5;
        assert!(r.contains(mid), "every segment stays in the preview: {p:?}-{q:?} mid {mid:?}");
    }
    // Even-odd leaves the hole empty: the same lines, cut around it.
    let mut even = net.clone();
    even.regions[0].winding = WindingRule::EvenOdd;
    let even_odd = fill_rule::area_hatch(&even, 0, r);
    assert!(even_odd.len() > 20, "the band is still hatched: {}", even_odd.len());
    assert_eq!(inside(&even_odd, hole), 0, "nothing lands in the hole");
    assert!(even_odd.len() < nonzero.len(), "the lines through the hole lost their middle");
}

#[test]
fn another_selection_closes_the_panel() {
    let (mut h, _) = crate::live_tests::Harness::new();
    let id = ringed(&mut h);
    assert!(fill_rule::toggle(&mut h.app, 260.0));
    h.frames(2);
    assert!(h.app.ui.fill_rule.is_some());
    let other =
        NodeId(h.app.session.execute("shape.rectangle", &json!({"x": 300, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    h.app.session.execute("select.set", &json!({"ids": [other]})).unwrap();
    h.frames(2);
    assert!(h.app.ui.fill_rule.is_none(), "the panel edits the layer it opened for");
    // Selecting it again does not bring the panel back by itself.
    h.app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    h.frames(2);
    assert!(h.app.ui.fill_rule.is_none());
    assert_eq!(Panel::open(id, 0.0).id, id, "the panel opens on a layer of its own");
}

#[test]
fn the_fill_rule_button_closes_the_panel_on_a_real_press_and_release() {
    let (mut h, _) = crate::live_tests::Harness::new();
    let id = ringed(&mut h);
    assert!(fill_rule::toggle(&mut h.app, 260.0), "a vector layer opens it");
    h.frames(2);
    // The right panel draws the Fill section's fill-rule button and records its rect for the
    // outside-press-close check.
    let btn = h.app.ui.fill_rule_button.expect("the Fill section shows a fill-rule button for a vector");
    // A real press on the button, then a release: with the bug the press counted as "outside" and
    // closed the panel, and the release reopened it, so it would stay open. Excluding the button's
    // rect from the outside check lets the press keep it open and the release toggle it shut.
    let at = btn.center();
    h.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
    ]);
    h.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ]);
    assert!(h.app.ui.fill_rule.is_none(), "the button toggles the panel shut, not reopen it");
    // The layer it opened for is still selected; opening it again works.
    assert!(fill_rule::toggle(&mut h.app, 260.0));
    assert_eq!(h.app.ui.fill_rule.as_ref().map(|p| p.id), Some(id));
}
