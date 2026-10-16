//! Branches of a file: a copy of the document to work on separately (in its own tab), reviewed
//! against the main file and merged back. A branch remembers the main document as it was when
//! the branch was made or last updated from main (its base); [`figcraft_collab::merge`] compares
//! both sides with it. Branches are saved in the main file ([`figcraft_format::Branch`]); a
//! branch tab's edits and its own version history go into its branch ([`sync`]) and are saved with
//! the main file. Merging adds a version to the main file's history; the branch keeps its own.

use std::collections::BTreeMap;
use std::sync::Arc;

pub use figcraft_collab::merge::Side;
use figcraft_collab::merge::{self, Conflict, Key, Plan};
use figcraft_collab::{Change, Entry, Prop, Target};
use figcraft_doc::{Document, NodeId};
use figcraft_format::Branch;
pub use figcraft_format::BranchState;
use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::devcompare::{self, ChangeKind, LayerChange, PropChange};
use crate::params::{get, string};
use crate::{DocState, EngineError, Result, Session, bad, normalize, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "branch.create",
            "Create branch",
            "{name?}  a copy of the file to work on separately, opened in a new tab; it remembers the file as it is now (its base) → {id, name, index (its tab)}",
            false,
            create
        ),
        spec!(
            "branch.list",
            "List branches",
            "{}  the branches of the file shown (or of the file a branch tab belongs to), oldest first → [{id, name, time (Unix seconds), state: active|merged|archived, index? (its open tab)}]",
            false,
            |s, _| list(s)
        ),
        spec!("branch.open", "Open branch", "{id}  open the branch in a tab (or switch to its tab) → {index}", false, |s, p| {
            let (mi, bi) = locate(s, "branch.open", string(p, "id").as_deref())?;
            let (uid, id) = s.docs.get(mi).and_then(|m| Some((m.uid, m.branches.get(bi)?.id.clone()))).ok_or(EngineError::NoDocument)?;
            Ok(json!({"index": open_tab(s, uid, &id)?}))
        }),
        spec!(
            "branch.review",
            "Review branch",
            "{id? (default: the branch tab shown)}  what changed on the branch and on the main file since the branch was made or last updated from main, and where they conflict (the same property changed on both sides, or a layer deleted on one side and edited on the other) → {id, name, state, branch: {layers, document, truncated}, main: {…}, conflicts: [{key, id?, name, property, branch, main, deletedBy?}]}",
            false,
            |s, p| serde_json::to_value(review(s, string(p, "id").as_deref())?).map_err(|e| EngineError::Other(e.to_string()))
        ),
        spec!(
            "branch.updateFromMain",
            "Update from main",
            "{id?, resolutions? ({<conflict key>: \"branch\" | \"main\"})}  bring the main file's changes since the branch was made into the branch; refused while a conflict has no resolution. Not undoable (the branch tab's undo history starts over) → {updated, changes, conflicts}",
            false,
            |s, p| combine(s, p, Side::Main)
        ),
        spec!(
            "branch.merge",
            "Merge branch",
            "{id?, resolutions? ({<conflict key>: \"branch\" | \"main\"})}  merge the branch into the main file as one undo step, then archive it and show the main file; refused while a conflict has no resolution → {merged, changes, conflicts, index}",
            false,
            |s, p| combine(s, p, Side::Branch)
        ),
        spec!(
            "branch.archive",
            "Archive branch",
            "{id?}  set the branch aside without merging (it stays in the file and can still be opened) and close its tab",
            false,
            archive
        ),
    ]
}

/// Everything shown for reviewing a branch.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub id: String,
    pub name: String,
    pub state: BranchState,
    /// What changed on the branch since its base.
    pub branch: SideChanges,
    /// What changed on the main file since the branch's base.
    pub main: SideChanges,
    pub conflicts: Vec<ConflictRow>,
}

/// One side's changes since the base.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideChanges {
    /// Layers added, removed or changed (property changes formatted like Inspect).
    pub layers: Vec<LayerChange>,
    /// Styles, variables, comments and file settings.
    pub document: Vec<DocChange>,
    /// More than [`devcompare::MAX_CHANGES`] layers changed: the list stops there.
    pub truncated: bool,
}

