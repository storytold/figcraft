//! The Design review window: the check it runs, the list it keeps and the repairs it makes.

use figcraft_doc::{DesignPreset, NodeId};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{Harness, text_center};

fn frame(s: &mut Session) -> NodeId {
    let v = s
        .execute(
            "node.create",
            &json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100,
                                         "props": {"layout": "vertical", "gap": 13, "padding": 11}}),
        )
        .unwrap();
    NodeId(v["id"].as_u64().unwrap())
}

#[test]
fn live_recheck_waits_for_the_document_to_be_still() {
    use crate::design_review::live_recheck_due;
    // A change at t=10 is pending, but not due until it has been still for 0.4 s.
    assert!(!live_recheck_due(true, 2, 1, 10.0, 10.2));
    assert!(live_recheck_due(true, 2, 1, 10.0, 10.4));
    // Off, or already checked: never due.
    assert!(!live_recheck_due(false, 2, 1, 10.0, 10.5));
    assert!(!live_recheck_due(true, 1, 1, 10.0, 10.5));
}

#[test]
fn the_review_window_checks_the_page_and_fixes_what_it_can() {
    let mut s = Session::new();
    let f = frame(&mut s);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    u.frame(vec![]);

    let report = u.app.ui.design_review.as_ref().and_then(|s| s.report.clone()).expect("the window checked the page");
    let rules = |r: &serde_json::Value| -> Vec<String> {
        r["findings"].as_array().unwrap().iter().filter_map(|f| f["rule"].as_str().map(str::to_string)).collect()
    };
    assert!(rules(&report).contains(&"spacing.off-grid".to_string()), "{report}");

    crate::design_review::fix_all(&mut u.app);
    let after = u.app.ui.design_review.as_ref().and_then(|s| s.report.clone()).expect("the window checked again");
    assert!(!rules(&after).contains(&"spacing.off-grid".to_string()), "{after}");
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().gap, 12.0);
}

#[test]
fn switching_documents_resets_the_review() {
    let mut s = Session::new();
    let _frame = frame(&mut s);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let first = u.app.session.docs[u.app.session.active].uid;
    u.app.session.execute("file.new", &json!({})).unwrap();
    u.app.session.active = 1;
    crate::design_review::sync_document(&mut u.app);
    let state = u.app.ui.design_review.as_ref().expect("the review follows the new document");
    assert_ne!(state.doc_uid, first);
    assert_eq!(state.doc_uid, u.app.session.docs[1].uid);
    let report = state.report.as_ref().expect("the new document was checked");
    assert!(report["findings"].as_array().unwrap().is_empty(), "an empty document has no findings: {report}");
}

#[test]
fn an_ignored_rule_is_not_fixed() {
    let mut s = Session::new();
    let f = frame(&mut s);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    u.app.ui.design_review.as_mut().unwrap().ignored_rules.insert("spacing.off-grid".to_string());
    crate::design_review::fix_all(&mut u.app);
    let n = u.app.session.doc().unwrap().node(f).unwrap();
    assert_eq!(n.layout.as_ref().unwrap().gap, 13.0, "the ignored rule is left alone");
}

