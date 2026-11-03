//! Attached control replies build compact layer trees before they leave the app.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    crate::control::handle(&mut u.app, &u.ctx.clone(), method, &params)
}

#[test]
fn control_inspection_filters_nodes_and_pages_before_replying() {
    let mut u = Ui::new(Session::new());
    let frame = call(&mut u, "engine.execute", json!({"command": "node.create", "params": {"type": "frame", "name": "Card"}})).unwrap()["id"].clone();
    let text =
        call(&mut u, "engine.execute", json!({"command": "node.create", "params": {"type": "text", "parent": frame, "text": "界".repeat(121)}}))
            .unwrap()["id"]
            .clone();
    let default = call(&mut u, "document.inspect", json!({"depth": 3})).unwrap();
    let selection = u.app.session.selection();
    let history = u.app.session.undo_depth();
    let compact = call(&mut u, "document.inspect", json!({"depth": 3, "fields": []})).unwrap();
    let page = &compact["pages"][0];
    assert!(page.get("name").is_none());
    assert_eq!(page["children"][0]["children"][0], json!({"id": text}));
    let shallow = call(&mut u, "node.get", json!({"id": frame, "depth": 0, "fields": []})).unwrap();
    assert_eq!(shallow, json!({"id": frame, "childCount": 1}));
    let outline = call(&mut u, "node.get", json!({"id": frame, "depth": 1, "outline": true})).unwrap();
    assert_eq!(outline["children"][0]["characters"], format!("{}...", "界".repeat(120)));
    assert_eq!(outline["visible"], true);
    assert!(outline.get("fills").is_none());
    assert_eq!(u.app.session.selection(), selection);
    assert_eq!(u.app.session.undo_depth(), history);
    assert_eq!(call(&mut u, "document.inspect", json!({"depth": 3})).unwrap(), default);
}

#[test]
fn control_inspection_rejects_invalid_options_consistently() {
    let mut u = Ui::new(Session::new());
    for method in ["document.inspect", "node.get"] {
        for mut params in [json!({"fields": ["Name"]}), json!({"fields": [2]}), json!({"outline": "true"}), json!({"outline": true, "fields": []})] {
            params["id"] = json!(u.app.session.page().unwrap());
            let error = call(&mut u, method, params).unwrap_err();
            assert!(error.contains("fields") || error.contains("field") || error.contains("outline"), "{error}");
        }
    }
}

/// `ui.key` with an arrow and nothing selected pans the view, as the window's own key path does
/// (before, it answered that nothing was selected); with a selection it nudges (Shift: farther).
#[test]
fn control_key_arrows_pan_with_nothing_selected() {
    let mut u = Ui::new(Session::new());
    let r =
        call(&mut u, "engine.execute", json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 40, "height": 40}})).unwrap()["id"]
            .clone();
    u.app.run("select.set", json!({"ids": []}));
    u.app.run("view.set", json!({"zoom": 2.0, "cx": 0, "cy": 0}));
    let view = |u: &Ui| {
        let v = &u.app.session.ds().unwrap().view;
        (v.cx, v.cy)
    };
    call(&mut u, "ui.key", json!({"key": "ArrowRight"})).unwrap();
    assert_eq!(view(&u), (10.0, 0.0), "20 screen pixels at 200%");
    call(&mut u, "ui.key", json!({"key": "ArrowUp", "shift": true})).unwrap();
    assert_eq!(view(&u), (10.0, -100.0), "Shift: ten times as far");
    u.app.run("select.set", json!({"ids": [r]}));
    call(&mut u, "ui.key", json!({"key": "ArrowRight"})).unwrap();
    assert_eq!(view(&u), (10.0, -100.0), "with a selection the view stays");
    let id = figcraft_engine::params::id_of(&r).unwrap();
    assert_eq!(u.app.session.doc().unwrap().world_bounds(id).unwrap().x0, 1.0, "the selection moved instead");
    // With a shape tool chosen and nothing selected the arrows move the placement crosshair
    // first (#1869), and the view stays.
    u.app.run("select.set", json!({"ids": []}));
    u.app.run("tool.rectangle", json!({}));
    let before = view(&u);
    call(&mut u, "ui.key", json!({"key": "ArrowRight"})).unwrap();
    assert_eq!(view(&u), before, "the crosshair, not a pan");
    assert!(u.app.session.crosshair().is_some());
}
