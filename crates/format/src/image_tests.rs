//! Images a native file can't give back: damaged or missing members are reported, not dropped
//! without a word.

use std::io::Write;

use figcraft_doc::{Document, NodeKind};

use super::*;

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 0, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn doc_with_image() -> (Document, String) {
    let mut d = Document::new("File");
    let p = d.first_page().unwrap();
    d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    let (hash, ..) = add_image(&mut d, png()).unwrap();
    (d, hash)
}

/// The archive of `d` rebuilt with `edit` applied to each member (None drops it).
fn rebuild(bytes: &[u8], edit: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = z.file_names().map(str::to_string).collect();
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    for n in names {
        let data = read_member(&mut z, &n, u64::MAX).unwrap();
        if let Some(data) = edit(&n, data) {
            w.start_file(n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            w.write_all(&data).unwrap();
        }
    }
    w.finish().unwrap();
    buf.into_inner()
}

#[test]
fn an_intact_file_loads_without_image_issues() {
    let (d, hash) = doc_with_image();
    let (loaded, issues) = load_bytes(&save_bytes(&d, None).unwrap()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(loaded.images.contains_key(&hash));
}

#[test]
fn a_missing_image_member_is_reported() {
    let (d, hash) = doc_with_image();
    let bytes = rebuild(&save_bytes(&d, None).unwrap(), |n, data| (!n.starts_with("images/")).then_some(data));
    let (loaded, issues) = load_bytes(&bytes).unwrap();
    assert!(!loaded.images.contains_key(&hash));
    assert!(issues.iter().any(|i| i.contains(&hash) && i.contains("missing") && i.contains("drops it")), "{issues:?}");
}

#[test]
fn a_damaged_image_member_is_reported_with_its_reason() {
    let (d, hash) = doc_with_image();
    let bytes = save_bytes(&d, None).unwrap();
    // The member's bytes change but not its checksum: reading it fails.
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let name = format!("images/{hash}");
    let stored = read_member(&mut z, &name, u64::MAX).unwrap();
    let at = bytes.windows(stored.len()).position(|w| w == stored).unwrap();
    let mut damaged = bytes.clone();
    damaged[at + stored.len() / 2] ^= 0xff;
    let (loaded, issues) = load_bytes(&damaged).unwrap();
    assert!(!loaded.images.contains_key(&hash));
    assert!(issues.iter().any(|i| i.contains(&hash) && i.contains("couldn't be read") && i.contains("drops it")), "{issues:?}");
}

#[test]
fn many_lost_images_are_counted_not_listed_one_by_one() {
    let mut d = Document::new("File");
    for k in 0..40u8 {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([k, 1, 0, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        add_image(&mut d, out).unwrap();
    }
    let bytes = rebuild(&save_bytes(&d, None).unwrap(), |n, data| (!n.starts_with("images/")).then_some(data));
    let (_, issues) = load_bytes(&bytes).unwrap();
    assert_eq!(issues.len(), MAX_IMAGE_ISSUES + 1, "{issues:?}");
    assert_eq!(issues.last().map(String::as_str), Some("20 more images couldn't be loaded"));
}

#[test]
fn image_members_with_path_like_names_are_dropped_and_not_saved_back() {
    let (d, hash) = doc_with_image();
    let bytes = save_bytes(&d, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let doc_json = read_member(&mut z, "document.json", u64::MAX).unwrap();
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("document.json", opts).unwrap();
    w.write_all(&doc_json).unwrap();
    for n in ["images/../../x", "images/a/b"] {
        w.start_file(n, opts).unwrap();
        w.write_all(&png()).unwrap();
    }
    w.finish().unwrap();
    let hostile = buf.into_inner();
    let loaded = load_bytes_with_versions(&hostile).unwrap();
    assert!(loaded.doc.images.keys().all(|k| k != "../../x" && k != "a/b"), "{:?}", loaded.doc.images.keys());
    assert!(!loaded.issues.is_empty());
    let saved = save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
    assert!(z.file_names().all(|n| !n.contains("..") && n.matches('/').count() <= 1), "{:?}", z.file_names().collect::<Vec<_>>());
    let _ = hash;
}

/// An unresolved `imageRef` from a hostile REST file never becomes the image's own key: the
/// save writer names archive members after keys, so `../../evil` would inject an
/// `images/../../evil` member into the next save. Such refs store the shared placeholder
/// under its own key instead.
#[test]
fn a_path_like_rest_image_ref_never_names_an_archive_member() {
    let rect = json!({"id": "X", "name": "X", "type": "RECTANGLE", "relativeTransform": [[1, 0, 0], [0, 1, 0]], "size": {"x": 10, "y": 10},
        "fills": [{"type": "IMAGE", "imageRef": "../../evil", "scaleMode": "FIT"}]});
    let file = json!({"name": "Hostile", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT",
        "children": [{"id": "0:Page", "name": "Page", "type": "CANVAS", "backgroundColor": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}, "children": [rect]}]}});
    let (doc, _warnings) = crate::rest::import(&file, &crate::rest::no_images).unwrap();
    assert!(!doc.images.contains_key("../../evil"), "the raw imageRef must not name an image: {:?}", doc.images.keys().collect::<Vec<_>>());
    assert!(doc.images.contains_key("rest-image-missing"), "the shared placeholder stores it instead: {:?}", doc.images.keys().collect::<Vec<_>>());
    let bytes = save_bytes(&doc, None).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    assert!(z.file_names().all(|n| !n.contains("..") && n.matches('/').count() <= 1), "{:?}", z.file_names().collect::<Vec<_>>());
    assert!(
        z.file_names().any(|n| n == "images/rest-image-missing"),
        "the placeholder is saved under its own name: {:?}",
        z.file_names().collect::<Vec<_>>()
    );
}

/// A key with `-` or `_` is a plain archive member name too: the REST importer's placeholder
/// (`rest-image-missing`) comes back after a save and load instead of being dropped as damaged.
#[test]
fn an_image_keyed_with_dashes_comes_back_after_a_save() {
    let mut d = Document::new("t");
    d.images.insert("rest-image-missing".into(), figcraft_doc::ImageData { asset: None, bytes: std::sync::Arc::new(png()), width: 2, height: 2 });
    let loaded = load_bytes_with_versions(&save_bytes(&d, None).unwrap()).unwrap();
    assert!(loaded.doc.images.contains_key("rest-image-missing"), "{:?}", loaded.issues);
}

/// An unresolved `imageRef` shaped like a content hash would claim the placeholder's bytes hash
/// to it, which they don't: a live session would refuse that image. It shares the placeholder
/// instead.
#[test]
fn a_hash_shaped_rest_image_ref_never_names_its_placeholder() {
    for image_ref in ["ab".repeat(32), "A".repeat(64)] {
        let rect = json!({"id": "X", "name": "X", "type": "RECTANGLE", "relativeTransform": [[1, 0, 0], [0, 1, 0]], "size": {"x": 10, "y": 10},
            "fills": [{"type": "IMAGE", "imageRef": image_ref, "scaleMode": "FIT"}]});
        let file = json!({"name": "F", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT",
            "children": [{"id": "0:Page", "name": "Page", "type": "CANVAS", "backgroundColor": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}, "children": [rect]}]}});
        let (doc, _) = crate::rest::import(&file, &crate::rest::no_images).unwrap();
        assert_eq!(doc.images.keys().collect::<Vec<_>>(), ["rest-image-missing"], "{image_ref}");
    }
}

#[test]
fn review_ordinary_save_strips_scopes_from_main_versions_and_branches() {
    let mut doc = Document::new("Synthetic portable history");
    let (key, _, _) = add_image(&mut doc, png()).unwrap();
    let image = doc.images.get_mut(&key).unwrap();
    let mut asset = image.reference();
    asset.scope = figcraft_assets::file_scope("wss://history.example/", "file");
    image.asset = Some(asset.clone());
    let version = Version::new("snapshot", "V", "", 0, Arc::new(doc.clone()));
    let branch = Branch {
        id: "branch".into(),
        name: "B".into(),
        time: 0,
        client: 1,
        state: BranchState::Active,
        base: Arc::new(doc.clone()),
        head: Arc::new(doc.clone()),
        versions: vec![version.clone()],
    };
    let recovery = save_recovery_bytes_with_history(&doc, &[version], &[branch]).unwrap();
    let loaded = load_recovery_bytes_with_versions(&recovery).unwrap();
    let saved = save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    for i in 0..zip.len() {
        use std::io::Read;
        let mut member = zip.by_index(i).unwrap();
        if member.name().ends_with(".json") {
            let mut text = String::new();
            member.read_to_string(&mut text).unwrap();
            assert!(!text.contains("remoteImageIndex"), "{}", member.name());
            assert!(!text.contains(&asset.scope), "{}", member.name());
        }
    }
    let portable = load_bytes_with_versions(&saved).unwrap();
    assert!(portable.issues.is_empty(), "{:?}", portable.issues);
    assert_eq!(portable.doc.images[&key].bytes, doc.images[&key].bytes);
    assert!(portable.versions[0].document().unwrap().images[&key].asset.is_none());
    assert!(portable.branches[0].head.images[&key].asset.is_none());
    assert!(doc.images[&key].asset.is_some(), "saving must not mutate the source");
}

#[test]
fn review_followup_recovery_cannot_introduce_path_like_image_keys() {
    let asset = figcraft_assets::AssetRef::of(b"synthetic recovery image");
    for key in ["../escape", "a/b", "", &"x".repeat(129)] {
        let mut value = to_json(&Document::new("Synthetic recovery names")).unwrap();
        value["remoteImageIndex"] = json!({key: {"width": 1, "height": 1, "asset": asset}});
        assert!(load_recovery_bytes_with_versions(&serde_json::to_vec(&value).unwrap()).is_err(), "{key:?}");
    }
    let mut value = to_json(&Document::new("Valid recovery name")).unwrap();
    value["remoteImageIndex"] = json!({"valid-name_1": {"width": 1, "height": 1, "asset": asset}});
    assert!(load_recovery_bytes_with_versions(&serde_json::to_vec(&value).unwrap()).is_ok());
}

#[test]
fn review_final_ordinary_archive_cannot_hide_a_remote_scope_behind_a_history_image() {
    use std::io::Read;
    let doc = Document::new("Synthetic scope smuggling");
    let mut history = doc.clone();
    let (key, _, _) = add_image(&mut history, png()).unwrap();
    let version = Version::new("v", "Version", "", 0, Arc::new(history));
    let archive = save_bytes_with_versions(&doc, None, &[version]).unwrap();
    assert!(load_bytes_with_versions(&archive).is_ok());
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(&archive)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut entry = input.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name == "document.json" {
            let mut value: Value = serde_json::from_slice(&data).unwrap();
            let mut foreign = figcraft_assets::AssetRef::of(b"other file's cached original");
            foreign.scope = figcraft_assets::file_scope("wss://scope.example/", "other");
            value["remoteImageIndex"] = json!({key.clone(): {"width": 2, "height": 2, "asset": foreign}});
            value["imageIndex"] = json!([]); // K exists only in the version and archive members.
            data = serde_json::to_vec(&value).unwrap();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&data).unwrap();
    }
    let hostile = out.finish().unwrap().into_inner();
    assert!(load_bytes_with_versions(&hostile).unwrap_err().to_string().contains("external cache"));
}
