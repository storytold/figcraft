//! A control request naming its `document` acts on that file, and the tab in front stays in front.

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::{FigApp, NoServices};

/// Two files open: "Back" behind "Front", which is in front. Returns the app and Back's id.
fn two_files() -> (FigApp, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Front"})).unwrap();
    s.docs[0].doc_mut().name = "Back".into();
    let back = s.docs[0].uid;
    (FigApp::new(s, Box::new(NoServices)), back)
}

fn handle(app: &mut FigApp, method: &str, p: Value) -> Result<Value, String> {
    super::handle(app, &egui::Context::default(), method, &p)
}

fn layers(app: &FigApp, i: usize) -> usize {
    let ds = &app.session.docs[i];
    ds.doc.node(ds.page).map_or(0, |p| p.children.len())
}

#[test]
fn a_call_naming_the_back_file_edits_it_and_leaves_the_front_one_alone() {
    let (mut app, back) = two_files();
    let rect = json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 20, "height": 10}, "document": back});
    handle(&mut app, "engine.execute", rect).unwrap();
    assert_eq!((layers(&app, 0), layers(&app, 1)), (1, 0), "the rectangle landed in Back only");
    assert_eq!(app.session.active, 1, "Front stays in front");
    let doc = handle(&mut app, "document.inspect", json!({"depth": 1, "document": back})).unwrap();
    assert_eq!((doc["name"].as_str(), doc["id"].as_u64()), (Some("Back"), Some(back)));
    let listed = handle(&mut app, "engine.execute", json!({"command": "doc.list"})).unwrap();
    assert_eq!(listed[0]["id"], json!(back));
    assert!(handle(&mut app, "ui.render", json!({"document": back})).is_ok());
    // An open file's path names it too.
    app.session.docs[0].path = Some("/work/back.figcraft".into());
    let doc = handle(&mut app, "document.inspect", json!({"depth": 0, "document": "/work/back.figcraft"})).unwrap();
    assert_eq!(doc["id"], json!(back));
    assert_eq!(app.session.active, 1);
}

#[test]
fn a_file_that_is_not_open_or_a_ui_method_on_the_back_file_is_an_error() {
    let (mut app, back) = two_files();
    let e = handle(&mut app, "document.inspect", json!({"document": 999_999})).unwrap_err();
    assert!(e.contains("no open document 999999"), "{e}");
    let e = handle(&mut app, "ui.type", json!({"text": "x", "document": back})).unwrap_err();
    assert!(e.contains("acts on the tab in front"), "{e}");
    let front = app.session.docs[1].uid;
    assert!(handle(&mut app, "ui.inspect", json!({"document": front})).is_ok(), "the file in front may be named");
    assert_eq!(app.session.active, 1);
}

#[test]
fn a_command_that_opens_a_file_still_brings_it_to_the_front() {
    let (mut app, back) = two_files();
    handle(&mut app, "engine.execute", json!({"command": "file.new", "document": back})).unwrap();
    assert_eq!(app.session.active, 2, "a new file opens in front");
}

#[test]
fn explicitly_switching_to_the_named_document_keeps_it_in_front() {
    // Selecting a file temporarily brought forward still counts as an explicit switch.
    let (mut app, back) = two_files();
    handle(&mut app, "engine.execute", json!({"command": "doc.switch", "params": {"index": 0}, "document": back})).unwrap();
    assert_eq!(app.session.active, 0);
    assert!(!app.ui.home);
}

#[test]
fn a_failed_foreground_command_leaves_the_front_document_in_place() {
    // A failed targeted command must not switch the user to another file.
    let (mut app, back) = two_files();
    for (command, params) in
        [("doc.switch", json!({"index": 999})), ("proto.present", json!({"start": 999})), ("proto.preview", json!({"start": 999}))]
    {
        assert!(handle(&mut app, "engine.execute", json!({"command": command, "params": params, "document": back})).is_err());
        assert_eq!(app.session.active, 1);
        assert!(app.ui.present.is_none());
        assert!(app.ui.preview.is_none());
    }
}

#[test]
fn presenting_a_named_document_uses_its_frames_and_retargets_the_player() {
    // Identical layer IDs in different files must not mix up their prototypes.
    let (mut app, back) = two_files();
    let create = |app: &mut FigApp, document: u64, name: &str| {
        handle(app, "engine.execute", json!({"document": document, "command": "frame.create", "params": {"name": name, "width": 200, "height": 200}}))
            .unwrap()["id"]
            .clone()
    };
    let front = app.session.docs[1].uid;
    let front_frame = create(&mut app, front, "Front frame");
    let back_frame = create(&mut app, back, "Back frame");
    let back_second = create(&mut app, back, "Back second");
    assert_eq!(front_frame, back_frame, "exercise colliding frame IDs");
    for (document, start, owner) in [(front, front_frame, 1), (back, back_frame, 0), (back, back_second, 0)] {
        handle(&mut app, "engine.execute", json!({"document": document, "command": "proto.present", "params": {"start": start}})).unwrap();
        assert_eq!(app.session.active, owner);
        assert_eq!(app.ui.present.as_ref().unwrap().start().0, start.as_u64().unwrap());
        let active = app.ui.present_active.unwrap();
        assert_eq!(app.ui.presents.iter().find(|t| t.id == active).unwrap().doc_uid, document);
    }
}

