//! Alt+Enter while editing text activates the link at the caret ("When editing text, press
//! ⌥Option Return/Alt Enter to activate a focused link", Accessibility at Figma): `text.openLink`
//! hands a web or mail link at the caret to the UI, refuses other schemes, and does nothing where
//! there is no link.

use serde_json::json;

use crate::Session;
use crate::params::id_of;

#[test]
fn the_link_at_the_caret_goes_to_the_ui_and_only_a_web_or_mail_link() {
    let mut s = Session::new();
    let t = id_of(&s.execute("text.create", &json!({"text": "See the docs now", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("text.style", &json!({"id": t, "start": 4, "end": 12, "hyperlink": "https://example.com/docs"})).unwrap();
    assert!(s.execute("text.openLink", &json!({})).is_err(), "nothing to activate while no text is edited");
    s.execute("text.edit.begin", &json!({"id": t, "caret": 6})).unwrap();
    let r = s.execute("text.openLink", &json!({})).unwrap();
    assert_eq!(r["url"], "https://example.com/docs");
    let req = s.ui_requests.iter().find(|r| r.kind == "open.url").expect("a UI request to open it");
    assert_eq!(req.data["url"], "https://example.com/docs");
    assert!(s.ds().unwrap().text_edit.is_some(), "still editing");
    // Right after the link's last char the caret still counts as on it; before its start it doesn't.
    s.execute("text.edit.caret", &json!({"index": 12})).unwrap();
    assert_eq!(s.execute("text.openLink", &json!({})).unwrap()["url"], "https://example.com/docs");
    s.execute("text.edit.caret", &json!({"index": 2})).unwrap();
    assert!(s.execute("text.openLink", &json!({})).unwrap()["url"].is_null(), "no link there");
    // A selection: its start decides.
    s.execute("text.edit.caret", &json!({"index": 14})).unwrap();
    s.execute("text.edit.caret", &json!({"index": 8, "extend": true})).unwrap();
    assert_eq!(s.execute("text.openLink", &json!({})).unwrap()["url"], "https://example.com/docs");
    // Mail links open; anything that isn't a web or mail link is refused and nothing is requested.
    s.execute("text.style", &json!({"id": t, "start": 4, "end": 12, "hyperlink": "mailto:hello@example.com"})).unwrap();
    s.execute("text.edit.caret", &json!({"index": 6})).unwrap();
    assert_eq!(s.execute("text.openLink", &json!({})).unwrap()["url"], "mailto:hello@example.com");
    let before = s.ui_requests.len();
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "example.com/docs"] {
        s.execute("text.style", &json!({"id": t, "start": 4, "end": 12, "hyperlink": bad})).unwrap();
        assert!(s.execute("text.openLink", &json!({})).is_err(), "{bad} is refused");
    }
    assert_eq!(s.ui_requests.len(), before, "nothing requested for a refused link");
}
