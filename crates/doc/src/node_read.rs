//! Read the large appearance payload directly when a text's type precedes it (as our
//! writer emits it). The ordinary flattened Node reader still handles every other field
//! and arbitrary field order; no unbounded intermediate tree is added here.
use std::{borrow::Cow, sync::Arc};

use serde::{
    Deserialize, Deserializer,
    de::{
        DeserializeSeed, MapAccess, Visitor,
        value::{BorrowedStrDeserializer, MapAccessDeserializer, StringDeserializer},
    },
};

use crate::{Node, NodeKind, TextAppearance};

pub(crate) struct ReadNode(pub(crate) Arc<Node>);
impl<'de> Deserialize<'de> for ReadNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Payload;
        impl<'de> Visitor<'de> for Payload {
            type Value = ReadNode;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a node")
            }
            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let mut appearance = None;
                let mut node = Node::deserialize(MapAccessDeserializer::new(Fields {
                    map,
                    appearance: &mut appearance,
                    text: false,
                    reading_type: false,
                    seen: false,
                }))?;
                if let Some(saved) = appearance
                    && let NodeKind::Text(text) = &mut node.kind
                {
                    text.appearance = saved;
                }
                Ok(ReadNode(Arc::new(node)))
            }
        }
        d.deserialize_map(Payload)
    }
}

struct Fields<'a, M> {
    map: M,
    appearance: &'a mut Option<Option<Arc<TextAppearance>>>,
    text: bool,
    reading_type: bool,
    seen: bool,
}
impl<'de, M: MapAccess<'de>> MapAccess<'de> for Fields<'_, M> {
    type Error = M::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        loop {
            let Some(key) = self.map.next_key::<Key<'de>>()? else { return Ok(None) };
            if key.0 == "appearance" {
                if self.text {
                    // An earlier appearance may already be in the flattened reader's buffer.
                    // Reject duplicates (including null) rather than silently replacing it.
                    if self.seen {
                        return Err(serde::de::Error::duplicate_field("appearance"));
                    }
                    self.seen = true;
                    *self.appearance = Some(self.map.next_value()?);
                    continue;
                }
                self.seen = true;
            }
            self.reading_type = key.0 == "type";
            return key.forward(seed).map(Some);
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        if self.reading_type {
            let kind = self.map.next_value::<Key<'de>>()?;
            self.text = kind.0 == "text";
            self.reading_type = false;
            kind.forward(seed)
        } else {
            self.map.next_value_seed(seed)
        }
    }
}

