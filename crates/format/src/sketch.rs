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
use serde_json::Value;

// Container parsing is shared; FigCraft document projection remains in this module.
pub(crate) use craft_sketch::Archive;

use crate::FormatError;

pub use craft_sketch::{MIN_SKETCH, NEWEST_TESTED_VERSION, TOO_OLD, is_sketch};
const MAX_JSON_MEMBER: u64 = craft_sketch::MAX_JSON_MEMBER;

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
    let archive =
        Archive::open_with(bytes, craft_sketch::Limits { max_page: max_member, max_image: figcraft_doc::ImageData::MAX_BYTES, ..Default::default() })
            .map_err(|error| match error {
                craft_sketch::Error::TooOld => FormatError::Io(TOO_OLD.into()),
                craft_sketch::Error::Corrupt(message) => FormatError::Corrupt(format!("Sketch file: {message}")),
            })?;
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
