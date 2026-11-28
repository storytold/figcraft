//! Transform behaviour suite, written from the public documentation of design tools: resizing
//! (Shift, Alt, both, flipping past an edge), rotating (Shift in 15° steps), groups and several
//! layers with different rotations or flips, frames and constraints, lines, text, nudging, the
//! pixel grid, smart guides and the transform box itself. Each rule is a table row or an assert
//! with a one-line derivation; where the documentation is silent the row says so and pins
//! today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{Cursor, Handle, PointerEvent, PointerKind, handle_at, handle_cursor, resized_box, selection_box};
use figcraft_doc::{NodeId, TextAutoResize};
use figcraft_geom::{Point, Rect, is_flipped, rotation_degrees};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

fn set(s: &mut Session, ids: &[NodeId], props: Value) {
    ex(s, "node.set", json!({"ids": ids, "props": props}));
}

/// A session with snapping to layers off (the rows that test snapping turn it back on).
fn session() -> Session {
    let mut s = Session::new();
    ex(&mut s, "view.snapObjects", json!({"on": false}));
    s
}

#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    alt: bool,
    cmd: bool,
}

fn ev(kind: PointerKind, p: Point, m: Mods) -> PointerEvent {
    let mut e = PointerEvent::at(kind, p.x, p.y, 1.0);
    (e.shift, e.alt, e.cmd) = (m.shift, m.alt, m.cmd);
    e
}

/// A pointer drag from `a` to `b` (one move), as one gesture.
fn drag(s: &mut Session, a: Point, b: Point, m: Mods) {
    s.pointer(&ev(PointerKind::Down, a, m)).unwrap();
    s.pointer(&ev(PointerKind::Move, b, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, b, m)).unwrap();
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(id).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn near_rect(a: Rect, b: Rect) -> bool {
    near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1)
}

fn size(s: &Session, id: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(id).unwrap();
    (n.width, n.height)
}

fn rotation(s: &Session, id: NodeId) -> f64 {
    rotation_degrees(s.doc().unwrap().node(id).unwrap().transform)
}

fn flipped(s: &Session, id: NodeId) -> bool {
    is_flipped(s.doc().unwrap().world_transform(id))
}

fn undo_steps(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false };
const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false };
const BOTH: Mods = Mods { shift: true, alt: true, cmd: false };
const CMD: Mods = Mods { shift: false, alt: false, cmd: true };

