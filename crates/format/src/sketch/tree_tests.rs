//! Sketch layers → document layers: kinds, positions, transforms, constraints, visibility.

use figcraft_doc::color::PaintKind;
use figcraft_doc::{Document, HConstraint, NodeKind, VConstraint};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;

/// Import one page holding `layers`, with extra archive `files`.
fn open_with(layers: Vec<Value>, files: &[(&str, &[u8])]) -> (Document, Vec<String>) {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "Page", layers)], files);
    import(&z, "f").unwrap()
}

fn open(layers: Vec<Value>) -> (Document, Vec<String>) {
    open_with(layers, &[])
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d figcraft_doc::Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer named {name}"))
}

fn solid(doc: &Document, name: &str) -> Option<[f32; 4]> {
    node(doc, name).fills.iter().find_map(|p| match &p.kind {
        PaintKind::Solid { color } => Some([color.r, color.g, color.b, p.opacity]),
        _ => None,
    })
}

#[test]
fn artboard_is_frame_with_white_fill_unless_background() {
    let plain = artboard("A", "Plain", 0.0, 0.0, 100.0, 50.0, vec![]);
    let red = with(artboard("B", "Red", 200.0, 0.0, 100.0, 50.0, vec![]), "hasBackgroundColor", json!(true));
    let red = with(red, "backgroundColor", json!({"_class": "color", "red": 1, "green": 0, "blue": 0, "alpha": 1}));
    let (doc, _) = open(vec![plain, red]);
    assert!(matches!(node(&doc, "Plain").kind, NodeKind::Frame));
    assert!(node(&doc, "Plain").clip);
    assert_eq!(solid(&doc, "Plain"), Some([1.0, 1.0, 1.0, 1.0]));
    assert_eq!(solid(&doc, "Red"), Some([1.0, 0.0, 0.0, 1.0]));
    let b = doc.world_bounds(node(&doc, "Red").id).unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (200.0, 0.0, 100.0, 50.0));
}

#[test]
fn group_children_positions_accumulate() {
    let r = rect("R", 5.0, 5.0, 10.0, 10.0);
    let g = group("G", 10.0, 20.0, 15.0, 15.0, vec![r]);
    let (doc, _) = open(vec![artboard("A", "Board", 100.0, 100.0, 300.0, 300.0, vec![g])]);
    assert!(matches!(node(&doc, "G").kind, NodeKind::Group));
    let b = doc.world_bounds(node(&doc, "R").id).unwrap();
    assert!((b.x0 - 115.0).abs() < 1e-6 && (b.y0 - 125.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn rotation_and_flip() {
    let rotated = with(rect("Turned", 0.0, 0.0, 40.0, 20.0), "rotation", json!(90));
    let flipped = with(rect("Flipped", 100.0, 0.0, 40.0, 20.0), "isFlippedHorizontal", json!(true));
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 300.0, 300.0, vec![rotated, flipped])]);
    let b = doc.world_bounds(node(&doc, "Turned").id).unwrap();
    assert!((b.width() - 20.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "{b:?}");
    assert!((b.center().x - 20.0).abs() < 1e-6 && (b.center().y - 10.0).abs() < 1e-6, "{b:?}");
    // Counter-clockwise on screen: the box's x axis points up.
    let t = doc.world_transform(node(&doc, "Turned").id).as_coeffs();
    assert!(t[1] < -0.99, "{t:?}");
    let t = doc.world_transform(node(&doc, "Flipped").id).as_coeffs();
    assert!(t[0] < -0.99, "{t:?}");
    let b = doc.world_bounds(node(&doc, "Flipped").id).unwrap();
    assert!((b.x0 - 100.0).abs() < 1e-6 && (b.width() - 40.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn resizing_constraint_bits() {
    use HConstraint as H;
    use VConstraint as V;
    // A cleared bit pins: right 1, width 2, left 4, bottom 8, height 16, top 32.
    let cases = [
        (63, H::Scale, V::Scale),
        (63 - 4 - 32, H::Left, V::Top),
        (63 - 1 - 8, H::Right, V::Bottom),
        (63 - 1 - 4, H::LeftRight, V::Scale),
        (63 - 8 - 32, H::Scale, V::TopBottom),
        (63 - 2 - 16, H::Center, V::Center),
        (63 - 1 - 2, H::Right, V::Scale),
        // Everything pinned is over-constrained: Sketch scales it.
        (0, H::Scale, V::Scale),
    ];
    let layers: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (mask, _, _))| with(rect(&format!("R{i}"), 0.0, 0.0, 10.0, 10.0), "resizingConstraint", json!(mask)))
        .collect();
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, layers)]);
    for (i, (mask, h, v)) in cases.iter().enumerate() {
        let c = node(&doc, &format!("R{i}")).constraints;
        assert_eq!((c.horizontal, c.vertical), (*h, *v), "mask {mask}");
    }
}

