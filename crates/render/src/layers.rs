//! Isolated layers (opacity, effects, blend modes) kept between canvas renders.
//!
//! A drag re-renders the damaged area from the document every frame: every layer in it is
//! rasterised again (fills, shadows, layer blurs…). This keeps the pixels of each isolated layer
//! on the page's device pixel grid, keyed by everything they depend on, so a frame with a few
//! moving layers composites the rest instead of drawing them again.
//!
//! Only layers that can't see the backdrop are kept (see [`cacheable`]): glass and background
//! blurs read what lies under them, and masked frames start as a copy of it. A layer whose
//! subtree is drawn as an outline or left out is drawn again too.

use std::collections::hash_map::{DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_doc::{Document, Node, NodeId, NodeKind};
use figcraft_geom::Affine;
use tiny_skia::Pixmap;

use crate::draw::Ctx;

/// An isolated layer's pixels, rasterised on the page's device pixel grid.
pub(crate) struct Layer {
    pub pix: Pixmap,
    /// Device pixels (page coordinates × scale) of its top-left pixel.
    pub anchor: (i32, i32),
    /// Drop shadows composited under it (they are drawn onto the backdrop, not into the layer).
    pub shadows: Vec<ShadowPart>,
}

/// A drop shadow as the layer draws it: `pix` at the layer's top-left.
pub(crate) struct ShadowPart {
    pub pix: Pixmap,
    pub opacity: f32,
    pub blend: figcraft_color::BlendMode,
}

/// What a layer's pixels depend on (two hashes: a collision would draw the wrong pixels).
type Key = (u64, u64);

/// Layers bigger than this many device pixels aren't kept: they'd crowd out everything else.
pub(crate) const MAX_LAYER_PIXELS: f64 = 4_194_304.0;
/// Kept layers, all together. The cache is a desktop canvas optimisation; a browser tab renders
/// stateless (`crate::Options::doc_id` is 0 there), so there is no budget to keep for one.
const BUDGET: usize = 128 << 20;

struct Entry {
    /// The layer's node, held so its address can't be reused by another node while the entry
    /// lives (see [`key`]).
    root: Arc<Node>,
    layer: Arc<Layer>,
    bytes: usize,
    /// The [`State::tick`] of the last use: the smallest ones are dropped first.
    used: u64,
}

struct State {
    map: HashMap<Key, Entry>,
    /// The keys kept for each node: a node's older versions are dropped when a new one is kept
    /// (see [`put`]).
    by_node: HashMap<NodeId, Vec<Key>>,
    bytes: usize,
    tick: u64,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State { map: HashMap::new(), by_node: HashMap::new(), bytes: 0, tick: 0 }))
}

/// Forget every kept layer (tests compare renders with and without the cache).
#[cfg(test)]
pub(crate) fn clear() {
    let mut state = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.map.clear();
    state.by_node.clear();
    state.bytes = 0;
    let mut seen = seen().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    seen.set.clear();
    seen.order.clear();
}

/// The bytes the kept layers take (tests check that a drag doesn't fill the cache).
#[cfg(test)]
pub(crate) fn kept_bytes() -> usize {
    state().lock().unwrap_or_else(std::sync::PoisonError::into_inner).bytes
}

/// Whether node `n`'s layer can be kept: nothing in it may read the backdrop (glass, background
/// blur, masks — see the module docs), it must be on whole device pixels, it must not be drawn
/// differently every frame (animated GIF frames, outlines), nothing under it may be drawn as an
/// outline or left out (per-node states a layer's pixels would keep from the frame they were made
/// in), and no paint in it may read a node outside it.
pub(crate) fn cacheable(ctx: &Ctx, n: &Node) -> bool {
    ctx.cache.is_some() && !ctx.outline_mode && ctx.time == 0.0 && whole(ctx.page_at.0) && whole(ctx.page_at.1) && cacheable_subtree(ctx, n, 0)
}

/// Whether `v` is a whole device pixel (the scale round trip leaves a hair either side).
fn whole(v: f64) -> bool {
    (v - v.round()).abs() < 1e-6
}

/// Whether any of `paints` draws a pattern, whose pixels belong to a node outside the layer.
fn paints_a_pattern(paints: &[figcraft_color::Paint]) -> bool {
    paints.iter().any(|p| matches!(p.kind, figcraft_color::PaintKind::Pattern(_)))
}

fn cacheable_subtree(ctx: &Ctx, n: &Node, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    if n.is_mask || ctx.outlined.contains(&n.id) || ctx.skip.contains(&n.id) || crate::backdrop::has_effects(n) {
        return false;
    }
    // A pattern fill's pixels are its source node's, which is outside this subtree: an edit there
    // changes nothing this layer's key hashes.
    if paints_a_pattern(&n.fills) || paints_a_pattern(&n.strokes) {
        return false;
    }
    // A text run with its own fills draws with those (the layout's styles), not the node's: they
    // can name a pattern source outside the subtree just as a node-level fill can.
    if let NodeKind::Text(t) = &n.kind
        && t.spans().iter().any(|(_, _, s)| s.fills.as_deref().is_some_and(paints_a_pattern))
    {
        return false;
    }
    n.children.iter().all(|c| ctx.doc.node(*c).is_none_or(|cn| cacheable_subtree(ctx, cn, depth + 1)))
}

