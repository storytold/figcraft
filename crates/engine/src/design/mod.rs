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
mod fix;
mod rules;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{CollectionId, DesignPreset, DesignReview, Document, GRID_MAX, GRID_MIN, MAX_BRAND_COLORS, MAX_RADII, ModeId, NodeId};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params;
use crate::{EngineError, Result, Session, bad, spec};
use rules::{RULES, Severity, presets_of, rule};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "design.audit",
            "Check design",
            "{scope?: selection|page|document (default: selection when something is selected, else page), ids?: [layer ids] (instead of scope), rules?: [rule ids], minSeverity?: low|medium|high (default low), preset?: recommended|strict|accessibility (default: the file's), modes?: {collection: mode} (resolve bound colours in these variable modes; the contrast check uses them), limit?: 1000-1000000 (nodes before the report says truncated; default 50000)} → {preset, grid, modes?, scope, score, counts: {high, medium, low}, byRule: {rule: count}, nodes, limit, findings: [{rule, severity, node, path, message, fixable, suggestion?}], truncated, unchecked: [{rule, reason, layers?}]}  reads the design and reports the AI-slop tells; changes nothing (unchecked names the rules the run could not check: truncated, or a measure cap)",
            false,
            audit_cmd
        ),
        spec!(
            "design.fix",
            "Fix design",
            "{scope?: selection|page|document (default: selection when something is selected, else page), ids?: [layer ids] (instead of scope), rules?: [rule ids], preset?: recommended|strict|accessibility (default: the file's), exact?: bool (with ids: repair only findings on those layers, not their descendants), limit?: 1000-1000000 (nodes the walk looks at; default 50000), dryRun?: bool (default false)} → {scope, dryRun, changed, skipped, byRule, before: {score, counts, nodes, findings}, after: {score, counts, nodes, findings}}  repairs the findings marked fixable in one undo step; content, naming and gradients are left alone",
            true,
            fix_cmd
        ),
        spec!(
            "design.assert",
            "Assert design",
            "{scope?: selection|page|document (default: selection when something is selected, else page), ids?: [layer ids] (instead of scope), rules?: [rule ids], preset?: recommended|strict|accessibility (default: the file's), modes?: {collection: mode} (resolve bound colours in these variable modes), failOn?: low|medium|high (default high), minScore?: 0-100 (default 0), limit?: 1000-1000000 (nodes before the audit says truncated; default 50000)} → {ok: true, score, counts}  fails with an error when a finding at or above failOn exists, the score is below minScore, or the audit could not check a rule it asked for (truncated at the node limit it ran with, or past a measure cap: what it did not reach is unchecked, so narrow the scope, filter the rules or raise `limit`); read-only, for CI",
            false,
            assert_cmd
        ),
        spec!(
            "design.configure",
            "Configure design review",
            "{preset?: recommended|strict|accessibility, grid?: 1-64 (px, default 4), radii?: [px] (empty: the grid scale and a pill), brand?: [colour] (accent colours the file owns), disabled?: [rule ids] (rules off in this file), reset?: bool} → {preset, grid, radii, brand, disabled}  changes the file's design review settings in one undo step; audit, fix and assert use them from then on",
            true,
            configure_cmd
        ),
        spec!(
            "design.rules",
            "List design rules",
            "{} → {rules: [{id, severity, fixable, description, presets}], presets: [{id, description}]}",
            false,
            |_, _| Ok(json!({
                "rules": RULES.iter().map(|r| json!({
                    "id": r.id,
                    "severity": r.severity.id(),
                    "fixable": r.fixable,
                    "description": r.description,
                    "presets": presets_of(r.id),
                })).collect::<Vec<_>>(),
                "presets": DesignPreset::ALL.iter().map(|p| json!({
                    "id": p.id(),
                    "description": preset_description(*p),
                })).collect::<Vec<_>>(),
            }))
        ),
    ]
}

