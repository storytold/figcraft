#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use figcraft_engine::Session;
use serde_json::json;

fn scopes(session: &Session, variable: &str) -> Option<Vec<String>> {
    session.doc().unwrap().variables.get(variable).unwrap().scopes.clone()
}

#[test]
fn null_restores_all_supported_properties() {
    let mut session = Session::new();
    session.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    let result = session.execute("variable.create", &json!({"collection": "Theme", "name": "Accent", "type": "color"})).unwrap();
    let variable = result["id"].as_str().unwrap();
    assert_eq!(scopes(&session, variable), None);
    session.execute("variable.update", &json!({"variable": variable, "scopes": []})).unwrap();
    assert_eq!(scopes(&session, variable), Some(vec![]));
    session.execute("variable.update", &json!({"variable": variable, "name": "Accent2"})).unwrap();
    assert_eq!(scopes(&session, variable), Some(vec![]));
    assert_eq!(session.execute("variable.update", &json!({"variable": variable, "scopes": null})).unwrap(), json!({"ok": true}));
    assert_eq!(scopes(&session, variable), None);
    session.execute("variable.update", &json!({"variable": variable, "scopes": []})).unwrap();
    session.execute("variable.update", &json!({"variable": variable, "scopes": ["ALL_SCOPES"]})).unwrap();
    assert_eq!(scopes(&session, variable), None);
}
