//! Sections and handoff: hide a section's contents, resolve a change to a ready-for-dev layer,
//! compare it with what it was when it was marked, and the focus view on one frame.
//! (Wrapping layers in a new section is `object.wrapInSection`, with the other wrappers.)

use figcraft_doc::{DevStatus, NodeId, NodeKind};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::devinspect::{dev_items, has_status};
use crate::params::{boolean, get, req_targets, string, targets};
use crate::{Result, Session, bad, spec};

/// Longest change reason kept.
const MAX_NOTE: usize = 500;
/// Room the focus view's bar takes at the top of the canvas (logical pixels).
const FOCUS_BAR_H: f64 = 56.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "section.toggleContents",
            "Hide/Show section contents",
            "{ids?, hidden? (default: hide unless all are hidden)} (a hidden section shows only its name and outline; its layers aren't drawn or hit)",
            true,
            toggle_contents
        ),
        spec!(
            "dev.resolveChange",
            "Resolve change",
            "{ids?, reason?} (ready-for-dev frames and sections that changed since they were marked go back to Ready for dev; the reason is kept)",
            true,
            resolve_change
        ),
        spec!(
            "dev.compare",
            "Compare changes",
            "{id? (default the first selected layer), version? (a version id from version.list: compare with that saved version instead; any top-level frame or section), show? (open the compare view in the app)} → {id, name, changed, snapshot, added, removed, modified, truncated, version: {id, name, time} | null, changes: [{id, name, type, change: added|removed|changed, depth, layers, properties?: [{section, property, before, after, text}]}]}  what changed in a ready-for-dev frame or section since it was marked, or since the version (`snapshot` is false when that isn't known: a file from before snapshots were kept)",
            false,
            compare
        ),
        spec!(
            "dev.focus",
            "Focus view",
            "{id? (a top-level frame or section; default the selection) | direction?: next|previous (the next ready-for-dev one, across pages)} → {id, name, page, index, count}  show only that layer on the canvas (Escape or `dev.unfocus` ends it)",
            false,
            focus
        ),
        spec!("dev.unfocus", "Exit focus view", "{} → {focused: false}", false, |s, _| {
            s.ds_mut()?.focus = None;
            Ok(json!({"focused": false}))
        }),
    ]
}

fn toggle_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "section.toggleContents";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc()?;
    let sections: Vec<NodeId> = ids.iter().copied().filter(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Section { .. }))).collect();
    if sections.is_empty() {
        return Err(bad(cmd, "select a section"));
    }
    let hide = boolean(p, "hidden").unwrap_or_else(|| !sections.iter().all(|i| doc.node(*i).is_some_and(|n| n.hides_contents())));
    // Selected layers inside a section being hidden give way to the section.
    let sel: Vec<NodeId> = s.selection();
    let mut new_sel: Vec<NodeId> = Vec::new();
    for i in sel {
        let inside = if hide { sections.iter().copied().find(|sec| doc.is_ancestor(*sec, i)) } else { None };
        let keep = inside.unwrap_or(i);
        if !new_sel.contains(&keep) {
            new_sel.push(keep);
        }
    }
    let doc = s.doc_mut()?;
    for sec in &sections {
        if let NodeKind::Section { contents_hidden } = &mut doc.get_mut(*sec)?.kind {
            *contents_hidden = hide;
        }
    }
    s.set_selection(new_sel);
    Ok(json!({"hidden": hide, "ids": sections}))
}

fn resolve_change(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dev.resolveChange";
    let ids = req_targets(cmd, s, p)?;
    let reason: String = string(p, "reason").unwrap_or_default().trim().chars().take(MAX_NOTE).collect();
    let doc = s.doc_mut()?;
    let mut out = Vec::new();
    for i in ids {
        let n = doc.get_mut(i)?;
        if n.dev_status == DevStatus::ReadyForDev && n.dev_changed {
            n.dev_changed = false;
            n.dev_note = reason.clone();
            out.push(i);
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, "nothing selected has changed since it was marked ready for dev"));
    }
    Ok(json!({"ids": out}))
}

fn compare(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dev.compare";
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a frame or section or pass `id`"))?;
    let version = match get(p, "version") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) => Some(v.clone()),
        Some(_) => return Err(bad(cmd, "version: a version id (version.list)")),
    };
    let ds = s.ds()?;
    let doc = &ds.doc;
    let n = doc.node(id).ok_or_else(|| bad(cmd, "no such layer"))?;
    if !has_status(doc, id) {
        return Err(bad(cmd, "compare a top-level frame or a section"));
    }
    let (cmp, against) = match &version {
        Some(v) => {
            let ver = ds.versions.iter().find(|x| x.id == *v).ok_or_else(|| bad(cmd, format!("no version `{v}`")))?;
            let before = crate::cmd::file::read_version(ver)?;
            (crate::devcompare::compare_with(&before, doc, id), json!({"id": ver.id, "name": ver.name, "time": ver.time}))
        }
        None if n.dev_status != DevStatus::ReadyForDev => return Err(bad(cmd, "compare a frame or section marked ready for dev, or pass `version`")),
        None => (crate::devcompare::compare(doc, id), Value::Null),
    };
    let mut v = serde_json::to_value(cmp).unwrap_or(Value::Null);
    v["version"] = against;
    if boolean(p, "show") == Some(true) {
        s.request_ui("view.devCompare", json!({"id": id, "version": version}));
    }
    Ok(v)
}

fn focus(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "dev.focus";
    let doc = s.doc()?.clone();
    let ready: Vec<NodeId> = dev_items(&doc, Some(DevStatus::ReadyForDev), None).into_iter().map(|i| i.id).collect();
    let id = match string(p, "direction") {
        Some(d) => {
            let step: isize = match d.to_ascii_lowercase().as_str() {
                "next" => 1,
                "previous" | "prev" => -1,
                _ => return Err(bad(cmd, "direction: next|previous")),
            };
            let count = ready.len() as isize;
            if count == 0 {
                return Err(bad(cmd, "nothing is ready for dev"));
            }
            let at = s.ds()?.focused().and_then(|f| ready.iter().position(|r| *r == f));
            let k = match at {
                Some(i) => (i as isize + step).rem_euclid(count),
                None if step > 0 => 0,
                None => count - 1,
            };
            ready.get(k as usize).copied().ok_or_else(|| bad(cmd, "nothing is ready for dev"))?
        }
        None => targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a frame or section or pass `id`"))?,
    };
    if !has_status(&doc, id) {
        return Err(bad(cmd, "focus on a frame at the top of a page or section, or a section"));
    }
    crate::cmd::edit::select_and_page(s, vec![id])?;
    let ds = s.ds_mut()?;
    ds.focus = Some(id);
    if let Some(b) = ds.doc.world_bounds(id) {
        // Fitted below the focus view's bar.
        let v = &mut ds.view;
        let vh = v.vh;
        v.vh = (vh - FOCUS_BAR_H).max(100.0);
        v.fit(b, 2.0);
        v.vh = vh;
        v.cy -= FOCUS_BAR_H / 2.0 / v.zoom.max(1e-9);
    }
    let name = doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
    let index = ready.iter().position(|r| *r == id);
    Ok(json!({"id": id, "name": name, "page": doc.page_of(id), "index": index, "count": ready.len()}))
}
