//! Sketch styles → paints, strokes, effects, opacity, blend modes and masks.

use figcraft_doc::color::{BlendMode, ColorProfile, GradientKind, ImageScaleMode, PaintKind};
use figcraft_doc::{Document, EffectKind, NodeKind, StrokeAlign, StrokeCap, StrokeJoin};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;

fn open_doc(document_json: Value, layers: Vec<Value>, files: &[(&str, &[u8])]) -> (Document, Vec<String>) {
    let z = sketch_zip("50.2", 103, document_json, &[page("P", "Page", vec![artboard("A", "Board", 0.0, 0.0, 400.0, 400.0, layers)])], files);
    import(&z, "f").unwrap()
}

fn open(layers: Vec<Value>) -> (Document, Vec<String>) {
    open_doc(document(&["P"]), layers, &[])
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d figcraft_doc::Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer named {name}"))
}

fn r(name: &str) -> Value {
    rect(name, 0.0, 0.0, 100.0, 50.0)
}

#[test]
fn solid_and_disabled_fills() {
    let mut second = solid_fill(0.0, 0.0, 1.0, 1.0);
    second["isEnabled"] = json!(false);
    let mut half = solid_fill(1.0, 0.0, 0.0, 1.0);
    half["contextSettings"]["opacity"] = json!(0.5);
    let (doc, _) = open(vec![styled(r("Two"), "fills", json!([half, second]))]);
    let fills = &node(&doc, "Two").fills;
    assert_eq!(fills.len(), 2);
    assert!(matches!(fills[0].kind, PaintKind::Solid { color } if color.r == 1.0 && color.b == 0.0));
    assert!((fills[0].opacity - 0.5).abs() < 1e-6 && fills[0].visible);
    assert!(!fills[1].visible);
}

#[test]
fn collapsed_shape_group_takes_the_groups_style() {
    let inner = with(r("Inner"), "booleanOperation", json!(-1));
    let g = styled(shape_group("Button", 0.0, 0.0, 100.0, 50.0, vec![inner]), "fills", json!([solid_fill(1.0, 0.0, 0.0, 1.0)]));
    let (doc, _) = open(vec![g]);
    assert!(matches!(node(&doc, "Button").fills[0].kind, PaintKind::Solid { color } if color.r == 1.0 && color.g == 0.0));
}

#[test]
fn boolean_shape_group_takes_the_groups_style() {
    let a = with(rect("A", 0.0, 0.0, 50.0, 50.0), "booleanOperation", json!(-1));
    let b = with(rect("B", 10.0, 10.0, 20.0, 20.0), "booleanOperation", json!(1));
    let g = styled(shape_group("Ring", 0.0, 0.0, 50.0, 50.0, vec![a, b]), "fills", json!([solid_fill(0.0, 1.0, 0.0, 1.0)]));
    let (doc, _) = open(vec![g]);
    assert_eq!(node(&doc, "Ring").fills.len(), 1);
}

