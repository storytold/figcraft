//! Level of detail: only canvas previews (`Options::lod`) at small scales are approximated, the
//! approximation stays close to the exact render (no holes, same colours on average), and exports
//! never see it.

use std::sync::{Arc, Once};

use figcraft_color::{Color, ImageScaleMode, Paint};
use figcraft_doc::{Document, ImageData, NodeId, NodeKind, Region, TextData, VectorNetwork};
use figcraft_geom::{Affine, Rect};

use crate::retained_tests::assert_same;
use crate::{Options, Pixmap, Retained, View, backdrop, lod, render_node, render_view, render_view_single};

fn fonts() {
    static ONCE: Once = Once::new();
    ONCE.call_once(figcraft_text::scan_system_fonts);
}

fn solid(hex: &str) -> Vec<Paint> {
    vec![Paint::solid(Color::from_hex(hex).unwrap())]
}

const COLS: usize = 12;
const ROWS: usize = 8;

/// A page of `COLS` × `ROWS` cards (a clipping frame holding a header bar, a rounded rectangle, an
/// ellipse, a group of small squares and two texts), like a zoomed-out page of screens.
fn cards() -> (Document, NodeId) {
    fonts();
    let mut d = Document::new("lod");
    let page = d.first_page().unwrap();
    for i in 0..COLS * ROWS {
        let (x, y) = ((i % COLS) as f64 * 220.0, (i / COLS) as f64 * 160.0);
        let card = d.create(page, NodeKind::Frame, "Card", x, y, 200.0, 140.0).unwrap();
        d.get_mut(card).unwrap().clip = true;
        let bar = d.create(card, NodeKind::Rectangle, "Bar", 0.0, 0.0, 200.0, 24.0).unwrap();
        d.get_mut(bar).unwrap().fills = solid("2D6CDF");
        let tile = d.create(card, NodeKind::Rectangle, "Tile", 12.0, 36.0, 60.0, 60.0).unwrap();
        let n = d.get_mut(tile).unwrap();
        n.fills = solid(["E8505B", "F9A03F", "35B37E"][i % 3]);
        n.corners.radii = [8.0; 4];
        let dot = d.create(card, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Dot", 150.0, 40.0, 30.0, 30.0).unwrap();
        d.get_mut(dot).unwrap().fills = solid("7A3FD1");
        let group = d.create(card, NodeKind::Group, "Squares", 84.0, 100.0, 100.0, 12.0).unwrap();
        for k in 0..6 {
            let s = d.create(group, NodeKind::Rectangle, "Square", k as f64 * 16.0, 0.0, 12.0, 12.0).unwrap();
            d.get_mut(s).unwrap().fills = solid("333333");
        }
        let mut title = TextData::new(format!("Card number {i}"));
        title.style.font_size = 16.0;
        d.create(card, NodeKind::Text(title), "Title", 84.0, 36.0, 110.0, 20.0).unwrap();
        let mut body = TextData::new("Some body text that wraps onto a second line");
        body.auto_resize = figcraft_doc::TextAutoResize::Height;
        body.style.font_size = 11.0;
        d.create(card, NodeKind::Text(body), "Body", 84.0, 60.0, 110.0, 30.0).unwrap();
    }
    (d, page)
}

fn lod() -> Options {
    Options { lod: true, ..Default::default() }
}

fn view(scale: f64) -> View {
    View {
        origin: (-20.0, -20.0),
        scale,
        width: ((COLS as f64 * 220.0 + 40.0) * scale) as u32,
        height: ((ROWS as f64 * 160.0 + 40.0) * scale) as u32,
    }
}

/// The largest difference between the mean colours of `block` × `block` blocks of two renders,
/// and the mean difference over all channels. A hole (a layer missing) shows as a block whose
/// colour is far off; approximations move colours a little.
fn block_diff(a: &Pixmap, b: &Pixmap, block: u32) -> (f64, f64) {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    let (w, h) = (a.width(), a.height());
    let mut worst: f64 = 0.0;
    for by in (0..h).step_by(block as usize) {
        for bx in (0..w).step_by(block as usize) {
            let mut sums = [0.0f64; 8];
            let mut count = 0.0;
            for y in by..(by + block).min(h) {
                for x in bx..(bx + block).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        sums[c] += f64::from(a.data()[i + c]);
                        sums[4 + c] += f64::from(b.data()[i + c]);
                    }
                    count += 1.0;
                }
            }
            for c in 0..4 {
                worst = worst.max((sums[c] - sums[4 + c]).abs() / count);
            }
        }
    }
    let mean = a.data().iter().zip(b.data()).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.data().len() as f64;
    (worst, mean)
}

