//! Prototype expressions: the values set-variable actions write and the conditions of
//! conditional actions, built from literals, variable references (also a variable's value in a
//! given mode of its collection), the current mode of a collection, and operators.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CollectionId, ModeId, VariableValue, Variables};

/// Deepest expression nesting evaluated (deeper parts evaluate to nothing).
pub const MAX_EXPR_DEPTH: usize = 32;
/// Longest string an expression builds (characters).
const MAX_STRING: usize = 10_000;

/// An expression over variables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Expr {
    /// A literal, or a variable reference (`VariableValue::Alias`).
    Value(VariableValue),
    /// An operator applied to its arguments.
    Op { op: ExprOp, args: Vec<Expr> },
    /// The name of the mode collection `collection` is in where the expression is evaluated (the
    /// layer whose interaction runs), as text: `mode(Theme) == "Dark"`.
    Mode { collection: CollectionId },
    /// Variable `variable`'s value in mode `mode` of its collection, whichever mode is chosen:
    /// `accent[Dark]`.
    InMode { variable: String, mode: ModeId },
}

/// Expression operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExprOp {
    /// Numbers add; with a string among the arguments, everything joins as text.
    Add,
    Subtract,
    Multiply,
    Divide,
    Equals,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    And,
    Or,
    /// Boolean not.
    Not,
    /// Numeric minus.
    Negate,
}

impl ExprOp {
    /// The operator as written between (or before) its operands.
    pub fn symbol(self) -> &'static str {
        match self {
            ExprOp::Add => "+",
            ExprOp::Subtract => "-",
            ExprOp::Multiply => "*",
            ExprOp::Divide => "/",
            ExprOp::Equals => "==",
            ExprOp::NotEqual => "!=",
            ExprOp::Less => "<",
            ExprOp::LessOrEqual => "<=",
            ExprOp::Greater => ">",
            ExprOp::GreaterOrEqual => ">=",
            ExprOp::And => "and",
            ExprOp::Or => "or",
            ExprOp::Not => "not",
            ExprOp::Negate => "-",
        }
    }
}

impl Expr {
    /// A reference to variable `id`.
    pub fn var(id: impl Into<String>) -> Expr {
        Expr::Value(VariableValue::Alias(id.into()))
    }

    pub fn op(op: ExprOp, args: Vec<Expr>) -> Expr {
        Expr::Op { op, args }
    }

    /// The value, with variables resolved in `modes`. `None` when a variable is missing, the
    /// types don't fit the operator, a number isn't finite or the nesting is too deep.
    pub fn eval(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>) -> Option<VariableValue> {
        self.eval_at(vars, modes, 0)
    }

    /// Whether a condition holds: a `true` boolean (anything else, or an error, doesn't).
    pub fn holds(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>) -> bool {
        matches!(self.eval(vars, modes), Some(VariableValue::Boolean(true)))
    }

    fn eval_at(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>, depth: usize) -> Option<VariableValue> {
        if depth > MAX_EXPR_DEPTH {
            return None;
        }
        let (op, args) = match self {
            Expr::Value(VariableValue::Alias(id)) => return vars.resolve(id, modes),
            Expr::Value(VariableValue::Number(n)) => return n.is_finite().then_some(VariableValue::Number(*n)),
            Expr::Value(v) => return Some(v.clone()),
            Expr::Mode { collection } => return vars.collection(collection)?.current_mode(modes).map(|m| VariableValue::String(m.name.clone())),
            Expr::InMode { variable, mode } => {
                let v = vars.get(variable)?;
                let c = vars.collection(&v.collection)?;
                let mut at = modes.clone();
                at.insert(c.id.clone(), c.find_mode(mode)?.id.clone());
                return vars.resolve(&v.id, &at);
            }
            Expr::Op { op, args } => (*op, args),
        };
        let vals: Vec<VariableValue> = args.iter().map(|a| a.eval_at(vars, modes, depth + 1)).collect::<Option<_>>()?;
        apply(op, &vals)
    }

    /// Every variable the expression reads.
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_variables(&mut out, 0);
        out
    }

    fn collect_variables<'a>(&'a self, out: &mut Vec<&'a str>, depth: usize) {
        match self {
            Expr::Value(VariableValue::Alias(id)) | Expr::InMode { variable: id, .. } => out.push(id),
            Expr::Op { args, .. } if depth < MAX_EXPR_DEPTH => args.iter().for_each(|a| a.collect_variables(out, depth + 1)),
            _ => {}
        }
    }

    /// Readable text, variables by name (`count + 1`, `open == false`); [`Expr::parse`] reads it
    /// back.
    pub fn display(&self, vars: &Variables) -> String {
        self.display_at(vars, 0)
    }

    fn display_at(&self, vars: &Variables, depth: usize) -> String {
        if depth > MAX_EXPR_DEPTH {
            return "…".into();
        }
        match self {
            Expr::Value(VariableValue::Alias(id)) => vars.get(id).map_or_else(|| "(missing variable)".into(), |v| var_ref(vars, v)),
            Expr::Value(VariableValue::String(s)) => quote(s),
            // Exact numbers and `#` colours, so the text reads back as the same expression.
            Expr::Value(VariableValue::Number(n)) => format!("{n}"),
            Expr::Value(VariableValue::Color(c)) => c.to_css_hex(),
            Expr::Value(v) => v.display(),
            Expr::Mode { collection } => {
                let name = vars.collection(collection).map_or("(missing collection)", |c| c.name.as_str());
                format!("mode({})", name_arg(name))
            }
            Expr::InMode { variable, mode } => {
                let var = vars.get(variable);
                let mode =
                    var.and_then(|v| vars.collection(&v.collection)).and_then(|c| c.find_mode(mode)).map_or("(missing mode)", |m| m.name.as_str());
                format!("{}[{}]", Expr::var(variable.clone()).display_at(vars, depth + 1), name_arg(mode))
            }
            Expr::Op { op, args } => {
                let parts: Vec<String> = args
                    .iter()
                    .map(|a| match a {
                        // `not` binds looser than what it may sit in (`(not a) == b`).
                        Expr::Op { op, args } if args.len() > 1 || *op == ExprOp::Not => format!("({})", a.display_at(vars, depth + 1)),
                        _ => a.display_at(vars, depth + 1),
                    })
                    .collect();
                match (op, parts.as_slice()) {
                    (ExprOp::Not, [x]) => format!("not {x}"),
                    (ExprOp::Negate, [x]) => format!("-{x}"),
                    _ => parts.join(&format!(" {} ", op.symbol())),
                }
            }
        }
    }
}

