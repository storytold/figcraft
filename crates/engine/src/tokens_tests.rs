//! Design tokens (DTCG 2025.10) through the `token.*` commands: sets and modifiers, tokens of the
//! format's types, contexts for the file and for frames, bindings that follow the context on the
//! canvas, renames that carry aliases and bindings along, and resolver and token file round trips.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{EffectKind, LineHeight, NodeId, NodeKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn fill(s: &Session, n: NodeId) -> String {
    s.doc().unwrap().node(n).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

/// A core set (blue, white, a spacing), a theme modifier (light/dark) whose `bg` aliases the core
/// colours.
fn themed(s: &mut Session) {
    run(s, "token.createSet", json!({"name": "core"}));
    run(s, "token.set", json!({"set": "core", "path": "color.blue", "type": "color", "value": "#0000ff"}));
    run(s, "token.set", json!({"set": "core", "path": "color.white", "type": "color", "value": "#ffffff"}));
    run(s, "token.set", json!({"set": "core", "path": "space.md", "type": "dimension", "value": "1rem"}));
    run(s, "token.createModifier", json!({"name": "theme", "contexts": ["light", "dark"]}));
    run(s, "token.set", json!({"modifier": "theme", "context": "light", "path": "bg", "type": "color", "value": "{color.white}"}));
    run(s, "token.set", json!({"modifier": "theme", "context": "dark", "path": "bg", "type": "color", "value": "{color.blue}"}));
}

