//! Compact, self-contained portable geometry. References never escape an appearance, so a
//! text node can still travel alone in a collaboration update or clipboard without a side table.
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{TextAppearance, TextDecorationPath, TextGlyph, TextLine, TextStyle};

const MAX_GLYPHS: usize = 100_000;
const MAX_PATH_BYTES: usize = 16 * 1024 * 1024;

/// Ordinary text needs only its scale: x/y are already present. Warped/path text retains the
/// complete affine matrix. Null is the old representation's already-placed path.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Placement {
    Scale(f64),
    Matrix([f64; 6]),
}

pub(crate) type CompactGlyph = (usize, f64, f64, f64, f64, usize, usize, usize, bool, Option<Placement>);

#[derive(Serialize)]
struct Saved<'a> {
    encoding: u8,
    fingerprint: &'a str,
    box_width: f64,
    box_height: f64,
    width: f64,
    height: f64,
    outlines: Vec<&'a str>,
    glyphs: String,
    #[serde(serialize_with = "crate::text_appearance_meta::lines")]
    lines: &'a [TextLine],
    decorations: &'a [TextDecorationPath],
    #[serde(serialize_with = "crate::text_appearance_meta::styles")]
    styles: &'a [TextStyle],
    truncated: bool,
}

impl Serialize for TextAppearance {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut indices = HashMap::new();
        let mut outlines = Vec::new();
        let glyphs = self
            .glyphs
            .iter()
            .map(|g| {
                let path = self.glyphs.serialized_outline(&g.d);
                let next = outlines.len();
                let index = *indices.entry(path).or_insert_with(|| {
                    outlines.push(path);
                    next
                });
                let placement = g.transform.map(|m| {
                    if m.map(f64::to_bits) == [m[0], 0.0, 0.0, m[0], g.x, g.y].map(f64::to_bits) {
                        Placement::Scale(m[0])
                    } else {
                        Placement::Matrix(m)
                    }
                });
                (index, g.x, g.y, g.size, g.advance, g.char_start, g.char_end, g.style, g.rtl, placement)
            })
            .collect::<Vec<_>>();
        Saved {
            encoding: 3,
            fingerprint: &self.fingerprint,
            box_width: self.box_width,
            box_height: self.box_height,
            width: self.width,
            height: self.height,
            outlines,
            glyphs: crate::text_glyph_bytes::encode(&glyphs).map_err(serde::ser::Error::custom)?,
            lines: &self.lines,
            decorations: &self.decorations,
            styles: &self.styles,
            truncated: self.truncated,
        }
        .serialize(serializer)
    }
}

enum ReadGlyph {
    Compact(CompactGlyph),
    Legacy(TextGlyph),
}

// Dispatch on sequence/map directly. An untagged enum buffers every number into serde's
// generic Content tree; a million positioned glyphs should be streamed instead.
impl<'de> Deserialize<'de> for ReadGlyph {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Glyph;
        impl<'de> serde::de::Visitor<'de> for Glyph {
            type Value = ReadGlyph;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a compact glyph tuple or a legacy glyph object")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                CompactGlyph::deserialize(serde::de::value::SeqAccessDeserializer::new(seq)).map(ReadGlyph::Compact)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                TextGlyph::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(ReadGlyph::Legacy)
            }
        }
        d.deserialize_any(Glyph)
    }
}

#[derive(Deserialize)]
struct Read {
    #[serde(default)]
    encoding: u8,
    fingerprint: String,
    box_width: f64,
    box_height: f64,
    width: f64,
    height: f64,
    #[serde(default, deserialize_with = "read_outlines")]
    outlines: Vec<Arc<str>>,
    #[serde(deserialize_with = "read_glyphs")]
    glyphs: ReadGlyphs,
    lines: Vec<TextLine>,
    decorations: Vec<TextDecorationPath>,
    styles: Vec<TextStyle>,
    truncated: bool,
}

