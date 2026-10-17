use super::{kiwi::Kv, map};

fn obj(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn scalar(value: f32) -> Kv<'static> {
    Kv::Float(value)
}

fn guid(local_id: u64) -> Kv<'static> {
    obj(vec![("sessionID", Kv::Uint(40_000_000)), ("localID", Kv::Uint(local_id))])
}

fn record(local_id: u64, children: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut fields = vec![("guidPath", obj(vec![("guids", Kv::Array(vec![guid(local_id)]))]))];
    fields.extend(children);
    obj(fields)
}

fn transform(x: f32, y: f32) -> Kv<'static> {
    obj(vec![("m00", scalar(1.0)), ("m01", scalar(0.0)), ("m02", scalar(x)), ("m10", scalar(0.0)), ("m11", scalar(1.0)), ("m12", scalar(y))])
}

fn point(x: f32, y: f32) -> Kv<'static> {
    obj(vec![("x", scalar(x)), ("y", scalar(y))])
}

fn path_blob() -> Vec<u8> {
    let mut bytes = vec![1]; // MoveTo
    bytes.extend(0.0f32.to_le_bytes());
    bytes.extend(0.0f32.to_le_bytes());
    bytes.push(2); // LineTo
    bytes.extend(20.0f32.to_le_bytes());
    bytes.extend(10.0f32.to_le_bytes());
    bytes
}

fn paint() -> Kv<'static> {
    obj(vec![
        ("type", Kv::Enum("SOLID")),
        ("color", obj(vec![("r", scalar(0.1)), ("g", scalar(0.2)), ("b", scalar(0.8)), ("a", scalar(1.0))])),
        ("visible", Kv::Bool(true)),
    ])
}

fn synthetic_connector(version: f32, include_text: bool, text_x: f32) -> Kv<'static> {
    let line_geometry = Kv::Array(vec![obj(vec![("commandsBlob", Kv::Uint(0))])]);
    let line = record(0, vec![("transform", transform(7.0, 9.0)), ("size", point(20.0, 10.0)), ("fillGeometry", line_geometry)]);
    let text = record(1, vec![("transform", transform(text_x, 3.0)), ("size", point(80.0, 24.0))]);
    let background = record(2, vec![("transform", transform(100.0, 30.0)), ("size", point(90.0, 30.0))]);
    let mut derived = vec![line];
    if include_text {
        derived.extend([text, background]);
    }
    let line_style = record(0, vec![("strokePaints", Kv::Array(vec![paint()]))]);
    let text_style = record(
        1,
        vec![
            ("textData", obj(vec![("characters", Kv::Str("synthetic label".into()))])),
            ("fontSize", scalar(24.0)),
            ("fontName", obj(vec![("family", Kv::Str("Inter".into())), ("style", Kv::Str("Regular".into()))])),
            ("fillPaints", Kv::Array(vec![paint()])),
            ("visible", Kv::Bool(false)),
        ],
    );
    let bg_style = record(2, vec![("fillPaints", Kv::Array(vec![])), ("visible", Kv::Bool(false))]);
    let mut styles = vec![line_style];
    if include_text {
        styles.extend([text_style, bg_style]);
    }
    obj(vec![
        (
            "nodeChanges",
            Kv::Array(vec![
                obj(vec![("guid", guid(0)), ("type", Kv::Enum("DOCUMENT")), ("name", Kv::Str("Doc".into()))]),
                obj(vec![
                    ("guid", guid(1)),
                    ("parentIndex", obj(vec![("guid", guid(0)), ("position", Kv::Str("!".into()))])),
                    ("type", Kv::Enum("CANVAS")),
                    ("name", Kv::Str("Page".into())),
                ]),
                obj(vec![
                    ("guid", obj(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(123))])),
                    ("parentIndex", obj(vec![("guid", guid(1)), ("position", Kv::Str("!".into()))])),
                    ("type", Kv::Enum("CONNECTOR")),
                    ("name", Kv::Str("synthetic connector".into())),
                    ("size", point(140.0, 50.0)),
                    ("opacity", scalar(0.5)),
                    ("connectorType", Kv::Enum("MANUAL")),
                    ("connectorVersion", scalar(version)),
                    ("connectorLineStyle", Kv::Enum("STRAIGHT")),
                    ("connectorStartCap", Kv::Enum("NONE")),
                    ("connectorEndCap", Kv::Enum("ARROW")),
                    ("derivedImmutableFrameData", obj(vec![("version", scalar(1.0)), ("overrides", Kv::Array(derived))])),
                    ("nodeGenerationData", obj(vec![("overrides", Kv::Array(styles))])),
                ]),
            ]),
        ),
        ("blobs", Kv::Array(vec![obj(vec![("bytes", Kv::Bytes(path_blob()))])])),
    ])
}

