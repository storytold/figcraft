//! Text metric rules measured against documents re-saved by other software: line heights worked
//! out from the font are whole pixels (ones given in pixels keep their fraction), advances sit on a grid of 1024 steps per em, and letter spacing goes between glyphs
//! only. Each case derives its numbers from the face in use (skipped on a machine without fonts);
//! the last test pins sizes that were measured, for the one font build they were measured with.

use super::*;
use figcraft_doc::{LetterSpacing, LineHeight, TextAutoResize, TextData};

fn face() -> Option<std::sync::Arc<Face>> {
    scan_system_fonts();
    db().write().unwrap().default_face()
}

fn text(s: &str, f: &Face, size: f64) -> TextData {
    let mut t = TextData::new(s);
    (t.style.font_family, t.style.font_style) = (f.family.clone(), f.style.clone());
    t.style.font_size = size;
    t.auto_resize = TextAutoResize::WidthAndHeight;
    t
}

#[test]
fn line_heights_are_whole_pixels() {
    let Some(f) = face() else { return };
    let own = |size: f64| (f.ascent + f.descent + f.line_gap.max(0.0)) * size / f.upem;
    // (font size, line height, expected line height, derivation)
    let cases = [
        (11.0, LineHeight::Percent(130.0), 14.0, "14.3 rounds down"),
        (13.0, LineHeight::Percent(140.0), 18.0, "18.2 rounds down"),
        (15.0, LineHeight::Percent(150.0), 23.0, "22.5: halves round up"),
        (17.0, LineHeight::Percent(150.0), 26.0, "25.5: halves round up"),
        (22.0, LineHeight::Percent(125.0), 28.0, "27.5: halves round up"),
        (48.0, LineHeight::Percent(110.0), 53.0, "52.8 rounds up"),
        (14.0, LineHeight::Pixels(20.0), 20.0, "whole pixels stay"),
        (14.0, LineHeight::Auto, (own(14.0) + 0.5).floor(), "Auto: the font's own line height, rounded"),
        (20.0, LineHeight::Auto, (own(20.0) + 0.5).floor(), "Auto at 20 px"),
        (24.0, LineHeight::Auto, (own(24.0) + 0.5).floor(), "Auto at 24 px"),
        (32.0, LineHeight::Auto, (own(32.0) + 0.5).floor(), "Auto at 32 px"),
        (0.5, LineHeight::Percent(100.0), 0.5, "under a pixel: kept, so lines don't collapse"),
    ];
    for (size, lh, want, why) in cases {
        let mut t = text("Ag\nAg", &f, size);
        t.style.line_height = lh;
        let l = layout(&t, None, None);
        assert_eq!(l.lines.len(), 2);
        assert!((l.height - 2.0 * want).abs() < 1e-9, "{why}: two lines are {} tall, not {}", l.height, 2.0 * want);
        // The rounded height is what the glyphs centre in; the baseline itself is not rounded.
        let (asc, desc) = (f.ascent * size / f.upem, f.descent * size / f.upem);
        for (i, line) in l.lines.iter().enumerate() {
            let baseline = i as f64 * want + (want - asc - desc) / 2.0 + asc;
            assert!((line.height - want).abs() < 1e-9 && (line.baseline - baseline).abs() < 1e-9, "{why}: line {i} baseline {}", line.baseline);
        }
    }
}

/// A line height given in pixels keeps its fraction: lines 22.5 px apart stay 22.5 apart, the
/// glyphs centred in each, and the box rounds the lines' total up to a whole pixel (three lines of
/// 22.5 are 68 tall, three of 18.4 are 56), as the reference tool lays them out.
#[test]
fn pixel_line_heights_keep_their_fraction() {
    let Some(f) = face() else { return };
    for (size, px, lines, want_box) in [(17.0, 22.5, 3, 68.0), (17.0, 22.5, 4, 90.0), (16.0, 18.4, 3, 56.0), (14.0, 20.4, 2, 41.0)] {
        let mut t = text(&vec!["Ag"; lines].join("\n"), &f, size);
        t.style.line_height = LineHeight::Pixels(px);
        let l = layout(&t, None, None);
        assert_eq!(l.lines.len(), lines);
        assert_eq!(l.height, want_box, "{lines} lines of {px} px");
        let (asc, desc) = (f.ascent * size / f.upem, f.descent * size / f.upem);
        for (i, line) in l.lines.iter().enumerate() {
            let baseline = i as f64 * px + (px - asc - desc) / 2.0 + asc;
            assert!((line.height - px).abs() < 1e-9 && (line.baseline - baseline).abs() < 1e-9, "{px} px: line {i} baseline {}", line.baseline);
        }
    }
}

