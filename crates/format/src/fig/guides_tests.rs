//! Ruler guides in `.fig` exports: a page's and a frame's `guides`, as a synthetic file saved from
//! Figma's web app wrote them.

use std::collections::BTreeMap;

use figcraft_doc::{Document, Guide, NodeId, NodeKind};

use super::kiwi::Kv;
use super::{Archive, decode_canvas, export};

/// A document with a page guide at `page` and a frame guide at `frame` (both vertical and
/// horizontal), and the frame's id.
fn doc_with(page: &[Guide], frame: &[Guide]) -> (Document, NodeId) {
    let mut doc = Document::new("t");
    let p = doc.first_page().unwrap();
    let f = doc.create(p, NodeKind::Frame, "Frame 1", 1000.0, 500.0, 400.0, 300.0).unwrap();
    doc.node_mut(p).unwrap().guides = page.to_vec();
    doc.node_mut(f).unwrap().guides = frame.to_vec();
    (doc, f)
}

fn vertical(offset: f64) -> Guide {
    Guide { vertical: true, offset }
}

fn horizontal(offset: f64) -> Guide {
    Guide { vertical: false, offset }
}

/// The `guides` of the CANVAS record and of the FRAME record, as (axis, offset, sessionID, localID).
type Written = Option<Vec<(String, f32, u64, u64)>>;

fn written(doc: &Document) -> (Written, Written, Vec<String>) {
    let (bytes, warnings) = export(doc, None).unwrap();
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let nodes = msg.get("nodeChanges").unwrap().list();
    let of = |ty: &str| {
        let n = nodes
            .iter()
            .find(|n| n.get("type").and_then(Kv::str) == Some(ty) && n.get("name").and_then(Kv::str) != Some("Internal Only Canvas"))
            .unwrap();
        n.get("guides").map(|g| {
            g.list()
                .iter()
                .map(|x| {
                    let id = x.get("guid").unwrap();
                    (
                        x.get("axis").and_then(Kv::str).unwrap().to_string(),
                        x.get("offset").and_then(Kv::f64).unwrap() as f32,
                        id.get("sessionID").and_then(Kv::f64).unwrap() as u64,
                        id.get("localID").and_then(Kv::f64).unwrap() as u64,
                    )
                })
                .collect()
        })
    };
    (of("CANVAS"), of("FRAME"), warnings)
}

/// A component with a vertical guide at 24 that holds a frame with a vertical guide at 7, and an instance of it as the
/// engine makes one: its root a copy of the main's, guides included, and its layer mirroring the main's. Returns the
/// instance.
fn component_and_instance() -> (Document, NodeId) {
    let mut doc = Document::new("t");
    let p = doc.first_page().unwrap();
    let kind = NodeKind::Component { description: String::new(), props: BTreeMap::new(), prop_order: Vec::new() };
    let main = doc.create(p, kind, "Card", 0.0, 0.0, 200.0, 100.0).unwrap();
    let inner = doc.create(main, NodeKind::Frame, "Inner", 20.0, 30.0, 100.0, 50.0).unwrap();
    doc.node_mut(main).unwrap().guides = vec![vertical(24.0)];
    doc.node_mut(inner).unwrap().guides = vec![vertical(7.0)];
    let inst = doc.alloc_id();
    let mut root = doc.node(main).unwrap().clone();
    (root.id, root.kind, root.parent, root.source) = (inst, NodeKind::Instance { component: main, props: BTreeMap::new() }, None, Some(main));
    root.children.clear();
    doc.insert(root, p, None).unwrap();
    let mut layer = doc.node(inner).unwrap().clone();
    (layer.id, layer.parent, layer.source) = (doc.alloc_id(), None, Some(inner));
    doc.insert(layer, inst, None).unwrap();
    (doc, inst)
}