/// Mean brightness (0–255) of a render.
fn lum(p: &Pixmap) -> f64 {
    p.data().chunks(4).map(|c| f64::from(c[0]) + f64::from(c[1]) + f64::from(c[2])).sum::<f64>() / (p.data().len() / 4 * 3) as f64
}

#[test]
fn lod_is_off_unless_asked_for() {
    assert!(!Options::default().lod);
}

#[test]
fn region_paints_disable_vector_and_ancestor_splatting() {
    let mut d = Document::new("region LOD");
    let page = d.first_page().unwrap();
    let parent = d.create(page, NodeKind::Frame, "Parent", 0.0, 0.0, 4.0, 4.0).unwrap();
    let mut network = VectorNetwork::default();
    network.regions.push(Region { loops: Vec::new(), winding: Default::default(), fills: Some(Vec::new()), ..Default::default() });
    let vector = d.create(parent, NodeKind::Vector { network }, "Styled", 0.0, 0.0, 2.0, 2.0).unwrap();
    let node = d.node(vector).unwrap();
    assert!(!lod::splattable(&d, node, 0));
    assert!(!lod::splattable(&d, d.node(parent).unwrap(), 0));
}

#[test]
fn zoomed_out_previews_are_close_to_exact() {
    let (d, p) = cards();
    for scale in [0.02, 0.05, 0.12, 0.25] {
        let v = view(scale);
        let exact = render_view(&d, p, &v, &Options::default()).unwrap();
        let approx = render_view(&d, p, &v, &lod()).unwrap();
        assert_ne!(exact.data(), approx.data(), "scale {scale}: LOD should kick in");
        let (worst, mean) = block_diff(&exact, &approx, 4);
        // Sub-pixel shapes are anti-aliased a little heavier on the exact path; holes (missing
        // layers) would differ by whole colours.
        let (a, b) = (lum(&exact), lum(&approx));
        assert!(
            worst < 48.0 && mean < 8.0 && (a - b).abs() < 4.0,
            "scale {scale}: blocks differ by up to {worst:.1}, {mean:.2} on average, brightness {a:.1} vs {b:.1}"
        );
    }
}

#[test]
fn close_up_previews_are_exact() {
    // Above `MAX_SCALE` (1:1 and closer included) previews are exact.
    let (d, p) = cards();
    for scale in [lod::MAX_SCALE + 0.01, 1.0, 2.0] {
        let v = View { origin: (-10.0, -10.0), scale, width: 480, height: 360 };
        let exact = render_view(&d, p, &v, &Options::default()).unwrap();
        let approx = render_view(&d, p, &v, &lod()).unwrap();
        assert_eq!(exact.data(), approx.data(), "scale {scale}");
    }
}

#[test]
fn exports_never_use_lod() {
    // An export at a tiny scale is the exact render of the same area, not the preview.
    let (d, p) = cards();
    let card = d.node(p).unwrap().children[0];
    let scale = 0.05;
    let png = render_node(&d, card, scale).unwrap();
    let b = crate::export_bounds(&d, card).unwrap();
    let v = View { origin: (b.x0, b.y0), scale, width: png.width(), height: png.height() };
    let mut exact = Pixmap::new(v.width, v.height).unwrap();
    crate::render_nodes_into(&d, &[card], &v, &mut exact);
    assert_eq!(png.data(), exact.data());
    let mut preview = Pixmap::new(v.width, v.height).unwrap();
    let bounds = crate::draw::Bounds::default();
    let ctx = crate::draw::Ctx {
        doc: &d,
        scale,
        outline_mode: false,
        outline: crate::Outline::default(),
        outlined: &[],
        skip: &[],
        lod: true,
        preview: false,
        magnified_image_pixels: false,
        under: None,
        time: 0.0,
        bounds: &bounds,
        page_at: (v.origin.0 * scale, v.origin.1 * scale),
        cache: None,
        images: 0,
        accelerate: false,
    };
    let parent = d.world_transform(p);
    crate::draw::draw_node(&ctx, &mut preview, card, v.transform() * parent, 0);
    assert_ne!(png.data(), preview.data(), "the preview of the same area is approximated");
}

