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

/// Crop chosen as the image fill's mode in the colour picker sets the mode with a placement crop
/// (#2499): the picker stays open, crop mode on the canvas does not start, and dragging a corner
/// handle without Cmd resizes the layer and morphs/squishes the image.
#[test]
fn crop_chosen_in_the_colour_picker_sets_mode_and_morphs_on_resize() {
    let (mut u, layer, _) = setup();
    u.app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills: true, index: 0, ids: vec![layer] });
    for _ in 0..4 {
        u.frame(vec![]);
    }
    choose_mode(&mut u, "Fill", "Fit");
    assert_eq!(mode(&u, layer), (ImageScaleMode::Fit, None));
    assert!(u.app.ui.picker.is_some() && u.app.ui.crop.is_none(), "Fit is only a mode");
    choose_mode(&mut u, "Fit", "Crop");
    assert_eq!(u.app.ui.crop, None, "Crop in picker does not enter canvas crop mode");
    assert!(u.app.ui.picker.is_some(), "picker stays open");
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
    // Close the picker to drag the canvas resize handles.
    u.app.ui.picker = None;
    u.frame(vec![]);
    // Dragging the bottom right corner without Cmd resizes the layer to 150x75 and morphs/squishes the image.
    let (from, to) = (u.app.to_screen(200.0, 100.0), u.app.to_screen(150.0, 75.0));
    u.drag_to(from, to);
    u.button(to, false);
    assert_eq!(u.app.session.selection(), vec![layer], "the layer stays selected");
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.width - 150.0).abs() < 1e-6 && (n.height - 75.0).abs() < 1e-6, "{} x {}", n.width, n.height);
    let (_, Some(after)) = mode(&u, layer) else { panic!("a crop") };
    assert_eq!(after, before, "normal resize without Cmd leaves crop transform unchanged to morph/squish");
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
    assert!(u.app.ui.picker.is_some());
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
    use crate::colorpicker::PickerTarget;
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

    // Two image layers at once: both take the mode, with each layer's crop sized to its own dimensions.
    let (mut u, layer, rect) = setup();
    let paint = picture(&u, layer);
    u.app.run("fill.set", json!({"ids": [rect], "index": 0, "paint": paint}));
    u.app.run("select.set", json!({"ids": [layer, rect]}));
    crop_only_sets_the_mode(&mut u, PickerTarget::Paint { fills: true, index: 0, ids: vec![layer, rect] }, "two layers");
    assert_eq!(mode(&u, layer).0, ImageScaleMode::Crop);
    assert_eq!(mode(&u, rect).0, ImageScaleMode::Crop);
    let (crop1, crop2) = (mode(&u, layer).1.unwrap(), mode(&u, rect).1.unwrap());
    assert_ne!(crop1, crop2, "differently sized layers must have distinct placement crops without stretching");

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
}

#[test]
fn in_crop_mode_the_edges_of_an_image_in_auto_layout_crop_it_too() {
    let (mut u, layer, _) = setup();
    let column = id(&u.app.session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    u.app.run("node.reparent", json!({"ids": [layer], "parent": column}));
    u.app.run("node.set", json!({"ids": [column], "props": {"layout": "vertical", "sizingV": "hug"}}));
    u.app.run("select.set", json!({"ids": [layer]}));
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let (_, Some(before)) = mode(&u, layer) else { panic!("a crop") };
    let (from, to) = (u.app.to_screen(100.0, 100.0), u.app.to_screen(100.0, 60.0));
    u.drag_to(from, to);
    u.button(to, false);
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.height - 60.0).abs() < 1e-6 && (n.width - 200.0).abs() < 1e-6, "the bottom edge moved up: {} x {}", n.width, n.height);
    let (m, Some(after)) = mode(&u, layer) else { panic!("a crop") };
    assert_eq!(m, ImageScaleMode::Crop);
    assert!((after[3] - before[3] * 0.6).abs() < 1e-9 && (after[0] - before[0]).abs() < 1e-9, "cropped, not squashed: {before:?} -> {after:?}");
    assert!((after[5] - before[5]).abs() < 1e-9, "the top stays: {before:?} -> {after:?}");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
}

#[test]
fn a_double_click_in_crop_mode_keeps_cropping_and_vector_edit_ends_crop_mode() {
    let (mut u, layer, _) = setup();
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let at = u.app.to_screen(100.0, 50.0);
    for _ in 0..2 {
        u.button(at, true);
        u.button(at, false);
    }
    u.frame(vec![]);
    assert!(u.app.session.ds().unwrap().vector_edit.is_none(), "a double-click while cropping doesn't edit the points");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
    let (from, to) = (u.app.to_screen(200.0, 50.0), u.app.to_screen(150.0, 50.0));
    u.drag_to(from, to);
    u.button(to, false);
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    assert!((n.width - 150.0).abs() < 1e-6 && matches!(n.kind, figcraft_doc::NodeKind::Rectangle), "the side crops; the layer stays a rectangle");
    u.app.run("vector.edit.begin", json!({"id": layer}));
    u.frame(vec![]);
    assert_eq!(u.app.ui.crop, None, "editing the points ends crop mode");
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    assert!(u.app.session.ds().unwrap().vector_edit.is_none(), "cropping ends editing the points");
    assert_eq!(u.app.ui.crop, Some((layer, 0)));
}