#[test]
fn advances_sit_on_a_grid_of_1024_per_em() {
    let Some(f) = face() else { return };
    for size in [11.0, 13.0, 15.0, 48.0] {
        let l = layout(&text("Hamburgefonstiv AVATAR To Ty.", &f, size), None, None);
        for g in &l.glyphs {
            let steps = g.advance / size * 1024.0;
            assert!((steps - steps.round()).abs() < 1e-6, "{size} px: glyph {} advances {steps} steps", g.gid);
        }
        // One glyph, so no kerning: its own advance, rounded to the grid, times the size.
        let l = layout(&text("n", &f, size), None, None);
        let own = (f.advance(f.glyph_for('n')) * 1024.0 / f.upem).round() / 1024.0 * size;
        assert!((l.width - own).abs() < 1e-9, "{size} px: {} != {own}", l.width);
    }
}

#[test]
fn letter_spacing_goes_between_glyphs_only() {
    let Some(f) = face() else { return };
    let plain = layout(&text("abc", &f, 20.0), None, None);
    for (ls, px) in [(LetterSpacing::Pixels(5.0), 5.0), (LetterSpacing::Percent(8.0), 1.6), (LetterSpacing::Percent(-3.0), -0.6)] {
        let mut t = text("abc", &f, 20.0);
        t.style.letter_spacing = ls;
        let l = layout(&t, None, None);
        assert!((l.width - plain.width - 2.0 * px).abs() < 1e-9, "{ls:?}: two gaps, {} vs {}", l.width, plain.width);
        for (i, (g, p)) in l.glyphs.iter().zip(&plain.glyphs).enumerate() {
            assert!((g.x - p.x - i as f64 * px).abs() < 1e-9, "{ls:?}: glyph {i} at {}", g.x);
        }
        // Trailing spaces stay outside the width, and so does the spacing before them.
        t.characters = "abc  ".into();
        assert!((layout(&t, None, None).width - l.width).abs() < 1e-9, "{ls:?}: trailing spaces");
        // A box as wide as the text holds it on one line.
        t.characters = "abc".into();
        t.auto_resize = TextAutoResize::Height;
        assert_eq!(layout(&t, Some(l.width.ceil()), None).lines.len(), 1, "{ls:?}: wraps in its own width");
    }
}

/// Box sizes measured for Inter 3 (2816 units per em); skipped where that build isn't installed.
#[test]
fn measured_box_sizes() {
    scan_system_fonts();
    // (text, style, size, line height %, letter spacing %, box width, box height)
    let cases = [
        ("RIDGELINE UI  ·  VERSION 2.4", "SemiBold", 11.0, 130.0, 8.0, 178.0, 14.0),
        ("Foundations and components", "Bold", 48.0, 110.0, -3.0, 661.0, 53.0),
        ("Colour", "Bold", 22.0, 125.0, -1.0, 72.0, 28.0),
        ("Pine", "SemiBold", 15.0, 140.0, 0.0, 32.0, 21.0),
        ("6 steps", "Medium", 13.0, 140.0, 0.0, 47.0, 18.0),
        ("Slate 300", "Medium", 13.0, 140.0, 0.0, 60.0, 18.0),
        ("Three ramps: Pine for actions, Sunset for highlights, Slate for text and surfaces.", "Regular", 15.0, 150.0, 0.0, 563.0, 23.0),
    ];
    for (s, style, size, lh, ls, w, h) in cases {
        let Some(f) = find_face("Inter", style).filter(|f| f.family == "Inter" && f.style == style && f.upem == 2816.0 && !f.is_variable()) else {
            continue;
        };
        let mut t = text(s, &f, size);
        t.style.line_height = LineHeight::Percent(lh);
        t.style.letter_spacing = LetterSpacing::Percent(ls);
        let l = layout(&t, None, None);
        assert_eq!((l.width.ceil(), l.height), (w, h), "{s:?}: line width {}", l.width);
    }
}
