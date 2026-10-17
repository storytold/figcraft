//! Text behaviour suite, the tail: lists (numbering and nesting, list spacing, hanging lists),
//! hanging punctuation and decoration styles, written from the public documentation of the target
//! app's text settings. Each case gives the rule and a one-line derivation; where the
//! documentation is silent the case says so and pins today's behaviour.
//!
//! Numbering and decoration shapes are font-free; layout geometry reads the default face and skips
//! on a machine without fonts.

use figcraft_doc::{
    DecorationLength, DecorationStyle, ListStyle, ParaList, TextAlignH, TextAutoResize, TextData, TextDecoration, color::Color, color::Paint,
};
use figcraft_geom::{Rect, Shape};

use crate::layout::list_markers;
use crate::{Decoration, TextLayout, db, layout, scan_system_fonts};

const EPS: f64 = 1e-6;

/// A 20 px text in the default face, or None (skip) without fonts.
fn text(s: &str) -> Option<TextData> {
    scan_system_fonts();
    let f = db().write().ok()?.default_face()?;
    let mut t = TextData::new(s);
    (t.style.font_family, t.style.font_style) = (f.family.clone(), f.style.clone());
    t.style.font_size = 20.0;
    Some(t)
}

fn laid(t: &TextData) -> Option<TextLayout> {
    Some(layout(t, Some(400.0), None)).filter(|l| !l.glyphs.is_empty())
}

/// The x of each line's first text glyph and of its marker (glyphs covering no char).
fn starts(l: &TextLayout) -> Vec<(f64, Option<f64>)> {
    l.lines
        .iter()
        .map(|ln| {
            let gl = l.glyphs.get(ln.glyph_start..ln.glyph_end).unwrap_or(&[]);
            let text = gl.iter().find(|g| g.char_end > g.char_start).map_or(f64::NAN, |g| g.x);
            (text, gl.iter().find(|g| g.char_end == g.char_start).map(|g| g.x))
        })
        .collect()
}

fn pl(list: ListStyle, indent: u8) -> ParaList {
    ParaList { list, indent, start_offset: 0 }
}

fn from(list: ListStyle, indent: u8, start_offset: u32) -> ParaList {
    ParaList { list, indent, start_offset }
}

#[test]
fn numbered_lists_count_from_their_start() {
    use ListStyle::{Bulleted as B, None as P, Numbered as N};
    let cases: Vec<(Vec<ParaList>, Vec<&str>, &str)> = vec![
        (vec![from(N, 0, 1)], vec!["2."], "a one-item list started at 2 (#686)"),
        (vec![from(N, 0, 3), pl(N, 0), pl(N, 0)], vec!["4.", "5.", "6."], "the items after the first count on from it"),
        (vec![from(N, 0, 3), from(N, 0, 3), from(N, 0, 3)], vec!["4.", "5.", "6."], "items that go on counting ignore their own start"),
        (vec![from(N, 0, 1), pl(P, 0), from(N, 0, 1)], vec!["2.", "", "2."], "a list after a plain paragraph starts from its own start"),
        (vec![pl(N, 0), from(N, 1, 1), pl(N, 1)], vec!["1.", "b.", "c."], "a nested level starts from its first item's start"),
        (vec![from(B, 0, 4), pl(B, 0)], vec!["\u{2022}", "\u{2022}"], "bullets have no number"),
        (vec![from(N, 0, 4_000), pl(N, 2)], vec!["4001.", "i."], "big starts"),
    ];
    for (lists, want, why) in cases {
        assert_eq!(list_markers(&lists), want, "{why}");
    }
    // The largest start never overflows, on 32-bit targets too.
    let max = list_markers(&[from(N, 0, u32::MAX), pl(N, 0), pl(N, 0)]);
    assert_eq!(max.len(), 3);
    assert!(max.iter().all(|m| m.starts_with("429496729")), "{max:?}");
}

