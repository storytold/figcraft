//! Auto layout behaviour suite: each case is a rule from the public documentation of auto layout
//! (hug / fill / fixed, min/max, padding, strokes, gaps, auto gaps, alignment, baselines, wrap,
//! absolute position and constraints, text, grid tracks and cells, canvas stacking) with the sizes
//! and positions the rule implies. The comment on each case is the one-line derivation. Where the
//! documentation is silent the case says so and pins the current behaviour instead.

use super::tests::FakeMeasure;
use super::*;
use figcraft_doc::color::{Color, Paint};
use figcraft_doc::{Constraints, GridTrack, Node, TextData};
use std::collections::HashMap;

use CellAlign as CA;
use LayoutMode::{Grid, Horizontal as H, Vertical as V};
use Sizing::{Fill, Fixed, Hug};

/// An auto layout frame: everything a case can set (`FRAME` holds the defaults).
#[derive(Clone, Copy)]
struct Frame<'a> {
    mode: LayoutMode,
    wrap: bool,
    /// The flow runs backwards: the children are laid out from the end of the main axis.
    reverse: bool,
    size: (f64, f64),
    sizing: (Sizing, Sizing),
    pad: [f64; 4],
    gap: f64,
    cross_gap: f64,
    main: MainAlign,
    cross: CrossAlign,
    /// How the wrapped lines share the cross axis.
    content: ContentAlign,
    /// A visible stroke: weight, alignment and optional per-side weights.
    stroke: Option<(f64, StrokeAlign, Option<[f64; 4]>)>,
    strokes_included: bool,
    min: (Option<f64>, Option<f64>),
    max: (Option<f64>, Option<f64>),
    columns: &'a [GridTrack],
    rows: &'a [GridTrack],
}

const FRAME: Frame<'static> = Frame {
    mode: H,
    wrap: false,
    reverse: false,
    size: (100.0, 100.0),
    sizing: (Fixed, Fixed),
    pad: [0.0; 4],
    gap: 0.0,
    cross_gap: 0.0,
    main: MainAlign::Min,
    cross: CrossAlign::Min,
    content: ContentAlign::Auto,
    stroke: None,
    strokes_included: true,
    min: (None, None),
    max: (None, None),
    columns: &[],
    rows: &[],
};

/// A child: a rectangle, or a text (natural width, height and first baseline) when `text` is set.
#[derive(Clone, Copy)]
struct Kid {
    size: (f64, f64),
    sizing: (Sizing, Sizing),
    min: (Option<f64>, Option<f64>),
    max: (Option<f64>, Option<f64>),
    text: Option<(f64, f64, f64)>,
    absolute: bool,
    at: (f64, f64),
    constraints: (HConstraint, VConstraint),
    cell: Option<(u32, u32)>,
    span: (u32, u32),
    align: (Option<CellAlign>, Option<CellAlign>),
    /// The child's own cross-axis alignment in a flex layout.
    align_self: Option<ItemAlign>,
}

const KID: Kid = Kid {
    size: (20.0, 20.0),
    sizing: (Fixed, Fixed),
    min: (None, None),
    max: (None, None),
    text: None,
    absolute: false,
    at: (0.0, 0.0),
    constraints: (HConstraint::Left, VConstraint::Top),
    cell: None,
    span: (1, 1),
    align: (None, None),
    align_self: None,
};

const fn kid(w: f64, h: f64) -> Kid {
    Kid { size: (w, h), ..KID }
}

const fn fixed(v: f64) -> GridTrack {
    GridTrack { sizing: Fixed, value: v, percent: false }
}

const fn fr(v: f64) -> GridTrack {
    GridTrack { sizing: Fill, value: v, percent: false }
}

/// A track that takes this share of the frame's inner size (the CSS `percent` track).
const fn pct(share: f64) -> GridTrack {
    GridTrack { sizing: Fixed, value: share, percent: true }
}

const HUG: GridTrack = GridTrack { sizing: Hug, value: 0.0, percent: false };

struct Case<'a> {
    rule: &'static str,
    frame: Frame<'a>,
    kids: &'a [Kid],
    /// Expected frame size and each child's `(x, y, w, h)` in the frame.
    want: (f64, f64),
    rects: &'a [(f64, f64, f64, f64)],
}

fn build(c: &Case<'_>) -> (Document, NodeId, Vec<NodeId>, FakeMeasure) {
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let f = c.frame;
    let id = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, f.size.0, f.size.1).unwrap();
    {
        let n = d.get_mut(id).unwrap();
        n.layout = Some(AutoLayout {
            mode: f.mode,
            wrap: f.wrap,
            gap: f.gap,
            cross_gap: f.cross_gap,
            padding: f.pad,
            main_align: f.main,
            cross_align: f.cross,
            grid_columns: f.columns.len().max(1) as u32,
            grid_rows: f.rows.len().max(1) as u32,
            column_tracks: f.columns.to_vec(),
            row_tracks: f.rows.to_vec(),
            strokes_included: f.strokes_included,
            reverse_z: false,
            reverse: f.reverse,
            content_align: f.content,
        });
        (n.child_layout.sizing_h, n.child_layout.sizing_v) = f.sizing;
        (n.child_layout.min_w, n.child_layout.min_h) = f.min;
        (n.child_layout.max_w, n.child_layout.max_h) = f.max;
        if let Some((w, align, sides)) = f.stroke {
            n.strokes = vec![Paint::solid(Color::BLACK)];
            n.stroke.weight = w;
            n.stroke.align = align;
            n.stroke.sides = sides;
        }
    }
    let mut texts = HashMap::new();
    let mut ids = Vec::new();
    for k in c.kids {
        let kind = if k.text.is_some() { NodeKind::Text(TextData::new("T")) } else { NodeKind::Rectangle };
        let kid = d.create(id, kind, "K", k.at.0, k.at.1, k.size.0, k.size.1).unwrap();
        let n = d.get_mut(kid).unwrap();
        if let (Some(t), NodeKind::Text(td)) = (k.text, &mut n.kind) {
            // Auto width hugs both axes; filling the width wraps (auto height).
            td.auto_resize = if k.sizing.0 == Fill { TextAutoResize::Height } else { TextAutoResize::WidthAndHeight };
            texts.insert(kid, t);
        }
        let cl = &mut n.child_layout;
        (cl.sizing_h, cl.sizing_v) = k.sizing;
        (cl.min_w, cl.min_h) = k.min;
        (cl.max_w, cl.max_h) = k.max;
        cl.absolute = k.absolute;
        (cl.grid_column, cl.grid_row) = (k.cell.map(|c| c.0), k.cell.map(|c| c.1));
        (cl.grid_column_span, cl.grid_row_span) = k.span;
        (cl.grid_align_h, cl.grid_align_v) = k.align;
        cl.align_self = k.align_self;
        n.constraints = Constraints { horizontal: k.constraints.0, vertical: k.constraints.1 };
        ids.push(kid);
    }
    (d, id, ids, FakeMeasure(texts))
}

