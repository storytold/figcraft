//! Effects through the session: adding every kind by name, editing noise/texture/glass, scaling,
//! inspecting and generated CSS.

use serde_json::json;

use crate::Session;
use crate::codegen::{CodeLang, code_for};
use crate::params::id_of;
use figcraft_doc::{Effect, EffectKind, Glass, NoiseMode};

fn rect(s: &mut Session) -> figcraft_doc::NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 60})).unwrap()["id"]).unwrap()
}

#[test]
fn effect_add_takes_every_type() {
    let mut s = Session::new();
    let r = rect(&mut s);
    for (_, ty) in Effect::TYPES {
        s.execute("effect.add", &json!({"ids": [r], "type": ty})).unwrap();
    }
    s.execute("effect.add", &json!({"ids": [r], "type": "GLASS"})).unwrap();
    let n = s.doc().unwrap().node(r).unwrap();
    let labels: Vec<&str> = n.effects.iter().map(Effect::label).collect();
    assert_eq!(labels, vec!["Drop shadow", "Inner shadow", "Layer blur", "Background blur", "Noise", "Texture", "Glass", "Glass"]);
    let err = s.execute("effect.add", &json!({"ids": [r], "type": "sparkle"})).unwrap_err();
    assert!(err.to_string().contains("sparkle"), "{err}");
    // Undo removes one at a time.
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().effects.len(), 7);
}

#[test]
fn effect_set_edits_noise_texture_and_glass() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute(
        "effect.add",
        &json!({"ids": [r], "effect": {"type": "noise", "mode": "duo", "size": 2, "density": 0.4, "secondary": {"r": 1, "g": 1, "b": 1, "a": 1}}}),
    )
    .unwrap();
    s.execute("effect.add", &json!({"ids": [r], "effect": {"type": "texture", "radius": 6, "clipToShape": true}})).unwrap();
    s.execute("effect.set", &json!({"ids": [r], "index": 1, "effect": {"type": "glass", "refraction": 0.3, "frost": 10, "splay": 0.5}})).unwrap();
    let n = s.doc().unwrap().node(r).unwrap();
    let EffectKind::Noise(nz) = &n.effects[0].kind else { panic!("{:?}", n.effects) };
    assert_eq!((nz.mode, nz.size, nz.density), (NoiseMode::Duo, 2.0, 0.4));
    let EffectKind::Glass(g) = &n.effects[1].kind else { panic!("{:?}", n.effects) };
    assert_eq!((g.refraction, g.frost, g.splay, g.depth), (0.3, 10.0, 0.5, Glass::default().depth));
    // Junk is refused, not stored.
    assert!(s.execute("effect.set", &json!({"ids": [r], "index": 0, "effect": {"type": "noise", "mode": "rainbow"}})).is_err());
    assert!(s.execute("effect.set", &json!({"ids": [r], "index": 0, "effect": {"type": "glass", "depth": "deep"}})).is_err());
}