#[test]
fn bound_layers_follow_the_files_context_and_their_frames() {
    let mut s = Session::new();
    themed(&mut s);
    let f = id(run(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let r = id(run(&mut s, "shape.rectangle", json!({"parent": f, "x": 0, "y": 0, "width": 10, "height": 10})));
    run(&mut s, "token.bind", json!({"ids": [r], "field": "fills/0/color", "token": "bg"}));
    run(&mut s, "token.bind", json!({"ids": [r], "field": "cornerRadius", "token": "space.md"}));
    assert_eq!(fill(&s, r), "FFFFFF", "light is the default");
    assert_eq!(s.doc().unwrap().node(r).unwrap().corners.radii, [16.0; 4], "1rem = 16px");
    run(&mut s, "token.setContext", json!({"modifier": "theme", "context": "dark"}));
    assert_eq!(fill(&s, r), "0000FF");
    // A frame picks its own context; clearing it inherits again.
    run(&mut s, "token.setContext", json!({"ids": [f], "modifier": "theme", "context": "light"}));
    assert_eq!(fill(&s, r), "FFFFFF");
    run(&mut s, "token.setContext", json!({"ids": [f], "modifier": "theme", "context": null}));
    assert_eq!(fill(&s, r), "0000FF");
    // Editing a token updates what's bound to it; undo puts it back.
    run(&mut s, "token.set", json!({"set": "core", "path": "color.blue", "value": "#112233"}));
    assert_eq!(fill(&s, r), "112233");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill(&s, r), "0000FF");
    // Types the field doesn't take are refused.
    assert!(s.execute("token.bind", &json!({"ids": [r], "field": "cornerRadius", "token": "bg"})).is_err());
    assert!(s.execute("token.bind", &json!({"ids": [r], "field": "fills/0/color", "token": "nope"})).is_err());
    run(&mut s, "token.bind", json!({"ids": [r], "field": "fills/0/color", "token": null}));
    assert!(!s.doc().unwrap().node(r).unwrap().token_bound.contains_key("fills/0/color"));
}

#[test]
fn composite_tokens_fill_typography_shadows_and_borders() {
    let mut s = Session::new();
    run(&mut s, "token.createSet", json!({"name": "base"}));
    run(&mut s, "token.set", json!({"set": "base", "path": "font.body", "type": "fontFamily", "value": ["Inter", "sans-serif"]}));
    run(
        &mut s,
        "token.set",
        json!({"set": "base", "path": "type.body", "type": "typography", "value": {
        "fontFamily": "{font.body}", "fontSize": {"value": 20, "unit": "px"}, "fontWeight": "bold",
        "letterSpacing": {"value": 1, "unit": "px"}, "lineHeight": 1.5}}),
    );
    run(
        &mut s,
        "token.set",
        json!({"set": "base", "path": "shadow.card", "type": "shadow", "value": [
        {"color": "#00000040", "offsetX": {"value": 0, "unit": "px"}, "offsetY": {"value": 2, "unit": "px"}, "blur": {"value": 8, "unit": "px"}, "spread": {"value": 0, "unit": "px"}},
        {"color": "#00000020", "offsetX": {"value": 0, "unit": "px"}, "offsetY": {"value": 0, "unit": "px"}, "blur": {"value": 1, "unit": "px"}, "spread": {"value": 0, "unit": "px"}, "inset": true}]}),
    );
    run(
        &mut s,
        "token.set",
        json!({"set": "base", "path": "border.focus", "type": "border", "value": {"color": "#ff0000", "width": {"value": 2, "unit": "px"}, "style": "dashed"}}),
    );
    let t = id(run(&mut s, "text.create", json!({"text": "Hello"})));
    run(&mut s, "token.bind", json!({"ids": [t], "field": "typography", "token": "type.body"}));
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.node(t).unwrap().kind else { panic!("text") };
    assert_eq!(td.style.font_size, 20.0);
    assert_eq!(td.style.font_family, "Inter");
    assert_eq!(td.style.letter_spacing, figcraft_doc::LetterSpacing::Pixels(1.0));
    assert_eq!(td.style.line_height, LineHeight::Percent(150.0));
    assert_eq!(td.style.weight(), 700);
    let r = id(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    run(&mut s, "token.bind", json!({"ids": [r], "field": "shadow", "token": "shadow.card"}));
    run(&mut s, "token.bind", json!({"ids": [r], "field": "border", "token": "border.focus"}));
    let n = s.doc().unwrap().node(r).unwrap().clone();
    assert!(matches!(&n.effects[0].kind, EffectKind::DropShadow(sh) if sh.radius == 8.0 && sh.offset == (0.0, 2.0)));
    assert!(matches!(&n.effects[1].kind, EffectKind::InnerShadow(_)));
    assert_eq!(n.strokes[0].solid_color().unwrap().to_hex(), "FF0000");
    assert_eq!(n.stroke.weight, 2.0);
    assert_eq!(n.stroke.dashes, vec![6.0, 4.0]);
    // A typography token doesn't fit a rectangle.
    assert!(s.execute("token.bind", &json!({"ids": [r], "field": "typography", "token": "type.body"})).is_err());
}

#[test]
fn values_are_checked_and_conveniences_converted() {
    let mut s = Session::new();
    run(&mut s, "token.createSet", json!({"name": "s"}));
    assert!(s.execute("token.set", &json!({"set": "s", "path": "a", "type": "dimension", "value": {"value": 1, "unit": "pt"}})).is_err());
    assert!(s.execute("token.set", &json!({"set": "s", "path": "b", "type": "color", "value": "not a colour"})).is_err());
    assert!(s.execute("token.set", &json!({"set": "s", "path": "c.$type", "type": "number", "value": 1})).is_err());
    assert!(s.execute("token.set", &json!({"set": "s", "path": "x", "value": 1})).is_err(), "no type and no group type");
    run(&mut s, "token.set", json!({"set": "s", "path": "d", "type": "duration", "value": 150}));
    run(&mut s, "token.set", json!({"set": "s", "path": "e", "type": "cubicBezier", "value": [0.4, 0, 0.2, 1]}));
    let t = &s.doc().unwrap().tokens.sets[0].tokens;
    assert_eq!(t["d"]["$value"], json!({"value": 150.0, "unit": "ms"}));
    // Changing a token's type starts it at that type's default value.
    run(&mut s, "token.set", json!({"set": "s", "path": "e", "type": "number"}));
    assert_eq!(s.doc().unwrap().tokens.sets[0].tokens["e"]["$value"], json!(0));
    // Description and deprecation.
    run(&mut s, "token.set", json!({"set": "s", "path": "e", "description": "Old", "deprecated": "Use d"}));
    let e = &s.doc().unwrap().tokens.sets[0].tokens["e"];
    assert_eq!((e["$description"].as_str(), e["$deprecated"].as_str()), (Some("Old"), Some("Use d")));
}

#[test]
fn renames_carry_aliases_and_bindings_along() {
    let mut s = Session::new();
    themed(&mut s);
    let r = id(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    run(&mut s, "token.bind", json!({"ids": [r], "field": "fills/0/color", "token": "color.white"}));
    let out = run(&mut s, "token.rename", json!({"set": "core", "path": "color", "to": "palette"}));
    assert_eq!(out["aliases"], 2);
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(r).unwrap().token_bound["fills/0/color"], "palette.white");
    let r2 = doc.tokens.resolve(&doc.tokens.input_for(&Default::default()));
    assert!(r2.errors.is_empty(), "{:?}", r2.errors);
    assert!(s.execute("token.rename", &json!({"set": "core", "path": "palette", "to": "palette.inner"})).is_err());
    assert!(s.execute("token.rename", &json!({"set": "core", "path": "palette.blue", "to": "space.md"})).is_err(), "taken");
    // Renaming a modifier and a context keeps the file's choice and frames' choices.
    run(&mut s, "token.setContext", json!({"modifier": "theme", "context": "dark"}));
    run(&mut s, "token.renameModifier", json!({"modifier": "theme", "name": "mode"}));
    run(&mut s, "token.renameContext", json!({"modifier": "mode", "context": "dark", "name": "night"}));
    assert_eq!(s.doc().unwrap().tokens.active["mode"], "night");
    assert!(s.execute("token.removeContext", &json!({"modifier": "mode", "context": "light"})).is_ok());
    assert!(s.execute("token.removeContext", &json!({"modifier": "mode", "context": "night"})).is_err(), "keeps one");
}

#[test]
fn resolver_documents_and_token_files_round_trip() {
    let mut s = Session::new();
    themed(&mut s);
    run(&mut s, "token.moveOrder", json!({"modifier": "theme", "to": 0}));
    let text = run(&mut s, "token.export", json!({}))["text"].as_str().unwrap().to_string();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["version"], "2025.10");
    assert_eq!(doc["resolutionOrder"][0], json!({"$ref": "#/modifiers/theme"}));
    // Token order is kept as written.
    let keys: Vec<&String> = doc["sets"]["core"]["sources"][0]["color"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["blue", "white"]);
    let mut t = Session::new();
    let r = run(&mut t, "token.import", json!({"tokens": text}));
    assert_eq!((r["sets"].as_u64(), r["modifiers"].as_u64()), (Some(1), Some(1)));
    assert_eq!(t.doc().unwrap().tokens.sets, s.doc().unwrap().tokens.sets);
    assert_eq!(t.doc().unwrap().tokens.modifiers, s.doc().unwrap().tokens.modifiers);
    // A token file for the dark context, aliases resolved.
    let out = run(&mut s, "token.export", json!({"format": "tokens", "input": {"theme": "dark"}, "resolveAliases": true}));
    let file: Value = serde_json::from_str(out["text"].as_str().unwrap()).unwrap();
    assert_eq!(file["bg"]["$value"]["hex"], "#0000ff");
    assert!(s.execute("token.export", &json!({"format": "tokens", "input": {"theme": "dusk"}})).is_err());
    // A plain token file goes into a set (of the given name), joining one that's there.
    let r = run(
        &mut s,
        "token.import",
        json!({"tokens": {"radius": {"$type": "dimension", "sm": {"$value": {"value": 2, "unit": "px"}}}}, "set": "core"}),
    );
    assert_eq!(r["sets"], 0);
    assert!(s.doc().unwrap().tokens.set("core").unwrap().tokens["radius"].is_object());
    assert!(s.doc().unwrap().tokens.set("core").unwrap().tokens["color"].is_object(), "joined, not replaced");
    assert!(s.execute("token.import", &json!({"tokens": "[1, 2]"})).is_err());
}

#[test]
fn the_preference_switches_between_variables_and_tokens() {
    let mut s = Session::new();
    assert!(!s.prefs.design_tokens);
    assert_eq!(run(&mut s, "pref.designTokens", json!({}))["on"], true);
    assert_eq!(run(&mut s, "pref.designTokens", json!({"on": false}))["on"], false);
}

#[test]
fn zip_bundles_export_and_import_with_a_file_per_set_and_context() {
    use base64::Engine as _;
    let mut s = Session::new();
    themed(&mut s);
    let data = run(&mut s, "token.export", json!({"format": "zip"}))["data"].as_str().unwrap().to_string();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&data).unwrap();
    assert!(figcraft_format::tokens_bundle::is_zip(&bytes));
    // Imported into another file: the same sets, modifiers and order.
    let mut t = Session::new();
    let r = run(&mut t, "token.import", json!({"data": data}));
    assert_eq!((r["sets"].as_u64(), r["modifiers"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(t.doc().unwrap().tokens.sets, s.doc().unwrap().tokens.sets);
    assert_eq!(t.doc().unwrap().tokens.modifiers, s.doc().unwrap().tokens.modifiers);
    // Each file of the bundle also imports on its own: a context's file into that context.
    let mut u = Session::new();
    run(&mut u, "token.createModifier", json!({"name": "theme", "contexts": ["light", "dark"]}));
    let dark = s.doc().unwrap().tokens.modifier("theme").unwrap().context("dark").unwrap().tokens.clone();
    run(&mut u, "token.import", json!({"tokens": dark, "modifier": "theme", "context": "dark"}));
    assert!(u.doc().unwrap().tokens.modifier("theme").unwrap().context("dark").unwrap().tokens["bg"].is_object());
    assert!(s.execute("token.import", &json!({"data": "not base64!"})).is_err());
    assert!(s.execute("token.export", &json!({"format": "rar"})).is_err());
}
