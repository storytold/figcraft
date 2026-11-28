//! A boolean group past the path operations' caps draws without its result (with a warning, once)
//! and costs nothing the next time it is drawn; other groups are unchanged.

use figcraft_color::{Color, Paint};
use figcraft_doc::{BoolOp, Document, NodeId, NodeKind, VectorNetwork};
use figcraft_geom::kurbo::{BezPath, Rect, Shape};

use crate::boolean::boolean_steps;
use crate::{Options, View, node_outline, render_view};

/// `n` long thin bars, across or down: 750 of each cross 2.25 million times, past the caps.
fn bars(n: usize, across: bool) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..n {
        let (at, len) = (i as f64 * 4.0, n as f64 * 4.0);
        let r = if across { Rect::new(0.0, at, len, at + 2.0) } else { Rect::new(at, 0.0, at + 2.0, len) };
        p.extend(r.path_elements(0.1));
    }
    p
}

/// A black union of the bars across and down, on a page of its own.
fn union_of_bars(n: usize) -> (Document, NodeId, NodeId) {
    let mut d = Document::new("t");
    let page = d.first_page().unwrap();
    let b = d.create(page, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 0.0, 0.0).unwrap();
    d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    for across in [true, false] {
        let network = VectorNetwork::from_path(&bars(n, across));
        let len = n as f64 * 4.0;
        d.create(b, NodeKind::Vector { network }, "Bars", 0.0, 0.0, len, len).unwrap();
    }
    d.fit_to_children(b).unwrap();
    (d, page, b)
}

/// Whether the page shows anything dark at all.
fn painted(d: &Document, page: NodeId, scale: f64) -> bool {
    let view = View { origin: (0.0, 0.0), scale, width: 100, height: 100 };
    let pix = render_view(d, page, &view, &Options::default()).unwrap();
    pix.data().chunks_exact(4).any(|px| px[3] > 0 && px[0] < 40)
}

#[test]
fn a_boolean_past_the_caps_draws_without_its_result() {
    let (d, page, b) = union_of_bars(750);
    let group = d.node(b).unwrap();
    assert!(node_outline(&d, group).is_none(), "refused: the group has no outline");
    let before = boolean_steps();
    assert!(node_outline(&d, group).is_none());
    assert_eq!(boolean_steps(), before, "the refusal is remembered");
    assert!(!painted(&d, page, 100.0 / 3000.0), "nothing of the group is drawn");
    // A small one of the same kind is combined and drawn as before.
    let (d, page, b) = union_of_bars(10);
    assert!(node_outline(&d, d.node(b).unwrap()).is_some_and(|(p, _)| p.area().abs() > 100.0));
    assert!(painted(&d, page, 100.0 / 40.0));
}
