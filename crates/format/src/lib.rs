//! The native `.figcraft` format.
//!
//! A zip archive holding `document.json` (the [`Document`] as JSON, with a `format`/`version`
//! header; layer fields at their default values are left out and read back as those defaults),
//! `images/<sha256>` and `fonts/<sha256>` for binary assets the document, versions or branches
//! use (deduplicated and indexed per snapshot), an optional
//! `thumbnail.png` and an optional version history: `versions/index.json` (`{"versions": [{id,
//! name, description, time, images}]}`, oldest first) plus one `versions/<id>.json` snapshot per
//! version (a `document.json` of its own). Branches are kept the same way: `branches/index.json`
//! (`{"branches": [{id, name, time, client, state, baseImages, headImages}]}`) plus
//! `branches/<id>/base.json` and `branches/<id>/head.json`; a branch's own version history is
//! kept like the file's, under `branches/<id>/versions/`. A published file also holds its
//! library (see [`library`]). Readers that don't know about versions, branches or libraries
//! ignore those entries. Plain
//! JSON (the `document.json` alone) also opens. Reading is lenient: structural problems are
//! repaired (see [`Document::repair`]) and reported, damaged versions and libraries are skipped.
//!
//! [`rest`] imports the public REST API file JSON; [`fig`] imports `.fig` files (best effort).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod fig;
pub mod library;
pub mod penpot;
pub mod rest;
pub mod thumbnail;

pub use library::{is_valid_library_id, read_library, read_library_within};

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::sync::Arc;

use figcraft_doc::{Document, ImageData};
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::Digest;

pub const EXTENSION: &str = "figcraft";
pub const FORMAT: &str = "figcraft";
pub const VERSION: u32 = 1;
/// Largest archive member we read (guards against zip bombs).
const MAX_MEMBER: u64 = 512 * 1024 * 1024;
/// `document.json` may unpack to [`DOCUMENT_RATIO`] times what it takes in the archive, within
/// [`MAX_MEMBER`] and [`MAX_DOCUMENT`]: a large document saved by us (a 900 000-layer file packs
/// 1.48 GB into 244 MB) opens again, while a small archive still can't unpack into gigabytes.
/// 32-bit targets (wasm32) have a 4 GB address space in all, so their ceiling is lower.
const DOCUMENT_RATIO: u64 = 16;
#[cfg(not(target_pointer_width = "32"))]
const MAX_DOCUMENT: u64 = 4 * 1024 * 1024 * 1024;
#[cfg(target_pointer_width = "32")]
const MAX_DOCUMENT: u64 = 1024 * 1024 * 1024;
/// Most versions a file keeps: saving more is refused, extra entries in a file are skipped.
pub const MAX_VERSIONS: usize = 100;
/// Longest version name and description kept (chars).
pub const MAX_VERSION_NAME: usize = 200;
pub const MAX_VERSION_DESCRIPTION: usize = 2000;
/// Largest version index and total size of the version snapshots read from one file.
const MAX_VERSION_INDEX: u64 = 16 * 1024 * 1024;
const MAX_VERSION_BYTES: u64 = 1024 * 1024 * 1024;
/// Most images, and image bytes together, read from one file (members past it are left out).
const IMAGE_BUDGET: ImageBudget = ImageBudget { images: 20_000, bytes: 4 * 1024 * 1024 * 1024 };
const BRANCH_INDEX: &str = "branches/index.json";
/// Most branches a file keeps: making more is refused, extra entries in a file are skipped.
pub const MAX_BRANCHES: usize = 100;
/// Deflate level of JSON members (see [`json_options`]).
const JSON_LEVEL: i64 = 3;
/// Limits on font binaries carried with a document (not the system's font registry).
const MAX_FONT_MEMBER: u64 = 32 * 1024 * 1024;
const MAX_FONT_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_FONT_COUNT: usize = 128;
/// Limit missing-font issue messages even when a damaged archive lists many hashes.
const MAX_FONT_ISSUES: usize = 20;

/// Keep diagnostics bounded even for untrusted font indexes with very large hash lists.
#[derive(Default)]
struct MissingFontIssues {
    first: BTreeSet<String>,
    additional_references: usize,
}

impl MissingFontIssues {
    fn note(&mut self, hash: &str) {
        let display = clip(hash, 80);
        if self.first.contains(&display) {
            return;
        }
        if self.first.len() < MAX_FONT_ISSUES {
            self.first.insert(display);
        } else {
            self.additional_references = self.additional_references.saturating_add(1);
        }
    }

    fn report(self, issues: &mut Vec<String>) {
        issues.extend(
            self.first
                .into_iter()
                .map(|hash| format!("Embedded font {hash} is listed in the document or its history but its data is missing; saving again drops it")),
        );
        if self.additional_references > 0 {
            issues.push(format!("{} additional references to missing embedded fonts", self.additional_references));
        }
    }
}

fn valid_font_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

/// Merge portable fonts while respecting the same asset limits as `.figcraft` loading.
/// Existing fonts take priority. Returns the number of new font/license records and the number skipped.
pub fn merge_embedded_fonts(target: &mut Document, source: &Document) -> (usize, usize) {
    let mut used: u64 = target.embedded_fonts.values().map(|b| b.len() as u64).sum();
    let mut accepted = 0;
    let mut skipped = 0;
    for (hash, bytes) in &source.embedded_fonts {
        if target.embedded_fonts.contains_key(hash) {
            continue;
        }
        let len = bytes.len() as u64;
        if !valid_font_hash(hash)
            || !is_sfnt_font(bytes)
            || image_hash(bytes) != *hash
            || len > MAX_FONT_MEMBER
            || used.saturating_add(len) > MAX_FONT_TOTAL
            || target.embedded_fonts.len() >= MAX_FONT_COUNT
        {
            skipped += 1;
            continue;
        }
        used += len;
        target.embedded_fonts.insert(hash.clone(), bytes.clone());
        accepted += 1;
    }
    for (hash, license) in &source.embedded_font_licenses {
        if target.embedded_fonts.contains_key(hash) && !target.embedded_font_licenses.contains_key(hash) {
            target.embedded_font_licenses.insert(hash.clone(), license.clone());
            accepted += 1;
        }
    }
    (accepted, skipped)
}

