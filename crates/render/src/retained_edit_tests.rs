//! Retained renders after edits inside transformed and clipping containers: every render must match
//! a full render of the same view (the engine's random walk drives whole documents through it).

use std::sync::Arc;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Blur, Document, Effect, EffectKind, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::retained_tests::assert_same;
use crate::{Options, Retained, View, Work, render_view};

fn check(r: &mut Retained, doc: &Arc<Document>, page: NodeId, v: &View, what: &str) {
    let (shown, pix, work) = r.render(doc, page, v, &Options::default()).unwrap();
    let pix = pix.clone();
    let full = render_view(doc, page, &shown, &Options::default()).unwrap();
    assert_same(&pix, &full, &format!("{what} ({work:?})"));
}

#[test]
fn moving_a_child_of_a_rotated_frame() {
    let mut d = Document::new("rot");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 100.0, 100.0, 300.0, 200.0).unwrap();
    d.get_mut(f).unwrap().transform = figcraft_geom::set_rotation_about_center(d.node(f).unwrap().transform, 300.0, 200.0, 25.0);
    let c = d.create(f, NodeKind::Rectangle, "C", 40.0, 40.0, 80.0, 60.0).unwrap();
    d.get_mut(c).unwrap().fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
    let doc = Arc::new(d);
    let v = View { origin: (0.0, 0.0), scale: 1.0, width: 500, height: 450 };
    let mut r = Retained::default();
    check(&mut r, &doc, page, &v, "first");
    let mut d2 = (*doc).clone();
    let pw = d2.world_transform(f);
    let n = d2.get_mut(c).unwrap();
    n.transform = pw.inverse() * Affine::translate((150.0, 60.0)) * pw * n.transform;
    check(&mut r, &Arc::new(d2), page, &v, "moved");
}

/// A clipping frame whose right edge lands exactly on a pixel boundary at 40 %: float noise in
/// one render's origin and not another's put the edge a hair past the boundary, and the clip,
/// rounded outwards, let a whole column of the child through.
#[test]
fn a_clip_edge_on_a_pixel_boundary_clips_the_same_in_every_render() {
    let mut d = Document::new("clip");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 212.0, 198.0, 220.0, 70.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::from_hex("FFE0B0").unwrap())];
    let c = d.create(f, NodeKind::Rectangle, "C", 58.0, 10.0, 40.0, 40.0).unwrap();
    d.get_mut(c).unwrap().fills = vec![Paint::solid(Color::from_hex("3070E0").unwrap())];
    d.get_mut(c).unwrap().effects.push(Effect { kind: EffectKind::LayerBlur(Blur { radius: 3.0, ..Blur::default() }), visible: true });
    let doc = Arc::new(d);
    // The frame's new right edge (x 280) is device pixel 172 exactly.
    let v = View { origin: (-150.0, 132.8), scale: 0.4, width: 320, height: 240 };
    for lod in [false, true] {
        let opts = Options { lod, ..Default::default() };
        let mut r = Retained::default();
        r.render(&doc, page, &v, &opts).unwrap();
        let mut smaller = (*doc).clone();
        smaller.get_mut(f).unwrap().width = 68.0;
        smaller.get_mut(f).unwrap().height = 54.0;
        let smaller = Arc::new(smaller);
        let (shown, pix, work) = r.render(&smaller, page, &v, &opts).unwrap();
        assert!(matches!(work, Work::Partial { .. }), "{work:?}");
        let pix = pix.clone();
        assert_same(&pix, &render_view(&smaller, page, &shown, &opts).unwrap(), &format!("lod {lod}"));
    }
}
/// A closed zigzag of `2 * teeth + 2` points, 240 × 120 units at (20, 20): a ridge.
fn ridge(d: &mut Document, page: NodeId, teeth: usize) -> NodeId {
    let mut pts = vec![figcraft_geom::Point::new(0.0, 120.0)];
    for i in 0..=teeth * 2 {
        pts.push(figcraft_geom::Point::new(240.0 * i as f64 / (teeth * 2) as f64, if i % 2 == 0 { 60.0 } else { 10.0 }));
    }
    pts.push(figcraft_geom::Point::new(240.0, 120.0));
    let network = figcraft_doc::VectorNetwork::from_points(&pts, true);
    let id = d.create(page, NodeKind::Vector { network }, "Ridge", 20.0, 20.0, 240.0, 120.0).unwrap();
    d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex("7FAE9C").unwrap())];
    id
}

fn move_point(d: &mut Document, id: NodeId, point: usize, by: (f64, f64)) {
    if let NodeKind::Vector { network } = &mut d.get_mut(id).unwrap().kind {
        network.vertices[point].x += by.0;
        network.vertices[point].y += by.1;
    }
}

