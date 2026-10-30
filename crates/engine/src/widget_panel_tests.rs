//! A widget opens its plugin's panel (plugin API 5), as a Figma widget's event handler shows UI:
//! the module stays loaded while the panel is open, and the panel's callbacks get the widget's id
//! and state and can redraw it, one undo step each. An older plugin's `ui_show` from a widget is
//! ignored as before. The test plugin outputs its input; a click on `open` shows a panel, its
//! `keep` button redraws the widget, its `set` button redraws it and closes the panel.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{ex, idv, node};
use crate::plugins_tests::{install, undo_len};
use crate::widgets_tests::{W, err, state};
use figcraft_doc::NodeId;

const ID: &str = "org.test.dial";

fn manifest(api: u32) -> Value {
    json!({"id": ID, "name": "Dial", "version": "1.0.0", "main": "d.wasm", "api": api, "widget": true, "permissions": {"access": "read"}})
}

fn look(n: i64, label: &str) -> Value {
    json!({"state": {"n": n}, "root": {"type": "frame", "name": "Dial", "layout": "horizontal", "padding": 10, "fill": "#FFFFFF", "children": [
        {"type": "text", "name": "Label", "text": label, "size": 12, "color": "#111111"},
        {"type": "rectangle", "target": "open", "width": 20, "height": 20, "fill": "#0D99FF"},
    ]}})
}

fn dial() -> Vec<u8> {
    let mut w = W::default();
    let (set, keep, open) = (w.has(r#""control":"set""#), w.has(r#""control":"keep""#), w.has(r#""target":"open""#));
    let panel = w.show(&json!({"title": "Dial", "controls": [{"type": "button", "name": "set", "label": "Set"}, {"type": "button", "name": "keep", "label": "Keep"}]}));
    let (five, seven, one) = (w.render(&look(5, "five")), w.render(&look(7, "seven")), w.render(&look(1, "one")));
    w.build(&format!(
        "(if {set} (then {five} (call $ui_close) (return (i32.const 0))))
         (if {keep} (then {seven} (return (i32.const 0))))
         (if {open} (then {panel} (return (i32.const 0))))
         {one}"
    ))
}

fn session(api: u32) -> (Session, NodeId) {
    let mut s = Session::new();
    install(&mut s, manifest(api), &dial());
    let w = idv(&ex(&mut s, "widget.insert", json!({"plugin": ID, "x": 40, "y": 40})));
    (s, w)
}

fn label(s: &Session, w: NodeId) -> String {
    let first = node(s, w).children[0];
    node(s, first).kind.text().unwrap().characters.clone()
}

#[test]
fn a_widget_opens_a_panel_whose_callbacks_redraw_it() {
    let (mut s, w) = session(5);
    let steps = undo_len(&s);
    ex(&mut s, "widget.click", json!({"id": w, "target": "open"}));
    let ui = ex(&mut s, "plugin.ui", json!({"id": ID}));
    assert_eq!((ui["title"].clone(), ui["command"].clone()), (json!("Dial"), json!("widget")), "{ui}");
    assert_eq!(undo_len(&s), steps, "opening the panel changed nothing");

    // A button: the plugin gets the widget's id and state and redraws it; the panel stays.
    let r = ex(&mut s, "plugin.event", json!({"id": ID, "control": "keep"}));
    assert_eq!(r["output"]["widget"], json!({"id": w, "state": {"n": 1}}), "{r}");
    assert_eq!(r["output"]["event"]["control"], "keep");
    assert_eq!((label(&s, w), state(&mut s, w)), ("seven".into(), json!({"n": 7})));
    assert!(!r["panel"].is_null());
    assert_eq!(undo_len(&s), steps + 1, "one undo step");

    // Another redraws and closes it.
    let r = ex(&mut s, "plugin.event", json!({"id": ID, "control": "set"}));
    assert_eq!(r["output"]["widget"]["state"], json!({"n": 7}));
    assert_eq!(label(&s, w), "five");
    assert!(r["panel"].is_null() && ex(&mut s, "plugin.ui", json!({"id": ID})).is_null());
    assert_eq!(undo_len(&s), steps + 2);
    s.undo().unwrap();
    assert_eq!(label(&s, w), "seven");
    s.undo().unwrap();
    assert_eq!(label(&s, w), "one");
}

#[test]
fn a_removed_widget_keeps_its_panel_but_is_not_redrawn() {
    let (mut s, w) = session(5);
    ex(&mut s, "widget.click", json!({"id": w, "target": "open"}));
    ex(&mut s, "edit.delete", json!({"ids": [w]}));
    let steps = undo_len(&s);
    let e = err(&mut s, "plugin.event", json!({"id": ID, "control": "keep"}));
    assert!(e.contains("removed"), "{e}");
    assert_eq!(undo_len(&s), steps, "rolled back");
    assert!(!ex(&mut s, "plugin.ui", json!({"id": ID})).is_null(), "the panel stays for its own buttons");
    // Undoing the delete brings the widget back for the panel to redraw.
    s.undo().unwrap();
    ex(&mut s, "plugin.event", json!({"id": ID, "control": "keep"}));
    assert_eq!(label(&s, w), "seven");
}

#[test]
fn an_older_widget_plugin_opens_no_panel() {
    let (mut s, w) = session(4);
    ex(&mut s, "widget.click", json!({"id": w, "target": "open"}));
    assert!(ex(&mut s, "plugin.ui", json!({"id": ID})).is_null(), "API 4 ignores ui_show from a widget, as before");
    assert_eq!(label(&s, w), "one");
}
