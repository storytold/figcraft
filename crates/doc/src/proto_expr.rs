//! Prototype expressions: the values set-variable actions write and the conditions of
//! conditional actions, built from literals, variable references (also a variable's value in a
//! given mode of its collection), the current mode of a collection, and operators.

use std::collections::BTreeMap;

use figcraft_color::Hsl as HslColor;
use serde::{Deserialize, Serialize};

use crate::{CollectionId, ModeId, VariableValue, Variables};

/// Deepest expression nesting evaluated (deeper parts evaluate to nothing).
pub const MAX_EXPR_DEPTH: usize = 32;
/// Longest string an expression builds (characters).
const MAX_STRING: usize = 10_000;
/// The most string bytes one operation's operands may hold together (generous: results cap at `MAX_STRING`).
const MAX_OPERAND_BYTES: usize = 1 << 20;

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
    /// Choose `then_expr` when `condition` is true, otherwise `else_expr`.
    Conditional { condition: Box<Expr>, then_expr: Box<Expr>, else_expr: Box<Expr> },
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
    /// Create an opaque colour from hue degrees and saturation/lightness percentages.
    Hsl,
    /// Interpolate between two colours in sRGB by a fraction clamped to 0..1.
    Lerp,
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
            ExprOp::Hsl => "hsl",
            ExprOp::Lerp => "lerp",
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
        self.eval_at(vars, modes, false, 0)
    }

    /// Evaluate a dynamic variable expression with every color result made opaque. Opacity is
    /// edited separately on the output variable, so color inputs and interpolation cannot carry
    /// their alpha into the result.
    pub fn eval_dynamic(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>) -> Option<VariableValue> {
        self.eval_at(vars, modes, true, 0)
    }

    /// Whether a condition holds: a `true` boolean (anything else, or an error, doesn't).
    pub fn holds(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>) -> bool {
        matches!(self.eval(vars, modes), Some(VariableValue::Boolean(true)))
    }

    fn eval_at(&self, vars: &Variables, modes: &BTreeMap<CollectionId, ModeId>, ignore_color_alpha: bool, depth: usize) -> Option<VariableValue> {
        if depth > MAX_EXPR_DEPTH {
            return None;
        }
        let (op, args) = match self {
            Expr::Conditional { condition, then_expr, else_expr } => {
                return match condition.eval_at(vars, modes, ignore_color_alpha, depth + 1)? {
                    VariableValue::Boolean(true) => then_expr.eval_at(vars, modes, ignore_color_alpha, depth + 1),
                    VariableValue::Boolean(false) => else_expr.eval_at(vars, modes, ignore_color_alpha, depth + 1),
                    _ => None,
                };
            }
            Expr::Value(VariableValue::Alias(id)) => return vars.resolve(id, modes).map(|value| expression_color(value, ignore_color_alpha)),
            Expr::Value(VariableValue::Number(n)) => return n.is_finite().then_some(VariableValue::Number(*n)),
            Expr::Value(v) => return Some(expression_color(v.clone(), ignore_color_alpha)),
            Expr::Mode { collection } => {
                return vars.collection_by_id(collection)?.current_mode(modes).map(|m| VariableValue::String(m.name.clone()));
            }
            Expr::InMode { variable, mode } => {
                let v = vars.get_by_id(variable)?;
                let c = vars.collection_by_id(&v.collection)?;
                let mut at = modes.clone();
                at.insert(c.id.clone(), c.mode_by_id(mode)?.id.clone());
                return vars.resolve(&v.id, &at).map(|value| expression_color(value, ignore_color_alpha));
            }
            Expr::Op { op, args } => (*op, args),
        };
        // Retained operands are bounded by bytes, so repeated references to one long string can't multiply it.
        let mut text_bytes = 0usize;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let v = a.eval_at(vars, modes, ignore_color_alpha, depth + 1)?;
            if let VariableValue::String(s) = &v {
                text_bytes = text_bytes.saturating_add(s.len());
                if text_bytes > MAX_OPERAND_BYTES {
                    return None;
                }
            }
            vals.push(v);
        }
        apply(op, &vals).map(|value| expression_color(value, ignore_color_alpha))
    }

    /// Every variable the expression reads.
    pub fn variables(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_variables(&mut out, 0);
        out
    }

    /// The variable ids this expression reads, mutable for import and library remapping.
    pub fn references_mut(&mut self) -> Option<Vec<&mut String>> {
        self.references_mut_at(0)
    }

    fn references_mut_at(&mut self, depth: usize) -> Option<Vec<&mut String>> {
        if depth > MAX_EXPR_DEPTH {
            return None;
        }
        match self {
            Expr::Value(VariableValue::Alias(id)) => Some(vec![id]),
            Expr::InMode { variable, .. } => Some(vec![variable]),
            Expr::Op { args, .. } => {
                let mut out = Vec::new();
                for arg in args {
                    out.extend(arg.references_mut_at(depth + 1)?);
                }
                Some(out)
            }
            Expr::Conditional { condition, then_expr, else_expr } => {
                let mut out = condition.references_mut_at(depth + 1)?;
                out.extend(then_expr.references_mut_at(depth + 1)?);
                out.extend(else_expr.references_mut_at(depth + 1)?);
                Some(out)
            }
            Expr::Value(_) | Expr::Mode { .. } => Some(Vec::new()),
        }
    }

    fn collect_variables<'a>(&'a self, out: &mut Vec<&'a str>, depth: usize) {
        match self {
            Expr::Value(VariableValue::Alias(id)) | Expr::InMode { variable: id, .. } => out.push(id),
            Expr::Op { args, .. } if depth < MAX_EXPR_DEPTH => args.iter().for_each(|a| a.collect_variables(out, depth + 1)),
            Expr::Conditional { condition, then_expr, else_expr } if depth < MAX_EXPR_DEPTH => {
                condition.collect_variables(out, depth + 1);
                then_expr.collect_variables(out, depth + 1);
                else_expr.collect_variables(out, depth + 1);
            }
            _ => {}
        }
    }

    /// Readable text, variables by name (`count + 1`, `open == false`); [`Expr::parse`] reads it
    /// back.
    pub fn display(&self, vars: &Variables) -> String {
        self.display_at(vars, 0, false)
    }

    /// Readable text for a dynamic-variable expression, with every variable reference braced;
    /// [`Expr::parse_dynamic`] reads it back.
    pub fn display_dynamic(&self, vars: &Variables) -> String {
        self.display_at(vars, 0, true)
    }

    fn display_at(&self, vars: &Variables, depth: usize, braced_variables: bool) -> String {
        if depth > MAX_EXPR_DEPTH {
            return "…".into();
        }
        match self {
            Expr::Conditional { condition, then_expr, else_expr } => {
                let part = |expr: &Expr| {
                    let shown = expr.display_at(vars, depth + 1, braced_variables);
                    if matches!(expr, Expr::Conditional { .. }) { format!("({shown})") } else { shown }
                };
                format!("if {} then {} else {}", part(condition), part(then_expr), part(else_expr))
            }
            Expr::Value(VariableValue::Alias(id)) => {
                vars.get_by_id(id).map_or_else(|| "(missing variable)".into(), |v| var_ref(vars, v, braced_variables))
            }
            Expr::Value(VariableValue::String(s)) => quote(s),
            // Exact numbers and wrapped hex colours, so the text reads back as the same expression.
            Expr::Value(VariableValue::Number(n)) => format!("{n}"),
            Expr::Value(VariableValue::Color(c)) => {
                if braced_variables {
                    format!("hex({})", c.to_css_hex())
                } else {
                    c.to_css_hex()
                }
            }
            Expr::Value(v) => v.display(),
            Expr::Mode { collection } => {
                let name = vars.collection_by_id(collection).map_or("(missing collection)", |c| c.name.as_str());
                format!("mode({})", name_arg(name))
            }
            Expr::InMode { variable, mode } => {
                let var = vars.get_by_id(variable);
                let mode = var
                    .and_then(|v| vars.collection_by_id(&v.collection))
                    .and_then(|c| c.mode_by_id(mode))
                    .map_or("(missing mode)", |m| m.name.as_str());
                format!("{}[{}]", Expr::var(variable.clone()).display_at(vars, depth + 1, braced_variables), name_arg(mode))
            }
            Expr::Op { op, args } => {
                let parts: Vec<String> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        if braced_variables {
                            display_dynamic_operand(*op, arg, index, vars, depth + 1)
                        } else {
                            match arg {
                                // `not` binds looser than what it may sit in (`(not a) == b`).
                                Expr::Op { op, args } if (args.len() > 1 && !matches!(op, ExprOp::Hsl | ExprOp::Lerp)) || *op == ExprOp::Not => {
                                    format!("({})", arg.display_at(vars, depth + 1, braced_variables))
                                }
                                _ => arg.display_at(vars, depth + 1, braced_variables),
                            }
                        }
                    })
                    .collect();
                match (op, parts.as_slice()) {
                    (ExprOp::Hsl | ExprOp::Lerp, _) => format!("{}({})", op.symbol(), parts.join(", ")),
                    // `not done` reads as a variable named "not done" if there is one: then
                    // `not (done)`.
                    (ExprOp::Not, [x]) => {
                        let bare = format!("not {x}");
                        let parses_back = if braced_variables {
                            Expr::parse_dynamic(&bare, vars).is_ok_and(|e| e == *self)
                        } else {
                            Expr::parse(&bare, vars).is_ok_and(|e| e == *self)
                        };
                        if x.starts_with('(') || parses_back { bare } else { format!("not ({x})") }
                    }
                    (ExprOp::Negate, [x]) => format!("-{x}"),
                    _ => parts.join(&format!(" {} ", op.symbol())),
                }
            }
        }
    }
}