#[test]
fn gradients() {
    let stops = json!([{"_class": "gradientStop", "position": 0, "color": color(1.0, 0.0, 0.0, 1.0)},
                       {"_class": "gradientStop", "position": 1, "color": color(0.0, 0.0, 1.0, 1.0)}]);
    let grad = |ty: i64, ellipse: f64| {
        json!({"_class": "fill", "isEnabled": true, "fillType": 1, "color": color(0.0, 0.0, 0.0, 1.0),
        "gradient": {"_class": "gradient", "gradientType": ty, "from": "{0.5, 0}", "to": "{0.5, 1}", "elipseLength": ellipse, "stops": stops.clone()}})
    };
    let (doc, _) = open(vec![
        styled(r("Linear"), "fills", json!([grad(0, 0.0)])),
        styled(r("Radial"), "fills", json!([grad(1, 0.5)])),
        styled(r("Angular"), "fills", json!([grad(2, 0.0)])),
    ]);
    let g = |name: &str| match &node(&doc, name).fills[0].kind {
        PaintKind::Gradient(g) => g.clone(),
        other => panic!("{name}: {other:?}"),
    };
    let lin = g("Linear");
    assert_eq!(lin.kind, GradientKind::Linear);
    assert_eq!((lin.from, lin.to), ((0.5, 0.0), (0.5, 1.0)));
    assert_eq!(lin.stops.len(), 2);
    assert_eq!(lin.stops[1].color.b, 1.0);
    assert_eq!(g("Radial").kind, GradientKind::Radial);
    // Sketch starts an angular gradient on the right and turns clockwise, whatever its from/to.
    let ang = g("Angular");
    assert_eq!(ang.kind, GradientKind::Angular);
    assert_eq!((ang.from, ang.to, ang.width), ((0.5, 0.5), (1.0, 0.5), (0.5, 1.0)));
    // The radial gradient's width handle: half the length, across the main axis (in pixels).
    let rad = g("Radial");
    let (dx, dy) = ((rad.width.0 - rad.from.0) * 100.0, (rad.width.1 - rad.from.1) * 50.0);
    assert!(((dx * dx + dy * dy).sqrt() - 25.0).abs() < 1e-6, "{rad:?}");
}

#[test]
fn pattern_fill_modes() {
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let pat = |mode: i64| {
        json!({"_class": "fill", "isEnabled": true, "fillType": 4, "patternFillType": mode, "patternTileScale": 1,
        "image": {"_class": "MSJSONFileReference", "_ref": "images/p1.png"}})
    };
    let layers = (0..4).map(|m| styled(r(&format!("M{m}")), "fills", json!([pat(m)]))).collect();
    let (doc, _) = open_doc(document(&["P"]), layers, &[("images/p1.png", &png)]);
    let mode = |name: &str| match &node(&doc, name).fills[0].kind {
        PaintKind::Image(i) => i.scale_mode,
        other => panic!("{other:?}"),
    };
    assert_eq!(mode("M0"), ImageScaleMode::Tile);
    assert_eq!(mode("M1"), ImageScaleMode::Fill);
    assert_eq!(mode("M2"), ImageScaleMode::Crop);
    assert_eq!(mode("M3"), ImageScaleMode::Fit);
    assert_eq!(doc.images.len(), 1);
}

#[test]
fn noise_fill_warns() {
    let noise = json!({"_class": "fill", "isEnabled": true, "fillType": 5, "noiseIntensity": 0.3});
    let (_, warnings) = open(vec![styled(r("Noisy"), "fills", json!([noise]))]);
    assert!(warnings.iter().any(|w| w.contains("noise")), "{warnings:?}");
}

#[test]
fn borders() {
    let border = |pos: i64, t: f64| json!({"_class": "border", "isEnabled": true, "fillType": 0, "color": color(0.0, 0.0, 0.0, 1.0), "position": pos, "thickness": t});
    let opts = json!({"_class": "borderOptions", "isEnabled": true, "dashPattern": [4, 2], "lineCapStyle": 1, "lineJoinStyle": 2});
    let (doc, warnings) = open(vec![
        styled(r("Center"), "borders", json!([border(0, 2.0)])),
        styled(styled(r("Inside"), "borders", json!([border(1, 3.0)])), "borderOptions", opts),
        styled(r("Outside"), "borders", json!([border(2, 1.0)])),
        styled(r("Two"), "borders", json!([border(1, 1.0), border(1, 5.0)])),
    ]);
    let s = |n: &str| node(&doc, n).stroke.clone();
    assert_eq!((s("Center").align, s("Center").weight), (StrokeAlign::Center, 2.0));
    assert_eq!((s("Inside").align, s("Inside").weight), (StrokeAlign::Inside, 3.0));
    assert_eq!(s("Inside").dashes, vec![4.0, 2.0]);
    assert_eq!((s("Inside").cap_start, s("Inside").join), (StrokeCap::Round, StrokeJoin::Bevel));
    assert_eq!(s("Outside").align, StrokeAlign::Outside);
    assert_eq!(node(&doc, "Two").strokes.len(), 2);
    assert_eq!(s("Two").weight, 1.0);
    assert!(warnings.iter().any(|w| w.contains("borders")), "{warnings:?}");
}

