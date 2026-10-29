//! Text behaviour suite, written from the public documentation of the target app's text settings
//! (and, for line breaking, from UAX #14 and the CJK line-start/line-end rules it describes). Each
//! case gives the rule, the numbers it implies and a one-line derivation. Where the documentation
//! is silent the case says so and pins today's behaviour.
//!
//! Metrics cases read the face actually used (any default face: craft-fonts or the system) and
//! derive the expected numbers from its own ascent, descent, line gap and cap height, so they hold
//! on every machine that has a font; with none they skip. Break and caret rules are font-free.

use super::*;
use crate::breaks::{break_between, grapheme_starts};
use figcraft_doc::{LeadingTrim, LetterSpacing, LineHeight, TextAlignH, TextAutoResize, TextData, TextDecoration};

/// The default face once system fonts are indexed (a Latin face where the system has one, so CJK
/// and emoji come from fallback faces), or None (skip) on a machine without fonts.
fn face() -> Option<std::sync::Arc<Face>> {
    scan_system_fonts();
    db().write().unwrap().default_face()
}

/// A 20 px text in the default face, with the given resize mode.
fn text(s: &str, resize: TextAutoResize) -> TextData {
    let mut t = TextData::new(s);
    if let Some(f) = face() {
        (t.style.font_family, t.style.font_style) = (f.family.clone(), f.style.clone());
    }
    t.style.font_size = 20.0;
    t.auto_resize = resize;
    t
}

/// The chars of each line.
fn line_texts(t: &TextData, l: &TextLayout) -> Vec<String> {
    let chars: Vec<char> = t.characters.chars().collect();
    l.lines.iter().map(|ln| chars.get(ln.char_start..ln.char_end).unwrap_or(&[]).iter().collect::<String>()).collect()
}

const EPS: f64 = 1e-6;

#[test]
fn line_height_auto_percent_and_pixels() {
    let Some(f) = face() else { return };
    let k = 20.0 / f.upem;
    let auto = ((f.ascent + f.descent + f.line_gap.max(0.0)) * k + 0.5).floor();
    // (line height, lines, expected height, derivation)
    let cases: [(LineHeight, &str, f64, &str); 5] = [
        (LineHeight::Auto, "A", auto, "Auto: the font's own line height (ascent + descent + line gap) in whole pixels"),
        (LineHeight::Auto, "A\nB\nC", 3.0 * auto, "every line takes the same Auto height"),
        (LineHeight::Percent(150.0), "A\nB", 60.0, "150% of 20 px = 30 px per line, two lines"),
        (LineHeight::Pixels(40.0), "A\nB\nC", 120.0, "40 px per line, three lines"),
        (LineHeight::Pixels(10.0), "A\nB", 20.0, "a line height below the font size still spaces lines by it"),
    ];
    for (lh, s, want, why) in cases {
        let mut t = text(s, TextAutoResize::WidthAndHeight);
        t.style.line_height = lh;
        let l = layout(&t, None, None);
        assert!((l.height - want).abs() < EPS, "{why}: {} != {want}", l.height);
    }
}

#[test]
fn leading_is_split_evenly_above_and_below_each_line() {
    let Some(f) = face() else { return };
    let k = 20.0 / f.upem;
    let (asc, desc) = (f.ascent * k, f.descent * k);
    for (lh, why) in [
        (LineHeight::Pixels(40.0), "40 px: half of (40 − ascent − descent) above the first line too"),
        (LineHeight::Percent(200.0), "200% = 40 px, same split"),
        (LineHeight::Pixels(12.0), "negative leading is split the same way (glyphs overflow both edges)"),
    ] {
        let mut t = text("A\nB", TextAutoResize::WidthAndHeight);
        t.style.line_height = lh;
        let l = layout(&t, None, None);
        let px = lh.resolve(20.0, 1.0);
        for (i, line) in l.lines.iter().enumerate() {
            let want = i as f64 * px + (px - asc - desc) / 2.0 + asc;
            assert!((line.baseline - want).abs() < EPS, "{why}: line {i} baseline {} != {want}", line.baseline);
        }
    }
}

