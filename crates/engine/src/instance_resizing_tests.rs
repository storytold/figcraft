//! Size and resizing overrides of layers inside instances: a layer in an instance can hug, fill or
//! keep a fixed size of its own, and that override is kept through relayout, edits of the main
//! (only the overridden property stays), undo, save and reopen (native and .fig), REST import,
//! detach and reset.

use base64::Engine;
use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{copy_of, ex, healthy, idv, node};
use figcraft_doc::*;

/// A "Field" component: a 240×40 fixed horizontal auto layout frame (no padding or gap) holding
/// "Atom", an auto layout frame hugging its 20×20 "Dot" with a min width of 100; and an instance of
/// it. Returns (component, atom, dot, instance).
fn field(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 240, "height": 40, "name": "Field"})));
    ex(s, "node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": 0, "gap": 0, "sizingH": "fixed", "sizingV": "fixed"}}));
    let atom = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 20, "parent": f, "name": "Atom"})));
    ex(
        s,
        "node.set",
        json!({"id": atom, "props": {"layout": "horizontal", "padding": 0, "gap": 0, "sizingH": "hug", "sizingV": "hug", "minWidth": 100}}),
    );
    let dot = idv(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20, "parent": atom, "props": {"name": "Dot"}})));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    let i = idv(&ex(s, "instance.create", json!({"component": c, "x": 0, "y": 100})));
    (c, atom, dot, i)
}

fn sizing_h(s: &Session, id: NodeId) -> Sizing {
    node(s, id).child_layout.sizing_h
}

/// The layer named `name` below instance `inst`.
fn named(s: &Session, inst: NodeId, name: &str) -> NodeId {
    let doc = s.doc().unwrap();
    doc.subtree(inst).into_iter().find(|d| *d != inst && doc.node(*d).is_some_and(|n| n.name == name)).unwrap()
}

/// The first instance in the document.
fn instance(s: &Session) -> NodeId {
    s.doc().unwrap().nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap().id
}

fn open(s: &mut Session, bytes: &[u8], name: &str) {
    ex(s, "file.open", json!({"data": base64::engine::general_purpose::STANDARD.encode(bytes), "name": name}));
}

#[test]
fn a_layer_in_an_instance_fills_and_keeps_it_through_main_edits() {
    let mut s = Session::new();
    let (c, atom, dot, i) = field(&mut s);
    let (ia, idot) = (copy_of(&s, i, atom), copy_of(&s, i, dot));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Hug, 100.0), "hugs its dot, at least 100 wide");
    ex(&mut s, "node.set", json!({"id": ia, "props": {"sizingH": "fill"}}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fill, 240.0), "fills the instance");
    assert!(node(&s, ia).overrides.contains("resizing"), "{:?}", node(&s, ia).overrides);
    // Edits of the main relay the instance out: the override stays, other properties follow.
    ex(&mut s, "node.set", json!({"id": dot, "props": {"width": 30, "fill": "#ff0000"}}));
    assert_eq!((node(&s, ia).width, node(&s, idot).width), (240.0, 30.0));
    ex(&mut s, "node.set", json!({"id": atom, "props": {"minWidth": 120, "fill": "#00ff00"}}));
    assert_eq!(node(&s, atom).width, 120.0);
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width, node(&s, ia).child_layout.min_w), (Sizing::Fill, 240.0, Some(100.0)));
    assert_eq!(node(&s, ia).fills, node(&s, atom).fills, "the fill isn't overridden: it follows the main");
    // The main fixing its own width is the same property: the instance keeps filling.
    ex(&mut s, "node.set", json!({"id": atom, "props": {"width": 80}}));
    assert_eq!((sizing_h(&s, atom), sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fixed, Sizing::Fill, 240.0));
    // A structure edit of the main rebuilds the instance: the atom fills what the new layer leaves.
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": c}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fill, 235.0));
    healthy(&s);
    // Back to the main's resizing: no override left.
    ex(&mut s, "component.resetOverride", json!({"id": ia, "group": "resizing"}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fixed, node(&s, atom).width));
    assert!(!node(&s, ia).overrides.contains("resizing"));
    healthy(&s);
}

#[test]
fn size_and_resizing_overrides_undo_reset_and_detach() {
    let mut s = Session::new();
    let (_, atom, dot, i) = field(&mut s);
    let (ia, idot) = (copy_of(&s, i, atom), copy_of(&s, i, dot));
    ex(&mut s, "node.set", json!({"id": ia, "props": {"sizingH": "fill"}}));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Hug, 100.0));
    assert!(node(&s, ia).overrides.is_empty());
    // A fixed size of its own: kept when the main resizes the layer.
    ex(&mut s, "node.set", json!({"id": idot, "props": {"width": 50, "height": 10}}));
    assert!(node(&s, idot).overrides.contains("size"));
    ex(&mut s, "node.set", json!({"id": dot, "props": {"width": 30}}));
    assert_eq!((node(&s, idot).width, node(&s, idot).height, node(&s, dot).width), (50.0, 10.0, 30.0));
    // A min width of its own.
    ex(&mut s, "node.set", json!({"id": ia, "props": {"minWidth": 150}}));
    assert_eq!(node(&s, ia).width, 150.0);
    ex(&mut s, "node.set", json!({"id": atom, "props": {"minWidth": 90}}));
    assert_eq!(node(&s, ia).width, 150.0);
    healthy(&s);
    // Reset all changes: the main's values again.
    ex(&mut s, "component.resetInstance", json!({"ids": [i]}));
    assert_eq!((node(&s, ia).width, node(&s, idot).width), (90.0, 30.0));
    assert!(node(&s, ia).overrides.is_empty() && node(&s, idot).overrides.is_empty());
    // Detaching keeps what the instance showed.
    ex(&mut s, "node.set", json!({"id": ia, "props": {"sizingH": "fill"}}));
    ex(&mut s, "component.detach", json!({"ids": [i]}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width, node(&s, ia).source), (Sizing::Fill, 240.0, None));
    healthy(&s);
}

