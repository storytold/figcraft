//! Variable bindings on paint and effect lists: a bound fill colour changes only the first fill,
//! effect bindings follow their effects when the list is reordered or shrinks, and removing the
//! first fill takes its binding with it.

use figcraft_color::{Color, PaintKind};
use figcraft_doc::{EffectKind, NodeId};
use serde_json::json;

use crate::Session;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ex(s: &mut Session, cmd: &str, p: serde_json::Value) {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
}

fn setup() -> (Session, NodeId) {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    ex(&mut s, "variable.createCollection", json!({"name": "T"}));
    for (name, kind, value) in [("brand", "color", json!("#FF0000")), ("small", "number", json!(2)), ("large", "number", json!(30))] {
        ex(&mut s, "variable.create", json!({"collection": "T", "name": name, "type": kind, "value": value}));
    }
    (s, r)
}

fn blur(s: &Session, r: NodeId, i: usize) -> f64 {
    match s.doc().unwrap().node(r).unwrap().effects.get(i).map(|e| &e.kind) {
        Some(EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b)) => b.radius,
        k => panic!("{k:?}"),
    }
}

fn bound(s: &Session, r: NodeId) -> Vec<(String, String)> {
    s.doc().unwrap().node(r).unwrap().bound.iter().map(|(k, v)| (k.clone(), v.to_string())).collect()
}

#[test]
fn a_bound_fill_colour_keeps_the_other_fills() {
    let (mut s, r) = setup();
    ex(&mut s, "fill.add", json!({"ids": [r], "color": "#00FF00"}));
    ex(&mut s, "fill.set", json!({"ids": [r], "index": 0, "color": "#0000FF", "opacity": 0.5}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    let fills = s.doc().unwrap().node(r).unwrap().fills.clone();
    assert_eq!(fills.len(), 2, "the second fill stays");
    assert_eq!(fills[0].kind, PaintKind::Solid { color: Color::from_rgb8_const(0xFF, 0, 0) });
    assert_eq!(fills[0].opacity, 0.5, "the bound paint keeps its opacity");
    assert_eq!(fills[1].kind, PaintKind::Solid { color: Color::from_rgb8_const(0, 0xFF, 0) });
    // Removing the bound (first) fill takes the binding with it.
    ex(&mut s, "fill.remove", json!({"ids": [r], "index": 0}));
    assert!(bound(&s, r).is_empty());
    ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#123456"}));
    let fills = s.doc().unwrap().node(r).unwrap().fills.clone();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].kind, PaintKind::Solid { color: Color::from_rgb8_const(0, 0xFF, 0) }, "the remaining fill isn't recoloured");
}

#[test]
fn effect_bindings_follow_their_effects() {
    let (mut s, r) = setup();
    for _ in 0..3 {
        ex(&mut s, "effect.add", json!({"ids": [r], "type": "layerBlur"}));
    }
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "effects/0/radius", "variable": "small"}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "effects/2/radius", "variable": "large"}));
    assert_eq!((blur(&s, r, 0), blur(&s, r, 2)), (2.0, 30.0));
    // Move the "small" effect to the top: its binding goes with it.
    ex(&mut s, "effect.move", json!({"ids": [r], "from": 0, "to": 2}));
    assert_eq!((blur(&s, r, 1), blur(&s, r, 2)), (30.0, 2.0));
    ex(&mut s, "variable.set", json!({"variable": "small", "value": 5}));
    assert_eq!((blur(&s, r, 1), blur(&s, r, 2)), (30.0, 5.0));
    // Removing the effect under the "large" one shifts its binding down.
    ex(&mut s, "effect.remove", json!({"ids": [r], "index": 0}));
    ex(&mut s, "variable.set", json!({"variable": "large", "value": 40}));
    assert_eq!((blur(&s, r, 0), blur(&s, r, 1)), (40.0, 5.0));
    // Removing a bound effect drops its binding.
    ex(&mut s, "effect.remove", json!({"ids": [r], "index": 1}));
    let keys: Vec<String> = bound(&s, r).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec!["effects/0/radius".to_string()]);
    // Undo restores the list and its bindings together.
    s.undo().unwrap();
    assert_eq!(bound(&s, r).len(), 2);
}

