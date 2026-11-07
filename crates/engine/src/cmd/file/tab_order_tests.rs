use super::*;

#[test]
fn moving_and_closing_inactive_tabs_preserves_the_active_document() {
    let mut session = Session::new();
    for name in ["Two", "Three", "Four"] {
        session.execute("file.new", &json!({"name": name})).unwrap();
    }
    session.execute("doc.switch", &json!({"index": 2})).unwrap();
    let active_uid = session.docs[session.active].uid;
    let moved_uid = session.docs[0].uid;
    session.execute("file.moveTab", &json!({"from": 0, "to": 2})).unwrap();
    assert_eq!(session.docs[2].uid, moved_uid);
    assert_eq!(session.docs[session.active].uid, active_uid);
    session.execute("file.close", &json!({"index": 0})).unwrap();
    assert_eq!(session.docs[session.active].uid, active_uid);
    assert!(session.execute("file.moveTab", &json!({"from": 0, "to": 99})).is_err());
}
