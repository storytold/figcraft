//! Shadows in SVG: spread offsets the outline like the renderer (mitred stroke bands, not a
//! morphology filter), negative spread and inner shadows cut with masks; layers without one outline
//! keep the morphology filter. Every export stays valid SVG.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use super::*;

fn with_effects(kind: NodeKind, effects: Vec<EffectKind>) -> (Document, NodeId) {
    let mut d = Document::new("svg fx");
    let p = d.first_page().unwrap();
    let n = d.create(p, kind, "S", 10.0, 10.0, 100.0, 80.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills = vec![Paint::solid(Color::from_hex("3366CC").unwrap())];
    node.effects = effects.into_iter().map(|kind| Effect { kind, visible: true }).collect();
    (d, n)
}

fn shadow(spread: f64) -> Shadow {
    Shadow { color: Color::from_hex("00000080").unwrap(), offset: (3.0, 5.0), radius: 8.0, spread, ..Default::default() }
}

/// Export and check the result parses as SVG.
fn export(d: &Document, id: NodeId) -> String {
    let svg = export_svg(d, &[id], &SvgOptions::default()).unwrap();
    let mut d2 = Document::new("back");
    let p2 = d2.first_page().unwrap();
    import_svg(&mut d2, svg.as_bytes(), p2, 0.0, 0.0, "Imported").unwrap();
    svg
}

#[test]
fn drop_shadow_spread_offsets_the_outline() {
    let star = NodeKind::Star { count: 5, inner_ratio: 0.4 };
    let (d, n) = with_effects(star, vec![EffectKind::DropShadow(shadow(6.0))]);
    let svg = export(&d, n);
    assert!(!svg.contains("feMorphology"), "{svg}");
    assert!(svg.contains("stroke-width=\"12\"") && svg.contains("stroke-linejoin=\"miter\"") && svg.contains("stroke-miterlimit=\"4\""), "{svg}");
    assert!(svg.contains("<feGaussianBlur stdDeviation=\"4\" result=\"b\"/>"), "{svg}");
    // Hidden behind the layer: cut out by an image of its outline.
    assert!(svg.contains("<feComposite in=\"b\" in2=\"cover\" operator=\"out\"/>"), "{svg}");
    assert!(svg.contains("opacity=\"0.502\""), "{svg}");
}

#[test]
fn negative_spread_and_inner_shadows_cut_with_masks() {
    let ellipse = NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 };
    let (d, n) = with_effects(ellipse.clone(), vec![EffectKind::DropShadow(shadow(-4.0))]);
    let svg = export(&d, n);
    assert!(svg.contains("<mask") && svg.contains("stroke=\"black\" stroke-width=\"8\""), "{svg}");
    let (d, n) = with_effects(ellipse, vec![EffectKind::InnerShadow(shadow(4.0))]);
    let svg = export(&d, n);
    assert!(svg.contains("<clipPath") && svg.contains("<mask"), "{svg}");
    assert!(svg.contains("stroke=\"black\" stroke-width=\"8\""), "the shrunk hole: {svg}");
}

#[test]
fn spread_without_an_outline_keeps_the_filter() {
    let mut d = Document::new("g");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 50.0, 50.0).unwrap();
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    d.get_mut(g).unwrap().effects = vec![Effect { kind: EffectKind::DropShadow(shadow(5.0)), visible: true }];
    let svg = export(&d, g);
    assert!(svg.contains("feMorphology") && svg.contains("operator=\"dilate\" radius=\"5\""), "{svg}");
    // No spread: no morphology at all.
    d.get_mut(g).unwrap().effects = vec![Effect { kind: EffectKind::DropShadow(shadow(0.0)), visible: true }];
    assert!(!export(&d, g).contains("feMorphology"));
}

#[test]
fn hostile_shadow_values_still_export() {
    for s in [f64::NAN, f64::INFINITY, -1e12, 1e12] {
        let (d, n) = with_effects(
            NodeKind::Rectangle,
            vec![
                EffectKind::DropShadow(Shadow { spread: s, radius: s, offset: (s, 0.0), ..Shadow::default() }),
                EffectKind::InnerShadow(Shadow { spread: s, radius: s, offset: (0.0, s), ..Shadow::default() }),
            ],
        );
        assert!(export_svg(&d, &[n], &SvgOptions::default()).is_some());
    }
}

/// The filter a spread shadow is blurred in makes room for the spread as the node's transform
/// draws it: on a star scaled three times, the spread's mitres reach three times as far, and on a
/// skewed one further across than down.
#[test]
fn a_scaled_spread_shadow_keeps_its_filter_room() {
    use figcraft_doc::SPREAD_MITER_LIMIT;
    use figcraft_geom::{Affine, kurbo};
    let star = NodeKind::Star { count: 5, inner_ratio: 0.4 };
    let s =
        Shadow { color: Color::from_hex("000000").unwrap(), offset: (6.0, -9.0), radius: 4.0, spread: 10.0, show_behind: true, ..Default::default() };
    for (why, xf) in [
        ("scaled 3", Affine::scale(3.0)),
        ("scaled (3, 1)", Affine::scale_non_uniform(3.0, 1.0)),
        ("skewed", Affine::new([1.0, 0.0, 2.0, 1.0, 0.0, 0.0])),
        ("turned and stretched", Affine::rotate(0.5) * Affine::scale_non_uniform(3.0, 1.0)),
        ("as drawn", Affine::IDENTITY),
    ] {
        let (mut d, n) = with_effects(star.clone(), vec![EffectKind::DropShadow(s.clone())]);
        let t = d.get(n).unwrap().transform;
        d.get_mut(n).unwrap().transform = t * xf;
        let svg = export(&d, n);
        let tag = &svg[svg.find("<filter ").unwrap()..];
        let attr = |name: &str| -> f64 {
            let at = tag.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
            tag[at..at + tag[at..].find('"').unwrap()].parse().unwrap()
        };
        let region = kurbo::Rect::new(attr("x"), attr("y"), attr("x") + attr("width"), attr("y") + attr("height"));
        // What the shadow paints, as the renderer draws it: the outline stroked twice the spread
        // wide with mitred joins, through the node's transform, moved by the offset (in the
        // export's frame, which starts at the node's export bounds) and blurred.
        let bounds = figcraft_render::export_bounds(&d, n).unwrap();
        let at = Affine::translate((s.offset.0 - bounds.x0, s.offset.1 - bounds.y0)) * d.world_transform(n);
        let shape = figcraft_render::node_outline(&d, d.get(n).unwrap()).unwrap();
        let stroke = kurbo::Stroke::new(s.spread * 2.0).with_join(kurbo::Join::Miter).with_miter_limit(SPREAD_MITER_LIMIT);
        let mut grown = kurbo::stroke(shape.0.iter(), &stroke, &Default::default(), 0.01);
        grown.apply_affine(at);
        let blur = s.radius * 1.5;
        let painted = figcraft_geom::Shape::bounding_box(&grown).inflate(blur, blur);
        assert!(region.union(painted) == region, "{why}: the filter {region:?} cuts the shadow at {painted:?}");
    }
}