/// An asymmetric synthetic image: every column is different, so recentering is visible.
fn image_layer(s: &mut Session) -> NodeId {
    use base64::Engine as _;
    let image = image::RgbaImage::from_fn(200, 100, |x, y| image::Rgba([x as u8, y as u8, 120, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    id(&ex(
        s,
        "file.import",
        json!({"data": base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()), "name": "crop.png", "x": 0, "y": 0}),
    ))
}

#[test]
fn command_resize_crops_image_pixels_and_round_trips() {
    let mut s = session();
    let layer = image_layer(&mut s);
    let original = s.doc().unwrap().clone();
    let before = figcraft_render::render_node(&original, layer, 1.0).unwrap();
    let steps = undo_steps(&s);
    // Public crop documentation: Command (macOS) / Ctrl (elsewhere) quickly crops an image.
    // Moving the right edge must keep the left half's pixels, not recenter or scale them.
    s.pointer(&ev(PointerKind::Down, Point::new(200.0, 50.0), CMD)).unwrap();
    for x in [150.0, 100.0] {
        s.pointer(&ev(PointerKind::Move, Point::new(x, 50.0), CMD)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, Point::new(100.0, 50.0), CMD)).unwrap();
    assert_eq!(size(&s, layer), (100.0, 100.0));
    let after = figcraft_render::render_node(s.doc().unwrap(), layer, 1.0).unwrap();
    for y in 0..100usize {
        assert_eq!(&after.data()[y * 400..(y + 1) * 400], &before.data()[y * 800..y * 800 + 400], "row {y}: the image stays put");
    }
    assert_eq!(s.doc().unwrap().images, original.images, "cropping keeps the source bytes");
    assert_eq!(undo_steps(&s), steps + 1);
    let cropped = s.doc().unwrap().clone();
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap(), &original);
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap(), &cropped);
    let path = std::env::temp_dir().join(format!("figcraft-command-crop-{}.figcraft", std::process::id()));
    ex(&mut s, "file.saveAs", json!({"path": path}));
    let mut reopened = session();
    ex(&mut reopened, "file.open", json!({"path": path}));
    assert_eq!(reopened.doc().unwrap().node(layer), cropped.node(layer));
    assert_eq!(reopened.doc().unwrap().images, cropped.images);
    assert_eq!(figcraft_render::render_node(reopened.doc().unwrap(), layer, 1.0).unwrap().data(), after.data());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn command_crop_keeps_image_placement_in_the_layers_axes() {
    use figcraft_doc::color::{ImageScaleMode, PaintKind};
    use figcraft_geom::Affine;
    // Each row exercises a distinct placement: Fill, Fit, and an existing affine Crop.
    // Geometry follows the existing resize rules; image pixels stay fixed in page space.
    let rows = [
        ("fill", None, 0.0, Point::new(0.0, 50.0), Point::new(40.0, 50.0), CMD, (40.0, 0.0, 160.0, 150.0)),
        ("fit", None, 30.0, Point::new(100.0, 0.0), Point::new(100.0, 20.0), CMD, (0.0, 20.0, 200.0, 130.0)),
        ("fill", None, 0.0, Point::new(0.0, 50.0), Point::new(240.0, 50.0), CMD, (240.0, 0.0, 40.0, 150.0)),
        (
            "crop",
            Some([0.8, 0.1, -0.2, 0.9, 0.1, 0.05]),
            30.0,
            Point::new(200.0, 150.0),
            Point::new(150.0, 112.5),
            Mods { alt: true, ..CMD },
            (50.0, 37.5, 100.0, 75.0),
        ),
    ];
    for (mode, crop, angle, from, to, mods, (x, y, w, h)) in rows {
        let mut s = session();
        let layer = image_layer(&mut s);
        let mut fills = s.doc().unwrap().node(layer).unwrap().fills.clone();
        let PaintKind::Image(img) = &mut fills[0].kind else { panic!("image fill") };
        img.scale_mode = match mode {
            "fit" => ImageScaleMode::Fit,
            "crop" => ImageScaleMode::Crop,
            _ => ImageScaleMode::Fill,
        };
        img.crop = crop;
        set(&mut s, &[layer], json!({"height": 150, "rotation": angle, "fills": fills}));
        let doc = s.doc().unwrap();
        let world = doc.world_transform(layer);
        let n = doc.node(layer).unwrap();
        let old_crop = crop.unwrap_or(if mode == "fit" { [1.0, 0.0, 0.0, 1.5, 0.0, -0.25] } else { [2.0 / 3.0, 0.0, 0.0, 1.0, 1.0 / 6.0, 0.0] });
        let old_mapping = Affine::new(old_crop) * Affine::scale_non_uniform(1.0 / n.width, 1.0 / n.height) * world.inverse();
        drag(&mut s, world * from, world * to, mods);
        let doc = s.doc().unwrap();
        let n = doc.node(layer).unwrap();
        assert!(near(n.width, w) && near(n.height, h), "{mode}: crop box size");
        assert!((doc.world_transform(layer) * Point::ZERO - world * Point::new(x, y)).hypot() < 1e-6, "{mode}: crop box anchor");
        let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
        assert_eq!(img.scale_mode, ImageScaleMode::Crop, "{mode}");
        let mapping =
            Affine::new(img.crop.unwrap()) * Affine::scale_non_uniform(1.0 / n.width, 1.0 / n.height) * doc.world_transform(layer).inverse();
        for p in [Point::ZERO, Point::new(80.0, 60.0), Point::new(170.0, 90.0)] {
            assert!((mapping * p - old_mapping * p).hypot() < 1e-6, "{mode}: same page point samples the same source pixel");
        }
    }
}

#[test]
fn ordinary_resize_and_scale_keep_image_fill_settings() {
    for (tool, cmd, ctrl) in [("move", false, false), ("move", false, true), ("scale", true, false)] {
        let mut s = session();
        let layer = image_layer(&mut s);
        let fills = s.doc().unwrap().node(layer).unwrap().fills.clone();
        ex(&mut s, "tool.set", json!({"tool": tool}));
        let to = Point::new(100.0, 50.0);
        for (kind, p) in [(PointerKind::Down, Point::new(200.0, 100.0)), (PointerKind::Move, to), (PointerKind::Up, to)] {
            let mut event = ev(kind, p, Mods { cmd, ..NONE });
            event.ctrl = ctrl;
            s.pointer(&event).unwrap();
        }
        assert_eq!(s.doc().unwrap().node(layer).unwrap().fills, fills, "{tool}, cmd={cmd}, ctrl={ctrl}");
        assert_eq!(size(&s, layer), (100.0, 50.0));
    }
}

#[test]
fn crop_mode_image_resizing_without_cmd_morphs_and_with_cmd_crops() {
    use figcraft_doc::color::{ImageScaleMode, PaintKind};
    let mut s = session();
    let layer = image_layer(&mut s);
    // Set the image fill mode to Crop without an explicit crop: fill.set initializes placement crop.
    let mut fills = s.doc().unwrap().node(layer).unwrap().fills.clone();
    let PaintKind::Image(img) = &mut fills[0].kind else { panic!("image fill") };
    img.scale_mode = ImageScaleMode::Crop;
    img.crop = None;
    ex(&mut s, "fill.set", json!({"ids": [layer], "index": 0, "paint": fills[0]}));

    // Verify fill.set initialized img.crop with placement crop.
    let initial_node = s.doc().unwrap().node(layer).unwrap().clone();
    let PaintKind::Image(img) = &initial_node.fills[0].kind else { panic!("image fill") };
    let Some(initial_crop) = img.crop else { panic!("crop initialized") };

    let before = figcraft_render::render_node(s.doc().unwrap(), layer, 1.0).unwrap();
    assert_eq!((before.width(), before.height()), (200, 100));

    // 1. Resize without Cmd (normal resize from bottom edge: 100 -> 50).
    // The layer height shrinks to 50, but img.crop stays unchanged, stretching/squishing the image.
    s.pointer(&ev(PointerKind::Down, Point::new(100.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(100.0, 50.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Up, Point::new(100.0, 50.0), NONE)).unwrap();
    assert_eq!(size(&s, layer), (200.0, 50.0));

    let n = s.doc().unwrap().node(layer).unwrap().clone();
    let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
    assert_eq!(img.crop, Some(initial_crop), "normal resize leaves crop transform unchanged");

    let morphed = figcraft_render::render_node(s.doc().unwrap(), layer, 1.0).unwrap();
    assert_eq!((morphed.width(), morphed.height()), (200, 50));

    // 2. Resize with Cmd from right edge: 200 -> 100.
    // The layer width shrinks to 100, and img.crop is updated so the pixels stay stationary on page (cropping).
    s.pointer(&ev(PointerKind::Down, Point::new(200.0, 25.0), CMD)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(100.0, 25.0), CMD)).unwrap();
    s.pointer(&ev(PointerKind::Up, Point::new(100.0, 25.0), CMD)).unwrap();
    assert_eq!(size(&s, layer), (100.0, 50.0));

    let n = s.doc().unwrap().node(layer).unwrap().clone();
    let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
    assert_ne!(img.crop, Some(initial_crop), "Cmd resize updates crop transform");

    let cropped = figcraft_render::render_node(s.doc().unwrap(), layer, 1.0).unwrap();
    assert_eq!((cropped.width(), cropped.height()), (100, 50));
    // The remaining 100x50 pixels match the top-left 100x50 of morphed (pixels stayed still in layer space).
    for y in 0..50usize {
        assert_eq!(&cropped.data()[y * 400..(y + 1) * 400], &morphed.data()[y * 800..y * 800 + 400], "row {y}: stationary cropped pixels");
    }
}

#[test]
fn multi_layer_crop_mode_selection_computes_placement_per_node() {
    use figcraft_doc::color::{ImageScaleMode, PaintKind};
    let mut s = session();
    let layer1 = image_layer(&mut s); // 200 x 100
    let layer2 = id(&ex(&mut s, "shape.rectangle", json!({"x": 300, "y": 0, "width": 100, "height": 100})));

    // Choose Crop mode on both layers simultaneously via fill.set with crop: None (as the picker does).
    let mut fill = s.doc().unwrap().node(layer1).unwrap().fills[0].clone();
    let PaintKind::Image(img) = &mut fill.kind else { panic!("image fill") };
    img.scale_mode = ImageScaleMode::Crop;
    img.crop = None;
    ex(&mut s, "fill.set", json!({"ids": [layer1, layer2], "index": 0, "paint": fill}));

    // Verify both layers received crop transforms tailored to their own dimensions, not stretched.
    let n1 = s.doc().unwrap().node(layer1).unwrap().clone();
    let n2 = s.doc().unwrap().node(layer2).unwrap().clone();
    let PaintKind::Image(img1) = &n1.fills[0].kind else { panic!("image fill") };
    let PaintKind::Image(img2) = &n2.fills[0].kind else { panic!("image fill") };

    let crop1 = img1.crop.expect("layer1 crop initialized");
    let crop2 = img2.crop.expect("layer2 crop initialized");

    // layer1 (200x100) and layer2 (100x100) have different aspect ratios, so their crop transforms must differ.
    assert_ne!(crop1, crop2, "differently sized layers must have distinct crop transforms");

    // Both should match expected placement crop for their respective dimensions (iw=200, ih=100 from image_layer):
    assert_eq!(crop1, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(crop2, [0.5, 0.0, 0.0, 1.0, 0.25, 0.0]);

    // Rotate 90° on the multi-selection (as the picker's Rotate button does):
    let mut rot_fill = n1.fills[0].clone();
    let PaintKind::Image(img) = &mut rot_fill.kind else { panic!("image fill") };
    img.rotation = 1;
    ex(&mut s, "fill.set", json!({"ids": [layer1, layer2], "index": 0, "paint": rot_fill}));

    let n1_rot = s.doc().unwrap().node(layer1).unwrap().clone();
    let n2_rot = s.doc().unwrap().node(layer2).unwrap().clone();
    let PaintKind::Image(img1_rot) = &n1_rot.fills[0].kind else { panic!("image fill") };
    let PaintKind::Image(img2_rot) = &n2_rot.fills[0].kind else { panic!("image fill") };

    let crop1_rot = img1_rot.crop.expect("layer1 rotated crop initialized");
    let crop2_rot = img2_rot.crop.expect("layer2 rotated crop initialized");
    assert_ne!(crop1_rot, crop2_rot, "differently sized layers must have distinct rotated crop transforms");
    assert_eq!(crop1_rot, [1.0, 0.0, 0.0, 0.25, 0.0, 0.375]);
    assert_eq!(crop2_rot, [1.0, 0.0, 0.0, 0.5, 0.0, 0.25]);

    // Shared filter edit on multi-selection in Crop mode (e.g. Exposure slider):
    let mut filter_fill = n1_rot.fills[0].clone();
    let PaintKind::Image(img) = &mut filter_fill.kind else { panic!("image fill") };
    img.filters.exposure = 0.5;
    ex(&mut s, "fill.set", json!({"ids": [layer1, layer2], "index": 0, "paint": filter_fill}));

    let n1_filt = s.doc().unwrap().node(layer1).unwrap().clone();
    let n2_filt = s.doc().unwrap().node(layer2).unwrap().clone();
    let PaintKind::Image(img1_filt) = &n1_filt.fills[0].kind else { panic!("image fill") };
    let PaintKind::Image(img2_filt) = &n2_filt.fills[0].kind else { panic!("image fill") };

    // Both layers keep their distinct crop transforms; neither is overwritten by the other.
    assert_eq!(img1_filt.crop, Some(crop1_rot));
    assert_eq!(img2_filt.crop, Some(crop2_rot));
    assert_eq!(img1_filt.filters.exposure, 0.5);
    assert_eq!(img2_filt.filters.exposure, 0.5);
}

#[test]
fn command_crop_preserves_each_image_fill_and_leaves_tiles() {
    use figcraft_doc::color::{Color, ImageScaleMode, Paint, PaintKind};
    let mut s = session();
    let layer = image_layer(&mut s);
    let image = s.doc().unwrap().node(layer).unwrap().fills[0].clone();
    let mut rotated = image.clone();
    rotated.opacity = 0.4;
    let PaintKind::Image(img) = &mut rotated.kind else { panic!("image fill") };
    img.rotation = 1;
    let mut tile = image.clone();
    let PaintKind::Image(img) = &mut tile.kind else { panic!("image fill") };
    img.scale_mode = ImageScaleMode::Tile;
    let mut expected = vec![Paint::solid(Color::from_rgba8(20, 40, 60, 255)), rotated, image, tile];
    set(&mut s, &[layer], json!({"fills": expected}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(100.0, 50.0), CMD);
    for (i, crop) in [(1, [0.5, 0.0, 0.0, 0.25, 0.0, 0.375]), (2, [0.5, 0.0, 0.0, 1.0, 0.0, 0.0])] {
        let PaintKind::Image(img) = &mut expected[i].kind else { panic!("image fill") };
        img.scale_mode = ImageScaleMode::Crop;
        img.crop = Some(crop);
    }
    assert_eq!(s.doc().unwrap().node(layer).unwrap().fills, expected);
}

#[test]
fn command_resize_leaves_a_texts_fills() {
    let mut s = session();
    let image = image_layer(&mut s);
    let fills = s.doc().unwrap().node(image).unwrap().fills.clone();
    let layer = id(&ex(
        &mut s,
        "text.create",
        json!({"text": "hello world hello world hello world", "props": {
            "width": 200, "height": 100, "fontSize": 24, "textAutoResize": "none", "fills": fills
        }}),
    ));
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    let doc = s.doc().unwrap();
    let world = doc.world_transform(layer);
    let n = doc.node(layer).unwrap();
    let (from, to) = (world * Point::new(n.width, n.height / 2.0), world * Point::new(10.0, n.height / 2.0));
    drag(&mut s, from, to, CMD);
    assert_eq!(s.doc().unwrap().node(layer).unwrap().fills, fills, "a text reflows: keep the existing resize behaviour");
}

fn image_units_per_layer_unit(s: &Session, layer: NodeId) -> (f64, f64, f64, f64) {
    use figcraft_doc::color::PaintKind;
    let doc = s.doc().unwrap();
    let n = doc.node(layer).unwrap();
    let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
    let data = doc.images.get(&img.image).unwrap();
    let c = img.effective_crop(f64::from(data.width), f64::from(data.height), n.width, n.height);
    (c[0] / n.width, c[3] / n.height, c[4], c[5])
}

#[test]
fn command_resize_crops_in_auto_layout_against_the_box_layout_leaves() {
    use figcraft_doc::color::{ImageScaleMode, PaintKind};
    for case in ["child", "child with a minimum width", "auto layout frame"] {
        let mut s = session();
        let image = image_layer(&mut s);
        let fills = s.doc().unwrap().node(image).unwrap().fills.clone();
        let frame = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 240, "height": 140})));
        set(&mut s, &[frame], json!({"layout": "horizontal", "padding": 20}));
        let (layer, to_x, width) = match case {
            "auto layout frame" => {
                set(&mut s, &[frame], json!({"fills": fills}));
                (frame, 10.0, 40.0)
            }
            _ => {
                ex(&mut s, "node.reparent", json!({"ids": [image], "parent": frame}));
                if case == "child" {
                    (image, 150.0, 150.0)
                } else {
                    set(&mut s, &[image], json!({"minWidth": 150}));
                    (image, 10.0, 150.0)
                }
            }
        };
        ex(&mut s, "select.set", json!({"ids": [layer]}));
        let before = image_units_per_layer_unit(&s, layer);
        let doc = s.doc().unwrap();
        let world = doc.world_transform(layer);
        let n = doc.node(layer).unwrap();
        let (from, to) = (world * Point::new(n.width, n.height / 2.0), world * Point::new(to_x, n.height / 2.0));
        drag(&mut s, from, to, CMD);
        let n = s.doc().unwrap().node(layer).unwrap().clone();
        assert!(near(n.width, width), "{case}: {} wide, as auto layout leaves it", n.width);
        let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
        assert_eq!(img.scale_mode, ImageScaleMode::Crop, "{case}: cropped");
        let after = image_units_per_layer_unit(&s, layer);
        assert!(near(after.0, before.0) && near(after.1, before.1), "{case}: the image keeps its scale, not stretched: {before:?} -> {after:?}");
        assert!(near(after.2, before.2) && near(after.3, before.3), "{case}: its left edge stays where it was in the layer: {before:?} -> {after:?}");
    }
}

