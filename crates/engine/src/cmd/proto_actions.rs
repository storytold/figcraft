//! Editing an interaction's actions one at a time, inside conditional actions too: add, replace,
//! remove and reorder actions, and the if / else if / else blocks of a conditional action with
//! their conditions (the interaction editor builds conditionals with these).
//!
//! An action is addressed by a `path`: `[i]` is the interaction's action `i`, `[i, b, j]` is action
//! `j` of block `b` of the conditional action `i`, and so on (action and block indexes take turns).
//! Expressions come as text (`expression: "count + 1"`, read by [`Expr::parse`]) or as expression
//! JSON (`condition`, `expr`); both are checked: a condition must give true or false, a set
//! variable's expression a value the variable can hold.

use std::collections::BTreeMap;

use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp, MAX_ACTION_DEPTH, walk};
use figcraft_doc::{Variable, VariableType, VariableValue, Variables};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::{bool_of, f64_of, get, id, parse, req_index, req_targets, string};
use crate::{Result, Session, bad, spec};

/// Most actions in one list (an interaction's, or a block's).
pub const MAX_ACTIONS: usize = 200;
/// Most blocks in one conditional action.
pub const MAX_BLOCKS: usize = 50;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "proto.action.add",
            "Add prototype action",
            "{id?, index (interaction), path? (where it goes: [n] appends to an interaction with n actions, [i, b, j] inside block b of the conditional i; default: after the last action), action {type: …, as proto.interaction.add; setVariable takes `expression` text, conditional blocks take `expression` or `condition`}} → {path}",
            true,
            add
        ),
        spec!(
            "proto.action.set",
            "Edit prototype action",
            "{id?, index, path [action, block, action, …], action {…}}  replaces the action there",
            true,
            set
        ),
        spec!(
            "proto.action.remove",
            "Remove prototype action",
            "{id?, index, path}  (an interaction keeps at least one action: remove the interaction instead)",
            true,
            remove
        ),
        spec!("proto.action.move", "Reorder prototype action", "{id?, index, path, to (its new index in the same list)} → {path}", true, move_action),
        spec!(
            "proto.conditional.addBlock",
            "Add else if / else",
            "{id?, index, path (a conditional action), else? bool (add the else block; default: an else if before it), expression? (text, e.g. \"count >= 3\") | condition? (expression JSON); default condition: the first variable equal to its value} → {block}",
            true,
            add_block
        ),
        spec!(
            "proto.conditional.setCondition",
            "Edit condition",
            "{id?, index, path (a conditional action), block, expression (text) | condition (expression JSON)}  the condition must give true or false",
            true,
            set_condition
        ),
        spec!(
            "proto.conditional.removeBlock",
            "Remove if / else if / else",
            "{id?, index, path (a conditional action), block}  removing the if when only an else (or nothing) is left removes the whole conditional action → {removedAction}",
            true,
            remove_block
        ),
        spec!(
            "proto.evaluate",
            "Evaluate prototype expression",
            "{expression (text, e.g. \"count + 1\", \"accent[Dark]\", \"mode(Theme) == 'Dark'\") | expr (expression JSON), id? (the layer it runs on: variables resolve in its modes; default: the document's default modes)} → {value, type, text}  without changing anything",
            false,
            evaluate
        ),
    ]
}

/// An expression's value where it would run, as the prototype player computes it.
fn evaluate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.evaluate";
    let doc = s.doc()?;
    let vars = &doc.variables;
    let e = match (string(p, "expression"), parse::<Expr>(cmd, p, "expr")?) {
        (Some(text), _) => Expr::parse(&text, vars).map_err(|e| bad(cmd, format!("`{text}`: {e}")))?,
        (None, Some(mut e)) => {
            super::proto::canonical_expr(vars, &mut e, 0);
            e
        }
        (None, None) => return Err(bad(cmd, "`expression` (text) or `expr` (expression JSON) required")),
    };
    let kind = e.type_of(vars).map_err(|m| bad(cmd, m))?;
    let modes = match id(p, "id") {
        Some(n) if doc.contains(n) => doc.modes_for(n),
        Some(n) => return Err(bad(cmd, format!("no layer {n}"))),
        None => BTreeMap::new(),
    };
    let v = e.eval(vars, &modes).ok_or_else(|| bad(cmd, "it gives no value here (a division by zero, or a number too large)"))?;
    Ok(json!({"value": v, "type": kind, "text": e.display(vars)}))
}

