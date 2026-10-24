//! Wrapped lines across the flow (rows of a horizontal layout, columns of a vertical one): packed
//! where the cross alignment says, stretched when every child fills, or with the free space
//! between them. Each case's comment is the hand-computed derivation.

use super::*;
use figcraft_doc::{Document, NodeKind};

use ContentAlign::{Auto, SpaceAround as Around, SpaceBetween as Between, SpaceEvenly as Evenly};
use CrossAlign::{Center, Max, Min};

struct Case<'a> {
    rule: &'static str,
    vertical: bool,
    size: (f64, f64),
    hug_cross: bool,
    pad: [f64; 4],
    cross_gap: f64,
    cross: CrossAlign,
    content: ContentAlign,
    /// Children `(w, h)`; `fill` makes them all fill across the flow.
    kids: &'a [(f64, f64)],
    fill: bool,
    /// Each child's `(x, y, w, h)` in the frame.
    want: &'a [(f64, f64, f64, f64)],
}

const CASE: Case<'static> = Case {
    rule: "",
    vertical: false,
    size: (100.0, 100.0),
    hug_cross: false,
    pad: [0.0; 4],
    cross_gap: 5.0,
    cross: Min,
    content: Auto,
    kids: &[(40.0, 20.0), (40.0, 20.0), (40.0, 20.0)],
    fill: false,
    want: &[],
};

fn run(c: &Case<'_>) -> Vec<(f64, f64, f64, f64)> {
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, c.size.0, c.size.1).unwrap();
    let n = d.get_mut(f).unwrap();
    n.layout = Some(AutoLayout {
        mode: if c.vertical { LayoutMode::Vertical } else { LayoutMode::Horizontal },
        wrap: true,
        gap: 10.0,
        cross_gap: c.cross_gap,
        padding: c.pad,
        cross_align: c.cross,
        content_align: c.content,
        ..AutoLayout::default()
    });
    if c.hug_cross {
        *(if c.vertical { &mut n.child_layout.sizing_h } else { &mut n.child_layout.sizing_v }) = Sizing::Hug;
    }
    let kids: Vec<NodeId> = c
        .kids
        .iter()
        .map(|(w, h)| {
            let k = d.create(f, NodeKind::Rectangle, "K", 0.0, 0.0, *w, *h).unwrap();
            if c.fill {
                let cl = &mut d.get_mut(k).unwrap().child_layout;
                *(if c.vertical { &mut cl.sizing_h } else { &mut cl.sizing_v }) = Sizing::Fill;
            }
            k
        })
        .collect();
    layout_frame(&mut d, f, &NoMeasure);
    kids.iter()
        .map(|k| {
            let n = d.node(*k).unwrap();
            let t = n.transform.as_coeffs();
            (t[4], t[5], n.width, n.height)
        })
        .collect()
}

