//! File menu: documents, save/open, import/export, versions.

use std::sync::Arc;

use base64::Engine as _;
use figcraft_doc::*;
use figcraft_format::Version;
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::*;
use crate::{DocState, EngineError, Result, Session, bad, spec};

/// A new file of `kind` in a tab of its own.
fn new_file(s: &mut Session, p: &Value, kind: FileKind) -> Result<Value> {
    let name = string(p, "name").unwrap_or_else(|| if kind == FileKind::Board { "Untitled board" } else { "Untitled" }.into());
    let mut doc = Document::new(name);
    doc.kind = kind;
    // A property of the page from here on: a later change of theme leaves it alone. Boards are
    // light whatever the theme.
    let background = if kind == FileKind::Board { BOARD_PAGE } else { s.new_page_background() };
    for page in doc.pages.clone() {
        if let Some(NodeKind::Page { background: b, .. }) = doc.node_mut(page).map(|n| &mut n.kind) {
            *b = background;
        }
    }
    s.docs.push(DocState::new(doc, None));
    s.active = s.docs.len() - 1;
    Ok(json!({"index": s.active, "kind": kind.id()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("file.new", "New design file", "{name?}", false, |s, p| new_file(s, p, FileKind::Design)),
        spec!(
            "file.newBoard",
            "New board",
            "{name?}  a board: sticky notes, shapes with text, connectors, marker and stamps on a light page",
            false,
            |s, p| new_file(s, p, FileKind::Board)
        ),
        spec!(
            "file.open",
            "Open",
            "{path | data (base64) + name (tells the format; opens as a new unsaved document), variables? (REST: the local variables JSON as an object, JSON text or a path; default `<name>.variables.json` or `variables.json` next to the file)} (.figcraft, .json: FigCraft or REST API file JSON, .fig (best effort), .svg, .png, .jpg …); REST and .fig imports return `warnings`; a file opened anew returns `summary` {name, pages, layers, images} (one that was already open just returns its `index`)",
            false,
            open
        ),
        spec!(
            "file.openRecent",
            "Open recent",
            "{path? | index? (into the list), clear? (forget them all), remove? (a path: forget that one)}  opens a recent file; a file that no longer exists is dropped from the list. Returns the list, newest first: {files: [{path, name, exists}]} (opening returns what `file.open` does)",
            false,
            open_recent
        ),
        spec!(
            "file.save",
            "Save",
            "{path?} (programmatic saves need a path the first time); a .fig path writes an experimental export copy of the shown document with `warnings`, keeping the native path, unsaved changes and version preview",
            false,
            save
        ),
        spec!(
            "file.saveAs",
            "Save as",
            "{path} (no extension adds .figcraft); a .fig path writes an experimental export copy of the shown document with `warnings`, keeping the native path, unsaved changes and version preview",
            false,
            save_as
        ),
        spec!(
            "file.export",
            "Export",
            "{ids? (default selection, else the page), format? (png|jpg|svg|pdf|webp, or fig: the whole file as .fig, experimental, with `warnings` for what was left out; other apps may not open every file), scale? (default 1) | size? (2x|512w|512h or {by: scale|width|height, value}: a scale, or a width or height in pixels), path? (else returns base64 `data`), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options)}",
            false,
            export
        ),
        spec!(
            "file.exportLayers",
            "Export layers",
            "{ids? (default the selection, else every layer on the page with export settings), assets? (true: the Inspect tab's assets in and under them instead: layers with export settings, icons as SVG, images as PNG), dir? (folder to write into; else each file comes back as base64 `data`), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options): for every PDF instead of each setting's own}  every layer once per export setting, named `<layer><suffix>.<format>` (\"@2x\" when a scaled export has no suffix; repeats get \" 2\"; slashes in a layer name make folders). Returns {files: [{name, id, bytes, path | data}], skipped (layers without settings)}",
            false,
            export_layers
        ),
        spec!("file.exportSvg", "Export selection as SVG", "{ids?, path?}", false, |s, p| {
            let mut q = p.clone();
            if let Some(o) = q.as_object_mut() {
                o.insert("format".into(), json!("svg"));
            }
            export(s, &q)
        }),
        spec!(
            "file.exportFramesPdf",
            "Export frames to PDF",
            "{path?, page? (page id, default the current page), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options)}  every top-level frame in reading order, one PDF page each; returns base64 `data` without a path",
            false,
            export_frames_pdf
        ),
        spec!(
            "file.import",
            "Import",
            "{path | data (base64) + name, x?, y?, center? (SVG: centre it on x, y), variables? (REST: the local variables JSON as an object, JSON text or a path)}  SVG becomes an editable frame (shapes, text, gradients, masks, effects; with `warnings` for what it left out), images become image layers, .figcraft, REST API file JSON and .fig pages are appended with their variables and flows (REST and .fig: with `warnings`; those return `summary` {name, pages, layers, images} of the file read)",
            true,
            import
        ),
        spec!("file.placeImage", "Place image", "{path | data, x?, y?}", true, super::create::place_image),
        spec!("file.close", "Close tab", "{index?, force? (discard unsaved changes)}", false, close),
        spec!("file.moveTab", "Move tab", "{from, to} (indices among open documents)", false, |s, p| {
            let from = req_index("file.moveTab", p, "from")?;
            let to = req_index("file.moveTab", p, "to")?;
            if from >= s.docs.len() || to >= s.docs.len() {
                return Err(bad("file.moveTab", "no such document"));
            }
            if from != to {
                let active_uid = s.docs.get(s.active).map(|d| d.uid);
                let doc = s.docs.remove(from);
                s.docs.insert(to, doc);
                s.active = active_uid.and_then(|uid| s.docs.iter().position(|d| d.uid == uid)).unwrap_or(0);
            }
            Ok(json!({"index": to}))
        }),
        spec!(
            "file.recover",
            "Recover file",
            "{path}  open a recovery copy (written while the app runs, read back after a crash) as a document with unsaved changes and no path: saving asks where → {index}",
            false,
            |s, p| {
                let path = req_string("file.recover", p, "path")?;
                s.docs.push(recovered(&path)?);
                s.active = s.docs.len() - 1;
                Ok(json!({"index": s.active}))
            }
        ),
        spec!("file.rename", "Rename file", "{name}", true, |s, p| {
            let name = req_string("file.rename", p, "name")?;
            s.doc_mut()?.name = name;
            ok()
        }),
        spec!(
            "file.colorProfile",
            "Color profile",
            "{profile?: srgb|displayP3, convert? (default true: convert colour values so colours keep their appearance; false: keep the values and reinterpret them)}  without `profile`, returns the current one (and the app opens the dialog). Returns {profile, changed}",
            true,
            color_profile
        ),
        spec!("file.duplicate", "Duplicate file", "{}", false, |s, _| {
            let ds = s.ds()?;
            let mut d = (*ds.doc).clone();
            d.name = format!("{} (Copy)", d.name);
            s.docs.push(DocState::new(d, None));
            s.active = s.docs.len() - 1;
            Ok(json!({"index": s.active}))
        }),
        spec!("file.revert", "Revert to saved", "{}  leave the live session first, if any", false, |s, _| {
            let path = s.ds()?.path.clone().ok_or_else(|| EngineError::Disabled("file.revert".into(), "the file was never saved".into()))?;
            let (loaded, _) = load_path(&path, None)?;
            let i = s.active;
            let uid = s.ds()?.uid;
            if let Some(slot) = s.docs.get_mut(i) {
                *slot = DocState::new(loaded.doc, Some(path)).with_versions(loaded.versions).with_branches(loaded.branches);
            }
            // Edits on its branch tabs are reverted too.
            crate::branches::close_tabs(s, uid, None);
            ok()
        }),
        spec!(
            "file.saveVersion",
            "Save to version history",
            "{name?, description?}  snapshot the document into the file's version history (on a branch tab: the branch's own history; kept when the file is saved); returns the version",
            false,
            save_version
        ),
        spec!("file.versionHistory", "Show version history", "{}  opens the panel; returns the versions like `version.list`", false, |s, _| {
            let list = version_list(s.ds()?);
            s.request_ui("panel.versionHistory", json!({}));
            Ok(list)
        }),
        spec!(
            "version.list",
            "List versions",
            "{}  the version history, oldest first: [{id, index, name, description, time (Unix seconds), auto?, previewing?}]",
            false,
            |s, _| Ok(version_list(s.ds()?))
        ),
        spec!(
            "version.restore",
            "Restore version",
            "{id | index}  replace the design with the version (one undo step; the file name and the comments stay; the design as it was is kept in the history first, unless a version of it was just saved or restored)",
            true,
            restore_version
        ),
        spec!("version.rename", "Rename version", "{id | index, name, description?}  a titled autosave becomes a named version", false, |s, p| {
            const CMD: &str = "version.rename";
            let name = req_string(CMD, p, "name")?;
            if name.trim().is_empty() {
                return Err(bad(CMD, "`name` can't be empty"));
            }
            let ds = s.ds_mut()?;
            let i = version_index(CMD, ds, p)?;
            let description = string(p, "description");
            let v = ds.versions_mut().get_mut(i).ok_or_else(|| bad(CMD, "no such version"))?;
            v.name = figcraft_format::clip(name.trim(), figcraft_format::MAX_VERSION_NAME);
            // A titled autosave is a named version (never dropped to make room).
            v.auto = false;
            if let Some(d) = description {
                v.description = figcraft_format::clip(d.trim(), figcraft_format::MAX_VERSION_DESCRIPTION);
            }
            Ok(version_json(i, v))
        }),
        spec!("version.delete", "Delete version", "{id | index}", false, |s, p| {
            let ds = s.ds_mut()?;
            let i = version_index("version.delete", ds, p)?;
            let v = ds.versions_mut().remove(i);
            if ds.preview.as_ref().is_some_and(|pv| pv.id == v.id) {
                ds.end_preview();
            }
            ok()
        }),
        spec!(
            "version.preview",
            "View version",
            "{id | index}  show the version on the canvas in place of the document (read-only; `version.restore` keeps it, `version.exitPreview` goes back)",
            false,
            |s, p| {
                let ds = s.ds_mut()?;
                let i = version_index("version.preview", ds, p)?;
                let v = ds.versions.get(i).ok_or_else(|| bad("version.preview", "no such version"))?;
                let (id, name, time, out) = (v.id.clone(), v.name.clone(), v.time, version_json(i, v));
                let doc = read_version(v)?;
                ds.start_preview(&id, &name, time, doc);
                Ok(out)
            }
        ),
        spec!(
            "version.autosave",
            "Autosave version",
            "{}  save an automatic checkpoint when the document has unsaved changes since the last version (with `pref.autoSave` on: changes made since it was opened); the app does this every 30 minutes; keeps the newest 20 autosaves → {saved, version?}",
            false,
            autosave
        ),
        spec!("version.exitPreview", "Back to current version", "{}  end a version preview", false, |s, _| {
            Ok(json!({"ended": s.ds_mut()?.end_preview()}))
        }),
        spec!(
            "version.duplicate",
            "Duplicate version",
            "{id | index}  open the version as a new document; returns its tab `index`",
            false,
            |s, p| {
                let ds = s.ds()?;
                let v = ds.versions.get(version_index("version.duplicate", ds, p)?).ok_or_else(|| bad("version.duplicate", "no such version"))?;
                let mut doc = (*read_version(v)?).clone();
                doc.name = format!("{} ({})", ds.doc.name, v.name);
                doc.take_dirty();
                s.docs.push(DocState::new(doc, None));
                s.active = s.docs.len() - 1;
                Ok(json!({"index": s.active}))
            }
        ),
        spec!("doc.switch", "Switch document", "{index}", false, |s, p| {
            let i = req_index("doc.switch", p, "index")?;
            if i >= s.docs.len() {
                return Err(bad("doc.switch", "no such document"));
            }
            s.active = i;
            ok()
        }),
        spec!("doc.nextTab", "Next tab", "{}", false, |s, _| cycle_doc(s, 1)),
        spec!("doc.previousTab", "Previous tab", "{}", false, |s, _| cycle_doc(s, -1)),
        spec!("doc.list", "List documents", "{}", false, |s, _| {
            Ok(Value::Array(
                s.docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| json!({"index": i, "name": d.doc.name, "path": d.path, "dirty": d.is_dirty(), "active": i == s.active}))
                    .collect(),
            ))
        }),
        spec!("doc.home", "Home", "{}", false, |s, _| {
            s.request_ui("home", json!({}));
            ok()
        }),
    ]
}

fn version_json(index: usize, v: &Version) -> Value {
    let mut j = json!({"id": v.id, "index": index, "name": v.name, "description": v.description, "time": v.time});
    if v.auto {
        j["auto"] = json!(true);
    }
    j
}

/// Autosaves kept per file; older ones make room for new ones.
pub const MAX_AUTOSAVES: usize = 20;

/// `version.autosave`: a checkpoint of a document with unsaved changes since the last version.
fn autosave(s: &mut Session, _: &Value) -> Result<Value> {
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
    Version::new(unique_id(|id| ds.versions.iter().any(|v| v.id == id)), name, description, super::comment::now_secs(), snap)
}

fn version_list(ds: &DocState) -> Value {
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
fn version_index(cmd: &str, ds: &DocState, p: &Value) -> Result<usize> {
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
fn save_version(s: &mut Session, p: &Value) -> Result<Value> {
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

fn restore_version(s: &mut Session, p: &Value) -> Result<Value> {
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
    super::comment::follow_anchors(d, &ds.doc);
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

/// The recovery copy at `path` as a document with unsaved changes and no path.
pub(crate) fn recovered(path: &str) -> Result<DocState> {
    let loaded = read(path).and_then(|b| figcraft_format::load_bytes_with_versions(&b).map_err(|e| EngineError::Other(e.to_string())))?;
    Ok(DocState::new(loaded.doc, None).with_versions(loaded.versions).with_branches(loaded.branches).unsaved())
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> Result<Vec<u8>> {
    crate::file_access::read(path)
}

#[cfg(target_arch = "wasm32")]
fn read(path: &str) -> Result<Vec<u8>> {
    Err(EngineError::Other(format!("{path}: no file system in the browser")))
}

/// The bytes of file `path`, refused when it's larger than `max`.
pub(crate) fn read_capped(path: &str, max: u64) -> Result<Vec<u8>> {
    crate::file_access::read_capped(path, max)
}

/// Write `bytes` to `path` through a uniquely named, exclusively created staging file and one
/// atomic rename (see [`figcraft_format::write_file_atomic`]): a failed or interrupted write never
/// truncates `path`, and no file or symlink at a staging name is ever written through.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write(path: &str, bytes: &[u8]) -> Result<()> {
    crate::file_access::write(path, bytes)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write(path: &str, _bytes: &[u8]) -> Result<()> {
    Err(EngineError::Other(format!("{path}: no file system in the browser")))
}

fn ext(path: &str) -> String {
    std::path::Path::new(path).extension().and_then(std::ffi::OsStr::to_str).unwrap_or("").to_ascii_lowercase()
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Untitled").to_string()
}

/// The counts the import report shows for a freshly loaded document.
fn import_summary(doc: &Document) -> Value {
    json!({"name": doc.name, "pages": doc.pages.len(), "layers": doc.nodes().filter(|n| !n.is_page()).count(), "images": doc.images.len()})
}

/// Load a document from any supported file (REST API file JSON images come from an `images`
/// folder next to the file).
pub fn load(path: &str) -> Result<(Document, Vec<String>)> {
    load_path(path, None).map(|(l, _)| (l.doc, l.issues))
}

pub fn load_bytes(bytes: &[u8], name_hint: &str) -> Result<(Document, Vec<String>)> {
    load_with(bytes, name_hint, None, &figcraft_format::rest::no_images).map(|(l, _)| (l.doc, l.issues))
}

/// [`load`] with the version history, also telling whether the file was REST API file JSON. A REST
/// import reads its variables from `variables`, else from a variables JSON next to the file.
pub fn load_path(path: &str, variables: Option<&Value>) -> Result<(figcraft_format::Loaded, bool)> {
    let bytes = read(path)?;
    let sibling = if variables.is_none() { sibling_variables(path) } else { None };
    load_with(&bytes, path, variables.or(sibling.as_ref()), &rest_images(path))
}

/// Load from bytes. Returns the document with its notes (repairs of a native file, or import
/// warnings) and version history, and whether it was imported from REST API file JSON or a `.fig`
/// file (such a file is never saved over). `variables`: the local variables JSON for a REST API file JSON.
fn load_with(
    bytes: &[u8],
    name_hint: &str,
    variables: Option<&Value>,
    images: figcraft_format::rest::ImageResolver,
) -> Result<(figcraft_format::Loaded, bool)> {
    match ext(name_hint).as_str() {
        "svg" => {
            let mut doc = Document::new(file_stem(name_hint));
            let page = doc.first_page().ok_or(EngineError::NoDocument)?;
            let notes = figcraft_svg::import_svg_with_notes(&mut doc, bytes, page, 0.0, 0.0, &file_stem(name_hint))
                .map_err(|e| EngineError::Other(e.to_string()))?
                .notes;
            Ok((figcraft_format::Loaded { issues: notes, ..doc.into() }, false))
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" => {
            let mut doc = Document::new(file_stem(name_hint));
            let page = doc.first_page().ok_or(EngineError::NoDocument)?;
            let (hash, w, h) = figcraft_format::add_image(&mut doc, bytes.to_vec()).map_err(|e| EngineError::Other(e.to_string()))?;
            let id = doc.create(page, NodeKind::Rectangle, &file_stem(name_hint), 0.0, 0.0, f64::from(w), f64::from(h))?;
            doc.get_mut(id)?.fills = vec![figcraft_color::Paint::image(hash, figcraft_color::ImageScaleMode::Fill)];
            Ok((doc.into(), false))
        }
        _ => {
            if let Some(r) = load_fig(bytes, &file_stem(name_hint)) {
                return Ok((r?, true));
            }
            if let Some(r) = figcraft_format::rest::try_import_with(bytes, variables, images) {
                let (mut doc, warnings) = r.map_err(|e| EngineError::Other(e.to_string()))?;
                link_instances(&mut doc);
                return Ok((figcraft_format::Loaded { issues: warnings, ..doc.into() }, true));
            }
            let mut loaded = figcraft_format::load_bytes_with_versions(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
            // Older files saved an untouched default title even after the user chose a filename.
            if loaded.doc.name.is_empty() || loaded.doc.name == "Untitled" {
                loaded.doc.name = file_stem(name_hint);
            }
            Ok((loaded, false))
        }
    }
}

/// Gives `None` when `bytes` are not a `.fig` file. Then other readers can try.
pub(crate) fn load_fig(bytes: &[u8], name: &str) -> Option<Result<figcraft_format::Loaded>> {
    let r = figcraft_format::fig::try_import(bytes, name)?;
    Some(r.map_err(|e| EngineError::Other(e.to_string())).map(|(mut doc, warnings)| {
        link_instances(&mut doc);
        figcraft_format::Loaded { issues: warnings, ..doc.into() }
    }))
}

/// Link imported instances to their mains (layer sources, overrides found by comparing values),
/// innermost first so outer instances settle last.
fn link_instances(doc: &mut Document) {
    super::styles::unfold_imported_bound_alpha(doc);
    crate::style_vars::after_load(doc);
    super::styles::apply_paint_bindings_on_load(doc);
    let mut insts: Vec<(usize, NodeId)> =
        doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })).map(|n| (doc.ancestors(n.id).len(), n.id)).collect();
    insts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, i) in insts {
        crate::components::rebase_instance(doc, i);
    }
}

/// Largest image file read for a REST API file JSON import.
#[cfg(not(target_arch = "wasm32"))]
const MAX_IMAGE_FILE: u64 = 256 * 1024 * 1024;

/// Image bytes for a REST API file JSON's `imageRef`: `images/<imageRef>[.png|.jpg|…]` next to
/// the file.
#[cfg(not(target_arch = "wasm32"))]
fn rest_images(path: &str) -> impl Fn(&str) -> Option<Vec<u8>> {
    let dir = std::path::Path::new(path).parent().map(|d| d.join("images"));
    move |r: &str| {
        // Image refs are content hashes; anything else never becomes a path.
        if r.is_empty() || r.len() > 128 || !r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return None;
        }
        let dir = dir.as_ref()?;
        ["", ".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .find_map(|e| crate::file_access::read_capped(dir.join(format!("{r}{e}")), MAX_IMAGE_FILE).ok())
    }
}

#[cfg(target_arch = "wasm32")]
fn rest_images(_path: &str) -> impl Fn(&str) -> Option<Vec<u8>> {
    figcraft_format::rest::no_images
}

/// Largest variables JSON read from disk.
#[cfg(not(target_arch = "wasm32"))]
const MAX_VARIABLES_FILE: u64 = 256 * 1024 * 1024;

/// A local variables JSON next to a REST API file JSON: `<name>.variables.json`, else
/// `variables.json`.
#[cfg(not(target_arch = "wasm32"))]
fn sibling_variables(path: &str) -> Option<Value> {
    if ext(path) != "json" {
        return None;
    }
    let file = std::path::Path::new(path);
    let dir = file.parent()?;
    [format!("{}.variables.json", file_stem(path)), "variables.json".to_string()].iter().map(|n| dir.join(n)).filter(|c| c != file).find_map(|c| {
        let v: Value = serde_json::from_slice(&crate::file_access::read_capped(c, MAX_VARIABLES_FILE).ok()?).ok()?;
        figcraft_format::rest::is_variables_json(&v).then_some(v)
    })
}

#[cfg(target_arch = "wasm32")]
fn sibling_variables(_path: &str) -> Option<Value> {
    None
}

/// The local variables JSON a command passes as `variables`: an object, JSON text or a file path.
pub(crate) fn variables_param(cmd: &str, p: &Value) -> Result<Option<Value>> {
    let parse = |text: &[u8]| serde_json::from_slice::<Value>(text).map_err(|e| bad(cmd, format!("variables: {e}")));
    let v = match get(p, "variables") {
        None | Some(Value::Null) => return Ok(None),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::String(s)) if s.trim_start().starts_with('{') => parse(s.as_bytes())?,
        Some(Value::String(path)) => parse(&read(path)?)?,
        Some(_) => return Err(bad(cmd, "variables: the local variables JSON (an object, JSON text or a path)")),
    };
    if !figcraft_format::rest::is_variables_json(&v) {
        return Err(bad(cmd, "variables: expected the local variables JSON (`meta.variableCollections` and `meta.variables`)"));
    }
    Ok(Some(v))
}

/// How many files File ▸ Open recent keeps.
pub const MAX_RECENT: usize = 20;

/// Put `path` at the top of the recent files.
fn remember(s: &mut Session, path: &str) {
    let r = &mut s.prefs.recent_files;
    r.retain(|p| p != path);
    r.insert(0, path.to_string());
    r.truncate(MAX_RECENT);
}

fn open_recent(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::file_access::{Access, allow, allowed};
    const CMD: &str = "file.openRecent";
    // Confined to automation roots, the recent files outside them are never listed, opened, removed
    // or named: `index` counts the files listed.
    let listed = |r: &String| allowed(r, Access::Read);
    if p.get("clear").and_then(Value::as_bool) == Some(true) {
        s.prefs.recent_files.retain(|r| !listed(r));
    }
    if let Some(gone) = string(p, "remove") {
        s.prefs.recent_files.retain(|r| *r != gone || !listed(r));
    }
    let path = match (string(p, "path"), num(p, "index")) {
        (Some(path), _) => {
            allow(&path, Access::Read)?;
            Some(path)
        }
        (None, Some(i)) => {
            Some(s.prefs.recent_files.iter().filter(|r| listed(r)).nth(i.max(0.0) as usize).cloned().ok_or_else(|| bad(CMD, "no such recent file"))?)
        }
        _ => None,
    };
    if let Some(path) = path {
        if !crate::file_access::exists(&path) {
            s.prefs.recent_files.retain(|r| *r != path);
            return Err(bad(CMD, format!("{path} no longer exists")));
        }
        return open(s, &json!({"path": path}));
    }
    let files: Vec<Value> = s
        .prefs
        .recent_files
        .iter()
        .filter(|r| listed(r))
        .map(|r| {
            let name = std::path::Path::new(r).file_name().map_or_else(|| r.clone(), |f| f.to_string_lossy().into_owned());
            json!({"path": r, "name": name, "exists": crate::file_access::exists(r)})
        })
        .collect();
    Ok(json!({"files": files}))
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let variables = variables_param("file.open", p)?;
    if string(p, "path").is_none()
        && let Some(d) = string(p, "data")
    {
        return open_data(s, &d, p, variables.as_ref());
    }
    let path = req_string("file.open", p, "path")?;
    // Already open: switch to it.
    if let Some(i) = s.docs.iter().position(|d| d.path.as_deref() == Some(path.as_str())) {
        s.active = i;
        remember(s, &path);
        return Ok(json!({"index": i}));
    }
    let (loaded, rest) = load_path(&path, variables.as_ref())?;
    remember(s, &path);
    let native = !rest && matches!(ext(&path).as_str(), "figcraft" | "json");
    let warnings = rest || ext(&path) == "svg";
    // A native file that lost something (a damaged image, say): the app says so, not only the result.
    if native && !loaded.issues.is_empty() {
        s.message = Some(match loaded.issues.as_slice() {
            [one] => format!("{}: {one}", file_stem(&path)),
            more => format!("{}: {} problems were repaired or left out while opening it, e.g. {}", file_stem(&path), more.len(), more[0]),
        });
    }
    let summary = import_summary(&loaded.doc);
    s.docs.push(DocState::new(loaded.doc, native.then_some(path)).with_versions(loaded.versions).with_branches(loaded.branches));
    s.active = s.docs.len() - 1;
    let mut out = json!({"index": s.active, "summary": summary});
    out[if warnings { "warnings" } else { "repairs" }] = json!(loaded.issues);
    Ok(out)
}

/// [`open`] for a file given as base64 `data` (a file dropped without a path on disk): `name` tells
/// the format. It opens as a new document with no path, so the first save asks where to put it.
fn open_data(s: &mut Session, data: &str, p: &Value, variables: Option<&Value>) -> Result<Value> {
    let name = req_string("file.open", p, "name")?;
    let data = data.split_once(',').map_or(data, |(_, b)| b);
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| bad("file.open", e.to_string()))?;
    let (loaded, rest) = load_with(&bytes, &name, variables, &figcraft_format::rest::no_images)?;
    let warnings = rest || ext(&name) == "svg";
    let summary = import_summary(&loaded.doc);
    s.docs.push(DocState::new(loaded.doc, None).with_versions(loaded.versions).with_branches(loaded.branches));
    s.active = s.docs.len() - 1;
    let mut out = json!({"index": s.active, "summary": summary});
    out[if warnings { "warnings" } else { "repairs" }] = json!(loaded.issues);
    Ok(out)
}

/// Thumbnail PNG of the first page's content (for file browsers / the home screen).
pub fn thumbnail(doc: &Document) -> Option<Vec<u8>> {
    let page = doc.first_page()?;
    let b = doc.page_content_bounds(page)?;
    let scale = (400.0 / b.width().max(b.height()).max(1.0)).min(2.0);
    let view = figcraft_render::View {
        origin: (b.x0, b.y0),
        scale,
        width: (b.width() * scale).ceil().max(1.0) as u32,
        height: (b.height() * scale).ceil().max(1.0) as u32,
    };
    let pix = figcraft_render::render_view(doc, page, &view, &Default::default())?;
    figcraft_render::encode_png(&pix).ok()
}

/// The `.figcraft` file for a document: content, thumbnail and version history.
pub fn file_bytes(ds: &DocState) -> Result<Vec<u8>> {
    FileSnapshot::of(ds).encode()
}

/// A document's file as it is at one moment, so the slow part of saving it (the thumbnail and the
/// encoding) can run on another thread while editing goes on: [`encode`](Self::encode) there, then
/// [`save`](Self::save) the bytes back on the session's thread.
pub struct FileSnapshot {
    uid: u64,
    path: Option<String>,
    revision: u64,
    doc: Arc<Document>,
    /// The document as last saved when the snapshot was taken: another save since makes it stale.
    saved: Arc<Document>,
    page: NodeId,
    versions: Vec<Version>,
    branches: Vec<figcraft_format::Branch>,
}

impl FileSnapshot {
    pub fn of(ds: &DocState) -> Self {
        Self {
            uid: ds.uid,
            path: ds.path.clone(),
            revision: ds.revision,
            doc: ds.doc.clone(),
            saved: ds.saved_doc.clone(),
            page: ds.page,
            versions: ds.versions.clone(),
            branches: ds.branches.clone(),
        }
    }

    /// The file's bytes: content, thumbnail and version history.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut doc = (*self.doc).clone();
        doc.current_page = Some(self.page);
        let thumb = thumbnail(&doc);
        figcraft_format::save_bytes_with_history(&doc, thumb.as_deref(), &self.versions, &self.branches)
            .map_err(|e| EngineError::Other(e.to_string()))
    }

    /// Write `bytes` (this snapshot's [`encode`](Self::encode)) to the document's file: it is saved
    /// as of the snapshot, and edits since stay unsaved. False (nothing written) when the document
    /// was closed, saved, reverted or given another path since: the bytes are out of date.
    pub fn save(self, s: &mut Session, bytes: &[u8]) -> Result<bool> {
        let Some(ds) = s.docs.iter_mut().find(|d| d.uid == self.uid && d.path == self.path && Arc::ptr_eq(&d.saved_doc, &self.saved)) else {
            return Ok(false);
        };
        write(self.path.as_deref().ok_or(EngineError::NoDocument)?, bytes)?;
        // The history or branches may have changed since: then they still count as unsaved.
        if ds.revision == self.revision {
            ds.versions_changed = false;
        }
        ds.saved_doc = self.doc;
        ds.revision += 1;
        branch_tabs_saved(s, self.uid);
        Ok(true)
    }
}

