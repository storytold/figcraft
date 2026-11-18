//! Resource budgets for what an SVG expands into while it is parsed: compressed (SVGZ) input,
//! and SVG documents embedded in `<image>` data URIs (themselves possibly compressed or nested).
//! usvg decompresses with no cap and parses embedded SVGs with no shared depth or byte limit, so
//! both are done here: a bounded gunzip before usvg sees the bytes, and our own data resolver.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use usvg::{ImageHrefResolver, ImageKind, Options};

/// Largest document, once decompressed.
const MAX_DOCUMENT_BYTES: usize = 64 << 20;
/// Largest total of all embedded SVG documents, once decompressed.
const MAX_EMBEDDED_BYTES: usize = 32 << 20;
/// Most SVG documents embedded inside one another.
const MAX_EMBEDDED_DEPTH: usize = 4;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub document: usize,
    pub embedded: usize,
    pub depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { document: MAX_DOCUMENT_BYTES, embedded: MAX_EMBEDDED_BYTES, depth: MAX_EMBEDDED_DEPTH }
    }
}

fn gzipped(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// `data` as plain text bytes: gunzipped when compressed, never past `limit` bytes.
fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if !gzipped(data) {
        return if data.len() > limit { Err(format!("SVG larger than {} MiB", limit >> 20)) } else { Ok(data.to_vec()) };
    }
    let mut out = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    flate2::read::GzDecoder::new(data).take(cap).read_to_end(&mut out).map_err(|_| "malformed compressed SVG".to_string())?;
    if out.len() > limit {
        return Err(format!("compressed SVG expands past {} MiB", limit >> 20));
    }
    // usvg gunzips what it is handed, with no limit: one layer is unpacked here, within the
    // budget, and what comes out must be the document itself.
    if gzipped(&out) {
        return Err("compressed SVG holds another compressed file, not a document".into());
    }
    Ok(out)
}

/// The document `data` (SVG or SVGZ) as plain bytes, within the document budget.
pub(super) fn document(data: &[u8], limits: &Limits) -> Result<Vec<u8>, String> {
    // The caller already holds a plain document; only what compression expands to is bounded.
    if gzipped(data) { inflate(data, limits.document) } else { Ok(data.to_vec()) }
}

/// What the embedded SVGs of one import have used so far.
struct Used {
    bytes: AtomicUsize,
    depth: AtomicUsize,
}

fn raster(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG")
        || data.starts_with(&[0xff, 0xd8, 0xff])
        || data.starts_with(b"GIF8")
        || (data.len() > 12 && data.get(8..12) == Some(b"WEBP"))
}

/// Whether `svg` has an `<image>` that usvg will drop before the resolver can say so: one linked
/// by a path or URL (nested SVGs never ask the resolver for those), or one whose data URI doesn't
/// decode (usvg decodes before it asks). Found by parsing the document, so a comment holding
/// `<image>` or spaces around `=` don't count. A document that doesn't parse reports nothing,
/// since usvg can't read it either.
///
/// It follows usvg where that is cheap: an element in the SVG namespace or none, the unprefixed
/// `href` over `xlink:href`, and no image under `display="none"` (attribute or `style`). It goes no
/// further: an image in `<defs>`, a `<symbol>` nothing uses, a `<use>` target, a mask or a pattern
/// is still counted. Such a false omission errs the safe way, since it only makes a paste prefer
/// the complete clipboard bitmap.
pub(super) fn drops_images(svg: &[u8]) -> bool {
    use usvg::roxmltree::{Document, Node, ParsingOptions};
    const SVG_NS: &str = "http://www.w3.org/2000/svg";
    const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
    // The display that applies to an element: a `display` in `style` wins over the attribute, as
    // usvg applies the inline style after the presentation attribute.
    fn display<'a>(n: Node<'a, '_>) -> Option<&'a str> {
        let from_style = n.attribute("style").and_then(|style| {
            style.split(';').filter_map(|d| d.split_once(':')).filter(|(k, _)| k.trim() == "display").map(|(_, v)| v.trim()).next_back()
        });
        from_style.or_else(|| n.attribute("display").map(str::trim))
    }
    fn hidden(n: Node) -> bool {
        n.ancestors().any(|a| display(a) == Some("none"))
    }
    let Ok(text) = std::str::from_utf8(svg) else { return false };
    let Ok(doc) = Document::parse_with_options(text, ParsingOptions { allow_dtd: true, ..ParsingOptions::default() }) else { return false };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "image" && matches!(n.tag_name().namespace(), None | Some(SVG_NS)))
        .filter(|n| !hidden(*n))
        .any(|n| {
            let find = |ns: Option<&str>| n.attributes().find(|a| a.name() == "href" && a.namespace() == ns).map(|a| a.value());
            let Some(href) = find(None).or_else(|| find(Some(XLINK_NS))) else { return false };
            match data_url::DataUrl::process(href.trim()) {
                Ok(url) => url.decode_to_vec().is_err(),
                Err(_) => true,
            }
        })
}

