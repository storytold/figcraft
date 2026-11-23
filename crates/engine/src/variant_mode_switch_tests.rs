//! A nested instance whose variant property is bound to a string variable switches variant when
//! the mode around it changes, and keeps what the main around it overrides: a Button main makes
//! its Chip's tint red, and a Button instance in a frame set to Dark shows the Dark Chip with that
//! red tint (the reference tool applies an enclosing main's overrides over whichever variant).

use serde_json::{Value, json};

use crate::Session;
use figcraft_doc::{NodeId, NodeKind};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd}: {e}"),
    }
}

fn id_of(v: &Value) -> u64 {
    v["id"].as_u64().or_else(|| v["ids"][0].as_u64()).unwrap_or_else(|| panic!("no id in {v}"))
}

/// A `name` frame holding a `Tint` rectangle filled `fill`; returns (frame, tint).
fn variant(s: &mut Session, name: &str, x: f64, fill: &str) -> (u64, u64) {
    let f = id_of(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 40, "height": 20, "name": name})));
    let t =
        id_of(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": f, "name": "Tint", "props": {"fill": fill}})));
    (f, t)
}

fn fill_hex(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex()
}

/// The scene: Light and Dark Chip variants (a white or black `Tint`), a Button main holding a
/// Chip whose tint it makes red and whose Mode it binds to the variable, and a Button instance in
/// a `Screen` frame. Returns the session, the screen and the Button instance.
fn scene() -> (Session, u64, u64) {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    ex(&mut s, "variable.createCollection", json!({"name": "Scheme", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Scheme", "name": "Mode", "type": "string", "values": {"Light": "Light", "Dark": "Dark"}}));
    let (light, _) = variant(&mut s, "Mode=Light", 0.0, "#FFFFFF");
    let (dark, _) = variant(&mut s, "Mode=Dark", 60.0, "#000000");
    for f in [light, dark] {
        ex(&mut s, "component.create", json!({"ids": [f]}));
    }
    let comps: Vec<u64> = [light, dark]
        .iter()
        .map(|f| {
            let doc = s.doc().unwrap();
            let n = doc.node(NodeId(*f)).unwrap();
            if matches!(n.kind, NodeKind::Component { .. }) { *f } else { n.children[0].0 }
        })
        .collect();
    ex(&mut s, "component.combineVariants", json!({"ids": comps}));
    // A Button main holding a Chip (Light) whose tint it makes red, its Mode bound to the variable.
    let button = id_of(&ex(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 40, "height": 20, "name": "Button"})));
    let chip = id_of(&ex(&mut s, "instance.create", json!({"component": comps[0], "x": 0, "y": 0, "parent": button})));
    ex(&mut s, "component.create", json!({"ids": [button]}));
    let tint_in_main = {
        let doc = s.doc().unwrap();
        doc.subtree(NodeId(chip)).into_iter().find(|i| doc.node(*i).is_some_and(|n| n.name == "Tint")).unwrap()
    };
    ex(&mut s, "node.set", json!({"ids": [tint_in_main.0], "props": {"fill": "#FF0000"}}));
    ex(&mut s, "variable.bind", json!({"ids": [chip], "field": "componentProperties/Mode", "variable": "Mode"}));
    // A screen with a Button, switched to Dark.
    let screen = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 200, "width": 100, "height": 60, "name": "Screen"})));
    let main_button = {
        let doc = s.doc().unwrap();
        doc.nodes().find(|n| n.name == "Button" && matches!(n.kind, NodeKind::Component { .. })).unwrap().id
    };
    let used = id_of(&ex(&mut s, "instance.create", json!({"component": main_button.0, "x": 10, "y": 10, "parent": screen})));
    (s, screen, used)
}

/// The Button instance's Chip shows the Dark variant with the Button main's red tint.
fn assert_dark_with_red_tint(s: &Session, used: u64) {
    let doc = s.doc().unwrap();
    let used_chip = doc
        .subtree(NodeId(used))
        .into_iter()
        .find(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Instance { .. }) && n.id.0 != used))
        .unwrap();
    let NodeKind::Instance { component, .. } = &doc.node(used_chip).unwrap().kind else { panic!("not an instance") };
    assert_eq!(doc.node(*component).unwrap().name, "Mode=Dark");
    let tint = doc.subtree(used_chip).into_iter().find(|i| doc.node(*i).is_some_and(|n| n.name == "Tint")).unwrap();
    assert_eq!(fill_hex(s, tint), "FF0000", "the Button main's red tint stays on the Dark variant");
}

#[test]
fn a_nested_variant_bound_to_a_mode_keeps_its_mains_overrides_when_the_mode_switches() {
    let (mut s, screen, used) = scene();
    ex(&mut s, "variable.setMode", json!({"ids": [screen], "collection": "Scheme", "mode": "Dark"}));
    assert_dark_with_red_tint(&s, used);
}

/// An opened file settles the same way: its layers inherit the main's overrides (as a `.fig`
/// file's instances do) when the variant switches on load.
#[test]
fn an_opened_files_nested_variant_keeps_its_mains_overrides() {
    let (mut s, screen, used) = scene();
    let doc = s.doc_mut().unwrap();
    let coll = doc.variables.collections.iter().find(|c| c.name == "Scheme").unwrap().clone();
    let dark = coll.modes.iter().find(|m| m.name == "Dark").unwrap().id.clone();
    doc.get_mut(NodeId(screen)).unwrap().var_modes.insert(coll.id.clone(), dark);
    crate::cmd::styles::apply_prop_bindings_on_load(doc);
    assert_dark_with_red_tint(&s, used);
}