/// The branch tabs of the file in tab `uid` were saved with it.
fn branch_tabs_saved(s: &mut Session, uid: u64) {
    for tab in s.docs.iter_mut().filter(|d| d.branch_of.as_ref().is_some_and(|(m, _)| *m == uid)) {
        tab.mark_saved();
    }
}

fn color_profile(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.colorProfile";
    let Some(name) = string(p, "profile") else {
        s.request_ui("dialog.colorProfile", json!({}));
        return Ok(json!({"profile": s.doc()?.color_profile.id(), "changed": false}));
    };
    let profile = figcraft_color::ColorProfile::parse(&name).ok_or_else(|| bad(cmd, "profile: srgb or displayP3"))?;
    let convert = boolean(p, "convert").unwrap_or(true);
    let changed = s.doc()?.color_profile != profile && s.doc_mut()?.set_color_profile(profile, convert);
    Ok(json!({"profile": profile.id(), "changed": changed}))
}

/// Native saves act on the current document; a `.fig` copy exports the version shown, and a
/// pending Save As choice must leave it shown until its destination is known.
pub(crate) fn save_ends_preview(s: &Session, cmd: &str, p: &Value) -> bool {
    let path =
        string(p, "path").or_else(|| if cmd == "file.save" { s.docs.get(crate::branches::file_tab(s)).and_then(|ds| ds.path.clone()) } else { None });
    path.is_some_and(|path| ext(&path) != "fig")
}

