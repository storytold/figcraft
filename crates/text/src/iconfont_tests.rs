//! Building an icon font from outlines: the result is a real font the database can read.

use super::*;
use crate::fontdb::{Face, FontBytes};
use figcraft_geom::Shape;
use figcraft_geom::kurbo::PathEl;
use std::sync::Arc;

/// A circle as four cubic segments, wound one way.
fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    let k = 0.5522847498307936 * r;
    let mut p = BezPath::new();
    p.move_to((cx + r, cy));
    p.curve_to((cx + r, cy + k), (cx + k, cy + r), (cx, cy + r));
    p.curve_to((cx - k, cy + r), (cx - r, cy + k), (cx - r, cy));
    p.curve_to((cx - r, cy - k), (cx - k, cy - r), (cx, cy - r));
    p.curve_to((cx + k, cy - r), (cx + r, cy - k), (cx + r, cy));
    p.close_path();
    p
}

/// The same circle wound the other way — used to oppose an outline so its centre is a hole.
fn circle_rev(cx: f64, cy: f64, r: f64) -> BezPath {
    let k = 0.5522847498307936 * r;
    let mut p = BezPath::new();
    p.move_to((cx + r, cy));
    p.curve_to((cx + r, cy - k), (cx + k, cy - r), (cx, cy - r));
    p.curve_to((cx - k, cy - r), (cx - r, cy - k), (cx - r, cy));
    p.curve_to((cx - r, cy + k), (cx - k, cy + r), (cx, cy + r));
    p.curve_to((cx + k, cy + r), (cx + r, cy + k), (cx + r, cy));
    p.close_path();
    p
}

/// A donut whose hole is already wound against its outline — the shape `normalized` must produce,
/// and the shape `build_ttf` must keep opposed (a `glyf` fills non-zero, so an unopposed hole
/// simply vanishes).
fn curved_donut() -> BezPath {
    let mut p = circle(500.0, 500.0, 450.0);
    p.extend(circle_rev(500.0, 500.0, 200.0).elements().iter().copied());
    p
}

/// The same donut but with both contours wound the same way — so the hole is *not* a hole.
fn curved_solid_donut() -> BezPath {
    let mut p = circle(500.0, 500.0, 450.0);
    p.extend(circle(500.0, 500.0, 200.0).elements().iter().copied());
    p
}

/// Split a path into its subpaths (each `MoveTo` starts a new one).
fn subpaths(p: &BezPath) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = Vec::new();
    for el in p.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            out.push(BezPath::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(*el);
        }
    }
    out
}

/// A square in the em square, y down.
fn square() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((100.0, 100.0));
    p.line_to((900.0, 100.0));
    p.line_to((900.0, 900.0));
    p.line_to((100.0, 900.0));
    p.close_path();
    p
}

/// A shape with a cubic segment (an ellipse's quarter): `glyf` has to be given quadratics.
fn curved() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((100.0, 500.0));
    p.curve_to((100.0, 200.0), (300.0, 100.0), (500.0, 100.0));
    p.line_to((900.0, 500.0));
    p.line_to((100.0, 500.0));
    p.close_path();
    p
}

fn parse(bytes: &[u8]) -> Face {
    let bytes = Arc::new(bytes.to_vec());
    Face::from_bytes(FontBytes::Shared(bytes), 0, None, None).expect("the font parses")
}

/// A table of a built font, by its four-byte tag.
fn table<'a>(ttf: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let num_tables = u16::from_be_bytes([ttf[4], ttf[5]]);
    (0..usize::from(num_tables)).find_map(|i| {
        let e = 12 + i * 16;
        let at = |o: usize| u32::from_be_bytes([ttf[e + o], ttf[e + o + 1], ttf[e + o + 2], ttf[e + o + 3]]) as usize;
        (&ttf[e..e + 4] == tag).then(|| ttf.get(at(8)..at(8) + at(12)).unwrap_or_default())
    })
}

/// `maxp` must say version 1.0, the version a `glyf`-outline font carries.
///
/// `write_fonts` derives the version from whether any of the `max_*` fields is set, so leaving them
/// out wrote 0.5 — the version a PostScript-outline font carries. A system reading 0.5 goes looking
/// for a `CFF ` table, finds none, and refuses the font outright: every exported icon font was
/// unopenable because of this one number.
#[test]
fn the_built_font_says_its_outlines_are_in_glyf() {
    let ttf = build_ttf("Icons", &[(0xE000, square()), (0xE001, curved())]).expect("built");
    let maxp = table(&ttf, b"maxp").expect("maxp");
    assert_eq!(u32::from_be_bytes([maxp[0], maxp[1], maxp[2], maxp[3]]), 0x0001_0000, "maxp is version 1.0");
    let glyphs = u16::from_be_bytes([maxp[4], maxp[5]]);
    assert_eq!(glyphs, 3, "`.notdef` and the two icons");
    // And the tables a system checks before it will open the font at all.
    let head = table(&ttf, b"head").expect("head");
    assert_eq!(u32::from_be_bytes([head[12], head[13], head[14], head[15]]), 0x5F0F_3CF5, "head's magic number");
    assert!(head[18..20] == UPEM.to_be_bytes(), "units per em");
    let os2 = table(&ttf, b"OS/2").expect("OS/2");
    assert_eq!(u16::from_be_bytes([os2[0], os2[1]]), 4, "OS/2 version 4, as the icon-font template has");
    let hhea = table(&ttf, b"hhea").expect("hhea");
    let asc = i16::from_be_bytes([hhea[4], hhea[5]]);
    let desc = i16::from_be_bytes([hhea[6], hhea[7]]);
    assert_eq!(i32::from(asc) - i32::from(desc), i32::from(UPEM), "the ascender-to-descender box is one em: the square slot");
    let hmtx = table(&ttf, b"hmtx").expect("hmtx");
    assert_eq!(u16::from_be_bytes([hmtx[0], hmtx[1]]), UPEM, "every icon advances a full em");
}

