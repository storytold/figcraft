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

struct HtmlAndImage(String);

impl Services for HtmlAndImage {
    fn clipboard_get_html(&mut self) -> Option<String> {
        Some(self.0.clone())
    }
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        Some((2, 2, vec![255; 16]))
    }
}

#[test]
fn editable_design_layers_take_priority_over_a_png_representation() {
    let mut session = Session::new();
    let selected = session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
    let mut app = FigApp::new(session, Box::new(HtmlAndImage(copied_html())));
    let before = app.session.doc().unwrap().node_count();

    crate::menus::paste(&mut app, None, None);

    assert_eq!(selected_names(&app), ["Copied layer"]);
    assert_eq!(app.session.doc().unwrap().node_count(), before + 1);
    assert!(matches!(
        app.session.doc().unwrap().node(figcraft_doc::NodeId(selected)).unwrap().fills[0].kind,
        figcraft_color::PaintKind::Solid { .. }
    ));
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