#[test]
fn layers_in_instances_resize_but_keep_their_place() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let title = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let it = copy_of(&s, i, title);
    ex(&mut s, "node.set", json!({"id": it, "props": {"width": 150}}));
    assert_eq!(node(&s, it).width, 150.0, "a text box resizes");
    assert!(node(&s, it).overrides.contains("size"));
    // Its place, rotation and constraints are still the main's.
    for props in [json!({"x": 50}), json!({"rotation": 15}), json!({"constraints": {"horizontal": "right"}})] {
        assert!(s.execute("node.set", &json!({"id": it, "props": props})).is_err(), "{props} is refused");
    }
    // The instance's own resizing is its own, not an override.
    ex(&mut s, "node.set", json!({"id": i, "props": {"minWidth": 100}}));
    assert!(!node(&s, i).overrides.contains("resizing"));
    healthy(&s);
}

#[test]
fn resizing_overrides_survive_save_and_reopen() {
    let mut s = Session::new();
    let (_, atom, dot, i) = field(&mut s);
    let (ia, idot) = (copy_of(&s, i, atom), copy_of(&s, i, dot));
    ex(&mut s, "node.set", json!({"id": ia, "props": {"sizingH": "fill"}}));
    ex(&mut s, "node.set", json!({"id": idot, "props": {"width": 40}}));
    let native = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (fig, _) = figcraft_format::fig::export(s.doc().unwrap(), None).unwrap();
    for (bytes, name) in [(native, "field.figcraft"), (fig, "field.fig")] {
        let mut s = Session::new();
        open(&mut s, &bytes, name);
        let i = instance(&s);
        let (ia, idot) = (named(&s, i, "Atom"), named(&s, i, "Dot"));
        assert!(node(&s, ia).overrides.contains("resizing"), "{name}: {:?}", node(&s, ia).overrides);
        assert!(node(&s, idot).overrides.contains("size"), "{name}: {:?}", node(&s, idot).overrides);
        // An edit of the main relays the instance out.
        let main = |s: &Session, id: NodeId| node(s, id).source.unwrap();
        let (atom, dot) = (main(&s, ia), main(&s, idot));
        ex(&mut s, "node.set", json!({"id": atom, "props": {"minWidth": 120}}));
        ex(&mut s, "node.set", json!({"id": dot, "props": {"width": 30}}));
        assert_eq!((sizing_h(&s, ia), node(&s, ia).width, node(&s, idot).width), (Sizing::Fill, 240.0, 40.0), "{name}");
        healthy(&s);
    }
}

#[test]
fn imported_resizing_overrides_survive_relayout() {
    let at = |x: f64, y: f64| json!([[1, 0, x], [0, 1, y]]);
    let dot = |id: &str| json!({"id": id, "name": "Dot", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 20, "y": 20}});
    let atom = |id: &str, dot_id: &str, w: f64, more: Value| {
        let mut a = json!({"id": id, "name": "Atom", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": w, "y": 20},
            "layoutMode": "HORIZONTAL", "primaryAxisSizingMode": "AUTO", "counterAxisSizingMode": "AUTO", "minWidth": 100, "children": [dot(dot_id)]});
        a.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        a
    };
    let field = |id: &str, ty: &str, y: f64, kids: Value| {
        json!({"id": id, "name": "Field", "type": ty, "relativeTransform": at(0.0, y), "size": {"x": 240, "y": 40}, "layoutMode": "HORIZONTAL",
            "primaryAxisSizingMode": "FIXED", "counterAxisSizingMode": "FIXED", "children": kids})
    };
    let mut inst =
        field("2:1", "INSTANCE", 100.0, json!([atom("I2:1;1:2", "I2:1;1:3", 240.0, json!({"layoutSizingHorizontal": "FILL", "layoutGrow": 1}))]));
    inst["componentId"] = json!("1:1");
    inst["overrides"] = json!([{"id": "I2:1;1:2", "overriddenFields": ["layoutSizingHorizontal", "layoutGrow"]}]);
    let file = json!({"name": "Field", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page", "type": "CANVAS", "children": [field("1:1", "COMPONENT", 0.0, json!([atom("1:2", "1:3", 100.0, json!({}))])), inst]}
    ]}, "components": {"1:1": {"key": "k", "name": "Field", "description": ""}}, "styles": {}});
    let mut s = Session::new();
    open(&mut s, file.to_string().as_bytes(), "field.json");
    let i = instance(&s);
    let ia = named(&s, i, "Atom");
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fill, 240.0));
    assert!(node(&s, ia).overrides.contains("resizing"), "{:?}", node(&s, ia).overrides);
    // The main's first edit relays every instance out from it.
    let main_atom = node(&s, ia).source.unwrap();
    ex(&mut s, "node.set", json!({"id": main_atom, "props": {"minWidth": 120}}));
    assert_eq!((sizing_h(&s, ia), node(&s, ia).width), (Sizing::Fill, 240.0));
    healthy(&s);
}