/// Recognise OpenType/TrueType files, never pass arbitrary ZIP bytes to a font parser.
fn is_sfnt_font(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && (bytes.starts_with(b"OTTO")
            || bytes.starts_with(b"true")
            || bytes.starts_with(b"typ1")
            || bytes.starts_with(&[0, 1, 0, 0])
            || bytes.starts_with(b"ttcf"))
}

#[derive(Debug)]
pub enum FormatError {
    Io(String),
    Corrupt(String),
    Newer(u32),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Io(e) => write!(f, "{e}"),
            FormatError::Corrupt(e) => write!(f, "not a valid FigCraft file: {e}"),
            FormatError::Newer(v) => write!(f, "this file was saved by a newer FigCraft (format version {v})"),
        }
    }
}

impl std::error::Error for FormatError {}

/// One entry of a file's version history: a named snapshot of the document.
#[derive(Clone, Debug)]
pub struct Version {
    /// Unique within the file; ASCII letters, digits, `-` and `_` (see [`is_valid_version_id`]).
    pub id: String,
    pub name: String,
    pub description: String,
    /// When it was saved (Unix seconds).
    pub time: u64,
    /// Saved automatically (a checkpoint) rather than by the user.
    pub auto: bool,
    snapshot: Snapshot,
}

#[derive(Clone, Debug)]
enum Snapshot {
    /// Saved in this session (or already read).
    Doc(Arc<Document>),
    /// As read from a file: its `document.json`-style JSON, parsed on first use, and the images
    /// it uses (both kinds of binary assets are shared with the active file).
    Stored { json: Arc<Vec<u8>>, images: BTreeMap<String, ImageData>, fonts: BTreeMap<String, Arc<Vec<u8>>> },
}

impl Version {
    /// A version of `doc`. The name and description are clipped to their maximum lengths.
    pub fn new(id: impl Into<String>, name: &str, description: &str, time: u64, doc: Arc<Document>) -> Self {
        Self {
            id: id.into(),
            name: clip(name, MAX_VERSION_NAME),
            description: clip(description, MAX_VERSION_DESCRIPTION),
            time,
            auto: false,
            snapshot: Snapshot::Doc(doc),
        }
    }

    /// The snapshot (with its images). Versions read from a file are parsed here, so a damaged
    /// one fails now rather than when the file opens.
    pub fn document(&self) -> Result<Arc<Document>, FormatError> {
        match &self.snapshot {
            Snapshot::Doc(d) => Ok(d.clone()),
            Snapshot::Stored { json, images, fonts } => {
                let text = std::str::from_utf8(json).map_err(|e| FormatError::Corrupt(e.to_string()))?;
                let (mut doc, _) = from_json(text)?;
                doc.images = images.clone();
                doc.embedded_fonts = fonts.clone();
                Ok(Arc::new(doc))
            }
        }
    }

    /// The images the snapshot uses, by hash.
    pub fn images(&self) -> &BTreeMap<String, ImageData> {
        match &self.snapshot {
            Snapshot::Doc(d) => &d.images,
            Snapshot::Stored { images, .. } => images,
        }
    }

    /// Portable fonts owned by this exact snapshot, not by the current document.
    pub fn fonts(&self) -> &BTreeMap<String, Arc<Vec<u8>>> {
        match &self.snapshot {
            Snapshot::Doc(d) => &d.embedded_fonts,
            Snapshot::Stored { fonts, .. } => fonts,
        }
    }
}

/// Where a branch is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchState {
    Active,
    /// Merged into the main file (and archived).
    Merged,
    /// Archived without merging.
    Archived,
}

/// A branch of the file: a copy of the document to work on separately, and the main document as
/// it was when the branch was made or last updated from it (its base).
#[derive(Clone, Debug)]
pub struct Branch {
    /// Unique among the file's branches; a valid version id ([`is_valid_version_id`]).
    pub id: String,
    pub name: String,
    /// When it was made (Unix seconds).
    pub time: u64,
    /// The client id space the branch makes ids in ([`Document::set_client_id`]), so they never
    /// clash with ids made on the main file or on other branches.
    pub client: u32,
    pub state: BranchState,
    pub base: Arc<Document>,
    pub head: Arc<Document>,
    /// The branch's own version history, oldest first.
    pub versions: Vec<Version>,
}

/// Whether `id` can name a version (and its archive entry).
pub fn is_valid_version_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `s` cut to at most `max` chars.
pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A file as read: the document, the repairs made (and versions skipped), and its version
/// history, oldest first.
#[derive(Debug)]
pub struct Loaded {
    pub doc: Document,
    pub issues: Vec<String>,
    pub versions: Vec<Version>,
    pub branches: Vec<Branch>,
}

impl From<Document> for Loaded {
    fn from(doc: Document) -> Self {
        Self { doc, issues: Vec::new(), versions: Vec::new(), branches: Vec::new() }
    }
}

/// Content hash used as an image's key.
pub fn image_hash(bytes: &[u8]) -> String {
    let d = sha2::Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Register encoded image bytes with the document; returns the hash and pixel size.
pub fn add_image(doc: &mut Document, bytes: Vec<u8>) -> Result<(String, u32, u32), FormatError> {
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| FormatError::Corrupt(e.to_string()))?
        .into_dimensions()
        .map_err(|e| FormatError::Corrupt(format!("unsupported image: {e}")))?;
    let hash = image_hash(&bytes);
    doc.images.insert(hash.clone(), ImageData { bytes: Arc::new(bytes), width: w, height: h });
    Ok((hash, w, h))
}

/// Image bytes with their pixel size (0×0 when they can't be decoded).
pub(crate) fn image_data(data: Vec<u8>) -> ImageData {
    let (w, h) =
        image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).unwrap_or((0, 0));
    ImageData { bytes: Arc::new(data), width: w, height: h }
}

/// `document.json`: the format header, the image index and the document's own fields, written
/// straight from the document (no intermediate JSON tree).
#[derive(Serialize)]
struct DocumentJson<'a> {
    format: &'static str,
    version: u32,
    #[serde(rename = "imageIndex", serialize_with = "image_index")]
    images: &'a BTreeMap<String, ImageData>,
    #[serde(rename = "fontIndex")]
    fonts: Vec<&'a String>,
    #[serde(flatten)]
    doc: &'a Document,
}

