//! Brushes and pen pressure through the session: `stroke.brush`, rendering and outlining brush
//! strokes, and the Pencil recording pressure (and its brush) into the stroke it draws.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, pressure_profile};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Point;
use figcraft_geom::kurbo::PathEl;

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

/// A Pencil stroke stays where it was drawn: the fitted curve shot far off the stroke, a spike
/// (#1741, its second point), where the stroke went out and back to the very point it left and
/// where samples ran on nearly straight. Pointer pixels become fractional page points, so going
/// back to a pixel lands on exactly the same point. Both sample runs are ones kurbo's fit took
/// hundreds of units off.
///
/// Each stroke is drawn in a fresh session, with the Pencil and with the Marker and Highlighter
/// of a board, whose ink comes from the same fit. Every point along the stroke drawn must lie
/// within 1.5 (twice the fit's accuracy) of the polyline through the samples.
#[test]
fn the_pencil_keeps_its_stroke_where_it_was_drawn() {
    let (p, q) = ((86.043_565_719_086_97, -173.744_402_349_100_94), (87.851_850_739_335_35, -179.202_846_445_954_52));
    let back = [(80.0, -160.0), p, q, p, (95.0, -185.0)];
    let straight = [
        (204.0, -198.0),
        (209.598_690_794_015_5, -204.909_328_119_739_88),
        (211.277_914_263_870_97, -207.190_974_020_753_4),
        (214.246_506_828_720_84, -211.228_827_590_623_24),
        (218.0, -214.0),
    ];
    for tool in ["pencil", "marker", "highlighter"] {
        for (what, samples) in [("out and back", &back[..]), ("nearly straight", &straight[..])] {
            let what = format!("{tool}, {what}");
            let mut s = if tool == "pencil" { Session::new() } else { crate::board_tests::board() };
            s.execute("tool.set", &json!({"tool": tool})).unwrap();
            let before: Vec<NodeId> = s.doc().unwrap().nodes().map(|n| n.id).collect();
            let (first, last) = (samples[0], samples[samples.len() - 1]);
            s.pointer(&PointerEvent::at(PointerKind::Down, first.0, first.1, 1.0)).unwrap();
            for (x, y) in &samples[1..] {
                s.pointer(&PointerEvent::at(PointerKind::Move, *x, *y, 1.0)).unwrap();
            }
            s.pointer(&PointerEvent::at(PointerKind::Up, last.0, last.1, 1.0)).unwrap();
            // The layer the stroke made, and only it.
            let doc = s.doc().unwrap();
            let made: Vec<&figcraft_doc::Node> =
                doc.nodes().filter(|n| !before.contains(&n.id) && matches!(n.kind, NodeKind::Vector { .. })).collect();
            let [n] = made[..] else { panic!("{what}: {} vectors made", made.len()) };
            let NodeKind::Vector { network } = &n.kind else { panic!("{what}: not a vector") };
            let drawn = doc.world_transform(n.id) * network.stroke_path();
            let polyline = figcraft_geom::BezPath::from_vec(
                samples
                    .iter()
                    .enumerate()
                    .map(|(k, (x, y))| if k == 0 { PathEl::MoveTo(Point::new(*x, *y)) } else { PathEl::LineTo(Point::new(*x, *y)) })
                    .collect(),
            );
            let d = crate::vector_behaviour_tests::strays(&drawn, &polyline);
            assert!(d <= 1.5, "{what}: the stroke strays {d} from the samples: {}", drawn.to_svg());
            let samples_box = figcraft_geom::points_bounds(samples.iter().map(|(x, y)| Point::new(*x, *y))).unwrap();
            let node_box = doc.world_bounds(n.id).unwrap();
            if tool == "pencil" {
                assert!(samples_box.inflate(1.5, 1.5).contains_rect(node_box), "{what}: the layer {node_box:?}, the samples {samples_box:?}");
            }
            // The turn is a point of the stroke, where it was drawn.
            if samples == back {
                let xf = doc.world_transform(n.id);
                assert!(
                    network.vertices.iter().any(|v| (xf * Point::new(v.x, v.y) - Point::new(q.0, q.1)).hypot() < 1e-6),
                    "{what}: no point at the turn"
                );
            }
        }
    }
}
