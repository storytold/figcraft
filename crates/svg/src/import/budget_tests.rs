//! Budgets on what SVGZ and embedded SVG documents expand into, with reduced limits so the
//! fixtures stay small.

use std::io::Write;

use base64::Engine as _;
use figcraft_doc::Document;

use super::budget::Limits;
use super::{import_svg, import_within};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn svg(body: &str) -> String {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">{body}</svg>"##)
}

fn uri(svg: &[u8]) -> String {
    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

/// `levels` SVGs, each embedding the next, the innermost holding one rectangle.
fn nested(levels: usize) -> String {
    let mut s = svg(r##"<rect width="10" height="10" fill="red"/>"##);
    for _ in 1..levels {
        s = svg(&format!(r##"<image width="100" height="100" xlink:href="{}"/>"##, uri(s.as_bytes())));
    }
    s
}

/// The layers an import makes (None when it is refused).
fn layers(data: &[u8], limits: Limits) -> Result<usize, String> {
    let mut d = Document::new("budget");
    let p = d.first_page().unwrap();
    let f = import_within(&mut d, data, p, 0.0, 0.0, "x", limits).map_err(|e| e.0)?.frame;
    assert!(d.validate().is_empty(), "{:?}", d.validate());
    Ok(d.subtree(f).len())
}

#[test]
fn an_svgz_is_unpacked_and_imported() {
    let z = gzip(svg(r##"<rect width="10" height="10"/>"##).as_bytes());
    let mut d = Document::new("z");
    let p = d.first_page().unwrap();
    assert!(import_svg(&mut d, &z, p, 0.0, 0.0, "z").is_ok());
}

#[test]
fn an_svgz_that_expands_past_the_budget_is_refused_early() {
    let limits = Limits { document: 4096, ..Limits::default() };
    // Tiny compressed, far past 4 KiB expanded.
    let z = gzip(svg(&" ".repeat(1 << 20)).as_bytes());
    assert!(z.len() < 4096, "the bomb is small: {}", z.len());
    let e = layers(&z, limits).unwrap_err();
    assert!(e.contains("expands past"), "{e}");
    // The same document is fine within a bigger budget.
    assert!(layers(&z, Limits { document: 2 << 20, ..Limits::default() }).is_ok());
}

#[test]
fn a_compressed_document_is_checked_for_nesting_once_unpacked() {
    let deep = format!("{}{}", "<g>".repeat(600), "</g>".repeat(600));
    let z = gzip(svg(&deep).as_bytes());
    assert!(layers(&z, Limits::default()).unwrap_err().contains("nested more than"));
}

#[test]
fn embedded_svgs_nest_up_to_the_depth_budget_and_no_further() {
    let data = nested(4).into_bytes();
    let full = layers(&data, Limits::default()).unwrap();
    let shallow = layers(&data, Limits { depth: 1, ..Limits::default() }).unwrap();
    assert!(shallow < full, "depth 1 drops the inner documents: {shallow} < {full}");
    // Far deeper than the budget: still imports, bounded.
    let deep = nested(40).into_bytes();
    assert!(layers(&deep, Limits::default()).is_ok());
}

#[test]
fn embedded_svgs_share_one_byte_budget() {
    let inner = svg(r##"<rect width="10" height="10"/>"##);
    let one = format!(r##"<image width="10" height="10" xlink:href="{}"/>"##, uri(inner.as_bytes()));
    let data = svg(&one.repeat(8)).into_bytes();
    let all = layers(&data, Limits::default()).unwrap();
    let limits = Limits { embedded: inner.len() * 3, ..Limits::default() };
    let some = layers(&data, limits).unwrap();
    assert!(some < all, "the budget runs out part way: {some} < {all}");
}

#[test]
fn an_embedded_compressed_bomb_is_dropped_not_expanded() {
    let bomb = gzip(svg(&" ".repeat(1 << 20)).as_bytes());
    let data = svg(&format!(
        r##"<image width="10" height="10" xlink:href="data:image/svg+xml;base64,{}"/>"##,
        base64::engine::general_purpose::STANDARD.encode(&bomb)
    ))
    .into_bytes();
    let limits = Limits { embedded: 4096, ..Limits::default() };
    let with_bomb = layers(&data, limits).unwrap();
    let empty = layers(svg("").as_bytes(), limits).unwrap();
    assert_eq!(with_bomb, empty, "the oversized embedded document left nothing behind");
}

#[test]
fn linked_files_and_other_mime_types_stay_unresolved() {
    let data = svg(
        r##"<image width="10" height="10" xlink:href="/etc/passwd"/><image width="10" height="10" xlink:href="data:application/x-foo;base64,AAAA"/>"##,
    );
    assert_eq!(layers(data.as_bytes(), Limits::default()).unwrap(), layers(svg("").as_bytes(), Limits::default()).unwrap());
}
