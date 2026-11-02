//! Widgets (plugin API 3): inserted and clicked as one undo step each (from commands and from the
//! canvas), kept through undo, save and reload, duplicating, export and live sessions; junk
//! descriptions, bad clicks, runaway callbacks and forged widget data are refused and roll back.
//! Test plugins are WAT: they output their input, then draw the widget the input asks for.

use serde_json::{Value, json};

use crate::components::tests::{ex, idv, node};
use crate::plugins_tests::{install, layer_count, lit, rect, undo_len};
use crate::tools::{Cursor, PointerEvent, PointerKind};
use crate::{EngineError, Session};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

const INPUT: usize = 100_000;
const REPLY: usize = 200_000;

/// Lays out a test plugin's strings and builds its module.
#[derive(Default)]
pub(crate) struct W {
    data: String,
    at: usize,
}

impl W {
    pub(crate) fn s(&mut self, text: &str) -> (usize, usize) {
        let a = 1000 + self.at;
        self.data += &format!("(data (i32.const {a}) \"{}\")\n", lit(text));
        self.at += text.len();
        assert!(self.at < INPUT - 1000);
        (a, text.len())
    }

    /// Draw `v`; when it's refused, output why and fail.
    pub(crate) fn render(&mut self, v: &Value) -> String {
        let (a, l) = self.s(&v.to_string());
        format!(
            "(if (call $render (i32.const {a}) (i32.const {l})) (then (call $set_output (i32.const {REPLY}) (call $reply_read (i32.const {REPLY}) (i32.const 50000))) (return (i32.const 1))))"
        )
    }

    /// Show the panel `spec` (API 2 panels; from a widget, API 5).
    pub(crate) fn show(&mut self, spec: &Value) -> String {
        let (a, l) = self.s(&spec.to_string());
        format!("(drop (call $ui_show (i32.const {a}) (i32.const {l})))")
    }

    /// Whether the input contains `needle`.
    pub(crate) fn has(&mut self, needle: &str) -> String {
        let (a, l) = self.s(needle);
        format!("(call $has (i32.const {a}) (i32.const {l}) (local.get $n))")
    }

    /// Call `cmd`; a refusal becomes the output and the run stops (successfully).
    pub(crate) fn call(&mut self, cmd: &str, params: &Value) -> String {
        let ((a, l), (b, m)) = (self.s(cmd), self.s(&params.to_string()));
        format!(
            "(if (call $call (i32.const {a}) (i32.const {l}) (i32.const {b}) (i32.const {m}))
               (then (call $set_output (i32.const {REPLY}) (call $reply_read (i32.const {REPLY}) (i32.const 50000))) (return (i32.const 0))))"
        )
    }

    /// The module: it reads and outputs its input, then runs `body`.
    pub(crate) fn build(self, body: &str) -> Vec<u8> {
        let wat = format!(
            r#"(module
              (import "figcraft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
              (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
              (import "figcraft" "set_output" (func $set_output (param i32 i32)))
              (import "figcraft" "widget_render" (func $render (param i32 i32) (result i32)))
              (import "figcraft" "ui_show" (func $ui_show (param i32 i32) (result i32)))
              (import "figcraft" "ui_close" (func $ui_close))
              (memory (export "memory") 4)
              {}
              (func $has (param $p i32) (param $l i32) (param $n i32) (result i32) (local $i i32) (local $j i32)
                (block $no (loop $outer
                  (br_if $no (i32.gt_s (i32.add (local.get $i) (local.get $l)) (local.get $n)))
                  (local.set $j (i32.const 0))
                  (block $miss (loop $inner
                    (if (i32.ge_s (local.get $j) (local.get $l)) (then (return (i32.const 1))))
                    (br_if $miss (i32.ne (i32.load8_u (i32.add (i32.const {INPUT}) (i32.add (local.get $i) (local.get $j))))
                                         (i32.load8_u (i32.add (local.get $p) (local.get $j)))))
                    (local.set $j (i32.add (local.get $j) (i32.const 1)))
                    (br $inner)))
                  (local.set $i (i32.add (local.get $i) (i32.const 1)))
                  (br $outer)))
                (i32.const 0))
              (func (export "figcraft_run") (param $n i32) (result i32)
                (drop (call $reply_read (i32.const {INPUT}) (local.get $n)))
                (call $set_output (i32.const {INPUT}) (local.get $n))
                {body}
                (i32.const 0)))"#,
            self.data
        );
        wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
    }
}

