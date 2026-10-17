//! PDF export depth: mask types, background blur, text hyperlinks, image deduplication, page sizes
//! and hostile inputs. Every document here is synthetic.

use std::sync::Arc;

use super::*;
use crate::tests::{inflated, page_count, png};
use figcraft_color::{Color, ImageScaleMode, Paint};
use figcraft_doc::{Effect, EffectKind, ImageData, MaskType, NodeKind, StyleRun, TextData};

const URL: &str = "https://example.com/figcraft";

fn ellipse() -> NodeKind {
    NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }
}

fn text(pdf: &[u8]) -> String {
    String::from_utf8_lossy(pdf).into_owned()
}

fn count(pdf: &[u8], needle: &str) -> usize {
    text(pdf).matches(needle).count()
}

/// A frame with a group masked by an ellipse of `kind` over a gradient rectangle.
fn masked_doc(kind: MaskType) -> (Document, NodeId) {
    let mut d = Document::new("masks");
    let p = d.first_page().unwrap_or(NodeId(0));
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 120.0, 120.0).unwrap();
    let g = d.create(f, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, ellipse(), "Mask", 10.0, 10.0, 80.0, 80.0).unwrap();
    {
        let n = d.get_mut(m).unwrap();
        n.is_mask = true;
        n.mask_type = kind;
        // A half-transparent grey: the alpha and luminance masks see different things.
        n.fills = vec![Paint::solid(Color::from_hex("80808080").unwrap())];
    }
    let r = d.create(g, NodeKind::Rectangle, "Content", 0.0, 0.0, 100.0, 100.0).unwrap();
    // Opaque: the only soft masks in the file are the mask layer's.
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::from_hex("2255FF").unwrap())];
    (d, f)
}

#[test]
fn mask_types_alpha_luminance_and_vector() {
    let soft = |kind| {
        let (d, f) = masked_doc(kind);
        export_pdf(&d, &[f], &PdfOptions::default()).unwrap()
    };
    let alpha = soft(MaskType::Alpha);
    assert!(text(&alpha).contains("/S/Alpha"), "an alpha soft mask");
    let lum = soft(MaskType::Luminance);
    assert!(text(&lum).contains("/S/Luminosity"), "a luminosity soft mask");
    assert!(!text(&lum).contains("/S/Alpha"));
    // A vector mask is the mask's outline as a clip path: no soft mask at all, and the clip's
    // curves (the ellipse) are in the content.
    let vector = soft(MaskType::Vector);
    let s = text(&vector);
    assert!(!s.contains("/SMask") && !s.contains("/S/Alpha") && !s.contains("/S/Luminosity"), "{s}");
    let content = inflated(&vector);
    let ops: Vec<&str> = content.split_whitespace().collect();
    assert!(ops.windows(2).filter(|w| matches!(w[0], "W" | "W*") && w[1] == "n").count() >= 2, "frame and mask clips: {content}");
    assert!(content.contains(" c"), "curves of the ellipse outline");
}

#[test]
fn degenerate_vector_mask_exports() {
    let (mut d, f) = masked_doc(MaskType::Vector);
    let m = d.nodes().find(|n| n.name == "Mask").map(|n| n.id).unwrap();
    // A zero-size mask: an empty clip (nothing shows through), and nothing breaks.
    d.get_mut(m).unwrap().width = 0.0;
    d.get_mut(m).unwrap().height = 0.0;
    let pdf = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

/// A frame with stripes under a frosted panel (background blur).
fn frosted_doc(radius: f64) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("frost");
    let p = d.first_page().unwrap_or(NodeId(0));
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 200.0, 120.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::WHITE)];
    for i in 0..10 {
        let r = d.create(f, NodeKind::Rectangle, "Stripe", f64::from(i) * 20.0, 0.0, 10.0, 120.0).unwrap();
        d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::from_hex("E0245E").unwrap())];
    }
    let panel = d.create(f, NodeKind::Rectangle, "Panel", 40.0, 30.0, 120.0, 60.0).unwrap();
    {
        let n = d.get_mut(panel).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("FFFFFF40").unwrap())];
        n.corners.radii = [12.0; 4];
        n.effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(radius)), visible: true }];
    }
    // Drawn after the panel: never part of its backdrop.
    let above = d.create(f, NodeKind::Rectangle, "Above", 0.0, 0.0, 30.0, 30.0).unwrap();
    d.get_mut(above).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    (d, f, panel)
}

