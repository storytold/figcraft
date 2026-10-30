//! Import of `.fig` files (best effort), written from public format knowledge only.
//!
//! A `.fig` file is a zip archive holding `canvas.fig` (the document), `meta.json`, a thumbnail
//! and `images/<hash>` files. `canvas.fig` (also accepted on its own) is an 8-byte `fig-…` magic,
//! a little-endian `u32` version and length-prefixed chunks, each compressed with deflate or
//! zstd: a binary schema in the kiwi format ([`kiwi`]), then a message encoded with it. Every file
//! carries its own schema, so the message is decoded generically and read by field name: a list
//! of node changes (each with its id, parent id and position among its siblings) that make up the
//! layer tree, plus blobs (vector outlines and editable vector networks, [`network`], and the bytes
//! of images that travel inside the message, [`embedded_images`]). [`map`]
//! turns that into the REST API file JSON shape and the REST importer builds the document, so both
//! imports share one mapping. Layer kinds the editor doesn't have (sticky notes, connectors,
//! tables, …) come in as the closest frames, text and vectors. Anything not understood is skipped
//! with a warning; malformed input never panics.

#[cfg(test)]
mod auto_layout_sizing_tests;
#[cfg(test)]
mod connector_label_tests;
#[cfg(test)]
mod deep_import_tests;
#[cfg(test)]
mod expansion_cost_tests;
mod export;
#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod interop_tests;
pub mod kiwi;
#[cfg(test)]
mod layer_cap_tests;
mod map;
mod network;
#[cfg(test)]
mod prop_value_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod source_identity_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_paragraph_tests;
mod text_path;
#[cfg(test)]
mod text_path_tests;
#[cfg(test)]
mod text_skip_ink_tests;
#[cfg(test)]
mod text_style_tests;
#[cfg(test)]
mod text_wrap_tests;
#[cfg(test)]
mod value_budget_tests;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use figcraft_doc::{Document, FileKind};

pub use export::export;

use crate::FormatError;
use kiwi::{Kv, Schema};

/// Largest `canvas.fig` member read, and largest decompressed chunk.
const MAX_CANVAS: u64 = 256 * 1024 * 1024;
const MAX_CHUNK: u64 = 512 * 1024 * 1024;
/// Largest image member read.
const MAX_IMAGE: u64 = figcraft_doc::ImageData::MAX_BYTES;
/// Total decompressed ZIP image bytes read per import: the native image budget (`IMAGE_BUDGET`) on
/// 64-bit targets, lower on 32-bit targets to leave address space for the canvas, layers and
/// decoded pixels.
#[cfg(not(target_pointer_width = "32"))]
const MAX_IMAGE_BYTES: u64 = crate::IMAGE_BUDGET.bytes;
#[cfg(target_pointer_width = "32")]
const MAX_IMAGE_BYTES: u64 = 1024 * 1024 * 1024;
/// Most chunks read from `canvas.fig`.
const MAX_CHUNKS: usize = 16;
/// Most layers imported from a `.fig` file: one per [`PACKED_BYTES_PER_LAYER`] bytes of its
/// compressed canvas, at least the REST import's cap and at most [`MAX_FIG_NODES`]. Instances
/// expand into a layer per layer of their component, and chunks inflate up to [`MAX_CHUNK`], so the
/// budget follows what the file takes, not what it unpacks to: a small file can't claim a large
/// count, however it's built; a large one gets room. Design systems are mostly instances, which
/// pack tightly: a 7.8 MB canvas stopped at 244 248 layers at 32 bytes a layer (#981). At 8, files
/// under about 1.6 MB keep the floor and the ceiling comes at a 16 MB canvas (about 11 GB and 40 s
/// to import 2 million synthetic layers; memory grows with the count, about 5.4 KB a layer).
const PACKED_BYTES_PER_LAYER: usize = 8;
const MAX_FIG_NODES: usize = 2_000_000;

/// Most layers imported from a `.fig` file whose compressed canvas is `packed` bytes.
pub(crate) fn layer_budget(packed: usize) -> usize {
    (packed / PACKED_BYTES_PER_LAYER).clamp(crate::rest::MAX_NODES, MAX_FIG_NODES)
}

/// The canvas header of a FigJam whiteboard (a `.jam`); a `.fig` has `fig-kiwi`.
pub const JAM_HEADER: &[u8] = b"fig-jam.";

