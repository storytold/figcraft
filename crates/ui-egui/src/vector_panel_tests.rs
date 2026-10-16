//! The Design panel in vector edit mode keeps its width, so the canvas doesn't move under the
//! pointer when the panel's contents change.
use serde_json::json;

#[test]
fn the_canvas_stays_put_entering_vector_edit_mode_and_picking_a_region() {
    let (mut h, _) = crate::live_tests::Harness::new();
    h.app.session.execute("file.newFromTemplate", &json!({"template": "mobile-app"})).unwrap();
    // A wide vector with a filled area: the panel shows its long hint until something is picked.
    let id = {
        let doc = h.app.session.doc().unwrap();
        doc.nodes().filter(|n| n.name == "Near ridge").max_by(|a, b| a.height.total_cmp(&b.height)).map(|n| n.id).unwrap()
    };
    let canvas = |h: &mut crate::live_tests::Harness| {
        h.frames(4);
        h.app.inspect()["canvas"]["rect"].clone()
    };
    h.app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = canvas(&mut h);
    h.app.session.execute("vector.edit.begin", &json!({})).unwrap();
    assert_eq!(canvas(&mut h), before, "the hint for an empty point selection wraps instead of widening the panel");
    h.app.session.execute("vector.edit.select", &json!({"regions": [0]})).unwrap();
    assert_eq!(canvas(&mut h), before, "picking a region doesn't move the canvas either");
    h.app.session.execute("vector.edit.tool", &json!({"tool": "width"})).unwrap();
    assert_eq!(canvas(&mut h), before, "nor does the width tool's hint");
}