#[test]
fn letter_spacing_percent_and_pixels() {
    if face().is_none() {
        return;
    }
    let plain = layout(&text("abc", TextAutoResize::WidthAndHeight), None, None).width;
    // (spacing, extra width for 3 chars, derivation)
    let cases: [(LetterSpacing, f64, &str); 4] = [
        (LetterSpacing::Percent(10.0), 4.0, "10% of 20 px = 2 px between the 3 chars (none after the last one: measured)"),
        (LetterSpacing::Pixels(3.0), 6.0, "3 px in each of the 2 gaps"),
        (LetterSpacing::Percent(-5.0), -2.0, "negative spacing tightens: −1 px per gap"),
        (LetterSpacing::Pixels(0.0), 0.0, "0 changes nothing"),
    ];
    for (ls, extra, why) in cases {
        let mut t = text("abc", TextAutoResize::WidthAndHeight);
        t.style.letter_spacing = ls;
        let w = layout(&t, None, None).width;
        assert!((w - plain - extra).abs() < 1e-3, "{why}: {w} vs {plain} + {extra}");
    }
}

#[test]
fn paragraph_spacing_and_indent() {
    if face().is_none() {
        return;
    }
    let mut t = text("A\nB\nC", TextAutoResize::WidthAndHeight);
    t.style.line_height = LineHeight::Pixels(30.0);
    t.paragraph_spacing = 10.0;
    let l = layout(&t, None, None);
    // Between paragraphs only: 3 × 30 + 2 × 10, nothing after the last.
    assert!((l.height - 110.0).abs() < EPS, "{}", l.height);
    assert!((l.lines[1].top - 40.0).abs() < EPS && (l.lines[2].top - 80.0).abs() < EPS);

    // Indent: the first line of every paragraph, on left-aligned text only.
    let words = "one two three four five six seven eight nine ten";
    for (align, want, why) in [
        (TextAlignH::Left, 24.0, "left: the first line starts 24 px in"),
        (TextAlignH::Center, 0.0, "documented: indent applies to left-aligned text only"),
        (TextAlignH::Right, 0.0, "right-aligned (left-to-right paragraph): no indent"),
        (TextAlignH::Justified, 0.0, "justified isn't left-aligned: no indent"),
    ] {
        let mut t = text(&format!("{words}\n{words}"), TextAutoResize::Height);
        t.paragraph_indent = 24.0;
        t.align_h = align;
        let mut plain = t.clone();
        plain.paragraph_indent = 0.0;
        let (l, p) = (layout(&t, Some(160.0), None), layout(&plain, Some(160.0), None));
        let first_x = |l: &TextLayout, li: usize| l.glyphs[l.lines[li].glyph_start].x;
        assert!((first_x(&l, 0) - first_x(&p, 0) - want).abs() < 1e-3, "{why}");
        assert!((first_x(&l, 1) - first_x(&p, 1)).abs() < 1e-3, "{why}: wrapped lines don't indent");
        let second_para = l.lines.iter().position(|ln| ln.char_start == words.chars().count() + 1).unwrap();
        let second_plain = p.lines.iter().position(|ln| ln.char_start == words.chars().count() + 1).unwrap();
        assert!((first_x(&l, second_para) - first_x(&p, second_plain) - want).abs() < 1e-3, "{why}: every paragraph");
    }
}

#[test]
fn resizing_auto_width_auto_height_fixed_and_truncate() {
    if face().is_none() {
        return;
    }
    let s = "one two three four five six seven eight";
    // Auto width: one line per paragraph whatever the box.
    assert_eq!(layout(&text(s, TextAutoResize::WidthAndHeight), Some(50.0), None).lines.len(), 1);
    // Auto height: wraps to the width and grows.
    let l = layout(&text(s, TextAutoResize::Height), Some(80.0), None);
    assert!(l.lines.len() >= 3 && l.lines.iter().all(|ln| ln.width <= 80.0 + EPS));
    // Fixed: wraps the same, the overflow stays (not truncated).
    let fixed = layout(&text(s, TextAutoResize::None), Some(80.0), Some(20.0));
    assert_eq!(fixed.lines.len(), l.lines.len());
    assert!(!fixed.truncated);
    // Truncate: lines that don't fit the box are hidden and the last one ends in an ellipsis.
    let one = layout(&text(s, TextAutoResize::Truncate), Some(80.0), Some(l.lines[0].height + 1.0));
    assert!(one.truncated && one.lines.len() == 1);
    assert!(one.glyphs.last().is_some_and(|g| g.char_start == g.char_end), "ellipsis glyph last");
    // Max lines: the number of lines before it truncates.
    let mut t = text(s, TextAutoResize::Truncate);
    t.max_lines = 2;
    let two = layout(&t, Some(80.0), Some(1000.0));
    assert!(two.truncated && two.lines.len() == 2);
}

