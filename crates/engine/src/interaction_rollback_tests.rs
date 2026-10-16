//! A command that fails part-way (a property that applies, then one that doesn't) leaves nothing
//! behind — also during an interaction, whose last valid preview and own snapshot survive it.

use serde_json::json;

use crate::Session;
use crate::params::id_of;

fn rect(s: &mut Session) -> crate::NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 80, "height": 60})).unwrap()["id"]).unwrap()
}

/// `width` applies (geometry keys go first), then `visible` fails validation.
fn partially_applicable(s: &mut Session, id: crate::NodeId) -> crate::Result<serde_json::Value> {
    s.execute("node.set", &json!({"id": id, "width": 123, "visible": "yes"}))
}

#[test]
fn a_failed_command_during_an_interaction_keeps_the_prior_preview() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.begin_interaction("Move").unwrap();
    let props = serde_json::Map::from_iter([("x".to_string(), json!(20.0))]);
    s.preview(|d, _| crate::cmd::props::apply("node.set", d, r, &props)).unwrap();
    let (previewed, selection, page) = {
        let ds = s.ds().unwrap();
        (ds.doc.clone(), ds.selection.clone(), ds.page)
    };
    let before_journal = s.journal.clone();
    let before_history = s.ds().unwrap().history.undo.len();

    assert!(partially_applicable(&mut s, r).is_err());

    // The rejected edit is gone: the command-entry state (the valid preview) is back.
    let ds = s.ds().unwrap();
    assert_eq!(*ds.doc, *previewed, "the document is what the preview showed");
    assert_eq!(ds.doc.node(r).unwrap().width, 80.0);
    assert_eq!(ds.selection, selection);
    assert_eq!(ds.page, page);
    assert_eq!(ds.history.undo.len(), before_history);
    assert_eq!(s.journal, before_journal);

    // The interaction is alive and commits the preview, not the rejected mutation.
    s.commit_interaction().unwrap();
    let n = s.doc().unwrap().node(r).unwrap();
    assert_eq!((n.width, n.visible), (80.0, true));
    assert_eq!(n.transform.as_coeffs()[4], 20.0, "the previewed move is committed");
    assert_eq!(s.undo_depth(), (before_history + 1, 0));
    assert_eq!(s.undo_labels(1), vec!["Move".to_string()]);
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().transform.as_coeffs()[4], 0.0);
}

#[test]
fn a_failed_command_without_an_interaction_still_rolls_back() {
    let mut s = Session::new();
    let r = rect(&mut s);
    let before_journal = s.journal.clone();
    let before_history = s.ds().unwrap().history.undo.len();

    assert!(partially_applicable(&mut s, r).is_err());

    let n = s.doc().unwrap().node(r).unwrap();
    assert_eq!((n.width, n.visible), (80.0, true));
    assert_eq!(s.ds().unwrap().history.undo.len(), before_history);
    assert_eq!(s.journal, before_journal);
}
