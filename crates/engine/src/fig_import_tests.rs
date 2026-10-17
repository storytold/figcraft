//! `.fig` files through the file commands. The importer's own tests (fixtures built in code) are
//! in `figcraft_format::fig`; the local check here runs only when `FIGCRAFT_FIG_SAMPLES` names a
//! folder of `.fig` files with their frames' PNG exports next to them.

use figcraft_doc::{ExportFormat, NodeKind};
use serde_json::json;

use crate::Session;

#[test]
fn damaged_fig_files_fail_cleanly() {
    let dir = std::env::temp_dir().join(format!("figcraft-fig-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("broken.fig");
    std::fs::write(&path, b"fig-kiwi\x65\0\0\0\xff\xff\xff\x7fjunk").unwrap();
    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"path": path.to_string_lossy()}));
    assert!(r.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Local only (`FIGCRAFT_FIG_SAMPLES`): each file opens without warnings, instances link to their
/// mains, and each top frame's 1× PNG export is close to the export saved next to the file.
#[test]
fn local_samples_match_their_exports() {
    let Some(dir) = std::env::var_os("FIGCRAFT_FIG_SAMPLES") else { return };
    let Ok(dir) = std::fs::read_dir(dir) else { return };
    for p in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "fig")) {
        let mut s = Session::new();
        let r = s.execute("file.open", &json!({"path": p.to_string_lossy()})).unwrap();
        assert_eq!(r["warnings"], json!([]), "{}", p.display());
        let doc = s.doc().unwrap();
        for n in doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })) {
            assert!(n.children.iter().all(|c| doc.node(*c).is_some_and(|c| c.source.is_some())), "{}: {}", p.display(), n.name);
        }
        let page = doc.first_page().unwrap();
        for f in doc.top_frames(page) {
            let name = &doc.node(f).unwrap().name;
            let Ok(want) = image::open(p.with_file_name(format!("{name}.png"))) else { continue };
            let got = crate::cmd::file::export_bytes(doc, &[f], ExportFormat::Png, 1.0).unwrap();
            let (got, want) = (image::load_from_memory(&got).unwrap().to_rgb8(), want.to_rgb8());
            // Exports with effects may be cropped differently; compare the ones of the same size.
            if got.dimensions() != want.dimensions() {
                continue;
            }
            let diff: u64 = got.as_raw().iter().zip(want.as_raw()).map(|(a, b)| u64::from(a.abs_diff(*b))).sum();
            let mean = diff as f64 / got.as_raw().len() as f64;
            // Text may use another font where the sample's isn't installed.
            assert!(mean < 4.0, "{name}: mean difference {mean:.2}");
        }
    }
}

/// Layers of an instance copy keep their link to the layer they stand for when the file is
/// opened, also after they were renamed in the copy (a name is no identity), and the override
/// that a rename is stays an override.
#[test]
fn opened_fig_links_instance_layers_by_identity_not_by_name() {
    use base64::Engine as _;
    let mut s = Session::new();
    let make = |s: &mut Session, cmd: &str, p: serde_json::Value| s.execute(cmd, &p).unwrap()["id"].as_u64().map(figcraft_doc::NodeId).unwrap();
    let body = make(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 20, "name": "Inner"}));
    let label = make(&mut s, "text.create", json!({"text": "Label", "x": 2, "y": 2, "parent": body, "props": {"name": "Label"}}));
    let inner = make(&mut s, "component.create", json!({"ids": [body]}));
    let frame = make(&mut s, "frame.create", json!({"x": 100, "y": 0, "width": 200, "height": 60, "name": "Outer"}));
    let outer = make(&mut s, "component.create", json!({"ids": [frame]}));
    let slot = make(&mut s, "instance.create", json!({"component": inner, "x": 0, "y": 0, "parent": outer}));
    s.execute("node.set", &json!({"id": slot, "props": {"name": "Slot"}})).unwrap();
    let copy = make(&mut s, "instance.create", json!({"component": outer, "x": 0, "y": 200}));
    let copy_slot = s.doc().unwrap().node(copy).unwrap().children[0];
    let copy_label = s.doc().unwrap().node(copy_slot).unwrap().children[0];
    s.execute("node.set", &json!({"id": copy_slot, "props": {"name": "Renamed slot"}})).unwrap();
    s.execute("node.set", &json!({"id": copy_label, "props": {"name": "Renamed label"}})).unwrap();
    let _ = label;

    let exported = s.execute("file.export", &json!({"format": "fig"})).unwrap();
    let path = std::env::temp_dir().join(format!("figcraft-slots-{}.fig", std::process::id()));
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode(exported["data"].as_str().unwrap()).unwrap()).unwrap();
    let mut opened = Session::new();
    let result = opened.execute("file.open", &json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    result.unwrap();
    let doc = opened.doc().unwrap();
    let outer = doc.nodes().find(|n| n.name == "Outer" && matches!(n.kind, NodeKind::Component { .. })).unwrap();
    let copy = doc.nodes().find(|n| n.name == "Outer" && matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    let slot = doc.node(copy.children[0]).unwrap();
    let label = doc.node(slot.children[0]).unwrap();
    assert_eq!((slot.name.as_str(), label.name.as_str()), ("Renamed slot", "Renamed label"));
    assert_eq!(slot.source, Some(outer.children[0]));
    assert_eq!(label.source, Some(doc.node(outer.children[0]).unwrap().children[0]));
    assert!(slot.overrides.contains("name") && label.overrides.contains("name"));
}
