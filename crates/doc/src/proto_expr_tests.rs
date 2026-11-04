//! Prototype expressions and nested (conditional) actions.

use std::collections::BTreeMap;

use crate::color::Color;
use crate::proto::*;
use crate::{Collection, Mode, NodeId, Variable, VariableType, VariableValue, Variables};

pub(crate) fn vars() -> Variables {
    let values = |a: VariableValue, b: VariableValue| BTreeMap::from([("light".to_string(), a), ("dark".to_string(), b)]);
    let var = |id: &str, kind, values| Variable {
        id: id.into(),
        name: format!("{id}-name"),
        collection: "c".into(),
        kind,
        values,
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    };
    Variables {
        collections: vec![Collection {
            id: "c".into(),
            name: "Theme".into(),
            modes: vec![Mode { id: "light".into(), name: "Light".into() }, Mode { id: "dark".into(), name: "Dark".into() }],
            extends: None,
        }],
        variables: vec![
            var("n", VariableType::Number, values(VariableValue::Number(3.0), VariableValue::Number(7.0))),
            var("s", VariableType::String, values(VariableValue::String("hi".into()), VariableValue::String("yo".into()))),
            var("b", VariableType::Boolean, values(VariableValue::Boolean(true), VariableValue::Boolean(false))),
            var("alias", VariableType::Number, values(VariableValue::Alias("n".into()), VariableValue::Alias("n".into()))),
        ],
    }
}

fn n(x: f64) -> Expr {
    Expr::Value(VariableValue::Number(x))
}

fn s(x: &str) -> Expr {
    Expr::Value(VariableValue::String(x.into()))
}

fn b(x: bool) -> Expr {
    Expr::Value(VariableValue::Boolean(x))
}

fn eval(e: &Expr) -> Option<VariableValue> {
    e.eval(&vars(), &BTreeMap::new())
}

#[test]
fn arithmetic_text_and_comparisons() {
    use ExprOp::*;
    let num = |x: f64| Some(VariableValue::Number(x));
    let yes = Some(VariableValue::Boolean(true));
    let no = Some(VariableValue::Boolean(false));
    assert_eq!(eval(&Expr::op(Add, vec![Expr::var("n"), n(1.0)])), num(4.0));
    assert_eq!(eval(&Expr::op(Subtract, vec![n(10.0), Expr::var("alias"), n(2.0)])), num(5.0), "left to right, through aliases");
    assert_eq!(eval(&Expr::op(Multiply, vec![Expr::var("n"), n(2.5)])), num(7.5));
    assert_eq!(eval(&Expr::op(Divide, vec![n(9.0), Expr::var("n")])), num(3.0));
    assert_eq!(eval(&Expr::op(Divide, vec![n(1.0), n(0.0)])), None, "division by zero");
    assert_eq!(eval(&Expr::op(Multiply, vec![n(1e300), n(1e300)])), None, "overflow");
    assert_eq!(eval(&Expr::op(Negate, vec![Expr::var("n")])), num(-3.0));
    // Text joins.
    assert_eq!(eval(&Expr::op(Add, vec![s("Count: "), Expr::var("n"), s(" / "), b(true)])), Some(VariableValue::String("Count: 3 / true".into())));
    assert_eq!(eval(&Expr::op(Add, vec![s("x"), Expr::Value(VariableValue::Color(Color::BLACK))])), None);
    // Comparisons.
    assert_eq!(eval(&Expr::op(Equals, vec![Expr::var("n"), n(3.0)])), yes);
    assert_eq!(eval(&Expr::op(Equals, vec![n(0.1 + 0.2), n(0.3)])), yes, "float noise");
    assert_eq!(eval(&Expr::op(NotEqual, vec![Expr::var("s"), s("hi")])), no);
    assert_eq!(eval(&Expr::op(Equals, vec![Expr::var("s"), n(3.0)])), no, "different types differ");
    assert_eq!(eval(&Expr::op(Less, vec![Expr::var("n"), n(4.0)])), yes);
    assert_eq!(eval(&Expr::op(LessOrEqual, vec![n(4.0), n(4.0)])), yes);
    assert_eq!(eval(&Expr::op(Greater, vec![s("b"), s("a")])), yes, "strings compare");
    assert_eq!(eval(&Expr::op(GreaterOrEqual, vec![n(1.0), n(2.0)])), no);
    assert_eq!(eval(&Expr::op(Greater, vec![n(1.0), s("a")])), None);
    assert_eq!(eval(&Expr::op(Less, vec![n(1.0)])), None, "two operands");
    // Logic.
    assert_eq!(eval(&Expr::op(And, vec![Expr::var("b"), b(true)])), yes);
    assert_eq!(eval(&Expr::op(Or, vec![b(false), b(false)])), no);
    assert_eq!(eval(&Expr::op(Not, vec![Expr::var("b")])), no);
    assert_eq!(eval(&Expr::op(And, vec![b(true), n(1.0)])), None);
    assert_eq!(eval(&Expr::op(And, vec![])), None);
}

