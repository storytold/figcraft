//! Embedded image profiles, with every ICC profile built here in code from published constants.

use std::sync::Arc;

use figcraft_color::{ColorProfile, ImageFilters};
use figcraft_doc::ImageData;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};

use crate::image_profile::{MAX_ICC_BYTES, Source};
use crate::images;

/// Adobe RGB (1998) colorants adapted to D50 (rXYZ, gXYZ, bXYZ) and its gamma, 563/256.
const ADOBE_D50: [[f64; 3]; 3] = [[0.6097, 0.3111, 0.0195], [0.2053, 0.6257, 0.0609], [0.1492, 0.0632, 0.7446]];
const ADOBE_GAMMA: f64 = 563.0 / 256.0;
/// sRGB colorants adapted to D50.
const SRGB_D50: [[f64; 3]; 3] = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7142]];
/// Linear Adobe RGB → XYZ (D65), and XYZ (D65) → linear sRGB and Display P3 (published matrices).
const ADOBE_TO_XYZ: [[f64; 3]; 3] =
    [[0.576_730_9, 0.185_554_0, 0.188_185_2], [0.297_376_9, 0.627_349_1, 0.075_274_1], [0.027_034_3, 0.070_687_2, 0.991_108_5]];
const XYZ_TO_SRGB: [[f64; 3]; 3] =
    [[3.240_454_2, -1.537_138_5, -0.498_531_4], [-0.969_266_0, 1.876_010_8, 0.041_556_0], [0.055_643_4, -0.204_025_9, 1.057_225_2]];
const XYZ_TO_P3: [[f64; 3]; 3] =
    [[2.493_496_9, -0.931_383_6, -0.402_710_8], [-0.829_489_0, 1.762_664_1, 0.023_624_7], [0.035_845_8, -0.076_172_4, 0.956_884_5]];

enum Trc {
    /// `curv` with one entry (u8Fixed8 gamma).
    Gamma(f64),
    /// `curv` with a table.
    Table(Vec<u16>),
    /// `para` function type 0 (one gamma).
    Para(f64),
}

fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(v: [f64; 3]) -> Vec<u8> {
    [b"XYZ \0\0\0\0".as_slice(), &s15(v[0]), &s15(v[1]), &s15(v[2])].concat()
}

fn trc_tag(t: &Trc) -> Vec<u8> {
    match t {
        Trc::Gamma(g) => [b"curv\0\0\0\0".as_slice(), &1u32.to_be_bytes(), &((g * 256.0).round() as u16).to_be_bytes(), &[0, 0]].concat(),
        Trc::Table(v) => {
            let mut b = [b"curv\0\0\0\0".as_slice(), &(v.len() as u32).to_be_bytes()].concat();
            b.extend(v.iter().flat_map(|e| e.to_be_bytes()));
            b
        }
        Trc::Para(g) => [b"para\0\0\0\0\0\0\0\0".as_slice(), &s15(*g)].concat(),
    }
}