fn display_dynamic_operand(parent: ExprOp, child: &Expr, index: usize, vars: &Variables, depth: usize) -> String {
    let shown = child.display_at(vars, depth, true);
    let parent_precedence = expression_precedence(parent);
    let child_precedence = match child {
        Expr::Conditional { .. } => 0,
        Expr::Op { op: ExprOp::Hsl | ExprOp::Lerp, .. } => return shown,
        Expr::Op { op, .. } => expression_precedence(*op),
        Expr::Value(_) | Expr::Mode { .. } | Expr::InMode { .. } => return shown,
    };
    let same_operator = matches!(child, Expr::Op { op, .. } if *op == parent);
    let comparison_parent =
        matches!(parent, ExprOp::Equals | ExprOp::NotEqual | ExprOp::Less | ExprOp::LessOrEqual | ExprOp::Greater | ExprOp::GreaterOrEqual);
    let comparison_child = matches!(
        child,
        Expr::Op { op: ExprOp::Equals | ExprOp::NotEqual | ExprOp::Less | ExprOp::LessOrEqual | ExprOp::Greater | ExprOp::GreaterOrEqual, .. }
    );
    let needs_parentheses = child_precedence < parent_precedence
        || (child_precedence == parent_precedence && (index > 0 || same_operator || (comparison_parent && comparison_child)));
    if needs_parentheses { format!("({shown})") } else { shown }
}

