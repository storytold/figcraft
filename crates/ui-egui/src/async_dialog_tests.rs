use std::path::PathBuf;

use figcraft_engine::Session;

use crate::{DialogFuture, FigApp, Services};

struct OpenFile(PathBuf);

impl Services for OpenFile {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
}

#[test]
fn open_dialog_finishes_after_the_async_picker_returns() {
    let dir = std::env::temp_dir().join(format!("figcraft-async-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("opened.figcraft");
    let doc = figcraft_doc::Document::new("Opened");
    std::fs::write(&path, figcraft_format::save_bytes(&doc, None).unwrap()).unwrap();

    let mut app = FigApp::new(Session::empty(), Box::new(OpenFile(path.clone())));
    app.ui.home = true;
    app.open_dialog();
    assert!(app.session.docs.is_empty(), "opening waits for the picker result");

    app.poll_dialog(&egui::Context::default());
    assert_eq!(app.session.docs.len(), 1);
    assert_eq!(app.session.docs[0].path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert!(!app.ui.home);
    let _ = std::fs::remove_dir_all(dir);
}

/// Every picker answers `self.0`.
struct Picks(PathBuf);

impl Services for Picks {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
    fn pick_save(&mut self, _title: &str, _name: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(Some(self.0.clone())))
    }
}

/// Two files, A (the first) in front, and where the picker saves.
fn two_files(tag: &str) -> (FigApp, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("figcraft-async-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.figcraft");
    let mut app = FigApp::new(Session::empty(), Box::new(Picks(path.clone())));
    for _ in 0..2 {
        app.session.execute("file.new", &serde_json::json!({})).unwrap();
    }
    app.session.active = 0;
    (app, dir, path)
}

#[test]
fn save_as_saves_the_file_it_was_opened_for_after_another_tab_comes_to_the_front() {
    let (mut app, dir, path) = two_files("pin");
    app.save_as_dialog();
    // The user switches tabs while the dialog is open.
    app.session.active = 1;
    // Another window draws first: the result waits for the window holding the file.
    app.windows.current = 7;
    app.poll_dialog(&egui::Context::default());
    assert!(app.session.docs.iter().all(|d| d.path.is_none()));
    app.windows.current = 0;
    app.poll_dialog(&egui::Context::default());
    let p = path.to_string_lossy();
    assert_eq!(app.session.docs[0].path.as_deref(), Some(p.as_ref()), "A is saved where the user picked");
    assert_eq!(app.session.docs[1].path, None, "B is untouched");
    assert_eq!(app.session.active, 1, "the tab the user switched to stays in front");
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_dialog_whose_file_closed_meanwhile_does_nothing() {
    let (mut app, dir, path) = two_files("closed");
    app.save_as_dialog();
    app.session.execute("file.close", &serde_json::json!({"index": 0, "force": true})).unwrap();
    app.poll_dialog(&egui::Context::default());
    assert!(app.session.docs.iter().all(|d| d.path.is_none()), "B isn't saved in A's place");
    assert!(!path.exists());
    assert!(app.pending_dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}