/// An ICC v2 display profile: `space` (`RGB `, `GRAY`, `CMYK`), a `desc`, a D50 `wtpt`, the
/// colorants (RGB only) and one curve for every channel.
fn icc(space: &[u8; 4], desc: &str, colorants: [[f64; 3]; 3], t: &Trc) -> Vec<u8> {
    let mut d = [b"desc\0\0\0\0".as_slice(), &(desc.len() as u32 + 1).to_be_bytes(), desc.as_bytes(), &[0]].concat();
    d.extend([0u8; 4 + 4 + 2 + 1 + 67]);
    let mut tags: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"desc", d), (b"wtpt", xyz_tag([0.9642, 1.0, 0.8249]))];
    match space {
        b"RGB " => {
            for (sig, c) in [(b"rXYZ", colorants[0]), (b"gXYZ", colorants[1]), (b"bXYZ", colorants[2])] {
                tags.push((sig, xyz_tag(c)));
            }
            for sig in [b"rTRC", b"gTRC", b"bTRC"] {
                tags.push((sig, trc_tag(t)));
            }
        }
        b"GRAY" => tags.push((b"kTRC", trc_tag(t))),
        _ => {}
    }
    let base = 128 + 4 + 12 * tags.len();
    let (mut table, mut data) = (Vec::new(), Vec::new());
    for (sig, body) in &tags {
        data.resize(data.len().next_multiple_of(4), 0);
        table.extend([sig.as_slice(), &((base + data.len()) as u32).to_be_bytes(), &(body.len() as u32).to_be_bytes()].concat());
        data.extend(body);
    }
    data.resize(data.len().next_multiple_of(4), 0);
    let mut h = vec![0u8; 128];
    h[0..4].copy_from_slice(&((base + data.len()) as u32).to_be_bytes());
    h[8..12].copy_from_slice(&0x0210_0000u32.to_be_bytes());
    h[12..16].copy_from_slice(b"mntr");
    h[16..20].copy_from_slice(space);
    h[20..24].copy_from_slice(b"XYZ ");
    h[36..40].copy_from_slice(b"acsp");
    for (k, v) in [0.9642, 1.0, 0.8249].into_iter().enumerate() {
        h[68 + k * 4..72 + k * 4].copy_from_slice(&s15(v));
    }
    [h, (tags.len() as u32).to_be_bytes().to_vec(), table, data].concat()
}

fn adobe() -> Vec<u8> {
    icc(b"RGB ", "Adobe RGB (1998)", ADOBE_D50, &Trc::Gamma(ADOBE_GAMMA))
}

fn encode(enc: impl ImageEncoder, pixels: &[u8], w: u32, h: u32, kind: ExtendedColorType, icc: Option<Vec<u8>>) {
    let mut enc = enc;
    if let Some(icc) = icc {
        enc.set_icc_profile(icc).unwrap();
    }
    enc.write_image(pixels, w, h, kind).unwrap();
}

fn png(img: &RgbaImage, icc: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    encode(image::codecs::png::PngEncoder::new(&mut out), img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8, icc);
    out
}

fn samples() -> RgbaImage {
    RgbaImage::from_raw(4, 1, vec![120, 160, 90, 255, 200, 80, 60, 255, 40, 90, 170, 255, 120, 160, 90, 128]).unwrap()
}

/// `bytes` decoded for a document in `target` (straight RGBA, every pixel).
fn pixels(bytes: &[u8], target: ColorProfile) -> Vec<[u8; 4]> {
    let data = ImageData { asset: None, bytes: Arc::new(bytes.to_vec()), width: 0, height: 0 };
    let hash = format!("icc-test-{:08x}-{}", crc32fast::hash(bytes), bytes.len());
    let p = images::get(&hash, Some(&data), &ImageFilters::default(), target).unwrap();
    p.pixels().iter().map(|c| c.demultiply()).map(|c| [c.red(), c.green(), c.blue(), c.alpha()]).collect()
}

fn apply(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
}

/// Adobe RGB 8-bit values in `target`, by the published matrices.
fn adobe_in(rgb: [u8; 3], to: &[[f64; 3]; 3]) -> [f64; 3] {
    let lin = rgb.map(|c| (f64::from(c) / 255.0).powf(ADOBE_GAMMA));
    let enc = |l: f64| {
        let l = l.clamp(0.0, 1.0);
        255.0 * if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
    };
    apply(to, apply(&ADOBE_TO_XYZ, lin)).map(enc)
}

fn near(got: [u8; 4], want: [f64; 3], tol: f64) -> bool {
    (0..3).all(|i| (f64::from(got[i]) - want[i]).abs() <= tol)
}

