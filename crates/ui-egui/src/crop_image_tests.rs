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
    // The picture's own corner, now outside the layer at (200, 100), scales the picture inside the
    // layer, which keeps its size and its selection.
    let (from, to) = (u.app.to_screen(200.0, 100.0), u.app.to_screen(240.0, 120.0));
    u.drag_to(from, to);
    u.button(to, false);
    assert_eq!(u.app.session.selection(), vec![layer], "the layer stays selected");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.width - 150.0).abs() < 1e-6 && (n.height - 75.0).abs() < 1e-6, "the layer keeps its size: {} x {}", n.width, n.height);
    let (_, Some(scaled)) = mode(&u, layer) else { panic!("a crop") };
    // The picture grew from 200 x 100 to 240 x 120: the layer shows 5/6 of what it showed.
    assert!((scaled[0] - after[0] * 5.0 / 6.0).abs() < 1e-6 && (scaled[3] - after[3] * 5.0 / 6.0).abs() < 1e-6, "{after:?} -> {scaled:?}");
}

/// The reporter's sequence (#2307): the colour picker open on an image fill held in Crop mode
/// (Crop chosen in its mode dropdown, or reopened on a fill that already is), then a press on the
/// layer and a drag. The drag moves the picture inside the layer (its crop), not the layer, the
/// press doesn't close the picker under it, and no internal gesture message reaches the user.
#[test]
fn crop_from_the_colour_picker_then_a_drag_on_the_image_moves_the_crop_not_the_layer() {
    let (mut u, layer, _) = setup();
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![layer] });
    for _ in 0..4 {
        u.frame(vec![]);
    }
    choose_mode(&mut u, "Fill", "Crop");
    // Crop mode on the canvas ended (a click away from the image); the picker opens on the Crop fill.
    u.app.ui.crop = None;
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![layer] });
    for _ in 0..4 {
        u.frame(vec![]);
    }
    let (_, Some(before)) = mode(&u, layer) else { panic!("a crop") };
    let at = u.app.session.doc().unwrap().node(layer).unwrap().transform.as_coeffs();
    let from = u.app.to_screen(100.0, 50.0);
    u.drag_to(from, from + vec2(30.0, 10.0));
    u.frame(vec![]);
    u.button(from + vec2(30.0, 10.0), false);
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert_eq!(n.transform.as_coeffs(), at, "the layer stays where it is");
    assert_eq!((n.width, n.height), (200.0, 100.0), "and keeps its size");
    let (m, Some(after)) = mode(&u, layer) else { panic!("a crop") };
    assert_eq!(m, ImageScaleMode::Crop);
    assert_ne!(after, before, "the drag moved the picture inside the layer");
    assert_eq!(u.app.session.selection(), vec![layer]);
    assert!(u.app.ui.toast.as_ref().is_none_or(|(t, _)| !t.contains("gesture")), "no internal gesture message: {:?}", u.app.ui.toast);
}

/// A drag the colour picker's close committed under the held button (a press on a layer closing
/// the picker) leaves the rest of its drag with no gesture: that says nothing to the user.
#[test]
fn a_drag_whose_gesture_was_committed_under_the_button_shows_no_message() {
    let (mut u, layer, _) = setup();
    u.app.run("select.set", json!({"ids": [layer]}));
    let from = u.app.to_screen(100.0, 50.0);
    u.button(from, true);
    u.app.session.commit_interaction().unwrap();
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(from + vec2(3.0, 1.0) * k as f32)]);
    }
    u.button(from + vec2(30.0, 10.0), false);
    assert!(u.app.ui.toast.is_none(), "no message: {:?}", u.app.ui.toast);
}

/// The first paint, an image, of `id`'s fills (`fills`) or strokes, or of region `region` of a vector.
fn image_at(u: &Ui, id: NodeId, fills: bool, region: Option<usize>) -> figcraft_color::ImagePaint {
    let n = u.app.session.doc().unwrap().node(id).unwrap();
    let list = match region {
        Some(r) => match &n.kind {
            figcraft_doc::NodeKind::Vector { network } => network.regions[r].fills.clone().unwrap(),
            _ => panic!("not a vector"),
        },
        None if fills => n.fills.clone(),
        None => n.strokes.clone(),
    };
    match &list[0].kind {
        PaintKind::Image(img) => img.clone(),
        k => panic!("not an image: {k:?}"),
    }
}

