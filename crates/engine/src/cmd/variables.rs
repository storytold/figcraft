//! Editing variables, modes and collections (the Variables editor): rename, describe, scope,
//! duplicate and delete; extended collections (overrides, keeping them in step with what they
//! extend); alias checks; and which scope a bindable property belongs to.

use std::collections::BTreeSet;

use figcraft_doc::{
    CODE_PLATFORMS, Collection, CollectionId, ColorOperand, Document, ModeId, NodeId, NodeKind, Variable, VariableId, VariableType, VariableValue,
    Variables, normalize_scopes, scope_known,
};
use serde_json::{Value, json};

use super::lists::BoundList;
use super::{CommandSpec, ok};
use crate::params::{boolean, get, num, req_string, string};
use crate::{Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "variable.update",
            "Edit variable",
            "{variable, name?, description?, scopes? ([scope ids]; null or [\"ALL_SCOPES\"] = every supported property, [] = hidden from pickers), hiddenFromPublishing? (left out when the file is published as a library), codeSyntax? {WEB?, iOS?, ANDROID?: name code uses (\"\" clears)}}",
            true,
            update
        ),
        spec!("variable.duplicate", "Duplicate variable", "{variable} → {id}", true, duplicate),
        spec!("variable.renameMode", "Rename mode", "{collection, mode, name}", true, rename_mode),
        spec!("variable.removeMode", "Delete mode", "{collection, mode} (a collection keeps at least one mode)", true, remove_mode),
        spec!("variable.moveMode", "Move mode", "{collection, mode, to (index; 0 = the default mode, layers on Auto follow it)}", true, move_mode),
        spec!("variable.renameCollection", "Rename collection", "{collection, name}", true, rename_collection),
        spec!(
            "variable.deleteCollection",
            "Delete collection",
            "{collection} (with its variables; bound layers keep their values; variables elsewhere aliasing them take their values and colour calculations reading them keep the colour they show)",
            true,
            delete_collection
        ),
        spec!(
            "variable.resetOverride",
            "Reset variable override",
            "{variable (a variable of an extended collection inherited from the collection it extends), mode? (id or name; every overridden mode when omitted)}  the value follows the inherited one again",
            true,
            reset_override
        ),
    ]
}

/// Props resolve an exact id first, then an unambiguous exact name or `Collection/name`. Compare
/// whole names: both collection and variable names can contain `/`. Other commands keep their
/// existing first-match lookup.
pub(crate) fn resolve_strict(cmd: &str, doc: &Document, name: &str) -> Result<VariableId> {
    let vars = &doc.variables;
    if let Some(v) = vars.variables.iter().find(|v| v.id == name) {
        return Ok(v.id.clone());
    }
    let collections: std::collections::BTreeMap<_, _> = vars.collections.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();
    let mut candidates = std::collections::BTreeMap::new();
    for v in &vars.variables {
        let collection = collections.get(v.collection.as_str()).copied().unwrap_or(v.collection.as_str());
        let qualified = format!("{collection}/{}", v.name);
        if v.name == name || qualified == name {
            candidates.insert(v.id.as_str(), qualified);
        }
    }
    match candidates.keys().next().copied() {
        None => Err(bad(cmd, format!("no such variable `{name}`"))),
        Some(id) if candidates.len() == 1 => Ok(id.to_string()),
        Some(_) => {
            let mut names: Vec<_> = candidates.into_values().collect();
            names.sort();
            Err(bad(cmd, format!("ambiguous variable `{name}`: {} (use a variable id to choose)", names.join(", "))))
        }
    }
}

/// The first of `base`, `base 2`, `base 3`… that no collection is named.
pub(crate) fn free_collection_name(doc: &Document, base: &str) -> String {
    (1..1000)
        .map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") })
        .find(|n| !doc.variables.collections.iter().any(|c| c.name == *n))
        .unwrap_or_else(|| base.to_string())
}

/// The error for editing what an extended collection inherits.
fn inherited(cmd: &str, what: &str) -> crate::EngineError {
    bad(cmd, format!("{what} comes from the collection it extends (edit it there)"))
}

/// Refuse mode edits of extended collections (they have the modes of the collection they extend).
pub(crate) fn check_modes_editable(doc: &Document, cmd: &str, collection: &str) -> Result<()> {
    match doc.variables.collection_by_id(collection) {
        Some(c) if c.extends.is_some() => Err(inherited(cmd, "an extended collection's modes")),
        _ => Ok(()),
    }
}

