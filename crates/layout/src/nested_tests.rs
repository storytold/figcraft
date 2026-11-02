//! Nested hug/fill edge cases: every expectation holds after a single layout pass.

use super::tests::{FakeMeasure, pos, text};
use super::*;
use figcraft_doc::{AutoLayout, Document, NodeKind};

/// An auto layout frame under `parent` (gap 10, no padding) with the given sizing.
fn al(d: &mut Document, parent: NodeId, mode: LayoutMode, (w, h): (f64, f64), (sh, sv): (Sizing, Sizing)) -> NodeId {
    let f = d.create(parent, NodeKind::Frame, "F", 0.0, 0.0, w, h).unwrap();
    let n = d.get_mut(f).unwrap();
    n.layout = Some(AutoLayout { mode, gap: 10.0, padding: [0.0; 4], ..Default::default() });
    n.child_layout.sizing_h = sh;
    n.child_layout.sizing_v = sv;
    f
}

fn rect(d: &mut Document, parent: NodeId, w: f64, h: f64) -> NodeId {
    d.create(parent, NodeKind::Rectangle, "R", 0.0, 0.0, w, h).unwrap()
}

fn sizing(d: &mut Document, id: NodeId, sh: Sizing, sv: Sizing) {
    let cl = &mut d.get_mut(id).unwrap().child_layout;
    cl.sizing_h = sh;
    cl.sizing_v = sv;
}

fn size(d: &Document, id: NodeId) -> (f64, f64) {
    let n = d.node(id).unwrap();
    (n.width, n.height)
}

fn layout_mut(d: &mut Document, id: NodeId) -> &mut AutoLayout {
    d.get_mut(id).unwrap().layout.as_mut().unwrap()
}

fn page(d: &Document) -> NodeId {
    d.first_page().unwrap()
}

/// A text that wraps (auto height) when it fills a width.
fn fill_text(d: &mut Document, parent: NodeId) -> NodeId {
    let t = text(d, parent, "Wrapping text");
    sizing(d, t, Sizing::Fill, Sizing::Hug);
    if let NodeKind::Text(td) = &mut d.get_mut(t).unwrap().kind {
        td.auto_resize = TextAutoResize::Height;
    }
    t
}

#[test]
fn hug_parent_treats_fill_children_as_fixed() {
    let mut d = Document::new("t");
    let p = page(&d);
    let f = al(&mut d, p, LayoutMode::Horizontal, (10.0, 10.0), (Sizing::Hug, Sizing::Hug));
    rect(&mut d, f, 50.0, 20.0);
    let b = rect(&mut d, f, 30.0, 40.0);
    sizing(&mut d, b, Sizing::Fill, Sizing::Fill);
    layout_frame(&mut d, f, &NoMeasure);
    // Nothing to fill: the fill child keeps its size and the frame gets the content size.
    assert_eq!(size(&d, f), (90.0, 40.0));
    assert_eq!(pos(&d, b), (60.0, 0.0, 30.0, 40.0));
    // Vertical, fill on the cross axis.
    let g = al(&mut d, p, LayoutMode::Vertical, (10.0, 10.0), (Sizing::Hug, Sizing::Hug));
    let c = rect(&mut d, g, 60.0, 20.0);
    sizing(&mut d, c, Sizing::Fill, Sizing::Fixed);
    rect(&mut d, g, 40.0, 30.0);
    layout_frame(&mut d, g, &NoMeasure);
    assert_eq!(size(&d, g), (60.0, 60.0));
    assert_eq!(size(&d, c), (60.0, 20.0));
}

