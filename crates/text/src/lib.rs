//! FigCraft text: fonts, shaping and layout.
//!
//! Fonts come from the optional craft-fonts build input (Inter and friends, embedded at build time)
//! and, on desktop, from the system's font folders. [`layout`] turns a text node's
//! [`TextData`](figcraft_doc::TextData) into positioned glyphs, lines and decorations; the renderer
//! fills [`Glyph::path`]s and the editor uses [`TextLayout::caret`] / [`TextLayout::hit`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod breaks;
pub mod color;
pub mod display;
pub mod fontdb;
mod layout;
mod onpath;

pub use breaks::grapheme_step;
pub use color::{ColorBitmap, ColorInk, ColorLayer};
pub use fontdb::{
    Axis, CRAFT_FONTS, Coords, Face, FontDb, NamedInstance, axis_label, db, face_with_char, family_names, family_styles, find_face, has_family,
    installed_fonts_known, no_coords, scan_system_fonts, scan_system_fonts_on_demand, system_fonts_scanned,
};
pub use layout::{Decoration, Glyph, Line, TextLayout, layout};
pub use onpath::OnPath;

use figcraft_doc::{Node, NodeKind, TextAlignV, TextAutoResize};

/// One cached layout: the exact text and box it was made for.
struct Cached {
    text: figcraft_doc::TextData,
    w: Option<u64>,
    h: Option<u64>,
    layout: std::sync::Arc<TextLayout>,
}

/// Cached layouts by a cheap fingerprint; entries are confirmed with `TextData`'s full equality.
#[derive(Default)]
struct LayoutCache {
    buckets: std::collections::HashMap<u64, Vec<Cached>>,
    len: usize,
}

/// Layouts kept before the cache starts over (a page of distinct texts stays cached while it
/// renders, pans and zooms).
const LAYOUT_CACHE_MAX: usize = 32_768;
/// Entries per fingerprint (texts that compare unequal to themselves, e.g. NaN sizes, can't pile up).
const BUCKET_MAX: usize = 8;

fn layout_cache() -> &'static std::sync::Mutex<LayoutCache> {
    static C: std::sync::OnceLock<std::sync::Mutex<LayoutCache>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

static LAYOUT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Forget cached layouts (after fonts were added). Bumps [`layout_generation`].
pub fn clear_layout_cache() {
    *layout_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = LayoutCache::default();
    LAYOUT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Changes whenever text may lay out differently than before (fonts were added), so renders kept
/// from before are stale.
pub fn layout_generation() -> u64 {
    LAYOUT_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// The parts of a text that usually tell texts apart, hashed cheaply (no formatting).
fn fingerprint(t: &figcraft_doc::TextData, w: Option<u64>, h: Option<u64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    t.characters.hash(&mut hs);
    t.style.font_family.hash(&mut hs);
    t.style.font_style.hash(&mut hs);
    t.style.font_size.to_bits().hash(&mut hs);
    t.runs.len().hash(&mut hs);
    (w, h).hash(&mut hs);
    hs.finish()
}

/// Layout with a process-wide cache keyed by the text content/style and box size.
pub fn layout_cached(t: &figcraft_doc::TextData, w: Option<f64>, h: Option<f64>) -> std::sync::Arc<TextLayout> {
    let (wb, hb) = (w.map(f64::to_bits), h.map(f64::to_bits));
    let key = fingerprint(t, wb, hb);
    let hit = |c: &Cached| c.w == wb && c.h == hb && c.text == *t;
    if let Some(l) =
        layout_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).buckets.get(&key).and_then(|b| b.iter().find(|c| hit(c)))
    {
        return l.layout.clone();
    }
    let l = std::sync::Arc::new(layout(t, w, h));
    let mut c = layout_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.len >= LAYOUT_CACHE_MAX {
        *c = LayoutCache::default();
    }
    let bucket = c.buckets.entry(key).or_default();
    if bucket.iter().any(hit) {
        // Another thread laid it out meanwhile.
        return l;
    }
    let dropped = if bucket.len() >= BUCKET_MAX {
        bucket.remove(0);
        1
    } else {
        0
    };
    bucket.push(Cached { text: t.clone(), w: wb, h: hb, layout: l.clone() });
    c.len = c.len + 1 - dropped;
    l
}

/// Lay out a text node at its current box, returning the (shared, cached) layout and the vertical
/// offset that applies its vertical alignment.
pub fn layout_node(n: &Node) -> Option<(std::sync::Arc<TextLayout>, f64)> {
    let NodeKind::Text(t) = &n.kind else { return None };
    // Text on a path has no box: one line per paragraph, placed by its path.
    if t.path.is_some() {
        return Some((layout_cached(t, None, None), 0.0));
    }
    let l = layout_cached(t, Some(n.width), Some(n.height));
    let dy = match (t.auto_resize, t.align_v) {
        (TextAutoResize::WidthAndHeight | TextAutoResize::Height, _) => 0.0,
        (_, TextAlignV::Top) => 0.0,
        (_, TextAlignV::Middle) => (n.height - l.height) / 2.0,
        (_, TextAlignV::Bottom) => n.height - l.height,
    };
    Some((l, dy))
}

/// The box size a text node should have after its content or style changed, per its resize mode.
pub fn auto_size(n: &Node) -> Option<(f64, f64)> {
    let NodeKind::Text(t) = &n.kind else { return None };
    if t.path.is_some() {
        return None;
    }
    match t.auto_resize {
        TextAutoResize::WidthAndHeight => {
            let l = layout_cached(t, None, None);
            Some((l.width.ceil().max(1.0), l.height.max(1.0)))
        }
        TextAutoResize::Height => {
            let l = layout_cached(t, Some(n.width), None);
            Some((n.width, l.height.max(1.0)))
        }
        TextAutoResize::None | TextAutoResize::Truncate => None,
    }
}

#[cfg(test)]
mod behaviour_tests;
#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod list_tests;
#[cfg(test)]
mod metrics_tests;
#[cfg(test)]
mod onpath_tests;
#[cfg(test)]
mod rtl_list_tests;
#[cfg(test)]
mod tail_tests;
#[cfg(test)]
mod tests;
