//! The colour picker's image well (#1682), as the help center describes it: choosing Image shows a
//! checkered placeholder and Upload from computer rather than a file dialog at once, and a file
//! dropped on the picker replaces the image, keeping the fill mode, crop and position.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use egui::{Event, Pos2, RawInput};
use figcraft_color::{ImageScaleMode, PaintKind};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use super::{PickerTarget, set_image_pending, window_id};
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::services::{ContentsFuture, DialogFuture};

fn png(r: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(8, 4, image::Rgba([r, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A dropped file whose contents arrive from memory, as in the browser (no file at its path).
#[derive(Debug)]
struct Dropped(std::path::PathBuf, Vec<u8>);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

/// A rectangle with an image fill in Fit mode, or a solid one.
fn layer(with_image: bool) -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 80, "height": 80, "props": {"fill": "#00ff00"}})).unwrap();
    let id = crate::present_tests::id(&r);
    if with_image {
        let data = base64::engine::general_purpose::STANDARD.encode(png(1));
        s.execute("image.place", &json!({"data": data, "target": id, "mode": "fit"})).unwrap();
    }
    (Ui::new(s), id)
}

fn open_picker(u: &mut Ui, id: NodeId, index: usize) -> PickerTarget {
    let target = PickerTarget::Paint { fills: true, index, ids: vec![id] };
    u.app.ui.picker = Some(target.clone());
    for _ in 0..4 {
        u.frame(vec![]);
    }
    target
}

/// Drop a PNG with the pointer at `at` (`None`: the platform doesn't say), and let it go through.
fn drop_png(u: &mut Ui, at: Option<Pos2>, bytes: Vec<u8>) {
    let events: Vec<Event> = at.map_or(vec![Event::PointerGone], |p| vec![Event::PointerMoved(p)]);
    u.frame_raw(RawInput { events: events.clone(), ..Default::default() });
    let file = Arc::new(Dropped(Path::new("figcraft-picker-drop").join("new.png"), bytes)) as egui::DroppedFileHandle;
    u.frame_raw(RawInput { events, dropped_files: vec![file], ..Default::default() });
    for _ in 0..6 {
        u.frame(vec![]);
    }
}

fn top_image(u: &Ui, id: NodeId, index: usize) -> figcraft_color::ImagePaint {
    let paint = u.app.session.doc().unwrap().node(id).unwrap().fills[index].clone();
    let PaintKind::Image(img) = paint.kind else { panic!("not an image: {paint:?}") };
    img
}

fn layers(u: &Ui) -> usize {
    u.app.session.doc().unwrap().node(u.app.session.page().unwrap()).unwrap().children.len()
}

#[test]
fn a_file_dropped_on_the_picker_replaces_the_image_and_keeps_its_settings() {
    let (mut u, id) = layer(true);
    let target = open_picker(&mut u, id, 1);
    let before = top_image(&u, id, 1);
    assert_eq!(before.scale_mode, ImageScaleMode::Fit);
    assert!(has_text(&flat(&u.frame_output(vec![]).shapes), "Upload from computer"));
    // Where the file is isn't known during the drag: the picker says it takes it.
    let hovered = vec![egui::HoveredFile { path: Some("new.png".into()), mime: String::new() }];
    let out = u.frame_raw(RawInput { events: vec![Event::PointerGone], hovered_files: hovered, ..Default::default() });
    assert!(has_text(&flat(&out.shapes), "Drop to use this image"));
    assert!(!has_text(&flat(&out.shapes), "Drop to import"));

    let count = layers(&u);
    drop_png(&mut u, None, png(2));
    let after = top_image(&u, id, 1);
    assert_ne!(after.image, before.image, "a new image");
    assert_eq!(after.scale_mode, ImageScaleMode::Fit, "the fill mode stays");
    assert_eq!(layers(&u), count, "no new layer");
    assert_eq!(u.app.ui.picker, Some(target), "the picker stays open");
    u.app.session.undo().unwrap();
    assert_eq!(top_image(&u, id, 1).image, before.image, "one undo step");
}

