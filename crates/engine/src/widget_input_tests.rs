//! Widget fields and property menus (plugin API 4): `widget.setText` and `widget.menu` call the
//! plugin back as one undo step each, from commands and from the canvas (a double-click edits a
//! field like any text); values, items and descriptions from plugins and files are checked, and
//! junk is refused and rolled back. The test plugin outputs its input; on a field edit it draws
//! nothing (the host keeps the value), on `reset` it redraws without a menu.

use serde_json::{Value, json};

use crate::components::tests::{ex, idv, node};
use crate::plugins_tests::{install, undo_len};
use crate::tools::{PointerEvent, PointerKind};
use crate::widgets_tests::{W, err, state};
use crate::{Session, textedit};
use figcraft_doc::NodeId;

const ID: &str = "org.test.note";

fn manifest(id: &str, api: u32) -> Value {
    json!({"id": id, "name": "Note", "version": "1.0.0", "main": "n.wasm", "api": api, "widget": true, "permissions": {"access": "read"}})
}

/// The note widget: a label field, a clickable square and (unless `bare`) a menu.
fn look(bare: bool) -> Value {
    let root = json!({"type": "frame", "name": "Note", "layout": "vertical", "gap": 8, "padding": 10, "fill": "#FFFFFF", "children": [
        {"type": "text", "name": "Label", "field": "label", "text": "Label", "size": 14, "width": 120, "color": "#111111"},
        {"type": "rectangle", "target": "inc", "width": 20, "height": 20, "fill": "#0D99FF"},
    ]});
    if bare {
        return json!({"state": {"n": 0}, "root": root});
    }
    json!({"root": root, "menu": [
        {"item": "reset"},
        {"item": "big", "label": "Big", "type": "toggle", "checked": true},
        {"item": "color", "label": "Colour", "type": "color", "options": ["#ff0000", "#00FF00"], "value": "#FF0000"},
    ]})
}