/// A variable as written in an expression: its bare name when that reads back as this variable,
/// otherwise `{name}` (or `{id}` when even the braced name wouldn't), so names like `1`, `2x` or
/// `"q"` don't read back as literals or errors.
fn var_ref(vars: &Variables, v: &crate::Variable) -> String {
    let this = Expr::var(v.id.clone());
    if Expr::parse(&v.name, vars).is_ok_and(|e| e == this) {
        return v.name.clone();
    }
    let braced = !v.name.contains('}') && v.name.trim() == v.name && vars.get(&v.name).is_some_and(|x| x.id == v.id);
    if braced { format!("{{{}}}", v.name) } else { format!("{{{}}}", v.id) }
}

/// A collection or mode name as written inside `mode(…)` or `[…]`: as it is, or quoted when it has
/// spaces at its ends, quotes or brackets.
fn name_arg(name: &str) -> String {
    let plain = !name.is_empty() && name.trim() == name && !name.contains(['"', '\'', '\\', '(', ')', '[', ']', '{', '}']);
    if plain { name.to_string() } else { quote(name) }
}

/// `s` as a double-quoted text literal ([`Expr::parse`] reads it back).
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn number(v: &VariableValue) -> Option<f64> {
    match v {
        VariableValue::Number(n) => Some(*n),
        _ => None,
    }
}

fn boolean(v: &VariableValue) -> Option<bool> {
    match v {
        VariableValue::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn finite(n: f64) -> Option<VariableValue> {
    n.is_finite().then_some(VariableValue::Number(n))
}

/// Text for joining: numbers without trailing zeros, booleans as `true` / `false`.
fn as_text(v: &VariableValue) -> Option<String> {
    match v {
        VariableValue::String(s) => Some(s.clone()),
        VariableValue::Number(_) | VariableValue::Boolean(_) => Some(v.display()),
        VariableValue::Color(_) | VariableValue::Alias(_) => None,
    }
}

fn same(a: &VariableValue, b: &VariableValue) -> bool {
    match (a, b) {
        (VariableValue::Number(x), VariableValue::Number(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        (VariableValue::Color(x), VariableValue::Color(y)) => x.to_rgba8() == y.to_rgba8(),
        _ => a == b,
    }
}

/// `op` over already evaluated arguments.
fn apply(op: ExprOp, vals: &[VariableValue]) -> Option<VariableValue> {
    use ExprOp::*;
    let nums = || vals.iter().map(number).collect::<Option<Vec<f64>>>();
    let bools = || vals.iter().map(boolean).collect::<Option<Vec<bool>>>();
    let fold = |f: fn(f64, f64) -> Option<f64>| {
        let n = nums()?;
        let (first, rest) = n.split_first()?;
        rest.iter().try_fold(*first, |acc, x| f(acc, *x)).and_then(finite)
    };
    match op {
        Add if vals.iter().any(|v| matches!(v, VariableValue::String(_))) => {
            let mut s = String::new();
            for v in vals {
                s.push_str(&as_text(v)?);
            }
            Some(VariableValue::String(s.chars().take(MAX_STRING).collect()))
        }
        Add => fold(|a, b| Some(a + b)),
        Subtract => fold(|a, b| Some(a - b)),
        Multiply => fold(|a, b| Some(a * b)),
        Divide => fold(|a, b| (b != 0.0).then(|| a / b)),
        Equals | NotEqual => {
            let (first, rest) = vals.split_first()?;
            if rest.is_empty() {
                return None;
            }
            let eq = rest.iter().all(|v| same(first, v));
            Some(VariableValue::Boolean(if op == Equals { eq } else { !eq }))
        }
        Less | LessOrEqual | Greater | GreaterOrEqual => {
            let [a, b] = vals else { return None };
            let ord = match (a, b) {
                (VariableValue::Number(x), VariableValue::Number(y)) => x.partial_cmp(y)?,
                (VariableValue::String(x), VariableValue::String(y)) => x.cmp(y),
                _ => return None,
            };
            use std::cmp::Ordering as O;
            Some(VariableValue::Boolean(match op {
                Less => ord == O::Less,
                LessOrEqual => ord != O::Greater,
                Greater => ord == O::Greater,
                _ => ord != O::Less,
            }))
        }
        And => bools().filter(|b| !b.is_empty()).map(|b| VariableValue::Boolean(b.iter().all(|x| *x))),
        Or => bools().filter(|b| !b.is_empty()).map(|b| VariableValue::Boolean(b.iter().any(|x| *x))),
        Not => match vals {
            [VariableValue::Boolean(b)] => Some(VariableValue::Boolean(!b)),
            _ => None,
        },
        Negate => match vals {
            [VariableValue::Number(n)] => finite(-n),
            [VariableValue::Boolean(b)] => Some(VariableValue::Boolean(!b)),
            _ => None,
        },
    }
}
