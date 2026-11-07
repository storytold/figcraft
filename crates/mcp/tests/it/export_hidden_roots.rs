//! Hidden top-level layers in an export take no room in the image (issue #447).

use crate::common;

use common::{Client, png_dims};
use serde_json::{Value, json};

fn dimensions(result: Value) -> (u32, u32) {
    assert_eq!(result["isError"], false, "{result}");
    let image = result["content"].as_array().unwrap().iter().find(|block| block["type"] == "image").unwrap();
    png_dims(image["data"].as_str().unwrap())
}

#[test]
fn hidden_root_does_not_expand_export() {
    let mut client = Client::new();
    let visible = client.create(json!({
        "type":"frame", "x":0, "y":0, "width":100, "height":100,
        "props":{"fill":"#FF0000"}
    }));
    let hidden = client.create(json!({
        "type":"frame", "x":900, "y":0, "width":300, "height":300,
        "props":{"visible":false}
    }));
    assert_eq!(client.node(&hidden)["visible"], false);

    let control = dimensions(client.call("export", json!({"ids":[visible], "format":"png", "size":"200w"})));
    assert_eq!(control, (200, 200));
    let mixed = dimensions(client.call("export", json!({"ids":[visible, hidden], "format":"png", "size":"200w"})));
    assert_eq!(mixed, control, "hidden roots must not enlarge the painted area");
    let natural = dimensions(client.call("export", json!({"ids":[visible, hidden], "format":"png"})));
    assert_eq!(natural, (100, 100));

    client.ok("select", json!({"ids":[visible, hidden]}));
    let selected = dimensions(client.call("export", json!({"format":"png", "size":"200w"})));
    assert_eq!(selected, (200, 200));
    client.ok("select", json!({"ids":[]}));
    let page = dimensions(client.call("export", json!({"format":"png"})));
    assert_eq!(page, (100, 100));

    // Controls: the bounding-box option and a hidden-only export keep their areas.
    let boxed = dimensions(client.call("export", json!({"ids":[visible, hidden], "format":"png", "includeBoundingBox":true})));
    assert_eq!(boxed, (1200, 300));
    let alone = dimensions(client.call("export", json!({"ids":[hidden], "format":"png"})));
    assert_eq!(alone, (300, 300));
}