#[test]
fn shadows_and_blurs() {
    let shadow = json!({"_class": "shadow", "isEnabled": true, "blurRadius": 8, "offsetX": 2, "offsetY": 4, "spread": 1, "color": color(0.0, 0.0, 0.0, 0.5),
        "contextSettings": {"_class": "graphicsContextSettings", "blendMode": 0, "opacity": 1}});
    let mut off = shadow.clone();
    off["isEnabled"] = json!(false);
    let blur = |ty: i64| json!({"_class": "blur", "isEnabled": true, "radius": 6, "type": ty});
    let (doc, warnings) = open(vec![
        styled(styled(r("Shadowed"), "shadows", json!([shadow, off])), "innerShadows", json!([shadow])),
        styled(r("Blurred"), "blur", blur(0)),
        styled(r("Frosted"), "blur", blur(3)),
        styled(r("Moving"), "blur", blur(1)),
    ]);
    let fx = &node(&doc, "Shadowed").effects;
    assert_eq!(fx.len(), 3);
    match &fx[0].kind {
        EffectKind::DropShadow(s) => assert_eq!((s.offset, s.radius, s.spread, s.color.a), ((2.0, 4.0), 8.0, 1.0, 0.5)),
        other => panic!("{other:?}"),
    }
    assert!(!fx[1].visible);
    assert!(matches!(fx[2].kind, EffectKind::InnerShadow(_)));
    assert!(matches!(node(&doc, "Blurred").effects[0].kind, EffectKind::LayerBlur(_)));
    assert!(matches!(node(&doc, "Frosted").effects[0].kind, EffectKind::BackgroundBlur(_)));
    // Motion blur: drawn as a plain blur of half the radius.
    assert!(matches!(node(&doc, "Moving").effects[0].kind, EffectKind::LayerBlur(_)));
    assert!(warnings.iter().any(|w| w.contains("motion")), "{warnings:?}");
}

#[test]
fn opacity_and_blend() {
    let modes = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::PlusDarker,
        BlendMode::PlusLighter,
    ];
    let layers = (0..modes.len())
        .map(|i| styled(r(&format!("B{i}")), "contextSettings", json!({"_class": "graphicsContextSettings", "opacity": 0.3, "blendMode": i})))
        .collect();
    let (doc, _) = open(layers);
    for (i, m) in modes.iter().enumerate() {
        let n = node(&doc, &format!("B{i}"));
        assert!((n.opacity - 0.3).abs() < 1e-6);
        // A normal layer passes through like any other.
        if i > 0 {
            assert_eq!(n.blend, *m, "mode {i}");
        }
    }
}

#[test]
fn clipping_mask_chain() {
    let mask = with(r("Mask"), "hasClippingMask", json!(true));
    let a = r("A");
    let b = with(r("B"), "shouldBreakMaskChain", json!(true));
    let c = r("C");
    let (doc, _) = open(vec![group("G", 0.0, 0.0, 100.0, 50.0, vec![mask, a, b, c])]);
    let m = node(&doc, "Mask");
    assert!(m.is_mask);
    // The mask and what it clips share a group of their own, so B and C stay unclipped.
    let holder = m.parent.unwrap();
    let kids: Vec<&str> = doc.node(holder).unwrap().children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
    assert_eq!(kids, ["Mask", "A"]);
    assert_ne!(node(&doc, "B").parent, Some(holder));
    let bb = doc.world_bounds(node(&doc, "A").id).unwrap();
    assert_eq!((bb.x0, bb.y0), (0.0, 0.0));
}

#[test]
fn display_p3_document() {
    let mut d = document(&["P"]);
    d["colorSpace"] = json!(2);
    let (doc, _) = open_doc(d, vec![], &[]);
    assert_eq!(doc.color_profile, ColorProfile::DisplayP3);
}