#[test]
fn truncation_fills_the_last_line_before_the_ellipsis() {
    // #1375: as with CSS `text-overflow: ellipsis`, the last line shown takes as much of the word
    // wrapped after it as fits before the ellipsis ("Lorem consec…", not "Lorem…").
    if face().is_none() {
        return;
    }
    let s = "Lorem consectetur";
    let full = layout(&text(s, TextAutoResize::WidthAndHeight), None, None);
    let lorem = layout(&text("Lorem", TextAutoResize::WidthAndHeight), None, None).width;
    // Room for "Lorem" and most of the next word, not all of it.
    let w = lorem + (full.width - lorem) * 0.75;
    let line_h = full.lines[0].height;
    let mut max_one = text(s, TextAutoResize::Truncate);
    max_one.max_lines = 1;
    let mut hug_one = text(s, TextAutoResize::Height);
    hug_one.max_lines = 1;
    for (l, why) in [
        (layout(&text(s, TextAutoResize::Truncate), Some(w), Some(line_h + 1.0)), "cut by the box height"),
        (layout(&max_one, Some(w), Some(1000.0)), "cut by max lines"),
        (layout(&hug_one, Some(w), None), "auto height with max lines"),
    ] {
        assert!(l.truncated && l.lines.len() == 1, "{why}");
        let last = l.glyphs.last().unwrap();
        assert_eq!(last.char_start, last.char_end, "{why}: the ellipsis ends the line");
        assert!(l.glyphs.iter().any(|g| g.char_start == 6), "{why}: the wrapped word starts on the line");
        assert!(l.glyphs.iter().all(|g| g.x + g.advance <= w + EPS), "{why}: within the box");
        assert!((l.height - line_h).abs() < EPS, "{why}: one line tall");
        let g7 = l.glyphs.iter().find(|g| g.char_start == 7).unwrap();
        assert_eq!(l.caret(7), (g7.x, 0), "{why}: a caret reaches the chars taken from the next line");
    }
    // A line that ends its paragraph keeps its words; the ellipsis follows them.
    let mut para = text("Lorem\nconsectetur", TextAutoResize::Truncate);
    para.max_lines = 1;
    let l = layout(&para, Some(w), Some(1000.0));
    assert!(l.truncated && l.glyphs.iter().all(|g| g.char_start < 5 || g.char_start == g.char_end));
}

#[test]
fn vertical_trim_cap_height_to_baseline() {
    let Some(f) = face() else { return };
    let k = 20.0 / f.upem;
    let cap = f.cap_height * k;
    for (s, lh, want, why) in [
        ("A", LineHeight::Pixels(40.0), cap, "one line: cap height to baseline = the cap height"),
        ("A\nB\nC", LineHeight::Pixels(40.0), cap + 80.0, "first cap height, then the baselines 40 px apart"),
        (
            "A\nB",
            LineHeight::Auto,
            cap + ((f.ascent + f.descent + f.line_gap.max(0.0)) * k + 0.5).floor(),
            "Auto line height (whole pixels), same rule",
        ),
    ] {
        let mut t = text(s, TextAutoResize::WidthAndHeight);
        t.style.line_height = lh;
        t.leading_trim = LeadingTrim::CapHeight;
        let l = layout(&t, None, None);
        assert!((l.height - want).abs() < 1e-6, "{why}: {} != {want}", l.height);
        assert!((l.lines[0].baseline - cap).abs() < 1e-6, "{why}: first baseline sits a cap height below the top");
        // Trimmed and untrimmed glyphs keep their spacing; only the box edges move.
        t.leading_trim = LeadingTrim::None;
        let u = layout(&t, None, None);
        let shift = u.lines[0].baseline - l.lines[0].baseline;
        for (a, b) in u.glyphs.iter().zip(&l.glyphs) {
            assert!((a.y - b.y - shift).abs() < 1e-9, "{why}");
        }
    }
    // Truncation counts the trimmed bottom: two 40 px lines need cap + 40 px, not 80.
    let mut t = text("A\nB\nC", TextAutoResize::Truncate);
    t.style.line_height = LineHeight::Pixels(40.0);
    t.leading_trim = LeadingTrim::CapHeight;
    let l = layout(&t, Some(200.0), Some(cap + 40.0 + 0.1));
    assert_eq!(l.lines.len(), 2, "two trimmed lines fit");
}

