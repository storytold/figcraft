//! Reading the picture a saved file carries: from `.figcraft` and `.fig` archives and image files,
//! and `None` (never a panic) for missing, damaged, oversized and non-zip input.

use std::io::{Cursor, Write};

use figcraft_doc::Document;

use crate::thumbnail::{FileKind, MAX_THUMBNAIL_BYTES, MAX_THUMBNAIL_SIDE, decode_thumbnail, file_thumbnail, thumbnail_png};

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A zip holding `members` (stored).
fn zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut buf);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in members {
        z.start_file(*name, stored).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap();
    buf.into_inner()
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-thumbnail-tests-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_figcraft_file_gives_its_thumbnail() {
    let thumb = png(40, 20);
    let bytes = crate::save_bytes(&Document::new("File"), Some(&thumb)).unwrap();
    assert_eq!(thumbnail_png(Cursor::new(&bytes)).as_deref(), Some(thumb.as_slice()));
    let t = decode_thumbnail(&thumb, 640).unwrap();
    assert_eq!((t.width, t.height, t.rgba.len()), (40, 20, 40 * 20 * 4));
    assert_eq!(t.rgba.get(..4), Some([10, 20, 30, 255].as_slice()));
    // And from disk.
    let path = dir("figcraft").join("a.figcraft");
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(file_thumbnail(&path, 640), Some(t));
}

#[test]
fn a_fig_file_gives_its_thumbnail() {
    let thumb = png(30, 60);
    let (bytes, _) = crate::fig::export(&Document::new("File"), Some(&thumb)).unwrap();
    assert_eq!(thumbnail_png(Cursor::new(&bytes)).as_deref(), Some(thumb.as_slice()));
    let path = dir("fig").join("b.fig");
    std::fs::write(&path, &bytes).unwrap();
    let t = file_thumbnail(&path, 640).unwrap();
    assert_eq!((t.width, t.height), (30, 60));
}

#[test]
fn large_pictures_are_scaled_down_to_fit_never_up() {
    let t = decode_thumbnail(&png(800, 400), 200).unwrap();
    assert_eq!((t.width, t.height), (200, 100));
    let t = decode_thumbnail(&png(300, 900), 300).unwrap();
    assert_eq!((t.width, t.height), (100, 300));
    let t = decode_thumbnail(&png(16, 8), 640).unwrap();
    assert_eq!((t.width, t.height), (16, 8));
}

#[test]
fn missing_damaged_oversized_and_non_zip_input_give_none() {
    // No thumbnail in the archive.
    let bytes = crate::save_bytes(&Document::new("File"), None).unwrap();
    assert_eq!(thumbnail_png(Cursor::new(&bytes)), None);
    // Not a zip: bare JSON, an old-style `.fig`, nothing at all.
    assert_eq!(thumbnail_png(Cursor::new(b"{\"format\":\"figcraft\"}".as_slice())), None);
    assert_eq!(thumbnail_png(Cursor::new(b"fig-kiwi\x0f\0\0\0".as_slice())), None);
    assert_eq!(thumbnail_png(Cursor::new(Vec::new())), None);
    // A thumbnail that isn't a picture, or is cut short.
    let damaged = zip(&[("thumbnail.png", b"\x89PNG\r\n\x1a\nnot really")]);
    let png_bytes = thumbnail_png(Cursor::new(&damaged)).unwrap();
    assert_eq!(decode_thumbnail(&png_bytes, 640), None);
    let whole = png(20, 20);
    assert_eq!(decode_thumbnail(&whole[..whole.len() / 2], 640), None);
    assert_eq!(decode_thumbnail(b"", 640), None);
    // Too many bytes for a thumbnail.
    let big = vec![0u8; MAX_THUMBNAIL_BYTES as usize + 1];
    assert_eq!(thumbnail_png(Cursor::new(zip(&[("thumbnail.png", &big)]))), None);
    // Too many pixels on a side, however small the file.
    assert_eq!(decode_thumbnail(&png(MAX_THUMBNAIL_SIDE + 1, 1), 640), None);
    // A zip with a damaged central directory.
    let mut cut = crate::save_bytes(&Document::new("File"), Some(&png(4, 4))).unwrap();
    cut.truncate(cut.len() - 10);
    assert_eq!(thumbnail_png(Cursor::new(cut)), None);
}

#[test]
fn files_on_disk_by_kind() {
    let d = dir("kinds");
    // An image is its own picture; an svg and a missing file have none.
    let image = d.join("photo.PNG");
    std::fs::write(&image, png(12, 6)).unwrap();
    assert_eq!(file_thumbnail(&image, 640).map(|t| (t.width, t.height)), Some((12, 6)));
    let svg = d.join("icon.svg");
    std::fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
    assert_eq!(file_thumbnail(&svg, 640), None);
    assert_eq!(file_thumbnail(&d.join("gone.figcraft"), 640), None);
    // A `.figcraft` that is plain JSON, and one that is junk.
    let json = d.join("plain.figcraft");
    std::fs::write(&json, "{}").unwrap();
    assert_eq!(file_thumbnail(&json, 640), None);
    let junk = d.join("junk.fig");
    std::fs::write(&junk, [0xFFu8; 64]).unwrap();
    assert_eq!(file_thumbnail(&junk, 640), None);
    // A folder named like a file.
    let folder = d.join("folder.figcraft");
    std::fs::create_dir_all(&folder).unwrap();
    assert_eq!(file_thumbnail(&folder, 640), None);
    let kinds: Vec<FileKind> =
        ["a.figcraft", "a.FIG", "a.svg", "a.jpeg", "a.webp", "a.txt", "noext"].iter().map(|p| FileKind::of(std::path::Path::new(p))).collect();
    assert_eq!(kinds, [FileKind::FigCraft, FileKind::Fig, FileKind::Svg, FileKind::Image, FileKind::Image, FileKind::Other, FileKind::Other]);
}

/// A document whose first page is `rgb`.
fn doc_on(rgb: [f32; 3]) -> Document {
    let mut d = Document::new("File");
    let p = d.first_page().unwrap();
    if let Some(figcraft_doc::NodeKind::Page { background, .. }) = d.node_mut(p).map(|n| &mut n.kind) {
        *background = figcraft_doc::color::Color { r: rgb[0], g: rgb[1], b: rgb[2], a: 1.0 };
    }
    d
}

#[test]
fn previews_carry_the_first_pages_colour() {
    use crate::thumbnail::{Preview, archive_preview, file_preview};
    let thumb = png(8, 4);
    // `.figcraft`: the first page's background from document.json.
    let bytes = crate::save_bytes(&doc_on([0.1, 0.2, 0.3]), Some(&thumb)).unwrap();
    let p = archive_preview(Cursor::new(&bytes), 640);
    assert_eq!(p.page, Some([0.1, 0.2, 0.3, 1.0]));
    assert_eq!(p.picture.map(|t| (t.width, t.height)), Some((8, 4)));
    // `.fig`: the canvas colour from meta.json.
    let (bytes, _) = crate::fig::export(&doc_on([0.5, 0.25, 0.75]), Some(&thumb)).unwrap();
    let p = archive_preview(Cursor::new(&bytes), 640);
    assert_eq!(p.page, Some([0.5, 0.25, 0.75, 1.0]));
    assert!(p.picture.is_some());
    // A file with nothing on it: its colour, no picture.
    let bytes = crate::save_bytes(&doc_on([0.9, 0.9, 0.9]), None).unwrap();
    assert_eq!(archive_preview(Cursor::new(&bytes), 640), Preview { page: Some([0.9, 0.9, 0.9, 1.0]), picture: None });
    // Damaged or odd metadata: no colour, never a panic.
    for meta in [&b"{"[..], b"{\"client_meta\":{\"background_color\":{\"r\":\"x\",\"g\":0,\"b\":0}}}", b"[]", b"{\"client_meta\":7}"] {
        assert_eq!(archive_preview(Cursor::new(zip(&[("meta.json", meta)])), 640).page, None, "{}", String::from_utf8_lossy(meta));
    }
    for doc in [
        &b"{\"pages\":[5],\"nodes\":{}}"[..],
        b"{\"pages\":[],\"nodes\":{}}",
        b"{\"pages\":\"x\"}",
        b"{\"pages\":[1],\"nodes\":{\"1\":{\"background\":[1,2]}}}",
    ] {
        assert_eq!(archive_preview(Cursor::new(zip(&[("document.json", doc)])), 640).page, None, "{}", String::from_utf8_lossy(doc));
    }
    // Out-of-range channels are clamped; alpha defaults to opaque.
    let doc = b"{\"pages\":[1],\"nodes\":{\"1\":{\"background\":{\"r\":2,\"g\":-1,\"b\":0.5}}}}";
    assert_eq!(archive_preview(Cursor::new(zip(&[("document.json", doc)])), 640).page, Some([1.0, 0.0, 0.5, 1.0]));
    // Not an archive, or no file at all.
    assert_eq!(archive_preview(Cursor::new(b"junk".to_vec()), 640), Preview::default());
    assert_eq!(file_preview(std::path::Path::new("/nonexistent/x.figcraft"), 640), Preview::default());
    let d = dir("preview");
    let svg = d.join("a.svg");
    std::fs::write(&svg, "<svg/>").unwrap();
    assert_eq!(file_preview(&svg, 640), Preview::default());
    let image = d.join("a.png");
    std::fs::write(&image, png(5, 5)).unwrap();
    let p = file_preview(&image, 640);
    assert!(p.page.is_none() && p.picture.is_some());
}
