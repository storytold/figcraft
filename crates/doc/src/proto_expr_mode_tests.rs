//! Variable mode lookups in prototype expressions: a variable's value in a given mode of its
//! collection (`n-name[Dark]`) and the mode a collection is in (`mode(Theme)`), typed, checked,
//! evaluated, written back and stored as JSON.

use std::collections::BTreeMap;

use crate::proto::*;
use crate::proto_expr_tests::vars;
use crate::{Collection, Mode, Variable, VariableType, VariableValue, Variables};

/// The test variables plus a collection "Color scheme" (modes "High contrast" and "a]b") with a
/// number variable `k` ("k-name").
fn more_vars() -> Variables {
    let mut v = vars();
    v.collections.push(Collection {
        id: "cs".into(),
        name: "Color scheme".into(),
        modes: vec![Mode { id: "hc".into(), name: "High contrast".into() }, Mode { id: "odd".into(), name: "a]b".into() }],
        extends: None,
    });
    v.variables.push(Variable {
        id: "k".into(),
        name: "k-name".into(),
        collection: "cs".into(),
        kind: VariableType::Number,
        values: BTreeMap::from([("hc".to_string(), VariableValue::Number(1.0)), ("odd".to_string(), VariableValue::Number(2.0))]),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    v
}

fn parse(text: &str) -> Expr {
    Expr::parse(text, &more_vars()).unwrap_or_else(|e| panic!("{text}: {e:?}"))
}

fn error(text: &str) -> String {
    match Expr::parse(text, &more_vars()) {
        Ok(e) => panic!("{text} parsed as {e:?}"),
        Err(e) => e.message,
    }
}

fn in_mode(var: &str, mode: &str) -> Expr {
    Expr::InMode { variable: var.into(), mode: mode.into() }
}

fn mode_of(c: &str) -> Expr {
    Expr::Mode { collection: c.into() }
}

#[test]
fn reads_mode_lookups_and_writes_them_back() {
    use ExprOp::*;
    assert_eq!(parse("n-name[Dark]"), in_mode("n", "dark"));
    assert_eq!(parse("n-name [ dark ] + 1"), Expr::op(Add, vec![in_mode("n", "dark"), Expr::Value(VariableValue::Number(1.0))]), "ids, spaces");
    assert_eq!(parse("{k-name}[High contrast]"), in_mode("k", "hc"));
    assert_eq!(parse(r#"k-name["a]b"]"#), in_mode("k", "odd"), "quoted names may hold brackets");
    assert_eq!(parse("mode(Theme)"), mode_of("c"));
    assert_eq!(
        parse("MODE ( 'Color scheme' ) == \"High contrast\""),
        Expr::op(Equals, vec![mode_of("cs"), Expr::Value(VariableValue::String("High contrast".into()))])
    );
    assert_eq!(parse("mode({Color scheme})"), mode_of("cs"));
    assert_eq!(parse("mode(c) != \"Dark\" and n-name[light] < n-name"), parse("(mode(Theme) != 'Dark') and (n-name[Light] < n)"));
    let vars = more_vars();
    for text in [
        "n-name[Dark]",
        "k-name[High contrast]",
        "k-name[\"a]b\"]",
        "mode(Theme) == \"Dark\"",
        "mode(Color scheme)",
        "(mode(Theme) == \"Light\") or (k-name[High contrast] > 0)",
    ] {
        let e = parse(text);
        assert_eq!(e.display(&vars), text, "{e:?}");
        assert_eq!(Expr::parse(&e.display(&vars), &vars), Ok(e.clone()), "{text} reads back");
        // Stored as JSON and read back.
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(serde_json::from_value::<Expr>(json).unwrap(), e);
    }
    assert_eq!(serde_json::to_value(in_mode("n", "dark")).unwrap(), serde_json::json!({"inMode": {"variable": "n", "mode": "dark"}}));
    assert_eq!(serde_json::to_value(mode_of("c")).unwrap(), serde_json::json!({"mode": {"collection": "c"}}));
}

#[test]
fn mode_lookup_errors_say_what_is_wrong() {
    assert_eq!(error("n-name[Sepia]"), "Theme has no mode `Sepia`");
    assert_eq!(error("n-name[High contrast]"), "Theme has no mode `High contrast`", "another collection's mode");
    assert_eq!(error("n-name[Dark"), "Add a closing ]");
    assert_eq!(error("n-name[]"), "Name a mode between the brackets");
    assert_eq!(error("mode(Nope)"), "No variable collection `Nope`");
    assert_eq!(error("mode(Theme"), "Add a closing )");
    assert_eq!(error("mode()"), "Name a collection between the brackets");
    assert_eq!(error("mode(\"Theme)"), "Close the text with \"");
    // `mode` alone isn't a function call (nor a variable here).
    assert_eq!(error("mode"), "No variable `mode`");
    // Type checks: a lookup has its variable's type, a mode is text.
    let vars = more_vars();
    assert_eq!(parse("n-name[Dark] + 1").type_of(&vars), Ok(VariableType::Number));
    assert_eq!(parse("mode(Theme)").type_of(&vars), Ok(VariableType::String));
    assert_eq!(parse("mode(Theme) == 1").check_condition(&vars), Err("Can't compare text with a number".into()));
    assert_eq!(parse("mode(Theme) == 'Dark'").check_condition(&vars), Ok(()));
    // Stored expressions whose collection, variable or mode is gone.
    assert_eq!(mode_of("gone").type_of(&vars), Err("No variable collection `gone`".into()));
    assert_eq!(in_mode("gone", "dark").type_of(&vars), Err("No variable `gone`".into()));
    assert_eq!(in_mode("n", "gone").type_of(&vars), Err("Theme has no mode `gone`".into()));
    assert_eq!(in_mode("n", "gone").display(&vars), "n-name[\"(missing mode)\"]");
    assert_eq!(mode_of("gone").display(&vars), "mode(\"(missing collection)\")");
}

#[test]
fn lookups_evaluate_in_their_mode_and_modes_follow_the_layer() {
    let vars = more_vars();
    let light = BTreeMap::new();
    let dark = BTreeMap::from([("c".to_string(), "dark".to_string())]);
    let ev = |text: &str, modes: &BTreeMap<String, String>| parse(text).eval(&vars, modes);
    // A lookup reads its mode whichever mode is chosen; plain references follow the chosen one.
    assert_eq!(ev("n-name[Dark]", &light), Some(VariableValue::Number(7.0)));
    assert_eq!(ev("n-name[Light]", &dark), Some(VariableValue::Number(3.0)));
    assert_eq!(ev("n-name", &dark), Some(VariableValue::Number(7.0)));
    // Aliases resolve in the looked-up mode.
    assert_eq!(ev("alias-name[Dark]", &light), Some(VariableValue::Number(7.0)));
    // The current mode by name: the default when none is chosen (or the chosen one is gone).
    assert_eq!(ev("mode(Theme)", &light), Some(VariableValue::String("Light".into())));
    assert_eq!(ev("mode(Theme)", &dark), Some(VariableValue::String("Dark".into())));
    assert_eq!(
        parse("mode(Theme)").eval(&vars, &BTreeMap::from([("c".to_string(), "gone".to_string())])),
        Some(VariableValue::String("Light".into()))
    );
    assert!(parse("mode(Theme) == \"Dark\" and k-name[\"a]b\"] == 2").holds(&vars, &dark));
    // Missing parts evaluate to nothing.
    assert_eq!(in_mode("n", "gone").eval(&vars, &light), None);
    assert_eq!(mode_of("gone").eval(&vars, &light), None);
    assert_eq!(in_mode("n", "dark").variables(), ["n"]);
}

#[test]
fn stored_variable_ids_keep_expression_evaluation_display_and_types() {
    let mut vars = more_vars();
    let mut shadow = vars.get_by_id("s").unwrap().clone();
    shadow.id = "shadow".into();
    shadow.name = "n".into();
    vars.variables.insert(0, shadow);
    let mut shadow = vars.collection("cs").unwrap().clone();
    shadow.id = "shadow".into();
    shadow.name = "c".into();
    vars.collections.insert(0, shadow);
    let modes = BTreeMap::new();
    for e in [Expr::var("n"), in_mode("n", "dark")] {
        assert_eq!(e.type_of(&vars), Ok(VariableType::Number));
        assert!(e.display(&vars).starts_with("n-name"));
    }
    assert_eq!(Expr::var("n").eval(&vars, &modes), Some(VariableValue::Number(3.0)));
    assert_eq!(in_mode("n", "dark").eval(&vars, &modes), Some(VariableValue::Number(7.0)));
    assert_eq!(Expr::parse("n-name[Dark]", &vars), Ok(in_mode("n", "dark")));
    assert_eq!(mode_of("c").eval(&vars, &modes), Some(VariableValue::String("Light".into())));
    assert_eq!(mode_of("c").display(&vars), "mode(Theme)");

    vars.variables.retain(|v| v.id != "n");
    for e in [Expr::var("n"), in_mode("n", "dark")] {
        assert_eq!(e.eval(&vars, &modes), None);
        assert_eq!(e.type_of(&vars), Err("No variable `n`".into()));
        assert!(e.display(&vars).contains("missing variable"));
    }
    vars.collections.retain(|c| c.id != "c");
    assert_eq!(mode_of("c").eval(&vars, &modes), None);
    assert_eq!(mode_of("c").type_of(&vars), Err("No variable collection `c`".into()));
    assert_eq!(mode_of("c").display(&vars), "mode(\"(missing collection)\")");
}
