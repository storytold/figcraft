//! Image payloads stored in a [`Document`](crate::Document): either held in memory or backed
//! lazily by a seekable archive on disk (#1313).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// On-demand backing store for encoded image bytes (such as a seekable `.figcraft` ZIP archive
/// with a bounded encoded-byte LRU cache).
pub trait ImageLoader: Send + Sync + std::fmt::Debug {
    /// Stable per-store identity used for O(1) equality checks between lazy handles from the
    /// same backing store.
    fn store_id(&self) -> u64;

    /// Load the encoded bytes for `key` (hitting the store's bounded LRU cache or reading the
    /// single member on demand). Returns `None` if missing, over cap, or hash-mismatched.
    fn load(&self, key: &str) -> Option<Arc<Vec<u8>>>;

    /// Stream the encoded bytes for `key` directly into `out` without retaining them in the
    /// store's LRU cache. Returns `Ok(true)` if the member was streamed, `Ok(false)` if absent.
    fn stream_to(&self, key: &str, out: &mut dyn Write) -> std::io::Result<bool>;

    /// Path of the on-disk file backing this store, when it is backed by a file.
    fn source_path(&self) -> Option<&Path> {
        None
    }

    /// Atomically replace the file at `dest` with `staging`, releasing any open read handle
    /// held by this store first so replacement succeeds on Windows as well as Unix, and
    /// reopening the new file before returning.
    fn replace_backing_file(&self, staging: &Path, dest: &Path) -> std::io::Result<()> {
        std::fs::rename(staging, dest)
    }

    /// Release any open read handle to the backing file (used when multiple stores share a path
    /// being replaced in place).
    fn release_backing_handle(&self) {}

    /// Reopen the backing file after an in-place replacement completes.
    fn reopen_backing_file(&self, _path: &Path) {}
}

#[derive(Clone, Debug)]
enum ImageSource {
    InMemory(Arc<Vec<u8>>),
    Lazy { key: String, byte_len: u64, loader: Arc<dyn ImageLoader> },
}

/// Raw image bytes (encoded PNG/JPEG/…) kept by content hash, either in memory or loaded on
/// demand from a backing archive.
#[derive(Clone, Debug)]
pub struct ImageData {
    source: ImageSource,
    pub width: u32,
    pub height: u32,
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        if self.width != other.width || self.height != other.height {
            return false;
        }
        match (&self.source, &other.source) {
            (ImageSource::InMemory(a), ImageSource::InMemory(b)) => Arc::ptr_eq(a, b) || a == b,
            (ImageSource::Lazy { key: ka, byte_len: la, loader: sa }, ImageSource::Lazy { key: kb, byte_len: lb, loader: sb }) => {
                if sa.store_id() == sb.store_id() && ka == kb {
                    return true;
                }
                if la == lb && Self::claims_content_hash(ka) && ka == kb {
                    return true;
                }
                sa.load(ka) == sb.load(kb)
            }
            (ImageSource::InMemory(mem), ImageSource::Lazy { key, byte_len, loader })
            | (ImageSource::Lazy { key, byte_len, loader }, ImageSource::InMemory(mem)) => {
                if mem.len() as u64 != *byte_len {
                    return false;
                }
                loader.load(key).is_some_and(|b| b.as_slice() == mem.as_slice())
            }
        }
    }
}

impl ImageData {
    /// Largest encoded image a document takes from outside: an image member of a `.fig` or
    /// `.sketch` file, an image file a REST import reads, an image entry from a live session.
    pub const MAX_BYTES: u64 = 256 * 1024 * 1024;

    /// Create an in-memory image entry.
    pub fn new(bytes: Arc<Vec<u8>>, width: u32, height: u32) -> Self {
        Self { source: ImageSource::InMemory(bytes), width, height }
    }

    /// Create a lazy image entry backed by `loader` under `key`.
    pub fn lazy(key: String, byte_len: u64, width: u32, height: u32, loader: Arc<dyn ImageLoader>) -> Self {
        Self { source: ImageSource::Lazy { key, byte_len, loader }, width, height }
    }

    /// Encoded image bytes, loading on demand from the backing store if lazy.
    pub fn bytes(&self) -> Option<Arc<Vec<u8>>> {
        match &self.source {
            ImageSource::InMemory(b) => Some(Arc::clone(b)),
            ImageSource::Lazy { key, loader, .. } => loader.load(key),
        }
    }

    /// Borrow the in-memory bytes without triggering disk I/O (`None` when lazy).
    pub fn as_in_memory(&self) -> Option<&Arc<Vec<u8>>> {
        match &self.source {
            ImageSource::InMemory(b) => Some(b),
            ImageSource::Lazy { .. } => None,
        }
    }

    /// Declared encoded byte length without triggering disk I/O.
    pub fn byte_len(&self) -> u64 {
        match &self.source {
            ImageSource::InMemory(b) => b.len() as u64,
            ImageSource::Lazy { byte_len, .. } => *byte_len,
        }
    }

    /// Whether this image's encoded bytes are held lazily on disk rather than in memory.
    pub fn is_lazy(&self) -> bool {
        matches!(self.source, ImageSource::Lazy { .. })
    }

    /// The backing loader if this image is lazy.
    pub fn loader(&self) -> Option<&Arc<dyn ImageLoader>> {
        match &self.source {
            ImageSource::InMemory(_) => None,
            ImageSource::Lazy { loader, .. } => Some(loader),
        }
    }

    /// Convert a lazy handle into an in-memory copy (used to pin undo-only images before
    /// replacing their backing archive on disk).
    pub fn materialize(&self) -> Option<Self> {
        match &self.source {
            ImageSource::InMemory(_) => Some(self.clone()),
            ImageSource::Lazy { key, loader, .. } => {
                let bytes = loader.load(key)?;
                Some(Self::new(bytes, self.width, self.height))
            }
        }
    }

    /// Stream encoded bytes straight into `out`. For lazy images this streams directly from the
    /// backing archive without retaining the full image in the LRU cache.
    pub fn stream_to(&self, out: &mut dyn Write) -> std::io::Result<bool> {
        match &self.source {
            ImageSource::InMemory(b) => {
                out.write_all(b)?;
                Ok(true)
            }
            ImageSource::Lazy { key, loader, .. } => loader.stream_to(key, out),
        }
    }

    /// Whether `key` claims to be a content hash: 64 hex digits, the shape of the sha256 hex
    /// `figcraft_format::image_hash` makes. Bytes stored under such a key must hash to it.
    pub fn claims_content_hash(key: &str) -> bool {
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
    }

    /// Whether `key` can name an image: 1–128 ASCII letters, digits, `-` and `_` (content hashes,
    /// the svg importer's FNV keys, the REST importer's `rest-image-missing` placeholder). A key
    /// becomes an archive member name (`images/<key>`) when the file is saved, so only these are
    /// read back from a file or taken from a collaborator: never `/`, `.` or other path text.
    pub fn is_valid_key(key: &str) -> bool {
        (1..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}
