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
