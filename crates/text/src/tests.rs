use super::*;
use figcraft_doc::{TextAlignH, TextData, TextDecoration};

fn have_fonts() -> bool {
    db().write().unwrap().default_face().is_some() || {
        scan_system_fonts();
        db().write().unwrap().default_face().is_some()
    }
}

#[test]
fn lays_out_single_line() {
    if !have_fonts() {
        eprintln!("no fonts; skipping");
        return;
    }
    let mut t = TextData::new("Hello");
    t.style.font_size = 20.0;
    let l = layout(&t, None, None);
    assert_eq!(l.lines.len(), 1);
    assert!(l.glyphs.len() >= 5);
    assert!(l.width > 30.0 && l.width < 100.0, "{}", l.width);
    assert!(l.height > 20.0 && l.height < 30.0, "{}", l.height);
    // Caret moves right.
    assert!(l.caret(3).0 > l.caret(1).0);
    assert_eq!(l.hit(-5.0, 5.0), 0);
    assert_eq!(l.hit(1000.0, 5.0), 5);
}

#[test]
fn newline_and_wrap() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("one two three four five six\nseven");
    t.auto_resize = figcraft_doc::TextAutoResize::Height;
    let l = layout(&t, Some(60.0), None);
    assert!(l.lines.len() >= 3, "{}", l.lines.len());
    assert!(l.lines.iter().all(|ln| ln.width <= 60.0 + 1e-6 || ln.glyph_end - ln.glyph_start <= 1));
    // Auto width ignores the box.
    t.auto_resize = figcraft_doc::TextAutoResize::WidthAndHeight;
    assert_eq!(layout(&t, Some(60.0), None).lines.len(), 2);
}

#[test]
fn alignment_and_decoration() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("Hi");
    t.auto_resize = figcraft_doc::TextAutoResize::None;
    t.align_h = TextAlignH::Right;
    t.style.decoration = TextDecoration::Underline;
    let l = layout(&t, Some(200.0), None);
    let last = l.glyphs.last().unwrap();
    assert!((last.x + last.advance - 200.0).abs() < 0.5);
    assert_eq!(l.decorations.len(), 1);
}

#[test]
fn empty_and_hostile_inputs() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("");
    let l = layout(&t, None, None);
    assert_eq!(l.lines.len(), 1);
    assert!(l.height > 0.0);
    t.characters = "\n\n".into();
    assert_eq!(layout(&t, None, None).lines.len(), 3);
    t.style.font_size = f64::NAN;
    t.characters = "x".into();
    let _ = layout(&t, Some(f64::NAN), Some(-1.0));
    t.runs.push(figcraft_doc::StyleRun { start: 5, end: 2, style: Default::default() });
    let _ = layout(&t, None, None);
    let mut emoji = TextData::new("a😀b 漢字 ß");
    emoji.style.case = figcraft_doc::TextCase::Upper;
    let l = layout(&emoji, None, None);
    assert!(!l.glyphs.is_empty());
}

#[test]
fn truncate() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("a very long line of text that will not fit at all");
    t.auto_resize = figcraft_doc::TextAutoResize::Truncate;
    let l = layout(&t, Some(80.0), Some(16.0));
    assert!(l.truncated);
    assert_eq!(l.lines.len(), 1);
}

#[test]
fn glyph_paths_have_area() {
    if !have_fonts() {
        return;
    }
    use figcraft_geom::Shape;
    let t = TextData::new("O");
    let l = layout(&t, None, None);
    let p = l.to_path();
    assert!(p.area().abs() > 1.0);
}

fn inter() -> Option<std::sync::Arc<Face>> {
    if !have_fonts() {
        return None;
    }
    db().write().unwrap().find("Inter", "Regular").filter(|f| f.family == "Inter")
}

