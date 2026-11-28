use super::{Arc, DocState, Document, EngineError, Result, Session, Value, Version, bad, get, json, req_index, string};

pub(super) fn version_json(index: usize, v: &Version) -> Value {
    let mut j = json!({"id": v.id, "index": index, "name": v.name, "description": v.description, "time": v.time});
    if v.auto {
        j["auto"] = json!(true);
    }
    j
}

/// Autosaves kept per file; older ones make room for new ones.
pub const MAX_AUTOSAVES: usize = 20;

/// `version.autosave`: a checkpoint of a document with unsaved changes since the last version.
pub(super) fn autosave(s: &mut Session, _: &Value) -> Result<Value> {
    let auto_save = s.prefs.auto_save;
    let ds = s.ds_mut()?;
    // A file that saves itself is rarely unsaved: edits since it was opened count then.
    let edited = ds.is_dirty() || (auto_save && !ds.history.undo.is_empty());
    if ds.preview.is_some() || !edited || !ds.changed_since_checkpoint() {
        return Ok(json!({"saved": false}));
    }
    Ok(match add_version(ds, AUTOSAVE, "", true) {
        Some(v) => json!({"saved": true, "version": v}),
        None => json!({"saved": false}),
    })
}

/// What checkpoints are called.
const AUTOSAVE: &str = "Autosave";

/// Add a version of `ds`'s document as it is now (`auto`: a checkpoint) and return it. The oldest
/// autosave makes room when the autosaves or the history are full; versions the user named are
/// never dropped, so a history full of them takes nothing (None).
pub(crate) fn add_version(ds: &mut DocState, name: &str, description: &str, auto: bool) -> Option<Value> {
    let autos = ds.versions.iter().filter(|v| v.auto).count();
    if ((auto && autos >= MAX_AUTOSAVES) || ds.versions.len() >= figcraft_format::MAX_VERSIONS)
        && let Some(oldest) = ds.versions.iter().position(|v| v.auto)
    {
        ds.versions_mut().remove(oldest);
    }
    if ds.versions.len() >= figcraft_format::MAX_VERSIONS {
        return None;
    }
    let mut v = snapshot_version(ds, name, description);
    v.auto = auto;
    let out = version_json(ds.versions.len(), &v);
    ds.versions_mut().push(v);
    ds.mark_checkpoint();
    Some(out)
}

/// Before the document is replaced (a restore, a merge): keep it in the history as it is, unless
/// it is what the last version saved or restored holds.
pub(crate) fn keep_in_history(ds: &mut DocState) {
    if ds.branch_of.is_none() && ds.changed_since_checkpoint() {
        add_version(ds, AUTOSAVE, "", true);
    }
}

/// A version of `ds`'s document as it is now, opening on the page shown.
fn snapshot_version(ds: &DocState, name: &str, description: &str) -> Version {
    let mut snap = ds.doc.clone();
    if snap.current_page != Some(ds.page) {
        Arc::make_mut(&mut snap).current_page = Some(ds.page);
    }
    Version::new(unique_id(|id| ds.versions.iter().any(|v| v.id == id)), name, description, crate::cmd::comment::now_secs(), snap)
}

pub(super) fn version_list(ds: &DocState) -> Value {
    let previewing = ds.preview.as_ref().map(|p| p.id.as_str());
    Value::Array(
        ds.versions
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mut j = version_json(i, v);
                if previewing == Some(v.id.as_str()) {
                    j["previewing"] = json!(true);
                }
                j
            })
            .collect(),
    )
}

/// The version named by `id`, else by `index` (validated: never negative, never past the end).
pub(super) fn version_index(cmd: &str, ds: &DocState, p: &Value) -> Result<usize> {
    if let Some(id) = string(p, "id") {
        return ds.versions.iter().position(|v| v.id == id).ok_or_else(|| bad(cmd, format!("no version `{id}`")));
    }
    if get(p, "index").is_none() {
        return Err(bad(cmd, "pass the version's `id` or `index`"));
    }
    let i = req_index(cmd, p, "index")?;
    if i < ds.versions.len() { Ok(i) } else { Err(bad(cmd, format!("no version at index {i}"))) }
}

pub(crate) fn read_version(v: &Version) -> Result<Arc<Document>> {
    v.document().map_err(|e| EngineError::Other(format!("version “{}” can't be read: {e}", v.name)))
}

/// A new version or branch id: the time in milliseconds (hex), not `taken`.
pub(crate) fn unique_id(taken: impl Fn(&str) -> bool) -> String {
    let mut n = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    loop {
        let id = format!("{n:x}");
        if !taken(&id) {
            return id;
        }
        n = n.wrapping_add(1);
    }
}

/// `file.saveVersion`; on a branch tab the version goes into the branch's own history.
pub(super) fn save_version(s: &mut Session, p: &Value) -> Result<Value> {
    let ds = s.ds_mut()?;
    if ds.versions.len() >= figcraft_format::MAX_VERSIONS {
        return Err(EngineError::Disabled(
            "file.saveVersion".into(),
            format!("the version history is full ({} versions): delete old versions first", figcraft_format::MAX_VERSIONS),
        ));
    }
    let name =
        string(p, "name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Version {}", ds.versions.len() + 1));
    let description = string(p, "description").unwrap_or_default();
    // The version opens on the page shown when it was saved.
    add_version(ds, &name, description.trim(), false).ok_or_else(|| {
        EngineError::Disabled(
            "file.saveVersion".into(),
            format!("the version history is full ({} versions): delete old versions first", figcraft_format::MAX_VERSIONS),
        )
    })
}

pub(super) fn restore_version(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "version.restore";
    // Refused before anything goes into the history.
    if let Some(why) = s.live_read_only(s.ds()?.uid) {
        return Err(EngineError::Disabled(CMD.into(), why.into()));
    }
    let ds = s.ds_mut()?;
    let i = version_index(CMD, ds, p)?;
    let v = ds.versions.get(i).ok_or_else(|| bad(CMD, "no such version"))?;
    let (id, mut doc) = (v.id.clone(), read_version(v)?);
    // Restoring never loses work: the document as it is goes into the history first.
    keep_in_history(ds);
    let out = ds.versions.iter().enumerate().find(|(_, v)| v.id == id).map_or(Value::Null, |(i, v)| version_json(i, v));
    let d = Arc::make_mut(&mut doc);
    d.name = ds.doc.name.clone();
    // What the file published belongs to the file, not to the version.
    d.published = ds.doc.published.clone();
    // Comments are a conversation about the file, kept apart from its design history: they stay,
    // and those on layers the version doesn't have stay where they were.
    d.comments = ds.doc.comments.clone();
    crate::cmd::comment::follow_anchors(d, &ds.doc);
    // The snapshot is already consistent: nothing to re-normalise.
    d.take_dirty();
    // Stay on the current page when the version has it.
    ds.page = Some(ds.page)
        .filter(|pg| d.pages.contains(pg))
        .or_else(|| d.current_page.filter(|pg| d.pages.contains(pg)))
        .or_else(|| d.first_page())
        .unwrap_or(ds.page);
    ds.doc = doc;
    ds.selection.clear();
    ds.text_edit = None;
    ds.vector_edit = None;
    // The restored design is in the history already: restoring it again keeps nothing new.
    ds.mark_checkpoint();
    Ok(out)
}