#[test]
fn hidden_locked_named() {
    let r = with(with(rect("Hidden", 0.0, 0.0, 10.0, 10.0), "isVisible", json!(false)), "isLocked", json!(true));
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![r])]);
    let n = node(&doc, "Hidden");
    assert!(!n.visible && n.locked);
}

#[test]
fn bad_numbers_are_sane() {
    let mut nan = rect("Junk", 0.0, 0.0, 10.0, 10.0);
    nan["frame"]["width"] = json!("NaN");
    nan["frame"]["x"] = json!(1e300);
    let mut neg = rect("Negative", 0.0, 0.0, 10.0, 10.0);
    neg["frame"]["height"] = json!(-5);
    neg["rotation"] = json!("x");
    let mut none = rect("Frameless", 0.0, 0.0, 10.0, 10.0);
    none.as_object_mut().unwrap().remove("frame");
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![nan, neg, none])]);
    for n in doc.nodes() {
        assert!(n.width.is_finite() && n.height.is_finite() && n.width >= 0.0 && n.height >= 0.0, "{}", n.name);
        assert!(n.transform.as_coeffs().iter().all(|c| c.is_finite()), "{}", n.name);
    }
}

#[test]
fn rectangle_corner_radii() {
    let mut each = rect("Each", 0.0, 0.0, 40.0, 40.0);
    for (i, r) in [1, 2, 3, 4].iter().enumerate() {
        each["points"][i]["cornerRadius"] = json!(r);
    }
    let fixed = with(rect("Fixed", 50.0, 0.0, 40.0, 40.0), "fixedRadius", json!(8));
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![each, fixed])]);
    assert!(matches!(node(&doc, "Each").kind, NodeKind::Rectangle));
    assert_eq!(node(&doc, "Each").corners.radii, [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(node(&doc, "Fixed").corners.radii, [8.0; 4]);
}

#[test]
fn oval_slice_and_bitmap() {
    let mut oval = rect("Round", 0.0, 0.0, 30.0, 20.0);
    oval["_class"] = json!("oval");
    let mut slice = rect("Cut", 0.0, 0.0, 30.0, 20.0);
    slice["_class"] = json!("slice");
    let mut bitmap = rect("Photo", 0.0, 0.0, 30.0, 20.0);
    bitmap["_class"] = json!("bitmap");
    bitmap["image"] = json!({"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": "images/abc123.png"});
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let (doc, _) = open_with(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![oval, slice, bitmap])], &[("images/abc123.png", &png)]);
    assert!(matches!(node(&doc, "Round").kind, NodeKind::Ellipse { .. }));
    assert!(matches!(node(&doc, "Cut").kind, NodeKind::Slice));
    let photo = node(&doc, "Photo");
    assert!(matches!(photo.kind, NodeKind::Rectangle));
    assert!(photo.fills.iter().any(|p| matches!(&p.kind, PaintKind::Image(_))));
    assert_eq!(doc.images.len(), 1);
}

#[test]
fn image_bombs_are_refused() {
    // A valid 2×2 PNG followed by 40 MB of zeros packs to a few dozen kilobytes.
    let mut png = Vec::new();
    image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png.resize(png.len() + 40 * 1024 * 1024, 0);
    let mut bitmap = rect("Photo", 0.0, 0.0, 30.0, 20.0);
    bitmap["_class"] = json!("bitmap");
    bitmap["image"] = json!({"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": "images/bomb.png"});
    let (doc, _) = open_with(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![bitmap])], &[("images/bomb.png", &png)]);
    let total: usize = doc.images.values().map(|i| i.bytes.len()).sum();
    assert!(total < 1024 * 1024, "{total} image bytes kept");
}

#[test]
fn unknown_class_is_a_warning() {
    let mut odd = rect("Odd", 0.0, 0.0, 10.0, 10.0);
    odd["_class"] = json!("MSSomethingNew");
    let (doc, warnings) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![odd])]);
    assert!(doc.nodes().all(|n| n.name != "Odd"));
    assert!(warnings.iter().any(|w| w.contains("MSSomethingNew")), "{warnings:?}");
}

#[test]
fn sketch_2025_frames_and_graphics() {
    let mut frame = artboard("F", "Frame", 0.0, 0.0, 100.0, 100.0, vec![rect("Inside", 0.0, 0.0, 10.0, 10.0)]);
    frame["_class"] = json!("frame");
    frame["clipsContent"] = json!(false);
    let mut graphic = group("Graphic", 10.0, 10.0, 10.0, 10.0, vec![rect("Leaf", 0.0, 0.0, 10.0, 10.0)]);
    graphic["_class"] = json!("graphic");
    frame["layers"].as_array_mut().unwrap().push(graphic);
    let (doc, _) = open(vec![frame]);
    assert!(matches!(node(&doc, "Frame").kind, NodeKind::Frame));
    assert!(!node(&doc, "Frame").clip);
    assert!(matches!(node(&doc, "Graphic").kind, NodeKind::Group));
}

