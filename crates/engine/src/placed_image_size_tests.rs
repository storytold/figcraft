use base64::Engine as _;
use figcraft_doc::NodeId;
use serde_json::json;

use crate::Session;

const PIXELS_PER_METER_AT_144_DPI: u32 = 5669;
const PIXELS_PER_METER_AT_72_DPI: u32 = 2835;
const PIXELS_PER_METER_AT_300_DPI: u32 = 11811;

fn png(width: u32, height: u32) -> Vec<u8> {
    figcraft_render::encode_png(&figcraft_render::Pixmap::new(width, height).unwrap()).unwrap()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn with_pixels_per_meter(png: &[u8], pixels_per_meter: u32) -> Vec<u8> {
    let after_header = 8 + 8 + 13 + 4;
    let mut chunk = b"pHYs".to_vec();
    chunk.extend(pixels_per_meter.to_be_bytes());
    chunk.extend(pixels_per_meter.to_be_bytes());
    chunk.push(1);
    let mut out = png[..after_header].to_vec();
    out.extend(9u32.to_be_bytes());
    out.extend(&chunk);
    out.extend(crc32(&chunk).to_be_bytes());
    out.extend(&png[after_header..]);
    out
}

fn place(s: &mut Session, bytes: &[u8], extra: serde_json::Value) -> (f64, f64) {
    let mut params = json!({"data": base64::engine::general_purpose::STANDARD.encode(bytes)});
    params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let placed = s.execute("image.place", &params).unwrap();
    let node = s.doc().unwrap().node(NodeId(placed["id"].as_u64().unwrap())).unwrap();
    (node.width, node.height)
}

#[test]
fn an_image_is_placed_at_its_pixel_size() {
    let mut s = Session::new();
    assert_eq!(place(&mut s, &png(2400, 1200), json!({})), (2400.0, 1200.0));
    assert_eq!(place(&mut s, &with_pixels_per_meter(&png(1440, 900), PIXELS_PER_METER_AT_72_DPI), json!({})), (1440.0, 900.0));
}

#[test]
fn a_retina_screenshot_is_placed_at_its_point_size() {
    let mut s = Session::new();
    let retina = with_pixels_per_meter(&png(800, 600), PIXELS_PER_METER_AT_144_DPI);
    assert_eq!(figcraft_format::pixel_density(&retina), 2.0);
    assert_eq!(place(&mut s, &retina, json!({})), (400.0, 300.0));
}

#[test]
fn a_print_resolution_is_not_read_as_a_screen_density() {
    let mut s = Session::new();
    let scan = with_pixels_per_meter(&png(600, 400), PIXELS_PER_METER_AT_300_DPI);
    assert_eq!(figcraft_format::pixel_density(&scan), 1.0);
    assert_eq!(place(&mut s, &scan, json!({})), (600.0, 400.0));
}

#[test]
fn a_density_near_a_screen_density_is_not_read_as_one() {
    for (dpi, pixels_per_meter) in [(141, 5551), (143, 5630), (145, 5709)] {
        let near = with_pixels_per_meter(&png(4, 4), pixels_per_meter);
        assert_eq!(figcraft_format::pixel_density(&near), 1.0, "{dpi} dpi");
    }
    for (density, pixels_per_meter) in [(2.0, 5670), (3.0, 8504), (4.0, 11339)] {
        let screen = with_pixels_per_meter(&png(4, 4), pixels_per_meter);
        assert_eq!(figcraft_format::pixel_density(&screen), density, "{pixels_per_meter} pixels per metre");
    }
}

#[test]
fn only_a_side_past_4096_is_fitted() {
    let mut s = Session::new();
    let (w, h) = place(&mut s, &png(8192, 100), json!({}));
    assert_eq!((w, h), (4096.0, 50.0));
}

#[test]
fn an_explicit_size_still_wins() {
    let mut s = Session::new();
    let retina = with_pixels_per_meter(&png(800, 600), PIXELS_PER_METER_AT_144_DPI);
    assert_eq!(place(&mut s, &retina, json!({"width": 200})), (200.0, 150.0));
    assert_eq!(place(&mut s, &png(2400, 1200), json!({"width": 100, "height": 70})), (100.0, 70.0));
}

#[test]
fn bytes_that_are_not_png_have_no_density() {
    assert_eq!(figcraft_format::pixel_density(b"GIF89a"), 1.0);
    assert_eq!(figcraft_format::pixel_density(&[]), 1.0);
    let truncated = &with_pixels_per_meter(&png(4, 4), PIXELS_PER_METER_AT_144_DPI)[..40];
    assert_eq!(figcraft_format::pixel_density(truncated), 1.0);
}
