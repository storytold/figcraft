//! A variable renamed to another one's id never takes its place (#1613), beside #1618's tests:
//! an id a person types (`variable.bind`, `variable.set`) finds the variable with that exact id
//! before one named like it, and the Tailwind export and expressions read the stored id. Names still
//! work where a person types them.

use figcraft_doc::VariableValue;
use figcraft_doc::proto::Expr;
use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::ex;

/// The issue's document: Theme with `shadow`, `surface` and `alias` (→ surface), then `shadow`
/// renamed to surface's id with Web code syntax `--shadow`. Ids of shadow, surface and alias.
fn renamed_like_an_id() -> (Session, String, String, String) {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let id = |v: Value| v["id"].as_str().unwrap().to_string();
    let shadow = id(ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "shadow", "type": "color", "value": "#FF0000"})));
    let surface = id(ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF"})));
    let alias = id(ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "alias", "type": "color", "value": {"alias": "surface"}})));
    ex(&mut s, "variable.update", json!({"variable": shadow, "name": surface, "codeSyntax": {"WEB": "--shadow"}}));
    (s, shadow, surface, alias)
}

#[test]
fn exported_aliases_point_at_the_stored_id() {
    let (mut s, ..) = renamed_like_an_id();
    let css = ex(&mut s, "variable.export", json!({"format": "css"}))["text"].as_str().unwrap().to_string();
    assert!(css.contains("--alias: var(--surface);"), "{css}");
    let tw = ex(&mut s, "variable.export", json!({"format": "tailwind"}))["text"].as_str().unwrap().to_string();
    assert!(tw.contains("var(--color-surface)") && !tw.contains("var(--shadow)"), "{tw}");
}

#[test]
fn dev_inspect_names_the_bound_variable() {
    let (mut s, _, surface, _) = renamed_like_an_id();
    let r = ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    let r = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": surface}));
    let doc = s.doc().unwrap();
    let found = crate::devinspect::bound_variable(doc, doc.node(r).unwrap(), "fill").unwrap();
    assert_eq!(found.name, "surface");
}

#[test]
fn expressions_read_and_show_the_stored_variable() {
    let (mut s, _, surface, _) = renamed_like_an_id();
    ex(&mut s, "variable.addMode", json!({"collection": "Theme", "name": "Dark"}));
    ex(&mut s, "variable.set", json!({"variable": surface, "mode": "Dark", "value": "#00FF00"}));
    let vars = &s.doc().unwrap().variables;
    let theme = vars.collection("Theme").unwrap().clone();
    let dark = theme.find_mode("Dark").unwrap().id.clone();
    let e = Expr::InMode { variable: surface.clone(), mode: dark };
    assert!(matches!(e.eval(vars, &Default::default()), Some(VariableValue::Color(c)) if c.to_hex() == "00FF00"));
    assert_eq!(e.display(vars), "surface[Dark]");
    assert_eq!(Expr::var(surface).display(vars), "surface");
}