#[test]
fn list_numbering_by_level() {
    use ListStyle::{Bulleted as B, None as P, Numbered as N};
    let many: Vec<ParaList> = std::iter::once(pl(N, 0)).chain(std::iter::repeat_n(pl(N, 1), 27)).collect();
    // (paragraphs, markers, derivation)
    let cases: Vec<(Vec<ParaList>, Vec<&str>, &str)> = vec![
        (vec![pl(N, 0), pl(N, 0), pl(N, 0)], vec!["1.", "2.", "3."], "a numbered list counts its items"),
        (
            vec![pl(N, 0), pl(N, 1), pl(N, 1), pl(N, 0)],
            vec!["1.", "a.", "b.", "2."],
            "nested items count on their own level; the outer count goes on",
        ),
        (
            vec![pl(N, 0), pl(N, 1), pl(N, 2), pl(N, 3)],
            vec!["1.", "a.", "i.", "1."],
            "each level numbers its own way (silent on which: numbers, letters, roman numerals, round again)",
        ),
        (
            vec![pl(B, 0), pl(B, 1), pl(B, 2), pl(B, 3)],
            vec!["\u{2022}", "\u{25E6}", "\u{25AA}", "\u{2022}"],
            "silent: bullets change shape per level, round again",
        ),
        (vec![pl(N, 0), pl(N, 0), pl(P, 0), pl(N, 0)], vec!["1.", "2.", "", "1."], "silent: a plain paragraph ends a list, the next starts at 1"),
        (vec![pl(N, 0), pl(B, 0), pl(N, 0)], vec!["1.", "\u{2022}", "1."], "silent: the other kind of list at the same level starts the count again"),
        (vec![pl(N, 0), pl(N, 1), pl(N, 0), pl(N, 1)], vec!["1.", "a.", "2.", "a."], "a shallower item starts the deeper count again"),
        (vec![pl(N, 2), pl(N, 2), pl(N, 2), pl(N, 2)], vec!["i.", "ii.", "iii.", "iv."], "roman numerals at the third level"),
    ];
    for (lists, want, why) in cases {
        assert_eq!(list_markers(&lists), want, "{why}");
    }
    assert_eq!(list_markers(&many).last().map(String::as_str), Some("aa."), "silent: letters go on past z as aa");
}

#[test]
fn nested_items_indent_by_one_gutter() {
    // Tab nests an item: each level moves the item and its marker in by one marker gutter (bullets:
    // one and a half ems, 30 px at 20 px); a hanging list puts the outer gutter outside the box so
    // item text lines up with plain paragraphs.
    let Some(mut t) = text("One\nTwo\nThree") else { return };
    t.paragraphs = vec![pl(ListStyle::Bulleted, 0), pl(ListStyle::Bulleted, 1), pl(ListStyle::Bulleted, 2)];
    let Some(l) = laid(&t) else { return };
    // (hanging, text x per level, marker x per level, derivation)
    for (hanging, text_x, marker_x, why) in [
        (false, [30.0, 60.0, 90.0], [0.0, 30.0, 60.0], "inside the box: level n's text at (n + 1) gutters"),
        (true, [0.0, 30.0, 60.0], [-30.0, 0.0, 30.0], "hanging list: one gutter less, the outer bullet left of the box"),
    ] {
        t.hanging_list = hanging;
        let l = if hanging { laid(&t).unwrap() } else { l.clone() };
        for (i, (x, m)) in starts(&l).into_iter().enumerate() {
            assert!((x - text_x[i]).abs() < EPS, "{why}: line {i} text at {x}");
            assert!(m.is_some_and(|m| (m - marker_x[i]).abs() < EPS), "{why}: line {i} marker at {m:?}");
        }
        assert_eq!(l.ink_bounds().is_some_and(|b| b.x0 <= -30.0), hanging, "{why}: exports reach the hanging bullet");
    }
    // A plain paragraph next to a hanging list starts where the items' text does.
    t.paragraphs.push(pl(ListStyle::None, 0));
    t.characters.push_str("\nPlain");
    let l = laid(&t).unwrap();
    assert!(starts(&l)[3].0.abs() < EPS && starts(&l)[0].0.abs() < EPS, "hanging list: items line up with plain text");
}

#[test]
fn list_spacing_between_items_only() {
    // List spacing is the space between items of a list; paragraphs around it keep the paragraph
    // spacing (silent on both set at once: between two items the list spacing replaces it).
    let Some(mut t) = text("Intro\nA\nB\nOutro") else { return };
    t.style.line_height = figcraft_doc::LineHeight::Pixels(30.0);
    t.paragraphs = vec![pl(ListStyle::None, 0), pl(ListStyle::Bulleted, 0), pl(ListStyle::Bulleted, 0), pl(ListStyle::None, 0)];
    (t.paragraph_spacing, t.list_spacing) = (10.0, 4.0);
    let Some(l) = laid(&t) else { return };
    let tops: Vec<f64> = l.lines.iter().map(|ln| ln.top).collect();
    assert_eq!(tops, [0.0, 40.0, 74.0, 114.0], "plain→item +10, item→item +4, item→plain +10");
    assert!((l.height - 144.0).abs() < EPS);
}

