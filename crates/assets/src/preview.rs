//! Small, disposable display resources. Originals remain the only document image assets.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::AssetRef;

pub const MAX_BYTES: usize = 8 << 20;
const CACHE_BYTES: usize = 64 << 20;
pub type Loader = dyn Fn(Request) -> bool + Send + Sync;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub asset: AssetRef,
    pub side: u16,
}

impl Request {
    pub fn valid(&self) -> bool {
        self.asset.valid() && (64..=1024).contains(&self.side) && self.side.is_power_of_two()
    }
}

#[derive(Default)]
struct State {
    loaders: BTreeMap<String, Vec<Weak<Loader>>>,
    pending: BTreeMap<Request, Instant>,
    failed: BTreeMap<Request, Instant>,
    bytes: BTreeMap<Request, (Arc<Vec<u8>>, u64)>,
    held: usize,
    tick: u64,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

pub fn register(scope: &str, loader: &Arc<Loader>) {
    if scope.len() != 64 || !scope.bytes().all(|c| c.is_ascii_hexdigit()) {
        return;
    }
    let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
    s.loaders.retain(|_, v| {
        v.retain(|l| l.strong_count() > 0);
        !v.is_empty()
    });
    if s.loaders.len() >= 1024 && !s.loaders.contains_key(scope) {
        return;
    }
    let list = s.loaders.entry(scope.into()).or_default();
    if list.len() < 16 && !list.iter().any(|l| Weak::ptr_eq(l, &Arc::downgrade(loader))) {
        list.push(Arc::downgrade(loader));
    }
}

/// Discard display buffers; keep pending requests so valid in-flight replies stay matched.
pub fn clear() {
    let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
    s.bytes.clear();
    s.failed.clear();
    s.held = 0;
    drop(s);
    crate::cache::notify_changed();
}

pub fn load(request: &Request) -> Option<Arc<Vec<u8>>> {
    let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
    s.tick = s.tick.wrapping_add(1);
    let tick = s.tick;
    s.bytes.get_mut(request).map(|(bytes, used)| {
        *used = tick;
        bytes.clone()
    })
}

pub enum Display {
    Ready(Arc<Vec<u8>>),
    Pending,
    /// An older/offline server: the caller can use the normal original-image path.
    Unsupported,
}

/// Disposable encoded preview resources held in this process (entries, bytes).
pub fn stats() -> (usize, usize) {
    let s = state().lock().unwrap_or_else(PoisonError::into_inner);
    (s.bytes.len(), s.held)
}

pub fn resolve(request: &Request) -> Display {
    if !request.valid() {
        return Display::Unsupported;
    }
    if let Some(bytes) = load(request) {
        return Display::Ready(bytes);
    }
    let loaders = {
        let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
        if s.pending.get(request).is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return Display::Pending;
        }
        if s.failed.get(request).is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return Display::Unsupported;
        }
        let loaders: Vec<_> = s.loaders.get(&request.asset.scope).into_iter().flatten().filter_map(Weak::upgrade).collect();
        if loaders.is_empty() {
            return Display::Unsupported;
        }
        s.pending.retain(|_, t| t.elapsed() < Duration::from_secs(30));
        if s.pending.len() >= 512 {
            return Display::Pending;
        }
        s.pending.insert(request.clone(), Instant::now());
        loaders
    };
    if loaders.iter().any(|l| l(request.clone())) {
        Display::Pending
    } else {
        failed(request);
        Display::Unsupported
    }
}

pub fn failed(request: &Request) {
    let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
    s.pending.remove(request);
    if s.failed.len() >= 2048 {
        s.failed.clear();
    }
    s.failed.insert(request.clone(), Instant::now());
    drop(s);
    crate::cache::notify_changed();
}

/// The network layer validates dimensions/format before inserting a received preview.
pub fn store(request: &Request, bytes: Arc<Vec<u8>>) -> Result<(), String> {
    if !request.valid() || bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("invalid image preview".into());
    }
    let mut s = state().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((old, _)) = s.bytes.remove(request) {
        s.held = s.held.saturating_sub(old.len());
    }
    while s.held.saturating_add(bytes.len()) > CACHE_BYTES || s.bytes.len() >= 2048 {
        let Some(old) = s.bytes.iter().min_by_key(|(_, (_, tick))| *tick).map(|(r, _)| r.clone()) else { break };
        if let Some((bytes, _)) = s.bytes.remove(&old) {
            s.held = s.held.saturating_sub(bytes.len());
        }
    }
    s.tick = s.tick.wrapping_add(1);
    let tick = s.tick;
    s.held += bytes.len();
    s.bytes.insert(request.clone(), (bytes, tick));
    s.pending.remove(request);
    s.failed.remove(request);
    drop(s);
    crate::cache::notify_changed();
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn previews_are_disposable_scoped_resources_and_requests_are_deduplicated() {
        let mut asset = AssetRef::of(b"synthetic-preview-source");
        asset.scope = "7".repeat(64);
        let a = Request { asset: asset.clone(), side: 128 };
        let mut b = a.clone();
        b.asset.scope = "8".repeat(64);
        store(&a, Arc::new(vec![1, 2, 3])).unwrap();
        assert_eq!(*load(&a).unwrap(), vec![1, 2, 3]);
        assert!(load(&b).is_none());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let loader: Arc<Loader> = Arc::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            true
        });
        register(&b.asset.scope, &loader);
        assert!(matches!(resolve(&b), Display::Pending));
        assert!(matches!(resolve(&b), Display::Pending));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        failed(&b);
        assert!(matches!(resolve(&b), Display::Unsupported));
        let mut invalid = a;
        invalid.side = 300;
        assert!(!invalid.valid());
        assert!(store(&invalid, Arc::new(vec![1])).is_err());
    }
}
