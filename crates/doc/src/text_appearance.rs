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
    lines: &'a [TextLine],
    decorations: &'a [TextDecorationPath],
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
                let next = outlines.len();
                let index = *indices.entry(g.d.as_ref()).or_insert_with(|| {
                    outlines.push(g.d.as_ref());
                    next
                });
                let placement = g.transform.map(|m| {
                    if m[0] == m[3] && m[1] == 0.0 && m[2] == 0.0 && m[4] == g.x && m[5] == g.y {
                        Placement::Scale(m[0])
                    } else {
                        Placement::Matrix(m)
                    }
                });
                (index, g.x, g.y, g.size, g.advance, g.char_start, g.char_end, g.style, g.rtl, placement)
            })
            .collect::<Vec<_>>();
        Saved {
            encoding: 2,
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
    struct Outlines;
    impl<'de> serde::de::Visitor<'de> for Outlines {
        type Value = Vec<Arc<str>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded portable outlines")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut paths = Vec::new();
            let mut bytes = 0usize;
            while let Some(path) = seq.next_element::<Arc<str>>()? {
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

impl<'de> Deserialize<'de> for TextAppearance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = Read::deserialize(d)?;
        if wire.encoding > 2 || wire.outlines.len() > MAX_GLYPHS {
            return Err(serde::de::Error::custom("unsupported portable geometry encoding or outline count"));
        }
        let records = match wire.glyphs {
            ReadGlyphs::Records(records) if wire.encoding < 2 => records,
            ReadGlyphs::Packed(text) if wire.encoding == 2 => {
                crate::text_glyph_bytes::decode(&text).map_err(serde::de::Error::custom)?.into_iter().map(ReadGlyph::Compact).collect()
            }
            _ => return Err(serde::de::Error::custom("glyph data does not match portable encoding")),
        };
        let mut bytes = 0usize;
        let glyphs = records
            .into_iter()
            .map(|g| {
                let g = match g {
                    ReadGlyph::Legacy(g) if wire.encoding == 0 => g,
                    ReadGlyph::Compact((index, x, y, size, advance, char_start, char_end, style, rtl, placement)) if wire.encoding >= 1 => {
                        let path = wire.outlines.get(index).ok_or_else(|| serde::de::Error::custom("missing portable outline"))?;
                        let transform = placement.map(|p| match p {
                            Placement::Scale(s) => [s, 0.0, 0.0, s, x, y],
                            Placement::Matrix(m) => m,
                        });
                        TextGlyph { d: path.clone(), transform, x, y, size, advance, char_start, char_end, style, rtl }
                    }
                    _ => return Err(serde::de::Error::custom("glyph does not match portable geometry encoding")),
                };
                bytes = bytes.saturating_add(g.d.len());
                if bytes > MAX_PATH_BYTES {
                    return Err(serde::de::Error::custom("portable outlines exceed byte limit"));
                }
                Ok(g)
            })
            .collect::<Result<Vec<_>, D::Error>>()?;
        Ok(Self {
            fingerprint: wire.fingerprint,
            box_width: wire.box_width,
            box_height: wire.box_height,
            width: wire.width,
            height: wire.height,
            glyphs,
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
                .collect(),
            lines: vec![],
            decorations: vec![],
            styles: vec![TextStyle::default()],
            truncated: false,
        }
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
        wire["encoding"] = json!(3);
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