#[test]
fn style_on_frames_and_text_kinds_untouched() {
    // An artboard keeps its background when its style has no fills.
    let (doc, _) = open(vec![]);
    assert!(matches!(node(&doc, "Board").kind, NodeKind::Frame));
    assert_eq!(node(&doc, "Board").fills.len(), 1);
}

#[test]
fn shape_group_masks_clip() {
    // Sketch 51 and older: the mask is a shape group.
    let inner = with(r("Outline"), "booleanOperation", json!(-1));
    let mask = with(shape_group("Card", 0.0, 0.0, 100.0, 50.0, vec![inner]), "hasClippingMask", json!(true));
    let a = rect("A", 0.0, 0.0, 300.0, 300.0);
    let b = with(r("B"), "shouldBreakMaskChain", json!(true));
    let (doc, _) = open(vec![group("G", 0.0, 0.0, 300.0, 300.0, vec![mask, a, b])]);
    let m = node(&doc, "Card");
    assert!(m.is_mask);
    let holder = doc.node(m.parent.unwrap()).unwrap();
    let kids: Vec<&str> = holder.children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
    assert_eq!(kids, ["Card", "A"]);
}

#[test]
fn masked_icon_keeps_its_geometry() {
    let oval = {
        let mut o = with(rect("Path", 0.0, 0.0, 67.0, 67.0), "booleanOperation", json!(-1));
        o["_class"] = json!("oval");
        o
    };
    let mask = styled(
        with(shape_group("Circle", 0.0, 0.0, 67.0, 67.0, vec![oval]), "hasClippingMask", json!(true)),
        "fills",
        json!([solid_fill(0.9, 0.9, 0.9, 1.0)]),
    );
    let p = |id: &str, x: f64, y: f64, w: f64, h: f64, op: i64| {
        with(shape_path(id, x, y, w, h, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], true), "booleanOperation", json!(op))
    };
    let content = styled(
        shape_group(
            "Body",
            12.0,
            12.0,
            45.0,
            62.0,
            vec![p("P1", 9.0, 9.0, 22.0, 22.0, -1), p("P2", 25.0, 15.0, 20.0, 15.0, 0), p("P3", 0.0, 17.0, 31.0, 45.0, 0)],
        ),
        "fills",
        json!([solid_fill(0.5, 0.5, 0.5, 1.0)]),
    );
    let (doc, _) = open(vec![group("Icon", 152.0, 20.0, 67.0, 67.0, vec![mask, content])]);
    let b = doc.world_bounds(node(&doc, "Circle").id).unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (152.0, 20.0, 67.0, 67.0), "mask");
    let b = doc.world_bounds(node(&doc, "Body").id).unwrap();
    assert!((b.x0 - 164.0).abs() < 1e-6 && (b.y0 - 41.0).abs() < 1e-6 && (b.width() - 45.0).abs() < 1e-6, "body {b:?}");
    assert!(matches!(node(&doc, "Circle").kind, NodeKind::Ellipse { .. }));
}

#[test]
fn two_styled_masks_each_draw() {
    let fill = json!([solid_fill(0.5, 0.5, 0.5, 1.0)]);
    let m1 = styled(with(r("Mask1"), "hasClippingMask", json!(true)), "fills", fill.clone());
    let m2 = styled(with(rect("Mask2", 200.0, 0.0, 100.0, 50.0), "hasClippingMask", json!(true)), "fills", fill);
    let (doc, _) = open(vec![group("G", 0.0, 0.0, 300.0, 50.0, vec![m1, r("A"), m2, rect("B", 200.0, 0.0, 10.0, 10.0)])]);
    // The drawn copy of the second mask isn't inside the first mask's group.
    let copy = doc.nodes().find(|n| n.name == "Mask2 (drawn)").unwrap();
    let first_group = node(&doc, "Mask1").parent.unwrap();
    assert_ne!(copy.parent, Some(first_group));
}

