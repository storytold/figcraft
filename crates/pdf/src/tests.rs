use std::io::Read;
use std::sync::Arc;

use super::*;
use figcraft_color::{BlendMode, Color, Gradient, GradientKind, ImageScaleMode, Paint};
use figcraft_doc::{BoolOp, Effect, EffectKind, ImageData, NodeKind, StrokeAlign, TextData};

pub(crate) fn page_count(pdf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(pdf);
    s.matches("/Type/Page").count() - s.matches("/Type/Pages").count()
}

/// Every stream inflated (krilla deflates content, fonts and CMaps), joined.
pub(crate) fn inflated(pdf: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(off) = find(&pdf[i..], b"stream") {
        let mut start = i + off + 6;
        if pdf.get(start) == Some(&b'\r') {
            start += 1;
        }
        if pdf.get(start) == Some(&b'\n') {
            start += 1;
        }
        let Some(end) = find(&pdf[start..], b"endstream") else { break };
        let mut z = flate2::read::ZlibDecoder::new(&pdf[start..start + end]);
        let mut buf = Vec::new();
        if z.read_to_end(&mut buf).is_ok() {
            out.push_str(&String::from_utf8_lossy(&buf));
        }
        i = start + end + 9;
    }
    out
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
    let mut p = figcraft_render::Pixmap::new(w, h).unwrap();
    for (i, px) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let v = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 40 };
        *px = [v, 80, 255 - v, 255];
    }
    figcraft_render::encode_png(&p).unwrap()
}

pub(crate) fn has_inter() -> bool {
    figcraft_text::find_face("Inter", "Regular").is_some()
}

/// A page with two frames: shapes, gradients, an image, strokes, a mask, a boolean, effects, text.
pub(crate) fn sample_doc() -> (Document, Vec<NodeId>) {
    let mut d = Document::new("sample");
    let p = d.first_page().unwrap();
    let f1 = d.create(p, NodeKind::Frame, "Card", 0.0, 0.0, 320.0, 240.0).unwrap();
    let f2 = d.create(p, NodeKind::Frame, "Second", 400.0, 0.0, 200.0, 240.0).unwrap();
    d.get_mut(f1).unwrap().fills = vec![Paint::solid(Color::from_hex("F4F1EA").unwrap())];
    // Solid rect with an inside stroke and rounded corners.
    let r = d.create(f1, NodeKind::Rectangle, "R", 16.0, 16.0, 120.0, 80.0).unwrap();
    {
        let n = d.get_mut(r).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("3366FF").unwrap())];
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 6.0;
        n.stroke.align = StrokeAlign::Inside;
        n.corners.radii = [12.0; 4];
        n.effects = vec![Effect::drop_shadow()];
    }
    // Linear gradient ellipse with an outside stroke.
    let e = d.create(f1, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "E", 160.0, 16.0, 120.0, 80.0).unwrap();
    {
        let n = d.get_mut(e).unwrap();
        n.fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::from_hex("FF5500").unwrap()))];
        n.strokes = vec![Paint::solid(Color::from_hex("222222").unwrap())];
        n.stroke.weight = 4.0;
        n.stroke.align = StrokeAlign::Outside;
    }
    // Radial gradient, rotated star.
    let st = d.create(f1, NodeKind::Star { count: 5, inner_ratio: 0.4 }, "S", 30.0, 120.0, 90.0, 90.0).unwrap();
    {
        let n = d.get_mut(st).unwrap();
        n.fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Radial, Color::from_hex("00AA66").unwrap()))];
        n.transform = Affine::translate((75.0, 165.0)) * Affine::rotate(0.4) * Affine::translate((-45.0, -45.0));
    }
    // Image fill.
    let bytes = png(32, 24);
    d.images.insert("img1".into(), ImageData { bytes: Arc::new(bytes), width: 32, height: 24 });
    let im = d.create(f1, NodeKind::Rectangle, "Img", 140.0, 120.0, 80.0, 60.0).unwrap();
    d.get_mut(im).unwrap().fills = vec![Paint::image("img1", ImageScaleMode::Fill)];
    // Text.
    let mut td = TextData::new("Hello PDF");
    td.style.font_size = 24.0;
    let t = d.create(f1, NodeKind::Text(td), "T", 16.0, 200.0, 280.0, 32.0).unwrap();
    d.get_mut(t).unwrap().fills = vec![Paint::solid(Color::from_hex("111111").unwrap())];
    // A child sticking out of the frame (clipped).
    d.create(f1, NodeKind::Rectangle, "Out", 280.0, 200.0, 100.0, 100.0).unwrap();

    // Second frame: mask, boolean, angular + diamond gradients, blend modes, inner shadow, blur.
    let g = d.create(f2, NodeKind::Group, "Masked group", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Mask", 10.0, 10.0, 80.0, 80.0).unwrap();
    d.get_mut(m).unwrap().is_mask = true;
    let masked = d.create(g, NodeKind::Rectangle, "Masked", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(masked).unwrap().fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Angular, Color::from_hex("8844FF").unwrap()))];
    let a = d.create(f2, NodeKind::Rectangle, "A", 110.0, 10.0, 60.0, 60.0).unwrap();
    let b = d.create(f2, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "B", 140.0, 40.0, 50.0, 50.0).unwrap();
    let bo = d.create(f2, NodeKind::BooleanOp { op: BoolOp::Subtract }, "Bool", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.reparent(a, bo, None).unwrap();
    d.reparent(b, bo, None).unwrap();
    d.fit_to_children(bo).unwrap();
    d.get_mut(bo).unwrap().fills = vec![Paint::solid(Color::from_hex("CC2244").unwrap())];
    let di = d.create(f2, NodeKind::Rectangle, "Diamond", 10.0, 120.0, 80.0, 80.0).unwrap();
    {
        let n = d.get_mut(di).unwrap();
        n.fills = vec![Paint::gradient(Gradient::default_for(GradientKind::Diamond, Color::from_hex("FFAA00").unwrap()))];
        n.effects = vec![Effect::inner_shadow()];
    }
    let bl = d.create(f2, NodeKind::Rectangle, "Blend", 100.0, 120.0, 80.0, 80.0).unwrap();
    {
        let n = d.get_mut(bl).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("00CCFF").unwrap())];
        n.blend = BlendMode::Multiply;
        n.opacity = 0.7;
    }
    let blur = d.create(f2, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Blurred", 120.0, 160.0, 60.0, 60.0).unwrap();
    d.get_mut(blur).unwrap().effects = vec![Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(6.0)), visible: true }];
    let frames = d.top_frames(p);
    (d, frames)
}

