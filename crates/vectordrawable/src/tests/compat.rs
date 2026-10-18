use super::*;
use crate::paint::nonzero;
use figcraft_color::BlendMode;
use figcraft_doc::{Effect, MaskType};
use figcraft_geom::{BezPath, Point, Shape};

fn fills(xml: &str) -> Vec<String> {
    tagged(&elements(xml), "path").iter().filter_map(|e| e.get("fillColor").map(str::to_string)).collect()
}

/// A 50% frame over two rectangles, apart or overlapping.
fn faded(overlap: bool) -> (Document, NodeId) {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "Card", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().fills.clear();
    d.get_mut(f).unwrap().opacity = 0.5;
    rect(&mut d, f, "A", 0.0, 0.0, 40.0, 40.0, "FF0000");
    let x = if overlap { 20.0 } else { 60.0 };
    rect(&mut d, f, "B", x, 20.0, 40.0, 40.0, "0000FF");
    (d, f)
}

#[test]
fn opacity_over_shapes_apart_is_exact_in_every_mode() {
    let (d, f) = faded(false);
    for compat in [Compat::BestEffort, Compat::Exact, Compat::Ignore] {
        let out = export(&d, &[f], compat);
        assert_eq!(fills(&out.xml), ["#80FF0000", "#800000FF"], "{compat:?}");
        assert!(out.warnings.is_empty(), "{compat:?}: {:?}", out.warnings);
    }
}

#[test]
fn opacity_over_overlapping_shapes_per_mode() {
    let (d, f) = faded(true);
    let best = export(&d, &[f], Compat::BestEffort);
    assert_eq!(fills(&best.xml), ["#80FF0000", "#800000FF"]);
    assert_eq!(best.warnings, ["layer opacity was applied to each shape: where shapes overlap they look denser (Card)"]);
    let exact = export(&d, &[f], Compat::Exact);
    assert!(fills(&exact.xml).is_empty(), "{}", exact.xml);
    assert_eq!(
        exact.warnings,
        [
            "layers with opacity over overlapping shapes were left out (exact mode) (Card)",
            "nothing in it could be drawn: the drawable is empty (Card)"
        ]
    );
    let ignore = export(&d, &[f], Compat::Ignore);
    assert_eq!(fills(&ignore.xml), ["#FF0000", "#0000FF"]);
    assert!(ignore.warnings.is_empty());
}

#[test]
fn a_faded_leaf_with_a_fill_and_a_stroke_overlaps_itself() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 40.0, 40.0, "FF0000");
    let n = d.get_mut(r).unwrap();
    n.strokes = vec![Paint::solid(hex("000000"))];
    n.stroke.weight = 2.0;
    n.stroke.align = figcraft_doc::StrokeAlign::Center;
    n.opacity = 0.5;
    let w = export(&d, &[r], Compat::BestEffort).warnings;
    assert_eq!(w, ["layer opacity was applied to each shape: where shapes overlap they look denser (R)"]);
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 40.0, 40.0, "FF0000");
    d.get_mut(r).unwrap().opacity = 0.5;
    let out = export(&d, &[r], Compat::Exact);
    assert_eq!((fills(&out.xml), out.warnings), (vec!["#80FF0000".to_string()], vec![]));
}

