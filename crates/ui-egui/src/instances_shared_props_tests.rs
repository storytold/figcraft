use egui::accesskit::{Node, Role};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::component_swap_default_tests::{centre, click, tree};
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn combos(nodes: &[Node]) -> Vec<String> {
    nodes.iter().filter(|n| n.role() == Role::ComboBox).filter_map(|n| n.value().map(str::to_string)).collect()
}

fn small_and_large_instances_selected(with_a_rectangle: bool) -> (Ui, Vec<(String, NodeId)>, Vec<NodeId>) {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, name) in ["Size=S, State=Default", "Size=L, State=Default", "Size=S, State=Hover", "Size=L, State=Hover"].iter().enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": 20 + k * 44, "width": 80, "height": 30, "name": name})).unwrap());
        variants.push((name.to_string(), id(s.execute("component.create", &json!({"ids": [f]})).unwrap())));
    }
    s.execute("component.combineVariants", &json!({"ids": variants.iter().map(|v| v.1).collect::<Vec<_>>()})).unwrap();
    let small = id(s.execute("instance.create", &json!({"component": variants[0].1, "x": 300, "y": 20})).unwrap());
    let large = id(s.execute("instance.create", &json!({"component": variants[1].1, "x": 300, "y": 80})).unwrap());
    let mut selected = vec![small, large];
    if with_a_rectangle {
        selected.push(id(s.execute("shape.rectangle", &json!({"x": 500, "y": 20, "width": 20, "height": 20})).unwrap()));
    }
    s.execute("select.set", &json!({"ids": selected})).unwrap();
    let u = Ui::new(s);
    u.ctx.enable_accesskit();
    (u, variants, vec![small, large])
}

fn main_of(u: &Ui, instance: NodeId) -> NodeId {
    match &u.app.session.doc().unwrap().node(instance).unwrap().kind {
        NodeKind::Instance { component, .. } => *component,
        _ => panic!("not an instance"),
    }
}

fn pick(u: &mut Ui, shown: &str, value: &str) {
    let nodes = tree(u);
    let dropdown = nodes.iter().find(|n| n.role() == Role::ComboBox && n.value() == Some(shown)).unwrap_or_else(|| panic!("a {shown} dropdown"));
    click(u, centre(dropdown));
    let nodes = tree(u);
    let item = nodes
        .iter()
        .find(|n| n.label() == Some(value) && !matches!(n.role(), Role::ComboBox | Role::TextInput) && n.bounds().is_some())
        .unwrap_or_else(|| panic!("{value} in the menu"));
    click(u, centre(item));
}

#[test]
fn two_selected_instances_show_their_shared_variant_properties_and_set_them_together() {
    let (mut u, variants, instances) = small_and_large_instances_selected(false);
    let variant = |name: &str| variants.iter().find(|v| v.0 == name).unwrap().1;

    let shown = combos(&tree(&mut u));
    assert!(shown.contains(&"Mixed".to_string()), "Size differs between them: {shown:?}");
    assert!(shown.contains(&"Default".to_string()), "State is the same: {shown:?}");

    pick(&mut u, "Mixed", "L");
    assert_eq!(instances.iter().map(|i| main_of(&u, *i)).collect::<Vec<_>>(), [variant("Size=L, State=Default"); 2]);

    pick(&mut u, "Default", "Hover");
    assert_eq!(instances.iter().map(|i| main_of(&u, *i)).collect::<Vec<_>>(), [variant("Size=L, State=Hover"); 2]);
}

#[test]
fn instances_selected_with_a_plain_layer_show_no_instance_properties() {
    let (mut u, _, _) = small_and_large_instances_selected(true);
    let shown = combos(&tree(&mut u));
    assert!(!shown.iter().any(|v| v == "Mixed" || v == "Default"), "no property rows: {shown:?}");
}