/// Whether `bytes` look like a `.fig` file: a zip holding `canvas.fig`, or a bare canvas.
pub fn is_fig(bytes: &[u8]) -> bool {
    bytes.starts_with(b"fig-") || (bytes.starts_with(b"PK") && zip_has_canvas(bytes))
}

fn zip_has_canvas(bytes: &[u8]) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok_and(|z| z.file_names().any(|n| n == "canvas.fig"))
}

/// Import `bytes` when they are a `.fig` file; `None` when they are something else.
pub fn try_import(bytes: &[u8], name: &str) -> Option<Result<(Document, Vec<String>), FormatError>> {
    is_fig(bytes).then(|| import(bytes, name))
}

/// Build a document from a `.fig` file. Returns it with the warnings about what was skipped or
/// approximated. `name`: the document name when the file doesn't carry one.
pub fn import(bytes: &[u8], name: &str) -> Result<(Document, Vec<String>), FormatError> {
    import_with_image_limit(bytes, name, MAX_IMAGE_BYTES)
}

fn import_with_image_limit(bytes: &[u8], name: &str, image_limit: u64) -> Result<(Document, Vec<String>), FormatError> {
    let mut archive = Archive::open(bytes)?;
    archive.image_budget.remaining.set(image_limit);
    // The budgets follow the compressed canvas, which the file itself bounds (its zip member may
    // inflate too).
    let packed = archive.canvas.len().min(bytes.len());
    // FigJam writes its whiteboards with this header: they open as boards.
    let board = archive.canvas.starts_with(JAM_HEADER);
    // Each stage's input is dropped as soon as the next one is built: in a large file every one
    // of them takes gigabytes, and holding them all at once runs out of memory.
    let (schema, message) = decode_canvas(&std::mem::take(&mut archive.canvas))?;
    let root = schema.find("Message").ok_or_else(|| corrupt("the schema has no Message definition"))?;
    let msg = schema.decode_message_within(root, &message, kiwi::value_budget(packed)).map_err(|e| corrupt(e.to_string()))?;
    let max_nodes = layer_budget(packed);
    drop(message);
    let file_name = archive.file_name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(name);
    let (json, mut warnings, networks) = map::to_rest(&msg, file_name, max_nodes);
    // The message goes before the layers are built, so the images it carries are copied out first.
    let embedded = embedded_copies(&msg);
    drop(msg);
    let images = |r: &str| archive.image(r).or_else(|| embedded.get(r).map(|b| b.to_vec()));
    let (mut doc, more) = crate::rest::import_with_networks(&json, &images, networks, max_nodes)?;
    let skipped = archive.image_budget.skipped.get();
    if skipped > 0 {
        warnings.push(format!("{skipped} ZIP image(s) skipped by the total ZIP image byte budget ({image_limit} bytes)"));
    }
    if board {
        doc.kind = FileKind::Board;
    }
    // Both stages stop at the same cap and say so in the same words: one note (#981).
    for w in more.into_iter().filter_map(map::adapt_warning) {
        let base = w.rsplit_once(" (×").map_or(w.as_str(), |(b, _)| b);
        if !warnings.iter().any(|seen| seen == base) {
            warnings.push(w);
        }
    }
    Ok((doc, warnings))
}

/// A font style under the name the format uses: the standard weight names as words ("SemiBold"
/// → "Semi Bold", "ExtraBoldItalic" → "Extra Bold Italic", "Regular Italic" → "Italic"). Other
/// names stay as they are.
pub(crate) fn style_name(style: &str) -> String {
    const WEIGHTS: [&str; 12] =
        ["Thin", "Extra Light", "Ultra Light", "Light", "Regular", "Medium", "Semi Bold", "Demi Bold", "Bold", "Extra Bold", "Ultra Bold", "Black"];
    let compact = |s: &str| s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect::<String>();
    let key = compact(style);
    let (base, italic) = match key.strip_suffix("italic") {
        Some(b) => (b, true),
        None => (key.as_str(), false),
    };
    let weight = if base.is_empty() && italic { Some("Regular") } else { WEIGHTS.into_iter().find(|w| compact(w) == base) };
    match (weight, italic) {
        (None, _) => style.to_string(),
        (Some("Regular"), true) => "Italic".into(),
        (Some(w), true) => format!("{w} Italic"),
        (Some(w), false) => w.into(),
    }
}

fn corrupt(msg: impl Into<String>) -> FormatError {
    FormatError::Corrupt(format!(".fig file: {}", msg.into()))
}

