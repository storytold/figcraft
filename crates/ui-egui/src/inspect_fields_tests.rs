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