#[test]
fn bitmap_color_controls_become_filters() {
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let mut bitmap = r("Photo");
    bitmap["_class"] = json!("bitmap");
    bitmap["image"] = json!({"_class": "MSJSONFileReference", "_ref": "images/a.png"});
    bitmap["style"]["colorControls"] =
        json!({"_class": "colorControls", "isEnabled": true, "brightness": 0, "contrast": 1, "hue": 0.5, "saturation": 0});
    let (doc, warnings) = open_doc(document(&["P"]), vec![bitmap], &[("images/a.png", &png)]);
    let PaintKind::Image(img) = &node(&doc, "Photo").fills[0].kind else { panic!("no image") };
    assert_eq!(img.filters.saturation, -1.0);
    assert_eq!(img.filters.contrast, 0.0);
    assert!(warnings.iter().any(|w| w.contains("hue")), "{warnings:?}");
}

/// An alpha mask (`clippingMaskMode` 1) only masks: Sketch doesn't draw its fill under the content.
#[test]
fn alpha_masks_draw_no_copy() {
    let fill = json!([solid_fill(0.0, 0.0, 0.0, 1.0)]);
    let m = styled(with(with(r("Fade"), "hasClippingMask", json!(true)), "clippingMaskMode", json!(1)), "fills", fill);
    let (doc, _) = open(vec![group("G", 0.0, 0.0, 100.0, 50.0, vec![m, r("Photo")])]);
    assert!(!doc.nodes().any(|n| n.name == "Fade (drawn)"));
    assert_eq!(node(&doc, "Fade").mask_type, figcraft_doc::MaskType::Alpha);
}

/// A progressive opacity (Fade: `contextSettings.isProgressive` with a gradient) becomes an alpha
/// mask of that gradient over the layer, in a group in the layer's place.
#[test]
fn fades_become_alpha_masks() {
    let fade = json!({"_class": "graphicsContextSettings", "blendMode": 0, "opacity": 1, "isProgressive": true,
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0.5, 0}", "to": "{0.5, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(0.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(0.0, 0.0, 0.0, 0.0)}]}});
    let l = styled(rect("Photo", 30.0, 40.0, 100.0, 50.0), "contextSettings", fade);
    let (doc, _) = open(vec![group("G", 0.0, 0.0, 200.0, 200.0, vec![l])]);
    // The group takes the layer's name too.
    let photo = doc.nodes().find(|n| n.name == "Photo" && matches!(n.kind, NodeKind::Rectangle)).unwrap();
    let wrap = doc.node(photo.parent.unwrap()).unwrap();
    assert_eq!(wrap.name, "Photo");
    assert!(matches!(wrap.kind, NodeKind::Group), "{:?}", wrap.kind);
    let mask = doc.node(wrap.children[0]).unwrap();
    assert!(mask.is_mask && mask.mask_type == figcraft_doc::MaskType::Alpha);
    assert!(matches!(mask.fills[0].kind, PaintKind::Gradient(_)));
    assert_eq!(doc.world_bounds(mask.id), doc.world_bounds(photo.id));
    let b = doc.world_bounds(photo.id).unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (30.0, 40.0, 100.0, 50.0));
}

/// Sketch's linear gradients run square in pixels: on a 200×100 layer, from the top left to the
/// bottom right, the stripes are at right angles to that line in pixels, not in the layer's 0–1.
#[test]
fn linear_gradients_are_square_in_pixels() {
    let g = json!({"_class": "fill", "isEnabled": true, "fillType": 1, "color": color(0.0, 0.0, 0.0, 1.0),
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0, 0}", "to": "{1, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(0.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(1.0, 1.0, 1.0, 1.0)}]}});
    let (doc, _) = open(vec![styled(rect("Wide", 0.0, 0.0, 200.0, 100.0), "fills", json!([g]))]);
    let PaintKind::Gradient(g) = &node(&doc, "Wide").fills[0].kind else { panic!("not a gradient") };
    // The line runs (200, 100) in pixels; square to it, (−100, 200), is (−0.5, 2) in 0–1.
    let (wx, wy) = ((g.width.0 - g.from.0) * 200.0, (g.width.1 - g.from.1) * 100.0);
    assert!((wx * 200.0 + wy * 100.0).abs() < 1e-6, "{g:?}");
}
