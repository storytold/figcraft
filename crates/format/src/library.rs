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
use std::io::{Read, Seek};
use std::sync::Arc;

use figcraft_doc::{
    ImageData, LibraryAsset, LibraryCollection, LibraryManifest, LibraryStyle, LibraryVariable, NodeId, NodeKind, PublishedLibrary, used_images,
};

use crate::{FormatError, MAX_MEMBER, Unpacked, from_json, image_data, io_err};

use craft_figcraft_format::library as native;

// Test helpers use these names when constructing deliberately bounded archive entries.
#[cfg(test)]
const MANIFEST: &str = native::MANIFEST;
#[cfg(test)]
const MAX_MANIFEST: u64 = native::MAX_MANIFEST_BYTES;

fn corrupt(e: impl std::fmt::Display) -> FormatError {
    FormatError::Corrupt(e.to_string())
}

/// Whether `id` can be a library id: 1–64 ASCII letters, digits, `-` and `_`.
pub fn is_valid_library_id(id: &str) -> bool {
    native::is_valid_id(id)
}

fn to_native_manifest(m: &LibraryManifest) -> native::Manifest {
    native::Manifest {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version,
        description: m.description.clone(),
        time: m.time,
        components: m
            .components
            .iter()
            .map(|a| native::Component { id: a.id.0, name: a.name.clone(), set: a.set, version: a.version, published: a.published })
            .collect(),
        styles: m
            .styles
            .iter()
            .map(|s| native::Style { id: s.id.clone(), name: s.name.clone(), kind: s.kind.clone(), version: s.version, published: s.published })
            .collect(),
        collections: m
            .collections
            .iter()
            .map(|c| native::Collection { id: c.id.clone(), name: c.name.clone(), version: c.version, published: c.published })
            .collect(),
        variables: m
            .variables
            .iter()
            .map(|v| native::Variable {
                id: v.id.clone(),
                name: v.name.clone(),
                collection: v.collection.clone(),
                kind: v.kind.clone(),
                version: v.version,
                published: v.published,
            })
            .collect(),
    }
}

fn from_native_manifest(m: native::Manifest) -> LibraryManifest {
    LibraryManifest {
        id: m.id,
        name: m.name,
        version: m.version,
        description: m.description,
        time: m.time,
        components: m
            .components
            .into_iter()
            .map(|a| LibraryAsset { id: NodeId(a.id), name: a.name, set: a.set, version: a.version, published: a.published })
            .collect(),
        styles: m
            .styles
            .into_iter()
            .map(|s| LibraryStyle { id: s.id, name: s.name, kind: s.kind, version: s.version, published: s.published })
            .collect(),
        collections: m
            .collections
            .into_iter()
            .map(|c| LibraryCollection { id: c.id, name: c.name, version: c.version, published: c.published })
            .collect(),
        variables: m
            .variables
            .into_iter()
            .map(|v| LibraryVariable { id: v.id, name: v.name, collection: v.collection, kind: v.kind, version: v.version, published: v.published })
            .collect(),
    }
}

/// Write the published library's entries. `written` are the images already in `images/`.
pub(crate) fn write(z: &mut craft_figcraft_format::ArchiveWriter, lib: &PublishedLibrary, written: &BTreeSet<&String>) -> Result<(), FormatError> {
    let already_written = written.iter().map(|hash| (*hash).clone()).collect();
    let images = lib.doc.images.iter().map(|(hash, image)| (hash.as_str(), image.bytes.as_slice()));
    native::write_members(
        z,
        &to_native_manifest(&lib.manifest),
        &crate::FiniteValue(&lib.doc),
        &crate::native_image_index(&lib.doc),
        &already_written,
        images,
    )
    .map_err(io_err)
}

/// The published library in an archive (`None` when the file was never published). `own` are
/// the images already read from `images/`; what it unpacks comes out of `unpacked` (beyond the
/// per-part caps).
pub(crate) fn read(
    z: &mut impl craft_figcraft_format::NativeArchive,
    own: &BTreeMap<String, ImageData>,
    unpacked: &mut Unpacked,
) -> Result<Option<PublishedLibrary>, FormatError> {
    if !z.contains(native::MANIFEST) {
        return Ok(None);
    }
    let wire_manifest: native::Manifest =
        serde_json::from_slice(&unpacked.read(z, native::MANIFEST, native::MAX_MANIFEST_BYTES)?).map_err(corrupt)?;
    let mut manifest = from_native_manifest(wire_manifest);
    if !is_valid_library_id(&manifest.id) {
        return Err(corrupt("the library has an invalid id"));
    }
    let text = String::from_utf8(unpacked.read(z, native::DOCUMENT, MAX_MEMBER)?).map_err(corrupt)?;
    let (mut doc, _) = from_json(&text)?;
    let available: BTreeSet<String> = own.keys().cloned().collect();
    let images = native::read_image_resources(z, used_images(&doc), &available, unpacked);
    for (hash, data) in images {
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
    manifest.name = native::clip_display(&manifest.name, native::MAX_NAME_CHARS);
    manifest.description = native::clip_display(&manifest.description, native::MAX_DESCRIPTION_CHARS);
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
    let mut z = craft_figcraft_format::SeekArchive::open(r, u64::MAX).map_err(corrupt)?;
    read(&mut z, &BTreeMap::new(), &mut Unpacked::new(max_bytes))
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
