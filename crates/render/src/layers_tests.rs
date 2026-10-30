//! Kept layers between canvas frames: a frame that reuses layers draws what a fresh render does.

use std::sync::Arc;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Effect, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::{Options, Retained, View};

/// The kept layers are one cache for the whole process: these tests take turns.
fn turn() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    super::layers::clear();
    guard
}

/// Two shadowed cards, a blurred one and a half-transparent group: (document, page, moved card).
fn scene() -> (Document, NodeId, NodeId) {
    let mut d = Document::new("kept layers");
    let p = d.first_page().unwrap();
    let mut cards = Vec::new();
    for i in 0..3u32 {
        let id = d.create(p, NodeKind::Rectangle, "Card", 20.0 + 100.0 * f64::from(i), 20.0, 80.0, 80.0).unwrap();
        let n = d.get_mut(id).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
        n.effects = vec![Effect::drop_shadow()];
        cards.push(id);
    }
    let blurred = d.create(p, NodeKind::Rectangle, "Blurred", 20.0, 140.0, 120.0, 80.0).unwrap();
    {
        let n = d.get_mut(blurred).unwrap();
        n.fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap())];
        n.effects = vec![Effect::layer_blur()];
    }
    // A group with its own opacity is an isolated layer too.
    let group = d.create(p, NodeKind::Group, "Group", 0.0, 0.0, 0.0, 0.0).unwrap();
    let inner = d.create(group, NodeKind::Rectangle, "Inner", 180.0, 140.0, 90.0, 90.0).unwrap();
    d.get_mut(inner).unwrap().fills = vec![Paint::solid(Color::from_hex("20C060").unwrap())];
    d.get_mut(group).unwrap().opacity = 0.5;
    (d, p, cards[1])
}

fn view() -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 }
}

/// The image of `doc` drawn fresh, with every layer drawn from the document.
fn fresh(doc: &Arc<Document>, p: NodeId) -> tiny_skia::Pixmap {
    let mut retained = Retained::default();
    let (_, pix, work) = retained.render(doc, p, &view(), &Options::default()).unwrap();
    assert_eq!(work, crate::Work::Full);
    pix.clone()
}

/// A frame of `doc` that may reuse layers kept by the frames before it (`doc_id`).
fn kept(rt: &mut Retained, doc: &Arc<Document>, p: NodeId, doc_id: u64) -> tiny_skia::Pixmap {
    let opts = Options { doc_id, ..Options::default() };
    rt.render(doc, p, &view(), &opts).unwrap().1.clone()
}

#[test]
fn a_kept_layer_draws_what_a_fresh_one_does() {
    let _turn = turn();
    let (d, p, card) = scene();
    let mut rt = Retained::default();
    let first = Arc::new(d.clone());
    let one = kept(&mut rt, &first, p, 7);
    assert_eq!(one.data(), fresh(&first, p).data(), "the first frame, layers and all");

    // The second frame keeps the layers it drew before, the third reuses them.
    let two = kept(&mut rt, &first, p, 7);
    assert_eq!(two.data(), fresh(&first, p).data(), "the second frame keeps the layers whole");

    // Move a card: the layers that didn't change are kept, the moved card's is drawn again.
    let mut moved = d;
    moved.get_mut(card).unwrap().transform = Affine::translate((10.0, 6.0));
    let second = Arc::new(moved);
    let three = kept(&mut rt, &second, p, 7);
    assert_eq!(three.data(), fresh(&second, p).data(), "the third frame reuses the kept layers");
    assert_ne!(one.data(), three.data(), "the frame did change");
    assert_ne!(two.data(), three.data(), "the card moved");
}

#[test]
fn a_dragged_layer_is_not_kept() {
    let _turn = turn();
    let (mut d, p, card) = scene();
    let mut rt = Retained::default();
    let mut bytes = 0;
    for i in 1..=5u32 {
        d.get_mut(card).unwrap().transform = Affine::translate((4.0 * f64::from(i), 2.0));
        let doc = Arc::new(d.clone());
        let frame = kept(&mut rt, &doc, p, 21);
        assert_eq!(frame.data(), fresh(&doc, p).data(), "frame {i} draws what a fresh render does");
        if i == 3 {
            bytes = super::layers::kept_bytes();
        }
    }
    assert_eq!(super::layers::kept_bytes(), bytes, "a layer that moves every frame is never kept");
}

#[test]
fn a_changed_layer_is_drawn_again() {
    let _turn = turn();
    let (mut d, p, card) = scene();
    let mut rt = Retained::default();
    let doc = Arc::new(d.clone());
    // Twice: the second frame keeps the layers.
    let _ = kept(&mut rt, &doc, p, 11);
    let _ = kept(&mut rt, &doc, p, 11);
    // The same card, a new colour.
    d.get_mut(card).unwrap().fills = vec![Paint::solid(Color::from_hex("F0C830").unwrap())];
    let changed = Arc::new(d);
    let now = kept(&mut rt, &changed, p, 11);
    assert_eq!(now.data(), fresh(&changed, p).data(), "the changed card is drawn again");
}

