//! Import of `.fig` files (best effort), written from public format knowledge only.
//!
//! A `.fig` file is a zip archive holding `canvas.fig` (the document), `meta.json`, a thumbnail
//! and `images/<hash>` files. `canvas.fig` (also accepted on its own) is an 8-byte `fig-…` magic,
//! a little-endian `u32` version and length-prefixed chunks, each compressed with deflate or
//! zstd: a binary schema in the kiwi format ([`kiwi`]), then a message encoded with it. Every file
//! carries its own schema, so the message is decoded generically and read by field name: a list
//! of node changes (each with its id, parent id and position among its siblings) that make up the
//! layer tree, plus blobs (vector outlines and editable vector networks, [`network`]). [`map`]
//! turns that into the REST API file JSON shape and the REST importer builds the document, so both
//! imports share one mapping. Layer kinds the editor doesn't have (sticky notes, connectors,
//! tables, …) come in as the closest frames, text and vectors. Anything not understood is skipped
//! with a warning; malformed input never panics.

mod export;
#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod interop_tests;
pub mod kiwi;
mod map;
mod network;
#[cfg(test)]
mod tests;

use std::io::Read;

use figcraft_doc::Document;

pub use export::export;

use crate::FormatError;
use kiwi::Schema;

/// Largest `canvas.fig` member read, and largest decompressed chunk.
const MAX_CANVAS: u64 = 256 * 1024 * 1024;
const MAX_CHUNK: u64 = 512 * 1024 * 1024;
/// Largest image member read.
const MAX_IMAGE: u64 = 256 * 1024 * 1024;
/// Most chunks read from `canvas.fig`.
const MAX_CHUNKS: usize = 16;
/// Most layers imported from a `.fig` file: one per [`PACKED_BYTES_PER_LAYER`] bytes of its
/// compressed canvas, at least the REST import's cap and at most [`MAX_FIG_NODES`]. Instances
/// expand into a layer per layer of their component, and chunks inflate up to [`MAX_CHUNK`], so the
/// budget follows what the file takes, not what it unpacks to: a small file can't claim a large
/// count, however it's built; a large one gets room.
const PACKED_BYTES_PER_LAYER: usize = 32;
const MAX_FIG_NODES: usize = 2_000_000;

/// Most layers imported from a `.fig` file whose compressed canvas is `packed` bytes.
pub(crate) fn layer_budget(packed: usize) -> usize {
    (packed / PACKED_BYTES_PER_LAYER).clamp(crate::rest::MAX_NODES, MAX_FIG_NODES)
}

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
    let mut archive = Archive::open(bytes)?;
    // The budgets follow the compressed canvas, which the file itself bounds (its zip member may
    // inflate too).
    let packed = archive.canvas.len().min(bytes.len());
    // Each stage's input is dropped as soon as the next one is built: in a large file every one
    // of them takes gigabytes, and holding them all at once runs out of memory.
    let (schema, message) = decode_canvas(&std::mem::take(&mut archive.canvas))?;
    let root = schema.find("Message").ok_or_else(|| corrupt("the schema has no Message definition"))?;
    let msg = schema.decode_message_within(root, &message, kiwi::value_budget(packed)).map_err(|e| corrupt(e.to_string()))?;
    let max_nodes = layer_budget(packed);
    drop(message);
    let file_name = archive.file_name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(name);
    let (json, mut warnings, networks) = map::to_rest(&msg, file_name, max_nodes);
    drop(msg);
    let images = |r: &str| archive.image(r);
    let (doc, more) = crate::rest::import_with_networks(&json, &images, networks, max_nodes)?;
    warnings.extend(more.into_iter().filter_map(map::adapt_warning));
    Ok((doc, warnings))
}

/// A font style under the name the format uses: the standard weight names as words ("SemiBold"
/// → "Semi Bold", "ExtraBoldItalic" → "Extra Bold Italic", "Regular Italic" → "Italic"). Other
/// names stay as they are.
fn style_name(style: &str) -> String {
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

/// The parts of a `.fig` archive the import reads.
struct Archive<'b> {
    canvas: Vec<u8>,
    file_name: Option<String>,
    /// `images/<hash>` members (or nothing for a bare canvas).
    zip: Option<zip::ZipArchive<std::io::Cursor<&'b [u8]>>>,
}

impl<'b> Archive<'b> {
    fn open(bytes: &'b [u8]) -> Result<Self, FormatError> {
        if bytes.starts_with(b"fig-") {
            return Ok(Archive { canvas: bytes.to_vec(), file_name: None, zip: None });
        }
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| corrupt(e.to_string()))?;
        let canvas = crate::read_member(&mut z, "canvas.fig", MAX_CANVAS)?;
        let file_name = crate::read_member(&mut z, "meta.json", 1 << 20)
            .ok()
            .and_then(|m| serde_json::from_slice::<serde_json::Value>(&m).ok())
            .and_then(|m| m.get("file_name").and_then(serde_json::Value::as_str).map(str::to_string));
        Ok(Archive { canvas, file_name, zip: Some(z) })
    }

    /// Bytes of image `hash` (lowercase hex) from `images/`.
    fn image(&self, hash: &str) -> Option<Vec<u8>> {
        if hash.is_empty() || hash.len() > 128 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // A clone shares the parsed directory and the bytes; the resolver only has shared access.
        let mut z = self.zip.clone()?;
        crate::read_member(&mut z, &format!("images/{hash}"), MAX_IMAGE).ok()
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
