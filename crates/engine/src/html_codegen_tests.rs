//! Code for whole frames: nested HTML with a stylesheet (flexbox for auto layout, absolute
//! positions elsewhere), unique class names, escaped text.

use serde_json::{Value, json};

use crate::Session;
use crate::codegen::{CodeLang, code_for};
use crate::params::id_of;

fn id(v: Value) -> figcraft_doc::NodeId {
    id_of(&v["id"]).unwrap()
}

#[test]
fn frames_become_nested_html_with_flexbox_and_absolute_children() {
    let mut s = Session::new();
    let card = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap());
    s.execute("node.set", &json!({"id": card, "props": {"name": "Card"}})).unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 20, "parent": card})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 30, "width": 50, "height": 20, "parent": card})).unwrap());
    for r in [a, b] {
        s.execute("node.set", &json!({"id": r, "props": {"name": "Row", "fill": "#0D99FF"}})).unwrap();
    }
    let t = id(s.execute("text.create", &json!({"text": "Hi & <bye>", "x": 0, "y": 60})).unwrap());
    s.execute("node.reparent", &json!({"ids": [t], "parent": card})).unwrap();
    s.execute("layout.add", &json!({"ids": [card]})).unwrap();
    s.execute("node.set", &json!({"id": card, "props": {"layoutMode": "vertical", "gap": 8}})).unwrap();

    let code = code_for(s.doc().unwrap(), card, CodeLang::Html);
    assert!(code.starts_with("<div class=\"card\">"), "{code}");
    assert!(code.contains("<div class=\"row\"></div>") && code.contains("<div class=\"row-2\"></div>"), "unique classes: {code}");
    assert!(code.contains("&amp; &lt;bye&gt;</p>"), "escaped text: {code}");
    assert!(code.contains(".card {\n  position: relative;\n") && code.contains("display: flex;") && code.contains("flex-direction: column;"));
    assert!(!code.contains(".row {\n  position: absolute"), "flex items flow");
    assert!(code.trim_end().ends_with("</style>"));

    // Without auto layout, children sit at their positions.
    s.execute("layout.remove", &json!({"ids": [card]})).unwrap();
    let code = code_for(s.doc().unwrap(), card, CodeLang::Html);
    assert!(code.contains(".row-2 {\n  position: absolute;\n  left: "), "{code}");
    // The command takes the language by id.
    let out = s.execute("inspect.code", &json!({"id": card, "language": "html"})).unwrap();
    assert!(out["code"].as_str().unwrap().contains("<style>"));
}

#[test]
fn class_names_are_valid_and_deep_trees_are_bounded() {
    let mut s = Session::new();
    let mut parent = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("node.set", &json!({"id": parent, "props": {"name": "1 Ünïcode / Name!!"}})).unwrap();
    let root = parent;
    for _ in 0..30 {
        parent = id(s.execute("frame.create", &json!({"x": 1, "y": 1, "width": 90, "height": 90, "parent": parent})).unwrap());
    }
    let code = code_for(s.doc().unwrap(), root, CodeLang::Html);
    assert!(code.starts_with("<div class=\"layer-1-unicode-name\">"), "{}", code.lines().next().unwrap());
    // Nesting stops at the depth limit instead of running away.
    assert!(code.matches("<div").count() <= 14, "{}", code.matches("<div").count());
}

#[test]
fn non_ascii_layer_names_transliterate_into_readable_classes() {
    let mut s = Session::new();
    // Korean reads as its romanization instead of layer- (#1235), with duplicates numbered.
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap());
    s.execute("node.set", &json!({"id": f, "props": {"name": "로그인 화면"}})).unwrap();
    let a = id(s.execute("text.create", &json!({"text": "Login", "x": 0, "y": 0, "parent": f})).unwrap());
    let b = id(s.execute("text.create", &json!({"text": "Login", "x": 0, "y": 40, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": a, "props": {"name": "로그인"}})).unwrap();
    s.execute("node.set", &json!({"id": b, "props": {"name": "로그인"}})).unwrap();
    let code = code_for(s.doc().unwrap(), f, CodeLang::Html);
    assert!(code.contains("<div class=\"rogeuin-hwamyeon\">"), "{code}");
    assert!(code.contains("<p class=\"rogeuin\">") && code.contains("<p class=\"rogeuin-2\">"), "{code}");
}

#[test]
fn other_scripts_and_symbol_names_transliterate_or_fall_back_to_the_kind() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "按钮"})).unwrap());
    let k = id(s.execute("text.create", &json!({"text": "Ok", "x": 0, "y": 0, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": k, "props": {"name": "кнопка"}})).unwrap();
    let hash = id(s.execute("text.create", &json!({"text": "###", "x": 0, "y": 20, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": hash, "props": {"name": "###"}})).unwrap();
    let digit = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 40, "width": 10, "height": 10, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": digit, "props": {"name": "4버튼"}})).unwrap();
    let code = code_for(s.doc().unwrap(), f, CodeLang::Html);
    assert!(code.contains("<div class=\"an-niu\">"), "Chinese: {code}");
    assert!(code.contains("<p class=\"knopka\">"), "Cyrillic: {code}");
    assert!(code.contains("<p class=\"text\">"), "symbols fall back to the kind: {code}");
    assert!(code.contains("layer-4beoteun"), "a leading digit keeps the layer- prefix: {code}");
}

#[test]
fn long_names_cut_at_a_word_boundary() {
    let mut s = Session::new();
    let long = format!("{}-{}", "a".repeat(30), "b".repeat(30));
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "name": long})).unwrap());
    let code = code_for(s.doc().unwrap(), f, CodeLang::Html);
    let word = "a".repeat(30);
    assert!(code.contains(&format!("<div class=\"{word}\">")), "cut back to the first word: {code}");
    let unbroken = "c".repeat(50);
    let g = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "name": unbroken})).unwrap());
    let code = code_for(s.doc().unwrap(), g, CodeLang::Html);
    assert!(code.contains(&format!("class=\"{}\"", "c".repeat(40))), "one long word cuts at 40: {code}");
}

