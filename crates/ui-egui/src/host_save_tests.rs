//! In a page host the browser hands files to the host: a Save of the document the host opened goes
//! out under the host's file name, a Save As under the document's own name. A host that confirms
//! what it stores leaves the document unsaved until it answers.

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

/// A host that answers later whether it stored a document file ([`crate::Delivery::Pending`]):
/// the files it was handed wait here; exports still arrive as plain downloads.
#[derive(Default)]
struct Confirming {
    documents: std::rc::Rc<std::cell::RefCell<Vec<(String, figcraft_engine::cmd::file::FileSnapshot)>>>,
    downloads: Saved,
}

impl Services for Confirming {
    fn downloads(&self) -> bool {
        true
    }
    fn download(&mut self, name: &str, _bytes: &[u8]) -> bool {
        self.downloads.lock().unwrap().push(name.to_string());
        true
    }
    fn deliver_document(&mut self, name: &str, _bytes: &[u8], file: figcraft_engine::cmd::file::FileSnapshot) -> crate::Delivery {
        self.documents.borrow_mut().push((name.to_string(), file));
        crate::Delivery::Pending
    }
}

type Documents = std::rc::Rc<std::cell::RefCell<Vec<(String, figcraft_engine::cmd::file::FileSnapshot)>>>;

fn confirming() -> (FigApp, Documents, Saved) {
    let host = Confirming::default();
    let (documents, downloads) = (host.documents.clone(), host.downloads.clone());
    let mut app = FigApp::new(Session::empty(), Box::new(host));
    app.session.execute("file.new", &json!({"name": "Original"})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    (app, documents, downloads)
}

fn toast(app: &FigApp) -> String {
    app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

#[test]
fn a_save_the_host_has_not_confirmed_keeps_the_unsaved_changes() {
    let (mut app, documents, downloads) = confirming();
    app.save_as_dialog();
    assert_eq!(documents.borrow().len(), 1, "the document file went to the host");
    assert!(downloads.lock().unwrap().is_empty(), "not as a plain download");
    assert!(app.session.ds().unwrap().is_dirty(), "nothing is saved until the host says so");
    assert!(!toast(&app).starts_with("Saved"), "the user isn't told it is saved yet: {}", toast(&app));
    let (name, file) = documents.borrow_mut().remove(0);
    app.document_stored(&name, file);
    assert!(!app.session.ds().unwrap().is_dirty());
    assert_eq!(toast(&app), "Saved Original.figcraft");
}

#[test]
fn edits_made_before_the_host_confirms_stay_unsaved() {
    let (mut app, documents, _) = confirming();
    app.save_as_dialog();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    let (name, file) = documents.borrow_mut().remove(0);
    app.document_stored(&name, file);
    assert!(app.session.ds().unwrap().is_dirty(), "the second rectangle never reached the host");
}

#[test]
fn a_save_the_host_did_not_store_stays_unsaved_and_says_why() {
    let (mut app, documents, _) = confirming();
    app.save_as_dialog();
    let (name, _file) = documents.borrow_mut().remove(0);
    app.document_not_stored(&name, "you can only view this file");
    assert!(app.session.ds().unwrap().is_dirty());
    let error = app.ui.last_error.clone().unwrap_or_default();
    assert!(error.contains("Original.figcraft") && error.contains("you can only view this file"), "{error}");
}

#[test]
fn exports_are_never_document_saves() {
    let (mut app, documents, downloads) = confirming();
    let id = app.session.ds().unwrap().selection[0];
    crate::export_panel::save_one(&mut app, &[id], &figcraft_doc::ExportSetting::default(), "Rectangle.png");
    assert!(documents.borrow().is_empty());
    assert_eq!(downloads.lock().unwrap().as_slice(), ["Rectangle.png"]);
    assert!(app.session.ds().unwrap().is_dirty(), "an export saves nothing");
}