/// A variable name with each `/`-separated group part trimmed; None when nothing is left.
pub(crate) fn clean_name(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
    let out = parts.join("/");
    (!out.is_empty()).then(|| out.chars().take(500).collect())
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.update";
    let v = req_string(cmd, p, "variable")?;
    let doc = s.doc_mut()?;
    let var = doc.variables.get(&v).cloned().ok_or_else(|| bad(cmd, "no such variable"))?;
    let name = match string(p, "name") {
        Some(_) if var.extends.is_some() => return Err(inherited(cmd, "an inherited variable's name")),
        Some(n) => {
            let n = clean_name(&n).ok_or_else(|| bad(cmd, "name can't be empty"))?;
            if doc.variables.variables.iter().any(|x| x.id != var.id && x.collection == var.collection && x.name == n) {
                return Err(bad(cmd, format!("a variable named `{n}` already exists in this collection")));
            }
            Some(n)
        }
        None => None,
    };
    let scopes = match p.get("scopes") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Array(a)) => {
            let mut ids = Vec::with_capacity(a.len());
            for x in a {
                let id = x.as_str().ok_or_else(|| bad(cmd, "scopes: [scope ids]"))?;
                if !scope_known(var.kind, id) {
                    return Err(bad(cmd, format!("`{id}` isn't a scope for {} variables", var.kind.id())));
                }
                ids.push(id);
            }
            // Every scope listed is the same as no limit.
            Some(normalize_scopes(var.kind, ids))
        }
        Some(_) => return Err(bad(cmd, "scopes: [scope ids] or null")),
    };
    let mut code = var.code_syntax.clone();
    match get(p, "codeSyntax") {
        None => {}
        Some(Value::Object(m)) => {
            for (k, x) in m {
                if !CODE_PLATFORMS.contains(&k.as_str()) {
                    return Err(bad(cmd, format!("codeSyntax: `{k}` isn't a platform (WEB, iOS, ANDROID)")));
                }
                match x {
                    Value::String(n) if !n.trim().is_empty() => {
                        code.insert(k.clone(), n.trim().chars().take(500).collect());
                    }
                    Value::String(_) | Value::Null => {
                        code.remove(k);
                    }
                    _ => return Err(bad(cmd, "codeSyntax: names are strings")),
                }
            }
        }
        Some(_) => return Err(bad(cmd, "codeSyntax: {WEB?, iOS?, ANDROID?}")),
    }
    let slot = doc.variables.variables.iter_mut().find(|x| x.id == var.id).ok_or_else(|| bad(cmd, "no such variable"))?;
    slot.code_syntax = code;
    if let Some(n) = name {
        slot.name = n;
    }
    if let Some(d) = string(p, "description") {
        slot.description = d.chars().take(10_000).collect();
    }
    if let Some(sc) = scopes {
        slot.scopes = sc;
    }
    if let Some(h) = boolean(p, "hiddenFromPublishing") {
        slot.hidden_from_publishing = h;
    }
    ok()
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.duplicate";
    let v = req_string(cmd, p, "variable")?;
    let doc = s.doc_mut()?;
    let pos = doc.variables.variables.iter().position(|x| x.id == v || x.name == v).ok_or_else(|| bad(cmd, "no such variable"))?;
    let mut copy = doc.variables.variables.get(pos).cloned().ok_or_else(|| bad(cmd, "no such variable"))?;
    let taken = |n: &str| doc.variables.variables.iter().any(|x| x.collection == copy.collection && x.name == n);
    let name = (1..1000)
        .map(|k| if k == 1 { format!("{} copy", copy.name) } else { format!("{} copy {k}", copy.name) })
        .find(|n| !taken(n))
        .ok_or_else(|| bad(cmd, "too many copies"))?;
    copy.id = doc.new_variable_id();
    copy.name = name;
    // A copy of an inherited variable is the extended collection's own.
    copy.extends = None;
    copy.overrides.clear();
    let id = copy.id.clone();
    doc.variables.variables.insert(pos + 1, copy);
    Ok(json!({"id": id}))
}