#[test]
fn wrapped_lines_across_the_flow() {
    // Unless a case says otherwise: a 100 × 100 frame, three 40 × 20 children, gap 10, so two
    // rows (40 + 10 + 40 = 90 fits, the third wraps) of 20 each, 5 apart: 45 of 100, 55 free.
    let cases = [
        Case { rule: "packed at the start", want: &[(0.0, 0.0, 40.0, 20.0), (50.0, 0.0, 40.0, 20.0), (0.0, 25.0, 40.0, 20.0)], ..CASE },
        Case {
            rule: "packed in the centre: 55 / 2 above",
            cross: Center,
            want: &[(0.0, 27.5, 40.0, 20.0), (50.0, 27.5, 40.0, 20.0), (0.0, 52.5, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "packed at the end: 55 above",
            cross: Max,
            want: &[(0.0, 55.0, 40.0, 20.0), (50.0, 55.0, 40.0, 20.0), (0.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between: the gap value gives way, 100 - 40 between two rows",
            content: Between,
            want: &[(0.0, 0.0, 40.0, 20.0), (50.0, 0.0, 40.0, 20.0), (0.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between three rows: (100 - 60) / 2 = 20 between",
            size: (50.0, 100.0),
            content: Between,
            want: &[(0.0, 0.0, 40.0, 20.0), (0.0, 40.0, 40.0, 20.0), (0.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between, items still align in their row: rows of 40 and 20, the short ones centred in row one",
            content: Between,
            cross: Center,
            kids: &[(40.0, 20.0), (40.0, 40.0), (40.0, 20.0)],
            want: &[(0.0, 10.0, 40.0, 20.0), (50.0, 0.0, 40.0, 40.0), (0.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between inside padding: inner height 80, 40 between",
            pad: [10.0, 0.0, 10.0, 0.0],
            content: Between,
            want: &[(0.0, 10.0, 40.0, 20.0), (50.0, 10.0, 40.0, 20.0), (0.0, 70.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between with no free space: rows touch and sit where the alignment says (30 - 40 = -10, half above)",
            size: (100.0, 30.0),
            content: Between,
            cross: Center,
            want: &[(0.0, -5.0, 40.0, 20.0), (50.0, -5.0, 40.0, 20.0), (0.0, 15.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between with one row: like packed, the items align in the frame",
            size: (200.0, 100.0),
            content: Between,
            cross: Max,
            want: &[(0.0, 80.0, 40.0, 20.0), (50.0, 80.0, 40.0, 20.0), (100.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space between in a frame hugging its height: nothing to share, rows stay 5 apart",
            hug_cross: true,
            content: Between,
            want: &[(0.0, 0.0, 40.0, 20.0), (50.0, 0.0, 40.0, 20.0), (0.0, 25.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "rows of nothing but fill children stretch: (100 - 5) / 2 = 47.5 each",
            fill: true,
            want: &[(0.0, 0.0, 40.0, 47.5), (50.0, 0.0, 40.0, 47.5), (0.0, 52.5, 40.0, 47.5)],
            ..CASE
        },
        Case {
            rule: "vertical wrap, columns packed in the centre: two columns of 20, 5 apart, 55 / 2 before",
            vertical: true,
            cross: Center,
            kids: &[(20.0, 40.0), (20.0, 40.0), (20.0, 40.0)],
            want: &[(27.5, 0.0, 20.0, 40.0), (27.5, 50.0, 20.0, 40.0), (52.5, 0.0, 20.0, 40.0)],
            ..CASE
        },
        Case {
            rule: "vertical wrap, space between columns: 100 - 40 between",
            vertical: true,
            content: Between,
            kids: &[(20.0, 40.0), (20.0, 40.0), (20.0, 40.0)],
            want: &[(0.0, 0.0, 20.0, 40.0), (0.0, 50.0, 20.0, 40.0), (80.0, 0.0, 20.0, 40.0)],
            ..CASE
        },
        Case {
            // 100 - 40 (two rows of 20) = 60 free: around gives each line 30, half of it at
            // each edge (15 above the first row, 15 below the last).
            rule: "space around: the free space is shared by the lines, half of it at each edge",
            cross_gap: 0.0,
            content: Around,
            want: &[(0.0, 15.0, 40.0, 20.0), (50.0, 15.0, 40.0, 20.0), (0.0, 65.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            // 60 free over three gaps (two between the lines and one at each edge): 20 each.
            rule: "space evenly: the free space is shared by the lines and the edges alike",
            cross_gap: 0.0,
            content: Evenly,
            want: &[(0.0, 20.0, 40.0, 20.0), (50.0, 20.0, 40.0, 20.0), (0.0, 60.0, 40.0, 20.0)],
            ..CASE
        },
        Case {
            rule: "space around, vertical wrap: the columns share the width, half of it at each edge",
            vertical: true,
            cross_gap: 0.0,
            content: Around,
            kids: &[(20.0, 40.0), (20.0, 40.0), (20.0, 40.0)],
            want: &[(15.0, 0.0, 20.0, 40.0), (15.0, 50.0, 20.0, 40.0), (65.0, 0.0, 20.0, 40.0)],
            ..CASE
        },
        Case {
            rule: "space evenly, vertical wrap: the columns and the edges share the width alike",
            vertical: true,
            cross_gap: 0.0,
            content: Evenly,
            kids: &[(20.0, 40.0), (20.0, 40.0), (20.0, 40.0)],
            want: &[(20.0, 0.0, 20.0, 40.0), (20.0, 50.0, 20.0, 40.0), (60.0, 0.0, 20.0, 40.0)],
            ..CASE
        },
        Case {
            rule: "space around with one row: the row keeps its place, the free space is all at the edge",
            size: (200.0, 100.0),
            cross_gap: 0.0,
            content: Around,
            cross: Max,
            want: &[(0.0, 80.0, 40.0, 20.0), (50.0, 80.0, 40.0, 20.0), (100.0, 80.0, 40.0, 20.0)],
            ..CASE
        },
    ];
    for c in &cases {
        let got = run(c);
        assert_eq!(got.len(), c.want.len(), "{}", c.rule);
        for (i, (g, w)) in got.iter().zip(c.want).enumerate() {
            let close = [(g.0, w.0), (g.1, w.1), (g.2, w.2), (g.3, w.3)].iter().all(|(a, b)| (a - b).abs() < 1e-6);
            assert!(close, "{}: child {i} is {g:?}, want {w:?}", c.rule);
        }
    }
}

#[test]
fn a_wrapping_frame_without_children_lays_out() {
    assert!(run(&Case { kids: &[], content: Between, ..CASE }).is_empty());
}

#[test]
fn non_finite_and_tiny_frames_place_finitely() {
    for (w, h, gap) in [(0.0, 0.0, 5.0), (100.0, 1e-9, f64::NAN), (1e12, 1e12, f64::INFINITY), (50.0, 100.0, -1e9)] {
        let got = run(&Case { size: (w, h), cross_gap: gap, content: Between, cross: Center, ..CASE });
        assert!(got.iter().all(|r| r.0.is_finite() && r.1.is_finite()), "{w} × {h}, gap {gap}: {got:?}");
    }
}
