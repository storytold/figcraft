//! A published library inside a `.figcraft` archive.
//!
//! `library/manifest.json` holds the [`LibraryManifest`]; `library/document.json` the snapshot of
//! the published assets (a `document.json` of its own). Images the snapshot uses are read from
//! `images/` (the file's own images) or, when the file itself no longer uses them,
//! `library/images/<sha256>`. Readers that don't know about libraries ignore these entries; files
//! without them load unchanged.
//!
//! Files using a library read only these entries ([`read_library`]), with size limits on each,
//! so a missing, damaged or oversized library file is an error, never a crash.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::sync::Arc;

use figcraft_doc::{ImageData, LibraryManifest, NodeKind, PublishedLibrary, used_images};

use crate::{FormatError, MAX_MEMBER, Unpacked, from_json, image_data, io_err, json_options, put, put_json};

const MANIFEST: &str = "library/manifest.json";
const SNAPSHOT: &str = "library/document.json";
const IMAGES: &str = "library/images/";
/// Largest manifest read.
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
/// Most bytes of images read for one library.
const MAX_IMAGE_BYTES: u64 = 1024 * 1024 * 1024;
/// Longest library id and name kept.
const MAX_ID: usize = 64;
const MAX_NAME: usize = 200;
const MAX_DESCRIPTION: usize = 2000;

fn corrupt(e: impl std::fmt::Display) -> FormatError {
    FormatError::Corrupt(e.to_string())
}

/// Whether `id` can be a library id: 1–64 ASCII letters, digits, `-` and `_`.
pub fn is_valid_library_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Write the published library's entries. `written` are the images already in `images/`.
pub(crate) fn write<W: Write + Seek>(z: &mut zip::ZipWriter<W>, lib: &PublishedLibrary, written: &BTreeSet<&String>) -> Result<(), FormatError> {
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    put(z, MANIFEST, json_options(), &serde_json::to_vec(&lib.manifest).map_err(io_err)?)?;
    put_json(z, SNAPSHOT, &lib.doc)?;
    for (hash, img) in &lib.doc.images {
        if !written.contains(hash) && ImageData::is_valid_key(hash) {
            put(z, &format!("{IMAGES}{hash}"), stored, &img.bytes)?;
        }
    }
    Ok(())
}

/// The published library in an archive (`None` when the file was never published). `own` are
/// the images already read from `images/`; what it unpacks comes out of `unpacked` (beyond the
/// per-part caps).
pub(crate) fn read<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    own: &BTreeMap<String, ImageData>,
    unpacked: &mut Unpacked,
) -> Result<Option<PublishedLibrary>, FormatError> {
    if z.index_for_name(MANIFEST).is_none() {
        return Ok(None);
    }
    let mut manifest: LibraryManifest = serde_json::from_slice(&unpacked.read(z, MANIFEST, MAX_MANIFEST)?).map_err(corrupt)?;
    if !is_valid_library_id(&manifest.id) {
        return Err(corrupt("the library has an invalid id"));
    }
    let text = String::from_utf8(unpacked.read(z, SNAPSHOT, MAX_MEMBER)?).map_err(corrupt)?;
    let mut budget = MAX_IMAGE_BYTES;
    let (mut doc, _) = from_json(&text)?;
    for hash in used_images(&doc) {
        if !ImageData::is_valid_key(&hash) {
            continue;
        }
        if let Some(img) = own.get(&hash) {
            doc.images.insert(hash, img.clone());
            continue;
        }
        let name = [format!("{IMAGES}{hash}"), format!("images/{hash}")].into_iter().find(|n| z.index_for_name(n).is_some());
        // A missing or oversized image shows as a placeholder, like in any other file.
        let Some(data) = name.and_then(|n| unpacked.read(z, &n, budget.min(MAX_MEMBER)).ok()) else { continue };
        budget = budget.saturating_sub(data.len() as u64);
        doc.images.insert(hash, image_data(data));
    }
    // Keep what the snapshot really holds; clip what people see.
    manifest.components.retain(|a| doc.node(a.id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. })));
    // Membership by set lookups: a long manifest against a large snapshot stays linear (#1078).
    let styles: BTreeSet<&str> = doc.styles.iter().map(|x| x.id.as_str()).collect();
    let collections: BTreeSet<&str> = doc.variables.collections.iter().map(|x| x.id.as_str()).collect();
    let variables: BTreeSet<(&str, &str)> = doc.variables.variables.iter().map(|x| (x.id.as_str(), x.collection.as_str())).collect();
    manifest.styles.retain(|s| styles.contains(s.id.as_str()));
    manifest.collections.retain(|c| collections.contains(c.id.as_str()));
    manifest.variables.retain(|v| variables.contains(&(v.id.as_str(), v.collection.as_str())));
    manifest.name = manifest.name.chars().take(MAX_NAME).collect();
    manifest.description = manifest.description.chars().take(MAX_DESCRIPTION).collect();
    Ok(Some(PublishedLibrary { manifest, doc: Arc::new(doc) }))
}

/// Read the published library of a `.figcraft` file (only its library entries are read): `None`
/// when the file was never published, an error when it isn't a FigCraft archive or is damaged.
pub fn read_library<R: Read + Seek>(r: R) -> Result<Option<PublishedLibrary>, FormatError> {
    read_library_within(r, u64::MAX)
}

/// [`read_library`] unpacking at most `max_bytes` in all (manifest, snapshot and images: an
/// archive that would unpack to more is refused, or loses the images past it), for libraries from
/// people the reader doesn't trust (a server).
pub fn read_library_within<R: Read + Seek>(r: R, max_bytes: u64) -> Result<Option<PublishedLibrary>, FormatError> {
    let mut z = zip::ZipArchive::new(r).map_err(corrupt)?;
    read(&mut z, &BTreeMap::new(), &mut Unpacked::new(max_bytes))
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
