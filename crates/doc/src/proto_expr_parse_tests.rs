//! Prototype expressions typed as text: parsing, errors, type checks and reading back what
//! `display` writes.

use crate::color::Color;
use crate::proto::*;
use crate::proto_expr_tests::vars;
use crate::{Variable, VariableType, VariableValue};

fn parse(text: &str) -> Expr {
    Expr::parse(text, &vars()).unwrap_or_else(|e| panic!("{text}: {e:?}"))
}

fn error(text: &str) -> String {
    match Expr::parse(text, &vars()) {
        Ok(e) => panic!("{text} parsed as {e:?}"),
        Err(e) => e.message,
    }
}

fn n(x: f64) -> Expr {
    Expr::Value(VariableValue::Number(x))
}

#[test]
fn reads_variables_literals_and_precedence() {
    use ExprOp::*;
    assert_eq!(parse("n-name + 1"), Expr::op(Add, vec![Expr::var("n"), n(1.0)]), "names with symbols read whole");
    assert_eq!(parse("{n-name}*2"), Expr::op(Multiply, vec![Expr::var("n"), n(2.0)]));
    assert_eq!(parse("n"), Expr::var("n"), "ids work too");
    assert_eq!(parse("1 + 2 * 3"), Expr::op(Add, vec![n(1.0), Expr::op(Multiply, vec![n(2.0), n(3.0)])]));
    assert_eq!(parse("10 - 2 - 3"), Expr::op(Subtract, vec![n(10.0), n(2.0), n(3.0)]), "one run, left to right");
    assert_eq!(parse("1 + 2 - 3"), Expr::op(Subtract, vec![Expr::op(Add, vec![n(1.0), n(2.0)]), n(3.0)]));
    assert_eq!(parse("-3"), n(-3.0));
    assert_eq!(parse("-n-name"), Expr::op(Negate, vec![Expr::var("n")]));
    assert_eq!(parse("2.5e1"), n(25.0));
    assert_eq!(parse("'single'"), Expr::Value(VariableValue::String("single".into())));
    assert_eq!(parse(r#""say \"hi\"""#), Expr::Value(VariableValue::String("say \"hi\"".into())));
    assert_eq!(parse("#ff0000"), Expr::Value(VariableValue::Color(Color::from_hex("ff0000").unwrap())));
    assert_eq!(parse("TRUE"), Expr::Value(VariableValue::Boolean(true)));
    // Logic: not binds looser than comparisons, and looser than not, or loosest.
    assert_eq!(parse("not n-name == 3"), Expr::op(Not, vec![Expr::op(Equals, vec![Expr::var("n"), n(3.0)])]));
    assert_eq!(
        parse("b-name and n-name >= 2 or !b-name"),
        Expr::op(
            Or,
            vec![Expr::op(And, vec![Expr::var("b"), Expr::op(GreaterOrEqual, vec![Expr::var("n"), n(2.0)])]), Expr::op(Not, vec![Expr::var("b")])]
        )
    );
    assert_eq!(parse("b-name && b-name || false"), parse("(b-name and b-name) or false"));
    assert_eq!(parse("n-name = 3"), parse("n-name == 3"), "a single = compares");
    assert_eq!(parse("1 == 1 == 1"), Expr::op(Equals, vec![n(1.0), n(1.0), n(1.0)]));
    assert_eq!(parse("  ( ( n-name ) ) "), Expr::var("n"));
}

#[test]
fn conditional_expressions_parse_type_check_and_evaluate() {
    let vars = vars();
    let expr = parse("if b-name then true else false");
    assert_eq!(expr.type_of(&vars), Ok(VariableType::Boolean));
    assert_eq!(expr.eval(&vars, &std::collections::BTreeMap::new()), Some(VariableValue::Boolean(true)));
    let modes = std::collections::BTreeMap::from([("c".to_string(), "dark".to_string())]);
    assert_eq!(expr.eval(&vars, &modes), Some(VariableValue::Boolean(false)));
    let shown = expr.display(&vars);
    assert_eq!(Expr::parse(&shown, &vars), Ok(expr));
    let nested = parse("if b-name then (if b-name then true else false) else false");
    let shown = nested.display(&vars);
    assert_eq!(Expr::parse(&shown, &vars), Ok(nested));
    assert!(Expr::parse("if b-name true else false", &vars).is_err());
    assert!(Expr::parse("if b-name then 1 else false", &vars).unwrap().type_of(&vars).is_err());
}

#[test]
fn color_constructors_and_lerp_parse_with_optional_whitespace() {
    let vars = vars();
    let expression = Expr::parse(" lerp ( hsl ( 1, 100, 100 ), hex ( #000000 ), 0.15 ) ", &vars).unwrap();
    assert_eq!(expression.type_of(&vars), Ok(VariableType::Color));
    assert_eq!(expression.display(&vars), "lerp(hsl(1, 100, 100), #000000, 0.15)");
    assert_eq!(Expr::parse(&expression.display(&vars), &vars), Ok(expression.clone()), "displayed constructor calls parse back to the same AST");
    let dynamic = Expr::parse_dynamic("lerp(hsl(1, 100, 100), hex(#000000), 0.15)", &vars).unwrap();
    assert_eq!(dynamic.display_dynamic(&vars), "lerp(hsl(1, 100, 100), hex(#000000), 0.15)");
    assert_eq!(
        serde_json::from_value::<Expr>(serde_json::to_value(&expression).unwrap()).unwrap(),
        expression,
        "color expressions survive document serialization"
    );
    assert_eq!(
        expression.eval(&vars, &std::collections::BTreeMap::new()).and_then(|v| match v {
            VariableValue::Color(c) => Some(c.to_rgba8()),
            _ => None,
        }),
        Some([217, 217, 217, 255])
    );
    assert_eq!(Expr::parse_dynamic("hex(#336699)", &vars).unwrap().type_of(&vars), Ok(VariableType::Color));
    assert!(Expr::parse_dynamic("#336699", &vars).is_err(), "dynamic expressions require the explicit hex() form");
    assert!(Expr::parse("#336699", &vars).is_ok(), "the shared prototype-expression syntax is unchanged");

    let variable_color = Expr::parse("lerp( hsl( 0, 100, 50 ), n-name, 0.5 )", &vars).unwrap();
    assert_eq!(variable_color.type_of(&vars), Err("`lerp` needs two colours and a numeric factor".into()));

    for invalid in ["hsl(1, 100)", "hsl(1, 100, 100, 1)", "hsl(1, 100, 100,)", "hex(#xyz)", "lerp(#000000, #FFFFFF)", "lerp(#000000, #FFFFFF, true)"]
    {
        assert!(Expr::parse(invalid, &vars).is_err() || Expr::parse(invalid, &vars).is_ok_and(|e| e.type_of(&vars).is_err()), "{invalid}");
    }
}

#[test]
fn prototype_variables_named_like_color_constructor_calls_remain_references() {
    let mut variables = vars();
    for name in ["hsl(0, 100, 50)", "hex(#000000)", "lerp(#000000, #FFFFFF, 0.5)"] {
        variables.variables.iter_mut().find(|variable| variable.id == "n").unwrap().name = name.to_string();
        assert_eq!(Expr::parse(name, &variables), Ok(Expr::var("n")), "{name}");
    }

    assert!(matches!(Expr::parse_dynamic("hsl(0, 100, 50)", &variables), Ok(Expr::Op { op: ExprOp::Hsl, .. })));
}

#[test]
fn dynamic_expressions_require_braces_and_round_trip_reserved_variable_names() {
    let mut variables = vars();
    assert_eq!(Expr::parse_dynamic("{n-name} + 1", &variables), Ok(Expr::op(ExprOp::Add, vec![Expr::var("n"), n(1.0)])));
    assert!(Expr::parse_dynamic("n-name + 1", &variables).is_err(), "bare variable names are not dynamic references");
    assert!(Expr::parse("n-name + 1", &variables).is_ok(), "prototype expressions keep their existing bare-name syntax");

    for name in ["and", "or", "hsl", "hex", "lerp"] {
        let variable = variables.variables.iter_mut().find(|variable| variable.id == "b").unwrap();
        variable.name = name.to_string();
        assert_eq!(Expr::parse_dynamic(&format!("{{{name}}}"), &variables), Ok(Expr::var("b")), "{name}");
        assert!(Expr::parse_dynamic(name, &variables).is_err(), "{name} must not resolve as a bare variable");
    }

    let expression = Expr::op(ExprOp::And, vec![Expr::op(ExprOp::Not, vec![Expr::var("b")]), Expr::Value(VariableValue::Boolean(true))]);
    let shown = expression.display_dynamic(&variables);
    assert_eq!(shown, "not {lerp} and true");
    assert_eq!(Expr::parse_dynamic(&shown, &variables), Ok(expression));
}

#[test]
fn dynamic_expressions_support_value_selection_and_grouped_boolean_conditions() {
    let variables = vars();
    let selection = Expr::parse_dynamic("{b-name} and \"Success\" or \"Fail\"", &variables).unwrap();
    assert_eq!(selection.type_of(&variables), Ok(VariableType::String));
    assert_eq!(selection.eval_dynamic(&variables, &std::collections::BTreeMap::new()), Some(VariableValue::String("Success".into())));
    let dark_mode = std::collections::BTreeMap::from([("c".to_string(), "dark".to_string())]);
    assert_eq!(selection.eval_dynamic(&variables, &dark_mode), Some(VariableValue::String("Fail".into())));
    let shown = selection.display_dynamic(&variables);
    assert_eq!(shown, "if {b-name} then \"Success\" else \"Fail\"");
    assert_eq!(Expr::parse_dynamic(&shown, &variables), Ok(selection));

    let grouped = Expr::parse_dynamic("if ({b-name} or false) and {b-name} then \"Yes\" else \"No\"", &variables).unwrap();
    assert_eq!(grouped.type_of(&variables), Ok(VariableType::String));
    assert_eq!(grouped.eval_dynamic(&variables, &std::collections::BTreeMap::new()), Some(VariableValue::String("Yes".into())));
    assert_eq!(grouped.eval_dynamic(&variables, &dark_mode), Some(VariableValue::String("No".into())));

    let comparison_condition = Expr::parse_dynamic("if ({n-name} > 100) and {b-name} then \"Mi\" else \"Bombo\"", &variables).unwrap();
    assert_eq!(
        comparison_condition.display_dynamic(&variables),
        "if {n-name} > 100 and {b-name} then \"Mi\" else \"Bombo\"",
        "comparison operands are not wrapped redundantly in boolean conditions"
    );

    let grouped_conditions =
        Expr::parse_dynamic("if ({b-name} and {b-name} and {b-name}) or {b-name} then \"Mi\" else \"Bombo\"", &variables).unwrap();
    let shown = grouped_conditions.display_dynamic(&variables);
    assert_eq!(shown, "if {b-name} and {b-name} and {b-name} or {b-name} then \"Mi\" else \"Bombo\"");
    assert_eq!(Expr::parse_dynamic(&shown, &variables), Ok(grouped_conditions));

    let required_grouping = Expr::parse_dynamic("if {b-name} and ({b-name} or false) then \"Yes\" else \"No\"", &variables).unwrap();
    let shown = required_grouping.display_dynamic(&variables);
    assert_eq!(shown, "if {b-name} and ({b-name} or false) then \"Yes\" else \"No\"");
    assert_eq!(Expr::parse_dynamic(&shown, &variables), Ok(required_grouping));

    let boolean_logic = Expr::parse_dynamic("{b-name} and false or true", &variables).unwrap();
    assert_eq!(boolean_logic.type_of(&variables), Ok(VariableType::Boolean), "boolean and/or retains its logical meaning");
    assert_eq!(boolean_logic.eval_dynamic(&variables, &std::collections::BTreeMap::new()), Some(VariableValue::Boolean(true)));

    let compound_selection = Expr::parse_dynamic("{b-name} and {b-name} and \"Success\" or \"Fail\"", &variables).unwrap();
    assert_eq!(
        compound_selection,
        Expr::Conditional {
            condition: Box::new(Expr::op(ExprOp::And, vec![Expr::var("b"), Expr::var("b")])),
            then_expr: Box::new(Expr::Value(VariableValue::String("Success".into()))),
            else_expr: Box::new(Expr::Value(VariableValue::String("Fail".into()))),
        }
    );
    assert_eq!(compound_selection.eval_dynamic(&variables, &std::collections::BTreeMap::new()), Some(VariableValue::String("Success".into())));
}

#[test]
fn prototype_display_parenthesizes_conditional_operands() {
    let vars = vars();
    let expression = Expr::parse("(if true then 1 else 2) + 3", &vars).unwrap();
    let original_value = expression.eval(&vars, &std::collections::BTreeMap::new());
    let shown = expression.display(&vars);

    assert_eq!(shown, "(if true then 1 else 2) + 3");
    let reparsed = Expr::parse(&shown, &vars).unwrap();
    assert_eq!(reparsed, expression);
    assert_eq!(reparsed.eval(&vars, &std::collections::BTreeMap::new()), original_value);
    assert_eq!(original_value, Some(VariableValue::Number(4.0)));
}

#[test]
fn dynamic_formatter_preserves_precedence_and_round_trips_every_boolean_combination() {
    let mut variables = vars();
    for (id, name, kind, value) in [
        ("A", "A", VariableType::Boolean, VariableValue::Boolean(false)),
        ("B", "B", VariableType::Boolean, VariableValue::Boolean(false)),
        ("C", "C", VariableType::Boolean, VariableValue::Boolean(false)),
        ("D", "D", VariableType::Boolean, VariableValue::Boolean(false)),
        ("paid", "isPaidUser", VariableType::Boolean, VariableValue::Boolean(false)),
        ("age", "AccountAge", VariableType::Number, VariableValue::Number(120.0)),
        ("num-a", "NumA", VariableType::Number, VariableValue::Number(20.0)),
        ("num-b", "NumB", VariableType::Number, VariableValue::Number(8.0)),
        ("num-c", "NumC", VariableType::Number, VariableValue::Number(2.0)),
    ] {
        variables.variables.push(Variable {
            id: id.into(),
            name: name.into(),
            collection: "c".into(),
            kind,
            values: std::collections::BTreeMap::from([("light".into(), value)]),
            expression: None,
            expression_opacity: std::collections::BTreeMap::new(),
            description: String::new(),
            scopes: None,
            hidden_from_publishing: false,
            extends: None,
            overrides: Default::default(),
            code_syntax: Default::default(),
        });
    }

    let cases = [
        ("{A} and ({B} or {C})", "{A} and ({B} or {C})"),
        ("{A} or {B} and {C}", "{A} or {B} and {C}"),
        ("({A} or {B}) and {C}", "({A} or {B}) and {C}"),
        ("({A} and {B} and {C}) or {D}", "{A} and {B} and {C} or {D}"),
        ("({A} and {B}) and {C}", "({A} and {B}) and {C}"),
        ("{A} and ({B} and {C})", "{A} and ({B} and {C})"),
        ("not ({A} and {B})", "not ({A} and {B})"),
        ("not {A} and {B}", "not {A} and {B}"),
        ("!{A} && {B}", "not {A} and {B}"),
        ("not {NumA} > 5", "not {NumA} > 5"),
        ("-{NumA} * {NumB}", "-{NumA} * {NumB}"),
        ("{NumA} + {NumB} * {NumC}", "{NumA} + {NumB} * {NumC}"),
        ("({NumA} - {NumB}) - {NumC}", "({NumA} - {NumB}) - {NumC}"),
        ("{NumA} - ({NumB} - {NumC})", "{NumA} - ({NumB} - {NumC})"),
        ("{NumA} / ({NumB} * {NumC})", "{NumA} / ({NumB} * {NumC})"),
        ("-({NumA} + {NumB})", "-({NumA} + {NumB})"),
        (r#""a string with \"quotes\"""#, r#""a string with \"quotes\"""#),
        ("if ({AccountAge} > 100) and {isPaidUser} then \"Mi\" else \"Bombo\"", "if {AccountAge} > 100 and {isPaidUser} then \"Mi\" else \"Bombo\""),
        ("if {A} then (if {B} then \"one\" else \"two\") else \"three\"", "if {A} then (if {B} then \"one\" else \"two\") else \"three\""),
        ("if (if {A} then {B} else {C}) then \"yes\" else \"no\"", "if (if {A} then {B} else {C}) then \"yes\" else \"no\""),
        ("((({A})))", "{A}"),
    ];
    let boolean_ids = ["A", "B", "C", "D", "paid"];
    for account_age in [50.0, 120.0] {
        for mask in 0..(1 << boolean_ids.len()) {
            let mut current = variables.clone();
            for (index, id) in boolean_ids.iter().enumerate() {
                if let Some(variable) = current.variables.iter_mut().find(|variable| variable.id == *id)
                    && let Some(value) = variable.values.get_mut("light")
                {
                    *value = VariableValue::Boolean(mask & (1 << index) != 0);
                }
            }
            if let Some(variable) = current.variables.iter_mut().find(|variable| variable.id == "age")
                && let Some(value) = variable.values.get_mut("light")
            {
                *value = VariableValue::Number(account_age);
            }

            for (source, expected_display) in cases {
                let original = Expr::parse_dynamic(source, &current).unwrap();
                let shown = original.display_dynamic(&current);
                assert_eq!(shown, expected_display, "{source} at age {account_age}, boolean combination {mask}");
                let displayed = Expr::parse_dynamic(&shown, &current).unwrap();
                assert_eq!(displayed, original, "{source} at age {account_age}, boolean combination {mask}");
                assert_eq!(displayed.display_dynamic(&current), shown, "display should be idempotent: {shown}");
                assert_eq!(
                    displayed.eval_dynamic(&current, &std::collections::BTreeMap::new()),
                    original.eval_dynamic(&current, &std::collections::BTreeMap::new()),
                    "evaluation changed after formatting {source} at age {account_age}, boolean combination {mask}"
                );
            }
        }
    }
}

#[test]
fn mutable_reference_walk_rejects_excessive_depth() {
    let mut expr = Expr::var("n");
    for _ in 0..=MAX_EXPR_DEPTH {
        expr = Expr::op(ExprOp::Not, vec![expr]);
    }
    assert!(expr.references_mut().is_none());
}

#[test]
fn errors_say_what_is_wrong() {
    assert_eq!(error(""), "Type an expression");
    assert_eq!(error("   "), "Type an expression");
    assert_eq!(error("count + 1"), "No variable `count`");
    assert_eq!(error("{nope}"), "No variable `nope`");
    assert_eq!(error("n-name +"), "Something is missing at the end");
    assert_eq!(error("(1 + 2"), "Add a closing )");
    assert_eq!(error("1 + 2)"), "Unexpected ): there's no ( to close");
    assert_eq!(error("\"open"), "Close the text with \"");
    assert_eq!(error("n-name s-name"), "Add an operator before `s`");
    assert_eq!(error("1 < 2 < 3"), "Compare two values at a time: join comparisons with `and` or `or`");
    assert_eq!(error("#12"), "`#12` isn't a colour (use #RRGGBB)");
    assert_eq!(error("1.2.3"), "`1.2.3` isn't a number");
    assert_eq!(error("{n-name"), "Add a closing }");
    assert_eq!(error("1 ? 2"), "Unexpected ?");
    assert_eq!(error("'it''s'"), "Unexpected '");
    let e = Expr::parse("n-name + nope", &vars()).unwrap_err();
    assert_eq!(e.at, 9, "where it went wrong");
}

#[test]
fn display_reads_back_as_the_same_expression() {
    let v = vars();
    for text in [
        "n-name + 1",
        "(n-name + 1) == 4",
        "not b-name",
        "not (b-name and false)",
        "(not b-name) == true",
        "not not b-name",
        "(not b-name) and b-name",
        "\"Count: \" + n-name",
        "\"quote \\\" and \\\\ slash\"",
        "0.125 * -3",
        "s-name != \"yo\" or n-name < 0.1",
        "-(n-name + 2)",
        "#FF8000 == #FF8000",
        "(1 + 2) + 3",
        "1 + 2 + 3",
    ] {
        let e = parse(text);
        let shown = e.display(&v);
        assert_eq!(Expr::parse(&shown, &v).as_ref(), Ok(&e), "{text} → {shown}");
    }
    assert_eq!(parse("0.125 * 2").display(&v), "0.125 * 2", "exact numbers");
}

#[test]
fn type_checks_explain_mismatches() {
    let v = vars();
    let ty = |t: &str| parse(t).type_of(&v);
    assert_eq!(ty("n-name + 1"), Ok(VariableType::Number));
    assert_eq!(ty("\"n = \" + n-name + b-name"), Ok(VariableType::String));
    assert_eq!(ty("n-name >= 2 and b-name"), Ok(VariableType::Boolean));
    assert_eq!(ty("-b-name"), Ok(VariableType::Boolean));
    assert_eq!(ty("b-name + 1"), Err("`+` adds numbers or joins text".into()));
    assert_eq!(ty("s-name * 2"), Err("`*` needs numbers".into()));
    assert_eq!(ty("n-name == s-name"), Err("Can't compare a number with text".into()));
    assert_eq!(ty("b-name < true"), Err("`<` compares two numbers or two texts".into()));
    assert_eq!(ty("n-name and b-name"), Err("`and` needs true or false on both sides".into()));
    assert_eq!(ty("not n-name"), Err("`not` needs true or false".into()));
    assert_eq!(ty("\"a\" + #FFFFFF"), Err("Colours can't be joined to text".into()));
    assert_eq!(Expr::var("gone").type_of(&v), Err("No variable `gone`".into()));
    assert_eq!(Expr::op(ExprOp::Equals, vec![n(1.0)]).type_of(&v), Err("`==` compares two values".into()));
    assert_eq!(n(f64::NAN).type_of(&v), Err("Numbers must be finite".into()));
    // Conditions give true or false; values fit their variable (text takes numbers).
    assert_eq!(parse("n-name > 1").check_condition(&v), Ok(()));
    assert_eq!(parse("n-name + 1").check_condition(&v), Err("A condition must be true or false, not a number".into()));
    assert_eq!(parse("n-name + 1").check_value(&v, VariableType::String), Ok(()));
    assert_eq!(parse("\"x\"").check_value(&v, VariableType::Number), Err("This gives text, but the variable holds a number".into()));
}

#[test]
fn hostile_text_is_bounded() {
    let v = vars();
    // Deep brackets stop with an error instead of overflowing the stack.
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(Expr::parse(&deep, &v).is_err());
    let nested = format!("{}1{}", "(".repeat(200), ")".repeat(200));
    assert_eq!(Expr::parse(&nested, &v).map_err(|e| e.message), Err("The expression is nested too deeply".into()));
    assert!(Expr::parse(&"not ".repeat(2000), &v).is_err());
    assert!(Expr::parse(&"1+".repeat(3000), &v).is_err(), "too long");
    // A long flat sum is fine, and type checks without recursion trouble.
    let flat = vec!["1"; 500].join("+");
    assert_eq!(Expr::parse(&flat, &v).map(|e| e.type_of(&v)), Ok(Ok(VariableType::Number)));
    // Deep expressions built by hand fail the type check, not the stack.
    let mut e = n(1.0);
    for _ in 0..(MAX_EXPR_DEPTH + 3) {
        e = Expr::op(ExprOp::Negate, vec![e]);
    }
    assert_eq!(e.type_of(&v), Err("The expression is nested too deeply".into()));
    for junk in ["\u{0}", "{}", "{", "#", "-", "--", "!", "!=", "e5", ".", "1e", "1e+", "\"\\", "and", "or or", "==", "((", "))", "🙂", "n-name🙂"]
    {
        let _ = Expr::parse(junk, &v);
    }
    // Mode lookups and modes as junk: whatever parses writes back to the same expression and
    // evaluates without trouble.
    let modes = std::collections::BTreeMap::from([("c".to_string(), "dark".to_string())]);
    for junk in [
        "n-name[",
        "n-name[]]",
        "n-name[[Dark]]",
        "n-name[Dark][Light]",
        "n-name [\u{0}]",
        "{n-name}[\"",
        "mode(",
        "mode((",
        "mode(Theme)[Dark]",
        "mode(mode(Theme))",
        "mode()()",
        "mode(\"Theme\"",
        "n-name[Dark] + mode(Theme)",
        "-n-name[Dark] * n-name[Light] / 0",
        "n-name[🙂]",
        "not mode(Theme)",
        "mode(Theme) < mode(Theme) == true",
    ] {
        if let Ok(e) = Expr::parse(junk, &v) {
            assert_eq!(Expr::parse(&e.display(&v), &v), Ok(e.clone()), "{junk}");
            let _ = e.eval(&v, &modes);
        }
    }
}

#[test]
fn names_that_look_like_literals_read_back_as_variables() {
    for name in ["count", "count total", "2x", "1", "-3", ".5", "(x)", "#ff", "\"q\"", "not", "true"] {
        let mut v = vars();
        if let Some(var) = v.variables.iter_mut().find(|x| x.id == "n") {
            var.name = name.into();
        }
        for e in [Expr::var("n"), Expr::op(ExprOp::Add, vec![Expr::var("n"), n(1.0)]), Expr::InMode { variable: "n".into(), mode: "dark".into() }] {
            let shown = e.display(&v);
            assert_eq!(Expr::parse(&shown, &v), Ok(e.clone()), "{name:?} shown as {shown:?}");
        }
    }
    // Plain names stay bare, and literals still read as literals.
    let v = vars();
    assert_eq!(Expr::var("n").display(&v), "n-name");
    assert_eq!(parse("1"), n(1.0));
    assert_eq!(parse("\"q\""), Expr::Value(VariableValue::String("q".into())));
}

#[test]
fn not_of_a_variable_reads_back_when_a_name_starts_with_not() {
    use ExprOp::*;
    let mut v = vars();
    let b = v.variables.iter().find(|x| x.id == "b").unwrap().clone();
    for (id, name) in [("done", "done"), ("nd", "not done")] {
        let mut x = b.clone();
        x.id = id.into();
        x.name = name.into();
        v.variables.push(x);
    }
    let not_done = Expr::op(Not, vec![Expr::var("done")]);
    for e in [
        not_done.clone(),
        Expr::op(And, vec![Expr::var("b"), not_done.clone()]),
        Expr::op(Equals, vec![not_done.clone(), Expr::Value(VariableValue::Boolean(true))]),
        Expr::op(Not, vec![not_done.clone()]),
    ] {
        let shown = e.display(&v);
        assert_eq!(Expr::parse(&shown, &v), Ok(e.clone()), "shown as {shown:?}");
    }
    assert_eq!(Expr::op(Not, vec![Expr::var("b")]).display(&vars()), "not b-name", "plain names stay bare");
}

#[test]
fn literal_true_false_survive_variables_named_true_false() {
    use ExprOp::*;
    let lit = |x: bool| Expr::Value(VariableValue::Boolean(x));
    for name in ["true", "false", "TRUE"] {
        let mut v = vars();
        if let Some(var) = v.variables.iter_mut().find(|x| x.id == "b") {
            var.name = name.into();
        }
        for e in [
            Expr::op(Equals, vec![Expr::var("b"), lit(true)]),
            Expr::op(Equals, vec![Expr::var("b"), lit(false)]),
            Expr::op(Not, vec![lit(false)]),
            Expr::var("b"),
        ] {
            let shown = e.display(&v);
            assert_eq!(Expr::parse(&shown, &v), Ok(e.clone()), "{name:?} shown as {shown:?}");
        }
        assert_eq!(Expr::parse(&format!("{{{name}}}"), &v), Ok(Expr::var("b")));
    }
}
