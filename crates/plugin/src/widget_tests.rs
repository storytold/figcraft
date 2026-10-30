//! Widgets (ABI v3): descriptions are validated (sizes, counts, depth, numbers, colours, images)
//! and reported by `widget_render`; manifests ask for API 3; junk never gets through.

use serde_json::{Value, json};

use crate::widget::{MAX_FIELD_CHARS, MAX_IMAGE_BYTES, MAX_NODES, MAX_STATE_BYTES, clean_field};
use crate::{Host, Instance, Limits, Manifest, MenuKind, WidgetKind, WidgetRender};

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

#[test]
fn fields_and_menus_are_checked_and_normalised() {
    let root = json!({"type": "frame", "children": [{"type": "text", "field": "label", "text": "Hi"}]});
    let w =
        parse(json!({"root": root, "menu": [{"item": "reset"}, {"item": "c", "type": "color", "options": ["#ff0000"], "value": "ff0000"}]})).unwrap();
    assert!(w.uses_api_4());
    assert_eq!((w.menu[0].label.as_str(), w.menu[0].kind), ("reset", MenuKind::Action), "a label defaults to the item");
    assert_eq!((w.menu[1].options.clone(), w.menu[1].value.clone()), (vec!["#FF0000".to_string()], Some("#FF0000".to_string())));
    assert!(!parse(json!({"root": {"type": "frame"}})).unwrap().uses_api_4(), "API 3 descriptions are unchanged");
    let menu = |m: Value| json!({"root": {"type": "frame"}, "menu": m});
    let field = |kind: &str, name: &str| json!({"root": {"type": "frame", "children": [{"type": kind, "field": name}]}});
    let junk = [
        menu(json!(vec![json!({"item": "a"}); 9])),
        menu(json!([{"item": "a"}, {"item": "a"}])),
        menu(json!([{"item": "has space"}])),
        menu(json!([{"item": ""}])),
        menu(json!([{"item": "a", "label": "x".repeat(33)}])),
        menu(json!([{"item": "a", "label": "line\nbreak"}])),
        menu(json!([{"item": "a", "type": "slider"}])),
        menu(json!([{"item": "a", "checked": true}])),
        menu(json!([{"item": "a", "type": "color"}])),
        menu(json!([{"item": "a", "type": "color", "options": vec!["#000000"; 9]}])),
        menu(json!([{"item": "a", "type": "color", "options": ["red"]}])),
        menu(json!([{"item": "a", "type": "color", "options": ["#000000"], "value": "#FFFFFF"}])),
        menu(json!([{"item": "a", "options": ["#000000"]}])),
        menu(json!({"item": "a"})),
        field("rectangle", "label"),
        field("frame", "label"),
        field("text", "has space"),
        field("text", &"x".repeat(65)),
    ];
    for j in junk {
        assert!(parse(j.clone()).is_err(), "accepted {}", j.to_string().chars().take(100).collect::<String>());
    }
}

#[test]
fn field_values_are_cleaned_and_bounded() {
    assert_eq!(clean_field("a\r\nb\rc\td\u{7}\u{0}é").unwrap(), "a\nb\nc dé");
    assert_eq!(clean_field(&"é".repeat(MAX_FIELD_CHARS)).unwrap().chars().count(), MAX_FIELD_CHARS);
    assert!(clean_field(&"x".repeat(MAX_FIELD_CHARS + 1)).is_err());
    assert_eq!(clean_field("").unwrap(), "");
}

#[test]
fn hover_styles_are_checked_and_normalised() {
    let w = parse(json!({"root": {"type": "frame", "hover": {"opacity": 0.8}, "children": [
        {"type": "rectangle", "target": "go", "fill": "#0D99FF", "hover": {"fill": "0b7fd6", "stroke": "#000"}},
        {"type": "text", "text": "Go", "hover": {"color": "#fff"}}]}}))
    .unwrap();
    assert_eq!(w.root.hover.as_ref().and_then(|h| h.opacity), Some(0.8));
    let rect = w.root.children[0].hover.clone().unwrap();
    assert_eq!((rect.fill.as_deref(), rect.stroke.as_deref()), (Some("#0B7FD6"), Some("#000000")));
    assert_eq!(w.root.children[1].hover.as_ref().and_then(|h| h.color.as_deref()), Some("#FFFFFF"));
    for (bad, why) in [
        (json!({"type": "rectangle", "hover": {}}), "needs a fill"),
        (json!({"type": "rectangle", "hover": {"fill": "red"}}), "hex colour"),
        (json!({"type": "rectangle", "hover": {"opacity": 1.5}}), "from 0 to 1"),
        (json!({"type": "rectangle", "hover": {"color": "#fff"}}), "for text layers"),
        (json!({"type": "text", "text": "x", "hover": {"fill": "#fff"}}), "takes `color`"),
    ] {
        let e = parse(json!({"root": {"type": "frame", "children": [bad]}})).unwrap_err();
        assert!(e.contains(why), "{e}");
    }
    assert!(!w.uses_api_4(), "a hover style needs no new API: older hosts draw the layer as it is");
}