#[test]
fn variables_resolve_in_the_modes_given() {
    let dark = BTreeMap::from([("c".to_string(), "dark".to_string())]);
    let e = Expr::op(ExprOp::Add, vec![Expr::var("n"), n(1.0)]);
    assert_eq!(e.eval(&vars(), &dark), Some(VariableValue::Number(8.0)));
    assert!(!Expr::var("b").holds(&vars(), &dark));
    assert!(Expr::var("b").holds(&vars(), &BTreeMap::new()));
    // Missing variables and non-boolean conditions don't hold.
    assert!(!Expr::var("missing").holds(&vars(), &dark));
    assert!(!n(1.0).holds(&vars(), &dark));
    assert_eq!(e.variables(), ["n"]);
}

#[test]
fn display_reads_like_a_formula() {
    let v = vars();
    let e = Expr::op(ExprOp::Equals, vec![Expr::op(ExprOp::Add, vec![Expr::var("n"), n(1.0)]), n(4.0)]);
    assert_eq!(e.display(&v), "(n-name + 1) == 4");
    assert_eq!(Expr::op(ExprOp::Not, vec![Expr::var("b")]).display(&v), "not b-name");
    assert_eq!(Expr::op(ExprOp::Add, vec![s("a"), Expr::var("gone")]).display(&v), "\"a\" + (missing variable)");
}

#[test]
fn deep_and_hostile_expressions_are_bounded() {
    let mut e = n(1.0);
    for _ in 0..(MAX_EXPR_DEPTH + 5) {
        e = Expr::op(ExprOp::Add, vec![e, n(1.0)]);
    }
    assert_eq!(eval(&e), None, "too deep");
    assert!(e.display(&vars()).contains('…'));
    assert!(e.variables().is_empty());
    assert_eq!(eval(&n(f64::NAN)), None);
    let long = "x".repeat(8000);
    let joined = Expr::op(ExprOp::Add, vec![s(&long), s(&long)]);
    let Some(VariableValue::String(t)) = eval(&joined) else { panic!() };
    assert_eq!(t.chars().count(), 10_000, "capped");
    // Past the cap a colour operand still fails the expression, and empty operands add nothing.
    let color = Expr::Value(VariableValue::Color(Color::BLACK));
    let mut ops = vec![s(&"x".repeat(10_000))];
    ops.extend((0..50).map(|_| s("")));
    assert!(matches!(eval(&Expr::op(ExprOp::Add, ops.clone())), Some(VariableValue::String(t)) if t.chars().count() == 10_000));
    ops.push(color);
    assert_eq!(eval(&Expr::op(ExprOp::Add, ops)), None, "a colour past the cap is still a type error");
}

fn nav(id: u64) -> Action {
    Action::Navigate { dest: NodeId(id), preserve_scroll: false }
}

fn cond(blocks: Vec<(Option<Expr>, Vec<Action>)>) -> Action {
    Action::Conditional { blocks: blocks.into_iter().map(|(condition, actions)| ConditionalBlock { condition, actions }).collect() }
}

