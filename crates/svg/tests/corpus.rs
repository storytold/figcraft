//! The SVG import corpus: synthetic files covering what design and illustration tools write.
//!
//! Each `tests/corpus/*.svg` carries its expected pixels as `<!-- probe X Y #RRGGBBAA ~TOL -->`
//! comments, worked out by hand from the SVG. The import is rendered with FigCraft's renderer and
//! every probe checked; the layer structure is checked per file; and every file must survive an
//! export and a second import looking the same.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::{View, to_rgba8};
use figcraft_svg::{SvgOptions, export_svg, import_svg};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");

fn corpus() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(DIR)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    v.sort();
    assert!(v.len() >= 15, "corpus missing");
    v
}

fn source(name: &str) -> String {
    std::fs::read_to_string(format!("{DIR}/{name}.svg")).unwrap()
}

pub fn import(src: &[u8]) -> (Document, NodeId) {
    let mut d = Document::new("corpus");
    let p = d.first_page().unwrap();
    let f = import_svg(&mut d, src, p, 0.0, 0.0, "Imported").unwrap();
    assert!(d.validate().is_empty(), "{:?}", d.validate());
    (d, f)
}

/// The frame's area rendered at 1x, straight RGBA.
fn render(d: &Document, frame: NodeId) -> (Vec<u8>, u32, u32) {
    let n = d.node(frame).unwrap();
    let (w, h) = (n.width.ceil().max(1.0) as u32, n.height.ceil().max(1.0) as u32);
    let o = d.world_transform(frame).translation();
    let view = View { origin: (o.x, o.y), scale: 1.0, width: w, height: h };
    let mut pix = figcraft_render::Pixmap::new(w, h).unwrap();
    figcraft_render::render_nodes_into(d, &[frame], &view, &mut pix);
    (to_rgba8(&pix), w, h)
}

struct Probe {
    x: u32,
    y: u32,
    rgba: [u8; 4],
    tol: u8,
}

fn probes(src: &str) -> Vec<Probe> {
    src.lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("<!-- probe ")?.strip_suffix("-->")?;
            let mut it = l.split_whitespace();
            let x = it.next()?.parse().ok()?;
            let y = it.next()?.parse().ok()?;
            let hex = it.next()?.strip_prefix('#')?;
            let b = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
            let rgba = [b(0)?, b(2)?, b(4)?, b(6)?];
            let tol = it.next().and_then(|t| t.strip_prefix('~')).and_then(|t| t.parse().ok()).unwrap_or(8);
            Some(Probe { x, y, rgba, tol })
        })
        .collect()
}

