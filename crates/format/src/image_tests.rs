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
