//! ABI v2 in the sandbox: an instance keeps its memory between calls, panels are validated and
//! reported, settings go through the host.

use std::collections::BTreeMap;

use serde_json::{Map, json};

use crate::{Host, Instance, Limits, Manifest, Panel, ParamKind, PluginError, UiChange};

/// A host that keeps settings.
#[derive(Default)]
struct Settings(BTreeMap<String, String>);

impl Host for Settings {
    fn call(&mut self, _command: &str, _params: &str) -> Result<String, String> {
        Ok("{}".into())
    }
    fn selection(&mut self) -> String {
        "[]".into()
    }
    fn notify(&mut self, _message: &str) {}
    fn setting(&mut self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_setting(&mut self, key: &str, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(v) if v.len() > 10 => Err("too big".into()),
            Some(v) => {
                self.0.insert(key.into(), v.into());
                Ok(())
            }
            None => {
                self.0.remove(key);
                Ok(())
            }
        }
    }
}

fn v2_module(data: &str, body: &str) -> Vec<u8> {
    let wat = format!(
        r#"(module
          (import "figcraft" "reply_len" (func $reply_len (result i32)))
          (import "figcraft" "reply_read" (func $reply_read (param i32 i32) (result i32)))
          (import "figcraft" "set_output" (func $set_output (param i32 i32)))
          (import "figcraft" "ui_show" (func $ui_show (param i32 i32) (result i32)))
          (import "figcraft" "ui_close" (func $ui_close))
          (import "figcraft" "setting_get" (func $setting_get (param i32 i32) (result i32)))
          (import "figcraft" "setting_set" (func $setting_set (param i32 i32 i32 i32) (result i32)))
          (import "figcraft" "setting_delete" (func $setting_delete (param i32 i32)))
          (memory (export "memory") 1)
          {data}
          (func (export "figcraft_run") (param $n i32) (result i32) {body}))"#
    );
    wat::parse_str(&wat).unwrap_or_else(|e| panic!("{e}\n{wat}"))
}

const PANEL: &str = r#"{"title":"T","controls":[{"type":"button","name":"go","label":"Go"}]}"#;

#[test]
fn an_instance_keeps_its_memory_and_reports_panel_changes() {
    // A counter at 100 ('0' + calls): call 1 shows a panel, call 2 an invalid one (the refusal is
    // its output), call 3 closes it.
    let bad = r#"{"controls":[{"type":"rocket"}]}"#;
    let m = v2_module(
        &format!(
            r#"(data (i32.const 100) "0") (data (i32.const 200) "{}") (data (i32.const 1000) "{}")"#,
            PANEL.replace('"', "\\\""),
            bad.replace('"', "\\\"")
        ),
        &format!(
            r#"(i32.store8 (i32.const 100) (i32.add (i32.load8_u (i32.const 100)) (i32.const 1)))
            (if (i32.eq (i32.load8_u (i32.const 100)) (i32.const 49)) (then (drop (call $ui_show (i32.const 200) (i32.const {})))))
            (if (i32.eq (i32.load8_u (i32.const 100)) (i32.const 50))
              (then (if (call $ui_show (i32.const 1000) (i32.const {}))
                (then (call $set_output (i32.const 2000) (call $reply_read (i32.const 2000) (call $reply_len))) (return (i32.const 0))))))
            (if (i32.eq (i32.load8_u (i32.const 100)) (i32.const 51)) (then (call $ui_close)))
            (call $set_output (i32.const 100) (i32.const 1))
            (i32.const 0)"#,
            PANEL.len(),
            bad.len()
        ),
    );
    let mut inst = Instance::new(&m, &Limits::default()).unwrap();
    let mut host = Settings::default();
    let first = inst.call("{}", &mut host).unwrap();
    assert_eq!((first.output.as_str(), first.ui), ("1", Some(UiChange::Show(Panel::parse(PANEL).unwrap()))));
    let second = inst.call("{}", &mut host).unwrap();
    assert!(second.output.contains("panel:") && second.ui.is_none(), "{second:?}");
    let third = inst.call("{}", &mut host).unwrap();
    assert_eq!((third.output.as_str(), third.ui), ("3", Some(UiChange::Close)));
}