/// A minimum width can give a layer back the width its left side was dragged in by: the crop then
/// slides back along the image instead of running past its right edge.
#[test]
fn command_crop_slides_back_onto_the_image_when_layout_widens_the_layer() {
    use figcraft_doc::color::{ImagePaint, PaintKind};
    let mut s = session();
    let image = image_layer(&mut s);
    let frame = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 240, "height": 140})));
    set(&mut s, &[frame], json!({"layout": "horizontal", "padding": 20}));
    ex(&mut s, "node.reparent", json!({"ids": [image], "parent": frame}));
    set(&mut s, &[image], json!({"minWidth": 150}));
    ex(&mut s, "select.set", json!({"ids": [image]}));
    let before = image_units_per_layer_unit(&s, image);
    let world = s.doc().unwrap().world_transform(image);
    drag(&mut s, world * Point::new(0.0, 50.0), world * Point::new(190.0, 50.0), CMD);
    let doc = s.doc().unwrap();
    let n = doc.node(image).unwrap();
    assert!(near(n.width, 150.0), "{} wide, as the minimum width leaves it", n.width);
    let PaintKind::Image(img) = &n.fills[0].kind else { panic!("image fill") };
    let data = doc.images.get(&img.image).unwrap();
    let crop = img.effective_crop(f64::from(data.width), f64::from(data.height), n.width, n.height);
    let (x0, _, x1, _) = ImagePaint::crop_rect(crop);
    assert!(x0 <= 1e-9 && x1 >= 1.0 - 1e-9, "the image covers the layer: {x0}..{x1}");
    let after = image_units_per_layer_unit(&s, image);
    assert!(near(after.0, before.0) && near(after.1, before.1), "the image keeps its scale: {before:?} -> {after:?}");
}

/// Where the image's edge stops a crop, Shift still keeps the ratio (both axes stop together) and
/// Alt still keeps the box centred (both sides stop together).
#[test]
fn command_crop_stopped_by_the_image_keeps_the_ratio_with_shift_and_the_centre_with_alt() {
    let mut s = session();
    let layer = image_layer(&mut s);
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    drag(&mut s, Point::new(200.0, 100.0), Point::new(150.0, 80.0), CMD);
    assert_eq!(size(&s, layer), (150.0, 80.0));
    drag(&mut s, Point::new(150.0, 80.0), Point::new(240.0, 128.0), Mods { shift: true, alt: false, cmd: true });
    assert!(
        near_rect(bounds(&s, layer), Rect::new(0.0, 0.0, 187.5, 100.0)),
        "the bottom stops at the image, the right side with it: {:?}",
        bounds(&s, layer)
    );

    let mut s = session();
    let layer = image_layer(&mut s);
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    drag(&mut s, Point::new(0.0, 50.0), Point::new(20.0, 50.0), CMD);
    drag(&mut s, Point::new(200.0, 50.0), Point::new(150.0, 50.0), CMD);
    assert!(near_rect(bounds(&s, layer), Rect::new(20.0, 0.0, 150.0, 100.0)), "{:?}", bounds(&s, layer));
    drag(&mut s, Point::new(150.0, 50.0), Point::new(190.0, 50.0), Mods { shift: false, alt: true, cmd: true });
    assert!(
        near_rect(bounds(&s, layer), Rect::new(0.0, 0.0, 170.0, 100.0)),
        "the left side stops at the image, the right side as far: {:?}",
        bounds(&s, layer)
    );
}

/// A resize is an explicit edit of the size: a size bound to a variable lets go of it (#809), so
/// Cmd/Ctrl crops a bound layer's image as any other's; other bindings stay.
#[test]
fn image_crop_detaches_a_bound_size_and_crops() {
    for (field, value, from, to, (w, h)) in [
        ("width", 200, Point::new(200.0, 50.0), Point::new(100.0, 50.0), (100usize, 100usize)),
        ("height", 100, Point::new(100.0, 100.0), Point::new(100.0, 50.0), (200, 50)),
        ("w", 200, Point::new(200.0, 50.0), Point::new(100.0, 50.0), (100, 100)),
        ("h", 100, Point::new(100.0, 100.0), Point::new(100.0, 50.0), (200, 50)),
        ("opacity", 100, Point::new(200.0, 50.0), Point::new(100.0, 50.0), (100, 100)),
    ] {
        for mods in [CMD, NONE] {
            let mut s = session();
            let layer = image_layer(&mut s);
            ex(&mut s, "variable.createCollection", json!({"name": "Tokens"}));
            ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "bound", "type": "number", "value": value}));
            ex(&mut s, "variable.bind", json!({"ids": [layer], "field": field, "variable": "bound"}));
            let before = s.doc().unwrap().clone();
            let pixels = figcraft_render::render_node(&before, layer, 1.0).unwrap();
            drag(&mut s, from, to, mods);
            let doc = s.doc().unwrap();
            let n = doc.node(layer).unwrap();
            assert_eq!(n.bound.is_empty(), field != "opacity", "{field}: a size binding goes, others stay");
            assert_eq!(doc.variables, before.variables, "{field}: do not edit the variable");
            assert_eq!((n.width, n.height), (w as f64, h as f64), "{field}");
            if mods.cmd {
                let after = figcraft_render::render_node(doc, layer, 1.0).unwrap();
                for y in 0..h {
                    assert_eq!(&after.data()[y * w * 4..(y + 1) * w * 4], &pixels.data()[y * 800..y * 800 + w * 4], "{field}: the image stays put");
                }
            } else {
                assert_eq!(n.fills, before.node(layer).unwrap().fills, "{field}: keep the existing resize behaviour");
            }
        }
    }
}