#[test]
fn multi_frame_document_one_page_each() {
    let (d, frames) = sample_doc();
    assert_eq!(frames.len(), 2);
    let pdf = export_pdf(&d, &frames, &PdfOptions::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert_eq!(page_count(&pdf), 2);
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/MediaBox[0 0 320 240]"), "page size from the frame box");
    assert!(s.contains("/MediaBox[0 0 200 240]"));
    // Images: the PNG fill plus effect rasters.
    assert!(s.contains("/Subtype/Image"));
    // Shadings for the vector gradients.
    assert!(s.contains("/ShadingType"));
}

#[test]
fn text_is_real_text() {
    if !has_inter() {
        return;
    }
    let (d, frames) = sample_doc();
    let pdf = export_pdf(&d, &frames[..1], &PdfOptions::default()).unwrap();
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/FontFile2") || s.contains("/FontFile3"), "an embedded font");
    assert!(s.contains("/ToUnicode"));
    let all = inflated(&pdf);
    // "H" maps to U+0048 in the ToUnicode CMap; the content uses text operators.
    assert!(all.contains("<0048>"), "ToUnicode for H");
    assert!(all.contains("Tj") || all.contains("TJ"));
    // Outlined text has no font.
    let outlined = export_pdf(&d, &frames[..1], &PdfOptions { outline_text: true, ..Default::default() }).unwrap();
    let so = String::from_utf8_lossy(&outlined);
    assert!(!so.contains("/FontFile"));
}

#[test]
fn nodes_on_one_page() {
    let (d, frames) = sample_doc();
    let pdf = export_nodes_pdf(&d, &frames).unwrap();
    assert_eq!(page_count(&pdf), 1);
    assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox[0 0 600 240]"));
}

#[test]
fn every_paint_and_scale_mode() {
    let mut d = Document::new("p");
    let p = d.first_page().unwrap();
    d.images.insert("i".into(), ImageData { bytes: Arc::new(png(16, 16)), width: 16, height: 16 });
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 400.0, 100.0).unwrap();
    for (i, mode) in [ImageScaleMode::Fill, ImageScaleMode::Fit, ImageScaleMode::Tile, ImageScaleMode::Crop].into_iter().enumerate() {
        let r = d.create(f, NodeKind::Rectangle, "R", i as f64 * 100.0, 0.0, 90.0, 60.0).unwrap();
        let mut paint = Paint::image("i", mode);
        if let figcraft_color::PaintKind::Image(img) = &mut paint.kind {
            img.rotation = i as i32;
            img.scale = 0.5;
            img.filters.exposure = if i == 3 { 0.3 } else { 0.0 };
        }
        paint.opacity = 0.8;
        d.get_mut(r).unwrap().fills = vec![paint];
    }
    // A missing image and a corrupt one draw nothing.
    d.images.insert("bad".into(), ImageData { bytes: Arc::new(vec![0x89, b'P', b'N', b'G', 1, 2, 3]), width: 9, height: 9 });
    let r = d.create(f, NodeKind::Rectangle, "Bad", 0.0, 70.0, 20.0, 20.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("missing", ImageScaleMode::Fill), Paint::image("bad", ImageScaleMode::Fill)];
    let pdf = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(String::from_utf8_lossy(&pdf).contains("/PatternType 1"), "tiled image pattern");
    if let Ok(stem) = std::env::var("FIGCRAFT_PDF_SAMPLE") {
        std::fs::write(format!("{stem}-images.pdf"), &pdf).unwrap();
        let pix = figcraft_render::render_node(&d, f, 2.0).unwrap();
        std::fs::write(format!("{stem}-images.png"), figcraft_render::encode_png(&pix).unwrap()).unwrap();
    }
}

