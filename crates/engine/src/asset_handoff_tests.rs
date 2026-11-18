//! `file.exportAssets`: originals stay byte for byte, names come from content, exports repeat cleanly.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;
use crate::file_access::{Roots, confined};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-assets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

/// An image file in `format`, with a little metadata re-encoding would lose: `seed` varies it.
fn image_bytes(format: image::ImageFormat, seed: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(16, 8, |x, y| image::Rgb([x as u8 * 16, y as u8 * 32, seed]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
    out
}

fn place(s: &mut Session, bytes: &[u8]) -> u64 {
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    s.execute("image.place", &json!({"data": data})).unwrap()["id"].as_u64().unwrap()
}

fn assets(v: &Value) -> &Vec<Value> {
    v["assets"].as_array().unwrap()
}

fn export(s: &mut Session, params: Value) -> Value {
    s.execute("file.exportAssets", &params).unwrap()
}

#[test]
fn images_are_the_files_own_bytes_named_by_their_content() {
    let (mut s, dir) = (session(), folder("original"));
    for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
        let bytes = image_bytes(format, 7);
        let id = place(&mut s, &bytes);
        let v = export(&mut s, json!({"ids": [id], "dir": dir}));
        let a = &assets(&v)[0];
        assert_eq!(a["source"], "original");
        let hash = figcraft_format::image_hash(&bytes);
        assert!(a["file"].as_str().unwrap().starts_with(&hash[..16]), "{a}");
        assert_eq!(std::fs::read(a["path"].as_str().unwrap()).unwrap(), bytes, "not re-encoded");
        assert_eq!((a["width"].as_u64(), a["height"].as_u64()), (Some(16), Some(8)));
    }
}

#[test]
fn the_same_image_twice_is_one_file_listing_both_layers() {
    let (mut s, dir) = (session(), folder("dedupe"));
    let bytes = image_bytes(image::ImageFormat::Png, 1);
    let (a, b) = (place(&mut s, &bytes), place(&mut s, &bytes));
    let v = export(&mut s, json!({"ids": [a, b], "dir": dir}));
    assert_eq!(assets(&v).len(), 1);
    assert_eq!(assets(&v)[0]["layers"].as_array().unwrap().len(), 2);
    assert_eq!(v["written"], 1);
}

#[test]
fn exporting_again_writes_nothing_and_names_dont_move() {
    let (mut s, dir) = (session(), folder("again"));
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 2));
    let first = export(&mut s, json!({"ids": [id], "dir": dir}));
    let path = assets(&first)[0]["path"].as_str().unwrap().to_string();
    let when = std::fs::metadata(&path).unwrap().modified().unwrap();
    let second = export(&mut s, json!({"ids": [id], "dir": dir}));
    assert_eq!((second["written"].as_u64(), second["reused"].as_u64()), (Some(0), Some(1)));
    assert_eq!(assets(&second)[0]["file"], assets(&first)[0]["file"]);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), when);
}

#[test]
fn a_damaged_file_at_the_name_is_put_right() {
    let (mut s, dir) = (session(), folder("damaged"));
    let bytes = image_bytes(image::ImageFormat::Png, 3);
    let id = place(&mut s, &bytes);
    let v = export(&mut s, json!({"ids": [id], "dir": dir}));
    let path = assets(&v)[0]["path"].as_str().unwrap().to_string();
    std::fs::write(&path, vec![0u8; bytes.len()]).unwrap();
    assert_eq!(export(&mut s, json!({"ids": [id], "dir": dir}))["written"], 1);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn listing_writes_nothing() {
    let mut s = session();
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 4));
    let v = export(&mut s, json!({"ids": [id]}));
    assert_eq!(assets(&v).len(), 1);
    assert!(assets(&v)[0].get("path").is_none() && v.get("written").is_none());
}

#[test]
fn a_vector_is_svg_and_says_so() {
    let (mut s, dir) = (session(), folder("vector"));
    let id =
        s.execute("node.create", &json!({"type": "star", "width": 24, "height": 24, "props": {"fill": "#112233"}})).unwrap()["id"].as_u64().unwrap();
    let v = export(&mut s, json!({"ids": [id], "dir": dir}));
    let a = &assets(&v)[0];
    assert_eq!((a["source"].as_str(), a["format"].as_str()), (Some("vector"), Some("svg")));
    assert!(std::fs::read_to_string(a["path"].as_str().unwrap()).unwrap().contains("<svg"));
}

#[test]
fn a_layers_export_settings_are_a_render_and_its_image_stays_original() {
    let (mut s, dir) = (session(), folder("settings"));
    let bytes = image_bytes(image::ImageFormat::Png, 5);
    let id = place(&mut s, &bytes);
    s.execute("export.add", &json!({"ids": [id]})).unwrap();
    let v = export(&mut s, json!({"ids": [id], "dir": dir}));
    let sources: Vec<&str> = assets(&v).iter().map(|a| a["source"].as_str().unwrap()).collect();
    assert!(sources.contains(&"original") && sources.contains(&"rendered"), "{v}");
}

#[test]
fn what_a_layer_does_to_its_image_is_listed() {
    let mut s = session();
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 6));
    s.execute("image.adjust", &json!({"ids": [id], "preset": "vivid"})).unwrap();
    let v = export(&mut s, json!({"ids": [id]}));
    assert!(assets(&v)[0]["adjustments"][0]["changes"]["filters"].is_object(), "{v}");
}

#[test]
fn an_image_the_file_lacks_is_skipped_with_a_reason() {
    let mut s = session();
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 8));
    let doc = s.doc_mut().unwrap();
    let key = doc.images.keys().next().unwrap().clone();
    doc.images.insert(key, figcraft_doc::ImageData { bytes: Arc::new(vec![]), width: 0, height: 0 });
    let v = export(&mut s, json!({"ids": [id]}));
    assert!(assets(&v).is_empty());
    assert_eq!(v["skipped"][0]["id"], id);
    assert!(v["skipped"][0]["why"].as_str().unwrap().contains("doesn't hold"));
}

#[test]
fn hidden_layers_stay_out() {
    let mut s = session();
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 9));
    s.execute("node.set", &json!({"ids": [id], "props": {"visible": false}})).unwrap();
    assert!(assets(&export(&mut s, json!({"ids": [id]}))).is_empty());
}

#[test]
fn nothing_is_written_outside_the_project() {
    let (mut s, inside, outside) = (session(), folder("inside"), folder("outside"));
    let id = place(&mut s, &image_bytes(image::ImageFormat::Png, 10));
    let roots = Arc::new(Roots::new(&[], &[inside.to_string_lossy().into_owned()]).unwrap());
    let refused = confined(Some(&roots), || s.execute("file.exportAssets", &json!({"ids": [id], "dir": outside})));
    assert!(refused.unwrap_err().to_string().contains("automation path rejected"));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, inside.join("out")).unwrap();
        assert!(confined(Some(&roots), || s.execute("file.exportAssets", &json!({"ids": [id], "dir": inside.join("out/deep")}))).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }
    let ok = confined(Some(&roots), || s.execute("file.exportAssets", &json!({"ids": [id], "dir": inside.join("public/figcraft")})));
    assert_eq!(ok.unwrap()["written"], 1);
}

#[test]
fn junk_is_refused_without_a_panic() {
    let mut s = session();
    for p in [json!({"dir": ""}), json!({"dir": "   "}), json!({"dir": 3})] {
        assert!(s.execute("file.exportAssets", &p).is_err(), "{p}");
    }
    assert!(assets(&export(&mut s, json!({}))).is_empty());
}