#[test]
fn decorations_use_the_fonts_metrics() {
    let Some(f) = face() else { return };
    let k = 20.0 / f.upem;
    for (deco, pos, why) in [
        (TextDecoration::Underline, f.underline_pos, "underline: the font's underline offset below the baseline"),
        (TextDecoration::Strikethrough, f.strike_pos, "strikethrough: the font's strikeout offset above it"),
    ] {
        let mut t = text("漢字 Hello", TextAutoResize::WidthAndHeight);
        t.style.decoration = deco;
        let l = layout(&t, None, None);
        assert_eq!(l.decorations.len(), 1, "{why}: one line through mixed scripts");
        let d = &l.decorations[0];
        let th = (f.underline_thickness * k).max(0.5);
        assert!((d.rect.height() - th).abs() < 1e-9, "{why}: the font's thickness");
        assert!(((d.rect.y0 + d.rect.y1) / 2.0 - (l.lines[0].baseline - pos * k)).abs() < 1e-9, "{why}");
    }
}

#[test]
fn fallback_fonts_dont_move_the_baseline() {
    // Documentation silent on fallback faces; rule chosen: a style's line metrics come from its own
    // font, so a line with CJK (drawn by a fallback face) is spaced like a Latin one.
    if face().is_none() {
        return;
    }
    for lh in [LineHeight::Auto, LineHeight::Pixels(32.0), LineHeight::Percent(140.0)] {
        let mut t = text("Ab\n漢字かな\nAb 한국어 😀\nAb", TextAutoResize::WidthAndHeight);
        t.style.line_height = lh;
        let l = layout(&t, None, None);
        let first = &l.lines[0];
        for line in &l.lines {
            assert!((line.height - first.height).abs() < 1e-9, "{lh:?}: same height");
            assert!((line.baseline - line.top - (first.baseline - first.top)).abs() < 1e-9, "{lh:?}: same baseline offset");
        }
    }
}

#[test]
fn hangul_comes_from_a_korean_font_when_one_is_installed() {
    // The default (Latin) face has no Hangul. With a Korean system font (Apple SD Gothic Neo on
    // macOS, Malgun Gothic on Windows, Noto Sans CJK KR on Linux) the fallback draws it instead
    // of the missing-glyph box, at the run's weight. Skips on a machine without one.
    let Some(f) = face() else { return };
    if f.has_char('\u{D55C}') {
        return;
    }
    let korean = ["Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans CJK KR", "Noto Sans KR"];
    if !korean.iter().any(|k| find_face(k, "Regular").is_some_and(|k| k.has_char('\u{D55C}'))) {
        return;
    }
    for style in ["Regular", "Bold"] {
        let mut t = text("Ab \u{D55C}\u{AD6D}\u{C5B4}", TextAutoResize::WidthAndHeight);
        t.style.font_style = style.into();
        let l = layout(&t, None, None);
        let hangul: Vec<_> = l.glyphs.iter().filter(|g| g.char_start >= 3).collect();
        assert!(!hangul.is_empty());
        for g in &hangul {
            assert!(g.face.has_char('\u{D55C}'), "{style}: drawn by {} {}, which has no Hangul", g.face.family, g.face.style);
        }
        let used = &hangul[0].face;
        let nearest = find_face(&used.family, style).map_or(used.weight, |n| n.weight);
        assert_eq!(used.weight, nearest, "{style}: the fallback face is the one nearest the run's style");
    }
}

