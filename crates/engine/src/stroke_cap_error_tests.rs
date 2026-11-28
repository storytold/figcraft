//! An unknown stroke cap is refused with the caps there are, as `strokeAlign` and `strokeJoin`
//! list theirs (#2005).

use serde_json::json;

use crate::Session;
use crate::cmd::props::STROKE_CAP_HELP;
use crate::params::id_of;
use figcraft_doc::StrokeCap;

#[test]
fn an_unknown_cap_lists_every_cap() {
    let names: Vec<String> = StrokeCap::ALL.iter().map(|c| serde_json::to_value(c).unwrap().as_str().unwrap().to_string()).collect();
    assert_eq!(STROKE_CAP_HELP.split('|').collect::<Vec<_>>(), names, "the list is every cap, in order");
    let mut s = Session::new();
    let v = id_of(&s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    for key in ["strokeCap", "startCap", "endCap"] {
        for junk in [json!("butt"), json!(3), json!("")] {
            let e = s.execute("node.set", &json!({"id": v, "props": {key: junk}})).unwrap_err().to_string();
            assert!(e.contains(&format!("{key}: {STROKE_CAP_HELP}")), "{key} {junk}: {e}");
        }
        // Every name the error lists is taken.
        for name in &names {
            s.execute("node.set", &json!({"id": v, "props": {key: name}})).unwrap_or_else(|e| panic!("{key} {name}: {e}"));
        }
    }
}