/// A widget showing `label` with the clickable parts `targets` (a rectangle, then ellipses).
fn look(n: i64, label: &str, targets: &[&str]) -> Value {
    let mut children = vec![json!({"type": "text", "name": "Label", "text": label, "size": 12, "color": "#111111"})];
    for (i, t) in targets.iter().enumerate() {
        let kind = if i == 0 { "rectangle" } else { "ellipse" };
        children.push(json!({"type": kind, "target": t, "width": 20, "height": 20, "fill": "#0D99FF"}));
    }
    json!({"state": {"n": n}, "root": {"type": "frame", "name": "Tally", "layout": "horizontal", "gap": 8, "padding": 10, "fill": "#FFFFFF", "children": children}})
}

/// The test widget: it shows 1 when inserted, 2 after `inc` and 0 after `dec`; `extra` runs
/// first on every call.
fn tally(extra: impl FnOnce(&mut W) -> String) -> Vec<u8> {
    let mut w = W::default();
    let pre = extra(&mut w);
    let (inc, dec) = (w.has(r#""target":"inc""#), w.has(r#""target":"dec""#));
    let (two, zero, one) = (w.render(&look(2, "two", &["inc", "dec"])), w.render(&look(0, "zero", &["inc"])), w.render(&look(1, "one", &["inc"])));
    w.build(&format!("{pre} (if {inc} (then {two} (return (i32.const 0)))) (if {dec} (then {zero} (return (i32.const 0)))) {one}"))
}

pub(crate) fn widget_manifest(id: &str, access: &str) -> Value {
    json!({"id": id, "name": "Tally", "version": "1.0.0", "main": "w.wasm", "api": 3, "widget": true, "permissions": {"access": access}})
}

/// A session with the tally widget plugin installed (read access).
pub(crate) fn session() -> Session {
    let mut s = Session::new();
    install(&mut s, widget_manifest("org.test.tally", "read"), &tally(|_| String::new()));
    s
}

pub(crate) fn insert(s: &mut Session) -> (NodeId, Value) {
    let r = ex(s, "widget.insert", json!({"plugin": "org.test.tally", "x": 50, "y": 60}));
    (idv(&r), r)
}

fn click(s: &mut Session, w: NodeId, target: &str) -> Value {
    ex(s, "widget.click", json!({"id": w, "target": target}))
}

pub(crate) fn state(s: &mut Session, w: NodeId) -> Value {
    let list = ex(s, "widget.list", json!({}));
    list["widgets"].as_array().unwrap().iter().find(|x| x["id"] == json!(w)).map(|x| x["state"].clone()).unwrap_or(Value::Null)
}

/// The text of widget `w`'s first layer.
fn label(s: &Session, w: NodeId) -> String {
    let first = node(s, w).children[0];
    node(s, first).kind.text().unwrap().characters.clone()
}

pub(crate) fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    match s.execute(cmd, &p) {
        Ok(v) => panic!("{cmd} accepted {p}: {v}"),
        Err(e) => {
            assert!(!matches!(e, EngineError::Internal { .. }), "{e}");
            e.to_string()
        }
    }
}

#[test]
fn a_widget_is_inserted_and_clicked_as_one_undo_step_each() {
    let mut s = session();
    let (w, r) = insert(&mut s);
    assert_eq!((r["state"].clone(), r["targets"].clone()), (json!({"n": 1}), json!(["inc"])));
    let input = &r["output"];
    assert_eq!((input["api"].clone(), input["command"].clone()), (json!(3), json!("widget")));
    assert_eq!(input["widget"], json!({"id": w, "state": {}, "event": null}), "a new widget starts with no state");
    assert_eq!(undo_len(&s), 1);
    assert_eq!(s.selection(), vec![w]);
    let n = node(&s, w);
    assert!(n.layout.is_some() && !n.locked && n.children.len() == 2);
    let at = n.transform.translation();
    assert!((at.x - 50.0).abs() < 1e-9 && (at.y - 60.0).abs() < 1e-9, "{at:?}");
    assert!(n.children.iter().all(|c| node(&s, *c).locked), "the layers inside are locked");
    assert_eq!(label(&s, w), "one");
    // A click on a layer inside reaches its widget, with the state and the target.
    let c = click(&mut s, n.children[1], "inc");
    assert_eq!(c["id"], json!(w));
    assert_eq!(c["output"]["widget"]["state"], json!({"n": 1}));
    assert_eq!(c["output"]["widget"]["event"], json!({"target": "inc"}));
    assert_eq!((c["state"].clone(), c["targets"].clone()), (json!({"n": 2}), json!(["inc", "dec"])));
    assert_eq!((label(&s, w), node(&s, w).children.len(), undo_len(&s)), ("two".into(), 3, 2));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!((state(&mut s, w), label(&s, w)), (json!({"n": 1}), "one".into()));
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!((state(&mut s, w), label(&s, w)), (json!({"n": 2}), "two".into()));
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(w).is_none(), "undoing the insert removes the widget");
    // Without a position it's centred in the view.
    let (cx, cy) = (s.ds().unwrap().view.cx, s.ds().unwrap().view.cy);
    let w = idv(&ex(&mut s, "widget.insert", json!({"plugin": "org.test.tally"})));
    let b = s.doc().unwrap().world_bounds(w).unwrap();
    assert!((b.center().x - cx).abs() <= 1.0 && (b.center().y - cy).abs() <= 1.0, "{b:?} vs {cx},{cy}");
}

