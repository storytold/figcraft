//! A plugin's `dialog.pluginSave` request: the browser downloads the file, a platform with save
//! dialogs asks the user (and writes nothing when they cancel).

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices, Services};

type Downloads = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Services that save by downloads and keep them (`deliver` false: the browser refused).
struct Browser {
    got: Downloads,
    deliver: bool,
}

impl Services for Browser {
    fn downloads(&self) -> bool {
        true
    }
    fn download(&mut self, name: &str, bytes: &[u8]) -> bool {
        self.got.lock().unwrap().push((name.to_string(), bytes.to_vec()));
        self.deliver
    }
}

fn request(app: &mut FigApp, name: &str, data: &str) {
    app.session.request_ui("dialog.pluginSave", json!({"plugin": "org.test.save", "name": name, "mime": "text/plain", "data": data}));
    app.handle_ui_requests();
}

fn toast(app: &FigApp) -> String {
    app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[test]
fn the_browser_downloads_the_file_with_its_exact_bytes() {
    let got = Downloads::default();
    let mut app = FigApp::new(Session::new(), Box::new(Browser { got: got.clone(), deliver: true }));
    request(&mut app, "out.jsx", &b64(b"\x00hello\xff"));
    let got = got.lock().unwrap();
    assert_eq!(got.as_slice(), [("out.jsx".to_string(), b"\x00hello\xff".to_vec())]);
    assert!(toast(&app).contains("out.jsx"), "{}", toast(&app));
}

#[test]
fn a_refused_download_and_broken_data_say_so() {
    let got = Downloads::default();
    let mut app = FigApp::new(Session::new(), Box::new(Browser { got: got.clone(), deliver: false }));
    request(&mut app, "out.jsx", &b64(b"x"));
    assert_eq!(toast(&app), "Couldn't save the file");
    app.ui.toast = None;
    request(&mut app, "out.jsx", "***not base64***");
    assert_eq!(toast(&app), "Couldn't save the file");
    assert_eq!(got.lock().unwrap().len(), 1, "broken data is never handed over");
}

#[test]
fn a_cancelled_save_dialog_writes_nothing() {
    // `NoServices` has no downloads and answers the save dialog with "cancelled".
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    request(&mut app, "out.jsx", &b64(b"x"));
    assert!(app.ui.toast.is_none());
}
