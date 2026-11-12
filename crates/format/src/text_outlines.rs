//! Native-file outline pooling. Only the serialization copy contains references; documents,
//! collaboration changes and clipboard nodes keep their self-contained geometry.
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use figcraft_doc::{Document, NodeKind};
use serde::{Deserialize, Deserializer, Serialize};

use crate::FormatError;

const MAX_ENTRIES: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_LAYER_BYTES: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct Pool {
    version: u8,
    #[serde(deserialize_with = "read_paths")]
    paths: Vec<Arc<str>>,
}

fn read_paths<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Arc<str>>, D::Error> {
    struct Paths;
    impl<'de> serde::de::Visitor<'de> for Paths {
        type Value = Vec<Arc<str>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a bounded file outline pool")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut paths = Vec::new();
            let mut bytes = 0usize;
            while let Some(path) = seq.next_element::<Arc<str>>()? {
                bytes = bytes.saturating_add(path.len());
                if paths.len() >= MAX_ENTRIES || bytes > MAX_BYTES {
                    return Err(serde::de::Error::custom("file outline pool exceeds limit"));
                }
                paths.push(path);
            }
            Ok(paths)
        }
    }
    d.deserialize_seq(Paths)
}

pub(crate) fn pack(doc: &Document) -> (Cow<'_, Document>, Option<Pool>) {
    let mut output = Cow::Borrowed(doc);
    let mut ids: HashMap<Arc<str>, Arc<str>> = HashMap::new();
    let mut pool = Pool { version: 1, paths: Vec::new() };
    let mut bytes = 0usize;
    for node in doc.nodes() {
        let Some(saved) = node.kind.text().and_then(|t| t.appearance.as_ref()) else { continue };
        let mut replacement = None;
        for (i, glyph) in saved.glyphs.iter().enumerate() {
            // Inline short/empty shapes: their references would take more space than the path.
            if glyph.d.len() <= 16 {
                continue;
            }
            let id = if let Some(id) = ids.get(&glyph.d) {
                id.clone()
            } else {
                // A document with many unique shapes remains saveable: pool what fits and keep
                // the rest inline, instead of an unbounded global dictionary or discarded paths.
                if pool.paths.len() >= MAX_ENTRIES || bytes.saturating_add(glyph.d.len()) > MAX_BYTES {
                    continue;
                }
                let id: Arc<str> = format!("@{}", pool.paths.len()).into();
                bytes += glyph.d.len();
                pool.paths.push(glyph.d.clone());
                ids.insert(glyph.d.clone(), id.clone());
                id
            };
            let appearance = replacement.get_or_insert_with(|| (**saved).clone());
            if let Some(g) = appearance.glyphs.get_mut(i) {
                g.d = id;
            }
        }
        if let Some(appearance) = replacement
            && let Some(n) = output.to_mut().node_mut(node.id)
            && let NodeKind::Text(t) = &mut n.kind
        {
            t.appearance = Some(Arc::new(appearance));
        }
    }
    let pool = (!pool.paths.is_empty()).then_some(pool);
    (output, pool)
}

pub(crate) fn expand(doc: &mut Document, pool: Option<&Pool>) -> Result<(), FormatError> {
    let corrupt = |s: &str| FormatError::Corrupt(s.into());
    if pool.is_some_and(|p| p.version != 1) {
        return Err(corrupt("unsupported file outline pool version"));
    }
    let ids: Vec<_> = doc.ids().collect();
    let mut expanded = 0u64;
    for id in ids {
        let needs_pool = doc
            .node(id)
            .and_then(|n| n.kind.text())
            .and_then(|t| t.appearance.as_ref())
            .is_some_and(|a| a.glyphs.iter().any(|g| g.d.starts_with('@')));
        if !needs_pool {
            if let Some(saved) = doc.node(id).and_then(|n| n.kind.text()).and_then(|t| t.appearance.as_ref()) {
                for glyph in &saved.glyphs {
                    charge_total(&mut expanded, glyph.d.len())?;
                }
            }
            continue;
        }
        let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) else { continue };
        let Some(saved) = &mut t.appearance else { continue };
        let mut bytes = 0usize;
        for glyph in &mut Arc::make_mut(saved).glyphs {
            if let Some(index) = glyph.d.strip_prefix('@') {
                let path = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| pool.and_then(|p| p.paths.get(i)))
                    .ok_or_else(|| corrupt("missing file outline reference"))?;
                glyph.d = path.clone();
            }
            bytes = bytes.saturating_add(glyph.d.len());
            charge_total(&mut expanded, glyph.d.len())?;
            if bytes > MAX_LAYER_BYTES {
                return Err(corrupt("expanded portable outlines exceed layer byte limit"));
            }
        }
    }
    Ok(())
}