/// Whether usvg can read a raster's size, which it needs after the resolver says yes.
fn sized(data: &[u8]) -> bool {
    matches!(imagesize::blob_size(data), Ok(size) if size.width > 0 && size.height > 0)
}

/// An embedded SVG document, if the budgets allow it.
fn embedded(used: &Used, limits: &Limits, data: &[u8], opts: &Options, omitted: &AtomicBool) -> Option<ImageKind> {
    if used.depth.load(Ordering::Relaxed) >= limits.depth {
        return None;
    }
    let left = limits.embedded.saturating_sub(used.bytes.load(Ordering::Relaxed));
    let plain = match inflate(data, left) {
        Ok(plain) => plain,
        // What a refused document unpacked before it was refused was work too: it uses up what is
        // left, so the next image can't repeat it.
        Err(_) => {
            used.bytes.fetch_add(left, Ordering::Relaxed);
            return None;
        }
    };
    used.bytes.fetch_add(plain.len(), Ordering::Relaxed);
    if super::too_wide(&plain) || super::too_deep(&plain) {
        return None;
    }
    // usvg ignores a nested SVG's linked images without asking our resolver, and drops a data URI
    // that doesn't decode before asking, so the document is searched for them.
    if drops_images(&plain) {
        omitted.store(true, Ordering::Relaxed);
    }
    used.depth.fetch_add(1, Ordering::Relaxed);
    let tree = usvg::Tree::from_data_nested(&plain, opts);
    used.depth.fetch_sub(1, Ordering::Relaxed);
    tree.ok().map(ImageKind::SVG)
}

/// Resolve `<image>` data URIs: rasters as they are, SVGs within `limits`; nothing else. Every
/// image it declines (a linked file, an unreadable or over-budget data URI, nested SVGs included)
/// sets `omitted`, so the importer can say the result lacks graphical content.
pub(super) fn resolver(limits: Limits, omitted: Arc<AtomicBool>) -> ImageHrefResolver<'static> {
    let used = Arc::new(Used { bytes: AtomicUsize::new(0), depth: AtomicUsize::new(0) });
    let data_omitted = Arc::clone(&omitted);
    let nested_omitted = Arc::clone(&omitted);
    ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, opts| {
            let kind = resolve_data(&used, &limits, mime, data, opts, &nested_omitted);
            if kind.is_none() {
                data_omitted.store(true, Ordering::Relaxed);
            }
            kind
        }),
        // Linked files are never read.
        resolve_string: Box::new(move |_, _| {
            omitted.store(true, Ordering::Relaxed);
            None
        }),
    }
}

fn resolve_data(used: &Used, limits: &Limits, mime: &str, data: Arc<Vec<u8>>, opts: &Options, omitted: &AtomicBool) -> Option<ImageKind> {
    match mime {
        "image/jpg" | "image/jpeg" if sized(&data) => Some(ImageKind::JPEG(data)),
        "image/png" if sized(&data) => Some(ImageKind::PNG(data)),
        "image/gif" if sized(&data) => Some(ImageKind::GIF(data)),
        "image/webp" if sized(&data) => Some(ImageKind::WEBP(data)),
        "image/svg+xml" => embedded(used, limits, &data, opts, omitted),
        "text/plain" if raster(&data) && sized(&data) => match data.first() {
            Some(0x89) => Some(ImageKind::PNG(data)),
            Some(0xff) => Some(ImageKind::JPEG(data)),
            Some(b'G') => Some(ImageKind::GIF(data)),
            _ => Some(ImageKind::WEBP(data)),
        },
        "text/plain" => embedded(used, limits, &data, opts, omitted),
        _ => None,
    }
}