#[test]
fn settings_go_through_the_host() {
    let m = v2_module(
        r#"(data (i32.const 0) "k") (data (i32.const 10) "[1,2]") (data (i32.const 20) "\"far too long\"")"#,
        r#"(if (call $setting_set (i32.const 0) (i32.const 1) (i32.const 10) (i32.const 5)) (then (return (i32.const 2))))
           ;; Refused by the host: status 1.
           (if (i32.eqz (call $setting_set (i32.const 0) (i32.const 1) (i32.const 20) (i32.const 14))) (then (return (i32.const 3))))
           (if (call $setting_get (i32.const 0) (i32.const 1)) (then (return (i32.const 4))))
           (call $set_output (i32.const 100) (call $reply_read (i32.const 100) (call $reply_len)))
           (call $setting_delete (i32.const 0) (i32.const 1))
           (if (i32.eqz (call $setting_get (i32.const 0) (i32.const 1))) (then (return (i32.const 5))))
           (i32.const 0)"#,
    );
    let mut host = Settings::default();
    let out = crate::run(&m, "{}", &Limits::default(), &mut host).unwrap();
    assert_eq!(out.output, "[1,2]");
    assert!(host.0.is_empty());
    // Settings cost host calls like any other.
    let limits = Limits { max_calls: 2, ..Limits::default() };
    assert!(matches!(crate::run(&m, "{}", &limits, &mut host), Err(PluginError::Limit(_))));
}

#[test]
fn panels_are_validated_and_keep_entered_values() {
    let p = Panel::parse(
        r##"{"title": "Swatches", "controls": [
            {"type": "list", "name": "palette", "label": "Palette", "options": ["Warm", "Cool"], "value": "Cool"},
            {"type": "note", "label": "Pick a palette"},
            {"type": "color", "name": "accent", "value": "#f0a"},
            {"type": "number", "name": "n", "min": 1, "max": 4, "value": 9},
            {"type": "text", "name": "t"},
            {"type": "slider", "name": "o", "min": 0, "max": 100, "value": -5},
            {"type": "button", "name": "go", "label": "Go"}
        ]}"##,
    )
    .unwrap();
    assert_eq!(p.control("accent").unwrap().default, Some(json!("#FF00AA")));
    assert_eq!(p.control("n").unwrap().default, Some(json!(4.0)), "values are clamped");
    assert_eq!(p.control("go").unwrap().kind, ParamKind::Button);
    let o = p.control("o").unwrap();
    assert_eq!((o.kind, o.default.clone()), (ParamKind::Slider, Some(json!(0.0))), "a slider is clamped like a number");
    assert_eq!(o.coerce(&json!("250")), Ok(json!(100.0)));
    assert!(o.coerce(&json!(true)).is_err());
    let mut old = Map::new();
    old.insert("t".into(), json!("typed"));
    old.insert("palette".into(), json!("Warm"));
    assert_eq!(
        json!(p.values(&old)),
        json!({"palette": "Cool", "accent": "#FF00AA", "n": 4.0, "t": "typed", "o": 0.0}),
        "the spec's values win, then the entered ones"
    );
    let deep = format!("{{\"controls\": {}{}}}", "[".repeat(5000), "]".repeat(5000));
    let many = json!({"controls": (0..65).map(|i| json!({"type": "button", "name": format!("b{i}")})).collect::<Vec<_>>()}).to_string();
    let junk = [
        "",
        "null",
        r#"{"controls": 5}"#,
        r#"{"controls": [{"type": "rocket", "name": "x"}]}"#,
        r#"{"controls": [{"type": "button"}]}"#,
        r#"{"controls": [{"type": "list", "name": "x"}]}"#,
        r#"{"controls": [{"type": "button", "name": "a"}, {"type": "text", "name": "a"}]}"#,
        r#"{"controls": [{"type": "number", "name": "a", "min": 3, "max": 1}]}"#,
        r#"{"controls": [{"type": "slider", "name": "a", "min": 3}]}"#,
        r#"{"controls": [{"type": "slider", "name": "a", "max": 3}]}"#,
        r#"{"title": "line\nbreak"}"#,
        &deep,
        &many,
    ];
    for j in junk {
        assert!(Panel::parse(j).is_err(), "accepted {}", j.chars().take(60).collect::<String>());
    }
    let huge = format!(r#"{{"controls": [{{"type": "note", "label": "{}"}}]}}"#, "x".repeat(70_000));
    assert!(Panel::parse(&huge).unwrap_err().contains("larger"));
    let long_note = format!(r#"{{"controls": [{{"type": "note", "label": "{}"}}]}}"#, "x".repeat(1001));
    assert!(Panel::parse(&long_note).is_err());
    // Junk values are dropped, not kept.
    let p = Panel::parse(
        r#"{"controls": [{"type": "color", "name": "c", "value": {"deep": [1, 2]}}, {"type": "boolean", "name": "b", "value": "yes"}]}"#,
    )
    .unwrap();
    assert_eq!(json!(p.values(&Map::new())), json!({"c": "#000000", "b": false}));
    // Buttons and notes don't go in manifests; colours and lists do.
    let manifest = |kind: &str| {
        let params = json!([{"name": "x", "type": kind, "options": ["a"]}]);
        Manifest::from_value(&json!({"id": "a.b", "name": "A", "main": "a.wasm", "commands": [{"id": "c", "label": "C", "params": params}]}))
    };
    assert!(manifest("button").is_err() && manifest("note").is_err());
    assert!(manifest("color").is_ok() && manifest("list").is_ok());
}
