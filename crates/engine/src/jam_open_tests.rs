//! FigJam `.jam` files (#1342) open and import like `.fig` files: File ▸ Open (`file.open`), the
//! command line (the same command) and File ▸ Import / drag and drop (`file.import`). The file is
//! made here from our own `.fig` export with the canvas header FigJam writes (`fig-jam.`).

use std::io::{Read, Write};

use serde_json::json;

use crate::Session;

/// A `.jam` holding one page with a rectangle named `Card`, written to a temporary file.
fn jam_file(tag: &str) -> std::path::PathBuf {
    let mut doc = figcraft_doc::Document::new("Synthetic board");
    let page = doc.first_page().unwrap();
    doc.create(page, figcraft_doc::NodeKind::Rectangle, "Card", 10.0, 10.0, 80.0, 40.0).unwrap();
    let (fig, _) = figcraft_format::fig::export(&doc, None).unwrap();
    let mut from = zip::ZipArchive::new(std::io::Cursor::new(fig)).unwrap();
    let mut to = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..from.len() {
        let mut member = from.by_index(i).unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        if member.name() == "canvas.fig" {
            assert!(bytes.starts_with(b"fig-kiwi"));
            bytes.splice(..8, *b"fig-jam.");
        }
        to.start_file(member.name(), zip::write::SimpleFileOptions::default()).unwrap();
        to.write_all(&bytes).unwrap();
    }
    let path = std::env::temp_dir().join(format!("figcraft-jam-{tag}-{}.jam", std::process::id()));
    std::fs::write(&path, to.finish().unwrap().into_inner()).unwrap();
    path
}

fn cards(s: &Session) -> usize {
    s.doc().unwrap().nodes().filter(|n| n.name == "Card").count()
}

#[test]
fn a_jam_file_opens_and_imports_like_a_fig() {
    let path = jam_file("open");
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(cards(&s), 1, "opened");
    assert!(crate::board::is_board(s.doc().unwrap()), "a whiteboard opens as a board");
    // Imported into a design file, it adds its pages to it (not an image of its bytes), which
    // stays a design file.
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    let before = s.doc().unwrap().pages.len();
    s.execute("file.import", &json!({"path": path})).unwrap();
    assert_eq!(s.doc().unwrap().pages.len(), before + 1);
    assert_eq!(cards(&s), 1);
    assert!(!crate::board::is_board(s.doc().unwrap()));
    std::fs::remove_file(path).unwrap();
}
