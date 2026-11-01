use super::*;
use figcraft_doc::{StrokeAlign, StrokeCap, StrokeJoin};
use figcraft_geom::{Affine, Shape};

fn stroked(align: StrokeAlign) -> (Document, NodeId) {
    let (mut d, p) = doc();
    let r = d.create(p, NodeKind::Rectangle, "R", 10.0, 10.0, 40.0, 30.0).unwrap();
    let n = d.get_mut(r).unwrap();
    n.fills.clear();
    n.strokes = vec![Paint::solid(hex("112233"))];
    n.stroke.weight = 4.0;
    n.stroke.align = align;
    (d, r)
}

#[test]
fn a_centred_stroke_is_drawn_by_android() {
    let (mut d, r) = stroked(StrokeAlign::Center);
    {
        let n = d.get_mut(r).unwrap();
        n.stroke.join = StrokeJoin::Bevel;
        n.stroke.cap_start = StrokeCap::Round;
        n.stroke.cap_end = StrokeCap::Round;
    }
    let out = export(&d, &[r], Compat::BestEffort);
    let paths = paths_of(&out.xml);
    assert_eq!(paths.len(), 1, "{}", out.xml);
    let s = &paths[0];
    assert_eq!((s.get("strokeColor"), s.get("strokeWidth")), (Some("#112233"), Some("4")));
    assert_eq!((s.get("strokeLineCap"), s.get("strokeLineJoin")), (Some("round"), Some("bevel")));
    assert!(s.get("fillColor").is_none() && s.get("strokeMiterLimit").is_none());
    // The viewport grows by half the stroke on each side.
    assert_eq!(elements(&out.xml)[0].get("width"), Some("44dp"));
    assert_eq!(bounds(s.get("pathData").unwrap()), (2.0, 2.0, 42.0, 32.0));
}

#[test]
fn a_miter_limit_other_than_four_is_written() {
    let (mut d, r) = stroked(StrokeAlign::Center);
    d.get_mut(r).unwrap().stroke.miter_limit = 10.0;
    let out = export(&d, &[r], Compat::BestEffort);
    assert_eq!(paths_of(&out.xml)[0].get("strokeMiterLimit"), Some("10"));
}

#[test]
fn a_huge_miter_limit_is_written_as_a_finite_number() {
    let (mut d, r) = stroked(StrokeAlign::Center);
    d.get_mut(r).unwrap().stroke.miter_limit = 1e306;
    let out = export(&d, &[r], Compat::BestEffort);
    let v = paths_of(&out.xml)[0].get("strokeMiterLimit").unwrap().to_string();
    assert!(v.parse::<f32>().unwrap().is_finite(), "{v}");
}

#[test]
fn a_uniform_scale_scales_the_stroke_width() {
    let (mut d, r) = stroked(StrokeAlign::Center);
    let n = d.get_mut(r).unwrap();
    n.transform *= Affine::rotate(0.4) * Affine::scale(2.5);
    let out = export(&d, &[r], Compat::BestEffort);
    assert_eq!(paths_of(&out.xml)[0].get("strokeWidth"), Some("10"));
}

#[test]
fn strokes_android_cant_draw_are_their_filled_outline() {
    type Edit = fn(&mut figcraft_doc::Node);
    let cases: [(&str, Edit); 4] = [
        ("inside", |n| n.stroke.align = StrokeAlign::Inside),
        ("outside", |n| n.stroke.align = StrokeAlign::Outside),
        ("dashed", |n| n.stroke.dashes = vec![6.0, 3.0]),
        ("stretched", |n| n.transform *= Affine::scale_non_uniform(2.0, 1.0)),
    ];
    for (what, edit) in cases {
        let (mut d, r) = stroked(StrokeAlign::Center);
        edit(d.get_mut(r).unwrap());
        for compat in [Compat::BestEffort, Compat::Exact, Compat::Ignore] {
            let out = export(&d, &[r], compat);
            let paths = paths_of(&out.xml);
            assert!(!out.xml.contains("strokeColor"), "{what}:\n{}", out.xml);
            assert!(!paths.is_empty() && paths.iter().all(|p| p.get("fillColor") == Some("#112233")), "{what}:\n{}", out.xml);
            // Rings fill even-odd in FigCraft: rewritten, or kept as the rule in exact/ignore.
            let ring = what == "inside" || what == "outside";
            let kept = paths.iter().any(|p| p.get("fillType") == Some("evenOdd"));
            match compat {
                Compat::BestEffort => assert!(!kept, "{what}: rewritten\n{}", out.xml),
                _ => assert_eq!(kept, ring, "{what} {compat:?}\n{}", out.xml),
            }
            if compat == Compat::BestEffort && ring {
                assert_eq!(out.warnings, ["even-odd fills were rewritten as non-zero (R)"], "{what}");
            } else {
                assert!(out.warnings.is_empty(), "{what} {compat:?}: {:?}", out.warnings);
            }
        }
    }
}

