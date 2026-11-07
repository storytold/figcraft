//! The `export` tool's `colorProfile`: a PNG in another profile than the document's is tagged for
//! it, and a bad value is refused.

use base64::Engine as _;
use serde_json::json;

use crate::common::Client;

fn png(c: &mut Client, args: serde_json::Value) -> Vec<u8> {
    let r = c.call("export", args);
    assert_eq!(r["isError"], false, "{r}");
    let image = r["content"].as_array().unwrap().iter().find(|block| block["type"] == "image").unwrap();
    base64::engine::general_purpose::STANDARD.decode(image["data"].as_str().unwrap()).unwrap()
}

fn has_chunk(bytes: &[u8], kind: &[u8; 4]) -> bool {
    bytes.windows(4).any(|w| w == kind)
}

#[test]
fn color_profile_tags_the_png() {
    let mut c = Client::new();
    let id = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 24, "height": 16}));
    let srgb = png(&mut c, json!({"ids": [id], "format": "png"}));
    assert!(has_chunk(&srgb, b"sRGB") && !has_chunk(&srgb, b"iCCP"));
    let p3 = png(&mut c, json!({"ids": [id], "format": "png", "colorProfile": "displayP3"}));
    assert!(has_chunk(&p3, b"iCCP") && has_chunk(&p3, b"cICP") && !has_chunk(&p3, b"sRGB"));
    assert!(c.err("export", json!({"ids": [id], "format": "png", "colorProfile": "cmyk"})).contains("colorProfile"));
}