/// Image bytes that travel inside the message, by image hash (lowercase hex): a paint's
/// `image.dataBlob` indexes the message's `blobs`. Copies written by plugins and other tools embed
/// their images this way, and a bare canvas (what the clipboard carries) has no `images/`. The
/// walk is iterative (no recursion on nested data) and skips blobs larger than [`MAX_IMAGE`].
/// Returns the index of each hash's blob.
fn embedded_images(msg: &Kv) -> BTreeMap<String, usize> {
    let blobs = msg.get("blobs").map(Kv::list).unwrap_or_default();
    let mut found = BTreeMap::new();
    let mut pending = vec![msg];
    while let Some(value) = pending.pop() {
        match value {
            Kv::Object(fields) => {
                if let Some((hash, index)) = value.get("image").and_then(|image| embedded_image(image, blobs)) {
                    found.entry(hash).or_insert(index);
                }
                pending.extend(fields.iter().filter(|(name, _)| *name != "blobs").map(|(_, field)| field));
            }
            Kv::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    found
}

/// The images that travel inside the message ([`embedded_images`]), copied out of it once per
/// blob: every hash naming the same blob shares its copy. A file can point any number of hashes
/// at one large blob, and a copy for each would multiply it (#1059).
fn embedded_copies(msg: &Kv) -> BTreeMap<String, Arc<[u8]>> {
    let blobs = msg.get("blobs").map(Kv::list).unwrap_or_default();
    let mut copies: BTreeMap<usize, Arc<[u8]>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for (hash, index) in embedded_images(msg) {
        let Some(bytes) = blob(blobs, index) else { continue };
        out.insert(hash, Arc::clone(copies.entry(index).or_insert_with(|| Arc::from(bytes))));
    }
    out
}

/// The hash and blob index of an `Image` value whose `dataBlob` points at a usable blob.
fn embedded_image(image: &Kv, blobs: &[Kv]) -> Option<(String, usize)> {
    let hash = image.get("hash").and_then(Kv::bytes).filter(|h| !h.is_empty() && h.len() <= 64)?;
    let Some(Kv::Uint(index)) = image.get("dataBlob") else { return None };
    let index = usize::try_from(*index).ok()?;
    let bytes = blob(blobs, index)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE {
        return None;
    }
    Some((hash.iter().map(|b| format!("{b:02x}")).collect(), index))
}

/// The bytes of blob `index`.
fn blob<'k>(blobs: &'k [Kv], index: usize) -> Option<&'k [u8]> {
    blobs.get(index)?.get("bytes").and_then(Kv::bytes)
}

/// Bytes left for ZIP image reads, including damaged images. The REST resolver caches references,
/// including failures, so each referenced member is read or counted as skipped only once.
struct ZipImageBudget {
    remaining: Cell<u64>,
    skipped: Cell<usize>,
}

/// The parts of a `.fig` archive the import reads.
struct Archive<'b> {
    canvas: Vec<u8>,
    file_name: Option<String>,
    /// `images/<hash>` members (or nothing for a bare canvas).
    zip: Option<zip::ZipArchive<std::io::Cursor<&'b [u8]>>>,
    image_budget: ZipImageBudget,
}

