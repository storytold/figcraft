//! Plugin API 4: what it adds to widget descriptions and panels is validated and capped, read as
//! before by plugins written for API 1–3 (ignored where they ignored it, refused where they
//! refused it), `widget_timer` is bounded and needs API 4, and junk (rows of it, mutations of a
//! good description, random bytes) is refused without a panic.

use serde_json::{Value, json};

use crate::widget::{MAX_MENU_ITEMS, MAX_PATH_BYTES, MAX_VECTORS, MenuKind, Sides};
use crate::{Host, Instance, Limits, Manifest, Panel, ParamKind, WidgetKind, WidgetRender};

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

fn widget(v: &Value, api: u32) -> Result<WidgetRender, String> {
    WidgetRender::parse(&v.to_string(), api)
}

/// A description using everything API 4 added.
fn rich() -> Value {
    json!({"state": {"n": 1}, "menu": [
        {"type": "action", "name": "reset", "label": " Reset "},
        {"type": "toggle", "name": "pinned"},
        {"type": "dropdown", "name": "size", "options": ["S", "M"], "value": "XL"},
        {"type": "color", "name": "tint", "options": ["#f00", "00ff00"], "value": "#00FF00"},
        {"type": "separator"},
        {"type": "link", "name": "docs", "label": "Docs", "url": "https://example.org/a?b=c"}
    ], "root": {"type": "frame", "layout": "vertical", "width": 200, "height": 300, "padding": [1, 2, 3, 4], "radius": [4, 3, 2, 1],
        "gradient": {"type": "radial", "stops": [{"at": 0, "color": "#fff"}, {"at": 1, "color": "#000000"}]},
        "shadow": {"y": 2, "blur": 8, "color": "#0004"}, "hover": {"opacity": 0.5}, "tooltip": " A card ", "children": [
        {"type": "text", "text": "Title", "input": "title", "maxLength": 20, "weight": 600, "textAlign": "right", "maxLines": 1, "width": 80,
         "hover": {"fill": "#f00", "stroke": "#00f"}},
        {"type": "text", "text": "Notes", "input": "notes", "multiline": true},
        {"type": "line", "width": 40, "stroke": "#ccc", "fillWidth": true},
        {"type": "vector", "path": "M0 0 L10 0 L5 8 Z", "fill": "#111", "fillHeight": true},
        {"type": "image", "data": "AAAA", "fit": "crop"}
    ]}})
}

#[test]
fn what_api_4_added_is_checked_and_normalised() {
    let w = widget(&rich(), 4).unwrap();
    let menu = w.menu.unwrap();
    assert_eq!(menu.len(), 6);
    assert_eq!((menu[0].title(), menu[1].title(), menu[1].value.clone()), ("Reset", "pinned", Some(json!(false))));
    assert_eq!(menu[2].value, Some(json!("S")), "a value that isn't an option: the first");
    assert_eq!((menu[3].options.clone(), menu[3].value.clone()), (vec!["#FF0000".to_string(), "#00FF00".into()], Some(json!("#00FF00"))));
    assert_eq!((menu[4].kind, menu[5].kind), (MenuKind::Separator, MenuKind::Link));
    let root = &w.root;
    assert_eq!((root.padding, root.radius), (Some(Sides::Each([1.0, 2.0, 3.0, 4.0])), Some(Sides::Each([4.0, 3.0, 2.0, 1.0]))));
    assert_eq!(root.gradient.as_ref().unwrap().stops[0].color, "#FFFFFF");
    assert_eq!((root.shadow.as_ref().unwrap().color.as_deref(), root.tooltip.as_str()), (Some("#00000044"), "A card"));
    assert_eq!(root.children[0].hover.as_ref().unwrap().fill.as_deref(), Some("#FF0000"));
    assert_eq!((root.children[2].kind, root.children[3].kind), (WidgetKind::Line, WidgetKind::Vector));
    // Menu values: an action has none, a toggle flips or takes a boolean, the rest one option.
    assert_eq!(menu[0].coerce(Some(&json!(5))), Ok(Value::Null));
    assert_eq!((menu[1].coerce(None), menu[1].coerce(Some(&json!(false)))), (Ok(json!(true)), Ok(json!(false))));
    assert!(menu[1].coerce(Some(&json!("on"))).is_err());
    assert_eq!((menu[2].coerce(Some(&json!("M"))), menu[3].coerce(Some(&json!("#ff0000")))), (Ok(json!("M")), Ok(json!("#FF0000"))));
    assert!(
        menu[2].coerce(Some(&json!("m"))).is_err() && menu[3].coerce(None).is_err() && menu[4].coerce(None).is_err() && menu[5].coerce(None).is_err()
    );
}

