//! Variables import and export: a file's collections as the local variables JSON (the shape the
//! REST importer reads, so exports round-trip) or as CSS custom properties, and such JSON read
//! back into a file.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::*;
use serde_json::{Value, json};

use super::CommandSpec;
use crate::codegen::{css_var_name, css_var_of};
use crate::library::{is_remote_collection, local_collections};
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "variable.export",
            "Export variables",
            "{collections? [ids or names; default every collection of this file, not library copies], format? (json (default): the local variables JSON that variable.import reads | css: custom properties, the default mode on :root and each other mode on [data-<collection>=\"<mode>\"]), path? (write it there)}  → {text}, or {path} when written",
            false,
            export
        ),
        spec!(
            "variable.import",
            "Import variables",
            "{variables (the local variables JSON, as variable.export writes it: an object, JSON text or a path; `variableCollections` and `variables` under `meta` or at the top level)}  collections join this file's collection of the same name (values by mode name; new modes and variables are added, a variable of another type is skipped), others are added  → {collections (added), variables (added), updated, warnings (also when it holds no variables, or fewer than its collections list)}",
            true,
            import
        ),
    ]
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.export";
    let doc = s.doc()?;
    let colls: Vec<&Collection> = match get(p, "collections") {
        None | Some(Value::Null) => local_collections(doc).collect(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| x.as_str().and_then(|k| doc.variables.collection(k)).ok_or_else(|| bad(cmd, format!("no collection {x}"))))
            .collect::<Result<_>>()?,
        Some(_) => return Err(bad(cmd, "collections: [collection ids or names]")),
    };
    let text = match string(p, "format").as_deref().unwrap_or("json") {
        "json" => serde_json::to_string_pretty(&figcraft_format::rest::export_variables(&doc.variables, &colls))
            .map_err(|e| EngineError::Other(e.to_string()))?,
        "css" => css(&doc.variables, &colls),
        _ => return Err(bad(cmd, "format: json|css")),
    };
    match string(p, "path") {
        Some(path) => {
            super::file::write(&path, text.as_bytes())?;
            Ok(json!({"path": path}))
        }
        None => Ok(json!({"text": text})),
    }
}

/// `text` as a CSS comment or quoted string body.
fn css_safe(text: &str) -> String {
    text.replace("*/", "* /").replace(['"', '\\', '\n'], " ")
}