#[test]
fn walking_nested_actions() {
    let mut actions = vec![
        nav(1),
        cond(vec![
            (Some(Expr::var("b")), vec![nav(2), cond(vec![(None, vec![nav(3)])])]),
            (None, vec![Action::SetVariable { variable: "n".into(), value: serde_json::Value::Null, expr: Some(Expr::var("alias")) }]),
        ]),
        Action::Back,
    ];
    let dests: Vec<NodeId> = walk(&actions).filter_map(Action::dest).collect();
    assert_eq!(dests, [NodeId(1), NodeId(2), NodeId(3)], "depth-first, in order");
    assert_eq!(walk(&actions).count(), 7);
    let mut vars_used: Vec<&str> = actions.iter().flat_map(Action::variables).collect();
    vars_used.sort_unstable();
    assert_eq!(vars_used, ["alias", "b", "n"]);
    walk_mut(&mut actions, &mut |a| {
        if let Action::Navigate { dest, .. } = a {
            dest.0 += 10;
        }
    });
    assert_eq!(walk(&actions).filter_map(Action::dest).collect::<Vec<_>>(), [NodeId(11), NodeId(12), NodeId(13)]);
    // Dropping navigations empties the inner conditional (gone) but not the outer one.
    retain_deep(&mut actions, &mut |a| !matches!(a, Action::Navigate { .. }));
    assert_eq!(walk(&actions).count(), 3, "{actions:?}");
    retain_deep(&mut actions, &mut |a| !matches!(a, Action::SetVariable { .. }));
    assert_eq!(actions, [Action::Back]);
}

#[test]
fn conditional_choice_and_deep_nesting() {
    let v = vars();
    let blocks = [
        ConditionalBlock { condition: Some(Expr::var("missing")), actions: vec![nav(1)] },
        ConditionalBlock { condition: Some(Expr::var("b")), actions: vec![nav(2)] },
        ConditionalBlock { condition: None, actions: vec![nav(3)] },
    ];
    assert_eq!(ConditionalBlock::choose(&blocks, &v, &BTreeMap::new()), Some(&[nav(2)][..]));
    let dark = BTreeMap::from([("c".to_string(), "dark".to_string())]);
    assert_eq!(ConditionalBlock::choose(&blocks, &v, &dark), Some(&[nav(3)][..]));
    assert_eq!(ConditionalBlock::choose(&blocks[..1], &v, &dark), None);
    // Nesting deeper than the limit stops walking (no stack overflow).
    let mut a = nav(9);
    for _ in 0..(MAX_ACTION_DEPTH * 4) {
        a = cond(vec![(None, vec![a])]);
    }
    let list = vec![a];
    assert_eq!(walk(&list).count(), MAX_ACTION_DEPTH + 1);
}

#[test]
fn serde_round_trip_and_old_files() {
    let a = vec![
        cond(vec![(Some(Expr::op(ExprOp::GreaterOrEqual, vec![Expr::var("n"), n(2.0)])), vec![nav(4)]), (None, vec![])]),
        Action::SetVariableMode { collection: "c".into(), mode: "dark".into() },
        Action::SetVariable { variable: "n".into(), value: serde_json::json!(1), expr: None },
    ];
    let j = serde_json::to_value(&a).unwrap_or_default();
    assert_eq!(j[2].get("expr"), None, "no expression: the field is left out");
    let back: Vec<Action> = serde_json::from_value(j).unwrap_or_default();
    assert_eq!(back, a);
    // Files written before expressions existed still read.
    let old: Action = serde_json::from_str(r#"{"type":"setVariable","variable":"n","value":2}"#).unwrap_or(Action::Back);
    assert_eq!(old, Action::SetVariable { variable: "n".into(), value: serde_json::json!(2), expr: None });
}

#[test]
fn operands_holding_too_many_string_bytes_are_refused() {
    let long = "x".repeat(300_000);
    let few = Expr::op(ExprOp::Add, vec![s(&long), s(&long)]);
    assert!(eval(&few).is_some());
    let many = Expr::op(ExprOp::Add, (0..8).map(|_| s(&long)).collect());
    assert_eq!(eval(&many), None);
}
