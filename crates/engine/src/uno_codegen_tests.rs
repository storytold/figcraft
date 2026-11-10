//! Uno C# Markup code: `inspect.code {language: "uno"}` writes a fluent C# expression tree of
//! WinUI elements for a layer and its subtree.

use crate::Session;
use crate::codegen::CodeLang;
use crate::components::tests::{ex, idv};
use figcraft_doc::*;
use serde_json::json;

fn uno(s: &mut Session, id: NodeId) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": "uno"}))["code"].as_str().unwrap_or_default().to_string()
}

#[test]
fn uno_is_a_code_language() {
    assert!(CodeLang::ALL.contains(&CodeLang::Uno));
    assert_eq!(CodeLang::Uno.id(), "uno");
    assert_eq!(CodeLang::Uno.label(), "Uno C# Markup");
    for alias in ["uno", "UNO", "csharp-markup", "cs-markup", "Uno C# Markup"] {
        assert_eq!(CodeLang::from_id(alias), Some(CodeLang::Uno), "{alias}");
    }
    assert!(!CodeLang::Uno.is_web() && !CodeLang::Uno.escapes_dollar());
    // Not connectable to code until components are mapped.
    assert!(crate::code_connect::language("uno").is_none());
}

#[test]
fn inspect_code_answers_in_uno() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 50, "name": "Box"})));
    let r = ex(&mut s, "inspect.code", json!({"id": f, "language": "uno"}));
    assert_eq!(r["language"], "uno");
    assert!(!uno(&mut s, f).is_empty());
}