#[test]
fn thai_comes_from_a_thai_font_when_one_is_installed() {
    // The default (Latin) face has no Thai. With a Thai face (Noto Sans Thai from craft-fonts or
    // Linux, Leelawadee UI or Tahoma on Windows, Thonburi on macOS) the fallback draws every Thai
    // cluster, marks included, instead of the missing-glyph box. Skips on a machine without one.
    // "ภาษาไทย" (the Thai language) and "น้ำ" (water: a tone mark over SARA AM).
    let s = "Ab \u{0E20}\u{0E32}\u{0E29}\u{0E32}\u{0E44}\u{0E17}\u{0E22} \u{0E19}\u{0E49}\u{0E33}";
    let chars: Vec<char> = s.chars().collect();
    let is_thai = |c: &char| ('\u{0E00}'..='\u{0E7F}').contains(c);
    let draws_all = |f: &Face| chars.iter().filter(|c| is_thai(c)).all(|c| f.has_char(*c));
    let Some(f) = face() else { return };
    if draws_all(&f) {
        return;
    }
    let thai = ["Noto Sans Thai", "Noto Sans Thai UI", "Leelawadee UI", "Thonburi", "Tahoma"];
    if !thai.iter().any(|t| find_face(t, "Regular").is_some_and(|t| draws_all(&t))) {
        return;
    }
    let l = layout(&text(s, TextAutoResize::WidthAndHeight), None, None);
    let glyphs: Vec<_> = l.glyphs.iter().filter(|g| chars.get(g.char_start).is_some_and(is_thai)).collect();
    assert!(!glyphs.is_empty());
    for g in &glyphs {
        assert!(g.gid != 0, "char {}: missing glyph in {} {}", g.char_start, g.face.family, g.face.style);
        let cluster = chars.get(g.char_start..g.char_end).unwrap_or(&[]);
        assert!(cluster.iter().all(|c| g.face.has_char(*c)), "chars {}..{}: drawn by {}, which lacks some", g.char_start, g.char_end, g.face.family);
    }
    let drawn: std::collections::BTreeSet<usize> = glyphs.iter().flat_map(|g| g.char_start..g.char_end).collect();
    let want: std::collections::BTreeSet<usize> = (3..chars.len()).filter(|i| chars[*i] != ' ').collect();
    assert_eq!(drawn, want, "every Thai char is in a drawn cluster");
}

#[test]
fn thai_emergency_break_can_back_up_to_the_first_letter() {
    // #1219: in "กเม" the overflow lands between the leading vowel and its consonant; the only
    // safe break is right after the first letter, so the vowel moves down with "ม".
    let s = "กเม";
    let thai = ["Noto Sans Thai", "Noto Sans Thai UI", "Leelawadee UI", "Thonburi", "Tahoma"];
    if face().is_none() || !thai.iter().any(|t| find_face(t, "Regular").is_some_and(|t| s.chars().all(|c| t.has_char(c)))) {
        return;
    }
    let mut t = text(s, TextAutoResize::Height);
    t.style.font_size = 16.0;
    let two = layout(&t, None, None).caret(2).0;
    let lines = layout(&t, Some(two + 0.5), None).lines;
    assert_eq!(lines.get(1).map(|l| l.char_start), Some(1), "the leading vowel stays with its consonant");
}

