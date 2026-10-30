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
pub(crate) fn panel_module() -> Vec<u8> {
    let spec = json!({"title": "Test panel", "controls": [
        {"type": "note", "label": "Hello from the plugin"},
        {"type": "number", "name": "size", "label": "Size", "value": 10},
        {"type": "boolean", "name": "labels", "label": "Labels"},
        {"type": "list", "name": "palette", "label": "Palette", "options": ["Warm", "Cool"]},
        {"type": "color", "name": "accent", "label": "Accent", "value": "#FF5A5F"},
        {"type": "select", "name": "shape", "label": "Shape", "options": ["Square", "Circle"]},
        {"type": "text", "name": "title", "label": "Title"},
        {"type": "image", "name": "logo", "label": "Brand mark", "data": png(40, 20)},
        {"type": "image", "label": "Broken chart", "data": "bm90IGFuIGltYWdl"},
        {"type": "image", "label": "Too wide", "data": png(5000, 1)},
        {"type": "slider", "name": "opacity", "label": "Opacity", "min": 0, "max": 100, "value": 0},
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

/// A PNG of `w` × `h` pixels, base64.
fn png(w: u32, h: u32) -> String {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([255, 90, 95, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(out)
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
    for label in ["Size", "Labels", "Warm", "Cool", "Accent", "Shape", "Title", "Opacity", "Go"] {
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

/// Asserts a plugin colour field's swatch opens the picker on that field, a picked colour
/// reaches the plugin, and a second plugin's panel opens while the first remains (two panels
/// present).
#[test]
fn a_colour_field_opens_the_colour_picker_and_two_panels_show_side_by_side() {
    let mut u = Ui::new(session());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes = |u: &mut Ui| u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    // The swatch beside the accent field (named for that field, not just "Color") opens the
    // colour picker on the plugin field, not on another colour control in the window.
    let swatch = nodes(&mut u).iter().map(|(_, n)| n).find(|n| n.label() == Some("Accent Color")).and_then(|n| n.bounds()).unwrap();
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

/// A panel shows its pictures at their size, named for screen readers; one that can't be decoded,
/// or is too large, shows its label instead; an image isn't a control `plugin.event` can use.
#[test]
fn panel_images_show_named_or_fall_back_to_their_label() {
    let mut u = Ui::new(session());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    let images: Vec<_> = nodes.iter().map(|(_, n)| n).filter(|n| n.role() == egui::accesskit::Role::Image).collect();
    assert_eq!(images.len(), 1, "only the decodable picture is drawn");
    assert_eq!(images[0].label(), Some("Brand mark"));
    let b = images[0].bounds().unwrap();
    assert_eq!(((b.x1 - b.x0).round(), (b.y1 - b.y0).round()), (40.0, 20.0), "shown at its size");
    let text =
        |label: &str| nodes.iter().any(|(_, n)| n.role() != egui::accesskit::Role::Image && (n.label() == Some(label) || n.value() == Some(label)));
    assert!(text("Broken chart") && text("Too wide"), "a picture that can't be shown shows its label");
    let err = u.app.session.execute("plugin.event", &json!({"id": "org.test.panel", "control": "logo"})).unwrap_err();
    assert!(err.to_string().contains("no control `logo`"), "{err}");
}

/// A slider shows the value it is dragged to as it moves and calls the plugin back once, with
/// that value, when let go.
#[test]
fn a_slider_calls_the_plugin_back_once_when_let_go() {
    let mut u = Ui::new(session());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let slider = |u: &mut Ui| {
        let nodes = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
        nodes.into_iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::Slider && n.label() == Some("Opacity")).unwrap()
    };
    let s = slider(&mut u);
    assert_eq!((s.min_numeric_value(), s.max_numeric_value(), s.numeric_value()), (Some(0.0), Some(100.0), Some(0.0)));
    let b = s.bounds().unwrap();
    let y = ((b.y0 + b.y1) / 2.0) as f32;
    let (from, to) = (Pos2::new(b.x0 as f32 + 6.0, y), Pos2::new(((b.x0 + b.x1) / 2.0) as f32, y));
    let before = layers(&u);
    u.drag_to(from, to);
    let live = slider(&mut u).numeric_value().unwrap();
    assert!((live - 50.0).abs() < 2.0, "the knob follows the drag: {live}");
    assert_eq!(layers(&u), before, "no call while dragging");
    u.button(to, false);
    assert_eq!(layers(&u), before + 1, "one call when let go");
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), 1);
    let value = u.app.session.plugins.panel("org.test.panel").unwrap().values["opacity"].as_f64().unwrap();
    assert!((value - live).abs() < 1e-9, "the plugin got the dragged value: {value}");
}
