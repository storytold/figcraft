//! `vector.offset` on layers with children: containers are left alone.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{NodeId, NodeKind};

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// No node that holds children may be a non-container.
fn assert_children_only_in_containers(s: &Session, ids: &[NodeId]) {
    let doc = s.doc().unwrap();
    for &i in ids {
        let n = doc.node(i).unwrap();
        assert!(n.kind.is_container() || n.kind.is_frame_like() || n.children.is_empty(), "a non-container layer now holds children");
    }
}

#[test]
fn offset_path_leaves_containers_with_children_alone() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let inner = id(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10, "parent": f})).unwrap());
    let a = id(&s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 220, "y": 0, "width": 10, "height": 10})).unwrap());
    let g = id(&s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    for target in [f, g] {
        s.execute("vector.offset", &json!({"ids": [target], "offset": 5})).unwrap();
        assert_children_only_in_containers(&s, &[f, g, inner, a, b]);
    }
    let doc = s.doc().unwrap();
    assert!(doc.node(f).unwrap().kind.is_container());
    assert_eq!(doc.node(f).unwrap().children, vec![inner]);
    assert!(doc.node(g).unwrap().kind.is_container());
    assert_eq!(doc.node(g).unwrap().children.len(), 2);
}

#[test]
fn offset_path_still_converts_a_plain_rectangle() {
    let mut s = Session::new();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("vector.offset", &json!({"ids": [r], "offset": 5})).unwrap();
    let n = s.doc().unwrap().node(r).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Vector { .. }));
    assert!(n.width > 10.0);
}