#[test]
fn leptos_wraps_the_markup_in_a_component_with_text_as_string_literals() {
    let mut s = Session::new();
    let card = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap());
    s.execute("node.set", &json!({"id": card, "props": {"name": "Card"}})).unwrap();
    let t = id(s.execute("text.create", &json!({"text": "Say \"hi\" {x}\nسلام", "x": 0, "y": 60})).unwrap());
    s.execute("node.reparent", &json!({"ids": [t], "parent": card})).unwrap();

    let code = code_for(s.doc().unwrap(), card, CodeLang::Leptos);
    assert!(
        code.starts_with("use leptos::prelude::*;\n\n#[component]\npub fn Card() -> impl IntoView {\n    view! {\n        <div class=\"card\">"),
        "{code}"
    );
    // Quotes and braces stay inside Rust string literals; lines break with a self-closed tag.
    assert!(code.contains(r#">"Say \"hi\" {x}"<br/>"سلام"</p>"#), "{code}");
    assert!(code.contains("/* card.css */\n") && code.contains(".card {\n  position: relative;\n"), "{code}");
    let out = s.execute("inspect.code", &json!({"id": card, "language": "leptos", "unit": "rem"})).unwrap();
    assert!(out["code"].as_str().unwrap().contains("width: 12.5rem;"), "{}", out["code"]);
}

fn web_code(s: &mut Session, id: figcraft_doc::NodeId, language: &str, unit: &str) -> String {
    let out = s.execute("inspect.code", &json!({"id": id, "language": language, "unit": unit})).unwrap();
    out["code"].as_str().unwrap().to_string()
}

#[test]
fn rem_preserves_text_in_web_markup() {
    let mut s = Session::new();
    let frame = id(s.execute("frame.create", &json!({"width": 200, "height": 100, "name": "Card"})).unwrap());
    s.execute("text.create", &json!({"text": "Tile 24px", "parent": frame, "name": "Label"})).unwrap();
    let mut changed = Vec::new();
    for language in ["html", "react", "leptos"] {
        let px = web_code(&mut s, frame, language, "px");
        let rem = web_code(&mut s, frame, language, "rem");
        assert!(px.contains("Tile 24px"), "{language}: px control: {px}");
        assert!(rem.contains("width: 12.5rem;"), "{language}: width: {rem}");
        assert!(rem.contains("font-size: 0.75rem;"), "{language}: font: {rem}");
        if !rem.contains("Tile 24px") {
            changed.push(language);
        }
    }
    assert!(changed.is_empty(), "text rewritten in {changed:?}");
}

#[test]
fn rem_preserves_text_property_default() {
    let mut s = Session::new();
    let frame = id(s.execute("frame.create", &json!({"width": 100, "height": 40, "name": "Chip"})).unwrap());
    let label = id(s.execute("text.create", &json!({"text": "24px", "parent": frame, "name": "Label"})).unwrap());
    let component = id(s.execute("component.create", &json!({"ids": [frame]})).unwrap());
    s.execute("component.addProperty", &json!({"id": component, "name": "Label", "type": "text", "default": "24px", "layer": label})).unwrap();
    for unit in ["px", "rem"] {
        let code = web_code(&mut s, component, "react", unit);
        assert!(code.contains("label = \"24px\""), "{unit}: changed default: {code}");
        if unit == "rem" {
            assert!(code.contains("width: 6.25rem;"), "rem still converts lengths: {code}");
        }
    }
}
