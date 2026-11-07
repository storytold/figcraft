//! `node.set {path}` replaces a vector's geometry and keeps the layer (#1001): its id, name, place in
//! the layer list, paints, effects, styles, bindings and constraints. The path is read as
//! `node.create` reads it (`width`/`height` scale it), its own box goes at the layer's origin (or at
//! `x`/`y` plus the box's offset, as create places it), undo brings the old path back, and data that
//! can't draw, layers that aren't vectors and layers inside an instance are refused.

use figcraft_doc::{NodeId, NodeKind, VectorNetwork};
use serde_json::{Value, json};

use crate::{EngineError, Session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    crate::params::id_of(&v["id"]).unwrap()
}

fn vector(s: &mut Session, p: Value) -> NodeId {
    let mut p = p;
    p["type"] = json!("vector");
    id(&ex(s, "node.create", p))
}

fn net(s: &Session, v: NodeId) -> VectorNetwork {
    crate::vectoredit::network(s.doc().unwrap(), v).unwrap().clone()
}

/// The layer's x, y, width and height as `get_node` reports them.
fn frame(s: &Session, v: NodeId) -> (f64, f64, f64, f64) {
    let d = crate::cmd::props::describe(s.doc().unwrap(), v);
    let f = |k: &str| d[k].as_f64().unwrap();
    (f("x"), f("y"), f("width"), f("height"))
}

fn bad_msg(r: crate::Result<Value>) -> String {
    match r {
        Err(EngineError::BadParams { msg, .. }) => msg,
        other => panic!("expected BadParams, got {other:?}"),
    }
}

#[test]
fn a_new_path_keeps_the_layer() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "T"}));
    ex(&mut s, "variable.create", json!({"collection": "T", "name": "brand", "type": "color", "value": "#FF0000"}));
    let below = vector(&mut s, json!({"path": "M0 0 L5 5"}));
    let v = vector(&mut s, json!({"name": "Chart line", "path": "M0 0 L10 10", "x": 40, "y": 30}));
    let above = vector(&mut s, json!({"path": "M0 0 L5 5"}));
    ex(
        &mut s,
        "node.set",
        json!({"id": v, "props": {"stroke": "#00FF00", "strokeWeight": 3, "opacity": 0.5,
            "effects": [{"type": "dropShadow", "color": "#00000040", "offset": [0, 2], "radius": 4}],
            "constraints": {"horizontal": "right", "vertical": "bottom"}}}),
    );
    ex(&mut s, "variable.bind", json!({"ids": [v], "field": "fill", "variable": "brand"}));
    let before = s.doc().unwrap().node(v).unwrap().clone();

    let out = ex(&mut s, "node.set", json!({"ids": [v], "props": {"path": "M0 0 L20 0 L20 8 Z"}}));
    assert!(out.get("warnings").is_none(), "{out}");
    let doc = s.doc().unwrap();
    let n = doc.node(v).unwrap();
    assert_eq!((n.id, n.name.as_str()), (v, "Chart line"));
    assert_eq!((&n.fills, &n.strokes, &n.stroke, &n.effects), (&before.fills, &before.strokes, &before.stroke, &before.effects));
    assert_eq!((n.opacity, &n.bound, &n.styles, &n.constraints), (before.opacity, &before.bound, &before.styles, &before.constraints));
    assert!(n.bound.contains_key("fill"), "the fill stays bound");
    let page = doc.node(n.parent.unwrap()).unwrap();
    assert_eq!(page.children, [below, v, above], "same place in the layer list");
    // The new geometry, at the layer's origin.
    assert_eq!(frame(&s, v), (40.0, 30.0, 20.0, 8.0));
    let net = net(&s, v);
    assert_eq!((net.vertices.len(), net.regions.len()), (3, 1), "a closed triangle: {net:?}");
}

#[test]
fn width_height_and_x_y_follow_node_create() {
    // Each row: props given with the path, and the frame node.set gives a layer at (100, 50).
    let rows: [(Value, (f64, f64, f64, f64)); 6] = [
        // The path's own box goes at the layer's origin: its offset (5, 5) doesn't move it.
        (json!({}), (100.0, 50.0, 10.0, 20.0)),
        // Both sides scale independently; one keeps the proportion.
        (json!({"width": 30, "height": 10}), (100.0, 50.0, 30.0, 10.0)),
        (json!({"w": 20}), (100.0, 50.0, 20.0, 40.0)),
        // With x/y the path's offset is added, as create places it.
        (json!({"x": 0, "y": 0}), (5.0, 5.0, 10.0, 20.0)),
        (json!({"x": 10, "height": 10}), (15.0, 50.0, 5.0, 10.0)),
        (json!({"x": 10, "y": 20, "width": 20, "height": 10}), (15.0, 25.0, 20.0, 10.0)),
    ];
    for (extra, want) in rows {
        let mut s = Session::new();
        let v = vector(&mut s, json!({"path": "M0 0 L1 1", "x": 100, "y": 50}));
        let mut props = json!({"path": "M5 5 L15 5 L15 25 Z"});
        props.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        ex(&mut s, "node.set", json!({"ids": [v], "props": props}));
        assert_eq!(frame(&s, v), want, "{extra}");
        // With both x and y, create makes the same layer. (With one, the other axis keeps the
        // layer's current place.)
        if extra.get("x").is_some() && extra.get("y").is_some() {
            let c = vector(&mut s, props.clone());
            assert_eq!(frame(&s, c), want, "create {extra}");
        }
    }
}

