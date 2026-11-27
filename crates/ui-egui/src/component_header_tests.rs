//! The Design panel's header with one component or component set selected, driven through egui
//! headless: it names the selected one ("Knapp"), not only its type (#2347). Other layers keep their
//! type ("Rectangle").

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::export_preview_tests::labelled;
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The text painted in the right panel's header strip (the top of the Design tab).
fn header(u: &mut Ui) -> Vec<String> {
    let right = 1440.0 - u.app.ui.right_width;
    labelled(u).into_iter().filter(|(_, at)| at.x > right && at.y < 190.0).map(|(t, _)| t).collect()
}

#[test]
fn the_header_names_the_selected_component_or_set() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Knapp"})).unwrap());
    let c = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let mut u = Ui::new(s);
    let shown = header(&mut u);
    assert!(shown.iter().any(|t| t == "Knapp"), "the component's name: {shown:?}");
    assert!(!shown.iter().any(|t| t == "Component"), "not only its type: {shown:?}");

    // As a set (Add variant), it is the set's name.
    u.app.run("component.addVariant", json!({"id": c}));
    let set = u.app.session.doc().unwrap().node(c).unwrap().parent.unwrap();
    u.app.run("node.set", json!({"ids": [set], "props": {"name": "Knappar"}}));
    u.app.run("select.set", json!({"ids": [set]}));
    let shown = header(&mut u);
    assert!(shown.iter().any(|t| t == "Knappar"), "the set's name: {shown:?}");

    // Another layer still shows its type.
    u.app.run("select.set", json!({"ids": [r]}));
    let shown = header(&mut u);
    assert!(shown.iter().any(|t| t == "Rectangle"), "{shown:?}");
}
