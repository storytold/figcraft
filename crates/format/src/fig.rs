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
mod empty_swap_tests;
#[cfg(test)]
mod expansion_cost_tests;
mod export;
#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod interop_tests;
#[cfg(test)]
mod layer_cap_tests;
mod map;
#[cfg(test)]
mod missing_image_tests;
#[cfg(test)]
mod paste_style_tests;
#[cfg(test)]
mod prop_value_tests;
#[cfg(test)]
mod scale_factor_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod source_identity_tests;
#[cfg(test)]
mod swapped_look_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_paragraph_tests;
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
#[cfg(test)]
mod vector_scale_tests;

use std::collections::BTreeMap;

use figcraft_doc::{Document, FileKind};

pub use export::export;

use crate::FormatError;
#[cfg(test)]
use craft_fig::rest::MAX_FIG_NODES;
pub(crate) use craft_fig::rest::layer_budget;
use craft_fig::{Archive, Limits, decode_canvas, embedded_aliases, embedded_copies, kiwi};
pub use craft_fig::{JAM_HEADER, is_fig};
#[cfg(test)]
use craft_fig::{MAX_CANVAS, chunks, network};
pub(crate) use craft_figma_rest::style_name;

/// Largest image member read.
const MAX_IMAGE: u64 = figcraft_doc::ImageData::MAX_BYTES;
/// Total decompressed ZIP image bytes read per import: the native image budget (`IMAGE_BUDGET`) on
/// 64-bit targets, lower on 32-bit targets to leave address space for the canvas, layers and
/// decoded pixels.
#[cfg(not(target_pointer_width = "32"))]
const MAX_IMAGE_BYTES: u64 = crate::IMAGE_BUDGET.bytes;
#[cfg(target_pointer_width = "32")]
const MAX_IMAGE_BYTES: u64 = 1024 * 1024 * 1024;

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
    let mut archive = Archive::open_with(bytes, Limits { max_image: MAX_IMAGE, max_image_total: image_limit }).map_err(fig_err)?;
    // The budgets follow the compressed canvas, which the file itself bounds (its zip member may
    // inflate too).
    let packed = archive.canvas.len().min(bytes.len());
    // FigJam writes its whiteboards with this header: they open as boards.
    let board = archive.is_board();
    // Each stage's input is dropped as soon as the next one is built: in a large file every one
    // of them takes gigabytes, and holding them all at once runs out of memory.
    let (schema, message) = decode_canvas(&std::mem::take(&mut archive.canvas)).map_err(fig_err)?;
    let root = schema.find("Message").ok_or_else(|| corrupt("the schema has no Message definition"))?;
    let msg = schema.decode_message_within(root, &message, craft_fig::value_budget(packed)).map_err(|e| corrupt(e.to_string()))?;
    let max_nodes = layer_budget(packed);
    drop(message);
    let file_name = archive.file_name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(name);
    let (mut json, mut warnings, networks) = map::to_rest(&msg, file_name, max_nodes);
    // The message goes before the layers are built, so the images it carries are copied out first.
    let embedded = embedded_copies(&msg, MAX_IMAGE);
    drop(msg);
    let aliases = embedded_aliases(&embedded, |hash| archive.has_image(hash));
    if !aliases.is_empty() {
        canonical_image_refs(&mut json, &aliases);
    }
    let images = |r: &str| archive.image(r).or_else(|| embedded.get(r).map(|b| b.to_vec()));
    let (mut doc, more) = crate::rest::import_with_networks(&json, &images, networks, max_nodes)?;
    let skipped = archive.images_skipped();
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

fn corrupt(msg: impl Into<String>) -> FormatError {
    FormatError::Corrupt(format!(".fig file: {}", msg.into()))
}

/// A `.fig` read or write failure as the app reports it (`Corrupt` already says `.fig file:`).
pub(crate) fn fig_err(e: craft_fig::Error) -> FormatError {
    match e {
        craft_fig::Error::Corrupt(_) => FormatError::Corrupt(e.to_string()),
        craft_fig::Error::TooLarge | craft_fig::Error::Io(_) => FormatError::Io(e.to_string()),
    }
}

/// Rename every `imageRef` in REST `json` found in `aliases` to the name it maps to.
fn canonical_image_refs(json: &mut serde_json::Value, aliases: &BTreeMap<String, String>) {
    use serde_json::Value;
    let mut pending = vec![json];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                for (key, field) in fields.iter_mut() {
                    match field {
                        Value::String(r) if key == "imageRef" => {
                            if let Some(canonical) = aliases.get(r.as_str()) {
                                r.clone_from(canonical);
                            }
                        }
                        _ => pending.push(field),
                    }
                }
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            _ => {}
        }
    }
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
