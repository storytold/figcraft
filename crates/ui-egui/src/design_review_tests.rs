//! The Design review window: the check it runs, the list it keeps and the repairs it makes.

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

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