fn expression_precedence(op: ExprOp) -> u8 {
    match op {
        ExprOp::Or => 1,
        ExprOp::And => 2,
        ExprOp::Not => 3,
        ExprOp::Equals | ExprOp::NotEqual | ExprOp::Less | ExprOp::LessOrEqual | ExprOp::Greater | ExprOp::GreaterOrEqual => 4,
        ExprOp::Add | ExprOp::Subtract => 5,
        ExprOp::Multiply | ExprOp::Divide => 6,
        ExprOp::Negate => 7,
        ExprOp::Hsl | ExprOp::Lerp => 8,
    }
}

fn expression_color(value: VariableValue, ignore_color_alpha: bool) -> VariableValue {
    match (value, ignore_color_alpha) {
        (VariableValue::Color(color), true) => VariableValue::Color(color.with_alpha(1.0)),
        (value, _) => value,
    }
}

/// A variable as written in an expression: its bare name when that reads back as this variable,
/// otherwise `{name}` (or `{id}` when even the braced name wouldn't), so names like `1`, `2x` or
/// `"q"` don't read back as literals or errors.
fn var_ref(vars: &Variables, v: &crate::Variable, braced_variables: bool) -> String {
    if braced_variables {
        let name_is_exact = !v.name.contains('}') && v.name.trim() == v.name && vars.get(&v.name).is_some_and(|x| x.id == v.id);
        return if name_is_exact { format!("{{{}}}", v.name) } else { format!("{{{}}}", v.id) };
    }
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
        VariableValue::Color(_) | VariableValue::Alias(_) | VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => None,
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
            let mut chars = 0;
            for v in vals {
                // Every operand is checked (a colour makes the whole expression None); only the
                // characters it adds are counted, and appending stops at the cap.
                let t = as_text(v)?;
                if chars < MAX_STRING {
                    let before = s.len();
                    s.extend(t.chars().take(MAX_STRING - chars));
                    chars += s[before..].chars().count();
                }
            }
            Some(VariableValue::String(s))
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
        Hsl => match vals {
            [VariableValue::Number(h), VariableValue::Number(s), VariableValue::Number(l)] => {
                let (h, s, l) = ((*h % 360.0) as f32, (*s / 100.0).clamp(0.0, 1.0) as f32, (*l / 100.0).clamp(0.0, 1.0) as f32);
                (h.is_finite() && s.is_finite() && l.is_finite()).then(|| VariableValue::Color(HslColor { h, s, l }.to_color()))
            }
            _ => None,
        },
        Lerp => match vals {
            [VariableValue::Color(a), VariableValue::Color(b), VariableValue::Number(t)] if t.is_finite() => {
                Some(VariableValue::Color(a.lerp(*b, t.clamp(0.0, 1.0) as f32)))
            }
            _ => None,
        },
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