#[test]
fn fill_children_redistribute_when_they_hit_min_or_max() {
    let mut d = Document::new("t");
    let p = page(&d);
    // Inner width 300 after two gaps.
    let f = al(&mut d, p, LayoutMode::Horizontal, (320.0, 50.0), (Sizing::Fixed, Sizing::Fixed));
    let k: Vec<NodeId> = (0..3).map(|_| rect(&mut d, f, 10.0, 10.0)).collect();
    for c in &k {
        sizing(&mut d, *c, Sizing::Fill, Sizing::Fixed);
    }
    let widths = |d: &mut Document| {
        layout_frame(d, f, &NoMeasure);
        k.iter().map(|c| size(d, *c).0).collect::<Vec<_>>()
    };
    assert_eq!(widths(&mut d), vec![100.0, 100.0, 100.0]);
    // One hits its max: the others share what it leaves, and a second one hitting its max then
    // hands the rest to the last.
    d.get_mut(k[0]).unwrap().child_layout.max_w = Some(50.0);
    assert_eq!(widths(&mut d), vec![50.0, 125.0, 125.0]);
    d.get_mut(k[1]).unwrap().child_layout.max_w = Some(100.0);
    assert_eq!(widths(&mut d), vec![50.0, 100.0, 150.0]);
    // A max and a min at once: the max wins first (the space it leaves lifts the other past its
    // min), so the min child isn't frozen at its minimum.
    d.get_mut(k[1]).unwrap().child_layout.max_w = None;
    d.get_mut(k[1]).unwrap().child_layout.min_w = Some(120.0);
    assert_eq!(widths(&mut d), vec![50.0, 125.0, 125.0]);
    // Mins that don't fit: they hold, the remaining fill child gets nothing and the row overflows.
    d.get_mut(k[0]).unwrap().child_layout.max_w = None;
    d.get_mut(k[0]).unwrap().child_layout.min_w = Some(200.0);
    d.get_mut(k[1]).unwrap().child_layout.min_w = Some(200.0);
    assert_eq!(widths(&mut d), vec![200.0, 200.0, 0.0]);
    assert_eq!(pos(&d, k[2]).0, 420.0);
}

#[test]
fn fill_text_wraps_and_the_parent_hugs_its_new_height() {
    let mut d = Document::new("t");
    let p = page(&d);
    // Vertical: the text fills the width (cross axis).
    let f = al(&mut d, p, LayoutMode::Vertical, (200.0, 10.0), (Sizing::Fixed, Sizing::Hug));
    layout_mut(&mut d, f).padding = [10.0; 4];
    let t = fill_text(&mut d, f);
    // Horizontal: the text fills the width (main axis) next to a fixed rectangle.
    let g = al(&mut d, p, LayoutMode::Horizontal, (200.0, 10.0), (Sizing::Fixed, Sizing::Hug));
    rect(&mut d, g, 50.0, 30.0);
    let t2 = fill_text(&mut d, g);
    let m = FakeMeasure([(t, (300.0, 20.0, 15.0)), (t2, (450.0, 20.0, 15.0))].into_iter().collect());
    layout_frame(&mut d, f, &m);
    // 180 wide → two lines.
    assert_eq!(size(&d, t), (180.0, 40.0));
    assert_eq!(size(&d, f), (200.0, 60.0));
    layout_frame(&mut d, g, &m);
    // 200 - 50 - 10 = 140 wide → four lines.
    assert_eq!(size(&d, t2), (140.0, 80.0));
    assert_eq!(size(&d, g), (200.0, 80.0));
    // Nested: a card that fills a fixed column hugs the text it wraps.
    let col = al(&mut d, p, LayoutMode::Vertical, (300.0, 10.0), (Sizing::Fixed, Sizing::Hug));
    let card = al(&mut d, col, LayoutMode::Vertical, (1.0, 1.0), (Sizing::Fill, Sizing::Hug));
    layout_mut(&mut d, card).padding = [10.0; 4];
    let t3 = fill_text(&mut d, card);
    let m = FakeMeasure([(t3, (560.0, 20.0, 15.0))].into_iter().collect());
    layout_page(&mut d, p, &m);
    assert_eq!(size(&d, t3), (280.0, 40.0));
    assert_eq!(size(&d, card), (300.0, 60.0));
    assert_eq!(size(&d, col), (300.0, 60.0));
}

