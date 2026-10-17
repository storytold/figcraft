use figcraft_engine::Session;
use serde_json::json;

use crate::import_report::{MAX_NOTE_CHARS, MAX_NOTES, Report, note_text};
use crate::{FigApp, NoServices};

#[test]
fn import_results_with_warnings_become_reports_with_counts() {
    let result =
        json!({"warnings": ["layers without a type were skipped"], "summary": {"name": "Synthetic design", "pages": 2, "layers": 12, "images": 3}});
    let report = Report::from_result("file.import", &json!({}), &result).unwrap();
    assert_eq!(report.name, "Synthetic design");
    assert_eq!((report.pages, report.layers, report.images), (Some(2), Some(12), Some(3)));
    assert_eq!(report.notes, ["layers without a type were skipped"]);
    assert_eq!(report.note_count, 1);
}

#[test]
fn only_import_commands_with_warnings_create_reports() {
    // A warning-free `.fig` or REST open still has a summary to show.
    let report = Report::from_result("file.open", &json!({}), &json!({"warnings": [], "summary": {"name": "Local", "pages": 1}})).unwrap();
    assert!(!report.appended && report.notes.is_empty());
    // A native file's repairs have their own message; an SVG without notes has no `warnings`.
    assert!(Report::from_result("file.open", &json!({}), &json!({"repairs": ["Repaired parent"]})).is_none());
    assert!(Report::from_result("file.open", &json!({}), &json!({"index": 0})).is_none());
    assert!(Report::from_result("file.export", &json!({}), &json!({"warnings": ["Export warning"]})).is_none());
    assert!(Report::from_result("shape.rectangle", &json!({}), &json!({})).is_none());
    // File ▸ Open recent opens by path and reports like File ▸ Open.
    let recent = json!({"warnings": ["layers without a type were skipped"], "summary": {"name": "Recent", "pages": 1}});
    assert!(Report::from_result("file.openRecent", &json!({"path": "Recent.json"}), &recent).is_some());
    assert!(Report::from_result("file.openRecent", &json!({"path": "Clean.svg"}), &json!({"warnings": []})).is_none());
}

#[test]
fn svg_placements_are_not_reported_as_appended_pages() {
    assert!(Report::from_result("file.import", &json!({}), &json!({"id": 7})).is_none());
    let report = Report::from_result("file.import", &json!({}), &json!({"id": 7, "warnings": ["Filter skipped"]})).unwrap();
    assert!(!report.appended);
    assert!(Report::from_result("file.import", &json!({}), &json!({"pages": [1], "summary": {"pages": 1}, "warnings": []})).unwrap().appended);
}

#[test]
fn report_retention_is_bounded_and_truncates_only_at_character_boundaries() {
    let long_note = "🦎".repeat(MAX_NOTE_CHARS + 20);
    let notes: Vec<_> = std::iter::repeat_n(long_note, MAX_NOTES + 37).collect();
    let report = Report::from_result("file.open", &json!({}), &json!({"warnings": notes})).unwrap();
    assert_eq!(report.note_count, MAX_NOTES + 37);
    assert_eq!(report.notes.len(), MAX_NOTES);
    assert!(report.notes.iter().all(|note| note.chars().count() == MAX_NOTE_CHARS + 1 && note.ends_with('…')));
}

#[test]
fn note_text_keeps_the_repeat_suffix_and_unknown_notes() {
    let base = "instances of components that aren't in the file were imported as frames";
    let repeated = note_text(&format!("{base} (×122)"));
    assert_eq!(repeated, "Instances of components that aren't in the file were imported as frames (×122)");
    assert_eq!(note_text("some unknown note"), "some unknown note");
    assert_eq!(note_text("weird (×x)"), "weird (×x)");
    assert_eq!(note_text("weird (×12) tail"), "weird (×12) tail");
}

#[test]
fn vector_style_notes_are_known_import_notes() {
    for note in [
        "some vector points refer to point settings missing from the file; they use the layer's joins and plain corners",
        "vector points with unknown stroke joins use the layer join",
        "point stroke joins were preserved; dashed or variable-width strokes render with the layer join",
    ] {
        let shown = note_text(note);
        assert!(shown.chars().next().is_some_and(char::is_uppercase), "{shown}");
        assert_eq!(shown[shown.chars().next().unwrap().len_utf8()..], note[note.chars().next().unwrap().len_utf8()..]);
    }
}

#[test]
fn opening_a_rest_json_file_reports_what_was_skipped() {
    let path = std::env::temp_dir().join(format!("figcraft-import-report-{}.json", std::process::id()));
    let file = json!({"document": {"id": "0:0", "name": "Synthetic", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page 1", "type": "CANVAS", "children": [
            {"id": "1:1", "name": "Card", "type": "FRAME", "size": {"x": 200, "y": 100}, "relativeTransform": [[1, 0, 10], [0, 1, 20]],
                "children": [{"id": "1:2", "name": "No type"}]}]}]}});
    std::fs::write(&path, file.to_string()).unwrap();
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    // A programmatic open (control channel, MCP) never opens the window.
    assert!(app.run("file.open", json!({"path": path.to_string_lossy()})).is_some());
    assert!(app.ui.import_report.is_none());
    let opened = app.run_user("file.open", json!({"path": path.to_string_lossy()}));
    std::fs::remove_file(&path).unwrap();
    assert!(opened.is_some());
    let report = app.ui.import_report.as_ref().expect("a report for the REST import");
    assert!(!report.appended);
    assert_eq!(report.pages, Some(1));
    assert!(report.layers.is_some_and(|n| n >= 1));
    assert!(report.notes.iter().any(|n| n == "layers without a type were skipped"), "{:?}", report.notes);
    assert_eq!(app.inspect()["importReport"]["warningCount"], report.note_count);
}

