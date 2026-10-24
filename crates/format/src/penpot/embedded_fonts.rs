//! Font binaries bundled with a Penpot export. No external fetches or OS installation.
//!
//! Objects can be TrueType/OpenType or the WOFF1/WOFF2 containers. We convert WOFF1 and
//! WOFF2 to SFNT because the FigCraft font shaper accepts SFNT, not web-font wrappers.
//! Font objects without suffixes are recognised by their four-byte signatures.

use std::io::{Cursor, Read};

use figcraft_doc::Document;
use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::{MAX_ENTRIES, warn};

const MAX_FONTS: usize = 128;
const MAX_FONT: usize = 32 * 1024 * 1024;
const MAX_ALL_FONTS: usize = 128 * 1024 * 1024;

/// Read a big-endian integer only when both bytes are present.
fn be_u16(b: &[u8], start: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(start..start.checked_add(2)?)?.try_into().ok()?))
}
/// Read a big-endian integer using a checked byte range.
fn be_u32(b: &[u8], start: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(start..start.checked_add(4)?)?.try_into().ok()?))
}

/// Recognize supported SFNT and collection signatures before font parsing.
fn valid_sfnt(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && (bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"OTTO") || bytes.starts_with(b"true") || bytes.starts_with(b"ttcf"))
}

/// Convert an OpenType WOFF v1 wrapper to a regular sfnt. Strict offsets, table sizes
/// and decompression budgets prevent an untrusted file from expanding without limit.
fn decode_woff1(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 44 || !bytes.starts_with(b"wOFF") || be_u32(bytes, 8)? as usize != bytes.len() {
        return None;
    }
    let flavor = bytes.get(4..8)?;
    if flavor != [0, 1, 0, 0] && flavor != b"OTTO" && flavor != b"true" {
        return None;
    }
    let count = usize::from(be_u16(bytes, 12)?);
    let expected_size = usize::try_from(be_u32(bytes, 16)?).ok()?;
    if count == 0 || count > 128 || expected_size < 12 + count * 16 || expected_size > MAX_FONT || 44 + count * 20 > bytes.len() {
        return None;
    }
    let mut sfnt = vec![0; 12 + count * 16];
    sfnt[..4].copy_from_slice(flavor);
    sfnt[4..6].copy_from_slice(&(count as u16).to_be_bytes());
    let power = 1usize << (usize::BITS - 1 - count.leading_zeros());
    sfnt[6..8].copy_from_slice(&((power * 16) as u16).to_be_bytes());
    sfnt[8..10].copy_from_slice(&(power.trailing_zeros() as u16).to_be_bytes());
    sfnt[10..12].copy_from_slice(&(((count - power) * 16) as u16).to_be_bytes());
    for i in 0..count {
        let at = 44 + i * 20;
        let offset = usize::try_from(be_u32(bytes, at + 4)?).ok()?;
        let compressed = usize::try_from(be_u32(bytes, at + 8)?).ok()?;
        let original = usize::try_from(be_u32(bytes, at + 12)?).ok()?;
        if compressed == 0 || original == 0 || compressed > original || original > MAX_FONT {
            return None;
        }
        let slice = bytes.get(offset..offset.checked_add(compressed)?)?;
        let data = if compressed == original {
            slice.to_vec()
        } else {
            let mut data = Vec::new();
            ZlibDecoder::new(slice).take((original + 1) as u64).read_to_end(&mut data).ok()?;
            data
        };
        if data.len() != original {
            return None;
        }
        let aligned = (sfnt.len() + 3) & !3;
        if aligned.checked_add(original)? > MAX_FONT {
            return None;
        }
        sfnt.resize(aligned, 0);
        let out = 12 + i * 16;
        sfnt[out..out + 4].copy_from_slice(bytes.get(at..at + 4)?);
        sfnt[out + 4..out + 8].copy_from_slice(bytes.get(at + 16..at + 20)?);
        sfnt[out + 8..out + 12].copy_from_slice(&(aligned as u32).to_be_bytes());
        sfnt[out + 12..out + 16].copy_from_slice(&(original as u32).to_be_bytes());
        sfnt.extend_from_slice(&data);
    }
    if sfnt.len() > expected_size || (sfnt.len() + 3) & !3 != expected_size {
        return None;
    }
    sfnt.resize(expected_size, 0);
    Some(sfnt)
}