fn view_at(origin: f64) -> View {
    View { origin: (origin, origin), scale: 1.0, width: 400, height: 300 }
}

/// A full render of `doc` through `v`: with `doc_id` it may reuse layers kept by earlier renders,
/// with `0` it draws every layer from the document.
fn drawn(doc: &Arc<Document>, p: NodeId, v: &View, doc_id: u64) -> tiny_skia::Pixmap {
    crate::render_view_single(doc.as_ref(), p, v, &Options { doc_id, ..Options::default() }).unwrap()
}

/// A layer too big to be kept whole is never kept: its kept pixels would be a crop of it, and the
/// rest would be missing once the view moves over it.
#[test]
fn a_layer_too_big_to_keep_is_not_kept() {
    let _turn = turn();
    let mut d = Document::new("big layer");
    let p = d.first_page().unwrap();
    // 3000 × 3000 at 100% is past the kept layer's pixel budget, so a render only ever draws a
    // crop of it.
    let big = d.create(p, NodeKind::Rectangle, "Big", 0.0, 0.0, 3000.0, 3000.0).unwrap();
    let n = d.get_mut(big).unwrap();
    n.fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
    n.opacity = 0.5;
    let doc = Arc::new(d);
    let a = view_at(0.0);
    let _ = drawn(&doc, p, &a, 41);
    let second = drawn(&doc, p, &a, 41);
    assert_eq!(second.data(), drawn(&doc, p, &a, 0).data(), "the second frame draws the layer as a fresh render does");
    // Panned past the crop the earlier frames were cut to.
    let b = view_at(800.0);
    let panned = drawn(&doc, p, &b, 41);
    assert_eq!(panned.data(), drawn(&doc, p, &b, 0).data(), "the panned frame draws the layer as a fresh render does");
}

/// A layer with a descendant drawn as an outline (or left out) is not kept: those are per-node
/// states, and the layer's pixels would keep them from the frame they were made in.
#[test]
fn a_layer_with_an_outlined_descendant_is_not_kept() {
    let _turn = turn();
    let mut d = Document::new("outlined");
    let p = d.first_page().unwrap();
    let group = d.create(p, NodeKind::Group, "Group", 20.0, 20.0, 120.0, 120.0).unwrap();
    let inner = d.create(group, NodeKind::Rectangle, "Inner", 0.0, 0.0, 60.0, 60.0).unwrap();
    d.get_mut(inner).unwrap().fills = vec![Paint::solid(Color::from_hex("20C060").unwrap())];
    d.get_mut(group).unwrap().opacity = 0.5;
    let doc = Arc::new(d);
    let twice = |outlined: Vec<NodeId>| {
        for _ in 0..2 {
            let opts = Options { doc_id: 51, outlined: outlined.clone(), ..Options::default() };
            let _ = crate::render_view_single(doc.as_ref(), p, &view(), &opts).unwrap();
        }
        super::layers::kept_bytes()
    };
    assert_eq!(twice(vec![inner]), 0, "an outlined descendant keeps the layer from being kept");
    assert!(twice(Vec::new()) > 0, "the same layer is kept without it");
}

