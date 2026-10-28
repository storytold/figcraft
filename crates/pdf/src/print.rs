//! Print options: bleed, printer's marks, page boxes and CMYK colour.
//!
//! - **Bleed.** The page grows by the bleed on every side and shows what lies past the trim
//!   there: a frame's fills reach out to the bleed edge (gradients and images keep their place;
//!   they just show further), its children are clipped to the bleed instead of the frame, and
//!   nothing is drawn past the bleed.
//! - **Marks.** Crop marks (at the trim's corners) and bleed marks (at the bleed's) are hairlines
//!   [`MARK_LENGTH`] long, starting [`MARK_GAP`] outside the bleed, in registration colour (all
//!   four inks in CMYK, black in RGB). The page grows to hold them.
//! - **Boxes.** MediaBox is the whole sheet, BleedBox the trim grown by the bleed, TrimBox the
//!   frame (the finished page).
//! - **CMYK.** Colours are converted from sRGB with the naive, profile-free formula
//!   `K = 1 − max(R, G, B)`, `C = (1 − R − K) / (1 − K)` (likewise M and Y), so the same file
//!   always converts the same way; images and rasterised effects are converted pixel by pixel.
//!   The colours are DeviceCMYK with no output intent: embedding one needs a CMYK ICC profile,
//!   and none ships with FigCraft (third-party profiles aren't allowed in the repository). Print
//!   shops should proof against their own profile. Soft masks stay greyscale/RGB (they're never
//!   printed), and transparency groups (blend modes, soft masks) blend in RGB as the PDF writer
//!   declares them.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use figcraft_color::Color;
use figcraft_doc::PrintSettings;
use figcraft_geom::{BezPath, Rect, Shape};
use krilla::color::{cmyk, rgb};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use krilla::paint::{FillRule, LineCap, Stroke};
use krilla::surface::Surface;

use crate::draw::{norm, to_path};

/// Length of a printer's mark, in points.
pub const MARK_LENGTH: f64 = 18.0;
/// Space between the bleed and the marks, in points.
pub const MARK_GAP: f64 = 3.0;
/// Width of a printer's mark, in points.
pub const MARK_WIDTH: f32 = 0.25;

/// sRGB → CMYK (all `0..=1`), the naive way: `K = 1 − max(R, G, B)`, `C = (1 − R − K) / (1 − K)`.
pub fn cmyk_of(r: f32, g: f32, b: f32) -> [f32; 4] {
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let (r, g, b) = (unit(r), unit(g), unit(b));
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 - 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let ink = |v: f32| ((1.0 - v - k) / (1.0 - k)).clamp(0.0, 1.0);
    [ink(r), ink(g), ink(b), k]
}

fn byte(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The colour model colours are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    Rgb,
    Cmyk,
    /// RGB with equal channels (Rec. 709 luma): a grayscale print.
    Gray,
}

/// Gray of an 8-bit colour with Rec. 709 weights, in integers that sum to 256 (white stays 255, black 0).
pub fn gray_of(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 54 + u32::from(g) * 183 + u32::from(b) * 19) >> 8) as u8
}

/// Turn RGBA pixels (4 bytes each; alpha is kept) gray in place with [`gray_of`].
pub fn gray_rgba_in_place(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        let y = gray_of(px[0], px[1], px[2]);
        px[..3].fill(y);
    }
}

impl Ink {
    pub(crate) fn color(self, c: Color) -> krilla::color::Color {
        let [r, g, b, _] = c.sanitized().to_rgba8();
        match self {
            Ink::Rgb => rgb::Color::new(r, g, b).into(),
            Ink::Gray => {
                let y = gray_of(r, g, b);
                rgb::Color::new(y, y, y).into()
            }
            Ink::Cmyk => {
                let f = |v: u8| f32::from(v) / 255.0;
                let [c, m, y, k] = cmyk_of(f(r), f(g), f(b));
                cmyk::Color::new(byte(c), byte(m), byte(y), byte(k)).into()
            }
        }
    }

    /// The colour that prints on every plate (marks): all four inks, or black.
    fn registration(self) -> krilla::color::Color {
        match self {
            Ink::Rgb | Ink::Gray => rgb::Color::black().into(),
            Ink::Cmyk => cmyk::Color::new(255, 255, 255, 255).into(),
        }
    }

    /// A `w × h` image from straight (not premultiplied) RGBA pixels, in this colour model.
    pub(crate) fn bitmap(self, mut rgba: Vec<u8>, w: u32, h: u32) -> Image {
        if self == Ink::Gray {
            gray_rgba_in_place(&mut rgba);
        }
        if self == Ink::Cmyk
            && let Some(img) = CmykImage::new(&rgba, w, h).and_then(|c| Image::from_custom(c, true).ok())
        {
            return img;
        }
        Image::from_rgba8(rgba, w, h)
    }
}

