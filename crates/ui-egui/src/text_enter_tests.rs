//! Enter and Escape on text, and the page a new file gets in each theme, driven through egui
//! headless: Enter on a selected text layer edits it with all of its text selected (what a
//! double-click into it does), typing replaces it, and Escape leaves editing with the layer still
//! selected; File ▸ New in the dark theme makes a dark page.

use egui::{Event, Key, Modifiers};
use figcraft_color::Color;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn press(u: &mut Ui, key: Key) {
    let modifiers = Modifiers::NONE;
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
