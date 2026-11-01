//! The images of a native file within a budget (#189): however many members each within the
//! per-member limit a file holds, the images read from it together stay within a byte and a count
//! budget. The document's own images are read first, and the ones past the budget are reported.

use std::io::Write;

use figcraft_doc::Document;

use super::*;

fn png(k: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([k, 7, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A saved document with `n` images, and their hashes and total size.
fn file_with_images(n: u8) -> (Vec<u8>, Vec<String>, u64) {
    let mut d = Document::new("File");
    let mut hashes = Vec::new();
    let mut total = 0;
    for k in 0..n {
        let bytes = png(k);
        total += bytes.len() as u64;
        hashes.push(add_image(&mut d, bytes).unwrap().0);
    }
    (save_bytes(&d, None).unwrap(), hashes, total)
}

/// `bytes` with one more member, `name` holding `data`.
fn with_member(bytes: &[u8], name: &str, data: &[u8]) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = z.file_names().map(str::to_string).collect();
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for n in names {
        let member = read_member(&mut z, &n, u64::MAX).unwrap();
        w.start_file(n, stored).unwrap();
        w.write_all(&member).unwrap();
    }
    w.start_file(name, stored).unwrap();
    w.write_all(data).unwrap();
    w.finish().unwrap();
    buf.into_inner()
}

#[test]
fn images_exactly_within_the_budget_all_load() {
    let (bytes, hashes, total) = file_with_images(3);
    let loaded = load_within(&bytes, ImageBudget { images: 3, bytes: total }).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert!(hashes.iter().all(|h| loaded.doc.images.contains_key(h)));
}

#[test]
fn images_past_the_byte_budget_are_left_out_and_reported() {
    let (bytes, hashes, total) = file_with_images(3);
    let loaded = load_within(&bytes, ImageBudget { images: 100, bytes: total - 1 }).unwrap();
    let held: u64 = loaded.doc.images.values().map(|i| i.bytes.len() as u64).sum();
    assert!(held < total, "{held} bytes held");
    assert_eq!(loaded.doc.images.len(), 2);
    let lost: Vec<&String> = hashes.iter().filter(|h| !loaded.doc.images.contains_key(*h)).collect();
    assert_eq!(lost.len(), 1);
    assert!(loaded.issues.iter().any(|i| i.contains(lost[0].as_str()) && i.contains("together")), "{:?}", loaded.issues);
}

#[test]
fn images_past_the_budget_are_counted_and_stay_in_the_file() {
    let (bytes, _, total) = file_with_images(3);
    let loaded = load_within(&bytes, ImageBudget { images: 100, bytes: total - 1 }).unwrap();
    assert_eq!(loaded.images_past_budget, 1);
    assert!(loaded.issues.iter().any(|i| i.contains("stays in the file")), "{:?}", loaded.issues);
    assert!(!loaded.issues.iter().any(|i| i.contains("drops it")), "{:?}", loaded.issues);
    let counted = load_within(&bytes, ImageBudget { images: 2, bytes: u64::MAX }).unwrap();
    assert_eq!(counted.images_past_budget, 1);
    let within = load_within(&bytes, ImageBudget { images: 3, bytes: total }).unwrap();
    assert_eq!(within.images_past_budget, 0);
}

#[test]
fn images_past_the_count_budget_are_left_out_and_reported() {
    let (bytes, _, _) = file_with_images(4);
    let loaded = load_within(&bytes, ImageBudget { images: 3, bytes: u64::MAX }).unwrap();
    assert_eq!(loaded.doc.images.len(), 3);
    assert!(loaded.issues.iter().any(|i| i.contains("more than 3 images")), "{:?}", loaded.issues);
}

#[test]
fn the_documents_own_images_are_read_before_others() {
    let (bytes, hashes, total) = file_with_images(2);
    // An image the document doesn't list, named to come first in the archive's order.
    let extra = with_member(&bytes, "images/0000", &png(99));
    let loaded = load_within(&extra, ImageBudget { images: 2, bytes: total }).unwrap();
    assert!(hashes.iter().all(|h| loaded.doc.images.contains_key(h)), "{:?}", loaded.issues);
    assert!(!loaded.issues.iter().any(|i| hashes.iter().any(|h| i.contains(h.as_str()))), "{:?}", loaded.issues);
}

#[test]
fn a_file_within_the_shipped_budget_loads_as_before() {
    let (bytes, hashes, _) = file_with_images(3);
    let loaded = load_bytes_with_versions(&bytes).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert!(hashes.iter().all(|h| loaded.doc.images.contains_key(h)));
}
