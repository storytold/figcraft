//! Save, Save as and Export where the platform saves by downloads (the browser, which has no save
//! dialogs or paths): the file arrives as a download instead of nothing happening (#969).

use base64::Engine as _;
use figcraft_doc::{ExportSetting, NodeId};
use figcraft_engine::Session;
use serde_json::json;

use crate::FigApp;
use crate::variables_export_ui_tests::{Browser, Downloads};

fn browser(deliver: bool) -> (FigApp, Downloads, NodeId) {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let id = serde_json::from_value(r["id"].clone()).unwrap();
    let got = Downloads::default();
    (FigApp::new(s, Box::new(Browser { got: got.clone(), deliver })), got, id)
}

fn toast(app: &FigApp) -> String {
    app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

#[test]
fn save_downloads_the_figcraft_file() {
    let (mut app, got, _) = browser(true);
    app.save();
    let got = got.lock().unwrap();
    let [(name, bytes)] = got.as_slice() else { panic!("one download, got {}", got.len()) };
    assert_eq!(name, "Untitled.figcraft");
    let mut reopened = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    reopened.execute("file.open", &json!({"data": data, "name": name})).unwrap();
    assert!(!app.session.ds().unwrap().is_dirty());
    assert_eq!(toast(&app), "Saved Untitled.figcraft");
}

#[test]
fn export_downloads_the_exported_file() {
    let (mut app, got, id) = browser(true);
    crate::export_panel::save_one(&mut app, &[id], &ExportSetting::default(), "Rectangle.png");
    let got = got.lock().unwrap();
    let [(name, bytes)] = got.as_slice() else { panic!("one download, got {}", got.len()) };
    assert_eq!(name, "Rectangle.png");
    assert!(bytes.starts_with(b"\x89PNG"));
    assert_eq!(toast(&app), "Exported Rectangle.png");
}

#[test]
fn a_refused_download_says_so() {
    let (mut app, got, _) = browser(false);
    app.save_as_dialog();
    assert_eq!(got.lock().unwrap().len(), 1);
    assert_eq!(toast(&app), "Couldn't download Untitled.figcraft");
    assert!(app.session.ds().unwrap().is_dirty(), "no file arrived: closing must still warn about the edits");
}

#[test]
fn several_layers_download_as_one_zip() {
    // The browser lets a page start one download at a time: two downloads lost the second.
    let (mut app, got, a) = browser(true);
    let r = app.session.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    let b: NodeId = serde_json::from_value(r["id"].clone()).unwrap();
    app.session.execute("export.add", &json!({"ids": [a, b]})).unwrap();
    crate::export_panel::export_layers(&mut app, &[a, b], false);
    let got = got.lock().unwrap();
    let [(name, bytes)] = got.as_slice() else { panic!("one download, got {}", got.len()) };
    assert_eq!(name, "Untitled.zip");
    assert!(bytes.starts_with(b"PK\x03\x04"));
    assert!(bytes.windows(15).any(|w| w == b"Rectangle 2.png"), "both layers are in the zip");
    assert_eq!(toast(&app), "Exported 2 files");
}
