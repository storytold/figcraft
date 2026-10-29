//! Inspect behaviour suite, written from the public documentation of design tools' inspect and
//! developer handoff modes: the reported layout (size, position, rotation, radius, opacity, blend),
//! colour formats, units, typography, the generated CSS, SwiftUI, Compose and Flutter for common
//! layers, the asset list and redline measurements. Each rule is a table row or an assert with a
//! one-line derivation; where the documentation is silent the row says so and pins today's
//! behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_color::{Color, Gradient, GradientKind, Paint, PaintKind};
use figcraft_doc::{LetterSpacing, LineHeight, NodeId};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    let r = id(&ex(s, "shape.rectangle", p));
    set(s, &[r], json!({"fills": ["#FF0000"]}));
    r
}

fn set(s: &mut Session, ids: &[NodeId], props: Value) {
    ex(s, "node.set", json!({"ids": ids, "props": props}));
}

/// The value of the inspected row `label` (first match), with extra `params`.
fn row(s: &mut Session, id: NodeId, label: &str, params: Value) -> Option<String> {
    let mut p = params;
    p["id"] = json!(id);
    let r = ex(s, "inspect.properties", p);
    r["sections"]
        .as_array()?
        .iter()
        .flat_map(|sec| sec["rows"].as_array().cloned().unwrap_or_default())
        .find_map(|row| (row["label"] == label).then(|| row["value"].as_str().unwrap_or_default().to_string()))
}

fn code(s: &mut Session, id: NodeId, params: Value) -> String {
    let mut p = params;
    p["id"] = json!(id);
    ex(s, "inspect.code", p)["code"].as_str().unwrap().to_string()
}

fn css(s: &mut Session, id: NodeId) -> String {
    code(s, id, json!({}))
}

fn gradient(kind: GradientKind, from: (f64, f64), to: (f64, f64)) -> Paint {
    let mut g = Gradient::default_for(kind, Color::BLACK);
    (g.from, g.to) = (from, to);
    Paint { kind: PaintKind::Gradient(g), ..Paint::solid(Color::BLACK) }
}

#[test]
fn layout_rows_report_size_position_and_look() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 100, "y": 100, "width": 400, "height": 300})));
    // Positions given with a parent are inside it.
    let r = rect(&mut s, 10.0, 20.0, 80.0, 40.0, Some(f));
    let a = rect(&mut s, 100.0, 50.0, 20.0, 20.0, Some(f));
    let b = rect(&mut s, 160.0, 50.0, 20.0, 20.0, Some(f));
    let g = id(&ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let top = rect(&mut s, 700.0, 50.0, 10.0, 10.0, None);
    set(&mut s, &[r], json!({"opacity": 0.5, "blend": "multiply", "cornerRadius": 8}));
    let mixed = rect(&mut s, 20.0, 100.0, 40.0, 40.0, Some(f));
    set(&mut s, &[mixed], json!({"topLeftRadius": 1, "topRightRadius": 2, "bottomRightRadius": 3, "bottomLeftRadius": 4}));
    let turned = rect(&mut s, 200.0, 200.0, 40.0, 40.0, Some(f));
    set(&mut s, &[turned], json!({"rotation": 30}));
    let _ = g;
    // (layer, label, value, derivation)
    let rows: [(NodeId, &str, &str, &str); 11] = [
        (r, "Width", "80", "the layer's width"),
        (r, "Height", "40", "the layer's height"),
        (r, "Left", "10", "positions are measured from the parent frame's top-left"),
        (r, "Top", "20", "positions are measured from the parent frame's top-left"),
        (a, "Left", "100", "groups aren't frames: a grouped layer is measured from the frame around the group"),
        (top, "Left", "700", "a top-level layer is measured from the canvas origin"),
        (r, "Opacity", "50%", "layer opacity in percent"),
        (r, "Blend", "Multiply", "a blend mode other than normal is reported"),
        (r, "Radius", "8", "one radius for all corners"),
        (mixed, "Radius", "1 2 3 4", "mixed radii in CSS order: top-left, top-right, bottom-right, bottom-left"),
        (turned, "Rotation", "30°", "rotation in degrees"),
    ];
    for (layer, label, want, why) in rows {
        assert_eq!(row(&mut s, layer, label, json!({})).as_deref(), Some(want), "{label}: {why}");
    }
    assert_eq!(row(&mut s, top, "Blend", json!({})), None, "normal blending isn't reported");
    assert_eq!(row(&mut s, top, "Opacity", json!({})), None, "full opacity isn't reported");
}