/// The index of collection `c` (id or name).
fn collection_index(doc: &Document, cmd: &str, c: &str) -> Result<usize> {
    doc.variables.collections.iter().position(|x| x.id == c || x.name == c).ok_or_else(|| bad(cmd, "no such collection"))
}

/// Collection `c`'s index and the id of its mode `m` (id or name).
fn mode_of(doc: &Document, cmd: &str, p: &Value) -> Result<(usize, ModeId)> {
    let ci = collection_index(doc, cmd, &req_string(cmd, p, "collection")?)?;
    let m = req_string(cmd, p, "mode")?;
    let mode = doc
        .variables
        .collections
        .get(ci)
        .and_then(|c| c.modes.iter().find(|x| x.id == m || x.name == m))
        .map(|x| x.id.clone())
        .ok_or_else(|| bad(cmd, "no such mode"))?;
    check_modes_editable(doc, cmd, doc.variables.collections.get(ci).map_or("", |c| &c.id))?;
    Ok((ci, mode))
}

fn rename_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.renameMode";
    let name = req_string(cmd, p, "name")?.trim().chars().take(200).collect::<String>();
    if name.is_empty() {
        return Err(bad(cmd, "name can't be empty"));
    }
    let doc = s.doc_mut()?;
    let (ci, mode) = mode_of(doc, cmd, p)?;
    if let Some(m) = doc.variables.collections.get_mut(ci).and_then(|c| c.modes.iter_mut().find(|x| x.id == mode)) {
        m.name = name;
    }
    ok()
}

fn remove_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.removeMode";
    let doc = s.doc_mut()?;
    let (ci, mode) = mode_of(doc, cmd, p)?;
    let coll = doc.variables.collections.get_mut(ci).ok_or_else(|| bad(cmd, "no such collection"))?;
    if coll.modes.len() < 2 {
        return Err(bad(cmd, "a collection keeps at least one mode"));
    }
    coll.modes.retain(|m| m.id != mode);
    let cid = coll.id.clone();
    for v in doc.variables.variables.iter_mut().filter(|v| v.collection == cid) {
        v.values.remove(&mode);
    }
    // Layers set to the deleted mode go back to the default.
    let users: Vec<NodeId> = doc.nodes().filter(|n| n.var_modes.get(&cid) == Some(&mode)).map(|n| n.id).collect();
    for id in users {
        doc.get_mut(id)?.var_modes.remove(&cid);
    }
    ok()
}

fn move_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.moveMode";
    let to = num(p, "to").filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, "`to` must be a number"))?;
    let doc = s.doc_mut()?;
    let (ci, mode) = mode_of(doc, cmd, p)?;
    let coll = doc.variables.collections.get_mut(ci).ok_or_else(|| bad(cmd, "no such collection"))?;
    let from = coll.modes.iter().position(|m| m.id == mode).ok_or_else(|| bad(cmd, "no such mode"))?;
    let m = coll.modes.remove(from);
    let at = (to.max(0.0) as usize).min(coll.modes.len());
    coll.modes.insert(at, m);
    ok()
}

fn rename_collection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.renameCollection";
    let name = req_string(cmd, p, "name")?.trim().chars().take(200).collect::<String>();
    if name.is_empty() {
        return Err(bad(cmd, "name can't be empty"));
    }
    let doc = s.doc_mut()?;
    let ci = collection_index(doc, cmd, &req_string(cmd, p, "collection")?)?;
    if doc.variables.collections.iter().enumerate().any(|(i, c)| i != ci && c.name == name) {
        return Err(bad(cmd, format!("a collection named `{name}` already exists")));
    }
    if let Some(c) = doc.variables.collections.get_mut(ci) {
        c.name = name;
    }
    ok()
}

fn delete_collection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.deleteCollection";
    let doc = s.doc_mut()?;
    let ci = collection_index(doc, cmd, &req_string(cmd, p, "collection")?)?;
    let cid = doc.variables.collections.get(ci).map(|c| c.id.clone()).ok_or_else(|| bad(cmd, "no such collection"))?;
    let gone: BTreeSet<VariableId> = doc.variables.variables.iter().filter(|v| v.collection == cid).map(|v| v.id.clone()).collect();
    forget_variables(doc, &gone)?;
    doc.variables.collections.retain(|c| c.id != cid);
    // Layers no longer follow the deleted modes either.
    let users: Vec<NodeId> = doc.nodes().filter(|n| n.var_modes.contains_key(&cid)).map(|n| n.id).collect();
    for id in users {
        doc.get_mut(id)?.var_modes.remove(&cid);
    }
    Ok(json!({"deletedVariables": gone.len()}))
}

