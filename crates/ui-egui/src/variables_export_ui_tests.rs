//! Variables ▸ Export… where the platform saves by downloads (the browser, which has no save
//! dialogs): the export arrives as a download instead of doing nothing.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::variables_panel::export_dialog;
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

fn session() -> Session {
    let mut s = Session::new();
    for (cmd, p) in [
        ("variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]})),
        (
            "variable.create",
            json!({"collection": "Theme", "name": "color/primary", "type": "color", "values": {"Light": "#0000ff", "Dark": "#ffffff"}}),
        ),
        ("variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "value": 4})),
    ] {
        s.execute(cmd, &p).unwrap();
    }
    s
}

fn browser(deliver: bool) -> (FigApp, Downloads) {
    let got = Downloads::default();
    (FigApp::new(session(), Box::new(Browser { got: got.clone(), deliver })), got)
}

fn toast(app: &FigApp) -> String {
    app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

#[test]
fn json_export_downloads_what_variable_export_writes() {
    let (mut app, got) = browser(true);
    let expected = app.session.execute("variable.export", &json!({"format": "json"})).unwrap()["text"].as_str().unwrap().to_string();
    export_dialog(&mut app, "json");
    let got = got.lock().unwrap();
    let [(name, bytes)] = got.as_slice() else { panic!("one download, got {}", got.len()) };
    assert!(name.ends_with(".variables.json"), "{name}");
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), expected);
    let parsed: Value = serde_json::from_slice(bytes).unwrap();
    assert!(parsed.is_object());
    assert!(toast(&app).contains(name.as_str()), "{}", toast(&app));
}

#[test]
fn css_export_downloads_custom_properties() {
    let (mut app, got) = browser(true);
    export_dialog(&mut app, "css");
    let got = got.lock().unwrap();
    let [(name, bytes)] = got.as_slice() else { panic!("one download, got {}", got.len()) };
    assert!(name.ends_with(".variables.css"), "{name}");
    let css = String::from_utf8(bytes.clone()).unwrap();
    assert!(css.contains(":root") && css.contains("--"), "{css}");
}

#[test]
fn a_refused_download_says_so() {
    let (mut app, got) = browser(false);
    export_dialog(&mut app, "json");
    assert_eq!(got.lock().unwrap().len(), 1);
    assert_eq!(toast(&app), "Couldn't download the variables");
}

#[test]
fn platforms_with_save_dialogs_keep_them() {
    // No dialog answer (cancelled) and no downloads: nothing happens, as before.
    let mut app = FigApp::new(session(), Box::new(NoServices));
    export_dialog(&mut app, "json");
    assert!(app.ui.toast.is_none());
}
