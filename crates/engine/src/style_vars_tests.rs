//! Variables bound inside shared styles: `style.bind`, what each style type takes, a layer using
//! the style showing it resolved in its own modes (and staying linked), the style's own value in
//! the default modes, variables and modes changing afterwards, styles made from bound layers,
//! undo, the REST and `.fig` mappings, junk.

use serde_json::{Value, json};

use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::{Node, NodeId, StyleKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn node(s: &Session, n: NodeId) -> Node {
    s.doc().unwrap().node(n).unwrap().clone()
}

fn fill(s: &Session, n: NodeId) -> String {
    node(s, n).fills[0].solid_color().unwrap().to_css_hex().to_uppercase()
}

fn style_fill(s: &Session, sid: &str) -> String {
    match &s.doc().unwrap().style(sid).unwrap().kind {
        StyleKind::Paint { paints } => paints[0].solid_color().unwrap().to_css_hex().to_uppercase(),
        k => panic!("{k:?}"),
    }
}

/// A Theme collection (Light, Dark) with a colour `bg` (white, black) and numbers `size` (16, 20)
/// and `blur` (4, 12).
fn theme(s: &mut Session) {
    run(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "bg", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#000000"}}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "size", "type": "number", "values": {"Light": 16, "Dark": 20}}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "blur", "type": "number", "values": {"Light": 4, "Dark": 12}}));
}

/// Two frames (the second set to Dark), a red rectangle in each, and a paint style "Surface"
/// (red) both rectangles use.
fn two_modes(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId, String) {
    theme(s);
    let frame = |s: &mut Session, x: f64| id(run(s, "frame.create", json!({"x": x, "y": 0, "width": 100, "height": 100})));
    let (light, dark) = (frame(s, 0.0), frame(s, 200.0));
    run(s, "variable.setMode", json!({"ids": [dark], "collection": "Theme", "mode": "Dark"}));
    let rect = |s: &mut Session, f: NodeId| {
        id(run(s, "shape.rectangle", json!({"parent": f, "x": 10, "y": 10, "width": 40, "height": 40, "props": {"fill": "#FF0000"}})))
    };
    let (a, b) = (rect(s, light), rect(s, dark));
    let sid = run(s, "style.create", json!({"type": "paint", "name": "Surface", "from": a}))["id"].as_str().unwrap().to_string();
    run(s, "style.apply", json!({"ids": [b], "style": sid}));
    (light, dark, a, b, sid)
}

#[test]
fn a_bound_paint_style_shows_in_each_layers_mode() {
    let mut s = Session::new();
    let (_, dark, a, b, sid) = two_modes(&mut s);
    run(&mut s, "style.bind", json!({"style": sid, "field": "fill", "variable": "bg"}));
    // The layer in the Light frame is white, the one in the Dark frame black; both stay linked.
    assert_eq!((fill(&s, a), fill(&s, b)), ("#FFFFFF".to_string(), "#000000".to_string()));
    assert_eq!((node(&s, a).styles.fill, node(&s, b).styles.fill), (Some(sid.clone()), Some(sid.clone())));
    // The style itself holds the default mode's value, under the one spelling of the key.
    assert_eq!(style_fill(&s, &sid), "#FFFFFF");
    let st = s.doc().unwrap().style(&sid).unwrap().clone();
    assert_eq!(st.bound.keys().collect::<Vec<_>>(), ["fills/0/color"]);
    // The variable's value changes: every user follows, in its mode.
    run(&mut s, "variable.set", json!({"variable": "bg", "mode": "Dark", "value": "#112233"}));
    assert_eq!((fill(&s, a), fill(&s, b)), ("#FFFFFF".to_string(), "#112233".to_string()));
    // A frame's mode changes: the layers in it follow, still linked.
    run(&mut s, "variable.setMode", json!({"ids": [dark], "collection": "Theme", "mode": "Light"}));
    assert_eq!((fill(&s, b), node(&s, b).styles.fill), ("#FFFFFF".to_string(), Some(sid.clone())));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill(&s, b), "#112233");
    // A layer moved into the other frame takes that frame's mode.
    run(&mut s, "node.reparent", json!({"id": a, "parent": dark}));
    assert_eq!((fill(&s, a), node(&s, a).styles.fill), ("#112233".to_string(), Some(sid.clone())));
    // Unbound: the style and its users keep the value they showed last by default.
    run(&mut s, "style.bind", json!({"style": sid, "field": "fills/0/color"}));
    assert!(s.doc().unwrap().style(&sid).unwrap().bound.is_empty());
    assert_eq!((style_fill(&s, &sid), fill(&s, a), fill(&s, b)), ("#FFFFFF".to_string(), "#FFFFFF".to_string(), "#FFFFFF".to_string()));
}

#[test]
fn editing_a_bound_style_or_its_users() {
    let mut s = Session::new();
    let (_, _, a, b, sid) = two_modes(&mut s);
    run(&mut s, "style.bind", json!({"style": sid, "field": "fill", "variable": "bg"}));
    // An edit of the layer's own fill still unlinks it (the documented rule for styled properties).
    run(&mut s, "node.set", json!({"id": b, "props": {"fill": "#00FF00"}}));
    assert_eq!((fill(&s, b), node(&s, b).styles.fill), ("#00FF00".to_string(), None));
    // Applying the style again gives the layer's mode's value, not the stored one.
    run(&mut s, "style.apply", json!({"ids": [b], "style": sid}));
    assert_eq!(fill(&s, b), "#000000");
    // A new value without the bound paint drops the binding with it.
    run(&mut s, "style.update", json!({"style": sid, "value": []}));
    assert!(s.doc().unwrap().style(&sid).unwrap().bound.is_empty());
    assert!(node(&s, a).fills.is_empty() && node(&s, a).styles.fill == Some(sid));
}

#[test]
fn effect_grid_and_text_styles_take_variables_too() {
    let mut s = Session::new();
    let (_, dark, a, b, _) = two_modes(&mut s);
    // Effect: a layer blur's radius.
    run(&mut s, "effect.add", json!({"ids": [a], "type": "layerBlur"}));
    let fx = run(&mut s, "style.create", json!({"type": "effect", "name": "Soft", "from": a}))["id"].as_str().unwrap().to_string();
    run(&mut s, "style.apply", json!({"ids": [b], "style": fx}));
    run(&mut s, "style.bind", json!({"style": fx, "field": "effects/0/radius", "variable": "blur"}));
    let radius = |s: &Session, n: NodeId| node(s, n).effects[0].blur().unwrap().radius;
    assert_eq!((radius(&s, a), radius(&s, b)), (4.0, 12.0));
    assert_eq!((node(&s, a).styles.effect.is_some(), node(&s, b).styles.effect.is_some()), (true, true));
    // Grid: the frames' layout grid size.
    let light = node(&s, a).parent.unwrap();
    run(&mut s, "grid.add", json!({"ids": [light]}));
    let grid = run(&mut s, "style.create", json!({"type": "grid", "name": "Base", "from": light}))["id"].as_str().unwrap().to_string();
    run(&mut s, "style.apply", json!({"ids": [dark], "style": grid}));
    run(&mut s, "style.bind", json!({"style": grid, "field": "layoutGrids/0/size", "variable": "size"}));
    assert_eq!((node(&s, light).grids[0].size, node(&s, dark).grids[0].size), (16.0, 20.0));
    // Text: the font size, for chars linked to the style.
    let text = |s: &mut Session, f: NodeId| id(run(s, "text.create", json!({"text": "Hello", "x": 0, "y": 60, "parent": f})));
    let (t1, t2) = (text(&mut s, light), text(&mut s, dark));
    let body = run(&mut s, "style.create", json!({"type": "text", "name": "Body", "from": t1}))["id"].as_str().unwrap().to_string();
    run(&mut s, "style.apply", json!({"ids": [t2], "style": body}));
    run(&mut s, "style.bind", json!({"style": body, "field": "fontSize", "variable": "size"}));
    let size = |s: &Session, n: NodeId| node(s, n).kind.text().unwrap().style.font_size;
    assert_eq!((size(&s, t1), size(&s, t2)), (16.0, 20.0));
    assert_eq!((node(&s, t1).styles.text.clone(), node(&s, t2).styles.text.clone()), (Some(body.clone()), Some(body.clone())));
    run(&mut s, "variable.set", json!({"variable": "size", "mode": "Dark", "value": 28}));
    assert_eq!((size(&s, t2), node(&s, t2).styles.text.clone(), node(&s, dark).grids[0].size), (28.0, Some(body), 28.0));
}

#[test]
fn a_style_made_from_a_bound_layer_keeps_the_binding() {
    let mut s = Session::new();
    theme(&mut s);
    let r =
        id(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40, "props": {"fill": "#FF0000", "strokes": ["#00FF00"]}})));
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "stroke", "variable": "bg"}));
    let sid = run(&mut s, "style.create", json!({"type": "stroke", "name": "Edge", "from": r}))["id"].as_str().unwrap().to_string();
    let st = s.doc().unwrap().style(&sid).unwrap().clone();
    assert_eq!(st.bound.keys().collect::<Vec<_>>(), ["fills/0/color"], "a paint style's paints are its fills");
    // The layer's own binding gave way to the style's.
    let n = node(&s, r);
    assert!(n.bound.is_empty() && n.styles.stroke == Some(sid) && n.strokes[0].solid_color().unwrap().to_css_hex().to_uppercase() == "#FFFFFF");
}

