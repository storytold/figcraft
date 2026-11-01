//! Dense legitimate canvases must open whole within the value budget their size gives them, and a
//! small canvas that unpacks past the floor (tens of millions of values) must stay refused. The dense ones take seconds and gigabytes:
//! `cargo test -p figcraft-format --release --lib value_budget -- --ignored --test-threads=1`.

use super::kiwi::{Kv, Schema, encode};
use super::tests::{Lcg, arr, at, f, g, node, o, s, schema, solid, u, xy};
use super::{decode_canvas, export, import, kiwi};

fn frac(r: &mut Lcg, hi: f64) -> f64 {
    (r.next() % 100_000) as f64 / 100_000.0 * hi
}

fn hex(r: &mut Lcg, nibbles: usize) -> String {
    (0..nibbles).map(|_| format!("{:x}", r.next() % 16)).collect()
}

fn word(r: &mut Lcg) -> &'static str {
    const WORDS: &[&str] = &[
        "frame", "card", "hero", "row", "cell", "icon", "badge", "button", "input", "label", "group", "section", "header", "footer", "nav", "list",
        "item", "menu", "panel", "view", "page", "modal", "form", "field", "text", "title", "body", "caption", "avatar", "thumb", "chart", "graph",
        "axis", "grid", "slot", "chip", "tag", "pill", "note", "hint", "tip", "alert", "toast", "banner", "strip", "rail", "tabs", "tab", "step",
        "flow", "state", "theme", "mode", "token",
    ];
    WORDS[(r.next() as usize) % WORDS.len()]
}

/// Strings and GUIDs with real entropy on every record (a key, a version, a path of GUIDs), so
/// deflate packs the fixture about as tightly as a real file rather than far tighter.
fn identity(r: &mut Lcg, depth: usize) -> Vec<(&'static str, Kv<'static>)> {
    let path = (0..depth).map(|_| g(r.next() % (1 << 30))).collect();
    vec![
        ("key", s(format!("{}:{}{}", hex(r, 4), hex(r, 4), r.next() % 997))),
        ("version", s(format!("{}:0", hex(r, 6)))),
        ("guidPath", o(vec![("guids", arr(path))])),
    ]
}

/// Somewhere on a page the size of a large design file.
fn placed(r: &mut Lcg) -> (&'static str, Kv<'static>) {
    ("transform", at(frac(r, 12_000.0), frac(r, 8_000.0)))
}

fn rect(id: u64, page: u64, r: &mut Lcg) -> Kv<'static> {
    let name = format!("{} {} {}", word(r), word(r), r.next() % 9_999);
    let mut fields = vec![
        ("size", xy(24.0 + frac(r, 300.0), 16.0 + frac(r, 200.0))),
        placed(r),
        ("fillPaints", arr(vec![solid(frac(r, 1.0), frac(r, 1.0), frac(r, 1.0))])),
        ("strokeWeight", f(1.0 + frac(r, 3.0))),
        ("cornerRadius", f(frac(r, 24.0))),
        ("opacity", f(0.5 + frac(r, 0.5))),
        ("visible", Kv::Bool(true)),
    ];
    fields.extend(identity(r, 3));
    node(id, Some((page, "A")), "RECTANGLE", &name, fields)
}

/// Text saved with per-character styles: its characters and one style id per character.
fn text(id: u64, page: u64, chars: usize, r: &mut Lcg) -> Kv<'static> {
    let mut characters = String::new();
    while characters.len() < chars {
        characters.push_str(word(r));
        characters.push_str(if r.next().is_multiple_of(5) { ". " } else { " " });
    }
    characters.truncate(chars);
    let styles = arr((0..characters.chars().count()).map(|_| u(0)).collect());
    let mut fields = vec![
        ("size", xy(320.0, 18.0)),
        placed(r),
        ("fontSize", f(11.0 + frac(r, 26.0))),
        ("fontName", o(vec![("family", s("Inter")), ("style", s("Regular"))])),
        ("fillPaints", arr(vec![solid(0.1, 0.1, 0.1)])),
        ("textData", o(vec![("characters", s(characters)), ("characterStyleIDs", styles)])),
    ];
    fields.extend(identity(r, 4));
    node(id, Some((page, "A")), "TEXT", "Body", fields)
}