impl<'a> DocumentJson<'a> {
    fn new(doc: &'a Document) -> Self {
        Self { format: FORMAT, version: VERSION, images: &doc.images, fonts: doc.embedded_fonts.keys().collect(), doc }
    }
}

/// The `imageIndex`: `[{hash, width, height}]` for every image the document holds.
fn image_index<S: Serializer>(images: &&BTreeMap<String, ImageData>, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Entry<'a> {
        hash: &'a str,
        width: u32,
        height: u32,
    }
    s.collect_seq(images.iter().map(|(hash, i)| Entry { hash, width: i.width, height: i.height }))
}

/// Serialize the document (without images) to the JSON value stored in `document.json`.
pub fn to_json(doc: &Document) -> Result<Value, FormatError> {
    serde_json::to_value(DocumentJson::new(doc)).map_err(io_err)
}

/// Write the document (without images) as `document.json` content to `w`.
pub fn write_json(doc: &Document, w: impl Write) -> Result<(), FormatError> {
    serde_json::to_writer(w, &DocumentJson::new(doc)).map_err(io_err)
}

/// Parse `document.json` content.
pub fn from_json(text: &str) -> Result<(Document, Vec<String>), FormatError> {
    parse_json(text).map(|(doc, issues, _, _)| (doc, issues))
}

/// The image hashes a `document.json` lists in its `imageIndex` (`None` without one).
type ImageIndex = Option<BTreeSet<String>>;
/// The font hashes a `document.json` lists in its `fontIndex` (`None` without one).
type FontIndex = Option<BTreeSet<String>>;
/// A parsed document, its validation issues, and the image and font indexes.
type ParsedJson = (Document, Vec<String>, ImageIndex, FontIndex);

/// The header of a `document.json` (format version, image and font indexes), read in a pass
/// that skips over everything else without building it.
#[derive(Default)]
struct Header {
    version: Option<u64>,
    images: ImageIndex,
    fonts: FontIndex,
}

impl<'de> Deserialize<'de> for Header {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct HeaderVisitor;
        impl<'de> Visitor<'de> for HeaderVisitor {
            type Value = Header;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a FigCraft document object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Header, A::Error> {
                let mut h = Header::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "version" => h.version = map.next_value::<Value>()?.as_u64(),
                        "fontIndex" => {
                            h.fonts = map
                                .next_value::<Value>()?
                                .as_array()
                                .map(|values| values.iter().filter_map(Value::as_str).map(str::to_owned).collect());
                        }
                        "imageIndex" => {
                            h.images = map
                                .next_value::<Value>()?
                                .as_array()
                                .map(|a| a.iter().filter_map(|i| i.get("hash").and_then(Value::as_str).map(str::to_string)).collect());
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(h)
            }
        }
        d.deserialize_map(HeaderVisitor)
    }
}

/// Parse `document.json` content; also returns its image and font indexes. The header is checked
/// first (a newer file is refused before its document is read), then the document is deserialized
/// straight from the text.
fn parse_json(text: &str) -> Result<ParsedJson, FormatError> {
    let corrupt = |e: serde_json::Error| FormatError::Corrupt(e.to_string());
    let header: Header = serde_json::from_str(text).map_err(corrupt)?;
    if let Some(ver) = header.version
        && ver > u64::from(VERSION)
    {
        return Err(FormatError::Newer(u32::try_from(ver).unwrap_or(u32::MAX)));
    }
    let mut doc: Document = serde_json::from_str(text).map_err(corrupt)?;
    let issues = doc.validate();
    if !issues.is_empty() {
        doc.repair();
    }
    Ok((doc, issues, header.images, header.fonts))
}

pub(crate) fn io_err(e: impl std::fmt::Display) -> FormatError {
    FormatError::Io(e.to_string())
}

/// Add one member to the archive.
pub(crate) fn put<W: Write + Seek>(
    z: &mut zip::ZipWriter<W>,
    name: &str,
    opts: zip::write::SimpleFileOptions,
    bytes: &[u8],
) -> Result<(), FormatError> {
    z.start_file(name, opts).map_err(io_err)?;
    z.write_all(bytes).map_err(io_err)
}

/// Add `doc` as a `document.json`-style member, serialized straight into the compressor.
fn put_json<W: Write + Seek>(z: &mut zip::ZipWriter<W>, name: &str, doc: &Document) -> Result<(), FormatError> {
    z.start_file(name, json_options()).map_err(io_err)?;
    // The serializer writes many small pieces: batch them before they reach the compressor.
    let mut w = std::io::BufWriter::with_capacity(1 << 16, z);
    write_json(doc, &mut w)?;
    w.flush().map_err(io_err)
}

/// How JSON members are compressed. Document JSON is very repetitive: a fast level compresses it
/// nearly as well as the default one (about 15% larger) in half the time.
pub(crate) fn json_options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(JSON_LEVEL))
}

/// Write a `.figcraft` archive to bytes.
pub fn save_bytes(doc: &Document, thumbnail_png: Option<&[u8]>) -> Result<Vec<u8>, FormatError> {
    save_bytes_with_versions(doc, thumbnail_png, &[])
}

/// Write a `.figcraft` archive with a version history (oldest first) to bytes.
pub fn save_bytes_with_versions(doc: &Document, thumbnail_png: Option<&[u8]>, versions: &[Version]) -> Result<Vec<u8>, FormatError> {
    save_bytes_with_history(doc, thumbnail_png, versions, &[])
}