/// Delete variables `gone`: layers bound to them keep the values they show but no longer follow
/// them; a variable aliasing one takes its value and a colour calculation reading one keeps the
/// colour it shows (per mode: what the reader resolves to in that mode, other collections in their
/// default modes).
pub(crate) fn forget_variables(doc: &mut Document, gone: &BTreeSet<VariableId>) -> Result<()> {
    // Every reader is resolved before anything is written, so a reader of a reader resolves
    // through the values as they were.
    let vars = &doc.variables;
    let mut detached: Vec<(VariableId, ModeId, VariableValue)> = Vec::new();
    for v in vars.variables.iter().filter(|v| !gone.contains(&v.id)) {
        for (m, val) in &v.values {
            if !val.references().iter().any(|r| gone.contains(*r)) {
                continue;
            }
            let modes = std::collections::BTreeMap::from([(v.collection.clone(), m.clone())]);
            if let Some(r) = vars.resolve(&v.id, &modes) {
                detached.push((v.id.clone(), m.clone(), r));
            }
        }
    }
    for (vid, m, r) in detached {
        if let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == vid) {
            slot.values.insert(m, r);
        }
    }
    doc.variables.variables.retain(|v| !gone.contains(&v.id));
    let users: Vec<NodeId> = doc.nodes().filter(|n| n.bound.values().any(|v| gone.contains(v))).map(|n| n.id).collect();
    for id in users {
        let n = doc.get_mut(id)?;
        for (field, _) in n.bound.iter().filter(|(_, v)| gone.contains(*v)) {
            super::styles::rebind_paint_alpha(&mut n.fills, &mut n.strokes, field, false);
        }
        n.bound.retain(|_, v| !gone.contains(v));
    }
    Ok(())
}

fn reset_override(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.resetOverride";
    let v = req_string(cmd, p, "variable")?;
    let doc = s.doc_mut()?;
    let var = doc.variables.get(&v).cloned().ok_or_else(|| bad(cmd, "no such variable"))?;
    let parent = var.extends.as_deref().and_then(|x| doc.variables.get_by_id(x)).cloned();
    let parent = parent.ok_or_else(|| bad(cmd, "only variables an extended collection inherits have overrides"))?;
    let modes: Vec<ModeId> = match string(p, "mode") {
        Some(m) => {
            let coll = doc.variables.collection_by_id(&var.collection).ok_or_else(|| bad(cmd, "variable has no collection"))?;
            vec![coll.find_mode(&m).ok_or_else(|| bad(cmd, "no such mode"))?.id.clone()]
        }
        None => var.overrides.iter().cloned().collect(),
    };
    if let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == var.id) {
        for m in modes {
            slot.overrides.remove(&m);
            if let Some(val) = parent.values.get(&m) {
                slot.values.insert(m, val.clone());
            }
        }
    }
    ok()
}

