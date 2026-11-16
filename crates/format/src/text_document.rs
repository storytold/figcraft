//! Stream the app-specific outline table alongside the ordinary document payload.
//! MapAccessDeserializer forwards values directly; flatten/Value buffering would allocate
//! the whole node tree before its decoded-memory guards could run.
use figcraft_doc::Document;
use serde::de::{
    DeserializeSeed, MapAccess, Visitor,
    value::{MapAccessDeserializer, StringDeserializer},
};
use serde::{Deserialize, Deserializer};

pub(crate) struct ReadDocument {
    pub(crate) doc: Document,
    pub(crate) outlines: Option<crate::text_outlines::Pool>,
}

impl<'de> Deserialize<'de> for ReadDocument {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Payload;
        impl<'de> Visitor<'de> for Payload {
            type Value = ReadDocument;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a document with an optional native outline table")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let mut outlines = None;
                let doc = Document::deserialize(MapAccessDeserializer::new(Fields { map, outlines: &mut outlines, seen: false }))?;
                Ok(ReadDocument { doc, outlines })
            }
        }
        d.deserialize_map(Payload)
    }
}

struct Fields<'a, A> {
    map: A,
    outlines: &'a mut Option<crate::text_outlines::Pool>,
    seen: bool,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Fields<'_, A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        loop {
            let Some(key) = self.map.next_key::<String>()? else { return Ok(None) };
            if key == "textOutlines" {
                // Match the previous derived table reader, including duplicate null entries.
                if self.seen {
                    return Err(serde::de::Error::duplicate_field("textOutlines"));
                }
                self.seen = true;
                *self.outlines = self.map.next_value()?;
            } else {
                return seed.deserialize(StringDeserializer::<A::Error>::new(key)).map(Some);
            }
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        self.map.next_value_seed(seed)
    }
}
