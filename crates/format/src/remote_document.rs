//! Decode the app-specific reference index with the shared native document envelope.
//! Forward typed document values directly so its bounded reader remains authoritative.
use figcraft_doc::Document;
use serde::{
    Deserialize, Deserializer,
    de::{
        DeserializeSeed, MapAccess, Visitor,
        value::{MapAccessDeserializer, StringDeserializer},
    },
};
use std::collections::BTreeMap;

pub(crate) struct RemoteDocument {
    pub(crate) doc: Document,
    pub(crate) remote: BTreeMap<String, super::RemoteImage>,
}
impl<'de> Deserialize<'de> for RemoteDocument {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Payload;
        impl<'de> Visitor<'de> for Payload {
            type Value = RemoteDocument;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a native document")
            }
            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let mut remote = BTreeMap::new();
                let doc = Document::deserialize(MapAccessDeserializer::new(Fields { map, remote: &mut remote }))?;
                Ok(RemoteDocument { doc, remote })
            }
        }
        d.deserialize_map(Payload)
    }
}
struct Fields<'a, M> {
    map: M,
    remote: &'a mut BTreeMap<String, super::RemoteImage>,
}
impl<'de, M: MapAccess<'de>> MapAccess<'de> for Fields<'_, M> {
    type Error = M::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        loop {
            let Some(key) = self.map.next_key::<String>()? else { return Ok(None) };
            if key == "remoteImageIndex" {
                *self.remote = self.map.next_value::<super::RemoteImages>()?.0;
            } else {
                return seed.deserialize(StringDeserializer::<M::Error>::new(key)).map(Some);
            }
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        self.map.next_value_seed(seed)
    }
}