impl<'b> Archive<'b> {
    fn open(bytes: &'b [u8]) -> Result<Self, FormatError> {
        let image_budget = ZipImageBudget { remaining: Cell::new(MAX_IMAGE_BYTES), skipped: Cell::new(0) };
        if bytes.starts_with(b"fig-") {
            return Ok(Archive { canvas: bytes.to_vec(), file_name: None, zip: None, image_budget });
        }
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| corrupt(e.to_string()))?;
        let canvas = crate::read_member(&mut z, "canvas.fig", MAX_CANVAS)?;
        let file_name = crate::read_member(&mut z, "meta.json", 1 << 20)
            .ok()
            .and_then(|m| serde_json::from_slice::<serde_json::Value>(&m).ok())
            .and_then(|m| m.get("file_name").and_then(serde_json::Value::as_str).map(str::to_string));
        Ok(Archive { canvas, file_name, zip: Some(z), image_budget })
    }

    /// Bytes of image `hash` (lowercase hex) from `images/`.
    fn image(&self, hash: &str) -> Option<Vec<u8>> {
        let budget = &self.image_budget;
        if hash.is_empty() || hash.len() > 128 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // A clone shares the parsed directory and the bytes; the resolver only has shared access.
        let mut z = self.zip.clone()?;
        let f = z.by_name(&format!("images/{hash}")).ok()?;
        let size = f.size();
        if size > MAX_IMAGE {
            return None;
        }
        let remaining = budget.remaining.get();
        if size > remaining {
            budget.skipped.set(budget.skipped.get() + 1);
            return None;
        }
        let checksum = f.crc32();
        let mut data = Vec::new();
        // Probe past the per-image cap only when the total budget has room. Even a dishonest
        // size cannot make the read exceed the remaining total; failed reads keep their charge.
        let limit = remaining.min(MAX_IMAGE + 1);
        let read = f.take(limit).read_to_end(&mut data);
        let used = data.len() as u64;
        budget.remaining.set(remaining - used);
        if used > size && used == remaining {
            budget.skipped.set(budget.skipped.get() + 1);
        }
        if read.is_err() || used != size {
            return None;
        }
        // `take` can stop exactly at the budget without asking ZIP for EOF (where it checks CRC).
        // Check it here too, without spending an extra byte on an overflow probe.
        if used == limit {
            let mut crc = flate2::Crc::new();
            crc.update(&data);
            if crc.sum() != checksum {
                return None;
            }
        }
        Some(data)
    }
}

/// The schema and the raw message of a `canvas.fig`.
fn decode_canvas(canvas: &[u8]) -> Result<(Schema, Vec<u8>), FormatError> {
    let chunks = chunks(canvas)?;
    let mut it = chunks.into_iter();
    let (Some(schema), Some(message)) = (it.next(), it.next()) else {
        return Err(corrupt("canvas.fig needs a schema and a message chunk"));
    };
    let schema = Schema::decode(&inflate(schema)?).map_err(|e| corrupt(e.to_string()))?;
    Ok((schema, inflate(message)?))
}

/// The length-prefixed chunks after the 12-byte header.
fn chunks(canvas: &[u8]) -> Result<Vec<&[u8]>, FormatError> {
    if canvas.len() < 12 || !canvas.starts_with(b"fig-") {
        return Err(corrupt("canvas.fig has no header"));
    }
    let mut out = Vec::new();
    let mut pos = 12usize;
    while pos < canvas.len() && out.len() < MAX_CHUNKS {
        let len = canvas.get(pos..pos + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| corrupt("truncated chunk"))?;
        let start = pos + 4;
        let end = start.checked_add(len as usize).filter(|e| *e <= canvas.len()).ok_or_else(|| corrupt("truncated chunk"))?;
        out.push(canvas.get(start..end).unwrap_or_default());
        pos = end;
    }
    Ok(out)
}

/// Decompress a chunk: zstd (by its magic number), else raw deflate, else zlib.
fn inflate(data: &[u8]) -> Result<Vec<u8>, FormatError> {
    fn capped(r: impl Read) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        r.take(MAX_CHUNK + 1).read_to_end(&mut out).ok()?;
        (out.len() as u64 <= MAX_CHUNK).then_some(out)
    }
    if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        let mut src = data;
        let dec = ruzstd::decoding::StreamingDecoder::new(&mut src).map_err(|e| corrupt(format!("zstd: {e}")))?;
        return capped(dec).ok_or_else(|| corrupt("damaged or oversized zstd chunk"));
    }
    capped(flate2::read::DeflateDecoder::new(data))
        .or_else(|| capped(flate2::read::ZlibDecoder::new(data)))
        .ok_or_else(|| corrupt("damaged or oversized deflate chunk"))
}

/// Decoded message `msg` as JSON (byte arrays as their length), for looking at files.
#[cfg(test)]
pub(crate) fn debug_json(v: &kiwi::Kv) -> serde_json::Value {
    use kiwi::Kv;
    use serde_json::{Value, json};
    match v {
        Kv::Bool(b) => Value::Bool(*b),
        Kv::Int(i) => json!(i),
        Kv::Uint(u) => json!(u),
        Kv::Float(f) => json!(f),
        Kv::Str(s) => json!(s),
        Kv::Bytes(b) => json!(format!("<{} bytes>", b.len())),
        Kv::Enum(e) => json!(e),
        Kv::Array(a) => Value::Array(a.iter().map(debug_json).collect()),
        Kv::Object(f) => Value::Object(f.iter().map(|(k, v)| (k.to_string(), debug_json(v))).collect()),
    }
}
