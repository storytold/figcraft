//! A design drawn before and after a `.fig` export and import looks the same: the round trip is
//! measured in pixels with the `.fig` fidelity harness (`figcraft_format::fig::fidelity`).

use figcraft_color::{Color, Gradient, GradientKind, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeKind, Shadow};
use figcraft_format::fig::fidelity::compare_rgba;

use super::*;

/// A clipping, rounded, outlined card holding a gradient band, an ellipse casting a soft shadow,
/// a rotated square and a translucent overlay crossing them.
fn scene() -> Document {
    let mut d = Document::new("round trip");
    let p = d.first_page().unwrap();
    let hex = |h: &str| Color::from_hex(h).unwrap();
    let card = d.create(p, NodeKind::Frame, "Card", 10.0, 10.0, 180.0, 180.0).unwrap();
    {
        let n = d.get_mut(card).unwrap();
        n.fills = vec![Paint::solid(hex("F4F1EA"))];
        n.strokes = vec![Paint::solid(hex("303030"))];
        n.stroke.weight = 2.0;
        n.corners.radii = [16.0; 4];
        n.clip = true;
    }
    let band = d.create(card, NodeKind::Rectangle, "Band", 0.0, 0.0, 180.0, 60.0).unwrap();
    d.get_mut(band).unwrap().fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, hex("2050D0")))];
    let dot = d.create(card, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Dot", 30.0, 80.0, 70.0, 70.0).unwrap();
    {
        let n = d.get_mut(dot).unwrap();
        n.fills = vec![Paint::solid(hex("E04040"))];
        let shadow = Shadow { color: Color::new(0.0, 0.0, 0.0, 0.4), offset: (4.0, 6.0), radius: 6.0, ..Shadow::default() };
        n.effects = vec![Effect { kind: EffectKind::DropShadow(shadow), visible: true }];
    }
    let square = d.create(card, NodeKind::Rectangle, "Square", 110.0, 90.0, 50.0, 50.0).unwrap();
    {
        let n = d.get_mut(square).unwrap();
        n.fills = vec![Paint::solid(hex("20A060"))];
        n.transform *= figcraft_doc::geom::Affine::rotate_about(0.4, (25.0, 25.0));
    }
    let veil = d.create(card, NodeKind::Rectangle, "Veil", 60.0, 40.0, 80.0, 100.0).unwrap();
    {
        let n = d.get_mut(veil).unwrap();
        n.fills = vec![Paint::solid(hex("FFD020"))];
        n.opacity = 0.5;
    }
    d
}

/// The page's layers on a white 200 × 200 view at 1×.
fn draw(d: &Document) -> Pixmap {
    let p = d.first_page().unwrap();
    let kids = d.node(p).unwrap().children.clone();
    let mut pix = Pixmap::new(200, 200).unwrap();
    pix.fill(tiny_skia::Color::WHITE);
    render_nodes_into(d, &kids, &View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 }, &mut pix);
    pix
}

#[test]
fn a_design_looks_the_same_after_a_fig_round_trip() {
    let before = scene();
    let (bytes, _) = figcraft_format::fig::export(&before, None).unwrap();
    let (after, _) = figcraft_format::fig::import(&bytes, "round trip").unwrap();
    let (a, b) = (draw(&before), draw(&after));
    let report = compare_rgba(a.data(), b.data(), a.width(), a.height(), 2).unwrap();
    println!("{}\n{}", report.to_json(), report.to_markdown());
    assert!(report.ssim > 0.99, "{}", report.to_json());
    assert!(report.psnr_db > 40.0, "{}", report.to_json());
    // The scene is not blank: it differs from an empty page as a broken import would.
    let mut blank = Pixmap::new(200, 200).unwrap();
    blank.fill(tiny_skia::Color::WHITE);
    let empty = compare_rgba(a.data(), blank.data(), 200, 200, 2).unwrap();
    assert!(empty.ssim < 0.9 && empty.diff_pct > 10.0, "{}", empty.to_json());
}
