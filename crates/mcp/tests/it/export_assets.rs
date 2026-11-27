//! `export_assets`: the original bytes land in the folder asked for, and nothing is written without one.

use base64::Engine as _;
use serde_json::json;

use crate::common::Client;

fn png() -> Vec<u8> {
    let img = image::RgbImage::from_fn(8, 4, |x, y| image::Rgb([x as u8 * 30, y as u8 * 60, 7]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-mcp-assets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn originals_are_written_where_asked_and_listed_without_a_folder() {
    let mut c = Client::new();
    let bytes = png();
    let placed = c.ok("run_command", json!({"command": "image.place", "params": {"data": base64::engine::general_purpose::STANDARD.encode(&bytes)}}));
    let id = placed["id"].clone();
    let listed = c.ok("export_assets", json!({"ids": [id], "dryRun": true}));
    assert_eq!(listed["assets"][0]["source"], "original");
    assert!(listed["assets"][0].get("path").is_none(), "a dry run writes nothing");
    let refused = c.call("export_assets", json!({"ids": [id]}));
    assert_eq!(refused["isError"], true, "no folder, no write: {refused}");
    let dir = folder("written");
    let done = c.ok("export_assets", json!({"ids": [id], "dir": dir}));
    let path = done["assets"][0]["path"].as_str().unwrap().to_string();
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "byte for byte");
    assert_eq!(c.ok("export_assets", json!({"ids": [id], "dir": dir}))["written"], 0, "nothing new the second time");
}