#[test]
fn handle_drags_resize_by_the_documented_rules() {
    // A 100×50 box; (handle, pointer in the box's axes, modifiers, new box x0 y0 x1 y1, derivation).
    let rows: [(Handle, (f64, f64), Mods, (f64, f64, f64, f64), &str); 11] = [
        (Handle::Corner(2), (150.0, 80.0), NONE, (0.0, 0.0, 150.0, 80.0), "a corner moves the two edges it joins"),
        (Handle::Edge(1), (150.0, 999.0), NONE, (0.0, 0.0, 150.0, 50.0), "an edge moves only its side"),
        (Handle::Corner(2), (150.0, 60.0), SHIFT, (0.0, 0.0, 150.0, 75.0), "Shift keeps the proportions (following the axis dragged the most)"),
        (Handle::Edge(1), (150.0, 0.0), SHIFT, (0.0, -12.5, 150.0, 62.5), "Shift on an edge scales the other axis about its middle"),
        (Handle::Corner(2), (150.0, 80.0), ALT, (-50.0, -30.0, 150.0, 80.0), "Alt resizes from the centre"),
        (Handle::Corner(2), (75.0, 40.0), ALT, (25.0, 10.0, 75.0, 40.0), "Alt shrinks about the centre too"),
        (Handle::Edge(3), (10.0, 0.0), ALT, (10.0, 0.0, 90.0, 50.0), "Alt on an edge moves the opposite edge the other way"),
        (Handle::Corner(2), (150.0, 60.0), BOTH, (-50.0, -25.0, 150.0, 75.0), "Shift+Alt: proportional, from the centre"),
        (Handle::Edge(1), (-40.0, 0.0), NONE, (0.0, 0.0, -40.0, 50.0), "dragging an edge past the opposite one flips (reversed box)"),
        (Handle::Corner(0), (110.0, 60.0), NONE, (110.0, 60.0, 100.0, 50.0), "a corner dragged past the opposite one flips both ways"),
        (Handle::Corner(2), (-50.0, -25.0), SHIFT, (0.0, 0.0, -50.0, -25.0), "a proportional flip keeps the proportions"),
    ];
    for (h, q, m, want, why) in rows {
        let got = resized_box(h, Point::new(q.0, q.1), 100.0, 50.0, m.shift, m.alt);
        assert!(near(got.0, want.0) && near(got.1, want.1) && near(got.2, want.2) && near(got.3, want.3), "{why}: {got:?}");
    }
}

#[test]
fn resizing_and_flipping_one_layer() {
    let mut s = session();
    // A rotated layer resizes along its own axes, keeping its rotation and the opposite corner.
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"rotation": 30}));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let world = s.doc().unwrap().world_transform(r);
    let steps = undo_steps(&s);
    drag(&mut s, world * Point::new(100.0, 50.0), world * Point::new(150.0, 80.0), NONE);
    assert!(near(size(&s, r).0, 150.0) && near(size(&s, r).1, 80.0) && near(rotation(&s, r), 30.0), "rotated layers resize in their own axes");
    assert!((s.doc().unwrap().world_transform(r) * Point::ZERO - world * Point::ZERO).hypot() < 1e-6, "the opposite corner stays put");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step per gesture");
    // Dragging the right edge past the left one flips the layer.
    let f = rect(&mut s, 300.0, 0.0, 100.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(400.0, 25.0), Point::new(260.0, 25.0), NONE);
    assert!(flipped(&s, f) && near_rect(bounds(&s, f), Rect::new(260.0, 0.0, 300.0, 50.0)), "flipped, 40 wide, left of where it was");
    // Rotation handles: just outside a corner; Shift turns in 15° steps.
    let t = rect(&mut s, 0.0, 200.0, 100.0, 100.0, None);
    ex(&mut s, "select.set", json!({"ids": [t]}));
    let (c, from) = (Point::new(50.0, 250.0), Point::new(110.0, 190.0));
    let a = (from - c).atan2() + 37f64.to_radians();
    let to = c + figcraft_geom::Vec2::from_angle(a) * 80.0;
    drag(&mut s, from, to, NONE);
    assert!(near(rotation(&s, t).abs(), 37.0), "a free turn: {}", rotation(&s, t));
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, from, to, SHIFT);
    assert!(near(rotation(&s, t).abs(), 30.0), "Shift: the nearest 15° step: {}", rotation(&s, t));
}

/// A rotate-handle drag on a layer at `start` degrees, turning it by `turn` degrees in the Design
/// panel's convention (positive is counter-clockwise on screen); returns the rotation it ends at.
fn rotate_drag_from(start: f64, turn: f64, m: Mods) -> f64 {
    let mut s = session();
    let r = rect(&mut s, 0.0, 200.0, 100.0, 100.0, None);
    set(&mut s, &[r], json!({"rotation": start}));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    // The handle sits just outside the top-left corner of the turned box; the pointer then circles
    // the centre by `turn` (the page angle runs the other way round, y being down).
    let world = s.doc().unwrap().world_transform(r);
    let (c, corner) = (world * Point::new(50.0, 50.0), world * Point::ZERO);
    let from = corner + (corner - c) * 0.15;
    let a = (from - c).atan2() - turn.to_radians();
    let to = c + figcraft_geom::Vec2::from_angle(a) * 80.0;
    drag(&mut s, from, to, m);
    rotation(&s, r)
}

#[test]
fn shift_rotating_lands_on_multiples_of_15_not_steps_from_the_start() {
    // Measured in Figma: a layer at 7° (or -4.27°) Shift-rotated by its corner ends at 15°, so the
    // steps are absolute multiples of 15°, not 15° steps from where the drag began.
    let got = rotate_drag_from(7.0, 37.0, SHIFT);
    assert!(near(got, 45.0), "7° turned by 37° is 44°, nearest multiple of 15 is 45°: {got}");
    // Without Shift the turn is free.
    let got = rotate_drag_from(7.0, 37.0, NONE);
    assert!(near(got, 44.0), "a free turn: {got}");
    // A small turn: 7° + 3° = 10°, nearest multiple of 15 is 15° (not 7° or 10°).
    let got = rotate_drag_from(7.0, 3.0, SHIFT);
    assert!(near(got, 15.0), "10° rounds to 15°: {got}");
    let got = rotate_drag_from(-4.27, 2.0, SHIFT);
    assert!(near(got, 0.0), "-2.27° rounds to 0°: {got}");
}

/// Rotates a 120 × 80 layer inside a frame turned `frame_rot`, the layer at `start` on the page,
/// by `turn` from its corner, and returns its rotation on the page.
fn rotate_drag_in_frame(frame_rot: f64, start: f64, turn: f64, m: Mods) -> f64 {
    let mut s = session();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 100, "width": 400, "height": 400})));
    set(&mut s, &[f], json!({"rotation": frame_rot}));
    let r = rect(&mut s, 100.0, 100.0, 120.0, 80.0, Some(f));
    set(&mut s, &[r], json!({"rotation": start - frame_rot}));
    let page = |s: &Session| rotation_degrees(s.doc().unwrap().world_transform(r));
    assert!(near(page(&s), start), "the layer starts at {start}° on the page: {}", page(&s));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let world = s.doc().unwrap().world_transform(r);
    let (c, corner) = (world * Point::new(60.0, 40.0), world * Point::ZERO);
    let from = corner + (corner - c) * 0.15;
    let a = (from - c).atan2() - turn.to_radians();
    let to = c + figcraft_geom::Vec2::from_angle(a) * 80.0;
    drag(&mut s, from, to, m);
    page(&s)
}

