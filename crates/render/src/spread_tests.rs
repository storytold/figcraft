//! Shadow spread offsets the shape's outline: ellipses grow as ellipses (not squares), star points
//! stay sharp and reach out along their tips, inner shadows shrink the outline the same way. The
//! renderer and the PDF export share the shadow code (`shadow`), so they agree.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, NodeId, NodeKind, Shadow};

use super::*;

fn shadow(spread: f64) -> Shadow {
    Shadow { color: Color::BLACK, offset: (0.0, 0.0), radius: 0.0, spread, show_behind: true, ..Default::default() }
}

/// A white shape of `kind` at (100, 100), 200 × 200, on a transparent page, with `effects`.
fn shape(kind: NodeKind, effects: Vec<EffectKind>) -> (Document, NodeId) {
    let mut d = Document::new("spread");
    let p = d.first_page().unwrap();
    let n = d.create(p, kind, "S", 100.0, 100.0, 200.0, 200.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills = vec![Paint::solid(Color::WHITE)];
    node.effects = effects.into_iter().map(|kind| Effect { kind, visible: true }).collect();
    (d, n)
}

/// Node `id` drawn on a transparent 400 × 400 view of the page.
fn render(d: &Document, id: NodeId) -> Pixmap {
    let mut pix = Pixmap::new(400, 400).unwrap();
    render_nodes_into(d, &[id], &View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 400 }, &mut pix);
    pix
}

/// Alpha at the page point `(x, y)` (pixel centres at +0.5).
fn alpha(p: &Pixmap, x: f64, y: f64) -> u8 {
    let i = ((y as u32 * p.width() + x as u32) * 4) as usize;
    p.data()[i + 3]
}

fn ellipse() -> NodeKind {
    NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }
}

