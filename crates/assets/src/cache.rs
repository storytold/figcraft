//! Native content-addressed disk cache. A miss starts a nonblocking request; saving can wait
//! for that same request. No network implementation or document state lives in this crate.
use crate::AssetRef;
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

pub type Loader = dyn Fn(AssetRef) -> bool + Send + Sync;

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

pub fn notify_changed() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

#[derive(Default)]
struct State {
    loaders: BTreeMap<String, Vec<Weak<Loader>>>,
    pending: BTreeMap<AssetRef, Instant>,
    failed: BTreeMap<AssetRef, (Instant, String, bool)>,
    #[cfg(target_arch = "wasm32")]
    bytes: BTreeMap<AssetRef, Arc<Vec<u8>>>,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static STATE: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(State::default()), Condvar::new()))
}

pub fn register(asset: &AssetRef, loader: &Arc<Loader>) {
    if !asset.valid() {
        return;
    }
    register_scope(&asset.scope, loader);
    state().0.lock().unwrap_or_else(PoisonError::into_inner).failed.remove(asset);
}

pub fn register_scope(scope: &str, loader: &Arc<Loader>) {
    if !scope.is_empty() && (scope.len() != 64 || !scope.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))) {
        return;
    }
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    // Weak registrations cannot keep a closed document or its network thread alive.
    if s.loaders.len() >= 1024 && !s.loaders.contains_key(scope) {
        s.loaders.retain(|_, loaders| {
            loaders.retain(|l| l.strong_count() > 0);
            !loaders.is_empty()
        });
        if s.loaders.len() >= 1024 {
            return;
        }
    }
    let loaders = s.loaders.entry(scope.to_string()).or_default();
    loaders.retain(|l| l.strong_count() > 0);
    if loaders.len() < 16 && !loaders.iter().any(|l| Weak::ptr_eq(l, &Arc::downgrade(loader))) {
        loaders.push(Arc::downgrade(loader));
    }
}

pub fn request(asset: &AssetRef) -> bool {
    match request_status(asset) {
        Request::Accepted => true,
        Request::Failed(reason) => {
            drop(reason);
            false
        }
        _ => false,
    }
}

enum Request {
    Accepted,
    Busy,
    Unavailable,
    Failed(String),
}

fn request_status(asset: &AssetRef) -> Request {
    if !asset.valid() {
        return Request::Unavailable;
    }
    let loaders = {
        let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
        if s.pending.get(asset).is_some_and(|at| at.elapsed() < Duration::from_secs(30)) {
            return Request::Accepted;
        }
        if let Some((at, reason, _)) = s.failed.get(asset)
            && at.elapsed() < Duration::from_secs(5)
        {
            return Request::Failed(reason.clone());
        }
        let loaders: Vec<_> = s.loaders.get(&asset.scope).into_iter().flatten().filter_map(Weak::upgrade).collect();
        if loaders.is_empty() {
            return Request::Unavailable;
        }
        s.pending.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        let held: u64 = s.pending.keys().map(|asset| asset.len).sum();
        if s.pending.len() >= 2048 || held > 0 && held.saturating_add(asset.len) > 128 << 20 {
            return Request::Busy;
        }
        s.failed.remove(asset);
        s.pending.insert(asset.clone(), Instant::now());
        loaders
    };
    if loaders.iter().any(|loader| loader(asset.clone())) {
        return Request::Accepted;
    }
    failed(asset, "the image's session is no longer connected");
    Request::Unavailable
}

pub fn failed(asset: &AssetRef, reason: &str) {
    failure(asset, reason, false);
}

pub fn unavailable(asset: &AssetRef, reason: &str) {
    failure(asset, reason, true);
}

pub fn is_unavailable(asset: &AssetRef) -> bool {
    state().0.lock().unwrap_or_else(PoisonError::into_inner).failed.get(asset).is_some_and(|(_, _, permanent)| *permanent)
}

fn failure(asset: &AssetRef, reason: &str, permanent: bool) {
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    s.pending.remove(asset);
    if s.failed.len() >= 20_000 {
        s.failed.clear();
    }
    s.failed.insert(asset.clone(), (Instant::now(), reason.chars().take(512).collect(), permanent));
    state().1.notify_all();
}