#[test]
fn an_inside_stroke_outline_covers_only_the_ring() {
    let (d, r) = stroked(StrokeAlign::Inside);
    let out = export(&d, &[r], Compat::BestEffort);
    let data = paths_of(&out.xml)[0].get("pathData").unwrap().to_string();
    let path = parse_path(&data);
    // Viewport = the rectangle itself (inside strokes don't grow it): (0,0)–(40,30).
    assert_ne!(path.winding((1.0, 1.0).into()), 0, "on the ring");
    assert_eq!(path.winding((20.0, 15.0).into()), 0, "the hole");
    assert_eq!(path.winding((-1.0, 15.0).into()), 0, "outside");
}

#[test]
fn a_line_stroke_is_native_with_its_caps() {
    let (mut document, page) = doc();
    let line = document.create(page, NodeKind::Line, "L", 0.0, 0.0, 50.0, 0.0).unwrap();
    let node = document.get_mut(line).unwrap();
    node.strokes = vec![Paint::solid(hex("000000"))];
    node.stroke.weight = 2.0;
    node.stroke.cap_start = StrokeCap::Square;
    node.stroke.cap_end = StrokeCap::Square;
    let out = export(&document, &[line], Compat::BestEffort);
    let path = paths_of(&out.xml)[0].clone();
    assert_eq!((path.get("strokeLineCap"), path.get("strokeWidth")), (Some("square"), Some("2")), "{}", out.xml);
}

#[test]
fn a_gradient_stroke_is_an_outline_filled_with_the_gradient() {
    let (mut d, r) = stroked(StrokeAlign::Center);
    d.get_mut(r).unwrap().strokes = vec![Paint::gradient(figcraft_color::Gradient::default_for(figcraft_color::GradientKind::Linear, hex("FF0000")))];
    let out = export(&d, &[r], Compat::BestEffort);
    assert!(!out.xml.contains("strokeColor") && out.xml.contains("android:type=\"linear\""), "{}", out.xml);
}

/// Parse path data as written ("M0,0 L1,0 C… Z") back into a path.
pub(super) fn parse_path(source: &str) -> figcraft_geom::BezPath {
    let mut path = figcraft_geom::BezPath::new();
    let mut command = ' ';
    let mut points: Vec<figcraft_geom::Point> = Vec::new();
    let flush = |command: char, points: &mut Vec<figcraft_geom::Point>, path: &mut figcraft_geom::BezPath| {
        match (command, points.as_slice()) {
            (' ', []) => {}
            ('M', [start]) => path.move_to(*start),
            ('L', [end]) => path.line_to(*end),
            ('Q', [control, end]) => path.quad_to(*control, *end),
            ('C', [control1, control2, end]) => path.curve_to(*control1, *control2, *end),
            ('Z', []) => path.close_path(),
            other => panic!("bad segment {other:?} in {source}"),
        }
        points.clear();
    };
    for token in source.split(' ') {
        let mut rest = token;
        if let Some(new_command) = token.chars().next().filter(char::is_ascii_alphabetic) {
            flush(command, &mut points, &mut path);
            command = new_command;
            rest = &token[1..];
        }
        if let Some((x, y)) = rest.split_once(',') {
            points.push(figcraft_geom::Point::new(x.parse().unwrap(), y.parse().unwrap()));
        }
    }
    flush(command, &mut points, &mut path);
    path
}