#[test]
fn shift_rotating_inside_a_rotated_frame_lands_the_page_rotation_on_multiples_of_15() {
    // Measured in Figma (2026-11-10): the rotation field of a layer inside a frame turned 10° shows
    // its rotation on the page, and Shift lands that on multiples of 15°: from 7°, turns of 40°,
    // 6° and -10° end at 45°, 15° and 0°; without Shift 6° ends at 13°. Groups behave the same.
    for (turn, want) in [(40.0, 45.0), (6.0, 15.0), (-10.0, 0.0)] {
        let got = rotate_drag_in_frame(10.0, 7.0, turn, SHIFT);
        assert!(near(got, want), "7° on the page turned by {turn}° with Shift: {got}, want {want}");
    }
    let got = rotate_drag_in_frame(10.0, 7.0, 6.0, NONE);
    assert!(near(got, 13.0), "a free turn: {got}");
}

#[test]
fn groups_scale_children_keeping_their_rotation_and_flip() {
    let mut s = session();
    // A group of a 100×20 bar turned 90° (40..60 × -40..60) and a 50×50 square at (100, 0), its
    // box 40..150 × -40..60. Stretching it to twice its width (right edge to 260):
    let bar = rect(&mut s, 0.0, 0.0, 100.0, 20.0, None);
    set(&mut s, &[bar], json!({"rotation": 90}));
    let sq = rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    ex(&mut s, "object.flipH", json!({"ids": [sq]}));
    let g = id(&ex(&mut s, "object.group", json!({"ids": [bar, sq]})));
    assert!(near_rect(bounds(&s, g), Rect::new(40.0, -40.0, 150.0, 60.0)));
    ex(&mut s, "select.set", json!({"ids": [g]}));
    drag(&mut s, Point::new(150.0, 10.0), Point::new(260.0, 10.0), NONE);
    assert!(near_rect(bounds(&s, g), Rect::new(40.0, -40.0, 260.0, 60.0)), "the group is twice as wide, as tall as before: {:?}", bounds(&s, g));
    // Each child scales with the group across the page: the bar's own height (now horizontal)
    // doubles and its length stays; both keep their rotation and flip.
    assert!(near(size(&s, bar).0, 100.0) && near(size(&s, bar).1, 40.0) && near(rotation(&s, bar), 90.0), "{:?}", size(&s, bar));
    assert!(near_rect(bounds(&s, bar), Rect::new(40.0, -40.0, 80.0, 60.0)), "{:?}", bounds(&s, bar));
    assert!(flipped(&s, sq) && near_rect(bounds(&s, sq), Rect::new(160.0, 0.0, 260.0, 50.0)), "{:?}", bounds(&s, sq));
}

#[test]
fn nested_turned_groups_and_constraints_scale_along_each_layers_axes() {
    let mut s = session();
    // A group turned 90° holding a 100×20 bar, inside an outer group with a square: stretching the
    // outer group twice as wide makes the bar's page width (its own height) twice as large.
    let bar = rect(&mut s, 0.0, 0.0, 100.0, 20.0, None);
    let inner = id(&ex(&mut s, "object.group", json!({"ids": [bar]})));
    set(&mut s, &[inner], json!({"rotation": 90}));
    let sq = rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    let outer = id(&ex(&mut s, "object.group", json!({"ids": [inner, sq]})));
    let (b0, o0) = (bounds(&s, bar), bounds(&s, outer));
    set(&mut s, &[outer], json!({"width": o0.width() * 2.0}));
    let b1 = bounds(&s, bar);
    assert!(near(b1.width(), b0.width() * 2.0) && near(b1.height(), b0.height()), "{b0:?} → {b1:?}");
    assert!(near(size(&s, bar).0, 100.0) && near(size(&s, bar).1, 40.0));
    // A frame's child turned 90° with Scale constraints: the frame twice as wide makes the child's
    // page width (its own height) twice as large, not its length.
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 200, "height": 200})));
    let c = rect(&mut s, 50.0, 50.0, 100.0, 20.0, Some(f));
    set(&mut s, &[c], json!({"rotation": 90, "constraints": {"horizontal": "scale", "vertical": "scale"}}));
    let before = bounds(&s, c);
    set(&mut s, &[f], json!({"width": 400}));
    let after = bounds(&s, c);
    assert!(near(after.width(), before.width() * 2.0) && near(after.height(), before.height()), "{before:?} → {after:?}");
    assert!(near(after.x0, before.x0 * 2.0) && near(rotation(&s, c), 90.0));
}

#[test]
fn several_layers_with_different_rotations_resize_together() {
    let mut s = session();
    // P upright (0..100 × 0..50); Q 100×50 turned 90° about its centre (225..275 × -25..75): a
    // page-aligned box 0..275 × -25..75.
    let p = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    let q = rect(&mut s, 200.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[q], json!({"rotation": 90}));
    ex(&mut s, "select.set", json!({"ids": [p, q]}));
    let b = selection_box(s.doc().unwrap(), &[p, q]).unwrap();
    assert_eq!(b[0], Point::new(0.0, -25.0));
    assert_eq!(b[2], Point::new(275.0, 75.0));
    // Twice as wide from the right edge: everything scales across the page, each layer along its
    // own axes, keeping its rotation.
    let steps = undo_steps(&s);
    drag(&mut s, Point::new(275.0, 25.0), Point::new(550.0, 25.0), NONE);
    assert!(near_rect(bounds(&s, p), Rect::new(0.0, 0.0, 200.0, 50.0)), "{:?}", bounds(&s, p));
    assert!(near_rect(bounds(&s, q), Rect::new(450.0, -25.0, 550.0, 75.0)), "{:?}", bounds(&s, q));
    assert!(near(size(&s, q).0, 100.0) && near(size(&s, q).1, 100.0) && near(rotation(&s, q), 90.0), "{:?}", size(&s, q));
    assert_eq!(undo_steps(&s), steps + 1);
    ex(&mut s, "edit.undo", json!({}));
    // Dragged past the opposite edge, the arrangement mirrors and each layer flips.
    drag(&mut s, Point::new(275.0, 25.0), Point::new(-275.0, 25.0), NONE);
    assert!(near_rect(bounds(&s, p), Rect::new(-100.0, 0.0, 0.0, 50.0)), "{:?}", bounds(&s, p));
    assert!(near_rect(bounds(&s, q), Rect::new(-275.0, -25.0, -225.0, 75.0)), "{:?}", bounds(&s, q));
    assert!(flipped(&s, p) && flipped(&s, q));
}

#[test]
fn design_panel_edits_reach_every_selected_layer() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    let b = rect(&mut s, 200.0, 0.0, 60.0, 30.0, None);
    set(&mut s, &[b], json!({"rotation": 20}));
    // Typing W, H, X or rotation into a mixed field sets it on each layer (one undo step).
    let steps = undo_steps(&s);
    set(&mut s, &[a, b], json!({"width": 80, "rotation": 45, "x": 10}));
    assert_eq!(undo_steps(&s), steps + 1);
    for i in [a, b] {
        assert!(near(size(&s, i).0, 80.0) && near(rotation(&s, i), 45.0), "{i:?}");
        assert!(near(s.doc().unwrap().node(i).unwrap().transform.as_coeffs()[4], 10.0));
    }
    // A flipped layer reads back the rotation typed and stays flipped (a horizontal flip reads
    // as 180°, the form the rotation field shows).
    ex(&mut s, "object.flipH", json!({"ids": [a]}));
    set(&mut s, &[a], json!({"rotation": 30}));
    assert!(near(rotation(&s, a), 30.0) && flipped(&s, a), "{}", rotation(&s, a));
}

