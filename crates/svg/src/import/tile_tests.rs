//! Pattern tiles: drawn once per pattern and scale in an import, their layers counted in the
//! import's layer budget.

use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, NodeKind};
use figcraft_geom::Affine;

use super::{Ctx, MAX_NODES, children, import_svg_with_notes};

fn svg(defs: &str, body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs>{defs}</defs>{body}</svg>"##)
}

/// What importing `data` with `spent` layers of the budget already used leaves behind: the tiles
/// drawn, the layers counted and the notes.
fn walk(data: &str, spent: usize) -> (usize, usize, Vec<String>) {
    let tree = usvg::Tree::from_data(data.as_bytes(), &usvg::Options::default()).unwrap();
    let mut d = Document::new("tiles");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "f", 0.0, 0.0, 100.0, 100.0).unwrap();
    let mut cx = Ctx::new(&mut d);
    cx.count = spent;
    children(&mut cx, tree.root(), frame, Affine::IDENTITY, 0);
    (cx.tiles.len(), cx.count, cx.notes.into_iter().collect())
}

const DOTS: &str = r##"<pattern id="p" patternUnits="userSpaceOnUse" width="10" height="10"><circle cx="5" cy="5" r="3"/><rect width="2" height="2"/><rect x="8" width="2" height="2"/></pattern>"##;

#[test]
fn a_pattern_used_a_thousand_times_is_drawn_once() {
    let uses: String = (0..1000).map(|i| format!(r#"<rect x="{}" y="{}" width="10" height="10" fill="url(#p)"/>"#, i % 10 * 10, i / 10)).collect();
    let (tiles, count, _) = walk(&svg(DOTS, &uses), 0);
    assert_eq!(tiles, 1);
    // A thousand rectangles and the tile's three layers, once.
    assert_eq!(count, 1003);
    // Another scale is another tile.
    let scaled = format!(r#"{uses}<rect transform="scale(3)" width="10" height="10" fill="url(#p)"/>"#);
    assert_eq!(walk(&svg(DOTS, &scaled), 0).0, 2);
}

/// The image fills of `data`'s layers, and the images they show.
fn fills(data: &str) -> Vec<(Paint, Vec<u8>)> {
    let mut d = Document::new("fills");
    let page = d.first_page().unwrap();
    let f = import_svg_with_notes(&mut d, data.as_bytes(), page, 0.0, 0.0, "x").unwrap().frame;
    let layers: Vec<_> = d.subtree(f).into_iter().filter_map(|id| d.node(id)).filter_map(|n| n.fills.first().cloned()).collect();
    layers
        .into_iter()
        .map(|p| {
            let PaintKind::Image(ip) = &p.kind else { panic!("not an image fill: {p:?}") };
            let bytes = d.images.get(&ip.image).unwrap().bytes.to_vec();
            (p, bytes)
        })
        .collect()
}

#[test]
fn a_reused_tile_is_the_tile_a_single_use_draws() {
    let once = fills(&svg(DOTS, r#"<rect width="40" height="40" fill="url(#p)"/>"#));
    let twice = fills(&svg(DOTS, r#"<rect width="40" height="40" fill="url(#p)"/><rect x="50" width="40" height="40" fill="url(#p)"/>"#));
    assert_eq!(once.len(), 1);
    assert_eq!(twice.len(), 2);
    assert!(twice.iter().all(|f| *f == once[0]), "both uses show the single use's tile");
}

#[test]
fn a_nested_pattern_bomb_is_refused_within_the_budget() {
    // Pattern `a` holds 50 layers, each filled with pattern `b` at its own scale (a tile each),
    // and `b` holds 50 layers: 2,601 layers from one rectangle (each of `a`'s layers comes with
    // the group carrying its transform).
    let b: String = (0..50).map(|_| r#"<rect width="1" height="1"/>"#).collect();
    let a: String = (1..=50).map(|i| format!(r#"<rect transform="scale(1.{i:02})" width="1" height="1" fill="url(#b)"/>"#)).collect();
    let defs = format!(
        r#"<pattern id="b" patternUnits="userSpaceOnUse" width="4" height="4">{b}</pattern><pattern id="a" patternUnits="userSpaceOnUse" width="4" height="4">{a}</pattern>"#
    );
    let data = svg(&defs, r#"<rect width="40" height="40" fill="url(#a)"/>"#);
    // Every tile's layers count.
    assert_eq!(walk(&data, 0).1, 2601);
    // With 500 layers left the tiles stop there.
    let (_, count, notes) = walk(&data, MAX_NODES - 500);
    assert!(count <= MAX_NODES, "{count}");
    assert!(notes.iter().any(|n| n.contains("the rest was left out")), "{notes:?}");
}