#[test]
fn choosing_image_shows_the_placeholder_until_a_file_arrives() {
    let (mut u, id) = layer(false);
    let target = open_picker(&mut u, id, 0);
    assert!(!has_text(&flat(&u.frame_output(vec![]).shapes), "Upload from computer"), "a solid paint has no well");
    let fills = |u: &Ui| u.app.session.doc().unwrap().node(id).unwrap().fills.clone();
    let before = fills(&u);
    // The Image tab, as its click does: nothing changes yet, and no dialog opens.
    set_image_pending(&u.ctx, Some(&target));
    assert!(has_text(&flat(&u.frame_output(vec![]).shapes), "Upload from computer"));
    assert_eq!(fills(&u), before);
    // A drop fills the placeholder: an image filling the layer, and the placeholder is done.
    drop_png(&mut u, None, png(3));
    assert_eq!(top_image(&u, id, 0).scale_mode, ImageScaleMode::Fill);
    u.frame(vec![]);
    assert!(!super::image_pending(&u.ctx, &target));
}

#[test]
fn a_drop_goes_to_the_canvas_when_the_picker_shows_no_image_or_the_pointer_is_elsewhere() {
    // A solid paint's picker takes nothing.
    let (mut u, id) = layer(false);
    open_picker(&mut u, id, 0);
    let count = layers(&u);
    drop_png(&mut u, None, png(4));
    assert_eq!(layers(&u), count + 1, "imported to the canvas");
    // Over the canvas, with the picker elsewhere: the canvas too.
    let (mut u, id) = layer(true);
    open_picker(&mut u, id, 1);
    let before = top_image(&u, id, 1).image;
    let count = layers(&u);
    let picker = u.ctx.memory(|m| m.area_rect(window_id())).unwrap();
    let canvas = u.app.canvas.rect;
    let at = Pos2::new(canvas.min.x + 20.0, canvas.min.y + 20.0);
    assert!(!picker.contains(at));
    drop_png(&mut u, Some(at), png(5));
    assert_eq!(layers(&u), count + 1);
    assert_eq!(top_image(&u, id, 1).image, before);
}

/// What the platform's picker does with Upload from computer, counting the times it is asked.
struct Picker {
    asked: Arc<AtomicUsize>,
    /// The browser: a name and contents; the desktop: a path. `None`: cancelled.
    file: Option<(PathBuf, Vec<u8>)>,
    browser: bool,
}

impl crate::Services for Picker {
    fn pick_open(&mut self, _title: &str, exts: &[&str]) -> DialogFuture {
        assert!(!self.browser, "a platform that picks contents is never asked for a path");
        assert!(exts.contains(&"png"), "{exts:?}");
        self.asked.fetch_add(1, Ordering::SeqCst);
        let path = self.file.as_ref().map(|f| f.0.clone());
        Box::pin(std::future::ready(path))
    }

    fn pick_open_contents(&mut self, _title: &str, exts: &[&str]) -> Option<ContentsFuture> {
        if !self.browser {
            return None;
        }
        assert!(exts.contains(&"png"), "{exts:?}");
        self.asked.fetch_add(1, Ordering::SeqCst);
        let file = self.file.as_ref().map(|(p, bytes)| Ok((p.to_string_lossy().into_owned(), bytes.clone())));
        Some(Box::pin(std::future::ready(file)))
    }
}

/// Put the picker in `u` that answers with `bytes` (`None`: cancelled), as the desktop or the
/// browser does; returns how many times it has been asked.
fn fake_picker(u: &mut Ui, bytes: Option<Vec<u8>>, browser: bool, dir: &Path) -> Arc<AtomicUsize> {
    let asked = Arc::new(AtomicUsize::new(0));
    let file = bytes.map(|b| {
        let path = dir.join("picked.png");
        std::fs::write(&path, &b).unwrap();
        (path, b)
    });
    u.app.services = Box::new(Picker { asked: asked.clone(), file, browser });
    asked
}