#[test]
fn absolute_rotation_sets_the_rotation_on_the_page_keeping_the_centre() {
    let mut s = session();
    let page = |s: &Session, i: NodeId| rotation_degrees(s.doc().unwrap().world_transform(i));
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 100, "width": 400, "height": 400})));
    set(&mut s, &[f], json!({"rotation": 10}));
    let r = rect(&mut s, 100.0, 100.0, 120.0, 80.0, Some(f));
    assert!(near(page(&s, r), 10.0) && near(rotation(&s, r), 0.0));
    let centre = s.doc().unwrap().world_transform(r) * Point::new(60.0, 40.0);
    set(&mut s, &[r], json!({"absoluteRotation": 7}));
    assert!(near(page(&s, r), 7.0), "{}", page(&s, r));
    assert!(near(rotation(&s, r), -3.0), "{}", rotation(&s, r));
    let c = s.doc().unwrap().world_transform(r) * Point::new(60.0, 40.0);
    assert!((c - centre).hypot() < 1e-6, "the centre stays put");
    set(&mut s, &[r], json!({"absoluteRotation": 0}));
    assert!(near(page(&s, r), 0.0) && near(rotation(&s, r), -10.0));
    // `node.get` still reports the local angle.
    let got = ex(&mut s, "node.get", json!({"id": r}));
    let rot = got["rotation"].as_f64().unwrap_or(f64::NAN);
    assert!(near(rot, -10.0), "{got}");
    // Groups: a child reads and sets the page angle, its sibling keeps its own.
    let a = rect(&mut s, 0.0, 600.0, 50.0, 50.0, None);
    let b = rect(&mut s, 100.0, 600.0, 50.0, 50.0, None);
    let g = id(&ex(&mut s, "object.group", json!({"ids": [a, b]})));
    set(&mut s, &[g], json!({"rotation": 10}));
    assert!(near(page(&s, a), 10.0));
    set(&mut s, &[a], json!({"absoluteRotation": 7}));
    assert!(near(page(&s, a), 7.0) && near(page(&s, b), 10.0));
    // Several layers: one value on the page for all, one undo step.
    let top = rect(&mut s, 800.0, 0.0, 50.0, 50.0, None);
    let steps = undo_steps(&s);
    set(&mut s, &[r, top], json!({"absoluteRotation": 45}));
    assert!(near(page(&s, r), 45.0) && near(page(&s, top), 45.0));
    assert_eq!(undo_steps(&s), steps + 1);
    // A flip is kept.
    ex(&mut s, "object.flipH", json!({"ids": [r]}));
    set(&mut s, &[r], json!({"absoluteRotation": 30}));
    assert!(near(page(&s, r), 30.0) && flipped(&s, r), "{}", page(&s, r));
    // One of the two keys per call.
    assert!(s.execute("node.set", &json!({"ids": [r], "props": {"rotation": 1, "absoluteRotation": 2}})).is_err());
}

#[test]
fn absolute_rotation_on_a_frame_and_a_layer_inside_it_lands_both_on_the_page_angle() {
    let mut s = session();
    let page = |s: &Session, i: NodeId| rotation_degrees(s.doc().unwrap().world_transform(i));
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 100, "width": 400, "height": 400})));
    set(&mut s, &[f], json!({"rotation": 10}));
    let r = rect(&mut s, 100.0, 100.0, 120.0, 80.0, Some(f));
    // The layer inside comes first in the ids: the frame's turn must not carry it past 45.
    set(&mut s, &[r, f], json!({"absoluteRotation": 45}));
    assert!(near(page(&s, f), 45.0), "{}", page(&s, f));
    assert!(near(page(&s, r), 45.0), "{}", page(&s, r));
}

#[test]
fn constrain_proportions_keeps_the_other_side_in_ratio() {
    let mut s = session();
    // The Layout panel's link is a per-layer property: typing W or H scales the other side by the
    // node's own ratio; both sides in one edit stay as typed, and so does an unlinked edit.
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"constrainProportions": true}));
    set(&mut s, &[r], json!({"width": 200}));
    assert_eq!(size(&s, r), (200.0, 100.0), "W ×2 makes H ×2");
    set(&mut s, &[r], json!({"height": 25}));
    assert_eq!(size(&s, r), (50.0, 25.0), "H 25 makes W follow the ratio");
    set(&mut s, &[r], json!({"width": 30, "height": 40}));
    assert_eq!(size(&s, r), (30.0, 40.0), "both sides in one edit stay as typed");
    set(&mut s, &[r], json!({"constrainProportions": false}));
    set(&mut s, &[r], json!({"width": 60}));
    assert_eq!(size(&s, r), (60.0, 40.0), "unlinked: the other side stays put");
    // The short aliases work the same, and each layer of a selection follows its own ratio.
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0, None);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 10.0, None);
    set(&mut s, &[a, b], json!({"constrainProportions": true}));
    set(&mut s, &[a, b], json!({"h": 20}));
    assert_eq!((size(&s, a), size(&s, b)), ((40.0, 20.0), (60.0, 20.0)), "each layer keeps its own ratio");
    // A zero side has no ratio to keep: the edit applies to the side typed only.
    let z = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[z], json!({"constrainProportions": true, "height": 0}));
    set(&mut s, &[z], json!({"width": 20}));
    assert_eq!(size(&s, z), (20.0, 0.0), "a zero height leaves nothing to scale");
}

#[test]
fn a_press_anywhere_on_a_corner_handle_resizes() {
    // The canvas draws each corner handle as an 8 px square centred on the corner, and the
    // corner-radius handle sits 10 px in along the diagonal: a press on the square's inner half
    // resizes (both sides, in ratio with the link on), it doesn't pick the radius handle.
    let radii = |s: &Session, r: NodeId| s.doc().unwrap().node(r).unwrap().corners.radii;
    for lock in [false, true] {
        let mut s = session();
        let r = rect(&mut s, 0.0, 0.0, 1000.0, 600.0, None);
        set(&mut s, &[r], json!({"constrainProportions": lock}));
        ex(&mut s, "select.set", json!({"ids": [r]}));
        let on_square = Point::new(997.0, 597.0);
        s.pointer(&ev(PointerKind::Hover, on_square, NONE)).unwrap();
        assert!(matches!(s.tool.overlay.cursor, Cursor::Resize(_)), "lock {lock}: {:?}", s.tool.overlay.cursor);
        drag(&mut s, on_square, Point::new(1197.0, 637.0), NONE);
        let (w, h) = size(&s, r);
        let want_h = if lock { 718.2 } else { 637.0 };
        assert!(near(w, 1197.0) && near(h, want_h), "lock {lock}: both sides follow the corner, got {w}×{h}");
        assert_eq!(radii(&s, r), [0.0; 4]);
    }
    // Further in, past the square, the radius handle still answers.
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 1000.0, 600.0, None);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    drag(&mut s, Point::new(993.0, 593.0), Point::new(980.0, 580.0), NONE);
    assert_eq!(size(&s, r), (1000.0, 600.0));
    assert!(radii(&s, r)[0] > 0.0, "the radius handle rounds the corners");
}

#[test]
fn frames_resize_with_constraints_and_cmd_ignores_them() {
    let mut s = session();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100})));
    let c = rect(&mut s, 150.0, 10.0, 40.0, 40.0, Some(f));
    set(&mut s, &[c], json!({"constraints": {"horizontal": "right"}}));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(300.0, 50.0), NONE);
    assert_eq!(bounds(&s, c).x0, 250.0, "a Right constraint keeps the distance to the right edge");
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(300.0, 50.0), CMD);
    assert_eq!(bounds(&s, c).x0, 150.0, "Cmd/Ctrl while resizing a frame ignores constraints");
    assert_eq!(bounds(&s, f).width(), 300.0);
    // From the left edge too: the children stay where they are on the page.
    drag(&mut s, Point::new(0.0, 50.0), Point::new(-50.0, 50.0), CMD);
    assert_eq!((bounds(&s, f).x0, bounds(&s, c).x0), (-50.0, 150.0));
}

#[test]
fn lines_and_text() {
    let mut s = session();
    // A line's ends set its length and angle; Shift keeps the angle to 45° steps.
    let l = id(&ex(&mut s, "shape.line", json!({"x": 0, "y": 0, "width": 100})));
    ex(&mut s, "select.set", json!({"ids": [l]}));
    drag(&mut s, Point::new(100.0, 0.0), Point::new(60.0, 80.0), NONE);
    assert!(near(size(&s, l).0, 100.0) && near(rotation(&s, l).abs(), 53.13010235415598), "{} {}", size(&s, l).0, rotation(&s, l));
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, Point::new(100.0, 0.0), Point::new(60.0, 80.0), SHIFT);
    assert!(near(rotation(&s, l).abs(), 45.0), "{}", rotation(&s, l));
    // Text: a side handle makes auto-width text fixed width (auto height); another handle a fixed
    // box.
    let t = id(&ex(&mut s, "text.create", json!({"text": "Hello world", "x": 0, "y": 200})));
    let auto = |s: &Session| s.doc().unwrap().node(t).unwrap().kind.text().map(|d| d.auto_resize);
    // Large type, so the box is big enough for side handles whatever font the machine falls back to.
    set(&mut s, &[t], json!({"fontSize": 48}));
    assert_eq!(auto(&s), Some(TextAutoResize::WidthAndHeight));
    ex(&mut s, "select.set", json!({"ids": [t]}));
    let b = bounds(&s, t);
    // Text sizes need a font (CI may have none).
    if b.height() > 4.0 {
        drag(&mut s, Point::new(b.x1 + 2.0, b.center().y), Point::new(b.x1 + 30.0, b.center().y), NONE);
        assert_eq!(auto(&s), Some(TextAutoResize::Height), "a side handle: auto height");
        let b = bounds(&s, t);
        drag(&mut s, Point::new(b.x1 + 2.0, b.y1 + 2.0), Point::new(b.x1 + 10.0, b.y1 + 20.0), NONE);
        assert_eq!(auto(&s), Some(TextAutoResize::None), "a corner: fixed size");
    }
}

