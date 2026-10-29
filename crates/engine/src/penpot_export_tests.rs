use base64::Engine as _;
use figcraft_doc::{Document, NodeKind};
use serde_json::{Value, json};

use crate::{EngineError, Session};

fn id(v: &Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-penpot-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

type Layer = (String, [f64; 4]);

fn outline(doc: &Document) -> Vec<Layer> {
    doc.pages
        .iter()
        .flat_map(|p| doc.subtree(*p))
        .filter_map(|i| doc.node(i))
        .map(|n| {
            let t = doc.world_transform(n.id).translation();
            let text = n.kind.text().map(|t| t.characters.clone()).unwrap_or_default();
            let inside = doc.ancestors(n.id).iter().any(|a| matches!(doc.node(*a).map(|x| &x.kind), Some(NodeKind::Instance { .. })));
            let label = format!(
                "{} {:?} {text:?} fills {} strokes {} effects {} children {} linked {} interactions {}",
                n.kind.type_name(),
                n.name,
                n.fills.len(),
                n.strokes.len(),
                n.effects.len(),
                n.children.len(),
                !inside || n.source.is_some(),
                n.interactions.len(),
            );
            (label, [t.x, t.y, n.width, n.height])
        })
        .collect()
}

fn same(a: &[Layer], b: &[Layer]) -> Option<(Layer, Layer)> {
    if a.len() != b.len() {
        return a.first().cloned().zip(b.first().cloned());
    }
    a.iter().zip(b).find(|(x, y)| x.0 != y.0 || x.1.iter().zip(y.1).any(|(p, q)| (p - q).abs() > 0.01)).map(|(x, y)| (x.clone(), y.clone()))
}

fn build(s: &mut Session) {
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    s.execute("text.create", &json!({"text": "Hello 😀", "x": 10, "y": 10, "parent": frame})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 40, "width": 50, "height": 20, "parent": frame})).unwrap();
    s.execute("layout.add", &json!({"ids": [frame], "direction": "vertical"})).unwrap();
    let card = id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 120, "height": 60})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": card})).unwrap();
    let comp = id(&s.execute("component.create", &json!({"ids": [card]})).unwrap());
    let inst = id(&s.execute("instance.create", &json!({"component": comp, "x": 300, "y": 200})).unwrap());
    s.execute("proto.interaction.add", &json!({"id": frame, "dest": inst})).unwrap();
}

#[test]
fn exported_penpot_files_open_with_the_same_layers() {
    let mut s = Session::new();
    build(&mut s);
    let path = temp("roundtrip.penpot");
    let r = s.execute("file.export", &json!({"format": "penpot", "path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["format"], "penpot");
    assert!(r["warnings"].is_array());
    let before = outline(s.doc().unwrap());
    s.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(same(&outline(doc), &before), None);
    let inst = doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    assert!(inst.children.iter().all(|c| doc.node(*c).is_some_and(|c| c.source.is_some())));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn penpot_export_without_a_path_returns_the_file() {
    let mut s = Session::new();
    build(&mut s);
    let r = s.execute("file.export", &json!({"format": "PENPOT"})).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    let (doc, _) = crate::cmd::file::load_bytes(&bytes, "x.penpot").unwrap();
    assert_eq!(same(&outline(&doc), &outline(s.doc().unwrap())), None);
    let path = temp("by-extension.penpot");
    let r = s.execute("file.export", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["format"], "penpot");
    let _ = std::fs::remove_file(&path);
}

/// Provider references remain diagnostic metadata; open/import never retrieve font bytes.
#[test]
fn missing_provider_fonts_stay_offline_and_text_stays_editable() {
    let mut source = Document::new("Offline font");
    let page = source.first_page().unwrap();
    let mut text = figcraft_doc::TextData::new("Offline text");
    text.style.font_family = "FigCraft Missing Provider Q7".into();
    source.create(page, NodeKind::Text(text), "Label", 0.0, 0.0, 200.0, 30.0).unwrap();
    let bytes = figcraft_format::penpot::export(&source).unwrap().0;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    for command in ["file.open", "file.import"] {
        let mut s = Session::new();
        let result = s.execute(command, &json!({"name": "offline.penpot", "data": data})).unwrap();
        let doc = s.doc().unwrap();
        let label = doc.nodes().find(|n| n.name == "Label").unwrap();
        assert_eq!(label.kind.text().unwrap().style.font_family, "FigCraft Missing Provider Q7");
        assert_eq!(label.kind.text().unwrap().characters, "Offline text");
        assert!(label.plugin_data.contains_key("penpot.googleFontRefs"));
        assert!(result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|w| w.contains("Missing"))));
    }
}

#[test]
fn penpot_export_survives_junk_params() {
    let junk = [
        json!({"format": "penpot", "path": [5]}),
        json!({"format": "penpot", "path": ""}),
        json!({"format": "penpot", "path": "\u{0}/x.penpot"}),
        json!({"format": "penpot", "ids": ["x"], "scale": "NaN"}),
        json!({"format": ["penpot"]}),
        json!({"path": "nowhere/../\u{0}.penpot"}),
    ];
    for j in &junk {
        let mut s = Session::new();
        build(&mut s);
        if let Err(EngineError::Internal { cmd, msg }) = s.execute("file.export", j) {
            panic!("{cmd} panicked on {j}: {msg}");
        }
    }
    let mut s = Session::new();
    assert!(s.execute("file.export", &json!({"format": "penpot"})).is_ok());
}

#[test]
fn local_penpot_samples_export_and_open_again() {
    let Some(dir) = std::env::var_os("FIGCRAFT_PENPOT_SAMPLES") else { return };
    let Ok(dir) = std::fs::read_dir(dir) else { return };
    for p in dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "penpot")) {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": p.to_string_lossy()})).unwrap();
        let before = outline(s.doc().unwrap());
        let out = temp(&format!("again-{}", p.file_name().unwrap().to_string_lossy()));
        s.execute("file.export", &json!({"format": "penpot", "path": out.to_string_lossy()})).unwrap();
        s.execute("file.open", &json!({"path": out.to_string_lossy()})).unwrap();
        let after = outline(s.doc().unwrap());
        assert_eq!(same(&before, &after), None, "{}: {} → {} layers", p.display(), before.len(), after.len());
        let _ = std::fs::remove_file(&out);
    }
}
