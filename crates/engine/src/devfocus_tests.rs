//! Dev Mode focus view: one frame shown alone, ready-for-dev navigation, clicks and Escape.

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::*;
use serde_json::json;

fn frame(s: &mut Session, x: f64, name: &str) -> NodeId {
    idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 200, "height": 200, "name": name})))
}

fn focused(s: &Session) -> Option<NodeId> {
    s.ds().ok().and_then(crate::DocState::focused)
}

fn click(s: &mut Session, x: f64, y: f64) {
    for kind in [PointerKind::Down, PointerKind::Up] {
        let ev = PointerEvent { kind, x, y, sx: 0.0, sy: 0.0, shift: false, alt: false, cmd: false, ctrl: false, zoom: 1.0, pressure: None };
        s.pointer(&ev).unwrap();
    }
}

#[test]
fn focus_shows_one_frame_and_steps_through_ready_ones() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0, "Login");
    let b = frame(&mut s, 300.0, "Signup");
    let plain = frame(&mut s, 600.0, "Draft");
    let pg2 = idv(&ex(&mut s, "page.add", json!({"name": "Settings"})));
    ex(&mut s, "page.select", json!({"id": pg2}));
    let c = frame(&mut s, 0.0, "Profile");
    ex(&mut s, "node.set", json!({"ids": [c], "props": {"devStatus": "readyForDev"}}));
    ex(&mut s, "select.set", json!({"ids": [a]}));
    ex(&mut s, "node.set", json!({"ids": [a, b], "props": {"devStatus": "readyForDev"}}));
    // Any top-level frame can be focused; the view fits it and selects it.
    let v = ex(&mut s, "dev.focus", json!({"id": plain}));
    assert_eq!((v["id"].clone(), v["index"].clone(), v["count"].clone()), (json!(plain), json!(null), json!(3)));
    assert_eq!(focused(&s), Some(plain));
    assert_eq!(s.selection(), vec![plain]);
    let view = s.ds().unwrap().view.clone();
    assert!((view.cx - 700.0).abs() < 1e-6, "centred on the frame");
    assert!((view.cy - (100.0 - 28.0 / view.zoom)).abs() < 1e-6, "below the focus view's bar");
    // Next / previous go through the ready-for-dev frames across pages, wrapping around.
    assert_eq!(ex(&mut s, "dev.focus", json!({"direction": "next"}))["id"], json!(a), "from outside the list: the first");
    assert_eq!(ex(&mut s, "dev.focus", json!({"direction": "next"}))["id"], json!(b));
    let v = ex(&mut s, "dev.focus", json!({"direction": "next"}));
    assert_eq!((v["id"].clone(), v["page"].clone()), (json!(c), json!(pg2)), "on to the next page");
    assert_eq!(s.page().unwrap(), pg2);
    assert_eq!(ex(&mut s, "dev.focus", json!({"direction": "next"}))["id"], json!(a), "wraps");
    assert_eq!(ex(&mut s, "dev.focus", json!({"direction": "previous"}))["id"], json!(c));
    // Leaving the page or deleting the frame ends it; dev.unfocus does too.
    let first = s.doc().unwrap().pages[0];
    ex(&mut s, "page.select", json!({"id": first}));
    assert_eq!(focused(&s), None);
    ex(&mut s, "dev.focus", json!({"id": a}));
    ex(&mut s, "edit.delete", json!({"ids": [a]}));
    assert_eq!(focused(&s), None);
    ex(&mut s, "dev.focus", json!({"id": b}));
    assert_eq!(ex(&mut s, "dev.unfocus", json!({})), json!({"focused": false}));
    assert_eq!(focused(&s), None);
    // Refused: layers inside frames, pages, nothing selected, nothing ready.
    let inner = idv(&ex(&mut s, "shape.rectangle", json!({"x": 310, "y": 10, "width": 10, "height": 10, "parent": b})));
    let page = s.page().unwrap();
    ex(&mut s, "select.set", json!({"ids": []}));
    for p in [json!({"id": inner}), json!({"id": page}), json!({}), json!({"direction": "up"})] {
        assert!(s.execute("dev.focus", &p).is_err(), "{p}");
    }
    let mut empty = Session::new();
    assert!(empty.execute("dev.focus", &json!({"direction": "next"})).is_err());
}

#[test]
fn only_the_focused_frame_is_hit_and_escape_leaves() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0, "A");
    let b = frame(&mut s, 300.0, "B");
    let in_a = idv(&ex(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 50, "height": 50, "parent": a})));
    // A layer on top of the focused frame, outside it in the tree.
    let over = idv(&ex(&mut s, "shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 80})));
    ex(&mut s, "dev.focus", json!({"id": a}));
    click(&mut s, 350.0, 100.0);
    assert!(s.selection().is_empty(), "frame B isn't hit");
    click(&mut s, 75.0, 75.0);
    assert_eq!(s.selection(), vec![in_a], "the layer on top of it isn't hit either");
    assert_ne!(s.selection(), vec![over]);
    // Escape: selection up to the frame, then out of the focus view.
    s.escape().unwrap();
    assert_eq!(s.selection(), vec![a]);
    assert_eq!(focused(&s), Some(a));
    s.escape().unwrap();
    assert_eq!(focused(&s), None, "Escape with the frame selected leaves");
    click(&mut s, 350.0, 100.0);
    assert_eq!(s.selection(), vec![b], "everything is hit again");
}

#[test]
fn nested_frames_stay_out_of_the_ready_list_and_its_navigation() {
    let mut s = Session::new();
    let screen = frame(&mut s, 0.0, "Screen");
    let card = idv(&ex(&mut s, "frame.create", json!({"parent": screen, "width": 100, "height": 50, "name": "Card"})));
    // A status a nested frame already stores (an older file, or set through node.set) is ignored.
    ex(&mut s, "node.set", json!({"id": card, "props": {"devStatus": "readyForDev"}}));
    assert_eq!(ex(&mut s, "inspect.readyForDev", json!({"status": "any"}))["items"], json!([]));
    assert!(s.execute("dev.focus", &json!({"direction": "next"})).is_err(), "nothing is ready");
    ex(&mut s, "node.set", json!({"id": screen, "props": {"devStatus": "readyForDev"}}));
    let ids = |v: serde_json::Value| -> Vec<serde_json::Value> {
        v["items"].as_array().map(|a| a.iter().map(|i| i["id"].clone()).collect()).unwrap_or_default()
    };
    assert_eq!(ids(ex(&mut s, "inspect.readyForDev", json!({}))), vec![json!(screen)]);
    ex(&mut s, "dev.focus", json!({"id": screen}));
    for direction in ["next", "previous"] {
        let v = ex(&mut s, "dev.focus", json!({"direction": direction}));
        assert_eq!((v["id"].clone(), v["count"].clone()), (json!(screen), json!(1)), "{direction} wraps to the only ready frame");
    }
    // A frame directly inside a section still counts.
    let sec = idv(&ex(&mut s, "node.create", json!({"type": "section", "x": 1000, "y": 0, "width": 400, "height": 300})));
    let in_sec = idv(&ex(&mut s, "frame.create", json!({"parent": sec, "width": 100, "height": 100, "name": "In section"})));
    ex(&mut s, "node.set", json!({"ids": [sec, in_sec], "props": {"devStatus": "readyForDev"}}));
    assert_eq!(ids(ex(&mut s, "inspect.readyForDev", json!({}))), vec![json!(screen), json!(sec), json!(in_sec)]);
}
