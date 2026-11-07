//! Switching tabs: the canvas shows the image a file had when it was last on screen from the first
//! frame back, instead of a blank page (and, where renders are slow, a coarse preview) while the
//! file renders again.

use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::idle_tests::wait_for_render;
use crate::{FigApp, NoServices};

struct Harness {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Harness {
    fn new(names: &[&str]) -> Self {
        figcraft_text::scan_system_fonts();
        let mut s = Session::empty();
        for n in names {
            s.execute("file.new", &json!({"name": n})).unwrap();
            s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 120})).unwrap();
        }
        Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 }
    }

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

    fn switch(&mut self, name: &str) {
        let i = self.app.session.docs.iter().position(|d| d.doc.name == name).unwrap();
        self.app.run("doc.switch", json!({"index": i}));
    }

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
    assert_ne!(h.app.canvas.tex_of, Some(a.0));

    h.switch("A");
    // One frame, before the render of A asked for in it can have come back.
    h.frame();
    let c = &h.app.canvas;
    assert_eq!(c.tex_of, Some(a.0), "A's image is on screen at once");
    assert_eq!((c.tex_seq, c.texture.as_ref().map(egui::TextureHandle::id)), (a.1, a.2), "the image A had when it was last shown");
    assert!(!c.tex_coarse);
    assert!(c.kept.iter().any(|k| k.of.0 != a.0.0), "B's image is kept in turn");
    wait_for_render(&h.app);
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
    h.app.run("file.close", json!({"index": f, "force": true}));
    h.switch("A");
    h.frame();
    assert!(h.app.canvas.kept.iter().all(|k| k.of.0 != f_uid), "a closed file's image goes");
}