fn ready(asset: &AssetRef) {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    s.pending.remove(asset);
    s.failed.remove(asset);
    state().1.notify_all();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn enqueue(asset: &AssetRef) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match request_status(asset) {
            Request::Accepted => return Ok(()),
            Request::Unavailable => return Err("an image is not cached and its session is unavailable; reconnect before saving or exporting".into()),
            Request::Failed(reason) => return Err(reason),
            Request::Busy => {
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("the image download budget is busy; try again in a moment".into());
                }
                let s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
                drop(state().1.wait_timeout(s, remaining.min(Duration::from_millis(100))).unwrap_or_else(PoisonError::into_inner));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn require(asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    if let Ok(bytes) = load(asset) {
        return Ok(bytes);
    }
    enqueue(asset)?;
    let until = Instant::now() + Duration::from_secs(30);
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if let Some((_, reason, _)) = s.failed.get(asset) {
            return Err(reason.clone());
        }
        if !s.pending.contains_key(asset) {
            drop(s);
            return load(asset);
        }
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out downloading an image; the file was not saved without it".into());
        }
        s = state().1.wait_timeout(s, remaining).unwrap_or_else(PoisonError::into_inner).0;
    }
}

#[cfg(target_arch = "wasm32")]
pub fn require(asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    load(asset).map_err(|_| "deferred image downloads require the native client; the browser uses the legacy complete document".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load(asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    disk().read(asset)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn store(asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
    disk().write(asset, bytes)?;
    ready(asset);
    Ok(())
}

/// Store an immutable resident buffer, reusing its verified hash and this process's file stamp.
/// Reads still validate payloads; metadata is only a fast path for redundant cache insertion.
#[cfg(not(target_arch = "wasm32"))]
pub fn store_resident(asset: &AssetRef, bytes: &Arc<Vec<u8>>) -> Result<(), String> {
    if disk().write_arc(asset, bytes)? {
        ready(asset);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load(asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    state().0.lock().unwrap_or_else(PoisonError::into_inner).bytes.get(asset).cloned().ok_or_else(|| "image is not cached".into())
}

#[cfg(target_arch = "wasm32")]
pub fn store(asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
    if !asset.verify(bytes) {
        return Err("invalid image checksum or length".into());
    }
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    let held: u64 = s.bytes.keys().map(|a| a.len).sum();
    if held.saturating_add(asset.len) > 64 << 20 {
        return Err("browser asset cache budget exceeded".into());
    }
    s.bytes.insert(asset.clone(), Arc::new(bytes.to_vec()));
    drop(s);
    ready(asset);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn disk() -> &'static DiskCache {
    static DISK: OnceLock<DiskCache> = OnceLock::new();
    DISK.get_or_init(|| {
        if let Some(root) = std::env::var_os("FIGCRAFT_ASSET_CACHE_DIR") {
            return DiskCache::new(root.into(), 8 << 30);
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".cache")))
            .unwrap_or_else(|| std::env::temp_dir().join(format!("figcraft-{}", std::process::id())));
        DiskCache::new(base.join("FigCraft").join("assets-v1"), 8 << 30)
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub struct DiskCache {
    root: std::path::PathBuf,
    max: u64,
    used: Mutex<Option<HashMap<String, (u64, std::time::SystemTime)>>>,
    verified: Mutex<HashMap<String, (u64, std::time::SystemTime)>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl DiskCache {
    pub fn new(root: std::path::PathBuf, max_bytes: u64) -> Self {
        Self { root, max: max_bytes, used: Mutex::new(None), verified: Mutex::new(HashMap::new()) }
    }

    /// Returns false when a verified, unchanged entry already exists, without reading its bytes.
    pub fn write_arc(&self, asset: &AssetRef, bytes: &Arc<Vec<u8>>) -> Result<bool, String> {
        if !asset.valid() || asset.len > self.max || !AssetRef::of_arc(bytes).same_content(asset) {
            return Err("invalid image checksum, length or cache budget".into());
        }
        let key = asset.cache_key();
        let stamp = || {
            let metadata = std::fs::metadata(self.root.join(&key)).ok()?;
            metadata.is_file().then_some((metadata.len(), metadata.modified().ok()?))
        };
        let known = self.verified.lock().unwrap_or_else(PoisonError::into_inner).get(&key).copied();
        if known.is_some() && known == stamp() {
            if let Some(index) = self.used.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                index.insert(key, (asset.len, std::time::SystemTime::now()));
            }
            return Ok(false);
        }
        self.write(asset, bytes)?;
        if let Some(stamp) = stamp() {
            let mut verified = self.verified.lock().unwrap_or_else(PoisonError::into_inner);
            if verified.len() >= 20_000 {
                verified.retain(|key, _| self.root.join(key).is_file());
                if verified.len() >= 20_000 {
                    verified.clear();
                }
            }
            verified.insert(key, stamp);
        }
        Ok(true)
    }

    pub fn read(&self, asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
        if !asset.valid() {
            return Err("invalid image reference".into());
        }
        let bytes = read_bytes(&self.root.join(asset.cache_key()), asset)?;
        if let Some(index) = self.used.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            index.insert(asset.cache_key(), (asset.len, std::time::SystemTime::now()));
        }
        Ok(bytes)
    }

    pub fn write(&self, asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if !asset.verify(bytes) || asset.len > self.max {
            return Err("invalid image checksum, length or cache budget".into());
        }
        self.verified.lock().unwrap_or_else(PoisonError::into_inner).remove(&asset.cache_key());
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        }
        // Serialize insertion/eviction, including concurrent writes of the same content.
        let mut held = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        let index = held.get_or_insert_with(|| {
            std::fs::read_dir(&self.root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_str()?.to_string();
                    if name.len() != 64 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {
                        return None;
                    }
                    let metadata = entry.metadata().ok()?;
                    Some((name, (metadata.len(), metadata.modified().unwrap_or(std::time::UNIX_EPOCH))))
                })
                .collect()
        });
        let key = asset.cache_key();
        let target = self.root.join(&key);
        let temporary = self.root.join(format!(".{}-{}-{}.part", key, std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let result = (|| -> Result<(), String> {
            if read_bytes(&target, asset).is_ok() {
                return Ok(());
            }
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
            drop(file);
            if let Err(error) = std::fs::rename(&temporary, &target) {
                // Another process may have inserted identical immutable content. Never remove
                // a valid cache entry just to replace it with the same bytes (Windows readers
                // can hold it open). A corrupted entry is replaced only after verification.
                if read_bytes(&target, asset).is_ok() {
                    return Ok(());
                }
                if !target.exists() {
                    return Err(error.to_string());
                }
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
                std::fs::rename(&temporary, &target).map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&temporary);
        result?;
        index.insert(key.clone(), (asset.len, std::time::SystemTime::now()));
        let mut total: u64 = index.values().map(|v| v.0).sum();
        while total > self.max || index.len() > 20_000 {
            let Some(oldest) = index.iter().filter(|(other, _)| *other != &key).min_by_key(|(_, v)| v.1).map(|(k, v)| (k.clone(), v.0)) else {
                break;
            };
            let _ = std::fs::remove_file(self.root.join(&oldest.0));
            index.remove(&oldest.0);
            self.verified.lock().unwrap_or_else(PoisonError::into_inner).remove(&oldest.0);
            total = total.saturating_sub(oldest.1);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bytes(path: &std::path::Path, asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "image is not cached")?;
    let mut bytes = Vec::new();
    file.take(asset.len + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if !asset.verify(&bytes) {
        return Err("cached image checksum or length does not match".into());
    }
    Ok(Arc::new(bytes))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn immutable_resident_inserts_are_cheap_and_missing_or_damaged_entries_are_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path().to_path_buf(), 1024);
        let bytes = Arc::new(b"synthetic resident cache source".to_vec());
        let asset = AssetRef::of_arc(&bytes);
        assert!(cache.write_arc(&asset, &bytes).unwrap());
        assert!(!cache.write_arc(&asset, &bytes).unwrap());
        assert!(cache.write_arc(&asset, &Arc::new(b"wrong source".to_vec())).is_err());
        std::fs::write(dir.path().join(asset.cache_key()), b"damaged").unwrap();
        assert!(cache.write_arc(&asset, &bytes).unwrap());
        assert_eq!(&*cache.read(&asset).unwrap(), &*bytes);
        std::fs::remove_file(dir.path().join(asset.cache_key())).unwrap();
        assert!(cache.write_arc(&asset, &bytes).unwrap());
        assert!(!cache.write_arc(&asset, &bytes).unwrap());
        let second = DiskCache::new(dir.path().to_path_buf(), 1024);
        assert!(second.write_arc(&asset, &bytes).unwrap());
    }
    #[test]
    fn separate_cache_instances_can_insert_identical_content_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let root = dir.path().to_path_buf();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let cache = DiskCache::new(root, 1024);
                let asset = AssetRef::of(b"synthetic concurrent content");
                barrier.wait();
                for _ in 0..20 {
                    cache.write(&asset, b"synthetic concurrent content").unwrap();
                    assert_eq!(cache.read(&asset).unwrap().as_slice(), b"synthetic concurrent content");
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
    }
    #[test]
    fn requests_are_deduplicated_and_reserve_a_bounded_encoded_byte_budget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let loader: Arc<Loader> = Arc::new(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            true
        });
        let mut a = AssetRef::of(b"budget synthetic a");
        a.scope = AssetRef::of(b"budget synthetic scope").sha256;
        a.len = 64 << 20;
        let mut b = a.clone();
        b.sha256 = AssetRef::of(b"budget synthetic b").sha256;
        let mut c = a.clone();
        c.sha256 = AssetRef::of(b"budget synthetic c").sha256;
        c.len = 1;
        register(&a, &loader);
        assert!(request(&a));
        assert!(request(&a));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(request(&b), "registration covers the authenticated file scope");
        assert!(!request(&c), "queue must reserve encoded bytes before starting more downloads");
        unavailable(&a, "synthetic missing");
        assert!(is_unavailable(&a));
        assert!(request(&c));
        failed(&b, "test finished");
        failed(&c, "test finished");
    }
    #[test]
    fn disk_cache_is_bounded_and_checks_content_not_just_names() {
        let dir = tempfile::tempdir().unwrap();
        let c = DiskCache::new(dir.path().to_path_buf(), 12);
        let a = AssetRef::of(b"first-image");
        let b = AssetRef::of(b"next-image");
        c.write(&a, b"first-image").unwrap();
        assert_eq!(c.read(&a).unwrap().as_slice(), b"first-image");
        c.write(&b, b"next-image").unwrap();
        assert!(c.read(&a).is_err());
        assert_eq!(c.read(&b).unwrap().as_slice(), b"next-image");
        std::fs::write(dir.path().join(&b.sha256), b"bad-image!").unwrap();
        assert!(c.read(&b).is_err());
        c.write(&b, b"next-image").unwrap();
        assert_eq!(c.read(&b).unwrap().as_slice(), b"next-image", "a re-download repairs corrupt cached data");
        assert!(c.write(&a, b"bad").is_err());
        assert!(c.read(&AssetRef { scope: String::new(), sha256: "../file".into(), len: 4 }).is_err());
    }

    #[test]
    fn identical_content_in_another_file_scope_is_not_authority_to_read_it() {
        let dir = tempfile::tempdir().unwrap();
        let c = DiskCache::new(dir.path().to_path_buf(), 1024);
        let mut a = AssetRef::of(b"private synthetic image");
        a.scope = "1".repeat(64);
        let mut b = a.clone();
        b.scope = "2".repeat(64);
        c.write(&a, b"private synthetic image").unwrap();
        assert!(c.read(&a).is_ok());
        assert!(c.read(&b).is_err());
        assert!(a.same_content(&b));
        assert_ne!(a.cache_key(), b.cache_key());
    }
}
