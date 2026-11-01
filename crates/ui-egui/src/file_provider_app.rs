//! Native archives and document-scoped persistence acknowledgements.
use crate::{FigApp, file_providers::Action};
use base64::Engine as _;
use serde_json::json;

pub(crate) fn save(app: &mut FigApp, save_as: bool) -> Result<(), String> {
    let host = app.file_providers.clone();
    let tab = figcraft_engine::branches::file_tab(&app.session);
    let doc = app.session.docs.get(tab).ok_or("no document")?;
    if host.pending_for(doc.uid) {
        return Err("a provider save is already pending for this document".into());
    }
    let uid = doc.uid;
    let name = host.bound_filename(uid).unwrap_or_else(|| native_filename(&doc.doc.name));
    // The engine produces its complete native archive, including assets, versions and branches.
    // Asking for data does not mark the document saved or assign a path.
    let result = app.session.execute("file.saveAs", &json!({"data":true})).map_err(|e| e.to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(result["data"].as_str().ok_or("native encoder returned no project")?)
        .map_err(|e| e.to_string())?;
    let snapshots: Vec<_> =
        app.session.docs.iter().filter(|d| d.uid == uid || d.branch_of.as_ref().is_some_and(|(main, _)| *main == uid)).cloned().collect();
    let revision = snapshots.iter().find(|d| d.uid == uid).map(|d| d.revision).ok_or("document closed during native capture")?;
    host.begin(snapshots, uid, revision, name, bytes, save_as)?;
    app.toast("Choose where to save the native project");
    Ok(())
}
fn native_filename(value: &str) -> String {
    const EXTENSION: &str = ".figcraft";
    let mut stem = String::new();
    for c in value.chars() {
        let c = if c.is_control() || matches!(c, '/' | '\\') { '_' } else { c };
        if stem.len() + c.len_utf8() + EXTENSION.len() > 255 {
            break;
        }
        stem.push(c);
    }
    if stem.trim().is_empty() {
        stem = "Untitled".into();
    }
    stem.push_str(EXTENSION);
    stem
}
pub(crate) fn tick(app: &mut FigApp) {
    let host = app.file_providers.clone();
    if !host.connected() {
        return;
    }
    for action in host.take_actions() {
        match action {
            Action::Save { request, capture, status, resource } => {
                let alive = app.session.docs.iter().any(|d| d.uid == capture.doc);
                if status == "saved" {
                    for alias in host.unbind_resource_aliases(capture.doc, &resource) {
                        let affected =
                            app.session.docs.iter_mut().filter(|d| d.uid == alias || d.branch_of.as_ref().is_some_and(|(main, _)| *main == alias));
                        for doc in affected {
                            doc.mark_unsaved();
                        }
                    }
                    for snapshot in &capture.snapshots {
                        if let Some(doc) = app.session.docs.iter_mut().find(|d| d.uid == snapshot.uid) {
                            doc.mark_saved_capture(snapshot);
                            doc.path = None;
                        }
                    }
                    app.toast("Saved with file provider");
                } else {
                    app.ui.close_after_save_as = None;
                    if status == "failed" {
                        app.toast("File provider save failed; the document is still unsaved");
                    }
                }
                host.finish_save(&request, &capture, &status, &resource, alive);
            }
            Action::Open { request, resource, name, bytes } => {
                let result = if let Some(uid) = host.bound_document(&resource).filter(|uid| app.session.docs.iter().any(|d| d.uid == *uid)) {
                    if let Some(index) = app.session.docs.iter().position(|d| d.uid == uid) {
                        app.session.active = index;
                    }
                    Ok(uid)
                } else {
                    app.run_user("file.open", json!({"name":name,"data":base64::engine::general_purpose::STANDARD.encode(&bytes)}))
                        .and_then(|_| app.session.ds().ok().map(|d| d.uid))
                        .ok_or_else(|| "native project could not be opened".to_string())
                };
                if result.is_ok() {
                    app.ui.home = false;
                }
                host.finish_open(&request, &resource, &name, result);
            }
        }
    }
    host.refresh(app.session.docs.iter().map(|d| (d.uid, d.revision, d.doc.name.clone(), d.is_dirty())).collect());
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use figcraft_engine::Session;
    struct NoServices;
    impl crate::Services for NoServices {}
    fn app() -> FigApp {
        let mut session = Session::new();
        session.execute("file.new", &json!({})).unwrap();
        let app = FigApp::new(session, Box::new(NoServices));
        app.file_providers.set_session("0123456789abcdef0123456789abcdef".into()).unwrap();
        app.file_providers.connect("test".into(), "Test storage".into(), "account".into()).unwrap();
        app
    }
    fn request(app: &mut FigApp) -> String {
        save(app, false).unwrap();
        let events = app.file_providers.take_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].metadata["type"], "saveDialog");
        events[0].metadata["requestId"].as_str().unwrap().to_string()
    }
    #[test]
    fn durable_overwrite_invalidates_main_and_branch_checkpoints_before_reopening() {
        for queued_together in [false, true] {
            for later_edit in [false, true] {
                let mut app = app();
                let first = app.session.ds().unwrap().uid;
                app.session.execute("branch.create", &json!({"name":"First branch"})).unwrap();
                let branch = app.session.ds().unwrap().uid;
                let first_request = request(&mut app);
                app.file_providers.resolve_dialog(&first_request, "provider", "shared.figcraft").unwrap();
                app.file_providers.take_events();
                app.session.execute("file.new", &json!({})).unwrap();
                let second = app.session.ds().unwrap().uid;
                std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Second".into();
                let second_request = request(&mut app);
                app.file_providers.resolve_dialog(&second_request, "provider", "shared.figcraft").unwrap();
                let bytes = app.file_providers.take_events().remove(0).bytes.unwrap();
                if later_edit {
                    std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Second after capture".into();
                }
                let before: Vec<_> = app.session.docs.iter().map(|doc| (doc.uid, doc.revision, doc.doc.clone())).collect();
                app.file_providers.result(first_request, "saved".into(), "shared".into()).unwrap();
                if !queued_together {
                    tick(&mut app);
                }
                app.file_providers.result(second_request, "saved".into(), "shared".into()).unwrap();
                tick(&mut app);
                assert_eq!(app.session.ds().unwrap().uid, second);
                for uid in [first, branch] {
                    assert!(app.session.docs.iter().find(|doc| doc.uid == uid).unwrap().is_dirty());
                    assert!(app.file_providers.bound_filename(uid).is_none());
                }
                assert_eq!(app.session.ds().unwrap().is_dirty(), later_edit);
                assert_eq!(app.file_providers.bound_document("shared"), Some(second));
                for (uid, revision, content) in before {
                    let doc = app.session.docs.iter().find(|doc| doc.uid == uid).unwrap();
                    assert_eq!(doc.revision, revision);
                    assert!(std::sync::Arc::ptr_eq(&doc.doc, &content));
                }
                app.session.active = 0;
                let read = app.file_providers.request_open("shared").unwrap();
                app.file_providers.open(read, "shared.figcraft".into(), bytes).unwrap();
                tick(&mut app);
                assert_eq!(app.session.docs.len(), 3);
                assert_eq!(app.session.ds().unwrap().uid, second);
                assert_eq!(app.session.ds().unwrap().is_dirty(), later_edit);
            }
        }
    }

    struct Downloads(std::rc::Rc<std::cell::Cell<usize>>);
    impl crate::Services for Downloads {
        fn downloads(&self) -> bool {
            true
        }
        fn download(&mut self, _: &str, _: &[u8]) -> bool {
            self.0.set(self.0.get() + 1);
            true
        }
    }

    #[test]
    fn paused_bound_saves_including_branches_do_not_fall_back_to_downloads() {
        for on_branch in [false, true] {
            let mut app = app();
            let request = request(&mut app);
            app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
            app.file_providers.take_events();
            app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
            tick(&mut app);
            app.file_providers.take_events();
            let main = app.session.ds().unwrap().uid;
            if on_branch {
                app.session.execute("branch.create", &json!({"name":"Working branch"})).unwrap();
            }
            std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Unsaved provider edit".into();
            let downloads = std::rc::Rc::new(std::cell::Cell::new(0));
            app.services = Box::new(Downloads(downloads.clone()));
            app.file_providers.set_enabled(false).unwrap();
            app.save();
            app.save_as_dialog();
            assert_eq!(downloads.get(), 0);
            assert!(app.session.ds().unwrap().is_dirty());
            assert!(app.session.ds().unwrap().path.is_none());
            assert_eq!(app.file_providers.bound_document("file-1"), Some(main));
            assert!(app.file_providers.take_events().is_empty());
            assert!(!app.ui.quit);
            app.file_providers.set_enabled(true).unwrap();
            assert_eq!(app.file_providers.bound_document("file-1"), Some(main));
            assert!(app.session.ds().unwrap().is_dirty());
        }
    }

    #[test]
    fn pausing_an_unbound_pending_save_does_not_enable_a_fallback_save() {
        for status in ["failed", "cancel", "saved"] {
            let mut app = app();
            std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Captured".into();
            let request = request(&mut app);
            app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
            app.file_providers.take_events();
            let downloads = std::rc::Rc::new(std::cell::Cell::new(0));
            app.services = Box::new(Downloads(downloads.clone()));
            app.file_providers.set_enabled(false).unwrap();
            app.save();
            app.save_as_dialog();
            assert_eq!(downloads.get(), 0);
            assert!(app.session.ds().unwrap().is_dirty());
            assert!(app.file_providers.take_events().is_empty());
            app.file_providers.result(request, status.into(), if status == "saved" { "file-1".into() } else { String::new() }).unwrap();
            tick(&mut app);
            assert_eq!(app.session.ds().unwrap().is_dirty(), status != "saved");
        }
    }

    #[test]
    fn native_fallback_filename_is_safe_and_bounded() {
        let name = native_filename("folder/notes\\draft\n");
        assert_eq!(name, "folder_notes_draft_.figcraft");
        let unicode = native_filename(&"é".repeat(300));
        assert!(unicode.len() <= 255);
        assert!(unicode.ends_with(".figcraft"));
        assert_eq!(native_filename("   "), "Untitled.figcraft");
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "folder/notes\\draft\n".into();
        save(&mut app, false).unwrap();
        let event = app.file_providers.take_events().remove(0);
        assert_eq!(event.metadata["type"], "saveDialog");
        assert_eq!(event.metadata["filename"], name);
    }

    #[test]
    fn bound_filename_is_preserved_when_the_document_name_changes() {
        let mut app = app();
        let first = request(&mut app);
        app.file_providers.resolve_dialog(&first, "provider", "Chosen.figcraft").unwrap();
        app.file_providers.take_events();
        app.file_providers.result(first, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        app.file_providers.take_events();
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "invalid/name".into();
        save(&mut app, false).unwrap();
        let event = app.file_providers.take_events().remove(0);
        assert_eq!(event.metadata["type"], "saveDialog");
        assert_eq!(event.metadata["filename"], "Chosen.figcraft");
    }

    fn captured_file_with_a_later_unsynced_branch_edit() -> (FigApp, u64, u64) {
        let mut app = app();
        app.session.execute("branch.create", &json!({"name":"Working branch"})).unwrap();
        let branch = app.session.ds().unwrap().uid;
        let main = app.session.ds().unwrap().branch_of.as_ref().unwrap().0;
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "provider", "Saved.figcraft").unwrap();
        app.file_providers.take_events();
        let doc = app.session.docs.iter_mut().find(|doc| doc.uid == branch).unwrap();
        std::sync::Arc::make_mut(&mut doc.doc).name = "Later branch edit".into();
        app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        assert!(!app.session.docs.iter().find(|doc| doc.uid == main).unwrap().is_dirty());
        assert!(app.session.docs.iter().find(|doc| doc.uid == branch).unwrap().is_dirty());
        (app, main, branch)
    }

    #[test]
    fn both_close_entrypoints_keep_post_capture_branch_edits_open() {
        for system_close in [false, true] {
            let (mut app, main, branch) = captured_file_with_a_later_unsynced_branch_edit();
            let ctx = egui::Context::default();
            let mut input = egui::RawInput::default();
            if system_close {
                input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() });
            }
            let mut output = ctx.run_ui(input, |ui| {
                if system_close {
                    crate::windows::on_close_request(&mut app, ui.ctx());
                } else {
                    crate::windows::request_close(&mut app, ui.ctx());
                }
            });
            output.textures_delta.clear();
            assert!(app.ui.confirm_close);
            assert!(app.session.docs.iter().find(|doc| doc.uid == main).unwrap().is_dirty());
            assert!(app.session.docs.iter().find(|doc| doc.uid == branch).unwrap().is_dirty());
        }
    }

    #[test]
    fn close_save_continuation_refuses_post_capture_branch_edits() {
        let (mut app, main, branch) = captured_file_with_a_later_unsynced_branch_edit();
        app.ui.close_after_save_as = Some(main);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| crate::windows::confirm_close(&mut app, ui.ctx()));
        output.textures_delta.clear();
        assert!(app.ui.close_after_save_as.is_none());
        assert!(!app.ui.quit);
        assert!(app.session.docs.iter().find(|doc| doc.uid == main).unwrap().is_dirty());
        assert!(app.session.docs.iter().find(|doc| doc.uid == branch).unwrap().is_dirty());
    }

    #[test]
    fn provider_ack_cannot_clear_edits_made_after_capture() {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Before capture".into();
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
        let events = app.file_providers.take_events();
        assert!(!events[0].bytes.as_ref().unwrap().is_empty());
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "After capture".into();
        app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        assert!(app.session.ds().unwrap().is_dirty());
    }
    #[test]
    fn successful_ack_updates_the_captured_document_not_the_current_tab() {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Before capture".into();
        let uid = app.session.ds().unwrap().uid;
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
        let captured_revision = app.session.ds().unwrap().revision;
        let captured_document = app.session.ds().unwrap().doc.clone();
        app.session.execute("file.new", &json!({})).unwrap();
        assert_ne!(app.session.ds().unwrap().uid, uid);
        let original = app.session.docs.iter().find(|d| d.uid == uid).unwrap();
        assert!(original.revision > captured_revision);
        assert!(std::sync::Arc::ptr_eq(&original.doc, &captured_document));
        app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        assert!(!app.session.docs.iter().find(|d| d.uid == uid).unwrap().is_dirty());
        assert_eq!(app.file_providers.bound_document("file-1"), Some(uid));
    }
    #[test]
    fn provider_ack_keeps_later_versions_and_branches_unsaved() {
        for command in ["file.saveVersion", "branch.create"] {
            let mut app = app();
            std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Before capture".into();
            let uid = app.session.ds().unwrap().uid;
            let request = request(&mut app);
            app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
            app.session.execute(command, &json!({"name":"Later metadata"})).unwrap();
            app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
            tick(&mut app);
            let original = app.session.docs.iter().find(|d| d.uid == uid).unwrap();
            assert!(original.is_dirty(), "{command} metadata was not part of the durable capture");
            assert!(!original.versions.is_empty() || !original.branches.is_empty());
        }
    }

    #[test]
    fn provider_ack_keeps_later_branch_tab_edits_unsaved() {
        let mut app = app();
        app.session.execute("branch.create", &json!({"name":"Working branch"})).unwrap();
        let branch = app.session.ds().unwrap().uid;
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
        app.session.execute("shape.rectangle", &json!({"x":1,"y":2,"width":10,"height":10})).unwrap();
        app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        assert!(app.session.docs.iter().find(|d| d.uid == branch).unwrap().is_dirty());
    }

    #[test]
    fn exact_provider_capture_acknowledges_its_version_history() {
        let mut app = app();
        app.session.execute("file.saveVersion", &json!({"name":"Captured version"})).unwrap();
        assert!(app.session.ds().unwrap().is_dirty());
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "provider", "saved.figcraft").unwrap();
        app.file_providers.result(request, "saved".into(), "file-1".into()).unwrap();
        tick(&mut app);
        assert!(!app.session.ds().unwrap().is_dirty());
        assert_eq!(app.session.ds().unwrap().versions.len(), 1);
    }

    #[test]
    fn download_result_does_not_claim_durable_provider_storage() {
        let mut app = app();
        std::sync::Arc::make_mut(&mut app.session.ds_mut().unwrap().doc).name = "Before capture".into();
        let request = request(&mut app);
        app.file_providers.resolve_dialog(&request, "device", "local.figcraft").unwrap();
        app.file_providers.result(request, "download".into(), String::new()).unwrap();
        tick(&mut app);
        assert!(app.session.ds().unwrap().is_dirty());
        assert!(app.file_providers.snapshot()["resources"].as_array().unwrap().is_empty());
    }
}
