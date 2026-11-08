//! The Pencil takes every pointer sample of a frame, not only where the pointer ended: a fast
//! stroke, or a slow frame, would otherwise draw a straight line across a curve.

use egui::{Event, Modifiers, PointerButton};
use figcraft_doc::NodeKind;
use figcraft_engine::Session;
use figcraft_geom::Point;
use figcraft_geom::kurbo::ParamCurveNearest;
use serde_json::json;

use crate::layers_drag_tests::Ui;

#[test]
fn a_stroke_keeps_the_samples_between_frames() {
    let mut u = Ui::new(Session::new());
    u.frame(vec![]);
    u.app.session.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    u.frame(vec![]);
    let (a, tip, b) = (u.app.to_screen(100.0, 100.0), u.app.to_screen(150.0, 200.0), u.app.to_screen(200.0, 100.0));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    u.frame(vec![Event::PointerMoved(a), button(a, true)]);
    // Down to the tip and back up in one frame: only the tip's sample says the stroke went there.
    let step = |t: f32, from: egui::Pos2, to: egui::Pos2| from + (to - from) * t;
    let mut moves: Vec<Event> = (1..=5).map(|k| Event::PointerMoved(step(k as f32 / 5.0, a, tip))).collect();
    moves.extend((1..=5).map(|k| Event::PointerMoved(step(k as f32 / 5.0, tip, b))));
    u.frame(moves);
    u.frame(vec![button(b, false)]);
    u.frame(vec![]);
    let doc = u.app.session.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a pencil stroke");
    let NodeKind::Vector { network } = &n.kind else { panic!("not a vector") };
    let path = doc.world_transform(n.id) * network.stroke_path();
    let off = path.segments().map(|s| s.nearest(Point::new(150.0, 200.0), 1e-6).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
    assert!(off < 5.0, "the stroke reaches the tip it was drawn through: {off} away");
}