#[test]
fn three_levels_of_mixed_sizing_settle_in_one_pass() {
    let mut d = Document::new("t");
    let p = page(&d);
    let outer = al(&mut d, p, LayoutMode::Vertical, (400.0, 1.0), (Sizing::Fixed, Sizing::Hug));
    layout_mut(&mut d, outer).padding = [10.0; 4];
    let header = rect(&mut d, outer, 1.0, 20.0);
    sizing(&mut d, header, Sizing::Fill, Sizing::Fixed);
    let middle = al(&mut d, outer, LayoutMode::Horizontal, (1.0, 1.0), (Sizing::Fill, Sizing::Hug));
    let icon = rect(&mut d, middle, 50.0, 20.0);
    let inner = al(&mut d, middle, LayoutMode::Vertical, (1.0, 1.0), (Sizing::Fill, Sizing::Hug));
    let bar = rect(&mut d, inner, 1.0, 30.0);
    sizing(&mut d, bar, Sizing::Fill, Sizing::Fixed);
    let tag = rect(&mut d, inner, 20.0, 40.0);
    let side = al(&mut d, middle, LayoutMode::Horizontal, (1.0, 1.0), (Sizing::Hug, Sizing::Hug));
    layout_mut(&mut d, side).padding = [5.0; 4];
    rect(&mut d, side, 10.0, 10.0);
    layout_page(&mut d, p, &NoMeasure);
    assert_eq!(size(&d, header), (380.0, 20.0));
    assert_eq!(size(&d, side), (20.0, 20.0));
    // 380 - icon 50 - side 20 - two gaps.
    assert_eq!(pos(&d, inner), (60.0, 0.0, 290.0, 80.0));
    assert_eq!(size(&d, bar), (290.0, 30.0));
    assert_eq!(pos(&d, tag), (0.0, 40.0, 20.0, 40.0));
    assert_eq!(pos(&d, middle), (10.0, 40.0, 380.0, 80.0));
    assert_eq!(pos(&d, side).0, 360.0);
    assert_eq!(pos(&d, icon), (0.0, 0.0, 50.0, 20.0));
    assert_eq!(size(&d, outer), (400.0, 130.0));
    // The outer column shrinks: everything follows in one pass.
    set_size(&mut d, outer, 300.0, 130.0);
    layout_page(&mut d, p, &NoMeasure);
    assert_eq!(size(&d, inner), (190.0, 80.0));
    assert_eq!(size(&d, bar), (190.0, 30.0));
    assert_eq!(pos(&d, side).0, 260.0);
}

#[test]
fn hug_ignores_absolute_and_hidden_children() {
    let mut d = Document::new("t");
    let p = page(&d);
    let outer = al(&mut d, p, LayoutMode::Vertical, (1.0, 1.0), (Sizing::Hug, Sizing::Hug));
    let f = al(&mut d, outer, LayoutMode::Horizontal, (1.0, 1.0), (Sizing::Hug, Sizing::Hug));
    rect(&mut d, f, 50.0, 20.0);
    let hidden = rect(&mut d, f, 100.0, 100.0);
    d.get_mut(hidden).unwrap().visible = false;
    let b = rect(&mut d, f, 30.0, 10.0);
    let abs = d.create(f, NodeKind::Rectangle, "Abs", 300.0, 300.0, 40.0, 40.0).unwrap();
    d.get_mut(abs).unwrap().child_layout.absolute = true;
    layout_page(&mut d, p, &NoMeasure);
    // No gap for the hidden child either.
    assert_eq!(size(&d, f), (90.0, 20.0));
    assert_eq!(pos(&d, b).0, 60.0);
    assert_eq!(size(&d, outer), (90.0, 20.0));
    // The absolute child stays where it was.
    assert_eq!(pos(&d, abs), (300.0, 300.0, 40.0, 40.0));
}

#[test]
fn space_between_with_one_child_starts_at_the_start() {
    let mut d = Document::new("t");
    let p = page(&d);
    for mode in [LayoutMode::Horizontal, LayoutMode::Vertical] {
        let f = al(&mut d, p, mode, (300.0, 300.0), (Sizing::Fixed, Sizing::Fixed));
        layout_mut(&mut d, f).padding = [5.0; 4];
        layout_mut(&mut d, f).main_align = MainAlign::SpaceBetween;
        let a = rect(&mut d, f, 50.0, 50.0);
        layout_frame(&mut d, f, &NoMeasure);
        assert_eq!(pos(&d, a), (5.0, 5.0, 50.0, 50.0), "{mode:?}");
        // A second child goes to the far end.
        let b = rect(&mut d, f, 50.0, 50.0);
        layout_frame(&mut d, f, &NoMeasure);
        let far = if mode == LayoutMode::Horizontal { pos(&d, b).0 } else { pos(&d, b).1 };
        assert_eq!(far, 245.0, "{mode:?}");
    }
}

