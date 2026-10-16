//! Colour glyph behaviour (colour emoji): bitmap strikes, COLR layers, and where they sit.
//!
//! The placement cases are pure numbers, so they run everywhere. The rest reads the machine's
//! colour emoji face and skips (like the other suites) on a machine without one; which format it
//! uses depends on the platform (Apple Color Emoji is sbix, Noto Color Emoji is CBDT, Segoe UI
//! Emoji is COLR), so a machine exercises the paths its own font has.

use super::*;
use crate::fontdb::{db, no_coords, scan_system_fonts};
use crate::layout::layout;
use figcraft_doc::TextData;
use skrifa::bitmap::{BitmapData, BitmapGlyph, MaskData, Origin};

/// A strike glyph described by hand, for the placement cases.
fn glyph(width: u32, height: u32, origin: Origin, bearing: (f32, f32), inner: (f32, f32), ppem: f32) -> BitmapGlyph<'static> {
    BitmapGlyph {
        data: BitmapData::Png(&[]),
        bearing_x: bearing.0,
        bearing_y: bearing.1,
        inner_bearing_x: inner.0,
        inner_bearing_y: inner.1,
        ppem_x: ppem,
        ppem_y: ppem,
        advance: None,
        width,
        height,
        placement_origin: origin,
    }
}

/// A strike glyph with data, for the decoding cases (the fields they don't read stay neutral).
fn strike(data: BitmapData<'_>, width: u32, height: u32) -> BitmapGlyph<'_> {
    BitmapGlyph {
        data,
        bearing_x: 0.0,
        bearing_y: 0.0,
        inner_bearing_x: 0.0,
        inner_bearing_y: 0.0,
        ppem_x: 10.0,
        ppem_y: 10.0,
        advance: None,
        width,
        height,
        placement_origin: Origin::TopLeft,
    }
}

#[test]
fn bitmap_placement_follows_the_strike() {
    // CBDT: bearings are pixels from the origin to the bitmap's left and top edges, read at the
    // strike's ppem and scaled to the size asked for: 2 px at ppem 10 in a 20 px run → 4 px.
    let cbdt = glyph(100, 50, Origin::TopLeft, (0.0, 0.0), (2.0, 8.0), 10.0);
    let (left, top) = placement(&cbdt, 20.0, 1000.0);
    assert_eq!((left, top), (4.0, -16.0), "the left edge sits right of the pen, the top above it");

    // sbix: bearings and origin offsets are font units, and the image is placed by its lower-left
    // corner, so its top is the whole image above the origin.
    let sbix = glyph(10, 10, Origin::BottomLeft, (-1.0, -3.0), (1.0, 2.0), 10.0);
    let (left, top) = placement(&sbix, 20.0, 1000.0);
    assert!((left - (2.0 - 0.02)).abs() < 1e-9, "{left}");
    assert!((top - (-24.0 + 0.06)).abs() < 1e-9, "{top}");

    // Doubling the size doubles the offset with it.
    let scaled = glyph(10, 10, Origin::TopLeft, (0.0, 0.0), (1.0, 1.0), 10.0);
    assert_eq!(placement(&scaled, 40.0, 1000.0).1, -4.0);
    assert_eq!(placement(&scaled, 20.0, 1000.0).1, -2.0);
}

/// A 2×2 PNG with straight alpha, the format strikes carry: row 0 is (255,0,0,128) then
/// (0,255,0,255), row 1 is (0,0,255,0) then (10,20,30,51).
const PNG_STRAIGHT: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d, 0x24, 0x00, 0x00, 0x00, 0x17, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xd0,
    0xc0, 0xf0, 0x1f, 0x08, 0x19, 0x80, 0x98, 0x4b, 0x44, 0xce, 0x18, 0x00, 0x34, 0xb8, 0x04, 0xec, 0xa7, 0x3b, 0x38, 0x04, 0x00, 0x00, 0x00, 0x00,
    0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn png_strikes_decode_to_premultiplied_rgba() {
    let (rgba, width, height) = decode(&strike(BitmapData::Png(PNG_STRAIGHT), 2, 2)).unwrap();
    assert_eq!((width, height), (2, 2));
    assert_eq!(
        rgba,
        vec![128, 0, 0, 128, 0, 255, 0, 255, 0, 0, 0, 0, 2, 4, 6, 51],
        "straight alpha times its own alpha, rounded: the renderer's surfaces are premultiplied"
    );
}

#[test]
fn a_png_strike_claiming_a_huge_size_is_refused_before_anything_is_allocated() {
    // PNG_STRAIGHT with its header saying 1 048 576 × 1 048 576 pixels (4 TiB as RGBA).
    let mut png = PNG_STRAIGHT.to_vec();
    assert_eq!(crc32(&png[12..29]).to_be_bytes(), png[29..33], "the patched header passes its checksum");
    png[16..24].copy_from_slice(&[0, 0x10, 0, 0, 0, 0x10, 0, 0]);
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(decode_png(&png).is_none());
}

/// The CRC-32 a PNG chunk ends with.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |c, b| (0..8).fold(c ^ u32::from(*b), |c, _| if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }))
}