/// The action at `path` in `actions`.
pub fn action_at<'a>(actions: &'a [Action], path: &[usize]) -> Option<&'a Action> {
    let (last, rest) = path.split_last()?;
    let mut list = actions;
    for pair in rest.chunks(2) {
        let [a, b] = pair else { return None };
        match list.get(*a)? {
            Action::Conditional { blocks } => list = &blocks.get(*b)?.actions,
            _ => return None,
        }
    }
    list.get(*last)
}

/// The action list `path` points into and the index in it (which may be one past the end).
fn list_mut<'a>(actions: &'a mut Vec<Action>, path: &[usize]) -> Option<(&'a mut Vec<Action>, usize)> {
    let (last, rest) = path.split_last()?;
    let mut list = actions;
    for pair in rest.chunks(2) {
        let [a, b] = pair else { return None };
        match list.get_mut(*a)? {
            Action::Conditional { blocks } => list = &mut blocks.get_mut(*b)?.actions,
            _ => return None,
        }
    }
    Some((list, *last))
}

/// A variable's value in its collection's default mode.
pub fn default_value(vars: &Variables, id: &str) -> Option<VariableValue> {
    vars.resolve(id, &BTreeMap::new())
}

/// A starting value for a variable of `kind`.
pub fn placeholder(kind: VariableType) -> VariableValue {
    match kind {
        VariableType::Number => VariableValue::Number(0.0),
        VariableType::String => VariableValue::String(String::new()),
        VariableType::Boolean => VariableValue::Boolean(true),
        VariableType::Color => VariableValue::Color(figcraft_color::Color::BLACK),
    }
}

/// What a condition or a set variable starts with for variable `v`: its value in the default
/// modes (a [`placeholder`] when it has none of its type).
pub fn start_value(vars: &Variables, v: &Variable) -> VariableValue {
    default_value(vars, &v.id).filter(|x| x.type_of() == Some(v.kind)).unwrap_or_else(|| placeholder(v.kind))
}

/// The condition a new if / else if starts with: the first boolean variable (else the first
/// variable) equal to its value; `true` when there are no variables.
pub fn default_condition(vars: &Variables) -> Expr {
    match vars.variables.iter().find(|v| v.kind == VariableType::Boolean).or(vars.variables.first()) {
        Some(v) => Expr::op(ExprOp::Equals, vec![Expr::var(v.id.clone()), Expr::Value(start_value(vars, v))]),
        None => Expr::Value(VariableValue::Boolean(true)),
    }
}

/// `p` with incoming names stored as ids and every `expression` text read: on a set variable
/// action it becomes `expr` (checked against the variable), elsewhere (conditional blocks) the
/// `condition` (checked to give true or false). Parsed text already holds ids, so it is never
/// canonicalized again. Existing actions that a command leaves alone never pass through here.
pub fn with_expressions(cmd: &str, vars: &Variables, p: &Value) -> Result<Value> {
    let mut v = p.clone();
    canonical_inputs(cmd, vars, &mut v, 0)?;
    resolve_expressions(cmd, vars, &mut v, 0)?;
    Ok(v)
}