/// An icon fills its slot: an outline using the whole em square comes out a square glyph spanning
/// the whole ascender-to-descender box, with nothing above, below or beside it.
#[test]
fn an_icon_fills_the_box_it_is_drawn_in() {
    let em = f64::from(UPEM);
    let mut full = BezPath::new();
    full.move_to((0.0, 0.0));
    full.line_to((em, 0.0));
    full.line_to((em, em));
    full.line_to((0.0, em));
    full.close_path();
    let ttf = build_ttf("Icons", &[(0xE000, full)]).expect("built");
    let head = table(&ttf, b"head").expect("head");
    let i16_at = |o: usize| i16::from_be_bytes([head[o], head[o + 1]]);
    // xMin, yMin, xMax, yMax.
    assert_eq!((i16_at(36), i16_at(40)), (0, UPEM as i16), "the glyph is as wide as the em");
    assert_eq!((i16_at(38), i16_at(42)), (DESCENDER, ASCENDER), "and as tall as the slot");
}

#[test]
fn a_built_font_reads_back_with_its_family_and_glyphs() {
    let bytes = build_ttf("Icons", &[(0xE000, square()), (0xE001, curved())]).expect("built");
    let face = parse(&bytes);
    assert_eq!(face.family, "Icons");
    assert_eq!(face.style, "Regular");
    assert_eq!(face.upem, f64::from(UPEM));
    assert!(face.has_char('\u{E000}'));
    assert!(face.has_char('\u{E001}'));
    assert!(!face.has_char('A'), "only the icons are mapped");
    let gid = face.glyph_for('\u{E000}');
    assert_ne!(gid, 0, "the code point maps to a real glyph");
    assert!(!face.outline(gid).elements().is_empty(), "the glyph has an outline");
    assert_eq!(face.advance(gid), f64::from(ICON_ADVANCE));
    // `.notdef` is glyph 0 and stays empty.
    assert!(face.outline(0).elements().is_empty());
}

#[test]
fn different_code_points_get_different_glyphs() {
    let bytes = build_ttf("Icons", &[(0xE000, square()), (0xE001, curved())]).expect("built");
    let face = parse(&bytes);
    let (a, b) = (face.glyph_for('\u{E000}'), face.glyph_for('\u{E001}'));
    assert_ne!(a, b);
    assert!(a != 0 && b != 0);
}

#[test]
fn the_outline_is_flipped_into_font_space_exactly_once() {
    // The document stores the square at y 100..900 with y down. In the font it must sit above the
    // baseline (y up) in the em square's slot: y 100..900 becomes asc-100..asc-900, that is
    // 775..-25. The y-down outline the layout reads back is the negation, -775..25: the flip
    // happens once, not zero or twice.
    let bytes = build_ttf("Icons", &[(0xE000, square())]).expect("built");
    let face = parse(&bytes);
    let outline = face.outline(face.glyph_for('\u{E000}'));
    let bb = outline.bounding_box();
    assert!((bb.x0 - 100.0).abs() < 2.0, "{bb:?}");
    assert!((bb.x1 - 900.0).abs() < 2.0, "{bb:?}");
    assert!((bb.y0 - f64::from(-ASCENDER + 100)).abs() < 2.0, "{bb:?}");
    assert!((bb.y1 - f64::from(-ASCENDER + 900)).abs() < 2.0, "{bb:?}");
}

#[test]
fn a_font_with_nothing_to_draw_is_an_error() {
    assert!(matches!(build_ttf("Icons", &[]), Err(IconFontError::Empty)));
    // An outline with no elements draws nothing.
    assert!(matches!(build_ttf("Icons", &[(0xE000, BezPath::new())]), Err(IconFontError::Empty)));
    // Neither does a lone move.
    let mut lone = BezPath::new();
    lone.move_to((10.0, 10.0));
    assert!(matches!(build_ttf("Icons", &[(0xE000, lone)]), Err(IconFontError::Empty)));
}

