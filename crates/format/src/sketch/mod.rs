//! Import of `.sketch` files saved by Sketch 48 or newer (best effort), written from the public
//! description of the format only.
//!
//! A `.sketch` file is a zip archive holding `document.json` (the document: its pages in order,
//! shared styles, swatches and library symbols copied into it), `meta.json` (the app version and
//! format version that saved it), one `pages/<id>.json` per page (the layer tree), `images/` and,
//! in newer files, `fonts/`. Every object carries its kind in `_class`. [`tree`] maps the layers
//! onto the REST API file JSON shape and the REST importer builds the document, so this import
//! shares its mapping, budgets and import report with `.fig` and file JSON imports. Files older
//! than Sketch 48 (a SQLite database before 43, text as binary property lists until 47) are
//! refused with a message saying how to convert them. Anything not understood is skipped with a
//! warning; malformed input never panics.

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod frames_tests;
mod path;
#[cfg(test)]
mod path_tests;
mod proto;
#[cfg(test)]
mod proto_tests;
mod shared;
#[cfg(test)]
mod shared_tests;
mod stack;
mod style;
#[cfg(test)]
mod style_tests;
mod symbols;
#[cfg(test)]
mod symbols_tests;
#[cfg(test)]
mod tests;
mod text;
#[cfg(test)]
mod text_tests;
mod tree;
#[cfg(test)]
mod tree_tests;

use std::collections::BTreeSet;

use figcraft_doc::Document;
use serde_json::{Value, json};

use crate::FormatError;

/// The oldest Sketch whose files open: from 48 on, text is plain JSON too.
pub const MIN_SKETCH: u32 = 48;
/// The newest `meta.json` format version checked against real files (Sketch 83); newer files
/// open with a warning.
pub const NEWEST_TESTED_VERSION: u64 = 139;
/// What opening a file saved by an older Sketch says.
pub const TOO_OLD: &str = "saved by a Sketch older than 48; open it in a newer Sketch and save it again";
/// Largest JSON member read (a real page of a few thousand layers takes 33 MB).
const MAX_JSON_MEMBER: u64 = 512 * 1024 * 1024;
/// Most bytes JSON members may unpack to, per compressed byte (real files pack 5–20 times; a
/// zero-filled array a thousand times), with a floor for small members.
const MAX_JSON_RATIO: u64 = 100;
const MIN_JSON_ALLOWANCE: u64 = 16 * 1024 * 1024;
/// Most JSON read from one file in all.
#[cfg(not(target_pointer_width = "32"))]
const MAX_JSON_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
const MAX_JSON_TOTAL: u64 = 512 * 1024 * 1024;
/// Largest image member read.
const MAX_IMAGE: u64 = 256 * 1024 * 1024;
/// Most pages read from one file.
const MAX_PAGES: usize = 1000;

/// Whether `bytes` look like a `.sketch` file: a zip holding `document.json` and `meta.json`, or
/// the SQLite database Sketch wrote before version 43 (so that it gets the "too old" message).
pub fn is_sketch(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SQLite format 3\0")
        || (bytes.starts_with(b"PK")
            && zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok_and(|z| {
                let names: BTreeSet<&str> = z.file_names().collect();
                names.contains("document.json") && names.contains("meta.json")
            }))
}

/// Finds the installed face a PostScript name belongs to: its family and style.
pub type FontLookup = Box<dyn Fn(&str) -> Option<(String, String)>>;

/// Import `bytes` when they are a `.sketch` file; `None` when they are something else.
pub fn try_import(bytes: &[u8], name: &str) -> Option<Result<(Document, Vec<String>), FormatError>> {
    try_import_with_fonts(bytes, name, None)
}

/// [`try_import`] with `fonts` finding the installed faces the file's fonts (PostScript names)
/// belong to; families of fonts it doesn't find are read from their names.
pub fn try_import_with_fonts(bytes: &[u8], name: &str, fonts: Option<FontLookup>) -> Option<Result<(Document, Vec<String>), FormatError>> {
    is_sketch(bytes).then(|| import_with_fonts(bytes, name, fonts))
}

/// Build a document from a `.sketch` file. Returns it with the warnings about what was skipped or
/// approximated. `name`: the document name.
pub fn import(bytes: &[u8], name: &str) -> Result<(Document, Vec<String>), FormatError> {
    import_with_fonts(bytes, name, None)
}