/// Save the file of the active tab (the main file for a branch tab, with its branches), or export
/// the active document as an experimental `.fig` copy without changing its native save state.
fn save_to(s: &mut Session, path: String) -> Result<Value> {
    if ext(&path) == "fig" {
        return export_fig(s, &json!({"path": path}));
    }
    let i = crate::branches::file_tab(s);
    let ds = s.docs.get_mut(i).ok_or(EngineError::NoDocument)?;
    let mut snapshot = FileSnapshot::of(ds);
    let follows_path = ds.path.as_deref().map_or(ds.doc.name == "Untitled", |old| ds.doc.name == file_stem(old));
    if follows_path && ds.path.as_deref() != Some(path.as_str()) {
        Arc::make_mut(&mut snapshot.doc).name = file_stem(&path);
    }
    let bytes = snapshot.encode()?;
    write(&path, &bytes)?;
    ds.doc = snapshot.doc;
    ds.path = Some(path.clone());
    ds.mark_saved();
    ds.revision += 1;
    let uid = ds.uid;
    branch_tabs_saved(s, uid);
    remember(s, &path);
    Ok(json!({"path": path, "bytes": bytes.len()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(path) = string(p, "path") {
        return save_to(s, path);
    }
    match s.docs.get(crate::branches::file_tab(s)).ok_or(EngineError::NoDocument)?.path.clone() {
        // Earlier builds wrote native data under a .fig name: never overwrite that file with an export.
        Some(path) if ext(&path) == "fig" => {
            s.request_ui("dialog.saveAs", json!({}));
            Err(EngineError::Disabled("file.save".into(), "the saved path ends in .fig: choose Save as or pass `path`".into()))
        }
        Some(path) => save_to(s, path),
        None => {
            s.request_ui("dialog.saveAs", json!({}));
            Err(EngineError::Disabled("file.save".into(), "the file has no path yet: pass `path`".into()))
        }
    }
}

fn save_as(s: &mut Session, p: &Value) -> Result<Value> {
    match string(p, "path") {
        Some(mut path) => {
            if ext(&path).is_empty() {
                path.push_str(".figcraft");
            }
            save_to(s, path)
        }
        None => {
            s.request_ui("dialog.saveAs", json!({}));
            Err(EngineError::Disabled("file.saveAs".into(), "pass `path`".into()))
        }
    }
}

/// Encode `ids` (or the page) in `format` at `scale`.
pub fn export_bytes(doc: &Document, ids: &[NodeId], format: ExportFormat, scale: f64) -> Result<Vec<u8>> {
    export_bytes_with(doc, ids, &ExportSetting { format, ..Default::default() }, scale)
}

/// The page area exporting `ids` with `setting` covers: what they paint (a clipping frame's own
/// area), or their boxes with "include bounding box" and for slices. Hidden layers paint nothing,
/// so they take no room next to visible ones (a hidden-only export keeps its own area).
pub fn export_area(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> Option<figcraft_geom::Rect> {
    let any_visible = ids.iter().any(|i| doc.node(*i).is_some_and(|n| n.visible));
    ids.iter()
        .filter_map(|i| {
            let node = doc.node(*i);
            let slice = node.is_some_and(|n| matches!(n.kind, NodeKind::Slice));
            if setting.include_bounding_box || slice {
                doc.world_bounds(*i)
            } else if any_visible && node.is_some_and(|n| !n.visible) {
                None
            } else {
                figcraft_render::export_bounds(doc, *i)
            }
        })
        .reduce(|a, b| a.union(b))
}

/// What an export of `ids` draws, bottom to top: the layers themselves, or (contents only off,
/// and always for slices) every top-level layer of their page, so whatever shows in the area is in.
fn drawn_layers(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> Vec<NodeId> {
    let slice = ids.iter().any(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Slice)));
    match ids.first().and_then(|i| doc.page_of(*i)).and_then(|p| doc.node(p)) {
        Some(page) if slice || !setting.contents_only => page.children.clone(),
        _ => super::edit::z_sorted(doc, ids),
    }
}

/// [`export_bytes`] with an export setting's format and options (its size is `scale`: see
/// [`export_scale`]): contents only, include bounding box, the SVG options and PDF print settings.
pub fn export_bytes_with(doc: &Document, ids: &[NodeId], setting: &ExportSetting, scale: f64) -> Result<Vec<u8>> {
    let nothing = || EngineError::Disabled("file.export".into(), "nothing to export".into());
    if ids.is_empty() {
        return Err(nothing());
    }
    let drawn = drawn_layers(doc, ids, setting);
    if matches!(setting.format, ExportFormat::Svg | ExportFormat::Pdf) {
        figcraft_render::check_export_depth(doc, &drawn).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    let area = export_area(doc, ids, setting).ok_or_else(nothing)?;
    // The layers' own area unless an option or a slice says otherwise.
    let own_area = setting.contents_only
        && !setting.include_bounding_box
        && !ids.iter().any(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Slice)));
    match setting.format {
        // SVG and PDF colours are sRGB: a Display P3 file exports converted (out-of-gamut colours clip).
        ExportFormat::Svg => {
            let opts = figcraft_svg::SvgOptions {
                include_ids: setting.svg_id_attribute,
                outline_text: setting.svg_outline_text,
                simplify_stroke: setting.svg_simplify_stroke,
                bounds: Some(area),
            };
            figcraft_svg::try_export_svg(&doc.in_srgb(), &drawn, &opts).map(String::into_bytes).map_err(|e| match e {
                figcraft_svg::SvgError::Empty => nothing(),
                other => EngineError::Other(other.to_string()),
            })
        }
        ExportFormat::Pdf => {
            let doc = &*doc.in_srgb();
            // Frames export one page each; anything else shares one page.
            let all_frames = own_area && ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like()));
            let groups: Vec<Vec<NodeId>> = if all_frames { ids.iter().map(|i| vec![*i]).collect() } else { vec![drawn] };
            pdf_bytes(doc, &groups, &setting.print, (!own_area).then_some(area))
        }
        _ => {
            let pix = figcraft_render::render_area(doc, &drawn, area, scale)
                .ok_or_else(|| EngineError::Other("render failed (empty or too large)".into()))?;
            figcraft_render::encode_export(&pix, setting.format, doc.color_profile).map_err(EngineError::Other)
        }
    }
}