/// A document-level change: a style, variable or comment added, changed or removed, or a file
/// setting changed.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocChange {
    pub what: String,
    pub change: ChangeKind,
}

/// A conflict, described for choosing a side.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRow {
    /// What resolutions name it by.
    pub key: String,
    /// The layer it is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<NodeId>,
    /// The layer, style, variable… it is about.
    pub name: String,
    /// What conflicts: a layer property ("opacity"), "layer" (deleted on one side and edited on
    /// the other), a file setting, or what the entry is ("style").
    pub property: String,
    /// What the branch has.
    pub branch: String,
    /// What the main file has.
    pub main: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<Side>,
}

/// Put each branch tab's document into its branch, so saving the main file (and its unsaved
/// state) covers the branch. Runs before every command.
pub(crate) fn sync(s: &mut Session) {
    if !s.docs.iter().any(|d| d.branch_of.is_some()) {
        return;
    }
    let mut tabs = Vec::new();
    for d in s.docs.iter_mut() {
        let Some((m, id)) = d.branch_of.clone() else { continue };
        // Versions saved, renamed or deleted on the tab since the last sync.
        let versions = std::mem::take(&mut d.versions_changed).then(|| d.versions.clone());
        tabs.push((m, id, d.document_itself().clone(), versions));
    }
    for (main, id, doc, versions) in tabs {
        let Some(ds) = s.docs.iter_mut().find(|d| d.uid == main) else { continue };
        let Some(i) = ds.branches.iter().position(|b| b.id == id && (versions.is_some() || !Arc::ptr_eq(&b.head, &doc))) else { continue };
        if let Some(b) = ds.branches_mut().get_mut(i) {
            b.head = doc;
            if let Some(v) = versions {
                b.versions = v;
            }
        }
    }
}

/// The branches of the file in tab `uid` as saving it now would write them: [`sync`] only puts a
/// branch tab's latest edits in before the next command.
pub fn current(s: &Session, uid: u64) -> Vec<Branch> {
    let Some(ds) = s.docs.iter().find(|d| d.uid == uid) else { return Vec::new() };
    let mut branches = ds.branches.clone();
    for d in &s.docs {
        let Some((_, id)) = d.branch_of.as_ref().filter(|(main, _)| *main == uid) else { continue };
        let Some(b) = branches.iter_mut().find(|b| b.id == *id) else { continue };
        b.head = d.document_itself().clone();
        if d.versions_changed {
            b.versions = d.versions.clone();
        }
    }
    branches
}

/// The tab a save writes: the main file's for a branch tab, else the active tab.
pub fn file_tab(s: &Session) -> usize {
    s.ds().ok().and_then(|d| d.branch_of.as_ref()).and_then(|(m, _)| s.docs.iter().position(|d| d.uid == *m)).unwrap_or(s.active)
}

/// The tab showing branch `id` of the file in the tab with uid `main`.
pub fn tab_of(s: &Session, main: u64, id: &str) -> Option<usize> {
    s.docs.iter().position(|d| d.branch_of.as_ref().is_some_and(|(m, b)| *m == main && b == id))
}

/// Close the tabs showing branches of the file in the tab with uid `main` (`id`: only that
/// branch's). Their edits are already in the branches ([`sync`]).
pub(crate) fn close_tabs(s: &mut Session, main: u64, id: Option<&str>) {
    let active = s.docs.get(s.active).map(|d| d.uid);
    s.docs.retain(|d| !d.branch_of.as_ref().is_some_and(|(m, b)| *m == main && id.is_none_or(|i| i == b)));
    s.active = active.and_then(|a| s.docs.iter().position(|d| d.uid == a)).unwrap_or(s.active).min(s.docs.len().saturating_sub(1));
}

/// The main file's tab: the active tab, or the file a branch tab belongs to.
fn main_index(s: &Session, cmd: &str) -> Result<usize> {
    let ds = s.ds()?;
    let main = ds.branch_of.as_ref().map_or(ds.uid, |(m, _)| *m);
    s.docs.iter().position(|d| d.uid == main).ok_or_else(|| EngineError::Disabled(cmd.into(), "the branch's main file isn't open".into()))
}

