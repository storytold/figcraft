//! Deeply nested layers and a resize (#210): constraints, group scaling and auto layout fill
//! follow a resize down to [`MAX_NESTING`] levels and stop there, instead of running out of stack.
//! Each case runs on a 1 MB stack, the main thread's on Windows; thousands of levels overflowed it.

use super::*;
use figcraft_doc::{AutoLayout, Constraints, Document, HConstraint, NodeKind, VConstraint};

/// Far deeper than the nesting the layout follows.
const DEEP: usize = 5000;

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(1 << 20).spawn(f).unwrap().join().unwrap();
}

fn width(d: &Document, id: NodeId) -> f64 {
    d.node(id).unwrap().width
}

/// A 100 × 100 frame holding `depth` frames, each inside the last and stretching with it; the
/// root and the chain from the top.
fn stretching_frames(depth: usize) -> (Document, NodeId, Vec<NodeId>) {
    let mut d = Document::new("deep");
    let page = d.first_page().unwrap();
    let root = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let mut chain = Vec::new();
    let mut parent = root;
    for _ in 0..depth {
        let f = d.create(parent, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.get_mut(f).unwrap().constraints = Constraints { horizontal: HConstraint::LeftRight, vertical: VConstraint::TopBottom };
        chain.push(f);
        parent = f;
    }
    (d, root, chain)
}

#[test]
fn deeply_nested_frames_follow_their_constraints_without_running_out_of_stack() {
    on_small_stack(|| {
        let (mut d, root, chain) = stretching_frames(DEEP);
        set_size(&mut d, root, 200.0, 200.0);
        apply_constraints(&mut d, root, 100.0, 100.0);
        assert_eq!(width(&d, chain[0]), 200.0);
        assert_eq!(width(&d, chain[MAX_NESTING - 2]), 200.0);
        assert_eq!(width(&d, chain[DEEP - 1]), 100.0, "past the nesting the layout follows, layers stay");
    });
}

#[test]
fn a_shallow_chain_follows_all_the_way_down() {
    let (mut d, root, chain) = stretching_frames(20);
    set_size(&mut d, root, 200.0, 200.0);
    apply_constraints(&mut d, root, 100.0, 100.0);
    assert!(chain.iter().all(|f| width(&d, *f) == 200.0));
}

#[test]
fn deeply_nested_groups_scale_without_running_out_of_stack() {
    on_small_stack(|| {
        let mut d = Document::new("deep");
        let mut parent = d.first_page().unwrap();
        let root = d.create(parent, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
        parent = root;
        let mut chain = Vec::new();
        for _ in 0..DEEP {
            parent = d.create(parent, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
            chain.push(parent);
        }
        scale_group_children(&mut d, root, 2.0, 2.0);
        assert_eq!(width(&d, chain[0]), 200.0);
    });
}

#[test]
fn deeply_nested_fill_frames_lay_out_without_running_out_of_stack() {
    on_small_stack(|| {
        let mut d = Document::new("deep");
        let page = d.first_page().unwrap();
        let root = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
        d.get_mut(root).unwrap().layout = Some(AutoLayout { padding: [0.0; 4], ..AutoLayout::default() });
        let mut parent = root;
        let mut chain = Vec::new();
        for _ in 0..DEEP {
            let f = d.create(parent, NodeKind::Frame, "F", 0.0, 0.0, 10.0, 10.0).unwrap();
            let n = d.get_mut(f).unwrap();
            n.layout = Some(AutoLayout { padding: [0.0; 4], ..AutoLayout::default() });
            n.child_layout.sizing_h = Sizing::Fill;
            chain.push(f);
            parent = f;
        }
        d.get_mut(root).unwrap().width = 300.0;
        layout_frame(&mut d, root, &NoMeasure);
        assert_eq!(width(&d, chain[0]), 300.0);
        assert_eq!(width(&d, chain[DEEP - 1]), 10.0, "past the nesting the layout follows, layers stay");
    });
}