#[test]
fn wrap_with_fill_children() {
    let mut d = Document::new("t");
    let p = page(&d);
    // Fixed height: fill height has room to fill (a frame hugging its height reads it as fixed).
    let f = al(&mut d, p, LayoutMode::Horizontal, (200.0, 100.0), (Sizing::Fixed, Sizing::Fixed));
    layout_mut(&mut d, f).wrap = true;
    let a = rect(&mut d, f, 10.0, 20.0);
    let b = rect(&mut d, f, 10.0, 20.0);
    for c in [a, b] {
        sizing(&mut d, c, Sizing::Fill, Sizing::Fixed);
        d.get_mut(c).unwrap().child_layout.min_w = Some(80.0);
    }
    let c = rect(&mut d, f, 50.0, 30.0);
    // Fill height in a wrapped row: as tall as the row.
    let e = rect(&mut d, f, 40.0, 10.0);
    sizing(&mut d, e, Sizing::Fixed, Sizing::Fill);
    layout_frame(&mut d, f, &NoMeasure);
    // Row 1: the two fill children (at their 80 min they fit, 50 more doesn't) share 190.
    assert_eq!(pos(&d, a), (0.0, 0.0, 95.0, 20.0));
    assert_eq!(pos(&d, b), (105.0, 0.0, 95.0, 20.0));
    // Row 2: the fixed child and the fill-height one, which takes the row's height.
    assert_eq!(pos(&d, c), (0.0, 30.0, 50.0, 30.0));
    assert_eq!(pos(&d, e), (60.0, 30.0, 40.0, 30.0));
    assert_eq!(size(&d, f), (200.0, 100.0));
    // A row of nothing but fill-height children keeps their own height.
    let g = al(&mut d, p, LayoutMode::Horizontal, (100.0, 200.0), (Sizing::Fixed, Sizing::Fixed));
    layout_mut(&mut d, g).wrap = true;
    let tall = rect(&mut d, g, 60.0, 50.0);
    let lone = rect(&mut d, g, 60.0, 25.0);
    sizing(&mut d, lone, Sizing::Fixed, Sizing::Fill);
    layout_frame(&mut d, g, &NoMeasure);
    assert_eq!(pos(&d, tall), (0.0, 0.0, 60.0, 50.0));
    assert_eq!(pos(&d, lone), (0.0, 60.0, 60.0, 25.0));
    // Hugging its height, the frame wraps both rows.
    d.get_mut(g).unwrap().child_layout.sizing_v = Sizing::Hug;
    layout_frame(&mut d, g, &NoMeasure);
    assert_eq!(size(&d, g), (100.0, 85.0));
}

#[test]
fn padding_and_included_strokes() {
    let mut d = Document::new("t");
    let p = page(&d);
    let f = al(&mut d, p, LayoutMode::Horizontal, (1.0, 1.0), (Sizing::Hug, Sizing::Hug));
    {
        let n = d.get_mut(f).unwrap();
        n.strokes = vec![figcraft_doc::color::Paint::solid(figcraft_doc::color::Color::BLACK)];
        n.stroke.weight = 5.0;
    }
    layout_mut(&mut d, f).padding = [1.0, 2.0, 3.0, 4.0];
    layout_mut(&mut d, f).strokes_included = false;
    let a = rect(&mut d, f, 50.0, 20.0);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(size(&d, f), (56.0, 24.0));
    assert_eq!(pos(&d, a), (4.0, 1.0, 50.0, 20.0));
    layout_mut(&mut d, f).strokes_included = true;
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(size(&d, f), (66.0, 34.0));
    assert_eq!(pos(&d, a), (9.0, 6.0, 50.0, 20.0));
    // A fixed frame: fill children share the space inside padding and the included stroke.
    let g = al(&mut d, p, LayoutMode::Horizontal, (100.0, 50.0), (Sizing::Fixed, Sizing::Fixed));
    {
        let n = d.get_mut(g).unwrap();
        n.strokes = vec![figcraft_doc::color::Paint::solid(figcraft_doc::color::Color::BLACK)];
        n.stroke.weight = 5.0;
    }
    layout_mut(&mut d, g).padding = [10.0; 4];
    layout_mut(&mut d, g).strokes_included = true;
    let c = rect(&mut d, g, 10.0, 10.0);
    sizing(&mut d, c, Sizing::Fill, Sizing::Fill);
    layout_frame(&mut d, g, &NoMeasure);
    assert_eq!(pos(&d, c), (15.0, 15.0, 70.0, 20.0));
}