/// A key an effect doesn't have is refused, naming what it takes: serde dropped it without a word,
/// so a shadow given `offsetX`/`offsetY` (the names variable bindings use) kept the default offset.
#[test]
fn effects_refuse_keys_they_dont_have() {
    let mut s = Session::new();
    let r = rect(&mut s);
    let shadow = json!({"type": "dropShadow", "color": "#00000040", "offsetX": 0, "offsetY": 1, "radius": 3});
    let err = s.execute("node.set", &json!({"ids": [r], "props": {"effects": [shadow]}})).unwrap_err().to_string();
    assert!(err.contains("effects[0]") && err.contains("offsetX, offsetY") && err.contains("offset"), "{err}");
    assert!(s.doc().unwrap().node(r).unwrap().effects.is_empty(), "nothing stored");
    let err = s.execute("effect.add", &json!({"ids": [r], "effect": {"type": "layerBlur", "radious": 8}})).unwrap_err().to_string();
    assert!(err.contains("radious"), "{err}");
    s.execute("effect.add", &json!({"ids": [r], "type": "dropShadow"})).unwrap();
    let err = s.execute("effect.set", &json!({"ids": [r], "index": 0, "effect": {"type": "dropShadow", "spred": 2}})).unwrap_err().to_string();
    assert!(err.contains("spred"), "{err}");
    // Every kind's own keys still go in, camelCase or snake_case, and so does what inspect reports.
    for ty in ["innerShadow", "layerBlur", "backgroundBlur", "noise", "texture", "glass"] {
        s.execute("effect.add", &json!({"ids": [r], "type": ty})).unwrap();
    }
    let reported = crate::inspect::node_json(s.doc().unwrap(), r, 0)["effects"].clone();
    s.execute("node.set", &json!({"ids": [r], "props": {"effects": reported}})).unwrap();
    let camel = json!({"type": "dropShadow", "offset": [0, 1], "showBehind": true, "visible": true});
    let snake = json!({"type": "innerShadow", "offset": [0, 1], "show_behind": false});
    s.execute("node.set", &json!({"ids": [r], "props": {"effects": [camel, snake]}})).unwrap();
}

#[test]
fn scaling_scales_grain_and_glass() {
    let mut s = Session::new();
    let r = rect(&mut s);
    for ty in ["noise", "texture", "glass"] {
        s.execute("effect.add", &json!({"ids": [r], "type": ty})).unwrap();
    }
    s.execute("transform.scale", &json!({"ids": [r], "factor": 2})).unwrap();
    let n = s.doc().unwrap().node(r).unwrap();
    let EffectKind::Noise(nz) = &n.effects[0].kind else { panic!() };
    let EffectKind::Texture(t) = &n.effects[1].kind else { panic!() };
    let EffectKind::Glass(g) = &n.effects[2].kind else { panic!() };
    let d = Glass::default();
    assert_eq!((nz.size, t.size, t.radius, g.depth, g.frost), (1.0, 1.0, 8.0, d.depth * 2.0, d.frost * 2.0));
    // Unitless values stay.
    assert_eq!((g.refraction, nz.density), (d.refraction, 0.5));
}

#[test]
fn inspect_and_css_show_the_new_effects() {
    let mut s = Session::new();
    let r = rect(&mut s);
    for ty in ["noise", "texture", "glass"] {
        s.execute("effect.add", &json!({"ids": [r], "type": ty})).unwrap();
    }
    let doc = s.doc().unwrap();
    let sections = crate::devinspect::properties(doc, r);
    let fx = sections.iter().find(|s| s.title == "Effects").unwrap();
    let labels: Vec<&str> = fx.rows.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, vec!["Noise", "Texture", "Glass"]);
    assert!(fx.rows[2].value.contains("refraction 80%"), "{}", fx.rows[2].value);
    let css = code_for(doc, r, CodeLang::Css);
    assert!(css.contains("backdrop-filter: blur(2px);"), "{css}");
}

#[test]
fn negative_or_fractional_index_is_refused_not_item_zero() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("effect.add", &json!({"ids": [r], "type": "layerBlur"})).unwrap();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    let before = s.doc().unwrap().node(r).unwrap().clone();
    assert_eq!(before.exports.len(), 2);
    for index in [json!(-1), json!(-0.5), json!(1.5)] {
        let set = json!({"ids": [r], "index": index, "effect": {"type": "dropShadow"}});
        assert!(s.execute("effect.set", &set).is_err(), "effect.set index {index}");
        let upd = json!({"ids": [r], "index": index, "format": "svg"});
        assert!(s.execute("export.update", &upd).is_err(), "export.update index {index}");
        assert!(s.execute("export.remove", &json!({"ids": [r], "index": index})).is_err(), "export.remove index {index}");
        let n = s.doc().unwrap().node(r).unwrap();
        assert_eq!(n.effects, before.effects, "index {index}");
        assert_eq!(n.exports, before.exports, "index {index}");
    }
}
