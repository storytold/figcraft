//! Icon fonts: compiling shapes into a real TrueType font at runtime.
//!
//! [`build_ttf`] turns a set of (code point, outline) pairs into a TTF. The document stores icon
//! outlines in the em square with y pointing down ([`figcraft_doc::IconGlyph`]); glyphs are flipped
//! to the font's y-up space here, and cubics become quadratics (a `glyf` outline holds lines and
//! quadratics only).
//!
//! The result is registered in the font database ([`crate::fontdb`]), so text can use an icon font
//! like any other family: pick it and type the code point.

use figcraft_geom::BezPath;
use figcraft_geom::kurbo::{CubicBez, PathEl, Point, Rect};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::Os2;
use write_fonts::tables::post::Post;
use write_fonts::types::{FWord, Fixed, GlyphId, LongDateTime, NameId, Tag, UfWord, Version16Dot16};

/// The em size of every generated icon font: the outlines are expected in a 0..1000 square.
///
/// The one definition of the em square, which is also where [`figcraft_doc::EM`] points: the
/// document stores glyph outlines in it and this crate compiles them at it.
pub const UPEM: u16 = figcraft_doc::EM as u16;
/// The advance width of every icon glyph (a full em, so icons line up like a monospaced font).
const ICON_ADVANCE: u16 = UPEM;
/// How far a cubic may deviate from the quadratics that replace it, in font units.
const QUAD_ACCURACY: f64 = 0.5;
/// The ascender of the generated font: ⅞ of the em, so the ascender-to-descender box is exactly one
/// em tall — the square slot an icon fills. (The icon fonts these are made like carry 896 over 1024.)
const ASCENDER: i16 = (UPEM as i16) * 7 / 8;
/// The descender of the generated font: the last ⅛ of the em, below the baseline.
const DESCENDER: i16 = -(UPEM as i16) / 8;
/// The line gap of the generated font: none, so stacked icons sit as close as the em allows.
const LINE_GAP: i16 = 0;
/// The font vendor id written to `OS/2`.
const VENDOR_ID: &[u8; 4] = b"FCON";
/// The time written to `head`, in seconds since 1904-01-01 (2020-01-01T00:00:00Z).
///
/// A fixed date rather than the clock: it only has to be a plausible timestamp, and a font built
/// twice from the same icons must come out byte for byte the same (see `iconfont`'s build cache).
const BUILT_AT: i64 = 3_660_681_600;

/// Why an icon font could not be built.
#[derive(Debug)]
pub enum IconFontError {
    /// No glyph could be made from the outlines.
    Empty,
    /// A table failed to compile.
    Build(String),
}

impl std::fmt::Display for IconFontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "no glyph to put in the font"),
            Self::Build(e) => write!(f, "can't compile the font: {e}"),
        }
    }
}

impl std::error::Error for IconFontError {}