#[test]
fn older_apis_read_descriptions_as_they_always_did() {
    // Fields API 3 didn't know are dropped (it ignored them); kinds and shapes it refused are refused.
    let mut old = rich();
    old["root"]["padding"] = json!(4);
    old["root"]["radius"] = json!(2);
    old["root"]["children"].as_array_mut().unwrap().retain(|c| c["type"] != "line" && c["type"] != "vector");
    let w = widget(&old, 3).unwrap();
    assert!(w.menu.is_none());
    let plain = widget(&json!({"state": {"n": 1}, "root": {"type": "frame", "layout": "vertical", "width": 200, "height": 300, "padding": 4, "radius": 2, "children": [
        {"type": "text", "text": "Title", "width": 80}, {"type": "text", "text": "Notes"}, {"type": "image", "data": "AAAA"}]}}), 3)
    .unwrap();
    assert_eq!(w, plain, "nothing API 4 added is left");
    for api in 1..=3 {
        assert!(widget(&rich(), api).unwrap_err().contains("plugin API 4"), "four paddings");
        assert!(
            widget(&json!({"root": {"type": "frame", "children": [{"type": "line"}]}}), api).unwrap_err().contains("a line layer needs plugin API 4")
        );
        assert!(widget(&json!({"root": {"type": "frame", "children": [{"type": "vector", "path": "M0 0"}]}}), api).is_err());
        // Junk in a field that's ignored is ignored too, as an unknown field was.
        assert!(widget(&json!({"root": {"type": "frame", "tooltip": "x".repeat(5000), "input": "a b"}, "menu": [{"type": "link"}]}), api).is_ok());
    }
    // Panels: the controls API 4 added are refused, its fields on old controls dropped.
    let spec = json!({"controls": [{"type": "number", "name": "n", "step": 5, "disabled": true, "error": "no", "value": 3}]}).to_string();
    let (old, new) = (Panel::parse(&spec, 2).unwrap(), Panel::parse(&spec, 4).unwrap());
    assert_eq!((old.controls[0].step, old.controls[0].disabled, old.controls[0].error.as_str()), (None, false, ""));
    assert_eq!(
        (new.controls[0].step, new.controls[0].disabled, new.controls[0].error.as_str(), new.controls[0].default.clone()),
        (Some(5.0), true, "no", Some(json!(3.0)))
    );
    for kind in ["slider", "image", "progress", "divider", "section"] {
        let spec = json!({"controls": [{"type": kind, "name": "x", "data": "AAAA"}]}).to_string();
        assert!(Panel::parse(&spec, 3).unwrap_err().contains("needs plugin API 4"), "{kind}");
    }
    assert!(Panel::parse(&format!(r#"{{"controls": [], "title": "{}"}}"#, " ".repeat(70_000)), 3).unwrap_err().contains("64 KB"));
    assert!(Panel::parse(&format!(r#"{{"controls": [], "title": "{}"}}"#, " ".repeat(70_000)), 4).is_ok(), "API 4 panels may carry images");
    // Manifests: a slider parameter needs API 4; display controls never go in a manifest.
    let manifest = |api: u32, kind: &str| {
        Manifest::from_value(
            &json!({"id": "p", "name": "P", "main": "p.wasm", "api": api, "commands": [{"id": "c", "label": "C", "params": [{"name": "x", "type": kind}]}]}),
        )
    };
    assert!(manifest(3, "slider").unwrap_err().contains("needs plugin API 4"));
    assert_eq!(manifest(4, "slider").unwrap().commands[0].params[0].default_value(), json!(0.0));
    assert!(manifest(4, "divider").unwrap_err().contains("only go in panels"));
    assert!(manifest(5, "text").is_err(), "a newer API than this build's is refused");
}

#[test]
fn api_4_panel_controls_are_checked() {
    let p = Panel::parse(
        &json!({"controls": [
            {"type": "slider", "name": "a", "min": 1, "max": 2, "step": 0.25, "value": 1.6},
            {"type": "slider", "name": "b", "value": 250},
            {"type": "image", "label": "Pic", "data": "AAAA", "height": 64, "fit": "fill"},
            {"type": "progress", "value": 7}, {"type": "progress", "value": "x"},
            {"type": "divider"}, {"type": "section", "label": "More"}, {"type": "section", "label": "Shut", "value": false}
        ]})
        .to_string(),
        4,
    )
    .unwrap();
    let c = &p.controls;
    assert_eq!((c[0].default.clone(), c[1].default.clone(), c[1].max), (Some(json!(1.5)), Some(json!(100.0)), Some(100.0)));
    assert_eq!((c[0].coerce(&json!(1.13)), c[0].coerce(&json!(-4)), c[0].coerce(&json!("1.9"))), (Ok(json!(1.25)), Ok(json!(1.0)), Ok(json!(2.0))));
    assert_eq!(
        (c[3].default.clone(), c[4].default.clone(), c[6].default.clone(), c[7].default.clone()),
        (Some(json!(1.0)), Some(json!(0.0)), None, Some(json!(false)))
    );
    assert!(c.iter().filter(|c| c.is_field()).map(|c| c.name.as_str()).eq(["a", "b"]), "only sliders hold a value here");
    assert_eq!(c[2].kind, ParamKind::Image);
    assert_eq!(p.values(&serde_json::Map::new()).len(), 2);
    let image = "A".repeat(400_000);
    for (controls, why) in [
        (json!([{"type": "slider", "name": "s", "step": 0}]), "bad step"),
        (json!([{"type": "slider", "name": "s", "step": -1}]), "bad step"),
        (json!([{"type": "slider", "name": "s", "min": 5, "max": 1}]), "bad bounds"),
        (json!([{"type": "slider"}]), "parameter name"),
        (json!([{"type": "image", "label": "No data"}]), "`data`"),
        (json!([{"type": "button", "name": "b", "data": "AAAA"}]), "`data`"),
        (json!([{"type": "image", "data": "AAAA", "height": 8}]), "high"),
        (json!([{"type": "image", "data": "AAAA", "fit": "stretch"}]), "unknown variant"),
        (json!([{"type": "image", "data": image}, {"type": "image", "data": image}]), "larger than 512 KB together"),
        (json!([{"type": "text", "name": "t", "error": "x".repeat(201)}]), "longer than"),
        (json!([{"type": "text", "name": "t", "error": "a\u{7}"}]), "control characters"),
    ] {
        let e = Panel::parse(&json!({ "controls": controls }).to_string(), 4).unwrap_err();
        assert!(e.contains(why), "{e}");
    }
    // A step can't push a value outside its bounds or to infinity.
    let odd = Panel::parse(&json!({"controls": [{"type": "slider", "name": "s", "min": 0, "max": 10, "step": 7, "value": 9}, {"type": "number", "name": "n", "min": -1e308, "step": 1e-300, "value": 1e308}]}).to_string(), 4).unwrap();
    assert_eq!(odd.controls[0].default, Some(json!(7.0)));
    assert!(odd.controls[1].default.as_ref().and_then(Value::as_f64).is_some_and(f64::is_finite));
}

#[test]
fn junk_in_what_api_4_added_is_refused() {
    let child = |c: Value| json!({"root": {"type": "frame", "layout": "vertical", "children": [c]}});
    let root = |mut extra: Value| {
        extra["type"] = json!("frame");
        json!({ "root": extra })
    };
    let menu = |m: Value| json!({"root": {"type": "frame"}, "menu": m});
    let stops = |n: usize| (0..n).map(|i| json!({"at": i as f64 / n.max(2) as f64, "color": "#000"})).collect::<Vec<_>>();
    let junk = [
        root(json!({"padding": [1, 2, 3]})),
        root(json!({"padding": [1, 2, 3, -4]})),
        root(json!({"radius": [1, 2, 3, 1e9]})),
        root(json!({"radius": "8"})),
        root(json!({"gradient": {"stops": stops(1)}})),
        root(json!({"gradient": {"stops": stops(9)}})),
        root(json!({"gradient": {"type": "conic", "stops": stops(2)}})),
        root(json!({"gradient": {"angle": 720, "stops": stops(2)}})),
        root(json!({"gradient": {"stops": [{"at": 2, "color": "#000"}, {"at": 0, "color": "#000"}]}})),
        root(json!({"gradient": {"stops": [{"at": 0, "color": "red"}, {"at": 1, "color": "#000"}]}})),
        root(json!({"shadow": {"blur": -1}})),
        root(json!({"shadow": {"x": 1e9}})),
        root(json!({"shadow": {"color": "shadow"}})),
        root(json!({"hover": {"fill": "nope"}})),
        root(json!({"hover": {"opacity": 3}})),
        root(json!({"tooltip": "x".repeat(201)})),
        root(json!({"tooltip": "a\nb"})),
        root(json!({"input": "title"})),
        root(json!({"path": "M0 0"})),
        root(json!({"weight": 400})),
        root(json!({"fit": "fill"})),
        // Nothing can fill a frame that hugs it, one without auto layout, or the canvas.
        child(json!({"type": "rectangle", "fillWidth": true})),
        child(json!({"type": "frame", "layout": "horizontal", "width": 50, "children": [{"type": "rectangle", "fillHeight": true}]})),
        child(json!({"type": "frame", "width": 50, "height": 50, "children": [{"type": "rectangle", "fillWidth": true}]})),
        root(json!({"layout": "vertical", "width": 100, "fillWidth": true})),
        child(json!({"type": "text", "input": "a b"})),
        child(json!({"type": "text", "input": "t", "maxLength": 0})),
        child(json!({"type": "text", "input": "t", "maxLength": 1001})),
        child(json!({"type": "rectangle", "maxLength": 5})),
        child(json!({"type": "rectangle", "multiline": true})),
        child(json!({"type": "text", "weight": 50})),
        child(json!({"type": "text", "weight": 1000})),
        child(json!({"type": "text", "textAlign": "justify"})),
        child(json!({"type": "text", "maxLines": 0})),
        child(json!({"type": "text", "gradient": {"stops": stops(2)}})),
        child(json!({"type": "image", "data": "AAAA", "fit": "stretch"})),
        child(json!({"type": "image", "data": "AAAA", "gradient": {"stops": stops(2)}})),
        child(json!({"type": "vector"})),
        child(json!({"type": "vector", "path": " "})),
        child(json!({"type": "vector", "path": "M0 0 L1 1 <script>"})),
        child(json!({"type": "vector", "path": "M0 0 L1 1 \u{0}"})),
        child(json!({"type": "vector", "path": format!("M0 0 {}", "L1 1 ".repeat(MAX_PATH_BYTES / 5 + 1))})),
        child(json!({"type": "line", "path": "M0 0"})),
        child(json!({"type": "line", "text": "x"})),
        child(json!({"type": "line", "children": [{"type": "frame"}]})),
        json!({"root": {"type": "frame", "children": [{"type": "text", "input": "t"}, {"type": "frame", "children": [{"type": "text", "input": "t"}]}]}}),
        json!({"root": {"type": "frame", "children": vec![json!({"type": "vector", "path": "M0 0 L1 1"}); MAX_VECTORS + 1]}}),
        menu(json!({"type": "action"})),
        menu(json!([{"type": "action"}])),
        menu(json!([{"type": "action", "name": "a b"}])),
        menu(json!([{"type": "action", "name": "a"}, {"type": "toggle", "name": "a"}])),
        menu(json!([{"type": "action", "name": "a", "options": ["x"]}])),
        menu(json!([{"type": "action", "name": "a", "url": "https://example.org"}])),
        menu(json!([{"type": "action", "name": "a", "label": "x".repeat(81)}])),
        menu(json!([{"type": "dropdown", "name": "d"}])),
        menu(json!([{"type": "dropdown", "name": "d", "options": vec!["o"; 33]}])),
        menu(json!([{"type": "dropdown", "name": "d", "options": ["a\u{1}"]}])),
        menu(json!([{"type": "color", "name": "c", "options": ["red"]}])),
        menu(json!([{"type": "link", "name": "l"}])),
        menu(json!([{"type": "link", "name": "l", "url": "javascript:alert(1)"}])),
        menu(json!([{"type": "link", "name": "l", "url": "file:///etc/passwd"}])),
        menu(json!([{"type": "link", "name": "l", "url": "https://example.org/a b"}])),
        menu(json!([{"type": "link", "name": "l", "url": format!("https://example.org/{}", "a".repeat(2100))}])),
        menu(json!([{"type": "submenu", "name": "s"}])),
        menu(json!(vec![json!({"type": "separator"}); MAX_MENU_ITEMS + 1])),
    ];
    for j in &junk {
        assert!(widget(j, 4).is_err(), "accepted {}", j.to_string().chars().take(120).collect::<String>());
    }
    assert!(widget(&menu(json!([])), 4).unwrap().menu.is_some_and(|m| m.is_empty()), "an empty menu removes it");
    // A frame that fills gives what's inside it room to fill too.
    let nested = json!({"root": {"type": "frame", "layout": "vertical", "width": 100, "children": [
        {"type": "frame", "layout": "horizontal", "fillWidth": true, "children": [{"type": "rectangle", "fillWidth": true}]}]}});
    assert!(widget(&nested, 4).is_ok());
    assert!(widget(&menu(json!(vec![json!({"type": "separator"}); MAX_MENU_ITEMS])), 4).is_ok());
}

/// xorshift: the same "random" bytes every run.
fn noise(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn mutated_and_random_descriptions_never_panic() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let panel = json!({"title": "P", "controls": [
        {"type": "slider", "name": "a", "min": 1, "max": 2, "step": 0.25, "value": 1.6, "error": "e", "disabled": true},
        {"type": "image", "label": "Pic", "data": "AAAA", "height": 64}, {"type": "progress", "value": 0.3}, {"type": "section", "label": "S"}]})
    .to_string();
    for good in [rich().to_string(), panel] {
        let bytes = good.as_bytes();
        for _ in 0..4000 {
            // Flip, drop or repeat a few bytes of a good description.
            let mut b = bytes.to_vec();
            for _ in 0..1 + noise(&mut seed) % 4 {
                let at = (noise(&mut seed) as usize) % b.len().max(1);
                match noise(&mut seed) % 3 {
                    0 => b[at] = (noise(&mut seed) % 256) as u8,
                    1 => drop(b.remove(at)),
                    _ => b.insert(at, b[at]),
                }
            }
            let text = String::from_utf8_lossy(&b);
            for api in [1, 3, 4, u32::MAX] {
                let _ = WidgetRender::parse(&text, api);
                let _ = Panel::parse(&text, api);
            }
        }
    }
    for len in [0usize, 1, 7, 64, 513, 4096] {
        for _ in 0..200 {
            let b: Vec<u8> = (0..len).map(|_| (noise(&mut seed) % 256) as u8).collect();
            let text = String::from_utf8_lossy(&b);
            let _ = (WidgetRender::parse(&text, 4), Panel::parse(&text, 4), Manifest::parse(&text));
        }
    }
}

/// A module that asks for a timer of `ms` and outputs the status and the reply.
fn timer_module(ms: i64) -> Vec<u8> {
    let wat = format!(
        r#"(module
          (import "figcraft" "widget_timer" (func $timer (param i32) (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          (func (export "figcraft_run") (param $n i32) (result i32)
            (i32.store8 (i32.const 0) (i32.add (i32.const 48) (call $timer (i32.const {ms}))))
            (call $set_output (i32.const 0) (i32.add (i32.const 1) (call $reply_read (i32.const 1) (i32.const 200))))
            (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap()
}

#[test]
fn widget_timer_is_bounded_charged_and_needs_api_4() {
    let run = |ms: i64, api: u32| Instance::new(&timer_module(ms), &Limits { api, ..Limits::default() }).unwrap().call("{}", &mut Quiet).unwrap();
    for ms in [100, 1000, 3_600_000] {
        let out = run(ms, 4);
        assert_eq!((out.timer, out.output.as_bytes()[0], out.calls), (Some(ms as u32), b'0', 3), "{ms}");
    }
    assert_eq!(run(0, 4).timer, Some(0), "0 cancels");
    let mut seed = 7u64;
    let junk =
        [99, 1, -1, 3_600_001, i64::from(i32::MAX), i64::from(i32::MIN)].into_iter().chain((0..300).map(|_| i64::from(noise(&mut seed) as i32)));
    for ms in junk.filter(|ms| *ms != 0 && !(100..=3_600_000).contains(ms)) {
        let out = run(ms, 4);
        assert!(out.timer.is_none() && out.output.starts_with("1a timer runs 100 to 3600000"), "{ms}: {}", out.output);
    }
    for api in 1..=3 {
        let out = run(1000, api);
        assert!(out.timer.is_none() && out.output.contains("need plugin API 4"), "{}", out.output);
    }
    // It costs what any host call costs: a plugin can't spin on it for free.
    let poor = Limits { fuel: 5_000, ..Limits::default() };
    assert!(Instance::new(&timer_module(1000), &poor).unwrap().call("{}", &mut Quiet).is_err());
    // A wrong signature is refused when the module is checked.
    let wrong = r#"(module (import "figcraft" "widget_timer" (func (param i32 i32) (result i32))) (memory (export "memory") 1)
        (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#;
    assert!(crate::check_module(&wat::parse_str(wrong).unwrap(), &Limits::default()).is_err());
}

/// A panel grid with two vector cells and one picture cell.
fn grid_panel() -> Value {
    json!({"title": "Icons", "controls": [
        {"type": "grid", "name": "icon", "label": "Icons", "columns": 2,
         "items": [
            {"value": "heart", "path": "M12 21C7 16 3 13 3 9c0-3 2-5 4.5-5C10 4 12 7 12 7s2-3 4.5-3C19 4 21 6 21 9c0 4-4 7-9 12Z", "label": "Heart", "color": "#F24822"},
            {"value": "plus", "path": "M12 5v14M5 12h14", "label": "Plus", "color": "#111111", "stroke": 2.0},
            {"value": "photo", "data": "AAAA", "label": "Photo"}
         ], "value": "plus"}
    ]})
}

#[test]
fn a_panel_grid_is_checked_and_coerced() {
    let p = Panel::parse(&grid_panel().to_string(), 4).unwrap();
    assert_eq!(p.controls.len(), 1);
    let g = &p.controls[0];
    assert_eq!((g.name.as_str(), g.items.len()), ("icon", 3));
    assert_eq!((g.items[0].color.as_str(), g.items[1].stroke), ("#F24822", Some(2.0)));
    assert_eq!(g.default_value(), json!("plus"), "the spec's value wins");
    assert_eq!(g.coerce(&json!("photo")), Ok(json!("photo")));
    assert!(g.coerce(&json!("missing")).is_err());
    // Older APIs refuse it; command parameters refuse it too.
    assert!(Panel::parse(&grid_panel().to_string(), 3).is_err());
    let mut cmd = grid_panel();
    cmd["controls"][0]["value"] = Value::Null;
    assert_eq!(Panel::parse(&cmd.to_string(), 4).unwrap().controls[0].default_value(), json!("heart"));
    for bad in [
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "items": []}]}),
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "items": [{"value": "a", "path": "M0 0"}, {"value": "a", "path": "M1 1"}]}]}),
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "items": [{"value": "a"}]}]}),
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "items": [{"value": "a", "path": "M0 0", "data": "AAAA"}]}]}),
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "columns": 13, "items": [{"value": "a", "path": "M0 0"}]}]}),
        json!({"title": "t", "controls": [{"type": "grid", "name": "g", "items": [{"value": "a", "path": "M0 0", "color": "nope"}]}]}),
        json!({"title": "t", "controls": [{"type": "text", "name": "t", "items": [{"value": "a", "path": "M0 0"}]}]}),
    ] {
        assert!(Panel::parse(&bad.to_string(), 4).is_err(), "{bad}");
    }
    let mut manifest = json!({"id": "x.y", "name": "y", "main": "p.wasm", "api": 4,
        "commands": [{"id": "run", "label": "Run", "params": [{"type": "grid", "name": "g", "items": [{"value": "a", "path": "M0 0"}]}]}]});
    assert!(Manifest::from_value(&manifest).is_err(), "grids only go in panels");
    manifest["commands"][0]["params"][0]["type"] = json!("text");
    manifest["commands"][0]["params"].as_array_mut().unwrap()[0].as_object_mut().unwrap().remove("items");
    assert!(Manifest::from_value(&manifest).is_ok());
}
