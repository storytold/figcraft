//! Depth refusal through the actual CLI, including its exit status and destination writes.

use figcraft_doc::color::{Color, Paint};
use figcraft_doc::{Document, NodeKind};
use std::process::Command;

#[test]
fn export_depth_cli_refuses_partial_files_and_accepts_an_inner_root() {
    let mut doc = Document::new("export depth");
    let root = doc.create(doc.first_page().unwrap(), NodeKind::Frame, "root", 0.0, 0.0, 32.0, 32.0).unwrap();
    let mut parent = root;
    for depth in 1..257 {
        let name = if depth == 1 { "inner-root" } else { "group" };
        parent = doc.create(parent, NodeKind::Group, name, 0.0, 0.0, 32.0, 32.0).unwrap();
    }
    let leaf = doc.create(parent, NodeKind::Rectangle, "leaf", 3.0, 5.0, 13.0, 17.0).unwrap();
    doc.get_mut(leaf).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("figcraft-cli-depth-process-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let input = dir.join("deep.figcraft");
    std::fs::write(&input, figcraft_format::save_bytes(&doc, None).unwrap()).unwrap();
    for ext in ["svg", "pdf"] {
        let path = dir.join(format!("previous.{ext}"));
        std::fs::write(&path, b"previous export").unwrap();
        for verb in ["convert", "export"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_figcraft-cli"));
            command.arg(verb).arg(&input);
            if verb == "export" {
                command.arg("-o");
            }
            let output = command.arg(&path).arg("--no-system-fonts").output().unwrap();
            let error = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{verb} {ext}: {error}");
            assert!(error.contains("maximum export depth of 256"), "{verb} {ext}: {error}");
            assert!(error.contains("select an inner layer"), "{verb} {ext}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), b"previous export");
        }
        let inner = dir.join(format!("inner.{ext}"));
        let output = Command::new(env!("CARGO_BIN_EXE_figcraft-cli"))
            .arg("export")
            .arg(&input)
            .arg("-o")
            .arg(&inner)
            .args(["--node", "inner-root", "--no-system-fonts"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let bytes = std::fs::read(inner).unwrap();
        if ext == "svg" {
            assert!(String::from_utf8_lossy(&bytes).contains("#FF0000"));
        } else {
            assert!(bytes.starts_with(b"%PDF"));
        }
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 5, "no staging files after refusal");
    std::fs::remove_dir_all(dir).unwrap();
}
