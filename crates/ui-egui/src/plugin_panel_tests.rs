//! Plugin panels in the UI, driven headless: their controls are drawn with names a screen reader
//! reads, pressing its button calls the plugin back, closing it ends the plugin's session, and an
//! open panel draws no frames while nothing changes.

use base64::Engine as _;
use egui::Pos2;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::colorpicker::PickerTarget;
use crate::idle_tests::Harness;
use crate::layers_drag_tests::Ui;

/// A plugin whose first call shows a panel with every kind of control and whose later calls (the
/// panel's events) make a rectangle.
fn panel_module() -> Vec<u8> {
    let spec = json!({"title": "Test panel", "controls": [
        {"type": "note", "label": "Hello from the plugin"},
        {"type": "number", "name": "size", "label": "Size", "value": 10},
        {"type": "boolean", "name": "labels", "label": "Labels"},
        {"type": "list", "name": "palette", "label": "Palette", "options": ["Warm", "Cool"]},
        {"type": "color", "name": "accent", "label": "Accent", "value": "#FF5A5F"},
        {"type": "select", "name": "shape", "label": "Shape", "options": ["Square", "Circle"]},
        {"type": "text", "name": "title", "label": "Title"},
        {"type": "button", "name": "go", "label": "Go"}
    ]})
    .to_string();
    let rect = r#"{"x":0,"y":0,"width":10,"height":10}"#;
    let wat = format!(
        r#"(module
          (import "figcraft" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "figcraft" "ui_show" (func $ui_show (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 1000) "{}") (data (i32.const 100) "shape.rectangle") (data (i32.const 200) "{}")
          (func (export "figcraft_run") (param $n i32) (result i32)
            (i32.store (i32.const 0) (i32.add (i32.load (i32.const 0)) (i32.const 1)))
            (if (i32.eq (i32.load (i32.const 0)) (i32.const 1))
              (then (return (call $ui_show (i32.const 1000) (i32.const {})))))
            (call $call (i32.const 100) (i32.const 15) (i32.const 200) (i32.const {}))))"#,
        spec.replace('"', "\\\""),
        rect.replace('"', "\\\""),
        spec.len(),
        rect.len()
    );
    wat::parse_str(&wat).unwrap()
}

/// A session with that plugin's panel open.
fn session() -> Session {
    let wasm = panel_module();
    let manifest = json!({"id": "org.test.panel", "name": "Panel test", "main": "p.wasm", "api": 2, "permissions": {"access": "write"}});
    let mut s = Session::new();
    s.execute("plugin.import", &json!({"manifest": manifest, "wasm": base64::engine::general_purpose::STANDARD.encode(wasm)})).unwrap();
    s.execute("plugin.run", &json!({"id": "org.test.panel"})).unwrap();
    s
}

fn layers(u: &Ui) -> usize {
    let ds = u.app.session.ds().unwrap();
    ds.doc.node(ds.page).unwrap().children.len()
}

#[test]
fn the_panel_names_its_controls_and_its_button_calls_the_plugin_back() {
    let mut u = Ui::new(session());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let tree = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    let named = |label: &str| tree.iter().map(|(_, n)| n).find(|n| n.label() == Some(label)).cloned();
    for label in ["Size", "Labels", "Warm", "Cool", "Accent", "Shape", "Title", "Go"] {
        assert!(named(label).is_some(), "no control named {label:?}");
    }
    let go = named("Go").and_then(|n| n.bounds()).unwrap();
    let at = Pos2::new(((go.x0 + go.x1) / 2.0) as f32, ((go.y0 + go.y1) / 2.0) as f32);
    let before = layers(&u);
    u.button(at, true);
    u.button(at, false);
    assert_eq!(layers(&u), before + 1, "Go made the plugin draw a rectangle");
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), 1);
    assert_eq!(u.app.session.plugins.panels.len(), 1, "the panel stays open");
    // Its close button ends the session.
    u.app.run("plugin.close", json!({"id": "org.test.panel"}));
    u.frame(vec![]);
    assert_eq!(u.app.session.execute("plugin.ui", &json!({})).unwrap(), Value::Null);
}

#[test]
fn an_open_plugin_panel_draws_no_frames_while_idle() {
    Harness::new(session()).assert_idle("a plugin panel");
}