/// Each collection's variables as custom properties (named as generated code names them): the
/// default mode's values on `:root`, every other mode's on `[data-<collection>="<mode>"]`;
/// aliases are `var(--name)`.
fn css(vars: &Variables, colls: &[&Collection]) -> String {
    let mut out = String::new();
    for c in colls {
        let attr = css_var_name(&c.name);
        for (i, m) in c.modes.iter().enumerate() {
            let sel = if i == 0 { ":root".to_string() } else { format!("[data-{}=\"{}\"]", attr.trim_start_matches('-'), css_safe(&m.name)) };
            out.push_str(&format!("/* {} · {} */\n{sel} {{\n", css_safe(&c.name), css_safe(&m.name)));
            for v in vars.variables.iter().filter(|v| v.collection == c.id) {
                let value = match v.values.get(&m.id) {
                    Some(VariableValue::Color(col)) => col.to_css(),
                    Some(VariableValue::Number(x)) => trim_num(*x),
                    Some(VariableValue::String(t)) => format!("\"{}\"", css_safe(t)),
                    Some(VariableValue::Boolean(b)) => b.to_string(),
                    Some(VariableValue::Alias(a)) => match vars.get(a) {
                        Some(t) => format!("var({})", css_var_of(t)),
                        None => continue,
                    },
                    None => continue,
                };
                out.push_str(&format!("  {}: {value};\n", css_var_of(v)));
            }
            out.push_str("}\n");
        }
    }
    out
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.import";
    let v =
        super::file::variables_param(cmd, p)?.ok_or_else(|| bad(cmd, "`variables` (the variables JSON: an object, JSON text or a path) required"))?;
    let (src, mut warnings) = figcraft_format::rest::read_variables(&v).ok_or_else(|| bad(cmd, "not a variables JSON"))?;
    let doc = s.doc_mut()?;
    // Imported collection → (this file's collection, imported mode → its mode).
    let mut colls: BTreeMap<CollectionId, (CollectionId, BTreeMap<ModeId, ModeId>)> = BTreeMap::new();
    let mut added_colls = 0usize;
    for c in &src.collections {
        // Never a library copy (read-only) or an extended collection (its modes are inherited).
        let same = doc.variables.collections.iter().find(|x| x.name == c.name && x.extends.is_none() && !is_remote_collection(doc, &x.id));
        let cid = match same.map(|x| x.id.clone()) {
            Some(id) => id,
            None => {
                let id = doc.new_collection_id();
                let name = super::variables::free_collection_name(doc, &c.name);
                doc.variables.collections.push(Collection { id: id.clone(), name, modes: Vec::new(), extends: None });
                added_colls += 1;
                id
            }
        };
        let mut modes = BTreeMap::new();
        for m in &c.modes {
            let coll = doc.variables.collections.iter().find(|x| x.id == cid);
            let mid = match coll.and_then(|x| x.modes.iter().find(|y| y.name == m.name)) {
                Some(y) => y.id.clone(),
                None => {
                    let first = coll.and_then(|x| x.default_mode()).cloned();
                    let Some(id) = doc.new_mode_id(&cid) else { continue };
                    if let Some(x) = doc.variables.collections.iter_mut().find(|x| x.id == cid) {
                        x.modes.push(Mode { id: id.clone(), name: m.name.clone() });
                    }
                    // Its variables start with their default mode's values (as with variable.addMode).
                    for var in doc.variables.variables.iter_mut().filter(|x| x.collection == cid) {
                        if let Some(val) = first.as_ref().and_then(|f| var.values.get(f)).cloned() {
                            var.values.insert(id.clone(), val);
                        }
                    }
                    id
                }
            };
            modes.insert(m.id.clone(), mid);
        }
        colls.insert(c.id.clone(), (cid, modes));
    }
    // Every variable's id here first (aliases point at any of them), then the values.
    let mut ids: BTreeMap<VariableId, VariableId> = BTreeMap::new();
    let mut added = 0usize;
    let mut updated = 0usize;
    for v in &src.variables {
        let Some((cid, _)) = colls.get(&v.collection) else { continue };
        match doc.variables.variables.iter().find(|x| x.collection == *cid && x.name == v.name) {
            Some(x) if x.kind != v.kind => {
                warnings.push(format!("`{}` is a {} variable in this file: the {} one was skipped", v.name, x.kind.id(), v.kind.id()));
            }
            Some(x) => {
                ids.insert(v.id.clone(), x.id.clone());
                updated += 1;
            }
            None => {
                let id = doc.new_variable_id();
                let blank = Variable {
                    id: id.clone(),
                    collection: cid.clone(),
                    values: BTreeMap::new(),
                    extends: None,
                    overrides: BTreeSet::new(),
                    ..v.clone()
                };
                doc.variables.variables.push(blank);
                ids.insert(v.id.clone(), id);
                added += 1;
            }
        }
    }
    for v in &src.variables {
        let (Some(local), Some((cid, modes))) = (ids.get(&v.id), colls.get(&v.collection)) else { continue };
        for (m, val) in &v.values {
            let Some(lm) = modes.get(m) else { continue };
            let val = match val {
                VariableValue::Alias(a) => {
                    let Some(var) = doc.variables.variables.iter().find(|x| x.id == *local) else { continue };
                    let target = ids.get(a).ok_or_else(|| "its alias target wasn't imported".to_string());
                    match target.and_then(|t| super::variables::alias_target(&doc.variables, var, t)) {
                        Ok(t) => VariableValue::Alias(t),
                        Err(e) => {
                            warnings.push(format!("`{}`: {e}", v.name));
                            continue;
                        }
                    }
                }
                other => other.clone(),
            };
            if let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == *local) {
                slot.values.insert(lm.clone(), val);
            }
        }
        // A value in every mode of its collection: modes the JSON lacks take the default mode's
        // (a new variable whose values all failed starts like one made with variable.create).
        let all: Vec<ModeId> =
            doc.variables.collections.iter().find(|x| x.id == *cid).map(|x| x.modes.iter().map(|m| m.id.clone()).collect()).unwrap_or_default();
        if let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == *local) {
            let first = all.iter().find_map(|m| slot.values.get(m)).cloned().unwrap_or_else(|| VariableValue::default_for(slot.kind));
            for m in all {
                slot.values.entry(m).or_insert_with(|| first.clone());
            }
        }
    }
    Ok(json!({"collections": added_colls, "variables": added, "updated": updated, "warnings": warnings}))
}
