//! In a page host the browser hands files to the host: a Save of the document the host opened goes
//! out under the host's file name, a Save As under the document's own name.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, Services};

type Saved = Arc<Mutex<Vec<String>>>;

/// Services that save by downloads, with a document file the host opened.
struct Host {
    got: Saved,
    file: Option<&'static str>,
}

impl Services for Host {
    fn downloads(&self) -> bool {
        true
    }
    fn download(&mut self, name: &str, _bytes: &[u8]) -> bool {
        self.got.lock().unwrap().push(name.to_string());
        true
    }
    fn host_file(&self) -> Option<String> {
        self.file.map(str::to_string)
    }
}

fn app(file: Option<&'static str>) -> (FigApp, Saved) {
    let got = Saved::default();
    let mut app = FigApp::new(Session::empty(), Box::new(Host { got: got.clone(), file }));
    app.session.execute("file.new", &json!({"name": "Original"})).unwrap();
    (app, got)
}

#[test]
fn save_keeps_the_name_the_host_opened() {
    let (mut app, got) = app(Some("client.figcraft"));
    app.save();
    assert_eq!(got.lock().unwrap().as_slice(), ["client.figcraft"]);
}

#[test]
fn save_as_uses_the_document_s_own_name() {
    let (mut app, got) = app(Some("client.figcraft"));
    app.save_as_dialog();
    assert_eq!(got.lock().unwrap().as_slice(), ["Original.figcraft"]);
}

#[test]
fn without_a_host_file_or_for_a_copy_format_save_is_a_save_as() {
    for file in [None, Some("design.json"), Some("export.fig")] {
        let (mut app, got) = app(file);
        app.save();
        assert_eq!(got.lock().unwrap().as_slice(), ["Original.figcraft"], "{file:?}");
    }
}
