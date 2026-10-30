//! The picture a saved file carries of itself: `thumbnail.png` inside a `.figcraft` or `.fig` zip,
//! `previews/preview.png` inside a `.sketch` zip (the home screen's recent files show it), and the colour of the page it sits on. Image files are
//! their own picture. Everything is read within caps: a missing, damaged or oversized picture gives
//! `None`, never an error or a panic.

use std::io::{Read, Seek};
use std::path::Path;

/// Largest `thumbnail.png` read from an archive.
pub const MAX_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;
/// Largest image file read as its own picture.
pub const MAX_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Widest or tallest picture decoded (pixels); a larger one gives none.
pub const MAX_THUMBNAIL_SIDE: u32 = 8192;
/// Largest `document.json` read for its page colour, and `.fig` `meta.json`.
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_META_BYTES: u64 = 1024 * 1024;
/// Most memory one decode may take.
const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;

/// A decoded picture: straight (unpremultiplied) RGBA, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The bytes of `thumbnail.png` in a zip archive (`.figcraft`, `.fig`); `None` when it isn't a zip,
/// has no thumbnail, or the thumbnail is larger than [`MAX_THUMBNAIL_BYTES`].
pub fn thumbnail_png<R: Read + Seek>(archive: R) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(archive).ok()?;
    crate::read_member(&mut z, "thumbnail.png", MAX_THUMBNAIL_BYTES).ok()
}

/// The bytes of `previews/preview.png` in a `.sketch` zip; `None` as [`thumbnail_png`] gives it.
pub fn sketch_preview_png<R: Read + Seek>(archive: R) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(archive).ok()?;
    crate::read_member(&mut z, "previews/preview.png", MAX_THUMBNAIL_BYTES).ok()
}

/// Decode a picture (PNG, JPEG, WebP, GIF, BMP or TIFF) no larger than [`MAX_THUMBNAIL_SIDE`] on
/// either side, scaled down to at most `fit` pixels on its longer side (never up).
pub fn decode_thumbnail(bytes: &[u8], fit: u32) -> Option<Thumbnail> {
    if crate::jpeg::finer_than_first(bytes) {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_THUMBNAIL_SIDE);
    limits.max_image_height = Some(MAX_THUMBNAIL_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let mut picture = reader.decode().ok()?;
    let fit = fit.clamp(1, MAX_THUMBNAIL_SIDE);
    if picture.width() > fit || picture.height() > fit {
        picture = picture.thumbnail(fit, fit);
    }
    let rgba = picture.to_rgba8();
    let (width, height) = rgba.dimensions();
    (width > 0 && height > 0).then(|| Thumbnail { width, height, rgba: rgba.into_raw() })
}

/// What kind of file `path` is, for its picture and its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// `.figcraft`
    FigCraft,
    /// `.fig`
    Fig,
    /// `.sketch`
    Sketch,
    /// `.svg`
    Svg,
    /// A raster image (`.png`, `.jpg`, `.webp`, …).
    Image,
    Other,
}

impl FileKind {
    pub fn of(path: &Path) -> FileKind {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "figcraft" => FileKind::FigCraft,
            // A FigJam `.jam` is the same archive.
            "fig" | "jam" => FileKind::Fig,
            "sketch" => FileKind::Sketch,
            "svg" => FileKind::Svg,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" => FileKind::Image,
            _ => FileKind::Other,
        }
    }
}

/// The picture of the file at `path`, at most `fit` pixels on its longer side: the thumbnail a
/// `.figcraft`, `.fig` or `.sketch` file carries, or an image file itself (up to [`MAX_IMAGE_FILE_BYTES`]).
/// `None` for anything else, and for a file that is missing, unreadable or damaged.
pub fn file_thumbnail(path: &Path, fit: u32) -> Option<Thumbnail> {
    match FileKind::of(path) {
        FileKind::FigCraft | FileKind::Fig => {
            let file = std::fs::File::open(path).ok()?;
            let png = thumbnail_png(std::io::BufReader::new(file))?;
            decode_thumbnail(&png, fit)
        }
        FileKind::Sketch => {
            let file = std::fs::File::open(path).ok()?;
            let png = sketch_preview_png(std::io::BufReader::new(file))?;
            decode_thumbnail(&png, fit)
        }
        FileKind::Image => {
            let file = std::fs::File::open(path).ok()?;
            let mut bytes = Vec::new();
            file.take(MAX_IMAGE_FILE_BYTES.saturating_add(1)).read_to_end(&mut bytes).ok()?;
            if bytes.len() as u64 > MAX_IMAGE_FILE_BYTES {
                return None;
            }
            decode_thumbnail(&bytes, fit)
        }
        FileKind::Svg | FileKind::Other => None,
    }
}

