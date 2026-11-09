//! Crop image on the canvas: the right-click menu offers it for one image layer (and not for a
//! layer without an image fill), it enters crop mode with the image's handles, dragging the image
//! moves its crop as one undo step, and Escape, Enter or a click outside the image finishes it
//! without deselecting.

use base64::Engine as _;
use egui::{Event, Key, Modifiers, Pos2, vec2};
use figcraft_color::{ImageScaleMode, PaintKind};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::present_tests::id;

/// A 200×100 image layer at (0, 0) and a plain rectangle beside it; the image is selected.
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let image = image::RgbaImage::from_fn(40, 20, |x, y| image::Rgba([(x * 6) as u8, (y * 12) as u8, 120, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let rect = id(&s.execute("shape.rectangle", &json!({"x": 300, "y": 0, "width": 80, "height": 80})).unwrap());
    let layer = id(&s.execute("image.place", &json!({"data": data, "x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [layer]})).unwrap();
    (Ui::new(s), layer, rect)
}

fn key(u: &mut Ui, key: Key) {
    let modifiers = Modifiers::NONE;
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn mode(u: &Ui, layer: NodeId) -> (ImageScaleMode, Option<[f64; 6]>) {
    match &u.app.session.doc().unwrap().node(layer).unwrap().fills[0].kind {
        PaintKind::Image(img) => (img.scale_mode, img.crop),
        _ => panic!("image fill"),
    }
}

fn menu_has_crop(u: &mut Ui, ids: Vec<NodeId>) -> bool {
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), ids));
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    u.app.ui.context_menu = None;
    has_text(&shapes, "Crop image")
}

#[test]
fn the_context_menu_enters_crop_mode_and_escape_or_enter_finishes_it() {
    let (mut u, layer, rect) = setup();
    u.app.run("select.set", json!({"ids": [rect]}));
    assert!(!menu_has_crop(&mut u, vec![rect]), "a layer without an image fill has nothing to crop");
    u.app.run("select.set", json!({"ids": [layer]}));
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![layer]));
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let at = text_center(&shapes, "Crop image");
    u.button(at, true);
    u.button(at, false);
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "crop mode on the image fill");
    assert_eq!(mode(&u, layer).0, ImageScaleMode::Crop);
    for k in [Key::Escape, Key::Enter] {
        u.frame(vec![]);
        key(&mut u, k);
        assert_eq!(u.app.ui.crop, None, "{k:?} finishes crop mode");
        assert_eq!(u.app.session.selection(), vec![layer], "{k:?} keeps the selection");
        crate::menus::invoke(&mut u.app, "image.crop");
        assert_eq!(u.app.ui.crop, Some((layer, 0)), "Object ▸ Crop image enters it again");
    }
}

#[test]
fn dragging_the_image_in_crop_mode_moves_its_crop_and_a_click_outside_finishes() {
    let (mut u, layer, _) = setup();
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let steps = u.app.session.ds().unwrap().history.undo.len();
    let (_, before) = mode(&u, layer);
    let from = u.app.to_screen(100.0, 50.0);
    u.drag_to(from, from + vec2(30.0, 0.0));
    u.button(from + vec2(30.0, 0.0), false);
    let (m, after) = mode(&u, layer);
    assert_eq!(m, ImageScaleMode::Crop);
    assert_ne!(after, before, "the drag moved the image inside the layer");
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), steps + 1, "one undo step");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
    // A click away from the image ends crop mode.
    let away = u.app.to_screen(250.0, 300.0);
    u.button(away, true);
    u.button(away, false);
    assert_eq!(u.app.ui.crop, None);
}