/// [`import`] with a font lookup (see [`try_import_with_fonts`]).
pub fn import_with_fonts(bytes: &[u8], name: &str, fonts: Option<FontLookup>) -> Result<(Document, Vec<String>), FormatError> {
    import_parts(bytes, name, MAX_JSON_MEMBER, crate::fig::layer_budget(bytes.len()), fonts)
}

/// [`import`] reading pages of at most `max_member` bytes of JSON each.
#[cfg(test)]
pub(crate) fn import_within(bytes: &[u8], name: &str, max_member: u64) -> Result<(Document, Vec<String>), FormatError> {
    import_parts(bytes, name, max_member, crate::fig::layer_budget(bytes.len()), None)
}

/// [`import`] with at most `max_nodes` layers.
#[cfg(test)]
pub(crate) fn import_within_nodes(bytes: &[u8], name: &str, max_nodes: usize) -> Result<(Document, Vec<String>), FormatError> {
    import_parts(bytes, name, MAX_JSON_MEMBER, max_nodes, None)
}

fn import_parts(
    bytes: &[u8],
    name: &str,
    max_member: u64,
    max_nodes: usize,
    fonts: Option<FontLookup>,
) -> Result<(Document, Vec<String>), FormatError> {
    if bytes.starts_with(b"SQLite format 3\0") {
        return Err(FormatError::Io(TOO_OLD.into()));
    }
    let archive = Archive::open(bytes, max_member)?;
    let map = tree::to_rest(&archive, name, max_nodes, fonts);
    if map.too_old {
        return Err(FormatError::Io(TOO_OLD.into()));
    }
    let mut warnings = archive.warnings.clone();
    warnings.extend(map.warnings);
    if let Some(note) = embedded_fonts(&archive.document) {
        warnings.push(note);
    }
    let images = |r: &str| archive.image(r);
    let (mut doc, more) = crate::rest::import_with_networks_plain(&map.json, &images, map.networks, max_nodes)?;
    if archive.document.get("colorSpace").and_then(Value::as_i64) == Some(2) {
        doc.color_profile = figcraft_doc::color::ColorProfile::DisplayP3;
    }
    warnings.extend(more);
    Ok((doc, warnings))
}

/// The note about fonts the file carries (Sketch 7x and newer can embed them, as `fontReferences`
/// with `fontData`): FigCraft doesn't use fonts from inside documents, so their text shows in
/// installed fonts. To do once `.figcraft` documents can carry fonts (loading them only for the
/// session would lose them on the first save and reopen).
fn embedded_fonts(document: &Value) -> Option<String> {
    let refs = document.get("fontReferences").and_then(Value::as_array)?;
    // A reference without font data only names a font the document uses.
    let families: BTreeSet<String> = refs
        .iter()
        .filter(|r| r.get("fontData").is_some())
        .filter_map(|r| r.get("fontFamilyName").and_then(Value::as_str))
        .map(|f| f.chars().take(64).collect())
        .take(20)
        .collect();
    if families.is_empty() {
        return None;
    }
    let list: Vec<&str> = families.iter().map(String::as_str).collect();
    Some(format!("the file has embedded fonts FigCraft doesn't use ({}); install them to see the text as designed", list.join(", ")))
}

fn corrupt(msg: impl Into<String>) -> FormatError {
    FormatError::Corrupt(format!("Sketch file: {}", msg.into()))
}

/// The parts of a `.sketch` archive: the document, the pages in document order and the images.
pub(crate) struct Archive<'b> {
    pub document: Value,
    /// `meta.json`'s format version (0 when missing).
    pub version: u64,
    /// `(id, page JSON)` in the document's order.
    pub pages: Vec<(String, Value)>,
    /// Problems met while reading (pages skipped).
    pub warnings: Vec<String>,
    zip: zip::ZipArchive<std::io::Cursor<&'b [u8]>>,
}