#[test]
fn colour_formats() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0, None);
    set(&mut s, &[r], json!({"fills": ["#FF8000"]}));
    let half = rect(&mut s, 20.0, 0.0, 10.0, 10.0, None);
    s.doc_mut().unwrap().get_mut(half).unwrap().fills = vec![Paint::solid(Color::from_hex("#FF8000").unwrap().with_alpha(0.5))];
    // (format, opaque, half transparent, derivation)
    let rows: [(&str, &str, &str, &str); 5] = [
        ("hex", "#FF8000", "#FF8000 · 50%", "hex with the opacity beside it (the default)"),
        ("rgb", "255, 128, 0", "255, 128, 0 · 50%", "red, green and blue from 0 to 255"),
        ("css", "rgb(255, 128, 0)", "rgba(255, 128, 0, 0.5)", "a CSS colour value"),
        ("hsl", "30, 100%, 50%", "30, 100%, 50% · 50%", "hue in degrees, saturation and lightness in percent"),
        ("hsb", "30, 100%, 100%", "30, 100%, 100% · 50%", "hue in degrees, saturation and brightness in percent"),
    ];
    for (format, opaque, translucent, why) in rows {
        assert_eq!(row(&mut s, r, "Solid", json!({"colorFormat": format})).as_deref(), Some(opaque), "{format}: {why}");
        assert_eq!(row(&mut s, half, "Solid", json!({"colorFormat": format})).as_deref(), Some(translucent), "{format}: {why}");
    }
    assert_eq!(row(&mut s, r, "Solid", json!({})).as_deref(), Some("#FF8000"), "hex is the default");
    assert!(s.execute("inspect.properties", &json!({"id": r, "colorFormat": "cmyk"})).is_err(), "unknown formats are refused");
}

#[test]
fn css_units() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 40.0, None);
    let px = css(&mut s, r);
    assert!(px.contains("width: 100px;"), "pixels are the default unit");
    let rem = code(&mut s, r, json!({"unit": "rem"}));
    assert!(rem.contains("width: 6.25rem;") && rem.contains("height: 2.5rem;"), "rem divides by the root font size, 16 by default:\n{rem}");
    let ten = code(&mut s, r, json!({"unit": "rem", "rootFontSize": 10}));
    assert!(ten.contains("width: 10rem;"), "the root font size can be changed");
    assert!(s.execute("inspect.code", &json!({"id": r, "unit": "em"})).is_err(), "unknown units are refused");
}

#[test]
fn typography_rows_and_css() {
    let mut s = Session::new();
    let t = id(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0})));
    set(&mut s, &[t], json!({"fontSize": 24, "textAlign": "center", "textCase": "upper", "textDecoration": "underline"}));
    if let Some(tx) = s.doc_mut().unwrap().get_mut(t).unwrap().kind.text_mut() {
        tx.style.line_height = LineHeight::Percent(150.0);
        tx.style.letter_spacing = LetterSpacing::Percent(5.0);
    }
    // (label, value, derivation)
    let rows: [(&str, &str, &str); 6] = [
        ("Font", "Inter", "the font family"),
        ("Style", "Regular", "the font style"),
        ("Size", "24", "the font size"),
        ("Line height", "150%", "line height as set (percent here)"),
        ("Letter spacing", "5%", "letter spacing as set"),
        ("Align", "center", "horizontal alignment"),
    ];
    for (label, want, why) in rows {
        assert_eq!(row(&mut s, t, label, json!({})).as_deref(), Some(want), "{label}: {why}");
    }
    let code = css(&mut s, t);
    // (declaration, derivation)
    let decls: [(&str, &str); 8] = [
        ("font-family: \"Inter\";", "the family, quoted"),
        ("font-size: 24px;", "the size in pixels"),
        ("font-weight: 400;", "Regular is weight 400"),
        ("line-height: 150%;", "a percent line height stays a percentage"),
        ("letter-spacing: 0.05em;", "percent letter spacing is a fraction of the font size (em)"),
        ("text-align: center;", "alignment becomes text-align"),
        ("text-transform: uppercase;", "upper case becomes text-transform"),
        ("text-decoration: underline;", "underline becomes text-decoration"),
    ];
    for (d, why) in decls {
        assert!(code.contains(d), "{d}: {why}\n{code}");
    }
}