#[test]
fn outline_mode_ignores_lod() {
    let (d, p) = cards();
    let v = view(0.05);
    let a = render_view(&d, p, &v, &Options { outline_mode: true, ..Default::default() }).unwrap();
    let b = render_view(&d, p, &v, &Options { outline_mode: true, lod: true, ..Default::default() }).unwrap();
    assert_eq!(a.data(), b.data());
}

#[test]
fn tiny_layers_keep_their_ink() {
    // A sub-pixel black square on white darkens the pixels it straddles by its area, wherever it
    // sits on the pixel grid (no layer vanishes or jumps as the view moves).
    let mut d = Document::new("tiny");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(r).unwrap().fills = solid("000000");
    for offset in [0.0, 0.25, 0.5, 0.8] {
        let v = View { origin: (-20.0 - offset * 12.5, -20.0), scale: 0.08, width: 6, height: 6 };
        let pix = render_view(&d, p, &v, &Options { lod: true, background: Some(Color::WHITE), ..Default::default() }).unwrap();
        let ink: f64 = pix.data().chunks(4).map(|c| f64::from(255 - c[0]) / 255.0).sum();
        // 0.8 × 0.8 device pixels.
        assert!((ink - 0.64).abs() < 0.02, "offset {offset}: ink {ink}");
    }
}

