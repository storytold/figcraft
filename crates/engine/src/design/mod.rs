//! Design review: deterministic checks for the usual AI-generated interface tells.
//!
//! `design.audit` reads a selection, a page or the whole document and reports what a designer
//! would fix first — spacing off the 4px grid, radii outside the scale, too many type sizes,
//! gradient text, glows, low-contrast text, emoji labels. It changes nothing. `design.fix` (the
//! planned follow-up) repairs the findings marked `fixable`.
//!
//! The rules are our own reading of the public "unslop" checklist (CC BY 4.0, heydamir/unslop):
//! only the ideas, no text or assets from it.

mod audit;
mod rules;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params;
use crate::{Result, Session, bad, spec};
use rules::{RULES, Severity, rule};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "design.audit",
            "Check design",
            "{scope?: selection|page|document (default: selection when something is selected, else page), rules?: [rule ids], minSeverity?: low|medium|high (default low)} → {scope, score, counts: {high, medium, low}, findings: [{rule, severity, node, path, message, fixable, suggestion?}], truncated}  reads the design and reports the AI-slop tells; changes nothing",
            false,
            audit_cmd
        ),
        spec!("design.rules", "List design rules", "{} → {rules: [{id, severity, fixable, description}]}", false, |_, _| Ok(
            json!({"rules": RULES.iter().map(|r| json!({
                "id": r.id,
                "severity": r.severity.id(),
                "fixable": r.fixable,
                "description": r.description,
            })).collect::<Vec<_>>()})
        )),
    ]
}

fn audit_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let scope = scope_arg(p)?;
    let min = severity_arg(p)?;
    let filter = filter_arg(p)?;
    let doc = s.doc()?;
    let selection = s.selection();
    let page = s.page()?;
    let (roots, label, anchor, path) = match scope.as_deref() {
        None if selection.is_empty() => (vec![page], "page", Some(page), node_label(doc, page)),
        None => selection_scope(doc, &selection),
        Some("selection") => selection_scope(doc, &selection),
        Some("page") => (vec![page], "page", Some(page), node_label(doc, page)),
        Some("document") => {
            let roots = doc.pages.clone();
            let anchor = roots.first().copied();
            let path = anchor.map(|id| node_label(doc, id)).unwrap_or_default();
            (roots, "document", anchor, path)
        }
        Some(other) => return Err(bad("design.audit", format!("`scope` must be selection, page or document, not `{other}`"))),
    };
    Ok(audit::run(doc, &roots, label, anchor, &path, min, filter.as_ref()))
}

fn selection_scope(doc: &Document, selection: &[NodeId]) -> (Vec<NodeId>, &'static str, Option<NodeId>, String) {
    let roots = top_most(doc, selection);
    let anchor = roots.first().copied();
    let path = anchor.map(|id| node_label(doc, id)).unwrap_or_default();
    (roots, "selection", anchor, path)
}

fn scope_arg(p: &Value) -> Result<Option<String>> {
    let Some(v) = params::get(p, "scope") else { return Ok(None) };
    let Value::String(s) = v else { return Err(bad("design.audit", "`scope` must be a string")) };
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Ok(None);
    }
    if matches!(s.as_str(), "selection" | "page" | "document") {
        Ok(Some(s))
    } else {
        Err(bad("design.audit", format!("`scope` must be selection, page or document, not `{s}`")))
    }
}

fn severity_arg(p: &Value) -> Result<Severity> {
    match params::string(p, "minSeverity").map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        None => Ok(Severity::Low),
        Some(s) => Severity::from_id(&s).ok_or_else(|| bad("design.audit", format!("`minSeverity` must be low, medium or high, not `{s}`"))),
    }
}

fn filter_arg(p: &Value) -> Result<Option<BTreeSet<String>>> {
    let Some(v) = params::get(p, "rules") else { return Ok(None) };
    let Value::Array(items) = v else { return Err(bad("design.audit", "`rules` must be an array of rule ids")) };
    let mut set = BTreeSet::new();
    for item in items {
        let Some(s) = item.as_str() else { return Err(bad("design.audit", "`rules` must be an array of rule ids")) };
        let id = s.trim();
        if rule(id).is_none() {
            return Err(bad("design.audit", format!("unknown rule `{id}` (run design.rules for the list)")));
        }
        set.insert(id.to_string());
    }
    Ok(Some(set))
}

/// Selected layers without the ones whose ancestor is selected too: their subtrees would be
/// audited twice.
fn top_most(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = Vec::new();
    for id in ids {
        if out.contains(id) {
            continue;
        }
        let mut parent = doc.node(*id).and_then(|n| n.parent);
        let mut depth = 0;
        let mut nested = false;
        while let Some(pid) = parent {
            if depth > 256 {
                break;
            }
            if ids.contains(&pid) {
                nested = true;
                break;
            }
            parent = doc.node(pid).and_then(|n| n.parent);
            depth += 1;
        }
        if !nested {
            out.push(*id);
        }
    }
    out
}

fn node_label(doc: &Document, id: NodeId) -> String {
    doc.node(id).map(|n| n.name.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Layer {}", id.0))
}