#[test]
fn rotation_stays_and_turns_about_the_new_box() {
    let mut s = Session::new();
    let v = vector(&mut s, json!({"path": "M0 0 L10 10", "x": 0, "y": 0}));
    ex(&mut s, "node.set", json!({"ids": [v], "props": {"rotation": 90}}));
    let t = s.doc().unwrap().node(v).unwrap().transform;
    ex(&mut s, "node.set", json!({"ids": [v], "props": {"path": "M0 0 L40 0 L40 20 Z"}}));
    let n = s.doc().unwrap().node(v).unwrap();
    assert_eq!(n.transform, t, "the layer keeps its origin and rotation");
    assert_eq!((n.width, n.height), (40.0, 20.0));
}

#[test]
fn undo_brings_the_old_path_back() {
    let mut s = Session::new();
    let v = vector(&mut s, json!({"path": "M0 0 L10 10", "x": 3, "y": 4}));
    let (old_net, old_frame) = (net(&s, v), frame(&s, v));
    ex(&mut s, "node.set", json!({"ids": [v], "props": {"path": "M0 0 C10 0 20 10 20 20", "width": 50}}));
    let (new_net, new_frame) = (net(&s, v), frame(&s, v));
    assert_ne!(new_net, old_net);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!((net(&s, v), frame(&s, v)), (old_net, old_frame));
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!((net(&s, v), frame(&s, v)), (new_net, new_frame));
}

#[test]
fn data_that_cant_draw_is_refused_and_nothing_changes() {
    let mut s = Session::new();
    let v = vector(&mut s, json!({"path": "M0 0 L10 10", "name": "V"}));
    let before = s.doc().unwrap().node(v).unwrap().clone();
    for (props, says) in [
        (json!({"path": "not a path"}), "not valid SVG path data"),
        (json!({"path": "M0 0 L10"}), "not valid SVG path data"),
        (json!({"path": ""}), "draws nothing"),
        (json!({"path": "M5 5"}), "draws nothing"),
        (json!({"path": "M5 5 L5 5 Z"}), "draws nothing"),
        (json!({"path": "M0 0 L1e308 0 L-1e308 0"}), "too large"),
        (json!({"path": 5}), "must be SVG path data"),
        (json!({"path": null}), "must be SVG path data"),
        (json!({"path": "M0 0 L10 10", "width": 0}), "positive"),
        (json!({"path": "M0 0 L10 10", "height": -4}), "positive"),
        (json!({"path": "M0 0 L10 10", "width": "wide"}), "must be a number"),
        // A size on an axis the path has none of.
        (json!({"path": "M0 0 L10 0", "height": 5}), "no vertical extent"),
        (json!({"path": "M0 0 L10 10", "name": "renamed", "width": -1}), "positive"),
    ] {
        let msg = bad_msg(s.execute("node.set", &json!({"ids": [v], "props": props})));
        assert!(msg.contains(says), "{props}: {msg}");
        assert_eq!(s.doc().unwrap().node(v).unwrap(), &before, "{props} changed nothing");
    }
    // A horizontal line can still be given height 0, as create allows.
    ex(&mut s, "node.set", json!({"ids": [v], "props": {"path": "M0 0 L10 0", "height": 0}}));
    assert_eq!(frame(&s, v).2, 10.0);
}

#[test]
fn layers_that_arent_vectors_are_refused_alone_and_skipped_among_vectors() {
    let mut s = Session::new();
    let r = id(&ex(&mut s, "node.create", json!({"type": "rectangle", "name": "Box"})));
    let star = id(&ex(&mut s, "node.create", json!({"type": "star"})));
    let msg = bad_msg(s.execute("node.set", &json!({"ids": [r], "props": {"path": "M0 0 L5 5"}})));
    assert!(msg.contains("vector") && msg.contains("Box"), "{msg}");
    assert!(s.execute("node.set", &json!({"ids": [r, star], "props": {"path": "M0 0 L5 5"}})).is_err(), "no vector among them");
    assert!(matches!(s.doc().unwrap().node(r).unwrap().kind, NodeKind::Rectangle));

    let v = vector(&mut s, json!({"path": "M0 0 L10 10"}));
    let out = ex(&mut s, "node.set", json!({"ids": [r, v], "props": {"path": "M0 0 L30 0", "opacity": 0.5}}));
    let w = out["warnings"][0].as_str().unwrap();
    assert!(w.contains("`path` skipped on") && w.contains("Box") && w.contains("vector"), "{w}");
    let doc = s.doc().unwrap();
    assert_eq!((doc.node(r).unwrap().width, doc.node(r).unwrap().opacity), (100.0, 0.5), "the rectangle gets the other keys");
    assert_eq!(frame(&s, v).2, 30.0);
}