/// What Copy as PNG puts on the clipboard: the layers together, at 2x (in the document's colour
/// profile; the clipboard wants sRGB).
pub fn copy_png(doc: &Document, ids: &[NodeId]) -> Option<figcraft_render::Pixmap> {
    let area = export_area(doc, ids, &ExportSetting::default())?;
    figcraft_render::render_area(doc, &super::edit::z_sorted(doc, ids), area, 2.0)
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export";
    let fig = match string(p, "format") {
        Some(f) => f.eq_ignore_ascii_case("fig"),
        None => string(p, "path").is_some_and(|pp| ext(&pp) == "fig"),
    };
    if fig {
        return export_fig(s, p);
    }
    let doc = s.doc()?;
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = doc.get(s.page()?)?.children.clone();
    }
    let mut setting = export_setting(cmd, ExportSetting::default(), p)?;
    if string(p, "format").is_none()
        && let Some(f) = string(p, "path").and_then(|pp| ExportFormat::from_ext(&ext(&pp)))
    {
        setting.format = f;
    }
    let scale = match size_param(cmd, p)? {
        Some(_) => export_scale(doc, &ids, &setting),
        None => num_or(p, "scale", 1.0).clamp(0.01, ExportSize::MAX_SCALE),
    };
    let format = setting.format;
    let bytes = export_bytes_with(doc, &ids, &setting, scale)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "format": format.ext()}))
        }
        None => Ok(json!({"format": format.ext(), "data": base64::engine::general_purpose::STANDARD.encode(&bytes)})),
    }
}