#[test]
fn adobe_rgb_png_converts_into_either_working_space() {
    let img = samples();
    let tagged = png(&img, Some(adobe()));
    assert_eq!(Source::of(&tagged).name.as_deref(), Some("Adobe RGB (1998)"));
    for (target, m) in [(ColorProfile::Srgb, XYZ_TO_SRGB), (ColorProfile::DisplayP3, XYZ_TO_P3)] {
        let got = pixels(&tagged, target);
        for (px, src) in got.iter().zip(img.pixels()) {
            let want = adobe_in([src[0], src[1], src[2]], &m);
            // The half-transparent pixel loses some precision to premultiplication.
            let tol = if src[3] == 255 { 2.0 } else { 3.0 };
            assert!(near(*px, want, tol), "{target:?} {src:?} → {px:?}, want {want:?}");
            assert_eq!(px[3], src[3], "alpha stays");
        }
    }
    // The same pixels without a profile are taken as sRGB, untouched in an sRGB document.
    let plain = png(&img, None);
    assert_eq!(Source::of(&plain).name, None);
    let got = pixels(&plain, ColorProfile::Srgb);
    assert_eq!(&got[..3], &[[120, 160, 90, 255], [200, 80, 60, 255], [40, 90, 170, 255]]);
    assert_ne!(got[0], pixels(&tagged, ColorProfile::Srgb)[0], "the profile makes a visible difference");
}

#[test]
fn every_container_carries_its_profile() {
    // shortcut: the image crate drops a TIFF's profile when the image is tiny (16×16 here), so
    // this one is 64×64; read the tag with the tiff crate directly if tiny profiled TIFFs matter.
    let s = samples();
    let img = RgbaImage::from_fn(64, 64, |x, _| *s.get_pixel(x % 4, 0));
    let rgb: Vec<u8> = img.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let (mut jpg, mut webp, mut tiff) = (Vec::new(), Vec::new(), std::io::Cursor::new(Vec::new()));
    encode(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 95), &rgb, 64, 64, ExtendedColorType::Rgb8, Some(adobe()));
    encode(image::codecs::webp::WebPEncoder::new_lossless(&mut webp), img.as_raw(), 64, 64, ExtendedColorType::Rgba8, Some(adobe()));
    encode(image::codecs::tiff::TiffEncoder::new(&mut tiff), img.as_raw(), 64, 64, ExtendedColorType::Rgba8, Some(adobe()));
    for (what, bytes) in [("jpg", jpg), ("webp", webp), ("tiff", tiff.into_inner())] {
        assert_eq!(Source::of(&bytes).name.as_deref(), Some("Adobe RGB (1998)"), "{what}");
        // JPEG is lossy: convert what it decodes to.
        let raw = image::load_from_memory(&bytes).unwrap().to_rgba8().get_pixel(1, 0).0;
        let px = pixels(&bytes, ColorProfile::Srgb)[1];
        assert!(near(px, adobe_in([raw[0], raw[1], raw[2]], &XYZ_TO_SRGB), 2.0), "{what}: {px:?}");
    }
}

#[test]
fn curve_types_table_and_parametric() {
    let img = samples();
    // An sRGB-primaries profile whose table samples the sRGB curve: (nearly) no change.
    let table: Vec<u16> = (0..1024)
        .map(|i| {
            let v = f64::from(i) / 1023.0;
            let l = if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            (l * 65535.0).round() as u16
        })
        .collect();
    let tabled = png(&img, Some(icc(b"RGB ", "Table sRGB", SRGB_D50, &Trc::Table(table))));
    for (px, src) in pixels(&tabled, ColorProfile::Srgb).iter().zip(img.pixels()).take(3) {
        assert!(near(*px, [src[0], src[1], src[2]].map(f64::from), 2.0), "{px:?} vs {src:?}");
    }
    // A parametric gamma behaves like the same `curv` gamma.
    let para = png(&img, Some(icc(b"RGB ", "Para", ADOBE_D50, &Trc::Para(ADOBE_GAMMA))));
    let curv = png(&img, Some(adobe()));
    for (a, b) in pixels(&para, ColorProfile::DisplayP3).iter().zip(pixels(&curv, ColorProfile::DisplayP3)) {
        assert!((0..3).all(|i| a[i].abs_diff(b[i]) <= 1), "{a:?} vs {b:?}");
    }
}

