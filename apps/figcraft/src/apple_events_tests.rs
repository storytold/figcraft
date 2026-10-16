//! Documents macOS hands over in an open-documents Apple event (a Finder double-click, Open With,
//! a drop on the Dock icon) open as File › Open opens them (#289).

use std::path::PathBuf;

use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

use crate::apple_events::open_documents;

#[test]
fn documents_from_finder_open_and_leave_the_home_screen() {
    let dir = std::env::temp_dir().join(format!("figcraft-apple-events-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("BLA-001.figcraft");
    let mut s = Session::new();
    s.execute("file.saveAs", &json!({"path": file.to_string_lossy()})).unwrap();

    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    app.ui.home = true;
    // A file that can't be read is reported and doesn't keep the others from opening.
    open_documents(&mut app, vec![dir.join("Missing.figcraft"), file.clone()]);
    let paths: Vec<_> = app.session.docs.iter().map(|d| d.path.clone().map(PathBuf::from)).collect();
    assert_eq!(paths, [Some(file)]);
    assert!(!app.ui.home, "the opened document shows");
    assert!(app.ui.last_error.as_deref().is_some_and(|e| e.contains("Missing.figcraft")), "{:?}", app.ui.last_error);
    let _ = std::fs::remove_dir_all(&dir);
}
