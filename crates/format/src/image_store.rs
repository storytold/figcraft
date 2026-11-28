//! On-demand image store backed by a seekable `.figcraft` ZIP archive with a bounded
//! encoded-byte LRU cache (#1313).
//!
//! Opening a large `.figcraft` file indexes its `images/<key>` entries without loading encoded
//! bytes into memory. Encoded bytes are read lazily when an image is drawn, exported, or edited,
//! verified against content hashes when `ImageData::claims_content_hash(key)` holds, and kept in
//! a bounded LRU cache so memory stays flat even on multi-gigabyte archives.
//!
//! During streaming pass-through save (`stream_to`), lazy members are copied chunk-by-chunk
//! straight from the source archive without polluting the LRU cache. When saving over the same
//! file, `replace_backing_file` holds the store mutex, drops the open file handle before rename
//! (required on Windows), and reopens the new archive before releasing the lock.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use figcraft_doc::{ImageData, ImageLoader};
use sha2::Digest;

use crate::MAX_MEMBER;

/// Default byte budget of the per-file encoded image LRU cache (256 MiB).
pub const DEFAULT_ENCODED_CACHE_BYTES: u64 = 256 * 1024 * 1024;

/// Maximum number of encoded image entries held simultaneously in one store's LRU cache.
const MAX_ENCODED_CACHE_ENTRIES: usize = 4096;

/// Chunk size used when streaming an image member directly to a writer.
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);

/// Bounded LRU cache of encoded image bytes.
#[derive(Default)]
pub(crate) struct EncodedLru {
    entries: HashMap<String, (Arc<Vec<u8>>, u64)>,
    bytes: u64,
    max_bytes: u64,
    tick: u64,
}

impl EncodedLru {
    pub(crate) fn new(max_bytes: u64) -> Self {
        Self { entries: HashMap::new(), bytes: 0, max_bytes, tick: 0 }
    }

    pub(crate) fn get(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.tick = self.tick.saturating_add(1);
        let tick = self.tick;
        self.entries.get_mut(key).map(|(data, used)| {
            *used = tick;
            Arc::clone(data)
        })
    }

    pub(crate) fn insert(&mut self, key: String, data: Arc<Vec<u8>>) {
        let size = data.len() as u64;
        self.remove(&key);
        while !self.entries.is_empty() && (self.bytes.saturating_add(size) > self.max_bytes || self.entries.len() >= MAX_ENCODED_CACHE_ENTRIES) {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else {
                break;
            };
            self.remove(&oldest);
        }
        self.tick = self.tick.saturating_add(1);
        self.entries.insert(key, (data, self.tick));
        self.bytes = self.bytes.saturating_add(size);
    }

    fn remove(&mut self, key: &str) {
        if let Some((data, _)) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(data.len() as u64);
        }
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

struct StoreInner {
    archive: Option<zip::ZipArchive<File>>,
    lru: EncodedLru,
    max_member: u64,
    #[cfg(test)]
    disk_reads: u64,
}

impl StoreInner {
    fn open_archive(path: &Path) -> std::io::Result<zip::ZipArchive<File>> {
        let file = File::open(path)?;
        zip::ZipArchive::new(file).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    fn ensure_archive(&mut self, path: &Path) -> std::io::Result<&mut zip::ZipArchive<File>> {
        if self.archive.is_none() {
            self.archive = Some(Self::open_archive(path)?);
        }
        self.archive.as_mut().ok_or_else(|| std::io::Error::other("image archive unavailable"))
    }

    fn find_member_index(archive: &zip::ZipArchive<File>, key: &str) -> Option<usize> {
        let primary = format!("images/{key}");
        archive.index_for_name(&primary).or_else(|| {
            let fallback = format!("library/images/{key}");
            archive.index_for_name(&fallback)
        })
    }

    fn read_member_bounded(&mut self, path: &Path, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        if !ImageData::is_valid_key(key) {
            return Ok(None);
        }
        let max_member = self.max_member;
        let archive = self.ensure_archive(path)?;
        let Some(index) = Self::find_member_index(archive, key) else {
            return Ok(None);
        };
        let member = archive.by_index(index).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        if member.size() > max_member {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} exceeds member limit")));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(member.size().min(16 * 1024 * 1024)).unwrap_or(0));
        {
            let mut limited = member.take(max_member.saturating_add(1));
            limited.read_to_end(&mut bytes)?;
        }
        if bytes.len() as u64 > max_member {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} exceeds member limit")));
        }
        if ImageData::claims_content_hash(key) && crate::image_hash(&bytes) != key {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} failed content-hash check")));
        }
        #[cfg(test)]
        {
            self.disk_reads = self.disk_reads.saturating_add(1);
        }
        Ok(Some(bytes))
    }