/// Keep extended collections in step with the collections they extend (after extending one or
/// updating library copies): the same modes, one inheriting variable per variable of the parent
/// (named like it, with its values in every mode not overridden) and values in every mode for
/// their own variables. When a parent collection or variable is gone, what extended it becomes a
/// plain collection or variable, keeping its values.
pub fn sync_extensions(doc: &mut Document) {
    let exts: Vec<(CollectionId, Option<Collection>)> = doc
        .variables
        .collections
        .iter()
        .filter_map(|c| Some((c.id.clone(), c.extends.as_ref().map(|p| doc.variables.collections.iter().find(|x| x.id == *p).cloned())?)))
        .collect();
    for (cid, parent) in exts {
        let vars = &mut doc.variables.variables;
        // shortcut: an extension of a local extended collection isn't followed: extending takes
        // library collections, and a library's extended collection arrives as a plain copy, so
        // chains go through libraries (`extended_chain_tests`); follow parents here if extending
        // local collections is ever offered.
        let Some(parent) = parent.filter(|p| p.extends.is_none()) else {
            for v in vars.iter_mut().filter(|v| v.collection == cid) {
                v.extends = None;
                v.overrides.clear();
            }
            if let Some(c) = doc.variables.collections.iter_mut().find(|c| c.id == cid) {
                c.extends = None;
            }
            continue;
        };
        let parents: Vec<Variable> = vars.iter().filter(|v| v.collection == parent.id).cloned().collect();
        let modes: Vec<ModeId> = parent.modes.iter().map(|m| m.id.clone()).collect();
        for v in vars.iter_mut().filter(|v| v.collection == cid) {
            match v.extends.as_ref().map(|e| parents.iter().find(|p| p.id == *e)) {
                Some(Some(p)) => {
                    if v.kind != p.kind {
                        v.kind = p.kind;
                        v.overrides.clear();
                    }
                    v.name.clone_from(&p.name);
                    v.overrides.retain(|m| modes.contains(m));
                    let own = std::mem::take(&mut v.values);
                    v.values =
                        p.values.iter().map(|(m, x)| (m.clone(), own.get(m).filter(|_| v.overrides.contains(m)).unwrap_or(x).clone())).collect();
                }
                Some(None) => {
                    v.extends = None;
                    v.overrides.clear();
                }
                None => {}
            }
            // Its own variables (and orphans) keep a value in every mode: new modes start with the
            // default mode's.
            if v.extends.is_none() {
                let first = modes.iter().find_map(|m| v.values.get(m)).or_else(|| v.values.values().next()).cloned();
                v.values.retain(|m, _| modes.contains(m));
                if let Some(f) = first {
                    for m in &modes {
                        v.values.entry(m.clone()).or_insert_with(|| f.clone());
                    }
                }
            }
        }
        let missing: Vec<Variable> =
            parents.into_iter().filter(|p| !vars.iter().any(|v| v.collection == cid && v.extends.as_ref() == Some(&p.id))).collect();
        if let Some(c) = doc.variables.collections.iter_mut().find(|c| c.id == cid)
            && c.modes != parent.modes
        {
            c.modes = parent.modes.clone();
        }
        for p in missing {
            let id = doc.new_variable_id();
            let extends = Some(p.id.clone());
            doc.variables.variables.push(Variable { id, collection: cid.clone(), extends, hidden_from_publishing: false, ..p });
        }
    }
}

/// Why a variable can't be read by an alias or a calculation.
enum TargetError {
    Missing,
    Kind(VariableType),
    Loop,
}

/// The id of `target` (the variable a value would read, None when there is none) as something a
/// value may read: it is of `kind` and doesn't lead back to `from` (the variable being set, once it
/// exists).
fn reference_target(
    vars: &Variables,
    target: Option<&Variable>,
    kind: VariableType,
    from: Option<&VariableId>,
) -> std::result::Result<VariableId, TargetError> {
    let t = target.ok_or(TargetError::Missing)?;
    if t.kind != kind {
        return Err(TargetError::Kind(t.kind));
    }
    if reaches_back(vars, &t.id, from) {
        return Err(TargetError::Loop);
    }
    Ok(t.id.clone())
}

/// Check that `var` may alias `target` (id or name): it exists, has the same type and doesn't
/// lead back to `var`. Returns the target's id.
pub fn alias_target(vars: &Variables, var: &Variable, target: &str) -> std::result::Result<VariableId, String> {
    let t = vars.get(target).ok_or_else(|| format!("no variable `{target}` to alias"))?;
    alias_target_by_id(vars, var, &t.id)
}

/// Check an alias whose target is already a stored or remapped id, without matching names.
pub(crate) fn alias_target_by_id(vars: &Variables, var: &Variable, target: &str) -> std::result::Result<VariableId, String> {
    reference_target(vars, vars.get_by_id(target), var.kind, Some(&var.id)).map_err(|e| match e {
        TargetError::Missing => format!("no variable `{target}` to alias"),
        TargetError::Kind(k) => format!("can't alias a {} variable to a {} one", var.kind.id(), k.id()),
        TargetError::Loop => "the alias would refer back to itself".into(),
    })
}