#[test]
fn images_and_masks_take_the_exact_path() {
    let mut d = Document::new("exact");
    let p = d.first_page().unwrap();
    let img = image::RgbaImage::from_fn(8, 8, |x, y| image::Rgba([(x * 30) as u8, (y * 30) as u8, 90, 255]));
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    // Its own id: image ids key the shared decode cache (see band_tests).
    d.images.insert("lod-tests-photo".into(), ImageData { bytes: Arc::new(bytes), width: 8, height: 8 });
    let photo = d.create(p, NodeKind::Rectangle, "Photo", 0.0, 0.0, 20.0, 20.0).unwrap();
    d.get_mut(photo).unwrap().fills = vec![Paint::image("lod-tests-photo", ImageScaleMode::Fill)];
    let group = d.create(p, NodeKind::Group, "Masked", 40.0, 0.0, 20.0, 20.0).unwrap();
    let mask = d.create(group, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Mask", 0.0, 0.0, 20.0, 20.0).unwrap();
    d.get_mut(mask).unwrap().is_mask = true;
    let under = d.create(group, NodeKind::Rectangle, "Under", 0.0, 0.0, 20.0, 20.0).unwrap();
    d.get_mut(under).unwrap().fills = solid("FF0000");
    for n in [photo, group] {
        assert!(!lod::splattable(&d, d.node(n).unwrap(), 0));
    }
    // The photo renders exactly as in an export.
    let v = View { origin: (-2.0, -2.0), scale: 0.06, width: 2, height: 2 };
    let exact = render_view(&d, p, &v, &Options::default()).unwrap();
    let approx = render_view(&d, p, &v, &lod()).unwrap();
    assert_eq!(exact.data(), approx.data());
    // The masked group is masked (its content may be approximated under the mask).
    let v = View { origin: (38.0, -2.0), scale: 0.06, width: 2, height: 2 };
    let exact = render_view(&d, p, &v, &Options::default()).unwrap();
    let approx = render_view(&d, p, &v, &lod()).unwrap();
    assert_same(&exact, &approx, "masked group");
}

#[test]
fn greeked_text_stays_inside_its_bounds() {
    fonts();
    let mut d = Document::new("greek");
    let p = d.first_page().unwrap();
    let mut t = TextData::new("Greeked text on a line\nand a second line");
    t.style.font_size = 20.0;
    let id = d.create(p, NodeKind::Text(t), "T", 100.0, 100.0, 300.0, 60.0).unwrap();
    let n = d.node(id).unwrap();
    assert!(lod::greeked(n, 0.1) && !lod::greeked(n, 0.5));
    let v = View { origin: (0.0, 0.0), scale: 0.1, width: 60, height: 30 };
    let pix = render_view(&d, p, &v, &Options { lod: true, background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let bounds = crate::paint_bounds(&d, id).unwrap();
    let mut inked = 0;
    for (i, c) in pix.data().chunks(4).enumerate() {
        if c[0] < 250 {
            let (x, y) = ((i % 60) as f64 / 0.1, (i / 60) as f64 / 0.1);
            assert!(bounds.inflate(10.0, 10.0).contains((x, y)), "ink at {x},{y} outside {bounds:?}");
            inked += 1;
        }
    }
    assert!(inked > 10, "greeked text draws its lines ({inked} pixels)");
}

#[test]
fn retained_previews_match_full_ones() {
    // Pans and edits of a zoomed-out preview patch the kept image into what a full preview shows.
    let (d, p) = cards();
    let doc = Arc::new(d);
    let mut r = Retained::default();
    let at = |x: f64, y: f64| View { origin: (x, y), scale: 0.07, width: 160, height: 90 };
    let check = |r: &mut Retained, doc: &Arc<Document>, v: &View| {
        let (shown, pix, work) = r.render(doc, p, v, &lod()).unwrap();
        let pix = pix.clone();
        let full = render_view(doc, p, &shown, &lod()).unwrap();
        assert_same(&pix, &full, &format!("{work:?}"));
        work
    };
    check(&mut r, &doc, &at(0.0, 0.0));
    for (x, y) in [(70.0, 0.0), (70.0, 45.0), (-30.0, 90.0)] {
        assert!(matches!(check(&mut r, &doc, &at(x, y)), crate::Work::Partial { .. }));
    }
    let card = doc.node(p).unwrap().children[5];
    let mut moved = (*doc).clone();
    let n = moved.get_mut(card).unwrap();
    n.transform = Affine::translate((30.0, 10.0)) * n.transform;
    let moved = Arc::new(moved);
    assert!(matches!(check(&mut r, &moved, &at(-30.0, 90.0)), crate::Work::Partial { .. }));
    // A preview and an exact render never share a kept image.
    let (_, _, work) = r.render(&moved, p, &at(-30.0, 90.0), &Options::default()).unwrap();
    assert_eq!(work, crate::Work::Full);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bands_match_one_piece_previews() {
    let (d, p) = cards();
    let v = view(0.1);
    let one = render_view_single(&d, p, &v, &lod()).unwrap();
    for bands in [2, 5] {
        let banded = crate::render_bands(&d, p, &v, &lod(), bands).unwrap();
        assert_same(&one, &banded, &format!("{bands} bands"));
    }
}

#[test]
fn splat_and_blit_survive_junk() {
    let mut pix = Pixmap::new(4, 4).unwrap();
    for r in [
        Rect::new(f64::NAN, 0.0, 1.0, 1.0),
        Rect::new(-1e300, -1e300, 1e300, 1e300),
        Rect::new(10.0, 10.0, 20.0, 20.0),
        Rect::new(3.0, 3.0, 1.0, 1.0),
        Rect::new(-5.0, 1.5, 0.5, f64::INFINITY),
    ] {
        lod::splat(&mut pix, r, [0.5, 0.5, 0.5, 1.0]);
        lod::splat(&mut pix, r, [f32::NAN, 2.0, -1.0, 0.5]);
    }
    // An exactly covered pixel takes the colour; half a pixel blends half of it.
    let mut pix = Pixmap::new(4, 1).unwrap();
    lod::splat(&mut pix, Rect::new(1.0, 0.0, 2.0, 1.0), [1.0, 0.0, 0.0, 1.0]);
    lod::splat(&mut pix, Rect::new(2.5, 0.0, 3.0, 1.0), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(&pix.data()[4..8], &[255, 0, 0, 255]);
    assert_eq!(&pix.data()[8..12], &[0, 0, 128, 128]);
    let src = Pixmap::new(3, 3).unwrap();
    let mut dst = Pixmap::new(4, 4).unwrap();
    for (x, y) in [(-2, -2), (3, 3), (-10, 0), (0, 10), (i32::MIN, i32::MAX), (i32::MAX, i32::MIN)] {
        backdrop::replace(&mut dst, (x, y), &src, None);
    }
}
