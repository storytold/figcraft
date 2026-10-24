//! Variable modes on instances: an instance is on the modes its main component sets, under the
//! ones set on the instance itself, so switching variants or changing the main's mode reaches it.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn fill(s: &Session, n: NodeId) -> String {
    s.doc().unwrap().node(n).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn set_mode(s: &mut Session, n: NodeId, mode: Value) {
    run(s, "variable.setMode", json!({"ids": [n], "collection": "Tone", "mode": mode}));
}

/// A "Tone" collection whose variable `c` is red in mode A and blue in mode B.
fn tone(s: &mut Session) {
    run(s, "variable.createCollection", json!({"name": "Tone", "modes": ["A", "B"]}));
    run(s, "variable.create", json!({"collection": "Tone", "name": "c", "type": "color", "values": {"A": "#FF0000", "B": "#0000FF"}}));
}

/// A component filled from `c`, on `mode` when one is given.
fn swatch(s: &mut Session, name: &str, mode: Option<&str>) -> NodeId {
    let f = id(run(s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 40, "name": name})));
    run(s, "variable.bind", json!({"ids": [f], "field": "fill", "variable": "c"}));
    if let Some(m) = mode {
        set_mode(s, f, json!(m));
    }
    id(run(s, "component.create", json!({"ids": [f]})))
}

/// A set "Kind" whose variants A and B each set their own Tone mode.
fn kinds(s: &mut Session) -> (NodeId, NodeId) {
    tone(s);
    let a = swatch(s, "Kind=A", Some("A"));
    let b = swatch(s, "Kind=B", Some("B"));
    let set = id(run(s, "component.combineVariants", json!({"ids": [a, b]})));
    (set, b)
}

fn instance(s: &mut Session, component: NodeId) -> NodeId {
    id(run(s, "instance.create", json!({"component": component, "x": 200, "y": 0})))
}

fn switch(s: &mut Session, i: NodeId, kind: &str) {
    run(s, "instance.setProperties", json!({"ids": [i], "properties": {"Kind": kind}}));
}

#[test]
fn switching_variants_takes_the_new_variants_mode() {
    let mut s = Session::new();
    let (set, _) = kinds(&mut s);
    let i = instance(&mut s, set);
    assert_eq!(fill(&s, i), "FF0000");
    switch(&mut s, i, "B");
    assert_eq!(fill(&s, i), "0000FF", "variant B sets mode B");
    switch(&mut s, i, "A");
    assert_eq!(fill(&s, i), "FF0000");
    assert!(s.doc().unwrap().node(i).unwrap().var_modes.is_empty(), "the instance sets no mode of its own");
}

#[test]
fn instances_follow_a_mode_their_main_sets_later() {
    let mut s = Session::new();
    tone(&mut s);
    let main = swatch(&mut s, "Chip", None);
    let i = instance(&mut s, main);
    assert_eq!(fill(&s, i), "FF0000", "no mode set: the collection's default");
    set_mode(&mut s, main, json!("B"));
    assert_eq!(fill(&s, i), "0000FF");
    set_mode(&mut s, main, Value::Null);
    assert_eq!(fill(&s, i), "FF0000", "the main back on Auto");
}

#[test]
fn a_mode_set_on_the_instance_wins_and_stays_through_variant_switches() {
    let mut s = Session::new();
    let (set, _) = kinds(&mut s);
    let i = instance(&mut s, set);
    set_mode(&mut s, i, json!("B"));
    assert_eq!(fill(&s, i), "0000FF");
    for kind in ["B", "A"] {
        switch(&mut s, i, kind);
        assert_eq!(fill(&s, i), "0000FF", "own mode B on variant {kind}");
    }
    set_mode(&mut s, i, Value::Null);
    assert_eq!(fill(&s, i), "FF0000", "back on Auto: variant A's mode");
}

#[test]
fn a_main_mode_is_closer_than_the_frame_around_the_instance() {
    let mut s = Session::new();
    let (_, b) = kinds(&mut s);
    let frame = id(run(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 200, "height": 100})));
    set_mode(&mut s, frame, json!("A"));
    let i = id(run(&mut s, "instance.create", json!({"component": b, "x": 10, "y": 10, "parent": frame})));
    assert_eq!(fill(&s, i), "0000FF", "the main's mode B, not the frame's A");
    // Nested: an outer component on mode A holding an instance of B.
    let outer_frame = id(run(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 100, "height": 100, "name": "Outer"})));
    run(&mut s, "instance.create", json!({"component": b, "x": 0, "y": 0, "parent": outer_frame}));
    set_mode(&mut s, outer_frame, json!("A"));
    let outer = id(run(&mut s, "component.create", json!({"ids": [outer_frame]})));
    let copy = instance(&mut s, outer);
    let nested = s.doc().unwrap().node(copy).unwrap().children[0];
    assert_eq!(fill(&s, nested), "0000FF");
}