/// "Figma distributes space amongst fill container children by the children's content area
/// instead of by their size, matching the CSS border-box model. A layer with a thicker stroke will
/// take up slightly more of the available width or height, so that its inner content area matches
/// its sibling's." (Use auto layout with CSS Flexbox in mind)
#[test]
fn fill_children_share_by_content_area() {
    let mut d = Document::new("t");
    let p = page(&d);
    let f = al(&mut d, p, LayoutMode::Horizontal, (200.0, 50.0), (Sizing::Fixed, Sizing::Fixed));
    layout_mut(&mut d, f).gap = 0.0;
    let a = al(&mut d, f, LayoutMode::Horizontal, (10.0, 50.0), (Sizing::Fill, Sizing::Fixed));
    let b = al(&mut d, f, LayoutMode::Horizontal, (10.0, 50.0), (Sizing::Fill, Sizing::Fixed));
    layout_mut(&mut d, b).padding = [0.0, 20.0, 0.0, 20.0];
    layout_frame(&mut d, f, &NoMeasure);
    // 200 less b's 40 of padding leaves 160 of content, 80 each: a is 80 wide, b 120.
    assert_eq!((size(&d, a).0, size(&d, b).0), (80.0, 120.0));
    assert_eq!(pos(&d, b).0, 80.0);
    // A 10 px inside stroke counted in b's layout instead: 90 and 110.
    layout_mut(&mut d, b).padding = [0.0; 4];
    {
        let n = d.get_mut(b).unwrap();
        n.strokes = vec![figcraft_doc::color::Paint::solid(figcraft_doc::color::Color::BLACK)];
        n.stroke.weight = 10.0;
        n.stroke.align = StrokeAlign::Inside;
    }
    layout_mut(&mut d, b).strokes_included = true;
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!((size(&d, a).0, size(&d, b).0), (90.0, 110.0));
    // A stroke not counted in the layout is no inset: an even split, as before.
    layout_mut(&mut d, b).strokes_included = false;
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!((size(&d, a).0, size(&d, b).0), (100.0, 100.0));
    // Equal insets: still even. A max on a freezes it there and b takes the rest, insets kept.
    layout_mut(&mut d, a).padding = [0.0, 20.0, 0.0, 20.0];
    layout_mut(&mut d, b).padding = [0.0, 20.0, 0.0, 20.0];
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!((size(&d, a).0, size(&d, b).0), (100.0, 100.0));
    d.get_mut(a).unwrap().child_layout.max_w = Some(60.0);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!((size(&d, a).0, size(&d, b).0), (60.0, 140.0));
}

#[test]
fn grid_hug_tracks_and_wrapping_text_rows() {
    use figcraft_doc::GridTrack;
    let mut d = Document::new("t");
    let p = page(&d);
    let f = al(&mut d, p, LayoutMode::Grid, (300.0, 1.0), (Sizing::Fixed, Sizing::Hug));
    {
        let l = layout_mut(&mut d, f);
        l.grid_columns = 2;
        l.grid_rows = 1;
        l.column_tracks = vec![GridTrack::hug(), GridTrack::fill(1.0)];
    }
    let a = rect(&mut d, f, 60.0, 20.0);
    let t = fill_text(&mut d, f);
    let m = FakeMeasure([(t, (480.0, 20.0, 15.0))].into_iter().collect());
    layout_frame(&mut d, f, &m);
    // The hug column fits the rectangle; the text wraps in the rest (230) on three lines and the
    // hugging row (and frame) grow to fit it in the same pass.
    assert_eq!(pos(&d, a), (0.0, 0.0, 60.0, 20.0));
    assert_eq!(pos(&d, t), (70.0, 0.0, 230.0, 60.0));
    assert_eq!(size(&d, f), (300.0, 60.0));
    // A fill child in a hug column asks for its min width.
    sizing(&mut d, a, Sizing::Fill, Sizing::Fixed);
    d.get_mut(a).unwrap().child_layout.min_w = Some(40.0);
    layout_frame(&mut d, f, &m);
    assert_eq!(pos(&d, a).2, 40.0);
    assert_eq!(pos(&d, t).0, 50.0);
}