fn charge_total(total: &mut u64, bytes: usize) -> Result<(), FormatError> {
    *total = total.saturating_add(bytes as u64);
    if *total > crate::MAX_DOCUMENT {
        return Err(FormatError::Corrupt("expanded portable outlines exceed document work limit".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::{TextAppearance, TextData, TextGlyph};
    use serde_json::json;

    fn document() -> Document {
        let mut doc = Document::new("Shared shapes");
        let page = doc.first_page().unwrap();
        let path: Arc<str> = "M0 0L20 0L20 30L0 30Z".repeat(20).into();
        for _ in 0..4 {
            let mut text = TextData::new("AAAAAAAA");
            text.appearance = Some(Arc::new(TextAppearance {
                fingerprint: "synthetic geometry".into(),
                box_width: 100.0,
                box_height: 20.0,
                width: 100.0,
                height: 20.0,
                glyphs: (0..8)
                    .map(|i| TextGlyph {
                        d: path.clone(),
                        transform: Some([0.5, 0.0, 0.0, 0.5, i as f64, 15.0]),
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
                styles: vec![text.style.clone()],
                truncated: false,
            }));
            doc.create(page, NodeKind::Text(text), "Text", 0.0, 0.0, 100.0, 20.0).unwrap();
        }
        doc
    }

    #[test]
    fn native_pool_is_shared_across_nodes_without_mutating_the_document() {
        let doc = document();
        let id = doc.nodes().find(|n| n.kind.text().is_some()).unwrap().id;
        let before = doc.node_arc(id).unwrap().clone();
        let encoded = crate::to_json(&doc).unwrap();
        assert_eq!(encoded["textOutlines"]["paths"].as_array().unwrap().len(), 1);
        assert!(Arc::ptr_eq(&before, doc.node_arc(id).unwrap()));
        let (back, issues) = crate::load_bytes(&crate::save_bytes(&doc, None).unwrap()).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        let paths: Vec<_> = back.nodes().filter_map(|n| n.kind.text()?.appearance.as_ref()).map(|a| &a.glyphs[0].d).collect();
        assert_eq!(paths.len(), 4);
        assert!(paths.iter().all(|p| Arc::ptr_eq(p, paths[0])));
        for n in doc.nodes() {
            assert_eq!(back.node(n.id).unwrap().kind, n.kind);
        }
        // A plain collaboration/clipboard document stays self-contained, without the file table.
        let plain = serde_json::to_string(&doc).unwrap();
        assert!(!plain.contains("\"@0\""));
        let (old, _) = crate::from_json(&plain).unwrap();
        assert_eq!(old.node(id).unwrap().kind, doc.node(id).unwrap().kind);
    }

    #[test]
    fn version_documents_keep_their_own_outline_pool() {
        let doc = document();
        let id = doc.nodes().find(|n| n.kind.text().is_some()).unwrap().id;
        let version = crate::Version::new("version-1", "Saved", "", 1, Arc::new(doc.clone()));
        let bytes = crate::save_bytes_with_versions(&doc, None, &[version]).unwrap();
        let loaded = crate::load_bytes_with_versions(&bytes).unwrap();
        assert_eq!(loaded.versions.len(), 1);
        assert_eq!(loaded.versions[0].document().unwrap().node(id).unwrap().kind, doc.node(id).unwrap().kind);
    }

    #[test]
    fn malformed_file_references_and_pool_versions_are_refused() {
        let mut wire = crate::to_json(&document()).unwrap();
        wire["textOutlines"]["paths"] = json!([]);
        assert!(crate::from_json(&wire.to_string()).is_err());
        wire.as_object_mut().unwrap().remove("textOutlines");
        assert!(crate::from_json(&wire.to_string()).is_err());
        let mut wire = crate::to_json(&document()).unwrap();
        wire["textOutlines"]["version"] = json!(2);
        assert!(crate::from_json(&wire.to_string()).is_err());
    }

    #[test]
    fn a_small_pool_cannot_expand_one_layer_past_the_work_budget() {
        let mut wire = crate::to_json(&document()).unwrap();
        wire["textOutlines"]["paths"] = json!(["x".repeat(MAX_LAYER_BYTES / 4 + 1)]);
        assert!(crate::from_json(&wire.to_string()).is_err());
    }

    #[test]
    fn a_small_file_cannot_hide_unbounded_outline_work_across_layers() {
        let mut doc = document();
        let page = doc.first_page().unwrap();
        let mut text = doc.nodes().find_map(|n| n.kind.text()).unwrap().clone();
        let path: Arc<str> = "x".repeat(1024 * 1024).into();
        for g in &mut Arc::make_mut(text.appearance.as_mut().unwrap()).glyphs {
            g.d = path.clone();
        }
        // Each layer is under 16 MiB, but the combined expanded work passes the document cap.
        for _ in 0..=crate::MAX_DOCUMENT / (8 * 1024 * 1024) {
            doc.create(page, NodeKind::Text(text.clone()), "Text", 0.0, 0.0, 100.0, 20.0).unwrap();
        }
        let wire = crate::to_json(&doc).unwrap();
        assert_eq!(wire["textOutlines"]["paths"].as_array().unwrap().len(), 2);
        assert!(crate::from_json(&wire.to_string()).is_err());
    }
}
