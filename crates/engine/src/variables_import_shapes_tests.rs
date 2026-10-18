//! `variable.import` reads `variables` next to `variableCollections`, under `meta` or at the top
//! level, and never imports nothing without saying so (#790).

use serde_json::{Value, json};

use crate::Session;

fn collections() -> Value {
    json!({"VC:1": {"id": "VC:1", "name": "Theme", "modes": [{"modeId": "1:0", "name": "Light"}], "defaultModeId": "1:0",
                    "variableIds": ["VariableID:1"]}})
}

fn variables() -> Value {
    json!({"VariableID:1": {"id": "VariableID:1", "name": "bg", "variableCollectionId": "VC:1", "resolvedType": "COLOR",
                            "valuesByMode": {"1:0": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}}}})
}

fn import(json: Value) -> Value {
    let mut s = Session::new();
    match s.execute("variable.import", &json!({"variables": json})) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn variables_import_reads_every_shape() {
    for shape in [
        json!({"meta": {"variableCollections": collections(), "variables": variables()}}),
        json!({"variableCollections": collections(), "variables": variables()}),
        json!({"meta": {"variableCollections": collections()}, "variables": variables()}),
        json!({"variableCollections": collections(), "meta": {"variables": variables()}}),
    ] {
        let r = import(shape.clone());
        assert_eq!((r["collections"].as_u64(), r["variables"].as_u64()), (Some(1), Some(1)), "{shape}: {r}");
        assert_eq!(r["warnings"], json!([]), "{shape}: {r}");
    }
}

#[test]
fn variables_import_never_imports_nothing_silently() {
    // Collections without variables: they are added, with a warning saying where variables go.
    let r = import(json!({"meta": {"variableCollections": collections()}}));
    assert_eq!((r["collections"].as_u64(), r["variables"].as_u64()), (Some(1), Some(0)), "{r}");
    assert!(r["warnings"].to_string().contains("no variables found: expected `meta.variables`"), "{r}");
    // An empty file's export reads back with the same warning, not an error.
    let mut s = Session::new();
    let text = s.execute("variable.export", &json!({})).unwrap()["text"].as_str().unwrap().to_string();
    let r = s.execute("variable.import", &json!({"variables": text})).unwrap();
    assert!(r["warnings"].to_string().contains("no variables found"), "{r}");
    // Collections listing variables that aren't there.
    let mut colls = collections();
    colls["VC:1"]["variableIds"] = json!(["VariableID:1", "VariableID:2"]);
    let r = import(json!({"meta": {"variableCollections": colls, "variables": variables()}}));
    assert_eq!(r["variables"].as_u64(), Some(1), "{r}");
    assert!(r["warnings"].to_string().contains("1 of the 2 variables the collections list in `variableIds` weren't imported"), "{r}");
    // No collections anywhere: an error naming the expected shape.
    let e = Session::new().execute("variable.import", &json!({"variables": {"variables": variables()}})).unwrap_err().to_string();
    assert!(e.contains("(`meta.variableCollections` and `meta.variables`)"), "{e}");
}
