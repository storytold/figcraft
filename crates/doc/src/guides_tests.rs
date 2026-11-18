//! Ruler guides survive a save and reopen (#1476).

use crate::{Document, Guide, Node, NodeKind};

/// A guide: vertical (at an x offset) or horizontal (at a y offset).
fn guide(vertical: bool, offset: f64) -> Guide {
    Guide { vertical, offset }
}

/// A page's guides are written once and read back onto the page.
#[test]
fn page_guides_round_trip_through_json() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    d.node_mut(page).unwrap().guides_mut().push(guide(true, 120.0));
    let json = serde_json::to_string(d.node(page).unwrap()).unwrap();
    assert_eq!(json.matches("\"guides\"").count(), 1, "{json}");
    let back: Node = serde_json::from_str(&json).unwrap();
    assert_eq!(back.guides(), &[guide(true, 120.0)]);
    assert_eq!(back, *d.node(page).unwrap());
}

/// A frame's guides round trip too, and a layer without guides writes no `guides` key.
#[test]
fn frame_guides_round_trip_and_none_are_left_out() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    for id in [page, frame] {
        let json = serde_json::to_value(d.node(id).unwrap()).unwrap();
        assert!(json.get("guides").is_none(), "{json}");
    }
    d.node_mut(frame).unwrap().guides_mut().push(guide(false, 40.0));
    let json = serde_json::to_string(d.node(frame).unwrap()).unwrap();
    assert_eq!(json.matches("\"guides\"").count(), 1, "{json}");
    let back: Node = serde_json::from_str(&json).unwrap();
    assert_eq!(back, *d.node(frame).unwrap());
}

/// A page saved with two `guides` keys reads both lists, in file order, and writes them once.
#[test]
fn a_page_written_with_two_guides_lists_opens_with_both() {
    // How a page was saved after its file had been reopened: the page kind's list (guides added
    // since), then the layer's (the ones the file had).
    let json = r#"{"id": 1, "name": "Page 1", "type": "page", "guides": [{"vertical": true, "offset": 5.0}],
        "width": 0.0, "height": 0.0, "guides": [{"vertical": false, "offset": 7.0}]}"#;
    let n: Node = serde_json::from_str(json).unwrap();
    assert!(matches!(n.kind, NodeKind::Page { .. }));
    assert_eq!(n.guides(), &[guide(true, 5.0), guide(false, 7.0)]);
    let again: Node = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
    assert_eq!(again.guides(), n.guides());
}

/// A `guides` value that isn't a list of guides is still refused.
#[test]
fn a_bad_guides_list_is_still_an_error() {
    for bad in [r#"3"#, r#"[{"vertical": "no"}]"#, r#"{"offset": 1}"#] {
        let json = format!(r#"{{"id": 1, "name": "F", "type": "frame", "width": 1.0, "height": 1.0, "guides": {bad}}}"#);
        assert!(serde_json::from_str::<Node>(&json).is_err(), "{bad}");
    }
}
