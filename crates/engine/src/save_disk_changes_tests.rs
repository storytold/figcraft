//! Saving never silently replaces what another app wrote to the open file since it was opened or
//! saved here (#1330): Save and auto-save refuse, `overwrite` and Revert to saved settle it.

use serde_json::json;

use crate::Session;
use crate::cmd::file::FileSnapshot;

/// A synthetic native path isolated by process and regression scenario.
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

/// Two independent app processes initially displaying the same empty native file (#1388).
fn two_windows(path: &str) -> (Session, Session) {
    let mut first = Session::new();
    first.execute("file.save", &json!({"path": path})).unwrap();
    let mut second = Session::empty();
    second.execute("file.open", &json!({"path": path})).unwrap();
    (first, second)
}

#[test]
fn another_windows_save_refreshes_the_clean_tab_and_survives_both_close_orders() {
    for close_first in [true, false] {
        let path = temp_file(if close_first { "close-first" } else { "close-second" });
        let (mut first, mut second) = two_windows(&path);
        let uid = second.ds().unwrap().uid;
        let stale = FileSnapshot::of(second.ds().unwrap());
        let stale_bytes = stale.encode().unwrap();
        let page = second.page().unwrap();
        second.ds_mut().unwrap().view.zoom = 2.0;
        first.execute("shape.rectangle", &json!({"width": 100, "height": 50})).unwrap();
        first.execute("file.save", &json!({})).unwrap();
        let refreshed = second.execute("file.refresh", &json!({})).unwrap();
        assert_eq!(refreshed["refreshed"], json!([uid]));
        assert_eq!(refreshed["errors"], json!([]));
        let ds = second.ds().unwrap();
        assert_eq!(ds.uid, uid, "the window still owns the same tab");
        assert_eq!(ds.page, page);
        assert_eq!(ds.view.zoom, 2.0);
        assert!(!ds.is_dirty(), "the refresh is a saved baseline");
        assert_eq!(second.undo_depth(), (0, 0));
        assert_eq!(first.doc().unwrap(), second.doc().unwrap());
        assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
        assert!(!stale.save(&mut second, &stale_bytes).unwrap(), "a queued save cannot put the old empty file back");
        // Closing neither window can save a stale empty version over the rectangle.
        let (a, b) = if close_first { (&mut first, &mut second) } else { (&mut second, &mut first) };
        a.execute("file.close", &json!({})).unwrap();
        b.execute("file.close", &json!({})).unwrap();
        let mut reopened = Session::empty();
        reopened.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(reopened.doc().unwrap().nodes().count(), 2);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn explicit_save_paths_cannot_replace_another_windows_changes() {
    for (name, command, alias) in [("save-path", "file.save", false), ("save-as-path", "file.saveAs", false), ("save-as-alias", "file.saveAs", true)]
    {
        let path = temp_file(name);
        let mut app = app_and_cli(&path);
        let target = if alias { path.replace("TESTE.figcraft", "./TESTE.figcraft") } else { path.clone() };
        assert!(app.execute(command, &json!({"path": target})).unwrap_err().to_string().contains("changed on disk"));
        assert_eq!(page_on_disk(&path), "CLI VENCEU");
        assert!(app.ds().unwrap().is_dirty());
        app.execute("file.save", &json!({"path": target, "overwrite": true})).unwrap();
        assert_ne!(page_on_disk(&path), "CLI VENCEU");
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn refresh_keeps_local_edits_and_retries_once_undone() {
    let path = temp_file("refresh-conflict");
    let (mut first, mut second) = two_windows(&path);
    second.execute("shape.ellipse", &json!({"width": 30, "height": 30})).unwrap();
    let local = second.doc().unwrap().clone();
    first.execute("page.rename", &json!({"name": "Saved elsewhere"})).unwrap();
    first.execute("file.save", &json!({})).unwrap();
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
    assert_eq!(second.doc().unwrap(), &local);
    assert!(second.execute("file.save", &json!({})).is_err());
    second.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"].as_array().unwrap().len(), 1);
    assert_eq!(second.undo_depth(), (0, 0), "old redo must not restore a stale document");
    assert_eq!(second.doc().unwrap().node(second.page().unwrap()).unwrap().name, "Saved elsewhere");
    let _ = std::fs::remove_file(path);
}

#[test]
fn refresh_never_ends_a_gesture_or_version_preview() {
    let path = temp_file("refresh-busy");
    let (mut first, mut second) = two_windows(&path);
    second.begin_interaction("Drag").unwrap();
    first.execute("page.rename", &json!({"name": "External"})).unwrap();
    first.execute("file.save", &json!({})).unwrap();
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
    assert!(second.ds().unwrap().interaction.is_some());
    second.cancel_interaction().unwrap();
    let ds = second.ds_mut().unwrap();
    ds.start_preview("test", "Preview", 0, ds.doc.clone());
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
    assert!(second.ds().unwrap().preview.is_some());
    second.ds_mut().unwrap().end_preview();
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_or_broken_replacements_leave_the_open_document_intact() {
    let path = temp_file("refresh-broken");
    let (first, mut second) = two_windows(&path);
    let before = second.ds().unwrap().doc.clone();
    std::fs::write(&path, b"broken replacement").unwrap();
    let result = second.execute("file.refresh", &json!({})).unwrap();
    assert_eq!(result["refreshed"], json!([]));
    assert!(!result["errors"].as_array().unwrap().is_empty());
    assert!(std::sync::Arc::ptr_eq(&second.ds().unwrap().doc, &before));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
    assert_eq!(first.doc().unwrap(), second.doc().unwrap());
}

#[test]
fn opening_a_path_alias_reuses_the_tab_and_save_as_cannot_overwrite_an_open_tab() {
    let path = temp_file("path-alias");
    let (mut first, _) = two_windows(&path);
    let uid = first.ds().unwrap().uid;
    let alias = path.replace("TESTE.figcraft", "./TESTE.figcraft");
    first.execute("file.open", &json!({"path": alias})).unwrap();
    assert_eq!(first.docs.len(), 1);
    assert_eq!(first.ds().unwrap().uid, uid);
    first.execute("file.new", &json!({})).unwrap();
    first.execute("shape.ellipse", &json!({"width": 20, "height": 20})).unwrap();
    assert!(first.execute("file.saveAs", &json!({"path": alias})).unwrap_err().to_string().contains("already open"));
    assert!(first.ds().unwrap().path.is_none());
    assert!(first.ds().unwrap().is_dirty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn same_size_replacements_in_the_same_timestamp_tick_are_not_missed() {
    let path = temp_file("same-timestamp");
    let (mut first, mut second) = two_windows(&path);
    let stamp = second.ds().unwrap().on_disk.clone().unwrap();
    first.execute("page.rename", &json!({"name": "PAGE 2"})).unwrap();
    first.execute("file.save", &json!({})).unwrap();
    // Reproduce a coarse timestamp filesystem deterministically, with no timing assumptions.
    let modified = stamp.modified.unwrap();
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
    let now = crate::library::stat(&path).unwrap();
    assert_eq!(stamp.modified, now.modified);
    assert_ne!(stamp.identity, now.identity);
    assert!(second.execute("file.save", &json!({})).is_err());
    assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"].as_array().unwrap().len(), 1);
    assert_eq!(second.doc().unwrap().node(second.page().unwrap()).unwrap().name, "PAGE 2");
    let _ = std::fs::remove_file(path);
}

#[test]
fn refresh_preserves_text_previews_live_documents_and_clean_branch_tabs() {
    for mode in ["text-preview", "live", "branch"] {
        let path = temp_file(mode);
        let (mut first, mut second) = two_windows(&path);
        match mode {
            "text-preview" => {
                let ds = second.ds_mut().unwrap();
                ds.text_preview = Some(ds.doc.clone());
            }
            "live" => {
                let script = std::sync::Arc::new(std::sync::Mutex::new(crate::live_tests::Script::default()));
                second.live.install(Box::new(crate::live_tests::Mock(script)));
                second.execute("collab.host", &json!({})).unwrap();
                assert!(second.live().doc_uid.is_some());
            }
            "branch" => {
                second.execute("branch.create", &json!({"name": "Local branch"})).unwrap();
                second.execute("file.save", &json!({})).unwrap();
                assert!(!second.docs[0].is_dirty());
                first.execute("file.revert", &json!({})).unwrap();
            }
            _ => panic!("unknown test mode: {mode}"),
        }
        let baseline = second.docs[0].doc.clone();
        let uid = second.docs[0].uid;
        first.execute("page.rename", &json!({"name": "Changed outside"})).unwrap();
        first.execute("file.save", &json!({})).unwrap();
        assert_eq!(second.execute("file.refresh", &json!({})).unwrap()["refreshed"], json!([]));
        assert!(std::sync::Arc::ptr_eq(&baseline, &second.docs[0].doc));
        assert_eq!(second.docs[0].uid, uid);
        if mode == "text-preview" {
            assert!(second.ds().unwrap().text_preview.is_some(), "background refresh never ends a preview");
        }
        if mode == "branch" {
            assert_eq!(second.docs.len(), 2);
            assert!(second.ds().unwrap().branch_of.is_some());
        }
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn automatic_refresh_backs_off_but_explicit_refresh_bypasses_the_deadline() {
    let path = temp_file("refresh-backoff");
    let (mut writer, mut reader) = two_windows(&path);
    writer.execute("page.rename", &json!({"name": "Recovered"})).unwrap();
    writer.execute("file.save", &json!({})).unwrap();
    let valid = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, vec![b'!'; valid.len()]).unwrap();
    let reset_time = || {
        std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
    };
    reset_time();
    assert!(!reader.execute("file.refresh", &json!({"automatic": true})).unwrap()["errors"].as_array().unwrap().is_empty());
    let failed = reader.ds().unwrap().refresh_failure.clone().unwrap();
    std::fs::write(&path, valid).unwrap();
    reset_time();
    assert_eq!(failed.stamp, crate::file_access::stamp(&path).unwrap(), "recovery does not change the observed stamp");
    let suppressed = reader.execute("file.refresh", &json!({"automatic": true})).unwrap();
    assert_eq!(suppressed["refreshed"], json!([]));
    assert_eq!(suppressed["errors"], json!([]));
    assert_eq!(reader.ds().unwrap().refresh_failure.as_ref().unwrap().retry_at, failed.retry_at);
    assert_eq!(reader.execute("file.refresh", &json!({})).unwrap()["refreshed"].as_array().unwrap().len(), 1);
    assert!(reader.ds().unwrap().refresh_failure.is_none());
    assert_eq!(reader.doc().unwrap().node(reader.page().unwrap()).unwrap().name, "Recovered");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_new_stamp_bypasses_backoff_and_busy_failures_do_not_spin() {
    let path = temp_file("refresh-backoff-generation");
    let (mut writer, mut reader) = two_windows(&path);
    std::fs::write(&path, b"broken").unwrap();
    reader.execute("file.refresh", &json!({"automatic": true})).unwrap();
    let before = reader.ds().unwrap().refresh_failure.clone().unwrap();
    writer.execute("page.rename", &json!({"name": "New replacement"})).unwrap();
    writer.execute("file.save", &json!({"overwrite": true})).unwrap();
    assert_eq!(reader.execute("file.refresh", &json!({"automatic": true})).unwrap()["refreshed"].as_array().unwrap().len(), 1);
    assert!(reader.ds().unwrap().refresh_failure.is_none());
    assert_ne!(before.stamp, reader.ds().unwrap().on_disk.clone().unwrap());
    std::fs::write(&path, b"broken again").unwrap();
    reader.execute("file.refresh", &json!({"automatic": true})).unwrap();
    reader.ds_mut().unwrap().refresh_failure.as_mut().unwrap().retry_at = web_time::Instant::now() - std::time::Duration::from_secs(1);
    reader.begin_interaction("Drag").unwrap();
    let deferred = reader.execute("file.refresh", &json!({"automatic": true})).unwrap();
    assert_eq!(deferred["errors"], json!([]));
    assert!(reader.ds().unwrap().refresh_failure.as_ref().unwrap().retry_at > web_time::Instant::now());
    assert!(reader.ds().unwrap().interaction.is_some());
    let _ = std::fs::remove_file(path);
}