#[test]
fn in_crop_mode_the_layers_edges_crop_without_moving_the_picture() {
    let (mut u, layer, _) = setup();
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let (_, Some(before)) = mode(&u, layer) else { panic!("a crop") };
    let (from, to) = (u.app.to_screen(200.0, 50.0), u.app.to_screen(150.0, 50.0));
    u.drag_to(from, to);
    u.button(to, false);
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.width - 150.0).abs() < 1e-6 && n.transform.as_coeffs()[4].abs() < 1e-6, "the right edge moved in: {} wide", n.width);
    let (m, Some(after)) = mode(&u, layer) else { panic!("a crop") };
    assert_eq!(m, ImageScaleMode::Crop);
    // The unit box shrank to 3/4 of its width over the same pixels: x scale 3/4, same offset.
    assert!((after[0] - before[0] * 0.75).abs() < 1e-9 && (after[4] - before[4]).abs() < 1e-9, "{before:?} -> {after:?}");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
    // The cut-away part shows dimmed: one band right of the layer, textured from the image.
    let ghost = |u: &mut Ui| {
        flat(&u.frame_output(vec![]).shapes).into_iter().find_map(|s| match s {
            egui::Shape::Mesh(m) if m.vertices.iter().all(|v| v.color == egui::Color32::from_white_alpha(115)) => Some(m),
            _ => None,
        })
    };
    let m = ghost(&mut u).expect("the dimmed image outside the layer");
    assert_ne!(m.texture_id, egui::TextureId::default());
    let (edge, end) = (u.app.to_screen(150.0, 0.0).x, u.app.to_screen(200.0, 0.0).x);
    assert_eq!(m.vertices.len(), 4, "one band");
    assert!(m.vertices.iter().all(|v| (v.pos.x - edge).abs() < 0.5 || (v.pos.x - end).abs() < 0.5), "from the layer's edge to the image's");
    assert!(m.vertices.iter().all(|v| v.uv.x >= 0.74 && v.uv.x <= 1.0 + 1e-4), "the image's right quarter");
    key(&mut u, Key::Escape);
    assert!(ghost(&mut u).is_none(), "gone with crop mode");
}

/// Choose `item` from the image mode dropdown of the open colour picker, which reads `shown`.
fn choose_mode(u: &mut Ui, shown: &str, item: &str) {
    for text in [shown, item] {
        // The picker is drawn over the Design panel, so its text comes last.
        let shapes = flat(&u.frame_output(vec![]).shapes);
        let (_, p, size) = crate::live_tests::texts(&shapes).into_iter().rfind(|(t, _, _)| t == text).unwrap_or_else(|| panic!("no {text:?}"));
        u.button(p + size / 2.0, true);
        u.button(p + size / 2.0, false);
    }
    u.frame(vec![]);
}

/// Crop chosen as the image fill's mode in the colour picker enters crop mode, as Crop image does
/// (#2150): the picker closes, the picture keeps its place, and a corner handle then crops it
/// with the layer still selected. The other modes only set the mode.
#[test]
fn crop_chosen_in_the_colour_picker_enters_crop_mode() {
    let (mut u, layer, _) = setup();
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![layer] });
    for _ in 0..4 {
        u.frame(vec![]);
    }
    choose_mode(&mut u, "Fill", "Fit");
    assert_eq!(mode(&u, layer), (ImageScaleMode::Fit, None));
    assert!(u.app.ui.picker.is_some() && u.app.ui.crop.is_none(), "Fit is only a mode");
    choose_mode(&mut u, "Fit", "Crop");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "crop mode on the image fill");
    assert!(u.app.ui.picker.is_none(), "crop mode takes the canvas from the picker");
    // The picture keeps its place: the crop is the one Crop image makes of the fitted image.
    let (m, Some(before)) = mode(&u, layer) else { panic!("a crop where the picture was") };
    assert_eq!(m, ImageScaleMode::Crop);
    let (mut menu, other, _) = setup();
    let mut fit = menu.app.session.doc().unwrap().node(other).unwrap().fills[0].clone();
    if let PaintKind::Image(img) = &mut fit.kind {
        img.scale_mode = ImageScaleMode::Fit;
    }
    menu.app.run("fill.set", json!({"ids": [other], "index": 0, "paint": fit}));
    assert_eq!(mode(&menu, other), (ImageScaleMode::Fit, None));
    menu.app.run("image.crop", json!({}));
    assert_eq!(mode(&menu, other), (ImageScaleMode::Crop, Some(before)));
    // The bottom right corner, dragged in, crops the picture to the top left 3/4 of the layer.
    let (from, to) = (u.app.to_screen(200.0, 100.0), u.app.to_screen(150.0, 75.0));
    u.drag_to(from, to);
    u.button(to, false);
    assert_eq!(u.app.session.selection(), vec![layer], "the layer stays selected");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.width - 150.0).abs() < 1e-6 && (n.height - 75.0).abs() < 1e-6, "{} x {}", n.width, n.height);
    let (_, Some(after)) = mode(&u, layer) else { panic!("a crop") };
    assert!((after[0] - before[0] * 0.75).abs() < 1e-9 && (after[3] - before[3] * 0.75).abs() < 1e-9, "{before:?} -> {after:?}");
    assert!((after[4] - before[4]).abs() < 1e-9 && (after[5] - before[5]).abs() < 1e-9, "the top left stays: {before:?} -> {after:?}");
}
