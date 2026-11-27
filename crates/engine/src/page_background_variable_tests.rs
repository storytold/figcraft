//! A page's background can follow a colour variable in the page's own variable mode (#2343), so
//! switching a page to a dark mode turns its canvas dark along with what's on it.

use figcraft_color::Color;
use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
}

fn background(s: &Session, page: NodeId) -> Color {
    match s.doc().unwrap().node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { background, .. }) => *background,
        _ => panic!("not a page"),
    }
}

/// A session with a Light/Dark `page-bg` colour variable, and its current page.
fn themed() -> (Session, NodeId, Color, Color) {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "page-bg", "type": "color", "value": "#F5F4F1"}));
    ex(&mut s, "variable.set", json!({"variable": "page-bg", "mode": "Dark", "value": "#141413"}));
    let page = s.page().unwrap();
    let (light, dark) = (Color::from_hex_or_name("#F5F4F1").unwrap(), Color::from_hex_or_name("#141413").unwrap());
    (s, page, light, dark)
}

#[test]
fn a_bound_page_background_follows_the_pages_mode() {
    let (mut s, page, light, dark) = themed();
    ex(&mut s, "page.setBackground", json!({"color": {"$var": "page-bg"}}));
    assert_eq!(background(&s, page), light, "the collection's default mode");

    ex(&mut s, "variable.setMode", json!({"ids": [page.0], "collection": "Theme", "mode": "Dark"}));
    assert_eq!(background(&s, page), dark, "the page's own mode");

    // A new value for the mode the page shows reaches the page too.
    ex(&mut s, "variable.set", json!({"variable": "page-bg", "mode": "Dark", "value": "#202020"}));
    assert_ne!(background(&s, page), dark);
    assert_eq!(background(&s, page), Color::from_hex_or_name("#202020").unwrap());
}

#[test]
fn a_colour_detaches_the_variable_and_undo_brings_it_back() {
    let (mut s, page, _, dark) = themed();
    ex(&mut s, "page.setBackground", json!({"color": {"$var": "page-bg"}}));
    ex(&mut s, "page.setBackground", json!({"color": "#FF0000"}));
    let red = Color::from_hex_or_name("#FF0000").unwrap();
    assert_eq!(background(&s, page), red);
    // Detached: a mode switch leaves the colour alone.
    ex(&mut s, "variable.setMode", json!({"ids": [page.0], "collection": "Theme", "mode": "Dark"}));
    assert_eq!(background(&s, page), red);

    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.doc().unwrap().node(page).unwrap().bound.contains_key("background"), "the binding is back");
    ex(&mut s, "variable.setMode", json!({"ids": [page.0], "collection": "Theme", "mode": "Dark"}));
    assert_eq!(background(&s, page), dark);
}

#[test]
fn only_a_colour_variable_binds_and_only_a_page_has_a_background() {
    let (mut s, page, _, _) = themed();
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "gap", "type": "number", "value": 8}));
    assert!(s.execute("page.setBackground", &json!({"color": {"$var": "gap"}})).is_err());
    assert!(s.execute("page.setBackground", &json!({"color": {"$var": "nope"}})).is_err());
    assert!(s.execute("page.setBackground", &json!({"color": {"$var": "page-bg", "x": 1}})).is_err());
    assert!(!s.doc().unwrap().node(page).unwrap().bound.contains_key("background"), "a refused call binds nothing");

    // The same binding through `variable.bind`, as the Design panel's variable button runs it.
    ex(&mut s, "variable.bind", json!({"ids": [page.0], "field": "background", "variable": "page-bg"}));
    assert!(s.doc().unwrap().node(page).unwrap().bound.contains_key("background"));
    let frame = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let frame = frame["id"].as_u64().unwrap();
    assert!(s.execute("variable.bind", &json!({"ids": [frame], "field": "background", "variable": "page-bg"})).is_err());
}
