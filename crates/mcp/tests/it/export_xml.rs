//! The `export` tool's Android Vector Drawable format: the XML as text, then its warnings.

use crate::common::Client;
use serde_json::{Value, json};

#[test]
fn xml_comes_back_as_text_and_its_warnings() {
    let mut c = Client::new();
    let id = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 24, "height": 16}));
    let r = c.call("export", json!({"ids": [id], "format": "xml"}));
    assert_eq!(r["isError"], false, "{r}");
    let blocks = r["content"].as_array().unwrap();
    let xml = blocks[0]["text"].as_str().unwrap();
    assert!(xml.starts_with("<?xml") && xml.contains("android:width=\"24dp\"") && xml.contains("android:height=\"16dp\""), "{xml}");
    let info: Value = serde_json::from_str(blocks[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!((info["format"].as_str(), info["warnings"].as_array().map(Vec::len)), (Some("xml"), Some(0)));

    c.ok("set_properties", json!({"ids": [id], "props": {"effects": [{"type": "dropShadow"}]}}));
    let r = c.call("export", json!({"ids": [id], "format": "xml"}));
    let info: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["warnings"], json!(["drop shadows were left out (Rectangle)"]), "{info}");
    let r = c.call("export", json!({"ids": [id], "format": "xml", "vdCompat": "ignore"}));
    let info: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["warnings"], json!([]));

    assert!(c.err("export", json!({"ids": [id], "format": "xml", "vdCompat": "loose"})).contains("vdCompat"));
}
