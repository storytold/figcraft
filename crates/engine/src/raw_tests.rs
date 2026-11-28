//! Camera raw files open, import and place as the picture they develop to; damaged ones are
//! errors. The raw files are DNGs written here from synthetic sensor data (craft-raw's writer).

use base64::Engine as _;
use craft_raw::opcodes::OpcodeLists;
use craft_raw::{BlackLevel, Cfa, ColorData, DngCompression, DngWriteOptions, Orientation, RawData, RawFormat, RawImage, Rect};
use serde_json::json;

use crate::Session;

/// Sensor columns a band of the scene covers.
const BAND: usize = 22;
const BLACK: f32 = 64.0;
const WHITE: f32 = 4095.0;
/// The camera's response to a neutral (`AsShotNeutral`): white balance multiplies red by 2 and blue by 1/0.7.
const NEUTRAL: [f64; 3] = [0.5, 1.0, 0.7];

/// A 3 × 22 by 48 RGGB raw, turned a quarter (Exif orientation 6), whose camera RGB is linear sRGB.
/// Its sensor columns hold three bands: a red, a mid grey, and a light that clips every channel.
fn dng() -> Vec<u8> {
    let (w, h) = (3 * BAND, 48);
    let cfa = Cfa::bayer("RGGB").unwrap();
    let scene = |x: usize| match x / BAND {
        0 => [0.40, 0.08, 0.06],
        1 => [0.18, 0.18, 0.18],
        _ => [4.0, 4.0, 4.0],
    };
    let data = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let c = cfa.color_at(x, y) as usize;
            let level = (scene(x)[c] * NEUTRAL[c] as f32).clamp(0.0, 1.0);
            (BLACK + (WHITE - BLACK) * level).round() as u16
        })
        .collect();
    // XYZ (D65) → linear sRGB (IEC 61966-2-1): this camera sees in sRGB primaries.
    let xyz_to_camera = [[3.2404542, -1.5371385, -0.4985314], [-0.9692660, 1.8760108, 0.0415560], [0.0556434, -0.2040259, 1.0572252]];
    let raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(cfa),
        bits: 12,
        black: BlackLevel::uniform(BLACK),
        white: vec![WHITE],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Rotate90,
        color: ColorData { illuminant: [21, 0], color_matrix: [Some(xyz_to_camera), None], as_shot_neutral: Some(NEUTRAL), ..Default::default() },
        wb_multipliers: None,
        linearized: true,
        linearization_table: None,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: Default::default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
        notices: Default::default(),
    };
    let opts = DngWriteOptions { compression: DngCompression::Uncompressed, software: "figcraft tests".into(), ..Default::default() };
    craft_raw::write_dng(&raw, &opts).unwrap()
}

fn import(s: &mut Session, bytes: &[u8], name: &str) -> crate::Result<serde_json::Value> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    s.execute("file.import", &json!({"data": b64, "name": name, "x": 0, "y": 0}))
}

/// The mean colour of the 5 × 5 pixels around (`x`, `y`) of the layer's image.
fn colour_at(s: &Session, id: figcraft_doc::NodeId, x: u32, y: u32) -> [i32; 3] {
    let doc = s.doc().unwrap();
    let hash = match &doc.node(id).unwrap().fills[0].kind {
        figcraft_color::PaintKind::Image(paint) => paint.image.clone(),
        other => panic!("not an image fill: {other:?}"),
    };
    let picture = image::load_from_memory(&doc.images[&hash].bytes().unwrap()).unwrap().to_rgb8();
    let mut sum = [0i32; 3];
    for dy in 0..5 {
        for dx in 0..5 {
            let p = picture.get_pixel(x + dx - 2, y + dy - 2);
            for c in 0..3 {
                sum[c] += i32::from(p[c]);
            }
        }
    }
    sum.map(|v| v / 25)
}

fn spread(c: [i32; 3]) -> i32 {
    c.iter().max().unwrap() - c.iter().min().unwrap()
}

#[test]
fn camera_raw_files_import_and_open_as_their_developed_picture() {
    let bytes = dng();
    let mut s = Session::new();
    let out = import(&mut s, &bytes, "shot.dng").unwrap();
    let id = crate::params::id_of(&out["id"]).unwrap();
    let n = s.doc().unwrap().node(id).unwrap();
    // The orientation tag is applied: the sensor's columns run down the picture.
    assert!(n.height > n.width, "{} x {}", n.width, n.height);
    let x = n.width as u32 / 2;
    let band = BAND as u32;
    let (red, grey, light) = (colour_at(&s, id, x, band / 2), colour_at(&s, id, x, band * 3 / 2), colour_at(&s, id, x, band * 5 / 2));
    // White balance and the colour matrix give back the scene: red stays red, grey stays neutral.
    assert!(red[0] > red[1] + 60 && red[0] > red[2] + 60, "red band: {red:?}");
    assert!(spread(grey) <= 8, "grey band: {grey:?}");
    // Clipped channels are rebuilt: white balance alone would turn the clipped light magenta.
    assert!(light.iter().all(|&c| c > grey[1]) && spread(light) <= 8, "clipped band: {light:?}, grey {grey:?}");

    let path = std::env::temp_dir().join(format!("figcraft-raw-{}.dng", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let opened = s.execute("file.open", &json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    opened.unwrap();
    assert_eq!(s.doc().unwrap().images.len(), 1);
}

#[test]
fn damaged_camera_raw_files_are_errors() {
    let mut s = Session::new();
    // A Panasonic RW2 header over junk: refused, saying why.
    let mut junk = b"IIU\0\x08\0\0\0".to_vec();
    junk.extend(std::iter::repeat_n(0x5a, 4096));
    let err = import(&mut s, &junk, "shot.rw2").unwrap_err().to_string();
    assert!(err.contains("camera raw"), "{err}");
    // Cut short anywhere: refused or placed, never a panic, and the renderer copes with what was placed.
    let bytes = dng();
    for len in [12, 200, bytes.len() / 3, bytes.len() - 100] {
        if let Ok(out) = import(&mut s, &bytes[..len], "shot.dng") {
            let id = crate::params::id_of(&out["id"]).unwrap();
            assert!(s.doc().unwrap().node(id).is_some());
        }
    }
}