/// A canvas whose message writes one list field (field 1) of `count` already-encoded elements.
fn one_list_canvas(sc: &Schema, count: u32, elements: impl IntoIterator<Item = u8>) -> Vec<u8> {
    let mut head = encode::Writer::default();
    head.var_uint(1);
    head.var_uint(count);
    let mut msg = head.0;
    msg.extend(elements);
    msg.push(0);
    export::canvas(&encode::schema(sc), &msg).unwrap()
}

/// `pages` pages of `rects` rectangles and `texts` texts of `chars` characters, framed the way the
/// exporter frames a file.
fn canvas(pages: usize, rects: usize, texts: usize, chars: usize) -> Vec<u8> {
    let sc = schema();
    let nc = sc.find("NodeChange").unwrap();
    let mut r = Lcg(0x5eed_1234_abcd_9999);
    // Layers are encoded one at a time into the one field written (`nodeChanges`): one Kv tree of
    // the whole message would take gigabytes of its own.
    let mut layers = Vec::new();
    let mut count = 0u32;
    let mut add = |n: Kv<'static>| {
        layers.extend(encode::message(&sc, nc, &n).unwrap());
        count += 1;
    };
    add(node(0, None, "DOCUMENT", "Fixture", vec![]));
    let mut id = 10;
    for p in 0..pages {
        let page = id;
        id += 1;
        add(node(page, Some((0, "A")), "CANVAS", &format!("Page {p}"), vec![]));
        for _ in 0..rects {
            add(rect(id, page, &mut r));
            id += 1;
        }
        for _ in 0..texts {
            add(text(id, page, chars, &mut r));
            id += 1;
        }
    }
    one_list_canvas(&sc, count, layers)
}

/// Every layer comes in and nothing is reported — and the fixture makes its point: one value per
/// compressed byte refuses it, and it decodes within 90% of its budget. Returns the canvas size.
fn opens_whole(label: &str, pages: usize, rects: usize, texts: usize, chars: usize) -> usize {
    let file = canvas(pages, rects, texts, chars);
    let (sc, msg) = decode_canvas(&file).unwrap();
    let root = sc.find("Message").unwrap();
    let one_per_byte = file.len().clamp(kiwi::value_budget(0), kiwi::value_budget(usize::MAX));
    let refused = sc.decode_message_within(root, &msg, one_per_byte).is_err_and(|e| e.0 == "message: too many values");
    assert!(refused, "{label}: one value per compressed byte no longer refuses it");
    let (doc, warnings) = import(&file, label).unwrap_or_else(|e| panic!("{label}: refused: {e}"));
    assert_eq!(doc.node_count(), pages * (rects + texts) + pages, "{label}: layers");
    assert!(warnings.is_empty(), "{label}: {warnings:?}");
    println!("{label}: {} canvas bytes, {} layers", file.len(), doc.node_count());
    drop(doc);
    assert!(sc.decode_message_within(root, &msg, kiwi::value_budget(file.len()) / 10 * 9).is_ok(), "{label}: not within 90% of its budget");
    file.len()
}

#[test]
#[ignore = "heavy: --release --ignored, about 4.5 GB"]
fn dense_canvases_open_whole() {
    opens_whole("21 MB design-shape canvas", 44, 3000, 2000, 140);
    opens_whole("15 MB text-heavy canvas", 12, 4500, 4500, 400);
}

/// From 32 MiB of canvas the budget is the ceiling. Its 83 million values are past the 32-bit
/// ceiling, so this one is for 64-bit targets.
#[cfg(not(target_pointer_width = "32"))]
#[test]
#[ignore = "heavy: --release --ignored, about 9 GB"]
fn a_canvas_budgeted_at_the_ceiling_opens_whole() {
    let packed = opens_whole("50 MB text-heavy canvas", 106, 0, 5000, 100);
    assert_eq!(kiwi::value_budget(packed), kiwi::value_budget(usize::MAX), "budgeted at the ceiling");
}

/// However far a small canvas inflates, the floor bounds what it may decode.
#[test]
fn a_small_canvas_that_unpacks_to_millions_of_values_is_refused() {
    let n = 30_000_000;
    let file = one_list_canvas(&Schema::parse("message Message uint[] xs").unwrap(), n, std::iter::repeat_n(0, n as usize));
    assert!(file.len() < 100_000, "{} bytes", file.len());
    let Err(e) = import(&file, "x") else { panic!("a {} byte canvas holding {n} values opened", file.len()) };
    assert_eq!(e.to_string(), "not a valid FigCraft file: .fig file: message: too many values");
}