#[test]
fn raw_bgra_strikes_are_already_premultiplied() {
    // BitmapData::Bgra is BGRA with premultiplied channels (the spec says so), so decoding only
    // moves the red and blue channels.
    let (rgba, width, height) = decode(&strike(BitmapData::Bgra(&[17, 34, 51, 255, 4, 8, 12, 16]), 2, 1)).unwrap();
    assert_eq!((width, height), (2, 1));
    assert_eq!(rgba, vec![51, 34, 17, 255, 12, 8, 4, 16]);
}

#[test]
fn strikes_that_are_not_colour_decode_to_nothing() {
    let mask = MaskData { bpp: 8, is_packed: false, data: &[0; 16] };
    assert!(decode(&strike(BitmapData::Mask(mask), 2, 2)).is_none(), "alpha-only strikes are masks, not colour");
    assert!(decode(&strike(BitmapData::Png(b"not a png"), 2, 2)).is_none(), "a broken PNG is skipped, not drawn");
    assert!(decode(&strike(BitmapData::Bgra(&[1, 2, 3]), 2, 2)).is_none(), "a buffer shorter than the glyph is skipped");
}

#[test]
fn scaling_keeps_the_pixels_premultiplied() {
    // A 4×4 face: one straight colour over a falling alpha, premultiplied by hand.
    let straight = [220u32, 180, 60];
    let ramp = [255u32, 190, 128, 64, 0];
    let mut src = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            let a = ramp[(x + y) % ramp.len()];
            src.extend(straight.iter().map(|c| ((c * a + 127) / 255) as u8));
            src.push(a as u8);
        }
    }
    for (dw, dh) in [(2, 2), (3, 4), (9, 7), (16, 16)] {
        let out = scaled(&src, 4, 4, dw, dh).unwrap();
        assert_eq!(out.len(), (dw * dh * 4) as usize);
        for px in out.chunks_exact(4) {
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "{dw}×{dh}: {px:?}");
        }
    }
    assert!(scaled(&src, 4, 4, 0, 4).is_none() && scaled(&src, 0, 0, 4, 4).is_none(), "an empty side scales to nothing");
}

/// The machine's colour emoji face, if it has one (tests skip without: no font ships in the repo).
fn emoji_face() -> Option<std::sync::Arc<Face>> {
    scan_system_fonts();
    let mut db = db().write().unwrap();
    db.emoji_fallbacks().into_iter().find(|f| f.has_color())
}