// Borrow unescaped JSON identifiers; escaped/from-value keys remain owned. Forwarding
// preserves the original key and the standard Node reader's unknown-field behavior.
struct Key<'a>(Cow<'a, str>);
impl<'de> Key<'de> {
    fn forward<S: DeserializeSeed<'de>, E: serde::de::Error>(self, seed: S) -> Result<S::Value, E> {
        match self.0 {
            Cow::Borrowed(s) => seed.deserialize(BorrowedStrDeserializer::<E>::new(s)),
            Cow::Owned(s) => seed.deserialize(StringDeserializer::<E>::new(s)),
        }
    }
}
impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Name;
        impl<'de> Visitor<'de> for Name {
            type Value = Key<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }
            fn visit_borrowed_str<E: serde::de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Borrowed(v)))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(v.into())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(Key(Cow::Owned(v)))
            }
        }
        d.deserialize_str(Name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, TextData, TextGlyph};

    fn text_node() -> Node {
        let mut text = TextData::new("A".repeat(117));
        text.appearance = Some(Arc::new(TextAppearance {
            fingerprint: "unchanged precise source".into(),
            box_width: 600.0,
            box_height: 80.0,
            width: 117.0 * 7.1250000000123,
            height: 30.123456789012345,
            glyphs: (0..117)
                .map(|i| TextGlyph {
                    d: format!("@{}", i % 32).into(),
                    x: i as f64 * 7.1250000000123,
                    y: -0.0,
                    size: 14.0,
                    advance: 7.1250000000123,
                    char_start: i,
                    char_end: i + 1,
                    style: 0,
                    rtl: false,
                    transform: Some([0.125, -0.0, 0.03125, 0.125, i as f64 * 7.1250000000123, -0.0]),
                })
                .collect::<Vec<_>>()
                .into(),
            lines: vec![],
            decorations: vec![],
            styles: vec![text.style.clone()],
            truncated: false,
        }));
        Node::new(NodeId(2), NodeKind::Text(text), "Exact text", 600.0, 80.0)
    }

    fn compare(input: &str) {
        let expected: Node = serde_json::from_str(input).unwrap();
        let back: ReadNode = serde_json::from_str(input).unwrap();
        assert_eq!(serde_json::to_string(&*back.0).unwrap(), serde_json::to_string(&expected).unwrap());
        let from_value: ReadNode = serde_json::from_value(serde_json::from_str::<serde_json::Value>(input).unwrap()).unwrap();
        assert_eq!(serde_json::to_string(&*from_value.0).unwrap(), serde_json::to_string(&expected).unwrap());
    }

    #[test]
    fn direct_and_fallback_reads_preserve_exact_values_and_escaped_keys() {
        let node = text_node();
        let input = serde_json::to_string(&node).unwrap();
        compare(&input);
        compare(&input.replace("\"type\"", r#""t\u0079pe""#).replace("\"appearance\"", r#""app\u0065arance""#));
        let mut fields = serde_json::to_value(&node).unwrap().as_object().unwrap().clone();
        let appearance = fields.remove("appearance").unwrap();
        let body = serde_json::to_string(&fields).unwrap();
        // The input is valid regardless of whether the tag is known at the appearance key.
        compare(&format!("{{\"appearance\":{appearance},{}", body.strip_prefix('{').unwrap()));
        compare(&format!("{},\"appearance\":{appearance}}}", body.strip_suffix('}').unwrap()));
        let back = serde_json::from_str::<ReadNode>(&input).unwrap();
        let glyph = &back.0.kind.text().unwrap().appearance.as_ref().unwrap().glyphs[0];
        assert_eq!(glyph.y.to_bits(), (-0.0f64).to_bits());
        assert_eq!(glyph.transform.unwrap()[1].to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn duplicates_null_unknown_fields_and_nontext_keep_standard_behavior() {
        let mut text = text_node();
        text.kind.text_mut().unwrap().appearance = None;
        let input = serde_json::to_string(&text).unwrap();
        compare(&input);
        compare(&format!("{},\"appearance\":null}}", input.strip_suffix('}').unwrap()));
        for fields in [
            "\"type\":\"text\",\"appearance\":null,\"appearance\":null",
            "\"appearance\":null,\"type\":\"text\",\"appearance\":null",
            "\"appearance\":null,\"appearance\":null,\"type\":\"text\"",
        ] {
            let input = input.replace("\"type\":\"text\"", fields);
            assert!(serde_json::from_str::<Node>(&input).is_err());
            assert!(serde_json::from_str::<ReadNode>(&input).is_err());
        }
        let rectangle = serde_json::to_string(&Node::new(NodeId(3), NodeKind::Rectangle, "Shape", 10.0, 20.0)).unwrap();
        // A non-text node ignores unknown appearance payloads rather than parsing them.
        compare(&format!("{},\"appearance\":{{\"not\":\"an appearance\"}},\"extra\":true}}", rectangle.strip_suffix('}').unwrap()));
    }

    #[test]
    #[ignore = "release timing: cargo test -p figcraft-doc --release portable_node_read_timings -- --ignored --nocapture"]
    fn portable_node_read_timings() {
        use std::{hint::black_box, time::Instant};
        for portable in [false, true] {
            let mut node = text_node();
            if !portable {
                node.kind.text_mut().unwrap().appearance = None;
            }
            let wire = serde_json::to_string(&node).unwrap();
            let mut old = vec![];
            let mut direct = vec![];
            for sample in 0..6 {
                let start = Instant::now();
                for _ in 0..10_000 {
                    black_box(serde_json::from_str::<Arc<Node>>(&wire).unwrap());
                }
                let before = start.elapsed().as_secs_f64() * 1000.0;
                let start = Instant::now();
                for _ in 0..10_000 {
                    black_box(serde_json::from_str::<ReadNode>(&wire).unwrap());
                }
                let after = start.elapsed().as_secs_f64() * 1000.0;
                if sample > 0 {
                    old.push(before);
                    direct.push(after);
                }
            }
            old.sort_by(f64::total_cmp);
            direct.sort_by(f64::total_cmp);
            println!("PORTABLE_NODE portable={portable} layers=10000 old_ms={:.3} direct_ms={:.3}", old[2], direct[2]);
        }
    }
}
