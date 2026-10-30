//! One budget for everything a `.figcraft` file unpacks when it opens (#1079): the document,
//! images, version history and branches share it, beyond each area's own cap, and what passes it
//! is left out (or, for the document, refused) with a note that says so. Images left out this way
//! stay in the file like those past the images' own budget (#1313).

use std::io::Write;

use figcraft_doc::Document;
use sha2::Digest;

use crate::{IMAGE_BUDGET, MAX_UNPACKED, load_limited, to_json};

fn png(k: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([k, 7, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn zip_of(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut buf);
    for (name, bytes) in members {
        z.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap();
    buf.into_inner()
}

/// A crafted file: a document, two images, and a version history with one snapshot, each well
/// within its own cap. The member sizes, in that order.
fn crafted() -> (Vec<u8>, [u64; 5]) {
    let doc = serde_json::to_vec(&to_json(&Document::new("Crafted")).unwrap()).unwrap();
    let (a, b) = (png(1), png(2));
    let index = br#"{"versions": [{"id": "v1", "name": "First"}]}"#.to_vec();
    let snapshot = doc.clone();
    let sizes = [doc.len(), a.len(), b.len(), index.len(), snapshot.len()].map(|n| n as u64);
    let members = vec![
        ("document.json".to_string(), doc),
        (format!("images/{}", hash(&a)), a),
        (format!("images/{}", hash(&b)), b),
        ("versions/index.json".to_string(), index),
        ("versions/v1.json".to_string(), snapshot),
    ];
    (zip_of(&members), sizes)
}

#[test]
fn everything_within_the_total_loads() {
    let (bytes, sizes) = crafted();
    let loaded = load_limited(&bytes, IMAGE_BUDGET, sizes.iter().sum()).unwrap();
    assert_eq!(loaded.doc.images.len(), 2);
    assert_eq!(loaded.versions.len(), 1);
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
}

#[test]
fn what_passes_the_total_is_left_out_with_a_note() {
    let (bytes, [doc, a, b, index, _]) = crafted();
    // Room for the document and one image: the other image stays in the file, left out.
    let loaded = load_limited(&bytes, IMAGE_BUDGET, doc + a.max(b)).unwrap();
    assert_eq!(loaded.doc.images.len(), 1);
    assert_eq!(loaded.images_past_budget, 1, "kept in the file, so saving over it is refused");
    assert!(loaded.issues.iter().any(|i| i.contains("the file's contents together pass")), "{:?}", loaded.issues);
    // Room for the document, the images and the index, not the snapshot: the version is skipped.
    let loaded = load_limited(&bytes, IMAGE_BUDGET, doc + a + b + index).unwrap();
    assert_eq!(loaded.doc.images.len(), 2);
    assert!(loaded.versions.is_empty());
    let note = loaded.issues.iter().find(|i| i.contains("version `v1` was skipped")).unwrap();
    assert!(note.contains("the file's contents together pass"), "{note}");
}

#[test]
fn a_document_past_the_total_is_refused_with_the_reason() {
    let (bytes, [doc, ..]) = crafted();
    let Err(e) = load_limited(&bytes, IMAGE_BUDGET, doc - 1) else { panic!("loaded") };
    assert!(e.to_string().contains("document.json: too large: the file's contents together pass"), "{e}");
}

#[test]
fn the_total_leaves_room_for_the_largest_real_files() {
    // A 900 000-layer document unpacks to 1.5 GB; with images up to their own cap it fits, so a
    // file whose images pass that cap (#1313) loads as many as before.
    #[cfg(target_pointer_width = "64")]
    const {
        assert!(MAX_UNPACKED >= 1_500_000_000 + IMAGE_BUDGET.bytes)
    };
    // Below what the per-area caps added up to (about 10 GB).
    const { assert!(MAX_UNPACKED < 10 * 1024 * 1024 * 1024) };
}
