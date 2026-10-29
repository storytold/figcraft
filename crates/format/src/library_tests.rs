//! Published libraries inside `.figcraft` archives.

use std::io::{Read, Write};

use super::*;
use crate::{add_image, load_bytes, save_bytes};
use figcraft_doc::color::{ImageScaleMode, Paint};
use figcraft_doc::{Document, LibraryAsset, NodeKind};

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 128, 255, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A published file: component "Tile" with an image fill; the file itself no longer uses the
/// image (it only lives on in the library).
fn published() -> (Document, figcraft_doc::NodeId, String) {
    let mut d = Document::new("Kit");
    let p = d.first_page().unwrap();
    let tile = d
        .create(
            p,
            NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() },
            "Tile",
            0.0,
            0.0,
            10.0,
            10.0,
        )
        .unwrap();
    let (hash, _, _) = add_image(&mut d, png()).unwrap();
    d.get_mut(tile).unwrap().fills = vec![Paint::image(hash.clone(), ImageScaleMode::Fill)];
    let snap = d.clone();
    d.images.clear();
    d.get_mut(tile).unwrap().fills.clear();
    let manifest = LibraryManifest {
        id: "kit-1".into(),
        name: "Kit".into(),
        version: 3,
        components: vec![
            LibraryAsset { id: tile, name: "Tile".into(), set: false, version: 2, published: true },
            LibraryAsset { id: figcraft_doc::NodeId(999), name: "Gone".into(), set: false, version: 1, published: true },
        ],
        ..Default::default()
    };
    d.published = Some(Arc::new(PublishedLibrary { manifest, doc: Arc::new(snap) }));
    (d, tile, hash)
}

#[test]
fn library_roundtrip_with_its_own_images() {
    let (d, tile, hash) = published();
    let bytes = save_bytes(&d, None).unwrap();
    // The author's file: its library comes back, with the image only the library uses.
    let (back, issues) = load_bytes(&bytes).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(back.images.is_empty(), "the file itself doesn't use the image");
    let lib = back.published.unwrap();
    assert_eq!(lib.manifest.version, 3);
    assert!(lib.doc.images.get(&hash).is_some_and(|i| (i.width, i.height) == (2, 2)));
    assert!(lib.manifest.component(figcraft_doc::NodeId(999)).is_none(), "assets the snapshot lacks are dropped");
    assert!(lib.manifest.component(tile).is_some());
    // Files using it read only the library entries.
    let read = read_library(std::io::Cursor::new(&bytes)).unwrap().unwrap();
    assert_eq!(read.manifest, lib.manifest);
    assert!(read.doc.images.contains_key(&hash));
    // Saving the loaded file again keeps everything.
    let again = save_bytes(&back_with(&bytes), None).unwrap();
    assert_eq!(read_library(std::io::Cursor::new(&again)).unwrap().unwrap().manifest, lib.manifest);
}

fn back_with(bytes: &[u8]) -> Document {
    load_bytes(bytes).unwrap().0
}

/// `zip_bytes` with member `name` replaced by `bytes` (or removed when `None`).
fn replace_member(zip_bytes: &[u8], name: &str, bytes: Option<&[u8]>) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let n = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        let data = if n == name { bytes.map(<[u8]>::to_vec) } else { Some(data) };
        if let Some(data) = data {
            out.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(&data).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn damaged_libraries_are_errors_never_crashes() {
    let (d, _, hash) = published();
    let good = save_bytes(&d, None).unwrap();
    let cases: [(&str, Option<&[u8]>); 6] = [
        ("library/manifest.json", Some(b"{nope")),
        ("library/manifest.json", Some(br#"{"id": "../../evil", "version": 1}"#)),
        ("library/manifest.json", Some(br#"{"id": "ok", "components": [{"id": "x"}]}"#)),
        ("library/document.json", Some(b"[1, 2]")),
        ("library/document.json", Some(br#"{"version": 999}"#)),
        ("library/document.json", None),
    ];
    for (member, bytes) in cases {
        let damaged = replace_member(&good, member, bytes);
        assert!(read_library(std::io::Cursor::new(&damaged)).is_err(), "{member} {bytes:?}");
        // The author's own file still opens: the library is skipped and reported.
        let (doc, issues) = load_bytes(&damaged).unwrap();
        assert!(doc.published.is_none() && issues.iter().any(|i| i.contains("library")), "{issues:?}");
    }
    // A missing image shows as a placeholder; the library still reads.
    let no_image = replace_member(&good, &format!("library/images/{hash}"), None);
    let lib = read_library(std::io::Cursor::new(&no_image)).unwrap().unwrap();
    assert!(lib.doc.images.is_empty());
    // Not an archive at all.
    assert!(read_library(std::io::Cursor::new(b"PK\x03\x04 nope")).is_err());
    assert!(read_library(std::io::Cursor::new(b"")).is_err());
}

#[test]
fn oversized_members_are_refused() {
    let (d, _, _) = published();
    let good = save_bytes(&d, None).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&good)).unwrap();
    assert!(read_member(&mut z, MANIFEST, 4).is_err_and(|e| e.to_string().contains("too large")));
    assert!(read_member(&mut z, MANIFEST, MAX_MANIFEST).is_ok());
    assert!(read_member(&mut z, "library/none.json", MAX_MANIFEST).is_err());
    assert!(is_valid_library_id("a1_B-2") && !is_valid_library_id("") && !is_valid_library_id("a/b") && !is_valid_library_id(&"x".repeat(65)));
}

#[test]
fn library_survives_a_save_with_versions() {
    let (d, tile, hash) = published();
    let mut old = d.clone();
    old.published = None;
    let versions = [crate::Version::new("v1", "Before", "", 1, Arc::new(old))];
    let bytes = crate::save_bytes_with_versions(&d, None, &versions).unwrap();
    let loaded = crate::load_bytes_with_versions(&bytes).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert_eq!(loaded.versions.len(), 1);
    let lib = loaded.doc.published.clone().unwrap();
    assert_eq!((lib.manifest.id.as_str(), lib.manifest.version), ("kit-1", 3));
    assert!(lib.doc.images.contains_key(&hash) && lib.manifest.component(tile).is_some());
    assert!(read_library(std::io::Cursor::new(&bytes)).unwrap().unwrap().doc.images.contains_key(&hash));
    // Saved again with its versions, it is still a library.
    let again = crate::save_bytes_with_versions(&loaded.doc, None, &loaded.versions).unwrap();
    assert_eq!(read_library(std::io::Cursor::new(&again)).unwrap().unwrap().manifest, lib.manifest);
    assert_eq!(crate::load_bytes_with_versions(&again).unwrap().versions.len(), 1);
}
