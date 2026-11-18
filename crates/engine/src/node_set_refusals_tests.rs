//! `node.set` doesn't store what can't apply: auto layout keys go only to layers that can have auto
//! layout (the others are named in `warnings`, and with none of them the call is refused), and a
//! `fontStyle` the family lacks says which face draws instead.

use serde_json::{Value, json};

use crate::{EngineError, Session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> crate::NodeId {
    crate::params::id_of(v).unwrap()
}

fn bad_msg(r: crate::Result<Value>) -> String {
    match r {
        Err(EngineError::BadParams { msg, .. }) => msg,
        other => panic!("expected BadParams, got {other:?}"),
    }
}

#[test]
fn auto_layout_keys_on_layers_without_auto_layout_are_refused() {
    let mut s = Session::new();
    let t = id(&ex(&mut s, "node.create", json!({"type": "text", "text": "Hi"}))["id"]);
    let r = id(&ex(&mut s, "node.create", json!({"type": "rectangle"}))["id"]);
    for (target, key, value) in [(t, "gap", json!(8)), (r, "padding", json!(4)), (t, "mainAlign", json!("center")), (r, "wrap", json!(true))] {
        let msg = bad_msg(s.execute("node.set", &json!({"ids": [target], "props": {key: value}})));
        assert!(msg.contains(key) && msg.contains("auto layout"), "{msg}");
        assert!(s.doc().unwrap().node(target).unwrap().layout.is_none(), "no hidden auto layout on {target}");
    }
    // Frames still take them, with or without a layout mode first.
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame"}))["id"]);
    let out = ex(&mut s, "node.set", json!({"ids": [f], "props": {"gap": 8, "padding": 4}}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.as_ref().map(|l| l.gap), Some(8.0));
    assert!(out.get("warnings").is_none(), "{out}");
}

#[test]
fn with_several_layers_the_frames_take_auto_layout_keys_and_the_others_are_named() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "Card"}))["id"]);
    let t = id(&ex(&mut s, "node.create", json!({"type": "text", "text": "Hi", "name": "Title"}))["id"]);
    let out = ex(&mut s, "node.set", json!({"ids": [f, t], "props": {"gap": 8, "opacity": 0.5}}));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(f).unwrap().layout.as_ref().map(|l| l.gap), Some(8.0));
    assert!(doc.node(t).unwrap().layout.is_none(), "no hidden auto layout on the text");
    assert_eq!(doc.node(t).unwrap().opacity, 0.5, "the text still gets the other keys");
    let w = out["warnings"][0].as_str().unwrap();
    assert!(w.contains("`gap`") && w.contains(&format!("{t} “Title”")) && w.contains("TEXT"), "{w}");
    // One undo step for all of it.
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(f).unwrap().layout.as_ref().is_none_or(|l| l.gap != 8.0));
}

#[test]
fn a_font_style_draws_in_the_nearest_face_and_says_so_when_it_differs() {
    figcraft_text::scan_system_fonts();
    // Any installed family with a regular and a bold face and nothing heavier (CI machines differ).
    let pick = figcraft_text::family_names().into_iter().find_map(|f| {
        let st = figcraft_text::family_styles(&f);
        let has = |w: u16| st.iter().any(|x| figcraft_doc::style_weight(x) == w && !x.to_lowercase().contains("italic"));
        (has(400) && has(700) && !st.iter().any(|x| figcraft_doc::style_weight(x) > 700)).then_some((f, st))
    });
    let Some((family, styles)) = pick else { return };
    let mut s = Session::new();
    let t = id(&ex(&mut s, "node.create", json!({"type": "text", "text": "Hi", "props": {"fontFamily": family}}))["id"]);
    let set = |s: &mut Session, style: &str| ex(s, "node.set", json!({"ids": [t], "props": {"fontStyle": style}}));
    // Any spelling of a style it has, and names that resolve to the weight they ask for, say nothing.
    let bold = styles.iter().find(|x| figcraft_doc::style_weight(x) == 700).unwrap().clone();
    for style in [bold.clone(), bold.to_uppercase(), bold.replace(' ', ""), "Roman".into(), "Normal".into()] {
        let out = set(&mut s, &style);
        assert!(out.get("warnings").is_none(), "{style}: {out}");
    }
    // A heavier weight than the family has applies (as a text layer resolves it) and says which face draws.
    let out = set(&mut s, "Black");
    let w = out["warnings"][0].as_str().unwrap();
    assert!(w.contains(&family) && w.contains("`Black`") && w.contains("nearest"), "{w}");
    assert_eq!(s.doc().unwrap().node(t).unwrap().kind.text().unwrap().spans()[0].2.font_style, "Black");
    // A family that isn't installed keeps any style name without a word (files from elsewhere name theirs).
    let out = ex(&mut s, "node.set", json!({"ids": [t], "props": {"fontFamily": "No Such Family 7f3a", "fontStyle": "Hairline"}}));
    assert!(out.get("warnings").is_none(), "{out}");
}

#[test]
fn a_refused_call_changes_nothing() {
    let mut s = Session::new();
    let t = id(&ex(&mut s, "node.create", json!({"type": "text", "text": "Hi"}))["id"]);
    let before = s.doc().unwrap().node(t).unwrap().fills.clone();
    // `fill` comes before `gap` (keys apply in order), so this checks the rollback.
    assert!(s.execute("node.set", &json!({"ids": [t], "props": {"fill": "#FF0000", "gap": 8}})).is_err());
    assert_eq!(s.doc().unwrap().node(t).unwrap().fills, before);
}