#[test]
fn widgets_survive_save_reload_duplicating_and_export() {
    let mut s = session();
    let (w, _) = insert(&mut s);
    click(&mut s, w, "inc");
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let mut t = session();
    t.ds_mut().unwrap().doc = std::sync::Arc::new(back);
    assert_eq!(state(&mut t, w), json!({"n": 2}), "the state is saved with the file");
    assert_eq!(click(&mut t, w, "dec")["state"], json!({"n": 0}), "and the reloaded widget still works");
    // A duplicate is a widget of its own, with the same state to start with.
    s.execute("select.set", &json!({"ids": [w]})).unwrap();
    ex(&mut s, "edit.duplicate", json!({}));
    let copy = s.selection()[0];
    assert_ne!(copy, w);
    assert_eq!(state(&mut s, copy), json!({"n": 2}));
    click(&mut s, copy, "dec");
    assert_eq!((state(&mut s, copy), state(&mut s, w)), (json!({"n": 0}), json!({"n": 2})));
    assert_eq!(ex(&mut s, "widget.list", json!({}))["widgets"].as_array().unwrap().len(), 2);
    // Exports draw the layers it's made of.
    let svg = crate::cmd::file::export_bytes(s.doc().unwrap(), &[w], figcraft_doc::ExportFormat::Svg, 1.0).unwrap();
    let svg = String::from_utf8(svg).unwrap().to_ascii_lowercase();
    assert!(svg.contains("<svg") && svg.contains("0d99ff"), "{svg}");
}