/// Write a `.figcraft` archive with a version history (oldest first) and branches to bytes.
pub fn save_bytes_with_history(
    doc: &Document,
    thumbnail_png: Option<&[u8]>,
    versions: &[Version],
    branches: &[Branch],
) -> Result<Vec<u8>, FormatError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        put_json(&mut z, "document.json", doc)?;
        // Every image once, whether the document, a version or a branch uses it.
        let mut written = BTreeSet::new();
        let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
        for (hash, img) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
            if written.insert(hash) {
                put(&mut z, &format!("images/{hash}"), stored, &img.bytes)?;
            }
        }
        // A snapshot/branch can use a font no longer present in the current file.
        // Store every distinct binary once, and retain ownership in each index.
        let branch_fonts =
            branches.iter().flat_map(|b| [&b.base.embedded_fonts, &b.head.embedded_fonts].into_iter().chain(b.versions.iter().map(Version::fonts)));
        let mut written_fonts = BTreeSet::new();
        let mut font_bytes = 0u64;
        for (hash, bytes) in std::iter::once(&doc.embedded_fonts).chain(versions.iter().map(Version::fonts)).chain(branch_fonts).flatten() {
            if written_fonts.contains(hash) {
                continue;
            }
            let len = bytes.len() as u64;
            if !valid_font_hash(hash)
                || !is_sfnt_font(bytes)
                || image_hash(bytes) != *hash
                || len > MAX_FONT_MEMBER
                || font_bytes.saturating_add(len) > MAX_FONT_TOTAL
                || written_fonts.len() >= MAX_FONT_COUNT
            {
                // Keep the file saveable. The indexed hash is retained so loading the file
                // reports that its binary is missing, including for versions and branches.
                continue;
            }
            written_fonts.insert(hash);
            font_bytes += len;
            put(&mut z, &format!("fonts/{hash}"), stored, bytes)?;
        }
        if let Some(t) = thumbnail_png {
            put(&mut z, "thumbnail.png", stored, t)?;
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
                let fonts = |d: &Document| d.embedded_fonts.keys().cloned().collect::<Vec<String>>();
                index.push(json!({"id": b.id, "name": b.name, "time": b.time, "client": b.client, "state": b.state,
                    "baseImages": images(&b.base), "headImages": images(&b.head),
                    "baseFonts": fonts(&b.base), "headFonts": fonts(&b.head)}));
            }
            put(&mut z, BRANCH_INDEX, json_options(), &serde_json::to_vec(&json!({"branches": index})).map_err(io_err)?)?;
        }
        if let Some(lib) = &doc.published {
            library::write(&mut z, lib, &written)?;
        }
        z.finish().map_err(io_err)?;
    }
    Ok(buf.into_inner())
}

/// Write version history `versions` (oldest first) under `dir`: `<dir>/index.json` and one
/// `<dir>/<id>.json` snapshot per version. Nothing when there are none.
fn put_versions<W: Write + Seek>(z: &mut zip::ZipWriter<W>, dir: &str, versions: &[Version]) -> Result<(), FormatError> {
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
            Snapshot::Doc(d) => put_json(z, &name, d)?,
            Snapshot::Stored { json, .. } => put(z, &name, json_options(), json)?,
        }
        let images: Vec<&String> = v.images().keys().collect();
        let fonts: Vec<&String> = v.fonts().keys().collect();
        let mut entry = json!({"id": v.id, "name": v.name, "description": v.description, "time": v.time, "images": images, "fonts": fonts});
        if v.auto {
            entry["auto"] = json!(true);
        }
        index.push(entry);
    }
    put(z, &format!("{dir}/index.json"), json_options(), &serde_json::to_vec(&json!({"versions": index})).map_err(io_err)?)?;
    Ok(())
}

/// Read a `.figcraft` archive (or bare JSON) from bytes. Returns the document and any repairs made.
pub fn load_bytes(bytes: &[u8]) -> Result<(Document, Vec<String>), FormatError> {
    load_bytes_with_versions(bytes).map(|l| (l.doc, l.issues))
}

/// Read archive member `name`, refusing members larger than `limit` bytes.
pub(crate) fn read_member<R: Read + Seek>(z: &mut zip::ZipArchive<R>, name: &str, limit: u64) -> Result<Vec<u8>, FormatError> {
    let f = z.by_name(name).map_err(|e| FormatError::Corrupt(format!("{name}: {e}")))?;
    let too_large = || FormatError::Corrupt(format!("{name} is too large"));
    if f.size() > limit {
        return Err(too_large());
    }
    let mut data = Vec::new();
    f.take(limit.saturating_add(1)).read_to_end(&mut data).map_err(|e| FormatError::Corrupt(format!("{name}: {e}")))?;
    if data.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(data)
}

/// Largest `document.json` read from an archive of `len` bytes in which it takes `packed`.
pub(crate) fn document_limit(packed: u64, len: u64) -> u64 {
    packed.min(len).saturating_mul(DOCUMENT_RATIO).clamp(MAX_MEMBER, MAX_DOCUMENT)
}

/// How many images, and how many bytes of them together, one file may give.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageBudget {
    pub images: usize,
    pub bytes: u64,
}

/// Read a `.figcraft` archive (or bare JSON) with its version history.
pub fn load_bytes_with_versions(bytes: &[u8]) -> Result<Loaded, FormatError> {
    load_within(bytes, IMAGE_BUDGET)
}