#[test]
fn hostile_inputs_error_or_skip() {
    let mut d = Document::new("h");
    let p = d.first_page().unwrap();
    assert_eq!(export_pdf(&d, &[], &PdfOptions::default()), Err(PdfError::Empty));
    assert_eq!(export_pdf(&d, &[NodeId(99_999)], &PdfOptions::default()), Err(PdfError::NoNode(NodeId(99_999))));
    let huge = d.create(p, NodeKind::Frame, "Huge", 0.0, 0.0, 1e9, 10.0).unwrap();
    assert!(matches!(export_pdf(&d, &[huge], &PdfOptions::default()), Err(PdfError::TooLarge(..))));
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    // NaN geometry, NaN transform, NaN opacity, empty vector, absurd stroke, NaN gradient.
    let a = d.create(f, NodeKind::Rectangle, "NaN", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(a).unwrap().width = f64::NAN;
    let b = d.create(f, NodeKind::Rectangle, "NaNxf", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(b).unwrap().transform = Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let c = d.create(f, NodeKind::Rectangle, "NaNop", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(c).unwrap().opacity = f32::NAN;
    d.create(f, NodeKind::Vector { network: Default::default() }, "Empty", 0.0, 0.0, 0.0, 0.0).unwrap();
    let s = d.create(f, NodeKind::Rectangle, "Stroke", 10.0, 10.0, 10.0, 10.0).unwrap();
    {
        let n = d.get_mut(s).unwrap();
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 1e7;
        n.stroke.align = StrokeAlign::Outside;
        let mut g = Gradient::default_for(GradientKind::Linear, Color::BLACK);
        g.to = (f64::NAN, 0.0);
        n.fills = vec![Paint::gradient(g)];
        n.effects = vec![Effect {
            kind: EffectKind::DropShadow(figcraft_doc::Shadow { radius: f64::NAN, spread: 1e9, ..Default::default() }),
            visible: true,
        }];
    }
    let t = d.create(f, NodeKind::Text(TextData::new("x")), "T", 0.0, 0.0, 10.0, 10.0).unwrap();
    if let NodeKind::Text(td) = &mut d.get_mut(t).unwrap().kind {
        td.style.font_size = f64::NAN;
    }
    let pdf = export_pdf(&d, &[f], &PdfOptions { raster_scale: f64::NAN, ..Default::default() }).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    // Deep nesting beyond the depth limit must refuse the export, not return partial artwork.
    let mut parent = f;
    for _ in 0..400 {
        parent = d.create(parent, NodeKind::Group, "g", 0.0, 0.0, 1.0, 1.0).unwrap();
    }
    d.create(parent, NodeKind::Rectangle, "deep", 0.0, 0.0, 5.0, 5.0).unwrap();
    assert!(matches!(export_pdf(&d, &[f], &PdfOptions::default()), Err(PdfError::Depth(_))));
    // A zero-size node still exports (as a tiny page); a NaN-sized frame errors or exports.
    let z = d.create(p, NodeKind::Rectangle, "Zero", 0.0, 0.0, 0.0, 0.0).unwrap();
    assert!(export_pdf(&d, &[z], &PdfOptions::default()).is_ok());
    let nf = d.create(p, NodeKind::Frame, "NaNFrame", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(nf).unwrap().height = f64::NAN;
    let _ = export_pdf(&d, &[nf], &PdfOptions::default());
}

/// Writes a sample PDF and the renderer's PNG of the same frames for a visual comparison:
/// `FIGCRAFT_PDF_SAMPLE=/path/stem cargo test -p figcraft-pdf sample`.
#[test]
fn sample_files() {
    let Ok(stem) = std::env::var("FIGCRAFT_PDF_SAMPLE") else { return };
    let (d, frames) = sample_doc();
    let pdf = export_pdf(&d, &frames, &PdfOptions::default()).unwrap();
    std::fs::write(format!("{stem}.pdf"), pdf).unwrap();
    for (i, f) in frames.iter().enumerate() {
        let pix = figcraft_render::render_node(&d, *f, 2.0).unwrap();
        std::fs::write(format!("{stem}-{}.png", i + 1), figcraft_render::encode_png(&pix).unwrap()).unwrap();
        std::fs::write(format!("{stem}-{}.pdf", i + 1), export_pdf(&d, &[*f], &PdfOptions::default()).unwrap()).unwrap();
    }
}