#[test]
fn effect_moves_and_removals_reject_junk() {
    let (mut s, r) = setup();
    ex(&mut s, "effect.add", json!({"ids": [r], "type": "layerBlur"}));
    for bad in [json!({"ids": [r], "from": -1, "to": 0}), json!({"ids": [r], "from": 0}), json!({"ids": [r], "from": "x", "to": 0})] {
        assert!(s.execute("effect.move", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("effect.remove", &json!({"ids": [r], "index": -1})).is_err());
    // Out-of-range indices change nothing.
    ex(&mut s, "effect.move", json!({"ids": [r], "from": 0, "to": 5}));
    ex(&mut s, "effect.remove", json!({"ids": [r], "index": 7}));
    assert_eq!(s.doc().unwrap().node(r).unwrap().effects.len(), 1);
}

fn fill_hex(s: &Session, r: NodeId) -> Vec<String> {
    s.doc().unwrap().node(r).unwrap().fills.iter().filter_map(|p| p.solid_color()).map(|c| c.to_hex()).collect()
}

fn brand_id(s: &Session) -> String {
    s.doc().unwrap().variables.get("brand").unwrap().id.clone()
}

#[test]
fn explicit_paint_edits_detach_the_colour_binding() {
    // An edit that reported success keeps its colour: the binding it overwrote lets go (as
    // selection colours already do), instead of putting the variable's colour back.
    let edits = [
        ("fill.set", json!({"color": "#00FF00"})),
        ("node.set", json!({"props": {"fill": "#00FF00"}})),
        ("node.set", json!({"props": {"fills": ["#00FF00"]}})),
    ];
    for (cmd, mut p) in edits {
        let (mut s, r) = setup();
        ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
        p["ids"] = json!([r]);
        ex(&mut s, cmd, p.clone());
        assert_eq!(fill_hex(&s, r), ["00FF00"], "{cmd} {p}");
        assert!(bound(&s, r).is_empty(), "{cmd} {p}");
        ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#123456"}));
        assert_eq!(fill_hex(&s, r), ["00FF00"], "{cmd} {p}: no longer follows the variable");
    }
    for (cmd, mut p) in [("stroke.set", json!({"color": "#00FF00"})), ("node.set", json!({"props": {"stroke": "#00FF00"}}))] {
        let (mut s, r) = setup();
        ex(&mut s, "node.set", json!({"ids": [r], "props": {"strokes": ["#000000"]}}));
        ex(&mut s, "variable.bind", json!({"ids": [r], "field": "stroke", "variable": "brand"}));
        p["ids"] = json!([r]);
        ex(&mut s, cmd, p);
        let n = s.doc().unwrap().node(r).unwrap();
        assert_eq!(n.strokes[0].solid_color().map(|c| c.to_hex()).as_deref(), Some("00FF00"), "{cmd}");
        assert!(n.bound.is_empty(), "{cmd}");
    }
}

#[test]
fn clearing_bound_fills_adds_no_paint_back() {
    let (mut s, r) = setup();
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"fills": []}}));
    assert!(s.doc().unwrap().node(r).unwrap().fills.is_empty());
    assert!(bound(&s, r).is_empty());
    ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#123456"}));
    assert!(s.doc().unwrap().node(r).unwrap().fills.is_empty(), "the variable doesn't bring the fill back");
}

#[test]
fn paint_opacity_edits_keep_the_colour_binding() {
    let (mut s, r) = setup();
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    ex(&mut s, "fill.set", json!({"ids": [r], "opacity": 0.5}));
    ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#0000FF"}));
    assert_eq!(fill_hex(&s, r), ["0000FF"], "the colour still follows the variable");
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills[0].opacity, 0.5);
    // The same colour with another opacity, through node.set, keeps it too.
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"fills": [{"type": "solid", "color": "#0000FF", "opacity": 0.25}]}}));
    assert_eq!(bound(&s, r).len(), 1);
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills[0].opacity, 0.25);
}

#[test]
fn paints_set_with_their_binding_in_the_same_call_keep_it() {
    let (mut s, r) = setup();
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    let brand = brand_id(&s);
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"fills": ["#00FF00"], "boundVariables": {"fill": brand}}}));
    assert_eq!(bound(&s, r), [("fill".to_string(), brand)]);
    assert_eq!(fill_hex(&s, r), ["FF0000"], "the binding given with the paints applies");
}