#[test]
fn an_empty_outline_is_skipped_without_losing_the_others() {
    let bytes = build_ttf("Icons", &[(0xE000, BezPath::new()), (0xE001, square())]).expect("built");
    let face = parse(&bytes);
    assert!(!face.has_char('\u{E000}'));
    assert!(face.has_char('\u{E001}'));
}

#[test]
fn code_points_outside_unicode_are_skipped() {
    // A surrogate half is not a code point a character can hold.
    let bytes = build_ttf("Icons", &[(0xD800, square()), (0xE000, square())]).expect("built");
    let face = parse(&bytes);
    assert!(face.has_char('\u{E000}'));
}

#[test]
fn a_family_name_becomes_a_usable_postscript_name() {
    assert_eq!(postscript_name("My Icons 2"), "MyIcons2");
    assert_eq!(postscript_name("图标"), "Icons", "nothing usable left");
    assert_eq!(postscript_name(""), "Icons");
    assert_eq!(postscript_name(&"a".repeat(100)).len(), 63);
    // The family name itself is kept as-is, non-ASCII and all.
    let bytes = build_ttf("图标 2", &[(0xE000, square())]).expect("built");
    assert_eq!(parse(&bytes).family, "图标 2");
}

#[test]
fn a_cubic_outline_compiles_to_quadratics() {
    // `curved()` holds a genuine cubic segment; `glyf` holds lines and quadratics only, so the
    // cubic must be turned into quadratics (and no cubics may survive into the font).
    let bytes = build_ttf("Icons", &[(0xE000, curved())]).expect("built");
    let outline = parse(&bytes).outline(parse(&bytes).glyph_for('\u{E000}'));
    assert!(outline.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))), "the cubic became quadratics");
    assert!(!outline.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))), "no cubics survive into the glyf");
    // The curve spans x 100..900 and y 100..500 (stored y-down); `face.outline` hands it back in
    // y-down space, so it is the negation of the font-space y, `asc - y`, in the same range.
    let bb = outline.bounding_box();
    assert!((bb.x0 - 100.0).abs() < 6.0 && (bb.x1 - 900.0).abs() < 6.0, "{bb:?}");
    assert!((bb.y0 - f64::from(-ASCENDER + 100)).abs() < 8.0, "{bb:?}");
    assert!((bb.y1 - f64::from(-ASCENDER + 500)).abs() < 8.0, "{bb:?}");
}

#[test]
fn a_curved_hole_stays_a_hole_through_the_font() {
    // The donut's two contours are already opposed, the way `normalized` leaves them. Building the
    // font must preserve both contours and their opposition (a `glyf` fills non-zero, so an unopposed
    // hole would be filled in and disappear).
    let bytes = build_ttf("Icons", &[(0xE000, curved_donut())]).expect("built");
    let face = parse(&bytes);
    let outline = face.outline(face.glyph_for('\u{E000}'));
    let subs = subpaths(&outline);
    assert_eq!(subs.len(), 2, "the hole is still a contour of its own");

    // Opposed contours: their signed areas point opposite ways, so non-zero fill leaves the middle
    // out. (`face.outline` hands back a path whose `winding` is uninformative, so we compare areas,
    // which is robust for curves and for the read-back space.)
    let areas: Vec<f64> = subs.iter().map(|s| s.area()).collect();
    assert!(areas.iter().all(|a| a.abs() > 1000.0), "each contour has real area: {areas:?}");
    assert!(areas[0].signum() * areas[1].signum() < 0.0, "the two contours wind against each other: {areas:?}");

    // The two contours differ in size: the hole is the smaller of the two. The readback is in
    // y-down space, but widths and heights survive the flip.
    let mut dims: Vec<f64> = subs
        .iter()
        .map(|s| {
            let b = s.bounding_box();
            b.width().max(b.height())
        })
        .collect();
    dims.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(dims[0] < 450.0, "the hole is the smaller contour: {dims:?}");
    assert!(dims[1] > 800.0, "the outline is the larger one: {dims:?}");

    // Both contours kept their curves.
    assert!(outline.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))));
}

#[test]
fn a_same_winding_curved_donut_fills_its_hole() {
    // If the two contours are wound the same way there is no hole under non-zero fill — and that is
    // exactly why `normalized` must oppose them. This negative control proves the test above is
    // checking the right thing: here the areas share a sign, so the centre would be painted.
    let bytes = build_ttf("Icons", &[(0xE000, curved_solid_donut())]).expect("built");
    let face = parse(&bytes);
    let outline = face.outline(face.glyph_for('\u{E000}'));
    let subs = subpaths(&outline);
    assert_eq!(subs.len(), 2, "still two contours, but wound the same way");
    let areas: Vec<f64> = subs.iter().map(|s| s.area()).collect();
    assert!(areas.iter().all(|a| a.abs() > 1000.0), "each contour has real area: {areas:?}");
    assert!(areas[0].signum() * areas[1].signum() > 0.0, "the contours wind the same way: {areas:?}");
}