/// Compile `glyphs` into a TrueType font named `family`.
///
/// Each entry is a Unicode code point and its outline in the em square with y pointing down.
/// Outlines that hold nothing drawable are skipped; a font with no usable glyph is
/// [`IconFontError::Empty`].
pub fn build_ttf(family: &str, glyphs: &[(u32, BezPath)]) -> Result<Vec<u8>, IconFontError> {
    let upem = f64::from(UPEM);
    let mut builder = GlyfLocaBuilder::new();
    // Glyph 0 is `.notdef`: an empty glyph, so a real glyph's id is its index here plus one.
    builder.add_glyph(&Glyph::Empty).map_err(build_err)?;

    let mut metrics: Vec<LongMetric> = vec![LongMetric::new(ICON_ADVANCE, 0)]; // `.notdef`
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    let mut bbox: Option<Rect> = None;
    let mut ranges = [0u32; 4];
    // The largest glyph's shape, for `maxp` (see below).
    let mut max_points = 0u16;
    let mut max_contours = 0u16;

    for (codepoint, path) in glyphs {
        let Some(ch) = char::from_u32(*codepoint) else { continue };
        if path.elements().is_empty() {
            continue;
        }
        // `glyf` holds lines and quadratics only, and its y axis points up.
        let outline = flip_y(&to_quadratic(path, QUAD_ACCURACY), upem);
        let Ok(glyph) = SimpleGlyph::from_bezpath(&outline) else { continue };
        // A path with nothing but a `move_to` still becomes a one-point contour: skip it.
        if !glyph.contours.iter().any(|c| c.len() >= 2) {
            continue;
        }
        let points: usize = glyph.contours.iter().map(|c| c.len()).sum();
        max_points = max_points.max(points.min(usize::from(u16::MAX)) as u16);
        max_contours = max_contours.max(glyph.contours.len().min(usize::from(u16::MAX)) as u16);
        let bb = outline.control_box();
        bbox = Some(match bbox {
            Some(b) => b.union(bb),
            None => bb,
        });
        metrics.push(LongMetric::new(ICON_ADVANCE, round_i16(bb.x0)));
        builder.add_glyph(&glyph).map_err(build_err)?;
        // One metric per glyph, `.notdef` first, so a glyph's id is its metric's index.
        mappings.push((ch, GlyphId::new(metrics.len() as u32 - 1)));
        set_unicode_range(&mut ranges, *codepoint);
    }

    if mappings.is_empty() {
        return Err(IconFontError::Empty);
    }
    // `.notdef` counts: the glyph count must match the `loca` offsets (one more than glyphs), and
    // `hhea.number_of_h_metrics` must match the long metrics `hmtx` holds.
    let num_glyphs = metrics.len() as u16;
    let (glyf, loca, loca_format) = builder.build();
    let bb = bbox.unwrap_or_default();
    let first = mappings.iter().map(|(c, _)| *c as u32).min().unwrap_or(0);
    let last = mappings.iter().map(|(c, _)| *c as u32).max().unwrap_or(0);

    let cmap = Cmap::from_mappings(mappings).map_err(build_err)?;

    let head = Head {
        font_revision: Fixed::from_i32(1),
        units_per_em: UPEM,
        x_min: round_i16(bb.x0),
        y_min: round_i16(bb.y0),
        x_max: round_i16(bb.x1),
        y_max: round_i16(bb.y1),
        lowest_rec_ppem: 16,
        created: LongDateTime::new(BUILT_AT),
        modified: LongDateTime::new(BUILT_AT),
        // 0 for short `loca` offsets, 1 for long ones: the two tables must agree.
        index_to_loc_format: match loca_format {
            LocaFormat::Short => 0,
            LocaFormat::Long => 1,
        },
        ..Default::default()
    };

    let hhea = Hhea::new(
        FWord::new(ASCENDER),
        FWord::new(DESCENDER),
        FWord::new(LINE_GAP),
        UfWord::new(ICON_ADVANCE),
        FWord::new(0),
        FWord::new(0),
        FWord::new(round_i16(bb.x1)),
        1,
        0,
        0,
        num_glyphs,
    );

    let hmtx = Hmtx::new(metrics, Vec::new());
    // `maxp` must be version 1.0, which `write_fonts` derives from whether any of these are set.
    // Left unset it writes version 0.5 — the version a PostScript-outline font carries — and a
    // system that believes it looks for a `CFF ` table, finds no outlines there, and refuses the
    // font outright. There is no hinting in a font of icons, so only the shape counts.
    let maxp = Maxp {
        num_glyphs,
        max_points: Some(max_points),
        max_contours: Some(max_contours),
        max_composite_points: Some(0),
        max_composite_contours: Some(0),
        max_zones: Some(2),
        max_twilight_points: Some(0),
        max_storage: Some(0),
        max_function_defs: Some(0),
        max_instruction_defs: Some(0),
        max_stack_elements: Some(0),
        max_size_of_instructions: Some(0),
        max_component_elements: Some(0),
        max_component_depth: Some(0),
    };

    // Records must be sorted by (platform, encoding, language, name id): 1, 2, 3, 4, 6 is.
    let record = |id: NameId, text: String| NameRecord::new(3, 1, 0x409, id, text.into());
    let name = Name::new(vec![
        record(NameId::FAMILY_NAME, family.to_string()),
        record(NameId::SUBFAMILY_NAME, "Regular".to_string()),
        record(NameId::UNIQUE_ID, format!("{family}: FigCraft icon font")),
        record(NameId::FULL_NAME, format!("{family} Regular")),
        record(NameId::POSTSCRIPT_NAME, postscript_name(family)),
    ]);

    // Version 4 (`sx_height` and `s_cap_height` are set): what fonts of icons ship, and the version
    // Windows recommends. Without them the table is written as version 0.
    let os2 = Os2 {
        x_avg_char_width: UPEM as i16,
        s_family_class: 0,
        panose_10: [0; 10],
        ul_unicode_range_1: ranges[0],
        ul_unicode_range_2: ranges[1],
        ul_unicode_range_3: ranges[2],
        ul_unicode_range_4: ranges[3],
        ach_vend_id: Tag::new(VENDOR_ID),
        us_first_char_index: clamp_u16(first),
        us_last_char_index: clamp_u16(last),
        s_typo_ascender: ASCENDER,
        s_typo_descender: DESCENDER,
        s_typo_line_gap: 0,
        us_win_ascent: UPEM,
        us_win_descent: DESCENDER.unsigned_abs(),
        sx_height: Some(500),
        s_cap_height: Some(700),
        us_default_char: Some(0),
        us_break_char: Some(0),
        us_max_context: Some(1),
        // Version 4 requires both of these. Bit 0 of range 1 is the 1252 Latin 1 code page: an icon
        // font holds no text, but the field has to be there or the table will not compile.
        ul_code_page_range_1: Some(1),
        ul_code_page_range_2: Some(0),
        ..Default::default()
    };

    // Version 3.0 keeps no glyph names: the smallest `post` a font can have.
    let post = Post { version: Version16Dot16::VERSION_3_0, ..Default::default() };

    let mut font = FontBuilder::new();
    font.add_table(&head).map_err(build_err)?;
    font.add_table(&hhea).map_err(build_err)?;
    font.add_table(&maxp).map_err(build_err)?;
    font.add_table(&os2).map_err(build_err)?;
    font.add_table(&hmtx).map_err(build_err)?;
    font.add_table(&cmap).map_err(build_err)?;
    font.add_table(&loca).map_err(build_err)?;
    font.add_table(&glyf).map_err(build_err)?;
    font.add_table(&name).map_err(build_err)?;
    font.add_table(&post).map_err(build_err)?;
    Ok(font.build())
}

