//! File > Export frames to PDF: the menu asks where to save and writes the frames' PDF there,
//! one visible top-level frame per page.

use std::path::PathBuf;

use figcraft_engine::Session;
use figcraft_ui_egui::{DialogFuture, FigApp, Services, menus};
use serde_json::json;

/// Services whose save dialog always returns the given path.
struct SaveTo(PathBuf);
impl Services for SaveTo {
    fn pick_save(&mut self, _title: &str, _name: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
}

/// The menu item, then a frame: the save dialog answers asynchronously and the frame takes its answer.
fn invoke(app: &mut FigApp, id: &str) {
    menus::invoke(app, id);
    let ctx = egui::Context::default();
    for _ in 0..2 {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }
}

fn out_path(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("figcraft-{}-{name}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn frame(app: &mut FigApp, x: i32, y: i32) -> serde_json::Value {
    app.session.execute("node.create", &json!({"type": "frame", "x": x, "y": y, "width": 100, "height": 80})).unwrap()
}

/// The number of `/Type /Page` objects (not `/Pages`) in a PDF.
fn page_count(pdf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(pdf).replace("/Type/Page", "/Type /Page");
    s.match_indices("/Type /Page").filter(|(i, _)| !s[i + 11..].starts_with('s')).count()
}

#[test]
fn frames_pdf_menu_saves_the_pdf() {
    let path = out_path("one");
    let mut app = FigApp::new(Session::new(), Box::new(SaveTo(path.clone())));
    frame(&mut app, 0, 0);
    invoke(&mut app, "file.exportFramesPdf");
    assert!(app.ui.last_error.is_none(), "{:?}", app.ui.last_error);
    let bytes = std::fs::read(&path).expect("the menu writes the PDF where the user chose");
    let _ = std::fs::remove_file(&path);
    assert!(bytes.starts_with(b"%PDF"));
    assert_eq!(page_count(&bytes), 1);

    // The command still returns the data when called without a destination.
    let value = app.session.execute("file.exportFramesPdf", &json!({})).unwrap();
    assert_eq!(value["format"], "pdf");
    assert_eq!(value["pages"], 1);
    assert!(value["data"].as_str().unwrap().starts_with("JVBERi0"));
}

#[test]
fn frames_pdf_menu_one_page_per_visible_frame() {
    let path = out_path("two");
    let mut app = FigApp::new(Session::new(), Box::new(SaveTo(path.clone())));
    frame(&mut app, 0, 0);
    frame(&mut app, 300, 0);
    let hidden = frame(&mut app, 600, 0);
    let id = hidden["id"].clone();
    app.session.execute("node.set", &json!({"id": id, "visible": false})).unwrap();
    invoke(&mut app, "file.exportFramesPdf");
    assert!(app.ui.last_error.is_none(), "{:?}", app.ui.last_error);
    let bytes = std::fs::read(&path).expect("the menu writes the PDF where the user chose");
    let _ = std::fs::remove_file(&path);
    assert_eq!(page_count(&bytes), 2);
}