fn audit_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (label, roots, anchor, path) = scope_of(s, "design.audit", p)?;
    let min = severity_arg("design.audit", p)?;
    let filter = filter_arg("design.audit", p)?;
    let preset = preset_arg("design.audit", p)?;
    let modes = modes_arg("design.audit", p, s.doc()?)?;
    let limit = limit_arg("design.audit", p)?;
    Ok(audit::run(s.doc()?, &roots, &label, anchor, &path, min, filter.as_ref(), preset, modes, limit))
}

fn fix_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (label, roots, anchor, path) = scope_of(s, "design.fix", p)?;
    let filter = filter_arg("design.fix", p)?;
    let preset = preset_arg("design.fix", p)?;
    let exact = params::boolean(p, "exact").unwrap_or(false);
    let exact_ids = if exact { ids_arg("design.fix", p)? } else { None };
    let limit = limit_arg("design.fix", p)?;
    let dry = params::boolean(p, "dryRun").unwrap_or(false);
    // A dry run or a fix that changes nothing must not mark the file unsaved or add an undo step:
    // work on a copy and put it back only when it differs.
    let mut doc = s.doc()?.clone();
    let result = fix::run(&mut doc, &roots, anchor, &path, &label, preset, filter.as_ref(), exact_ids.as_deref(), limit, dry);
    if !dry && s.doc()? != &doc {
        *s.doc_mut()? = doc;
    }
    Ok(result)
}

/// `design.assert`: a read-only gate for CI. Returns the score when the design passes; an error
/// that names the counts when it does not.
fn assert_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (label, roots, anchor, path) = scope_of(s, "design.assert", p)?;
    let filter = filter_arg("design.assert", p)?;
    let fail_on = severity_named("design.assert", p, "failOn", Severity::High)?;
    let preset = preset_arg("design.assert", p)?;
    let modes = modes_arg("design.assert", p, s.doc()?)?;
    let min_score = match params::get(p, "minScore") {
        None => 0.0,
        Some(v) => match params::f64_of(v) {
            Some(v) if (0.0..=100.0).contains(&v) => v,
            _ => return Err(bad("design.assert", "`minScore` must be a number between 0 and 100")),
        },
    };
    // The score covers the whole review (as the window shows it); only findings at or above
    // `failOn` fail the gate.
    let limit = limit_arg("design.assert", p)?;
    let report = audit::run(s.doc()?, &roots, &label, anchor, &path, Severity::Low, filter.as_ref(), preset, modes, limit);
    let failing: Vec<&Value> = report["findings"]
        .as_array()
        .map(|v| v.iter().filter(|f| f["severity"].as_str().and_then(Severity::from_id).is_some_and(|s| s >= fail_on)).collect())
        .unwrap_or_default();
    let score = report["score"].as_f64().unwrap_or(0.0);
    // A truncated audit checked only the first nodes of the scope: what it did not reach is
    // unchecked, so it can never prove a design passes (a white-on-white label past the cap
    // would sail through as score 100).
    if report["truncated"].as_bool() == Some(true) {
        let nodes = report["nodes"].as_u64().unwrap_or(0);
        return Err(EngineError::Other(format!(
            "design review failed: the audit stopped at the {limit}-node limit after {nodes} nodes, so what it did not reach is unchecked; audit a smaller scope (selection, page or ids) or raise `limit`, then assert again"
        )));
    }
    // What the audit reported as unchecked, restricted to the rules this gate asked for. The
    // audit names each rule it could not check (`limit` truncation is handled above with its own
    // advice), so a cap added there fails this gate without a list here to keep in step.
    let unchecked: Vec<&str> = report["unchecked"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter(|e| e["reason"].as_str() == Some("measure-cap"))
                .filter_map(|e| e["rule"].as_str())
                .filter(|rule| filter.as_ref().is_none_or(|set| set.contains(*rule)))
                .collect()
        })
        .unwrap_or_default();
    if !unchecked.is_empty() {
        return Err(EngineError::Other(format!(
            "design review failed: the audit did not measure all of {} on this scope (past the per-run measure cap), so what it skipped is unchecked; audit a smaller scope, then assert again",
            unchecked.join(", ")
        )));
    }
    if !failing.is_empty() || score < min_score {
        let mut by_rule: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for f in &failing {
            if let Some(rule) = f["rule"].as_str() {
                *by_rule.entry(rule).or_default() += 1;
            }
        }
        let rules: Vec<String> = by_rule.iter().take(4).map(|(rule, count)| format!("{rule} ×{count}")).collect();
        let detail = if rules.is_empty() { String::new() } else { format!(" ({})", rules.join(", ")) };
        return Err(EngineError::Other(format!(
            "design review failed: score {score:.0}, {} finding(s) at {} or above{detail}",
            failing.len(),
            fail_on.id()
        )));
    }
    Ok(json!({"ok": true, "score": score, "counts": report["counts"]}))
}

