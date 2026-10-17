//! Paragraph options (list spacing, hanging list and punctuation, leading trim) in encoded `.fig`
//! files: the highest valid value wins, invalid instance rows fall through, native saves keep them.

use super::kiwi::Kv;
use super::{export, import};

fn paragraph_fig(shared: Vec<(&'static str, Kv<'static>)>, local: Vec<(&'static str, Kv<'static>)>) -> Vec<u8> {
    use super::kiwi::{Schema, encode};

    let schema = Schema::parse(
        "enum NodeType NONE DOCUMENT CANVAS TEXT\n\
         enum StyleType NONE FILL TEXT\n\
         enum LeadingTrim NONE CAP_HEIGHT\n\
         struct GUID uint sessionID, uint localID\n\
         struct ParentIndex GUID guid, string position\n\
         struct FontName string family, string style, string postscript\n\
         struct StyleId GUID guid\n\
         message TextData string characters\n\
         message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, StyleType styleType, FontName fontName, float fontSize, StyleId styleIdForText, TextData textData, float paragraphSpacing, float paragraphIndent, float listSpacing, bool hangingList, bool hangingPunctuation, LeadingTrim leadingTrim\n\
         message Message NodeChange[] nodeChanges",
    )
    .unwrap();
    fn object(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
        Kv::Object(fields)
    }
    fn guid(id: u64) -> Kv<'static> {
        object(vec![("sessionID", Kv::Uint(7)), ("localID", Kv::Uint(id))])
    }
    fn node(id: u64, ty: &'static str, name: &str, parent: Option<u64>, fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
        let mut out = vec![("guid", guid(id)), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
        if let Some(parent) = parent {
            out.push(("parentIndex", object(vec![("guid", guid(parent)), ("position", Kv::Str("a".into()))])));
        }
        out.extend(fields);
        object(out)
    }
    fn font(family: &str) -> Kv<'static> {
        object(vec![("family", Kv::Str(family.into())), ("style", Kv::Str("Regular".into())), ("postscript", Kv::Str(String::new()))])
    }
    let mut base = vec![("styleType", Kv::Enum("TEXT")), ("fontName", font("Inter")), ("fontSize", Kv::Float(20.0))];
    base.extend(shared);
    let mut layer = vec![
        ("fontName", font("Stale")),
        ("fontSize", Kv::Float(40.0)),
        ("styleIdForText", object(vec![("guid", guid(10))])),
        ("textData", object(vec![("characters", Kv::Str("a\nb".into()))])),
    ];
    layer.extend(local);
    let msg = object(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, "DOCUMENT", "Document", None, vec![]),
            node(2, "CANVAS", "Page", Some(1), vec![]),
            node(10, "TEXT", "Shared", Some(2), base),
            node(11, "TEXT", "Label", Some(2), layer),
        ]),
    )]);
    let root = schema.find("Message").unwrap();
    let mut deflated = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write;
    deflated.write_all(&encode::schema(&schema)).unwrap();
    let schema_chunk = deflated.finish().unwrap();
    let message = encode::message(&schema, root, &msg).unwrap();
    let message_chunk = ruzstd::encoding::compress_to_vec(message.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
    let mut bytes = b"fig-kiwi".to_vec();
    bytes.extend_from_slice(&101u32.to_le_bytes());
    for chunk in [schema_chunk, message_chunk] {
        bytes.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

#[test]
fn encoded_fig_maps_paragraph_list_fields_through_native_round_trip() {
    let shared = vec![
        ("listSpacing", Kv::Float(18.0)),
        ("hangingList", Kv::Bool(false)),
        ("hangingPunctuation", Kv::Bool(false)),
        ("leadingTrim", Kv::Enum("CAP_HEIGHT")),
    ];
    let (inherited, warnings) = import(&paragraph_fig(shared.clone(), vec![]), "Synthetic paragraph options").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let text = inherited.nodes().find(|node| node.name == "Label").unwrap().kind.text().unwrap();
    assert_eq!(text.list_spacing, 18.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::CapHeight);
    assert_eq!(text.style.font_family, "Inter");

    let reset = vec![
        ("listSpacing", Kv::Float(0.0)),
        ("hangingList", Kv::Bool(false)),
        ("hangingPunctuation", Kv::Bool(false)),
        ("leadingTrim", Kv::Enum("NONE")),
    ];
    let (cleared, warnings) = import(&paragraph_fig(shared, reset), "Synthetic paragraph resets").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let text = cleared.nodes().find(|node| node.name == "Label").unwrap().kind.text().unwrap();
    assert_eq!(text.list_spacing, 0.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::None);
    assert_eq!(text.style.font_family, "Inter");

    let bytes = crate::save_bytes(&inherited, None).unwrap();
    let (back, warnings) = crate::load_bytes(&bytes).unwrap();
    assert!(warnings.is_empty(), "native round trip: {warnings:?}");
    let text = back.nodes().find(|node| node.name == "Label").unwrap().kind.text().unwrap();
    assert_eq!(text.list_spacing, 18.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::CapHeight);

    let (_, export_warnings) = export(&inherited, None).unwrap();
    assert!(
        export_warnings.iter().any(|warning| warning.contains("list spacing, hanging punctuation, leading trim")),
        "existing .fig export loss must remain visible: {export_warnings:?}"
    );
}

fn instance_paragraph_fig(overrides: Vec<(&'static str, Kv<'static>)>, nonfinite_spacing: bool) -> Vec<u8> {
    use super::kiwi::{Schema, encode};

    let schema = Schema::parse(
        "enum NodeType NONE DOCUMENT CANVAS TEXT SYMBOL INSTANCE\n\
         enum StyleType NONE FILL TEXT\n\
         enum LeadingTrim NONE CAP_HEIGHT\n\
         struct GUID uint sessionID, uint localID\n\
         struct ParentIndex GUID guid, string position\n\
         struct FontName string family, string style, string postscript\n\
         struct StyleId GUID guid\n\
         struct GUIDPath GUID[] guids\n\
         struct SymbolOverride GUIDPath guidPath, float listSpacing, bool hangingList, bool hangingPunctuation, LeadingTrim leadingTrim\n\
         struct SymbolData GUID symbolID, SymbolOverride[] symbolOverrides\n\
         message TextData string characters\n\
         message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, StyleType styleType, FontName fontName, float fontSize, StyleId styleIdForText, TextData textData, float listSpacing, bool hangingList, bool hangingPunctuation, LeadingTrim leadingTrim, SymbolData symbolData\n\
         message Message NodeChange[] nodeChanges",
    )
    .unwrap();
    fn object(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
        Kv::Object(fields)
    }
    fn guid(id: u64) -> Kv<'static> {
        object(vec![("sessionID", Kv::Uint(7)), ("localID", Kv::Uint(id))])
    }
    fn node(id: u64, ty: &'static str, name: &str, parent: Option<u64>, fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
        let mut out = vec![("guid", guid(id)), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
        if let Some(parent) = parent {
            out.push(("parentIndex", object(vec![("guid", guid(parent)), ("position", Kv::Str("a".into()))])));
        }
        out.extend(fields);
        object(out)
    }
    fn font() -> Kv<'static> {
        object(vec![("family", Kv::Str("Inter".into())), ("style", Kv::Str("Regular".into())), ("postscript", Kv::Str(String::new()))])
    }
    let main_text = node(
        21,
        "TEXT",
        "Label",
        Some(20),
        vec![
            ("fontName", font()),
            ("fontSize", Kv::Float(20.0)),
            ("styleIdForText", object(vec![("guid", guid(10))])),
            ("textData", object(vec![("characters", Kv::Str("a\nb".into()))])),
        ],
    );
    let mut override_fields = vec![("guidPath", object(vec![("guids", Kv::Array(vec![guid(21)]))]))];
    override_fields.extend(overrides);
    let instance = node(
        30,
        "INSTANCE",
        "Copy",
        Some(2),
        vec![("symbolData", object(vec![("symbolID", guid(20)), ("symbolOverrides", Kv::Array(vec![object(override_fields)]))]))],
    );
    let message = object(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, "DOCUMENT", "Document", None, vec![]),
            node(2, "CANVAS", "Page", Some(1), vec![]),
            node(
                10,
                "TEXT",
                "Shared",
                Some(2),
                vec![
                    ("styleType", Kv::Enum("TEXT")),
                    ("fontName", font()),
                    ("fontSize", Kv::Float(20.0)),
                    ("listSpacing", Kv::Float(18.0)),
                    ("hangingList", Kv::Bool(true)),
                    ("hangingPunctuation", Kv::Bool(true)),
                    ("leadingTrim", Kv::Enum("CAP_HEIGHT")),
                ],
            ),
            node(20, "SYMBOL", "Main", Some(2), vec![]),
            main_text,
            instance,
        ]),
    )]);
    let root = schema.find("Message").unwrap();
    let mut deflated = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write;
    deflated.write_all(&encode::schema(&schema)).unwrap();
    let schema_chunk = deflated.finish().unwrap();
    let mut message = encode::message(&schema, root, &message).unwrap();
    if nonfinite_spacing {
        let finite = 37.0_f32.to_bits().rotate_right(23).to_le_bytes();
        let nan = f32::NAN.to_bits().rotate_right(23).to_le_bytes();
        let matches: Vec<usize> = message.windows(finite.len()).enumerate().filter_map(|(i, w)| (w == finite).then_some(i)).collect();
        assert_eq!(matches.len(), 1, "finite test token must identify the instance listSpacing field");
        let offset = matches[0];
        message[offset..offset + nan.len()].copy_from_slice(&nan);
    }
    let message_chunk = ruzstd::encoding::compress_to_vec(message.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
    let mut bytes = b"fig-kiwi".to_vec();
    bytes.extend_from_slice(&101u32.to_le_bytes());
    for chunk in [schema_chunk, message_chunk] {
        bytes.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

#[test]
fn encoded_instance_paragraph_overrides_fall_back_only_when_invalid() {
    let valid = vec![
        ("listSpacing", Kv::Float(24.0)),
        ("hangingList", Kv::Bool(false)),
        ("hangingPunctuation", Kv::Bool(false)),
        ("leadingTrim", Kv::Enum("NONE")),
    ];
    let (doc, warnings) = import(&instance_paragraph_fig(valid, false), "Synthetic instance paragraph values").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let copy = doc.nodes().find(|node| node.name == "Copy").unwrap();
    let label = doc.node(*copy.children.first().unwrap()).unwrap();
    let text = label.kind.text().unwrap();
    assert_eq!(text.list_spacing, 24.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::None);

    let reset = vec![
        ("listSpacing", Kv::Float(0.0)),
        ("hangingList", Kv::Bool(false)),
        ("hangingPunctuation", Kv::Bool(false)),
        ("leadingTrim", Kv::Enum("NONE")),
    ];
    let (doc, warnings) = import(&instance_paragraph_fig(reset, false), "Synthetic instance paragraph resets").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let copy = doc.nodes().find(|node| node.name == "Copy").unwrap();
    let label = doc.node(*copy.children.first().unwrap()).unwrap();
    let text = label.kind.text().unwrap();
    assert_eq!(text.list_spacing, 0.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::None);

    let invalid = vec![
        ("listSpacing", Kv::Float(37.0)),
        ("hangingList", Kv::Bool(false)),
        ("hangingPunctuation", Kv::Bool(false)),
        ("leadingTrim", Kv::Uint(77)),
    ];
    let (doc, warnings) = import(&instance_paragraph_fig(invalid, true), "Synthetic malformed instance paragraph values").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let copy = doc.nodes().find(|node| node.name == "Copy").unwrap();
    let label = doc.node(*copy.children.first().unwrap()).unwrap();
    let text = label.kind.text().unwrap();
    assert_eq!(text.list_spacing, 18.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::CapHeight);

    let bytes = crate::save_bytes(&doc, None).unwrap();
    let (back, warnings) = crate::load_bytes(&bytes).unwrap();
    assert!(warnings.is_empty(), "native round trip: {warnings:?}");
    let copy = back.nodes().find(|node| node.name == "Copy").unwrap();
    let label = back.node(*copy.children.first().unwrap()).unwrap();
    let text = label.kind.text().unwrap();
    assert_eq!(text.list_spacing, 18.0);
    assert!(!text.hanging_list);
    assert!(!text.hanging_punctuation);
    assert_eq!(text.leading_trim, figcraft_doc::LeadingTrim::CapHeight);
}