/// The whole document as a `.fig` file (experimental: only round trips through our own importer
/// are verified), with the warnings about what was left out.
fn export_fig(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let (bytes, warnings) = fig_bytes(doc)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "format": "fig", "warnings": warnings}))
        }
        None => Ok(json!({"format": "fig", "data": base64::engine::general_purpose::STANDARD.encode(&bytes), "warnings": warnings})),
    }
}

/// `doc` as a `.fig` file (with a thumbnail) and the warnings about what was left out.
pub fn fig_bytes(doc: &Document) -> Result<(Vec<u8>, Vec<String>)> {
    figcraft_format::fig::export(doc, thumbnail(doc).as_deref()).map_err(|e| EngineError::Other(e.to_string()))
}

/// The scale that gives `ids` (their export area together) `setting`'s export size.
pub fn export_scale(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> f64 {
    // Size-based exports ask for bounds before encoding. Let encoding report the depth error
    // without entering a recursive bounds walk here; SVG/PDF do not use this raster scale.
    if matches!(setting.format, ExportFormat::Svg | ExportFormat::Pdf) && figcraft_render::check_export_depth(doc, ids).is_err() {
        return 1.0;
    }
    export_area(doc, ids, setting).map_or(1.0, |b| setting.size.scale_for(b.width(), b.height()))
}

/// `e` with the `size` ("2x", "512w", "512h" or `{by, value}`), `suffix` and `format` that `p` sets.
pub(super) fn export_setting(cmd: &str, mut e: ExportSetting, p: &Value) -> Result<ExportSetting> {
    if let Some(size) = size_param(cmd, p)? {
        e.size = size;
    }
    if let Some(s) = string(p, "suffix") {
        e.suffix = s.chars().filter(|c| !c.is_control()).take(200).collect();
    }
    if let Some(f) = string(p, "format") {
        e.format = ExportFormat::from_ext(&f).ok_or_else(|| bad(cmd, "format: png|jpg|svg|pdf|webp"))?;
    }
    e.print = print_param(cmd, p, e.print)?;
    for (key, slot) in [
        ("contentsOnly", &mut e.contents_only),
        ("includeBoundingBox", &mut e.include_bounding_box),
        ("svgOutlineText", &mut e.svg_outline_text),
        ("svgIdAttribute", &mut e.svg_id_attribute),
        ("svgSimplifyStroke", &mut e.svg_simplify_stroke),
    ] {
        match get(p, key) {
            None | Some(Value::Null) => {}
            Some(v) => *slot = bool_of(v).ok_or_else(|| bad(cmd, format!("{key}: true or false")))?,
        }
    }
    Ok(e)
}

/// Whether `p` sets any PDF print option.
fn has_print_param(p: &Value) -> bool {
    ["bleed", "cropMarks", "bleedMarks", "cmyk"].iter().any(|k| get(p, k).is_some_and(|v| !v.is_null()))
}

/// `base` with the PDF print options `p` sets: `bleed` (points, or a length like "3mm" or
/// "0.125in"), `cropMarks`, `bleedMarks`, `cmyk`.
pub(super) fn print_param(cmd: &str, p: &Value, mut base: PrintSettings) -> Result<PrintSettings> {
    match get(p, "bleed") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let pt = match v {
                Value::String(s) => PrintSettings::parse_length(s),
                v => v.as_f64().filter(|n| n.is_finite() && *n >= 0.0),
            };
            base.bleed = pt
                .filter(|b| *b <= PrintSettings::MAX_BLEED)
                .ok_or_else(|| bad(cmd, "bleed: points, or a length like \"3mm\" or \"0.125in\" (up to 144 pt, 2 in)"))?;
        }
    }
    for (key, slot) in [("cropMarks", &mut base.crop_marks), ("bleedMarks", &mut base.bleed_marks), ("cmyk", &mut base.cmyk)] {
        match get(p, key) {
            None | Some(Value::Null) => {}
            Some(v) => *slot = bool_of(v).ok_or_else(|| bad(cmd, format!("{key}: true or false")))?,
        }
    }
    Ok(base)
}

/// An export size passed as `size`: "2x", "512w", "512h", a number (scale) or `{by, value}`.
fn size_param(cmd: &str, p: &Value) -> Result<Option<ExportSize>> {
    let size = match get(p, "size") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => ExportSize::parse(s),
        Some(Value::Number(n)) => n.as_f64().and_then(|value| ExportSize::Scale { value }.checked()),
        Some(v) => serde_json::from_value::<ExportSize>(v.clone()).ok().and_then(ExportSize::checked),
    };
    size.map(Some).ok_or_else(|| bad(cmd, "size: 2x, 512w or 512h (or {by: scale|width|height, value})"))
}

/// Most files one `file.exportLayers` writes.
const MAX_EXPORT_FILES: usize = 1000;

/// One file of a layer export: the layer, its setting and the file name.
pub struct ExportJob {
    pub id: NodeId,
    pub setting: ExportSetting,
    pub file: String,
}

/// The files exporting `ids` with their export settings makes, with unique names (repeats get
/// " 2", " 3"…). Layers without settings make none.
pub fn export_jobs(doc: &Document, ids: &[NodeId]) -> Vec<ExportJob> {
    jobs_for(ids.iter().filter_map(|i| Some((*i, doc.node(*i)?.exports.clone()))), doc)
}