/// The `guides` of every instance override row in the file, as (axis, offset).
fn overridden(doc: &Document) -> Vec<Vec<(String, f32)>> {
    let (bytes, _) = export(doc, None).unwrap();
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let nodes = msg.get("nodeChanges").unwrap().list();
    let rows = nodes.iter().flat_map(|n| n.get("symbolData").and_then(|d| d.get("symbolOverrides")).map(Kv::list).unwrap_or_default());
    rows.filter_map(|row| row.get("guides"))
        .map(|g| {
            g.list()
                .iter()
                .map(|x| (x.get("axis").and_then(Kv::str).unwrap().to_string(), x.get("offset").and_then(Kv::f64).unwrap() as f32))
                .collect()
        })
        .collect()
}

#[test]
fn a_page_guide_at_120_and_a_frame_guide_at_40_are_written_as_figma_writes_them() {
    let (doc, _) = doc_with(&[horizontal(120.0), vertical(120.0)], &[horizontal(40.0), vertical(40.0)]);
    let (page, frame, warnings) = written(&doc);
    // Axis Y is a horizontal guide and X a vertical one; the offsets are the ruler readings (a frame's in its own
    // coordinates); localID is 1 and sessionID counts the file's guides from 3.
    assert_eq!(page, Some(vec![("Y".into(), 120.0, 3, 1), ("X".into(), 120.0, 4, 1)]));
    assert_eq!(frame, Some(vec![("Y".into(), 40.0, 5, 1), ("X".into(), 40.0, 6, 1)]));
    assert!(warnings.iter().all(|w| !w.contains("ruler guides")), "{warnings:?}");
}

#[test]
fn a_frame_without_guides_and_a_page_without_guides_write_no_field() {
    let (doc, _) = doc_with(&[], &[]);
    let (page, frame, warnings) = written(&doc);
    assert_eq!(page, None);
    assert_eq!(frame, None);
    assert!(warnings.iter().all(|w| !w.contains("ruler guides")), "{warnings:?}");
}

#[test]
fn a_frame_guide_is_not_shifted_by_the_frames_position() {
    let (doc, _) = doc_with(&[], &[vertical(78.0)]);
    let (_, frame, _) = written(&doc);
    assert_eq!(frame, Some(vec![("X".into(), 78.0, 3, 1)]));
}

#[test]
fn an_instance_overrides_guides_only_where_they_differ_from_its_mains() {
    let (mut doc, inst) = component_and_instance();
    // Untouched: the instance's guides and its layer's are the main's.
    assert_eq!(overridden(&doc), Vec::<Vec<(String, f32)>>::new());
    // Its own guide: one override, on the instance.
    doc.node_mut(inst).unwrap().guides = vec![horizontal(60.0)];
    assert_eq!(overridden(&doc), vec![vec![("Y".to_string(), 60.0)]]);
    // Its guides removed: an empty list, or it would show the main's.
    doc.node_mut(inst).unwrap().guides.clear();
    assert_eq!(overridden(&doc), vec![Vec::new()]);
}

#[test]
fn a_guide_whose_offset_is_not_finite_is_left_out() {
    let (doc, _) = doc_with(&[vertical(f64::NAN), vertical(10.0), horizontal(f64::INFINITY)], &[horizontal(f64::NEG_INFINITY)]);
    let (page, frame, _) = written(&doc);
    assert_eq!(page, Some(vec![("X".into(), 10.0, 3, 1)]));
    assert_eq!(frame, None);
}

#[test]
fn the_schema_numbers_the_guides() {
    let s = super::kiwi::Schema::parse(super::export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap();
    let field = |d: &str, f: &str| def(d).fields.iter().find(|x| x.name == f).unwrap().value;
    assert_eq!(field("NodeChange", "guides"), 138);
    assert_eq!(field("Guide", "axis"), 1);
    assert_eq!(field("Guide", "offset"), 2);
    assert_eq!(field("Guide", "guid"), 3);
    let axis = def("Axis");
    assert_eq!(axis.fields.iter().find(|x| x.name == "X").unwrap().value, 0);
    assert_eq!(axis.fields.iter().find(|x| x.name == "Y").unwrap().value, 1);
}