/// The second press of a double-click on a side of the layer begins a resize; its release, which
/// is the double-click, ends it, and crop mode stays.
#[test]
fn a_double_click_on_a_side_in_crop_mode_leaves_no_resize_open() {
    let (mut u, layer, _) = setup();
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let at = u.app.to_screen(200.0, 50.0);
    for _ in 0..2 {
        u.button(at, true);
        u.button(at, false);
    }
    u.frame(vec![]);
    assert!(u.app.session.ds().unwrap().interaction.is_none(), "the resize the second press began is over");
    assert_eq!(u.app.ui.crop, Some((layer, 0)), "still cropping");
}

#[test]
fn in_crop_mode_the_images_corners_scale_freely_and_with_shift_in_proportion() {
    let image_size = |u: &Ui, layer: NodeId| {
        let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
        let (_, Some(c)) = mode(u, layer) else { panic!("a crop") };
        let (x0, y0, x1, y1) = figcraft_color::ImagePaint::crop_rect(c);
        ((x1 - x0) * n.width, (y1 - y0) * n.height)
    };
    for shift in [false, true] {
        let (mut u, layer, _) = setup();
        u.app.run("image.crop", json!({}));
        u.frame(vec![]);
        let (from, to) = (u.app.to_screen(200.0, 50.0), u.app.to_screen(150.0, 50.0));
        u.drag_to(from, to);
        u.button(to, false);
        let (w0, h0) = image_size(&u, layer);
        let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        let (from, to) = (u.app.to_screen(200.0, 100.0), u.app.to_screen(260.0, 110.0));
        u.frame_with(
            vec![Event::PointerMoved(from), Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }],
            modifiers,
        );
        for k in 1..=10 {
            u.frame_with(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))], modifiers);
        }
        u.frame_with(vec![Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers }], modifiers);
        let (w1, h1) = image_size(&u, layer);
        let kept = (w1 / h1 - w0 / h0).abs() < 1e-6;
        assert!(w1 > w0, "the corner scaled the image: {w0}x{h0} -> {w1}x{h1}");
        assert_eq!(kept, shift, "Shift {shift}: {w0}x{h0} -> {w1}x{h1}");
    }
}

#[test]
fn in_auto_layout_a_side_dragged_past_the_image_leaves_no_empty_strip() {
    let mut s = Session::new();
    let image = image::RgbaImage::from_fn(300, 200, |x, y| image::Rgba([(x % 255) as u8, (y % 255) as u8, 120, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let content = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 1092, "height": 1400})).unwrap());
    id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 160, "parent": content})).unwrap());
    let layer = id(&s.execute("image.place", &json!({"data": data, "x": 0, "y": 300, "width": 804, "height": 969})).unwrap());
    s.execute("node.reparent", &json!({"ids": [layer], "parent": content})).unwrap();
    s.execute("node.set", &json!({"ids": [content], "props": {"layout": "vertical", "sizingV": "hug", "padding": [112, 0, 100, 0]}})).unwrap();
    s.execute("node.set", &json!({"ids": [layer], "props": {"constrainProportions": true}})).unwrap();
    s.execute("select.set", &json!({"ids": [layer]})).unwrap();
    let mut u = Ui::new(s);
    u.app.run("view.set", json!({"zoom": 0.5, "cx": 546, "cy": 700}));
    u.app.run("image.crop", json!({}));
    u.frame(vec![]);
    let b = u.app.session.doc().unwrap().world_bounds(layer).unwrap();
    let (from, to) = (u.app.to_screen(400.0, b.y0), u.app.to_screen(400.0, b.y0 - 120.0));
    u.drag_to(from, to);
    u.button(to, false);
    let n = u.app.session.doc().unwrap().node(layer).unwrap().clone();
    let (_, Some(c)) = mode(&u, layer) else { panic!("a crop") };
    let (x0, y0, x1, y1) = figcraft_color::ImagePaint::crop_rect(c);
    assert!(x0 <= 1e-9 && y0 <= 1e-9 && x1 >= 1.0 - 1e-9 && y1 >= 1.0 - 1e-9, "the image covers the layer: {:?}", (x0, y0, x1, y1));
    assert!(n.width >= 1.0 && n.height >= 1.0, "{} x {}", n.width, n.height);
    assert!(u.app.session.ds().unwrap().vector_edit.is_none());
}