#[test]
fn detaching_keeps_the_mode_it_took_from_its_main() {
    let mut s = Session::new();
    let (set, b) = kinds(&mut s);
    let i = instance(&mut s, set);
    switch(&mut s, i, "B");
    run(&mut s, "component.detach", json!({"ids": [i]}));
    set_mode(&mut s, b, json!("A"));
    assert_eq!(fill(&s, i), "0000FF", "the detached frame keeps mode B");
    assert_eq!(s.doc().unwrap().node(i).unwrap().var_modes.len(), 1);
}

/// A "Button" holding an instance of a swatch "Icon" on mode A, and a "Modal" holding a Button
/// instance whose icon copy was set to mode B: (the Button main, its icon layer, the copy in Modal).
fn modal(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    tone(s);
    let icon = swatch(s, "Icon", None);
    let frame = id(run(s, "frame.create", json!({"x": 100, "y": 0, "width": 60, "height": 40, "name": "Button"})));
    let button_icon = id(run(s, "instance.create", json!({"component": icon, "x": 10, "y": 10, "parent": frame})));
    set_mode(s, button_icon, json!("A"));
    let button = id(run(s, "component.create", json!({"ids": [frame]})));
    let outer = id(run(s, "frame.create", json!({"x": 300, "y": 0, "width": 200, "height": 100, "name": "Modal"})));
    let close = id(run(s, "instance.create", json!({"component": button, "x": 10, "y": 10, "parent": outer})));
    let nested = s.doc().unwrap().node(close).unwrap().children[0];
    set_mode(s, nested, json!("B"));
    run(s, "component.create", json!({"ids": [outer]}));
    (button, button_icon, nested)
}

#[test]
fn a_mode_set_on_a_layer_inside_an_instance_is_an_override() {
    let mut s = Session::new();
    let (button, button_icon, nested) = modal(&mut s);
    assert_eq!(fill(&s, nested), "0000FF");
    // An edit the copies can't take as they are rebuilds the Button instance.
    run(&mut s, "node.set", json!({"ids": [button], "layout": "horizontal"}));
    assert_eq!(fill(&s, nested), "0000FF", "a rebuild keeps the mode set on the copy");
    // The main's layer choosing another mode doesn't reach it either.
    set_mode(&mut s, button_icon, json!("B"));
    set_mode(&mut s, button_icon, Value::Null);
    assert_eq!(fill(&s, nested), "0000FF");
    run(&mut s, "component.resetOverride", json!({"id": nested, "group": "modes"}));
    assert_eq!(fill(&s, nested), "FF0000", "reset: the main's layer is on Auto (mode A)");
    assert!(s.doc().unwrap().node(nested).unwrap().var_modes.is_empty());
}

#[test]
fn a_mode_set_on_an_icon_inside_a_placed_instance_survives_main_edits() {
    let mut s = Session::new();
    let (button, _, _) = modal(&mut s);
    // A Button placed on the page, its icon recoloured through the icon's mode.
    let placed = instance(&mut s, button);
    let icon = s.doc().unwrap().node(placed).unwrap().children[0];
    set_mode(&mut s, icon, json!("B"));
    assert!(s.doc().unwrap().node(icon).unwrap().overrides.contains("modes"), "it shows as an override");
    run(&mut s, "node.set", json!({"ids": [button], "layout": "vertical"}));
    assert_eq!(fill(&s, icon), "0000FF", "a rebuild of the Button keeps it");
    run(&mut s, "component.resetInstance", json!({"ids": [placed]}));
    assert_eq!(fill(&s, icon), "FF0000", "reset all brings back the main's mode");
}

#[test]
fn a_file_from_before_modes_were_overrides_keeps_the_modes_chosen_inside_its_instances() {
    let mut s = Session::new();
    let (button, _, nested) = modal(&mut s);
    // Saved before `modes` was an override group: the copy has its mode but no marker.
    s.doc_mut().unwrap().node_mut(nested).unwrap().overrides.remove("modes");
    let path = std::env::temp_dir().join(format!("figcraft-instance-modes-{}.figcraft", std::process::id()));
    let p = path.to_string_lossy().into_owned();
    run(&mut s, "file.saveAs", json!({"path": p}));
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": p}));
    let _ = std::fs::remove_file(&path);
    let doc = s.doc().unwrap();
    assert!(doc.node(nested).unwrap().overrides.contains("modes"), "opening it marks the mode the copy chose");
    assert!(doc.nodes().filter(|n| n.id != nested).all(|n| !n.overrides.contains("modes")), "layers on their main's modes stay unmarked");
    run(&mut s, "node.set", json!({"ids": [button], "layout": "horizontal"}));
    assert_eq!(fill(&s, nested), "0000FF", "a rebuild keeps it");
}