#[test]
fn hanging_punctuation_outside_the_aligned_edge() {
    // Hanging punctuation: opening quotes and brackets at a line's start sit left of a left-aligned
    // (or justified) box; closing punctuation at its end right of a right-aligned (or justified)
    // one, so the letters line up. Silent on centred text and right-to-left paragraphs: no hang.
    let Some(mut t) = text("\u{201C}Quote\u{201D} here.") else { return };
    t.auto_resize = TextAutoResize::Height;
    let Some(plain) = laid(&t) else { return };
    let open = plain.glyphs[0].advance;
    // Kerning may draw a glyph off its pen position (an x offset, e.g. Helvetica Neue's "re"), so
    // each hanging line is checked against the same line set flush: every glyph moves by exactly
    // the hanging mark's advance, which puts the letters' pen edge on the box edge.
    let moved = |a: &TextLayout, b: &TextLayout, by: f64| {
        a.glyphs.len() == b.glyphs.len() && a.glyphs.iter().zip(&b.glyphs).all(|(a, b)| (a.x - b.x - by).abs() < EPS)
    };
    t.hanging_punctuation = true;
    let l = laid(&t).unwrap();
    assert!(plain.glyphs[0].x.abs() < EPS && moved(&l, &plain, -open), "left: the quote hangs, Q starts the line");
    t.align_h = TextAlignH::Right;
    t.hanging_punctuation = false;
    let flush = laid(&t).unwrap();
    t.hanging_punctuation = true;
    let l = laid(&t).unwrap();
    let stop = l.glyphs.last().unwrap().advance;
    let edge = flush.lines[0].x + flush.lines[0].width;
    assert!((edge - 400.0).abs() < EPS && moved(&l, &flush, stop), "right: the full stop hangs past the edge");
    t.align_h = TextAlignH::Center;
    let (a, b) = (laid(&t).unwrap(), {
        t.hanging_punctuation = false;
        laid(&t).unwrap()
    });
    assert!((a.glyphs[0].x - b.glyphs[0].x).abs() < EPS, "centred: nothing hangs");
    // Hanging punctuation takes no room in the box: a line that fits only without its opening
    // quote stays on one line.
    let mut t = text("\u{201C}ab cd").unwrap();
    t.auto_resize = TextAutoResize::Height;
    let w = layout(&t, None, None).width;
    let room = w - plain.glyphs[0].advance + 0.01;
    assert_eq!(layout(&t, Some(room), None).lines.len(), 2, "without hanging it wraps");
    t.hanging_punctuation = true;
    assert_eq!(layout(&t, Some(room), None).lines.len(), 1, "with hanging it fits");
}

/// (decoration, look change, top edge, height, derivation)
type Case<'a> = (TextDecoration, &'a dyn Fn(&mut figcraft_doc::DecorationLook), Option<f64>, f64, &'a str);