fn severity_named(cmd: &str, p: &Value, key: &str, default: Severity) -> Result<Severity> {
    match params::string(p, key).map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(s) => Severity::from_id(&s).ok_or_else(|| bad(cmd, format!("`{key}` must be low, medium or high, not `{s}`"))),
    }
}

/// The `limit` parameter: how many nodes one audit walk may look at before the report says
/// `truncated`. Default 50,000; a CI gate over a huge file may raise it, never past the hard cap.
fn limit_arg(cmd: &str, p: &Value) -> Result<usize> {
    let Some(v) = params::get(p, "limit") else { return Ok(audit::MAX_NODES) };
    let Some(n) = params::f64_of(v) else { return Err(bad(cmd, "`limit` must be a whole number of nodes between 1,000 and 1,000,000")) };
    if !n.is_finite() || n.fract() != 0.0 || !(1_000.0..=(audit::MAX_LIMIT as f64)).contains(&n) {
        return Err(bad(cmd, "`limit` must be a whole number of nodes between 1,000 and 1,000,000"));
    }
    Ok(n as usize)
}

fn preset_arg(cmd: &str, p: &Value) -> Result<Option<DesignPreset>> {
    match params::string(p, "preset").map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => {
            DesignPreset::from_id(&s).map(Some).ok_or_else(|| bad(cmd, format!("`preset` must be recommended, strict or accessibility, not `{s}`")))
        }
    }
}

fn preset_description(preset: DesignPreset) -> &'static str {
    match preset {
        DesignPreset::Recommended => "The usual AI-generated tells",
        DesignPreset::Strict => "Recommended plus accent colours and frames without auto layout",
        DesignPreset::Accessibility => "Contrast, the type floor, gradient text and clipped text only",
    }
}

/// The variable modes a run resolves colours in: `{collection: mode}`, by id or by name.
fn modes_arg(cmd: &str, p: &Value, doc: &Document) -> Result<BTreeMap<CollectionId, ModeId>> {
    let Some(v) = params::get(p, "modes") else { return Ok(BTreeMap::new()) };
    let Value::Object(map) = v else { return Err(bad(cmd, "`modes` must be an object of collection → mode")) };
    if map.len() > 64 {
        return Err(bad(cmd, "at most 64 collections in `modes`"));
    }
    let mut out = BTreeMap::new();
    for (collection_key, mode_value) in map {
        let Some(mode_key) = mode_value.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return Err(bad(cmd, "`modes` values must be mode ids or names"));
        };
        let Some(collection) = doc.variables.collection(collection_key.trim()) else {
            return Err(bad(cmd, format!("unknown collection `{collection_key}`")));
        };
        let Some(mode) = collection.find_mode(mode_key) else {
            return Err(bad(cmd, format!("unknown mode `{mode_key}` in `{}`", collection.name)));
        };
        out.insert(collection.id.clone(), mode.id.clone());
    }
    Ok(out)
}

