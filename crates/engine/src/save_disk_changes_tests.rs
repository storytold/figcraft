//! Saving never silently replaces what another app wrote to the open file since it was opened or
//! saved here (#1330): Save and auto-save refuse, `overwrite` and Revert to saved settle it.

use serde_json::json;

use crate::Session;
use crate::cmd::file::FileSnapshot;

fn temp_file(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("figcraft-disk-changes-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("TESTE.figcraft").to_string_lossy().into_owned()
}

/// What the file on disk names its first page.
fn page_on_disk(path: &str) -> String {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let ds = s.ds().unwrap();
    ds.doc.node(ds.page).unwrap().name.clone()
}

/// The app has `path` open with an unsaved edit, after figcraft-cli renamed its page on disk.
fn app_and_cli(path: &str) -> Session {
    let mut setup = Session::new();
    setup.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    setup.execute("file.save", &json!({"path": path})).unwrap();
    let mut app = Session::new();
    app.execute("file.open", &json!({"path": path})).unwrap();
    let mut cli = Session::new();
    cli.execute("file.open", &json!({"path": path})).unwrap();
    cli.execute("page.rename", &json!({"name": "CLI VENCEU"})).unwrap();
    cli.execute("file.save", &json!({})).unwrap();
    app.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    app
}

#[test]
fn saving_over_changes_made_on_disk_is_refused_until_asked() {
    let path = temp_file("save");
    let mut app = app_and_cli(&path);
    let err = app.execute("file.save", &json!({})).unwrap_err().to_string();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(page_on_disk(&path), "CLI VENCEU", "the CLI's change is still in the file");
    assert!(app.ds().unwrap().is_dirty(), "the app's edit stays unsaved");
    // Asked to, the save replaces it; from then on the file is the app's again.
    app.execute("file.save", &json!({"overwrite": true})).unwrap();
    assert_ne!(page_on_disk(&path), "CLI VENCEU");
    app.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    app.execute("file.save", &json!({})).unwrap();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reverting_loads_the_changes_made_on_disk_and_saves_again() {
    let path = temp_file("revert");
    let mut app = app_and_cli(&path);
    app.execute("file.revert", &json!({})).unwrap();
    let ds = app.ds().unwrap();
    assert_eq!(ds.doc.node(ds.page).unwrap().name, "CLI VENCEU");
    app.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    app.execute("file.save", &json!({})).unwrap();
    assert_eq!(page_on_disk(&path), "CLI VENCEU", "saved on top of the CLI's change");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn auto_save_leaves_changes_made_on_disk_alone() {
    let path = temp_file("auto");
    let mut app = app_and_cli(&path);
    let snapshot = FileSnapshot::of(app.ds().unwrap());
    let bytes = snapshot.encode().unwrap();
    assert!(snapshot.save(&mut app, &bytes).is_err());
    assert_eq!(page_on_disk(&path), "CLI VENCEU");
    let _ = std::fs::remove_file(&path);
}

/// The name of the page the active tab shows.
fn page_in_tab(s: &Session) -> String {
    let ds = s.ds().unwrap();
    ds.doc.node(ds.page).unwrap().name.clone()
}

/// Opening a file that is already open says when it changed on disk since the tab read or saved
/// it, and `reload` reads it again into the tab, unless the tab has unsaved changes (#997).
#[test]
fn opening_an_open_file_says_it_is_stale_and_reloads_on_request() {
    let path = temp_file("reopen");
    let mut setup = Session::new();
    setup.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    setup.execute("file.save", &json!({"path": path})).unwrap();
    let mut app = Session::new();
    let index = app.execute("file.open", &json!({"path": path})).unwrap()["index"].clone();
    let before = page_in_tab(&app);
    assert_eq!(app.execute("file.open", &json!({"path": path})).unwrap(), json!({"index": index}), "unchanged: not stale");
    let mut cli = Session::new();
    cli.execute("file.open", &json!({"path": path})).unwrap();
    cli.execute("page.rename", &json!({"name": "Renamed on disk by the CLI"})).unwrap();
    cli.execute("file.save", &json!({})).unwrap();
    assert_eq!(app.execute("file.open", &json!({"path": path})).unwrap(), json!({"index": index, "stale": true}));
    assert_eq!(page_in_tab(&app), before, "without reload the tab keeps what it read");
    assert_eq!(app.execute("file.open", &json!({"path": path, "reload": true})).unwrap(), json!({"index": index, "reloaded": true}));
    assert_eq!(page_in_tab(&app), "Renamed on disk by the CLI");
    assert_eq!(app.execute("file.open", &json!({"path": path})).unwrap(), json!({"index": index}), "read again: no longer stale");
    app.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    let e = app.execute("file.open", &json!({"path": path, "reload": true})).unwrap_err().to_string();
    assert!(e.contains("unsaved changes"), "{e}");
    assert!(app.ds().unwrap().is_dirty(), "the unsaved edit is still there");
    let _ = std::fs::remove_file(&path);
}
