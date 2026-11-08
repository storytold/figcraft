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