#[test]
fn display_p3_images_round_trip_and_convert() {
    let img = samples();
    let p3 = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    let tagged = png(&img, Some(p3));
    assert!(Source::of(&tagged).name.unwrap().contains("P3"));
    // In a Display P3 document the values stay; in an sRGB one they match the document conversion.
    let to_srgb = ColorProfile::DisplayP3.conversion_to(ColorProfile::Srgb);
    for ((same, srgb), src) in pixels(&tagged, ColorProfile::DisplayP3).iter().zip(pixels(&tagged, ColorProfile::Srgb)).zip(img.pixels()).take(3) {
        assert!((0..3).all(|i| same[i].abs_diff(src[i]) <= 1), "{same:?} vs {src:?}");
        let want = to_srgb.apply(figcraft_color::Color::from_rgba8(src[0], src[1], src[2], 255)).to_rgba8();
        assert!((0..3).all(|i| srgb[i].abs_diff(want[i]) <= 2), "{srgb:?} vs {want:?}");
    }
}

#[test]
fn grey_profiles() {
    let grey = image::GrayImage::from_raw(2, 1, vec![128, 255]).unwrap();
    let mut bytes = Vec::new();
    encode(
        image::codecs::png::PngEncoder::new(&mut bytes),
        grey.as_raw(),
        2,
        1,
        ExtendedColorType::L8,
        Some(icc(b"GRAY", "Linear grey", SRGB_D50, &Trc::Gamma(1.0))),
    );
    assert_eq!(Source::of(&bytes).name.as_deref(), Some("Linear grey"));
    // Linear 0.502 is sRGB 0.735 (188).
    let px = pixels(&bytes, ColorProfile::Srgb);
    assert!(near(px[0], [188.0; 3], 2.0) && near(px[1], [255.0; 3], 1.0), "{px:?}");
}

#[test]
fn unusable_profiles_fall_back_to_srgb() {
    let img = samples();
    let plain = pixels(&png(&img, None), ColorProfile::DisplayP3);
    // CMYK (and other non-RGB spaces), broken and oversized profiles: taken as sRGB.
    let mut huge = adobe();
    huge.resize(MAX_ICC_BYTES + 4, 0);
    let len = huge.len() as u32;
    huge[0..4].copy_from_slice(&len.to_be_bytes());
    for icc in [icc(b"CMYK", "Some press", SRGB_D50, &Trc::Gamma(1.0)), b"junk".to_vec(), adobe()[..200].to_vec(), huge] {
        let bytes = png(&img, Some(icc));
        assert_eq!(Source::of(&bytes).name, None);
        assert_eq!(pixels(&bytes, ColorProfile::DisplayP3), plain);
    }
}