fn rect_of(d: &Document, id: NodeId) -> (f64, f64, f64, f64) {
    let n: &Node = d.node(id).unwrap();
    let c = n.transform.as_coeffs();
    (c[4], c[5], n.width, n.height)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn check(cases: &[Case<'_>]) {
    let mut failures = Vec::new();
    for c in cases {
        let (mut d, f, ids, m) = build(c);
        layout_frame(&mut d, f, &m);
        let n = d.node(f).unwrap();
        let got: Vec<_> = ids.iter().map(|k| rect_of(&d, *k)).collect();
        let size_ok = close(n.width, c.want.0) && close(n.height, c.want.1);
        let rects_ok = got.len() == c.rects.len()
            && got.iter().zip(c.rects).all(|(g, w)| close(g.0, w.0) && close(g.1, w.1) && close(g.2, w.2) && close(g.3, w.3));
        if !size_ok || !rects_ok {
            failures.push(format!("{}\n  frame {:?} want {:?}\n  kids {:?}\n  want {:?}", c.rule, (n.width, n.height), c.want, got, c.rects));
        }
        // Laying out again changes nothing (one pass settles).
        let before: Vec<_> = ids.iter().map(|k| rect_of(&d, *k)).collect();
        layout_frame(&mut d, f, &m);
        let after: Vec<_> = ids.iter().map(|k| rect_of(&d, *k)).collect();
        if before != after {
            failures.push(format!("{}: a second pass moved children {before:?} -> {after:?}", c.rule));
        }
    }
    assert!(failures.is_empty(), "{} rule(s) diverge:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn sizing_rules() {
    check(&[
        Case {
            rule: "hug: a 40 px text plus 10 px left and right padding makes a 60 px frame",
            // 10 + 40 + 10 = 60 wide; 20 tall (no vertical padding).
            frame: Frame { sizing: (Hug, Hug), pad: [0.0, 10.0, 0.0, 10.0], ..FRAME },
            kids: &[Kid { text: Some((40.0, 20.0, 15.0)), sizing: (Hug, Hug), ..KID }],
            want: (60.0, 20.0),
            rects: &[(10.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "fixed: the frame keeps its size whatever its children need",
            // 3 × 50 = 150 overflows the fixed 100 px frame; it stays 100 × 50.
            frame: Frame { size: (100.0, 50.0), ..FRAME },
            kids: &[kid(50.0, 20.0), kid(50.0, 20.0), kid(50.0, 20.0)],
            want: (100.0, 50.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (50.0, 0.0, 50.0, 20.0), (100.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "fill: children share the space left after padding, gaps and fixed siblings",
            // 300 - 2×10 padding - 60 fixed - 2×20 gaps = 180 → 90 each.
            frame: Frame { size: (300.0, 40.0), pad: [0.0, 10.0, 0.0, 10.0], gap: 20.0, ..FRAME },
            kids: &[kid(60.0, 20.0), Kid { sizing: (Fill, Fixed), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (300.0, 40.0),
            rects: &[(10.0, 0.0, 60.0, 20.0), (90.0, 0.0, 90.0, 20.0), (200.0, 0.0, 90.0, 20.0)],
        },
        Case {
            rule: "fill across the flow stretches to the frame's inner height",
            // 100 - 2×10 padding = 80.
            frame: Frame { size: (100.0, 100.0), pad: [10.0; 4], ..FRAME },
            kids: &[Kid { sizing: (Fixed, Fill), ..KID }],
            want: (100.0, 100.0),
            rects: &[(10.0, 10.0, 20.0, 80.0)],
        },
        Case {
            rule: "fill on an axis the frame hugs has nothing to fill: the child keeps its size",
            // A hugging frame has no free space; it hugs 50 + 30.
            frame: Frame { sizing: (Hug, Hug), ..FRAME },
            kids: &[kid(50.0, 20.0), Kid { size: (30.0, 40.0), sizing: (Fill, Fill), ..KID }],
            want: (80.0, 40.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (50.0, 0.0, 30.0, 40.0)],
        },
        Case {
            rule: "min width: a fixed child is never narrower than its minimum",
            frame: Frame { sizing: (Hug, Hug), ..FRAME },
            kids: &[Kid { size: (50.0, 20.0), min: (Some(80.0), None), ..KID }],
            want: (80.0, 20.0),
            rects: &[(0.0, 0.0, 80.0, 20.0)],
        },
        Case {
            rule: "max width on a fill child: it stops at its max and the other fill child takes the rest",
            // 200 shared: one stops at 50, the other fills the remaining 150.
            frame: Frame { size: (200.0, 20.0), ..FRAME },
            kids: &[Kid { sizing: (Fill, Fixed), max: (Some(50.0), None), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (200.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (50.0, 0.0, 150.0, 20.0)],
        },
        Case {
            rule: "max width on a hugging frame: it hugs up to its max",
            // Content 300 wide, max 200.
            frame: Frame { sizing: (Hug, Hug), max: (Some(200.0), None), ..FRAME },
            kids: &[kid(300.0, 20.0)],
            want: (200.0, 20.0),
            rects: &[(0.0, 0.0, 300.0, 20.0)],
        },
        Case {
            rule: "min height on a hugging frame: it hugs, but never below its min",
            frame: Frame { sizing: (Hug, Hug), min: (None, Some(60.0)), ..FRAME },
            kids: &[kid(30.0, 20.0)],
            want: (30.0, 60.0),
            rects: &[(0.0, 0.0, 30.0, 20.0)],
        },
        Case {
            rule: "a hugging text wider than its max width wraps at the max",
            // 150 px of text at max 100 wraps to 2 lines of 20.
            frame: Frame { sizing: (Hug, Hug), ..FRAME },
            kids: &[Kid { text: Some((150.0, 20.0, 15.0)), sizing: (Hug, Hug), max: (Some(100.0), None), ..KID }],
            want: (100.0, 40.0),
            rects: &[(0.0, 0.0, 100.0, 40.0)],
        },
        Case {
            rule: "a text filling the width wraps and its hugging frame grows to its new height",
            // 150 px of text in an 80 px inner width (100 - 2×10) needs 2 lines: 40 + 20 padding.
            frame: Frame { size: (100.0, 10.0), sizing: (Fixed, Hug), pad: [10.0; 4], ..FRAME },
            kids: &[Kid { text: Some((150.0, 20.0, 15.0)), sizing: (Fill, Hug), ..KID }],
            want: (100.0, 60.0),
            rects: &[(10.0, 10.0, 80.0, 40.0)],
        },
    ]);
}

#[test]
fn padding_and_stroke_rules() {
    let inside = Some((4.0, StrokeAlign::Inside, None));
    check(&[
        Case {
            rule: "padding: space between the frame's edge and its children, per side",
            // 4 + 50 + 2 wide, 1 + 20 + 3 tall; the child sits at (left, top).
            frame: Frame { sizing: (Hug, Hug), pad: [1.0, 2.0, 3.0, 4.0], ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (56.0, 24.0),
            rects: &[(4.0, 1.0, 50.0, 20.0)],
        },
        Case {
            rule: "a fixed frame is never smaller than its padding: it sizes up to fit",
            // 10 px fixed, 20 px padding each side → 40.
            frame: Frame { size: (10.0, 10.0), pad: [20.0; 4], ..FRAME },
            kids: &[],
            want: (40.0, 40.0),
            rects: &[],
        },
        Case {
            rule: "the padding floor wins over a max size",
            // max width 10 < 20 + 20 padding → 40.
            frame: Frame { size: (100.0, 50.0), pad: [0.0, 20.0, 0.0, 20.0], max: (Some(10.0), None), ..FRAME },
            kids: &[],
            want: (40.0, 50.0),
            rects: &[],
        },
        Case {
            rule: "an empty hugging frame is its padding plus its included inside stroke",
            // 8 + 2 on each side → 20 × 20.
            frame: Frame { sizing: (Hug, Hug), pad: [8.0; 4], stroke: Some((2.0, StrokeAlign::Inside, None)), ..FRAME },
            kids: &[],
            want: (20.0, 20.0),
            rects: &[],
        },
        Case {
            rule: "strokes included: the frame's inside stroke takes up space like padding",
            // 4 + 50 + 4 by 4 + 20 + 4.
            frame: Frame { sizing: (Hug, Hug), stroke: inside, ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (58.0, 28.0),
            rects: &[(4.0, 4.0, 50.0, 20.0)],
        },
        Case {
            rule: "strokes excluded: the stroke takes no space",
            frame: Frame { sizing: (Hug, Hug), stroke: inside, strokes_included: false, ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "center strokes are never counted, even when strokes are included",
            frame: Frame { sizing: (Hug, Hug), stroke: Some((4.0, StrokeAlign::Center, None)), ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "outside strokes are never counted, even when strokes are included",
            frame: Frame { sizing: (Hug, Hug), stroke: Some((4.0, StrokeAlign::Outside, None)), ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0)],
        },
        Case {
            // Docs silent on per-side weights: each side counts its own inside weight.
            rule: "individual inside stroke weights count per side",
            // 4 + 50 + 2 by 1 + 20 + 3.
            frame: Frame { sizing: (Hug, Hug), stroke: Some((4.0, StrokeAlign::Inside, Some([1.0, 2.0, 3.0, 4.0]))), ..FRAME },
            kids: &[kid(50.0, 20.0)],
            want: (56.0, 24.0),
            rects: &[(4.0, 1.0, 50.0, 20.0)],
        },
        Case {
            rule: "fill children share the space inside padding and the included stroke",
            // 100 - 2×(10 + 5) = 70 by 50 - 30 = 20.
            frame: Frame { size: (100.0, 50.0), pad: [10.0; 4], stroke: Some((5.0, StrokeAlign::Inside, None)), ..FRAME },
            kids: &[Kid { sizing: (Fill, Fill), ..KID }],
            want: (100.0, 50.0),
            rects: &[(15.0, 15.0, 70.0, 20.0)],
        },
    ]);
}

#[test]
fn gap_rules() {
    let three = &[kid(20.0, 20.0), kid(20.0, 20.0), kid(20.0, 20.0)];
    check(&[
        Case {
            rule: "gap: a fixed distance between children; a hugging frame includes it",
            // 20 + 10 + 20 + 10 + 20 = 80.
            frame: Frame { sizing: (Hug, Hug), gap: 10.0, ..FRAME },
            kids: three,
            want: (80.0, 20.0),
            rects: &[(0.0, 0.0, 20.0, 20.0), (30.0, 0.0, 20.0, 20.0), (60.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "negative gap: children overlap by the gap",
            // 50 - 10 = 40 per step; hug 50 + 40 = 90.
            frame: Frame { sizing: (Hug, Hug), gap: -10.0, ..FRAME },
            kids: &[kid(50.0, 20.0), kid(50.0, 20.0)],
            want: (90.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (40.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "vertical gap: the same along y",
            frame: Frame { mode: V, sizing: (Hug, Hug), gap: 5.0, ..FRAME },
            kids: &[kid(20.0, 30.0), kid(40.0, 10.0)],
            want: (40.0, 45.0),
            rects: &[(0.0, 0.0, 20.0, 30.0), (0.0, 35.0, 40.0, 10.0)],
        },
        Case {
            rule: "auto gap between: as far apart as possible, no space at the edges",
            // 200 - 60 = 140 over 2 gaps → 70.
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceBetween, gap: 10.0, ..FRAME },
            kids: three,
            want: (200.0, 20.0),
            rects: &[(0.0, 0.0, 20.0, 20.0), (90.0, 0.0, 20.0, 20.0), (180.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "auto gap evenly: equal space in every gap and at both edges (1:1)",
            // 140 over 4 spaces → 35.
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceEvenly, ..FRAME },
            kids: three,
            want: (200.0, 20.0),
            rects: &[(35.0, 0.0, 20.0, 20.0), (90.0, 0.0, 20.0, 20.0), (145.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "auto gap around: gaps twice the edge space (2:1)",
            // 140 = 2e + 2×(2e) → e = 23.33, gap 46.67.
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceAround, ..FRAME },
            kids: three,
            want: (200.0, 20.0),
            rects: &[(70.0 / 3.0, 0.0, 20.0, 20.0), (90.0, 0.0, 20.0, 20.0), (470.0 / 3.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "auto gap stops at 0 when the children don't fit (no overlap)",
            // 30 + 30 > 50: the gap is 0, not -10.
            frame: Frame { size: (50.0, 20.0), main: MainAlign::SpaceBetween, ..FRAME },
            kids: &[kid(30.0, 20.0), kid(30.0, 20.0)],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 30.0, 20.0), (30.0, 0.0, 30.0, 20.0)],
        },
        Case {
            rule: "auto gap evenly stops at 0 too (children start at the edge)",
            frame: Frame { size: (50.0, 20.0), main: MainAlign::SpaceEvenly, ..FRAME },
            kids: &[kid(30.0, 20.0), kid(30.0, 20.0)],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 30.0, 20.0), (30.0, 0.0, 30.0, 20.0)],
        },
        Case {
            rule: "a single child of auto gap between aligns to the start (left)",
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceBetween, ..FRAME },
            kids: &[kid(20.0, 20.0)],
            want: (200.0, 20.0),
            rects: &[(0.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "a single child of auto gap evenly has equal space on both sides",
            // (200 - 20) / 2 = 90.
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceEvenly, ..FRAME },
            kids: &[kid(20.0, 20.0)],
            want: (200.0, 20.0),
            rects: &[(90.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "auto gap with fill children: they take the free space, leaving no gap",
            // Fill takes 200 - 20 - 20; between has nothing left to share.
            frame: Frame { size: (200.0, 20.0), main: MainAlign::SpaceBetween, gap: 10.0, ..FRAME },
            kids: &[kid(20.0, 20.0), Kid { sizing: (Fill, Fixed), ..KID }, kid(20.0, 20.0)],
            want: (200.0, 20.0),
            rects: &[(0.0, 0.0, 20.0, 20.0), (20.0, 0.0, 160.0, 20.0), (180.0, 0.0, 20.0, 20.0)],
        },
    ]);
}

#[test]
fn alignment_rules() {
    let one = &[kid(50.0, 20.0)];
    check(&[
        Case {
            rule: "align top left (the default)",
            frame: Frame { size: (200.0, 100.0), ..FRAME },
            kids: one,
            want: (200.0, 100.0),
            rects: &[(0.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "align center: centered on both axes",
            // ((200 - 50) / 2, (100 - 20) / 2).
            frame: Frame { size: (200.0, 100.0), main: MainAlign::Center, cross: CrossAlign::Center, ..FRAME },
            kids: one,
            want: (200.0, 100.0),
            rects: &[(75.0, 40.0, 50.0, 20.0)],
        },
        Case {
            rule: "align bottom right",
            frame: Frame { size: (200.0, 100.0), main: MainAlign::Max, cross: CrossAlign::Max, ..FRAME },
            kids: one,
            want: (200.0, 100.0),
            rects: &[(150.0, 80.0, 50.0, 20.0)],
        },
        Case {
            rule: "vertical flow: main alignment is vertical, cross alignment horizontal",
            // Main max → y = 100 - 20; cross center → x = 75.
            frame: Frame { mode: V, size: (200.0, 100.0), main: MainAlign::Max, cross: CrossAlign::Center, ..FRAME },
            kids: one,
            want: (200.0, 100.0),
            rects: &[(75.0, 80.0, 50.0, 20.0)],
        },
        Case {
            rule: "alignment works inside the padding",
            // Center of the inner 180 × 80 box at (10, 10).
            frame: Frame { size: (200.0, 100.0), pad: [10.0; 4], main: MainAlign::Center, cross: CrossAlign::Center, ..FRAME },
            kids: one,
            want: (200.0, 100.0),
            rects: &[(75.0, 40.0, 50.0, 20.0)],
        },
        Case {
            rule: "a hugging frame has no extra space: alignment changes nothing visible",
            frame: Frame { sizing: (Hug, Hug), main: MainAlign::Max, cross: CrossAlign::Max, ..FRAME },
            kids: one,
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0)],
        },
        Case {
            rule: "auto gap keeps the cross alignment (top / center / bottom)",
            frame: Frame { size: (200.0, 100.0), main: MainAlign::SpaceBetween, cross: CrossAlign::Center, ..FRAME },
            kids: &[kid(20.0, 20.0), kid(20.0, 40.0)],
            want: (200.0, 100.0),
            rects: &[(0.0, 40.0, 20.0, 20.0), (180.0, 30.0, 20.0, 40.0)],
        },
        Case {
            rule: "text baselines: texts of different sizes sit on one baseline",
            // Baselines 10 and 20: the first moves down 10; the frame hugs max(10 + 14, 28).
            frame: Frame { sizing: (Hug, Hug), cross: CrossAlign::Baseline, gap: 5.0, ..FRAME },
            kids: &[
                Kid { text: Some((30.0, 14.0, 10.0)), sizing: (Hug, Hug), ..KID },
                Kid { text: Some((60.0, 28.0, 20.0)), sizing: (Hug, Hug), ..KID },
            ],
            want: (95.0, 28.0),
            rects: &[(0.0, 10.0, 30.0, 14.0), (35.0, 0.0, 60.0, 28.0)],
        },
        Case {
            // Docs silent on layers without text: they keep to the top of the line.
            rule: "baseline alignment: a layer without text sits at the top",
            frame: Frame { sizing: (Hug, Hug), cross: CrossAlign::Baseline, ..FRAME },
            kids: &[kid(10.0, 10.0), Kid { text: Some((60.0, 28.0, 20.0)), sizing: (Hug, Hug), ..KID }],
            want: (70.0, 28.0),
            rects: &[(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 60.0, 28.0)],
        },
    ]);
}

#[test]
fn wrap_rules() {
    let three = &[kid(40.0, 20.0), kid(40.0, 20.0), kid(40.0, 20.0)];
    check(&[
        Case {
            rule: "horizontal wrap: left to right, a new row when the width runs out",
            // 40 + 10 + 40 = 90 fits in 100; the third wraps to y = 20 + 5.
            frame: Frame { wrap: true, size: (100.0, 10.0), sizing: (Fixed, Hug), gap: 10.0, cross_gap: 5.0, ..FRAME },
            kids: three,
            want: (100.0, 45.0),
            rects: &[(0.0, 0.0, 40.0, 20.0), (50.0, 0.0, 40.0, 20.0), (0.0, 25.0, 40.0, 20.0)],
        },
        Case {
            rule: "vertical wrap: top to bottom, a new column when the height runs out",
            // 40 + 10 + 40 = 90 fits in 100; the third starts a column at x = 20 + 5.
            frame: Frame { mode: V, wrap: true, size: (10.0, 100.0), sizing: (Hug, Fixed), gap: 10.0, cross_gap: 5.0, ..FRAME },
            kids: &[kid(20.0, 40.0), kid(20.0, 40.0), kid(20.0, 40.0)],
            want: (45.0, 100.0),
            rects: &[(0.0, 0.0, 20.0, 40.0), (0.0, 50.0, 20.0, 40.0), (25.0, 0.0, 20.0, 40.0)],
        },
        Case {
            rule: "vertical wrap is not masonry: a column fills to the height before the next starts",
            // 30 + 30 + 30 = 90 ≤ 100 stay in column 1 even though column 2 is empty.
            frame: Frame { mode: V, wrap: true, size: (100.0, 100.0), ..FRAME },
            kids: &[kid(20.0, 30.0), kid(20.0, 30.0), kid(20.0, 30.0), kid(20.0, 30.0)],
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 20.0, 30.0), (0.0, 30.0, 20.0, 30.0), (0.0, 60.0, 20.0, 30.0), (20.0, 0.0, 20.0, 30.0)],
        },
        Case {
            rule: "wrap: auto gap spreads each row on its own",
            // Row 1: 100 - 80 between 2 → gap 20; row 2: a single child at the start.
            frame: Frame { wrap: true, size: (100.0, 100.0), main: MainAlign::SpaceBetween, gap: 10.0, cross_gap: 5.0, ..FRAME },
            kids: three,
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 40.0, 20.0), (60.0, 0.0, 40.0, 20.0), (0.0, 25.0, 40.0, 20.0)],
        },
        Case {
            rule: "wrap: cross alignment within each row",
            // Row heights 40 and 20; the short child centers in row 1: (40 - 20) / 2.
            frame: Frame { wrap: true, size: (100.0, 10.0), sizing: (Fixed, Hug), gap: 10.0, cross: CrossAlign::Center, ..FRAME },
            kids: &[kid(40.0, 20.0), kid(40.0, 40.0), kid(40.0, 20.0)],
            want: (100.0, 60.0),
            rects: &[(0.0, 10.0, 40.0, 20.0), (50.0, 0.0, 40.0, 40.0), (0.0, 40.0, 40.0, 20.0)],
        },
    ]);
}

#[test]
fn absolute_position_rules() {
    check(&[
        Case {
            rule: "ignore auto layout: the child keeps its place and the frame hugs only the flow",
            frame: Frame { sizing: (Hug, Hug), ..FRAME },
            kids: &[kid(50.0, 20.0), Kid { size: (200.0, 200.0), absolute: true, at: (5.0, 5.0), ..KID }],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (5.0, 5.0, 200.0, 200.0)],
        },
        Case {
            rule: "absolute children follow their constraints when the frame resizes (right / bottom)",
            // The frame hugs from 100 × 100 to 50 × 20: right and bottom offsets of 10 stay.
            frame: Frame { sizing: (Hug, Hug), ..FRAME },
            kids: &[
                kid(50.0, 20.0),
                Kid { size: (10.0, 10.0), absolute: true, at: (80.0, 80.0), constraints: (HConstraint::Right, VConstraint::Bottom), ..KID },
            ],
            want: (50.0, 20.0),
            rects: &[(0.0, 0.0, 50.0, 20.0), (30.0, 0.0, 10.0, 10.0)],
        },
        Case {
            rule: "absolute children follow their constraints (left and right / center)",
            // 200 → 100 wide: left and right keeps 10 px each side (width 80); center moves by -50.
            frame: Frame { size: (200.0, 100.0), sizing: (Hug, Fixed), ..FRAME },
            kids: &[
                kid(100.0, 20.0),
                Kid { size: (180.0, 10.0), absolute: true, at: (10.0, 50.0), constraints: (HConstraint::LeftRight, VConstraint::Top), ..KID },
                Kid { size: (20.0, 10.0), absolute: true, at: (90.0, 0.0), constraints: (HConstraint::Center, VConstraint::Top), ..KID },
            ],
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 100.0, 20.0), (10.0, 50.0, 80.0, 10.0), (40.0, 0.0, 20.0, 10.0)],
        },
    ]);
}

#[test]
fn grid_rules() {
    check(&[
        Case {
            rule: "fill tracks share the space by fr: 1fr and 2fr take a third and two thirds",
            frame: Frame { mode: Grid, size: (300.0, 50.0), columns: &[fr(1.0), fr(2.0)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fill), ..KID }, Kid { sizing: (Fill, Fill), ..KID }],
            want: (300.0, 50.0),
            rects: &[(0.0, 0.0, 100.0, 50.0), (100.0, 0.0, 200.0, 50.0)],
        },
        Case {
            rule: "fixed tracks keep their size; fill tracks share the rest after gaps and padding",
            // 300 - 2×10 padding - 10 gap - 100 = 170.
            frame: Frame { mode: Grid, size: (300.0, 50.0), pad: [0.0, 10.0, 0.0, 10.0], gap: 10.0, columns: &[fixed(100.0), fr(1.0)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fixed), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (300.0, 50.0),
            rects: &[(10.0, 0.0, 100.0, 20.0), (120.0, 0.0, 170.0, 20.0)],
        },
        Case {
            rule: "hug tracks keep the smallest size around their children",
            frame: Frame { mode: Grid, size: (300.0, 50.0), columns: &[HUG, fr(1.0)], ..FRAME },
            kids: &[kid(60.0, 20.0), Kid { sizing: (Fill, Fixed), ..KID }],
            want: (300.0, 50.0),
            rects: &[(0.0, 0.0, 60.0, 20.0), (60.0, 0.0, 240.0, 20.0)],
        },
        Case {
            rule: "fr tracks behave like the CSS fr unit: a fixed child taller than its share keeps its row",
            // Share 50 < 80: row 1 keeps 80, row 2 gets the remaining 20.
            frame: Frame { mode: Grid, size: (100.0, 100.0), columns: &[fr(1.0)], rows: &[fr(1.0), fr(1.0)], ..FRAME },
            kids: &[kid(20.0, 80.0), Kid { sizing: (Fixed, Fill), ..KID }],
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 20.0, 80.0), (0.0, 80.0, 20.0, 20.0)],
        },
        Case {
            rule: "fr tracks: children bigger than every share keep their sizes (the grid overflows)",
            frame: Frame { mode: Grid, size: (100.0, 100.0), columns: &[fr(1.0)], rows: &[fr(1.0), fr(1.0)], ..FRAME },
            kids: &[kid(20.0, 80.0), kid(20.0, 70.0)],
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 20.0, 80.0), (0.0, 80.0, 20.0, 70.0)],
        },
        Case {
            rule: "fr columns: a fixed child wider than its share keeps its column",
            // Share 100 < 150: column 1 keeps 150; column 2 (2fr alone) gets the other 150.
            frame: Frame { mode: Grid, size: (300.0, 20.0), columns: &[fr(1.0), fr(2.0)], ..FRAME },
            kids: &[kid(150.0, 20.0), Kid { sizing: (Fill, Fixed), ..KID }],
            want: (300.0, 20.0),
            rects: &[(0.0, 0.0, 150.0, 20.0), (150.0, 0.0, 150.0, 20.0)],
        },
        Case {
            rule: "placement: left to right, top to bottom; rows grow to fit the children",
            // 2 columns of 50, 3 children → 2 rows of 50 (fill rows share 100).
            frame: Frame { mode: Grid, size: (100.0, 100.0), columns: &[fr(1.0), fr(1.0)], rows: &[fr(1.0)], ..FRAME },
            kids: &[kid(10.0, 10.0), kid(10.0, 10.0), kid(10.0, 10.0)],
            want: (100.0, 100.0),
            rects: &[(0.0, 0.0, 10.0, 10.0), (50.0, 0.0, 10.0, 10.0), (0.0, 50.0, 10.0, 10.0)],
        },
        Case {
            rule: "gap between columns and between rows",
            frame: Frame {
                mode: Grid,
                size: (110.0, 105.0),
                gap: 10.0,
                cross_gap: 5.0,
                columns: &[fr(1.0), fr(1.0)],
                rows: &[fr(1.0), fr(1.0)],
                ..FRAME
            },
            kids: &[kid(10.0, 10.0), kid(10.0, 10.0), kid(10.0, 10.0)],
            want: (110.0, 105.0),
            rects: &[(0.0, 0.0, 10.0, 10.0), (60.0, 0.0, 10.0, 10.0), (0.0, 55.0, 10.0, 10.0)],
        },
        Case {
            rule: "a child spanning two columns fills both and the gap between them",
            frame: Frame { mode: Grid, size: (110.0, 20.0), gap: 10.0, columns: &[fr(1.0), fr(1.0)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fill), span: (2, 1), ..KID }],
            want: (110.0, 20.0),
            rects: &[(0.0, 0.0, 110.0, 20.0)],
        },
        Case {
            rule: "a child anchored to a cell sits there; the rest flow around it",
            frame: Frame { mode: Grid, size: (100.0, 100.0), columns: &[fr(1.0), fr(1.0)], rows: &[fr(1.0), fr(1.0)], ..FRAME },
            kids: &[kid(10.0, 10.0), Kid { cell: Some((0, 0)), ..KID }],
            want: (100.0, 100.0),
            rects: &[(50.0, 0.0, 10.0, 10.0), (0.0, 0.0, 20.0, 20.0)],
        },
        Case {
            rule: "children align in their cells (the frame's alignment)",
            // Cells 50 × 50: centered → (15, 15) and (65, 15).
            frame: Frame {
                mode: Grid,
                size: (100.0, 50.0),
                main: MainAlign::Center,
                cross: CrossAlign::Center,
                columns: &[fr(1.0), fr(1.0)],
                ..FRAME
            },
            kids: &[kid(20.0, 20.0), kid(20.0, 20.0)],
            want: (100.0, 50.0),
            rects: &[(15.0, 15.0, 20.0, 20.0), (65.0, 15.0, 20.0, 20.0)],
        },
        Case {
            rule: "a child aligns independently in its cell (its own alignment wins)",
            // The second child: right and bottom of its cell → (100 - 20, 50 - 20).
            frame: Frame {
                mode: Grid,
                size: (100.0, 50.0),
                main: MainAlign::Center,
                cross: CrossAlign::Center,
                columns: &[fr(1.0), fr(1.0)],
                ..FRAME
            },
            kids: &[kid(20.0, 20.0), Kid { align: (Some(CA::Max), Some(CA::Max)), ..KID }],
            want: (100.0, 50.0),
            rects: &[(15.0, 15.0, 20.0, 20.0), (80.0, 30.0, 20.0, 20.0)],
        },
        Case {
            rule: "a hugging grid keeps the smallest size around its tracks (fill tracks hug too)",
            // Columns 30 and 40 + gap 10 = 80; one row of 20.
            frame: Frame { mode: Grid, sizing: (Hug, Hug), gap: 10.0, columns: &[fr(1.0), fr(1.0)], ..FRAME },
            kids: &[kid(30.0, 20.0), kid(40.0, 10.0)],
            want: (80.0, 20.0),
            rects: &[(0.0, 0.0, 30.0, 20.0), (40.0, 0.0, 40.0, 10.0)],
        },
        Case {
            rule: "percentage tracks in a hugging grid size to their children's content",
            // Neither axis has a definite parent size: columns 30 and 40 + gap 10 = 80.
            frame: Frame { mode: Grid, sizing: (Hug, Hug), gap: 10.0, columns: &[pct(0.5), pct(0.5)], rows: &[pct(0.5)], ..FRAME },
            kids: &[kid(30.0, 20.0), kid(40.0, 10.0)],
            want: (80.0, 20.0),
            rects: &[(0.0, 0.0, 30.0, 20.0), (40.0, 0.0, 40.0, 10.0)],
        },
        Case {
            rule: "a percentage track participating in a spanning child also hugs its share",
            frame: Frame { mode: Grid, sizing: (Hug, Hug), gap: 10.0, columns: &[pct(0.5), fr(1.0)], ..FRAME },
            kids: &[Kid { size: (100.0, 20.0), span: (2, 1), ..KID }],
            want: (100.0, 20.0),
            rects: &[(0.0, 0.0, 100.0, 20.0)],
        },
        Case {
            rule: "padding: the space between the frame's edge and its cells",
            frame: Frame { mode: Grid, sizing: (Hug, Hug), pad: [1.0, 2.0, 3.0, 4.0], columns: &[HUG], ..FRAME },
            kids: &[kid(30.0, 20.0)],
            want: (36.0, 24.0),
            rects: &[(4.0, 1.0, 30.0, 20.0)],
        },
        Case {
            rule: "a percentage track takes its share of the frame's inner width",
            // 200 × 0.5 = 100.
            frame: Frame { mode: Grid, size: (200.0, 50.0), columns: &[pct(0.5), pct(0.5)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fixed), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (200.0, 50.0),
            rects: &[(0.0, 0.0, 100.0, 20.0), (100.0, 0.0, 100.0, 20.0)],
        },
        Case {
            rule: "a percentage track shares the width with the fill tracks, after the gaps",
            // 200 - 10 gap = 190; the percentage track takes 95, the fill track the other 95.
            frame: Frame { mode: Grid, size: (200.0, 50.0), gap: 10.0, columns: &[pct(0.5), fr(1.0)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fixed), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (200.0, 50.0),
            rects: &[(0.0, 0.0, 95.0, 20.0), (105.0, 0.0, 95.0, 20.0)],
        },
        Case {
            rule: "a percentage track takes its share of the inner width, without the padding",
            // (200 - 2×10) × 0.5 = 90.
            frame: Frame { mode: Grid, size: (200.0, 50.0), pad: [0.0, 10.0, 0.0, 10.0], columns: &[pct(0.5), pct(0.5)], ..FRAME },
            kids: &[Kid { sizing: (Fill, Fixed), ..KID }, Kid { sizing: (Fill, Fixed), ..KID }],
            want: (200.0, 50.0),
            rects: &[(10.0, 0.0, 90.0, 20.0), (100.0, 0.0, 90.0, 20.0)],
        },
    ]);
}

/// A reversed flow: children appear in reverse order along the main axis, without reversing
/// the main-axis alignment (Start still packs at the start, End still packs at the end).
#[test]
fn reverse_direction_rules() {
    let two = &[kid(40.0, 20.0), kid(40.0, 20.0)];
    check(&[
        Case {
            rule: "a reversed row: the children are laid out in the opposite order",
            // The children keep their layer order but swap places: the second one comes first.
            frame: Frame { reverse: true, size: (100.0, 100.0), ..FRAME },
            kids: two,
            want: (100.0, 100.0),
            rects: &[(40.0, 0.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "a reversed row keeps the gap between the children",
            // The children swap places; the gap of 10 stays between them.
            frame: Frame { reverse: true, size: (100.0, 100.0), gap: 10.0, ..FRAME },
            kids: two,
            want: (100.0, 100.0),
            rects: &[(50.0, 0.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "a reversed row keeps the main alignment: the flow still starts at the main axis' start",
            // "max" packs the children at the end: 100 - 80 = 20, so 20 and 60.
            frame: Frame { reverse: true, size: (100.0, 100.0), main: MainAlign::Max, ..FRAME },
            kids: two,
            want: (100.0, 100.0),
            rects: &[(60.0, 0.0, 40.0, 20.0), (20.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "a reversed column: the children are laid out from the end of the flow",
            frame: Frame { mode: V, reverse: true, size: (100.0, 100.0), ..FRAME },
            kids: two,
            want: (100.0, 100.0),
            rects: &[(0.0, 20.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "a reversed row with auto space between: the free space goes between the children",
            // The auto gap is not mirrored: the space still sits between the two children.
            frame: Frame { reverse: true, size: (100.0, 100.0), main: MainAlign::SpaceBetween, ..FRAME },
            kids: two,
            want: (100.0, 100.0),
            rects: &[(60.0, 0.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0)],
        },
        Case {
            rule: "a reversed row wraps like a normal one: the lines fill from the start",
            // Row 1: the first two children (reversed); row 2: the third.
            frame: Frame { reverse: true, wrap: true, size: (100.0, 100.0), gap: 10.0, cross_gap: 5.0, ..FRAME },
            kids: &[kid(40.0, 20.0), kid(40.0, 20.0), kid(40.0, 20.0)],
            want: (100.0, 100.0),
            rects: &[(50.0, 0.0, 40.0, 20.0), (0.0, 0.0, 40.0, 20.0), (0.0, 25.0, 40.0, 20.0)],
        },
    ]);
}

/// A stretched cross axis: the children that don't ask for an alignment of their own take the
/// line's cross size.
#[test]
fn stretch_cross_axis_rules() {
    check(&[
        Case {
            rule: "a stretched row: the children take the frame's height",
            // The children are packed at the start of the main axis: 0 and 40 (no gap).
            frame: Frame { size: (100.0, 60.0), cross: CrossAlign::Stretch, ..FRAME },
            kids: &[kid(40.0, 20.0), kid(40.0, 20.0)],
            want: (100.0, 60.0),
            rects: &[(0.0, 0.0, 40.0, 60.0), (40.0, 0.0, 40.0, 60.0)],
        },
        Case {
            rule: "a fixed-width stretched row hugs the tallest child's natural height",
            frame: Frame { size: (100.0, 90.0), sizing: (Fixed, Hug), cross: CrossAlign::Stretch, ..FRAME },
            kids: &[kid(40.0, 20.0), kid(40.0, 35.0)],
            want: (100.0, 35.0),
            rects: &[(0.0, 0.0, 40.0, 35.0), (40.0, 0.0, 40.0, 35.0)],
        },
        Case {
            rule: "a stretched column: the children take the frame's width",
            frame: Frame { mode: V, size: (60.0, 100.0), cross: CrossAlign::Stretch, ..FRAME },
            kids: &[kid(40.0, 20.0), kid(40.0, 20.0)],
            want: (60.0, 100.0),
            rects: &[(0.0, 0.0, 60.0, 20.0), (0.0, 20.0, 60.0, 20.0)],
        },
        Case {
            rule: "a stretched row inside the padding: the children take the inner height",
            frame: Frame { size: (100.0, 60.0), pad: [10.0; 4], cross: CrossAlign::Stretch, ..FRAME },
            kids: &[kid(40.0, 20.0)],
            want: (100.0, 60.0),
            rects: &[(10.0, 10.0, 40.0, 40.0)],
        },
        Case {
            rule: "a child with its own alignment keeps it: it is not stretched",
            frame: Frame { size: (100.0, 60.0), cross: CrossAlign::Stretch, ..FRAME },
            kids: &[Kid { sizing: (Fixed, Fixed), align_self: Some(ItemAlign::Center), ..KID }],
            want: (100.0, 60.0),
            rects: &[(0.0, 20.0, 20.0, 20.0)],
        },
    ]);
}

/// Canvas stacking changes which child is drawn (and hit) on top, never the places or the layer
/// order.
#[test]
fn canvas_stacking_rule() {
    let case = Case {
        rule: "negative gap stack",
        frame: Frame { sizing: (Hug, Hug), gap: -20.0, ..FRAME },
        kids: &[kid(50.0, 20.0), kid(50.0, 20.0)],
        want: (80.0, 20.0),
        rects: &[(0.0, 0.0, 50.0, 20.0), (30.0, 0.0, 50.0, 20.0)],
    };
    let (mut d, f, ids, m) = build(&case);
    layout_frame(&mut d, f, &m);
    let page = d.first_page().unwrap();
    let overlap = figcraft_geom::Point::new(40.0, 10.0);
    let hit = |d: &Document| d.hit_test(page, overlap, 0.0, figcraft_doc::geometry::BoolResult::default());
    // Last on top by default: the right-most child.
    assert_eq!(hit(&d), Some(ids[1]));
    assert_eq!(d.node(f).unwrap().paint_order().collect::<Vec<_>>(), ids);
    d.get_mut(f).unwrap().layout.as_mut().unwrap().reverse_z = true;
    layout_frame(&mut d, f, &m);
    // First on top: the left-most child is drawn and hit on top; places and layers stay.
    assert_eq!(hit(&d), Some(ids[0]));
    assert_eq!(d.node(f).unwrap().paint_order().collect::<Vec<_>>(), vec![ids[1], ids[0]]);
    assert_eq!(d.node(f).unwrap().children, ids);
    assert_eq!(rect_of(&d, ids[1]), (30.0, 0.0, 50.0, 20.0));
}
