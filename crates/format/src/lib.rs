//! The native `.figcraft` format.
//!
//! A zip archive holding `document.json` (the [`Document`] as JSON, with a `format`/`version`
//! header; layer fields at their default values are left out and read back as those defaults),
//! `images/<sha256>` for every image the document (or a saved version) uses, an optional
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
pub mod rest;
pub mod thumbnail;

pub use library::{is_valid_library_id, read_library, read_library_within};

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, Write};
use std::sync::Arc;

use figcraft_doc::{Document, ImageData, MAX_VIDEO_BYTES, VideoData, degrade_videos, used_videos, validate_videos};
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::Digest;

pub const EXTENSION: &str = "figcraft";
pub const FORMAT: &str = "figcraft";
pub const VERSION: u32 = 2;
pub const MAX_VIDEO_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
/// Largest archive member we read (guards against zip bombs).
const MAX_MEMBER: u64 = 512 * 1024 * 1024;
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
    /// it uses.
    Stored { json: Arc<Vec<u8>>, images: BTreeMap<String, ImageData>, videos: BTreeMap<String, VideoData> },
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
            Snapshot::Stored { json, images, videos } => {
                let text = std::str::from_utf8(json).map_err(|e| FormatError::Corrupt(e.to_string()))?;
                let (mut doc, _) = from_json(text)?;
                doc.images = images.clone();
                doc.videos = videos.clone();
                drop_dangling_videos(&mut doc);
                validate_video_store(&doc)?;
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

    pub fn videos(&self) -> &BTreeMap<String, VideoData> {
        match &self.snapshot {
            Snapshot::Doc(d) => &d.videos,
            Snapshot::Stored { videos, .. } => videos,
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

/// Register bounded encoded video bytes with the document; decoding belongs to the playback backend.
pub fn add_video(doc: &mut Document, bytes: Vec<u8>) -> Result<String, FormatError> {
    if bytes.len() > MAX_VIDEO_BYTES {
        return Err(FormatError::Corrupt("video exceeds the 16 MiB limit".into()));
    }
    let hash = image_hash(&bytes);
    let mut videos = doc.videos.clone();
    videos.insert(hash.clone(), VideoData { bytes: Arc::new(bytes) });
    validate_videos(&videos).map_err(FormatError::Corrupt)?;
    doc.videos = videos;
    Ok(hash)
}

/// Image bytes with their pixel size (0×0 when they can't be decoded).
pub(crate) fn image_data(data: Vec<u8>) -> ImageData {
    let (w, h) =
        image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).unwrap_or((0, 0));
    ImageData { bytes: Arc::new(data), width: w, height: h }
}

fn valid_video_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Show the poster for every video paint whose bytes the document does not hold, so a damaged
/// file can still be edited and saved. Returns how many paints changed.
fn drop_dangling_videos(doc: &mut Document) -> usize {
    let held: BTreeSet<String> = doc.videos.keys().cloned().collect();
    degrade_videos(doc, |hash| held.contains(hash))
}

/// What saving checks for a version: a parsed snapshot fully; a stored one (kept verbatim) only
/// its video bytes, so an unreadable stored snapshot never blocks saving.
fn validate_version_videos(version: &Version) -> Result<(), FormatError> {
    match &version.snapshot {
        Snapshot::Doc(d) => validate_video_store(d),
        Snapshot::Stored { videos, .. } => validate_stored_videos(videos),
    }
}

fn validate_stored_videos(videos: &BTreeMap<String, VideoData>) -> Result<(), FormatError> {
    validate_videos(videos).map_err(FormatError::Corrupt)?;
    match videos.iter().find(|(hash, video)| !valid_video_hash(hash) || image_hash(&video.bytes) != **hash) {
        Some((hash, _)) => Err(FormatError::Corrupt(format!("video {hash} has an invalid content hash"))),
        None => Ok(()),
    }
}

/// `doc` as it is saved, when that differs from `doc`: video paints without bytes show their
/// poster and, with `prune`, videos no layer (or published library asset) paints are left out.
fn healed_document(doc: &Document, prune: bool) -> Option<Document> {
    let mut used = used_videos(doc);
    let dangling = used.iter().any(|h| !doc.videos.contains_key(h));
    let unused = prune && doc.videos.keys().any(|h| !used.contains(h));
    let lib_dangling = doc.published.as_ref().is_some_and(|lib| used_videos(&lib.doc).iter().any(|h| !lib.doc.videos.contains_key(h)));
    if !dangling && !unused && !lib_dangling {
        return None;
    }
    let mut d = doc.clone();
    if dangling {
        drop_dangling_videos(&mut d);
        used = used_videos(&d);
    }
    if let Some(lib) = d.published.as_mut().filter(|_| lib_dangling) {
        drop_dangling_videos(Arc::make_mut(&mut Arc::make_mut(lib).doc));
    }
    if let Some(lib) = &d.published {
        used.extend(used_videos(&lib.doc));
    }
    if prune {
        d.videos.retain(|hash, _| used.contains(hash));
    }
    Some(d)
}

/// A history entry as it is saved: a parsed snapshot is healed like the document; a stored one is
/// written as it is read.
fn healed_version(v: &Version) -> Version {
    match &v.snapshot {
        Snapshot::Doc(d) => match healed_document(d, false) {
            Some(h) => Version { snapshot: Snapshot::Doc(Arc::new(h)), ..v.clone() },
            None => v.clone(),
        },
        Snapshot::Stored { .. } => v.clone(),
    }
}

fn validate_video_store(doc: &Document) -> Result<(), FormatError> {
    validate_videos(&doc.videos).map_err(FormatError::Corrupt)?;
    for (hash, video) in &doc.videos {
        if !valid_video_hash(hash) || image_hash(&video.bytes) != *hash {
            return Err(FormatError::Corrupt(format!("video {hash} has an invalid content hash")));
        }
    }
    if let Some(missing) = used_videos(doc).into_iter().find(|hash| !doc.videos.contains_key(hash)) {
        return Err(FormatError::Corrupt(format!("video {missing} is referenced but its bytes are missing")));
    }
    Ok(())
}

/// `document.json`: the format header, the image index and the document's own fields, written
/// straight from the document (no intermediate JSON tree).
#[derive(Serialize)]
struct DocumentJson<'a> {
    format: &'static str,
    version: u32,
    #[serde(rename = "imageIndex", serialize_with = "image_index")]
    images: &'a BTreeMap<String, ImageData>,
    #[serde(rename = "videoIndex", serialize_with = "video_index")]
    videos: &'a BTreeMap<String, VideoData>,
    #[serde(flatten)]
    doc: &'a Document,
}

impl<'a> DocumentJson<'a> {
    fn new(doc: &'a Document) -> Self {
        Self::at_version(doc, if doc.videos.is_empty() && used_videos(doc).is_empty() { 1 } else { 2 })
    }

    fn at_version(doc: &'a Document, version: u32) -> Self {
        Self { format: FORMAT, version, images: &doc.images, videos: &doc.videos, doc }
    }
}

fn video_index<S: Serializer>(videos: &&BTreeMap<String, VideoData>, s: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Entry<'a> {
        hash: &'a str,
    }
    s.collect_seq(videos.keys().map(|hash| Entry { hash }))
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
    let (doc, mut issues, _, videos) = parse_json(text)?;
    let hashes: BTreeSet<String> = used_videos(&doc).into_iter().chain(videos.into_iter().flatten()).collect();
    if !hashes.is_empty() {
        issues.extend(
            hashes.into_iter().take(MAX_IMAGE_ISSUES).map(|hash| format!("video {hash} has no bytes in bare JSON: saving this file again drops it")),
        );
    }
    Ok((doc, issues))
}

