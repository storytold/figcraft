use super::*;
use figcraft_color::{Color, ColorStop, Gradient, GradientKind, Paint};
use figcraft_doc::{Effect, EffectKind, NodeKind, Shadow, StrokeAlign, TextData};

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = &p.data()[i..i + 4];
    [d[0], d[1], d[2], d[3]]
}

fn doc_with_rect(fill: Color) -> (figcraft_doc::Document, figcraft_doc::NodeId, figcraft_doc::NodeId) {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 10.0, 10.0, 50.0, 30.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(fill)];
    (d, p, r)
}

fn view(w: u32, h: u32) -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h }
}

#[test]
fn solid_rect_and_page_background() {
    let (d, p, _) = doc_with_rect(Color::from_hex("FF0000").unwrap());
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 20), [255, 0, 0, 255]);
    assert_eq!(px(&pix, 5, 5), [0xF5, 0xF5, 0xF5, 255]);
}

#[test]
fn zoom_scales() {
    let (d, p, _) = doc_with_rect(Color::BLACK);
    let v = View { origin: (0.0, 0.0), scale: 2.0, width: 200, height: 200 };
    let pix = render_view(&d, p, &v, &Options::default()).unwrap();
    assert_eq!(px(&pix, 100, 60)[3], 255);
    assert_eq!(px(&pix, 118, 78), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 125, 60)[0], 0xF5);
}

#[test]
fn opacity_and_layer() {
    let (mut d, p, r) = doc_with_rect(Color::BLACK);
    d.get_mut(r).unwrap().opacity = 0.5;
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let v = px(&pix, 30, 20)[0];
    assert!((120..=135).contains(&v), "{v}");
}

#[test]
fn drop_shadow_paints_outside() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    d.get_mut(r).unwrap().effects.push(Effect {
        kind: EffectKind::DropShadow(Shadow { color: Color::BLACK, offset: (0.0, 10.0), radius: 0.0, ..Default::default() }),
        visible: true,
    });
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // Below the rect (y 40..50) is shadow.
    assert!(px(&pix, 30, 45)[0] < 30, "{:?}", px(&pix, 30, 45));
    // The rect itself stays white.
    assert_eq!(px(&pix, 30, 20)[0], 255);
}

#[test]
fn inner_shadow_and_blur_dont_crash() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    let n = d.get_mut(r).unwrap();
    n.effects.push(Effect::inner_shadow());
    n.effects.push(Effect::layer_blur());
    n.effects.push(Effect::background_blur());
    n.effects.push(Effect {
        kind: EffectKind::Noise(figcraft_doc::Noise { size: 1.0, density: 0.5, color: Color::BLACK, ..Default::default() }),
        visible: true,
    });
    let _ = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
}

#[test]
fn gradients_render() {
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Angular, GradientKind::Diamond] {
        let (mut d, p, r) = doc_with_rect(Color::BLACK);
        let g = Gradient {
            kind,
            stops: vec![
                ColorStop { position: 0.0, color: Color::from_hex("FF0000").unwrap() },
                ColorStop { position: 1.0, color: Color::from_hex("0000FF").unwrap() },
            ],
            from: (0.0, 0.5),
            to: (1.0, 0.5),
            width: (0.0, 1.0),
        };
        let g = if kind == GradientKind::Linear { g } else { Gradient { from: (0.5, 0.5), to: (1.0, 0.5), width: (0.5, 1.0), ..g } };
        d.get_mut(r).unwrap().fills = vec![Paint::gradient(g)];
        let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
        let left = px(&pix, if kind == GradientKind::Linear { 11 } else { 35 }, 25);
        let right = px(&pix, 58, 25);
        assert_ne!(left, right, "{kind:?}");
    }
}

#[test]
fn frame_clips_children() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 50.0, 50.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 25.0, 25.0, 50.0, 50.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 40, 40), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 60, 60)[0], 0xF5);
    d.get_mut(f).unwrap().clip = false;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 60, 60), [0, 0, 0, 255]);
}

#[test]
fn strokes_align() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    let n = d.get_mut(r).unwrap();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 4.0;
    n.stroke.align = StrokeAlign::Outside;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 8), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 30, 12), [255, 255, 255, 255]);
    d.get_mut(r).unwrap().stroke.align = StrokeAlign::Inside;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 12), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 30, 8)[0], 0xF5);
}

#[test]
fn mask_clips_siblings() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "M", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(m).unwrap().is_mask = true;
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 2, 2)[0], 0xF5);
    assert_eq!(px(&pix, 80, 80)[0], 0xF5);
}