#[test]
fn background_blur_is_a_blurred_backdrop_image() {
    let (d, f, panel) = frosted_doc(16.0);
    let pdf = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    // The blurred backdrop (an RGBA image: colour plus its alpha as a soft mask).
    assert!(count(&pdf, "/Subtype/Image") >= 1);
    // Without the effect there is nothing to rasterise.
    let mut plain = d.clone();
    plain.get_mut(panel).unwrap().effects.clear();
    let pdf = export_pdf(&plain, &[f], &PdfOptions::default()).unwrap();
    assert_eq!(count(&pdf, "/Subtype/Image"), 0);
}

#[test]
fn backdrop_leaves_out_the_layer_and_what_is_above_it() {
    let (d, f, panel) = frosted_doc(8.0);
    let mut tmp = d.clone();
    figcraft_render::backdrop::hide_above(&mut tmp, panel, &[f]);
    let hidden: Vec<&str> = tmp.nodes().filter(|n| !n.visible).map(|n| n.name.as_str()).collect();
    assert_eq!(hidden.len(), 2, "{hidden:?}");
    assert!(hidden.contains(&"Panel") && hidden.contains(&"Above"));
    // Nested: later siblings at every level, and later layers of the page, go too.
    let mut d = d;
    let p = d.first_page().unwrap_or(NodeId(0));
    let g = d.create(f, NodeKind::Group, "G", 0.0, 0.0, 10.0, 10.0).unwrap();
    let inner = d.create(g, NodeKind::Rectangle, "Inner", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.create(g, NodeKind::Rectangle, "Sibling", 0.0, 0.0, 10.0, 10.0).unwrap();
    let later = d.create(p, NodeKind::Frame, "Later", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(f).unwrap().strokes = vec![Paint::solid(Color::BLACK)];
    let mut tmp = d.clone();
    figcraft_render::backdrop::hide_above(&mut tmp, inner, &[f, later]);
    let hidden: Vec<&str> = tmp.nodes().filter(|n| !n.visible).map(|n| n.name.as_str()).collect();
    assert_eq!(hidden.len(), 3, "{hidden:?}");
    assert!(["Inner", "Sibling", "Later"].iter().all(|h| hidden.contains(h)), "{hidden:?}");
    assert!(tmp.node(f).unwrap().strokes.is_empty(), "strokes paint over children");
    assert!(tmp.nodes().filter(|n| n.name == "Stripe").all(|n| n.visible));
}

#[test]
fn text_hyperlinks_become_link_annotations() {
    let mut d = Document::new("links");
    let p = d.first_page().unwrap_or(NodeId(0));
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 100.0).unwrap();
    let mut td = TextData::new("Read the docs here");
    td.style.font_size = 20.0;
    let mut linked = td.style.clone();
    linked.link = Some(URL.into());
    // "docs" (chars 9..13) links.
    td.runs = vec![StyleRun { start: 9, end: 13, style: linked }];
    let t = d.create(f, NodeKind::Text(td), "T", 10.0, 10.0, 280.0, 40.0).unwrap();
    d.get_mut(t).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let Some((layout, _)) = figcraft_text::layout_node(d.node(t).unwrap()) else { return };
    if layout.glyphs.is_empty() {
        // No fonts on this machine: nothing to lay out or link.
        return;
    }
    let pdf = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    let s = text(&pdf);
    assert_eq!(s.matches("/Subtype/Link").count(), 1, "{s}");
    assert!(s.contains(URL), "the URL is in the link action");
    // The link covers part of the line, inside the page.
    let rect = s.split("/Subtype/Link").nth(1).and_then(|r| r.split("/Rect[").nth(1)).and_then(|r| r.split(']').next()).unwrap();
    let v: Vec<f64> = rect.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    assert_eq!(v.len(), 4, "{rect}");
    assert!(v[0] > 10.0 && v[2] < 290.0 && v[2] > v[0], "{v:?}");
    assert!(v[1] >= 0.0 && v[3] <= 100.0 && v[3] > v[1], "{v:?}");

    // Links inside a mask layer aren't clickable; outlined text keeps its links.
    let outlined = export_pdf(&d, &[f], &PdfOptions { outline_text: true, ..Default::default() }).unwrap();
    assert_eq!(text(&outlined).matches("/Subtype/Link").count(), 1);
    d.get_mut(t).unwrap().is_mask = true;
    let masked = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    assert_eq!(text(&masked).matches("/Subtype/Link").count(), 0);
}

#[test]
fn hostile_links_are_skipped() {
    let mut d = Document::new("links");
    let p = d.first_page().unwrap_or(NodeId(0));
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 50.0).unwrap();
    for (i, url) in ["   ", &"x".repeat(MAX_URL + 1), "javascript:alert(1)\u{0}"].into_iter().enumerate() {
        let mut td = TextData::new("Link");
        td.style.link = Some(url.to_string());
        // One off the page entirely.
        let x = if i == 2 { 500.0 } else { 0.0 };
        d.create(f, NodeKind::Text(td), "T", x, 0.0, 80.0, 20.0).unwrap();
    }
    let pdf = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    assert_eq!(text(&pdf).matches("/Subtype/Link").count(), 0);
    assert!(link_annotation(Rect::new(f64::NAN, 0.0, 1.0, 1.0), URL.into(), 10.0, 10.0).is_none());
    assert!(link_annotation(Rect::new(-50.0, -50.0, -10.0, -10.0), URL.into(), 10.0, 10.0).is_none());
    assert!(link_annotation(Rect::new(-5.0, -5.0, 5.0, 5.0), URL.into(), 10.0, 10.0).is_some(), "cut to the page");
}

