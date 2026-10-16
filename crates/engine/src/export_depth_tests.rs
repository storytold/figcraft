use crate::Session;
use crate::cmd::file::{export_bytes, export_bytes_with};
use figcraft_doc::{ExportFormat, ExportSetting, NodeId, NodeKind};
use serde_json::json;

fn nested(depth: usize) -> (Session, NodeId) {
    let mut s = Session::new();
    let doc = s.doc_mut().unwrap();
    let root = doc.create(doc.first_page().unwrap(), NodeKind::Frame, "deep", 0.0, 0.0, 32.0, 32.0).unwrap();
    let mut parent = root;
    for _ in 1..depth {
        parent = doc.create(parent, NodeKind::Group, "group", 0.0, 0.0, 32.0, 32.0).unwrap();
    }
    doc.create(parent, NodeKind::Rectangle, "leaf", 3.0, 5.0, 13.0, 17.0).unwrap();
    (s, root)
}

#[test]
fn export_depth_errors_reach_commands_and_preserve_existing_files() {
    let (mut s, root) = nested(257);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("figcraft-export-depth-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for format in [ExportFormat::Svg, ExportFormat::Pdf] {
        let path = dir.join(format!("deep.{}", format.ext()));
        std::fs::write(&path, b"previous export").unwrap();
        s.doc_mut().unwrap().get_mut(root).unwrap().exports = vec![ExportSetting { format, ..Default::default() }];
        let mut commands = vec![
            ("file.export", json!({"ids": [root], "format": format.ext(), "size": "512w", "path": path})),
            ("file.exportLayers", json!({"ids": [root], "dir": dir})),
        ];
        if format == ExportFormat::Pdf {
            commands.push(("file.exportFramesPdf", json!({"path": path})));
        }
        for (command, params) in commands {
            let error = s.execute(command, &params).unwrap_err().to_string();
            assert!(error.contains("maximum export depth of 256"), "{command}: {error}");
            assert!(error.contains("select an inner layer"), "{command}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), b"previous export");
        }
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "no staging or partial export files");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn export_depth_is_relative_to_drawn_roots_including_slice_contents() {
    let (mut s, root) = nested(257);
    let doc = s.doc_mut().unwrap();
    let page = doc.first_page().unwrap();
    let shallow = doc.create(page, NodeKind::Rectangle, "shallow", 0.0, 0.0, 8.0, 8.0).unwrap();
    let slice = doc.create(page, NodeKind::Slice, "slice", 0.0, 0.0, 32.0, 32.0).unwrap();
    let inner = doc.node(root).unwrap().children[0];
    for format in [ExportFormat::Svg, ExportFormat::Pdf] {
        assert!(export_bytes(doc, &[shallow], format, 1.0).is_ok());
        assert!(export_bytes(doc, &[inner], format, 1.0).is_ok());
        assert!(export_bytes(doc, &[slice], format, 1.0).unwrap_err().to_string().contains("maximum export depth"));
        let setting = ExportSetting { format, contents_only: false, ..Default::default() };
        assert!(export_bytes_with(doc, &[shallow], &setting, 1.0).unwrap_err().to_string().contains("maximum export depth"));
    }
}

#[test]
fn export_depth_is_checked_before_recursive_bounds() {
    let (mut s, root) = nested(2048);
    // Unclipped frames require recursive descendant bounds unless preflight refuses them first.
    s.doc_mut().unwrap().get_mut(root).unwrap().clip = false;
    for format in [ExportFormat::Svg, ExportFormat::Pdf] {
        assert!(export_bytes(s.doc().unwrap(), &[root], format, 1.0).unwrap_err().to_string().contains("maximum export depth"));
    }
}