#[test]
fn preview_opens_on_the_requested_file_instead_of_behind_its_presenter() {
    let (mut app, back) = two_files();
    let frame = handle(&mut app, "engine.execute", json!({"document": back, "command": "frame.create", "params": {"width": 200, "height": 200}}))
        .unwrap()["id"]
        .clone();
    app.session.active = 0;
    handle(&mut app, "engine.execute", json!({"command": "proto.present", "params": {"start": frame}})).unwrap();
    assert!(app.ui.present.is_some());
    handle(&mut app, "engine.execute", json!({"document": back, "command": "proto.preview", "params": {"start": frame}})).unwrap();
    assert!(app.ui.present.is_none(), "the preview must be visible");
    assert!(app.ui.preview.is_some());
    assert_eq!(app.session.active, 0);
    assert!(!app.ui.home);

    // Setting the UI state must produce the same result as executing the command.
    handle(&mut app, "engine.execute", json!({"command": "proto.present", "params": {"start": frame}})).unwrap();
    handle(&mut app, "ui.set", json!({"preview": frame})).unwrap();
    assert!(app.ui.present.is_none());
    assert!(app.ui.preview.is_some());
}

#[test]
fn a_batch_present_request_keeps_its_document_for_the_ui() {
    // A batch passes its request to the UI after executing the nested steps.
    let (mut app, back) = two_files();
    let frame = handle(&mut app, "engine.execute", json!({"document": back, "command": "frame.create", "params": {"width": 200, "height": 200}}))
        .unwrap()["id"]
        .clone();
    handle(
        &mut app,
        "engine.execute",
        json!({"document": back, "command": "command.batch", "params": {"commands": [{"command": "proto.present", "params": {"start": frame}}]}}),
    )
    .unwrap();
    assert_eq!(app.session.active, 0);
    assert!(app.ui.present.is_some());
    let active = app.ui.present_active.unwrap();
    assert_eq!(app.ui.presents.iter().find(|t| t.id == active).unwrap().doc_uid, back);
}

#[test]
fn nested_batches_track_only_successful_document_switches() {
    // A failed nested batch with continue must not count as a completed switch.
    let switch = json!({"command": "doc.switch", "params": {"index": 0}});
    let failed = json!({"command": "command.batch", "params": {"commands": [switch, {"command": "does.not.exist"}]}});
    for (params, expected) in [
        (json!({"commands": [switch]}), 0usize),
        (json!({"commands": [{"command": "command.batch", "params": {"commands": [switch]}}]}), 0),
        (json!({"onError": "continue", "commands": [failed]}), 1),
        (json!({"onError": "continue", "commands": [{"command": "doc.switch", "params": {"index": 999}}]}), 1),
    ] {
        let (mut app, back) = two_files();
        app.ui.home = true;
        handle(&mut app, "engine.execute", json!({"document": back, "command": "command.batch", "params": params})).unwrap();
        assert_eq!(app.session.active, expected);
        assert_eq!(app.ui.home, expected == 1);
    }
}

#[test]
fn failed_batches_discard_their_player_requests_but_keep_older_ui_requests() {
    // Colliding frame IDs expose a failed batch presenting another file's frame.
    for named in [false, true] {
        let (mut app, back) = two_files();
        let back_frame =
            handle(&mut app, "engine.execute", json!({"document": back, "command": "frame.create", "params": {}})).unwrap()["id"].clone();
        let front_frame = handle(&mut app, "engine.execute", json!({"command": "frame.create", "params": {}})).unwrap()["id"].clone();
        assert_eq!(back_frame, front_frame);
        app.session.request_ui("panel.assets", json!({}));
        let result = handle(
            &mut app,
            "engine.execute",
            json!({
                "document": named.then_some(back), "command": "command.batch", "params": {"commands": [
                    {"command": "proto.present", "params": {"start": back_frame}}, {"command": "does.not.exist"}
                ]}
            }),
        );
        assert!(result.is_err());
        assert_eq!(app.session.active, 1);
        assert!(app.ui.present.is_none());
        assert_eq!(app.inspect()["leftTab"], json!("Assets"));
    }
}

#[test]
fn a_failed_direct_group_rolls_back_ui_requests_and_document_selection() {
    // A direct grouped call must also roll back effects without an outer command.batch.
    let (mut app, _) = two_files();
    let frame = handle(&mut app, "engine.execute", json!({"command": "frame.create", "params": {}})).unwrap()["id"].clone();
    let revision = app.session.document_selection_revision();
    app.session.request_ui("panel.assets", json!({}));
    let result: Result<(), figcraft_engine::EngineError> = app.session.grouped("failed presentation", |session| {
        session.execute("proto.present", &json!({"start": frame}))?;
        Err(figcraft_engine::EngineError::Other("abort".into()))
    });
    assert!(result.is_err());
    assert_eq!(app.session.document_selection_revision(), revision);
    app.handle_ui_requests();
    assert!(app.ui.present.is_none());
    assert_eq!(app.inspect()["leftTab"], json!("Assets"));
}

#[test]
fn refused_file_commands_keep_their_interactive_dialog_requests() {
    // A failed standalone command may intentionally request a save or close confirmation.
    for (command, dialog) in [("file.save", "dialog.saveAs"), ("file.saveAs", "dialog.saveAs"), ("file.close", "dialog.unsaved")] {
        let (mut app, _) = two_files();
        app.session.execute("shape.rectangle", &json!({})).unwrap();
        assert!(app.session.execute(command, &json!({})).is_err());
        assert!(app.session.ui_requests.iter().any(|request| request.kind == dialog), "{command}");
        assert_eq!(app.session.docs.len(), 2);
    }
}
