//! Paint bounds worked out once per render: the bounds kept for a layer are the ones worked out
//! without keeping any, and the largest background blur follows the document's edits.

use super::*;
use figcraft_doc::{Effect, EffectKind};

#[test]
fn kept_paint_bounds_are_the_bounds_worked_out_afresh() {
    // Unclipped frames nested past the 64-level walk, a rectangle at the bottom: bounds asked for
    // deep first and then from the top, and the other way round, are those worked out alone.
    let mut d = Document::new("deep");
    let p = d.first_page().unwrap();
    let mut chain = Vec::new();
    let mut parent = p;
    for k in 0..80 {
        let f = d.create(parent, NodeKind::Frame, &format!("F{k}"), 1.0, 1.0, 10.0, 10.0).unwrap();
        d.node_mut(f).unwrap().clip = false;
        chain.push(f);
        parent = f;
    }
    d.create(parent, NodeKind::Rectangle, "R", 500.0, 0.0, 10.0, 10.0).unwrap();
    for order in [chain.iter().rev().copied().collect::<Vec<_>>(), chain.clone()] {
        let memo = draw::Bounds::default();
        for id in &order {
            let n = d.node(*id).unwrap();
            for depth in [0, 20] {
                let world = d.world_transform(*id);
                let kept = draw::bounds_within(&d, n, world, depth, None, Some(&memo)).0;
                assert_eq!(kept, draw::local_paint_bounds(&d, n, world, depth, None), "{} at depth {depth}", n.name);
            }
        }
    }
}

#[test]
fn the_largest_background_blur_follows_edits() {
    let mut d = Document::new("blur");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    assert_eq!(max_backdrop_blur(&d), 0.0);
    let mut blur = Effect::background_blur();
    if let EffectKind::BackgroundBlur(b) = &mut blur.kind {
        b.radius = 12.0;
    }
    d.node_mut(r).unwrap().effects.push(blur);
    assert_eq!(max_backdrop_blur(&d), 12.0);
    // A copy of the document keeps it, and an edit of the copy changes only the copy's.
    let mut copy = d.clone();
    if let Some(EffectKind::BackgroundBlur(b)) = copy.node_mut(r).unwrap().effects.first_mut().map(|e| &mut e.kind) {
        b.radius = 30.0;
    }
    assert_eq!(max_backdrop_blur(&copy), 30.0);
    assert_eq!(max_backdrop_blur(&d), 12.0);
    copy.node_mut(r).unwrap().effects.clear();
    assert_eq!(max_backdrop_blur(&copy), 0.0);
    // A deleted layer's blur no longer counts.
    d.remove(r).unwrap();
    assert_eq!(max_backdrop_blur(&d), 0.0);
}

#[test]
fn kept_splat_answers_are_the_answers_worked_out_afresh() {
    use figcraft_color::{ImageScaleMode, Paint};
    // Groups nested past the 64-level walk, with an image-filled rectangle at the bottom of one
    // branch and a plain one at the bottom of the other: answers asked for deep-first and
    // top-first, at depths 0 and 20, are those worked out alone.
    let mut d = Document::new("deep");
    let p = d.first_page().unwrap();
    let mut chain = Vec::new();
    for image in [false, true] {
        let mut parent = p;
        for k in 0..80 {
            let g = d.create(parent, NodeKind::Frame, &format!("{image} {k}"), 0.0, 0.0, 10.0, 10.0).unwrap();
            d.node_mut(g).unwrap().clip = false;
            chain.push(g);
            parent = g;
        }
        let r = d.create(parent, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
        if image {
            d.node_mut(r).unwrap().fills = vec![Paint::image("missing", ImageScaleMode::Fill)];
        }
    }
    for order in [chain.iter().rev().copied().collect::<Vec<_>>(), chain.clone()] {
        let memo = draw::Bounds::default();
        for id in &order {
            let n = d.node(*id).unwrap();
            for depth in [0, 20] {
                let kept = lod::splattable_within(&d, n, depth, Some(&memo)).0;
                assert_eq!(kept, lod::splattable(&d, n, depth), "{} at depth {depth}", n.name);
            }
        }
    }
}
