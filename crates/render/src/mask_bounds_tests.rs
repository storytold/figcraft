//! Mask groups draw only over the pixels they can paint, and nested isolated layers share one
//! memory budget (#1061). Each scene renders the same with a mask's pixmaps cut to its area as with
//! the whole view (how masks drew before).

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, EffectKind, MaskType, NodeId, NodeKind, Shadow};
use figcraft_geom::Affine;
use tiny_skia::Pixmap;

use crate::draw::WHOLE_MASK_AREA;
use crate::layer_budget::{BUDGET, within};
use crate::{Options, View, render_view_single};

const ELLIPSE: NodeKind = NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 };

fn shape(d: &mut Document, parent: NodeId, kind: NodeKind, (x, y, w, h): (f64, f64, f64, f64), hex: &str) -> NodeId {
    let id = d.create(parent, kind, "Shape", x, y, w, h).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
    id
}

/// A group under `parent` whose first layer, `mask`, masks the rest.
fn mask_group(d: &mut Document, parent: NodeId, kind: NodeKind, rect: (f64, f64, f64, f64), mask_type: MaskType) -> (NodeId, NodeId) {
    let g = d.create(parent, NodeKind::Group, "Mask group", 0.0, 0.0, 1.0, 1.0).unwrap();
    let m = shape(d, g, kind, rect, "808080");
    let n = d.get_mut(m).unwrap();
    n.is_mask = true;
    n.mask_type = mask_type;
    (g, m)
}

fn turn(d: &mut Document, id: NodeId, degrees: f64) {
    let n = d.get_mut(id).unwrap();
    let (cx, cy) = (n.width / 2.0, n.height / 2.0);
    n.transform = n.transform * Affine::translate((cx, cy)) * Affine::rotate(degrees.to_radians()) * Affine::translate((-cx, -cy));
}

/// Nested masks (alpha, luminance, vector, a mask with opacity), rotated masks and content,
/// content with opacity, a drop shadow and a background blur, and a mask group running off the
/// right and bottom of an 800 × 600 view.
fn scene() -> (Document, NodeId) {
    let mut d = Document::new("masks");
    let p = d.first_page().unwrap();
    shape(&mut d, p, NodeKind::Rectangle, (0.0, 0.0, 800.0, 600.0), "EEEEEE");
    let (g1, _) = mask_group(&mut d, p, ELLIPSE, (100.0, 80.0, 320.0, 220.0), MaskType::Alpha);
    let c1 = shape(&mut d, g1, NodeKind::Rectangle, (60.0, 50.0, 400.0, 300.0), "E03030");
    let n = d.get_mut(c1).unwrap();
    n.opacity = 0.6;
    n.effects = vec![Effect {
        kind: EffectKind::DropShadow(Shadow { color: Color::BLACK, offset: (8.0, 12.0), radius: 10.0, ..Shadow::default() }),
        visible: true,
    }];
    let c2 = shape(&mut d, g1, NodeKind::Rectangle, (150.0, 120.0, 160.0, 90.0), "2040E0");
    turn(&mut d, c2, 30.0);
    let pane = shape(&mut d, g1, NodeKind::Rectangle, (120.0, 100.0, 140.0, 80.0), "FFFFFF33");
    d.get_mut(pane).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
    // Nested: a turned luminance mask inside the first group.
    let (g2, m2) = mask_group(&mut d, g1, NodeKind::Rectangle, (200.0, 140.0, 150.0, 110.0), MaskType::Luminance);
    turn(&mut d, m2, 20.0);
    let c3 = shape(&mut d, g2, ELLIPSE, (180.0, 120.0, 200.0, 160.0), "20C060");
    d.get_mut(c3).unwrap().opacity = 0.8;
    // A vector mask with opacity running off the view.
    let (g3, m3) = mask_group(&mut d, p, NodeKind::Star { count: 5, inner_ratio: 0.5 }, (640.0, 420.0, 300.0, 300.0), MaskType::Vector);
    d.get_mut(m3).unwrap().opacity = 0.5;
    let c4 = shape(&mut d, g3, NodeKind::Rectangle, (560.0, 380.0, 500.0, 400.0), "F0A020");
    turn(&mut d, c4, -15.0);
    // An alpha mask with opacity: its own layer.
    let (g4, m4) = mask_group(&mut d, p, NodeKind::Rectangle, (20.0, 420.0, 200.0, 150.0), MaskType::Alpha);
    d.get_mut(m4).unwrap().opacity = 0.4;
    shape(&mut d, g4, ELLIPSE, (0.0, 400.0, 260.0, 190.0), "8020C0");
    (d, p)
}

