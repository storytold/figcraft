//! Streaming atomic `.figcraft` file writer with pass-through lazy image copying (#1313).
//!
//! Writes the archive straight to a uniquely named sibling staging file through a buffered
//! `zip::ZipWriter<BufWriter<File>>`, streaming each lazy image member chunk-by-chunk from its
//! backing store instead of holding the full archive or all images in memory. On completion, the
//! staging file is closed and atomically renamed into place (releasing any matching backing
//! store's read handle across the rename so in-place saves succeed on Windows too).

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use figcraft_doc::{Document, ImageData, ImageLoader, PublishedLibrary};

use crate::{
    BRANCH_INDEX, Branch, BranchState, FiniteValue, FormatError, NativeBranchEntry, NativeBranchIndex, NativeBranchState, NativeVersionEntry,
    NativeVersionIndex, Snapshot, Version, io_err, is_valid_version_id, library, native_image_index,
};

static STREAM_STAGING: AtomicU64 = AtomicU64::new(0);

/// Threshold above which ZIP64 entry headers are enabled proactively (3.5 GiB).
const ZIP64_THRESHOLD: u64 = 3584 * 1024 * 1024;

fn json_opts() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(3))
}

fn stored_opts(large: bool) -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(large)
}

fn put_json(w: &mut zip::ZipWriter<BufWriter<File>>, name: &str, doc: &Document) -> Result<(), FormatError> {
    w.start_file(name, json_opts()).map_err(io_err)?;
    craft_figcraft_format::write_document_json(&FiniteValue(doc), &native_image_index(doc), &mut *w).map_err(io_err)
}

fn put_bytes(w: &mut zip::ZipWriter<BufWriter<File>>, name: &str, opts: zip::write::SimpleFileOptions, bytes: &[u8]) -> Result<(), FormatError> {
    w.start_file(name, opts).map_err(io_err)?;
    w.write_all(bytes).map_err(io_err)
}

fn put_image(w: &mut zip::ZipWriter<BufWriter<File>>, name: &str, img: &ImageData, large: bool) -> Result<(), FormatError> {
    w.start_file(name, stored_opts(large)).map_err(io_err)?;
    match img.stream_to(w).map_err(io_err)? {
        true => Ok(()),
        false => Err(FormatError::Io(format!("{name}: image data unavailable in backing archive"))),
    }
}

fn put_versions(w: &mut zip::ZipWriter<BufWriter<File>>, dir: &str, versions: &[Version]) -> Result<(), FormatError> {
    if versions.is_empty() {
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    let mut index = Vec::with_capacity(versions.len());
    for v in versions {
        if !is_valid_version_id(&v.id) || !ids.insert(v.id.as_str()) {
            return Err(FormatError::Io(format!("invalid or duplicate version id `{}`", v.id)));
        }
        let name = format!("{dir}/{}.json", v.id);
        match &v.snapshot {
            Snapshot::Doc(d) => put_json(w, &name, d)?,
            Snapshot::Stored { json, .. } => put_bytes(w, &name, json_opts(), json)?,
        }
        index.push(NativeVersionEntry {
            id: v.id.clone(),
            name: v.name.clone(),
            description: v.description.clone(),
            time: v.time,
            auto: v.auto,
            images: v.images().keys().cloned().collect(),
        });
    }
    let index_bytes = craft_figcraft_format::write_version_index(&NativeVersionIndex { versions: index }).map_err(io_err)?;
    put_bytes(w, &format!("{dir}/index.json"), json_opts(), &index_bytes)
}

fn put_library(w: &mut zip::ZipWriter<BufWriter<File>>, lib: &PublishedLibrary, written: &BTreeSet<String>, large: bool) -> Result<(), FormatError> {
    let manifest_bytes = craft_figcraft_format::library::write_manifest(&library::to_native_manifest(&lib.manifest)).map_err(io_err)?;
    put_bytes(w, craft_figcraft_format::library::MANIFEST, json_opts(), &manifest_bytes)?;
    put_json(w, craft_figcraft_format::library::DOCUMENT, &lib.doc)?;
    for (hash, img) in &lib.doc.images {
        if written.contains(hash) || !ImageData::is_valid_key(hash) {
            continue;
        }
        let member = format!("{}{hash}", craft_figcraft_format::library::IMAGES_PREFIX);
        put_image(w, &member, img, large)?;
    }
    Ok(())
}

/// Collect the deduplicated set of image keys written for `(doc, versions, branches, published)`.
pub fn collect_live_image_keys(doc: &Document, versions: &[Version], branches: &[Branch]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
    for (hash, _) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
        keys.insert(hash.clone());
    }
    if let Some(lib) = &doc.published {
        for hash in lib.doc.images.keys() {
            if ImageData::is_valid_key(hash) {
                keys.insert(hash.clone());
            }
        }
    }
    keys
}

/// Find any distinct lazy image loaders referenced by `(doc, versions, branches)` plus `extra`
/// whose backing file is the same file as `dest`.
fn matching_backing_stores(
    dest: &Path,
    doc: &Document,
    versions: &[Version],
    branches: &[Branch],
    extra: &[Arc<dyn ImageLoader>],
) -> Vec<Arc<dyn ImageLoader>> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut consider = |loader: &Arc<dyn ImageLoader>| {
        if !seen.insert(loader.store_id()) {
            return;
        }
        if let Some(src) = loader.source_path()
            && is_same_dest(src, dest)
        {
            out.push(Arc::clone(loader));
        }
    };
    let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
    for (_, img) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
        if let Some(loader) = img.loader() {
            consider(loader);
        }
    }
    if let Some(lib) = &doc.published {
        for img in lib.doc.images.values() {
            if let Some(loader) = img.loader() {
                consider(loader);
            }
        }
    }
    for loader in extra {
        consider(loader);
    }
    out
}

