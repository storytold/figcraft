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
