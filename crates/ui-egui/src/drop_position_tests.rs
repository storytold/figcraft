//! Where a file dropped on the canvas lands. A dragged file from another app comes with no pointer
//! position on macOS until the window has had the pointer (a quick drop from Finder): the file then
//! lands in the middle of the view, where it can be seen, instead of at the page's 0, 0.

use egui::{Event, RawInput};
use figcraft_engine::Session;

use crate::devmode_tail_tests::DroppedBytes;
use crate::layers_drag_tests::Ui;

/// A 4×4 red PNG.
fn png() -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Drops `Photo.png` on the window with `events` in the same frame, then lets the import run.
/// The imported layer's page position.
fn drop_photo(u: &mut Ui, events: Vec<Event>) -> (f64, f64) {
    let file = std::sync::Arc::new(DroppedBytes("figcraft-drop-position/Photo.png".into(), png())) as egui::DroppedFileHandle;
    u.frame_raw(RawInput { events, dropped_files: vec![file], ..Default::default() });
    for _ in 0..4 {
        u.frame(vec![]);
    }
    let doc = u.app.session.doc().unwrap();
    let page = doc.node(u.app.session.page().unwrap()).unwrap();
    let id = *page.children.last().expect("the photo is imported");
    let b = doc.world_bounds(id).unwrap();
    (b.x0, b.y0)
}

#[test]
fn a_file_dropped_with_no_pointer_position_lands_in_the_middle_of_the_view() {
    let mut u = Ui::new(Session::new());
    u.app.session.ds_mut().unwrap().view.cx = 5000.0;
    u.app.session.ds_mut().unwrap().view.cy = 3000.0;
    u.frame(vec![Event::PointerGone]);
    let middle = u.app.to_page(u.app.canvas.rect.center());
    assert_eq!(drop_photo(&mut u, vec![]), (middle.x.round(), middle.y.round()), "not at the page's 0, 0, out of view");

    // With the pointer over the canvas, the file lands under it.
    let at = u.app.canvas.rect.min + egui::vec2(100.0, 80.0);
    let under = u.app.to_page(at);
    assert_eq!(drop_photo(&mut u, vec![Event::PointerMoved(at)]), (under.x.round(), under.y.round()));
}