#[test]
fn even_odd_fills_per_mode() {
    let (mut d, p) = doc();
    let a = rect(&mut d, p, "A", 0.0, 0.0, 60.0, 60.0, "000000");
    let b = rect(&mut d, p, "B", 20.0, 20.0, 20.0, 20.0, "000000");
    let bo = d.create(p, NodeKind::BooleanOp { op: figcraft_doc::BoolOp::Exclude }, "Ring", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.reparent(a, bo, None).unwrap();
    d.reparent(b, bo, None).unwrap();
    d.fit_to_children(bo).unwrap();
    d.get_mut(bo).unwrap().fills = vec![Paint::solid(hex("336699"))];
    let best = export(&d, &[bo], Compat::BestEffort);
    let path = paths_of(&best.xml)[0].clone();
    assert!(path.get("fillType").is_none(), "{}", best.xml);
    let ring = super::strokes::parse_path(path.get("pathData").unwrap());
    assert_ne!(ring.winding(Point::new(5.0, 5.0)), 0);
    assert_eq!(ring.winding(Point::new(30.0, 30.0)), 0, "the hole stays a hole under non-zero");
    // Exact keeps the rule (AndroidX draws it), as does ignore.
    for compat in [Compat::Exact, Compat::Ignore] {
        let out = export(&d, &[bo], compat);
        assert_eq!(paths_of(&out.xml)[0].get("fillType"), Some("evenOdd"), "{compat:?}");
        assert!(out.warnings.is_empty());
    }
    assert_eq!(best.warnings, ["even-odd fills were rewritten as non-zero (Ring)"]);
}

/// Even-odd inside-ness of `p` against non-zero inside-ness of `nonzero(p)`, on a grid.
fn same_area(p: &BezPath) {
    let q = nonzero(p).unwrap();
    let b = p.bounding_box();
    let mut inside = 0;
    for i in 0..60 {
        for j in 0..60 {
            let pt = Point::new(b.x0 + b.width() * (i as f64 + 0.37) / 60.0, b.y0 + b.height() * (j as f64 + 0.41) / 60.0);
            let even = p.winding(pt) % 2 != 0;
            assert_eq!(even, q.winding(pt) != 0, "at {pt:?}");
            inside += usize::from(even);
        }
    }
    assert!(inside > 100, "{inside}");
}

#[test]
fn the_non_zero_rewrite_fills_the_same_area() {
    // A pentagram: the middle pentagon is winding 2, outside under even-odd.
    let mut star = BezPath::new();
    for k in 0..5 {
        let a = std::f64::consts::TAU * (k * 2 % 5) as f64 / 5.0 - std::f64::consts::FRAC_PI_2;
        let pt = Point::new(50.0 + 40.0 * a.cos(), 50.0 + 40.0 * a.sin());
        if k == 0 { star.move_to(pt) } else { star.line_to(pt) }
    }
    star.close_path();
    same_area(&star);
    // Nested squares wound the same way (three levels) and an overlapping circle.
    let mut nest = BezPath::new();
    for (o, s) in [(0.0, 90.0), (15.0, 60.0), (30.0, 30.0)] {
        nest.extend(figcraft_geom::Rect::new(o, o, o + s, o + s).to_path(0.1));
    }
    nest.extend(figcraft_geom::kurbo::Circle::new((80.0, 80.0), 25.0).to_path(0.1));
    same_area(&nest);
}

#[test]
fn alpha_masks_per_mode() {
    let (mut d, p) = doc();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, ellipse(), "Fade", 10.0, 10.0, 80.0, 80.0).unwrap();
    d.get_mut(m).unwrap().is_mask = true;
    d.get_mut(m).unwrap().mask_type = MaskType::Alpha;
    d.get_mut(m).unwrap().fills = vec![Paint::solid(hex("80808080"))];
    rect(&mut d, g, "Content", 0.0, 0.0, 100.0, 100.0, "2255FF");
    let best = export(&d, &[g], Compat::BestEffort);
    assert!(best.xml.contains("<clip-path") && best.xml.contains("#2255FF"), "{}", best.xml);
    assert_eq!(best.warnings, ["alpha and luminance masks clip to the mask's outline (Fade)"]);
    let exact = export(&d, &[g], Compat::Exact);
    assert!(!exact.xml.contains("#2255FF"), "{}", exact.xml);
    assert_eq!(
        exact.warnings,
        ["layers under alpha or luminance masks were left out (exact mode) (Fade)", "nothing in it could be drawn: the drawable is empty (G)"]
    );
    let ignore = export(&d, &[g], Compat::Ignore);
    assert!(ignore.xml.contains("<clip-path") && ignore.warnings.is_empty());
}

#[test]
fn what_has_no_equivalent_is_left_out_and_named_once_per_kind() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 200.0, 200.0).unwrap();
    d.get_mut(f).unwrap().fills.clear();
    for i in 0..5 {
        let r = rect(&mut d, f, &format!("Shadow {i}"), 40.0 * i as f64, 0.0, 30.0, 30.0, "FF0000");
        d.get_mut(r).unwrap().effects = vec![Effect::drop_shadow()];
    }
    let b = rect(&mut d, f, "", 0.0, 100.0, 30.0, 30.0, "00FF00");
    d.get_mut(b).unwrap().blend = BlendMode::Multiply;
    d.get_mut(b).unwrap().effects = vec![Effect::layer_blur()];
    let img = rect(&mut d, f, "Photo", 100.0, 100.0, 30.0, 30.0, "000000");
    d.get_mut(img).unwrap().fills = vec![Paint::image("abc", figcraft_color::ImageScaleMode::Fill)];
    let out = export(&d, &[f], Compat::BestEffort);
    assert_eq!(
        out.warnings,
        [
            "drop shadows were left out (Shadow 0, Shadow 1, Shadow 2, 2 more)",
            "layer blurs were left out (unnamed layer)",
            "blend modes were drawn as normal (unnamed layer)",
            "image fills were left out (Photo)",
        ]
    );
    // The shapes themselves are still drawn; the image fill isn't.
    assert_eq!(fills(&out.xml).len(), 6, "{}", out.xml);
    assert!(export(&d, &[f], Compat::Ignore).warnings.is_empty());
    assert_eq!(export(&d, &[f], Compat::Exact).warnings.len(), 4);
}