/// The main file's tab and branch `id` in it (default: the branch the active tab shows).
pub fn locate(s: &Session, cmd: &str, id: Option<&str>) -> Result<(usize, usize)> {
    let mi = main_index(s, cmd)?;
    let id = id.map(str::to_string).or_else(|| s.ds().ok().and_then(|d| d.branch_of.as_ref()).map(|(_, b)| b.clone()));
    let id = id.ok_or_else(|| bad(cmd, "pass the branch `id`"))?;
    let bi = s.docs.get(mi).and_then(|m| m.branches.iter().position(|b| b.id == id)).ok_or_else(|| bad(cmd, format!("no branch `{id}`")))?;
    Ok((mi, bi))
}

/// Branch `bi` of the file in tab `mi`: its base, its document (from its tab when open) and the
/// main document.
pub fn documents(s: &Session, mi: usize, bi: usize) -> Option<(Arc<Document>, Arc<Document>, Arc<Document>)> {
    let m = s.docs.get(mi)?;
    let b = m.branches.get(bi)?;
    let head = tab_of(s, m.uid, &b.id).and_then(|t| s.docs.get(t)).map_or_else(|| b.head.clone(), |t| t.document_itself().clone());
    Some((b.base.clone(), head, m.document_itself().clone()))
}

fn disabled(cmd: &str, why: impl Into<String>) -> EngineError {
    EngineError::Disabled(cmd.into(), why.into())
}

/// shortcut: branches of a file in a live session are refused (its participants' client ids
/// aren't known here, so a branch's id space could clash); allow them once the server hands out
/// branch id spaces.
fn refuse_live(s: &Session, cmd: &str, uid: u64) -> Result<()> {
    if s.live_doc(uid) { Err(disabled(cmd, "branches aren't available in a live session")) } else { Ok(()) }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "branch.create";
    let ds = s.ds()?;
    if ds.branch_of.is_some() {
        return Err(disabled(CMD, "this tab is a branch: make branches from the main file"));
    }
    refuse_live(s, CMD, ds.uid)?;
    if ds.branches.len() >= figcraft_format::MAX_BRANCHES {
        return Err(disabled(CMD, format!("the file has the most branches it can keep ({})", figcraft_format::MAX_BRANCHES)));
    }
    let name =
        string(p, "name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Branch {}", ds.branches.len() + 1));
    let name = figcraft_format::clip(&name, figcraft_format::MAX_VERSION_NAME);
    let base = ds.document_itself().clone();
    // Its own id space, above every space the file, its live sessions and its branches used.
    let client = ds.branches.iter().map(|b| b.client).chain([base.highest_id_space(), base.client_id()]).max().unwrap_or(0);
    let client = client.checked_add(1).ok_or_else(|| disabled(CMD, "no id space left for a new branch"))?;
    let mut head = (*base).clone();
    head.name = name.clone();
    head.set_client_id(client);
    let id = crate::cmd::file::unique_id(|id| ds.branches.iter().any(|b| b.id == id));
    let uid = ds.uid;
    let time = crate::cmd::comment::now_secs();
    s.ds_mut()?.branches_mut().push(Branch {
        id: id.clone(),
        name: name.clone(),
        time,
        client,
        state: BranchState::Active,
        base,
        head: Arc::new(head),
        versions: Vec::new(),
    });
    let index = open_tab(s, uid, &id)?;
    Ok(json!({"id": id, "name": name, "index": index}))
}