#[test]
fn a_vector_inside_an_instance_is_refused_and_its_main_reaches_it() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "width": 50, "height": 50})));
    let v = vector(&mut s, json!({"path": "M0 0 L10 10", "parent": f}));
    let comp = id(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": comp, "x": 200, "y": 0})));
    let copy = s.doc().unwrap().node(inst).unwrap().children[0];
    assert!(matches!(s.doc().unwrap().node(copy).unwrap().kind, NodeKind::Vector { .. }));
    let msg = bad_msg(s.execute("node.set", &json!({"ids": [copy], "props": {"path": "M0 0 L30 0"}})));
    assert!(msg.contains("inside an instance"), "{msg}");
    // The main's vector takes it, and the instance follows.
    let main_vector = s.doc().unwrap().node(comp).unwrap().children[0];
    assert_eq!(main_vector, v);
    ex(&mut s, "node.set", json!({"ids": [main_vector], "props": {"path": "M0 0 L30 0 L30 12 Z"}}));
    assert_eq!(net(&s, copy), net(&s, main_vector));
    assert_eq!((frame(&s, copy).2, frame(&s, copy).3), (30.0, 12.0));
}

#[test]
fn a_boolean_operand_takes_a_new_path_and_the_group_follows() {
    let mut s = Session::new();
    let a = vector(&mut s, json!({"path": "M0 0 L40 0 L40 40 L0 40 Z"}));
    let b = vector(&mut s, json!({"path": "M0 0 L40 0 L40 40 L0 40 Z", "x": 20, "y": 20}));
    let g = id(&ex(&mut s, "bool.union", json!({"ids": [a, b]})));
    assert_eq!(s.doc().unwrap().node(b).unwrap().parent, Some(g));
    ex(&mut s, "node.set", json!({"ids": [b], "props": {"path": "M0 0 L100 0 L100 100 L0 100 Z"}}));
    let bounds = s.doc().unwrap().world_bounds(g).unwrap();
    assert_eq!((bounds.width(), bounds.height()), (120.0, 120.0), "{bounds:?}");
}

#[test]
fn junk_paths_never_panic() {
    let long = "M0 0 ".to_string() + &"L1 1 ".repeat(5000);
    let junk = [
        json!("M"),
        json!("Z"),
        json!("M0 0 Z Z Z"),
        json!("M NaN 0 L 1 1"),
        json!("M inf 0 L 1 1"),
        json!("M0 0 L1e308 1e308"),
        json!("M-1e308 -1e308 L1e308 1e308"),
        json!("M0 0 A 1e308 1e308 0 1 1 10 10"),
        json!("M0 0 A 0 0 0 0 0 10 10"),
        json!("m0 0 l1 1 h-1e308 v1e308 z"),
        json!("M0 0 Q 5 5"),
        json!("M0,0L10,10\u{0}"),
        json!("\u{1F4A5}"),
        json!(long),
        json!([1, 2]),
        json!({"d": "M0 0 L1 1"}),
        json!(true),
    ];
    let sizes = [json!(null), json!(1e308), json!(-1e308), json!(1e-308), json!("NaN"), json!([1])];
    let mut s = Session::new();
    let v = vector(&mut s, json!({"path": "M0 0 L10 10"}));
    for path in &junk {
        for size in &sizes {
            for props in [json!({"path": path, "width": size}), json!({"path": path, "height": size, "x": size, "rotation": size})] {
                if let Err(EngineError::Internal { msg, .. }) = s.execute("node.set", &json!({"ids": [v], "props": props})) {
                    panic!("node.set panicked on {props}: {msg}");
                }
                let n = s.doc().unwrap().node(v).unwrap();
                assert!(n.width.is_finite() && n.height.is_finite() && n.transform.as_coeffs().iter().all(|c| c.is_finite()), "{props}");
            }
        }
    }
}

#[test]
fn a_new_path_while_editing_drops_points_that_are_gone() {
    let mut s = Session::new();
    let v = vector(&mut s, json!({"path": "M0 0 L10 0 L10 10 L0 10 Z"}));
    ex(&mut s, "vector.edit.begin", json!({"id": v}));
    ex(&mut s, "vector.edit.selectAll", json!({}));
    ex(&mut s, "node.set", json!({"ids": [v], "props": {"path": "M0 0 L20 0"}}));
    let ve = s.ds().unwrap().vector_edit.clone().expect("still editing");
    let n = net(&s, v);
    assert!(ve.vertices.iter().all(|i| *i < n.vertices.len()) && ve.segments.iter().all(|i| *i < n.segments.len()), "{ve:?}");
    ex(&mut s, "vector.edit.state", json!({}));
}
