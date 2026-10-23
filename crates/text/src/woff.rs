//! WOFF: a TrueType font with its tables deflated, in the wrapper a web page expects.
//!
//! [`from_ttf`] takes the bytes [`crate::iconfont::build_ttf`] produces and packs them as WOFF 1.0:
//! a 44-byte header, a 20-byte directory entry per table, then the tables themselves, each one
//! zlib-deflated (or stored as-is when deflating it would not make it smaller, which the format
//! allows and which a table of a few bytes often needs).
//!
//! The result is the same font — a browser reads it in place of the TTF — and it is a good deal
//! smaller, which is the whole point of the format for the icons a page loads.

use std::io::Write as _;

/// The WOFF signature, `wOFF`.
const SIGNATURE: u32 = 0x774F_4646;
/// The header size, and the size of one directory entry in the WOFF's own directory.
const HEADER: usize = 44;
const ENTRY: usize = 20;
/// The size of an entry in the sfnt (TTF) directory this reads: the WOFF's has one field more.
const SFNT_ENTRY: usize = 16;

/// `ttf` as a WOFF file, or `None` when it is not a font this can read.
pub fn from_ttf(ttf: &[u8]) -> Option<Vec<u8>> {
    let flavor = be32(ttf, 0)?;
    let num_tables = u16::from_be_bytes([*ttf.get(4)?, *ttf.get(5)?]);
    // The directory has to be sorted by tag, which a sfnt's already is.
    let mut tables: Vec<Entry> = Vec::with_capacity(usize::from(num_tables));
    for i in 0..usize::from(num_tables) {
        let e = 12 + i * SFNT_ENTRY;
        let tag: [u8; 4] = ttf.get(e..e + 4)?.try_into().ok()?;
        let checksum = be32(ttf, e + 4)?;
        let at = be32(ttf, e + 8)? as usize;
        let len = be32(ttf, e + 12)? as usize;
        let data = ttf.get(at..at.checked_add(len)?)?.to_vec();
        tables.push(Entry { tag, checksum, orig_len: len as u32, stored: Vec::new(), at: 0, data });
    }
    if tables.is_empty() {
        return None;
    }

    // Deflate what deflating helps, and lay the result out 4 bytes apart from the directory.
    let mut at = HEADER + ENTRY * tables.len();
    for e in &mut tables {
        let deflated = deflate(&e.data)?;
        e.stored = if deflated.len() < e.data.len() { deflated } else { e.data.clone() };
        e.at = at as u32;
        at += e.stored.len() + padding(e.stored.len());
    }
    let total = at;

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&SIGNATURE.to_be_bytes());
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(&(ttf.len() as u32).to_be_bytes()); // the sfnt this came from
    out.extend_from_slice(&1u16.to_be_bytes()); // major version
    out.extend_from_slice(&0u16.to_be_bytes()); // minor version
    for _ in 0..3 {
        out.extend_from_slice(&0u32.to_be_bytes()); // no metadata
    }
    for _ in 0..2 {
        out.extend_from_slice(&0u32.to_be_bytes()); // no private data
    }
    for e in &tables {
        out.extend_from_slice(&e.tag);
        out.extend_from_slice(&e.at.to_be_bytes());
        out.extend_from_slice(&(e.stored.len() as u32).to_be_bytes());
        out.extend_from_slice(&e.orig_len.to_be_bytes());
        out.extend_from_slice(&e.checksum.to_be_bytes());
    }
    for e in &tables {
        out.extend_from_slice(&e.stored);
        out.resize(out.len() + padding(e.stored.len()), 0);
    }
    (out.len() == total).then_some(out)
}

/// One table on its way into the file.
struct Entry {
    tag: [u8; 4],
    /// The checksum the sfnt directory gave it, which WOFF keeps.
    checksum: u32,
    orig_len: u32,
    data: Vec<u8>,
    /// The bytes actually written: the data deflated, or the data itself.
    stored: Vec<u8>,
    /// Where `stored` goes in the file.
    at: u32,
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(b))
}

/// The bytes that pad `len` up to a multiple of four.
fn padding(len: usize) -> usize {
    (4 - len % 4) % 4
}

fn deflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).ok()?;
    e.finish().ok()
}

#[cfg(test)]
#[path = "woff_tests.rs"]
mod tests;