#[test]
fn css_for_common_layers() {
    let mut s = Session::new();
    let plain = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    let styled = rect(&mut s, 0.0, 100.0, 100.0, 50.0, None);
    set(
        &mut s,
        &[styled],
        json!({"cornerRadius": 8, "opacity": 0.5, "strokes": ["#000000"], "strokeWeight": 2, "blend": "multiply", "rotation": 30}),
    );
    ex(&mut s, "effect.add", json!({"ids": [styled], "type": "dropShadow"}));
    let linear = rect(&mut s, 0.0, 200.0, 100.0, 50.0, None);
    let radial = rect(&mut s, 0.0, 300.0, 100.0, 50.0, None);
    {
        let d = s.doc_mut().unwrap();
        d.get_mut(linear).unwrap().fills = vec![gradient(GradientKind::Linear, (0.0, 0.5), (1.0, 0.5))];
        d.get_mut(radial).unwrap().fills = vec![gradient(GradientKind::Radial, (0.5, 0.5), (0.5, 1.0))];
    }
    let shadow = css(&mut s, styled).lines().find(|l| l.starts_with("box-shadow")).map(str::to_string);
    // (layer, declaration, derivation)
    let rows: [(NodeId, &str, &str); 11] = [
        (plain, "width: 100px;", "the width in pixels"),
        (plain, "height: 50px;", "the height in pixels"),
        (plain, "background: #FF0000;", "a solid fill is the background"),
        (styled, "border-radius: 8px;", "corner radius is border-radius"),
        (styled, "opacity: 0.5;", "layer opacity is opacity"),
        (styled, "border: 2px solid #000000;", "docs silent: a stroke is a solid border of its weight"),
        (styled, "mix-blend-mode: multiply;", "a blend mode is mix-blend-mode"),
        (styled, "transform: rotate(-30deg);", "rotation turns counter-clockwise, CSS rotate turns clockwise"),
        (linear, "background: linear-gradient(90deg, ", "a linear gradient keeps its direction (left to right is 90deg)"),
        (radial, "background: radial-gradient(", "a radial gradient is radial-gradient"),
        (styled, "box-shadow: 0px 4px 4px 0px rgba(0, 0, 0, 0.25);", "a drop shadow is box-shadow: x, y, blur, spread, colour"),
    ];
    for (layer, d, why) in rows {
        let c = css(&mut s, layer);
        assert!(c.contains(d), "{d}: {why}\n{c}\nshadow: {shadow:?}");
    }
}

/// Individual stroke sides are borders on those sides only: a header with a divider under it isn't
/// boxed in, and a side bound to a variable names it.
#[test]
fn css_borders_follow_individual_stroke_sides() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"strokes": ["#000000"], "strokeWeight": 1, "strokeSides": [0, 0, 2, 0]}));
    let c = css(&mut s, r);
    assert!(c.contains("border-bottom: 2px solid #000000;"), "{c}");
    assert!(!c.contains("border:") && !c.contains("border-top") && !c.contains("border-right") && !c.contains("border-left"), "{c}");

    let col = ex(&mut s, "variable.createCollection", json!({"name": "Borders"}));
    let col = col["id"].as_str().unwrap().to_string();
    ex(&mut s, "variable.create", json!({"collection": col, "name": "border/top", "type": "number", "value": 3}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "strokeTopWeight", "variable": "border/top"}));
    let c = css(&mut s, r);
    assert!(c.contains("border-top: var(--border-top, 3px) solid #000000;"), "{c}");
    assert!(c.contains("border-bottom: 2px solid #000000;"), "{c}");
}

#[test]
fn native_code_for_a_simple_layer() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"cornerRadius": 8}));
    let swift = code(&mut s, r, json!({"language": "swiftui"}));
    let compose = code(&mut s, r, json!({"language": "compose"}));
    let flutter = code(&mut s, r, json!({"language": "flutter"}));
    // (code, expected fragment, derivation)
    let rows: [(&str, &str, &str); 7] = [
        (&swift, "frame(width: 100, height: 50)", "SwiftUI sizes a view with frame, in points"),
        (&swift, "Color(red: 1, green: 0, blue: 0, opacity: 1)", "docs silent: the fill as a SwiftUI colour"),
        (&compose, "Modifier.size(100.dp, 50.dp)", "Compose sizes in dp"),
        (&compose, "RoundedCornerShape(8.dp)", "docs silent: corner radius as a rounded shape"),
        (&flutter, "width: 100,\n  height: 50,", "Flutter sizes a Container in logical pixels"),
        (&flutter, "Color(0xFFFF0000)", "docs silent: the fill as a Flutter colour"),
        (&flutter, "borderRadius: BorderRadius.circular(8)", "docs silent: corner radius as a rounded border"),
    ];
    for (c, want, why) in rows {
        assert!(c.contains(want), "{want}: {why}\n{c}");
    }
}