#[test]
fn escape_closes_the_import_report_like_other_simple_dialogs() {
    let mut u = crate::layers_drag_tests::Ui::new(Session::new());
    let mut report = Report::from_result("file.open", &json!({}), &json!({"warnings": ["Synthetic note"], "summary": {"name": "Synthetic design"}}));
    report.as_mut().unwrap().uid = u.app.session.ds().unwrap().uid;
    u.app.ui.import_report = report;
    u.frame(vec![]);
    let escape =
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
    u.frame(escape);
    assert!(u.app.ui.import_report.is_none(), "Escape closes the report");
}

pub(crate) fn shown_texts(shapes: Vec<egui::epaint::ClippedShape>) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    out
}

#[test]
fn a_clean_svg_open_has_no_report_and_a_placed_svg_says_where_it_went() {
    let svg_open = Report::from_result("file.open", &json!({"path": "icons/Logo.SVG"}), &json!({"warnings": []}));
    assert!(svg_open.is_none(), "a clean SVG opened with file.open has nothing to report");
    let noted = Report::from_result("file.open", &json!({"path": "Logo.svg"}), &json!({"warnings": ["Filter skipped"]})).unwrap();
    assert_eq!(noted.name, "Logo.svg");

    let placed = Report::from_result("file.import", &json!({"path": "dir/Logo.svg"}), &json!({"id": 7, "warnings": ["Filter skipped"]})).unwrap();
    assert!(placed.placed && !placed.appended);
    assert_eq!(placed.name, "Logo.svg");
    let mut u = crate::layers_drag_tests::Ui::new(Session::new());
    let mut placed = placed;
    placed.uid = u.app.session.ds().unwrap().uid;
    u.app.ui.import_report = Some(placed);
    let texts = shown_texts({
        u.frame(vec![]);
        u.frame_output(vec![]).shapes
    });
    assert!(texts.iter().any(|t| t == "Placed on the current page"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Opened as a new document"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Logo.svg"), "{texts:?}");
}

#[test]
fn the_report_goes_when_its_document_closes_and_counts_omitted_notes_in_the_singular() {
    let mut u = crate::layers_drag_tests::Ui::new(Session::new());
    let mut report = Report::from_result("file.open", &json!({}), &json!({"warnings": ["One note"], "summary": {"name": "Synthetic"}})).unwrap();
    report.uid = u.app.session.ds().unwrap().uid;
    report.note_count = 2;
    u.app.ui.import_report = Some(report);
    let texts = shown_texts({
        u.frame(vec![]);
        u.frame_output(vec![]).shapes
    });
    assert!(texts.iter().any(|t| t == "1 more note omitted from this report"), "{texts:?}");

    u.app.session.execute("file.new", &json!({"name": "Other"})).unwrap();
    u.frame(vec![]);
    assert!(u.app.ui.import_report.is_some(), "another tab was added; the report's document is still open");
    u.app.session.execute("file.close", &json!({"index": 0, "force": true})).unwrap();
    u.frame(vec![]);
    assert!(u.app.ui.import_report.is_none(), "the report left with its document");
}

#[test]
fn a_dropped_file_shows_its_report_once_it_opens() {
    let path = std::env::temp_dir().join(format!("figcraft-import-report-drop-{}.json", std::process::id()));
    let file = json!({"document": {"id": "0:0", "name": "Dropped", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page 1", "type": "CANVAS", "children": [{"id": "1:1", "name": "No type"}]}]}});
    std::fs::write(&path, file.to_string()).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(Session::new());
    u.app.ui.pending_opens.push(crate::pending_open::PendingOpen {
        label: "Dropped.json".into(),
        params: json!({"path": path.to_string_lossy()}),
        command: "file.open",
        target: None,
    });
    for _ in 0..6 {
        u.frame(vec![]);
    }
    std::fs::remove_file(&path).unwrap();
    let report = u.app.ui.import_report.as_ref().expect("the dropped file's report");
    assert_eq!(report.uid, u.app.session.ds().unwrap().uid);
}

#[test]
fn a_file_opened_from_base64_data_has_a_summary_too() {
    use base64::Engine as _;
    let file = json!({"document": {"id": "0:0", "name": "From data", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page 1", "type": "CANVAS", "children": [{"id": "1:1", "name": "No type"}]}]}});
    let data = base64::engine::general_purpose::STANDARD.encode(file.to_string());
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    let opened = app.run_user("file.open", json!({"data": data, "name": "Dropped.json"})).unwrap();
    assert_eq!(opened["summary"]["pages"], 1);
    let report = app.ui.import_report.as_ref().expect("a report for the REST import");
    assert_eq!(report.pages, Some(1));
    assert!(report.notes.iter().any(|n| n == "layers without a type were skipped"), "{:?}", report.notes);
    // Shown as a sentence, worked out once when the report opens.
    assert!(report.shown.iter().any(|n| n == "Layers without a type were skipped"), "{:?}", report.shown);
}
