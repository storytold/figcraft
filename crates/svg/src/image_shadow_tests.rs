//! Shadows of image fills in SVG follow what the image paints, like the renderer (#1289): a
//! picture with transparent pixels hides its drop shadows behind its alpha (not an image of its
//! box) and is an embedded render when it has inner shadows; an opaque one keeps its outline.

use figcraft_color::{Color, ImageScaleMode, Paint};
use figcraft_doc::{Document, Effect, EffectKind, ImageData, NodeId, NodeKind, Shadow};

use super::{SvgOptions, export_svg};

/// A 100 × 100 rectangle filled with a 100 × 100 red PNG: a circle on transparent pixels
/// (`circle`) or red all over, with `effect`.
fn scene(circle: bool, effect: EffectKind) -> (Document, NodeId) {
    let mut pm = figcraft_render::Pixmap::new(100, 100).unwrap();
    for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % 100) as f64 + 0.5 - 50.0, (i / 100) as f64 + 0.5 - 50.0);
        if !circle || x.hypot(y) <= 25.0 {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
    }
    let mut d = Document::new("svg image shadow");
    let p = d.first_page().unwrap();
    // Image ids key the renderer's shared caches: fixtures running in parallel need ids of their own.
    let id = if circle { "svg-image-shadow-circle" } else { "svg-image-shadow-opaque" };
    let bytes = figcraft_render::encode_png(&pm).unwrap();
    d.images.insert(id.into(), ImageData { bytes: std::sync::Arc::new(bytes), width: 100, height: 100 });
    let n = d.create(p, NodeKind::Rectangle, "R", 10.0, 10.0, 100.0, 100.0).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills = vec![Paint::image(id, ImageScaleMode::Fill)];
    node.effects = vec![Effect { kind: effect, visible: true }];
    (d, n)
}

fn export(d: &Document, id: NodeId) -> String {
    export_svg(d, &[id], &SvgOptions::default()).unwrap()
}

fn shadow() -> Shadow {
    Shadow { color: Color::BLACK, offset: (6.0, 6.0), radius: 4.0, show_behind: false, ..Shadow::default() }
}

#[test]
fn a_transparent_images_drop_shadow_hides_behind_its_alpha() {
    let (d, n) = scene(true, EffectKind::DropShadow(shadow()));
    let svg = export(&d, n);
    assert!(svg.contains("in2=\"SourceAlpha\" operator=\"out\"") && !svg.contains("result=\"cover\""), "{svg}");
    let (d, n) = scene(false, EffectKind::DropShadow(shadow()));
    let svg = export(&d, n);
    assert!(svg.contains("in2=\"cover\" operator=\"out\""), "an opaque picture hides it behind its box: {svg}");
}

#[test]
fn a_transparent_images_inner_shadow_is_an_embedded_render() {
    let (d, n) = scene(true, EffectKind::InnerShadow(shadow()));
    let svg = export(&d, n);
    assert!(!svg.contains("<pattern") && !svg.contains("<mask"), "the render holds the picture and its shade: {svg}");
    let (d, n) = scene(false, EffectKind::InnerShadow(shadow()));
    let svg = export(&d, n);
    assert!(svg.contains("<pattern") && svg.contains("<mask"), "an opaque picture stays vector, shaded from its box: {svg}");
}
