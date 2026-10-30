//! Layers pasted from another design app whose paints come from a shared paint style (#1638): the
//! clipboard links the style by the guid it has in the library it came from, which this payload
//! doesn't hold, and by its library asset, whose copy it does hold. The layers (a component's
//! glyph and the same glyph in an instance of it) take the style's colour and stay linked to it.
//! A link to a style the payload doesn't hold at all keeps the colour stored with the layer, with
//! a note. Built in code and read as `edit.pasteDesignLayers` reads a clipboard's design data.

use figcraft_doc::{Document, Node};

use super::kiwi::Kv;
use super::tests::{arr, canvas, e, g, node, o, s, schema, solid, xy};
use super::try_import;

/// A style link by `guid` (absent from the payload) and, given, the library asset `key`.
fn link(guid: u64, asset: Option<&str>) -> Kv<'static> {
    let mut fields = vec![("guid", g(guid))];
    if let Some(key) = asset {
        fields.push(("assetRef", o(vec![("key", s(key)), ("version", s("7"))])));
    }
    o(fields)
}

/// The pasted payload: on the internal page, the file's copy of library style "Red" (#FF4040);
/// on the page, component "Icon" whose "Glyph" stores stale black and links the style, an
/// instance of it, and "Lost", which stores #1F1F1F and links a style the payload doesn't hold.
fn pasted() -> (Document, Vec<String>) {
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(
            10,
            Some((2, "!")),
            "RECTANGLE",
            "Red",
            vec![("styleType", e("FILL")), ("key", s("lib-red")), ("version", s("7")), ("fillPaints", arr(vec![solid(1.0, 0.25, 0.25)]))],
        ),
        node(20, Some((1, "!")), "SYMBOL", "Icon", vec![("size", xy(24.0, 24.0))]),
        node(
            21,
            Some((20, "!")),
            "RECTANGLE",
            "Glyph",
            vec![("size", xy(16.0, 16.0)), ("fillPaints", arr(vec![solid(0.0, 0.0, 0.0)])), ("styleIdForFill", link(99, Some("lib-red")))],
        ),
        node(30, Some((1, "\"")), "INSTANCE", "Icon copy", vec![("size", xy(24.0, 24.0)), ("symbolData", o(vec![("symbolID", g(20))]))]),
        node(
            40,
            Some((1, "#")),
            "RECTANGLE",
            "Lost",
            vec![("size", xy(16.0, 16.0)), ("fillPaints", arr(vec![solid(0.12, 0.12, 0.12)])), ("styleIdForFill", link(98, None))],
        ),
    ];
    let bytes = canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))]));
    try_import(&bytes, "Pasted layers").expect("design data").expect("it imports")
}

fn fill_hex(n: &Node) -> Option<String> {
    n.fills.first()?.solid_color().map(|c| c.to_hex())
}

#[test]
fn a_library_paint_style_colours_the_component_and_its_instance() {
    let (doc, _) = pasted();
    let glyphs: Vec<&Node> = doc.nodes().filter(|n| n.name == "Glyph").collect();
    assert_eq!(glyphs.len(), 2, "the component's glyph and the instance's");
    for glyph in glyphs {
        assert_eq!(fill_hex(glyph).as_deref(), Some("FF4040"), "the style's colour, not the stale black");
        let style = glyph.styles.fill.as_deref().and_then(|id| doc.style(id)).expect("linked to the style");
        assert_eq!(style.name, "Red");
    }
}

#[test]
fn a_style_the_paste_doesnt_hold_keeps_the_stored_colour() {
    let (doc, warnings) = pasted();
    let lost = doc.nodes().find(|n| n.name == "Lost").unwrap();
    assert_eq!(fill_hex(lost).as_deref(), Some("1F1F1F"), "the colour stored with the layer, not black");
    assert!(lost.styles.fill.is_none());
    assert!(warnings.iter().any(|w| w.contains("shared styles that aren't in the file")), "{warnings:?}");
}
