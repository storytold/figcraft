//! A control request naming its `document` acts on that file, and the tab in front stays in front.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::{FigApp, NoServices};

/// Two files open: "Back" behind "Front", which is in front. Returns the app and Back's id.
fn two_files() -> (FigApp, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Front"})).unwrap();
    s.docs[0].doc_mut().name = "Back".into();
    let back = s.docs[0].uid;
    (FigApp::new(s, Box::new(NoServices)), back)
}

fn handle(app: &mut FigApp, method: &str, p: Value) -> Result<Value, String> {
    super::handle(app, &egui::Context::default(), method, &p)
}

fn layers(app: &FigApp, i: usize) -> usize {
    let ds = &app.session.docs[i];
    ds.doc.node(ds.page).map_or(0, |p| p.children.len())
}

#[test]
fn a_call_naming_the_back_file_edits_it_and_leaves_the_front_one_alone() {
    let (mut app, back) = two_files();
    let rect = json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 20, "height": 10}, "document": back});
    handle(&mut app, "engine.execute", rect).unwrap();
    assert_eq!((layers(&app, 0), layers(&app, 1)), (1, 0), "the rectangle landed in Back only");
    assert_eq!(app.session.active, 1, "Front stays in front");
    let doc = handle(&mut app, "document.inspect", json!({"depth": 1, "document": back})).unwrap();
    assert_eq!((doc["name"].as_str(), doc["id"].as_u64()), (Some("Back"), Some(back)));
    let listed = handle(&mut app, "engine.execute", json!({"command": "doc.list"})).unwrap();
    assert_eq!(listed[0]["id"], json!(back));
    assert!(handle(&mut app, "ui.render", json!({"document": back})).is_ok());
    // An open file's path names it too.
    app.session.docs[0].path = Some("/work/back.figcraft".into());
    let doc = handle(&mut app, "document.inspect", json!({"depth": 0, "document": "/work/back.figcraft"})).unwrap();
    assert_eq!(doc["id"], json!(back));
    assert_eq!(app.session.active, 1);
}

#[test]
fn a_file_that_is_not_open_or_a_ui_method_on_the_back_file_is_an_error() {
    let (mut app, back) = two_files();
    let e = handle(&mut app, "document.inspect", json!({"document": 999_999})).unwrap_err();
    assert!(e.contains("no open document 999999"), "{e}");
    let e = handle(&mut app, "ui.type", json!({"text": "x", "document": back})).unwrap_err();
    assert!(e.contains("acts on the tab in front"), "{e}");
    let front = app.session.docs[1].uid;
    assert!(handle(&mut app, "ui.inspect", json!({"document": front})).is_ok(), "the file in front may be named");
    assert_eq!(app.session.active, 1);
}

#[test]
fn a_command_that_opens_a_file_still_brings_it_to_the_front() {
    let (mut app, back) = two_files();
    handle(&mut app, "engine.execute", json!({"command": "file.new", "document": back})).unwrap();
    assert_eq!(app.session.active, 2, "a new file opens in front");
}
