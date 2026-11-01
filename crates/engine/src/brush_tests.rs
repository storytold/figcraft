//! Brushes and pen pressure through the session: `stroke.brush`, rendering and outlining brush
//! strokes, and the Pencil recording pressure (and its brush) into the stroke it draws.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, pressure_profile};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Point;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A 200-long horizontal vector with a 10 px black stroke.
fn line(s: &mut Session) -> NodeId {
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 0, "y": 0})).unwrap());
    s.execute("node.set", &json!({"id": v, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center"}})).unwrap();
    v
}

/// The alpha of the page's layers at page point (`x`, `y`), rendered at 1x without the page colour.
fn alpha(s: &Session, x: f64, y: f64) -> u8 {
    let doc = s.doc().unwrap();
    let view = figcraft_render::View { origin: (x - 1.0, y - 1.0), scale: 1.0, width: 3, height: 3 };
    let mut pix = figcraft_render::Pixmap::new(3, 3).unwrap();
    figcraft_render::render_nodes_into(doc, &doc.node(s.page().unwrap()).unwrap().children, &view, &mut pix);
    pix.data()[(3 + 1) * 4 + 3]
}

#[test]
fn brushes_draw_along_the_stroke_and_outline_to_vectors() {
    let mut s = Session::new();
    let v = line(&mut s);
    assert_eq!(alpha(&s, 15.0, 0.0), 255, "a plain stroke covers the line");
    assert!(s.execute("stroke.brush", &json!({"ids": [v], "preset": "glitter"})).is_err());
    // Dots every 2 weights: centres at 10, 30, 50 … with gaps between.
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    assert_eq!(alpha(&s, 10.0, 0.0), 255);
    assert_eq!(alpha(&s, 20.0, 0.0), 0, "the gap between two dots");
    let b = s.execute("node.get", &json!({"id": v})).unwrap()["brush"].clone();
    assert_eq!((b["preset"].as_str(), b["spacing"].as_f64()), (Some("dots"), Some(2.0)), "{b}");
    // A stretch brush tapers to a point at both ends.
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "taper"})).unwrap();
    assert_eq!(alpha(&s, 100.0, 2.0), 255);
    assert_eq!(alpha(&s, 2.0, 3.0), 0);
    // Outline stroke turns the brush into a filled vector.
    s.execute("vector.outlineStroke", &json!({"ids": [v]})).unwrap();
    let doc = s.doc().unwrap();
    let outlined = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. }) && !n.fills.is_empty()).expect("an outlined vector");
    assert!(outlined.strokes.is_empty() || outlined.stroke.brush.is_none());
    assert_eq!(alpha(&s, 100.0, 2.0), 255, "looks the same after outlining");
    // Removing the brush: a plain stroke again.
    let mut s = Session::new();
    let v = line(&mut s);
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "dots", "spacing": 2})).unwrap();
    s.execute("stroke.brush", &json!({"ids": [v], "preset": null})).unwrap();
    assert_eq!(alpha(&s, 20.0, 0.0), 255);
    // Exports draw it too.
    s.execute("stroke.brush", &json!({"ids": [v], "preset": "stars"})).unwrap();
    let doc = s.doc().unwrap();
    assert!(figcraft_svg::export_svg(doc, &[v], &figcraft_svg::SvgOptions::default()).unwrap().contains("<path"));
    assert!(figcraft_pdf::export_nodes_pdf(doc, &[v]).is_ok());
}

#[test]
fn pressure_becomes_a_width_profile() {
    let pts: Vec<Point> = (0..=10).map(|i| Point::new(f64::from(i) * 10.0, 0.0)).collect();
    assert!(pressure_profile(&pts, &[None; 11]).is_none(), "a mouse: a plain stroke");
    let light_to_hard: Vec<Option<f32>> = (0..=10).map(|i| Some(i as f32 / 10.0)).collect();
    let prof = pressure_profile(&pts, &light_to_hard).unwrap();
    assert_eq!(prof.len(), 11);
    assert_eq!(prof.first(), Some(&[0.0, 0.1]), "no pressure still draws a hairline");
    assert_eq!(prof.get(5), Some(&[0.5, 1.0]), "medium pressure: the stroke weight");
    assert_eq!(prof.last(), Some(&[1.0, 2.0]));
    assert!(pressure_profile(&[Point::ZERO, Point::ZERO], &[Some(0.5), Some(0.5)]).is_none(), "no length");
}