#[test]
fn luminance_and_vector_masks() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    // The mask: white on the left half, black on the right.
    let m = d.create(g, NodeKind::Rectangle, "M", 0.0, 0.0, 100.0, 100.0).unwrap();
    let grad = figcraft_color::Gradient {
        kind: figcraft_color::GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::WHITE },
            figcraft_color::ColorStop { position: 0.49, color: Color::WHITE },
            figcraft_color::ColorStop { position: 0.51, color: Color::BLACK },
            figcraft_color::ColorStop { position: 1.0, color: Color::BLACK },
        ],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 1.0),
    };
    d.get_mut(m).unwrap().fills = vec![Paint::gradient(grad)];
    d.get_mut(m).unwrap().is_mask = true;
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    // Alpha: the mask is opaque everywhere, so everything shows.
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 80, 50), [0, 0, 0, 255]);
    // Luminance: only the white half shows.
    d.get_mut(m).unwrap().mask_type = figcraft_doc::MaskType::Luminance;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 20, 50), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 80, 50)[0], 0xF5, "black hides");
    // Vector: a fully transparent ellipse still masks by its outline.
    let e = d.create(g, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "E", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(e).unwrap().fills = vec![Paint::solid(Color::new(0.0, 0.0, 0.0, 0.0))];
    d.get_mut(e).unwrap().is_mask = true;
    d.get_mut(e).unwrap().mask_type = figcraft_doc::MaskType::Vector;
    d.get_mut(g).unwrap().children = vec![e, r];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25), [0, 0, 0, 255], "inside the outline");
    assert_eq!(px(&pix, 80, 80)[0], 0xF5, "outside");
    // As an alpha mask the transparent ellipse hides everything.
    d.get_mut(e).unwrap().mask_type = figcraft_doc::MaskType::Alpha;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25)[0], 0xF5);
}

#[test]
fn boolean_subtract_renders_hole() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let b = d.create(p, NodeKind::BooleanOp { op: figcraft_doc::BoolOp::Subtract }, "B", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    d.create(b, NodeKind::Rectangle, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.create(b, NodeKind::Rectangle, "Hole", 25.0, 25.0, 50.0, 50.0).unwrap();
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 50, 50)[0], 0xF5);
}

#[test]
fn text_renders_something() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let mut td = TextData::new("Hello");
    td.style.font_size = 32.0;
    d.create(p, NodeKind::Text(td), "T", 0.0, 0.0, 200.0, 50.0).unwrap();
    let pix = render_view(&d, p, &view(200, 60), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let dark = pix.data().as_chunks::<4>().0.iter().filter(|c| c[0] < 100).count();
    if figcraft_text::find_face("Inter", "Regular").is_some() {
        assert!(dark > 50, "{dark}");
    }
}

/// The machine's colour emoji face, if it has one (this test skips without: no font ships here).
fn emoji_face() -> Option<(String, String)> {
    figcraft_text::scan_system_fonts();
    let mut db = figcraft_text::db().write().unwrap();
    db.emoji_fallbacks().into_iter().find(|f| f.has_color()).map(|f| (f.family.clone(), f.style.clone()))
}

#[test]
fn colour_emoji_draw_their_pixels() {
    let Some((family, style)) = emoji_face() else { return };
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let mut td = TextData::new("\u{1F600}");
    (td.style.font_family, td.style.font_style) = (family.clone(), style.clone());
    td.style.font_size = 48.0;
    d.create(p, NodeKind::Text(td), "T", 0.0, 0.0, 200.0, 64.0).unwrap();
    let pix = render_view(&d, p, &view(100, 80), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // The emoji is the only thing in this document that paints saturated colour.
    let coloured = pix
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|c| {
            let (r, g, b) = (i16::from(c[0]), i16::from(c[1]), i16::from(c[2]));
            c[3] == 255 && (r.max(g).max(b) - r.min(g).min(b)) > 60
        })
        .count();
    assert!(coloured > 200, "{family} drew {coloured} coloured pixels");
}

#[test]
fn export_png_roundtrip() {
    let (d, _, r) = doc_with_rect(Color::from_hex("00FF00").unwrap());
    let pix = render_node(&d, r, 2.0).unwrap();
    assert_eq!((pix.width(), pix.height()), (100, 60));
    let png = encode_png(&pix).unwrap();
    let back = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(back.get_pixel(50, 30).0, [0, 255, 0, 255]);
    let _ = encode_jpeg(&pix, Color::WHITE, 90).unwrap();
}

#[test]
fn exports_carry_the_colour_profile() {
    use figcraft_color::ColorProfile;
    use figcraft_doc::ExportFormat;
    use image::ImageDecoder;
    let (d, _, r) = doc_with_rect(Color::from_hex("00FF00").unwrap());
    let pix = render_node(&d, r, 1.0).unwrap();
    for profile in ColorProfile::ALL {
        let png = encode_export(&pix, ExportFormat::Png, profile).unwrap();
        // The inserted chunk has a valid CRC: the decoder accepts the file and keeps the pixels.
        let mut dec = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&png)).unwrap();
        let icc = dec.icc_profile().unwrap();
        assert_eq!(icc.is_some(), !profile.is_srgb(), "{profile:?}");
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(5, 5).0, [0, 255, 0, 255]);
        for f in [ExportFormat::Jpg, ExportFormat::Webp] {
            assert!(!encode_export(&pix, f, profile).unwrap().is_empty());
        }
        assert!(encode_export(&pix, ExportFormat::Svg, profile).is_err());
    }
    assert_eq!(png_insert_chunk(b"not a png".to_vec(), b"sRGB", &[0]), b"not a png");
}