fn press(u: &mut Ui, at: Pos2) {
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        u.frame(vec![Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]);
    }
    for _ in 0..4 {
        u.frame(vec![]);
    }
}

/// Click the picker's Image tab: the sixth of its 30 wide cells, under the title.
fn click_image_tab(u: &mut Ui) {
    let picker = u.ctx.memory(|m| m.area_rect(window_id())).unwrap();
    press(u, picker.min + egui::vec2(177.0, 52.0));
}

fn click_upload(u: &mut Ui) {
    let at = text_center(&flat(&u.frame_output(vec![]).shapes), "Upload from computer");
    press(u, at);
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-upload-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The Image tab opens nothing; its button asks the platform, and the file becomes the paint.
#[test]
fn upload_from_computer_fills_the_placeholder_on_the_desktop_and_in_the_browser() {
    for browser in [false, true] {
        let dir = temp_dir(&format!("placeholder-{browser}"));
        let (mut u, id) = layer(false);
        open_picker(&mut u, id, 0);
        let asked = fake_picker(&mut u, Some(png(6)), browser, &dir);
        click_image_tab(&mut u);
        assert_eq!(asked.load(Ordering::SeqCst), 0, "the tab opens no dialog");
        click_upload(&mut u);
        assert_eq!(asked.load(Ordering::SeqCst), 1, "the button asks the platform once");
        assert_eq!(top_image(&u, id, 0).scale_mode, ImageScaleMode::Fill, "browser: {browser}");
        assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Upload on an existing image replaces it and keeps its fill mode and crop, one undo step.
#[test]
fn upload_from_computer_replaces_an_image_keeping_its_fill_mode_and_crop() {
    for browser in [false, true] {
        let dir = temp_dir(&format!("replace-{browser}"));
        let (mut u, id) = layer(true);
        u.app.session.execute("image.crop", &json!({"ids": [id], "index": 1})).unwrap();
        let target = PickerTarget::Paint { fills: true, index: 1, ids: vec![id] };
        u.app.ui.picker = Some(target);
        for _ in 0..4 {
            u.frame(vec![]);
        }
        let before = top_image(&u, id, 1);
        assert_eq!(before.scale_mode, ImageScaleMode::Crop);
        let (asked, steps) = (fake_picker(&mut u, Some(png(7)), browser, &dir), u.app.session.undo_depth().0);
        click_upload(&mut u);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        let after = top_image(&u, id, 1);
        assert_ne!(after.image, before.image, "browser: {browser}");
        assert_eq!((after.scale_mode, after.crop), (ImageScaleMode::Crop, before.crop));
        assert_eq!(u.app.session.undo_depth().0, steps + 1, "one undo step");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Cancelling the picker changes nothing: no paint, no undo step, and the placeholder stays.
#[test]
fn cancelling_the_upload_changes_nothing() {
    let dir = temp_dir("cancel");
    for browser in [false, true] {
        let (mut u, id) = layer(false);
        open_picker(&mut u, id, 0);
        let asked = fake_picker(&mut u, None, browser, &dir);
        let fills = |u: &Ui| u.app.session.doc().unwrap().node(id).unwrap().fills.clone();
        let (before, steps) = (fills(&u), u.app.session.undo_depth());
        click_image_tab(&mut u);
        click_upload(&mut u);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(fills(&u), before, "browser: {browser}");
        assert_eq!(u.app.session.undo_depth(), steps);
        assert!(has_text(&flat(&u.frame_output(vec![]).shapes), "Upload from computer"), "the placeholder stays");

        let (mut u, id) = layer(true);
        open_picker(&mut u, id, 1);
        fake_picker(&mut u, None, browser, &dir);
        let (before, steps) = (top_image(&u, id, 1), u.app.session.undo_depth());
        click_upload(&mut u);
        assert_eq!(top_image(&u, id, 1), before);
        assert_eq!(u.app.session.undo_depth(), steps);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