fn map_synthetic(version: f32, include_text: bool, text_x: f32) -> (serde_json::Value, Vec<String>) {
    let (rest, warnings, _) = map::to_rest(&synthetic_connector(version, include_text, text_x), "synthetic", crate::rest::MAX_NODES);
    (rest, warnings)
}

fn find_node<'a>(doc: &'a figcraft_doc::Document, name: &str) -> &'a figcraft_doc::Node {
    doc.nodes().find(|node| node.name == name).expect("native node")
}

#[test]
fn synthetic_label_style_transform_and_container_opacity_survive_native_roundtrip() {
    let (rest, warnings) = map_synthetic(1.0, true, 8.5);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["type"], "FRAME");
    assert_eq!(connector["opacity"], 0.5);
    assert_eq!(connector["children"].as_array().unwrap().len(), 2, "empty/hidden background is omitted; hidden text remains explicit");
    let text = &connector["children"][1];
    assert_eq!(text["type"], "TEXT");
    assert_eq!(text["visible"], false);
    assert_eq!(text["fills"].as_array().unwrap().len(), 1);
    assert!((text["fills"][0]["color"]["b"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    assert_eq!(text["style"]["fontSize"], 24.0);
    assert_eq!(text["relativeTransform"], serde_json::json!([[1.0, 0.0, 108.5], [0.0, 1.0, 33.0]]));
    assert!(warnings.iter().any(|warning| warning.contains("live endpoint bindings are not retained")));

    let (native, _) = crate::rest::import_fig(&rest, &|_| None).unwrap();
    let bytes = crate::save_bytes(&native, None).unwrap();
    let (reopened, _) = crate::load_bytes(&bytes).unwrap();
    let frame = find_node(&reopened, "synthetic connector");
    assert_eq!(frame.opacity, 0.5);
    let text = find_node(&reopened, "synthetic label");
    assert!(!text.visible);
    assert_eq!(text.opacity, 1.0, "root opacity must not be copied to child");
    assert_eq!(text.transform.as_coeffs()[4..], [108.5, 33.0]);
    assert_eq!(text.fills.len(), 1);
    let figcraft_doc::NodeKind::Text(data) = &text.kind else { panic!("text child kind") };
    assert_eq!(data.style.font_size, 24.0);
}

#[test]
fn synthetic_connector_missing_optional_derived_label_keeps_vector() {
    let (rest, warnings) = map_synthetic(1.0, false, 0.0);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["type"], "VECTOR", "an unlabelled connector keeps the single-vector shape");
    assert!(connector.get("children").is_none());
    assert!(warnings.iter().all(|warning| !warning.contains("connector labels were skipped")));
}

#[test]
fn synthetic_unlabelled_connector_imports_as_one_vector_with_line_style() {
    let (rest, warnings) = map_synthetic(1.0, false, 0.0);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["type"], "VECTOR");
    assert_eq!(connector["name"], "synthetic connector");
    assert_eq!(connector["opacity"], 0.5);
    assert_eq!(connector["strokes"].as_array().map(Vec::len), Some(1));
    assert_eq!(connector["strokeCapEnd"], "ARROW");
    assert!(warnings.iter().all(|warning| !warning.contains("connector label")), "{warnings:?}");

    let (native, _) = crate::rest::import_fig(&rest, &|_| None).unwrap();
    let node = find_node(&native, "synthetic connector");
    assert!(matches!(node.kind, figcraft_doc::NodeKind::Vector { .. }));
}

