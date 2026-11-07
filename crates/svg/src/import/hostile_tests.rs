//! Hostile input: malformed, extreme and self-referencing SVG, and the corpus mangled byte by
//! byte. The importer never panics, and whatever it makes is a valid document that renders.

use figcraft_doc::Document;

use super::import_svg;

/// Import `data`; when it parses, check the document and draw it.
fn survive(data: &[u8]) {
    let mut d = Document::new("hostile");
    let Some(p) = d.first_page() else { return };
    if let Ok(f) = import_svg(&mut d, data, p, 0.0, 0.0, "x") {
        assert!(d.validate().is_empty(), "{:?}", d.validate());
        let view = figcraft_render::View { origin: (0.0, 0.0), scale: 0.5, width: 64, height: 64 };
        if let Some(mut pix) = figcraft_render::Pixmap::new(64, 64) {
            figcraft_render::render_nodes_into(&d, &[f], &view, &mut pix);
        }
    }
}

fn svg(body: &str) -> Vec<u8> {
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">{body}</svg>"##)
        .into_bytes()
}

#[test]
fn malformed_documents() {
    for data in [
        &b""[..],
        b"<",
        b"<svg",
        b"<svg>",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"10\" height=\"10\">",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"-5\" height=\"0\"/>",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 0 0\"><rect width=\"1\" height=\"1\"/></svg>",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"nan nan inf -inf\" width=\"1e39\" height=\"1e39\"/>",
        b"<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY a \"&a;\">]><svg xmlns=\"http://www.w3.org/2000/svg\">&a;</svg>",
        b"<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><svg:rect width=\"5\" height=\"5\"/></svg:svg>",
        b"\xef\xbb\xbf<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><circle r=\"3\"/></svg>",
        b"\xff\xfe\x00<\x00s\x00v\x00g",
        b"not svg at all",
    ] {
        survive(data);
    }
}

#[test]
fn extreme_values() {
    for body in [
        r##"<rect x="1e38" y="-1e38" width="1e38" height="1e38"/>"##,
        r##"<rect width="NaN" height="inf"/><circle r="-5"/><ellipse rx="0" ry="1e30"/>"##,
        r##"<path d="M0 0 L1e38 1e38 L-1e38 0 Z" stroke="red" stroke-width="1e30" stroke-dasharray="0.000001 0.000001"/>"##,
        r##"<path d="M0 0 A1e30 1e-30 1e9 1 1 10 10 Z M nan 0 L 5 inf"/>"##,
        r##"<g transform="scale(0)"><rect width="10" height="10"/></g><g transform="matrix(1e30 1e30 1e30 1e30 0 0)"><rect width="10" height="10"/></g>"##,
        r##"<g transform="rotate(1e30) skewX(90)"><rect width="10" height="10" rx="1e30"/></g>"##,
        r##"<line x2="10" stroke="black" stroke-width="2" stroke-dashoffset="1e30" stroke-dasharray="3 1 2"/>"##,
        r##"<filter id="f"><feGaussianBlur stdDeviation="1e9"/></filter><rect width="10" height="10" filter="url(#f)"/>"##,
        r##"<filter id="f"><feDropShadow dx="1e30" dy="-1e30" stdDeviation="1e20"/></filter><rect width="10" height="10" filter="url(#f)"/>"##,
        r##"<filter id="f"><feMorphology operator="dilate" radius="1e20"/><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter><rect width="10" height="10" filter="url(#f)"/>"##,
        r##"<linearGradient id="g"/><rect width="10" height="10" fill="url(#g)"/>"##,
        r##"<linearGradient id="g" x1="0" x2="0" spreadMethod="repeat"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient><rect width="10" height="10" fill="url(#g)"/>"##,
        r##"<linearGradient id="g" x2="1e-30" spreadMethod="reflect"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient><rect width="1e30" height="10" fill="url(#g)"/>"##,
        r##"<radialGradient id="g" r="1e-30" spreadMethod="repeat"><stop offset="0" stop-color="#000"/><stop offset="1"/></radialGradient><rect width="10" height="10" fill="url(#g)"/>"##,
        r##"<pattern id="p" width="1e-30" height="1e30" patternUnits="userSpaceOnUse"><rect width="1" height="1"/></pattern><rect width="10" height="10" fill="url(#p)"/>"##,
        r##"<pattern id="p" width="1e30" height="1e30" patternUnits="userSpaceOnUse"><circle r="1e29"/></pattern><rect width="1e30" height="1e30" fill="url(#p)"/>"##,
        r##"<text x="1e38" y="nan" font-size="1e30" letter-spacing="-1e30">x</text><text font-size="0">y</text>"##,
        r##"<text><tspan x="0 1 2 3" y="5" dx="1e30" rotate="1e9 nan">abcd</tspan><tspan>&#xFFFF;&#x10FFFF;</tspan></text>"##,
        r##"<image width="10" height="10" href="data:image/png;base64,iVBORw0KGgoAAAANSU"/><image width="1e30" height="0" href="data:,"/>"##,
        r##"<image width="10" height="10" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3Cimage href='data:image/svg+xml,%253Csvg/%253E'/%3E%3C/svg%3E"/>"##,
        r##"<style>* { fill: url(#nope); stroke-width: -3; } rect { transform: scale(-1e30) }</style><rect width="10" height="10"/>"##,
    ] {
        survive(&svg(body));
    }
}