#[test]
fn fill_in_a_plain_frame_behaves_as_fixed() {
    let mut d = Document::new("t");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "Plain", 0.0, 0.0, 200.0, 100.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 10.0, 10.0, 50.0, 20.0).unwrap();
    sizing(&mut d, r, Sizing::Fill, Sizing::Fill);
    layout_page(&mut d, p, &NoMeasure);
    assert_eq!(pos(&d, r), (10.0, 10.0, 50.0, 20.0));
    // Resizing the frame applies the child's constraints (left/top), not fill.
    set_size(&mut d, f, 400.0, 300.0);
    apply_constraints(&mut d, f, 200.0, 100.0);
    assert_eq!(pos(&d, r), (10.0, 10.0, 50.0, 20.0));
    // And the sizing itself is made coherent: fill without an auto layout parent is fixed.
    assert!(coherent_sizing(&mut d, r));
    let cl = &d.node(r).unwrap().child_layout;
    assert_eq!((cl.sizing_h, cl.sizing_v), (Sizing::Fixed, Sizing::Fixed));
}

#[test]
fn coherent_sizing_rules() {
    let mut d = Document::new("t");
    let p = page(&d);
    let f = al(&mut d, p, LayoutMode::Horizontal, (200.0, 50.0), (Sizing::Hug, Sizing::Fixed));
    let a = rect(&mut d, f, 10.0, 10.0);
    sizing(&mut d, a, Sizing::Fill, Sizing::Fill);
    let abs = rect(&mut d, f, 10.0, 10.0);
    sizing(&mut d, abs, Sizing::Fill, Sizing::Fixed);
    d.get_mut(abs).unwrap().child_layout.absolute = true;
    // A plain rectangle can't hug.
    let r = rect(&mut d, p, 10.0, 10.0);
    sizing(&mut d, r, Sizing::Hug, Sizing::Fixed);
    // The parent hugs its width: the child's fill width becomes fixed; its fill height stays.
    assert!(coherent_sizing(&mut d, f));
    let cl = |d: &Document, id: NodeId| {
        let c = &d.node(id).unwrap().child_layout;
        (c.sizing_h, c.sizing_v)
    };
    assert_eq!(cl(&d, a), (Sizing::Fixed, Sizing::Fill));
    assert_eq!(cl(&d, abs), (Sizing::Fixed, Sizing::Fixed));
    assert_eq!(cl(&d, f), (Sizing::Hug, Sizing::Fixed));
    assert!(!coherent_sizing(&mut d, f), "already coherent");
    assert!(coherent_sizing(&mut d, r));
    assert_eq!(cl(&d, r), (Sizing::Fixed, Sizing::Fixed));
    // Text hugs without auto layout; a missing node is a no-op.
    let t = text(&mut d, p, "T");
    sizing(&mut d, t, Sizing::Hug, Sizing::Hug);
    assert!(!coherent_sizing(&mut d, t));
    // A text's sizing follows its resize mode: auto height hugs the height, a fixed box neither.
    let mode = |d: &mut Document, m: TextAutoResize| {
        if let NodeKind::Text(td) = &mut d.get_mut(t).unwrap().kind {
            td.auto_resize = m;
        }
    };
    mode(&mut d, TextAutoResize::None);
    assert!(coherent_sizing(&mut d, t));
    assert_eq!(cl(&d, t), (Sizing::Fixed, Sizing::Fixed));
    mode(&mut d, TextAutoResize::Height);
    assert!(coherent_sizing(&mut d, t));
    assert_eq!(cl(&d, t), (Sizing::Fixed, Sizing::Hug));
    mode(&mut d, TextAutoResize::WidthAndHeight);
    assert!(coherent_sizing(&mut d, t));
    assert_eq!(cl(&d, t), (Sizing::Hug, Sizing::Hug));
    assert!(!coherent_sizing(&mut d, NodeId(u64::MAX)));
}