#[test]
fn hidden_and_transparent_layers_paint_nothing() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().fills.clear();
    let a = rect(&mut d, f, "A", 0.0, 0.0, 10.0, 10.0, "FF0000");
    let b = rect(&mut d, f, "B", 20.0, 0.0, 10.0, 10.0, "00FF00");
    rect(&mut d, f, "C", 40.0, 0.0, 10.0, 10.0, "0000FF");
    d.get_mut(a).unwrap().visible = false;
    d.get_mut(b).unwrap().opacity = 0.0;
    assert_eq!(fills(&export(&d, &[f], Compat::Exact).xml), ["#0000FF"]);
}

#[test]
fn huge_coordinates_are_left_out_with_a_warning() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().clip = false;
    d.get_mut(f).unwrap().fills.clear();
    rect(&mut d, f, "Near", 0.0, 0.0, 10.0, 10.0, "FF0000");
    rect(&mut d, f, "Far", 3.0e7, 0.0, 10.0, 10.0, "00FF00");
    let opts = VdOptions { bounds: Some(figcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0)), ..Default::default() };
    let out = export_with(&d, &[f], opts);
    assert_eq!(fills(&out.xml), ["#FF0000"]);
    assert_eq!(out.warnings, ["shapes with coordinates beyond ±10,000,000 were left out (Far)"]);
}

/// A group masked by an even-odd outline (an Exclude boolean ring as a vector mask).
fn ring_masked() -> (Document, NodeId) {
    let (mut d, p) = doc();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 60.0, 60.0).unwrap();
    let bo = d.create(g, NodeKind::BooleanOp { op: figcraft_doc::BoolOp::Exclude }, "Ring", 0.0, 0.0, 1.0, 1.0).unwrap();
    let a = rect(&mut d, bo, "A", 0.0, 0.0, 60.0, 60.0, "000000");
    let b = rect(&mut d, bo, "B", 20.0, 20.0, 20.0, 20.0, "000000");
    let _ = (a, b);
    d.fit_to_children(bo).unwrap();
    d.get_mut(bo).unwrap().is_mask = true;
    d.get_mut(bo).unwrap().mask_type = MaskType::Vector;
    rect(&mut d, g, "Content", 0.0, 0.0, 60.0, 60.0, "2255FF");
    (d, g)
}

fn clip_of(xml: &str) -> BezPath {
    let els = elements(xml);
    let clips = tagged(&els, "clip-path");
    assert_eq!(clips.len(), 1, "{xml}");
    super::strokes::parse_path(clips[0].get("pathData").unwrap())
}

#[test]
fn even_odd_clips_per_mode() {
    let (d, g) = ring_masked();
    // bestEffort and exact both rewrite the outline: the hole stays out of the clip.
    for compat in [Compat::BestEffort, Compat::Exact] {
        let out = export(&d, &[g], compat);
        let clip = clip_of(&out.xml);
        assert_ne!(clip.winding(Point::new(5.0, 5.0)), 0, "{compat:?}");
        assert_eq!(clip.winding(Point::new(30.0, 30.0)), 0, "{compat:?}: the hole stays out of the clip");
        assert!(out.xml.contains("#2255FF"), "{compat:?}");
        let want: &[&str] = if compat == Compat::BestEffort { &["even-odd clip outlines were rewritten as non-zero (Ring)"] } else { &[] };
        assert_eq!(out.warnings, want, "{compat:?}");
    }
    // ignore writes the outline as it is (no rewrite), silently.
    let out = export(&d, &[g], Compat::Ignore);
    assert!(clip_of(&out.xml).winding(Point::new(5.0, 5.0)) != 0 && out.xml.contains("#2255FF"), "{}", out.xml);
    assert!(out.warnings.is_empty());
}

#[test]
fn an_export_that_draws_nothing_says_so() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "Spacer", 0.0, 0.0, 10.0, 10.0, "FF0000");
    d.get_mut(r).unwrap().fills.clear();
    let out = export(&d, &[r], Compat::BestEffort);
    assert!(out.xml.contains("<vector"), "{}", out.xml);
    assert_eq!(out.warnings, ["nothing in it could be drawn: the drawable is empty (Spacer)"]);
    assert!(export(&d, &[r], Compat::Ignore).warnings.is_empty());
}