fn is_same_dest(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Create an exclusive uniquely named sibling staging file next to `path`.
fn create_staging(path: &Path) -> std::io::Result<(PathBuf, File)> {
    for _ in 0..3 {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".tmp-figcraft-{}-{}", std::process::id(), STREAM_STAGING.fetch_add(1, Ordering::Relaxed)));
        let tmp = PathBuf::from(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => {
                if let Ok(old) = std::fs::metadata(path) {
                    let _ = file.set_permissions(old.permissions());
                }
                return Ok((tmp, file));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free staging name next to the file"))
}

/// Write a `.figcraft` archive with history and branches directly to `path` via a sibling
/// staging file and atomic rename, streaming image members without buffering the archive in
/// memory. Returns the final archive size in bytes.
pub fn save_file_with_history(
    doc: &Document,
    path: &Path,
    thumbnail_png: Option<&[u8]>,
    versions: &[Version],
    branches: &[Branch],
    extra_stores: &[Arc<dyn ImageLoader>],
) -> Result<u64, FormatError> {
    let (tmp, file) = create_staging(path).map_err(|e| FormatError::Io(format!("{}: {e}", path.display())))?;
    let write_result = (|| -> Result<u64, FormatError> {
        let mut z = zip::ZipWriter::new(BufWriter::with_capacity(256 * 1024, file));
        put_json(&mut z, "document.json", doc)?;

        // Compute total declared image bytes to proactively enable ZIP64 on >3.5 GiB archives.
        let mut total_image_bytes: u64 = 0;
        let mut counted = BTreeSet::new();
        let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
        for (hash, img) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
            if counted.insert(hash) {
                total_image_bytes = total_image_bytes.saturating_add(img.byte_len());
            }
        }
        let large = total_image_bytes >= ZIP64_THRESHOLD;

        let mut written = BTreeSet::new();
        let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
        for (hash, img) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
            if written.insert(hash.clone()) {
                put_image(&mut z, &format!("images/{hash}"), img, large)?;
            }
        }
        if let Some(t) = thumbnail_png {
            put_bytes(&mut z, "thumbnail.png", stored_opts(large), t)?;
        }
        put_versions(&mut z, "versions", versions)?;
        if !branches.is_empty() {
            let mut ids = BTreeSet::new();
            let mut index = Vec::with_capacity(branches.len());
            for b in branches {
                if !is_valid_version_id(&b.id) || !ids.insert(b.id.as_str()) {
                    return Err(FormatError::Io(format!("invalid or duplicate branch id `{}`", b.id)));
                }
                put_json(&mut z, &format!("branches/{}/base.json", b.id), &b.base)?;
                put_json(&mut z, &format!("branches/{}/head.json", b.id), &b.head)?;
                put_versions(&mut z, &format!("branches/{}/versions", b.id), &b.versions)?;
                let images = |d: &Document| d.images.keys().cloned().collect::<Vec<String>>();
                index.push(NativeBranchEntry {
                    id: b.id.clone(),
                    name: b.name.clone(),
                    time: b.time,
                    client: b.client,
                    state: match b.state {
                        BranchState::Active => NativeBranchState::Active,
                        BranchState::Merged => NativeBranchState::Merged,
                        BranchState::Archived => NativeBranchState::Archived,
                    },
                    base_images: images(&b.base),
                    head_images: images(&b.head),
                });
            }
            let branch_bytes = craft_figcraft_format::write_branch_index(&NativeBranchIndex { branches: index }).map_err(io_err)?;
            put_bytes(&mut z, BRANCH_INDEX, json_opts(), &branch_bytes)?;
        }
        if let Some(lib) = &doc.published {
            put_library(&mut z, lib, &written, large)?;
        }
        let mut buf = z.finish().map_err(io_err)?;
        buf.flush().map_err(io_err)?;
        let file = buf.into_inner().map_err(|e| io_err(e.into_error()))?;
        let len = file.metadata().map_err(io_err)?.len();
        drop(file);
        Ok(len)
    })();

    let written_bytes = match write_result {
        Ok(len) => len,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };

    // If saving over a file currently backing one or more ZipImageStores, coordinate the rename
    // through the backing store so its open read handle is dropped before rename (Windows safety)
    // and reopened on the new archive afterwards.
    let stores = matching_backing_stores(path, doc, versions, branches, extra_stores);
    if let Some((primary, others)) = stores.split_first() {
        for other in others {
            other.release_backing_handle();
        }
        let swapped = primary.replace_backing_file(&tmp, path);
        for other in others {
            other.reopen_backing_file(path);
        }
        if let Err(e) = swapped {
            let _ = std::fs::remove_file(&tmp);
            return Err(FormatError::Io(format!("{}: {e}", path.display())));
        }
    } else if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(FormatError::Io(format!("{}: {e}", path.display())));
    }
    Ok(written_bytes)
}