/// Even unused dictionary entries count toward the budget; otherwise an untrusted file could
/// allocate a large pool while referring only to a tiny valid entry.
fn read_outlines<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Arc<str>>, D::Error> {
    // Serde's generic Arc<str> path goes through String/Box<str>. Read directly into the
    // shared string instead of allocating and copying a temporary string per palette entry.
    struct Outline(Arc<str>);
    impl<'de> Deserialize<'de> for Outline {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Path;
            impl serde::de::Visitor<'_> for Path {
                type Value = Outline;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a bounded outline string")
                }
                fn visit_str<E: serde::de::Error>(self, path: &str) -> Result<Outline, E> {
                    if path.len() > MAX_PATH_BYTES {
                        return Err(E::custom("portable outline dictionary exceeds limit"));
                    }
                    Ok(Outline(Arc::from(path)))
                }
            }
            d.deserialize_str(Path)
        }
    }
    struct Outlines;
    impl<'de> serde::de::Visitor<'de> for Outlines {
        type Value = Vec<Arc<str>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded portable outlines")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut paths = Vec::with_capacity(seq.size_hint().unwrap_or_default().min(MAX_GLYPHS));
            let mut bytes = 0usize;
            while let Some(Outline(path)) = seq.next_element::<Outline>()? {
                bytes = bytes.saturating_add(path.len());
                if paths.len() >= MAX_GLYPHS || bytes > MAX_PATH_BYTES {
                    return Err(serde::de::Error::custom("portable outline dictionary exceeds limit"));
                }
                paths.push(path);
            }
            Ok(paths)
        }
    }
    d.deserialize_seq(Outlines)
}

enum ReadGlyphs {
    Records(Vec<ReadGlyph>),
    Packed(String),
}

/// Refuse the next record before growing a vector past the display/capture limit. Repeated
/// references are also charged their expanded bytes below, bounding restoration work.
fn read_glyphs<'de, D: Deserializer<'de>>(d: D) -> Result<ReadGlyphs, D::Error> {
    struct Glyphs;
    impl<'de> serde::de::Visitor<'de> for Glyphs {
        type Value = ReadGlyphs;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded portable glyphs")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut glyphs = Vec::new();
            while let Some(glyph) = seq.next_element()? {
                if glyphs.len() >= MAX_GLYPHS {
                    return Err(serde::de::Error::custom("too many portable glyphs"));
                }
                glyphs.push(glyph);
            }
            Ok(ReadGlyphs::Records(glyphs))
        }
        fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
            if text.len() > crate::text_glyph_bytes::MAX_ENCODED {
                return Err(E::custom("portable glyph bytes exceed limit"));
            }
            Ok(ReadGlyphs::Packed(text.to_owned()))
        }
    }
    d.deserialize_any(Glyphs)
}

fn compact_glyph(c: CompactGlyph, outlines: &[Arc<str>]) -> Result<TextGlyph, &'static str> {
    let (index, x, y, size, advance, char_start, char_end, style, rtl, placement) = c;
    let path = outlines.get(index).ok_or("missing portable outline")?;
    let transform = placement.map(|p| match p {
        Placement::Scale(s) => [s, 0.0, 0.0, s, x, y],
        Placement::Matrix(m) => m,
    });
    Ok(TextGlyph { d: path.clone(), transform, x, y, size, advance, char_start, char_end, style, rtl })
}