#[test]
fn opentype_features_change_shaping() {
    let Some(face) = inter() else {
        eprintln!("Inter not embedded; skipping");
        return;
    };
    let mut t = TextData::new("1111 0");
    t.style.font_size = 20.0;
    let base = layout(&t, None, None);
    if face.has_feature(b"tnum") {
        t.features.insert("tnum".into(), true);
        let tab = layout(&t, None, None);
        assert!((tab.width - base.width).abs() > 0.01, "tnum should change digit widths");
        t.features.clear();
    }
    if face.has_feature(b"zero") {
        t.features.insert("zero".into(), true);
        let z = layout(&t, None, None);
        let zero_gid = |l: &TextLayout| l.glyphs.iter().find(|g| g.char_start == 5).map(|g| g.gid);
        assert_ne!(zero_gid(&z), zero_gid(&base), "zero should pick the slashed zero");
    }
    // Invalid tags are ignored, never a crash.
    t.features.insert("bogus-tag".into(), true);
    t.features.insert("".into(), false);
    let _ = layout(&t, None, None);
}

#[test]
fn small_caps_native_or_synthesised() {
    let Some(face) = inter() else { return };
    let mut t = TextData::new("Ab");
    t.style.font_size = 20.0;
    let orig = layout(&t, None, None);
    t.style.case = figcraft_doc::TextCase::SmallCaps;
    let sc = layout(&t, None, None);
    let b = |l: &TextLayout| l.glyphs.iter().find(|g| g.char_start == 1).cloned().unwrap();
    let (ob, sb) = (b(&orig), b(&sc));
    assert_ne!(ob.gid, sb.gid, "the lower-case b becomes a small capital");
    if !face.has_feature(b"smcp") {
        // Synthesised: a capital B drawn smaller than the A.
        assert_eq!(sb.gid, face.glyph_for('B'));
        assert!(sb.size < 20.0);
    }
    // Upper-case letters are untouched.
    assert_eq!(sc.glyphs.iter().find(|g| g.char_start == 0).unwrap().gid, face.glyph_for('A'));
}

#[test]
fn title_case_across_runs() {
    let Some(face) = inter() else { return };
    let mut t = TextData::new("hello world");
    t.style.case = figcraft_doc::TextCase::Title;
    // "hel" bold, "lo world" regular: the word continues across the run boundary.
    t.restyle(0, 3, |s| s.font_style = "Bold".into());
    let l = layout(&t, None, None);
    let gid_at = |i: usize| l.glyphs.iter().find(|g| g.char_start == i).map(|g| g.gid);
    assert_eq!(gid_at(3), Some(face.glyph_for('l')), "mid-word char after a run boundary stays lower-case");
    assert_eq!(gid_at(6), Some(face.glyph_for('W')));
}

#[test]
fn lists_hang_and_number() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("one two three four five six seven\nb");
    t.auto_resize = figcraft_doc::TextAutoResize::Height;
    t.list = figcraft_doc::ListStyle::Bulleted;
    t.paragraph_indent = 30.0;
    let l = layout(&t, Some(90.0), None);
    assert!(l.lines.len() >= 3);
    let first_text = |li: usize| {
        let ln = &l.lines[li];
        l.glyphs[ln.glyph_start..ln.glyph_end].iter().find(|g| g.char_end > g.char_start).map(|g| g.x).unwrap()
    };
    // Wrapped lines hang at the same x as the first line's text (paragraph indent is ignored in lists).
    assert!((first_text(0) - first_text(1)).abs() < 0.01);
    assert!(first_text(0) > 0.0);
    // The bullet sits left of the text, and carets never land on it.
    let ln = &l.lines[0];
    let marker = l.glyphs[ln.glyph_start..ln.glyph_end].iter().find(|g| g.char_end == g.char_start).unwrap();
    assert!(marker.x < first_text(0));
    assert!(l.caret(0).0 >= first_text(0) - 0.01);

    // Numbered: ten paragraphs, the text column clears "10.".
    let mut n = TextData::new((1..=10).map(|i| format!("p{i}")).collect::<Vec<_>>().join("\n"));
    n.list = figcraft_doc::ListStyle::Numbered;
    let l = layout(&n, None, None);
    assert_eq!(l.lines.len(), 10);
    let last = &l.lines[9];
    let gl = &l.glyphs[last.glyph_start..last.glyph_end];
    let marker_end = gl.iter().filter(|g| g.char_end == g.char_start).map(|g| g.x + g.advance).fold(0.0, f64::max);
    let text_x = gl.iter().find(|g| g.char_end > g.char_start).unwrap().x;
    assert!(marker_end < text_x, "{marker_end} < {text_x}");
}

