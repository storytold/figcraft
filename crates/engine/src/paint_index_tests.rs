//! `fill.set` / `stroke.set` reject a negative, fractional or non-finite `index` instead of
//! clamping or truncating it onto another paint (like `fill.remove` / `stroke.remove`).

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn paints(s: &Session, id: NodeId) -> String {
    let n = s.doc().unwrap().node(id).unwrap();
    format!("{:?} {:?}", n.fills, n.strokes)
}

#[test]
fn paint_set_rejects_bad_index_and_leaves_the_paints_alone() {
    let mut s = Session::new();
    let r = NodeId(ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    for add in ["fill.add", "stroke.add"] {
        for c in ["#0000FF", "#FFFF00"] {
            ex(&mut s, add, json!({"ids": [r], "color": c}));
        }
    }
    let before = paints(&s, r);
    for cmd in ["fill.set", "stroke.set"] {
        for bad in [json!(-1), json!(0.5), json!(1.9), json!("-1"), json!("1.5"), json!("NaN"), json!("inf"), json!("abc")] {
            let res = s.execute(cmd, &json!({"ids": [r], "index": bad, "color": "#FF0000"}));
            assert!(res.is_err(), "{cmd} index {bad} was accepted: {res:?}");
            assert_eq!(paints(&s, r), before, "{cmd} index {bad} changed the document");
        }
        ex(&mut s, cmd, json!({"ids": [r], "index": 1, "color": "#FF0000"}));
        assert_ne!(paints(&s, r), before, "{cmd} with a valid index still edits");
        ex(&mut s, "edit.undo", json!({}));
        assert_eq!(paints(&s, r), before);
    }
}
