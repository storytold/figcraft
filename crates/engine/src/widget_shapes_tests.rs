//! Lines and SVG in a widget (plugin API 6), as a Figma widget's `Line` and `SVG` draw: a line is
//! 100 long, black and 1 thick by default and turns around its top-left corner; an SVG comes in as
//! locked layers, scaled to fit a size; a click on it reaches the plugin, and one with too many
//! layers leaves the widget as it was.

use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{ex, idv, node};
use crate::plugins_tests::{install, undo_len};
use crate::widgets_tests::{W, err};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::rotation_degrees;

const ID: &str = "org.test.shapes";
const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="24" height="12" fill="#ff0000"/><circle cx="6" cy="6" r="4" fill="#00ff00"/></svg>"##;

fn widget(children: Value) -> Value {
    json!({"root": {"type": "frame", "width": 200, "height": 100, "fill": "#FFFFFF", "children": children}})
}

/// A session with a plugin of `api` that draws `v` when inserted, `clicked` when clicked.
fn session_with(api: u32, v: &Value, clicked: &Value) -> Session {
    let mut w = W::default();
    let click = w.has(r#""target":"icon""#);
    let (first, second) = (w.render(v), w.render(clicked));
    let wasm = w.build(&format!("(if {click} (then {second} (return (i32.const 0)))) {first}"));
    let mut s = Session::new();
    let m = json!({"id": ID, "name": "Shapes", "version": "1.0.0", "main": "s.wasm", "api": api, "widget": true, "permissions": {"access": "read"}});
    install(&mut s, m, &wasm);
    s
}

fn insert(s: &mut Session) -> NodeId {
    idv(&ex(s, "widget.insert", json!({"plugin": ID, "x": 40, "y": 40})))
}

fn only_child(s: &Session, w: NodeId) -> NodeId {
    let children = &node(s, w).children;
    assert_eq!(children.len(), 1);
    children[0]
}

#[test]
fn a_line_has_the_documented_defaults() {
    let v = widget(json!([{"type": "line", "x": 10, "y": 20}]));
    let mut s = session_with(6, &v, &v);
    let w = insert(&mut s);
    let line = only_child(&s, w);
    let n = node(&s, line);
    assert!(matches!(n.kind, NodeKind::Line));
    assert_eq!([n.width, n.height], [100.0, 0.0]);
    assert!(n.locked);
    assert_eq!(n.strokes.len(), 1);
    assert!(matches!(n.strokes[0].kind, figcraft_color::PaintKind::Solid { color: c } if c.to_css_hex() == "#000000"));
    assert_eq!(n.stroke.weight, 1.0);
}

#[test]
fn a_line_turns_around_its_top_left_corner() {
    let v = widget(json!([{"type": "line", "x": 10, "y": 20, "width": 50, "rotation": 90, "stroke": "#0D99FF", "strokeWeight": 2}]));
    let mut s = session_with(6, &v, &v);
    let w = insert(&mut s);
    let n = node(&s, only_child(&s, w));
    assert!((rotation_degrees(n.transform) - 90.0).abs() < 1e-6);
    let [.., e, f] = n.transform.as_coeffs();
    assert_eq!([e, f], [10.0, 20.0], "the corner stays where the description puts it");
    // Counter-clockwise on a y-down canvas: the far end is above the corner.
    assert!(n.transform.as_coeffs()[1] < 0.0);
}

#[test]
fn an_svg_comes_in_as_locked_layers_of_its_own_size() {
    let v = widget(json!([{"type": "svg", "name": "Icon", "src": ICON, "x": 8, "y": 4, "opacity": 0.5}]));
    let mut s = session_with(6, &v, &v);
    let w = insert(&mut s);
    let icon = only_child(&s, w);
    let n = node(&s, icon);
    assert_eq!(n.name, "Icon");
    assert_eq!([n.width, n.height], [24.0, 12.0]);
    assert_eq!(n.opacity, 0.5);
    assert_eq!(n.transform.as_coeffs()[4..], [8.0, 4.0]);
    let doc = s.doc().unwrap();
    let layers = doc.subtree(icon);
    assert!(layers.len() > 1, "the picture's shapes are layers: {layers:?}");
    assert!(layers.iter().all(|i| doc.node(*i).unwrap().locked), "nothing in it can be picked on its own");
}

#[test]
fn a_size_scales_the_svg_to_fit_keeping_its_proportions() {
    for (size, expected) in [
        (json!({"width": 48}), [48.0, 24.0]),
        (json!({"height": 6}), [12.0, 6.0]),
        (json!({"width": 48, "height": 6}), [12.0, 6.0]),
        (json!({"width": 48, "height": 100}), [48.0, 24.0]),
    ] {
        let mut d = json!({"type": "svg", "src": ICON});
        d.as_object_mut().unwrap().extend(size.as_object().unwrap().clone());
        let v = widget(json!([d]));
        let mut s = session_with(6, &v, &v);
        let w = insert(&mut s);
        let n = node(&s, only_child(&s, w));
        assert_eq!([n.width, n.height], expected, "{size}");
        let circle = *n.children.last().unwrap();
        assert!((node(&s, circle).width - expected[0] / 3.0).abs() < 1e-6, "{size}: what's inside scaled too");
    }
}

#[test]
fn a_click_on_an_svg_calls_the_plugin_back() {
    let v = widget(json!([{"type": "svg", "target": "icon", "src": ICON}]));
    let after = widget(json!([{"type": "line", "name": "Clicked"}]));
    let mut s = session_with(6, &v, &after);
    let w = insert(&mut s);
    ex(&mut s, "widget.click", json!({"id": w, "target": "icon"}));
    assert_eq!(node(&s, only_child(&s, w)).name, "Clicked");
}

#[test]
fn an_older_plugin_cant_use_them() {
    for kind in [json!({"type": "line"}), json!({"type": "svg", "src": ICON})] {
        let v = widget(json!([kind]));
        let mut s = session_with(5, &v, &v);
        let e = err(&mut s, "widget.insert", json!({"plugin": ID, "x": 0, "y": 0}));
        assert!(e.contains("need plugin API 6"), "{e}");
    }
}

#[test]
fn an_svg_of_too_many_layers_or_junk_leaves_the_widget_as_it_was() {
    let mut many = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="10">"#);
    for i in 0..300 {
        write!(many, r##"<rect x="{i}" width="1" height="10" fill="#000000"/>"##).unwrap();
    }
    many.push_str("</svg>");
    for (bad, why) in [(many.as_str(), "more than"), ("not an svg", "")] {
        let fine = widget(json!([{"type": "line", "name": "Fine", "target": "icon"}]));
        let mut s = session_with(6, &fine, &widget(json!([{"type": "svg", "target": "icon", "src": bad}])));
        let w = insert(&mut s);
        let undo = undo_len(&s);
        let e = err(&mut s, "widget.click", json!({"id": w, "target": "icon"}));
        assert!(e.contains(why), "{e}");
        assert_eq!(node(&s, only_child(&s, w)).name, "Fine");
        assert_eq!(undo_len(&s), undo);
    }
}