#[test]
fn asset_list_holds_exports_images_and_icons() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 400})));
    let marked = rect(&mut s, 10.0, 10.0, 50.0, 50.0, Some(f));
    ex(&mut s, "export.add", json!({"ids": [marked]}));
    let png = figcraft_render::encode_png(&figcraft_render::Pixmap::new(2, 2).unwrap()).unwrap();
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    let photo = id(&ex(&mut s, "file.placeImage", json!({"data": data, "x": 500, "y": 0})));
    let v1 = id(&ex(&mut s, "node.create", json!({"type": "star", "x": 200, "y": 10, "width": 12, "height": 12, "parent": f})));
    let v2 = id(&ex(&mut s, "node.create", json!({"type": "polygon", "x": 214, "y": 10, "width": 12, "height": 12, "parent": f})));
    let icon = id(&ex(&mut s, "object.group", json!({"ids": [v1, v2]})));
    set(&mut s, &[icon], json!({"name": "icon/close"}));
    let card = rect(&mut s, 10.0, 200.0, 300.0, 100.0, Some(f));
    let r = ex(&mut s, "inspect.assets", json!({"ids": [f, photo]}));
    let assets = r["assets"].as_array().unwrap();
    let kind = |id: NodeId| assets.iter().find(|a| a["id"] == json!(id)).map(|a| a["kind"].as_str().unwrap_or_default().to_string());
    // (layer, kind, derivation)
    let rows: [(NodeId, Option<&str>, &str); 4] = [
        (marked, Some("export"), "layers with export settings are assets"),
        (photo, Some("image"), "layers filled with an image are assets"),
        (icon, Some("icon"), "small vector-only layers are icons (docs silent on the exact rule: vectors only, at most 128 a side)"),
        (card, None, "a plain rectangle is not an asset"),
    ];
    for (layer, want, why) in rows {
        assert_eq!(kind(layer).as_deref(), want, "{why}");
    }
    assert!(!assets.iter().any(|a| a["id"] == json!(v1)), "an icon's own vectors aren't listed again");
}

#[test]
fn redlines_between_layers() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200})));
    let a = rect(&mut s, 20.0, 20.0, 50.0, 50.0, Some(f));
    let b = rect(&mut s, 120.0, 20.0, 50.0, 50.0, Some(f));
    let c = rect(&mut s, 40.0, 40.0, 50.0, 50.0, Some(f));
    let labels = |s: &mut Session, from: NodeId, to: Option<NodeId>| -> Vec<String> {
        let mut p = json!({"ids": [from]});
        if let Some(to) = to {
            p["to"] = json!(to);
        }
        let r = ex(s, "measure.distances", p);
        let mut v: Vec<String> = r["lines"].as_array().unwrap().iter().filter_map(|l| l["label"].as_str().map(str::to_string)).collect();
        v.sort();
        v
    };
    // (from, to, labels, derivation)
    let rows: [(NodeId, Option<NodeId>, &[&str], &str); 3] = [
        (a, Some(b), &["50"], "side by side: the gap between them"),
        (a, None, &["130", "20", "20", "230"], "with nothing hovered: to each edge of the parent frame"),
        (a, Some(c), &["20", "20", "20", "20"], "overlapping: between matching edges"),
    ];
    for (from, to, want, why) in rows {
        let mut want: Vec<String> = want.iter().map(|w| w.to_string()).collect();
        want.sort();
        assert_eq!(labels(&mut s, from, to), want, "{why}");
    }
}

#[test]
fn inspected_json_reports_the_layer() {
    let mut s = Session::new();
    let r = rect(&mut s, 10.0, 20.0, 80.0, 40.0, None);
    set(&mut s, &[r], json!({"rotation": 15, "opacity": 0.25, "blend": "screen", "cornerRadius": 6}));
    // What `inspect_document` and the control channel return for a layer.
    let n = crate::inspect::node_json(s.doc().unwrap(), r, 0);
    // (field, value, derivation)
    let rows: [(&str, Value, &str); 6] = [
        ("width", json!(80.0), "width"),
        ("height", json!(40.0), "height"),
        ("rotation", json!(15.0), "rotation in degrees"),
        ("opacity", json!(0.25), "opacity as a fraction"),
        ("blend", json!("screen"), "the blend mode"),
        ("cornerRadius", json!(6.0), "the radius"),
    ];
    for (k, want, why) in rows {
        assert_eq!(n[k], want, "{k}: {why}");
    }
}

#[test]
fn rem_conversion_leaves_names_alone() {
    let f = crate::codegen::px_to_rem;
    // Lengths convert, names ending in "px" don't.
    assert_eq!(f("width: 32px; --gap-8px: 1; margin: -8px 0px;", 16.0), "width: 2rem; --gap-8px: 1; margin: -0.5rem 0rem;");
    assert_eq!(f("épx 1px px", 16.0), "épx 0.0625rem px");
    assert_eq!(f("", 16.0), "");
}

#[test]
fn rem_conversion_keeps_the_original_pixels() {
    let f = crate::codegen::px_to_rem;
    assert_eq!(f("padding: 3px;", 16.0), "padding: 0.1875rem;");
    assert_eq!(f("gap: 0.5px;", 16.0), "gap: 0.03125rem;");
}