#[test]
fn what_a_style_cannot_bind_is_refused() {
    let mut s = Session::new();
    let (_, _, a, _, sid) = two_modes(&mut s);
    // (field, variable, what the message says)
    for (field, var, says) in [
        ("fill", "size", "takes a color variable"),
        ("fills/3/color", "bg", "no field"),
        ("effects/0/radius", "blur", "no field"),
        ("fontSize", "size", "no field"),
        ("opacity", "size", "no field"),
        ("fill", "nope", "no such variable"),
    ] {
        let e = s.execute("style.bind", &json!({"style": sid, "field": field, "variable": var})).unwrap_err().to_string();
        assert!(e.contains(says), "{field} ← {var}: {e}");
    }
    assert!(s.execute("style.bind", &json!({"style": "nope", "field": "fill", "variable": "bg"})).is_err());
    assert!(s.doc().unwrap().style(&sid).unwrap().bound.is_empty());
    assert_eq!(fill(&s, a), "#FF0000");
    // Junk never takes the engine down.
    for j in [json!(null), json!(true), json!(-1), json!([1]), json!({"a": 1}), json!("x".repeat(5000)), json!("fills/99999999999999999999/color")] {
        for p in [
            json!({"style": j, "field": "fill"}),
            json!({"style": sid, "field": j, "variable": "bg"}),
            json!({"style": sid, "field": "fill", "variable": j}),
        ] {
            let r = s.execute("style.bind", &p);
            assert!(!matches!(r, Err(EngineError::Internal { .. })), "{p}: {r:?}");
        }
    }
}