/// `design.configure`: change the file's design system and rules in one undo step.
fn configure_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let mut config = if params::boolean(p, "reset").unwrap_or(false) { DesignReview::default() } else { s.doc()?.design_review.clone() };
    if let Some(preset) = preset_arg("design.configure", p)? {
        config.preset = preset;
    }
    if let Some(v) = params::get(p, "grid") {
        let grid = params::f64_of(v)
            .filter(|g| g.is_finite() && (GRID_MIN..=GRID_MAX).contains(g))
            .ok_or_else(|| bad("design.configure", format!("`grid` must be a number from {GRID_MIN} to {GRID_MAX}")))?;
        config.grid = grid;
    }
    if let Some(v) = params::get(p, "radii") {
        let Value::Array(items) = v else { return Err(bad("design.configure", "`radii` must be an array of numbers")) };
        if items.len() > MAX_RADII {
            return Err(bad("design.configure", format!("at most {MAX_RADII} radii")));
        }
        let mut radii = Vec::new();
        for item in items {
            let r = params::f64_of(item)
                .filter(|r| r.is_finite() && (0.0..=999.0).contains(r))
                .ok_or_else(|| bad("design.configure", "each radius must be a number from 0 to 999"))?;
            radii.push((r * 100.0).round() / 100.0);
        }
        radii.sort_by(f64::total_cmp);
        radii.dedup_by(|a, b| (*a - *b).abs() <= 1e-6);
        config.radii = radii;
    }
    if let Some(v) = params::get(p, "brand") {
        let Value::Array(items) = v else { return Err(bad("design.configure", "`brand` must be an array of colours")) };
        if items.len() > MAX_BRAND_COLORS {
            return Err(bad("design.configure", format!("at most {MAX_BRAND_COLORS} brand colours")));
        }
        let mut brand = Vec::new();
        for item in items {
            let color = params::color_of(item).ok_or_else(|| bad("design.configure", "each brand colour must be a colour"))?;
            if color.a > 0.05 {
                brand.push(color);
            }
        }
        config.brand = brand;
    }
    if let Some(v) = params::get(p, "disabled") {
        let Value::Array(items) = v else { return Err(bad("design.configure", "`disabled` must be an array of rule ids")) };
        let mut ids = BTreeSet::new();
        for item in items {
            let Some(id) = item.as_str().map(str::trim) else {
                return Err(bad("design.configure", "`disabled` must be an array of rule ids"));
            };
            if rule(id).is_none() {
                return Err(bad("design.configure", format!("unknown rule `{id}` (run design.rules for the list)")));
            }
            ids.insert(id.to_string());
        }
        config.disabled = ids.into_iter().collect();
    }
    if s.doc()?.design_review == config {
        return Ok(settings_json(&config));
    }
    let doc = s.doc_mut()?;
    doc.design_review = config;
    Ok(settings_json(&doc.design_review))
}

