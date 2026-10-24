//! Measurements kept on the canvas: placed between two layers, across a side or to the layer
//! around one, following the layers, removed, undoable, copied with what they measure, refused on
//! bad input, synced, saved (old files without them still open) and allowed in Dev Mode.

use figcraft_collab::{Change, Prop, Replica};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn rect(s: &mut Session, x: f64, w: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": 20, "width": w, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id_of(&ok(s, "shape.rectangle", p)["id"]).unwrap()
}

/// A frame with two rectangles 30 apart: (frame, a, b).
fn setup() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let f = id_of(&ok(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 100}))["id"]).unwrap();
    let a = rect(&mut s, 20.0, 50.0, Some(f));
    let b = rect(&mut s, 100.0, 60.0, Some(f));
    (s, f, a, b)
}

fn labels(s: &mut Session, id: Option<NodeId>) -> Vec<Vec<String>> {
    let p = id.map_or(json!({}), |i| json!({"id": i}));
    ok(s, "devMode.measurements", p)
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["labels"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect())
        .collect()
}

#[test]
fn measurements_are_placed_follow_their_layers_and_are_removed() {
    let (mut s, f, a, b) = setup();
    let m = ok(&mut s, "devMode.addMeasurement", json!({"from": a, "to": b}));
    assert_eq!((&m["from"], &m["index"]), (&json!(a), &json!(0)));
    assert_eq!(m["lines"][0]["label"], "30", "the gap between them");
    // Across a side: the side's length; with neither, to the frame around it.
    ok(&mut s, "devMode.addMeasurement", json!({"from": b, "side": "top"}));
    ok(&mut s, "select.set", json!({"ids": [a]}));
    ok(&mut s, "devMode.addMeasurement", json!({}));
    assert_eq!(labels(&mut s, None), [vec!["30"], vec!["20", "230", "20", "40"], vec!["60"]]);
    let listed = ok(&mut s, "devMode.measurements", json!({"id": b}));
    assert_eq!(listed.as_array().unwrap().len(), 2, "from or to the layer");
    assert_eq!((&listed[1]["side"], &listed[1]["to"]), (&json!("top"), &Value::Null));
    assert_eq!(listed[0]["toName"], s.doc().unwrap().node(b).unwrap().name.as_str());
    // They follow the layers.
    ok(&mut s, "node.set", json!({"id": b, "props": {"x": 110, "width": 80}}));
    assert_eq!(labels(&mut s, Some(b)), [vec!["40"], vec!["80"]]);
    // Undo takes back an edit, and a placement.
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(labels(&mut s, Some(b)), [vec!["30"], vec!["60"]]);
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(labels(&mut s, None).len(), 2);
    ok(&mut s, "edit.redo", json!({}));
    // A deleted layer takes its measurements off the canvas; undo brings them back.
    ok(&mut s, "edit.delete", json!({"ids": [b]}));
    assert_eq!(labels(&mut s, None), [vec!["20", "230", "20", "40"]]);
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(labels(&mut s, None).len(), 3);
    // Removed one by one, or all of a layer's.
    assert_eq!(ok(&mut s, "devMode.removeMeasurement", json!({"from": a, "index": 1}))["removed"], 1);
    assert_eq!(labels(&mut s, None), [vec!["30"], vec!["60"]]);
    ok(&mut s, "devMode.addMeasurement", json!({"from": a, "side": "left"}));
    assert_eq!(ok(&mut s, "devMode.removeMeasurement", json!({"from": a}))["removed"], 2);
    assert!(s.execute("devMode.removeMeasurement", &json!({"from": a})).is_err(), "nothing left");
    assert_eq!(labels(&mut s, None), [vec!["60"]]);
    let _ = f;
}

#[test]
fn bad_measurements_are_refused() {
    let (mut s, f, a, b) = setup();
    let page = s.page().unwrap();
    let other = id_of(&ok(&mut s, "page.add", json!({}))["id"]).unwrap();
    let far = rect(&mut s, 0.0, 10.0, Some(other));
    ok(&mut s, "page.select", json!({"id": page}));
    let top = rect(&mut s, 400.0, 10.0, None);
    ok(&mut s, "select.none", json!({}));
    for p in [
        json!({"from": a, "to": a}),
        json!({"from": a, "to": page}),
        json!({"from": a, "to": far}),
        json!({"from": a, "to": 99_999}),
        json!({"from": page, "side": "top"}),
        json!({"from": a, "to": b, "side": "top"}),
        json!({"from": a, "side": "middle"}),
        json!({"from": a, "side": "auto"}),
        json!({"from": a, "side": 3}),
        json!({"from": "x", "side": "top"}),
        json!({"from": top}),
        json!({"side": "top"}),
    ] {
        assert!(s.execute("devMode.addMeasurement", &p).is_err(), "{p}");
    }
    ok(&mut s, "devMode.addMeasurement", json!({"from": a, "to": b}));
    assert!(s.execute("devMode.addMeasurement", &json!({"from": a, "to": b})).is_err(), "already there");
    assert!(s.execute("devMode.addMeasurement", &json!({"from": b, "to": a})).is_err(), "the same two layers");
    assert!(s.execute("devMode.removeMeasurement", &json!({"from": a, "index": 5})).is_err());
    assert!(s.execute("devMode.measurements", &json!({"page": a})).is_err());
    assert_eq!(ok(&mut s, "devMode.measurements", json!({"page": other})), json!([]));
    // A layer starts a bounded number.
    for _ in 1..crate::dev_measurements::MAX_PER_LAYER {
        let t = rect(&mut s, 200.0, 5.0, Some(f));
        ok(&mut s, "devMode.addMeasurement", json!({"from": a, "to": t}));
    }
    assert!(s.execute("devMode.addMeasurement", &json!({"from": a, "side": "top"})).is_err(), "capped");
    assert_eq!(s.doc().unwrap().node(a).unwrap().measurements.len(), crate::dev_measurements::MAX_PER_LAYER);
}