/// Open branch `id` of the file in the tab with uid `main` in a tab, or switch to its tab.
fn open_tab(s: &mut Session, main: u64, id: &str) -> Result<usize> {
    if let Some(i) = tab_of(s, main, id) {
        s.active = i;
        return Ok(i);
    }
    let m = s.docs.iter_mut().find(|d| d.uid == main).ok_or(EngineError::NoDocument)?;
    let b = m.branches.iter_mut().find(|b| b.id == id).ok_or_else(|| bad("branch.open", format!("no branch `{id}`")))?;
    let mut doc = (*b.head).clone();
    // A branch read from a file has no client id yet.
    doc.set_client_id(b.client);
    // The branch's own version history: the tab's versions while it is open ([`sync`]).
    let mut tab = DocState::new(doc, None).with_versions(b.versions.clone());
    tab.branch_of = Some((main, id.to_string()));
    // The same content: opening isn't an edit of the main file.
    b.head = tab.doc.clone();
    s.docs.push(tab);
    s.active = s.docs.len() - 1;
    Ok(s.active)
}

fn list(s: &Session) -> Result<Value> {
    let m = s.docs.get(main_index(s, "branch.list")?).ok_or(EngineError::NoDocument)?;
    Ok(Value::Array(
        m.branches
            .iter()
            .map(|b| {
                let mut j = json!({"id": b.id, "name": b.name, "time": b.time, "state": b.state});
                if let Some(i) = tab_of(s, m.uid, &b.id) {
                    j["index"] = json!(i);
                }
                j
            })
            .collect(),
    ))
}

fn archive(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "branch.archive";
    let (mi, bi) = locate(s, CMD, string(p, "id").as_deref())?;
    let m = s.docs.get_mut(mi).ok_or(EngineError::NoDocument)?;
    if m.branches.get(bi).is_none_or(|b| b.state != BranchState::Active) {
        return Err(disabled(CMD, "the branch was already merged or archived"));
    }
    let (uid, id) = (m.uid, m.branches.get(bi).map(|b| b.id.clone()).unwrap_or_default());
    if let Some(b) = m.branches_mut().get_mut(bi) {
        b.state = BranchState::Archived;
    }
    close_tabs(s, uid, Some(&id));
    Ok(json!({"archived": true}))
}

/// The side for each conflict key in `resolutions`.
fn resolutions(cmd: &str, p: &Value) -> Result<BTreeMap<String, Side>> {
    let Some(v) = get(p, "resolutions") else { return Ok(BTreeMap::new()) };
    let obj = v.as_object().ok_or_else(|| bad(cmd, "`resolutions` is an object: {<conflict key>: \"branch\" | \"main\"}"))?;
    obj.iter()
        .map(|(k, v)| {
            let side = v.as_str().and_then(Side::parse).ok_or_else(|| bad(cmd, format!("the resolution of `{k}` is \"branch\" or \"main\"")))?;
            Ok((k.clone(), side))
        })
        .collect()
}