/// [`load_bytes_with_versions`] reading at most `budget` of images.
pub(crate) fn load_within(bytes: &[u8], budget: ImageBudget) -> Result<Loaded, FormatError> {
    let trimmed = bytes.iter().position(|b| !b.is_ascii_whitespace()).and_then(|i| bytes.get(i));
    if trimmed == Some(&b'{') {
        let text = std::str::from_utf8(bytes).map_err(|e| FormatError::Corrupt(e.to_string()))?;
        let (doc, issues) = from_json(text)?;
        return Ok(Loaded { issues, ..Loaded::from(doc) });
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let packed = z.by_name("document.json").map(|f| f.compressed_size()).unwrap_or(0);
    let limit = document_limit(packed, bytes.len() as u64);
    let text = String::from_utf8(read_member(&mut z, "document.json", limit)?).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let (mut doc, mut issues, own_images, own_fonts) = parse_json(&text)?;
    let (images, unread) = read_images(&mut z, own_images.as_ref(), budget);
    report_lost_images(&mut issues, &unread, own_images.as_ref(), &images);
    match library::read(&mut z, &images, u64::MAX) {
        Ok(lib) => doc.published = lib.map(Arc::new),
        Err(e) => issues.push(format!("the published library was skipped: {e}")),
    }
    let all_fonts = read_fonts(&mut z, &mut issues);
    let mut missing_fonts = MissingFontIssues::default();
    let mut budget = MAX_VERSION_BYTES;
    let versions = read_versions(&mut z, "versions", &images, &all_fonts, &mut missing_fonts, &mut issues, &mut budget);
    let branches = read_branches(&mut z, &images, &all_fonts, &mut missing_fonts, &mut issues);
    // Images only the version history or branches use stay with them.
    let branch_images =
        branches.iter().flat_map(|b| b.base.images.keys().chain(b.head.images.keys()).chain(b.versions.iter().flat_map(|v| v.images().keys())));
    let history_only: BTreeSet<&String> = versions
        .iter()
        .flat_map(|v| v.images().keys())
        .chain(branch_images)
        .filter(|h| own_images.as_ref().is_some_and(|o| !o.contains(*h)))
        .collect();
    doc.images = images.into_iter().filter(|(h, _)| !history_only.contains(h)).collect();
    // Files saved before `fontIndex` carried only fonts of the active document.
    // Keep that legacy behaviour; new archives retain precise snapshot ownership.
    doc.embedded_fonts = match own_fonts {
        Some(owned) => select_fonts(&all_fonts, owned.iter().map(String::as_str), &mut missing_fonts),
        None => all_fonts,
    };
    missing_fonts.report(&mut issues);
    Ok(Loaded { doc, issues, versions, branches })
}

/// Load font binaries saved by FigCraft, without extracting anything to disk. Damaged
/// fonts are reported instead of silently changing the document's typography.
fn read_fonts<R: Read + Seek>(z: &mut zip::ZipArchive<R>, issues: &mut Vec<String>) -> BTreeMap<String, Arc<Vec<u8>>> {
    let mut fonts = BTreeMap::new();
    let mut names: Vec<String> = z.file_names().filter(|n| n.starts_with("fonts/")).map(str::to_owned).collect();
    names.sort();
    if names.len() > MAX_FONT_COUNT {
        issues.push(format!("{} fonts exceed the embedded-font limit; extras were skipped", names.len() - MAX_FONT_COUNT));
    }
    let mut left = MAX_FONT_TOTAL;
    for name in names.into_iter().take(MAX_FONT_COUNT) {
        let Some(hash) = name.strip_prefix("fonts/").filter(|s| valid_font_hash(s)) else {
            issues.push("An embedded font has an invalid filename and was skipped".into());
            continue;
        };
        let limit = left.min(MAX_FONT_MEMBER);
        match read_member(z, &name, limit) {
            Ok(bytes) if is_sfnt_font(&bytes) && format!("{:x}", sha2::Sha256::digest(&bytes)) == hash => {
                left -= bytes.len() as u64;
                fonts.insert(hash.to_owned(), Arc::new(bytes));
            }
            Ok(_) => issues.push(format!("Embedded font {hash} failed validation; original font name stays in the text")),
            Err(err) => issues.push(format!("Embedded font {hash} could not be read: {err}")),
        }
    }
    fonts
}

/// Fonts listed by one snapshot, sharing their verified bytes with the ZIP reader.
fn select_fonts<'a>(
    all: &BTreeMap<String, Arc<Vec<u8>>>,
    hashes: impl Iterator<Item = &'a str>,
    missing: &mut MissingFontIssues,
) -> BTreeMap<String, Arc<Vec<u8>>> {
    hashes
        .filter_map(|h| match all.get(h) {
            Some(bytes) => Some((h.to_owned(), bytes.clone())),
            None => {
                missing.note(h);
                None
            }
        })
        .collect()
}

/// The archive's `images/` members by hash, within `budget`, and the ones that couldn't be read
/// with why. The document's own images (`own`) are read first, so a file past the budget loses
/// the images only its history uses before its own.
fn read_images<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    own: Option<&BTreeSet<String>>,
    budget: ImageBudget,
) -> (BTreeMap<String, ImageData>, Vec<(String, FormatError)>) {
    let mut hashes: Vec<String> = z.file_names().filter_map(|n| n.strip_prefix("images/")).filter(|h| !h.is_empty()).map(str::to_string).collect();
    hashes.sort_by_key(|h| !own.is_some_and(|o| o.contains(h)));
    let mut images = BTreeMap::new();
    let mut unread = Vec::new();
    // Member names that aren't plain hashes (`../x`, `a/b`) are never read, so never written back.
    hashes.retain(|h| {
        let ok = library::is_valid_hash(h);
        if !ok {
            unread.push((h.clone(), FormatError::Corrupt("its name isn't a valid image hash".to_string())));
        }
        ok
    });
    let mut left = budget.bytes;
    for (i, hash) in hashes.into_iter().enumerate() {
        let name = format!("images/{hash}");
        if i >= budget.images {
            unread.push((hash, FormatError::Corrupt(format!("the file has more than {} images", budget.images))));
            continue;
        }
        if z.by_name(&name).is_ok_and(|f| f.size() > left) {
            let mb = budget.bytes / (1024 * 1024);
            unread.push((hash, FormatError::Corrupt(format!("the file's images together pass {mb} MB"))));
            continue;
        }
        match read_member(z, &name, left.min(MAX_MEMBER)) {
            Ok(data) => {
                left = left.saturating_sub(data.len() as u64);
                images.insert(hash, image_data(data));
            }
            Err(e) => unread.push((hash, e)),
        }
    }
    (images, unread)
}

/// Most lost images reported one by one (the rest as a count).
const MAX_IMAGE_ISSUES: usize = 20;

/// Report the images a file lost on loading: members that couldn't be read (damaged, too large)
/// and images its document lists but the archive doesn't hold. They are left out, so saving the
/// file again would drop them for good.
fn report_lost_images(
    issues: &mut Vec<String>,
    unread: &[(String, FormatError)],
    listed: Option<&BTreeSet<String>>,
    images: &BTreeMap<String, ImageData>,
) {
    let mut lost: Vec<String> = unread
        .iter()
        .map(|(hash, e)| {
            let why = match e {
                FormatError::Corrupt(m) | FormatError::Io(m) => m.clone(),
                FormatError::Newer(v) => format!("format version {v}"),
            };
            format!("image {hash} couldn't be read ({why}): it is left out, and saving this file again drops it")
        })
        .collect();
    let unread_hashes: BTreeSet<&str> = unread.iter().map(|(h, _)| h.as_str()).collect();
    for hash in listed.into_iter().flatten() {
        if !images.contains_key(hash) && !unread_hashes.contains(hash.as_str()) {
            lost.push(format!("image {hash} is used by the file but its data is missing: it is left out, and saving this file again drops it"));
        }
    }
    let more = lost.len().saturating_sub(MAX_IMAGE_ISSUES);
    lost.truncate(MAX_IMAGE_ISSUES);
    issues.extend(lost);
    if more > 0 {
        issues.push(format!("{more} more images couldn't be loaded"));
    }
}

