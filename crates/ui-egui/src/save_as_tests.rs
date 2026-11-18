//! Native Save as accepts both file formats; .fig copies preserve native save state.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use serde_json::json;

use crate::dialogs::file_saved;
use crate::{DialogFuture, FigApp, Services};

type Picks = Arc<Mutex<Vec<(String, String, Vec<String>)>>>;

struct Picker {
    path: Option<PathBuf>,
    picks: Picks,
}

impl Services for Picker {
    fn pick_save(&mut self, title: &str, name: &str, exts: &[&str]) -> DialogFuture {
        self.picks.lock().unwrap().push((title.into(), name.into(), exts.iter().map(|e| (*e).into()).collect()));
        Box::pin(std::future::ready(self.path.take()))
    }
}

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("figcraft-save-as-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Ok(files) = std::fs::read_dir(&self.0) {
            for file in files.flatten() {
                let _ = std::fs::remove_file(file.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn session() -> Session {
    let mut s = Session::new();
    s.doc_mut().unwrap().name = "Prototype".into();
    s.execute("shape.rectangle", &json!({"width": 100, "height": 80})).unwrap();
    s
}

fn picker(path: Option<PathBuf>) -> (Picker, Picks) {
    let picks = Picks::default();
    (Picker { path, picks: picks.clone() }, picks)
}

fn app(path: Option<PathBuf>) -> (FigApp, Picks) {
    let (picker, picks) = picker(path);
    (FigApp::new(session(), Box::new(picker)), picks)
}

fn toast(app: &FigApp) -> &str {
    &app.ui.toast.as_ref().unwrap().0
}

fn expected_picker() -> Vec<(String, String, Vec<String>)> {
    vec![("Save as".into(), "Prototype.figcraft".into(), vec!["figcraft".into(), "fig".into()])]
}

#[test]
fn save_as_menu_and_first_save_open_the_platform_picker_directly_with_both_extensions() {
    for action in ["direct", "menu", "first"] {
        let (mut app, picks) = app(None);
        let before = app.session.doc().unwrap().clone();
        match action {
            "direct" => app.save_as_dialog(),
            "menu" => crate::menus::invoke(&mut app, "file.saveAs"),
            "first" => app.save(),
            _ => panic!("unknown save action {action}"),
        }
        app.poll_dialog(&egui::Context::default());
        assert_eq!(*picks.lock().unwrap(), expected_picker());
        assert_eq!(app.session.doc().unwrap(), &before);
        assert!(app.session.ds().unwrap().path.is_none());
        assert!(app.session.ds().unwrap().is_dirty());
        assert!(app.ui.toast.is_none());
    }
}

#[test]
fn a_pathless_engine_request_uses_the_same_platform_picker() {
    let (mut app, picks) = app(None);
    let before = app.session.doc().unwrap().clone();
    assert!(app.run("file.saveAs", json!({})).is_none(), "the engine requests the UI and refuses a pathless programmatic save");
    app.poll_dialog(&egui::Context::default());
    assert_eq!(*picks.lock().unwrap(), expected_picker());
    assert_eq!(app.session.doc().unwrap(), &before);
    assert!(app.session.ds().unwrap().path.is_none());
    assert!(app.session.ds().unwrap().is_dirty());
}

#[test]
fn native_paths_and_an_extensionless_default_save_native_files_and_show_the_written_name() {
    let temp = Temp::new();
    for (chosen, written) in [("native", "native.figcraft"), ("saved.figcraft", "saved.figcraft"), ("upper.FIGCRAFT", "upper.FIGCRAFT")] {
        let (mut app, picks) = app(Some(temp.path(chosen)));
        app.save_as_dialog();
        app.poll_dialog(&egui::Context::default());
        let path = temp.path(written);
        assert_eq!(*picks.lock().unwrap(), expected_picker());
        assert!(path.is_file());
        assert_eq!(app.session.ds().unwrap().path.as_deref(), path.to_str());
        assert!(!app.session.ds().unwrap().is_dirty());
        assert_eq!(toast(&app), format!("Saved {written}"));
        let mut reopened = Session::empty();
        reopened.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(reopened.doc().unwrap().nodes().count(), app.session.doc().unwrap().nodes().count());
        assert_eq!(reopened.doc().unwrap().name, "Prototype");
    }
}

#[test]
fn returned_fig_paths_export_real_copies_without_changing_native_save_state() {
    let temp = Temp::new();
    for name in ["copy.fig", "upper.FIG"] {
        let path = temp.path(name);
        let (mut app, picks) = app(Some(path.clone()));
        let native = temp.path("working.figcraft").to_string_lossy().into_owned();
        app.session.ds_mut().unwrap().path = Some(native.clone());
        let before = app.session.doc().unwrap().clone();
        app.save_as_dialog();
        app.poll_dialog(&egui::Context::default());
        assert_eq!(*picks.lock().unwrap(), expected_picker());
        assert!(path.is_file());
        assert_eq!(app.session.doc().unwrap(), &before);
        assert_eq!(app.session.ds().unwrap().path.as_deref(), Some(native.as_str()));
        assert!(app.session.ds().unwrap().is_dirty());
        assert_eq!(toast(&app), format!("Exported {name}"));
        let mut reopened = Session::empty();
        reopened.execute("file.open", &json!({"path": path})).unwrap();
        assert_eq!(reopened.doc().unwrap().nodes().count(), before.nodes().count());
    }
}

#[test]
fn a_fig_copy_reports_actual_engine_warnings_and_keeps_the_original_content() {
    let temp = Temp::new();
    let (mut app, _) = app(Some(temp.path("comments.fig")));
    app.run("comment.add", json!({"x": 10, "y": 20, "text": "Synthetic comment"})).unwrap();
    let before = app.session.doc().unwrap().clone();
    app.save_as_dialog();
    app.poll_dialog(&egui::Context::default());
    assert!(temp.path("comments.fig").is_file());
    assert_eq!(toast(&app), "Exported comments.fig; 1 kind of content was left out");
    assert_eq!(app.session.doc().unwrap(), &before);
    assert_eq!(app.session.doc().unwrap().comments.len(), 1);
    assert!(app.session.ds().unwrap().path.is_none());
    assert!(app.session.ds().unwrap().is_dirty());
}

#[test]
fn cancelling_the_platform_dialog_preserves_document_selection_path_and_dirty_state() {
    let (mut app, picks) = app(None);
    app.session.ds_mut().unwrap().path = Some("working.figcraft".into());
    let before = app.session.doc().unwrap().clone();
    let selection = app.session.ds().unwrap().selection.clone();
    app.save_as_dialog();
    app.poll_dialog(&egui::Context::default());
    assert_eq!(*picks.lock().unwrap(), expected_picker());
    assert_eq!(app.session.doc().unwrap(), &before);
    assert_eq!(app.session.ds().unwrap().selection, selection);
    assert_eq!(app.session.ds().unwrap().path.as_deref(), Some("working.figcraft"));
    assert!(app.session.ds().unwrap().is_dirty());
    assert!(app.ui.toast.is_none());
}

#[test]
fn unsupported_extensions_are_rejected_without_overwriting_the_chosen_file() {
    let temp = Temp::new();
    for name in ["wrong.svg", "wrong.png"] {
        let path = temp.path(name);
        std::fs::write(&path, b"existing document").unwrap();
        let (mut app, _) = app(Some(path.clone()));
        let before = app.session.doc().unwrap().clone();
        app.save_as_dialog();
        app.poll_dialog(&egui::Context::default());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing document");
        assert_eq!(app.session.doc().unwrap(), &before);
        assert!(app.session.ds().unwrap().path.is_none());
        assert!(app.session.ds().unwrap().is_dirty());
        assert_eq!(toast(&app), "Use the .figcraft or .fig extension.");
    }
}

#[test]
fn adding_the_default_extension_never_overwrites_another_existing_path_without_confirmation() {
    let temp = Temp::new();
    let path = temp.path("existing.figcraft");
    std::fs::write(&path, b"existing document").unwrap();
    let (mut app, _) = app(Some(temp.path("existing")));
    app.save_as_dialog();
    app.poll_dialog(&egui::Context::default());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing document");
    assert!(!temp.path("existing").exists());
    assert!(app.session.ds().unwrap().path.is_none());
    assert!(app.session.ds().unwrap().is_dirty());
    assert_eq!(toast(&app), "A .figcraft file with this name already exists. Choose its full name to confirm replacement.");
}

#[test]
fn fig_save_results_report_the_actual_name_and_plural_warning_count_without_marking_native_state_saved() {
    let (mut app, _) = app(None);
    let before = app.session.doc().unwrap().clone();
    app.session.ds_mut().unwrap().path = Some("working.figcraft".into());
    file_saved(&mut app, &json!({"format": "fig", "path": "written.fig", "warnings": ["synthetic warning", "another synthetic warning"]}));
    assert_eq!(toast(&app), "Exported written.fig; 2 kinds of content were left out");
    assert_eq!(app.session.doc().unwrap(), &before);
    assert_eq!(app.session.ds().unwrap().path.as_deref(), Some("working.figcraft"));
    assert!(app.session.ds().unwrap().is_dirty());
}

#[test]
fn save_with_a_native_path_keeps_using_it_without_a_dialog() {
    let temp = Temp::new();
    let (mut app, picks) = app(None);
    let path = temp.path("working.figcraft");
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.run("shape.rectangle", json!({"width": 10, "height": 20})).unwrap();
    assert!(app.session.ds().unwrap().is_dirty());
    app.save();
    assert!(picks.lock().unwrap().is_empty());
    assert_eq!(app.session.ds().unwrap().path.as_deref(), path.to_str());
    assert!(!app.session.ds().unwrap().is_dirty());
    assert_eq!(toast(&app), "Saved");
}

#[test]
fn saving_legacy_fig_paths_keeps_the_success_toast_and_original_source_bytes() {
    for legacy_name in ["legacy.fig", "legacy.FIG"] {
        for destination in ["corrected.figcraft", "copy.fig"] {
            let temp = Temp::new();
            let target = temp.path(destination);
            let legacy = temp.path(legacy_name);
            let (mut app, picks) = app(Some(target.clone()));
            let original = temp.path("original.figcraft");
            app.run("file.saveAs", json!({"path": original})).unwrap();
            std::fs::rename(&original, &legacy).unwrap();
            let legacy_path = legacy.to_string_lossy().into_owned();
            app.session.ds_mut().unwrap().path = Some(legacy_path.clone());
            app.run("shape.rectangle", json!({"width": 10, "height": 20})).unwrap();
            let before = app.session.doc().unwrap().clone();
            let legacy_bytes = std::fs::read(&legacy).unwrap();
            app.save();
            app.poll_dialog(&egui::Context::default());
            assert_eq!(*picks.lock().unwrap(), expected_picker());
            assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
            assert!(target.is_file());
            assert!(app.ui.last_error.is_none());
            assert_eq!(app.session.doc().unwrap(), &before);
            if destination == "corrected.figcraft" {
                assert_eq!(toast(&app), "Saved corrected.figcraft");
                assert_eq!(app.session.ds().unwrap().path.as_deref(), target.to_str());
                assert!(!app.session.ds().unwrap().is_dirty());
            } else {
                assert_eq!(toast(&app), "Exported copy.fig");
                assert_eq!(app.session.ds().unwrap().path.as_deref(), Some(legacy_path.as_str()));
                assert!(app.session.ds().unwrap().is_dirty());
            }
        }
    }
}

#[test]
fn failed_writes_surface_the_engine_error_and_keep_the_document_dirty() {
    let temp = Temp::new();
    for extension in ["figcraft", "fig"] {
        let path = temp.path(&format!("missing/copy.{extension}"));
        let (mut app, _) = app(Some(path.clone()));
        let before = app.session.doc().unwrap().clone();
        app.save_as_dialog();
        app.poll_dialog(&egui::Context::default());
        assert!(!path.exists());
        assert_eq!(app.session.doc().unwrap(), &before);
        assert!(app.session.ds().unwrap().path.is_none());
        assert!(app.session.ds().unwrap().is_dirty());
        assert_eq!(app.ui.last_error.as_deref(), Some(toast(&app)));
        assert!(!toast(&app).starts_with("Saved") && !toast(&app).starts_with("Exported"));
    }
}

#[test]
fn an_empty_destination_is_rejected_without_writing_or_changing_save_state() {
    let (mut app, _) = app(Some(PathBuf::new()));
    let before = app.session.doc().unwrap().clone();
    app.save_as_dialog();
    app.poll_dialog(&egui::Context::default());
    assert_eq!(app.session.doc().unwrap(), &before);
    assert!(app.session.ds().unwrap().path.is_none());
    assert!(app.session.ds().unwrap().is_dirty());
    assert_eq!(toast(&app), "Use the .figcraft or .fig extension.");
}

#[test]
fn save_as_with_no_document_never_opens_a_dialog() {
    let (picker, picks) = picker(None);
    let mut app = FigApp::new(Session::empty(), Box::new(picker));
    app.save_as_dialog();
    app.poll_dialog(&egui::Context::default());
    assert!(picks.lock().unwrap().is_empty());
    assert!(app.ui.toast.is_none());
}
