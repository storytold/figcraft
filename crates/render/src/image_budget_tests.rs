//! Decoded images within a memory budget (#203): an image too large is refused from its header,
//! before its pixels are decoded, and the decoded-image caches hold a bounded number of bytes
//! however many images, filters and colour profiles a document uses.

use std::sync::Arc;

use figcraft_color::ColorProfile;
use tiny_skia::Pixmap;

use super::*;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn asymmetric_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_fn(24, 16, |x, y| {
        image::Rgb(match (x < 12, y < 8) {
            (true, true) => [220, 30, 40],
            (false, true) => [20, 190, 50],
            (true, false) => [30, 60, 210],
            (false, false) => [230, 200, 20],
        })
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
    out
}

/// Add one EXIF orientation tag without changing the JPEG's compressed pixels.
fn jpeg_with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    // EXIF header, little-endian TIFF header, and an IFD containing one SHORT orientation.
    let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0; 6]); // Inline value padding, then no next IFD.
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(&exif);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn assert_jpeg_keeps_stored_pixels(orientation: u16) {
    let jpeg = asymmetric_jpeg();
    let oriented = jpeg_with_orientation(&jpeg, orientation);
    let expected = images::decode(&jpeg).unwrap();
    let actual = images::decode(&oriented).unwrap();
    assert_eq!((expected.width(), expected.height()), (24, 16));
    assert_eq!((actual.width(), actual.height()), (expected.width(), expected.height()));
    assert_eq!(actual.data(), expected.data(), "EXIF orientation {orientation} must leave stored pixels in place");

    let (expected, _) = images::decode_within(&jpeg, ColorProfile::Srgb, 12, u64::MAX).unwrap();
    let (actual, _) = images::decode_within(&oriented, ColorProfile::Srgb, 12, u64::MAX).unwrap();
    assert_eq!((expected.width(), expected.height()), (12, 8));
    assert_eq!((actual.width(), actual.height()), (expected.width(), expected.height()));
    assert_eq!(actual.data(), expected.data(), "thumbnail must keep stored pixels for EXIF orientation {orientation}");
}

#[test]
fn a_jpeg_with_exif_rotation_keeps_its_stored_pixels() {
    assert_jpeg_keeps_stored_pixels(6);
}

#[test]
fn a_jpeg_with_exif_mirroring_keeps_its_stored_pixels() {
    assert_jpeg_keeps_stored_pixels(2);
}

/// A PNG whose header claims `w` × `h` but which holds no pixel data at all.
fn png_header_only(w: u32, h: u32) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend_from_slice(&13u32.to_be_bytes());
    let chunk: Vec<u8> = b"IHDR".iter().copied().chain(ihdr).collect();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[test]
fn an_image_within_the_limits_decodes() {
    let (p, _) = images::decode_within(&png(30, 20), ColorProfile::Srgb, 30, 30 * 20 * 4).unwrap();
    assert_eq!((p.width(), p.height()), (30, 20));
}

#[test]
fn an_image_wider_or_taller_than_the_limit_is_scaled_down_to_fit() {
    let (p, _) = images::decode_within(&png(30, 20), ColorProfile::Srgb, 15, u64::MAX).unwrap();
    assert_eq!((p.width(), p.height()), (15, 10));
    let (p, _) = images::decode_within(&png(20, 30), ColorProfile::Srgb, 15, u64::MAX).unwrap();
    assert_eq!((p.width(), p.height()), (10, 15));
}

#[test]
fn an_image_whose_pixels_pass_the_byte_budget_is_refused() {
    assert!(images::decode_within(&png(30, 20), ColorProfile::Srgb, 1000, 30 * 20 * 4 - 1).is_none());
}

#[test]
fn a_huge_image_is_refused_from_its_header_alone() {
    // 16 000 × 16 000 is within the side limit; its pixels (1 GB) aren't within a 1 MB budget, and
    // the file has none to decode: only the header is read.
    assert!(images::decode_within(&png_header_only(16_000, 16_000), ColorProfile::Srgb, 16_384, 1 << 20).is_none());
    // Past the side limit, the shipped decode reads its header and finds no pixels to scale down.
    assert!(images::decode(&png_header_only(20_000, 10)).is_none());
}

