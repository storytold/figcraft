use figcraft_engine::Session;
use figcraft_engine::cmd::edit::{CLIP_CLOSE, CLIP_OPEN};
use serde_json::json;

use crate::{FigApp, Services};

struct Clip {
    html: String,
    text: Option<String>,
}

impl Services for Clip {
    fn clipboard_get_html(&mut self) -> Option<String> {
        Some(self.html.clone())
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.text.clone()
    }
}

/// The HTML another design app puts on the clipboard for copied layers (a `.fig` canvas inside).
fn copied_html() -> String {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 40, "height": 20, "name": "Copied layer"})).unwrap();
    let r = s.execute("file.export", &json!({"format": "fig"})).unwrap();
    format!(r#"<span data-buffer="<!--{CLIP_OPEN}{}{CLIP_CLOSE}-->"></span>"#, r["data"].as_str().unwrap())
}

fn selected_names(app: &FigApp) -> Vec<String> {
    let doc = app.session.doc().unwrap();
    app.session.selection().iter().map(|i| doc.node(*i).unwrap().name.clone()).collect()
}

fn pasted_text(app: &FigApp) -> String {
    let doc = app.session.doc().unwrap();
    let id = app.session.selection()[0];
    doc.node(id).unwrap().kind.text().unwrap().characters.clone()
}

#[test]
fn paste_takes_layers_copied_in_another_design_app() {
    let html = copied_html();
    let mut app = FigApp::new(Session::new(), Box::new(Clip { html, text: Some(String::new()) }));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(selected_names(&app), ["Copied layer"]);
}

#[test]
fn paste_here_puts_copied_design_layers_at_the_point() {
    let html = copied_html();
    let mut app = FigApp::new(Session::new(), Box::new(Clip { html, text: Some(String::new()) }));
    crate::menus::paste(&mut app, None, Some((300.0, 200.0)));
    let id = app.session.selection()[0];
    let b = app.session.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.x0, b.y0), (300.0, 200.0));
}

#[test]
fn html_from_other_apps_pastes_as_before() {
    let clip = Clip { html: "<b>Hello</b>".into(), text: Some("Hello".into()) };
    let mut app = FigApp::new(Session::new(), Box::new(clip));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(pasted_text(&app), "Hello");
}

#[test]
fn html_that_only_mentions_the_marker_pastes_its_text() {
    let text = format!("Markers {CLIP_OPEN} and {CLIP_CLOSE} explained");
    let clip = Clip { html: format!("<p>{text}</p>"), text: Some(text.clone()) };
    let mut app = FigApp::new(Session::new(), Box::new(clip));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(pasted_text(&app), text);
    assert!(app.ui.last_error.is_none(), "{:?}", app.ui.last_error);
}

/// A clipboard whose contents the test replaces between pastes.
struct SharedClip(std::rc::Rc<std::cell::RefCell<Clip>>);

impl Services for SharedClip {
    fn clipboard_get_html(&mut self) -> Option<String> {
        Some(self.0.borrow().html.clone())
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.0.borrow().text.clone()
    }
}

#[test]
fn text_copied_after_a_design_paste_pastes_as_text() {
    let clip = std::rc::Rc::new(std::cell::RefCell::new(Clip { html: copied_html(), text: None }));
    let mut app = FigApp::new(Session::new(), Box::new(SharedClip(clip.clone())));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(selected_names(&app), ["Copied layer"]);
    *clip.borrow_mut() = Clip { html: String::new(), text: Some("Fresh text".into()) };
    crate::menus::paste(&mut app, None, None);
    assert_eq!(pasted_text(&app), "Fresh text");
}

/// A clipboard that keeps what the app writes.
#[derive(Default)]
struct Written {
    html: Option<String>,
    text: Option<String>,
}

struct WritingClip(std::rc::Rc<std::cell::RefCell<Written>>);

impl Services for WritingClip {
    fn clipboard_set_text(&mut self, s: &str) {
        *self.0.borrow_mut() = Written { html: None, text: Some(s.to_string()) };
    }
    fn clipboard_set_html(&mut self, html: &str, text: &str) {
        *self.0.borrow_mut() = Written { html: Some(html.to_string()), text: Some(text.to_string()) };
    }
    fn clipboard_get_html(&mut self) -> Option<String> {
        self.0.borrow().html.clone()
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.0.borrow().text.clone()
    }
}

#[test]
fn copy_puts_design_layers_on_the_clipboard_and_our_paste_keeps_its_own_copy() {
    let clip = std::rc::Rc::new(std::cell::RefCell::new(Written::default()));
    let mut app = FigApp::new(Session::new(), Box::new(WritingClip(clip.clone())));
    let r = app.session.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 40, "height": 20, "name": "Mine"})).unwrap();
    app.session.execute("select.set", &json!({"ids": [r["id"].clone()]})).unwrap();
    crate::menus::invoke(&mut app, "edit.copy");
    let html = clip.borrow().html.clone().expect("HTML on the clipboard");
    assert!(figcraft_engine::cmd::edit::design_clipboard_buffer(&html).is_some());
    assert!(clip.borrow().text.as_deref().is_some_and(figcraft_engine::looks_like_svg), "SVG for apps that read text");
    // Our paste takes our copied layers, not a round trip through the design data.
    let before = app.session.journal.len();
    crate::menus::paste(&mut app, None, None);
    assert_eq!(selected_names(&app), ["Mine"]);
    assert!(app.session.journal[before..].iter().all(|(c, _)| c != "edit.pasteDesignLayers"));
}