#[test]
fn thai_emergency_breaks_keep_marks_and_leading_vowels_with_their_letter() {
    // #1219: Thai writes no spaces between words, so a paragraph is one run with no break
    // opportunity and wraps where it overflows. Even that emergency break must not start a line
    // with a mark (it belongs to the letter before it) or end one with a leading vowel (it is
    // written before the consonant it is read with). Skips without a Thai face.
    let s = "ระบบช่วยให้ทีมพัฒนาซอฟต์แวร์จัดทำเอกสารที่มีโครงสร้างได้ในที่เดียว";
    let chars: Vec<char> = s.chars().collect();
    let thai = ["Noto Sans Thai", "Noto Sans Thai UI", "Leelawadee UI", "Thonburi", "Tahoma"];
    if face().is_none() {
        return;
    }
    if !thai.iter().any(|t| find_face(t, "Regular").is_some_and(|t| chars.iter().all(|c| t.has_char(*c)))) {
        return;
    }
    let mut t = text(s, TextAutoResize::Height);
    t.style.font_size = 16.0;
    let total = layout(&t, None, None).caret(chars.len()).0;
    let is_mark = |c: char| matches!(c, '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}');
    let is_leading_vowel = |c: char| ('\u{0E40}'..='\u{0E44}').contains(&c);
    let mut breaks = 0;
    for k in 1..40 {
        let width = total * f64::from(k) / 40.0;
        for line in layout(&t, Some(width), None).lines.iter().skip(1) {
            let (Some(start), Some(&first)) = (line.char_start.checked_sub(1), chars.get(line.char_start)) else { continue };
            let Some(&before) = chars.get(start) else { continue };
            assert!(!is_mark(first), "at {width:.1} px a line starts with U+{:04X}: {s}", first as u32);
            assert!(!is_leading_vowel(before), "at {width:.1} px a line ends with {before:?} before {first:?}: {s}");
            breaks += 1;
        }
    }
    assert!(breaks > 0, "the sentence wraps at least once across these widths");
}

/// Wrapped Thai lines start words at every width that fits the widest word (#1219).
#[test]
fn thai_lines_break_between_words() {
    // #1219: Thai writes no spaces, so lines break where a word starts (ICU4X's word data), never
    // inside a word that fits on a line: at every width from the widest word's to the sentence's,
    // each wrapped line starts a word. Font-free: without a Thai face, missing glyphs measure too.
    let words = ["ระบบ", "ช่วย", "ให้", "ทีม", "พัฒนา", "ซอฟต์แวร์", "จัด", "ทำ", "เอกสาร", "ที่", "มี", "โครงสร้าง", "ได้", "ใน", "ที่", "เดียว"];
    if face().is_none() {
        return;
    }
    let starts: Vec<usize> = words
        .iter()
        .scan(0, |at, w| {
            let start = *at;
            *at += w.chars().count();
            Some(start)
        })
        .collect();
    let s = words.concat();
    let width = |s: &str| layout(&text(s, TextAutoResize::WidthAndHeight), None, None).caret(s.chars().count()).0;
    let widest = words.iter().map(|w| width(w)).fold(0.0, f64::max) + 1.0;
    let total = width(&s);
    let t = text(&s, TextAutoResize::Height);
    let mut wraps = 0;
    for k in 0..20 {
        let box_width = widest + (total - widest) * f64::from(k) / 20.0;
        let l = layout(&t, Some(box_width), None);
        for line in l.lines.iter().skip(1) {
            assert!(starts.contains(&line.char_start), "at {box_width:.1} px a line starts inside a word: {:?}", line_texts(&t, &l));
            wraps += 1;
        }
    }
    assert!(wraps >= 20, "the sentence wraps at every width below its own");
}

