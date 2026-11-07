//! A colour variable's alpha applies once to the paint it is bound to (#802): a `#RRGGBBAA` fill keeps
//! its alpha in the paint opacity, and binding a variable with alpha hands the variable the whole
//! colour, so the two alphas don't multiply (canvas, SVG, `.fig` round trips, mode switches).

use figcraft_doc::NodeId;
use serde_json::json;

use crate::Session;

/// 0x80 / 255 and 0x40 / 255.
const HALF: f32 = 128.0 / 255.0;
const QUARTER: f32 = 64.0 / 255.0;

fn ex(s: &mut Session, cmd: &str, p: serde_json::Value) {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
}

/// A rectangle filled `#0D6EFD80` and a colour variable `tint` (Light `light`, Dark `dark`).
fn setup(light: &str, dark: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "props": {"fill": "#0D6EFD80"}})).unwrap();
    let r = NodeId(r["id"].as_u64().unwrap());
    ex(&mut s, "variable.createCollection", json!({"name": "Tokens", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "tint", "type": "color", "value": light}));
    ex(&mut s, "variable.set", json!({"variable": "tint", "mode": "Dark", "value": dark}));
    (s, r)
}

/// The fill's effective alpha (colour alpha times paint opacity).
fn alpha(s: &Session, r: NodeId) -> f32 {
    s.doc().unwrap().node(r).unwrap().fills[0].solid_color().unwrap().a
}

/// The alpha byte of the rectangle rendered on its own (transparent backdrop).
fn pixel_alpha(s: &Session, r: NodeId) -> u8 {
    let pix = figcraft_render::render_node(s.doc().unwrap(), r, 1.0).unwrap();
    let i = ((5 * pix.width() + 5) * 4 + 3) as usize;
    pix.data()[i]
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 2e-3
}

#[test]
fn a_hex_with_alpha_keeps_its_alpha_in_the_opacity() {
    let (s, r) = setup("#0D6EFD80", "#0D6EFD40");
    let p = &s.doc().unwrap().node(r).unwrap().fills[0];
    assert_eq!(p.kind, figcraft_color::PaintKind::Solid { color: figcraft_color::Color::from_rgb8_const(0x0D, 0x6E, 0xFD) });
    assert!(near(p.opacity, HALF));
    assert_eq!(pixel_alpha(&s, r), 128);
}

#[test]
fn a_bound_variable_alpha_applies_once() {
    // The fill and the variable are both #0D6EFD80: the result is 50%, not 25%.
    let (mut s, r) = setup("#0D6EFD80", "#0D6EFD40");
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    assert!(near(alpha(&s, r), HALF), "{}", alpha(&s, r));
    assert_eq!(pixel_alpha(&s, r), 128);
    let svg = figcraft_svg::export_svg(s.doc().unwrap(), &[r], &figcraft_svg::SvgOptions::default()).unwrap();
    assert!(svg.contains("fill=\"#0D6EFD\" fill-opacity=\"0.502\""), "{svg}");
    // Binding the same variable again changes nothing.
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    assert!(near(alpha(&s, r), HALF));
}

#[test]
fn unbinding_keeps_the_colour_with_its_alpha_in_the_opacity() {
    let (mut s, r) = setup("#0D6EFD40", "#0D6EFD");
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    assert_eq!(pixel_alpha(&s, r), 64);
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": null}));
    let p = &s.doc().unwrap().node(r).unwrap().fills[0];
    assert_eq!(p.kind, figcraft_color::PaintKind::Solid { color: figcraft_color::Color::from_rgb8_const(0x0D, 0x6E, 0xFD) });
    assert!(near(p.opacity, QUARTER), "{}", p.opacity);
    assert_eq!(pixel_alpha(&s, r), 64);
    // Undo restores the binding and its paint.
    ex(&mut s, "edit.undo", json!({}));
    assert!(!s.doc().unwrap().node(r).unwrap().bound.is_empty());
    assert_eq!(pixel_alpha(&s, r), 64);
}

#[test]
fn modes_switch_the_alpha_without_accumulating() {
    let (mut s, r) = setup("#0D6EFD40", "#0D6EFD80");
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    for (mode, want) in [("Dark", HALF), ("Light", QUARTER), ("Dark", HALF), ("Light", QUARTER)] {
        ex(&mut s, "variable.setMode", json!({"collection": "Tokens", "mode": mode}));
        assert!(near(alpha(&s, r), want), "{mode}: {}", alpha(&s, r));
    }
    // An opacity set while bound multiplies on top and survives mode switches.
    ex(&mut s, "fill.set", json!({"ids": [r.0], "index": 0, "opacity": 0.5}));
    ex(&mut s, "variable.setMode", json!({"collection": "Tokens", "mode": "Dark"}));
    assert!(near(alpha(&s, r), HALF * 0.5));
}

