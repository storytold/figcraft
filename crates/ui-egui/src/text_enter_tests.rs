//! Enter, Escape and Tab on text, and the page a new file gets in each theme, driven through egui
//! headless: Enter on a selected text layer edits it with all of its text selected (what a
//! double-click into it does), typing replaces it, and Escape leaves editing with the layer still
//! selected; Tab while editing goes on to the next text layer; File ▸ New in the dark theme makes
//! a dark page.

use egui::{Event, Key, Modifiers};
use figcraft_color::Color;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn press(u: &mut Ui, key: Key) {
    press_with(u, key, Modifiers::NONE);
}

fn press_with(u: &mut Ui, key: Key, modifiers: Modifiers) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn text(u: &Ui, id: NodeId) -> String {
    match &u.app.session.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Text(t) => t.characters.clone(),
        _ => panic!("not text"),
    }
}

fn page_colour(u: &Ui) -> Color {
    let ds = u.app.session.ds().unwrap();
    match ds.doc.node(ds.page).unwrap().kind {
        NodeKind::Page { background, .. } => background,
        _ => panic!("not a page"),
    }
}

#[test]
fn enter_edits_a_selected_text_with_all_of_it_selected_and_escape_keeps_the_layer_selected() {
    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "Hello world", "x": 20, "y": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    let mut u = Ui::new(s);
    press(&mut u, Key::Enter);
    let te = u.app.session.ds().unwrap().text_edit.clone().expect("Enter starts editing");
    assert_eq!((te.id, te.anchor.min(te.caret), te.anchor.max(te.caret)), (t, 0, 11), "all of the text is selected");
    // Typing replaces the selection, as after a double-click into the text.
    u.frame(vec![Event::Text("Bye".into())]);
    assert_eq!(text(&u, t), "Bye");
    press(&mut u, Key::Escape);
    let ds = u.app.session.ds().unwrap();
    assert!(ds.text_edit.is_none(), "Escape leaves editing");
    assert_eq!(ds.selection, vec![t], "the layer stays selected");
    assert_eq!(text(&u, t), "Bye", "what was typed is kept");
    // One undo step for the whole edit.
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&u, t), "Hello world");
    // A second Escape deselects, as it does for any layer.
    press(&mut u, Key::Escape);
    assert!(u.app.session.selection().is_empty());
}

/// The text layer inside instance `inst`.
fn label_of(s: &Session, inst: NodeId) -> NodeId {
    let doc = s.doc().unwrap();
    doc.subtree(inst).into_iter().find(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))).unwrap()
}

#[test]
fn tab_edits_the_label_in_the_next_row_of_a_form() {
    // A form: a vertical auto layout frame of instances of an option (a box and a label), each
    // label set on its instance.
    let mut s = Session::new();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    let option = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 160, "height": 24, "name": "Option"})).unwrap());
    s.execute("node.set", &json!({"id": option, "props": {"layout": "horizontal", "gap": 8}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 16, "height": 16, "parent": option})).unwrap();
    let main_label = id(s.execute("text.create", &json!({"text": "Label", "x": 0, "y": 0, "parent": option})).unwrap());
    let option = id(s.execute("component.create", &json!({"ids": [option]})).unwrap());
    let form = id(s.execute("frame.create", &json!({"x": 0, "y": 200, "width": 240, "height": 200, "name": "Form"})).unwrap());
    s.execute("node.set", &json!({"id": form, "props": {"layout": "vertical", "gap": 12}})).unwrap();
    let mut labels = Vec::new();
    for name in ["Instagram", "Architect", "Designer"] {
        let inst = id(s.execute("instance.create", &json!({"component": option, "parent": form})).unwrap());
        let label = label_of(&s, inst);
        s.execute("text.set", &json!({"id": label, "characters": name})).unwrap();
        labels.push(label);
    }
    s.execute("text.edit.begin", &json!({"id": labels[0], "caret": 9})).unwrap();
    let mut u = Ui::new(s);
    press(&mut u, Key::Tab);
    let te = u.app.session.ds().unwrap().text_edit.clone().expect("still editing");
    assert_eq!((te.id, te.anchor.min(te.caret), te.anchor.max(te.caret)), (labels[1], 0, 9), "Architect, all of it selected");
    u.frame(vec![Event::Text("Writer".into())]);
    assert_eq!(text(&u, labels[1]), "Writer", "typing replaces it");
    assert_eq!((text(&u, labels[0]), text(&u, main_label)), ("Instagram".into(), "Label".into()), "the others stay");
    press_with(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(u.app.session.ds().unwrap().text_edit.as_ref().map(|te| te.id), Some(labels[0]), "Shift+Tab: back to Instagram");
    // An override: the main's label changing leaves it as typed.
    press(&mut u, Key::Escape);
    u.app.session.execute("text.set", &json!({"id": main_label, "characters": "Choice"})).unwrap();
    assert_eq!(text(&u, labels[1]), "Writer", "kept as the instance's own text");
}

#[test]
fn copying_and_cutting_selected_text_use_clipboard_service() {
    use std::{cell::RefCell, rc::Rc};

    struct Clipboard(Rc<RefCell<Vec<String>>>);
    impl crate::Services for Clipboard {
        fn clipboard_set_text(&mut self, text: &str) {
            self.0.borrow_mut().push(text.to_string());
        }
    }

    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "Hello world", "x": 20, "y": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    let mut u = Ui::new(s);
    let copied = Rc::new(RefCell::new(Vec::new()));
    u.app.services = Box::new(Clipboard(copied.clone()));
    press(&mut u, Key::Enter);
    u.frame_output(vec![Event::Copy]);
    u.frame_output(vec![Event::Cut]);
    assert_eq!(&*copied.borrow(), &["Hello world", "Hello world"]);
    assert_eq!(u.app.session.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters, "");
}

#[test]
fn a_new_file_takes_the_page_colour_of_the_theme_and_keeps_it() {
    let mut s = Session::new();
    s.prefs.theme = "dark".into();
    let mut u = Ui::new(s);
    assert_eq!(page_colour(&u), Color::DARK_PAGE, "the file the app opened with, untouched");
    u.app.run("file.new", json!({}));
    assert_eq!(page_colour(&u), Color::DARK_PAGE);
    u.app.run("view.themeLight", json!({}));
    u.frame(vec![]);
    assert_eq!(page_colour(&u), Color::DARK_PAGE, "a change of theme leaves the page alone");
    u.app.run("file.new", json!({}));
    assert_eq!(page_colour(&u), Color::DEFAULT_PAGE, "a file made in the light theme");
    u.app.run("view.themeDark", json!({}));
    u.frame(vec![]);
    u.app.run("page.add", json!({}));
    assert_eq!(page_colour(&u), Color::DEFAULT_PAGE, "a page added in the dark theme follows its light page");
    u.app.run("file.new", json!({}));
    assert_eq!(page_colour(&u), Color::DARK_PAGE, "a file made in the dark theme");
}
