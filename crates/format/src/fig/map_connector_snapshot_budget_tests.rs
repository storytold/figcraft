//! Connector snapshot flattening stays within the node and depth budgets.

use super::{MAX_DEPTH, Mapper, View};
use crate::fig::kiwi::Kv;
use crate::rest::MAX_NODES;
use figcraft_doc::geom::Affine;

fn obj(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn guid(local_id: u64) -> Kv<'static> {
    obj(vec![("sessionID", Kv::Uint(40_000_000)), ("localID", Kv::Uint(local_id))])
}

fn path_blob() -> Vec<u8> {
    let mut bytes = vec![1];
    bytes.extend(0.0f32.to_le_bytes());
    bytes.extend(0.0f32.to_le_bytes());
    bytes.push(2);
    bytes.extend(20.0f32.to_le_bytes());
    bytes.extend(10.0f32.to_le_bytes());
    bytes
}

fn record(local_id: u64, mut fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    fields.push(("guidPath", obj(vec![("guids", Kv::Array(vec![guid(local_id)]))])));
    obj(fields)
}

fn message() -> Kv<'static> {
    let line = record(
        0,
        vec![
            (
                "transform",
                obj(vec![
                    ("m00", Kv::Float(1.0)),
                    ("m01", Kv::Float(0.0)),
                    ("m02", Kv::Float(0.0)),
                    ("m10", Kv::Float(0.0)),
                    ("m11", Kv::Float(1.0)),
                    ("m12", Kv::Float(0.0)),
                ]),
            ),
            ("size", obj(vec![("x", Kv::Float(20.0)), ("y", Kv::Float(10.0))])),
            ("fillGeometry", Kv::Array(vec![obj(vec![("commandsBlob", Kv::Uint(0))])])),
        ],
    );
    let text = record(
        1,
        vec![
            (
                "transform",
                obj(vec![
                    ("m00", Kv::Float(1.0)),
                    ("m01", Kv::Float(0.0)),
                    ("m02", Kv::Float(0.0)),
                    ("m10", Kv::Float(0.0)),
                    ("m11", Kv::Float(1.0)),
                    ("m12", Kv::Float(0.0)),
                ]),
            ),
            ("size", obj(vec![("x", Kv::Float(50.0)), ("y", Kv::Float(20.0))])),
        ],
    );
    let background = record(
        2,
        vec![
            (
                "transform",
                obj(vec![
                    ("m00", Kv::Float(1.0)),
                    ("m01", Kv::Float(0.0)),
                    ("m02", Kv::Float(0.0)),
                    ("m10", Kv::Float(0.0)),
                    ("m11", Kv::Float(1.0)),
                    ("m12", Kv::Float(0.0)),
                ]),
            ),
            ("size", obj(vec![("x", Kv::Float(60.0)), ("y", Kv::Float(30.0))])),
        ],
    );
    let line_style = record(0, vec![]);
    let text_style = record(1, vec![("textData", obj(vec![("characters", Kv::Str("budget label".into()))]))]);
    let background_style = record(2, vec![("fillPaints", Kv::Array(vec![]))]);
    obj(vec![
        (
            "nodeChanges",
            Kv::Array(vec![obj(vec![
                ("guid", obj(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(123))])),
                ("type", Kv::Enum("CONNECTOR")),
                ("name", Kv::Str("budget connector".into())),
                ("connectorType", Kv::Enum("MANUAL")),
                ("connectorVersion", Kv::Float(1.0)),
                ("connectorLineStyle", Kv::Enum("STRAIGHT")),
                ("derivedImmutableFrameData", obj(vec![("version", Kv::Float(1.0)), ("overrides", Kv::Array(vec![line, text, background]))])),
                ("nodeGenerationData", obj(vec![("overrides", Kv::Array(vec![line_style, text_style, background_style]))])),
            ])]),
        ),
        ("blobs", Kv::Array(vec![obj(vec![("bytes", Kv::Bytes(path_blob()))])])),
    ])
}

#[test]
fn connector_snapshot_respects_last_and_zero_child_slots() {
    let message = message();
    let node = message.get("nodeChanges").unwrap().list().first().unwrap();
    let view = View::of(node);
    let mut mapper = Mapper::new(&message, MAX_NODES);
    mapper.count = MAX_NODES - 1;
    let children = mapper.connector_snapshot_children(&view, "1:123");
    assert_eq!(children.len(), 1, "the final available slot preserves the saved vector");
    assert_eq!(children[0]["type"], "VECTOR");
    assert!(!children[0]["fillGeometry"].as_array().unwrap().is_empty(), "saved outline survives the budget limit");
    assert_eq!(children[0]["fillGeometry"][0]["path"], "M 0 0 L 20 10");
    assert_eq!(mapper.count, MAX_NODES);
    assert!(mapper.warnings.iter().any(|warning| warning == &format!("the file has more than {MAX_NODES} layers; connector label was skipped")));

    let mut mapper = Mapper::new(&message, MAX_NODES);
    mapper.count = MAX_NODES;
    let children = mapper.connector_snapshot_children(&view, "1:123");
    assert!(children.is_empty());
    assert_eq!(mapper.count, MAX_NODES, "the node budget is never exceeded");
    assert!(mapper.warnings.iter().any(|warning| warning.contains("connector snapshot children were skipped")));
}

#[test]
fn connector_snapshot_flattens_vector_at_depth_and_node_limits() {
    let message = message();
    let mut mapper = Mapper::new(&message, MAX_NODES);
    let depth_limited = mapper.layer("1:123", None, Affine::IDENTITY, None, None, MAX_DEPTH).expect("terminal connector");
    assert_eq!(depth_limited["type"], "VECTOR");
    assert!(!depth_limited["fillGeometry"].as_array().unwrap().is_empty());
    assert!(depth_limited.get("children").is_none());
    assert!(mapper.warnings.iter().any(|warning| warning == "connector label was skipped: depth limit prevents nested snapshot children"));

    let mut mapper = Mapper::new(&message, MAX_NODES);
    mapper.count = MAX_NODES - 1;
    let budget_limited = mapper.layer("1:123", None, Affine::IDENTITY, None, None, 0).expect("last allowed connector");
    assert_eq!(budget_limited["type"], "VECTOR");
    assert!(!budget_limited["fillGeometry"].as_array().unwrap().is_empty());
    assert!(budget_limited.get("children").is_none());
    assert_eq!(mapper.count, MAX_NODES);
    assert!(mapper.warnings.iter().any(|warning| warning == "connector label was skipped: node budget prevents nested snapshot children"));
}
