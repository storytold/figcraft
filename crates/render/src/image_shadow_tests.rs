//! Shadows of image fills follow what the image paints (#1289): a picture with transparent pixels
//! casts its drop shadow from its alpha and hides it only behind its pixels, and its inner shadow
//! shades the inside of its alpha; an opaque picture keeps its outline.

use std::sync::Arc;

use figcraft_color::{Color, ImageScaleMode, Paint};
use figcraft_doc::{Document, Effect, EffectKind, ImageData, NodeId, NodeKind, Shadow};

use super::{Pixmap, View, render_nodes_into, shadow};

/// A 200 × 200 PNG, red: a circle of radius 50 in its middle on transparent pixels (`circle`), or
/// all over.
fn png(circle: bool) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(200, 200, |x, y| {
        let r = (f64::from(x) + 0.5 - 100.0).hypot(f64::from(y) + 0.5 - 100.0);
        image::Rgba(if !circle || r <= 50.0 { [255, 0, 0, 255] } else { [0; 4] })
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A 200 × 200 rectangle at (100, 100) filled with the image, with `effect`, at `opacity`.
fn scene(circle: bool, effect: EffectKind, opacity: f32) -> (Document, NodeId) {
    let mut d = Document::new("image shadow");
    let p = d.first_page().unwrap();
    // Image ids key the shared decode cache: fixtures running in parallel need ids of their own.
    let id = if circle { "image-shadow-tests-circle" } else { "image-shadow-tests-opaque" };
    d.images.insert(id.into(), ImageData { bytes: Arc::new(png(circle)), width: 200, height: 200 });
    let n = d.create(p, NodeKind::Rectangle, "R", 100.0, 100.0, 200.0, 200.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills = vec![Paint::image(id, ImageScaleMode::Fill)];
    node.effects = vec![Effect { kind: effect, visible: true }];
    node.opacity = opacity;
    (d, n)
}

fn shadow(offset: (f64, f64), show_behind: bool) -> Shadow {
    Shadow { color: Color::BLACK, offset, radius: 0.0, show_behind, ..Default::default() }
}

/// Node `id` drawn on a transparent 400 × 400 view of the page.
fn render(d: &Document, id: NodeId) -> Pixmap {
    let mut pix = Pixmap::new(400, 400).unwrap();
    render_nodes_into(d, &[id], &View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 400 }, &mut pix);
    pix
}

/// The premultiplied RGBA at the page point `(x, y)`.
fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    p.data()[i..i + 4].try_into().unwrap()
}

fn near(v: u8, want: u8) -> bool {
    v.abs_diff(want) <= 3
}

#[test]
fn a_transparent_images_drop_shadow_follows_its_alpha() {
    for show_behind in [false, true] {
        // At half opacity, a shadow under the circle shows through it.
        let (d, n) = scene(true, EffectKind::DropShadow(shadow((20.0, 20.0), show_behind)), 0.5);
        assert!(shadow::cover(&d, d.node(n).unwrap()).is_none(), "the image's pixels hide its shadow, not its box");
        let p = render(&d, n);
        // Inside the box, outside the circle, inside the circle's shadow: the shadow.
        let s = px(&p, 265, 220);
        assert!(s[..3] == [0, 0, 0] && near(s[3], 128), "show behind {show_behind}: {s:?}");
        // Inside the box, away from the circle and its shadow: nothing.
        assert_eq!(px(&p, 150, 150), [0; 4]);
        // Where the circle covers the shadow, the option decides.
        let c = px(&p, 225, 225);
        assert!(near(c[3], if show_behind { 191 } else { 128 }), "show behind {show_behind}: {c:?}");
    }
}

#[test]
fn an_opaque_images_drop_shadow_still_hides_behind_its_box() {
    let (d, n) = scene(false, EffectKind::DropShadow(shadow((20.0, 20.0), false)), 0.5);
    let node = d.node(n).unwrap();
    assert!(shadow::cover(&d, node).is_some() && shadow::silhouette(&d, node).is_some(), "an opaque fill keeps its outline");
    let p = render(&d, n);
    assert!(near(px(&p, 250, 250)[3], 128), "behind the picture: hidden");
    let s = px(&p, 310, 250);
    assert!(s[..3] == [0, 0, 0] && near(s[3], 128), "past it: the shadow {s:?}");
    // Fit leaves part of a wider box empty: the pixels decide there too.
    let (mut d, n) = scene(false, EffectKind::DropShadow(shadow((20.0, 20.0), false)), 0.5);
    d.get_mut(n).unwrap().width = 300.0;
    d.get_mut(n).unwrap().fills = vec![Paint::image("image-shadow-tests-opaque", ImageScaleMode::Fit)];
    assert!(shadow::cover(&d, d.node(n).unwrap()).is_none());
}

#[test]
fn a_transparent_images_inner_shadow_shades_its_alpha() {
    let (d, n) = scene(true, EffectKind::InnerShadow(shadow((0.0, 10.0), false)), 1.0);
    let p = render(&d, n);
    // The top 10 of the circle (it starts at y 150) is in shadow; its middle stays red.
    assert_eq!(px(&p, 200, 155), [0, 0, 0, 255]);
    assert_eq!(px(&p, 200, 200), [255, 0, 0, 255]);
    // The box's top edge is transparent: no shadow there.
    assert_eq!(px(&p, 200, 105), [0; 4]);
    // An opaque picture is shaded from its box's edge, as before.
    let (d, n) = scene(false, EffectKind::InnerShadow(shadow((0.0, 10.0), false)), 1.0);
    let p = render(&d, n);
    assert_eq!(px(&p, 200, 105), [0, 0, 0, 255]);
    assert_eq!(px(&p, 200, 200), [255, 0, 0, 255]);
}
