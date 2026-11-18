//! A boolean group past the path operations' caps: it stays a boolean group, flattening it is
//! refused with the reason, and the document is unchanged.

use figcraft_doc::{NodeKind, VectorNetwork};
use figcraft_geom::kurbo::{BezPath, Rect, Shape};
use serde_json::json;

use crate::Session;
use crate::params::id_of;

/// `n` long thin bars, across or down: 750 of each cross 2.25 million times, past the caps.
fn bars(n: usize, across: bool) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..n {
        let (at, len) = (i as f64 * 4.0, n as f64 * 4.0);
        let r = if across { Rect::new(0.0, at, len, at + 2.0) } else { Rect::new(at, 0.0, at + 2.0, len) };
        p.extend(r.path_elements(0.1));
    }
    p
}

#[test]
fn flattening_a_boolean_past_the_caps_is_refused() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let ids: Vec<_> = [true, false]
        .into_iter()
        .map(|across| {
            let network = VectorNetwork::from_path(&bars(750, across));
            s.doc_mut().unwrap().create(page, NodeKind::Vector { network }, "Bars", 0.0, 0.0, 3000.0, 3000.0).unwrap()
        })
        .collect();
    let g = id_of(&s.execute("bool.union", &json!({"ids": ids})).unwrap()["id"]).unwrap();
    let e = s.execute("vector.flatten", &json!({"ids": [g]})).unwrap_err().to_string();
    assert!(e.contains("too complex to combine"), "{e}");
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(g).map(|n| &n.kind), Some(NodeKind::BooleanOp { .. })), "the group is kept");
    assert_eq!(doc.node(g).map(|n| n.children.len()), Some(2));
}
