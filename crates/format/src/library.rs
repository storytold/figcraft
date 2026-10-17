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

use figcraft_doc::{ImageData, LibraryManifest, MAX_VIDEO_BYTES, NodeKind, PublishedLibrary, VideoData, used_images, used_videos, validate_videos};

use crate::{FormatError, MAX_MEMBER, from_json, image_data, io_err, json_options, put, put_json, read_member};

const MANIFEST: &str = "library/manifest.json";
const SNAPSHOT: &str = "library/document.json";
const IMAGES: &str = "library/images/";
const VIDEOS: &str = "library/videos/";
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

/// Image hashes become archive member names: only plain hex-like names are looked up.
fn is_valid_hash(h: &str) -> bool {
    (1..=128).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Write the published library's entries. `written` are the images already in `images/`.
pub(crate) fn write<W: Write + Seek>(
    z: &mut zip::ZipWriter<W>,
    lib: &PublishedLibrary,
    written: &BTreeSet<&String>,
    written_videos: &BTreeSet<&String>,
) -> Result<(), FormatError> {
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    put(z, MANIFEST, json_options(), &serde_json::to_vec(&lib.manifest).map_err(io_err)?)?;
    put_json(z, SNAPSHOT, &lib.doc)?;
    for (hash, img) in &lib.doc.images {
        if !written.contains(hash) && is_valid_hash(hash) {
            put(z, &format!("{IMAGES}{hash}"), stored, &img.bytes)?;
        }
    }
    validate_videos(&lib.doc.videos).map_err(FormatError::Corrupt)?;
    if let Some(hash) = used_videos(&lib.doc).into_iter().find(|hash| !lib.doc.videos.contains_key(hash)) {
        return Err(corrupt(format!("library video {hash} is referenced but its bytes are missing")));
    }
    for (hash, video) in &lib.doc.videos {
        if !is_valid_hash(hash) || crate::image_hash(&video.bytes) != *hash {
            return Err(corrupt(format!("library video {hash} has an invalid content hash")));
        }
        if !written_videos.contains(hash) {
            put(z, &format!("{VIDEOS}{hash}"), stored, &video.bytes)?;
        }
    }
    Ok(())
}

/// The published library in an archive (`None` when the file was never published). `own` are
/// the images already read from `images/`; `budget` caps the bytes its snapshot and images unpack
/// to, together (beyond the per-part caps).
pub(crate) fn read<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    own: &BTreeMap<String, ImageData>,
    own_videos: &BTreeMap<String, VideoData>,
    budget: u64,
    additional_video_budget: u64,
) -> Result<Option<PublishedLibrary>, FormatError> {
    if z.index_for_name(MANIFEST).is_none() {
        return Ok(None);
    }
    let mut manifest: LibraryManifest = serde_json::from_slice(&read_member(z, MANIFEST, MAX_MANIFEST)?).map_err(corrupt)?;
    if !is_valid_library_id(&manifest.id) {
        return Err(corrupt("the library has an invalid id"));
    }
    let text = String::from_utf8(read_member(z, SNAPSHOT, budget.min(MAX_MEMBER))?).map_err(corrupt)?;
    let mut budget = budget.saturating_sub(text.len() as u64).min(MAX_IMAGE_BYTES);
    let (mut doc, _) = from_json(&text)?;
    for hash in used_images(&doc) {
        if !is_valid_hash(&hash) {
            continue;
        }
        if let Some(img) = own.get(&hash) {
            doc.images.insert(hash, img.clone());
            continue;
        }
        let name = [format!("{IMAGES}{hash}"), format!("images/{hash}")].into_iter().find(|n| z.index_for_name(n).is_some());
        // A missing or oversized image shows as a placeholder, like in any other file.
        let Some(data) = name.and_then(|n| read_member(z, &n, budget.min(MAX_MEMBER)).ok()) else { continue };
        budget = budget.saturating_sub(data.len() as u64);
        doc.images.insert(hash, image_data(data));
    }
    let mut video_budget = budget.min(MAX_VIDEO_BYTES as u64 * 2).min(additional_video_budget);
    for hash in used_videos(&doc) {
        if let Some(video) = own_videos.get(&hash) {
            doc.videos.insert(hash, video.clone());
            continue;
        }
        if !is_valid_hash(&hash) {
            return Err(corrupt(format!("library video has invalid hash {hash}")));
        }
        let name = [format!("{VIDEOS}{hash}"), format!("videos/{hash}")]
            .into_iter()
            .find(|name| z.index_for_name(name).is_some())
            .ok_or_else(|| corrupt(format!("library video {hash} is missing")))?;
        let data = read_member(z, &name, video_budget.min(MAX_VIDEO_BYTES as u64))
            .map_err(|e| corrupt(format!("library video {hash} is missing or invalid: {e}")))?;
        video_budget = video_budget.saturating_sub(data.len() as u64);
        if crate::image_hash(&data) != hash {
            return Err(corrupt(format!("library video {hash} content hash does not match")));
        }
        doc.videos.insert(hash, VideoData { bytes: Arc::new(data) });
    }
    validate_videos(&doc.videos).map_err(corrupt)?;
    // Keep what the snapshot really holds; clip what people see.
    manifest.components.retain(|a| doc.node(a.id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. })));
    manifest.styles.retain(|s| doc.styles.iter().any(|x| x.id == s.id));
    manifest.collections.retain(|c| doc.variables.collections.iter().any(|x| x.id == c.id));
    manifest.variables.retain(|v| doc.variables.variables.iter().any(|x| x.id == v.id && x.collection == v.collection));
    manifest.name = manifest.name.chars().take(MAX_NAME).collect();
    manifest.description = manifest.description.chars().take(MAX_DESCRIPTION).collect();
    Ok(Some(PublishedLibrary { manifest, doc: Arc::new(doc) }))
}

/// Read the published library of a `.figcraft` file (only its library entries are read): `None`
/// when the file was never published, an error when it isn't a FigCraft archive or is damaged.
pub fn read_library<R: Read + Seek>(r: R) -> Result<Option<PublishedLibrary>, FormatError> {
    let mut z = zip::ZipArchive::new(r).map_err(corrupt)?;
    read(&mut z, &BTreeMap::new(), &BTreeMap::new(), u64::MAX, crate::MAX_VIDEO_ARCHIVE_BYTES)
}

/// [`read_library`] unpacking at most `max_bytes` in all (snapshot and images: an archive that
/// would unpack to more is refused, or loses the images past it), for libraries from people the
/// reader doesn't trust (a server).
pub fn read_library_within<R: Read + Seek>(r: R, max_bytes: u64) -> Result<Option<PublishedLibrary>, FormatError> {
    let mut z = zip::ZipArchive::new(r).map_err(corrupt)?;
    read(&mut z, &BTreeMap::new(), &BTreeMap::new(), max_bytes, crate::MAX_VIDEO_ARCHIVE_BYTES)
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