#[test]
fn an_image_past_the_side_limit_is_drawn_downscaled() {
    // #1336: one side over 16 384 px drew nothing.
    let p = images::decode(&png(16_385, 4)).unwrap();
    assert_eq!(p.width(), 16_384);
    assert!(p.height() >= 1 && p.height() <= 4);
    assert!(p.pixels().iter().all(|c| c.alpha() == 255));
    let p = images::decode(&png(4, 18_078)).unwrap();
    assert_eq!(p.height(), 16_384);
    assert!(p.width() >= 1 && p.width() <= 4);
}

fn key(name: &str) -> images::Key {
    (name.to_string(), [0; 7], ColorProfile::Srgb, 0)
}

fn pixmap(side: u32) -> Arc<Pixmap> {
    Arc::new(Pixmap::new(side, side).unwrap())
}

#[test]
fn the_image_cache_keeps_within_its_byte_budget() {
    // 10 × 10 pixmaps are 400 bytes; 1000 bytes hold two.
    let mut c = images::PixmapCache::default();
    for name in ["a", "b", "c", "d", "e"] {
        c.insert(key(name), pixmap(10), 1000);
        assert!(c.bytes() <= 1000, "{} bytes held", c.bytes());
        assert!(c.get(&key(name)).is_some(), "{name} was just added");
    }
}

#[test]
fn a_full_image_cache_drops_the_least_recently_used_image_only() {
    // 1000 bytes hold two 400-byte pixmaps: the third drops the one used longest ago.
    let mut c = images::PixmapCache::default();
    c.insert(key("a"), pixmap(10), 1000);
    c.insert(key("b"), pixmap(10), 1000);
    assert!(c.get(&key("a")).is_some());
    c.insert(key("c"), pixmap(10), 1000);
    assert!(c.get(&key("a")).is_some(), "used since b: kept");
    assert!(c.get(&key("b")).is_none(), "used longest ago: dropped");
    assert!(c.get(&key("c")).is_some());
    assert_eq!(c.bytes(), 800);
}

#[test]
fn an_image_larger_than_the_budget_is_held_alone() {
    let mut c = images::PixmapCache::default();
    c.insert(key("small"), pixmap(10), 1000);
    c.insert(key("big"), pixmap(20), 1000);
    assert!(c.get(&key("big")).is_some());
    assert!(c.get(&key("small")).is_none());
    assert_eq!(c.bytes(), 20 * 20 * 4);
    // The next image takes its place.
    c.insert(key("next"), pixmap(10), 1000);
    assert_eq!(c.bytes(), 400);
}

#[test]
fn filter_and_profile_variants_count_against_one_budget() {
    let mut c = images::PixmapCache::default();
    for (i, profile) in [ColorProfile::Srgb, ColorProfile::DisplayP3].into_iter().enumerate() {
        for f in 0..4 {
            c.insert(("img".to_string(), [f, 0, 0, 0, 0, 0, i as i32], profile, 0), pixmap(10), 1000);
            assert!(c.bytes() <= 1000, "{} bytes held", c.bytes());
        }
    }
}

#[test]
fn zero_size_and_corrupt_inputs_are_refused() {
    let corrupt: [&[u8]; 6] = [b"", b"not an image", b"\x89PNG\r\n\x1a\n", b"\xff\xd8\xff\xe0", b"RIFF\x00\x00\x00\x00WEBP", b"GIF89a"];
    for bytes in corrupt {
        assert!(images::dimensions(bytes).is_none());
        assert!(images::decode(bytes).is_none());
        assert!(images::decode_within(bytes, ColorProfile::Srgb, 256, u64::MAX).is_none());
        assert!(crate::image_profile::Source::of(bytes).name.is_none());
    }
    for (w, h) in [(0, 0), (0, 4), (4, 0)] {
        let bad_png = png_header_only(w, h);
        assert!(images::dimensions(&bad_png).is_none(), "{w}x{h}");
        assert!(images::decode(&bad_png).is_none(), "{w}x{h}");
        assert!(images::decode_within(&bad_png, ColorProfile::Srgb, 256, u64::MAX).is_none(), "{w}x{h}");
    }
}

#[test]
fn dimensions_reads_valid_formats() {
    let p = Pixmap::new(12, 7).unwrap();
    let png_bytes = crate::encode_png(&p).unwrap();
    let jpg_bytes = crate::encode_jpeg(&p, figcraft_color::Color::WHITE, 85).unwrap();
    let webp_bytes = crate::encode_webp(&p).unwrap();
    for bytes in [png_bytes, jpg_bytes, webp_bytes] {
        assert_eq!(images::dimensions(&bytes), Some((12, 7)));
    }
}
