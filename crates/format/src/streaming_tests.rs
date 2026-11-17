//! The seekable on-disk reader uses the same format and recovery behavior as the byte reader.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use figcraft_doc::{Document, NodeKind};

use super::*;

fn png(red: u8) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([red, 0, 0, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

#[test]
fn seekable_reader_matches_byte_reader_for_history_and_branch_archives() {
    let mut document = Document::new("Current");
    let current_image = add_image(&mut document, png(10)).unwrap().0;

    let mut older = Document::new("Older");
    let old_image = add_image(&mut older, png(20)).unwrap().0;
    let version = Version::new("v1", "Before", "", 1, Arc::new(older.clone()));

    let mut head = document.clone();
    let page = head.first_page().unwrap();
    head.create(page, NodeKind::Rectangle, "Branch shape", 1.0, 2.0, 3.0, 4.0).unwrap();
    add_image(&mut head, png(30)).unwrap();
    let branch = Branch {
        id: "b1".into(),
        name: "Explore".into(),
        time: 2,
        client: 4,
        state: BranchState::Active,
        base: Arc::new(document.clone()),
        versions: vec![Version::new("v2", "Branch base", "", 3, Arc::new(document.clone()))],
        head: Arc::new(head),
    };

    let bytes = save_bytes_with_history(&document, None, &[version], &[branch]).unwrap();
    let from_bytes = load_bytes_with_versions(&bytes).unwrap();
    assert_eq!(from_bytes.versions.len(), 1);
    assert_eq!(from_bytes.branches.len(), 1);
    assert!(from_bytes.doc.images.contains_key(&current_image));
    assert!(from_bytes.versions[0].images().contains_key(&old_image));

    let path = std::env::temp_dir().join(format!("figcraft-stream-reader-{}.figcraft", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let from_file = load_file(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(from_file, (from_bytes.doc, from_bytes.issues));
}

#[test]
fn seekable_reader_keeps_newer_document_error_precedence() {
    let bytes = crate::zip_files([("document.json", br#"{"version":999,"document":["malformed"]}"#.as_slice())]).unwrap();
    let path = std::env::temp_dir().join(format!("figcraft-stream-newer-{}.figcraft", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let result = load_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(matches!(result, Err(FormatError::Newer(999))));
}
