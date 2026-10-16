//! CMYK export converts the colours inside vector pattern tiles (#426).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind, PrintSettings};
use figcraft_pdf::{PdfOptions, export_pdf};
use std::io::Read;

/// Every zlib stream of `pdf`, inflated and joined.
fn streams(pdf: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = pdf;
    while let Some(i) = rest.windows(7).position(|w| w == b"stream\n") {
        rest = &rest[i + 7..];
        let end = rest.windows(9).position(|w| w == b"endstream").unwrap();
        let mut bytes = Vec::new();
        if flate2::read::ZlibDecoder::new(&rest[..end]).read_to_end(&mut bytes).is_ok() {
            out.push_str(&String::from_utf8_lossy(&bytes));
            out.push('\n');
        }
        rest = &rest[end + 9..];
    }
    out
}

fn export(pattern: bool, cmyk: bool) -> (String, String) {
    let mut doc = Document::new("fixture");
    let page = doc.first_page().unwrap();
    let source = doc.create(page, NodeKind::Rectangle, "Source", 40.0, 0.0, 10.0, 10.0).unwrap();
    let green = Paint::solid(Color::from_hex("00FF00").unwrap());
    doc.get_mut(source).unwrap().fills = vec![green.clone()];
    let target = doc.create(page, NodeKind::Rectangle, "Target", 0.0, 0.0, 20.0, 20.0).unwrap();
    doc.get_mut(target).unwrap().fills = vec![if pattern { Paint::pattern(source.0) } else { green }];
    let opts = PdfOptions { print: PrintSettings { cmyk, ..Default::default() }, ..Default::default() };
    let pdf = export_pdf(&doc, &[target], &opts).unwrap();
    (String::from_utf8_lossy(&pdf).into_owned(), streams(&pdf))
}

fn has_rgb_op(content: &str) -> bool {
    content.split_whitespace().any(|op| op == "rg" || op == "RG")
}

#[test]
fn pattern_cmyk_controls() {
    let (_, solid) = export(false, true);
    assert!(solid.contains("1 0 1 0 k"), "{solid}");
    assert!(!has_rgb_op(&solid), "{solid}");
    let (pdf, pattern) = export(true, false);
    assert!(pdf.contains("/PatternType 1"));
    assert!(pattern.contains("0 1 0 rg"), "{pattern}");
}

#[test]
fn pattern_colors_follow_cmyk_setting() {
    let (pdf, content) = export(true, true);
    assert!(pdf.contains("/PatternType 1"));
    assert!(!has_rgb_op(&content), "RGB operator remains in CMYK pattern: {content}");
    assert!(content.contains("1 0 1 0 k"), "green must convert to CMYK: {content}");
}