#[test]
fn images_are_embedded_once() {
    let mut d = Document::new("dedupe");
    let p = d.first_page().unwrap_or(NodeId(0));
    d.images.insert("same".into(), ImageData { bytes: Arc::new(png(16, 16)), width: 16, height: 16 });
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 400.0, 100.0).unwrap();
    for i in 0..4 {
        let r = d.create(f, NodeKind::Rectangle, "R", f64::from(i) * 100.0, 0.0, 90.0, 90.0).unwrap();
        d.get_mut(r).unwrap().fills = vec![Paint::image("same", ImageScaleMode::Fill)];
    }
    // The same image on a second page is still one image.
    let f2 = d.create(p, NodeKind::Frame, "F2", 0.0, 200.0, 100.0, 100.0).unwrap();
    let r = d.create(f2, NodeKind::Rectangle, "R", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::image("same", ImageScaleMode::Fit)];
    let pdf = export_pdf(&d, &[f, f2], &PdfOptions::default()).unwrap();
    assert_eq!(page_count(&pdf), 2);
    assert_eq!(count(&pdf, "/Subtype/Image"), 1, "one image object for every use");
    // Adjusted copies are images of their own.
    if let figcraft_color::PaintKind::Image(img) = &mut d.get_mut(r).unwrap().fills[0].kind {
        img.filters.exposure = 0.5;
    }
    let pdf = export_pdf(&d, &[f, f2], &PdfOptions::default()).unwrap();
    assert_eq!(count(&pdf, "/Subtype/Image"), 2);
}

#[test]
fn page_sizes_are_frame_sizes_in_points() {
    let mut d = Document::new("sizes");
    let p = d.first_page().unwrap_or(NodeId(0));
    let a4 = d.create(p, NodeKind::Frame, "A4", 0.0, 0.0, 595.0, 842.0).unwrap();
    let phone = d.create(p, NodeKind::Frame, "Phone", 700.0, 0.0, 393.0, 852.0).unwrap();
    // A rotated frame exports its bounding box.
    let rot = d.create(p, NodeKind::Frame, "Rotated", 0.0, 1000.0, 100.0, 100.0).unwrap();
    d.get_mut(rot).unwrap().transform =
        Affine::translate((50.0, 1050.0)) * Affine::rotate(std::f64::consts::FRAC_PI_4) * Affine::translate((-50.0, -50.0));
    let pdf = export_groups(&d, &[vec![a4], vec![phone], vec![rot]], &PdfOptions { title: Some("Sizes".into()), ..Default::default() }).unwrap();
    let s = text(&pdf);
    assert_eq!(page_count(&pdf), 3);
    assert!(s.contains("/MediaBox[0 0 595 842]") && s.contains("/MediaBox[0 0 393 852]"), "{s}");
    let side = 100.0 * std::f64::consts::SQRT_2;
    let rotated = s.split("/MediaBox[0 0 ").filter_map(|r| r.split(' ').next()?.parse::<f64>().ok()).any(|w| (w - side).abs() < 0.01);
    assert!(rotated, "rotated frame box");
    assert!(s.contains("Sizes"), "title metadata");
}