/// Visit input actions and conditional blocks, never literal values or parsed expressions.
fn canonical_inputs(cmd: &str, vars: &Variables, v: &mut Value, depth: usize) -> Result<()> {
    if depth > 4 * MAX_ACTION_DEPTH {
        return Err(bad(cmd, "actions are nested too deeply"));
    }
    match v {
        Value::Array(a) => a.iter_mut().try_for_each(|x| canonical_inputs(cmd, vars, x, depth + 1)),
        Value::Object(o) => {
            canonical_action_names(vars, o);
            canonical_json_exprs(cmd, vars, o)?;
            for key in ["action", "actions", "interaction", "blocks"] {
                if let Some(child) = o.get_mut(key) {
                    canonical_inputs(cmd, vars, child, depth + 1)?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Only command input accepts names: settle action targets before reading text expressions.
fn canonical_action_names(vars: &Variables, o: &mut serde_json::Map<String, Value>) {
    match o.get("type").and_then(Value::as_str) {
        Some("setVariable") => {
            if let Some(v) = o.get("variable").and_then(Value::as_str).and_then(|n| vars.get(n)) {
                o.insert("variable".into(), json!(v.id));
            }
        }
        Some("setVariableMode") => {
            if let Some(c) = o.get("collection").and_then(Value::as_str).and_then(|n| vars.collection(n)) {
                if let Some(m) = o.get("mode").and_then(Value::as_str).and_then(|n| c.find_mode(n)) {
                    o.insert("mode".into(), json!(m.id));
                }
                o.insert("collection".into(), json!(c.id));
            }
        }
        _ => {}
    }
}

/// JSON expressions accept names too; text replaces them and is parsed separately.
fn canonical_json_exprs(cmd: &str, vars: &Variables, o: &mut serde_json::Map<String, Value>) -> Result<()> {
    if o.get("expression").is_some_and(Value::is_string) {
        return Ok(());
    }
    for key in ["expr", "condition"] {
        if let Some(value) = o.get_mut(key).filter(|v| !v.is_null()) {
            let mut e: Expr = serde_json::from_value(value.clone()).map_err(|e| bad(cmd, format!("{key}: {e}")))?;
            super::proto::canonical_expr(vars, &mut e, 0);
            *value = expr_json(cmd, &e)?;
        }
    }
    Ok(())
}

fn resolve_expressions(cmd: &str, vars: &Variables, v: &mut Value, depth: usize) -> Result<()> {
    if depth > 4 * MAX_ACTION_DEPTH {
        return Err(bad(cmd, "actions are nested too deeply"));
    }
    match v {
        Value::Array(a) => a.iter_mut().try_for_each(|x| resolve_expressions(cmd, vars, x, depth + 1)),
        Value::Object(o) => {
            if let Some(Value::String(text)) = o.get("expression") {
                let e = Expr::parse(text, vars).map_err(|e| bad(cmd, format!("`{text}`: {e}")))?;
                if o.get("type").and_then(Value::as_str) == Some("setVariable") {
                    let var = o
                        .get("variable")
                        .and_then(Value::as_str)
                        .and_then(|n| vars.get_by_id(n))
                        .ok_or_else(|| bad(cmd, "setVariable: no such `variable`"))?;
                    e.check_value(vars, var.kind).map_err(|m| bad(cmd, m))?;
                    o.insert("expr".into(), expr_json(cmd, &e)?);
                    o.entry("value").or_insert(Value::Null);
                } else {
                    e.check_condition(vars).map_err(|m| bad(cmd, m))?;
                    o.insert("condition".into(), expr_json(cmd, &e)?);
                }
                o.remove("expression");
            }
            o.values_mut().try_for_each(|x| resolve_expressions(cmd, vars, x, depth + 1))
        }
        _ => Ok(()),
    }
}

fn expr_json(cmd: &str, e: &Expr) -> Result<Value> {
    serde_json::to_value(e).map_err(|err| bad(cmd, err.to_string()))
}

/// Expressions in a new action must work: set variable names a variable and its expression gives
/// a value it can hold; conditions give true or false.
fn check_action(cmd: &str, vars: &Variables, a: &Action) -> Result<()> {
    for x in walk(std::slice::from_ref(a)) {
        match x {
            Action::SetVariable { variable, expr, .. } => {
                let v = vars.get_by_id(variable).ok_or_else(|| bad(cmd, format!("no variable `{variable}`")))?;
                if let Some(e) = expr {
                    e.check_value(vars, v.kind).map_err(|m| bad(cmd, m))?;
                }
            }
            Action::Conditional { blocks } => {
                if blocks.len() > MAX_BLOCKS {
                    return Err(bad(cmd, format!("at most {MAX_BLOCKS} blocks")));
                }
                for c in blocks.iter().filter_map(|b| b.condition.as_ref()) {
                    c.check_condition(vars).map_err(|m| bad(cmd, m))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The `action` param: expressions read, names stored as ids, checked.
fn action_param(cmd: &str, s: &Session, p: &Value) -> Result<Action> {
    let doc = s.doc()?;
    let p = with_expressions(cmd, &doc.variables, p)?;
    let a: Action = parse(cmd, &p, "action")?.ok_or_else(|| bad(cmd, "`action` required"))?;
    check_action(cmd, &doc.variables, &a)?;
    super::proto::check_action_dests(cmd, doc, std::slice::from_ref(&a))?;
    Ok(a)
}

/// The condition in `expression` (text) or `condition` (expression JSON), checked.
fn condition_param(cmd: &str, s: &Session, p: &Value) -> Result<Option<Expr>> {
    let vars = &s.doc()?.variables;
    let e = match (string(p, "expression"), parse::<Expr>(cmd, p, "condition")?) {
        (Some(text), _) => Expr::parse(&text, vars).map_err(|e| bad(cmd, format!("`{text}`: {e}")))?,
        (None, Some(mut e)) => {
            super::proto::canonical_expr(vars, &mut e, 0);
            e
        }
        (None, None) => return Ok(None),
    };
    e.check_condition(vars).map_err(|m| bad(cmd, m))?;
    Ok(Some(e))
}

/// A `path` param: whole numbers, an odd count (it ends at an action), not deeper than actions nest.
fn path_param(cmd: &str, p: &Value, key: &str) -> Result<Option<Vec<usize>>> {
    let Some(v) = get(p, key) else { return Ok(None) };
    let list = v.as_array().ok_or_else(|| bad(cmd, format!("`{key}` must be a list of indexes")))?;
    if list.len() % 2 == 0 || list.len() > 2 * MAX_ACTION_DEPTH + 1 {
        return Err(bad(cmd, format!("`{key}` must be [action] or [action, block, action, …]")));
    }
    list.iter()
        .map(|x| f64_of(x).filter(|f| (0.0..1e6).contains(f) && f.fract() == 0.0).map(|f| f as usize))
        .collect::<Option<Vec<usize>>>()
        .map(Some)
        .ok_or_else(|| bad(cmd, format!("`{key}` holds whole numbers ≥ 0")))
}

fn req_path(cmd: &str, p: &Value) -> Result<Vec<usize>> {
    path_param(cmd, p, "path")?.ok_or_else(|| bad(cmd, "`path` required"))
}

/// Apply `f` to the actions of interaction `index` of each target (all or nothing: an error rolls
/// the command back). Returns `f`'s result for the last target.
fn edit(cmd: &str, s: &mut Session, p: &Value, f: impl Fn(&mut Vec<Action>) -> Result<Value>) -> Result<Value> {
    let k = req_index(cmd, p, "index")?;
    let mut out = None;
    for i in req_targets(cmd, s, p)? {
        let Some(mut actions) = s.doc()?.get(i)?.interactions.get(k).map(|it| it.actions.clone()) else { continue };
        let r = f(&mut actions)?;
        if actions.is_empty() {
            return Err(bad(cmd, "an interaction needs an action: remove the interaction instead"));
        }
        if let Some(it) = s.doc_mut()?.get_mut(i)?.interactions.get_mut(k) {
            it.actions = actions;
        }
        out = Some(r);
    }
    out.ok_or_else(|| bad(cmd, format!("no interaction {k}")))
}

fn no_list(cmd: &str) -> crate::EngineError {
    bad(cmd, "no action there: `path` must lead through conditional actions and their blocks")
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.action.add";
    let path = path_param(cmd, p, "path")?;
    let action = action_param(cmd, s, p)?;
    edit(cmd, s, p, |actions| {
        let path = path.clone().unwrap_or_else(|| vec![actions.len()]);
        let (list, at) = list_mut(actions, &path).ok_or_else(|| no_list(cmd))?;
        if at > list.len() {
            return Err(bad(cmd, format!("index {at} is past the end ({} actions)", list.len())));
        }
        if list.len() >= MAX_ACTIONS {
            return Err(bad(cmd, format!("at most {MAX_ACTIONS} actions in a list")));
        }
        list.insert(at, action.clone());
        Ok(json!({"path": path}))
    })
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.action.set";
    let path = req_path(cmd, p)?;
    let action = action_param(cmd, s, p)?;
    edit(cmd, s, p, |actions| {
        let (list, at) = list_mut(actions, &path).ok_or_else(|| no_list(cmd))?;
        *list.get_mut(at).ok_or_else(|| no_list(cmd))? = action.clone();
        Ok(json!({"path": path}))
    })
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.action.remove";
    let path = req_path(cmd, p)?;
    edit(cmd, s, p, |actions| {
        let (list, at) = list_mut(actions, &path).ok_or_else(|| no_list(cmd))?;
        if at >= list.len() {
            return Err(no_list(cmd));
        }
        list.remove(at);
        Ok(json!({"ok": true}))
    })
}

fn move_action(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.action.move";
    let path = req_path(cmd, p)?;
    let to = req_index(cmd, p, "to")?;
    edit(cmd, s, p, |actions| {
        let (list, at) = list_mut(actions, &path).ok_or_else(|| no_list(cmd))?;
        if at >= list.len() || to >= list.len() {
            return Err(bad(cmd, format!("the list has {} actions", list.len())));
        }
        let a = list.remove(at);
        list.insert(to, a);
        let mut moved = path.clone();
        if let Some(last) = moved.last_mut() {
            *last = to;
        }
        Ok(json!({"path": moved}))
    })
}

/// The blocks of the conditional action at `path`.
fn blocks_mut<'a>(cmd: &str, actions: &'a mut Vec<Action>, path: &[usize]) -> Result<&'a mut Vec<ConditionalBlock>> {
    let (list, at) = list_mut(actions, path).ok_or_else(|| no_list(cmd))?;
    match list.get_mut(at) {
        Some(Action::Conditional { blocks }) => Ok(blocks),
        Some(_) => Err(bad(cmd, "that action isn't a conditional")),
        None => Err(no_list(cmd)),
    }
}

fn add_block(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.conditional.addBlock";
    let path = req_path(cmd, p)?;
    let cond = condition_param(cmd, s, p)?;
    let is_else = match get(p, "else") {
        None => false,
        Some(v) => bool_of(v).ok_or_else(|| bad(cmd, "`else` must be true or false"))?,
    };
    if is_else && cond.is_some() {
        return Err(bad(cmd, "an else block has no condition"));
    }
    let fallback = default_condition(&s.doc()?.variables);
    edit(cmd, s, p, |actions| {
        let blocks = blocks_mut(cmd, actions, &path)?;
        if blocks.len() >= MAX_BLOCKS {
            return Err(bad(cmd, format!("at most {MAX_BLOCKS} blocks")));
        }
        let has_else = blocks.last().is_some_and(|b| b.condition.is_none());
        let at = if is_else {
            if has_else {
                return Err(bad(cmd, "it already has an else"));
            }
            blocks.len()
        } else if has_else {
            blocks.len() - 1
        } else {
            blocks.len()
        };
        let condition = if is_else { None } else { Some(cond.clone().unwrap_or_else(|| fallback.clone())) };
        blocks.insert(at, ConditionalBlock { condition, actions: vec![] });
        Ok(json!({"block": at}))
    })
}

fn set_condition(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.conditional.setCondition";
    let path = req_path(cmd, p)?;
    let block = req_index(cmd, p, "block")?;
    let cond = condition_param(cmd, s, p)?.ok_or_else(|| bad(cmd, "`expression` or `condition` required"))?;
    edit(cmd, s, p, |actions| {
        let blocks = blocks_mut(cmd, actions, &path)?;
        let b = blocks.get_mut(block).ok_or_else(|| bad(cmd, format!("no block {block}")))?;
        b.condition = Some(cond.clone());
        Ok(json!({"ok": true}))
    })
}

fn remove_block(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "proto.conditional.removeBlock";
    let path = req_path(cmd, p)?;
    let block = req_index(cmd, p, "block")?;
    edit(cmd, s, p, |actions| {
        let blocks = blocks_mut(cmd, actions, &path)?;
        if block >= blocks.len() {
            return Err(bad(cmd, format!("no block {block}")));
        }
        blocks.remove(block);
        // Without an "if" first there's nothing to decide: the conditional goes.
        let gone = blocks.first().is_none_or(|b| b.condition.is_none());
        if gone && let Some((list, at)) = list_mut(actions, &path) {
            list.remove(at);
        }
        Ok(json!({"removedAction": gone}))
    })
}