/// `branch.merge` (`from` the branch into main) and `branch.updateFromMain` (`from` main into
/// the branch).
fn combine(s: &mut Session, p: &Value, from: Side) -> Result<Value> {
    let cmd = if from == Side::Branch { "branch.merge" } else { "branch.updateFromMain" };
    let (mi, bi) = locate(s, cmd, string(p, "id").as_deref())?;
    let m = s.docs.get_mut(mi).ok_or(EngineError::NoDocument)?;
    let main_uid = m.uid;
    let (id, state) = m.branches.get(bi).map(|b| (b.id.clone(), b.state)).ok_or(EngineError::NoDocument)?;
    if state != BranchState::Active {
        return Err(disabled(cmd, "the branch was merged or archived"));
    }
    // The main file itself, not a version shown in its place.
    m.end_preview();
    refuse_live(s, cmd, main_uid)?;
    let picks = resolutions(cmd, p)?;
    let (base, head, main) = documents(s, mi, bi).ok_or(EngineError::NoDocument)?;
    let plan = merge::plan(&base, &head, &main).map_err(|e| EngineError::Other(e.to_string()))?;
    let pick = |k: &Key| picks.get(&k.to_string()).copied();
    let open: Vec<String> = plan.unresolved(&pick).iter().map(|c| c.key.to_string()).collect();
    if !open.is_empty() {
        return Err(disabled(cmd, format!("resolve every conflict first (\"branch\" or \"main\" in `resolutions`): {}", open.join(", "))));
    }
    let into = if from == Side::Branch { &main } else { &head };
    let (mut doc, touched) = plan.merge(into, from, &pick).map_err(|e| EngineError::Other(format!("can't merge: {e}")))?;
    doc.mark_dirty(touched);
    normalize(&mut doc, into);
    let doc = Arc::new(doc);
    let mut out = json!({"changes": plan.changes(from).len(), "conflicts": plan.conflicts.len()});
    if from == Side::Branch {
        let m = s.docs.get_mut(mi).ok_or(EngineError::NoDocument)?;
        let (selection, page) = (m.selection.clone(), m.page);
        // The main file's history keeps it as it was before the merge, then the merge itself; the
        // branch keeps its own history.
        crate::cmd::file::keep_in_history(m);
        show(m, doc);
        let name = m.branches.get(bi).map(|b| b.name.clone()).unwrap_or_default();
        crate::cmd::file::add_version(m, &format!("Merged “{name}”"), "", false);
        if let Some(b) = m.branches_mut().get_mut(bi) {
            b.state = BranchState::Merged;
        }
        s.record(main_uid, Some("Merge branch"), main, selection, page);
        close_tabs(s, main_uid, Some(&id));
        s.active = s.docs.iter().position(|d| d.uid == main_uid).unwrap_or(s.active);
        out["merged"] = json!(true);
        out["index"] = json!(s.active);
    } else {
        if let Some(t) = tab_of(s, main_uid, &id).and_then(|t| s.docs.get_mut(t)) {
            show(t, doc.clone());
            // Undo can't step back past the new base: that would undo main's changes on the branch.
            t.history.undo.clear();
            t.history.redo.clear();
        }
        if let Some(b) = s.docs.get_mut(mi).and_then(|m| m.branches_mut().get_mut(bi)) {
            b.base = main;
            b.head = doc;
        }
        out["updated"] = json!(true);
    }
    Ok(out)
}

/// Show merged `doc` in tab `ds`.
fn show(ds: &mut DocState, doc: Arc<Document>) {
    ds.doc = doc;
    ds.text_edit = None;
    ds.vector_edit = None;
    ds.clean_selection();
    if !ds.doc.pages.contains(&ds.page)
        && let Some(p) = ds.doc.first_page()
    {
        ds.page = p;
    }
    ds.revision += 1;
}

/// What changed on branch `id` (default: the branch tab shown) and on its main file, and their
/// conflicts.
pub fn review(s: &Session, id: Option<&str>) -> Result<Review> {
    let (mi, bi) = locate(s, "branch.review", id)?;
    review_at(s, mi, bi)
}

/// [`review`] of branch `bi` of the file in tab `mi`.
pub fn review_at(s: &Session, mi: usize, bi: usize) -> Result<Review> {
    let b = s.docs.get(mi).and_then(|m| m.branches.get(bi)).ok_or(EngineError::NoDocument)?;
    let (base, head, main) = documents(s, mi, bi).ok_or(EngineError::NoDocument)?;
    let plan = merge::plan(&base, &head, &main).map_err(|e| EngineError::Other(e.to_string()))?;
    Ok(Review {
        id: b.id.clone(),
        name: b.name.clone(),
        state: b.state,
        branch: side_changes(&plan, Side::Branch, &base, &head),
        main: side_changes(&plan, Side::Main, &base, &main),
        conflicts: plan.conflicts.iter().map(|c| conflict_row(&plan, c, &base, &head, &main)).collect(),
    })
}

fn side_changes(plan: &Plan, side: Side, base: &Document, doc: &Document) -> SideChanges {
    let cmp = devcompare::compare_docs(base, doc);
    let document = plan.changes(side).iter().filter_map(|c| doc_change(plan, c)).collect();
    SideChanges { layers: cmp.changes, document, truncated: cmp.truncated }
}