/// The files exporting each layer with the settings beside it makes (see [`export_jobs`]).
pub fn jobs_for(items: impl IntoIterator<Item = (NodeId, Vec<ExportSetting>)>, doc: &Document) -> Vec<ExportJob> {
    let mut used = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (id, settings) in items {
        let Some(n) = doc.node(id) else { continue };
        let base = export_path(&n.name);
        for e in settings {
            if out.len() >= MAX_EXPORT_FILES {
                return out;
            }
            let stem = format!("{base}{}", safe_file_name(&e.file_suffix()));
            let ext = e.format.ext();
            let file =
                (1..).map(|k| if k == 1 { format!("{stem}.{ext}") } else { format!("{stem} {k}.{ext}") }).find(|f| used.insert(f.to_lowercase()));
            if let Some(file) = file {
                out.push(ExportJob { id, setting: e, file });
            }
        }
    }
    out
}

/// The file path a layer name exports to, without extension: each `/` makes a folder; `.` and
/// `..` segments are dropped, so it stays inside the export folder.
fn export_path(name: &str) -> String {
    let parts: Vec<String> =
        name.split('/').map(|seg| safe_file_name(seg).trim().trim_end_matches('.').to_string()).filter(|seg| !seg.is_empty()).take(32).collect();
    if parts.is_empty() { "Untitled".into() } else { parts.join("/") }
}

/// `name` without path separators and characters file systems reject (`/ \ : * ? " < > |` and
/// control characters become `_`); everything else is kept, up to the first 200 characters.
/// `figcraft-cli export` names its files by this rule too (and keeps at most 100 characters before
/// the number and extension it adds).
pub fn safe_file_name(name: &str) -> String {
    name.chars().take(200).map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect()
}

/// Layers on `page` that have export settings (document order).
pub fn exportable_layers(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.subtree(page).into_iter().skip(1).filter(|i| doc.node(*i).is_some_and(|n| !n.exports.is_empty())).collect()
}

fn export_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.exportLayers";
    let doc = s.doc()?;
    let assets = boolean(p, "assets").unwrap_or(false);
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = if assets { vec![s.page()?] } else { exportable_layers(doc, s.page()?) };
    }
    let jobs = if assets { crate::devinspect::asset_jobs(doc, &ids) } else { export_jobs(doc, &ids) };
    if jobs.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "no export settings (add one with export.add)".into()));
    }
    let dir = string(p, "dir");
    // Print options passed here stand in for every PDF setting's own.
    let print = has_print_param(p).then(|| print_param(cmd, p, PrintSettings::default())).transpose()?;
    let mut files = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let setting = ExportSetting { print: print.unwrap_or(job.setting.print), ..job.setting.clone() };
        let scale = export_scale(doc, &[job.id], &setting);
        let bytes = export_bytes_with(doc, &[job.id], &setting, scale)?;
        let mut f = json!({"name": job.file, "id": job.id, "bytes": bytes.len()});
        match &dir {
            Some(d) => {
                let path = std::path::Path::new(d).join(&job.file);
                // A layer name with slashes exports into folders.
                if let Some(parent) = path.parent() {
                    let _ = crate::file_access::create_dir_all(parent);
                }
                let path = path.to_string_lossy().to_string();
                write(&path, &bytes)?;
                f["path"] = json!(path);
            }
            None => f["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
        }
        files.push(f);
    }
    let with_settings: std::collections::BTreeSet<NodeId> = jobs.iter().map(|j| j.id).collect();
    Ok(json!({"files": files, "skipped": ids.len().saturating_sub(with_settings.len())}))
}

/// A PDF with one page per group of layers (an sRGB document), titled after the file; `area`
/// sizes a single page to that page area instead of the layers' bounds.
fn pdf_bytes(doc: &Document, groups: &[Vec<NodeId>], print: &PrintSettings, area: Option<figcraft_geom::Rect>) -> Result<Vec<u8>> {
    let opts = figcraft_pdf::PdfOptions { title: Some(doc.name.clone()), print: *print, area, ..Default::default() };
    figcraft_pdf::export_groups(doc, groups, &opts).map_err(|e| EngineError::Other(e.to_string()))
}

/// Every top-level frame of `page` (reading order) as one PDF page each, and the page count.
pub fn frames_pdf(doc: &Document, page: NodeId, print: &PrintSettings) -> Result<(Vec<u8>, usize)> {
    let mut frames = doc.top_frames(page);
    frames.retain(|f| doc.node(*f).is_some_and(|n| n.visible));
    if frames.is_empty() {
        return Err(EngineError::Disabled("file.exportFramesPdf".into(), "the page has no visible frames".into()));
    }
    let groups: Vec<Vec<NodeId>> = frames.iter().map(|f| vec![*f]).collect();
    Ok((pdf_bytes(&doc.in_srgb(), &groups, print, None)?, frames.len()))
}

/// Every top-level frame of a page (reading order), one PDF page each.
fn export_frames_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let page = match id(p, "page") {
        Some(pg) if doc.node(pg).is_some_and(Node::is_page) => pg,
        Some(_) => return Err(bad("file.exportFramesPdf", "`page` is not a page")),
        None => s.page()?,
    };
    let print = print_param("file.exportFramesPdf", p, PrintSettings::default())?;
    let (bytes, pages) = frames_pdf(doc, page, &print)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "pages": pages}))
        }
        None => Ok(json!({"format": "pdf", "pages": pages, "data": base64::engine::general_purpose::STANDARD.encode(&bytes)})),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = string(p, "path");
    let variables = variables_param("file.import", p)?;
    let (bytes, name) = if let Some(path) = &path {
        (read(path)?, path.clone())
    } else if let Some(d) = string(p, "data") {
        let d = d.split_once(',').map(|(_, b)| b.to_string()).unwrap_or(d);
        (
            base64::engine::general_purpose::STANDARD.decode(d.trim()).map_err(|e| bad("file.import", e.to_string()))?,
            string(p, "name").unwrap_or_else(|| "import.png".into()),
        )
    } else {
        return Err(bad("file.import", "pass `path` or `data`"));
    };
    let page = s.page()?;
    let (x, y) = (num(p, "x"), num(p, "y"));
    let center = s.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
    match ext(&name).as_str() {
        "svg" => {
            let doc = s.doc_mut()?;
            let (x, y) = (x.unwrap_or(center.0), y.unwrap_or(center.1));
            let imported =
                figcraft_svg::import_svg_with_notes(doc, &bytes, page, x, y, &file_stem(&name)).map_err(|e| EngineError::Other(e.to_string()))?;
            let id = imported.frame;
            // Centred on the point (a paste lands in the middle of the view).
            if boolean(p, "center").unwrap_or(false)
                && let Some(n) = doc.node_mut(id)
            {
                n.transform = figcraft_geom::Affine::translate(((x - n.width / 2.0).round(), (y - n.height / 2.0).round()));
            }
            s.set_selection(vec![id]);
            let mut out = json!({"id": id});
            if !imported.notes.is_empty() {
                out["warnings"] = json!(imported.notes);
            }
            Ok(out)
        }
        "figcraft" | "json" | "fig" => {
            let resolver = path.as_deref().map(rest_images);
            let images: figcraft_format::rest::ImageResolver = match &resolver {
                Some(r) => r,
                None => &figcraft_format::rest::no_images,
            };
            let sibling = if variables.is_none() { path.as_deref().and_then(sibling_variables) } else { None };
            let (other, rest) = load_with(&bytes, &name, variables.as_ref().or(sibling.as_ref()), images)?;
            let summary = import_summary(&other.doc);
            let pages = append_pages(s.doc_mut()?, &other.doc)?;
            let mut out = json!({"pages": pages, "summary": summary});
            if rest {
                out["warnings"] = json!(other.issues);
            }
            Ok(out)
        }
        _ => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            super::create::place_image(s, &json!({"data": b64, "x": x.unwrap_or(center.0), "y": y.unwrap_or(center.1), "name": file_stem(&name)}))
        }
    }
}

