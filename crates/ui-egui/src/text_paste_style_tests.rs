//! Pasting into a text edit: characters FigCraft copied keep their formatting, while
//! Shift/Alt+Cmd/Ctrl+V and text from other apps take the style at the caret.
use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Key, Modifiers};
use figcraft_doc::{NodeId, TextData};
use figcraft_engine::{Session, textedit};
use serde_json::json;

use crate::layers_drag_tests::Ui;

struct Clipboard(Rc<RefCell<String>>);

impl crate::Services for Clipboard {
    fn clipboard_set_text(&mut self, text: &str) {
        *self.0.borrow_mut() = text.to_string();
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        Some(self.0.borrow().clone())
    }
}

fn text(u: &Ui, id: NodeId) -> TextData {
    u.app.session.doc().unwrap().node(id).unwrap().kind.text().unwrap().clone()
}

fn layer(s: &mut Session, chars: &str, size: f64, y: f64) -> NodeId {
    NodeId(s.execute("text.create", &json!({"text": chars, "x": 0, "y": y, "props": {"fontSize": size}})).unwrap()["id"].as_u64().unwrap())
}

fn copy_the_bold_word() -> (Ui, NodeId, Rc<RefCell<String>>) {
    let mut s = Session::new();
    let source = layer(&mut s, "Bold plain", 24.0, 0.0);
    let target = layer(&mut s, "Target", 12.0, 100.0);
    textedit::begin(&mut s, source, Some(0), false).unwrap();
    textedit::set_caret(&mut s, 4, true).unwrap();
    s.execute("text.bold", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let clipboard = Rc::new(RefCell::new(String::new()));
    u.app.services = Box::new(Clipboard(clipboard.clone()));
    u.frame(vec![Event::Copy]);
    textedit::end(&mut u.app.session).unwrap();
    textedit::begin(&mut u.app.session, target, Some(6), false).unwrap();
    (u, target, clipboard)
}

#[test]
fn pasting_copied_text_brings_its_formatting() {
    let (mut u, target, clipboard) = copy_the_bold_word();
    assert_eq!(*clipboard.borrow(), "Bold");
    u.frame(vec![Event::Paste("Bold".into())]);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetBold");
    assert_eq!((t.style_at(6).font_style.as_str(), t.style_at(6).font_size), ("Bold", 24.0), "the copied characters keep their look");
    assert_eq!((t.style_at(0).font_style.as_str(), t.style_at(0).font_size), ("Regular", 12.0), "the text around them keeps its own");
}

#[test]
fn alt_cmd_v_pastes_the_characters_in_the_style_at_the_caret() {
    let (mut u, target, _) = copy_the_bold_word();
    let held = Modifiers { alt: true, command: true, mac_cmd: true, ..Modifiers::NONE };
    let v = Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: true, repeat: false, modifiers: held };
    u.frame_with(vec![v], held);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetBold");
    assert_eq!((t.style_at(6).font_style.as_str(), t.style_at(6).font_size), ("Regular", 12.0), "the destination's look");
    assert!(t.runs.is_empty());
}

#[test]
fn text_copied_elsewhere_pastes_in_the_style_at_the_caret() {
    let (mut u, target, _) = copy_the_bold_word();
    u.frame(vec![Event::Paste("From a browser".into())]);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetFrom a browser");
    assert!(t.runs.is_empty(), "nothing FigCraft copied: plain text");
}

fn painted(s: &mut Session, id: NodeId, hex: &str) {
    let paint = figcraft_color::Paint::solid(figcraft_color::Color::from_hex(hex).unwrap());
    s.ds_mut().unwrap().doc_mut().get_mut(id).unwrap().fills = vec![paint];
}

fn color_at(u: &Ui, id: NodeId, i: usize) -> Option<figcraft_color::Color> {
    let n = u.app.session.doc().unwrap().node(id).unwrap();
    let fills = n.kind.text().unwrap().style_at(i).fills.clone().unwrap_or_else(|| n.fills.clone());
    fills.first().and_then(figcraft_color::Paint::solid_color)
}

fn red_source_and_black_target() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let source = layer(&mut s, "Red", 24.0, 0.0);
    let target = layer(&mut s, "Target", 12.0, 100.0);
    painted(&mut s, source, "#FF0000");
    painted(&mut s, target, "#000000");
    let mut u = Ui::new(s);
    u.app.services = Box::new(Clipboard(Rc::new(RefCell::new(String::new()))));
    (u, source, target)
}

#[test]
fn copied_characters_keep_the_colour_of_their_layer() {
    let (mut u, source, target) = red_source_and_black_target();
    textedit::begin(&mut u.app.session, source, Some(0), false).unwrap();
    textedit::set_caret(&mut u.app.session, 3, true).unwrap();
    u.frame(vec![Event::Copy]);
    textedit::end(&mut u.app.session).unwrap();
    textedit::begin(&mut u.app.session, target, Some(6), false).unwrap();
    u.frame(vec![Event::Paste("Red".into())]);
    assert_eq!(text(&u, target).characters, "TargetRed");
    assert_eq!(color_at(&u, target, 6), figcraft_color::Color::from_hex("#FF0000"), "the copied characters stay red");
    assert_eq!(color_at(&u, target, 0), figcraft_color::Color::from_hex("#000000"), "the target's own text stays black");
    assert_eq!(text(&u, target).style_at(6).font_size, 24.0);
}

#[test]
fn a_copied_text_layer_pastes_into_an_edit_with_its_look() {
    let (mut u, source, target) = red_source_and_black_target();
    u.app.session.execute("select.set", &json!({"ids": [source.0]})).unwrap();
    u.frame(vec![Event::Copy]);
    textedit::begin(&mut u.app.session, target, Some(6), false).unwrap();
    u.frame(vec![Event::Paste("Red".into())]);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetRed");
    assert_eq!(t.style_at(6).font_size, 24.0);
    assert_eq!(color_at(&u, target, 6), figcraft_color::Color::from_hex("#FF0000"));
    assert_eq!(t.style_at(0).font_size, 12.0);
}

#[test]
fn shift_cmd_v_pastes_the_characters_in_the_style_at_the_caret() {
    let (mut u, target, _) = copy_the_bold_word();
    let held = Modifiers { shift: true, command: true, mac_cmd: true, ..Modifiers::NONE };
    let v = Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: true, repeat: false, modifiers: held };
    u.frame_with(vec![v], held);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetBold");
    assert!(t.runs.is_empty(), "the destination's look");
}

#[test]
fn shift_insert_is_an_ordinary_paste_and_keeps_the_formatting() {
    let (mut u, target, _) = copy_the_bold_word();
    u.frame_with(vec![Event::Paste("Bold".into())], Modifiers::SHIFT);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetBold");
    assert_eq!(t.style_at(6).font_style, "Bold");
}

#[test]
fn the_chord_matches_the_style_though_its_keys_are_up_by_the_paste() {
    let (mut u, target, _) = copy_the_bold_word();
    let chord = Modifiers { shift: true, command: true, mac_cmd: true, ..Modifiers::NONE };
    let v = Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: true, repeat: false, modifiers: chord };
    u.frame_with(vec![v, Event::Paste("Bold".into())], Modifiers::NONE);
    let t = text(&u, target);
    assert_eq!(t.characters, "TargetBold");
    assert!(t.runs.is_empty(), "the destination's look");
}
