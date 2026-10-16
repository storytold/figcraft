//! Text made by commands in the app (the interface reported its theme) is readable where it is
//! drawn, like text drawn with the Text tool: white on a dark page or a dark frame unless a fill is
//! named. A session without an interface (CLI, MCP, exports) keeps the model's black everywhere.

use figcraft_color::{Color, Paint};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn fills(s: &Session, id: &Value) -> Vec<Paint> {
    s.doc().unwrap().node(id_of(&id["id"]).unwrap()).unwrap().fills.clone()
}

/// A new file made with the interface in `theme` (None: no interface).
fn session(theme: Option<bool>) -> Session {
    let mut s = Session::empty();
    if let Some(dark) = theme {
        s.set_ui_dark(dark);
    }
    ex(&mut s, "file.new", json!({}));
    s
}

#[test]
fn text_made_by_commands_is_readable_on_its_page() {
    let (white, black) = (vec![Paint::solid(Color::WHITE)], vec![Paint::solid(Color::BLACK)]);
    // (the interface's theme, the page's colour set afterwards, what new text is filled with, why)
    let cases = [
        (Some(true), None, &white, "the app in the dark theme: a dark page, white text"),
        (Some(false), None, &black, "the app in the light theme: a light page, black text"),
        (Some(false), Some("#101010"), &white, "it is the page that counts, not the theme: a page made dark"),
        (Some(true), Some("#FFFFFF"), &black, "and a page made light in the dark theme"),
        (None, None, &black, "no interface: black"),
        (None, Some("#101010"), &black, "no interface, a dark page: still black, scripts make the same thing everywhere"),
    ];
    for (theme, page_color, want, why) in cases {
        let mut s = session(theme);
        if let Some(c) = page_color {
            ex(&mut s, "page.setBackground", json!({"color": c}));
        }
        let made = [
            ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 0})),
            ex(&mut s, "node.create", json!({"type": "text", "text": "Hi", "x": 0, "y": 40})),
            ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 80, "props": {"fontSize": 20}})),
        ];
        for m in &made {
            assert_eq!(&fills(&s, m), want, "{why}");
        }
        // A fill that is asked for is the fill.
        let red = ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 120, "props": {"fill": "#FF0000"}}));
        assert_eq!(fills(&s, &red), vec![Paint::solid(Color::from_hex("#FF0000").unwrap())], "{why}: a named fill");
        // Other layers keep their defaults.
        let rect = ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 200, "width": 10, "height": 10}));
        assert_ne!(fills(&s, &rect), white, "{why}: shapes are not text");
    }
}

#[test]
fn text_in_a_frame_is_readable_on_the_frame() {
    // #362: what the text is drawn on decides: the nearest layer around it with a solid fill (a
    // new frame's white too), else its page; made in one undo step.
    let (white, black) = (vec![Paint::solid(Color::WHITE)], vec![Paint::solid(Color::BLACK)]);
    let mut s = session(Some(true));
    let frame = |s: &mut Session, x: i64, parent: Option<NodeId>, fills: Option<Value>| -> NodeId {
        let mut p = json!({"x": x, "y": 0, "width": 200, "height": 100});
        if let Some(parent) = parent {
            p["parent"] = json!(parent);
        }
        let f = id_of(&ex(s, "frame.create", p)["id"]).unwrap();
        if let Some(fills) = fills {
            ex(s, "node.set", json!({"id": f, "props": {"fills": fills}}));
        }
        f
    };
    let light = frame(&mut s, 0, None, None);
    let dark = frame(&mut s, 300, None, Some(json!([{"type": "solid", "color": "#202020"}])));
    let bare = frame(&mut s, 600, None, Some(json!([])));
    let inner = frame(&mut s, 0, Some(light), Some(json!([])));
    let faint = frame(&mut s, 900, None, Some(json!([{"type": "solid", "color": "#FFFFFF", "opacity": 0.2}])));
    let stacked = frame(&mut s, 1200, None, Some(json!([{"type": "solid", "color": "#202020"}, {"type": "solid", "color": "#FFFFFF"}])));
    let cases = [
        (light, &black, "a white frame on a dark page: black text"),
        (dark, &white, "a dark frame: white"),
        (bare, &white, "a frame without a fill: the page decides"),
        (inner, &black, "a frame without a fill in a white one: the white one"),
        (faint, &white, "a faint fill lets the page show through: the page decides"),
        (stacked, &black, "the fill on top counts"),
    ];
    for (parent, want, why) in cases {
        let t = ex(&mut s, "text.create", json!({"text": "Hi", "x": 10, "y": 10, "parent": parent}));
        assert_eq!(&fills(&s, &t), want, "{why}");
    }
    // The Text tool reads it the same way.
    use crate::tools::{PointerEvent, PointerKind, ToolId};
    let at = s.doc().unwrap().world_bounds(light).unwrap().center();
    s.set_tool(ToolId::Text);
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::at(kind, at.x, at.y, 1.0)).unwrap();
    }
    let typed = s.ds().unwrap().text_edit.as_ref().unwrap().id;
    assert_eq!(s.doc().unwrap().node(typed).unwrap().fills, black, "the Text tool in a white frame");
    crate::textedit::end(&mut s).unwrap();
    let t = ex(&mut s, "text.create", json!({"text": "Hi", "x": 10, "y": 10, "parent": light}));
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(id_of(&t["id"]).unwrap()).is_none());
}
