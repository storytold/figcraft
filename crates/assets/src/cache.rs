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

type Waker = dyn Fn() + Send + Sync;
fn wakers() -> &'static Mutex<Vec<Weak<Waker>>> {
    static WAKERS: OnceLock<Mutex<Vec<Weak<Waker>>>> = OnceLock::new();
    WAKERS.get_or_init(Default::default)
}
pub fn watch(waker: &Arc<Waker>) {
    let mut list = wakers().lock().unwrap_or_else(PoisonError::into_inner);
    list.retain(|w| w.strong_count() > 0);
    if list.len() < 1024 && !list.iter().any(|w| Weak::ptr_eq(w, &Arc::downgrade(waker))) {
        list.push(Arc::downgrade(waker));
    }
}
pub fn notify_changed() {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    let listeners: Vec<_> = wakers().lock().unwrap_or_else(PoisonError::into_inner).iter().filter_map(Weak::upgrade).collect();
    for w in listeners {
        w();
    }
}

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

/// Encoded originals retained on disk; disposable, not file storage.
pub const DISK_BUDGET: u64 = 512 << 20;
const MEMORY_BUDGET: usize = 64 << 20;

fn memory(asset: &AssetRef) -> Option<Arc<Vec<u8>>> {
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    s.tick = s.tick.wrapping_add(1);
    let tick = s.tick;
    s.bytes.get_mut(asset).map(|(bytes, used)| {
        *used = tick;
        bytes.clone()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn remember(asset: &AssetRef, bytes: Arc<Vec<u8>>) {
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    remember_locked(&mut s, asset, bytes);
}

#[cfg(not(target_arch = "wasm32"))]
fn remember_locked(s: &mut State, asset: &AssetRef, bytes: Arc<Vec<u8>>) {
    s.bytes.remove(asset);
    let mut held: usize = s.bytes.values().map(|(b, _)| b.len()).sum();
    while !s.bytes.is_empty() && (held.saturating_add(bytes.len()) > MEMORY_BUDGET || s.bytes.len() >= 1024) {
        let Some(oldest) = s.bytes.iter().min_by_key(|(_, (_, used))| *used).map(|(a, _)| a.clone()) else { break };
        if let Some((b, _)) = s.bytes.remove(&oldest) {
            held = held.saturating_sub(b.len());
        }
    }
    s.tick = s.tick.wrapping_add(1);
    let tick = s.tick;
    s.bytes.insert(asset.clone(), (bytes, tick));
}

/// Paint only looks in memory and queues work. The single bounded worker probes the disk
/// and starts a download on a miss; decoding and network completion invalidate rendering.
pub fn display(asset: &AssetRef) -> Option<Arc<Vec<u8>>> {
    if let Some(bytes) = memory(asset) {
        return Some(bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !asset.valid() {
            return None;
        }
        let epoch = {
            let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
            if s.pending.get(asset).is_some_and(|at| at.elapsed() < Duration::from_secs(30))
                || s.failed.get(asset).is_some_and(|(at, _, _)| at.elapsed() < Duration::from_secs(5))
                || s.reading.len() >= 64
                || !s.reading.insert(asset.clone())
            {
                return None;
            }
            s.epoch
        };
        if reader().as_ref().is_none_or(|tx| tx.try_send((asset.clone(), epoch)).is_err()) {
            state().0.lock().unwrap_or_else(PoisonError::into_inner).reading.remove(asset);
        }
    }
    #[cfg(target_arch = "wasm32")]
    request(asset);
    None
}

#[cfg(not(target_arch = "wasm32"))]
type ReadJob = (AssetRef, u64);
#[cfg(not(target_arch = "wasm32"))]
fn reader() -> &'static Option<std::sync::mpsc::SyncSender<ReadJob>> {
    static READER: OnceLock<Option<std::sync::mpsc::SyncSender<ReadJob>>> = OnceLock::new();
    READER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ReadJob>(64);
        std::thread::Builder::new()
            .name("image-cache-read".into())
            .spawn(move || {
                for (asset, epoch) in rx {
                    let result = disk().read(&asset);
                    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
                    if s.epoch != epoch {
                        continue;
                    }
                    s.reading.remove(&asset);
                    match result {
                        Ok(bytes) => {
                            remember_locked(&mut s, &asset, bytes);
                            drop(s);
                            notify_changed();
                        }
                        Err(_) => {
                            drop(s);
                            request(&asset);
                        }
                    }
                }
            })
            .ok()
            .map(|_| tx)
    })
}

/// Explicit cache clearing never removes a document, its resident originals or server data.
/// Pending downloads may populate new entries again while their session remains open.
pub fn clear() -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    disk().clear()?;
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    s.epoch = s.epoch.wrapping_add(1);
    s.bytes.clear();
    s.reading.clear();
    s.failed.clear();
    drop(s);
    notify_changed();
    Ok(())
}