/// Replace cubic segments with quadratics (a `glyf` outline has no cubics).
fn to_quadratic(path: &BezPath, accuracy: f64) -> BezPath {
    let mut out = BezPath::new();
    let mut current = Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                current = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                current = p;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                current = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                for (_, _, quad) in CubicBez::new(current, c1, c2, p).to_quads(accuracy) {
                    out.quad_to(quad.p1, quad.p2);
                }
                current = p;
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// Turn an outline in the em square (y down) into font units (y up, baseline at 0).
///
/// The em square lands exactly on the ascender..descender box, which is one em tall: an icon fills
/// that square slot edge to edge, the way the template icon fonts do, instead of sitting in a corner
/// of a taller box with margins around it.
fn flip_y(path: &BezPath, upem: f64) -> BezPath {
    let mut out = BezPath::new();
    let asc = f64::from(ASCENDER);
    let span = f64::from(ASCENDER - DESCENDER);
    let scale = span / upem;
    for el in path.elements() {
        let p = |pt: Point| Point::new(pt.x, asc - pt.y * scale);
        match *el {
            PathEl::MoveTo(a) => out.move_to(p(a)),
            PathEl::LineTo(a) => out.line_to(p(a)),
            PathEl::QuadTo(a, b) => out.quad_to(p(a), p(b)),
            PathEl::CurveTo(a, b, c) => out.curve_to(p(a), p(b), p(c)),
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// Set the `OS/2` Unicode range bit for a code point's block (the private use areas).
fn set_unicode_range(ranges: &mut [u32; 4], codepoint: u32) {
    match codepoint {
        0xE000..=0xF8FF => ranges[0] |= 1 << 31,      // Private Use Area
        0xF_0000..=0xF_FFFD => ranges[2] |= 1 << 26,  // Plane 15 Private Use Area
        0x10_0000..=0x10_FFFD => ranges[3] |= 1 << 2, // Plane 16 Private Use Area
        _ => {}
    }
}

/// `value` as an `i16`, clamped: font coordinates are 16-bit.
fn round_i16(value: f64) -> i16 {
    let r = value.round();
    if !r.is_finite() {
        return 0;
    }
    r.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// `value` as a `u16`, clamped (the `OS/2` character indexes cover the BMP only).
fn clamp_u16(value: u32) -> u16 {
    value.min(u32::from(u16::MAX)) as u16
}

/// A PostScript name: ASCII letters and digits only, at most 63 characters.
fn postscript_name(family: &str) -> String {
    let cleaned: String = family.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if cleaned.is_empty() {
        return "Icons".to_string();
    }
    cleaned.chars().take(63).collect()
}

fn build_err(e: impl std::fmt::Display) -> IconFontError {
    IconFontError::Build(e.to_string())
}

#[cfg(test)]
#[path = "iconfont_tests.rs"]
mod tests;