#[test]
fn paragraph_indent_first_line_only() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("aaa bbb ccc ddd eee fff ggg");
    t.auto_resize = figcraft_doc::TextAutoResize::Height;
    t.paragraph_indent = 20.0;
    let l = layout(&t, Some(70.0), None);
    assert!(l.lines.len() >= 2);
    let x0 = |li: usize| l.glyphs[l.lines[li].glyph_start].x;
    assert!((x0(0) - 20.0).abs() < 0.5, "{}", x0(0));
    assert!(x0(1) < 1.0);
}

#[test]
fn truncate_max_lines_with_ellipsis() {
    let Some(face) = inter() else { return };
    let mut t = TextData::new("first line\nsecond line\nthird line");
    t.auto_resize = figcraft_doc::TextAutoResize::Truncate;
    t.max_lines = 2;
    let l = layout(&t, Some(300.0), Some(1000.0));
    assert!(l.truncated);
    assert_eq!(l.lines.len(), 2);
    let last = l.glyphs.last().unwrap();
    assert_eq!(last.gid, face.glyph_for('…'));
    // max_lines is ignored unless truncating.
    t.auto_resize = figcraft_doc::TextAutoResize::None;
    assert_eq!(layout(&t, Some(300.0), Some(1000.0)).lines.len(), 3);
}

#[test]
fn vertical_alignment_in_fixed_box() {
    if !have_fonts() {
        return;
    }
    let mut t = TextData::new("Hi");
    t.auto_resize = figcraft_doc::TextAutoResize::None;
    t.align_v = figcraft_doc::TextAlignV::Bottom;
    let mut n = figcraft_doc::Node::new(figcraft_doc::NodeId(1), figcraft_doc::NodeKind::Text(t.clone()), "t", 100.0, 100.0);
    let (l, dy) = layout_node(&n).unwrap();
    assert!((dy + l.height - 100.0).abs() < 1e-6);
    t.align_v = figcraft_doc::TextAlignV::Middle;
    n.kind = figcraft_doc::NodeKind::Text(t.clone());
    let (l, dy) = layout_node(&n).unwrap();
    assert!((dy - (100.0 - l.height) / 2.0).abs() < 1e-6);
    // Auto height ignores vertical alignment.
    t.auto_resize = figcraft_doc::TextAutoResize::Height;
    n.kind = figcraft_doc::NodeKind::Text(t);
    assert_eq!(layout_node(&n).unwrap().1, 0.0);
}

/// A variable font with a usable weight axis, if one is installed (craft-fonts or the system).
fn variable_face() -> Option<std::sync::Arc<Face>> {
    if !have_fonts() {
        return None;
    }
    scan_system_fonts();
    ["Inter", "Bahnschrift", "Segoe UI Variable", "Roboto Flex", "Noto Sans", "Cantarell", "Source Sans 3", "Open Sans", "Roboto"]
        .into_iter()
        .filter_map(|fam| find_face(fam, "Regular").filter(|f| f.family.eq_ignore_ascii_case(fam)))
        .find(|f| f.axis("wght").is_some_and(|a| a.max - a.min >= 200.0))
}