#[test]
fn the_pencil_records_pressure_and_its_brush() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    assert!(s.execute("tool.pencilBrush", &json!({"preset": "nope"})).is_err());
    s.execute("tool.pencilBrush", &json!({"preset": "taper"})).unwrap();
    let ev = |kind, x: f64, p: f32| PointerEvent { pressure: Some(p), ..PointerEvent::at(kind, x, 50.0 + (x / 20.0).sin() * 10.0, 1.0) };
    s.pointer(&ev(PointerKind::Down, 0.0, 0.2)).unwrap();
    for i in 1..=40 {
        s.pointer(&ev(PointerKind::Move, f64::from(i) * 5.0, 0.2 + 0.02 * i as f32)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a pencil stroke");
    let prof = n.stroke.profile().expect("pressure recorded");
    assert!(prof.scale_at(0.0) < 0.5 && prof.scale_at(1.0) > 1.9, "light to hard: {prof:?}");
    assert_eq!(n.stroke.brush.as_ref().map(|b| b.preset.as_str()), Some("taper"));
    // A mouse (no pressure) draws a plain uniform stroke.
    s.execute("tool.pencilBrush", &json!({"preset": null})).unwrap();
    s.execute("edit.selectAll", &json!({})).ok();
    let before = s.doc().unwrap().nodes().count();
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 50.0, 220.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 200.0, 1.0)).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.nodes().count(), before + 1);
    let last = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Vector { .. })).max_by_key(|n| n.id).unwrap();
    assert!(last.stroke.profile().is_none() && last.stroke.brush.is_none());
}

/// The Pencil draws black on a light page and white on a dark page or inside a dark frame, the
/// rule new text follows (#1741; Figma: "in black, unless you're sketching on a dark canvas or
/// frame"), and the stroke in progress previews in that colour.
#[test]
fn the_pencil_draws_white_on_a_dark_background() {
    use figcraft_color::Color;
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    // A stroke from (x, y); its colour, and the preview colour the overlay carried while drawing.
    let stroke = |s: &mut Session, x: f64, y: f64| {
        s.pointer(&PointerEvent::at(PointerKind::Down, x, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Move, x + 40.0, y + 10.0, 1.0)).unwrap();
        let preview = s.tool.overlay.pen_color;
        s.pointer(&PointerEvent::at(PointerKind::Move, x + 80.0, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, x + 80.0, y, 1.0)).unwrap();
        assert!(s.tool.overlay.pen_color.is_none(), "no preview colour after the stroke");
        let doc = s.doc().unwrap();
        let last = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Vector { .. })).max_by_key(|n| n.id).unwrap();
        (last.strokes.first().and_then(|p| p.solid_color()).unwrap(), preview)
    };
    assert_eq!(stroke(&mut s, 0.0, 0.0), (Color::BLACK, Some(Color::BLACK)), "a light page");
    s.execute("page.setBackground", &json!({"color": "#1E1E1E"})).unwrap();
    assert_eq!(stroke(&mut s, 0.0, 100.0), (Color::WHITE, Some(Color::WHITE)), "a dark page");
    // A dark frame on a light page, and a light frame on a dark page.
    s.execute("page.setBackground", &json!({"color": "#F5F5F5"})).unwrap();
    s.execute("node.create", &json!({"type": "frame", "x": 300, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#202020"]}})).unwrap();
    assert_eq!(stroke(&mut s, 320.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "inside a dark frame");
    s.execute("page.setBackground", &json!({"color": "#1E1E1E"})).unwrap();
    s.execute("node.create", &json!({"type": "frame", "x": 600, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#FFFFFF"]}})).unwrap();
    assert_eq!(stroke(&mut s, 620.0, 320.0), (Color::BLACK, Some(Color::BLACK)), "inside a light frame on a dark page");
    // A frame without a fill, or with a see-through one, is looked through to what is behind it.
    s.execute("node.create", &json!({"type": "frame", "x": 900, "y": 300, "width": 200, "height": 200, "props": {"fills": []}})).unwrap();
    assert_eq!(stroke(&mut s, 920.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "a frame without a fill on a dark page");
    s.execute("node.create", &json!({"type": "frame", "x": 1200, "y": 300, "width": 200, "height": 200, "props": {"fills": ["#FFFFFF40"]}})).unwrap();
    assert_eq!(stroke(&mut s, 1220.0, 320.0), (Color::WHITE, Some(Color::WHITE)), "a mostly transparent light frame on a dark page");
    // The threshold is Color::is_dark: white reads better than black from #757575 down.
    for (bg, want) in [("#767676", Color::BLACK), ("#757575", Color::WHITE)] {
        s.execute("page.setBackground", &json!({"color": bg})).unwrap();
        assert_eq!(stroke(&mut s, 0.0, 600.0).0, want, "on {bg}");
    }
}