#[test]
fn hostile_effects_and_masks_never_panic() {
    for radius in [f64::NAN, f64::INFINITY, -5.0, 0.0, 1e12] {
        let (d, f, _) = frosted_doc(radius);
        assert!(export_pdf(&d, &[f], &PdfOptions::default()).is_ok(), "radius {radius}");
    }
    for kind in MaskType::ALL {
        let (mut d, f) = masked_doc(kind);
        let m = d.nodes().find(|n| n.name == "Mask").map(|n| n.id).unwrap();
        for w in [f64::NAN, 1e15, -10.0] {
            d.get_mut(m).unwrap().width = w;
            let _ = export_pdf(&d, &[f], &PdfOptions::default());
        }
        // A mask with nothing above it, and a mask that is the frame's only child.
        let g = d.nodes().find(|n| n.name == "G").map(|n| n.id).unwrap();
        let kids = d.node(g).unwrap().children.clone();
        if let Some(last) = kids.last() {
            d.remove(*last).unwrap();
        }
        assert!(export_pdf(&d, &[f], &PdfOptions::default()).is_ok());
    }
    // Background blur on a page root itself, and nested three groups deep.
    let (mut d, f, _) = frosted_doc(8.0);
    d.get_mut(f).unwrap().effects = vec![Effect { kind: EffectKind::BackgroundBlur(figcraft_doc::Blur::uniform(4.0)), visible: true }];
    assert!(export_pdf(&d, &[f], &PdfOptions::default()).is_ok());
}

/// Writes PDFs and PNG renders of the documents above for a visual comparison:
/// `FIGCRAFT_PDF_SAMPLE=/path/stem cargo test -p figcraft-pdf depth_samples`.
#[test]
fn depth_samples() {
    let Ok(stem) = std::env::var("FIGCRAFT_PDF_SAMPLE") else { return };
    let mut docs: Vec<(String, Document, NodeId)> = MaskType::ALL
        .into_iter()
        .map(|k| {
            let (d, f) = masked_doc(k);
            (format!("mask-{}", k.id()), d, f)
        })
        .collect();
    let (d, f, _) = frosted_doc(16.0);
    docs.push(("frost".into(), d, f));
    for (name, d, f) in docs {
        std::fs::write(format!("{stem}-{name}.pdf"), export_pdf(&d, &[f], &PdfOptions::default()).unwrap()).unwrap();
        let pix = figcraft_render::render_node(&d, f, 2.0).unwrap();
        std::fs::write(format!("{stem}-{name}.png"), figcraft_render::encode_png(&pix).unwrap()).unwrap();
    }
}

/// A frame with a hyperlinked word, set in red (`None` without a font on this machine).
fn linked_doc() -> Option<(Document, NodeId)> {
    let mut d = Document::new("links");
    let p = d.first_page()?;
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 100.0).ok()?;
    let mut td = TextData::new("Read the docs here");
    td.style.font_size = 20.0;
    let mut linked = td.style.clone();
    linked.link = Some(URL.into());
    td.runs = vec![StyleRun { start: 9, end: 13, style: linked }];
    let t = d.create(f, NodeKind::Text(td), "T", 10.0, 10.0, 280.0, 40.0).ok()?;
    d.get_mut(t).ok()?.fills = vec![Paint::solid(Color::from_hex("FF0000")?)];
    let (layout, _) = figcraft_text::layout_node(d.node(t)?)?;
    (!layout.glyphs.is_empty()).then_some((d, f))
}

#[test]
fn without_annotations_the_pdf_has_no_links_and_the_page_is_the_same() {
    let Some((d, f)) = linked_doc() else { return };
    let with = export_pdf(&d, &[f], &PdfOptions::default()).unwrap();
    assert!(PdfOptions::default().annotations, "links are written by default");
    assert_eq!(count(&with, "/Subtype/Link"), 1);
    for opts in [
        PdfOptions { annotations: false, ..Default::default() },
        PdfOptions { annotations: false, outline_text: true, ..Default::default() },
        PdfOptions { annotations: false, grayscale: true, ..Default::default() },
        PdfOptions {
            annotations: false,
            fit: Some(Fit { paper: (595.28, 841.89), orientation: FitOrientation::Auto, scale: FitScale::Shrink, margin: 18.0 }),
            ..Default::default()
        },
    ] {
        let pdf = export_pdf(&d, &[f], &opts).unwrap();
        let s = text(&pdf);
        for k in ["/Annots", "/Link", "/URI", URL] {
            assert!(!s.contains(k), "{k} in a PDF without annotations");
        }
    }
    // The page content (text, its colour) is the same with and without the annotations.
    let without = export_pdf(&d, &[f], &PdfOptions { annotations: false, ..Default::default() }).unwrap();
    assert!(inflated(&without).contains("1 0 0 rg"), "the red text is still drawn");
    assert_eq!(inflated(&with), inflated(&without));
}
