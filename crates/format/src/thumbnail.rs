//! The picture a saved file carries of itself: `thumbnail.png` inside a `.figcraft` or `.fig` zip,
//! `previews/preview.png` inside a `.sketch` zip (the home screen's recent files show it), and the colour of the page it sits on. Image files are
//! their own picture. Everything is read within caps: a missing, damaged or oversized picture gives
//! `None`, never an error or a panic.

use std::io::{Read, Seek};
use std::path::Path;

/// Largest `thumbnail.png` read from an archive.
pub const MAX_THUMBNAIL_BYTES: u64 = craft_figcraft_format::MAX_PREVIEW_THUMBNAIL_BYTES;
/// Maximum ZIP index bytes read while locating a Sketch preview.
const MAX_SKETCH_PREVIEW_METADATA_BYTES: u64 = 64 * 1024 * 1024;
/// Largest image file read as its own picture.
pub const MAX_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Widest or tallest picture decoded (pixels); a larger one gives none.
pub const MAX_THUMBNAIL_SIDE: u32 = 8192;
/// Largest `.fig` `meta.json` read for its page colour.
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
    craft_figcraft_format::read_preview_thumbnail(archive).ok().flatten()
}

/// The bytes of `previews/preview.png` in a `.sketch` zip; `None` as [`thumbnail_png`] gives it.
pub fn sketch_preview_png<R: Read + Seek>(archive: R) -> Option<Vec<u8>> {
    let limits = craft_zip::SeekableLimits {
        max_entries: craft_sketch::MAX_ENTRIES,
        max_metadata_bytes: MAX_SKETCH_PREVIEW_METADATA_BYTES,
        max_total_bytes: MAX_THUMBNAIL_BYTES,
    };
    let mut archive = craft_zip::SeekableArchive::open(archive, limits).ok()?;
    archive.read("previews/preview.png", MAX_THUMBNAIL_BYTES).ok().flatten()
}

/// Decode a picture (PNG, JPEG, WebP, GIF, BMP or TIFF) no larger than [`MAX_THUMBNAIL_SIDE`] on
/// either side, scaled down to at most `fit` pixels on its longer side (never up).
pub fn decode_thumbnail(bytes: &[u8], fit: u32) -> Option<Thumbnail> {
    if crate::jpeg::finer_than_first(bytes) {
        return None;
    }
    let opts = craft_image_codecs::DecodeOptions {
        limits: craft_image_codecs::Limits::new(MAX_THUMBNAIL_SIDE, MAX_THUMBNAIL_SIDE, u64::MAX, MAX_DECODE_ALLOC),
        keep_orientation: true,
    };
    let img = craft_image_codecs::decode_with(bytes, &opts).ok()?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let fit = fit.clamp(1, MAX_THUMBNAIL_SIDE);
    let shrink = w > fit || h > fit;
    let layout = img.layout();
    if shrink && img.sample_type() == craft_image_codecs::SampleType::U16 && (layout.is_gray() || layout.is_rgb()) {
        use craft_image_codecs::ChannelLayout;
        use image::{DynamicImage, ImageBuffer};

        // Keep the original channel width until the thumbnail has been averaged.
        let samples = img.to_u16_samples()?;
        drop(img);
        let scale = f64::from(fit) / f64::from(w.max(h));
        let width = (f64::from(w) * scale).round().max(1.0) as u32;
        let height = (f64::from(h) * scale).round().max(1.0) as u32;

        // Allow a pixel for floating-point bin boundaries. A u32 sum of u16 samples,
        // including the rounding term, is safe for at most 65,536 samples.
        let footprint = (w.div_ceil(width) + 1) * (h.div_ceil(height) + 1);
        let picture = if footprint > 65_536 {
            // Widen on read, so image's thumbnail sampler uses u64 sums without
            // allocating a widened full-size source.
            let source = WideThumbnailSamples { width: w, height: h, channels: layout.channels(), samples: &samples };
            let thumb = image::imageops::thumbnail(&source, width, height);
            let native = thumb.into_raw().into_iter().map(|sample| sample as u16).collect::<Vec<_>>();
            DynamicImage::ImageRgba16(ImageBuffer::from_raw(width, height, native)?)
        } else {
            let source = match layout {
                ChannelLayout::Gray => DynamicImage::ImageLuma16(ImageBuffer::from_raw(w, h, samples)?),
                ChannelLayout::GrayA => DynamicImage::ImageLumaA16(ImageBuffer::from_raw(w, h, samples)?),
                ChannelLayout::Rgb => DynamicImage::ImageRgb16(ImageBuffer::from_raw(w, h, samples)?),
                ChannelLayout::Rgba => DynamicImage::ImageRgba16(ImageBuffer::from_raw(w, h, samples)?),
                _ => return None,
            };
            source.thumbnail_exact(width, height)
        };
        let thumbnail = picture.into_rgba8();
        let (width, height) = thumbnail.dimensions();
        return Some(Thumbnail { width, height, rgba: thumbnail.into_raw() });
    }
    let rgba_img = img.converted(craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8);
    let (width, height, rgba) = if shrink {
        let buf = image::RgbaImage::from_raw(w, h, rgba_img.into_owned().into_data())?;
        let thumb = image::DynamicImage::ImageRgba8(buf).thumbnail(fit, fit).into_rgba8();
        let (nw, nh) = thumb.dimensions();
        (nw, nh, thumb.into_raw())
    } else {
        (w, h, rgba_img.into_owned().into_data())
    };
    (width > 0 && height > 0).then_some(Thumbnail { width, height, rgba })
}

struct WideThumbnailSamples<'a> {
    width: u32,
    height: u32,
    channels: usize,
    samples: &'a [u16],
}

impl image::GenericImageView for WideThumbnailSamples<'_> {
    type Pixel = image::Rgba<u32>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let start = (y as usize * self.width as usize + x as usize) * self.channels;
        let rgba = match self.samples.get(start..start + self.channels) {
            Some(&[gray]) => [gray, gray, gray, u16::MAX],
            Some(&[gray, alpha]) => [gray, gray, gray, alpha],
            Some(&[red, green, blue]) => [red, green, blue, u16::MAX],
            Some(&[red, green, blue, alpha]) => [red, green, blue, alpha],
            _ => [0; 4],
        };
        image::Rgba(rgba.map(u32::from))
    }
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
    let Ok(mut z) = craft_figcraft_format::SeekArchive::open(archive, u64::MAX) else { return Preview::default() };
    let native = craft_figcraft_format::read_preview_archive_from(&mut z);
    let picture = native.thumbnail_png.and_then(|png| decode_thumbnail(&png, fit));
    let (page, kind) = if native.has_document {
        native.document.as_ref().map(figcraft_page).unwrap_or_default()
    } else {
        let meta = z.read("meta.json", MAX_META_BYTES).ok().flatten();
        (meta.as_deref().and_then(fig_page), figcraft_doc::FileKind::Design)
    };
    Preview { page, picture, kind }
}

/// The first page's background in a `.figcraft` `document.json`, and the file's kind.
fn figcraft_page(head: &craft_figcraft_format::PreviewDocument) -> (Option<[f32; 4]>, figcraft_doc::FileKind) {
    let page = head.pages.first().and_then(|first| head.nodes.get(&first.to_string())).and_then(|n| n.background.as_ref()).and_then(rgba);
    let kind = match head.kind {
        craft_figcraft_format::PreviewKind::Design => figcraft_doc::FileKind::Design,
        craft_figcraft_format::PreviewKind::Board => figcraft_doc::FileKind::Board,
    };
    (page, kind)
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