#[test]
fn node_set_and_style_bindings_apply_the_alpha_once() {
    let (mut s, r) = setup("#0D6EFD80", "#0D6EFD40");
    let var = s.doc().unwrap().variables.get("tint").unwrap().id.clone();
    ex(&mut s, "node.set", json!({"id": r.0, "props": {"boundVariables": {"fill": var}}}));
    assert!(near(alpha(&s, r), HALF));
    ex(&mut s, "node.set", json!({"id": r.0, "props": {"boundVariables": {"fill": null}}}));
    assert!(near(s.doc().unwrap().node(r).unwrap().fills[0].opacity, HALF));
    // A paint style made from a `#RRGGBBAA` colour, bound to the variable.
    let st = s.execute("style.create", &json!({"type": "paint", "name": "Tint", "value": "#0D6EFD80"})).unwrap()["id"].clone();
    ex(&mut s, "style.bind", json!({"style": st, "field": "fills/0/color", "variable": "tint"}));
    ex(&mut s, "style.apply", json!({"ids": [r.0], "style": st}));
    assert!(near(alpha(&s, r), HALF), "{}", alpha(&s, r));
}

#[test]
fn a_fig_round_trip_keeps_the_alpha_once() {
    let (mut s, r) = setup("#0D6EFD80", "#0D6EFD40");
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    ex(&mut s, "fill.set", json!({"ids": [r.0], "index": 0, "opacity": 0.5}));
    let (bytes, _) = figcraft_format::fig::export(s.doc().unwrap(), None).unwrap();
    let back = crate::cmd::file::load_fig(&bytes, "x").unwrap().unwrap().doc;
    let n = back.nodes().find(|n| !n.bound.is_empty()).unwrap();
    let a = n.fills[0].solid_color().unwrap().a;
    assert!(near(a, HALF * 0.5), "{a}");
}

#[test]
fn code_gives_a_translucent_stroke_its_alpha_once() {
    let (mut s, r) = setup("#0D6EFD80", "#0D6EFD40");
    ex(&mut s, "stroke.add", json!({"ids": [r.0], "color": "#FF000080"}));
    let code = s.execute("inspect.code", &json!({"id": r.0})).unwrap()["code"].as_str().unwrap_or_default().to_string();
    assert!(code.contains("solid rgba(255, 0, 0, 0.50)"), "{code}");
}

#[test]
fn binding_keeps_an_opacity_set_on_the_paint() {
    // A 15% tint whose colour comes from a variable (#923): the bind replaces the colour only.
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Tokens", "modes": ["Light"]}));
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "red", "type": "color", "value": "#FF0000"}));
    let props = json!({"fills": [{"type": "solid", "color": "#000000", "opacity": 0.15}]});
    let r = s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "props": props})).unwrap();
    let r = NodeId(r["id"].as_u64().unwrap());
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "red"}));
    let p = s.doc().unwrap().node(r).unwrap().fills[0].clone();
    assert_eq!(p.kind, figcraft_color::PaintKind::Solid { color: figcraft_color::Color::from_rgb8_const(0xFF, 0, 0) });
    assert!(near(p.opacity, 0.15), "{}", p.opacity);
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": null}));
    assert!(near(s.doc().unwrap().node(r).unwrap().fills[0].opacity, 0.15));
    // An opacity set on a `#RRGGBBAA` fill is the user's too, and stays.
    let (mut s, r) = setup("#FF0000", "#FF0000");
    ex(&mut s, "fill.set", json!({"ids": [r.0], "index": 0, "opacity": 0.25}));
    ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": "tint"}));
    assert!(near(alpha(&s, r), 0.25), "{}", alpha(&s, r));
}

#[test]
fn an_unbind_and_rebind_keeps_the_opacity_set_on_the_paint() {
    // A paint at 50% bound to a half-transparent colour shows a quarter, unbound or bound again.
    let (mut s, r) = setup("#FF000080", "#FF000080");
    ex(&mut s, "fill.set", json!({"ids": [r.0], "index": 0, "opacity": 0.5}));
    for variable in [json!("tint"), json!(null), json!("tint"), json!(null)] {
        ex(&mut s, "variable.bind", json!({"ids": [r.0], "field": "fill", "variable": variable}));
        assert!(near(alpha(&s, r), HALF / 2.0), "{variable}: {}", alpha(&s, r));
    }
}