fn render(d: &Document, page: NodeId, view: &View, whole: bool) -> Pixmap {
    WHOLE_MASK_AREA.with(|c| c.set(whole));
    let pix = render_view_single(d, page, view, &Options { background: Some(Color::WHITE), ..Default::default() });
    WHOLE_MASK_AREA.with(|c| c.set(false));
    pix.unwrap()
}

#[test]
fn masks_cut_to_their_area_draw_the_same() {
    let (d, page) = scene();
    let views = [
        View { origin: (0.0, 0.0), scale: 1.0, width: 800, height: 600 },
        // Fractional origin and zoom: the area's edges fall between pixels.
        View { origin: (-37.3, 12.6), scale: 1.7, width: 900, height: 700 },
        View { origin: (300.0, 200.0), scale: 0.45, width: 320, height: 240 },
        // Only part of each group in view.
        View { origin: (250.0, 250.0), scale: 2.0, width: 500, height: 400 },
    ];
    for v in &views {
        let (before, after) = (render(&d, page, v, true), render(&d, page, v, false));
        let differ = before.data().chunks(4).zip(after.data().chunks(4)).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{v:?}: {differ} pixels differ");
        let colours: std::collections::BTreeSet<&[u8]> = after.data().chunks(4).collect();
        assert!(colours.len() > 3, "{v:?}: the masks show their content");
    }
}

#[test]
fn a_small_mask_holds_a_small_area() {
    let mut d = Document::new("small mask");
    let p = d.first_page().unwrap();
    let (g, _) = mask_group(&mut d, p, ELLIPSE, (100.0, 100.0, 40.0, 40.0), MaskType::Alpha);
    shape(&mut d, g, NodeKind::Rectangle, (90.0, 90.0, 60.0, 60.0), "E03030");
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 3000, height: 2000 };
    let (before, whole) = within(BUDGET, || render(&d, p, &v, true));
    let (after, cut) = within(BUDGET, || render(&d, p, &v, false));
    assert_eq!(before.data(), after.data());
    // Four 3000 × 2000 pixmaps before (96 MB), four 62 × 62 now (61.5 kB).
    assert_eq!(whole, 4 * 4 * 3000 * 2000);
    assert_eq!(cut, 4 * 4 * 62 * 62, "the mask's 40 × 40 plus its content, rounded out and a pixel round");
}

#[test]
fn a_deep_nest_of_isolated_layers_stays_within_the_budget() {
    let mut d = Document::new("deep");
    let p = d.first_page().unwrap();
    let mut parent = p;
    // 64 levels, each a full-view group at half opacity or a mask group, every one isolating.
    for i in 0..64 {
        parent = if i % 2 == 0 {
            let g = d.create(parent, NodeKind::Group, "Group", 0.0, 0.0, 1.0, 1.0).unwrap();
            d.get_mut(g).unwrap().opacity = 0.5;
            g
        } else {
            mask_group(&mut d, parent, NodeKind::Rectangle, (0.0, 0.0, 1000.0, 1000.0), MaskType::Alpha).0
        };
        shape(&mut d, parent, NodeKind::Rectangle, (0.0, 0.0, 1000.0, 1000.0), "2040E0");
    }
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 500, height: 500 };
    // Unbounded, the nest holds all its levels at once.
    let (_, all) = within(u64::MAX, || render(&d, p, &v, false));
    assert_eq!(all, 32 * 4 * 500 * 500 * 5, "32 groups of one layer, 32 mask groups of four");
    let limit = 16 << 20;
    let (pix, peak) = within(limit, || render(&d, p, &v, false));
    assert!(peak <= limit, "held {peak} bytes at once");
    // The outer levels still draw (the innermost, past the budget, are left out).
    assert_ne!(&pix.data()[..4], &[255, 255, 255, 255]);
}
