//! `compare_design`: a capture of the frame itself matches it, and the capture is required.

use base64::Engine as _;
use serde_json::json;

use crate::common::Client;

#[test]
fn a_capture_of_the_frame_matches_and_one_is_required() {
    let mut c = Client::new();
    let frame = c
        .ok("run_command", json!({"command": "node.create", "params": {"type": "frame", "width": 200, "height": 100, "props": {"fill": "#FFFFFF"}}}));
    let id = frame["id"].clone();
    let png = c.ok("run_command", json!({"command": "file.export", "params": {"ids": [id], "format": "png"}}));
    let bytes = base64::engine::general_purpose::STANDARD.decode(png["data"].as_str().unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("figcraft-mcp-compare-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let shot = dir.join("shot.png");
    std::fs::write(&shot, bytes).unwrap();
    let v = c.ok("compare_design", json!({"id": id, "image": shot}));
    assert_eq!(v["total"], 0, "{v}");
    assert_eq!(v["viewport"], json!({"width": 200.0, "height": 100.0}));
    for args in [json!({"id": id}), json!({"id": id, "image": "  "}), json!({"id": id, "image": 3})] {
        assert_eq!(c.call("compare_design", args.clone())["isError"], true, "{args}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
