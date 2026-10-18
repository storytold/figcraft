//! WOFF: the same font as the TTF, with its tables deflated.

use std::io::Read as _;

use super::*;
use crate::iconfont::build_ttf;
use figcraft_geom::BezPath;

/// A square in the em square, y down: a glyph with something in it.
fn square() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((100.0, 100.0));
    p.line_to((900.0, 100.0));
    p.line_to((900.0, 900.0));
    p.line_to((100.0, 900.0));
    p.close_path();
    p
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn a_woff_holds_the_same_font_smaller() {
    let ttf = build_ttf("Icons", &[(0xE000, square()), (0xE001, square())]).expect("built");
    let woff = from_ttf(&ttf).expect("packed");
    assert_eq!(u32_at(&woff, 0), SIGNATURE, "the file starts with 'wOFF'");
    assert_eq!(&woff[4..8], &ttf[0..4], "and keeps the flavour (the sfnt version)");
    assert_eq!(&woff[8..12], &(woff.len() as u32).to_be_bytes(), "the length it says it is");
    let tables = u16::from_be_bytes([woff[12], woff[13]]);
    assert_eq!(tables, u16::from_be_bytes([ttf[4], ttf[5]]), "every table comes along");
    assert_eq!(u32_at(&woff, 16), ttf.len() as u32, "and says how big the sfnt it came from is");
    assert!(woff.len() < ttf.len(), "the point of it: {} bytes against {}", woff.len(), ttf.len());
}

/// Each table comes back out of the WOFF byte for byte the one the TTF holds, which is what makes
/// it the same font rather than a lookalike.
#[test]
fn every_table_deflates_back_to_the_one_in_the_ttf() {
    let ttf = build_ttf("Icons", &[(0xE000, square()), (0xE001, square())]).expect("built");
    let woff = from_ttf(&ttf).expect("packed");
    let tables = usize::from(u16::from_be_bytes([woff[12], woff[13]]));
    for i in 0..tables {
        let e = HEADER + i * ENTRY;
        let tag = &woff[e..e + 4];
        let at = u32_at(&woff, e + 4) as usize;
        let stored_len = u32_at(&woff, e + 8) as usize;
        let orig_len = u32_at(&woff, e + 12) as usize;
        assert_eq!(at % 4, 0, "{tag:?} is 4-byte aligned");
        let stored = &woff[at..at + stored_len];
        let back = if stored_len == orig_len {
            stored.to_vec()
        } else {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(stored).read_to_end(&mut out).expect("inflates");
            out
        };
        assert_eq!(back.len(), orig_len, "{tag:?} is as long as it says");
        // The same table of the TTF, at the offset its own directory gives it.
        let sfnt = (0..usize::from(u16::from_be_bytes([ttf[4], ttf[5]])))
            .map(|j| 12 + j * 16)
            .find(|j| &ttf[*j..*j + 4] == tag)
            .expect("the table is in the ttf too");
        let t_at = u32_at(&ttf, sfnt + 8) as usize;
        assert_eq!(back, ttf[t_at..t_at + orig_len], "{tag:?} survived the round trip");
    }
}

/// The directory keeps the checksums the sfnt gave it: a reader checks a table against them.
#[test]
fn the_checksums_are_the_ones_the_sfnt_had() {
    let ttf = build_ttf("Icons", &[(0xE000, square())]).expect("built");
    let woff = from_ttf(&ttf).expect("packed");
    let tables = usize::from(u16::from_be_bytes([woff[12], woff[13]]));
    for i in 0..tables {
        let e = HEADER + i * ENTRY;
        let tag = &woff[e..e + 4];
        let checksum = u32_at(&woff, e + 16);
        let sfnt = (0..usize::from(u16::from_be_bytes([ttf[4], ttf[5]])))
            .map(|j| 12 + j * 16)
            .find(|j| &ttf[*j..*j + 4] == tag)
            .expect("the table is in the ttf too");
        assert_eq!(checksum, u32_at(&ttf, sfnt + 4), "{tag:?} kept its checksum");
    }
}