impl<'b> Archive<'b> {
    fn open(bytes: &'b [u8], max_member: u64) -> Result<Self, FormatError> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| corrupt(e.to_string()))?;
        let mut total = 0u64;
        let meta = json_member(&mut z, "meta.json", MAX_JSON_MEMBER, &mut total)?;
        if !meta.is_object() {
            return Err(corrupt("meta.json is not an object"));
        }
        if app_major(&meta).is_some_and(|v| v < MIN_SKETCH) {
            return Err(FormatError::Io(TOO_OLD.into()));
        }
        let mut warnings = Vec::new();
        if meta.get("version").and_then(Value::as_u64).is_some_and(|v| v > NEWEST_TESTED_VERSION) {
            warnings.push("the file was saved by a newer Sketch than the import was checked with; some layers may differ".to_string());
        }
        // A document.json that can't be read (nested too deep, damaged) loses its shared styles and
        // library copies, not the pages: they are then taken in archive order.
        let document = match json_member(&mut z, "document.json", MAX_JSON_MEMBER, &mut total) {
            Ok(d) if d.is_object() => d,
            other => {
                let why = other.err().map_or_else(|| "not an object".to_string(), |e| e.to_string());
                warnings.push(format!(
                    "the document's shared styles and library symbols were skipped: {}",
                    why.trim_start_matches("not a valid FigCraft file: ")
                ));
                let pages: Vec<Value> = z
                    .file_names()
                    .filter_map(|n| n.strip_prefix("pages/")?.strip_suffix(".json"))
                    .map(|id| json!({"_ref": format!("pages/{id}")}))
                    .collect();
                json!({"pages": pages})
            }
        };
        let refs = document.get("pages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let mut pages = Vec::new();
        for r in refs.iter().take(MAX_PAGES) {
            let Some(id) = r.get("_ref").and_then(Value::as_str).and_then(|s| s.strip_prefix("pages/")).filter(|id| valid_id(id)) else {
                warnings.push("pages the document lists without a valid reference were skipped".into());
                continue;
            };
            match json_member(&mut z, &format!("pages/{id}.json"), max_member, &mut total) {
                Ok(page) if page.is_object() => pages.push((id.to_string(), page)),
                Ok(_) => warnings.push("pages that aren't JSON objects were skipped".into()),
                Err(e) => warnings.push(format!("a page was skipped: {}", e.to_string().trim_start_matches("not a valid FigCraft file: "))),
            }
        }
        if refs.len() > MAX_PAGES {
            warnings.push(format!("the file has more than {MAX_PAGES} pages; the rest were skipped"));
        }
        let version = meta.get("version").and_then(Value::as_u64).unwrap_or(0);
        Ok(Archive { document, version, pages, warnings, zip: z })
    }

    /// The bytes of the image a layer references (`images/<name>`).
    pub(crate) fn image(&self, r: &str) -> Option<Vec<u8>> {
        let name = r.strip_prefix("images/").unwrap_or(r);
        if !valid_id(name.split('.').next().unwrap_or("")) || name.split('.').count() > 2 {
            return None;
        }
        // A clone shares the parsed directory and the bytes; the resolver only has shared access.
        let mut z = self.zip.clone();
        crate::read_member(&mut z, &format!("images/{name}"), MAX_IMAGE).ok()
    }
}

/// JSON member `name`, refused before unpacking when it would be larger than `limit`, than
/// [`MAX_JSON_RATIO`] times what it takes in the archive (past a floor), or than what is left of
/// the file's [`MAX_JSON_TOTAL`] (`total`: read so far).
fn json_member(z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str, limit: u64, total: &mut u64) -> Result<Value, FormatError> {
    let (size, packed) = {
        let f = z.by_name(name).map_err(|e| corrupt(format!("{name}: {e}")))?;
        (f.size(), f.compressed_size())
    };
    let allowed = limit.min(packed.saturating_mul(MAX_JSON_RATIO).max(MIN_JSON_ALLOWANCE)).min(MAX_JSON_TOTAL.saturating_sub(*total));
    if size > allowed {
        return Err(corrupt(format!("{name} is too large")));
    }
    let data = crate::read_member(z, name, allowed)?;
    *total += data.len() as u64;
    serde_json::from_slice(&data).map_err(|e| corrupt(format!("{name}: {e}")))
}

/// The major version of the Sketch that saved the file (`meta.json` `appVersion`, "50.2" → 50).
fn app_major(meta: &Value) -> Option<u32> {
    meta.get("appVersion").and_then(Value::as_str)?.split('.').next()?.trim().parse().ok()
}

/// Ids name archive members: letters, digits, `-` and `_` only, never a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
