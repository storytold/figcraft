//! Reading design tokens written by other tools (#1687): the 2025.10 shapes, inherited types,
//! aliases, the older string values, and what variables can't hold.

use figcraft_doc::color::{Color, ColorProfile};
use figcraft_doc::{VariableType, VariableValue, Variables};
use serde_json::{Value, json};

use crate::dtcg::{import, is_tokens};

fn read(v: &Value) -> (Variables, Vec<String>) {
    import(v, ColorProfile::Srgb, &|_| None).expect("an object reads")
}

fn value<'a>(vars: &'a Variables, name: &str) -> &'a VariableValue {
    let v = vars.variables.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no {name} in {vars:?}"));
    v.values.values().next().expect("a value")
}

// A file written by our own hand in the 2025.10 shape: a group's `$type` handed to its tokens,
// colours and dimensions as objects, an alias, the older string forms, and a composite token.
#[test]
fn a_tokens_file_reads_as_collections_of_variables() {
    let file = json!({
        "color": {
            "$type": "color",
            "brand": {
                "primary": {"$value": {"colorSpace": "srgb", "components": [0.2, 0.4, 0.6], "hex": "#336699"}, "$description": "Main"},
                "old": {"$value": "#ff0000"}
            },
            "link": {"$value": "{color.brand.primary}"}
        },
        "spacing": {
            "$type": "dimension",
            "gap": {"$value": {"value": 16, "unit": "px"}},
            "wide": {"$value": {"value": 2, "unit": "rem"}},
            "legacy": {"$value": "8px"}
        },
        "type": {
            "weight": {"$type": "fontWeight", "$value": "bold"},
            "family": {"$type": "fontFamily", "$value": ["Inter", "sans-serif"]},
            "heading": {"$type": "typography", "$value": {"fontFamily": "Inter", "fontSize": {"value": 20, "unit": "px"}}}
        }
    });
    assert!(is_tokens(&file));
    let (vars, warnings) = read(&file);
    let names: Vec<&str> = vars.collections.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["color", "spacing", "type"], "a collection per top-level group");

    let primary = vars.variables.iter().find(|v| v.name == "brand/primary").expect("named by its path");
    assert_eq!(primary.kind, VariableType::Color, "the group's type is handed down");
    assert_eq!(primary.description, "Main");
    assert_eq!(*value(&vars, "brand/primary"), VariableValue::Color(Color::from_hex("336699").unwrap()));
    assert_eq!(*value(&vars, "brand/old"), VariableValue::Color(Color::from_hex("ff0000").unwrap()), "a hex string still reads");
    assert_eq!(*value(&vars, "link"), VariableValue::Alias(primary.id.clone()), "an alias names its token's variable");

    let num = |name| match value(&vars, name) {
        VariableValue::Number(x) => *x,
        other => panic!("{name}: {other:?}"),
    };
    assert!(num("wide") > num("gap") && num("gap") > num("legacy"), "px, rem and an older `8px` string are all lengths");
    assert!(num("weight") > 400.0, "a weight's name reads as its number");
    assert_eq!(*value(&vars, "family"), VariableValue::String("Inter".into()), "a family list gives its first family");

    assert!(vars.variables.iter().all(|v| v.name != "heading"), "a composite token isn't a variable");
    assert!(warnings.iter().any(|w| w.contains("typography")), "but it is named in the warnings: {warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("rem")), "and reading a rem at 16 px is said: {warnings:?}");
}

#[test]
fn untyped_tokens_take_their_type_from_their_value_or_alias() {
    let file = json!({
        "flag": {"$value": true},
        "count": {"$value": 3},
        "set": {"main": {"$value": "{flag}"}, "lost": {"$value": "{nowhere.at.all}"}}
    });
    let (vars, warnings) = read(&file);
    let kind = |name: &str| vars.variables.iter().find(|v| v.name == name).map(|v| v.kind);
    assert_eq!(kind("flag"), Some(VariableType::Boolean));
    assert_eq!(kind("count"), Some(VariableType::Number));
    assert_eq!(kind("main"), Some(VariableType::Boolean), "an untyped alias is its target's type");
    assert_eq!(kind("lost"), None, "an alias to nothing is skipped");
    assert!(warnings.iter().any(|w| w.contains("lost")), "and said: {warnings:?}");
    let tokens = vars.collections.iter().find(|c| c.name == "Tokens").expect("top-level tokens share a collection");
    assert!(vars.variables.iter().any(|v| v.name == "flag" && v.collection == tokens.id));
}

#[test]
fn junk_tokens_never_panic() {
    let deep = (0..200).fold(json!({"$value": 1}), |inner, _| json!({ "g": inner }));
    for junk in [
        json!({"a": {"$value": null}}),
        json!({"a": {"$type": "color", "$value": {"colorSpace": "srgb", "components": "x"}}}),
        json!({"a": {"$type": "color", "$value": {"colorSpace": "lab", "components": [1, 2, 3]}}}),
        json!({"a": {"$type": "dimension", "$value": {"value": 1e308, "unit": "rem"}}}),
        json!({"a": {"$type": "dimension", "$value": "px"}}),
        json!({"a": {"$value": "{a}"}}),
        json!({"a": {"$value": "{}"}, "b": {"$value": "{b.c}"}}),
        json!({"a": {"$type": 7, "$value": [1, 2]}, "$extensions": 3}),
        json!({"a": {"$extensions": {"ai.storyteller.figcraft": {"modes": [1, "", "x", "x"]}}, "t": {"$value": 1}}}),
        json!({"g": {"$root": {"$value": 1}, "$root2": {"$value": 2}}}),
        deep,
    ] {
        let (vars, _) = read(&junk);
        assert!(vars.variables.iter().all(|v| !v.values.is_empty()), "{junk}");
    }
    assert!(import(&json!([1, 2]), ColorProfile::Srgb, &|_| None).is_none(), "not an object");
}
