//! Step references in `command.batch`: a step's params can use what an earlier step returned.
//!
//! `{"$step": n}` stands for the `id` in the result of step `n` (zero-based), and `{"$ref": "name"}`
//! for the `id` in the result of the step that called itself `name` with `"as": "name"`. Either
//! takes a `field`, a key of that result or a slash path into it (`ids/0`, `map/3`, `bounds/x`), the
//! way `variable.bind` names `fills/0/color`. The CLI's script runner resolves its steps the same way.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::{Result, bad};

/// Deepest params nesting the resolver walks before giving up.
const MAX_DEPTH: usize = 64;

/// One step of a batch: a command, its params and the name later steps can call it by.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchStep {
    pub command: String,
    pub params: Value,
    /// The step's `as`, for `{"$ref": name}` in later steps.
    pub name: Option<String>,
}

impl BatchStep {
    /// A step without a name.
    pub fn new(command: impl Into<String>, params: Value) -> Self {
        Self { command: command.into(), params, name: None }
    }
}

/// A step's `as` (`step` is its index, for the error): `None` when it has none.
pub fn step_name(step: usize, json: &Value) -> Result<Option<String>> {
    match json.get("as") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.clone())),
        Some(_) => Err(bad("command.batch", format!("step {step}: `as` must be a name (a non-empty string)"))),
    }
}

/// Refuse a step key other than `command`, `params` and `as` (`step` is its index, for the
/// error): `args` in place of `params` would run the command without them.
pub fn refuse_unknown_step_keys(step: usize, json: &Value) -> Result<()> {
    let Some(o) = json.as_object() else { return Ok(()) };
    let unknown: Vec<String> = o.keys().filter(|k| !matches!(k.as_str(), "command" | "params" | "as")).map(|k| format!("`{k}`")).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let s = if unknown.len() == 1 { "" } else { "s" };
    Err(bad("command.batch", format!("step {step}: unknown key{s} {} (a step takes command, params and as)", unknown.join(", "))))
}

/// The steps of one batch run so far: their results, and which step each `as` name belongs to.
/// A nested `command.batch` step gets its own scope, so its references are to its own steps.
#[derive(Clone, Debug, Default)]
pub struct BatchScope {
    names: HashMap<String, usize>,
    results: Vec<Value>,
    failed: HashSet<usize>,
}

impl BatchScope {
    /// The scope for running `steps`. Two steps with the same name fail here, before any step runs.
    pub fn new(steps: &[BatchStep]) -> Result<Self> {
        let mut names = HashMap::new();
        for (i, step) in steps.iter().enumerate() {
            let Some(name) = &step.name else { continue };
            if let Some(first) = names.insert(name.clone(), i) {
                return Err(bad("command.batch", format!("step {i}: `as` \"{name}\" is already step {first}'s name")));
            }
        }
        Ok(Self { names, results: Vec::with_capacity(steps.len()), failed: HashSet::new() })
    }

    /// The results of the steps run so far, in order (failed slots hold null).
    pub fn results(&self) -> &[Value] {
        &self.results
    }

    pub fn into_results(self) -> Vec<Value> {
        self.results
    }

    /// The index of the step that runs next.
    pub fn next_step(&self) -> usize {
        self.results.len()
    }

    pub(crate) fn push(&mut self, result: Value) {
        self.results.push(result);
    }

    pub(crate) fn push_failed(&mut self) {
        // Keep positions even after a failure; a successful null result is not a failed step.
        self.failed.insert(self.next_step());
        self.results.push(Value::Null);
    }

    /// `params` of the next step with every reference in it replaced.
    pub(crate) fn resolve(&self, params: &Value) -> Result<Value> {
        self.resolve_in(params, 0)
    }

    fn resolve_in(&self, v: &Value, depth: usize) -> Result<Value> {
        let step = self.next_step();
        if depth > MAX_DEPTH {
            return Err(bad("command.batch", format!("step {step}: params nest deeper than {MAX_DEPTH} levels")));
        }
        match v {
            Value::Object(o) if o.contains_key("$step") || o.contains_key("$ref") => self.reference(o),
            Value::Object(o) => {
                let mut out = Map::with_capacity(o.len());
                for (k, x) in o {
                    out.insert(k.clone(), self.resolve_in(x, depth + 1)?);
                }
                Ok(Value::Object(out))
            }
            Value::Array(a) => a.iter().map(|x| self.resolve_in(x, depth + 1)).collect::<Result<Vec<_>>>().map(Value::Array),
            _ => Ok(v.clone()),
        }
    }