#[test]
fn clicking_a_widget_on_the_canvas_calls_its_plugin_back() {
    let mut s = session();
    let (w, _) = insert(&mut s);
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let kids = node(&s, w).children.clone();
    let center = |s: &Session, id: NodeId| s.doc().unwrap().world_bounds(id).unwrap().center();
    let (target, text) = (center(&s, kids[1]), center(&s, kids[0]));
    let at = |kind, p: Point| PointerEvent::at(kind, p.x, p.y, 1.0);
    // Over the label: picked as one layer, no callback.
    s.pointer(&at(PointerKind::Move, text)).unwrap();
    assert_eq!(s.tool.overlay.cursor, Cursor::Default);
    s.pointer(&at(PointerKind::Down, text)).unwrap();
    s.pointer(&at(PointerKind::Up, text)).unwrap();
    assert_eq!((s.selection(), state(&mut s, w)), (vec![w], json!({"n": 1})));
    // Over a target: a pointing hand, and a click calls back (one undo step).
    s.pointer(&at(PointerKind::Move, target)).unwrap();
    assert_eq!(s.tool.overlay.cursor, Cursor::Pointer);
    let undo = undo_len(&s);
    s.pointer(&at(PointerKind::Down, target)).unwrap();
    s.pointer(&at(PointerKind::Up, target)).unwrap();
    assert_eq!((state(&mut s, w), undo_len(&s)), (json!({"n": 2}), undo + 1));
    // A quick second click (a double click) is a click too.
    s.pointer(&at(PointerKind::Down, target)).unwrap();
    s.pointer(&at(PointerKind::DoubleClick, target)).unwrap();
    assert_eq!((undo_len(&s), s.ds().unwrap().interaction.is_none()), (undo + 2, true));
    // Dragging by a target moves the widget instead.
    let before = node(&s, w).transform.translation();
    let to = Point::new(target.x + 40.0, target.y + 30.0);
    s.pointer(&at(PointerKind::Down, target)).unwrap();
    s.pointer(&at(PointerKind::Move, to)).unwrap();
    s.pointer(&at(PointerKind::Up, to)).unwrap();
    let after = node(&s, w).transform.translation();
    assert!(after.x - before.x > 30.0 && state(&mut s, w) == json!({"n": 2}), "{before:?} → {after:?}");
}

#[test]
fn a_shift_click_on_a_selected_widget_only_deselects_it() {
    let mut s = session();
    let (w, _) = insert(&mut s);
    let target = s.doc().unwrap().world_bounds(node(&s, w).children[1]).unwrap().center();
    let mut down = PointerEvent::at(PointerKind::Down, target.x, target.y, 1.0);
    down.shift = true;
    // Shift let go before the button: still a Shift-click, not a click on the widget.
    s.pointer(&down).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, target.x, target.y, 1.0)).unwrap();
    assert_eq!((s.selection(), state(&mut s, w)), (vec![], json!({"n": 1})));
}

#[test]
fn bad_widgets_and_clicks_are_refused_and_roll_back() {
    let mut s = session();
    let r = rect(&mut s, 400.0);
    assert!(err(&mut s, "widget.insert", json!({"plugin": "org.nope"})).contains("no plugin"));
    // A plugin that isn't a widget.
    let plain = json!({"id": "org.test.plain", "name": "Plain", "main": "p.wasm", "api": 2, "permissions": {"access": "write"}});
    install(&mut s, plain, &tally(|_| String::new()));
    assert!(err(&mut s, "widget.insert", json!({"plugin": "org.test.plain"})).contains("doesn't draw a widget"));
    // Junk, or nothing, drawn: refused, nothing left behind.
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    let mut junk = W::default();
    let body = junk.render(&json!({"root": {"type": "text", "text": "not a frame"}}));
    install(&mut s, widget_manifest("org.test.junk", "read"), &junk.build(&body));
    assert!(err(&mut s, "widget.insert", json!({"plugin": "org.test.junk"})).contains("root must be a frame"));
    install(&mut s, widget_manifest("org.test.blank", "read"), &W::default().build(""));
    assert!(err(&mut s, "widget.insert", json!({"plugin": "org.test.blank"})).contains("drew no widget"));
    assert_eq!((layer_count(&s), undo_len(&s)), (layers, undo));
    // Clicks on what isn't a widget, or on what it doesn't have.
    let (w, _) = insert(&mut s);
    assert!(err(&mut s, "widget.click", json!({"id": r, "target": "inc"})).contains("isn't a widget"));
    assert!(err(&mut s, "widget.click", json!({"id": w, "target": "nope"})).contains("it has: inc"));
    assert!(err(&mut s, "widget.click", json!({"id": w})).contains("`target` required"));
    // A runaway callback runs out of fuel and is rolled back.
    install(&mut s, widget_manifest("org.test.tally", "read"), &tally(|_| "(loop $l (br $l))".into()));
    let undo = undo_len(&s);
    assert!(err(&mut s, "widget.click", json!({"id": w, "target": "inc", "fuel": 100000})).contains("instruction budget"));
    assert_eq!((state(&mut s, w), undo_len(&s), label(&s, w)), (json!({"n": 1}), undo, "one".into()));
    // Without its plugin a widget stays as it is, and says so when clicked.
    ex(&mut s, "plugin.remove", json!({"id": "org.test.tally"}));
    assert!(err(&mut s, "widget.click", json!({"id": w, "target": "inc"})).contains("no plugin"));
    assert_eq!(node(&s, w).children.len(), 2);
}