/// The version history kept under `dir` in an archive (none when it has no index), reading at
/// most `budget` bytes of snapshots. Damaged entries are skipped and reported in `issues`; nothing
/// here fails the load.
fn read_versions<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    dir: &str,
    images: &BTreeMap<String, ImageData>,
    fonts: &BTreeMap<String, Arc<Vec<u8>>>,
    missing_fonts: &mut MissingFontIssues,
    issues: &mut Vec<String>,
    budget: &mut u64,
) -> Vec<Version> {
    let index_name = format!("{dir}/index.json");
    if z.index_for_name(&index_name).is_none() {
        return Vec::new();
    }
    let index = match read_member(z, &index_name, MAX_VERSION_INDEX)
        .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| FormatError::Corrupt(e.to_string())))
    {
        Ok(v) => v,
        Err(e) => {
            issues.push(format!("the version history was skipped: {e}"));
            return Vec::new();
        }
    };
    let entries = index.get("versions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut out: Vec<Version> = Vec::new();
    for e in entries {
        if out.len() >= MAX_VERSIONS {
            issues.push(format!("only the first {MAX_VERSIONS} versions were kept"));
            break;
        }
        let id = e.get("id").and_then(Value::as_str).unwrap_or_default();
        if !is_valid_version_id(id) || out.iter().any(|v| v.id == id) {
            issues.push("a version with an invalid id was skipped".into());
            continue;
        }
        let json = match read_member(z, &format!("{dir}/{id}.json"), (*budget).min(MAX_MEMBER)) {
            Ok(j) => j,
            Err(err) => {
                issues.push(format!("version `{id}` was skipped: {err}"));
                continue;
            }
        };
        *budget = budget.saturating_sub(json.len() as u64);
        let text = |k: &str, max: usize| e.get(k).and_then(Value::as_str).map(|s| clip(s, max)).unwrap_or_default();
        let images = e
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|h| images.get(h).map(|i| (h.to_string(), i.clone())))
            .collect();
        let owned_fonts = e.get("fonts").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        out.push(Version {
            id: id.to_string(),
            name: text("name", MAX_VERSION_NAME),
            description: text("description", MAX_VERSION_DESCRIPTION),
            time: e.get("time").and_then(Value::as_u64).unwrap_or(0),
            auto: e.get("auto").and_then(Value::as_bool).unwrap_or(false),
            snapshot: Snapshot::Stored { json: Arc::new(json), images, fonts: select_fonts(fonts, owned_fonts, missing_fonts) },
        });
    }
    out
}

/// The branches of an archive (none when it has no index), parsed. Damaged entries are skipped and
/// reported in `issues`; nothing here fails the load.
fn read_branches<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    images: &BTreeMap<String, ImageData>,
    fonts: &BTreeMap<String, Arc<Vec<u8>>>,
    missing_fonts: &mut MissingFontIssues,
    issues: &mut Vec<String>,
) -> Vec<Branch> {
    if z.index_for_name(BRANCH_INDEX).is_none() {
        return Vec::new();
    }
    let index = match read_member(z, BRANCH_INDEX, MAX_VERSION_INDEX)
        .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| FormatError::Corrupt(e.to_string())))
    {
        Ok(v) => v,
        Err(e) => {
            issues.push(format!("the branches were skipped: {e}"));
            return Vec::new();
        }
    };
    let entries = index.get("branches").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut out: Vec<Branch> = Vec::new();
    let mut budget = MAX_VERSION_BYTES;
    for e in entries {
        if out.len() >= MAX_BRANCHES {
            issues.push(format!("only the first {MAX_BRANCHES} branches were kept"));
            break;
        }
        let id = e.get("id").and_then(Value::as_str).unwrap_or_default();
        if !is_valid_version_id(id) || out.iter().any(|b| b.id == id) {
            issues.push("a branch with an invalid id was skipped".into());
            continue;
        }
        let mut snapshot = |part: &str, list: &str| -> Result<Arc<Document>, FormatError> {
            let json = read_member(z, &format!("branches/{id}/{part}.json"), budget.min(MAX_MEMBER))?;
            budget = budget.saturating_sub(json.len() as u64);
            let (mut doc, _) = from_json(std::str::from_utf8(&json).map_err(|e| FormatError::Corrupt(e.to_string()))?)?;
            let hashes = e.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
            doc.images = hashes.filter_map(|h| images.get(h).map(|i| (h.to_string(), i.clone()))).collect();
            let font_key = if part == "base" { "baseFonts" } else { "headFonts" };
            let owned = e.get(font_key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
            doc.embedded_fonts = select_fonts(fonts, owned, missing_fonts);
            Ok(Arc::new(doc))
        };
        let (base, head) = match (snapshot("base", "baseImages"), snapshot("head", "headImages")) {
            (Ok(b), Ok(h)) => (b, h),
            (Err(err), _) | (_, Err(err)) => {
                issues.push(format!("branch `{id}` was skipped: {err}"));
                continue;
            }
        };
        out.push(Branch {
            id: id.to_string(),
            name: e.get("name").and_then(Value::as_str).map(|s| clip(s, MAX_VERSION_NAME)).unwrap_or_default(),
            time: e.get("time").and_then(Value::as_u64).unwrap_or(0),
            client: e.get("client").and_then(Value::as_u64).and_then(|c| u32::try_from(c).ok()).unwrap_or(0),
            state: e.get("state").and_then(|s| BranchState::deserialize(s).ok()).unwrap_or(BranchState::Active),
            versions: read_versions(z, &format!("branches/{id}/versions"), images, fonts, missing_fonts, issues, &mut budget),
            base,
            head,
        });
    }
    out
}