#[test]
fn variable_font_axes_change_shaping_and_outlines() {
    let Some(face) = variable_face() else {
        eprintln!("no variable font installed; skipping");
        return;
    };
    let wght = face.axis("wght").cloned().unwrap();
    let wi = face.axes.iter().position(|a| a.tag == "wght").unwrap();
    let mut t = TextData::new("Hamburgefonts");
    t.style.font_family = face.family.clone();
    t.style.font_style = face.style.clone();
    t.style.font_size = 40.0;
    t.style.variations.insert("wght".into(), wght.min);
    let light = layout(&t, None, None);
    t.style.variations.insert("wght".into(), wght.max);
    let heavy = layout(&t, None, None);
    assert!(heavy.glyphs.iter().filter(|g| g.face.id == face.id).all(|g| !g.coords.is_empty()), "glyphs carry the instance");
    let first = |l: &TextLayout| l.glyphs.first().map(|g| g.path().elements().to_vec());
    assert_ne!(first(&light), first(&heavy), "the weight axis changes the outlines");
    assert!(heavy.width >= light.width, "heavier is not narrower: {} < {}", heavy.width, light.width);
    // Values clamp to the axis; unknown axes and junk are ignored.
    t.style.variations.insert("wght".into(), 1e9);
    t.style.variations.insert("ZZZZ".into(), 5.0);
    t.style.variations.insert("wdth".into(), f64::NAN);
    let v = face.axis_values(&t.style);
    assert_eq!(v.len(), face.axes.len());
    assert_eq!(v.get(wi).copied(), Some(wght.max));
    assert!(v.iter().all(|x| x.is_finite()));
    let _ = layout(&t, None, None);
    // Without custom values, the style name implies the weight (or picks the named instance).
    t.style.variations.clear();
    t.style.font_style = "Bold".into();
    let v = face.axis_values(&t.style);
    let want = face.instance("Bold").and_then(|i| i.coords.get(wi).copied()).unwrap_or_else(|| wght.clamp(700.0));
    assert_eq!(v.get(wi).copied(), Some(want));
    // Named instances are offered as styles.
    if face.instances.len() > 1 {
        assert!(family_styles(&face.family).len() >= 2);
    }
    // The default instance needs no coordinates.
    assert!(face.coords(&face.axes.iter().map(|a| a.default).collect::<Vec<_>>()).is_empty());
}

#[test]
fn static_fonts_have_no_axes() {
    if !have_fonts() {
        return;
    }
    let Some(face) = db().write().unwrap().default_face() else { return };
    if face.is_variable() {
        return;
    }
    let mut st = figcraft_doc::TextStyle::default();
    st.variations.insert("wght".into(), 900.0);
    assert!(face.axis_values(&st).is_empty());
    assert!(face.coords(&[700.0]).is_empty());
}

#[test]
fn bidi_visual_order_follows_levels() {
    assert_eq!(crate::layout::visual_order(&[0, 0, 1, 1, 1, 0]), vec![0, 1, 4, 3, 2, 5]);
    // A left-to-right run inside a right-to-left paragraph keeps its order.
    assert_eq!(crate::layout::visual_order(&[1, 1, 2, 2, 1]), vec![4, 2, 3, 1, 0]);
    assert_eq!(crate::layout::visual_order(&[0, 0, 0]), vec![0, 1, 2]);
    assert!(crate::layout::visual_order(&[]).is_empty());
    // Plain left-to-right text skips the analysis.
    assert!(crate::layout::Bidi::of("Hello, world 123").is_none());
    let b = crate::layout::Bidi::of("abc \u{5D0}\u{5D1}\u{5D2}\n\u{5D3} x").unwrap();
    assert_eq!(b.levels.get(..4), Some(&[0, 0, 0, 0][..]));
    assert!(b.levels.get(4..7).unwrap().iter().all(|l| l % 2 == 1));
    // Paragraph direction comes from each paragraph's first strong char.
    assert!(!b.rtl_para[0] && b.rtl_para[8]);
}