#[test]
fn nudging_and_the_pixel_grid() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    // Arrows move by 1, Shift+arrows by the big nudge (10, a preference).
    ex(&mut s, "transform.nudge", json!({"dx": 1, "dy": 0}));
    ex(&mut s, "transform.nudge", json!({"dx": 0, "dy": 1, "big": true}));
    assert_eq!(bounds(&s, r).origin(), Point::new(1.0, 10.0));
    ex(&mut s, "pref.nudge", json!({"big": 8}));
    ex(&mut s, "transform.nudge", json!({"dx": -1, "dy": 0, "big": true}));
    assert_eq!(bounds(&s, r).origin(), Point::new(-7.0, 10.0));
    // With the pixel grid on, moves, resizes and new shapes land on whole pixels.
    drag(&mut s, Point::new(20.0, 20.0), Point::new(30.3, 20.6), NONE);
    assert_eq!(bounds(&s, r).origin(), Point::new(3.0, 11.0), "moved by a whole pixel");
    drag(&mut s, Point::new(103.0, 61.0), Point::new(150.4, 80.6), NONE);
    assert_eq!(size(&s, r), (147.0, 70.0), "resized to whole pixels");
    // Off, they follow the pointer exactly.
    ex(&mut s, "view.snapPixel", json!({"on": false}));
    drag(&mut s, Point::new(150.0, 81.0), Point::new(160.5, 81.25), NONE);
    assert_eq!(size(&s, r), (157.5, 70.25));
}

#[test]
fn smart_guides_snap_to_edges_centres_and_equal_spacing() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
    let c = rect(&mut s, 400.0, 0.0, 100.0, 100.0, None);
    let b = rect(&mut s, 200.0, 300.0, 100.0, 100.0, None);
    let _ = (a, c);
    ex(&mut s, "select.set", json!({"ids": [b]}));
    // (drag to, B's box after, derivation): B is grabbed at its centre (250, 350).
    let rows: [((f64, f64), (f64, f64), &str); 4] = [
        ((153.0, 350.0), (100.0, 300.0), "silent: an edge within 5 screen px of another layer's edge snaps to it (pinned)"),
        ((302.0, 52.0), (252.0, 0.0), "edges and centres snap (its top to the others' tops)"),
        ((248.0, 52.0), (200.0, 0.0), "between two layers, it snaps to equal gaps on both sides"),
        ((852.0, 52.0), (800.0, 0.0), "beyond the last of a row, to the gap the row already has"),
    ];
    for (to, want, why) in rows {
        drag(&mut s, Point::new(250.0, 350.0), Point::new(to.0, to.1), NONE);
        assert_eq!(bounds(&s, b).origin(), Point::new(want.0, want.1), "{why}");
        ex(&mut s, "edit.undo", json!({}));
    }
    // While dragging, the equal gaps show as labelled lines; Cmd/Ctrl turns snapping off.
    s.pointer(&ev(PointerKind::Down, Point::new(250.0, 350.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(248.0, 52.0), NONE)).unwrap();
    let labels: Vec<String> = s.tool.overlay.guides.iter().filter_map(|g| g.label.clone()).collect();
    assert_eq!(labels, vec!["100", "100"], "{:?}", s.tool.overlay.guides);
    s.pointer(&ev(PointerKind::Move, Point::new(248.0, 52.0), CMD)).unwrap();
    s.pointer(&ev(PointerKind::Up, Point::new(248.0, 52.0), CMD)).unwrap();
    assert_eq!(bounds(&s, b).origin(), Point::new(198.0, 2.0));
}

#[test]
fn a_layer_edge_snaps_within_4_5_screen_pixels_at_any_zoom() {
    // Measured in Figma (2026-11-08): a dragged edge snaps at a gap of 4.4 screen px and not at 4.6,
    // at 100% and at 200% zoom.
    for (zoom, gap_px, snaps) in [(1.0, 4.4, true), (1.0, 4.8, false), (2.0, 4.4, true), (2.0, 4.8, false)] {
        let mut s = Session::new();
        ex(&mut s, "view.snapPixel", json!({"on": false}));
        rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
        let b = rect(&mut s, 300.0, 300.0, 100.0, 100.0, None);
        ex(&mut s, "select.set", json!({"ids": [b]}));
        let gap = gap_px / zoom;
        let to = Point::new(350.0 - (200.0 - gap), 350.0);
        for (kind, p) in [(PointerKind::Down, Point::new(350.0, 350.0)), (PointerKind::Move, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::at(kind, p.x, p.y, zoom)).unwrap();
        }
        let want = if snaps { 100.0 } else { 100.0 + gap };
        let got = bounds(&s, b).x0;
        assert!(near(got, want), "zoom {zoom}, gap {gap_px} screen px: left edge at {got}, wanted {want}");
    }
}

#[test]
fn snapping_to_layers_wins_over_the_pixel_grid() {
    // Both on (the defaults): a layer's centre at x = 5.5 is still a target.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 11.0, 11.0, None);
    let b = rect(&mut s, 200.0, 300.0, 100.0, 100.0, None);
    let _ = a;
    ex(&mut s, "select.set", json!({"ids": [b]}));
    // Moving: B's centre (grabbed) snaps to A's; the free axis stays on whole pixels.
    s.pointer(&ev(PointerKind::Down, Point::new(250.0, 350.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(6.0, 352.4), NONE)).unwrap();
    assert!(s.tool.overlay.guides.iter().any(|g| near(g.x0, 5.5) && near(g.x1, 5.5)), "{:?}", s.tool.overlay.guides);
    s.pointer(&ev(PointerKind::Up, Point::new(6.0, 352.4), NONE)).unwrap();
    assert_eq!(bounds(&s, b).origin(), Point::new(-44.5, 302.0), "moved onto A's centre");
    ex(&mut s, "edit.undo", json!({}));
    // Out of reach of any layer, a move lands on whole pixels.
    drag(&mut s, Point::new(250.0, 350.0), Point::new(260.3, 360.6), NONE);
    assert_eq!(bounds(&s, b).origin(), Point::new(210.0, 311.0));
    ex(&mut s, "edit.undo", json!({}));
    // Resizing: the left handle snaps to A's centre; the bottom one lands on whole pixels.
    drag(&mut s, Point::new(200.0, 350.0), Point::new(6.0, 350.0), NONE);
    assert!(near(bounds(&s, b).x0, 5.5), "{:?}", bounds(&s, b));
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, Point::new(250.0, 400.0), Point::new(250.0, 450.6), NONE);
    assert_eq!(size(&s, b), (100.0, 151.0));
}

