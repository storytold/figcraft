//! Style dedupe in `shape_text` (#1102): equal spans share a style entry whatever their order
//! (blocks, alternation, pathological alternation), fields the fingerprint skips still split by
//! equality, and −0.0 merges like `PartialEq` does.

use figcraft_doc::{StyleRun, TextData, TextStyle};

use crate::layout::layout;

fn solid(hex: &str) -> Option<Vec<figcraft_doc::color::Paint>> {
    Some(vec![figcraft_doc::color::Paint::solid(figcraft_doc::color::Color::from_hex(hex).unwrap())])
}

fn style_with(fills: Option<Vec<figcraft_doc::color::Paint>>) -> TextStyle {
    TextStyle { fills, ..Default::default() }
}

fn text_with_runs(runs: Vec<(usize, usize, TextStyle)>) -> TextData {
    let mut t = TextData::new("x".repeat(runs.iter().map(|r| r.1).max().unwrap_or(0)));
    t.runs = runs.into_iter().map(|(s, e, st)| StyleRun { start: s, end: e, style: st }).collect();
    t
}

#[test]
fn equal_runs_share_one_style_entry() {
    // A run styled exactly like the base style adds no second entry, in either order.
    let mut t = TextData::new("Hello World");
    t.runs.push(StyleRun { start: 0, end: 5, style: TextStyle::default() });
    assert_eq!(layout(&t, None, None).styles.len(), 1);
    t.runs.clear();
    t.runs.push(StyleRun { start: 6, end: 11, style: TextStyle::default() });
    assert_eq!(layout(&t, None, None).styles.len(), 1);
}

#[test]
fn runs_of_one_fingerprint_dedupe_to_one_entry() {
    // Five alternating runs of the same style: the last-style cache and the buckets merge them.
    let s = style_with(solid("#00AA44"));
    let t = text_with_runs(vec![(0, 1, s.clone()), (1, 2, style_with(None)), (2, 3, s.clone()), (3, 4, style_with(None)), (4, 5, s)]);
    let l = layout(&t, None, None);
    assert_eq!(l.styles.len(), 2);
}

#[test]
fn fields_the_fingerprint_skips_still_split_by_equality() {
    // The fingerprint skips `link` and `style_id`; equality after it must still split them.
    let mut a = style_with(None);
    a.link = Some("https://a".into());
    let mut b = style_with(None);
    b.link = Some("https://b".into());
    let t = text_with_runs(vec![(0, 1, a), (1, 2, b)]);
    assert_eq!(layout(&t, None, None).styles.len(), 2);

    let mut c = style_with(None);
    c.style_id = Some("style-1".into());
    let mut d = style_with(None);
    d.style_id = Some("style-2".into());
    let t = text_with_runs(vec![(0, 1, c), (1, 2, d)]);
    assert_eq!(layout(&t, None, None).styles.len(), 2);

    // Two gradients hash to the same tag; their stops still split them.
    let grad = |base: &str| {
        Some(vec![figcraft_doc::color::Paint::gradient(figcraft_doc::color::Gradient::default_for(
            figcraft_doc::color::GradientKind::Linear,
            figcraft_doc::color::Color::from_hex(base).unwrap(),
        ))])
    };
    let t = text_with_runs(vec![(0, 1, style_with(grad("#FF0000"))), (1, 2, style_with(grad("#00FF00")))]);
    assert_eq!(layout(&t, None, None).styles.len(), 2);
}

#[test]
fn negative_zero_merges_like_equality() {
    // PartialEq merges −0.0 and 0.0; the fingerprint folds the sign of zero the same way.
    let mut a = style_with(None);
    a.font_size = 0.0;
    let mut b = style_with(None);
    b.font_size = -0.0;
    let t = text_with_runs(vec![(0, 1, a), (1, 2, b)]);
    assert_eq!(layout(&t, None, None).styles.len(), 1);
}

#[test]
fn pathological_alternation_dedupes_like_the_scan() {
    // 50 styles alternating twice: 100 runs dedupe to 50 entries, first-seen order kept.
    let styles: Vec<TextStyle> = (0..50)
        .map(|i| {
            let mut s = style_with(None);
            s.font_size = 12.0 + i as f64;
            s
        })
        .collect();
    let runs: Vec<(usize, usize, TextStyle)> = (0..100).map(|i| (i, i + 1, styles[i % 50].clone())).collect();
    let l = layout(&text_with_runs(runs), None, None);
    assert_eq!(l.styles.len(), 50, "alternating runs dedupe to the distinct styles");
    for (k, st) in l.styles.iter().enumerate() {
        assert_eq!(st.font_size, 12.0 + k as f64, "first-seen order kept at {k}");
    }
    // Every glyph points at the style of its run.
    for (i, g) in l.glyphs.iter().enumerate() {
        assert_eq!(l.styles[g.style].font_size, 12.0 + (i % 50) as f64, "glyph {i} points at its run's style");
    }
}