/// Save atomically: write a staging file next to `path`, then rename over it.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_file(doc: &Document, path: &std::path::Path, thumbnail_png: Option<&[u8]>) -> Result<(), FormatError> {
    let bytes = save_bytes(doc, thumbnail_png)?;
    write_file_atomic(path, &bytes).map_err(|e| FormatError::Io(format!("{}: {e}", path.display())))
}

/// Write `bytes` to `path` through a staging file and one atomic rename, so a reader of `path`
/// sees either the previous file or the complete new one, never a partial write. The staging file
/// is a uniquely named sibling (`<path>.tmp-figcraft-<process id>-<counter>`) created exclusively
/// (`create_new`) and written through its own handle: a file or symlink that already exists at a
/// staging name is never truncated or followed, and separate writers (also of separate processes)
/// never share a staging file. On failure only the staging file this call created is removed, and
/// `path` is replaced only on success. The rename replaces an existing destination on every
/// platform (`std::fs::rename` uses `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` on Windows).
/// (No `sync_all` yet: nothing around here promises durability across a power cut.)
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static STAGING: AtomicU64 = AtomicU64::new(0);
    // Three tries at a fresh name: the counter is unique within this process, so only a leftover
    // of a dead process that now shares our process id can collide, and the next counter won't.
    for _ in 0..3 {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".tmp-figcraft-{}-{}", std::process::id(), STAGING.fetch_add(1, Relaxed)));
        let tmp = std::path::PathBuf::from(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(bytes) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                return match std::fs::rename(&tmp, path) {
                    Ok(()) => Ok(()),
                    // Leave nothing behind when the file can't take its place.
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free staging name next to the file"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(path: &std::path::Path) -> Result<(Document, Vec<String>), FormatError> {
    let bytes = std::fs::read(path).map_err(|e| FormatError::Io(format!("{}: {e}", path.display())))?;
    load_bytes(&bytes)
}

#[cfg(test)]
mod branch_tests;
#[cfg(test)]
mod image_budget_tests;
#[cfg(test)]
mod image_tests;
#[cfg(test)]
mod large_tests;
#[cfg(test)]
mod staging_tests;
#[cfg(test)]
mod thumbnail_tests;
#[cfg(test)]
mod version_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::NodeKind;

    fn png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn roundtrip_with_images() {
        let mut d = Document::new("My file");
        let p = d.first_page().unwrap();
        d.create(p, NodeKind::Rectangle, "R", 1.0, 2.0, 3.0, 4.0).unwrap();
        let (hash, w, h) = add_image(&mut d, png()).unwrap();
        assert_eq!((w, h), (3, 2));
        let bytes = save_bytes(&d, None).unwrap();
        let (back, issues) = load_bytes(&bytes).unwrap();
        assert!(issues.is_empty());
        assert_eq!(back.name, "My file");
        assert_eq!(back.node_count(), d.node_count());
        assert!(back.images.contains_key(&hash));
        // Bare JSON opens too.
        let json = serde_json::to_vec(&to_json(&d).unwrap()).unwrap();
        assert!(load_bytes(&json).is_ok());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(load_bytes(b"not a zip").is_err());
        assert!(load_bytes(b"{\"nope\":1}").is_err());
        assert!(matches!(load_bytes(b"{\"version\":999}"), Err(FormatError::Newer(999))));
        assert!(add_image(&mut Document::new("x"), b"zzz".to_vec()).is_err());
    }
}