/// Read a WOFF2 UIntBase128 value, rejecting overflow and noncanonical leading zeroes.
fn base128(bytes: &[u8], at: &mut usize) -> Option<usize> {
    let mut value = 0u32;
    for i in 0..5 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        if (i == 0 && byte == 0x80) || value > u32::MAX >> 7 {
            return None;
        }
        value = (value << 7) | u32::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return usize::try_from(value).ok();
        }
    }
    None
}

/// Check declared table and output sizes before invoking the WOFF2 decoder.
fn woff2_sizes_fit(bytes: &[u8], cap: usize) -> Option<bool> {
    let count = usize::from(be_u16(bytes, 12)?);
    if count == 0 || count > 128 || usize::try_from(be_u32(bytes, 16)?).ok()? > cap {
        return Some(false);
    }
    let mut at = 48;
    let mut stored = 0usize;
    let mut sfnt = 12 + count * 16;
    for _ in 0..count {
        let flags = *bytes.get(at)?;
        at += 1;
        let tag: [u8; 4] = match flags & 0x3f {
            10 => *b"glyf",
            11 => *b"loca",
            63 => {
                at += 4;
                bytes.get(at - 4..at)?.try_into().ok()?
            }
            _ => [0; 4],
        };
        let original = base128(bytes, &mut at)?;
        let version = flags >> 6;
        let transformed = if &tag == b"glyf" || &tag == b"loca" { version != 3 } else { version != 0 };
        let length = if transformed { base128(bytes, &mut at)? } else { original };
        stored = stored.checked_add(length)?;
        sfnt = sfnt.checked_add(original.checked_add(3)? & !3)?;
    }
    Some(stored <= cap && sfnt <= cap)
}

/// Decode a preflight-checked WOFF2 and reject oversized or non-SFNT output.
fn decode_woff2(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 48 || !bytes.starts_with(b"wOF2") || be_u32(bytes, 8)? as usize != bytes.len() || !woff2_sizes_fit(bytes, MAX_FONT)? {
        return None;
    }
    let sfnt = wuff::decompress_woff2(bytes).ok()?;
    (sfnt.len() <= MAX_FONT && valid_sfnt(&sfnt)).then_some(sfnt)
}

/// A Penpot object may be stored under a UUID (with no type suffix), or as a
/// `.bin` asset. Recognise its actual file signature rather than trusting the
/// suffix.
fn font_signature(header: &[u8; 4]) -> bool {
    header == b"OTTO" || header == b"true" || header == b"ttcf" || header == b"wOFF" || header == b"wOF2" || header == &[0, 1, 0, 0]
}

/// Select candidate font members while excluding known non-font assets.
fn asset_path(path: &str) -> bool {
    (path.starts_with("objects/") || path.starts_with("fonts/") || path.starts_with("assets/fonts/"))
        && !path.ends_with('/')
        && !matches!(
            path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref(),
            Some("json" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "xml" | "txt" | "css" | "avif" | "pdf")
        )
}

/// Recognize font suffixes for diagnostics when signature checks fail.
fn advertised_font(path: &str) -> bool {
    matches!(path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref(), Some("ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2"))
}

