//! Prototyping behaviour from a user report ("the prototype features are still very different"),
//! each checked against Figma's help center.

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// Two top-level frames, A and B, and a button in A.
fn screens(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 300, "name": "A"})));
    let b = idv(&ex(s, "frame.create", json!({"x": 300, "y": 0, "width": 200, "height": 300, "name": "B"})));
    let btn = idv(&ex(s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 30, "parent": a})));
    (a, b, btn)
}

fn flows(s: &Session) -> Vec<(String, NodeId)> {
    let page = s.page().unwrap();
    match &s.doc().unwrap().node(page).unwrap().kind {
        NodeKind::Page { flows, .. } => flows.iter().map(|f| (f.name.clone(), f.start)).collect(),
        _ => vec![],
    }
}

/// "When you add a connection between two frames with no existing connections, Figma will create
/// a starting point on the top-level frame where the connection started": the page's first
/// connection adds Flow 1 at its frame, in the same undo step; later ones add none.
#[test]
fn the_first_connection_starts_a_flow() {
    let mut s = Session::new();
    let (a, b, btn) = screens(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "dest": b}));
    assert_eq!(flows(&s), vec![("Flow 1".to_string(), a)]);
    ex(&mut s, "proto.interaction.add", json!({"ids": [b], "dest": a}));
    assert_eq!(flows(&s).len(), 1, "a page with connections gets no more starting points");
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(flows(&s).is_empty(), "one undo takes the connection and its starting point");
    // Within one frame (a scroll target, say) there's no flow to start.
    let mut s = Session::new();
    let (a, _, btn) = screens(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "dest": a}));
    assert!(flows(&s).is_empty());
}

/// A connection whose destination was deleted stays editable: its trigger and animation can
/// change (only a destination an edit brings in must exist).
#[test]
fn a_connection_to_a_deleted_frame_stays_editable() {
    let mut s = Session::new();
    let (_, b, btn) = screens(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "dest": b}));
    ex(&mut s, "edit.delete", json!({"ids": [b]}));
    ex(&mut s, "proto.interaction.set", json!({"ids": [btn], "index": 0, "durationMs": 500}));
    assert_eq!(s.doc().unwrap().node(btn).unwrap().interactions[0].duration_ms, 500);
    assert!(
        s.execute("proto.interaction.set", &json!({"ids": [btn], "index": 0, "action": {"type": "navigate", "dest": 9999}})).is_err(),
        "a new missing destination is still refused"
    );
}

fn overlay_of(s: &Session, n: NodeId) -> (String, bool) {
    match &s.doc().unwrap().node(n).unwrap().interactions[0].actions[0] {
        figcraft_doc::proto::Action::OpenOverlay { position, background, .. } => (format!("{position:?}"), background.is_some()),
        a => panic!("not an overlay: {a:?}"),
    }
}

/// An overlay's settings belong to the overlay ("These settings apply to the overlay itself, not
/// the connection"): changed through one connection, every connection that opens it opens it the
/// same way, and a new connection to it takes them.
#[test]
fn overlay_settings_are_the_overlays() {
    let mut s = Session::new();
    let (a, b, one) = screens(&mut s);
    let two = idv(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 80, "width": 80, "height": 30, "parent": a})));
    for btn in [one, two] {
        ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "action": {"type": "openOverlay", "dest": b}}));
    }
    ex(
        &mut s,
        "proto.action.set",
        json!({"ids": [one], "index": 0, "path": [0], "action": {"type": "openOverlay", "dest": b, "position": "bottomCenter", "background": "#00000080"}}),
    );
    assert_eq!(overlay_of(&s, two), ("BottomCenter".to_string(), true), "the other connection opens it the same way");
    let three = idv(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 140, "width": 80, "height": 30, "parent": a})));
    ex(&mut s, "proto.interaction.add", json!({"ids": [three], "action": {"type": "openOverlay", "dest": b}}));
    assert_eq!(overlay_of(&s, three), ("BottomCenter".to_string(), true), "a new connection takes them");
}

/// The action editor keeps a connection to a deleted frame editable too.
#[test]
fn the_action_editor_keeps_a_connection_to_a_deleted_frame_editable() {
    let mut s = Session::new();
    let (_, b, btn) = screens(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "action": {"type": "openOverlay", "dest": b}}));
    ex(&mut s, "edit.delete", json!({"ids": [b]}));
    ex(
        &mut s,
        "proto.action.set",
        json!({"ids": [btn], "index": 0, "path": [0], "action": {"type": "openOverlay", "dest": b, "position": "topLeft"}}),
    );
    assert_eq!(overlay_of(&s, btn).0, "TopLeft");
}

/// A flow's starting point moves to another top-level frame (its tag dragged there), keeping its
/// name; not onto a frame that starts another flow, nor onto a layer inside a frame.
#[test]
fn a_flow_moves_to_another_frame() {
    let mut s = Session::new();
    let (a, b, btn) = screens(&mut s);
    ex(&mut s, "proto.interaction.add", json!({"ids": [btn], "dest": b}));
    ex(&mut s, "proto.flow.move", json!({"start": a, "to": b}));
    assert_eq!(flows(&s), vec![("Flow 1".to_string(), b)]);
    assert!(s.execute("proto.flow.move", &json!({"start": b, "to": btn})).is_err(), "not inside a frame");
    ex(&mut s, "proto.flow.add", json!({"start": a, "name": "Other"}));
    assert!(s.execute("proto.flow.move", &json!({"start": a, "to": b})).is_err(), "B starts a flow already");
}