#[test]
fn copies_measure_what_was_copied_with_them() {
    let (mut s, f, a, b) = setup();
    ok(&mut s, "devMode.addMeasurement", json!({"from": a, "to": b}));
    ok(&mut s, "devMode.addMeasurement", json!({"from": a, "side": "bottom"}));
    // The frame copied: the copy of `a` measures to the copy of `b`.
    ok(&mut s, "edit.duplicate", json!({"ids": [f]}));
    let copy = s.selection()[0];
    let doc = s.doc().unwrap();
    let kids = doc.node(copy).unwrap().children.clone();
    let ca = doc.node(kids[0]).unwrap();
    assert_eq!(ca.measurements.len(), 2);
    assert_eq!(ca.measurements[0].to, Some(kids[1]));
    // `a` alone copied: only its side measurement comes along.
    ok(&mut s, "edit.duplicate", json!({"ids": [a]}));
    let lone = s.selection()[0];
    let m = &s.doc().unwrap().node(lone).unwrap().measurements;
    assert_eq!((m.len(), m[0].to, m[0].side), (1, None, Some(figcraft_doc::Side::Bottom)));
}

#[test]
fn measurements_sync_save_and_old_files_open() {
    let (mut s, _, a, b) = setup();
    let before = s.doc().unwrap().clone();
    ok(&mut s, "devMode.addMeasurement", json!({"from": a, "to": b}));
    ok(&mut s, "devMode.addMeasurement", json!({"from": b, "side": "right"}));
    // A change collaborators receive.
    let d = figcraft_collab::diff(&Replica::new(before), s.doc().unwrap()).unwrap();
    assert!(d.changes.iter().any(|c| matches!(c, Change::Set { id, prop: Prop::Measurements, .. } if *id == a)), "{:?}", d.changes);
    // What agents inspect.
    assert_eq!(crate::inspect::node_json(s.doc().unwrap(), b, 0)["measurements"], json!([{"side": "right"}]));
    // Saved and opened again.
    let doc = s.doc().unwrap();
    let bytes = figcraft_format::save_bytes(doc, None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!(back.node(a).unwrap().measurements, doc.node(a).unwrap().measurements);
    assert_eq!(back.node(b).unwrap().measurements, doc.node(b).unwrap().measurements);
    // A file from before measurements (no such field) opens with none.
    fn strip(v: &mut Value) {
        match v {
            Value::Object(o) => {
                o.remove("measurements");
                o.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut old = figcraft_format::to_json(doc).unwrap();
    assert!(old.to_string().contains("\"measurements\""));
    strip(&mut old);
    let (back, _) = figcraft_format::from_json(&old.to_string()).unwrap();
    assert!(back.node(a).unwrap().measurements.is_empty() && back.node(b).unwrap().name == doc.node(b).unwrap().name);
}

#[test]
fn dev_mode_places_measurements() {
    use crate::devinspect::dev_mode_allows;
    for id in ["devMode.addMeasurement", "devMode.removeMeasurement", "devMode.measurements"] {
        assert!(dev_mode_allows(id), "{id}");
    }
}

#[test]
fn instances_keep_their_own_measurements_across_main_edits() {
    let mut s = Session::new();
    let f = id_of(&ok(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40}))["id"]).unwrap();
    let r = rect(&mut s, 5.0, 10.0, Some(f));
    ok(&mut s, "devMode.addMeasurement", json!({"from": r, "side": "top"}));
    let main = id_of(&ok(&mut s, "component.create", json!({"ids": [f]}))["id"]).unwrap();
    let inst = id_of(&ok(&mut s, "instance.create", json!({"component": main, "x": 200, "y": 0}))["id"]).unwrap();
    let inner = s.doc().unwrap().node(inst).unwrap().children[0];
    // The main's measurements aren't copied onto its instances.
    assert!(s.doc().unwrap().node(inner).unwrap().measurements.is_empty());
    ok(&mut s, "devMode.addMeasurement", json!({"from": inner}));
    ok(&mut s, "devMode.addMeasurement", json!({"from": inst, "side": "left"}));
    ok(&mut s, "node.set", json!({"id": r, "props": {"opacity": 0.5}}));
    ok(&mut s, "node.set", json!({"id": main, "props": {"width": 120}}));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(inner).unwrap().measurements[0].to, Some(inst), "kept across a main edit");
    assert_eq!(doc.node(inst).unwrap().measurements.len(), 1);
    assert_eq!(doc.node(r).unwrap().measurements.len(), 1);
}