/// Whether what `seed` references (aliases and colour-calculation operands, in every mode) can
/// lead back to `only`. Bounded at 10,000 variables for hand-crafted files with bigger graphs.
fn reaches_back(vars: &Variables, seed: &VariableId, only: Option<&VariableId>) -> bool {
    let Some(only) = only else { return false };
    let mut stack = vec![seed.clone()];
    let mut seen = BTreeSet::new();
    while let Some(cur) = stack.pop() {
        if cur == *only {
            return true;
        }
        if !seen.insert(cur.clone()) || seen.len() > 10_000 {
            continue;
        }
        for v in vars.get_by_id(&cur).map(|x| x.values.values()).into_iter().flatten() {
            stack.extend(v.references().into_iter().cloned());
        }
    }
    false
}

/// `{"alpha": …}` / `{"mix": …}` values: a colour derived from what it reads, checked the moment it
/// is set (unknown operand, non-colour operand, bad parameter, a reference back to the variable
/// being defined). Names bind to ids here, the way aliases bind at the call sites.
pub(crate) fn colour_calc(cmd: &str, vars: &Variables, kind: VariableType, for_id: Option<&VariableId>, v: &Value) -> Result<VariableValue> {
    if kind != VariableType::Color {
        return Err(bad(cmd, "colour calculations only fit colour variables"));
    }
    // One operation per value: a second key beside it (another operation, `alias`, a typo) would
    // otherwise be dropped without a word.
    let obj = v.as_object().filter(|o| o.len() == 1).ok_or_else(|| bad(cmd, "a colour calculation is {alpha: …} or {mix: …} on its own"))?;
    match obj.get("alpha") {
        Some(a) => {
            // Only these two keys: an extra one (a typo beside `opacity`, an option this doesn't take)
            // must not be dropped without a word.
            let a = a
                .as_object()
                .filter(|a| a.keys().all(|k| k == "variable" || k == "opacity"))
                .ok_or_else(|| bad(cmd, "alpha: {variable, opacity}"))?;
            let name = a.get("variable").and_then(Value::as_str).ok_or_else(|| bad(cmd, "alpha needs the variable to fade"))?;
            // Parameters before operands: their errors don't depend on what the operands are.
            let opacity = unit(cmd, a, "opacity")?;
            Ok(VariableValue::ColorAlpha { variable: calc_operand(cmd, vars, for_id, name)?, opacity })
        }
        None => match obj.get("mix") {
            Some(m) => {
                let m = m
                    .as_object()
                    .filter(|m| m.keys().all(|k| matches!(k.as_str(), "from" | "to" | "t")))
                    .ok_or_else(|| bad(cmd, "mix: {from, to, t}"))?;
                let t = unit(cmd, m, "t")?;
                Ok(VariableValue::ColorMix {
                    from: mix_operand(cmd, vars, for_id, m.get("from"))?,
                    to: mix_operand(cmd, vars, for_id, m.get("to"))?,
                    t,
                })
            }
            None => Err(bad(cmd, "not a colour calculation")),
        },
    }
}

/// A calculation whose operands all exist can still read more variables than one resolve may (a
/// mix of two long alias chains): refused when it is set, rather than left showing nothing. It is
/// checked in `mode` with every other collection in its default mode, the modes exports write it
/// in; another mode of a collection it reads, or a later change to what it reads, can still take
/// it past the limit, as an alias chain can.
pub(crate) fn check_resolves(cmd: &str, vars: &Variables, id: &str, collection: &str, mode: &str, val: &VariableValue) -> Result<()> {
    let calc = matches!(val, VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. });
    if calc && vars.flattened_as(id, collection, mode, val).is_none() {
        return Err(bad(cmd, format!("the colour calculation reads more than {} variables", Variables::RESOLVE_LIMIT)));
    }
    Ok(())
}

/// `obj[key]` as a number from 0 to 1. Strict, unlike `fraction_of` (which reads numbers past 1 as
/// percentages): a mistyped 1.5 is refused instead of being stored as 1.5%. -0.0 is stored as 0.0
/// so saved files read cleanly.
fn unit(cmd: &str, obj: &serde_json::Map<String, Value>, key: &str) -> Result<f32> {
    let x = obj.get(key).and_then(Value::as_f64).filter(|x| (0.0..=1.0).contains(x));
    let x = x.ok_or_else(|| bad(cmd, format!("`{key}` must be a number from 0 to 1")))?;
    Ok(if x == 0.0 { 0.0 } else { x as f32 })
}