/// Crop chosen in the picker for a paint that has no crop mode on the canvas (#2150) — a stroke,
/// several layers' fills at once, or a region's fill in vector edit mode — only sets the paint's
/// mode: crop mode doesn't start and the picker stays open. Targets that aren't one layer's image
/// fill have nothing for Crop image to crop either.
#[test]
fn crop_in_the_colour_picker_only_sets_the_mode_where_crop_image_does_not_apply() {
    use crate::colorpicker::{PickerTarget, crop_on_canvas};
    let picture = |u: &Ui, id: NodeId| u.app.session.doc().unwrap().node(id).unwrap().fills[0].clone();
    // Opens the picker on `target`, chooses Crop and checks crop mode stays off.
    let crop_only_sets_the_mode = |u: &mut Ui, target: PickerTarget, what: &str| {
        u.app.ui.picker = Some(target.clone());
        for _ in 0..4 {
            u.frame(vec![]);
        }
        choose_mode(u, "Fill", "Crop");
        assert_eq!(u.app.ui.crop, None, "{what}: no crop mode");
        assert_eq!(u.app.ui.picker, Some(target), "{what}: the picker stays open");
    };

    // The layer's image as its stroke: the stroke's mode is set, the fill is left alone.
    let (mut u, layer, _) = setup();
    let paint = picture(&u, layer);
    u.app.run("stroke.add", json!({"ids": [layer], "paint": paint}));
    crop_only_sets_the_mode(&mut u, PickerTarget::Paint { fills: false, index: 0, ids: vec![layer] }, "a stroke");
    assert_eq!(image_at(&u, layer, false, None).scale_mode, ImageScaleMode::Crop);
    assert_eq!(mode(&u, layer), (ImageScaleMode::Fill, None), "the fill is untouched");

    // Two image layers at once: both take the mode.
    let (mut u, layer, rect) = setup();
    let paint = picture(&u, layer);
    u.app.run("fill.set", json!({"ids": [rect], "index": 0, "paint": paint}));
    u.app.run("select.set", json!({"ids": [layer, rect]}));
    crop_only_sets_the_mode(&mut u, PickerTarget::Paint { fills: true, index: 0, ids: vec![layer, rect] }, "two layers");
    assert_eq!(mode(&u, layer).0, ImageScaleMode::Crop);
    assert_eq!(mode(&u, rect).0, ImageScaleMode::Crop);

    // A region's own image fill in vector edit mode: the region's mode is set, the layer's image
    // fill is untouched.
    let (u0, layer, _) = setup();
    let paint = picture(&u0, layer);
    let mut s = Session::new();
    let vector = crate::region_fill_rows_tests::two_region_vector(&mut s);
    // The layer's own fill is an image too, so cropping it instead would be possible, and wrong.
    s.execute("fill.set", &json!({"ids": [vector.0], "index": 0, "paint": paint})).unwrap();
    s.execute("vector.edit.begin", &json!({"id": vector.0})).unwrap();
    s.execute("vector.edit.select", &json!({"regions": [1]})).unwrap();
    s.execute("fill.set", &json!({"ids": [vector.0], "index": 0, "paint": paint})).unwrap();
    let layer_fills = s.doc().unwrap().node(vector).unwrap().fills.clone();
    let mut u = Ui::new(s);
    crop_only_sets_the_mode(&mut u, PickerTarget::Paint { fills: true, index: 0, ids: vec![vector] }, "a region");
    assert_eq!(image_at(&u, vector, true, Some(1)).scale_mode, ImageScaleMode::Crop);
    assert_eq!(u.app.session.doc().unwrap().node(vector).unwrap().fills, layer_fills, "the layer's fills are untouched");

    // A fill that isn't an image, and targets that aren't a layer's paint, have no crop on the canvas.
    let (u, layer, rect) = setup();
    assert_eq!(crop_on_canvas(&u.app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![layer] }), Some((layer, 0)));
    assert_eq!(crop_on_canvas(&u.app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![rect] }), None, "a solid fill");
    assert_eq!(crop_on_canvas(&u.app, &PickerTarget::Paint { fills: true, index: 1, ids: vec![layer] }), None, "no fill there");
    assert_eq!(crop_on_canvas(&u.app, &PickerTarget::PageBackground), None);
    assert_eq!(crop_on_canvas(&u.app, &PickerTarget::Style { id: "S:1".into(), index: 0 }), None);
}
