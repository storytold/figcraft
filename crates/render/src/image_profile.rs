//! Images' own colour spaces: embedded ICC profiles (PNG `iCCP`, JPEG APP2, WebP `ICCP`, TIFF) and
//! PNG `sRGB` / `gAMA` + `cHRM` chunks. Decoding converts an image from its space into the
//! document's (sRGB or Display P3) with moxcms, which runs matrix/TRC and LUT RGB and grey
//! profiles. Images without a profile, or with one that can't be used (CMYK, Lab, damaged), are
//! taken as sRGB.

use std::io::Cursor;

use figcraft_color::ColorProfile;
use moxcms::{Chromaticity, ColorPrimaries, DataColorSpace, Layout, ParsingOptions, ProfileText, ToneReprCurve, TransformOptions, XyY};

/// The most bytes an embedded profile may have; bigger ones are ignored.
pub const MAX_ICC_BYTES: usize = 4 << 20;
/// The most characters of a profile's name that are kept.
const MAX_NAME_CHARS: usize = 64;

/// An image's own colour space.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// The profile's name (`"Display P3"`, `"Adobe RGB (1998)"`, `"sRGB"`; empty for a usable
    /// profile without one); `None` when unknown, and the image is taken as sRGB.
    pub name: Option<String>,
    /// The profile the pixels convert from; `None` for sRGB.
    profile: Option<moxcms::ColorProfile>,
}

impl Source {
    fn srgb() -> Source {
        Source { name: Some("sRGB".into()), profile: None }
    }

    /// The colour space of encoded image `bytes` (reads the headers or embedded profile).
    pub fn of(bytes: &[u8]) -> Source {
        let Some(format) = craft_image_codecs::detect(bytes) else {
            return Source::default();
        };
        if !matches!(
            format,
            craft_image_codecs::Format::Png | craft_image_codecs::Format::Jpeg | craft_image_codecs::Format::WebP | craft_image_codecs::Format::Tiff
        ) {
            return Source::default();
        }
        let icc = craft_image_codecs::embedded_icc(bytes).ok().flatten();
        Source::from_decoded(bytes, icc.as_deref())
    }

    /// The colour space of `bytes` given the ICC profile returned by `craft-image-codecs`.
    /// shortcut: GIF and BMP profiles and a PNG `cICP` without `iCCP` are ignored (taken as sRGB);
    /// read them if such files turn up.
    pub(crate) fn from_decoded(bytes: &[u8], icc: Option<&[u8]>) -> Source {
        let Some(format) = craft_image_codecs::detect(bytes) else {
            return Source::default();
        };
        if !matches!(
            format,
            craft_image_codecs::Format::Png | craft_image_codecs::Format::Jpeg | craft_image_codecs::Format::WebP | craft_image_codecs::Format::Tiff
        ) {
            return Source::default();
        }
        match icc {
            Some(icc) => from_icc(icc),
            None if format == craft_image_codecs::Format::Png => png_chunks(bytes),
            None => Source::default(),
        }
    }

    /// Whether this source uses a greyscale ICC profile.
    pub(crate) fn is_gray(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| p.color_space == DataColorSpace::Gray)
    }

    /// `pixels` (8-bit `GrayAlpha` when `is_gray`, else `Rgba`) converted into `target`; `None`
    /// when it's taken as sRGB.
    pub(crate) fn convert_u8(&self, w: u32, h: u32, is_gray: bool, pixels: &[u8], target: ColorProfile) -> Option<Vec<u8>> {
        let src = self.profile.as_ref()?;
        let grey = src.color_space == DataColorSpace::Gray;
        if grey && !is_gray {
            return None;
        }
        let layout = if grey { Layout::GrayAlpha } else { Layout::Rgba };
        let len = usize::try_from(w).ok()?.checked_mul(usize::try_from(h).ok()?)?.checked_mul(4)?;
        let mut out = vec![0u8; len];
        src.create_transform_8bit(layout, &working(target), Layout::Rgba, TransformOptions::default()).ok()?.transform(pixels, &mut out).ok()?;
        Some(out)
    }
}

/// The document's colour space as a moxcms profile.
fn working(p: ColorProfile) -> moxcms::ColorProfile {
    match p {
        ColorProfile::Srgb => moxcms::ColorProfile::new_srgb(),
        ColorProfile::DisplayP3 => moxcms::ColorProfile::new_display_p3(),
    }
}

