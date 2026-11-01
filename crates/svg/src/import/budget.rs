//! Resource budgets for what an SVG expands into while it is parsed: compressed (SVGZ) input,
//! and SVG documents embedded in `<image>` data URIs (themselves possibly compressed or nested).
//! usvg decompresses with no cap and parses embedded SVGs with no shared depth or byte limit, so
//! both are done here: a bounded gunzip before usvg sees the bytes, and our own data resolver.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use usvg::{ImageHrefResolver, ImageKind, Options};

/// Largest document, once decompressed.
const MAX_DOCUMENT_BYTES: usize = 64 << 20;
/// Largest total of all embedded SVG documents, once decompressed.
const MAX_EMBEDDED_BYTES: usize = 32 << 20;
/// Most SVG documents embedded inside one another.
const MAX_EMBEDDED_DEPTH: usize = 4;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub document: usize,
    pub embedded: usize,
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { document: MAX_DOCUMENT_BYTES, embedded: MAX_EMBEDDED_BYTES, depth: MAX_EMBEDDED_DEPTH }
    }
}

fn gzipped(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// `data` as plain text bytes: gunzipped when compressed, never past `limit` bytes.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if !gzipped(data) {
        return if data.len() > limit { Err(format!("SVG larger than {} MiB", limit >> 20)) } else { Ok(data.to_vec()) };
    }
    let mut out = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    flate2::read::GzDecoder::new(data).take(cap).read_to_end(&mut out).map_err(|_| "malformed compressed SVG".to_string())?;
    if out.len() > limit {
        return Err(format!("compressed SVG expands past {} MiB", limit >> 20));
    }
    Ok(out)
}

/// The document `data` (SVG or SVGZ) as plain bytes, within the document budget.
pub(super) fn document(data: &[u8], limits: &Limits) -> Result<Vec<u8>, String> {
    // The caller already holds a plain document; only what compression expands to is bounded.
    if gzipped(data) { inflate(data, limits.document) } else { Ok(data.to_vec()) }
}

/// What the embedded SVGs of one import have used so far.
struct Used {
    bytes: AtomicUsize,
    depth: AtomicUsize,
}

fn raster(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG")
        || data.starts_with(&[0xff, 0xd8, 0xff])
        || data.starts_with(b"GIF8")
        || (data.len() > 12 && data.get(8..12) == Some(b"WEBP"))
}

/// An embedded SVG document, if the budgets allow it.
fn embedded(used: &Used, limits: &Limits, data: &[u8], opts: &Options) -> Option<ImageKind> {
    if used.depth.load(Ordering::Relaxed) >= limits.depth {
        return None;
    }
    let left = limits.embedded.saturating_sub(used.bytes.load(Ordering::Relaxed));
    let plain = inflate(data, left).ok()?;
    used.bytes.fetch_add(plain.len(), Ordering::Relaxed);
    if super::too_deep(&plain) {
        return None;
    }
    used.depth.fetch_add(1, Ordering::Relaxed);
    let tree = usvg::Tree::from_data_nested(&plain, opts);
    used.depth.fetch_sub(1, Ordering::Relaxed);
    tree.ok().map(ImageKind::SVG)
}

/// Resolve `<image>` data URIs: rasters as they are, SVGs within `limits`; nothing else.
pub(super) fn resolver(limits: Limits) -> ImageHrefResolver<'static> {
    let used = Arc::new(Used { bytes: AtomicUsize::new(0), depth: AtomicUsize::new(0) });
    ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, opts| match mime {
            "image/jpg" | "image/jpeg" => Some(ImageKind::JPEG(data)),
            "image/png" => Some(ImageKind::PNG(data)),
            "image/gif" => Some(ImageKind::GIF(data)),
            "image/webp" => Some(ImageKind::WEBP(data)),
            "image/svg+xml" => embedded(&used, &limits, &data, opts),
            "text/plain" if raster(&data) => match data.first() {
                Some(0x89) => Some(ImageKind::PNG(data)),
                Some(0xff) => Some(ImageKind::JPEG(data)),
                Some(b'G') => Some(ImageKind::GIF(data)),
                _ => Some(ImageKind::WEBP(data)),
            },
            "text/plain" => embedded(&used, &limits, &data, opts),
            _ => None,
        }),
        // Linked files are never read.
        resolve_string: Box::new(|_, _| None),
    }
}
