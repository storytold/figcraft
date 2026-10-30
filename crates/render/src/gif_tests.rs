//! Animated GIFs: frames and their timing, the caps on decoding, junk input, and the first frame
//! everywhere but the presenter.

use std::sync::Arc;

use figcraft_color::{ColorProfile, ImageFilters, ImageScaleMode, Paint};
use figcraft_doc::{Document, ImageData, NodeKind};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};

use crate::images::{self, MAX_FRAMES};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A synthetic animated GIF: one solid `w`×`h` frame per (colour, delay in ms).
fn gif(frames: &[([u8; 4], u32)], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        let frames =
            frames.iter().map(|(c, ms)| Frame::from_parts(RgbaImage::from_pixel(w, h, image::Rgba(*c)), 0, 0, Delay::from_numer_denom_ms(*ms, 1)));
        enc.encode_frames(frames).unwrap();
    }
    out
}

fn data(bytes: Vec<u8>) -> ImageData {
    ImageData { asset: None, bytes: Arc::new(bytes), width: 4, height: 4 }
}

#[test]
fn frames_keep_their_delays_and_loop() {
    let bytes = gif(&[(RED, 100), (GREEN, 200), (BLUE, 0)], 4, 4);
    let frames = images::decode_frames(&bytes).unwrap();
    let secs: Vec<f64> = frames.iter().map(|f| f.1).collect();
    assert_eq!(secs, vec![0.1, 0.2, 0.1], "a 0 ms delay plays as 100 ms");
    let a = images::animation("gif-timing", Some(&data(bytes)), &ImageFilters::default(), ColorProfile::Srgb).unwrap();
    assert_eq!(a.len(), 3);
    let at = |t: f64| {
        let (i, next) = a.at(t);
        (i, (next * 1000.0).round())
    };
    assert_eq!(at(0.0), (0, 100.0));
    assert_eq!(at(0.15), (1, 150.0));
    assert_eq!(at(0.35), (2, 50.0));
    assert_eq!(at(0.45), (0, 50.0), "loops after 400 ms");
    assert_eq!(at(f64::NAN), (0, 100.0));
}

#[test]
fn stills_and_junk_never_animate() {
    let still = gif(&[(RED, 100)], 4, 4);
    assert!(images::decode_frames(&still).is_none(), "one frame is a still");
    let png = crate::encode_png(&tiny_skia::Pixmap::new(2, 2).unwrap()).unwrap();
    assert!(images::decode_frames(&png).is_none());
    let moving = gif(&[(RED, 100), (GREEN, 100)], 8, 8);
    for n in 0..moving.len() {
        // Every truncation decodes to something or nothing, never a panic.
        let _ = images::decode_frames(&moving[..n]);
        let _ = images::decode(&moving[..n]);
    }
    let mut junk = b"GIF89a".to_vec();
    junk.extend((0..200u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
    assert!(images::decode_frames(&junk).is_none());
}

#[test]
fn decoding_is_capped() {
    let many: Vec<([u8; 4], u32)> = (0..MAX_FRAMES + 5).map(|i| (if i % 2 == 0 { RED } else { GREEN }, 100)).collect();
    assert_eq!(images::decode_frames(&gif(&many, 1, 1)).unwrap().len(), MAX_FRAMES);
    // A canvas so big that one frame is past the byte cap doesn't play (its first frame is still
    // the image, where the renderer can decode it).
    let mut huge = gif(&[(RED, 100), (GREEN, 100)], 1, 1);
    huge[6..10].copy_from_slice(&[0x28, 0x23, 0x28, 0x23]);
    assert!(images::decode_frames(&huge).is_none());
}

#[test]
fn the_first_frame_draws_unless_time_moves_on() {
    let mut d = Document::new("t");
    let page = d.add_page("Page 1");
    let id = d.create(page, NodeKind::Rectangle, "GIF", 0.0, 0.0, 4.0, 4.0).unwrap();
    let bytes = gif(&[(RED, 100), (GREEN, 100)], 4, 4);
    let hash = "gif-first-frame".to_string();
    d.images.insert(hash.clone(), data(bytes));
    d.get_mut(id).unwrap().fills = vec![Paint::image(hash, ImageScaleMode::Fill)];
    let centre = |p: tiny_skia::Pixmap| {
        let c = p.pixel(2, 2).unwrap();
        [c.red(), c.green(), c.blue()]
    };
    assert_eq!(centre(crate::render_node(&d, id, 1.0).unwrap()), [255, 0, 0], "exports and the canvas: the first frame");
    assert_eq!(centre(crate::render_node_at(&d, id, 1.0, 0.15).unwrap()), [0, 255, 0], "the presenter, 150 ms in");
    let (frame, next) = images::animation_clock(&d, page, 0.15).unwrap();
    assert!(frame != images::animation_clock(&d, page, 0.05).unwrap().0 && (next - 0.05).abs() < 1e-9);
    d.get_mut(id).unwrap().visible = false;
    assert!(images::animation_clock(&d, page, 0.15).is_none(), "hidden layers don't keep the presenter awake");
}