fn settings_json(config: &DesignReview) -> Value {
    let brand: Vec<String> = config
        .brand
        .iter()
        .map(|c| {
            let rgba = c.to_rgba8();
            format!("#{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2])
        })
        .collect();
    json!({
        "preset": config.preset.id(),
        "grid": config.grid(),
        "radii": config.radii,
        "brand": brand,
        "disabled": config.disabled,
    })
}

/// The layers a command works on, its scope label, the layer aggregate findings point at and that
/// layer's path.
/// The `ids` parameter, parsed without collapsing ancestors: `exact` needs the layers the caller
/// listed, while the walk uses their top-most roots.
fn ids_arg(cmd: &str, p: &Value) -> Result<Option<Vec<NodeId>>> {
    let Some(v) = params::get(p, "ids") else { return Ok(None) };
    let Value::Array(items) = v else { return Err(bad(cmd, "`ids` must be an array of layer ids")) };
    let mut ids = Vec::new();
    for item in items {
        let Some(id) = params::id_of(item) else { return Err(bad(cmd, "`ids` must be an array of layer ids")) };
        ids.push(id);
    }
    Ok(Some(ids))
}

fn scope_of(s: &Session, cmd: &str, p: &Value) -> Result<(String, Vec<NodeId>, Option<NodeId>, String)> {
    let doc = s.doc()?;
    // `ids` names the layers directly (the review window fixes only the findings the user kept).
    // An empty or unknown id checks nothing, which a CI gate must not mistake for a pass.
    if let Some(ids) = ids_arg(cmd, p)? {
        if ids.is_empty() {
            return Err(bad(cmd, "`ids` is empty: name at least one layer to audit, or drop `ids` for the selection, page or document"));
        }
        let missing: Vec<String> = ids.iter().filter(|id| doc.node(**id).is_none()).map(|id| id.to_string()).collect();
        if !missing.is_empty() {
            return Err(bad(cmd, format!("`ids` names layers that do not exist: {}", missing.join(", "))));
        }
        let roots = top_most(doc, &ids);
        let anchor = roots.first().copied();
        let path = anchor.map(|id| node_label(doc, id)).unwrap_or_default();
        return Ok(("selection".into(), roots, anchor, path));
    }
    let scope = scope_arg(cmd, p)?;
    let selection = s.selection();
    let page = s.page()?;
    // An explicitly empty selection checks nothing either; only the implicit default falls back
    // to the page. A CI gate must not read an unchecked scope as a pass.
    if scope.as_deref() == Some("selection") && selection.is_empty() {
        return Err(bad(cmd, "nothing is selected: name `ids`, pick a scope of page or document, or select the layers to audit"));
    }
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
        Some(other) => return Err(bad(cmd, format!("`scope` must be selection, page or document, not `{other}`"))),
    };
    Ok((label.to_string(), roots, anchor, path))
}

fn selection_scope(doc: &Document, selection: &[NodeId]) -> (Vec<NodeId>, &'static str, Option<NodeId>, String) {
    let roots = top_most(doc, selection);
    let anchor = roots.first().copied();
    let path = anchor.map(|id| node_label(doc, id)).unwrap_or_default();
    (roots, "selection", anchor, path)
}

fn scope_arg(cmd: &str, p: &Value) -> Result<Option<String>> {
    let Some(v) = params::get(p, "scope") else { return Ok(None) };
    let Value::String(s) = v else { return Err(bad(cmd, "`scope` must be a string")) };
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Ok(None);
    }
    if matches!(s.as_str(), "selection" | "page" | "document") {
        Ok(Some(s))
    } else {
        Err(bad(cmd, format!("`scope` must be selection, page or document, not `{s}`")))
    }
}

fn severity_arg(cmd: &str, p: &Value) -> Result<Severity> {
    match params::string(p, "minSeverity").map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        None => Ok(Severity::Low),
        Some(s) => Severity::from_id(&s).ok_or_else(|| bad(cmd, format!("`minSeverity` must be low, medium or high, not `{s}`"))),
    }
}

fn filter_arg(cmd: &str, p: &Value) -> Result<Option<BTreeSet<String>>> {
    let Some(v) = params::get(p, "rules") else { return Ok(None) };
    let Value::Array(items) = v else { return Err(bad(cmd, "`rules` must be an array of rule ids")) };
    // A filter that names nothing checks nothing: a gate would pass over a review that never
    // ran, the same way a truncated audit would.
    if items.is_empty() {
        return Err(bad(
            cmd,
            "`rules` names no rule, so nothing would be checked; list at least one (design.rules has them) or leave `rules` out to check them all",
        ));
    }
    let mut set = BTreeSet::new();
    for item in items {
        let Some(s) = item.as_str() else { return Err(bad(cmd, "`rules` must be an array of rule ids")) };
        let id = s.trim();
        if rule(id).is_none() {
            return Err(bad(cmd, format!("unknown rule `{id}` (run design.rules for the list)")));
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