#[test]
fn synthetic_connector_unsupported_version_uses_legacy_warning() {
    let (rest, warnings) = map_synthetic(2.0, true, 8.5);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["type"], "VECTOR");
    assert!(warnings.iter().any(|warning| warning == "connector labels were skipped"));
}

#[test]
fn synthetic_connector_malformed_label_bounds_or_transform_keep_vector() {
    let mut malformed_bounds = synthetic_connector(1.0, true, 8.5);
    set_override_field(&mut malformed_bounds, "derivedImmutableFrameData", 1, "size", point(f32::NAN, 24.0));
    let (rest, warnings, _) = map::to_rest(&malformed_bounds, "synthetic", crate::rest::MAX_NODES);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["children"].as_array().unwrap().len(), 1);
    assert_eq!(connector["children"][0]["type"], "VECTOR");
    assert!(warnings.iter().any(|warning| warning == "connector label was skipped: text bounds are invalid"));

    let mut malformed_transform = synthetic_connector(1.0, true, 8.5);
    set_override_field(&mut malformed_transform, "derivedImmutableFrameData", 1, "transform", transform(f32::NAN, 3.0));
    let (rest, warnings, _) = map::to_rest(&malformed_transform, "synthetic", crate::rest::MAX_NODES);
    let connector = &rest["document"]["children"][0]["children"][0];
    assert_eq!(connector["children"].as_array().unwrap().len(), 1);
    assert_eq!(connector["children"][0]["type"], "VECTOR");
    assert!(warnings.iter().any(|warning| warning == "connector label was skipped: text transform is invalid"));
}

#[test]
fn synthetic_connector_authored_hidden_background_is_retained() {
    let mut message = synthetic_connector(1.0, true, 8.5);
    set_override_field(&mut message, "nodeGenerationData", 2, "fillPaints", Kv::Array(vec![paint()]));
    let (rest, _, _) = map::to_rest(&message, "synthetic", crate::rest::MAX_NODES);
    let connector = &rest["document"]["children"][0]["children"][0];
    let background = connector["children"].as_array().unwrap().iter().find(|child| child["type"] == "RECTANGLE").expect("authored background");
    assert_eq!(background["visible"], false);
    assert_eq!(background["fills"].as_array().unwrap().len(), 1);
}

fn field_mut<'a, 's>(value: &'a mut Kv<'s>, key: &str) -> Option<&'a mut Kv<'s>> {
    let Kv::Object(fields) = value else { return None };
    fields.iter_mut().find(|(name, _)| *name == key).map(|(_, value)| value)
}

fn connector_mut<'a, 's>(message: &'a mut Kv<'s>, local_id: u64) -> Option<&'a mut Kv<'s>> {
    let Kv::Object(fields) = message else { return None };
    let node_changes = fields.iter_mut().find(|(name, _)| *name == "nodeChanges").map(|(_, value)| value)?;
    let Kv::Array(nodes) = node_changes else { return None };
    nodes.iter_mut().find(|node| {
        node.get("type").and_then(Kv::str) == Some("CONNECTOR")
            && node.get("guid").and_then(|guid| guid.get("localID")).and_then(Kv::f64) == Some(local_id as f64)
    })
}

fn set_override_field(message: &mut Kv<'_>, field: &str, local_id: u64, key: &'static str, value: Kv<'static>) {
    let connector = connector_mut(message, 123).expect("synthetic connector");
    let section = field_mut(connector, field).expect("connector override section");
    let overrides = field_mut(section, "overrides").expect("override list");
    let Kv::Array(rows) = overrides else { panic!("override list array") };
    let row = rows
        .iter_mut()
        .find(|row| {
            row.get("guidPath")
                .and_then(|path| path.get("guids"))
                .and_then(|ids| ids.list().first())
                .and_then(|id| id.get("localID"))
                .and_then(Kv::f64)
                == Some(local_id as f64)
        })
        .expect("override row");
    let Kv::Object(fields) = row else { panic!("override row object") };
    if let Some((_, current)) = fields.iter_mut().find(|(name, _)| *name == key) {
        *current = value;
    } else {
        fields.push((key, value));
    }
}