#[test]
fn symbols_come_from_a_symbol_font_when_one_is_installed() {
    // ⌖ (U+2316) and ⤒ (U+2912) are in no Latin text face. With a symbol font that has them (Apple
    // Symbols on macOS, Segoe UI Symbol on Windows, Noto Sans Symbols 2 and Noto Sans Math on
    // Linux) the fallback draws them instead of the missing-glyph box (#1156). Each character
    // skips on a machine with no installed fallback family that has it.
    let Some(f) = face() else { return };
    let families = ["Apple Symbols", "Segoe UI Symbol", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Symbols2", "Noto Sans Math"];
    for c in ['\u{2316}', '\u{2912}'] {
        if f.has_char(c) || !families.iter().any(|k| find_face(k, "Regular").is_some_and(|k| k.has_char(c))) {
            continue;
        }
        let t = text(&format!("A {c}"), TextAutoResize::WidthAndHeight);
        let l = layout(&t, None, None);
        let drawn: Vec<_> = l.glyphs.iter().filter(|g| g.char_start == 2).collect();
        assert!(!drawn.is_empty(), "U+{:04X} has a glyph", c as u32);
        for g in drawn {
            assert!(g.face.has_char(c), "U+{:04X} drawn by {} {}, which lacks it", c as u32, g.face.family, g.face.style);
        }
    }
}

#[test]
fn break_opportunities() {
    // (before, after, may break between, derivation)
    let cases: [(char, char, bool, &str); 32] = [
        ('a', 'b', false, "inside a Latin word: never"),
        (' ', 'b', true, "after a space"),
        ('a', ' ', false, "before a space: the space hangs at the line's end"),
        ('-', 'b', true, "after a hyphen"),
        ('\u{2014}', 'b', true, "after an em dash"),
        ('日', '本', true, "between ideographs"),
        ('あ', 'い', true, "between kana"),
        ('한', '국', true, "between Hangul syllables (documentation silent on Korean word mode: by syllable, like CSS normal)"),
        ('a', '日', true, "Latin then an ideograph"),
        ('日', 'a', true, "an ideograph then Latin"),
        ('本', '。', false, "kinsoku: an ideographic full stop never starts a line"),
        ('本', '、', false, "kinsoku: nor an ideographic comma"),
        ('本', '」', false, "kinsoku: nor a closing bracket"),
        ('本', 'ー', false, "kinsoku: nor the prolonged sound mark"),
        ('キ', 'ャ', false, "kinsoku: nor a small kana"),
        ('本', '々', false, "kinsoku: nor an iteration mark"),
        ('本', '！', false, "kinsoku: nor full-width punctuation"),
        ('「', '日', false, "kinsoku: an opening bracket never ends a line"),
        ('（', '日', false, "kinsoku: nor a full-width opening parenthesis"),
        ('。', '日', true, "after a full stop"),
        ('日', '「', true, "before an opening bracket"),
        ('\u{3000}', '日', true, "after an ideographic space"),
        ('キ', 'ｬ', false, "kinsoku: nor a halfwidth small kana"),
        ('本', 'ｯ', false, "kinsoku: nor the halfwidth small tsu"),
        ('キ', 'ﾞ', false, "kinsoku: nor the halfwidth voiced sound mark"),
        ('キ', 'ﾟ', false, "kinsoku: nor the halfwidth semi-voiced sound mark"),
        ('本', '゛', false, "kinsoku: nor the spacing voiced sound mark"),
        ('本', '゜', false, "kinsoku: nor the spacing semi-voiced sound mark"),
        ('本', '〱', false, "kinsoku: nor a kana iteration mark"),
        ('本', '〳', false, "kinsoku: nor a vertical kana repeat mark"),
        ('本', '｠', false, "kinsoku: nor a halfwidth closing parenthesis"),
        ('｟', '日', false, "kinsoku: a halfwidth opening parenthesis never ends a line"),
    ];
    for (a, b, want, why) in cases {
        assert_eq!(break_between(a, b), want, "{a}{b}: {why}");
    }
}

#[test]
fn cjk_lines_wrap_between_ideographs_with_kinsoku() {
    if face().is_none() {
        return;
    }
    // Box width cut right after char `fit` (chars 0..fit fit, the next one doesn't): the first line
    // and how the second starts. (text, fit, first line, second line starts with, derivation)
    let cases: [(&str, usize, &str, &str, &str); 7] = [
        ("一二三四五六", 3, "一二三", "四", "ideographs break anywhere"),
        ("一二三。四五", 3, "一二", "三。", "。 can't start a line: 三 moves down with it"),
        ("一二「三」", 3, "一二", "「三", "「 can't end a line: it moves down"),
        ("日本語abc def", 5, "日本語", "abc", "no break inside a Latin word: before it"),
        ("abc 日本語です", 6, "abc 日本", "語", "a space or an ideograph is a break"),
        ("한국어 텍스트입니다", 5, "한국어 텍", "스", "Korean breaks between syllables (documentation silent)"),
        ("一二キｬｯﾄﾞ", 3, "一二", "キｬ", "a halfwidth small kana stays off the line start: キ moves down with it"),
    ];
    for (s, fit, first, second, why) in cases {
        let t = text(s, TextAutoResize::Height);
        let natural = layout(&t, None, None);
        let (fits, next) = (natural.caret(fit).0, natural.caret(fit + 1).0);
        assert!(next > fits, "{why}: chars need advances");
        let l = layout(&t, Some(fits + (next - fits) / 4.0), None);
        let got = line_texts(&t, &l);
        assert_eq!(got[0], first, "{why}");
        assert!(got[1].starts_with(second), "{why}: {got:?}");
    }
}

#[test]
fn a_manual_newline_may_start_a_line_with_a_prohibited_character() {
    if face().is_none() {
        return;
    }
    // The author's own line breaks are kept: 禁則 only applies to automatic wrapping.
    let t = text("き\nっく", TextAutoResize::Height);
    let l = layout(&t, None, None);
    assert_eq!(line_texts(&t, &l), ["き\n", "っく"], "a manual break is not pushed out by kinsoku");
}

#[test]
fn full_width_space_hangs_and_breaks() {
    if face().is_none() {
        return;
    }
    let t = text("日本\u{3000}語", TextAutoResize::Height);
    let natural = layout(&t, None, None);
    let w = natural.caret(2).0 + 0.1;
    let l = layout(&t, Some(w), None);
    assert_eq!(line_texts(&t, &l), ["日本\u{3000}", "語"], "the ideographic space ends the line and hangs");
    assert!(l.lines[0].width <= w);
}

#[test]
fn grapheme_clusters() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"; // 5 chars, one cluster
    let flag = "\u{1F1EF}\u{1F1F5}"; // two regional indicators
    let accent = "e\u{0301}"; // e + combining acute
    let thumbs = "\u{1F44D}\u{1F3FD}"; // emoji + skin tone
    let s = format!("a{family}b{flag}{accent}{thumbs}\u{2764}\u{FE0F}");
    // Every boundary in order: a | family | b | flag | é | thumbs | heart.
    let stops = [0, 1, 6, 7, 9, 11, 13, 15];
    for w in stops.windows(2) {
        assert_eq!(grapheme_step(&s, w[0], true), w[1], "forward from {}", w[0]);
        assert_eq!(grapheme_step(&s, w[1], false), w[0], "back from {}", w[1]);
    }
    assert_eq!(grapheme_step(&s, 15, true), 15, "the end stays");
    assert_eq!(grapheme_step(&s, 0, false), 0, "the start stays");
    assert_eq!(grapheme_step(&s, 3, true), 6, "from inside a cluster: its end");
    assert_eq!(grapheme_step(&s, 3, false), 1, "back from inside: its start");
    assert_eq!(grapheme_step("", 0, true), 0);
    let starts = grapheme_starts(&s);
    assert_eq!(starts.len(), s.chars().count());
    assert_eq!(starts.iter().filter(|b| **b).count(), stops.len() - 1);
    assert!(grapheme_starts("plain ascii").iter().all(|b| *b));
}