    /// The value a `{"$step": n}` or `{"$ref": "name"}` object (with an optional `field`) stands for.
    fn reference(&self, o: &Map<String, Value>) -> Result<Value> {
        let step = self.next_step();
        let fail = |msg: String| bad("command.batch", format!("step {step}: {msg}"));
        let key = if o.contains_key("$step") { "$step" } else { "$ref" };
        if let Some(k) = o.keys().find(|k| *k != key && *k != "field") {
            return Err(fail(format!("a `{key}` reference takes only `field`, not `{k}`")));
        }
        let (n, who) = if key == "$step" {
            let n = o.get("$step").and_then(Value::as_u64).ok_or_else(|| fail("`$step` must be a step index (0, 1, …)".into()))?;
            let n = usize::try_from(n).ok().filter(|n| *n < step).ok_or_else(|| {
                let range = if step == 0 { "there is none before step 0".to_string() } else { format!("0 to {}", step - 1) };
                fail(format!("`$step` {n} must be an earlier step ({range})"))
            })?;
            (n, format!("step {n}"))
        } else {
            let name = o.get("$ref").and_then(Value::as_str).ok_or_else(|| fail("`$ref` must be a step name (a string)".into()))?;
            let n = *self.names.get(name).ok_or_else(|| fail(format!("`$ref` \"{name}\" names no step ({})", self.known_names())))?;
            if n >= step {
                return Err(fail(format!("`$ref` \"{name}\" is step {n}, not an earlier step")));
            }
            (n, format!("step {n} (\"{name}\")"))
        };
        if self.failed.contains(&n) {
            return Err(fail(format!("references {who}, which failed")));
        }
        let path = match o.get("field") {
            None => "id",
            Some(Value::String(f)) => f.as_str(),
            Some(_) => return Err(fail("`field` must be a string".into())),
        };
        let result = self.results.get(n).ok_or_else(|| fail(format!("{who} has no result")))?;
        lookup(result, path).cloned().map_err(|why| fail(format!("{who}'s result has no `{path}`{why}")))
    }

    fn known_names(&self) -> String {
        let mut names: Vec<_> = self.names.iter().collect();
        names.sort_by_key(|(_, i)| **i);
        if names.is_empty() {
            return "no step has an `as` name".into();
        }
        let list: Vec<_> = names.iter().map(|(name, i)| format!("\"{name}\" is step {i}")).collect();
        list.join(", ")
    }
}

/// `path` in `v`: a key as written, else a slash path (`ids/0`, `bounds/x`; a leading `/` as in
/// JSON Pointer is allowed). The error is the detail after "has no `path`".
fn lookup<'a>(v: &'a Value, path: &str) -> std::result::Result<&'a Value, String> {
    // A key as written first, so a key holding `/` still works as it did before paths.
    if let Some(x) = v.get(path) {
        return Ok(x);
    }
    let hint = if path.contains('.') && !path.contains('/') { " (paths use `/`, as in `ids/0`)" } else { "" };
    walk(v, path.strip_prefix('/').unwrap_or(path)).map_err(|why| format!("{why}{hint}"))
}

fn walk<'a>(v: &'a Value, path: &str) -> std::result::Result<&'a Value, String> {
    let mut cur = v;
    let mut walked = String::new();
    for seg in path.split('/') {
        let here = if walked.is_empty() { "the result".to_string() } else { format!("`{walked}`") };
        cur = match cur {
            Value::Object(o) => o.get(seg).ok_or_else(|| if walked.is_empty() { String::new() } else { format!(": {here} has no `{seg}`") })?,
            Value::Array(a) => match seg.parse::<usize>() {
                Ok(i) => a.get(i).ok_or_else(|| {
                    let items = if a.len() == 1 { "1 item".to_string() } else { format!("{} items", a.len()) };
                    format!(": {here} has {items}")
                })?,
                Err(_) => return Err(format!(": {here} is a list, so `{seg}` must be an index (0, 1, …)")),
            },
            _ => return Err(format!(": {here} is {}", kind(cur))),
        };
        if !walked.is_empty() {
            walked.push('/');
        }
        walked.push_str(seg);
    }
    Ok(cur)
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}