/// The x of the glyph for char `i`.
fn glyph_x(l: &TextLayout, i: usize) -> f64 {
    l.glyphs.iter().find(|g| g.char_start == i && g.char_end > i).map(|g| g.x).unwrap()
}

#[test]
fn bidi_runs_lay_out_right_to_left() {
    if !have_fonts() {
        eprintln!("no fonts; skipping");
        return;
    }
    // Hebrew inside an English line: its first letter is right of its last.
    let mut t = TextData::new("abc \u{5D0}\u{5D1}\u{5D2}");
    t.style.font_size = 20.0;
    let l = layout(&t, None, None);
    assert!(!l.lines[0].rtl);
    assert!(glyph_x(&l, 2) < glyph_x(&l, 6), "Hebrew after the English");
    assert!(glyph_x(&l, 4) > glyph_x(&l, 6), "א right of ג");
    assert!(l.glyphs.iter().filter(|g| g.char_start >= 4).all(|g| g.rtl));
    // Glyphs are stored left to right and don't overlap.
    let xs: Vec<f64> = l.glyphs.iter().map(|g| g.x).collect();
    assert!(xs.windows(2).all(|w| w[0] <= w[1] + 1e-6), "{xs:?}");

    // A Hebrew paragraph with English inside: English sits on the left, in reading order.
    let t = TextData::new("\u{5D0}\u{5D1}\u{5D2} abc");
    let l = layout(&t, None, None);
    assert!(l.lines[0].rtl);
    assert!(glyph_x(&l, 4) < glyph_x(&l, 5) && glyph_x(&l, 6) < glyph_x(&l, 2));
    assert!(glyph_x(&l, 0) > glyph_x(&l, 2));
    // The caret starts at the right and ends at the left; clicks map back.
    let right = l.glyphs.iter().map(|g| g.x + g.advance).fold(0.0, f64::max);
    assert!((l.caret(0).0 - right).abs() < 1e-6, "{} vs {right}", l.caret(0).0);
    assert!(l.caret(7).0 < glyph_x(&l, 2));
    assert_eq!(l.hit(right + 50.0, 5.0), 0);
    assert_eq!(l.hit(-50.0, 5.0), 4, "left of the English run: before 'a'");
    // Selecting the Hebrew word highlights its glyphs only.
    let r = l.selection_rects(0, 3);
    assert_eq!(r.len(), 1);
    assert!(r[0].x0 >= glyph_x(&l, 2) - 1e-6 && r[0].x1 <= right + 1e-6 && r[0].x0 > glyph_x(&l, 6));
}

#[test]
fn arrows_move_visually_through_right_to_left_runs() {
    if !have_fonts() {
        eprintln!("no fonts; skipping");
        return;
    }
    for text in ["abc \u{5D0}\u{5D1}\u{5D2} def", "\u{5D0}\u{5D1}\u{5D2} abc \u{5D3}\u{5D4}"] {
        let l = layout(&TextData::new(text), None, None);
        let n = text.chars().count();
        // Start at the visual left edge and step right until the caret stops moving right.
        let mut i = (0..=n).min_by(|a, b| l.caret(*a).0.total_cmp(&l.caret(*b).0)).unwrap();
        let mut xs = vec![l.caret(i).0];
        let mut seen = vec![i];
        for _ in 0..(2 * n) {
            let Some(j) = l.visual_step(i, true) else { panic!("bidi line steps visually") };
            if j > n || l.caret(j.min(n)).0 <= *xs.last().unwrap() {
                break;
            }
            i = j;
            xs.push(l.caret(i).0);
            seen.push(i);
        }
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{text}: {xs:?}");
        assert!(seen.len() >= n - 1, "{text}: visited {seen:?}");
        // And back.
        let back = l.visual_step(i, false).unwrap();
        assert!(l.caret(back).0 < l.caret(i).0);
    }
    // Plain left-to-right lines step by char (the caller's default).
    assert!(layout(&TextData::new("plain"), None, None).visual_step(1, true).is_none());
}