#[test]
fn many_distinct_warnings_are_capped() {
    let layers: Vec<Value> = (0..5000)
        .map(|i| {
            let mut l = rect(&format!("R{i}"), 0.0, 0.0, 1.0, 1.0);
            l["_class"] = json!(format!("MSOdd{i}"));
            l
        })
        .collect();
    let (_, warnings) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, layers)]);
    assert!(warnings.len() <= 250, "{}", warnings.len());
}

#[test]
fn bomb_pages_are_skipped() {
    // 64 MB of `[0,0,…]` packs into a few hundred kilobytes.
    let bomb = format!("{{\"_class\":\"page\",\"do_objectID\":\"B\",\"name\":\"Bomb\",\"layers\":[],\"junk\":[{}0]}}", "0,".repeat(32 * 1024 * 1024));
    let mut out = Vec::new();
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("document.json", o).unwrap();
        z.write_all(document(&["B", "P"]).to_string().as_bytes()).unwrap();
        z.start_file("meta.json", o).unwrap();
        z.write_all(json!({"app": "com.bohemiancoding.sketch3", "appVersion": "50.2", "version": 103}).to_string().as_bytes()).unwrap();
        z.start_file("pages/B.json", o).unwrap();
        z.write_all(bomb.as_bytes()).unwrap();
        z.start_file("pages/P.json", o).unwrap();
        z.write_all(page("P", "Fine", vec![]).to_string().as_bytes()).unwrap();
        z.finish().unwrap();
    }
    let (doc, warnings) = import(&out, "f").unwrap();
    let names: Vec<String> = doc.pages.iter().map(|p| doc.node(*p).unwrap().name.clone()).collect();
    assert!(!names.contains(&"Bomb".to_string()) && names.contains(&"Fine".to_string()), "{names:?}");
    assert!(warnings.iter().any(|w| w.contains("too large")), "{warnings:?}");
}

#[test]
fn deep_document_json_still_opens_the_pages() {
    let mut deep = json!({"_class": "symbolMaster"});
    for _ in 0..150 {
        deep = json!({"_class": "group", "layers": [deep]});
    }
    let mut d = document(&["P"]);
    d["foreignSymbols"] = json!([{"symbolMaster": deep}]);
    let z = sketch_zip("50.2", 103, d, &[page("P", "Kept", vec![])], &[]);
    let (doc, warnings) = import(&z, "f").unwrap();
    assert!(doc.pages.iter().any(|p| doc.node(*p).unwrap().name == "Kept"));
    assert!(!warnings.is_empty());
}

/// Sketch flips a rotated layer on screen, after rotating it.
#[test]
fn flip_applies_after_rotation() {
    let r = with(with(rect("Turned", 0.0, 0.0, 40.0, 20.0), "rotation", json!(90)), "isFlippedHorizontal", json!(true));
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 300.0, 300.0, vec![r])]);
    let t = doc.world_transform(node(&doc, "Turned").id).as_coeffs();
    // Rotated a quarter turn counter-clockwise (x → up, y → right), then mirrored left to right.
    assert!(t[0].abs() < 1e-9 && (t[1] + 1.0).abs() < 1e-9 && (t[2] + 1.0).abs() < 1e-9 && t[3].abs() < 1e-9, "{t:?}");
}

#[test]
fn edited_shapes_keep_their_points() {
    let mut drop = rect("Drop", 0.0, 0.0, 20.0, 30.0);
    drop["_class"] = json!("oval");
    drop["edited"] = json!(true);
    drop["points"] = json!([point(0.5, 0.0), point(1.0, 0.7), point(0.5, 1.0), point(0.0, 0.7)]);
    let mut box_ = rect("Slanted", 50.0, 0.0, 20.0, 20.0);
    box_["edited"] = json!(true);
    box_["points"][1]["point"] = json!("{1, 0.3}");
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 300.0, 300.0, vec![drop, box_])]);
    let NodeKind::Vector { network } = &node(&doc, "Drop").kind else { panic!("not a vector") };
    assert_eq!((network.vertices[0].x, network.vertices[0].y), (10.0, 0.0));
    assert!(matches!(node(&doc, "Slanted").kind, NodeKind::Vector { .. }));
}

/// Sketch draws an artboard's background colour only; a style it may carry (borders, shadows) isn't
/// drawn. Sketch 2025 frames do draw theirs.
#[test]
fn artboards_ignore_their_style() {
    let border = json!([{"_class": "border", "isEnabled": true, "fillType": 0, "color": {"_class": "color", "red": 0, "green": 0, "blue": 0, "alpha": 1}, "position": 2, "thickness": 1}]);
    let mut a = artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![]);
    a["style"]["borders"] = border.clone();
    let mut f = artboard("F", "Frame", 200.0, 0.0, 100.0, 100.0, vec![]);
    f["_class"] = json!("frame");
    f["style"]["borders"] = border;
    let (doc, _) = open(vec![a, f]);
    assert!(node(&doc, "Board").strokes.is_empty());
    assert_eq!(node(&doc, "Frame").strokes.len(), 1);
}