#[test]
fn backspace_removes_marks_but_keeps_other_graphemes_atomic() {
    assert_eq!(crate::backspace_step("น้ำ", 3), 2); // Thai spacing mark
    assert_eq!(crate::backspace_step("e\u{0301}", 2), 1); // Latin combining mark
    assert_eq!(crate::backspace_step("👨‍👩‍👧", 5), 0); // Emoji ZWJ sequence
    assert_eq!(crate::backspace_step("🇯🇵", 2), 0); // Flag
    assert_eq!(crate::backspace_step("x", 1), 0); // Ordinary character
    assert_eq!(crate::backspace_step("\u{1112}\u{1161}\u{11AB}", 3), 0); // Hangul syllable as conjoining jamo
    assert_eq!(crate::backspace_step("a\u{200D}", 2), 0); // A letter ending in ZWJ
    assert_eq!(crate::backspace_step("1\u{FE0F}\u{20E3}", 3), 0); // Keycap
}

#[test]
fn a_cluster_is_shaped_with_one_face() {
    if face().is_none() {
        return;
    }
    // Wherever the base char's face is, its marks and joiners go with it (one glyph run).
    for s in ["e\u{0301}", "\u{1F468}\u{200D}\u{1F469}", "漢\u{FE00}"] {
        let l = layout(&text(s, TextAutoResize::WidthAndHeight), None, None);
        let faces: std::collections::HashSet<u32> = l.glyphs.iter().map(|g| g.face.id).collect();
        assert_eq!(faces.len(), 1, "{s:?}");
    }
}
