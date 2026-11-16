//! Bounded cache of immutable, unplaced SVG outlines for missing-font restoration.
//! Only parsing is reused. Each glyph still clones and applies its exact affine eagerly.
use figcraft_geom::{BezPath, kurbo::PathEl};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const MAX_ENTRIES: usize = 2048;
#[cfg(not(target_pointer_width = "32"))]
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct Cache {
    entries: HashMap<Arc<str>, Arc<BezPath>>,
    bytes: usize,
}
impl Cache {
    fn get(&self, source: &str) -> Option<Arc<BezPath>> {
        self.entries.get(source).cloned()
    }
    fn admit(&mut self, source: Arc<str>, path: Arc<BezPath>, charge: usize, limit: usize, count: usize) -> Arc<BezPath> {
        if let Some(hit) = self.get(&source) {
            return hit;
        }
        if charge > limit || count == 0 {
            return path;
        }
        if self.bytes.checked_add(charge).is_none_or(|n| n > limit) || self.entries.len() >= count {
            *self = Self::default();
        }
        self.bytes += charge;
        self.entries.insert(source, path.clone());
        path
    }
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn parse(source: &str) -> Option<(Arc<BezPath>, usize)> {
    let parsed = BezPath::from_svg(source).ok()?;
    if !parsed.is_finite() {
        return None;
    }
    let elements = parsed.into_elements();
    // from_vec has a debug assertion: never feed it an invalid initial path element.
    if !matches!(elements.first(), None | Some(PathEl::MoveTo(_))) {
        return None;
    }
    // Charge full vector capacity, source bytes retained by the key, and an entry allowance.
    // Entry count is bounded separately, including HashMap allocation overhead.
    let charge = source.len().saturating_add(elements.capacity().saturating_mul(std::mem::size_of::<PathEl>())).saturating_add(128);
    Some((Arc::new(BezPath::from_vec(elements)), charge))
}
pub(crate) fn get(source: &Arc<str>) -> Option<Arc<BezPath>> {
    if let Some(hit) = cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(source) {
        return Some(hit);
    }
    // Parse outside the mutex: a long, uncached path must not block unrelated cache hits.
    let (path, charge) = parse(source)?;
    Some(cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).admit(source.clone(), path, charge, MAX_BYTES, MAX_ENTRIES))
}
pub(crate) fn clear() {
    *cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Cache::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hits_share_only_immutable_unplaced_geometry() {
        let source: Arc<str> = "M-0 0C1.123456789012345 2 3 4 5 6Z".into();
        let (path, charge) = parse(&source).unwrap();
        let original = path.to_svg();
        let mut c = Cache::default();
        let first = c.admit(source.clone(), path, charge, 4096, 8);
        let hit = c.get(&source).unwrap();
        assert!(Arc::ptr_eq(&first, &hit));
        let mut placed = (*hit).clone();
        placed.apply_affine(figcraft_geom::Affine::new([0.125, -0.0, 0.25, 0.125, 9.123456789012345, -0.0]));
        assert_eq!(c.get(&source).unwrap().to_svg(), original);
        assert_ne!(placed.to_svg(), original);
        assert_eq!(first.to_svg(), BezPath::from_svg(&source).unwrap().to_svg());
    }
    #[test]
    fn byte_and_entry_bounds_are_independent_and_counters_do_not_overflow() {
        let source: Arc<str> = "M0 0L10 10Z".into();
        let (path, charge) = parse(&source).unwrap();
        let mut bytes = Cache::default();
        bytes.admit(source.clone(), path.clone(), charge, charge, 8);
        bytes.admit("M1 1L10 10Z".into(), path.clone(), charge, charge, 8);
        assert_eq!(bytes.entries.len(), 1);
        assert!(bytes.get(&source).is_none());
        let mut entries = Cache::default();
        entries.admit(source.clone(), path.clone(), charge, charge * 8, 1);
        entries.admit("M1 1L10 10Z".into(), path.clone(), charge, charge * 8, 1);
        assert_eq!(entries.entries.len(), 1);
        assert!(entries.get(&source).is_none());
        let mut overflow = Cache::default();
        overflow.admit(source.clone(), path.clone(), usize::MAX, usize::MAX, 8);
        overflow.admit("M1 1L10 10Z".into(), path, 1, usize::MAX, 8);
        assert_eq!(overflow.bytes, 1);
        assert_eq!(overflow.entries.len(), 1);
        assert!(overflow.get(&source).is_none());
    }

    #[test]
    fn admission_bounds_bytes_entries_and_uncached_large_paths() {
        let source: Arc<str> = "M0 0L10 10Z".into();
        let (path, charge) = parse(&source).unwrap();
        let mut c = Cache::default();
        c.admit(source.clone(), path.clone(), charge, charge, 1);
        assert_eq!(c.bytes, charge);
        c.admit("M1 1L10 10Z".into(), path.clone(), charge, charge, 1);
        assert!(c.get(&source).is_none());
        assert_eq!(c.entries.len(), 1);
        assert!(c.bytes <= charge);
        let before = c.bytes;
        c.admit("too large to retain".into(), path, charge + 1, charge, 1);
        assert_eq!(c.bytes, before);
        assert!(c.get("too large to retain").is_none());
        assert!(parse("not an SVG path").is_none());
    }
}