#[test]
fn decoration_style_thickness_offset_and_colour() {
    let Some(mut t) = text("Hello") else { return };
    t.style.decoration = TextDecoration::Underline;
    let Some(base) = laid(&t) else { return };
    let baseline = base.lines[0].baseline;
    let red = Color::new(1.0, 0.0, 0.0, 1.0);
    let set = |t: &mut TextData, d: TextDecoration, f: &dyn Fn(&mut figcraft_doc::DecorationLook)| {
        t.style.decoration = d;
        t.style.decoration_look = None;
        t.style.set_look(f);
    };
    let auto = base.decorations[0].rect;
    let cases: [Case; 5] = [
        (TextDecoration::Underline, &|l| l.thickness = DecorationLength::Pixels(3.0), None, 3.0, "thickness in px"),
        (TextDecoration::Underline, &|l| l.thickness = DecorationLength::Percent(10.0), None, 2.0, "thickness in % of the font size: 10% of 20"),
        (
            TextDecoration::Underline,
            &|l| l.offset = DecorationLength::Pixels(4.0),
            Some(baseline + 4.0),
            auto.height(),
            "offset: baseline to the line's top",
        ),
        (
            TextDecoration::Underline,
            &|l| l.offset = DecorationLength::Percent(-10.0),
            Some(baseline - 2.0),
            auto.height(),
            "a negative offset lifts it above the baseline",
        ),
        (
            TextDecoration::Strikethrough,
            &|l| l.offset = DecorationLength::Pixels(4.0),
            None,
            auto.height(),
            "silent: strikethrough ignores the offset",
        ),
    ];
    for (d, f, top, h, why) in cases {
        set(&mut t, d, f);
        let r = laid(&t).unwrap().decorations[0].rect;
        assert!((r.height() - h).abs() < EPS, "{why}: height {}", r.height());
        if let Some(y) = top {
            assert!((r.y0 - y).abs() < EPS, "{why}: top {}", r.y0);
        }
    }
    let mut strike = t.clone();
    strike.style.decoration_look = None;
    let plain_strike = laid(&strike).unwrap().decorations[0].rect;
    assert!((laid(&t).unwrap().decorations[0].rect.y0 - plain_strike.y0).abs() < EPS, "strikethrough stays at the font's position");
    // Wavy swings either side; dotted is a row of dots; both inside their rect.
    for style in [DecorationStyle::Wavy, DecorationStyle::Dotted] {
        set(&mut t, TextDecoration::Underline, &|l| l.style = style);
        let d = laid(&t).unwrap().decorations[0].clone();
        assert_eq!(d.kind, style);
        let b = d.path().bounding_box();
        assert!(b.x0 >= d.rect.x0 - EPS && b.x1 <= d.rect.x1 + EPS && b.y0 >= d.rect.y0 - EPS && b.y1 <= d.rect.y1 + EPS, "{style:?} inside {b:?}");
        assert!(style != DecorationStyle::Wavy || d.rect.height() > d.thickness, "a wave is taller than its thickness");
    }
    // Colour: the line paints with its own colour, the glyphs keep theirs.
    set(&mut t, TextDecoration::Underline, &|l| l.color = Some(red));
    let l = laid(&t).unwrap();
    let d = &l.decorations[0];
    assert!(l.glyphs.iter().all(|g| g.style != d.style));
    assert_eq!(l.styles[d.style].fills, Some(vec![Paint::solid(red)]));
    assert_eq!(l.styles[l.glyphs[0].style].fills, None);
}

#[test]
fn skip_ink_breaks_under_descenders() {
    // Skip ink: an underline stops short of descenders that cross it (silent on the gap: one line
    // thickness, at least 1 px, each side).
    let Some(mut t) = text("gypsy") else { return };
    t.style.decoration = TextDecoration::Underline;
    let Some(l) = laid(&t) else { return };
    assert_eq!(l.decorations.len(), 1, "off: one line");
    t.style.set_look(|l| l.skip_ink = true);
    let l = laid(&t).unwrap();
    assert!(l.decorations.len() > 1, "on: broken under g, y and p");
    let t2 = TextData { characters: "mmm".into(), ..t.clone() };
    let l2 = laid(&t2).unwrap();
    // Only when the line and its gap clear the baseline: some fonts put the underline so close
    // that it touches every glyph's foot, and then it breaks under those too.
    let clear = l2.decorations.first().zip(l2.lines.first()).is_some_and(|(d, ln)| d.rect.y0 - d.thickness.max(1.0) / 2.0 > ln.baseline);
    if clear {
        assert_eq!(l2.decorations.len(), 1, "no descenders: one line");
    }
}

#[test]
fn decoration_shapes() {
    // Font-free: the path of each style for a 100 px line, 2 px thick.
    let d = |kind| Decoration { rect: Rect::new(0.0, 10.0, 100.0, 12.0), style: 0, kind, thickness: 2.0 };
    assert_eq!(d(DecorationStyle::Solid).path().bounding_box(), Rect::new(0.0, 10.0, 100.0, 12.0), "solid: the bar");
    let dots = d(DecorationStyle::Dotted).path();
    let count = dots.elements().iter().filter(|e| matches!(e, figcraft_geom::kurbo::PathEl::MoveTo(_))).count();
    assert_eq!(count, 25, "dotted: a 2 px dot every 4 px");
    let wave = Decoration { rect: Rect::new(0.0, 7.0, 100.0, 15.0), ..d(DecorationStyle::Wavy) };
    let b = wave.path().bounding_box();
    assert!((b.x0, b.x1) == (0.0, 100.0) && b.y0 >= 7.0 - EPS && b.y1 <= 15.0 + EPS, "wavy: across, within the band {b:?}");
}