#[cfg(test)]
mod portable_font_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn portable_font_survives_save_and_reopen() {
        // Minimal SFNT signature is sufficient for archive persistence; rendering
        // validates font tables in figcraft-text, which can reject this stub.
        let font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let hash = format!("{:x}", sha2::Sha256::digest(&font));
        let mut doc = Document::new("Embedded-font persistence");
        doc.embedded_fonts.insert(hash.clone(), Arc::new(font.clone()));
        doc.embedded_font_licenses.insert(hash.clone(), "SIL OPEN FONT LICENSE Version 1.1".into());
        let bytes = save_bytes(&doc, None).unwrap();
        let (loaded, issues) = load_bytes(&bytes).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(loaded.embedded_fonts.get(&hash).map(|a| a.as_slice()), Some(font.as_slice()));
        assert_eq!(loaded.embedded_font_licenses.get(&hash).map(String::as_str), Some("SIL OPEN FONT LICENSE Version 1.1"));
    }

    // Regression: versions and branch heads may own faces that the active file no longer uses.
    // They must survive multiple save/reopen cycles without leaking into the active design.
    #[test]
    fn historical_fonts_survive_save_and_reopen() {
        fn font(seed: u8) -> (String, Arc<Vec<u8>>) {
            let bytes = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, seed];
            (format!("{:x}", sha2::Sha256::digest(&bytes)), Arc::new(bytes))
        }
        let current = Document::new("Current has no fonts");
        let mut historic = Document::new("Old typeface");
        let (version_hash, version_font) = font(1);
        historic.embedded_fonts.insert(version_hash.clone(), version_font.clone());
        historic.embedded_font_licenses.insert(version_hash.clone(), "OFL notice".into());
        let version = Version::new("font-v1", "Typeface", "", 1, Arc::new(historic));

        let mut head = current.clone();
        let (branch_hash, branch_font) = font(2);
        head.embedded_fonts.insert(branch_hash.clone(), branch_font.clone());
        let branch = Branch {
            id: "font-branch".into(),
            name: "Branch".into(),
            time: 2,
            client: 3,
            state: BranchState::Active,
            base: Arc::new(current.clone()),
            head: Arc::new(head),
            versions: vec![version.clone()],
        };
        let bytes = save_bytes_with_history(&current, None, &[version], &[branch]).unwrap();
        let zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let binary_members = zip.file_names().filter(|n| n.starts_with("fonts/")).count();
        assert_eq!(binary_members, 2, "shared fonts are written once");
        for archive in [bytes.clone(), {
            let loaded = load_bytes_with_versions(&bytes).unwrap();
            save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap()
        }] {
            let loaded = load_bytes_with_versions(&archive).unwrap();
            assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
            assert!(loaded.doc.embedded_fonts.is_empty(), "history fonts must not leak into the active design");
            let version_doc = loaded.versions[0].document().unwrap();
            assert_eq!(version_doc.embedded_fonts.get(&version_hash), Some(&version_font));
            assert_eq!(version_doc.embedded_font_licenses[&version_hash], "OFL notice");
            assert!(loaded.branches[0].base.embedded_fonts.is_empty());
            assert_eq!(loaded.branches[0].head.embedded_fonts.get(&branch_hash), Some(&branch_font));
            assert_eq!(loaded.branches[0].versions[0].document().unwrap().embedded_fonts.get(&version_hash), Some(&version_font));
        }
    }

    #[test]
    fn skipped_fonts_are_reported_for_active_version_and_branch_owners() {
        let broken = |seed: u8| {
            let bytes = vec![seed; 12]; // Not an SFNT; save deliberately omits it.
            let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
            (hash, Arc::new(bytes))
        };
        let mut doc = Document::new("Active invalid font");
        let (active_hash, active_bytes) = broken(1);
        doc.embedded_fonts.insert(active_hash.clone(), active_bytes);
        let mut previous = Document::new("Version invalid font");
        let (version_hash, version_bytes) = broken(2);
        previous.embedded_fonts.insert(version_hash.clone(), version_bytes);
        let version = Version::new("v1", "Earlier", "", 1, Arc::new(previous));
        let mut head = Document::new("Branch invalid font");
        let (branch_hash, branch_bytes) = broken(3);
        head.embedded_fonts.insert(branch_hash.clone(), branch_bytes);
        let mut branch_previous = Document::new("Branch version invalid font");
        let (branch_version_hash, branch_version_bytes) = broken(4);
        branch_previous.embedded_fonts.insert(branch_version_hash.clone(), branch_version_bytes);
        let branch = Branch {
            id: "branch-1".into(),
            name: "Bad font branch".into(),
            time: 2,
            client: 3,
            state: BranchState::Active,
            base: Arc::new(Document::new("Empty")),
            head: Arc::new(head),
            versions: vec![Version::new("branch-version", "Old branch", "", 1, Arc::new(branch_previous))],
        };
        let bytes = save_bytes_with_history(&doc, None, &[version], &[branch]).unwrap();
        let loaded = load_bytes_with_versions(&bytes).unwrap();
        assert!(loaded.doc.embedded_fonts.is_empty());
        assert_eq!(loaded.versions.len(), 1);
        assert_eq!(loaded.branches.len(), 1);
        for hash in [active_hash, version_hash, branch_hash, branch_version_hash] {
            assert!(loaded.issues.iter().any(|issue| issue.contains(&hash) && issue.contains("missing")), "no issue for {hash}: {:?}", loaded.issues);
        }
    }

    #[test]
    fn missing_font_diagnostics_have_a_fixed_size_budget() {
        let mut doc = Document::new("Many invalid font binaries");
        for seed in 0..(MAX_FONT_ISSUES + 50) {
            let bytes = vec![seed as u8; 12];
            let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
            doc.embedded_fonts.insert(hash, Arc::new(bytes));
        }
        let archive = save_bytes(&doc, None).unwrap();
        let (_, issues) = load_bytes(&archive).unwrap();
        assert_eq!(issues.len(), MAX_FONT_ISSUES + 1, "{issues:?}");
        assert!(issues.last().is_some_and(|message| message.contains("additional references")));
    }

    #[test]
    fn merged_portable_fonts_obey_limits_and_keep_licenses() {
        let mut source = Document::new("Downloaded faces");
        for seed in 0..130u16 {
            let mut font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
            font.extend(seed.to_le_bytes());
            let hash = image_hash(&font);
            source.embedded_fonts.insert(hash.clone(), Arc::new(font));
            source.embedded_font_licenses.insert(hash, "OFL notice".into());
        }
        let mut target = Document::new("Existing project");
        let (added, skipped) = merge_embedded_fonts(&mut target, &source);
        assert_eq!((added, skipped), (256, 2));
        assert_eq!(target.embedded_fonts.len(), 128);
        assert_eq!(target.embedded_font_licenses.len(), 128);
        // Merging twice never duplicates fonts or license records.
        let (added, skipped) = merge_embedded_fonts(&mut target, &source);
        assert_eq!((added, skipped), (0, 2));
        let bytes = save_bytes(&target, None).unwrap();
        let (loaded, issues) = load_bytes(&bytes).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(loaded.embedded_fonts.len(), 128);
    }

    #[test]
    fn invalid_fonts_do_not_prevent_saving_other_assets() {
        let mut doc = Document::new("Invalid font bytes");
        doc.embedded_fonts.insert("not-a-sha256".into(), Arc::new(vec![0, 1, 2, 3]));
        let bytes = save_bytes(&doc, None).unwrap();
        let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
        assert_eq!(archive.file_names().filter(|n| n.starts_with("fonts/")).count(), 0);
        let (opened, issues) = load_bytes(&bytes).unwrap();
        assert!(opened.embedded_fonts.is_empty());
        assert!(issues.iter().any(|issue| issue.contains("not-a-sha256") && issue.contains("missing")), "{issues:?}");
    }

    #[test]
    fn tampered_embedded_font_is_reported_not_used() {
        let font = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let hash = format!("{:x}", sha2::Sha256::digest(&font));
        let mut doc = Document::new("Font integrity");
        doc.embedded_fonts.insert(hash.clone(), Arc::new(font));
        let archive = save_bytes(&doc, None).unwrap();
        let mut source = zip::ZipArchive::new(std::io::Cursor::new(&archive)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..source.len() {
            let mut entry = source.by_index(i).unwrap();
            let name = entry.name().to_owned();
            output.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            if name.starts_with("fonts/") {
                output.write_all(b"NOT A VALID FONT").unwrap();
            } else {
                std::io::copy(&mut entry, &mut output).unwrap();
            }
        }
        let tampered = output.finish().unwrap().into_inner();
        let (loaded, issues) = load_bytes(&tampered).unwrap();
        assert!(loaded.embedded_fonts.is_empty());
        assert!(issues.iter().any(|e| e.contains("Embedded font")), "{issues:?}");
    }
}