    fn stream_member_bounded(&mut self, path: &Path, key: &str, out: &mut dyn Write) -> std::io::Result<bool> {
        if !ImageData::is_valid_key(key) {
            return Ok(false);
        }
        if let Some(cached) = self.lru.get(key) {
            out.write_all(&cached)?;
            return Ok(true);
        }
        let max_member = self.max_member;
        let verify_hash = ImageData::claims_content_hash(key);
        let mut hasher = verify_hash.then(sha2::Sha256::new);
        {
            let archive = self.ensure_archive(path)?;
            let Some(index) = Self::find_member_index(archive, key) else {
                return Ok(false);
            };
            let mut member = archive.by_index(index).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            if member.size() > max_member {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} exceeds member limit")));
            }
            let mut total: u64 = 0;
            let mut buf = [0u8; STREAM_CHUNK_BYTES];
            loop {
                let n = member.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                total = total.saturating_add(n as u64);
                if total > max_member {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} exceeds member limit")));
                }
                if let Some(h) = hasher.as_mut() {
                    h.update(&buf[..n]);
                }
                out.write_all(&buf[..n])?;
            }
        }
        if let Some(h) = hasher {
            let digest: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if digest != key {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("image {key} failed content-hash check")));
            }
        }
        #[cfg(test)]
        {
            self.disk_reads = self.disk_reads.saturating_add(1);
        }
        Ok(true)
    }
}

/// Seekable `.figcraft` archive store for on-demand image loading and streaming save pass-through.
pub struct ZipImageStore {
    id: u64,
    path: PathBuf,
    inner: Mutex<StoreInner>,
}

impl std::fmt::Debug for ZipImageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipImageStore").field("id", &self.id).field("path", &self.path).finish()
    }
}

impl ZipImageStore {
    /// Wrap an already-opened seekable `.figcraft` ZIP archive with a bounded encoded LRU cache.
    pub fn new(path: PathBuf, archive: zip::ZipArchive<File>, cache_bytes: u64) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed),
            path,
            inner: Mutex::new(StoreInner {
                archive: Some(archive),
                lru: EncodedLru::new(cache_bytes),
                max_member: MAX_MEMBER,
                #[cfg(test)]
                disk_reads: 0,
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn cached_bytes(&self) -> u64 {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).lru.bytes()
    }

    #[cfg(test)]
    pub(crate) fn cached_entries(&self) -> usize {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).lru.len()
    }

    #[cfg(test)]
    pub(crate) fn disk_reads(&self) -> u64 {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner).disk_reads
    }
}

impl ImageLoader for ZipImageStore {
    fn store_id(&self) -> u64 {
        self.id
    }

    fn load(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hit) = inner.lru.get(key) {
            return Some(hit);
        }
        let bytes = inner.read_member_bounded(&self.path, key).ok()??;
        let arc = Arc::new(bytes);
        inner.lru.insert(key.to_string(), Arc::clone(&arc));
        Some(arc)
    }

    fn stream_to(&self, key: &str, out: &mut dyn Write) -> std::io::Result<bool> {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.stream_member_bounded(&self.path, key, out)
    }

    fn source_path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn replace_backing_file(&self, staging: &Path, dest: &Path) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Drop the open handle before rename so Windows MoveFileExW succeeds cleanly.
        inner.archive = None;
        match std::fs::rename(staging, dest) {
            Ok(()) => {
                inner.archive = StoreInner::open_archive(dest).ok();
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_file(staging);
                inner.archive = StoreInner::open_archive(&self.path).ok();
                Err(e)
            }
        }
    }

    fn release_backing_handle(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.archive = None;
    }

    fn reopen_backing_file(&self, path: &Path) {
        let mut inner = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.archive = StoreInner::open_archive(path).ok();
    }
}
