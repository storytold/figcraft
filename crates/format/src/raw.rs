//! Camera raw files (Panasonic RW2, Canon CR2 / CR3, Nikon NEF, Sony ARW, Fujifilm RAF, Olympus
//! ORF, DNG and the other containers [`craft_raw`] decodes) as images.
//!
//! A raw file holds sensor samples, not a picture, so it is developed once, when it enters a
//! document, and the document keeps the result: a JPEG at quality 95 with full chroma resolution,
//! untagged and so read as sRGB. Drawing, export, collaboration and the browser build then see an
//! ordinary image; the raw file itself is not stored. (A PNG of the same picture is several times
//! larger, and every collaborator downloads the document's images.)
//!
//! The development is the camera's as-shot rendering at neutral settings, made of the shared
//! stages in `craft-develop`:
//!
//! 1. [`craft_raw::decode_with`]: black and white levels, the active area and default crop, the
//!    file's own corrections as DNG opcodes (defect pixels, the in-camera distortion correction of
//!    RW2 and NEF files).
//! 2. The demosaic (AHD) of the white-balanced mosaic, cropped and with the pixel aspect applied
//!    ([`RawDevelopExt::develop_balanced_masked`]).
//! 3. Clipped highlights rebuilt from the channels that did not clip ([`highlight`]).
//! 4. The as-shot white balance and the camera's colour matrices to linear ITU-R BT.2020 (the DNG
//!    colour model, DNG 1.7.1 chapter 6), raised by the file's baseline exposure.
//! 5. The generic tone curve on luminance ([`ToneMap::rolloff_hdr`] at neutral settings).
//! 6. BT.2020 to BT.709 primaries, colours outside them desaturated toward their luminance
//!    ([`gamut_map`]), the sRGB transfer function (IEC 61966-2-1) and the file's orientation.
//!
//! Not applied: the look tables and tone curve of a camera profile that some DNGs carry, lens
//! corrections a file only records as metadata, noise reduction and sharpening.

use craft_color_transform::{Mat3, REC2020, SRGB};
use craft_develop::finish::gamut_map;
use craft_develop::raster::{Image, par_rows};
use craft_develop::sensor::{Method, RawDevelopExt, color, highlight};
use craft_develop::tone::ToneMap;

use crate::FormatError;

/// A normalised sensor sample (1 = the white level) at or above this counts as clipped.
const CLIP: f32 = 0.99;

/// The most samples a raw file may declare: the pixels of the largest image the renderer decodes
/// (512 MiB of RGBA), so a raw that develops is also one the canvas can draw.
const MAX_SAMPLES: usize = 1 << 27;

/// JPEG quality of the developed picture.
const QUALITY: u8 = 95;

/// Luminance weights of the BT.2020 primaries (ITU-R BT.2020, table 4).
const LUMA_2020: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// Luminance weights of the BT.709 primaries (ITU-R BT.709, item 3.2).
const LUMA_709: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The file extensions of the raw containers [`craft_raw`] decodes, lower case. Opening goes by the
/// extension; whether the bytes are a raw file is still [`is_camera_raw`]'s to say.
pub const EXTENSIONS: [&str; 21] = [
    "3fr", "arw", "cr2", "cr3", "crw", "dng", "erf", "fff", "mef", "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw",
];

/// Whether `ext` (any case, without the dot) is one of [`EXTENSIONS`].
pub fn is_extension(ext: &str) -> bool {
    EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether `bytes` are a camera raw file (decodable in this build or not).
pub fn is_camera_raw(bytes: &[u8]) -> bool {
    craft_raw::probe(bytes).is_some()
}

/// A camera raw file developed into an image.
#[derive(Clone, Debug)]
pub struct Developed {
    /// The picture, encoded.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Develop the camera raw file `bytes` (see the module docs). An error names what stopped it: not
/// a raw file, a raw variant this build doesn't decode, or damaged data.
pub fn develop(bytes: &[u8]) -> Result<Developed, FormatError> {
    let format = craft_raw::probe(bytes).ok_or_else(|| raw_error("not a camera raw file"))?;
    if !format.is_supported() {
        return Err(raw_error(format!("{format:?} files can't be read")));
    }
    let mut limits = craft_raw::Limits::default();
    limits.max_samples = MAX_SAMPLES;
    let raw = craft_raw::decode_with(bytes, &limits).map_err(|e| raw_error(e.to_string()))?;

    let white = color::as_shot_white_xy(&raw);
    let transform = color::camera_transform(&raw, white);
    let wb = transform.wb;
    let (mut picture, clipped) = raw.develop_balanced_masked(Method::Ahd, wb, CLIP).map_err(|e| raw_error(e.to_string()))?;
    match clipped {
        Some(mask) => highlight::reconstruct_masked(&mut picture, wb, CLIP, [1.0; 3], &mask),
        None => highlight::reconstruct(&mut picture, wb, CLIP),
    };

    let gain = 2f64.powf(transform.baseline_exposure);
    let gain = if gain.is_finite() { gain } else { 1.0 };
    // White-balanced camera RGB → scene-linear BT.2020, the baseline exposure included.
    let to_2020 = rows(&transform.matrix.mul(&Mat3::diag(f64::from(wb[0]) * gain, f64::from(wb[1]) * gain, f64::from(wb[2]) * gain)));
    let to_709 = rows(&SRGB.from_xyz().mul(&REC2020.to_xyz()));
    let tone = ToneMap::rolloff_hdr(0.0, 0.0, 0.0, 1.0);

    let (width, height) = (picture.width, picture.height);
    let mut rgb: Image<[u8; 3]> = Image::new(width, height);
    let source = &picture.data;
    par_rows(&mut rgb.data, width, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let scene = apply(&to_2020, source[y * width + x]);
            let luminance = dot(LUMA_2020, scene);
            let display = if luminance > 0.0 { scene.map(|c| c * tone.apply(luminance) / luminance) } else { [0.0; 3] };
            let (linear, _) = gamut_map(apply(&to_709, display), LUMA_709);
            *out = linear.map(encode_srgb);
        }
    });
    drop(picture);
    let rgb = rgb.into_oriented(craft_geom::Orientation::from_exif(raw.orientation.to_exif()));
    let (width, height) = (u32::try_from(rgb.width).map_err(|_| too_large())?, u32::try_from(rgb.height).map_err(|_| too_large())?);

    let image = craft_image_codecs::Image::from_u8(width, height, craft_image_codecs::ChannelLayout::Rgb, rgb.data.into_flattened())
        .map_err(|e| raw_error(e.to_string()))?;
    let options = craft_image_codecs::EncodeOptions {
        jpeg_quality: QUALITY,
        jpeg_chroma_subsampling: false,
        embed_icc: false,
        embed_metadata: false,
        ..Default::default()
    };
    let bytes = craft_image_codecs::encode(&image, craft_image_codecs::Format::Jpeg, &options).map_err(|e| raw_error(e.to_string()))?;
    Ok(Developed { bytes, width, height })
}

fn raw_error(reason: impl std::fmt::Display) -> FormatError {
    FormatError::Corrupt(format!("unsupported image: camera raw: {reason}"))
}

fn too_large() -> FormatError {
    raw_error("the picture is too large")
}

fn rows(m: &Mat3) -> [[f32; 3]; 3] {
    m.0.map(|row| row.map(|v| v as f32))
}

fn apply(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    m.map(|row| dot(row, v))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Linear `v` (0 to 1) as an 8-bit sRGB code (IEC 61966-2-1, the inverse of its equation 1).
fn encode_srgb(v: f32) -> u8 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let e = if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0 + 0.5) as u8
}