/// Append `other`'s pages (with their settings and flows, its images, styles and variables) to
/// `doc`. Ids are remapped across all pages at once, so instances keep their main, instance layers
/// their source and interactions and flows their targets wherever those are. Styles, variables and
/// collections whose id the document already has are kept as they are.
fn append_pages(doc: &mut Document, other: &Document) -> Result<Vec<NodeId>> {
    for (k, v) in &other.images {
        doc.images.entry(k.clone()).or_insert_with(|| v.clone());
    }
    for st in &other.styles {
        if !doc.styles.iter().any(|s| s.id == st.id) {
            doc.styles.push(st.clone());
        }
    }
    let have: std::collections::BTreeSet<String> = doc.variables.collections.iter().map(|c| c.id.clone()).collect();
    doc.variables.collections.extend(other.variables.collections.iter().filter(|c| !have.contains(&c.id)).cloned());
    let have: std::collections::BTreeSet<String> = doc.variables.variables.iter().map(|v| v.id.clone()).collect();
    doc.variables.variables.extend(other.variables.variables.iter().filter(|v| !have.contains(&v.id)).cloned());
    let mut map = std::collections::BTreeMap::new();
    for pg in &other.pages {
        for id in other.subtree(*pg).into_iter().skip(1) {
            map.insert(id, doc.alloc_id());
        }
    }
    // Library copies a .fig import keeps (cached definitions) have no library record to follow:
    // the layers of its store come along into the store of `doc`. Recorded copies are left to
    // `merge_store`, which shares them with copies `doc` already has.
    let cached: Vec<NodeId> = other
        .library_store()
        .and_then(|s| other.node(s))
        .map(|s| s.children.iter().copied().filter(|root| other.libraries.import_of(*root).is_none()).collect())
        .unwrap_or_default();
    for root in &cached {
        for id in other.subtree(*root) {
            map.insert(id, doc.alloc_id());
        }
    }
    let mut new_pages = Vec::new();
    for pg in &other.pages {
        let Some(pn) = other.node(*pg) else { continue };
        let np = doc.add_page(&pn.name);
        let mut kind = pn.kind.clone();
        if let NodeKind::Page { flows, .. } = &mut kind {
            flows.retain_mut(|f| map.get(&f.start).map(|n| f.start = *n).is_some());
        }
        if let Some(n) = doc.node_mut(np) {
            n.kind = kind;
        }
        for c in &pn.children {
            let nodes: Vec<Node> = other.subtree(*c).into_iter().filter_map(|i| other.node(i).cloned()).collect();
            doc.paste_nodes(&nodes, &map, *c, np, None)?;
        }
        new_pages.push(np);
    }
    if !cached.is_empty() {
        let store = doc.ensure_library_store();
        for root in &cached {
            let nodes: Vec<Node> = other.subtree(*root).into_iter().filter_map(|i| other.node(i).cloned()).collect();
            doc.paste_nodes(&nodes, &map, *root, store, None)?;
        }
    }
    for new in map.values() {
        let src = doc.node(*new).and_then(|n| n.source).and_then(|s| map.get(&s).copied());
        if let (Some(src), Some(n)) = (src, doc.node_mut(*new)) {
            n.source = Some(src);
        }
    }
    crate::library::merge_store(doc, other, &map)?;
    Ok(new_pages)
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let i = if get(p, "index").is_none() { s.active } else { req_index("file.close", p, "index")? };
    let Some(ds) = s.docs.get(i) else { return Err(bad("file.close", "no such document")) };
    // A branch tab's edits are in its main file already (which then has unsaved changes).
    if ds.branch_of.is_none() && ds.is_dirty() && !boolean(p, "force").unwrap_or(false) {
        s.request_ui("dialog.unsaved", json!({"index": i}));
        return Err(EngineError::Disabled("file.close".into(), "the document has unsaved changes (pass force: true to discard)".into()));
    }
    let uid = ds.uid;
    let active_uid = s.docs.get(s.active).map(|d| d.uid);
    s.docs.remove(i);
    // Removing a tab before the active one shifts its index; keep the same document selected.
    s.active = active_uid.and_then(|a| s.docs.iter().position(|d| d.uid == a)).unwrap_or(i.min(s.docs.len().saturating_sub(1)));
    crate::branches::close_tabs(s, uid, None);
    ok()
}

#[cfg(test)]
mod tab_order_tests {
    use super::*;

    #[test]
    fn moving_and_closing_inactive_tabs_preserves_the_active_document() {
        let mut session = Session::new();
        for name in ["Two", "Three", "Four"] {
            session.execute("file.new", &json!({"name": name})).unwrap();
        }
        session.execute("doc.switch", &json!({"index": 2})).unwrap();
        let active_uid = session.docs[session.active].uid;
        let moved_uid = session.docs[0].uid;
        session.execute("file.moveTab", &json!({"from": 0, "to": 2})).unwrap();
        assert_eq!(session.docs[2].uid, moved_uid);
        assert_eq!(session.docs[session.active].uid, active_uid);
        session.execute("file.close", &json!({"index": 0})).unwrap();
        assert_eq!(session.docs[session.active].uid, active_uid);
        assert!(session.execute("file.moveTab", &json!({"from": 0, "to": 99})).is_err());
    }
}

/// Ctrl+Tab / Ctrl+Shift+Tab: the next or previous open document, wrapping around.
fn cycle_doc(s: &mut Session, dir: i64) -> Result<Value> {
    let n = s.docs.len() as i64;
    if n == 0 {
        return Err(crate::EngineError::NoDocument);
    }
    s.active = (s.active as i64 + dir).rem_euclid(n) as usize;
    Ok(json!({"index": s.active}))
}

#[cfg(test)]
mod rest_import_tests {
    use super::*;