#[test]
fn style_fields_offer_their_properties_scopes() {
    let mut s = Session::new();
    let (_, _, a, _, sid) = two_modes(&mut s);
    run(&mut s, "effect.add", json!({"ids": [a], "type": "dropShadow"}));
    let fx = run(&mut s, "style.create", json!({"type": "effect", "name": "Drop", "from": a}))["id"].as_str().unwrap().to_string();
    let light = node(&s, a).parent.unwrap();
    run(&mut s, "grid.add", json!({"ids": [light]}));
    let grid = run(&mut s, "style.create", json!({"type": "grid", "name": "Base", "from": light}))["id"].as_str().unwrap().to_string();
    let body = run(&mut s, "style.create", json!({"type": "text", "name": "Body", "value": {"fontFamily": "Inter", "fontSize": 14}}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let doc = s.doc().unwrap();
    let scopes = |st: &str, key: &str| crate::style_vars::field_scopes(&doc.style(st).unwrap().kind, key);
    let fills: &[&str] = &["FRAME_FILL", "SHAPE_FILL", "TEXT_FILL", "STROKE_COLOR"];
    for (st, key, want) in [
        (&sid, "fill", Some(fills)),
        (&sid, "fills/0/color", Some(fills)),
        (&fx, "effects/0/color", Some(&["EFFECT_COLOR"][..])),
        (&fx, "effects/0/radius", Some(&["EFFECT_FLOAT"][..])),
        (&fx, "effects/0/offsetX", Some(&["EFFECT_FLOAT"][..])),
        (&body, "fontSize", Some(&["FONT_SIZE"][..])),
        (&body, "lineHeight", Some(&["LINE_HEIGHT"][..])),
        (&body, "letterSpacing", Some(&["LETTER_SPACING"][..])),
        (&body, "fontFamily", Some(&["FONT_FAMILY"][..])),
        (&body, "fontStyle", Some(&["FONT_STYLE"][..])),
        // Grid fields take any number variable, as on layers; fields a style lacks take none.
        (&grid, "layoutGrids/0/size", None),
        (&sid, "fills/3/color", None),
        (&body, "opacity", None),
    ] {
        assert_eq!(scopes(st, key), want, "{st} {key}");
    }
    // As on layers, scopes limit pickers only: an out-of-scope variable still binds.
    run(&mut s, "variable.update", json!({"variable": "size", "scopes": ["CORNER_RADIUS"]}));
    run(&mut s, "style.bind", json!({"style": body, "field": "fontSize", "variable": "size"}));
    assert!(s.doc().unwrap().style(&body).unwrap().bound.contains_key("fontSize"));
}

#[test]
fn rest_and_fig_carry_style_bindings() {
    let mut s = Session::new();
    let (_, _, a, b, sid) = two_modes(&mut s);
    run(&mut s, "style.bind", json!({"style": sid, "field": "fill", "variable": "bg"}));
    let doc = s.doc().unwrap().clone();
    let (bytes, _) = figcraft_format::fig::export(&doc, None).unwrap();
    let (mut back, warnings) = figcraft_format::fig::import(&bytes, "x").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    // The file gives a styled layer its style's value; opening it resolves that per layer.
    crate::style_vars::after_load(&mut back);
    let st = back.styles.iter().find(|x| x.name == "Surface").unwrap();
    assert_eq!(st.bound.len(), 1, "{st:?}");
    assert!(st.bound.contains_key("fills/0/color"));
    // The layers using it carry no binding of their own on what the style sets, and still show
    // their modes' values.
    let named = |n: NodeId| back.nodes().find(|x| x.name == doc.node(n).unwrap().name && x.fills == doc.node(n).unwrap().fills).unwrap();
    for n in [a, b] {
        let got = named(n);
        assert!(got.bound.is_empty() && got.styles.fill.as_deref() == Some(st.id.as_str()), "{got:?}");
    }
}