#[test]
fn an_ignored_child_finding_is_not_fixed_by_fix_all() {
    let mut s = Session::new();
    let parent = NodeId(
        s.execute(
            "node.create",
            &json!({"type": "frame", "name": "Parent", "x": 0, "y": 0, "width": 200, "height": 100,
                    "props": {"layout": "vertical", "cornerRadius": 3}}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let child = NodeId(
        s.execute(
            "node.create",
            &json!({"type": "rectangle", "name": "Child", "parent": parent, "x": 0, "y": 0, "width": 80, "height": 40,
                    "props": {"cornerRadius": 3}}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    u.app.ui.design_review.as_mut().unwrap().ignored.insert(("radius.off-scale".to_string(), child.0));
    crate::design_review::fix_all(&mut u.app);
    let doc = u.app.session.doc().unwrap();
    assert_ne!(doc.node(parent).unwrap().corners.radii[0], 3.0, "the kept parent finding is fixed");
    assert_eq!(doc.node(child).unwrap().corners.radii[0], 3.0, "the ignored child finding stays");
}

#[test]
fn a_finding_can_become_a_comment() {
    let mut s = Session::new();
    let f = frame(&mut s);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    crate::design_review::comment_on(&mut u.app, f, "spacing.off-grid", "Off the 4px grid", "Frame 1");
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.comments.len(), 1);
    assert_eq!(doc.comments[0].anchor, Some(f));
    assert!(doc.comments[0].first.text.contains("spacing.off-grid"), "{}", doc.comments[0].first.text);
}

#[test]
fn the_actions_palette_offers_the_review() {
    let rows = crate::menus::command_sections("");
    let offered = rows.iter().any(|(_, rows)| rows.iter().any(|row| row.key == "command:design.audit"));
    assert!(offered, "the palette should offer Review design without a search");
}

#[test]
fn invoking_the_command_opens_the_window() {
    let mut u = Ui::new(Session::new());
    crate::menus::invoke(&mut u.app, "design.audit");
    assert!(u.app.ui.design_review.is_some());
}

#[test]
fn closing_the_window_keeps_the_document_as_it_was() {
    let mut s = Session::new();
    let f = frame(&mut s);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    u.app.ui.design_review = None;
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().gap, 13.0);
}

#[test]
fn ignores_are_loaded_by_document_path() {
    let mut s = Session::new();
    s.ds_mut().unwrap().path = Some("review.figcraft".into());
    s.prefs.design_review_ignored.insert("review.figcraft".into(), vec!["spacing.off-grid".into(), "type.weight@4".into()]);
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let state = u.app.ui.design_review.as_ref().unwrap();
    assert!(state.ignored_rules.contains("spacing.off-grid"));
    assert!(state.ignored.contains(&("type.weight".to_string(), 4)));
}

#[test]
fn ignoring_writes_the_preferences() {
    let mut s = Session::new();
    s.ds_mut().unwrap().path = Some("review.figcraft".into());
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let mut state = u.app.ui.design_review.take().unwrap();
    state.ignored_rules.insert("spacing.off-grid".to_string());
    crate::design_review::persist_ignores(&mut u.app, &state);
    assert_eq!(u.app.session.prefs.design_review_ignored.get("review.figcraft"), Some(&vec!["spacing.off-grid".to_string()]));
}

#[test]
fn a_file_saved_while_the_window_is_open_keeps_its_ignores() {
    let s = Session::new();
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let mut state = u.app.ui.design_review.take().unwrap();
    state.ignored.insert(("type.weight".to_string(), 4));
    u.app.session.ds_mut().unwrap().path = Some("saved-later.figcraft".into());
    crate::design_review::persist_ignores(&mut u.app, &state);
    assert_eq!(u.app.session.prefs.design_review_ignored.get("saved-later.figcraft"), Some(&vec!["type.weight@4".to_string()]));
}

#[test]
fn a_findings_values_read_without_the_english_sentence() {
    let spacing = json!({"gap": 13.0, "padding": [11, 11, 11, 11], "grid": 4.0});
    assert_eq!(crate::design_review::detail_line("spacing.off-grid", &spacing), "Gap 13 · Padding 11 11 11 11 · 4 px");
    let contrast = json!({"ratio": 3.55, "needed": 4.5, "foreground": "777777", "background": "222222", "modes": {"Theme": "Dark"}});
    assert_eq!(crate::design_review::detail_line("color.contrast", &contrast), "3.55:1 < 4.50:1 · #777777 / #222222 · Theme: Dark");
    assert_eq!(crate::design_review::detail_line("naming.auto", &json!({"name": "Frame 1"})), "“Frame 1”");
    let tokens = json!({"hits": [
        {"field": "paddingTop", "value": 12.0, "token": "space"},
        {"field": "paddingBottom", "value": 12.0, "token": "space"}
    ]});
    assert_eq!(crate::design_review::detail_line("token.hardcoded-number", &tokens), "Padding 12 = space");
    assert_eq!(crate::design_review::detail_line("icon.emoji", &json!({})), "", "a static rule has no value line");
}

#[test]
fn the_window_reads_the_files_preset_and_grid() {
    let mut s = Session::new();
    s.execute("design.configure", &json!({"preset": "strict", "grid": 8})).unwrap();
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let state = u.app.ui.design_review.as_ref().unwrap();
    assert_eq!(state.preset, DesignPreset::Strict);
    assert_eq!(state.grid, 8.0);
}

#[test]
fn report_only_findings_are_labelled_in_the_list() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "C"})).unwrap();
    s.execute("variable.create", &json!({"collection": "C", "name": "space", "type": "number", "value": 13})).unwrap();
    let bound = s
        .execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100, "props": {"layout": "vertical", "gap": 13}}))
        .unwrap();
    s.execute("variable.bind", &json!({"ids": [bound["id"]], "field": "gap", "variable": "space"})).unwrap();
    s.execute("node.create", &json!({"type": "frame", "x": 300, "y": 0, "width": 200, "height": 100, "props": {"layout": "vertical", "gap": 13}}))
        .unwrap();
    let mut u = Ui::new(s);
    crate::design_review::open(&mut u.app);
    let mut shapes = Vec::new();
    for _ in 0..5 {
        shapes = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    }
    assert!(crate::live_tests::has_text(&shapes, "Report only"), "the bound finding shows the chip");
    assert!(crate::live_tests::has_text(&shapes, "Fix"), "the loose finding keeps its Fix button");
}