/// CMYK pixels (and alpha) converted from RGBA.
#[derive(Clone)]
struct CmykImage {
    color: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
    size: (u32, u32),
    key: u64,
}

impl CmykImage {
    fn new(rgba: &[u8], w: u32, h: u32) -> Option<Self> {
        let n = (w as usize).checked_mul(h as usize)?;
        if rgba.len() != n.checked_mul(4)? || n == 0 {
            return None;
        }
        let mut color = Vec::with_capacity(n * 4);
        let mut alpha = Vec::with_capacity(n);
        for px in rgba.chunks_exact(4) {
            let f = |i: usize| f32::from(px.get(i).copied().unwrap_or(0)) / 255.0;
            color.extend(cmyk_of(f(0), f(1), f(2)).map(byte));
            alpha.push(px.get(3).copied().unwrap_or(255));
        }
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        rgba.hash(&mut hs);
        let opaque = alpha.iter().all(|a| *a == 255);
        Some(CmykImage { color: Arc::new(color), alpha: (!opaque).then(|| Arc::new(alpha)), size: (w, h), key: hs.finish() })
    }
}

impl Hash for CmykImage {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.size.hash(state);
    }
}

impl CustomImage for CmykImage {
    fn color_channel(&self) -> &[u8] {
        self.color.as_slice()
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        self.alpha.as_deref().map(Vec::as_slice)
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Cmyk
    }
}

/// A printed page: its trim (the frame, `w × h` points) on a sheet grown by the bleed and the
/// marks. Rectangles are in sheet coordinates (points, y down from the sheet's top-left).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sheet {
    pub width: f64,
    pub height: f64,
    pub trim: Rect,
    pub bleed: f64,
    /// Space around the trim (bleed and marks).
    pub margin: f64,
    pub crop_marks: bool,
    pub bleed_marks: bool,
}

impl Sheet {
    pub(crate) fn new(w: f64, h: f64, p: &PrintSettings) -> Self {
        let bleed = p.bleed();
        let marks = p.marks();
        let margin = if marks { bleed + 2.0 * MARK_GAP + MARK_LENGTH } else { bleed };
        Sheet {
            width: w + 2.0 * margin,
            height: h + 2.0 * margin,
            trim: Rect::new(margin, margin, margin + w, margin + h),
            bleed,
            margin,
            crop_marks: p.crop_marks,
            bleed_marks: p.bleed_marks && bleed > 0.0,
        }
    }

    pub(crate) fn bleed_box(&self) -> Rect {
        self.trim.inflate(self.bleed, self.bleed)
    }

    /// The marks: at each corner of `r`, a horizontal and a vertical line outside the bleed.
    fn mark_lines(&self, r: Rect) -> BezPath {
        let (near, far) = (self.bleed + MARK_GAP, self.bleed + MARK_GAP + MARK_LENGTH);
        let t = self.trim;
        let mut p = BezPath::new();
        let mut line = |a: (f64, f64), b: (f64, f64)| {
            p.move_to(a);
            p.line_to(b);
        };
        for (x, sx) in [(r.x0, -1.0), (r.x1, 1.0)] {
            for (y, sy) in [(r.y0, -1.0), (r.y1, 1.0)] {
                // Out from the trim's corner, along the line of `r`'s edge.
                let (tx, ty) = (if sx < 0.0 { t.x0 } else { t.x1 }, if sy < 0.0 { t.y0 } else { t.y1 });
                line((tx + sx * near, y), (tx + sx * far, y));
                line((x, ty + sy * near), (x, ty + sy * far));
            }
        }
        p
    }

    /// Draw the printer's marks (the surface in sheet coordinates).
    pub(crate) fn draw_marks(&self, s: &mut Surface, ink: Ink) {
        let mut lines = BezPath::new();
        if self.crop_marks {
            lines.extend(self.mark_lines(self.trim));
        }
        if self.bleed_marks {
            lines.extend(self.mark_lines(self.bleed_box()));
        }
        let Some(path) = to_path(&lines) else { return };
        s.set_fill(None);
        s.set_stroke(Some(Stroke {
            paint: ink.registration().into(),
            width: MARK_WIDTH,
            line_cap: LineCap::Butt,
            opacity: norm(1.0),
            ..Stroke::default()
        }));
        s.draw_path(&path);
        s.set_stroke(None);
    }

    /// Clip what follows to the bleed (pop it after).
    pub(crate) fn push_bleed_clip(&self, s: &mut Surface) -> bool {
        let Some(p) = to_path(&self.bleed_box().to_path(0.1)) else { return false };
        s.push_clip_path(&p, &FillRule::NonZero);
        true
    }
}