#[test]
fn images_are_managed_into_the_file_profile() {
    use figcraft_color::{ColorProfile, ImageScaleMode, Paint};
    let (mut d, _, r) = doc_with_rect(Color::WHITE);
    let mut red = Pixmap::new(4, 4).unwrap();
    red.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    d.images.insert("red".into(), figcraft_doc::ImageData { bytes: std::sync::Arc::new(encode_png(&red).unwrap()), width: 4, height: 4 });
    d.get_mut(r).unwrap().fills = vec![Paint::image("red", ImageScaleMode::Fill)];
    let center = |d: &Document| {
        let pix = render_node(d, r, 1.0).unwrap();
        let (x, y) = (pix.width() / 2, pix.height() / 2);
        let px = pix.pixel(x, y).unwrap();
        (pix, [px.red(), px.green(), px.blue()])
    };
    assert_eq!(center(&d).1, [255, 0, 0]);
    // In a Display P3 file the (sRGB) image is converted into P3 values…
    d.color_profile = ColorProfile::DisplayP3;
    let (mut pix, rgb) = center(&d);
    assert!(rgb[0].abs_diff(234) <= 1 && rgb[1].abs_diff(51) <= 1 && rgb[2].abs_diff(35) <= 1, "{rgb:?}");
    // …and comes back to the screen unchanged.
    to_srgb(&mut pix, d.color_profile);
    let px = pix.pixel(pix.width() / 2, pix.height() / 2).unwrap();
    assert!(px.red() >= 254 && px.green() <= 1 && px.blue() <= 1, "{px:?}");
}

#[test]
fn hostile_sizes() {
    let (mut d, p, r) = doc_with_rect(Color::BLACK);
    let n = d.get_mut(r).unwrap();
    n.width = f64::NAN;
    n.height = 1e12;
    n.effects.push(Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(1e9)), visible: true });
    let _ = render_view(&d, p, &view(64, 64), &Options::default());
    let _ = render_node(&d, r, f64::NAN);
    let _ = render_view(&d, p, &View { origin: (f64::NAN, 0.0), scale: 0.0, width: 0, height: 0 }, &Options::default());
}

#[test]
fn inner_shadow_reaches_every_edge() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    d.get_mut(r).unwrap().effects.push(Effect {
        kind: EffectKind::InnerShadow(Shadow { color: Color::BLACK, offset: (0.0, 4.0), radius: 8.0, ..Default::default() }),
        visible: true,
    });
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // Left edge (no offset that way) is still shaded; the centre is not.
    assert!(px(&pix, 11, 25)[0] < 235, "{:?}", px(&pix, 11, 25));
    assert!(px(&pix, 35, 30)[0] > 245);
}

#[test]
fn rect_clip_frame_matches_same_rect_fill_on_fractional_edges() {
    let white = Options { background: Some(Color::WHITE), ..Default::default() };
    for (x, y, w, h) in [(0.5, 0.5, 10.0, 10.0), (0.0, 0.0, 10.5, 10.5), (0.0, 0.0, 10.0, 10.0)] {
        let mut clipped = figcraft_doc::Document::new("clipped");
        let p = clipped.first_page().unwrap();
        let f = clipped.create(p, NodeKind::Frame, "F", x, y, w, h).unwrap();
        clipped.get_mut(f).unwrap().fills.clear();
        clipped.get_mut(f).unwrap().clip = true;
        let c = clipped.create(f, NodeKind::Rectangle, "C", -5.0, -5.0, w + 10.0, h + 10.0).unwrap();
        clipped.get_mut(c).unwrap().fills = vec![Paint::solid(Color::BLACK)];

        let mut direct = figcraft_doc::Document::new("direct");
        let dp = direct.first_page().unwrap();
        let r = direct.create(dp, NodeKind::Rectangle, "R", x, y, w, h).unwrap();
        direct.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];

        let a = render_view(&clipped, p, &view(12, 12), &white).unwrap();
        let e = render_view(&direct, dp, &view(12, 12), &white).unwrap();
        for (px_x, px_y) in [(0, 0), (0, 5), (5, 0), (5, 5), (10, 5), (5, 10), (10, 10), (11, 11)] {
            assert_eq!(px(&a, px_x, px_y), px(&e, px_x, px_y), "frame {x},{y} {w}x{h} at ({px_x},{px_y})");
        }
    }
}