/// Bring font binaries into Document so they can be registered by figcraft-engine
/// and round-tripped by figcraft-format. Never perform network requests or extract
/// paths. Read only the first four bytes of unknown assets; this prevents images
/// hidden behind UUID names from being decompressed just to look for fonts.
pub(super) fn import(z: &mut ZipArchive<Cursor<&[u8]>>, doc: &mut Document, warnings: &mut Vec<String>) {
    let mut names: Vec<String> = z.file_names().filter(|p| asset_path(p)).take(MAX_ENTRIES).map(str::to_owned).collect();
    names.sort();
    let mut total = 0usize;
    let mut decoded_total = 0usize;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for path in names {
        let Ok(mut member) = z.by_name(&path) else { continue };
        if member.size() < 4 {
            continue;
        }
        let mut header = [0_u8; 4];
        if member.read_exact(&mut header).is_err() {
            if advertised_font(&path) {
                warn(warnings, format!("Penpot font '{path}' could not be read"));
            }
            continue;
        }
        if !font_signature(&header) {
            if advertised_font(&path) {
                warn(warnings, format!("Penpot font '{path}' has an unrecognised file signature"));
            }
            continue;
        }
        if imported >= MAX_FONTS {
            skipped += 1;
            continue;
        }
        let size = usize::try_from(member.size()).unwrap_or(usize::MAX);
        if size > MAX_FONT || size > MAX_ALL_FONTS.saturating_sub(total) {
            warn(warnings, format!("Penpot font '{}' exceeds the embedded-font size limit", path.chars().take(120).collect::<String>()));
            continue;
        }
        let mut raw = Vec::with_capacity(size.min(MAX_FONT));
        raw.extend_from_slice(&header);
        if member.take((MAX_FONT + 1 - header.len()) as u64).read_to_end(&mut raw).is_err() || raw.len() != size {
            warn(warnings, format!("Penpot font '{}' could not be fully read", path.chars().take(120).collect::<String>()));
            continue;
        }
        total += raw.len();
        let file = if raw.starts_with(b"wOFF") {
            match decode_woff1(&raw) {
                Some(v) => v,
                None => {
                    warn(warnings, format!("Penpot WOFF font '{path}' could not be decoded"));
                    continue;
                }
            }
        } else if raw.starts_with(b"wOF2") {
            match decode_woff2(&raw) {
                Some(v) => v,
                None => {
                    warn(warnings, format!("Penpot WOFF2 font '{path}' could not be decoded"));
                    continue;
                }
            }
        } else {
            raw
        };
        if !valid_sfnt(&file) {
            warn(warnings, format!("Penpot font '{path}' is not a supported OpenType/TrueType file"));
            continue;
        }
        if file.len() > MAX_ALL_FONTS.saturating_sub(decoded_total) {
            warn(warnings, "Penpot decoded-font budget exceeded; remaining fonts were not loaded".into());
            continue;
        }
        decoded_total += file.len();
        let hash = format!("{:x}", Sha256::digest(&file));
        doc.embedded_fonts.entry(hash).or_insert_with(|| std::sync::Arc::new(file));
        imported += 1;
    }
    if skipped > 0 {
        warn(warnings, format!("Penpot contains more than {MAX_FONTS} embedded fonts; {skipped} were not imported"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn woff_roundtrip_preserves_table_and_rejects_bad_sizes() {
        // One SFNT table, deliberately minimal. Tests bounds and the WOFF decoder,
        // not whether the SFNT describes a usable font.
        let mut w = vec![0u8; 68];
        w[..4].copy_from_slice(b"wOFF");
        w[4..8].copy_from_slice(&[0, 1, 0, 0]);
        w[8..12].copy_from_slice(&72u32.to_be_bytes());
        w[12..14].copy_from_slice(&1u16.to_be_bytes());
        w[16..20].copy_from_slice(&32u32.to_be_bytes());
        w[44..48].copy_from_slice(b"name");
        w[48..52].copy_from_slice(&68u32.to_be_bytes());
        w[52..56].copy_from_slice(&4u32.to_be_bytes());
        w[56..60].copy_from_slice(&4u32.to_be_bytes());
        w.extend_from_slice(b"TEST");
        let out = decode_woff1(&w).unwrap();
        assert_eq!(&out[28..32], b"TEST");
        w[56..60].copy_from_slice(&10u32.to_be_bytes());
        assert!(decode_woff1(&w).is_none());
    }

    #[test]
    fn penpot_import_collects_bundled_sfnt_without_extracting() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("objects/font-id.ttf", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&[0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut zip = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut doc = Document::new("Penpot fonts");
        let mut warnings = Vec::new();
        import(&mut zip, &mut doc, &mut warnings);
        assert_eq!(doc.embedded_fonts.len(), 1);
        assert!(warnings.is_empty());
    }
    #[test]
    fn detects_font_objects_without_an_extension_and_ignores_images() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in [
            ("objects/uuid-no-extension", &[0_u8, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0][..]),
            ("objects/image-like-uuid.bin", &[137_u8, 80, 78, 71, 1, 2, 3, 4][..]),
            ("assets/fonts/untyped.data", &b"OTTO\0\0\0\0\0\0\0\0"[..]),
        ] {
            writer.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut doc = Document::new("Embedded UUID fonts");
        let mut warnings = Vec::new();
        import(&mut archive, &mut doc, &mut warnings);
        assert_eq!(doc.embedded_fonts.len(), 2);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn undecodable_woff2_is_reported_instead_of_going_missing_silently() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("objects/font-uuid", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"wOF2fakefont").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut doc = Document::new("WOFF2");
        let mut warnings = Vec::new();
        import(&mut archive, &mut doc, &mut warnings);
        assert!(doc.embedded_fonts.is_empty());
        assert!(warnings.iter().any(|m| m.contains("WOFF2") && m.contains("font-uuid") && m.contains("could not be decoded")));
    }

    fn brotli_stored(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut used = 1u32;
        for chunk in data.chunks(1 << 16) {
            let mut header = ((((chunk.len() as u64) - 1) << 3) | (1 << 19)) << used;
            used += 20;
            while used > 0 {
                out.push(header as u8);
                header >>= 8;
                used = used.saturating_sub(8);
            }
            out.extend_from_slice(chunk);
        }
        out.push(0b11 << used);
        out
    }

    fn base128_bytes(value: u32) -> Vec<u8> {
        let mut out = vec![(value & 0x7f) as u8];
        let mut rest = value >> 7;
        while rest > 0 {
            out.insert(0, 0x80 | (rest & 0x7f) as u8);
            rest >>= 7;
        }
        out
    }

    fn tiny_woff2(collection: bool) -> Vec<u8> {
        let mut glyf = Vec::new();
        for v in [0u16, 0, 2, 0] {
            glyf.extend_from_slice(&v.to_be_bytes());
        }
        for len in [4u32, 1, 4, 17, 0, 4, 0] {
            glyf.extend_from_slice(&len.to_be_bytes());
        }
        glyf.extend_from_slice(&[0, 0, 0, 1]);
        glyf.push(4);
        glyf.extend_from_slice(&[127, 127, 0xff, 126]);
        glyf.extend_from_slice(&[0, 100, 0, 0, 1, 144, 0, 0, 0, 0, 2, 188, 1, 144, 0, 0, 0]);
        glyf.extend_from_slice(&[0; 4]);
        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        for (i, v) in [100i16, 0, 500, 700].into_iter().enumerate() {
            head[36 + i * 2..38 + i * 2].copy_from_slice(&v.to_be_bytes());
        }
        let mut hhea = vec![0u8; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[10..12].copy_from_slice(&600u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let hmtx = [0x01, 0x01, 0xF4, 0x02, 0x58];
        let maxp = [0, 0, 0x50, 0, 0, 2];
        let tables: [(u8, u32, Option<u32>, &[u8]); 6] = [
            (10, 32, Some(glyf.len() as u32), &glyf),
            (11, 6, Some(0), &[]),
            (1, 54, None, &head),
            (2, 36, None, &hhea),
            (0x43, 8, Some(hmtx.len() as u32), &hmtx),
            (4, 6, None, &maxp),
        ];
        let mut directory = Vec::new();
        let mut data = Vec::new();
        let mut sfnt_size = 12 + 16 * tables.len() as u32;
        for (flags, original, transformed, bytes) in tables {
            directory.push(flags);
            directory.extend(base128_bytes(original));
            if let Some(len) = transformed {
                directory.extend(base128_bytes(len));
            }
            data.extend_from_slice(bytes);
            sfnt_size += (original + 3) & !3;
        }
        if collection {
            directory.extend_from_slice(&[0, 1, 0, 0, 1, tables.len() as u8, 0, 1, 0, 0]);
            directory.extend(0..tables.len() as u8);
        }
        let compressed = brotli_stored(&data);
        let mut woff = if collection { b"wOF2ttcf".to_vec() } else { b"wOF2\0\x01\0\0".to_vec() };
        woff.extend_from_slice(&0u32.to_be_bytes());
        woff.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        woff.extend_from_slice(&0u16.to_be_bytes());
        woff.extend_from_slice(&sfnt_size.to_be_bytes());
        woff.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        woff.extend_from_slice(&[0, 1, 0, 0]);
        woff.extend_from_slice(&[0; 20]);
        woff.extend(directory);
        woff.extend(compressed);
        woff.resize((woff.len() + 3) & !3, 0);
        let len = woff.len() as u32;
        woff[8..12].copy_from_slice(&len.to_be_bytes());
        woff
    }

    #[derive(Default)]
    struct Path(Vec<String>);

    impl skrifa::outline::OutlinePen for Path {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(format!("M{x},{y}"));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(format!("L{x},{y}"));
        }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
            self.0.push(format!("Q{cx},{cy},{x},{y}"));
        }
        fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
            self.0.push(format!("C{cx0},{cy0},{cx1},{cy1},{x},{y}"));
        }
        fn close(&mut self) {
            self.0.push("Z".into());
        }
    }

    #[test]
    fn woff2_with_transformed_glyf_loca_and_hmtx_decodes_to_a_usable_font() {
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::DrawSettings;
        use skrifa::raw::TableProvider;
        use skrifa::{FontRef, GlyphId, MetadataProvider};

        let sfnt = decode_woff2(&tiny_woff2(false)).unwrap();
        let font = FontRef::new(&sfnt).unwrap();
        assert_eq!(font.maxp().unwrap().num_glyphs(), 2);
        assert_eq!(font.head().unwrap().index_to_loc_format(), 0);
        let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(metrics.advance_width(GlyphId::new(0)), Some(500.0));
        assert_eq!(metrics.advance_width(GlyphId::new(1)), Some(600.0));
        assert_eq!(metrics.left_side_bearing(GlyphId::new(1)), Some(100.0));
        let settings = || DrawSettings::unhinted(Size::unscaled(), LocationRef::default());
        let mut empty = Path::default();
        font.outline_glyphs().get(GlyphId::new(0)).unwrap().draw(settings(), &mut empty).unwrap();
        assert!(empty.0.is_empty());
        let mut path = Path::default();
        font.outline_glyphs().get(GlyphId::new(1)).unwrap().draw(settings(), &mut path).unwrap();
        assert_eq!(path.0, ["M100,0", "L500,0", "Q500,700,100,700", "Z"]);
    }

    #[test]
    fn woff2_size_claims_are_checked_before_decoding() {
        let woff = tiny_woff2(false);
        assert_eq!(woff2_sizes_fit(&woff, 256), Some(true));
        assert_eq!(woff2_sizes_fit(&woff, 255), Some(false));
        let mut claims_too_much = woff.clone();
        claims_too_much[16..20].copy_from_slice(&(MAX_FONT as u32 + 1).to_be_bytes());
        assert!(decode_woff2(&claims_too_much).is_none());
    }

    #[test]
    fn woff2_collection_decodes_to_a_ttc() {
        let ttc = decode_woff2(&tiny_woff2(true)).unwrap();
        assert!(ttc.starts_with(b"ttcf"));
        assert_eq!(skrifa::FontRef::from_index(&ttc, 0).unwrap().table_directory.num_tables(), 6);
    }

    #[test]
    fn damaged_woff2_never_panics() {
        for woff in [tiny_woff2(false), tiny_woff2(true)] {
            for len in 0..woff.len() {
                let mut cut = woff[..len].to_vec();
                if cut.len() >= 12 {
                    cut[8..12].copy_from_slice(&(len as u32).to_be_bytes());
                }
                assert!(decode_woff2(&cut).is_none());
            }
            for i in 0..woff.len() {
                for mask in [0x01, 0x10, 0x80, 0xff] {
                    let mut damaged = woff.clone();
                    damaged[i] ^= mask;
                    let _ = decode_woff2(&damaged);
                }
            }
        }
    }

    #[test]
    fn penpot_import_decodes_embedded_woff2() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("objects/font-uuid", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&tiny_woff2(false)).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut doc = Document::new("WOFF2");
        let mut warnings = Vec::new();
        import(&mut archive, &mut doc, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(doc.embedded_fonts.len(), 1);
        assert!(doc.embedded_fonts.values().all(|font| font.starts_with(&[0, 1, 0, 0])));
    }
}
