//! Widgets (ABI v3): descriptions are validated (sizes, counts, depth, numbers, colours, images)
//! and reported by `widget_render`; manifests ask for API 3; junk never gets through.

use serde_json::{Value, json};

use crate::widget::{MAX_IMAGE_BYTES, MAX_NODES, MAX_STATE_BYTES};
use crate::{Host, Instance, Limits, Manifest, WidgetKind, WidgetRender};

struct Quiet;

impl Host for Quiet {
    fn call(&mut self, _command: &str, _params: &str) -> Result<String, String> {
        Ok("{}".into())
    }
    fn selection(&mut self) -> String {
        "[]".into()
    }
    fn notify(&mut self, _message: &str) {}
}

fn parse(v: Value) -> Result<WidgetRender, String> {
    WidgetRender::parse(&v.to_string())
}

#[test]
fn descriptions_are_checked_and_normalised() {
    let w = parse(
        json!({"state": {"n": 2}, "root": {"type": "frame", "layout": "horizontal", "gap": 8, "padding": 12, "fill": "#fff", "justify": "center",
        "children": [{"type": "text", "text": "2", "size": 24, "color": "#112233", "bold": true},
                     {"type": "rectangle", "target": "inc", "width": 24, "height": 24, "radius": 4, "fill": "0d99ff80"}]}}),
    )
    .unwrap();
    assert_eq!(w.root.fill.as_deref(), Some("#FFFFFF"), "colours come out as #RRGGBB(AA)");
    assert_eq!(w.root.children[1].fill.as_deref(), Some("#0D99FF80"));
    assert_eq!(w.root.children[0].kind, WidgetKind::Text);
    assert_eq!(w.state, Some(json!({"n": 2}).as_object().unwrap().clone()));
    assert!(parse(json!({"root": {"type": "frame"}})).unwrap().state.is_none(), "no state: keep the old one");
}

#[test]
fn junk_descriptions_are_refused() {
    let deep = (0..20).fold(json!({"type": "frame"}), |inner, _| json!({"type": "frame", "children": [inner]}));
    let wide = json!({"root": {"type": "frame", "children": vec![json!({"type": "rectangle"}); MAX_NODES]}});
    let image = |bytes: usize| json!({"root": {"type": "frame", "children": [{"type": "image", "data": "A".repeat(bytes / 3 * 4)}]}});
    let junk = [
        json!(null),
        json!({}),
        json!({"root": {"type": "text", "text": "a root must be a frame"}}),
        json!({"root": {"type": "frame", "width": -1}}),
        json!({"root": {"type": "frame", "width": 1e9}}),
        json!({"root": {"type": "frame", "opacity": 2}}),
        json!({"root": {"type": "frame", "fill": "red"}}),
        json!({"root": {"type": "frame", "fill": "#1234567890"}}),
        json!({"root": {"type": "frame", "children": [{"type": "text", "text": "x", "children": [{"type": "frame"}]}]}}),
        json!({"root": {"type": "frame", "children": [{"type": "rectangle", "text": "not here"}]}}),
        json!({"root": {"type": "frame", "children": [{"type": "text", "justify": "center"}]}}),
        json!({"root": {"type": "frame", "justify": "middle"}}),
        json!({"root": {"type": "frame", "children": [{"type": "text", "text": "a\u{7}b"}]}}),
        json!({"root": {"type": "frame", "children": [{"type": "text", "text": "x".repeat(1001)}]}}),
        json!({"root": {"type": "frame", "children": [{"type": "text", "size": 0}]}}),
        json!({"root": {"type": "frame", "target": "has space"}}),
        json!({"root": {"type": "frame", "name": "x".repeat(65)}}),
        json!({"root": {"type": "frame", "children": [{"type": "image"}]}}),
        json!({"root": {"type": "frame", "children": [{"type": "video"}]}}),
        json!({"state": [1], "root": {"type": "frame"}}),
        json!({"state": {"": 1}, "root": {"type": "frame"}}),
        json!({"state": {"k": "x".repeat(MAX_STATE_BYTES)}, "root": {"type": "frame"}}),
        json!({"root": deep}),
        wide,
        image(MAX_IMAGE_BYTES + 3000),
    ];
    for j in junk {
        assert!(parse(j.clone()).is_err(), "accepted {}", j.to_string().chars().take(80).collect::<String>());
    }
    assert!(parse(image(1000)).is_ok());
    assert!(WidgetRender::parse(&"[".repeat(100_000)).is_err(), "deep JSON fails cleanly");
    assert!(WidgetRender::parse(&" ".repeat(5 * 1024 * 1024)).is_err());
}

#[test]
fn widget_render_reports_the_description_or_why_not() {
    let good = r#"{"root":{"type":"frame","children":[{"type":"text","text":"hi"}]}}"#;
    let bad = r#"{"root":{"type":"text"}}"#;
    let wat = format!(
        r#"(module
          (import "figcraft" "widget_render" (func $render (param i32 i32) (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 100) "{}") (data (i32.const 1000) "{}")
          (func (export "figcraft_run") (param $n i32) (result i32)
            (if (call $render (i32.const 100) (i32.const {}))
              (then (call $set_output (i32.const 5000) (call $reply_read (i32.const 5000) (i32.const 1000)))))
            (drop (call $render (i32.const 1000) (i32.const {})))
            (i32.const 0)))"#,
        bad.replace('"', "\\\""),
        good.replace('"', "\\\""),
        bad.len(),
        good.len()
    );
    let wasm = wat::parse_str(&wat).unwrap();
    let out = Instance::new(&wasm, &Limits::default()).unwrap().call("{}", &mut Quiet).unwrap();
    assert!(out.output.contains("root must be a frame"), "{}", out.output);
    assert_eq!(out.widget.unwrap().root.children[0].text, "hi");
}

#[test]
fn widgets_need_api_3() {
    let m = |api: u32| Manifest::from_value(&json!({"id": "w", "name": "W", "main": "w.wasm", "api": api, "widget": true}));
    assert!(m(2).unwrap_err().contains("API 3"));
    let ok = m(3).unwrap();
    assert!(ok.widget && ok.commands.is_empty(), "a widget-only plugin has no menu command");
    assert!(Manifest::from_value(&json!({"id": "w", "name": "W", "main": "w.wasm", "api": 5})).is_err());
    let v2 = Manifest::from_value(&json!({"id": "p", "name": "P", "main": "p.wasm", "api": 2})).unwrap();
    assert!(!v2.widget && v2.commands.len() == 1, "v2 manifests are unchanged");
}