#[test]
fn plugins_cant_forge_widgets_or_call_widget_commands() {
    let mut s = session();
    let r = rect(&mut s, 0.0);
    let (w, _) = insert(&mut s);
    for (i, (cmd, params)) in [
        ("node.set", json!({"ids": [r], "props": {"pluginData": {"figcraft.widget": "org.test.tally"}}})),
        ("node.set", json!({"ids": [w], "props": {"pluginData": {"figcraft.widget.state": "{\"n\": 99}"}}})),
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5, "props": {"pluginData": {"figcraft.widget.target": "inc"}}})),
        ("widget.click", json!({"id": w, "target": "inc"})),
        ("widget.list", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let mut b = W::default();
        let body = b.call(cmd, &params);
        let id = format!("org.test.forge{i}");
        install(&mut s, json!({"id": id, "name": "Forge", "main": "f.wasm", "permissions": {"access": "write"}}), &b.build(&body));
        let out = ex(&mut s, "plugin.run", json!({"id": id}))["output"].clone();
        let out = out.as_str().unwrap_or_default().to_string();
        assert!(out.contains("can't write") || out.contains("isn't available to plugins"), "{cmd}: {out}");
    }
    assert!(!node(&s, r).plugin_data.contains_key("figcraft.widget"));
    // Text that merely mentions the keys is fine.
    let mut b = W::default();
    let body = b.call("text.create", &json!({"text": "figcraft.widget", "props": {"pluginData": {"note": "figcraft.widget"}}}));
    install(&mut s, json!({"id": "org.test.text", "name": "Text", "main": "t.wasm", "permissions": {"access": "write"}}), &b.build(&body));
    let layers = layer_count(&s);
    ex(&mut s, "plugin.run", json!({"id": "org.test.text"}));
    assert_eq!(layer_count(&s), layers + 1);
    assert_eq!(state(&mut s, w), json!({"n": 1}));
    // A write widget's own edits elsewhere join its click's undo step.
    let mut s = Session::new();
    install(
        &mut s,
        widget_manifest("org.test.tally", "write"),
        &tally(|b| b.call("shape.rectangle", &json!({"x": 300, "y": 0, "width": 5, "height": 5}))),
    );
    let (w, _) = insert(&mut s);
    let (layers, undo) = (layer_count(&s), undo_len(&s));
    click(&mut s, w, "inc");
    assert_eq!((layer_count(&s), undo_len(&s)), (layers + 1, undo + 1));
}

#[test]
fn widget_state_syncs_in_a_live_session() {
    use crate::live_tests::{push, relay, serve_as, session as live};
    let (mut s, script) = live();
    install(&mut s, widget_manifest("org.test.tally", "read"), &tally(|_| String::new()));
    s.execute("collab.host", &json!({})).unwrap();
    let (mut server, me, welcome) = serve_as(s.doc().unwrap(), crate::live::Access::Edit);
    push(&script, vec![welcome]);
    s.live_poll();
    let (w, _) = insert(&mut s);
    click(&mut s, w, "inc");
    s.live_sync();
    let back = relay(&script, &mut server, me);
    push(&script, back);
    s.live_poll();
    let theirs = server.document().node(w).unwrap();
    assert_eq!(theirs.plugin_data.get("figcraft.widget.state").map(String::as_str), Some(r#"{"n":2}"#));
    assert_eq!(theirs.children.len(), 3, "the redrawn layers reached the server");
}