#[test]
fn references_to_themselves() {
    for body in [
        r##"<use id="u" href="#u"/>"##,
        r##"<g id="a"><use href="#b"/></g><g id="b"><use href="#a"/></g>"##,
        r##"<clipPath id="c" clip-path="url(#c)"><rect width="5" height="5"/></clipPath><rect width="10" height="10" clip-path="url(#c)"/>"##,
        r##"<mask id="m" mask="url(#m)"><rect width="5" height="5" fill="white"/></mask><rect width="10" height="10" mask="url(#m)"/>"##,
        r##"<pattern id="p" width="5" height="5" patternUnits="userSpaceOnUse" href="#p"><rect width="2" height="2" fill="url(#p)"/></pattern><rect width="10" height="10" fill="url(#p)"/>"##,
        r##"<linearGradient id="g" href="#h"/><linearGradient id="h" href="#g"/><rect width="10" height="10" fill="url(#g)"/>"##,
        r##"<filter id="f" href="#f"><feImage href="#r"/></filter><rect id="r" width="10" height="10" filter="url(#f)"/>"##,
    ] {
        survive(&svg(body));
    }
}

#[test]
fn deep_and_wide_documents_are_bounded() {
    survive(&svg(&format!("{}<rect width=\"1\" height=\"1\"/>{}", "<g>".repeat(3000), "</g>".repeat(3000))));
    survive(&svg(&format!("{}<rect width=\"1\" height=\"1\"/>{}", "<g transform=\"scale(1.01)\" opacity=\"0.99\">".repeat(200), "</g>".repeat(200))));
    let many: String = (0..5000).map(|i| format!("<rect x=\"{}\" width=\"1\" height=\"1\"/>", i % 100)).collect();
    survive(&svg(&many));
    let prims: String = (0..500).map(|i| format!("<feOffset dx=\"{i}\"/>")).collect();
    survive(&svg(&format!(r##"<filter id="f">{prims}</filter><rect width="10" height="10" filter="url(#f)"/>"##)));
    let dashes: String = (0..10_000).map(|i| format!("{} ", i % 7 + 1)).collect();
    survive(&svg(&format!(r##"<line x2="100" stroke="black" stroke-dasharray="{dashes}"/>"##)));
}

/// A small deterministic generator for the mangling.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn the_corpus_mangled_byte_by_byte() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");
    let files: Vec<Vec<u8>> = std::fs::read_dir(dir).unwrap().filter_map(|e| std::fs::read(e.ok()?.path()).ok()).collect();
    assert!(files.len() > 10);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let junk: &[&[u8]] = &[b"<", b">", b"\"", b"-", b"e", b"9999999", b"url(#", b"NaN", b"\0", b"</g>", b"<g transform=\"scale(1e30)\">"];
    for f in &files {
        // Cut short at a spread of points.
        for k in 1..12 {
            survive(f.get(..f.len() * k / 12).unwrap_or(f));
        }
        // Bytes changed, dropped and inserted.
        for _ in 0..24 {
            let mut m = f.clone();
            for _ in 0..1 + rng.below(6) {
                let at = rng.below(m.len());
                match rng.below(3) {
                    0 => {
                        if let Some(b) = m.get_mut(at) {
                            *b = (rng.next() & 0xff) as u8;
                        }
                    }
                    1 => {
                        let end = (at + rng.below(16)).min(m.len());
                        m.drain(at..end);
                    }
                    _ => {
                        let j = junk[rng.below(junk.len())];
                        m.splice(at..at, j.iter().copied());
                    }
                }
            }
            survive(&m);
        }
    }
}

#[test]
fn nesting_is_measured_before_parsing() {
    use super::nesting;
    assert_eq!(nesting(b"<svg><g><g/><g a='>' b=\"/>\"></g></g></svg>", 99), 3);
    assert_eq!(nesting(b"<?xml version='1.0'?><!-- <g><g> --><svg><![CDATA[<g><g>]]></svg>", 99), 1);
    assert_eq!(nesting(b"<svg><g", 99), 2);
    let deep = format!("{}{}", "<g>".repeat(5000), "</g>".repeat(5000));
    assert_eq!(nesting(deep.as_bytes(), 512), 513, "stops once past the limit");
    // Far too deep for the parser's stack: refused.
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    assert!(import_svg(&mut d, &svg(&deep), p, 0.0, 0.0, "x").is_err());
}

#[test]
fn entity_expansion_cannot_bypass_the_nesting_limit() {
    // An internal-DTD entity whose value nests 300 deep, referenced inside a body nesting 300 deep:
    // the parsed tree is ~600 deep although the raw bytes never are.
    let inner = format!("{}{}", "<g>".repeat(300), "</g>".repeat(300));
    let body = format!("{}&deep;{}", "<g>".repeat(300), "</g>".repeat(300));
    let data = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY deep \"{inner}\">]><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">{body}</svg>"
    );
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let err = import_svg(&mut d, data.as_bytes(), p, 0.0, 0.0, "x").err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("nested more than 512 deep"), "{err:?}");
}