/// A calculation operand given by name: an existing colour variable, never one that leads back to
/// the variable being set. Returns its id.
fn calc_operand(cmd: &str, vars: &Variables, for_id: Option<&VariableId>, name: &str) -> Result<VariableId> {
    reference_target(vars, vars.get(name), VariableType::Color, for_id).map_err(|e| match e {
        TargetError::Missing => bad(cmd, format!("no variable `{name}` to calculate from")),
        TargetError::Kind(k) => bad(cmd, format!("colour calculations take colour variables: `{name}` is a {} one", k.id())),
        TargetError::Loop => bad(cmd, "the colour calculation would refer back to itself"),
    })
}

/// A mix operand: `{variable: name-or-id}` or `{literal: colour}` — a bare string is a colour name
/// as easily as a variable name, so the kind is always spelled out.
fn mix_operand(cmd: &str, vars: &Variables, for_id: Option<&VariableId>, v: Option<&Value>) -> Result<ColorOperand> {
    let shape = || bad(cmd, "mix operands are {variable: …} or {literal: colour}");
    let o = v.and_then(Value::as_object).filter(|o| o.len() == 1).ok_or_else(shape)?;
    if let Some(name) = o.get("variable").and_then(Value::as_str) {
        return Ok(ColorOperand::Variable { variable: calc_operand(cmd, vars, for_id, name)? });
    }
    let literal = crate::params::color_of(o.get("literal").ok_or_else(shape)?).ok_or_else(|| bad(cmd, "mix literal must be a colour"))?;
    Ok(ColorOperand::Literal { literal })
}

/// The variable scope of bindable property `field` on `node` (None: any variable of the type fits).
pub fn field_scope(doc: &Document, node: NodeId, field: &str) -> Option<&'static str> {
    Some(match field {
        "width" | "height" | "minWidth" | "maxWidth" | "minHeight" | "maxHeight" => "WIDTH_HEIGHT",
        "gap" | "columnGap" | "rowGap" | "crossGap" | "padding" | "paddingHorizontal" | "paddingVertical" | "paddingTop" | "paddingRight"
        | "paddingBottom" | "paddingLeft" => "GAP",
        "cornerRadius" | "topLeftRadius" | "topRightRadius" | "bottomRightRadius" | "bottomLeftRadius" => "CORNER_RADIUS",
        "opacity" => "OPACITY",
        "strokeWeight" | "strokeTopWeight" | "strokeRightWeight" | "strokeBottomWeight" | "strokeLeftWeight" => "STROKE_FLOAT",
        "fontSize" => "FONT_SIZE",
        "fontWeight" => "FONT_WEIGHT",
        "paragraphSpacing" => "PARAGRAPH_SPACING",
        "paragraphIndent" => "PARAGRAPH_INDENT",
        "lineHeight" => "LINE_HEIGHT",
        "letterSpacing" => "LETTER_SPACING",
        "characters" => "TEXT_CONTENT",
        "fontFamily" => "FONT_FAMILY",
        "fontStyle" => "FONT_STYLE",
        f => match super::lists::list_binding(f)? {
            (BoundList::Strokes, _, _) => "STROKE_COLOR",
            (BoundList::Effects, _, "color") => "EFFECT_COLOR",
            (BoundList::Effects, _, _) => "EFFECT_FLOAT",
            (BoundList::Fills, _, _) => match doc.node(node).map(|n| &n.kind) {
                Some(NodeKind::Text(_)) => "TEXT_FILL",
                Some(k) if k.is_frame_like() => "FRAME_FILL",
                _ => "SHAPE_FILL",
            },
            // Layout grid fields take any number variable.
            (BoundList::Grids, _, _) => return None,
        },
    })
}

/// Whether property `field` of `node` takes a variable of type `kind`: the type its scope belongs
/// to (number variables also fill text content), booleans for visibility, numbers for layout grid
/// fields; fields without a known type take any.
pub fn field_takes(doc: &Document, node: NodeId, field: &str, kind: VariableType) -> bool {
    match (field, field_scope(doc, node, field)) {
        ("visible", _) => kind == VariableType::Boolean,
        (_, Some(scope)) => scope_known(kind, scope),
        (f, None) => !matches!(super::lists::list_binding(f), Some((BoundList::Grids, _, _))) || kind == VariableType::Number,
    }
}
