//! `screenshot` with `ids` must honour `maxSize` for the rendered paint area (shadows, overflow).
use crate::common;
use common::{Client, png_dims};
use serde_json::{Value, json};

fn dimensions(client: &mut Client, args: Value) -> (u32, u32) {
    let result = client.call("screenshot", args);
    assert_eq!(result["isError"], false, "{result}");
    let size = png_dims(result["content"][0]["data"].as_str().unwrap());
    let meta: Value = serde_json::from_str(result["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(meta["width"], size.0, "{meta}");
    assert_eq!(meta["height"], size.1, "{meta}");
    size
}

#[test]
fn layer_screenshots_fit_the_requested_size() {
    let mut shadow_client = Client::new();
    let shadow = shadow_client.create(json!({
        "type": "rectangle", "x": 0, "y": 0,
        "width": 100, "height": 100,
        "props": {"effects": [{
            "type": "dropShadow", "color": "#00000080",
            "offset": [0, 40], "radius": 60
        }]}
    }));
    let shadow_size = dimensions(&mut shadow_client, json!({"ids": [shadow], "maxSize": 100}));

    let mut frame_client = Client::new();
    let frame = frame_client.create(json!({
        "type": "frame", "x": 0, "y": 0,
        "width": 100, "height": 100, "props": {"clip": false}
    }));
    frame_client.create(json!({
        "type": "rectangle", "parent": frame,
        "x": 900, "y": 900, "width": 100, "height": 100
    }));
    let frame_size = dimensions(&mut frame_client, json!({"ids": [frame], "maxSize": 100}));

    assert!(shadow_size.0 <= 100 && shadow_size.1 <= 100, "shadow screenshot exceeds maxSize: {shadow_size:?}");
    assert!(shadow_size.0 >= 90 && shadow_size.1 >= 90, "shadow screenshot shrank too far: {shadow_size:?}");
    assert!(frame_size.0 <= 100 && frame_size.1 <= 100, "frame screenshot exceeds maxSize: {frame_size:?}");
    assert!(frame_size.0 >= 90 && frame_size.1 >= 90, "frame screenshot shrank too far: {frame_size:?}");
}

#[test]
fn plain_layer_screenshot_sizes_are_unchanged() {
    let mut client = Client::new();
    let rect = client.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100}));
    assert_eq!(dimensions(&mut client, json!({"ids": [rect], "maxSize": 100})), (100, 100));
    assert_eq!(dimensions(&mut client, json!({"ids": [rect], "scale": 2})), (200, 200));
}