#[test]
fn selection_through_text_end_stays_inside_auto_width_box() {
    if !have_fonts() {
        return;
    }
    for size in [12.0, 24.0, 96.0] {
        let mut t = TextData::new("Hello world");
        t.style.font_size = size;
        let l = layout(&t, None, None);
        let n = "Hello world".chars().count();
        let right = l.selection_rects(0, n).last().map_or(f64::NAN, |r| r.x1);
        assert!(right <= l.width.ceil(), "size {size}: right {right} > ceil(width) {}", l.width.ceil());
    }
    // A selected paragraph break still shows past the end of its line.
    let t = TextData::new("Hello\nworld");
    let l = layout(&t, None, None);
    let r = l.selection_rects(0, 6);
    let first = r.first().map_or(f64::NAN, |r| r.x1);
    assert!(first > l.caret(5).0 + 1.0, "{first}");
}

/// The selection rect drawn on line `li` of `l`, if any.
fn rect_on(l: &TextLayout, rects: &[figcraft_geom::Rect], li: usize) -> Option<figcraft_geom::Rect> {
    let top = l.lines.get(li)?.top;
    rects.iter().find(|r| (r.y0 - top).abs() < 1e-6).copied()
}

#[test]
fn select_all_highlights_an_empty_last_line() {
    if !have_fonts() {
        return;
    }
    // "Hello\n": the trailing break leaves an empty second line (#665).
    let l = layout(&TextData::new("Hello\n"), None, None);
    assert_eq!(l.lines.len(), 2);
    let Some(r) = rect_on(&l, &l.selection_rects(0, 6), 1) else { panic!("no highlight on the empty last line") };
    // A paragraph-break mark at the line's start, not at the end of "Hello" above.
    assert!(r.x0.abs() < 1e-6 && (r.width() - 4.0).abs() < 1e-6, "{r:?}");
    assert!(rect_on(&l, &l.selection_rects(5, 6), 1).is_some());
    // Selecting part of the first line, or a caret, leaves the empty line alone.
    assert!(l.selection_rects(0, 1).iter().all(|r| r.y0 < l.lines[1].top - 1e-6), "{:?}", l.selection_rects(0, 1));
    assert!(l.selection_rects(6, 6).is_empty());
    // A right-to-left paragraph marks its empty last line too.
    let rtl = layout(&TextData::new("\u{5E9}\u{5DC}\u{5D5}\u{5DD}\n"), None, None);
    let Some(r) = rect_on(&rtl, &rtl.selection_rects(0, 5), 1) else { panic!("no highlight on the empty right-to-left line") };
    assert!(r.x0.abs() < 1e-6 && (r.width() - 4.0).abs() < 1e-6, "{r:?}");
}

#[test]
fn selection_after_a_line_break_starts_at_the_line_start() {
    if !have_fonts() {
        return;
    }
    // Not at the end of the line above, where `caret` puts the index after the break.
    let l = layout(&TextData::new("Hello\nworld"), None, None);
    let Some(r) = rect_on(&l, &l.selection_rects(0, 11), 1) else { panic!("no highlight on the second line") };
    assert!(r.x0.abs() < 1e-6 && r.x1 > l.caret(10).0, "{r:?}");
    // An empty line in the middle shows its break mark at its start.
    let m = layout(&TextData::new("Hello\n\nworld"), None, None);
    let Some(r) = rect_on(&m, &m.selection_rects(0, 12), 1) else { panic!("no highlight on the empty line") };
    assert!(r.x0.abs() < 1e-6 && (r.width() - 4.0).abs() < 1e-6, "{r:?}");
    // A break followed by text still doesn't highlight the next line's start.
    assert!(rect_on(&l, &l.selection_rects(0, 6), 1).is_none());
}