#[test]
fn png_colour_chunks() {
    let img = samples();
    let plain = png(&img, None);
    let chunk = |kind: &[u8; 4], data: &[u8]| crate::png_insert_chunk(plain.clone(), kind, data);
    let be = |vs: &[u32]| vs.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let srgb_chrm = be(&[31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000]);
    // `sRGB`, and `gAMA` 1/2.2 (with or without sRGB's `cHRM`), mean sRGB: no change.
    let srgb_gama = crate::png_insert_chunk(chunk(b"gAMA", &be(&[45455])), b"cHRM", &srgb_chrm);
    for bytes in [chunk(b"sRGB", &[0]), chunk(b"gAMA", &be(&[45455])), srgb_gama] {
        assert_eq!(Source::of(&bytes).name.as_deref(), Some("sRGB"));
        assert_eq!(pixels(&bytes, ColorProfile::Srgb), pixels(&plain, ColorProfile::Srgb));
    }
    // A linear `gAMA`: 128 is linear 0.502 (sRGB 188).
    let grey = png(&RgbaImage::from_raw(1, 1, vec![128, 128, 128, 255]).unwrap(), None);
    let linear = crate::png_insert_chunk(grey, b"gAMA", &be(&[100_000]));
    assert_eq!(Source::of(&linear).name.as_deref(), Some("Gamma 1.00"));
    assert!(near(pixels(&linear, ColorProfile::Srgb)[0], [188.0; 3], 2.0));
    // Adobe RGB primaries with its gamma in `gAMA` + `cHRM` work like the ICC profile.
    let wide = crate::png_insert_chunk(chunk(b"gAMA", &be(&[45471])), b"cHRM", &be(&[31270, 32900, 64000, 33000, 21000, 71000, 15000, 6000]));
    assert_eq!(Source::of(&wide).name.as_deref(), Some("Gamma 2.20"));
    let want = adobe_in([200, 80, 60], &XYZ_TO_SRGB);
    assert!(near(pixels(&wide, ColorProfile::Srgb)[1], want, 2.0));
    // Nonsense chunks: taken as sRGB.
    for bytes in [chunk(b"gAMA", &be(&[0])), crate::png_insert_chunk(chunk(b"gAMA", &be(&[100_000])), b"cHRM", &be(&[0; 8]))] {
        let _ = pixels(&bytes, ColorProfile::DisplayP3);
    }
    assert_eq!(Source::of(&chunk(b"gAMA", &be(&[0]))).name, None);
}

#[test]
fn profile_names_are_cached_and_cleaned() {
    let img = samples();
    let bytes = png(&img, Some(icc(b"RGB ", "  Odd\u{1}name  ", ADOBE_D50, &Trc::Gamma(2.2))));
    let data = ImageData { asset: None, bytes: Arc::new(bytes), width: 4, height: 1 };
    assert_eq!(images::profile_name("icc-name-test", Some(&data)).as_deref(), Some("Oddname"));
    assert_eq!(images::profile_name("icc-name-test", None).as_deref(), Some("Oddname"), "cached");
    assert_eq!(images::profile_name("icc-name-missing", None), None);
}

/// A small xorshift generator, so the sweep is the same on every run.
fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn profile_parser_survives_junk() {
    let img = RgbaImage::from_raw(1, 1, vec![10, 20, 30, 255]).unwrap();
    let mut seed = 0x9E37_79B9_7F4A_7C15;
    let bases = [adobe(), icc(b"GRAY", "g", SRGB_D50, &Trc::Table(vec![0, 30000, 65535])), moxcms::ColorProfile::new_display_p3().encode().unwrap()];
    for i in 0..600 {
        let mut icc = bases[i % bases.len()].clone();
        match rng(&mut seed) % 4 {
            // Flip some bytes, anywhere.
            0 | 1 => {
                for _ in 0..1 + rng(&mut seed) % 16 {
                    let at = (rng(&mut seed) as usize) % icc.len();
                    icc[at] = rng(&mut seed) as u8;
                }
            }
            // Truncate.
            2 => icc.truncate((rng(&mut seed) as usize) % icc.len()),
            // Absurd tag counts, offsets and sizes.
            _ => {
                let at = [0usize, 128, 136, 140].get((rng(&mut seed) % 4) as usize).copied().unwrap_or(128);
                icc[at..at + 4].copy_from_slice(&(rng(&mut seed) as u32).to_be_bytes());
            }
        }
        let bytes = png(&img, Some(icc));
        let _ = Source::of(&bytes);
        for target in ColorProfile::ALL {
            let data = ImageData { asset: None, bytes: Arc::new(bytes.clone()), width: 1, height: 1 };
            let _ = images::get(&format!("icc-fuzz-{i}"), Some(&data), &ImageFilters::default(), target);
        }
    }
    let _ = Source::of(b"");
    let _ = Source::of(&[0x89, b'P', b'N', b'G']);
}