fn pixel(px: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

/// Probe mismatches of `name` as messages.
fn check(name: &str, src: &str, d: &Document, f: NodeId) -> Vec<String> {
    let (px, w, h) = render(d, f);
    let mut bad = Vec::new();
    for p in probes(src) {
        if p.x >= w || p.y >= h {
            bad.push(format!("{name}: probe {} {} outside {w}x{h}", p.x, p.y));
            continue;
        }
        let got = pixel(&px, w, p.x, p.y);
        let diff = |i: usize| got[i].abs_diff(p.rgba[i]);
        // Colour only means something where there's enough coverage.
        let ok = if p.rgba[3] < 32 { diff(3) <= p.tol } else { (0..4).all(|i| diff(i) <= p.tol) };
        if !ok {
            bad.push(format!("{name}: probe {} {} expected {:02x?} got {:02x?} (±{})", p.x, p.y, p.rgba, got, p.tol));
        }
    }
    bad
}

#[test]
fn corpus_renders_match_the_probes() {
    let mut bad = Vec::new();
    let mut count = 0;
    for (name, src) in corpus() {
        let (d, f) = import(src.as_bytes());
        count += probes(&src).len();
        bad.extend(check(&name, &src, &d, f));
    }
    assert!(count > 140, "{count} probes");
    assert!(bad.is_empty(), "{} of {count} probes failed:\n{}", bad.len(), bad.join("\n"));
}

/// Mean absolute difference per channel between two renders of the same size.
fn mean_diff(a: &[u8], b: &[u8]) -> f64 {
    let sum: u64 = a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    sum as f64 / a.len().max(1) as f64
}

#[test]
fn corpus_round_trips_through_export() {
    let mut bad = Vec::new();
    for (name, src) in corpus() {
        let (d, f) = import(src.as_bytes());
        let svg = export_svg(&d, &[f], &SvgOptions::default()).unwrap();
        let (d2, f2) = import(svg.as_bytes());
        let (a, w, h) = render(&d, f);
        let (b, w2, h2) = render(&d2, f2);
        if (w, h) != (w2, h2) {
            bad.push(format!("{name}: size {w}x{h} became {w2}x{h2}"));
            continue;
        }
        let m = mean_diff(&a, &b);
        if m > 1.5 {
            bad.push(format!("{name}: mean difference {m:.2} after export and import"));
        }
        // The probes still hold on the second import.
        bad.extend(check(&format!("{name} (round trip)"), &src, &d2, f2).into_iter().filter(|_| !name.starts_with("text")));
        // And exporting again gives the same file.
        let again = export_svg(&d2, &[f2], &SvgOptions::default()).unwrap();
        let (d3, f3) = import(again.as_bytes());
        let (c, ..) = render(&d3, f3);
        if mean_diff(&b, &c) > 0.5 {
            bad.push(format!("{name}: not stable on a second round trip"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Writes each import's render and layer tree to `$SVG_DUMP_DIR` (run with `--ignored`).
#[test]
#[ignore]
fn dump() {
    let Ok(dir) = std::env::var("SVG_DUMP_DIR") else { return };
    for (name, src) in corpus() {
        let (d, f) = import(src.as_bytes());
        let (px, w, h) = render(&d, f);
        let img = image_png(&px, w, h);
        std::fs::write(format!("{dir}/{name}.png"), img).unwrap();
        let mut tree = String::new();
        outline(&d, f, 0, &mut tree);
        std::fs::write(format!("{dir}/{name}.txt"), tree).unwrap();
    }
}

fn image_png(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut pix = figcraft_render::Pixmap::new(w, h).unwrap();
    for (i, c) in px.chunks(4).enumerate() {
        let a = u32::from(c[3]);
        let pm = |v: u8| ((u32::from(v) * a + 127) / 255) as u8;
        let d = pix.data_mut();
        d[i * 4] = pm(c[0]);
        d[i * 4 + 1] = pm(c[1]);
        d[i * 4 + 2] = pm(c[2]);
        d[i * 4 + 3] = c[3];
    }
    figcraft_render::encode_png(&pix).unwrap()
}

fn outline(d: &Document, id: NodeId, depth: usize, out: &mut String) {
    let n = d.node(id).unwrap();
    let c = n.transform.as_coeffs();
    let extra = match &n.kind {
        NodeKind::Text(t) => format!(" {:?} {} {} {:?}", t.characters, t.style.font_family, t.style.font_size, t.align_h),
        _ => String::new(),
    };
    out.push_str(&format!(
        "{}{} {:?} {:.1}x{:.1} [{:.2} {:.2} {:.2} {:.2} {:.1} {:.1}] fills={} strokes={} fx={} mask={} op={}{extra}\n",
        "  ".repeat(depth),
        n.kind.type_name(),
        n.name,
        n.width,
        n.height,
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        n.fills.len(),
        n.strokes.len(),
        n.effects.len(),
        n.is_mask,
        n.opacity
    ));
    for k in &n.children {
        outline(d, *k, depth + 1, out);
    }
}

// ---- Layer structure: editable and sensible ----

fn imported(name: &str) -> (Document, NodeId) {
    import(source(name).as_bytes())
}

/// The first layer called `name` under `f`.
fn layer<'a>(d: &'a Document, f: NodeId, name: &str) -> &'a figcraft_doc::Node {
    d.subtree(f).into_iter().filter_map(|i| d.node(i)).find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name:?}"))
}

fn kinds(d: &Document, f: NodeId) -> Vec<&'static str> {
    d.subtree(f).into_iter().skip(1).filter_map(|i| d.node(i)).map(|n| n.kind.type_name()).collect()
}

fn count(d: &Document, f: NodeId, kind: &str) -> usize {
    kinds(d, f).iter().filter(|k| **k == kind).count()
}

#[test]
fn groups_keep_their_ids_and_transforms_bake_into_shapes() {
    let (d, f) = imported("transforms");
    for g in ["moved", "turned", "scaled", "skewed"] {
        assert_eq!(layer(&d, f, g).kind.type_name(), "GROUP");
    }
    // Rotated: still a 40×40 rectangle, turned 45°.
    let b = layer(&d, f, "b");
    assert_eq!(b.kind.type_name(), "RECTANGLE");
    assert!((b.width - 40.0).abs() < 1e-6 && (b.height - 40.0).abs() < 1e-6);
    let c = d.world_transform(d.subtree(f).into_iter().find(|i| d.node(*i).is_some_and(|n| n.name == "b")).unwrap()).as_coeffs();
    assert!((c[1].atan2(c[0]).to_degrees() - 45.0).abs() < 1e-6, "{c:?}");
    // Scaled 2 × 1.5: the size takes it.
    let sc = layer(&d, f, "c");
    assert_eq!((sc.kind.type_name(), sc.width.round(), sc.height.round()), ("RECTANGLE", 40.0, 30.0));
    // Skewed: only a vector can take it.
    assert_eq!(layer(&d, f, "d").kind.type_name(), "VECTOR");
}

#[test]
fn fills_close_open_subpaths() {
    let (d, f) = imported("open_subpaths");
    let loops = |n: &figcraft_doc::Node| match &n.kind {
        NodeKind::Vector { network } => network.regions.iter().map(|r| r.loops.len()).sum::<usize>(),
        _ => 0,
    };
    // One filled layer: all four subpaths are fill loops, the 0.001 gap and the triangle included.
    let filled = layer(&d, f, "filled");
    assert_eq!((filled.kind.type_name(), loops(filled), filled.strokes.len()), ("VECTOR", 4, 0));
    // Filled and stroked with an open triangle: a fill layer under the stroke, which stays open.
    let (fill, stroke) = (layer(&d, f, "stroked fill"), layer(&d, f, "stroked"));
    assert_eq!((loops(fill), fill.fills.len(), fill.strokes.len()), (2, 1, 0));
    assert_eq!((loops(stroke), stroke.fills.len(), stroke.strokes.len()), (0, 0, 1));
    // Only near-closed ends: one layer, closed, stroke and all.
    let near = layer(&d, f, "near");
    assert_eq!((loops(near), near.fills.len(), near.strokes.len()), (1, 1, 1));
    let order: Vec<&str> = d.node(f).unwrap().children.iter().filter_map(|c| d.node(*c)).map(|n| n.name.as_str()).collect();
    assert_eq!(order, ["filled", "stroked fill", "stroked", "near", "under", "under fill"], "paint order: fill then stroke, or the stroke first");
}

#[test]
fn basic_shapes_stay_rectangles_ellipses_and_lines() {
    let (d, f) = imported("primitives");
    let card = layer(&d, f, "card");
    assert_eq!((card.kind.type_name(), card.width, card.height), ("RECTANGLE", 50.0, 30.0));
    assert_eq!(card.corners.radii, [8.0; 4]);
    assert_eq!(count(&d, f, "ELLIPSE"), 2);
    assert_eq!(count(&d, f, "LINE"), 2);
    // Elliptical corners, a polyline and a polygon are vectors.
    assert_eq!(count(&d, f, "VECTOR"), 3);
    let line = d.subtree(f).into_iter().filter_map(|i| d.node(i)).find(|n| n.kind.type_name() == "LINE").unwrap();
    assert_eq!((line.width, line.stroke.weight, line.strokes.len()), (180.0, 2.0, 1));
}

#[test]
fn strokes_keep_dashes_caps_and_joins() {
    use figcraft_doc::{StrokeCap, StrokeJoin};
    let (d, f) = imported("strokes");
    assert_eq!(layer(&d, f, "dashed").stroke.dashes, vec![20.0, 10.0]);
    assert_eq!(layer(&d, f, "round-cap").stroke.cap_end, StrokeCap::Round);
    assert_eq!(layer(&d, f, "square-cap").stroke.cap_start, StrokeCap::Square);
    assert_eq!(layer(&d, f, "butt-cap").stroke.cap_start, StrokeCap::None);
    assert_eq!(layer(&d, f, "round-join").stroke.join, StrokeJoin::Round);
    assert_eq!(layer(&d, f, "bevel").stroke.join, StrokeJoin::Bevel);
    assert_eq!(layer(&d, f, "miter").stroke.miter_limit, 4.0);
}

#[test]
fn gradients_stay_gradients() {
    use figcraft_color::{GradientKind, PaintKind};
    let (d, f) = imported("gradients");
    for (name, kind) in
        [("bbox-units", GradientKind::Linear), ("rotated", GradientKind::Linear), ("radial", GradientKind::Radial), ("focal", GradientKind::Radial)]
    {
        let n = layer(&d, f, name);
        assert!(matches!(&n.fills[0].kind, PaintKind::Gradient(g) if g.kind == kind), "{name}: {:?}", n.fills);
    }
    // Reflect over four periods: the stops are unrolled.
    let PaintKind::Gradient(g) = &layer(&d, f, "reflect").fills[0].kind else { panic!() };
    assert_eq!(g.stops.len(), 8, "{:?}", g.stops);
}

#[test]
fn clip_paths_and_masks_become_mask_layers() {
    use figcraft_doc::MaskType;
    let (d, f) = imported("clip_mask");
    let mask_of = |name: &str| {
        let id = d.subtree(f).into_iter().find(|i| d.node(*i).is_some_and(|n| n.name == name)).unwrap();
        let parent = d.node(d.node(id).unwrap().parent.unwrap()).unwrap();
        let m = d.node(parent.children[0]).unwrap();
        assert!(m.is_mask, "{name}: {parent:?}");
        (m.kind.type_name(), m.mask_type, parent.name.clone())
    };
    assert_eq!(mask_of("clipped"), ("ELLIPSE", MaskType::Vector, "Mask group".into()));
    assert_eq!(mask_of("masked"), ("GROUP", MaskType::Luminance, "Mask group".into()));
    assert_eq!(mask_of("faded").1, MaskType::Luminance);
    assert_eq!(mask_of("alpha-masked").1, MaskType::Alpha);
    // The clipped group keeps its name, with the clip at the bottom.
    let g = layer(&d, f, "clipped-group");
    assert!(d.node(g.children[0]).unwrap().is_mask && g.children.len() == 3);
}

#[test]
fn filters_become_effects() {
    use figcraft_doc::EffectKind;
    let (d, f) = imported("filters");
    let fx = |name: &str| layer(&d, f, name).effects.iter().map(|e| e.kind.clone()).collect::<Vec<_>>();
    assert!(matches!(&fx("blurred")[..], [EffectKind::LayerBlur(b)] if (b.radius - 8.0).abs() < 1e-6));
    let [EffectKind::DropShadow(s)] = &fx("dropped")[..] else { panic!("{:?}", fx("dropped")) };
    assert_eq!((s.offset, s.radius, s.color.a), ((4.0, 6.0), 4.0, 0.5));
    let [EffectKind::DropShadow(s)] = &fx("classic-shadow")[..] else { panic!("{:?}", fx("classic-shadow")) };
    assert_eq!((s.offset, s.radius), ((0.0, 4.0), 6.0));
    assert!((s.color.r - 1.0).abs() < 1e-6 && (s.color.a - 0.25).abs() < 1e-6);
    let [EffectKind::DropShadow(drop), EffectKind::InnerShadow(inner)] = &fx("matrixed")[..] else { panic!("{:?}", fx("matrixed")) };
    assert_eq!((drop.offset, drop.radius, inner.offset, inner.radius), ((2.0, 2.0), 2.0, (0.0, -3.0), 3.0));
    assert!(!drop.show_behind, "cut out under the layer");
    assert!(fx("unsupported").is_empty());
}

#[test]
fn unsupported_parts_are_reported() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let imported = figcraft_svg::import_svg_with_notes(&mut d, source("filters").as_bytes(), p, 0.0, 0.0, "x").unwrap();
    assert!(imported.notes.iter().any(|n| n.contains("noise")), "{:?}", imported.notes);
    let clean = figcraft_svg::import_svg_with_notes(&mut d, source("primitives").as_bytes(), p, 0.0, 0.0, "x").unwrap();
    assert!(clean.notes.is_empty(), "{:?}", clean.notes);
}

#[test]
fn blend_modes_opacity_and_hidden_layers() {
    use figcraft_color::BlendMode;
    let (d, f) = imported("blend_visibility");
    assert_eq!(layer(&d, f, "multiplied").blend, BlendMode::Multiply);
    assert_eq!(layer(&d, f, "screened").blend, BlendMode::Screen);
    for gone in ["gone", "hidden"] {
        assert!(d.subtree(f).into_iter().all(|i| d.node(i).is_some_and(|n| n.name != gone)), "{gone}");
    }
    assert_eq!(layer(&d, f, "shown").kind.type_name(), "RECTANGLE");
    let (d, f) = imported("opacity");
    assert_eq!(layer(&d, f, "faded").opacity, 0.5);
}

#[test]
fn sizes_follow_units_and_view_boxes() {
    let size = |name: &str| {
        let (d, f) = imported(name);
        let n = d.node(f).unwrap();
        (n.width.round(), n.height.round())
    };
    assert_eq!(size("units"), (100.0, 96.0));
    assert_eq!(size("viewbox_meet"), (200.0, 100.0));
    assert_eq!(size("illustration"), (120.0, 80.0));
    assert_eq!(size("icon"), (48.0, 48.0));
    // The icon's 2 px strokes on a 24 grid drawn at 48 px are 4 px.
    let (d, f) = imported("icon");
    let ring = d.subtree(f).into_iter().filter_map(|i| d.node(i)).find(|n| n.kind.type_name() == "ELLIPSE").unwrap();
    assert_eq!((ring.width, ring.stroke.weight, ring.fills.len()), (36.0, 4.0, 0));
}

#[test]
fn use_and_symbols_resolve_without_extra_wrappers() {
    let (d, f) = imported("use_symbol");
    assert_eq!(kinds(&d, f), ["ELLIPSE", "RECTANGLE", "RECTANGLE"]);
    let (d, f) = imported("illustration");
    assert_eq!(layer(&d, f, "Walls").kind.type_name(), "GROUP");
    assert!(d.subtree(f).into_iter().all(|i| !d.node(i).unwrap().is_mask));
}

#[test]
fn images_are_image_fills() {
    use figcraft_color::{ImageScaleMode, PaintKind};
    let (d, f) = imported("image");
    let photo = layer(&d, f, "photo");
    assert_eq!((photo.kind.type_name(), photo.width, photo.height), ("RECTANGLE", 40.0, 20.0));
    assert!(matches!(&photo.fills[0].kind, PaintKind::Image(i) if i.scale_mode == ImageScaleMode::Fill));
    assert_eq!(d.images.len(), 1, "the same image is stored once");
    assert_eq!(d.images.values().next().map(|i| (i.width, i.height)), Some((4, 2)));
    // Patterns: a repeating tile, and images (drawn or embedded) shown once.
    let (d, f) = imported("patterns");
    let mode = |name: &str| match &layer(&d, f, name).fills[0].kind {
        PaintKind::Image(i) => i.scale_mode,
        k => panic!("{name}: {k:?}"),
    };
    assert_eq!((mode("tiled"), mode("spotted"), mode("pictured")), (ImageScaleMode::Tile, ImageScaleMode::Fill, ImageScaleMode::Fill));
}

#[test]
fn text_becomes_text_layers() {
    use figcraft_doc::{LetterSpacing, LineHeight, TextAlignH};
    let (d, f) = imported("text");
    let texts: Vec<&figcraft_doc::Node> = d.subtree(f).into_iter().filter_map(|i| d.node(i)).filter(|n| n.kind.text().is_some()).collect();
    if figcraft_text::fontdb::find_face("Inter", "Regular").is_none() {
        // Without any font usvg can't lay text out (CI without fonts).
        return;
    }
    assert_eq!(texts.len(), 5, "{:?}", texts.iter().map(|t| &t.name).collect::<Vec<_>>());
    let t = |name: &str| layer(&d, f, name).kind.text().unwrap().clone();
    let title = t("title");
    assert_eq!((title.characters.as_str(), title.style.font_size, title.style.font_style.as_str()), ("Hello", 24.0, "Bold"));
    let sans = ["Inter", "Helvetica", "Arial", "Segoe UI", "Roboto", "Noto Sans", "DejaVu Sans", "Liberation Sans"];
    assert!(sans.contains(&title.style.font_family.as_str()), "{}", title.style.font_family);
    let centered = t("Centered");
    assert_eq!((centered.align_h, centered.style.letter_spacing), (TextAlignH::Center, LetterSpacing::Pixels(2.0)));
    assert_eq!(t("End").align_h, TextAlignH::Right);
    assert_eq!(t("End").style.font_style, "Italic");
    // Two tspans 20 apart: one layer, two lines.
    let lines = t("Line one");
    assert_eq!((lines.characters.as_str(), lines.style.line_height), ("Line one\nLine two", LineHeight::Pixels(20.0)));
    // A bold green tspan: a run.
    let run = t("Plain bold run");
    assert_eq!(run.runs.len(), 1, "{:?}", run.runs);
    assert_eq!((run.runs[0].start, run.runs[0].end, run.runs[0].style.font_style.as_str()), (6, 10, "Bold"));
    assert!(run.runs[0].style.fills.is_some());
    // The first baseline sits on y; the anchor on x.
    for (name, x, y) in [("title", 10.0, 40.0), ("Centered", 150.0, 80.0), ("End", 290.0, 120.0)] {
        let id = d.subtree(f).into_iter().find(|i| d.node(*i).is_some_and(|n| n.name == name)).unwrap();
        let n = d.node(id).unwrap();
        let (l, _) = figcraft_text::layout_node(n).unwrap();
        let o = d.world_transform(id).translation();
        let k = match n.kind.text().unwrap().align_h {
            TextAlignH::Center => 0.5,
            TextAlignH::Right => 1.0,
            _ => 0.0,
        };
        assert!((o.x + k * n.width - x).abs() < 0.01, "{name}: x {}", o.x);
        assert!((o.y + l.lines[0].baseline - y).abs() < 0.01, "{name}: y {}", o.y);
    }
}