#[test]
fn colour_emoji_carry_their_own_pixels() {
    let Some(face) = emoji_face() else { return };
    // A grinning face: a bitmap strike (Apple, Noto) or COLR layers (Segoe) must cover it.
    let gid = face.glyph_for('\u{1F600}');
    assert_ne!(gid, 0, "U+1F600 is not in {}", face.family);
    if let Some(layers) = face.color_layers(gid) {
        assert!(!layers.is_empty());
        assert!(layers.len() < 64, "{} layers", layers.len());
        assert!(layers.iter().all(|l| l.gid != 0), "every layer names a glyph: {layers:?}");
        return;
    }
    let Some(bitmap) = face.color_bitmap(gid, &no_coords(), 32.0) else {
        panic!("{} has colour tables but draws nothing for U+1F600", face.family);
    };
    // The image is a sane size for 32 px of text and its pixels are premultiplied (what the
    // renderer's surfaces hold), so no channel is brighter than its alpha.
    assert!(bitmap.width >= 8 && bitmap.width <= 128, "{} px wide", bitmap.width);
    assert!(bitmap.height >= 8 && bitmap.height <= 128, "{} px tall", bitmap.height);
    assert_eq!(bitmap.rgba.len(), bitmap.width as usize * bitmap.height as usize * 4);
    let mut opaque = 0;
    for px in bitmap.rgba.chunks_exact(4) {
        assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "premultiplied: {px:?}");
        opaque += usize::from(px[3] == 255);
    }
    assert!(opaque > 20, "the glyph draws something: {opaque} opaque pixels");
    // A bigger strike is scaled to a bigger image, and a smaller one to a smaller image.
    let small = face.color_bitmap(gid, &no_coords(), 16.0);
    let large = face.color_bitmap(gid, &no_coords(), 64.0);
    if let (Some(small), Some(large)) = (small, large) {
        assert!(small.width < bitmap.width && bitmap.width < large.width, "{} {} {}", small.width, bitmap.width, large.width);
    }
}

#[test]
fn emoji_in_plain_text_draw_from_a_colour_face() {
    // The style's own face (DejaVu Sans on Linux) or a symbol font among the fallbacks (Windows has
    // one) draws U+1F600 in one colour: the emoji face comes first for it.
    if emoji_face().is_none() {
        return;
    }
    let l = layout(&TextData::new("Hi \u{1F600}"), None, None);
    assert!(l.glyphs.iter().any(|g| g.color(1.0).is_some()), "the emoji has colour ink");
}

#[test]
fn glyphs_place_their_ink_at_a_scale() {
    let Some(face) = emoji_face() else { return };
    let mut t = TextData::new("\u{1F600}");
    (t.style.font_family, t.style.font_style) = (face.family.clone(), face.style.clone());
    t.style.font_size = 32.0;
    let l = layout(&t, None, None);
    let Some(g) = l.glyphs.first() else { return };
    // Zoomed 2:1 the glyph asks for a 64 px raster and places it in layout coordinates.
    let Some(ColorInk::Bitmap(bitmap, place)) = g.color(2.0) else {
        assert!(g.color(2.0).is_some(), "the emoji glyph has ink");
        return;
    };
    // Some emoji faces draw wider than the em (Noto Color Emoji's strikes are about 1.25 em).
    assert!(bitmap.width >= 32 && bitmap.width <= 96, "{} px at 2:1", bitmap.width);
    let corner = place * figcraft_geom::Point::new(0.0, 0.0);
    assert!(corner.x >= g.x - g.size && corner.x <= g.x + g.size, "x {} vs pen {}", corner.x, g.x);
    assert!(corner.y >= g.y - g.size && corner.y <= g.y + g.size, "y {} vs baseline {}", corner.y, g.y);
    // Past the raster cap the image stops growing (a canvas zoomed far in).
    let Some(ColorInk::Bitmap(huge, _)) = g.color(1000.0) else { return };
    assert!(f64::from(huge.width) <= MAX_PX_PER_EM + 1.0, "{} px", huge.width);
}

#[test]
fn text_without_colour_ink_keeps_its_outlines() {
    // Faces without colour tables answer "no ink" without touching the font again, so plain text
    // (the whole canvas, most of the time) pays one flag per glyph.
    let Some(face) = crate::fontdb::find_face("Inter", "Regular") else { return };
    if face.has_color() {
        return;
    }
    let gid = face.glyph_for('A');
    assert!(gid != 0, "Inter draws A");
    assert!(face.color_layers(gid).is_none() && face.color_bitmap(gid, &no_coords(), 32.0).is_none());
}