/// The note plugin's module; `on_field` runs on a field edit (default: return without drawing).
fn note(on_field: &str) -> Vec<u8> {
    let mut w = W::default();
    let (field, reset) = (w.has(r#""field":"label""#), w.has(r#""menu":"reset""#));
    let (bare, full) = (w.render(&look(true)), w.render(&look(false)));
    let on_field = if on_field.is_empty() { "(return (i32.const 0))" } else { on_field };
    w.build(&format!("(if {field} (then {on_field})) (if {reset} (then {bare} (return (i32.const 0)))) {full}"))
}

fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    install(&mut s, manifest(ID, 4), &note(""));
    let w = idv(&ex(&mut s, "widget.insert", json!({"plugin": ID, "x": 40, "y": 40})));
    (s, w)
}

/// The text of widget `w`'s label field.
fn label(s: &Session, w: NodeId) -> String {
    let first = node(s, w).children[0];
    node(s, first).kind.text().unwrap().characters.clone()
}

#[test]
fn a_field_edit_reaches_the_state_the_layer_and_the_plugin_as_one_undo_step() {
    let (mut s, w) = session();
    let list = ex(&mut s, "widget.list", json!({}));
    let item = &list["widgets"][0];
    assert_eq!((item["fields"].clone(), item["targets"].clone()), (json!(["label"]), json!(["inc"])));
    assert_eq!(item["menu"][0], json!({"item": "reset", "label": "reset", "type": "action"}), "a label defaults to the item's name");
    assert_eq!(item["menu"][2]["options"], json!(["#FF0000", "#00FF00"]), "colours are normalised");
    let undo = undo_len(&s);
    // Tabs become spaces and other control characters are dropped.
    let inner = node(&s, w).children[1];
    let r = ex(&mut s, "widget.setText", json!({"id": inner, "field": "label", "value": "Hello\tworld\u{7}"}));
    assert_eq!(r["output"]["widget"]["event"], json!({"field": "label", "value": "Hello world"}));
    assert_eq!(r["output"]["widget"]["state"], json!({"label": "Hello world"}), "the plugin sees the value in the state too");
    assert_eq!((state(&mut s, w), label(&s, w), undo_len(&s)), (json!({"label": "Hello world"}), "Hello world".into(), undo + 1));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!((state(&mut s, w), label(&s, w)), (json!({}), "Label".into()));
    // Bad edits are refused and change nothing.
    for (p, why) in [
        (json!({"id": w, "field": "nope", "value": "x"}), "it has: label"),
        (json!({"id": w, "field": "label"}), "`value` required"),
        (json!({"id": w, "field": "label", "value": 5}), "`value` required"),
        (json!({"id": w, "field": "label", "value": "x".repeat(1001)}), "longer than 1000"),
        (json!({"id": w, "value": "x"}), "`field` required"),
        (json!({"id": 999_999, "field": "label", "value": "x"}), "isn't a widget"),
    ] {
        assert!(err(&mut s, "widget.setText", p.clone()).contains(why), "{p}");
    }
    assert_eq!((state(&mut s, w), undo_len(&s)), (json!({}), undo));
    assert_eq!(
        ex(&mut s, "widget.setText", json!({"id": w, "field": "label", "value": "x".repeat(1000)}))["state"]["label"].as_str().map(str::len),
        Some(1000)
    );
}

#[test]
fn a_runaway_or_failing_field_callback_rolls_the_value_back() {
    let mut s = Session::new();
    install(&mut s, manifest(ID, 4), &note("(loop $l (br $l))"));
    let w = idv(&ex(&mut s, "widget.insert", json!({"plugin": ID})));
    let undo = undo_len(&s);
    assert!(err(&mut s, "widget.setText", json!({"id": w, "field": "label", "value": "Hi", "fuel": 100_000})).contains("instruction budget"));
    assert_eq!((state(&mut s, w), label(&s, w), undo_len(&s)), (json!({}), "Label".into(), undo));
    // A plugin that returns an error.
    install(&mut s, manifest(ID, 4), &note("(return (i32.const 1))"));
    assert!(s.execute("widget.setText", &json!({"id": w, "field": "label", "value": "Hi"})).is_err());
    assert_eq!((state(&mut s, w), undo_len(&s)), (json!({}), undo));
}

#[test]
fn menu_items_call_the_plugin_back_with_their_values() {
    let (mut s, w) = session();
    let event = |s: &mut Session, p: Value| ex(s, "widget.menu", p)["output"]["widget"]["event"].clone();
    // A toggle flips by default, or takes the value given; a colour is matched in any case.
    assert_eq!(event(&mut s, json!({"id": w, "item": "big"})), json!({"menu": "big", "value": false}));
    assert_eq!(event(&mut s, json!({"id": w, "item": "big", "value": true})), json!({"menu": "big", "value": true}));
    assert_eq!(event(&mut s, json!({"id": w, "item": "color", "value": "#00ff00"})), json!({"menu": "color", "value": "#00FF00"}));
    let undo = undo_len(&s);
    for (p, why) in [
        (json!({"id": w, "item": "color", "value": "#0000FF"}), "must be one of #FF0000, #00FF00"),
        (json!({"id": w, "item": "color"}), "must be one of"),
        (json!({"id": w, "item": "color", "value": "#".repeat(5000)}), "must be one of"),
        (json!({"id": w, "item": "nope"}), "it has: reset, big, color"),
        (json!({"id": w}), "`item` required"),
    ] {
        assert!(err(&mut s, "widget.menu", p.clone()).contains(why), "{p}");
    }
    assert_eq!(undo_len(&s), undo);
    // An action; this one redraws without a menu, which then has nothing to choose.
    let r = ex(&mut s, "widget.menu", json!({"id": w, "item": "reset"}));
    assert_eq!(
        (r["output"]["widget"]["event"].clone(), r["state"].clone(), r["menu"].clone()),
        (json!({"menu": "reset", "value": null}), json!({"n": 0}), json!([]))
    );
    assert_eq!(undo_len(&s), undo + 1);
    assert!(err(&mut s, "widget.menu", json!({"id": w, "item": "reset"})).contains("no menu item"));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(crate::widgets::menu_of(s.doc().unwrap(), w).len(), 3, "undo brings the menu back");
}

#[test]
fn fields_and_menus_need_api_4_and_forged_menus_are_ignored() {
    let mut s = Session::new();
    install(&mut s, manifest("org.test.old", 3), &note(""));
    assert!(err(&mut s, "widget.insert", json!({"plugin": "org.test.old"})).contains("need plugin API 4"));
    assert!(ex(&mut s, "widget.list", json!({}))["widgets"].as_array().unwrap().is_empty(), "rolled back");
    // A file (or a peer) can carry anything in a widget's menu data: junk reads as no menu.
    let (mut s, w) = session();
    let deep = "[".repeat(100_000);
    for junk in ["not json", r#"[{"item": "has space"}]"#, r#"[{"item": "c", "type": "color"}]"#, "{}", deep.as_str()] {
        s.ds_mut().unwrap().doc_mut().get_mut(w).unwrap().plugin_data.insert("figcraft.widget.menu".into(), junk.to_string());
        assert!(crate::widgets::menu_of(s.doc().unwrap(), w).is_empty(), "{}", &junk[..junk.len().min(40)]);
        assert!(err(&mut s, "widget.menu", json!({"id": w, "item": "c"})).contains("no menu item"));
    }
}

#[test]
fn double_clicking_a_field_edits_it_on_the_canvas() {
    let (mut s, w) = session();
    let field = node(&s, w).children[0];
    // Without any font a text may have no size to click on.
    let Some(b) = s.doc().unwrap().world_bounds(field).filter(|b| b.width() > 1.0 && b.height() > 1.0) else { return };
    let p = b.center();
    let at = |kind| PointerEvent::at(kind, p.x, p.y, 1.0);
    s.pointer(&at(PointerKind::Down)).unwrap();
    s.pointer(&at(PointerKind::Up)).unwrap();
    assert_eq!(s.selection(), vec![w], "a click selects the widget");
    s.pointer(&at(PointerKind::Down)).unwrap();
    s.pointer(&at(PointerKind::DoubleClick)).unwrap();
    assert_eq!(s.ds().unwrap().text_edit.as_ref().map(|te| te.id), Some(field), "a double-click edits the field");
    let undo = undo_len(&s);
    // Ending without a change makes no undo step and doesn't call the plugin.
    textedit::end(&mut s).unwrap();
    assert_eq!((undo_len(&s), state(&mut s, w), s.selection()), (undo, json!({}), vec![w]));
    // Typing replaces the text (it's all selected); ending sends it to the plugin, one undo step.
    s.pointer(&at(PointerKind::Down)).unwrap();
    s.pointer(&at(PointerKind::DoubleClick)).unwrap();
    textedit::insert(&mut s, "Hi").unwrap();
    ex(&mut s, "text.edit.end", json!({}));
    assert_eq!((state(&mut s, w), label(&s, w), undo_len(&s)), (json!({"label": "Hi"}), "Hi".into(), undo + 1));
    assert_eq!((s.selection(), s.ds().unwrap().text_edit.is_none()), (vec![w], true));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(label(&s, w), "Label");
    // A failed callback leaves the field as it was and says why.
    ex(&mut s, "plugin.remove", json!({"id": ID}));
    s.pointer(&at(PointerKind::Down)).unwrap();
    s.pointer(&at(PointerKind::DoubleClick)).unwrap();
    textedit::insert(&mut s, "Lost").unwrap();
    textedit::end(&mut s).unwrap();
    assert_eq!(label(&s, w), "Label");
    assert!(s.message.as_deref().is_some_and(|m| m.contains("no plugin")), "{:?}", s.message);
}

#[test]
fn a_field_edit_syncs_in_a_live_session() {
    use crate::live_tests::{push, relay, serve_as, session as live};
    let (mut s, script) = live();
    install(&mut s, manifest(ID, 4), &note(""));
    s.execute("collab.host", &json!({})).unwrap();
    let (mut server, me, welcome) = serve_as(s.doc().unwrap(), crate::live::Access::Edit);
    push(&script, vec![welcome]);
    s.live_poll();
    let w = idv(&ex(&mut s, "widget.insert", json!({"plugin": ID})));
    ex(&mut s, "widget.setText", json!({"id": w, "field": "label", "value": "Shared"}));
    s.live_sync();
    let back = relay(&script, &mut server, me);
    push(&script, back);
    s.live_poll();
    let theirs = server.document().node(w).unwrap();
    assert_eq!(theirs.plugin_data.get("figcraft.widget.state").map(String::as_str), Some(r#"{"label":"Shared"}"#));
    assert!(theirs.plugin_data.contains_key("figcraft.widget.menu"), "the menu travels with the widget");
}
