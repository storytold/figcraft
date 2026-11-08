//! The vector edit panel's Point section: X and Y show the value the selected points share, or
//! "Mixed", and a typed value sets every selected point's own coordinate.

use egui::Key;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::field_tests::{field, type_into};
use crate::layers_drag_tests::Ui;

fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 60})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("vector.edit.begin", &json!({"id": id})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    (u, id)
}

fn xs(u: &Ui, id: NodeId) -> Vec<(f64, f64)> {
    let doc = u.app.session.doc().unwrap();
    // In the parent's coordinates: the layer re-fits its box after an edit.
    let t = doc.node(id).unwrap().transform;
    figcraft_engine::vectoredit::network(doc, id).unwrap().vertices.iter().map(|v| t * v.point()).map(|p| (p.x, p.y)).collect()
}

#[test]
fn x_reads_mixed_for_points_that_differ_and_typing_sets_every_point() {
    let (mut u, id) = setup();
    u.app.session.execute("vector.edit.select", &json!({"vertices": [0, 1]})).unwrap();
    let before = xs(&u, id);
    // Points 0 and 1 share their y but not their x.
    assert_eq!(field(&mut u, "X").1, "Mixed");
    assert_eq!(field(&mut u, "Y").1, "0");
    type_into(&mut u, "X", "-200", Key::Enter);
    let after = xs(&u, id);
    assert_eq!((after[0].0, after[1].0), (-200.0, -200.0));
    assert_eq!((after[0].1, after[1].1), (before[0].1, before[1].1));
    assert_eq!(field(&mut u, "X").1, "-200", "the points share an x now");
    // A relative edit of a Mixed field goes to each point from its own value.
    u.app.session.execute("vector.edit.select", &json!({"vertices": [1, 2]})).unwrap();
    assert_eq!(field(&mut u, "X").1, "Mixed");
    let b = xs(&u, id);
    type_into(&mut u, "X", "+10", Key::Enter);
    let a = xs(&u, id);
    assert_eq!((a[1].0, a[2].0), (b[1].0 + 10.0, b[2].0 + 10.0));
}

#[test]
fn one_point_shows_its_own_value() {
    let (mut u, id) = setup();
    u.app.session.execute("vector.edit.select", &json!({"vertices": [2]})).unwrap();
    let p = xs(&u, id)[2];
    assert_eq!(field(&mut u, "X").1, format!("{}", p.0));
    assert_eq!(field(&mut u, "Y").1, format!("{}", p.1));
}
