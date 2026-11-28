//! Commands never leave a layer with a non-finite position or size: such a file saves but can't be
//! reopened (JSON writes NaN and infinity as null).

use serde_json::json;

use crate::Session;

fn rect(s: &mut Session) -> crate::NodeId {
    let r = s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 10, "width": 100, "height": 50})).unwrap();
    crate::params::id_of(&r["id"]).unwrap()
}

fn assert_roundtrips(s: &Session) {
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    assert!(figcraft_format::load_bytes(&bytes).is_ok(), "the saved file reopens");
}

#[test]
fn huge_moves_are_refused_and_the_file_still_reopens() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("transform.move", &json!({"ids": [r], "dx": 1e308, "dy": 0})).ok();
    let second = s.execute("transform.move", &json!({"ids": [r], "dx": 1e308, "dy": 1e308}));
    assert!(second.is_err(), "a move past the f64 range is refused");
    let n = s.doc().unwrap().node(r).unwrap();
    assert!(n.transform.as_coeffs().iter().all(|c| c.is_finite()), "{:?}", n.transform);
    assert_roundtrips(&s);
}

#[test]
fn huge_scale_is_refused_and_the_file_still_reopens() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("transform.scale", &json!({"ids": [r], "factor": 1e300})).ok();
    let second = s.execute("transform.scale", &json!({"ids": [r], "factor": 1e300}));
    assert!(second.is_err(), "a size past the f64 range is refused");
    let n = s.doc().unwrap().node(r).unwrap();
    assert!(n.width.is_finite() && n.height.is_finite(), "{} x {}", n.width, n.height);
    assert_roundtrips(&s);
}

/// A settled document of `groups` groups of `per` rectangles each, and one of the rectangles.
fn large(groups: usize, per: usize) -> (figcraft_doc::Document, crate::NodeId) {
    use figcraft_doc::NodeKind;
    let mut doc = figcraft_doc::Document::new("Large");
    let page = doc.first_page().unwrap();
    let mut last = None;
    for g in 0..groups {
        let group = doc.create(page, NodeKind::Group, "group", 0.0, g as f64 * 20.0, 10.0, 10.0).unwrap();
        for i in 0..per {
            last = Some(doc.create(group, NodeKind::Rectangle, "rect", i as f64 * 20.0, 0.0, 10.0, 10.0).unwrap());
        }
    }
    doc.take_dirty();
    (doc, last.unwrap())
}

#[test]
fn an_edit_checks_only_what_it_changed_not_the_whole_document() {
    // #1672: the check went over every layer of the file after every edit, so a batch slowed down
    // with the size of the file, not of the edit. Count the layers it looks at instead of timing it.
    let (big, id) = large(2000, 100);
    let mut s = Session::with_document(big);
    let before = s.doc().unwrap().clone();
    s.execute("node.set", &json!({"ids": [id], "props": {"name": "renamed"}})).unwrap();
    let checked = crate::session::finite_check_candidates(s.doc().unwrap(), &before);
    assert!(checked.contains(&id), "the renamed layer is checked: {checked:?}");
    assert!(checked.len() <= 10, "a rename among 200,000 layers checks {} of them", checked.len());
}

#[test]
fn a_layer_already_out_of_range_does_not_block_edits_to_others() {
    // A damaged file can hold a layer with a non-finite size; edits to other layers still go
    // through, and an edit that touches the damaged layer is refused.
    let (mut doc, id) = large(2, 3);
    let page = doc.first_page().unwrap();
    let broken = doc.create(page, figcraft_doc::NodeKind::Rectangle, "broken", 0.0, 0.0, 10.0, 10.0).unwrap();
    doc.node_mut(broken).unwrap().width = f64::NAN;
    doc.take_dirty();
    let mut s = Session::with_document(doc);
    assert!(s.execute("node.set", &json!({"ids": [id], "props": {"name": "renamed"}})).is_ok());
    let moved = s.execute("transform.move", &json!({"ids": [broken], "dx": 5, "dy": 0}));
    let err = moved.expect_err("an edit of the broken layer is refused").to_string();
    assert!(err.contains("broken"), "{err}");
}