/// The image hashes a `document.json` lists in its `imageIndex` (`None` without one).
type ImageIndex = Option<BTreeSet<String>>;

/// The header of a `document.json` (its format version and image index), read in a pass that
/// skips over everything else without building it.
#[derive(Default)]
struct Header {
    version: Option<u64>,
    images: ImageIndex,
    videos: ImageIndex,
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
                        "imageIndex" => {
                            h.images = map
                                .next_value::<Value>()?
                                .as_array()
                                .map(|a| a.iter().filter_map(|i| i.get("hash").and_then(Value::as_str).map(str::to_string)).collect());
                        }
                        "videoIndex" => {
                            h.videos = map
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

/// Parse `document.json` content; also returns its [`ImageIndex`]. The header is checked first
/// (a newer file is refused before its document is read), then the document is deserialized
/// straight from the text.
fn parse_json(text: &str) -> Result<(Document, Vec<String>, ImageIndex, ImageIndex), FormatError> {
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
    Ok((doc, issues, header.images, header.videos))
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
    put_json_at_version(z, name, doc, DocumentJson::new(doc).version)
}

fn put_json_at_version<W: Write + Seek>(z: &mut zip::ZipWriter<W>, name: &str, doc: &Document, version: u32) -> Result<(), FormatError> {
    z.start_file(name, json_options()).map_err(io_err)?;
    // The serializer writes many small pieces: batch them before they reach the compressor.
    let mut w = std::io::BufWriter::with_capacity(1 << 16, z);
    serde_json::to_writer(&mut w, &DocumentJson::at_version(doc, version)).map_err(io_err)?;
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
    // Saving never fails over video bytes that are missing: such paints show their poster in the
    // file. Videos nothing in the current document paints any more are not written.
    let healed_doc;
    let doc = match healed_document(doc, true) {
        Some(d) => {
            healed_doc = d;
            &healed_doc
        }
        None => doc,
    };
    let versions: Vec<Version> = versions.iter().map(healed_version).collect();
    let versions = versions.as_slice();
    let branches: Vec<Branch> = branches
        .iter()
        .map(|b| Branch {
            base: healed_document(&b.base, false).map_or_else(|| b.base.clone(), Arc::new),
            head: healed_document(&b.head, false).map_or_else(|| b.head.clone(), Arc::new),
            versions: b.versions.iter().map(healed_version).collect(),
            ..b.clone()
        })
        .collect();
    let branches = branches.as_slice();
    validate_video_store(doc)?;
    for version in versions {
        validate_version_videos(version)?;
    }
    for branch in branches {
        validate_video_store(&branch.base)?;
        validate_video_store(&branch.head)?;
        for version in &branch.versions {
            validate_version_videos(version)?;
        }
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let contains_video = !doc.videos.is_empty()
            || !used_videos(doc).is_empty()
            || versions.iter().any(|v| !v.videos().is_empty())
            || branches.iter().any(|b| !b.base.videos.is_empty() || !b.head.videos.is_empty() || b.versions.iter().any(|v| !v.videos().is_empty()))
            || doc.published.as_ref().is_some_and(|lib| !lib.doc.videos.is_empty() || !used_videos(&lib.doc).is_empty());
        put_json_at_version(&mut z, "document.json", doc, if contains_video { 2 } else { 1 })?;
        // Every image once, whether the document, a version or a branch uses it.
        let mut written = BTreeSet::new();
        let branch_images = branches.iter().flat_map(|b| [&b.base.images, &b.head.images].into_iter().chain(b.versions.iter().map(Version::images)));
        for (hash, img) in std::iter::once(&doc.images).chain(versions.iter().map(Version::images)).chain(branch_images).flatten() {
            if written.insert(hash) {
                put(&mut z, &format!("images/{hash}"), stored, &img.bytes)?;
            }
        }
        let mut written_videos = BTreeSet::new();
        let branch_videos = branches.iter().flat_map(|b| [&b.base.videos, &b.head.videos].into_iter().chain(b.versions.iter().map(Version::videos)));
        let mut total_video_bytes = 0u64;
        for (hash, video) in std::iter::once(&doc.videos).chain(versions.iter().map(Version::videos)).chain(branch_videos).flatten() {
            if written_videos.insert(hash) {
                total_video_bytes = total_video_bytes.saturating_add(video.bytes.len() as u64);
                if total_video_bytes > MAX_VIDEO_ARCHIVE_BYTES {
                    return Err(FormatError::Corrupt("video archive exceeds the 256 MiB limit".into()));
                }
                put(&mut z, &format!("videos/{hash}"), stored, &video.bytes)?;
            }
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
                let videos = |d: &Document| d.videos.keys().cloned().collect::<Vec<String>>();
                index.push(json!({"id": b.id, "name": b.name, "time": b.time, "client": b.client, "state": b.state,
                    "baseImages": images(&b.base), "headImages": images(&b.head), "baseVideos": videos(&b.base), "headVideos": videos(&b.head)}));
            }
            put(&mut z, BRANCH_INDEX, json_options(), &serde_json::to_vec(&json!({"branches": index})).map_err(io_err)?)?;
        }
        if let Some(lib) = &doc.published {
            validate_video_store(&lib.doc)?;
            let library_video_bytes = lib
                .doc
                .videos
                .iter()
                .filter(|(hash, _)| !written_videos.contains(*hash))
                .map(|(_, video)| video.bytes.len() as u64)
                .fold(0u64, u64::saturating_add);
            if total_video_bytes.saturating_add(library_video_bytes) > MAX_VIDEO_ARCHIVE_BYTES {
                return Err(FormatError::Corrupt("video archive exceeds the 256 MiB limit".into()));
            }
            library::write(&mut z, lib, &written, &written_videos)?;
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
        let videos: Vec<&String> = v.videos().keys().collect();
        let mut entry = json!({"id": v.id, "name": v.name, "description": v.description, "time": v.time, "images": images, "videos": videos});
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
        let (mut doc, issues) = from_json(text)?;
        // Bare JSON has no video bytes: the fills keep their posters.
        drop_dangling_videos(&mut doc);
        return Ok(Loaded { issues, ..Loaded::from(doc) });
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let text = String::from_utf8(read_member(&mut z, "document.json", MAX_MEMBER)?).map_err(|e| FormatError::Corrupt(e.to_string()))?;
    let (mut doc, mut issues, own_images, own_videos) = parse_json(&text)?;
    let (images, unread) = read_images(&mut z, own_images.as_ref(), budget);
    report_lost_images(&mut issues, &unread, own_images.as_ref(), &images);
    let names: Vec<String> = z.file_names().filter(|n| n.starts_with("videos/")).map(str::to_string).collect();
    let mut videos = BTreeMap::new();
    let mut unread_videos = Vec::new();
    let mut total_video_bytes = 0u64;
    for name in names {
        let hash = name.trim_start_matches("videos/").to_string();
        let read = if !valid_video_hash(&hash) {
            Err(FormatError::Corrupt("invalid video content hash".into()))
        } else if total_video_bytes >= MAX_VIDEO_ARCHIVE_BYTES {
            Err(FormatError::Corrupt("video archive exceeds the 256 MiB limit".into()))
        } else {
            read_member(&mut z, &name, MAX_VIDEO_BYTES as u64)
        };
        match read {
            Ok(data) if image_hash(&data) == hash => {
                total_video_bytes = total_video_bytes.saturating_add(data.len() as u64);
                if total_video_bytes <= MAX_VIDEO_ARCHIVE_BYTES {
                    videos.insert(hash, VideoData { bytes: Arc::new(data) });
                } else {
                    unread_videos.push((hash, FormatError::Corrupt("video archive exceeds the 256 MiB limit".into())));
                }
            }
            Ok(_) => unread_videos.push((hash, FormatError::Corrupt("video content hash does not match its bytes".into()))),
            Err(e) => unread_videos.push((hash, e)),
        }
    }
    let mut current_videos = own_videos.unwrap_or_default();
    current_videos.extend(used_videos(&doc));
    report_lost_videos(&mut issues, &unread_videos, Some(&current_videos), &videos);
    match library::read(&mut z, &images, &videos, u64::MAX, MAX_VIDEO_ARCHIVE_BYTES.saturating_sub(total_video_bytes)) {
        Ok(lib) => doc.published = lib.map(Arc::new),
        Err(e) => issues.push(format!("the published library was skipped: {e}")),
    }
    let mut budget = MAX_VERSION_BYTES;
    let versions = read_versions(&mut z, "versions", &images, &videos, &mut issues, &mut budget);
    let branches = read_branches(&mut z, &images, &videos, &mut issues);
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
    doc.videos = current_videos.into_iter().filter_map(|h| videos.get(&h).map(|v| (h, v.clone()))).collect();
    validate_videos(&doc.videos).map_err(FormatError::Corrupt)?;
    let degraded = drop_dangling_videos(&mut doc);
    if degraded > 0 {
        issues.push(format!("{degraded} video fill(s) lost their video and now show their poster image"));
    }
    Ok(Loaded { doc, issues, versions, branches })
}

/// The archive's `images/` members by hash, within `budget`, and the ones that couldn't be read
/// with why. The document's own images (`own`) are read first, so a file past the budget loses
/// the images only its history uses before its own.
fn read_images<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    own: Option<&BTreeSet<String>>,
    budget: ImageBudget,
) -> (BTreeMap<String, ImageData>, Vec<(String, FormatError)>) {
    let mut hashes: Vec<String> = z.file_names().filter_map(|n| n.strip_prefix("images/")).map(str::to_string).collect();
    hashes.sort_by_key(|h| !own.is_some_and(|o| o.contains(h)));
    let mut images = BTreeMap::new();
    let mut unread = Vec::new();
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

fn report_lost_videos(
    issues: &mut Vec<String>,
    unread: &[(String, FormatError)],
    listed: Option<&BTreeSet<String>>,
    videos: &BTreeMap<String, VideoData>,
) {
    let mut lost: Vec<String> =
        unread.iter().map(|(hash, e)| format!("video {hash} couldn't be read ({e}): it is left out, and saving this file again drops it")).collect();
    let unread_hashes: BTreeSet<&str> = unread.iter().map(|(h, _)| h.as_str()).collect();
    for hash in listed.into_iter().flatten() {
        if !videos.contains_key(hash) && !unread_hashes.contains(hash.as_str()) {
            lost.push(format!("video {hash} is used by the file but its data is missing: it is left out, and saving this file again drops it"));
        }
    }
    let more = lost.len().saturating_sub(MAX_IMAGE_ISSUES);
    lost.truncate(MAX_IMAGE_ISSUES);
    issues.extend(lost);
    if more > 0 {
        issues.push(format!("{more} more videos couldn't be loaded"));
    }
}

/// The version history kept under `dir` in an archive (none when it has no index), reading at
/// most `budget` bytes of snapshots. Damaged entries are skipped and reported in `issues`; nothing
/// here fails the load.
fn read_versions<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    dir: &str,
    images: &BTreeMap<String, ImageData>,
    videos: &BTreeMap<String, VideoData>,
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
        let video_hashes: Vec<&str> = e.get("videos").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let version_videos: BTreeMap<String, VideoData> =
            video_hashes.iter().filter_map(|h| videos.get(*h).map(|v| ((*h).to_string(), v.clone()))).collect();
        if version_videos.len() != video_hashes.len() {
            // Kept: the snapshot shows the poster where its video is gone (see `Version::document`).
            issues.push(format!("version `{id}` lost one or more video assets: its videos show their posters"));
        }
        if let Err(error) = validate_videos(&version_videos) {
            issues.push(format!("version `{id}` was skipped: {error}"));
            continue;
        }
        out.push(Version {
            id: id.to_string(),
            name: text("name", MAX_VERSION_NAME),
            description: text("description", MAX_VERSION_DESCRIPTION),
            time: e.get("time").and_then(Value::as_u64).unwrap_or(0),
            auto: e.get("auto").and_then(Value::as_bool).unwrap_or(false),
            snapshot: Snapshot::Stored { json: Arc::new(json), images, videos: version_videos },
        });
    }
    out
}

/// The branches of an archive (none when it has no index), parsed. Damaged entries are skipped and
/// reported in `issues`; nothing here fails the load.
fn read_branches<R: Read + Seek>(
    z: &mut zip::ZipArchive<R>,
    images: &BTreeMap<String, ImageData>,
    videos: &BTreeMap<String, VideoData>,
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
        let mut snapshot = |part: &str, list: &str, video_list: &str| -> Result<Arc<Document>, FormatError> {
            let json = read_member(z, &format!("branches/{id}/{part}.json"), budget.min(MAX_MEMBER))?;
            budget = budget.saturating_sub(json.len() as u64);
            let (mut doc, _) = from_json(std::str::from_utf8(&json).map_err(|e| FormatError::Corrupt(e.to_string()))?)?;
            let hashes = e.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
            doc.images = hashes.filter_map(|h| images.get(h).map(|i| (h.to_string(), i.clone()))).collect();
            let video_hashes: Vec<&str> = e.get(video_list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            doc.videos = video_hashes.iter().filter_map(|h| videos.get(*h).map(|v| ((*h).to_string(), v.clone()))).collect();
            if doc.videos.len() != video_hashes.len() {
                issues.push(format!("branch `{id}` lost one or more video assets: its videos show their posters"));
            }
            drop_dangling_videos(&mut doc);
            validate_video_store(&doc)?;
            Ok(Arc::new(doc))
        };
        let (base, head) = match (snapshot("base", "baseImages", "baseVideos"), snapshot("head", "headImages", "headVideos")) {
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
            versions: read_versions(z, &format!("branches/{id}/versions"), images, videos, issues, &mut budget),
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
mod video_save_tests;

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

    #[test]
    fn video_assets_roundtrip_through_versions_and_branches() {
        let mut doc = Document::new("Video file");
        let page = doc.first_page().unwrap();
        let node = doc.create(page, NodeKind::Rectangle, "Clip", 0.0, 0.0, 20.0, 20.0).unwrap();
        let (poster_hash, ..) = add_image(&mut doc, png()).unwrap();
        let video_hash = add_video(&mut doc, b"synthetic video bytes".to_vec()).unwrap();
        doc.node_mut(node).unwrap().fills =
            vec![figcraft_doc::color::Paint::video(&video_hash, &poster_hash, figcraft_doc::color::ImageScaleMode::Crop)];
        assert!(used_videos(&doc).contains(&video_hash));
        assert!(figcraft_doc::used_images(&doc).contains(&poster_hash));
        let version = Version::new("v1", "Video", "snapshot", 1, Arc::new(doc.clone()));
        let branch = Branch {
            id: "b1".into(),
            name: "Video branch".into(),
            time: 2,
            client: 0,
            state: BranchState::Active,
            base: Arc::new(doc.clone()),
            head: Arc::new(doc.clone()),
            versions: vec![version.clone()],
        };
        let json = to_json(&doc).unwrap();
        assert_eq!(json["version"], 2);
        let bytes = save_bytes_with_history(&doc, None, &[version], &[branch]).unwrap();
        let loaded = load_bytes_with_versions(&bytes).unwrap();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert_eq!(loaded.doc.videos[&video_hash].bytes.as_slice(), b"synthetic video bytes");
        assert_eq!(loaded.versions[0].document().unwrap().videos[&video_hash].bytes.as_slice(), b"synthetic video bytes");
        assert_eq!(loaded.branches[0].base.videos[&video_hash].bytes.as_slice(), b"synthetic video bytes");
        assert_eq!(loaded.branches[0].versions[0].videos()[&video_hash].bytes.as_slice(), b"synthetic video bytes");
    }

    #[test]
    fn video_free_document_keeps_format_version_one() {
        assert_eq!(to_json(&Document::new("Old format")).unwrap()["version"], 1);
    }

    #[test]
    fn history_only_video_sets_archive_version_and_survives_resave() {
        let mut past = Document::new("Past video");
        let hash = add_video(&mut past, b"history-only source".to_vec()).unwrap();
        let version = Version::new("past", "Past", "", 1, Arc::new(past));
        let current = Document::new("No current video");
        let bytes = save_bytes_with_versions(&current, None, &[version]).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let header: Value = serde_json::from_slice(&read_member(&mut archive, "document.json", MAX_MEMBER).unwrap()).unwrap();
        assert_eq!(header["version"], 2, "old readers must refuse video-bearing history");
        let loaded = load_bytes_with_versions(&bytes).unwrap();
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
        assert!(loaded.doc.videos.is_empty());
        assert_eq!(loaded.versions[0].document().unwrap().videos[&hash].bytes.as_slice(), b"history-only source");
        let again = save_bytes_with_versions(&loaded.doc, None, &loaded.versions).unwrap();
        let reopened = load_bytes_with_versions(&again).unwrap();
        assert_eq!(reopened.versions[0].videos()[&hash].bytes.as_slice(), b"history-only source");
    }

    #[test]
    fn a_video_reference_missing_from_the_index_is_reported() {
        let mut doc = Document::new("Broken reference");
        let page = doc.first_page().unwrap();
        let node = doc.create(page, NodeKind::Rectangle, "Clip", 0.0, 0.0, 10.0, 10.0).unwrap();
        let hash = "a".repeat(64);
        doc.node_mut(node).unwrap().fills = vec![figcraft_doc::color::Paint::video(&hash, "poster", figcraft_doc::color::ImageScaleMode::Fill)];
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        put_json(&mut archive, "document.json", &doc).unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let (_, issues) = load_bytes(&bytes).unwrap();
        assert!(issues.iter().any(|issue| issue.contains(&hash) && issue.contains("missing")), "{issues:?}");
    }

    #[test]
    fn missing_video_member_is_reported() {
        let mut doc = Document::new("Video file");
        let hash = add_video(&mut doc, b"missing later".to_vec()).unwrap();
        let page = doc.first_page().unwrap();
        let node = doc.create(page, NodeKind::Rectangle, "Clip", 0.0, 0.0, 20.0, 20.0).unwrap();
        let (poster, ..) = add_image(&mut doc, png()).unwrap();
        doc.node_mut(node).unwrap().fills = vec![figcraft_doc::color::Paint::video(&hash, &poster, figcraft_doc::color::ImageScaleMode::Fill)];
        let saved = save_bytes(&doc, None).unwrap();
        let mut source = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
        let names: Vec<String> = source.file_names().filter(|name| !name.starts_with("videos/")).map(str::to_string).collect();
        let mut output = std::io::Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut output);
            for name in names {
                let bytes = read_member(&mut source, &name, MAX_MEMBER).unwrap();
                put(&mut archive, &name, zip::write::SimpleFileOptions::default(), &bytes).unwrap();
            }
            archive.finish().unwrap();
        }
        let loaded = load_bytes_with_versions(&output.into_inner()).unwrap();
        assert!(loaded.doc.videos.is_empty());
        assert!(loaded.issues.iter().any(|issue| issue.contains(&hash) && issue.contains("missing") && issue.contains("drops it")));
    }

    #[test]
    fn adding_video_over_store_budget_is_atomic() {
        let mut doc = Document::new("Video file");
        assert!(add_video(&mut doc, vec![1; MAX_VIDEO_BYTES]).is_ok());
        assert!(add_video(&mut doc, vec![2; MAX_VIDEO_BYTES]).is_ok());
        assert!(add_video(&mut doc, vec![3]).is_err());
        assert_eq!(doc.videos.len(), 2);
    }
}