#[test]
fn the_window_edits_the_design_system() {
    let (mut h, _) = Harness::new();
    crate::design_review::open(&mut h.app);
    let state = h.app.ui.design_review.take().unwrap();
    assert!(crate::design_review::parse_num_list("0, 6 10;  14", 32).is_some());
    assert!(crate::design_review::parse_num_list("4, wide", 32).is_none());
    assert!(
        crate::design_review::parse_num_list(
            "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33",
            32
        )
        .is_none()
    );
    assert_eq!(crate::design_review::parse_color_list("#ff0000, #0a0", 32), Some(vec!["#FF0000".to_string(), "#00AA00".to_string()]));
    assert!(crate::design_review::parse_color_list("#FF0000, blue", 32).is_none());

    assert!(crate::design_review::apply_design_system(&mut h.app, Some(vec![0.0, 6.0, 10.0]), Some(vec!["#FF0000".to_string()]), None));
    let config = &h.app.session.ds().unwrap().doc.design_review;
    assert_eq!(config.radii, vec![0.0, 6.0, 10.0]);
    assert_eq!(config.brand.len(), 1);
    assert!(crate::design_review::apply_design_system(&mut h.app, None, None, Some(("icon.emoji".to_string(), true))));
    let config = &h.app.session.ds().unwrap().doc.design_review;
    assert_eq!(config.disabled, vec!["icon.emoji".to_string()]);
    assert!(!crate::design_review::apply_design_system(&mut h.app, None, None, None), "nothing to apply is a no-op");
    drop(state);
}

#[test]
fn the_window_saves_the_preset_and_grid_in_the_file() {
    let (mut h, _) = Harness::new();
    crate::design_review::open(&mut h.app);
    let shapes = h.frames(4);
    h.click(text_center(&shapes, "Recommended"));
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Strict"));
    h.frames(2);
    assert_eq!(h.app.session.ds().unwrap().doc.design_review.preset, DesignPreset::Strict);
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "8 px"));
    h.frames(2);
    assert_eq!(h.app.session.ds().unwrap().doc.design_review.grid(), 8.0);
    let report = h.app.ui.design_review.as_ref().and_then(|s| s.report.clone()).expect("the window checked again");
    assert_eq!(report["preset"], "strict", "{report}");
    assert_eq!(report["grid"], 8.0, "{report}");
}

#[test]
fn the_ignore_button_saves_the_ignore_for_the_document() {
    let (mut h, _) = Harness::new();
    h.app
        .session
        .execute(
            "node.create",
            &json!({"type": "frame", "x": 300, "y": 300, "width": 200, "height": 100,
                "props": {"layout": "vertical", "gap": 13}}),
        )
        .unwrap();
    h.app.session.ds_mut().unwrap().path = Some("harness.figcraft".into());
    crate::design_review::open(&mut h.app);
    // The window paints a frame after it opens.
    let shapes = h.frames(4);
    h.click(text_center(&shapes, "…"));
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Ignore this finding"));
    h.frames(2);
    let saved = h.app.session.prefs.design_review_ignored.get("harness.figcraft").expect("the ignore is saved");
    assert!(saved.iter().any(|e| e.contains('@')), "{saved:?}");
}