/// An ICC profile's colour space; RGB and grey display/input profiles only.
fn from_icc(icc: &[u8]) -> Source {
    if icc.len() > MAX_ICC_BYTES {
        return Source::default();
    }
    let options = ParsingOptions { max_profile_size: MAX_ICC_BYTES + 1, ..Default::default() };
    match moxcms::ColorProfile::new_from_slice_with_options(icc, options) {
        Ok(p) => managed(name(&p), p),
        Err(_) => Source::default(),
    }
}

/// `p` as a source when it converts into the working spaces (finite colorants, a supported colour
/// space and tags moxcms can run); unknown otherwise.
fn managed(name: String, p: moxcms::ColorProfile) -> Source {
    let layout = match p.color_space {
        DataColorSpace::Rgb => Layout::Rgba,
        DataColorSpace::Gray => Layout::GrayAlpha,
        _ => return Source::default(),
    };
    let finite = p.rgb_to_xyz_matrix().v.iter().flatten().all(|v| v.is_finite());
    if !finite || p.create_transform_8bit(layout, &working(ColorProfile::Srgb), Layout::Rgba, TransformOptions::default()).is_err() {
        return Source::default();
    }
    Source { name: Some(name), profile: Some(p) }
}

/// A profile's description, cleaned up for display.
fn name(p: &moxcms::ColorProfile) -> String {
    let text = match &p.description {
        Some(ProfileText::PlainString(s)) => s.as_str(),
        Some(ProfileText::Localizable(l)) => l.iter().find(|s| s.language == "en").or(l.first()).map_or("", |s| s.value.as_str()),
        Some(ProfileText::Description(d)) if d.ascii_string.trim_matches(char::from(0)).is_empty() => d.unicode_string.as_str(),
        Some(ProfileText::Description(d)) => d.ascii_string.as_str(),
        None => "",
    };
    text.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect::<String>().trim().to_string()
}

/// sRGB's white point and primaries (CIE 1931 xy), as a PNG `cHRM` chunk lists them.
const SRGB_CHRM: [(f32, f32); 4] = [(0.3127, 0.3290), (0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];

/// A PNG's `sRGB` chunk, or its `gAMA` chunk with `cHRM` primaries (sRGB's when missing or invalid).
fn png_chunks(bytes: &[u8]) -> Source {
    let Ok(reader) = png::Decoder::new(Cursor::new(bytes)).read_info() else { return Source::default() };
    let info = reader.info();
    if info.srgb.is_some() {
        return Source::srgb();
    }
    let Some(gamma) = info.gama_chunk.map(|g| g.into_value()).filter(|g| g.is_finite() && (0.1..=10.0).contains(g)) else {
        return Source::default();
    };
    let xy = |(x, y): (png::ScaledFloat, png::ScaledFloat)| Chromaticity::new(x.into_value(), y.into_value());
    let chrm = info
        .chrm_chunk
        .map(|c| [xy(c.white), xy(c.red), xy(c.green), xy(c.blue)])
        .filter(|cs| cs.iter().all(|c| c.x.is_finite() && c.y.is_finite() && c.x >= 0.0 && c.y > 0.001 && c.x + c.y <= 1.0));
    let near = |cs: &[Chromaticity; 4]| cs.iter().zip(SRGB_CHRM).all(|(c, (x, y))| (c.x - x).abs() < 0.01 && (c.y - y).abs() < 0.01);
    // gAMA 1/2.2 is what PNG writers put next to (or instead of) an `sRGB` chunk: it means sRGB.
    if chrm.as_ref().is_none_or(near) && (gamma * 2.2 - 1.0).abs() < 0.01 {
        return Source::srgb();
    }
    let mut p = moxcms::ColorProfile::new_srgb();
    if let Some([w, red, green, blue]) = chrm {
        p.update_rgb_colorimetry(XyY::new(f64::from(w.x), f64::from(w.y), 1.0), ColorPrimaries { red, green, blue });
    }
    // Without this moxcms may take the sRGB curve that sRGB's CICP code points name.
    p.cicp = None;
    let curve = ToneReprCurve::Parametric(vec![1.0 / gamma]);
    (p.red_trc, p.green_trc, p.blue_trc) = (Some(curve.clone()), Some(curve.clone()), Some(curve));
    managed(format!("Gamma {:.2}", 1.0 / gamma), p)
}