/// How a file looks on the home screen: the colour of its first page (straight RGBA, 0 to 1) and
/// the picture it carries. Either may be missing: a file with nothing on it has a page colour and no
/// picture; an image file has a picture and no page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub page: Option<[f32; 4]>,
    pub picture: Option<Thumbnail>,
    /// A design file or a board (`.fig` and `.sketch` files and images: design).
    pub kind: figcraft_doc::FileKind,
}

/// The [`Preview`] of the file at `path`, its picture at most `fit` pixels on its longer side.
/// Empty for a file that is missing, unreadable, damaged or of another kind.
pub fn file_preview(path: &Path, fit: u32) -> Preview {
    match FileKind::of(path) {
        FileKind::FigCraft | FileKind::Fig => {
            let Ok(file) = std::fs::File::open(path) else { return Preview::default() };
            archive_preview(std::io::BufReader::new(file), fit)
        }
        // A Sketch file's page colour isn't read: the preview is its picture alone, as for an image.
        FileKind::Sketch | FileKind::Image => Preview { picture: file_thumbnail(path, fit), ..Preview::default() },
        FileKind::Svg | FileKind::Other => Preview::default(),
    }
}

/// The [`Preview`] of a `.figcraft` or `.fig` archive: `thumbnail.png`, and the first page's colour
/// from `document.json` (`.figcraft`) or `meta.json` (`.fig`).
pub fn archive_preview<R: Read + Seek>(archive: R, fit: u32) -> Preview {
    let Ok(mut z) = zip::ZipArchive::new(archive) else { return Preview::default() };
    let picture = crate::read_member(&mut z, "thumbnail.png", MAX_THUMBNAIL_BYTES).ok().and_then(|png| decode_thumbnail(&png, fit));
    let (page, kind) = if z.file_names().any(|n| n == "document.json") {
        crate::read_member(&mut z, "document.json", MAX_DOCUMENT_BYTES).ok().map(|json| figcraft_page(&json)).unwrap_or_default()
    } else {
        (crate::read_member(&mut z, "meta.json", MAX_META_BYTES).ok().and_then(|json| fig_page(&json)), figcraft_doc::FileKind::Design)
    };
    Preview { page, picture, kind }
}

/// The first page's background in a `.figcraft` `document.json`, and the file's kind.
fn figcraft_page(json: &[u8]) -> (Option<[f32; 4]>, figcraft_doc::FileKind) {
    #[derive(serde::Deserialize)]
    struct Head {
        #[serde(default)]
        pages: Vec<u64>,
        #[serde(default)]
        nodes: std::collections::BTreeMap<String, Node>,
        #[serde(default)]
        kind: figcraft_doc::FileKind,
    }
    #[derive(serde::Deserialize)]
    struct Node {
        #[serde(default)]
        background: Option<serde_json::Value>,
    }
    let Ok(head) = serde_json::from_slice::<Head>(json) else { return (None, figcraft_doc::FileKind::Design) };
    let page = head.pages.first().and_then(|first| head.nodes.get(&first.to_string())).and_then(|n| n.background.as_ref()).and_then(rgba);
    (page, head.kind)
}

/// The canvas colour a `.fig` file's `meta.json` gives (`client_meta.background_color`).
fn fig_page(json: &[u8]) -> Option<[f32; 4]> {
    let meta: serde_json::Value = serde_json::from_slice(json).ok()?;
    rgba(meta.get("client_meta")?.get("background_color")?)
}

/// `{r, g, b, a?}` (0 to 1) as a colour; `None` unless every channel is a finite number.
fn rgba(v: &serde_json::Value) -> Option<[f32; 4]> {
    let channel = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32);
    let a = if v.get("a").is_some() { channel("a")? } else { 1.0 };
    Some([channel("r")?, channel("g")?, channel("b")?, a])
}
