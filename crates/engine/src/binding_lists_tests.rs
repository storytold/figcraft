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
    assert_eq!(fills[0].opacity, 1.0, "the variable brings the whole colour, alpha included");
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