/// A layer with a pattern fill is not kept: its pixels are another node's, outside the subtree the
/// layer's key hashes.
#[test]
fn a_layer_with_a_pattern_fill_is_not_kept() {
    let _turn = turn();
    let mut d = Document::new("pattern");
    let p = d.first_page().unwrap();
    let source = d.create(p, NodeKind::Rectangle, "Source", 0.0, 0.0, 20.0, 20.0).unwrap();
    d.get_mut(source).unwrap().fills = vec![Paint::solid(Color::from_hex("20C060").unwrap())];
    let group = d.create(p, NodeKind::Group, "Group", 40.0, 40.0, 120.0, 120.0).unwrap();
    let filled = d.create(group, NodeKind::Rectangle, "Filled", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(filled).unwrap().fills = vec![Paint::pattern(source.0)];
    d.get_mut(group).unwrap().opacity = 0.5;
    let doc = Arc::new(d);
    for _ in 0..2 {
        let _ = drawn(&doc, p, &view(), 53);
    }
    assert_eq!(super::layers::kept_bytes(), 0, "a patterned layer is drawn every frame");
}

/// A text run painted with a pattern keeps its layer from being kept too: the run's fills come
/// from the layout's styles, not the node's own `fills`, which the cache check also reads.
#[test]
fn a_text_run_with_a_pattern_fill_is_not_kept() {
    let _turn = turn();
    let mut d = Document::new("patterned text");
    let p = d.first_page().unwrap();
    let source = d.create(p, NodeKind::Rectangle, "Source", 0.0, 0.0, 20.0, 20.0).unwrap();
    d.get_mut(source).unwrap().fills = vec![Paint::solid(Color::from_hex("20C060").unwrap())];
    let mut td = figcraft_doc::TextData::new("patterned");
    td.style.font_size = 20.0;
    td.runs = vec![figcraft_doc::StyleRun {
        start: 0,
        end: 9,
        style: figcraft_doc::TextStyle { fills: Some(vec![Paint::pattern(source.0)]), ..td.style.clone() },
    }];
    let text = d.create(p, NodeKind::Text(td), "Text", 40.0, 40.0, 120.0, 40.0).unwrap();
    d.get_mut(text).unwrap().opacity = 0.5;
    let doc = Arc::new(d);
    for _ in 0..2 {
        let _ = drawn(&doc, p, &view(), 55);
    }
    assert_eq!(super::layers::kept_bytes(), 0, "a text layer with a patterned run is drawn every frame");
}

/// An edited layer's older version is dropped with the entry it was kept in: its key never hits
/// again, and the node it holds is memory the pixel budget doesn't count.
#[test]
fn a_kept_layer_drops_the_nodes_older_versions() {
    let _turn = turn();
    let mut d = Document::new("edited");
    let p = d.first_page().unwrap();
    let card = d.create(p, NodeKind::Rectangle, "Card", 20.0, 20.0, 120.0, 80.0).unwrap();
    let n = d.get_mut(card).unwrap();
    n.fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
    n.effects = vec![Effect::layer_blur()];
    let doc = Arc::new(d.clone());
    let _ = drawn(&doc, p, &view(), 61);
    let _ = drawn(&doc, p, &view(), 61);
    let before = super::layers::kept_bytes();
    assert!(before > 0, "the blurred card is kept");
    // A new colour: the new version is kept whole, and the old one goes with its entry.
    d.get_mut(card).unwrap().fills = vec![Paint::solid(Color::from_hex("2040E0").unwrap())];
    let edited = Arc::new(d);
    let _ = drawn(&edited, p, &view(), 61);
    let _ = drawn(&edited, p, &view(), 61);
    assert_eq!(super::layers::kept_bytes(), before, "the older version's entry is dropped");
}

/// An image the store gains after a frame was drawn (a collaborative image entry, an import
/// finishing) draws the layer that names it again, and the kept frame too: no node changed, but
/// the fill shows something else now.
#[test]
fn an_image_that_arrives_draws_the_layer_again() {
    let _turn = turn();
    let (mut d, p, card) = scene();
    let hash = "a".repeat(64);
    d.get_mut(card).unwrap().fills = vec![Paint::image(hash.clone(), figcraft_color::ImageScaleMode::Fill)];
    let before = Arc::new(d.clone());
    // Two frames settle the card's layer, which the third reuses.
    let _ = drawn(&before, p, &view(), 31);
    let _ = drawn(&before, p, &view(), 31);
    assert!(super::layers::kept_bytes() > 0, "the card's layer is kept");
    let empty = drawn(&before, p, &view(), 31);
    assert_eq!(empty.data(), drawn(&before, p, &view(), 0).data(), "without its bytes the card draws nothing");

    let mut png = tiny_skia::Pixmap::new(2, 2).unwrap();
    png.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    let bytes = png.encode_png().unwrap();
    let mut with = d;
    with.images.insert(hash, figcraft_doc::ImageData { bytes: Arc::new(bytes), width: 2, height: 2 });
    let after = Arc::new(with);
    let now = drawn(&after, p, &view(), 31);
    assert_eq!(now.data(), drawn(&after, p, &view(), 0).data(), "the layer draws the image once it arrives");
    assert_ne!(now.data(), empty.data(), "the frame changed");

    // The kept frame too: only the store changed, no node did.
    let mut rt = Retained::default();
    let first = kept(&mut rt, &before, p, 33);
    let (_, again, work) = rt.render(&before, p, &view(), &Options { doc_id: 33, ..Options::default() }).unwrap();
    assert_eq!(work, crate::Work::Reused, "nothing changed: the frame is reused as it is");
    assert_eq!(again.data(), first.data());
    let (_, arrived, work) = rt.render(&after, p, &view(), &Options { doc_id: 33, ..Options::default() }).unwrap();
    assert_ne!(work, crate::Work::Reused, "the arrived image redraws the kept frame");
    let mut fresh_rt = Retained::default();
    let (_, want, _) = fresh_rt.render(&after, p, &view(), &Options::default()).unwrap();
    assert_eq!(arrived.data(), want.data());
}