/// A point of a vector dragged while the vector fills the view (zoomed to it, as the vector
/// editor does): the shape looks different only around the point's segments, and only that is
/// drawn again. (Its whole box was, and past 60 % of the view that made every frame of the
/// drag a full render.)
#[test]
fn a_moved_point_redraws_the_vector_around_it_only() {
    type Style = fn(&mut figcraft_doc::Node);
    let styles: [(&str, Style); 4] = [
        ("filled", |_| {}),
        ("stroked, mitred", |n| {
            n.strokes = vec![Paint::solid(Color::from_hex("203040").unwrap())];
            n.stroke.weight = 6.0;
            n.stroke.miter_limit = 4.0;
        }),
        ("rounded points", |n| {
            if let NodeKind::Vector { network } = &mut n.kind {
                network.vertices.iter_mut().for_each(|v| v.radius = 9.0);
            }
        }),
        ("stroked only", |n| {
            n.fills.clear();
            n.strokes = vec![Paint::solid(Color::from_hex("203040").unwrap())];
            n.stroke.weight = 3.0;
        }),
    ];
    for (what, style) in styles {
        let mut d = Document::new("ridge");
        let page = d.first_page().unwrap();
        let id = ridge(&mut d, page, 6);
        style(d.get_mut(id).unwrap());
        let doc = Arc::new(d);
        // The ridge is 720 × 360 of the view's 780 × 420 pixels.
        let v = View { origin: (10.0, 10.0), scale: 3.0, width: 780, height: 420 };
        let mut r = Retained::default();
        check(&mut r, &doc, page, &v, what);
        let mut d2 = (*doc).clone();
        move_point(&mut d2, id, 6, (4.0, -7.0));
        let d2 = Arc::new(d2);
        let (_, _, work) = r.render(&d2, page, &v, &Options::default()).unwrap();
        let Work::Partial { pixels } = work else { panic!("{what}: a full render ({work:?})") };
        // A tooth of six each way from the point, and what the stroke adds (36 pixels each way
        // for the mitred one); the ridge's box is 259,200 pixels.
        assert!(pixels < 780 * 420 * 2 / 5, "{what}: {pixels} pixels drawn again");
        check(&mut r, &d2, page, &v, what);
        // And back: the same again.
        let (_, _, work) = r.render(&doc, page, &v, &Options::default()).unwrap();
        assert!(matches!(work, Work::Partial { .. }), "{what}: {work:?}");
        check(&mut r, &doc, page, &v, what);
    }
}

/// What depends on the whole outline is drawn again whole: a gradient laid out in the vector's
/// box, dashes that run on from the moved point, an open path's closing line.
#[test]
fn a_moved_point_redraws_a_vector_whole_when_its_look_depends_on_all_of_it() {
    type Style = fn(&mut figcraft_doc::Node);
    let styles: [(&str, Style); 3] = [
        ("dashed", |n| {
            n.strokes = vec![Paint::solid(Color::from_hex("203040").unwrap())];
            n.stroke.weight = 3.0;
            n.stroke.dashes = vec![8.0, 5.0];
        }),
        ("a shadow", |n| n.effects.push(Effect::inner_shadow())),
        ("an open path", |n| {
            if let NodeKind::Vector { network } = &mut n.kind {
                network.segments.pop();
            }
        }),
    ];
    for (what, style) in styles {
        let mut d = Document::new("ridge");
        let page = d.first_page().unwrap();
        let id = ridge(&mut d, page, 6);
        style(d.get_mut(id).unwrap());
        let doc = Arc::new(d);
        let v = View { origin: (10.0, 10.0), scale: 3.0, width: 780, height: 420 };
        let mut r = Retained::default();
        check(&mut r, &doc, page, &v, what);
        let mut d2 = (*doc).clone();
        move_point(&mut d2, id, 6, (4.0, -7.0));
        let d2 = Arc::new(d2);
        assert_eq!(r.render(&d2, page, &v, &Options::default()).unwrap().2, Work::Full, "{what}");
        check(&mut r, &d2, page, &v, what);
    }
}

/// A vector's strokes are as wide on the page as its transform makes them. Scaled up with its
/// points scaled down to match, its paths stay where they were and every stroke doubles: with a
/// point moved too, all of it is drawn again, not the area around that point.
#[test]
fn a_moved_point_with_a_scaled_vector_redraws_all_of_it() {
    let mut d = Document::new("ridge");
    let page = d.first_page().unwrap();
    let id = ridge(&mut d, page, 6);
    let n = d.get_mut(id).unwrap();
    n.strokes = vec![Paint::solid(Color::from_hex("203040").unwrap())];
    n.stroke.weight = 3.0;
    let doc = Arc::new(d);
    let v = View { origin: (10.0, 10.0), scale: 3.0, width: 780, height: 420 };
    let mut r = Retained::default();
    check(&mut r, &doc, page, &v, "first");
    let mut d2 = (*doc).clone();
    let n = d2.get_mut(id).unwrap();
    n.transform *= Affine::scale(2.0);
    (n.width, n.height) = (n.width / 2.0, n.height / 2.0);
    if let NodeKind::Vector { network } = &mut n.kind {
        for p in &mut network.vertices {
            (p.x, p.y) = (p.x / 2.0, p.y / 2.0);
        }
    }
    move_point(&mut d2, id, 6, (2.0, -3.5));
    check(&mut r, &Arc::new(d2), page, &v, "scaled, a point moved");
}
