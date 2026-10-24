//! Sketch prototype links, hotspots, export options, layout grids, smart layout and locked
//! overrides.

use figcraft_doc::{Document, ExportFormat, Node, NodeKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;

fn open(layers: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "Page", layers)], &[]);
    import(&z, "f").unwrap()
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}"))
}

fn screens(button: Value) -> Vec<Value> {
    vec![artboard("A1", "Home", 0.0, 0.0, 100.0, 100.0, vec![button]), artboard("A2", "Detail", 200.0, 0.0, 100.0, 100.0, vec![])]
}

#[test]
fn flow_navigates() {
    let b = with(
        rect("Go", 0.0, 0.0, 20.0, 20.0),
        "flow",
        json!({"_class": "MSImmutableFlowConnection", "destinationArtboardID": "A2", "animationType": 1}),
    );
    let (doc, _) = open(screens(b));
    let go = node(&doc, "Go");
    assert_eq!(go.interactions.len(), 1, "{:?}", go.interactions);
    let detail = node(&doc, "Detail").id;
    let text = format!("{:?}", go.interactions[0]);
    assert!(text.contains(&format!("{detail:?}")), "{text}");
}

#[test]
fn flow_back() {
    let b = with(
        rect("Back", 0.0, 0.0, 20.0, 20.0),
        "flow",
        json!({"_class": "MSImmutableFlowConnection", "destinationArtboardID": "back", "animationType": 0}),
    );
    let (doc, _) = open(screens(b));
    let text = format!("{:?}", node(&doc, "Back").interactions);
    assert!(text.contains("Back"), "{text}");
}

#[test]
fn hotspot_is_transparent_frame_with_interaction() {
    let mut h = with(
        rect("Hot", 0.0, 0.0, 20.0, 20.0),
        "flow",
        json!({"_class": "MSImmutableFlowConnection", "destinationArtboardID": "A2", "animationType": 0}),
    );
    h["_class"] = json!("MSImmutableHotspotLayer");
    let (doc, _) = open(screens(h));
    let n = node(&doc, "Hot");
    assert!(matches!(n.kind, NodeKind::Frame));
    assert!(n.fills.is_empty());
    assert_eq!(n.interactions.len(), 1);
}

#[test]
fn flow_home_is_starting_point() {
    let mut home = artboard("A1", "Home", 0.0, 0.0, 100.0, 100.0, vec![]);
    home["isFlowHome"] = json!(true);
    let (doc, _) = open(vec![home]);
    let page = doc.node(doc.pages[0]).unwrap();
    let NodeKind::Page { flows, .. } = &page.kind else { panic!("not a page") };
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].start, node(&doc, "Home").id);
}

#[test]
fn export_options() {
    let mut r = rect("Icon", 0.0, 0.0, 20.0, 20.0);
    r["exportOptions"] = json!({"_class": "exportOptions", "exportFormats": [
        {"_class": "exportFormat", "fileFormat": "png", "scale": 2, "name": "@2x", "namingScheme": 0, "absoluteSize": 0, "visibleScaleType": 0},
        {"_class": "exportFormat", "fileFormat": "svg", "scale": 1, "name": "", "namingScheme": 0, "absoluteSize": 0, "visibleScaleType": 0},
        {"_class": "exportFormat", "fileFormat": "png", "scale": 1, "name": "", "namingScheme": 0, "absoluteSize": 64, "visibleScaleType": 1},
    ]});
    let (doc, _) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![r])]);
    let e = &node(&doc, "Icon").exports;
    assert_eq!(e.len(), 3);
    assert_eq!(e[0].format, ExportFormat::Png);
    assert_eq!(e[0].suffix, "@2x");
    assert_eq!(e[1].format, ExportFormat::Svg);
    let text = format!("{:?}", e[2]);
    assert!(text.contains("64"), "{text}");
}

#[test]
fn layout_and_square_grids() {
    let mut a = artboard("A", "Board", 0.0, 0.0, 1200.0, 800.0, vec![]);
    a["layout"] = json!({"_class": "layoutGrid", "isEnabled": true, "columnWidth": 84, "gutterWidth": 16, "horizontalOffset": 24, "numberOfColumns": 12,
        "totalWidth": 1152, "guttersOutside": false, "drawVertical": true, "drawHorizontal": false, "gutterHeight": 8, "rowHeightMultiplication": 3});
    a["grid"] = json!({"_class": "simpleGrid", "isEnabled": true, "gridSize": 8, "thickGridTimes": 0});
    let (doc, _) = open(vec![a]);
    let grids = format!("{:?}", node(&doc, "Board").grids);
    assert_eq!(node(&doc, "Board").grids.len(), 2, "{grids}");
    assert!(grids.contains("12") && grids.contains("16"), "{grids}");
    // Hidden, as Sketch shows them only when its layout view is on.
    assert!(node(&doc, "Board").grids.iter().all(|g| !g.visible), "{grids}");
}

#[test]
fn smart_layout_warns_once() {
    let g = |id: &str| {
        with(
            group(id, 0.0, 0.0, 10.0, 10.0, vec![rect(&format!("{id}r"), 0.0, 0.0, 10.0, 10.0)]),
            "groupLayout",
            json!({"_class": "MSImmutableInferredGroupLayout", "axis": 0}),
        )
    };
    let (_, warnings) = open(vec![artboard("A", "Board", 0.0, 0.0, 100.0, 100.0, vec![g("G1"), g("G2"), g("G3")])]);
    assert_eq!(warnings.iter().filter(|w| w.contains("Smart Layout")).count(), 1, "{warnings:?}");
}

#[test]
fn locked_overrides_are_noted() {
    let mut m = master("M", "S", "Locked", 50.0, 20.0, vec![text("L", 0.0, 0.0, 50.0, 20.0, "x", &[(0, 1, "Helvetica", 9.0, [0.0, 0.0, 0.0, 1.0])])]);
    m["overrideProperties"] = json!([{"_class": "MSImmutableOverrideProperty", "overrideName": "L_stringValue", "canOverride": false}]);
    let (_, warnings) = open(vec![m]);
    assert!(warnings.iter().any(|w| w.contains("locked")), "{warnings:?}");
}