#[derive(Default)]
struct State {
    loaders: BTreeMap<String, Vec<Weak<Loader>>>,
    pending: BTreeMap<AssetRef, Instant>,
    failed: BTreeMap<AssetRef, (Instant, String, bool)>,
    bytes: BTreeMap<AssetRef, (Arc<Vec<u8>>, u64)>,
    tick: u64,
    reading: std::collections::BTreeSet<AssetRef>,
    epoch: u64,
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
    if let Some(bytes) = memory(asset) {
        return Ok(bytes);
    }
    let bytes = disk().read(asset)?;
    remember(asset, bytes.clone());
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn store(asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
    store_download(disk(), asset, bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn store_download(cache: &DiskCache, asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
    if !asset.verify(bytes) {
        return Err("invalid image checksum or length".into());
    }
    // Persistence is disposable. Verified downloads remain usable on read-only/full disks.
    let _ = cache.write_verified(asset, bytes);
    remember(asset, Arc::new(bytes.to_vec()));
    ready(asset);
    notify_changed();
    Ok(())
}

/// Store an immutable resident buffer, reusing its verified hash and this process's file stamp.
/// Reads still validate payloads; metadata is only a fast path for redundant cache insertion.
#[cfg(not(target_arch = "wasm32"))]
pub fn store_resident(asset: &AssetRef, bytes: &Arc<Vec<u8>>) -> Result<(), String> {
    if !asset.valid() || !asset.same_content(&AssetRef::of_arc(bytes)) {
        return Err("invalid image checksum or length".into());
    }
    match disk().write_arc(asset, bytes) {
        Ok(true) => ready(asset),
        Ok(false) => {}
        Err(_) => {
            remember(asset, bytes.clone());
            ready(asset);
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn load(asset: &AssetRef) -> Result<Arc<Vec<u8>>, String> {
    state()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .bytes
        .get(asset)
        .map(|(bytes, _)| bytes.clone())
        .ok_or_else(|| "image is not cached".into())
}

#[cfg(target_arch = "wasm32")]
pub fn store(asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
    if !asset.verify(bytes) {
        return Err("invalid image checksum or length".into());
    }
    let mut s = state().0.lock().unwrap_or_else(PoisonError::into_inner);
    let held: u64 = s.bytes.keys().map(|a| a.len).sum();
    if held.saturating_add(asset.len) > MEMORY_BUDGET as u64 {
        return Err("browser asset cache budget exceeded".into());
    }
    s.tick = s.tick.wrapping_add(1);
    let tick = s.tick;
    s.bytes.insert(asset.clone(), (Arc::new(bytes.to_vec()), tick));
    drop(s);
    ready(asset);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn disk() -> &'static DiskCache {
    static DISK: OnceLock<DiskCache> = OnceLock::new();
    DISK.get_or_init(|| {
        if let Some(root) = std::env::var_os("FIGCRAFT_ASSET_CACHE_DIR") {
            return DiskCache::new(root.into(), DISK_BUDGET);
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".cache")))
            .unwrap_or_else(|| std::env::temp_dir().join(format!("figcraft-{}", std::process::id())));
        DiskCache::new(base.join("FigCraft").join("assets-v1"), DISK_BUDGET)
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

    pub fn clear(&self) -> Result<(), String> {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        match std::fs::read_dir(&self.root) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) && entry.file_type().is_ok_and(|t| t.is_file()) {
                        std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        *used = Some(HashMap::new());
        self.verified.lock().unwrap_or_else(PoisonError::into_inner).clear();
        Ok(())
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
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        let index = used.get_or_insert_with(|| cache_index(&self.root));
        self.prune(index, None)?;
        let bytes = read_bytes(&self.root.join(asset.cache_key()), asset)?;
        index.insert(asset.cache_key(), (asset.len, std::time::SystemTime::now()));
        Ok(bytes)
    }

    pub fn write(&self, asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
        if !asset.verify(bytes) {
            return Err("invalid image checksum or length".into());
        }
        self.write_verified(asset, bytes)
    }

    fn write_verified(&self, asset: &AssetRef, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if asset.len > self.max {
            return Err("image exceeds the disk cache budget".into());
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
        let index = held.get_or_insert_with(|| cache_index(&self.root));
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
        self.prune(index, Some(&key))
    }

    fn prune(&self, index: &mut HashMap<String, (u64, std::time::SystemTime)>, keep: Option<&str>) -> Result<(), String> {
        let mut total: u64 = index.values().map(|v| v.0).sum();
        while total > self.max || index.len() > 20_000 {
            let Some(oldest) = index.iter().filter(|(key, _)| Some(key.as_str()) != keep).min_by_key(|(_, v)| v.1).map(|(k, v)| (k.clone(), v.0))
            else {
                break;
            };
            match std::fs::remove_file(self.root.join(&oldest.0)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            index.remove(&oldest.0);
            self.verified.lock().unwrap_or_else(PoisonError::into_inner).remove(&oldest.0);
            total = total.saturating_sub(oldest.1);
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn cache_index(root: &std::path::Path) -> HashMap<String, (u64, std::time::SystemTime)> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some((name, (meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH))))
        })
        .collect()
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn review_followup_verified_downloads_survive_disk_failure_but_corruption_does_not() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let cache = DiskCache::new(file.path().into(), 1024); // A file cannot be a cache directory.
        let bytes = b"synthetic memory-only download after disk error";
        let asset = AssetRef::of(bytes);
        assert!(cache.write(&asset, bytes).is_err());
        store_download(&cache, &asset, bytes).unwrap();
        assert_eq!(load(&asset).unwrap().as_slice(), bytes);
        assert_eq!(display(&asset).unwrap().as_slice(), bytes);
        assert!(store_download(&cache, &asset, b"bad checksum").is_err());
        assert_eq!(load(&asset).unwrap().as_slice(), bytes);
    }

    #[test]
    fn warming_host_sources_does_not_signal_a_download_but_valid_downloads_do() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = Arc::new(AtomicUsize::new(0));
        let changed = count.clone();
        let caller = std::thread::current().id();
        let waker: Arc<Waker> = Arc::new(move || {
            if std::thread::current().id() == caller {
                changed.fetch_add(1, Ordering::Relaxed);
            }
        });
        watch(&waker);
        let bytes = Arc::new(b"synthetic host source wake regression".to_vec());
        let asset = AssetRef::of_arc(&bytes);
        store_resident(&asset, &bytes).unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        store(&asset, &bytes).unwrap();
        assert!(count.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn smaller_budget_trims_old_cache_and_clear_preserves_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = AssetRef::of(b"first-image");
        let b = AssetRef::of(b"next-image");
        let old = DiskCache::new(dir.path().into(), 1024);
        old.write(&a, b"first-image").unwrap();
        old.write(&b, b"next-image").unwrap();
        std::fs::write(dir.path().join("keep.figcraft"), b"document").unwrap();
        let cache = DiskCache::new(dir.path().into(), 12);
        let _ = cache.read(&b);
        let index = cache_index(dir.path());
        assert!(index.values().map(|v| v.0).sum::<u64>() <= 12);
        cache.clear().unwrap();
        assert!(cache_index(dir.path()).is_empty());
        assert_eq!(std::fs::read(dir.path().join("keep.figcraft")).unwrap(), b"document");
        cache.write(&a, b"first-image").unwrap();
        assert_eq!(cache.read(&a).unwrap().as_slice(), b"first-image");
    }

    #[test]
    fn display_returns_while_disk_reader_is_blocked_and_wakes_on_completion() {
        let bytes = b"synthetic cold disk-only image";
        let mut asset = AssetRef::of(bytes);
        asset.scope = AssetRef::of(b"paint IO test scope").sha256;
        disk().write(&asset, bytes).unwrap();
        state().0.lock().unwrap().bytes.remove(&asset);
        let held = disk().used.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let a = asset.clone();
        let thread = std::thread::spawn(move || tx.send(display(&a).is_none()).unwrap());
        let result = rx.recv_timeout(Duration::from_secs(2));
        drop(held);
        thread.join().unwrap();
        assert!(result.unwrap(), "paint waited for disk or returned uncached data");
        let until = Instant::now() + Duration::from_secs(5);
        while memory(&asset).is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(display(&asset).unwrap().as_slice(), bytes);
    }

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