#[test]
fn a_colour_field_opens_the_colour_picker_and_two_panels_show_side_by_side() {
    let mut u = Ui::new(session());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes = |u: &mut Ui| u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    // The swatch beside a colour field opens the colour picker on it.
    let swatch = nodes(&mut u).iter().map(|(_, n)| n).find(|n| n.label() == Some("Color")).and_then(|n| n.bounds()).unwrap();
    let at = Pos2::new(((swatch.x0 + swatch.x1) / 2.0) as f32, ((swatch.y0 + swatch.y1) / 2.0) as f32);
    u.button(at, true);
    u.button(at, false);
    let picked = Some(PickerTarget::PluginField { plugin: "org.test.panel".into(), control: "accent".into(), pending: None });
    assert_eq!(u.app.ui.picker, picked);
    u.frame(vec![]);
    assert_eq!(u.app.ui.picker, picked, "the picker shows the field's colour");
    // A colour picked there calls the plugin back with it.
    let before = layers(&u);
    crate::plugins_ui::set_field(&mut u.app, "org.test.panel", "accent", "#00FF00");
    assert_eq!(u.app.session.plugins.panel("org.test.panel").unwrap().values["accent"], "#00FF00");
    assert_eq!(layers(&u), before + 1);
    // A second plugin's panel opens beside the first.
    let other = json!({"id": "org.test.other", "name": "Other", "main": "p.wasm", "api": 2, "permissions": {"access": "write"}});
    let module = base64::engine::general_purpose::STANDARD.encode(panel_module());
    u.app.session.execute("plugin.import", &json!({"manifest": other, "wasm": module})).unwrap();
    u.app.session.execute("plugin.run", &json!({"id": "org.test.other"})).unwrap();
    u.frame(vec![]);
    let accents = nodes(&mut u).iter().filter(|(_, n)| n.label() == Some("Accent")).count();
    assert_eq!((accents, u.app.session.plugins.panels.len()), (2, 2), "both panels are drawn");
}

/// A plugin whose first call shows a panel with a grid of two vector cells.
fn grid_module() -> Vec<u8> {
    let spec = json!({"title": "Grid panel", "controls": [
        {"type": "grid", "name": "icon", "label": "Icons", "columns": 2, "items": [
            {"value": "heart", "path": "M12 21C7 16 3 13 3 9c0-3 2-5 4.5-5C10 4 12 7 12 7s2-3 4.5-3C19 4 21 6 21 9c0 4-4 7-9 12Z", "label": "Heart", "color": "#F24822"},
            {"value": "plus", "path": "M12 5v14M5 12h14", "label": "Plus", "color": "#111111", "stroke": 2.0}
        ]}
    ]})
    .to_string();
    let wat = format!(
        r#"(module
          (import "figcraft" "ui_show" (func $ui_show (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 1000) "{}")
          (func (export "figcraft_run") (param $n i32) (result i32)
            (drop (call $ui_show (i32.const 1000) (i32.const {})))
            (i32.const 0)))"#,
        spec.replace('"', "\\\""),
        spec.len()
    );
    wat::parse_str(&wat).unwrap()
}

#[test]
fn a_panel_grid_names_its_cells_and_a_click_picks_one() {
    let manifest = json!({"id": "org.test.grid", "name": "Grid test", "main": "p.wasm", "api": 4, "permissions": {}});
    let mut s = Session::new();
    s.execute("plugin.import", &json!({"manifest": manifest, "wasm": base64::engine::general_purpose::STANDARD.encode(grid_module())}))
        .unwrap();
    s.execute("plugin.run", &json!({"id": "org.test.grid"})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes = |u: &mut Ui| u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    for label in ["Heart", "Plus"] {
        assert!(nodes(&mut u).iter().any(|(_, n)| n.label() == Some(label)), "no cell named {label:?}");
    }
    let heart = nodes(&mut u).iter().map(|(_, n)| n).find(|n| n.label() == Some("Heart")).and_then(|n| n.bounds()).unwrap();
    let at = Pos2::new(((heart.x0 + heart.x1) / 2.0) as f32, ((heart.y0 + heart.y1) / 2.0) as f32);
    u.button(at, true);
    u.button(at, false);
    assert_eq!(u.app.session.plugins.panel("org.test.grid").unwrap().values["icon"], "heart");
    let plus = nodes(&mut u).iter().map(|(_, n)| n).find(|n| n.label() == Some("Plus")).and_then(|n| n.bounds()).unwrap();
    let at = Pos2::new(((plus.x0 + plus.x1) / 2.0) as f32, ((plus.y0 + plus.y1) / 2.0) as f32);
    u.button(at, true);
    u.button(at, false);
    assert_eq!(u.app.session.plugins.panel("org.test.grid").unwrap().values["icon"], "plus");
}