/// `prop`'s name in words: `childLayout` → "child layout".
fn words<T: Serialize>(prop: &T) -> String {
    let name = serde_json::to_value(prop).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// "Style “Primary”", "Variable “gap”"… (`None` for entries not worth listing: images, ready-for-dev
/// snapshots).
fn entry_label(plan: &Plan, entry: &Entry, value: Option<&Value>) -> Option<(&'static str, String)> {
    let kind = match entry {
        Entry::Style(_) => "style",
        Entry::Variable(_) => "variable",
        Entry::Collection(_) => "variable collection",
        Entry::Comment(_) => "comment",
        Entry::DevSnapshot(_) | Entry::Image(_) => return None,
    };
    let base = plan.base().value(&Target::Entry(entry.clone())).ok().flatten();
    let named = value.or(base.as_ref().map(|j| &j.0)).and_then(|v| v.get("name").or_else(|| v.get("text"))).and_then(Value::as_str);
    Some((kind, figcraft_format::clip(named.unwrap_or_default(), 60)))
}

/// What `c` did to its entry: added, changed or removed.
fn entry_change(plan: &Plan, c: &Change) -> ChangeKind {
    match c {
        Change::Put { entry, .. } if plan.base().value(&Target::Entry(entry.clone())).ok().flatten().is_none() => ChangeKind::Added,
        Change::Remove { .. } => ChangeKind::Removed,
        _ => ChangeKind::Changed,
    }
}

fn doc_change(plan: &Plan, c: &Change) -> Option<DocChange> {
    let what = match c {
        Change::SetDoc { prop, .. } => capitalized(&words(prop)),
        Change::Put { entry, value } => entry_label(plan, entry, Some(&value.0)).map(|(k, n)| format!("{} “{n}”", capitalized(k)))?,
        Change::Remove { entry } => entry_label(plan, entry, None).map(|(k, n)| format!("{} “{n}”", capitalized(k)))?,
        _ => return None,
    };
    Some(DocChange { what, change: entry_change(plan, c) })
}

fn conflict_row(plan: &Plan, c: &Conflict, base: &Document, head: &Document, main: &Document) -> ConflictRow {
    let layer = |id: NodeId| main.node(id).or(head.node(id)).or(base.node(id)).map_or_else(|| id.to_string(), |n| n.name.clone());
    let text = |side: Side| match c.changes(side).first().and_then(|i| plan.changes(side).get(*i)) {
        Some(ch) => entry_change(plan, ch).label().to_lowercase(),
        None => "unchanged".to_string(),
    };
    let (id, name, property, branch, main_text) = match &c.key {
        Key::Node(id, prop) => {
            // Main's layer, and main's layer with only the branch's change to this property.
            let with = plan.with_changes(main, Side::Branch, &c.branch);
            let changes = devcompare::layer_changes(main, &with, *id);
            let join = |f: fn(&PropChange) -> &str| changes.iter().map(|pc| format!("{} {}", pc.property, f(pc))).collect::<Vec<_>>().join(", ");
            let (b, m) = if changes.is_empty() { ("changed".into(), "changed".into()) } else { (join(|pc| &pc.after), join(|pc| &pc.before)) };
            let property = if *prop == Prop::Parent { "place in layers".to_string() } else { words(prop) };
            (Some(*id), layer(*id), property, b, m)
        }
        Key::Layer(id) => {
            let (b, m) = if c.deleted_by == Some(Side::Branch) { ("deleted", "edited") } else { ("edited", "deleted") };
            (Some(*id), layer(*id), "layer".to_string(), b.to_string(), m.to_string())
        }
        Key::Doc(prop) => (None, "File".to_string(), words(prop), "changed".to_string(), "changed".to_string()),
        Key::Entry(entry) => {
            let changes = c.branch.iter().filter_map(|i| plan.branch.get(*i)).chain(c.main.iter().filter_map(|i| plan.main.get(*i)));
            let value = changes.into_iter().find_map(|ch| match ch {
                Change::Put { value, .. } => Some(value.0.clone()),
                _ => None,
            });
            let (kind, name) = entry_label(plan, entry, value.as_ref()).unwrap_or(("entry", String::new()));
            (None, name, kind.to_string(), text(Side::Branch), text(Side::Main))
        }
    };
    ConflictRow { key: c.key.to_string(), id, name, property, branch, main: main_text, deleted_by: c.deleted_by }
}