/// The kept layer of `n` drawn through `page_xf` (local → page device pixels), if there is one.
/// `root` is the node itself: the entry's own copy is compared by pointer, so a key that matches
/// another node (a hash collision, or an address reused after the entry was made) is a miss.
pub(crate) fn get(key: Key, root: &Arc<Node>) -> Option<Arc<Layer>> {
    let mut state = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.tick += 1;
    let tick = state.tick;
    let entry = state.map.get_mut(&key)?;
    if !Arc::ptr_eq(&entry.root, root) {
        return None;
    }
    entry.used = tick;
    Some(Arc::clone(&entry.layer))
}

/// Whether `key` was drawn before: a layer is only kept from its second sight, because a layer
/// that changes every frame (one being dragged) is never found again, and drawing the whole of it
/// to keep it would cost more than cutting it to what a render needs.
pub(crate) fn settled(key: Key) -> bool {
    let mut seen = seen().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.set.insert(key) {
        return true;
    }
    seen.order.push_back(key);
    while seen.order.len() > SEEN_MAX {
        if let Some(old) = seen.order.pop_front() {
            seen.set.remove(&old);
        }
    }
    false
}

/// Drop the entry kept at `key`, from both indexes, and give its bytes back.
fn drop_entry(state: &mut State, key: Key) {
    let Some(entry) = state.map.remove(&key) else { return };
    state.bytes = state.bytes.saturating_sub(entry.bytes);
    let Some(keys) = state.by_node.get_mut(&entry.root.id) else { return };
    if let Some(i) = keys.iter().position(|k| *k == key) {
        keys.swap_remove(i);
    }
    if keys.is_empty() {
        state.by_node.remove(&entry.root.id);
    }
}

/// Keep `layer`, `n`'s pixels drawn through `page_xf` with its top-left at `anchor`.
pub(crate) fn put(key: Key, root: Arc<Node>, layer: Arc<Layer>) {
    let bytes = layer.pix.data().len() + layer.shadows.iter().map(|s| s.pix.data().len()).sum::<usize>();
    if bytes > BUDGET {
        return;
    }
    let mut state = state().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.tick += 1;
    let tick = state.tick;
    drop_entry(&mut state, key);
    // A node edited since an entry was made is a new `Arc`: entries keyed on the old one never hit
    // again, and the nodes they hold (text, paths, children) take memory the pixel budget doesn't
    // count. Drop them; the same node drawn at another transform keeps its entry.
    let id = root.id;
    let mut kept_keys = vec![key];
    for k in state.by_node.remove(&id).unwrap_or_default() {
        if state.map.get(&k).is_some_and(|e| Arc::ptr_eq(&e.root, &root)) {
            kept_keys.push(k);
        } else {
            drop_entry(&mut state, k);
        }
    }
    state.by_node.insert(id, kept_keys);
    while state.bytes + bytes > BUDGET {
        let Some(oldest) = state.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) else { break };
        drop_entry(&mut state, oldest);
    }
    state.bytes += bytes;
    state.map.insert(key, Entry { root, layer, bytes, used: tick });
}

/// What the document's images are right now: every key and the bytes stored under it. A content
/// hash names fixed bytes, but the store can gain an image after a frame that named it was drawn
/// (a collaborative image entry, an import finishing), or hold other bytes under a key a document
/// named before they arrived — both change what a fill draws without changing a node, so a kept
/// layer has to see them.
pub(crate) fn images_id(doc: &Document) -> u64 {
    let mut h = DefaultHasher::new();
    for (key, data) in &doc.images {
        key.hash(&mut h);
        (Arc::as_ptr(&data.bytes) as usize).hash(&mut h);
    }
    h.finish()
}

/// What [`settled`] remembers: the keys drawn lately, oldest out first.
struct Seen {
    set: std::collections::HashSet<Key>,
    order: std::collections::VecDeque<Key>,
}

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(Seen { set: std::collections::HashSet::new(), order: std::collections::VecDeque::new() }))
}

/// Keys remembered as drawn (a page of a few hundred effect layers fits many times over).
const SEEN_MAX: usize = 2048;

pub(crate) fn key(ctx: &Ctx, n: &Node, page_xf: Affine) -> Key {
    let mut first = DefaultHasher::new();
    (ctx.cache, ctx.lod, figcraft_text::layout_generation()).hash(&mut first);
    // Images are decoded into the file's colour space: another profile draws them differently.
    ctx.doc.color_profile.hash(&mut first);
    // An image that arrived (or changed) after a layer was drawn changes what its paints show
    // without changing a node: a collaborative image entry, an import finishing.
    ctx.images.hash(&mut first);
    subtree(ctx.doc, n, 0).hash(&mut first);
    let mut second = DefaultHasher::new();
    for v in page_xf.as_coeffs().map(f64::to_bits) {
        v.hash(&mut second);
    }
    (first.finish(), second.finish())
}

/// The identity of a node and everything under it: nodes are shared `Arc`s, so a changed node is a
/// new address, and a changed child shows here as a changed address too. An address alone could
/// name another node once the old one is freed, so a [`Entry`] holds its layer's node (the root,
/// whose change is what moves its children) and [`get`] compares them by pointer: a reused address
/// then never yields old pixels.
fn subtree(doc: &Document, n: &Node, depth: usize) -> u64 {
    let mut h = DefaultHasher::new();
    (n as *const Node as usize).hash(&mut h);
    if depth < 64 {
        for c in &n.children {
            if let Some(cn) = doc.node(*c) {
                subtree(doc, cn, depth + 1).hash(&mut h);
            }
        }
    }
    h.finish()
}