#[test]
fn drop_shadow_spread_grows_an_ellipse_as_an_ellipse() {
    let (d, n) = shape(ellipse(), vec![EffectKind::DropShadow(shadow(20.0))]);
    let p = render(&d, n);
    // Along the diagonal, 100 from the centre (200, 200) is the edge: the shadow ends 20 further.
    let at = |r: f64| alpha(&p, 200.0 + r * std::f64::consts::FRAC_1_SQRT_2, 200.0 + r * std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(at(117.0), 255);
    assert_eq!(at(123.0), 0, "a square spread would reach ~128 along the diagonal");
    // And along the axes.
    assert_eq!(alpha(&p, 200.5, 82.0), 255);
    assert_eq!(alpha(&p, 200.5, 78.0), 0);
}

#[test]
fn negative_spread_shrinks_an_ellipse_as_an_ellipse() {
    // Shown behind the (half transparent) shape, a shrunk shadow ends 20 inside the edge.
    let (mut d, n) = shape(ellipse(), vec![EffectKind::DropShadow(shadow(-20.0))]);
    d.get_mut(n).unwrap().fills = vec![];
    d.get_mut(n).unwrap().strokes = vec![];
    let sil = shadow::silhouette(&d, d.node(n).unwrap());
    assert!(sil.is_none(), "no fill, no stroke: nothing casts by outline");
    let (d, n) = shape(ellipse(), vec![EffectKind::DropShadow(shadow(-20.0))]);
    let node = d.node(n).unwrap();
    let sil = shadow::silhouette(&d, node).unwrap();
    let xf = Affine::translate((100.0, 100.0));
    let a = shadow::spread_alpha(&sil, -20.0, xf, 400, 400).unwrap();
    let at = |r: f64| alpha(&a, 200.0 + r * std::f64::consts::FRAC_1_SQRT_2, 200.0 + r * std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(at(77.0), 255);
    assert_eq!(at(83.0), 0);
}

#[test]
fn spread_keeps_star_points_sharp() {
    let star = NodeKind::Star { count: 5, inner_ratio: 0.381_966 };
    let (d, n) = shape(star, vec![EffectKind::DropShadow(shadow(10.0))]);
    let p = render(&d, n);
    // The top point is at (200, 100) with a 36° tip: offsetting its sides by 10 moves the mitred
    // tip 10 / sin(18°) ≈ 32 up. A square or round spread ends 10 above it.
    assert_eq!(alpha(&p, 200.5, 75.0), 255);
    assert_eq!(alpha(&p, 200.5, 64.0), 0);
}

#[test]
fn rectangle_spread_grows_the_corner_radius() {
    let (mut d, n) = shape(NodeKind::Rectangle, vec![EffectKind::DropShadow(shadow(20.0))]);
    d.get_mut(n).unwrap().corners.radii = [40.0; 4];
    let p = render(&d, n);
    // Corner centre at (140, 140); the shadow's corner is a radius 60 arc around it.
    let at = |r: f64| alpha(&p, 140.0 - r * std::f64::consts::FRAC_1_SQRT_2, 140.0 - r * std::f64::consts::FRAC_1_SQRT_2);
    assert_eq!(at(57.0), 255);
    assert_eq!(at(63.0), 0);
    // Sharp corners stay sharp (CSS box-shadow spread).
    let (d, n) = shape(NodeKind::Rectangle, vec![EffectKind::DropShadow(shadow(20.0))]);
    let p = render(&d, n);
    assert_eq!(alpha(&p, 81.0, 81.0), 255);
}

#[test]
fn inner_shadow_spread_shrinks_the_outline() {
    // A black inner shadow, no blur or offset: a ring 20 wide inside the ellipse's edge.
    let (d, n) = shape(ellipse(), vec![EffectKind::InnerShadow(shadow(20.0))]);
    let p = render(&d, n);
    let red = |r: f64| {
        let (x, y) = (200.0 + r * std::f64::consts::FRAC_1_SQRT_2, 200.0 + r * std::f64::consts::FRAC_1_SQRT_2);
        let i = ((y as u32 * p.width() + x as u32) * 4) as usize;
        p.data()[i]
    };
    assert_eq!(red(83.0), 0, "inside the ring: shadow");
    assert_eq!(red(77.0), 255, "past the ring: the white fill");
}

#[test]
fn spread_of_layers_without_one_outline_grows_their_pixels() {
    // A group has no outline of its own: its pixels spread (square), as before.
    let mut d = Document::new("g");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 100.0, 100.0, 200.0, 200.0).unwrap();
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 200.0, 200.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    d.get_mut(g).unwrap().effects = vec![Effect { kind: EffectKind::DropShadow(shadow(10.0)), visible: true }];
    assert!(shadow::silhouette(&d, d.node(g).unwrap()).is_none());
    let pix = render(&d, g);
    assert_eq!(alpha(&pix, 92.0, 92.0), 255);
    assert_eq!(alpha(&pix, 88.0, 88.0), 0);
}

#[test]
fn spread_bands_are_cached_and_hostile_values_are_safe() {
    let (d, n) = shape(ellipse(), vec![]);
    let sil = shadow::silhouette(&d, d.node(n).unwrap()).unwrap();
    let a = shadow::spread_alpha(&sil, 7.0, Affine::IDENTITY, 300, 300).unwrap();
    let b = shadow::spread_alpha(&sil, 7.0, Affine::IDENTITY, 300, 300).unwrap();
    assert_eq!(a.data(), b.data());
    for s in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e12, -1e12, 0.0] {
        assert!(shadow::spread_alpha(&sil, s, Affine::IDENTITY, 50, 50).is_some());
        assert!(shadow::spread_alpha(&sil, 3.0, Affine::scale(s), 50, 50).is_some());
    }
    for s in [f64::NAN, 1e12, -1e12] {
        let (d, n) =
            shape(NodeKind::Star { count: 5, inner_ratio: 0.4 }, vec![EffectKind::DropShadow(shadow(s)), EffectKind::InnerShadow(shadow(s))]);
        let _ = render(&d, n);
    }
}

/// An arc (a pie slice, or a ring cut short) has sharp corners where its ends meet its sides, and
/// a spread mitres them out past the spread: its layer makes room for that, as a star's does. A
/// clear shadow spread four times as far gives the same layer room to the full mitre: the shadow
/// it casts is the same.
#[test]
fn arc_spread_mitres_are_not_cut_off() {
    for kind in [
        NodeKind::Ellipse { start: 1.0, sweep: 0.3, inner: 0.0 },
        NodeKind::Ellipse { start: 0.0, sweep: 3.0, inner: 0.0 },
        NodeKind::Ellipse { start: 1.0, sweep: 2.0, inner: 0.8 },
    ] {
        for spread in [25.0, 40.0] {
            let (d, n) = shape(kind.clone(), vec![EffectKind::DropShadow(shadow(spread))]);
            let clear = Shadow { color: Color::new(0.0, 0.0, 0.0, 0.0), ..shadow(spread * 4.0) };
            let (roomy, m) = shape(kind.clone(), vec![EffectKind::DropShadow(shadow(spread)), EffectKind::DropShadow(clear)]);
            let (a, b) = (render(&d, n), render(&roomy, m));
            let worst = a.data().chunks(4).zip(b.data().chunks(4)).map(|(p, q)| p[3].abs_diff(q[3])).max().unwrap_or(0);
            assert!(worst <= 2, "{kind:?}, a spread of {spread}: {worst} levels cut off");
        }
    }
}
