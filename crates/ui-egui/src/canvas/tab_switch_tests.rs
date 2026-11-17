//! Switching tabs: the canvas shows the image a file had when it was last on screen from the first
//! frame back, instead of a blank page (and, where renders are slow, a coarse preview) while the
//! file renders again.

use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::idle_tests::wait_for_render;
use crate::{FigApp, NoServices};

/// The whole UI, driven headless, with files of their own names open.
struct Harness {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Harness {
    /// The UI with a file named after each of `names` (each with a rectangle); the last is shown.
    fn new(names: &[&str]) -> Self {
        figcraft_text::scan_system_fonts();
        let mut s = Session::empty();
        for n in names {
            s.execute("file.new", &json!({"name": n})).unwrap();
            s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 120})).unwrap();
        }
        Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 }
    }

    /// One frame with no input, 1/60 s after the last.
    fn frame(&mut self) {
        self.time += 1.0 / 60.0;
        self.app.clock.set_test_now(Some(self.time));
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    /// Frames until the canvas shows a refined image of the file and page on screen.
    fn settle(&mut self) {
        for _ in 0..200 {
            self.frame();
            wait_for_render(&self.app);
            let ds = self.app.session.ds().unwrap();
            let c = &self.app.canvas;
            if c.tex_of == Some((ds.uid, ds.page)) && !c.tex_coarse && c.texture.is_some() {
                return;
            }
        }
        panic!("the canvas never showed a refined image");
    }

    /// Show the file named `name`, as clicking its tab does.
    fn switch(&mut self, name: &str) {
        let i = self.app.session.docs.iter().position(|d| d.doc.name == name).unwrap();
        self.app.run("doc.switch", json!({"index": i}));
    }

    /// The file and page on screen.
    fn shown(&self) -> (u64, figcraft_doc::NodeId) {
        let ds = self.app.session.ds().unwrap();
        (ds.uid, ds.page)
    }
}

#[test]
fn switching_back_to_a_tab_shows_its_image_on_the_first_frame() {
    let mut h = Harness::new(&["A", "B"]);
    h.switch("A");
    h.settle();
    let a = (h.shown(), h.app.canvas.tex_seq, h.app.canvas.texture.as_ref().map(egui::TextureHandle::id));
    h.switch("B");
    h.settle();
    let b = (h.shown(), h.app.canvas.tex_seq);
    assert_ne!(h.app.canvas.tex_of, Some(a.0));

    // The image brought back is the one A had when it was last shown. Looked at without a frame:
    // in one, a render of A asked for in it could already have come back and replaced it.
    let c = &mut h.app.canvas;
    c.switch_texture(Some(a.0), |_| true);
    assert_eq!((c.tex_of, c.tex_seq, c.texture.as_ref().map(egui::TextureHandle::id)), (Some(a.0), a.1, a.2));
    c.switch_texture(Some(b.0), |_| true);
    assert_eq!((c.tex_of, c.tex_seq), (Some(b.0), b.1), "B's image was kept in turn");

    h.switch("A");
    h.frame();
    let c = &h.app.canvas;
    assert_eq!(c.tex_of, Some(a.0), "A's image is on screen at once");
    assert!(!c.tex_coarse, "and it is the refined one, not a preview");
    assert!(c.kept.iter().any(|k| k.of == b.0), "B's image is kept in turn");
    h.settle();
}

#[test]
fn switching_back_to_a_page_shows_its_image_on_the_first_frame() {
    let mut h = Harness::new(&["A"]);
    h.settle();
    let first = (h.shown(), h.app.canvas.tex_seq, h.app.canvas.texture.as_ref().map(egui::TextureHandle::id));
    h.app.run("page.add", json!({}));
    h.app.run("shape.ellipse", json!({"x": 40, "y": 40, "width": 120, "height": 120}));
    h.settle();
    let second = (h.shown(), h.app.canvas.tex_seq);
    assert_ne!(first.0, second.0);

    // Looked at without a frame, as for tabs: a render asked for in one could replace the image.
    let c = &mut h.app.canvas;
    c.switch_texture(Some(first.0), |_| true);
    assert_eq!((c.tex_of, c.tex_seq, c.texture.as_ref().map(egui::TextureHandle::id)), (Some(first.0), first.1, first.2));
    c.switch_texture(Some(second.0), |_| true);
    assert_eq!((c.tex_of, c.tex_seq), (Some(second.0), second.1), "the second page's image was kept in turn");

    h.app.run("page.select", json!({"index": 0}));
    h.frame();
    let c = &h.app.canvas;
    assert_eq!(c.tex_of, Some(first.0), "the first page's image is on screen at once");
    assert!(!c.tex_coarse);
    assert!(c.kept.iter().any(|k| k.of == second.0), "the second page's image is kept in turn");
    h.settle();
}

#[test]
fn kept_images_are_bounded_and_dropped_with_their_files() {
    let names = ["A", "B", "C", "D", "E", "F", "G"];
    let mut h = Harness::new(&names);
    for n in names {
        h.switch(n);
        h.settle();
    }
    assert_eq!(h.app.canvas.kept.len(), super::KEPT);

    let f = h.app.session.docs.iter().position(|d| d.doc.name == "F").unwrap();
    let f_uid = h.app.session.docs[f].uid;
    assert!(h.app.canvas.kept.iter().any(|k| k.of.0 == f_uid));
    // A tab closed in the background: the file shown stays the same.
    let before = h.shown();
    h.app.run("file.close", json!({"index": f, "force": true}));
    h.frame();
    assert_eq!(h.shown(), before);
    assert!(h.app.canvas.kept.iter().all(|k| k.of.0 != f_uid), "a closed file's image goes");
}

#[test]
fn closing_the_last_file_drops_its_kept_images() {
    let mut h = Harness::new(&["A", "B"]);
    h.switch("A");
    h.settle();
    h.switch("B");
    h.settle();
    assert_eq!(h.app.canvas.kept.len(), 1, "A's image is kept");
    // Both close before the next frame, which then shows Home, not the canvas.
    let a = h.app.session.docs.iter().position(|d| d.doc.name == "A").unwrap();
    h.app.run("file.close", json!({"index": a, "force": true}));
    h.app.run("file.close", json!({"index": 0, "force": true}));
    assert!(h.app.session.docs.is_empty());
    h.frame();
    assert!(h.app.canvas.kept.is_empty(), "no closed file's image stays behind Home");
}