    /// A synthetic REST API file JSON: a component with a text layer, an instance of it whose text
    /// is overridden, and an image fill.
    fn kit() -> Value {
        let at = |x: f64, y: f64| json!([[1, 0, x], [0, 1, y]]);
        let label = |id: &str, text: &str| {
            json!({"id": id, "name": "Label", "type": "TEXT", "relativeTransform": at(8.0, 8.0), "size": {"x": 40, "y": 20}, "characters": text,
                   "style": {"fontFamily": "Inter", "fontSize": 14}, "fills": [{"type": "SOLID", "color": {"r": 0, "g": 0, "b": 0, "a": 1}}]})
        };
        json!({"name": "Kit", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
            {"id": "0:1", "name": "Main", "type": "CANVAS", "backgroundColor": {"r": 0.9, "g": 0.9, "b": 0.9, "a": 1}, "children": [
                {"id": "1:1", "name": "Button", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 40},
                 "fills": [{"type": "SOLID", "color": {"r": 0.2, "g": 0.4, "b": 1, "a": 1}}], "children": [label("1:2", "OK")]},
                {"id": "2:1", "name": "Button", "type": "INSTANCE", "componentId": "1:1", "relativeTransform": at(0.0, 100.0), "size": {"x": 100, "y": 40},
                 "fills": [{"type": "SOLID", "color": {"r": 0.2, "g": 0.4, "b": 1, "a": 1}}], "children": [label("I2:1;1:2", "Go")]},
                {"id": "3:1", "name": "Photo", "type": "RECTANGLE", "relativeTransform": at(200.0, 0.0), "size": {"x": 30, "y": 20},
                 "fills": [{"type": "IMAGE", "imageRef": "photo1", "scaleMode": "FILL"}]}
            ]}
        ]}, "components": {"1:1": {"key": "k", "name": "Button", "description": ""}}, "styles": {}})
    }

    fn instance(doc: &Document) -> &Node {
        doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap()
    }

    /// What a .fig import with a cached library component leaves: the main in the hidden store, a
    /// visible instance of it and a layer linked to the main's layer.
    fn cached_import() -> Document {
        let mut doc = Document::new("Cached");
        let page = doc.first_page().unwrap();
        let store = doc.ensure_library_store();
        let main = doc
            .create(store, NodeKind::Component { description: String::new(), props: Default::default() }, "Cached main", 0.0, 0.0, 20.0, 20.0)
            .unwrap();
        let body = doc.create(main, NodeKind::Rectangle, "Body", 0.0, 0.0, 20.0, 20.0).unwrap();
        let inst =
            doc.create(page, NodeKind::Instance { component: main, props: Default::default() }, "Cached instance", 0.0, 0.0, 20.0, 20.0).unwrap();
        let mut copy = doc.node(body).unwrap().clone();
        copy.id = doc.alloc_id();
        copy.parent = None;
        copy.source = Some(body);
        let copy_id = copy.id;
        doc.insert(copy, inst, None).unwrap();
        assert!(doc.is_remote(main) && doc.node(copy_id).unwrap().source == Some(body));
        doc
    }

    /// Pasting layers that use a cached main into another document leaves the main behind, as it
    /// does for other mains that are not published: no link points at a layer of the target.
    #[test]
    fn pasting_instances_of_cached_components_leaves_no_dangling_links() {
        let src = cached_import();
        let mut doc = Document::new("Target");
        let page = doc.first_page().unwrap();
        for i in 0..8 {
            doc.create(page, NodeKind::Rectangle, &format!("Filler {i}"), 0.0, 0.0, 5.0, 5.0).unwrap();
        }
        let inst = src.nodes().find(|n| n.name == "Cached instance").unwrap().id;
        let ids = src.subtree(inst);
        let map: std::collections::BTreeMap<NodeId, NodeId> = ids.iter().map(|i| (*i, doc.alloc_id())).collect();
        let nodes: Vec<Node> = ids.iter().filter_map(|i| src.node(*i).cloned()).collect();
        doc.paste_nodes(&nodes, &map, inst, page, None).unwrap();
        crate::library::adopt_pasted(&mut doc, &src, None, &map).unwrap();
        let pasted = doc.node(map[&inst]).unwrap();
        let NodeKind::Instance { component, .. } = pasted.kind else { panic!() };
        assert!(!doc.contains(component), "the main stays behind: {component:?}");
        assert!(doc.node(pasted.children[0]).unwrap().source.is_none_or(|s| !doc.contains(s)));
    }

    #[test]
    fn appending_pages_with_cached_library_components_brings_the_components() {
        let other = cached_import();
        let mut doc = Document::new("Target");
        let page = doc.first_page().unwrap();
        // Ids of the target that would collide with the other document's store.
        for i in 0..8 {
            doc.create(page, NodeKind::Rectangle, &format!("Filler {i}"), 0.0, 0.0, 5.0, 5.0).unwrap();
        }
        append_pages(&mut doc, &other).unwrap();
        assert!(doc.validate().is_empty(), "{:?}", doc.validate());
        assert!(crate::components::mirror_issues(&doc).is_empty(), "{:?}", crate::components::mirror_issues(&doc));
        let inst = doc.nodes().find(|n| n.name == "Cached instance").unwrap();
        let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
        assert_eq!(doc.node(component).unwrap().name, "Cached main");
        assert!(doc.is_remote(component), "the main comes along into the store");
        let layer = doc.node(inst.children[0]).unwrap();
        assert_eq!(layer.source, doc.node(component).unwrap().children.first().copied());
    }

    #[test]
    fn rest_json_loads_with_linked_instances() {
        let bytes = serde_json::to_vec(&kit()).unwrap();
        let (doc, warnings) = load_bytes(&bytes, "kit.json").unwrap();
        assert_eq!(doc.name, "Kit");
        assert!(warnings.iter().any(|w| w.contains("placeholder")), "{warnings:?}");
        let inst = instance(&doc);
        let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
        assert_eq!(inst.source, Some(component));
        let main_label = doc.node(component).unwrap().children[0];
        let label = doc.node(inst.children[0]).unwrap();
        assert_eq!(label.source, Some(main_label));
        assert!(label.overrides.contains("characters"), "{:?}", label.overrides);
        assert!(crate::components::mirror_issues(&doc).is_empty());
        assert!(thumbnail(&doc).is_some());
    }

    #[test]
    fn open_takes_data_and_a_name_as_a_new_unsaved_document() {
        let mut saved = Session::new();
        saved.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        let bytes = file_bytes(saved.ds().unwrap()).unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mut s = Session::new();
        let recent = s.prefs.recent_files.clone();
        let r = s.execute("file.open", &json!({"data": data, "name": "Dropped.figcraft"})).unwrap();
        let open = s.docs.len();
        assert_eq!(r["index"], open - 1, "a document of its own, made active");
        assert_eq!(s.ds().unwrap().path, None, "no path to save over");
        assert!(s.doc().unwrap().nodes().any(|n| matches!(n.kind, NodeKind::Rectangle)));
        assert_eq!(s.prefs.recent_files, recent, "nothing on disk to remember");
        assert!(s.execute("file.open", &json!({"data": data})).is_err(), "the name tells the format");
        assert!(s.execute("file.open", &json!({"data": "not base64!", "name": "x.svg"})).is_err());
        assert_eq!(s.docs.len(), open, "a refused open adds nothing");
    }

    #[test]
    fn open_and_import_rest_json() {
        let dir = std::env::temp_dir().join(format!("figcraft-rest-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("images")).unwrap();
        let png = {
            let mut d = Document::new("x");
            let p = d.first_page().unwrap();
            let id = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 3.0, 2.0).unwrap();
            export_bytes(&d, &[id], ExportFormat::Png, 1.0).unwrap()
        };
        std::fs::write(dir.join("images").join("photo1.png"), &png).unwrap();
        let path = dir.join("kit.json");
        std::fs::write(&path, serde_json::to_vec(&kit()).unwrap()).unwrap();
        let path = path.to_string_lossy().to_string();

        let mut s = Session::new();
        let r = s.execute("file.open", &json!({"path": path})).unwrap();
        assert!(r["warnings"].as_array().is_some_and(|w| w.iter().all(|w| !w.as_str().unwrap_or("").contains("placeholder"))), "{r}");
        assert!(r.get("repairs").is_none());
        assert_eq!(s.ds().unwrap().path, None, "an import is never saved over its JSON");
        let doc = s.doc().unwrap();
        let photo = doc.nodes().find(|n| n.name == "Photo").unwrap();
        let figcraft_color::PaintKind::Image(img) = &photo.fills[0].kind else { panic!() };
        assert_eq!(doc.images.get(&img.image).map(|i| (i.width, i.height)), Some((3, 2)), "read from images/ next to the file");

        // Import into another document: pages are appended and references follow the new ids.
        let mut s = Session::new();
        let data = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&kit()).unwrap());
        let r = s.execute("file.import", &json!({"data": data, "name": "kit.json"})).unwrap();
        assert_eq!(r["pages"].as_array().map(Vec::len), Some(1));
        assert!(r["warnings"].is_array(), "{r}");
        let doc = s.doc().unwrap();
        assert_eq!(doc.pages.len(), 2);
        let page = doc.node(doc.pages[1]).unwrap();
        assert!(matches!(page.kind, NodeKind::Page { background, .. } if (background.r - 0.9).abs() < 1e-6));
        let inst = instance(doc);
        let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
        assert!(doc.node(component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })));
        let label = doc.node(inst.children[0]).unwrap();
        assert!(label.source.is_some_and(|src| doc.is_ancestor(component, src)));
        assert!(crate::components::mirror_issues(doc).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod staging_write_tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("figcraft-engine-staging-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A symlink `from` → `to`; false when the platform won't make one (Windows needs privileges).
    #[cfg(unix)]
    fn symlink(from: &std::path::Path, to: &std::path::Path) -> bool {
        std::os::unix::fs::symlink(to, from).is_ok()
    }

    #[cfg(windows)]
    fn symlink(from: &std::path::Path, to: &std::path::Path) -> bool {
        std::os::windows::fs::symlink_file(to, from).is_ok()
    }

    #[test]
    fn an_existing_staging_file_is_never_written_through() {
        let d = dir("existing");
        let path = d.join("shot.svg");
        // The old deterministic staging name, held by an unrelated file: the write must pick its
        // own staging name instead of truncating this one.
        let old = d.join("shot.svg.tmp-figcraft");
        std::fs::write(&old, b"not yours").unwrap();
        write(&path.to_string_lossy(), b"<svg/>").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"<svg/>");
        assert_eq!(std::fs::read(&old).unwrap(), b"not yours", "an unrelated file at a staging name is untouched");
        // Just the file and the untouched staging file: no new staging leftovers.
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_staging_symlink_is_never_followed() {
        let d = dir("symlink");
        let path = d.join("shot.svg");
        let victim = d.join("victim.txt");
        std::fs::write(&victim, b"innocent").unwrap();
        let old = d.join("shot.svg.tmp-figcraft");
        if !symlink(&old, &victim) {
            return; // Making a symlink needs privileges on some Windows setups.
        }
        write(&path.to_string_lossy(), b"<svg/>").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"innocent", "the symlink was never written through");
        assert_eq!(std::fs::read(&path).unwrap(), b"<svg/>");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn concurrent_writers_of_one_file_never_interleave() {
        let d = dir("concurrent");
        let path = d.join("shot.svg");
        for round in 0..5u8 {
            let mut handles = Vec::new();
            for w in 0u8..4 {
                let (path, bytes) = (path.clone(), vec![b'0' + w; 8192]);
                handles.push(std::thread::spawn(move || write(&path.to_string_lossy(), &bytes)));
            }
            let mut wins = 0;
            for h in handles {
                // On Windows a rename over a file another writer just replaced can lose the race.
                if h.join().unwrap().is_ok() {
                    wins += 1;
                }
            }
            assert!(wins > 0, "round {round}: every writer lost");
            // Whatever landed is one writer's complete payload, never a mix.
            let got = std::fs::read(&path).unwrap();
            assert_eq!(got.len(), 8192, "round {round}: a complete payload landed");
            assert!(got.windows(2).all(|w| w[0] == w[1]), "round {round}: one writer's payload, not a mix");
        }
        // No staging file was left stuck (a failed writer removes its own).
        let left = std::fs::read_dir(&d).unwrap().count();
        assert!(left <= if cfg!(windows) { 5 } else { 1 }, "{left} files left in the staging directory");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_leaves_the_destination_alone_and_cleans_up() {
        let d = dir("failure");
        // The destination is a directory: the rename fails, our staging file is removed and the
        // directory is untouched.
        let taken = d.join("over-a-directory");
        std::fs::create_dir_all(&taken).unwrap();
        assert!(write(&taken.to_string_lossy(), b"payload").is_err());
        assert!(taken.is_dir());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no staging file left behind");
        // A missing parent directory fails without creating anything anywhere.
        assert!(write(&d.join("no").join("such.svg").to_string_lossy(), b"payload").is_err());
        assert!(!d.join("no").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A read-only directory refuses the staging file, and the old destination stays.
            let ro = d.join("read-only");
            std::fs::create_dir_all(&ro).unwrap();
            let dest = ro.join("keep.svg");
            std::fs::write(&dest, b"keep").unwrap();
            let mut perms = std::fs::metadata(&ro).unwrap().permissions();
            perms.set_mode(0o500);
            std::fs::set_permissions(&ro, perms).unwrap();
            // Running as root would defeat the read-only directory: skip instead of failing.
            let probe = ro.join(".probe");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
            } else {
                assert!(write(&dest.to_string_lossy(), b"payload").is_err());
                assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
                assert_eq!(std::fs::read_dir(&ro).unwrap().count(), 1, "no staging file left behind");
            }
            let mut perms = std::fs::metadata(&ro).unwrap().permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(&ro, perms).unwrap();
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
