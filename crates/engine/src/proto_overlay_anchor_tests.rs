use serde_json::{Value, json};

use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::NodeId;
use figcraft_doc::proto::{Action, OverlayAnchor, OverlayPosition};

fn frame(s: &mut Session, x: f64, w: f64, h: f64, name: &str) -> NodeId {
    id_of(&s.execute("frame.create", &json!({"x": x, "y": 0, "width": w, "height": h, "name": name})).unwrap()["id"]).unwrap()
}

fn button(s: &mut Session, parent: NodeId) -> NodeId {
    let r = s.execute("node.create", &json!({"type": "rectangle", "x": 10, "y": 30, "width": 100, "height": 40, "parent": parent, "name": "Button"}));
    id_of(&r.unwrap()["id"]).unwrap()
}

fn action(s: &Session, id: NodeId) -> Action {
    s.doc().unwrap().node(id).unwrap().interactions[0].actions[0].clone()
}

fn dropdown(s: &mut Session) -> (NodeId, NodeId) {
    let screen = frame(s, 0.0, 400.0, 800.0, "Screen");
    let menu = frame(s, 600.0, 60.0, 200.0, "Menu");
    let b = button(s, screen);
    let open = json!({"type": "openOverlay", "dest": menu, "position": "bottomLeft", "anchor": "triggerOutside"});
    s.execute("proto.interaction.add", &json!({"id": b, "action": open})).unwrap();
    (b, menu)
}

#[test]
fn the_anchor_is_set_read_and_undone() {
    let mut s = Session::new();
    let (b, menu) = dropdown(&mut s);
    assert!(matches!(action(&s, b), Action::OpenOverlay { anchor: OverlayAnchor::TriggerOutside, position: OverlayPosition::BottomLeft, .. }));
    s.execute(
        "proto.interaction.set",
        &json!({"id": b, "index": 0, "action": {"type": "openOverlay", "dest": menu, "position": "topCenter", "anchor": "triggerInside"}}),
    )
    .unwrap();
    assert!(matches!(action(&s, b), Action::OpenOverlay { anchor: OverlayAnchor::TriggerInside, position: OverlayPosition::TopCenter, .. }));
    assert!(s.undo().unwrap());
    assert!(matches!(action(&s, b), Action::OpenOverlay { anchor: OverlayAnchor::TriggerOutside, .. }));
}

#[test]
fn fig_writes_a_manual_offset_from_the_trigger() {
    let mut s = Session::new();
    let (b, _) = dropdown(&mut s);
    let (bytes, _) = figcraft_format::fig::export(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::fig::import(&bytes, "x").unwrap();
    let hot = back.nodes().find(|n| n.name == "Button").unwrap();
    assert!(
        matches!(hot.interactions[0].actions[0], Action::OpenOverlay { position: OverlayPosition::Manual, offset: Some([0.0, 40.0]), .. }),
        "{:?} (from {b:?})",
        hot.interactions[0].actions
    );
}

#[test]
fn junk_anchors_are_refused_not_fatal() {
    let junk = [json!(true), json!(-1), json!("above"), json!([1, 2]), json!({"a": 1}), json!("x".repeat(10_000))];
    for j in &junk {
        let mut s = Session::new();
        let (a, menu) = (frame(&mut s, 0.0, 100.0, 100.0, "A"), frame(&mut s, 200.0, 50.0, 50.0, "M"));
        let p: Value = json!({"id": a, "action": {"type": "openOverlay", "dest": menu, "anchor": j}});
        let r = s.execute("proto.interaction.add", &p);
        assert!(!matches!(r, Err(EngineError::Internal { .. })), "{p}: {r:?}");
        assert!(s.doc().unwrap().node(a).unwrap().interactions.is_empty(), "{j}");
    }
}