#[test]
fn frames_land_on_whole_pixels_whatever_they_snap_to() {
    // Figma's help: frames, sections and components "always snap to the pixel grid, even if Snap
    // to pixel grid is disabled". Other layers follow the setting.
    let frame = |s: &mut Session, x: f64, y: f64| id(&ex(s, "frame.create", json!({"x": x, "y": y, "width": 100, "height": 100})));
    let whole = |r: Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.fract() == 0.0);
    let mut s = Session::new();
    // A neighbour off the pixel grid, as a scaled or imported layer leaves one: its right edge at
    // 100.4.
    rect(&mut s, 0.4, 0.0, 100.0, 100.0, None);
    let f = frame(&mut s, 300.0, 300.0);
    let r = rect(&mut s, 300.0, 600.0, 100.0, 100.0, None);
    // Moved next to it, a rectangle snaps onto 100.4 and the frame to the whole pixel there.
    ex(&mut s, "select.set", json!({"ids": [r]}));
    drag(&mut s, Point::new(350.0, 650.0), Point::new(151.5, 650.0), Mods::default());
    assert!(near(bounds(&s, r).x0, 100.4), "{:?}", bounds(&s, r));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(350.0, 350.0), Point::new(151.5, 350.0), Mods::default());
    assert_eq!(bounds(&s, f).origin(), Point::new(100.0, 300.0), "moved by the snap, onto the whole pixel");
    // With Snap to pixel grid off, frames still move, resize and draw on whole pixels.
    ex(&mut s, "view.snapPixel", json!({"on": false}));
    ex(&mut s, "view.snapObjects", json!({"on": false}));
    drag(&mut s, Point::new(150.0, 350.0), Point::new(160.3, 360.6), Mods::default());
    assert_eq!(bounds(&s, f).origin(), Point::new(110.0, 311.0), "moved");
    drag(&mut s, Point::new(210.0, 411.0), Point::new(250.4, 450.6), Mods::default());
    assert_eq!(size(&s, f), (140.0, 140.0), "resized");
    ex(&mut s, "tool.set", json!({"tool": "frame"}));
    drag(&mut s, Point::new(500.3, 500.6), Point::new(560.6, 540.2), Mods::default());
    let drawn = s.selection()[0];
    assert!(whole(bounds(&s, drawn)), "drawn: {:?}", bounds(&s, drawn));
    // A frame already off the pixel grid lands on it once moved.
    ex(&mut s, "tool.set", json!({"tool": "move"}));
    let off = frame(&mut s, 800.4, 800.7);
    ex(&mut s, "select.set", json!({"ids": [off]}));
    drag(&mut s, Point::new(850.0, 850.0), Point::new(860.0, 850.0), Mods::default());
    assert!(whole(bounds(&s, off)), "{:?}", bounds(&s, off));
    // A rectangle follows the pointer.
    ex(&mut s, "select.set", json!({"ids": [r]}));
    drag(&mut s, Point::new(150.0, 650.0), Point::new(160.3, 650.0), Mods::default());
    assert!(near(bounds(&s, r).x0, 110.7), "{:?}", bounds(&s, r));
}

#[test]
fn frames_resized_together_each_land_on_whole_pixels() {
    let mut s = session();
    let frame = |s: &mut Session, x: f64| id(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 100, "height": 100})));
    let (a, b) = (frame(&mut s, 0.0), frame(&mut s, 200.0));
    ex(&mut s, "select.set", json!({"ids": [a, b]}));
    // 300 wide to 400: a third more, so each frame alone would be 133.33 wide.
    drag(&mut s, Point::new(300.0, 100.0), Point::new(400.0, 100.0), Mods::default());
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 133.0, 100.0));
    assert_eq!(bounds(&s, b), Rect::new(267.0, 0.0, 400.0, 100.0));
}

#[test]
fn a_frame_dropped_into_a_frame_off_the_pixel_grid_lands_on_its_whole_pixels() {
    let mut s = session();
    let target = id(&ex(&mut s, "frame.create", json!({"x": 300.5, "y": 0.5, "width": 300, "height": 300})));
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50})));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(25.0, 25.0), Point::new(425.0, 125.0), Mods::default());
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert_eq!(n.parent, Some(target));
    let [.., x, y] = n.transform.as_coeffs();
    assert_eq!((x.fract(), y.fract()), (0.0, 0.0), "at {x}, {y} in the frame");
}

#[test]
fn corner_cursors_stay_diagonal_whatever_the_proportions() {
    let mut s = session();
    let mod180 = |a: f64, b: f64| near((a - b).rem_euclid(180.0).min((b - a).rem_euclid(180.0)), 0.0);
    for (w, h) in [(65.0, 15.0), (100.0, 100.0), (60.0, 200.0), (200.0, 50.0)] {
        let r = rect(&mut s, 0.0, 0.0, w, h, None);
        let doc = s.doc().unwrap().clone();
        let q = selection_box(&doc, &[r]).unwrap();
        for (i, want) in [(0u8, 45.0), (1, 135.0), (2, 45.0), (3, 135.0)] {
            let Cursor::Resize(a) = handle_cursor(&q, Handle::Corner(i)) else { panic!() };
            assert!(mod180(a, want), "{w}x{h} corner {i}: {a}, want {want}");
        }
        // Turned, the corner cursors turn with the layer's own axes.
        set(&mut s, &[r], json!({"rotation": 30}));
        let doc = s.doc().unwrap().clone();
        let q = selection_box(&doc, &[r]).unwrap();
        let axis = (q[1] - q[0]).atan2().to_degrees();
        for (i, off) in [(0u8, 45.0), (1, 135.0), (2, 45.0), (3, 135.0)] {
            let Cursor::Resize(a) = handle_cursor(&q, Handle::Corner(i)) else { panic!() };
            assert!(mod180(a, axis + off), "{w}x{h} turned corner {i}: {a}, want {}", axis + off);
        }
    }
}

#[test]
fn the_transform_box_and_its_handles() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
    let doc = s.doc().unwrap().clone();
    let z = 1.0;
    // Cursors follow the handle and the box's rotation; outside a corner, the rotation cursor.
    let q = selection_box(&doc, &[r]).unwrap();
    for (h, angle) in [(Handle::Corner(0), 45.0), (Handle::Corner(1), 135.0), (Handle::Edge(0), 90.0), (Handle::Edge(1), 180.0)] {
        assert!(matches!(handle_cursor(&q, h), Cursor::Resize(a) if near(a, angle)), "{h:?}");
    }
    assert!(matches!(handle_cursor(&q, Handle::Rotate(1)), Cursor::Rotate(a) if near(a, -45.0)), "turned to the top right corner");
    assert_eq!(handle_at(&doc, &[r], Point::new(110.0, -10.0), z), Some(Handle::Rotate(1)));
    set(&mut s, &[r], json!({"rotation": 30}));
    let doc = s.doc().unwrap().clone();
    let q = selection_box(&doc, &[r]).unwrap();
    let Cursor::Resize(a) = handle_cursor(&q, Handle::Edge(1)) else { panic!() };
    assert!(near((a - 150.0).rem_euclid(180.0), 0.0), "the edge cursor turns with the layer: {a}");
    // Hovering sets the cursor.
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let corner = doc.world_transform(r) * Point::new(100.0, 100.0);
    s.pointer(&ev(PointerKind::Hover, corner, NONE)).unwrap();
    assert!(matches!(s.tool.overlay.cursor, Cursor::Resize(_)));
    // A layer smaller than its handles on screen: a press inside it moves it, the handles still
    // answer just outside it.
    let tiny = rect(&mut s, 300.0, 0.0, 4.0, 4.0, None);
    ex(&mut s, "select.set", json!({"ids": [tiny]}));
    let doc = s.doc().unwrap().clone();
    assert_eq!(handle_at(&doc, &[tiny], Point::new(302.0, 2.0), z), None, "inside: no handle");
    assert_eq!(handle_at(&doc, &[tiny], Point::new(297.0, 297.0 - 300.0), z), Some(Handle::Corner(0)), "outside the corner: its handle");
    drag(&mut s, Point::new(302.0, 2.0), Point::new(322.0, 12.0), NONE);
    assert_eq!(bounds(&s, tiny), Rect::new(320.0, 10.0, 324.0, 14.0), "a press inside moves it");
    drag(&mut s, Point::new(326.0, 16.0), Point::new(340.0, 30.0), NONE);
    assert_eq!(bounds(&s, tiny), Rect::new(320.0, 10.0, 340.0, 30.0), "the corner handle just outside resizes it");
    // At a larger zoom the same layer is big enough on screen for its handles inside it.
    assert_eq!(handle_at(s.doc().unwrap(), &[tiny], Point::new(320.2, 10.2), 20.0), Some(Handle::Corner(0)));
}

#[test]
fn command_crop_changes_a_locked_ratio_and_stops_at_the_images_edges() {
    let mut s = session();
    let layer = image_layer(&mut s);
    set(&mut s, &[layer], json!({"constrainProportions": true}));
    ex(&mut s, "select.set", json!({"ids": [layer]}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(150.0, 50.0), CMD);
    assert_eq!(size(&s, layer), (150.0, 100.0), "a side changes the ratio, lock or not");
    drag(&mut s, Point::new(150.0, 50.0), Point::new(260.0, 50.0), CMD);
    assert_eq!(size(&s, layer), (200.0, 100.0), "the side stops at the image's edge");
    drag(&mut s, Point::new(100.0, 0.0), Point::new(100.0, -40.0), CMD);
    assert!(near_rect(bounds(&s, layer), Rect::new(0.0, 0.0, 200.0, 100.0)), "the top stops at the image's top: {:?}", bounds(&s, layer));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(100.0, 50.0), Mods { shift: true, alt: false, cmd: true });
    assert_eq!(size(&s, layer), (100.0, 50.0), "Shift keeps the ratio");
}