impl<'de> Deserialize<'de> for TextAppearance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = Read::deserialize(d)?;
        if wire.encoding > 3 || wire.outlines.len() > MAX_GLYPHS {
            return Err(serde::de::Error::custom("unsupported portable geometry encoding or outline count"));
        }
        let mut bytes = 0usize;
        let mut finish = |glyph: TextGlyph| {
            bytes = bytes.saturating_add(glyph.d.len());
            if bytes > MAX_PATH_BYTES {
                return Err("portable outlines exceed byte limit");
            }
            Ok(glyph)
        };
        let glyphs = match wire.glyphs {
            ReadGlyphs::Records(records) if wire.encoding < 2 => {
                let mut glyphs = Vec::with_capacity(records.len());
                for record in records {
                    let glyph = match record {
                        ReadGlyph::Legacy(g) if wire.encoding == 0 => g,
                        ReadGlyph::Compact(c) if wire.encoding == 1 => compact_glyph(c, &wire.outlines).map_err(serde::de::Error::custom)?,
                        _ => return Err(serde::de::Error::custom("glyph does not match portable geometry encoding")),
                    };
                    glyphs.push(finish(glyph).map_err(serde::de::Error::custom)?);
                }
                glyphs
            }
            ReadGlyphs::Packed(text) if matches!(wire.encoding, 2 | 3) => {
                crate::text_glyph_bytes::decode_map(&text, wire.encoding == 3, |c| finish(compact_glyph(c, &wire.outlines)?))
                    .map_err(serde::de::Error::custom)?
            }
            _ => return Err(serde::de::Error::custom("glyph data does not match portable encoding")),
        };
        Ok(Self {
            fingerprint: wire.fingerprint,
            box_width: wire.box_width,
            box_height: wire.box_height,
            width: wire.width,
            height: wire.height,
            glyphs: crate::TextGlyphs::decoded(glyphs, if wire.encoding == 0 { bytes } else { wire.outlines.iter().map(|p| p.len()).sum() }),
            lines: wire.lines,
            decorations: wire.decorations,
            styles: wire.styles,
            truncated: wire.truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn appearance() -> TextAppearance {
        let path: Arc<str> = "M0 0L10 0L10 10Z".repeat(20).into();
        TextAppearance {
            fingerprint: "synthetic".into(),
            box_width: 100.0,
            box_height: 20.0,
            width: 100.0,
            height: 20.0,
            glyphs: (0..1000)
                .map(|i| TextGlyph {
                    d: path.clone(),
                    transform: Some([0.125, 0.0, 0.0, 0.125, i as f64, 15.0]),
                    x: i as f64,
                    y: 15.0,
                    size: 12.0,
                    advance: 5.0,
                    char_start: i,
                    char_end: i + 1,
                    style: 0,
                    rtl: false,
                })
                .collect::<Vec<_>>()
                .into(),
            lines: vec![],
            decorations: vec![],
            styles: vec![TextStyle::default()],
            truncated: false,
        }
    }

    #[test]
    fn packed_model_construction_rejects_missing_paths_and_expansion() {
        let mut wire = serde_json::to_value(appearance()).unwrap();
        wire["outlines"] = json!([]);
        assert!(serde_json::from_value::<TextAppearance>(wire.clone()).unwrap_err().to_string().contains("missing portable outline"));
        wire["outlines"] = json!(["M0 0L10 0L10 10Z".repeat(1200)]);
        assert!(serde_json::from_value::<TextAppearance>(wire).unwrap_err().to_string().contains("portable outlines exceed byte limit"));
    }

    #[test]
    fn compact_scale_does_not_erase_signed_zero_affine_components() {
        let mut a = appearance();
        a.glyphs.truncate(1);
        let matrix = [0.125, -0.0, 0.0, 0.125, -0.0, 15.0];
        a.glyphs[0].transform = Some(matrix);
        let back: TextAppearance = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(back.glyphs[0].transform.unwrap().map(f64::to_bits), matrix.map(f64::to_bits));
    }

    #[test]
    fn compressed_geometry_has_a_document_wide_memory_budget() {
        use crate::{Document, NodeKind, TextData};
        let mut doc = Document::new("Geometry budget");
        let page = doc.first_page().unwrap();
        let mut text = TextData::new("Synthetic");
        let a = appearance();
        let one = a.glyphs.len() * std::mem::size_of::<TextGlyph>() + a.glyphs[0].d.len();
        text.appearance = Some(Arc::new(a));
        for _ in 0..2 {
            doc.create(page, NodeKind::Text(text.clone()), "Text", 0.0, 0.0, 100.0, 20.0).unwrap();
        }
        let encoded = serde_json::to_string(&doc.nodes).unwrap();
        let mut d = serde_json::Deserializer::from_str(&encoded);
        let back = crate::deserialize_nodes_limited(&mut d, one * 2).unwrap();
        assert_eq!(back, doc.nodes);
        let mut d = serde_json::Deserializer::from_str(&encoded);
        let error = crate::deserialize_nodes_limited(&mut d, one * 2 - 1).unwrap_err();
        assert!(error.to_string().contains("portable geometry exceeds memory limit"));
    }

    #[test]
    fn appearance_encoding_two_remains_readable() {
        let mut a = appearance();
        a.glyphs.truncate(1);
        let mut wire = serde_json::to_value(&a).unwrap();
        wire["encoding"] = json!(2);
        // Encoding-two's single glyph columns have the original, unpredicted IEEE bytes.
        use base64::Engine as _;
        let mut bytes = 1u32.to_le_bytes().to_vec();
        for n in [0u32, 0, 1, 0] {
            bytes.extend(n.to_le_bytes());
        }
        for n in [0.0f64, 15.0, 12.0, 5.0] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend([0, 1]);
        bytes.extend(0.125f64.to_le_bytes());
        wire["glyphs"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        let read: TextAppearance = serde_json::from_value(wire).unwrap();
        assert_eq!(read, a);
    }

    #[test]
    fn grouping_serialized_nodes_preserves_layer_order_and_round_trip() {
        use crate::{Document, NodeId, NodeKind, TextData};
        #[derive(Deserialize)]
        struct Order {
            nodes: Keys,
        }
        struct Keys(Vec<NodeId>);
        impl<'de> Deserialize<'de> for Keys {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = Keys;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("nodes map")
                    }
                    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Keys, M::Error> {
                        let mut keys = Vec::new();
                        while let Some((k, _)) = map.next_entry::<NodeId, serde::de::IgnoredAny>()? {
                            keys.push(k);
                        }
                        Ok(Keys(keys))
                    }
                }
                d.deserialize_map(V)
            }
        }
        let mut doc = Document::new("Synthetic grouping");
        let page = doc.first_page().unwrap();
        let mut ids = Vec::new();
        for size in [20.0, 12.0, 20.0, 12.0] {
            let mut text = TextData::new("Grouped");
            text.style.font_size = size;
            text.appearance = Some(Arc::new(appearance()));
            ids.push(doc.create(page, NodeKind::Text(text), "Text", 0.0, 0.0, 100.0, 20.0).unwrap());
        }
        let encoded = serde_json::to_string(&doc).unwrap();
        let order: Order = serde_json::from_str(&encoded).unwrap();
        assert_eq!(order.nodes.0, [page, ids[1], ids[3], ids[0], ids[2]]);
        assert_eq!(encoded, serde_json::to_string(&doc).unwrap());
        let back: Document = serde_json::from_str(&encoded).unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.node(page).unwrap().children, ids);
        // A malformed node's embedded id must never replace its map key during encoding.
        doc.node_mut(ids[0]).unwrap().id = NodeId(900);
        let malformed: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(malformed, doc);
        for id in ids {
            if let NodeKind::Text(t) = &mut doc.node_mut(id).unwrap().kind {
                t.appearance = None;
            }
        }
        let order: Order = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(order.nodes.0, doc.ids().collect::<Vec<_>>());
    }

    #[test]
    fn repeated_paths_are_written_once_and_read_as_shared_exact_geometry() {
        let a = appearance();
        let encoded = serde_json::to_value(&a).unwrap();
        assert_eq!(encoded["outlines"].as_array().unwrap().len(), 1);
        assert!(encoded["glyphs"].is_string());
        let back: TextAppearance = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(back, a);
        assert!(Arc::ptr_eq(&back.glyphs[0].d, &back.glyphs[999].d));
        let mut legacy = encoded;
        legacy.as_object_mut().unwrap().remove("encoding");
        legacy.as_object_mut().unwrap().remove("outlines");
        legacy["glyphs"] = serde_json::to_value(&a.glyphs).unwrap();
        assert!(serde_json::to_vec(&a).unwrap().len() * 4 < serde_json::to_vec(&legacy).unwrap().len());
    }

    #[test]
    fn legacy_placed_paths_and_full_affines_round_trip_without_quantization() {
        let mut a = appearance();
        a.glyphs.truncate(2);
        a.glyphs[0].transform = None;
        a.glyphs[1].transform = Some([0.125, 0.2, -0.2, 0.125, 3.123456789012345, -8.5]);
        let mut old = serde_json::to_value(&a).unwrap();
        old.as_object_mut().unwrap().remove("encoding");
        old.as_object_mut().unwrap().remove("outlines");
        old["glyphs"] = serde_json::to_value(&a.glyphs).unwrap();
        let read: TextAppearance = serde_json::from_value(old).unwrap();
        assert_eq!(read, a);
        let back: TextAppearance = serde_json::from_slice(&serde_json::to_vec(&read).unwrap()).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn invalid_references_versions_and_expansion_are_refused() {
        let mut wire = serde_json::to_value(appearance()).unwrap();
        wire["encoding"] = json!(1);
        wire["glyphs"] = json!([[100, 0.0, 0.0, 12.0, 5.0, 0, 1, 0, false, null]]);
        assert!(serde_json::from_value::<TextAppearance>(wire.clone()).is_err());
        wire["glyphs"][0][0] = json!(0);
        wire["encoding"] = json!(4);
        assert!(serde_json::from_value::<TextAppearance>(wire.clone()).is_err());
        wire["encoding"] = json!(1);
        wire["outlines"] = json!(["x".repeat(MAX_PATH_BYTES / 2 + 1)]);
        wire["glyphs"] = json!(vec![json!([0, 0.0, 0.0, 12.0, 5.0, 0, 1, 0, false, null]); 3]);
        assert!(serde_json::from_value::<TextAppearance>(wire).is_err());
    }

    #[test]
    fn unused_pool_bytes_and_glyph_counts_are_bounded() {
        let mut wire = serde_json::to_value(appearance()).unwrap();
        wire["outlines"] = json!(["", "x".repeat(MAX_PATH_BYTES + 1)]);
        assert!(serde_json::from_value::<TextAppearance>(wire).is_err());
        let glyphs = format!("[{}]", vec!["[0,0,0,12,5,0,1,0,false,null]"; MAX_GLYPHS + 1].join(","));
        let mut d = serde_json::Deserializer::from_str(&glyphs);
        assert!(read_glyphs(&mut d).is_err());
    }
}
