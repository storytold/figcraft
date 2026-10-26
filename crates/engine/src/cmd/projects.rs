//! The home screen's project library ([`crate::projects`]): folders, the files in them and
//! favourites. Nothing here moves, renames or deletes a file on disk.

use std::path::Path;

use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::file_access::{Access, allow, allowed};
use crate::params::*;
use crate::projects::{Favorite, Projects};
use crate::{Result, Session, bad, spec};

/// Folders [`import_folder`] reads at most this deep below the one picked, and files at most.
pub const IMPORT_DEPTH: usize = 8;
pub const IMPORT_FILES: usize = 5_000;
/// Folders [`import_folder`] looks into at most (skipped folders count too).
const IMPORT_DIRS: usize = 20_000;

/// The design files a library keeps (what Open opens as a document of its own).
pub fn is_project_file(path: &Path) -> bool {
    matches!(path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref(), Some("figcraft" | "fig" | "sketch"))
}

/// A folder id param: a string, or `null`/missing for the top level.
fn folder_param(cmd: &str, s: &Session, p: &Value, key: &str) -> Result<Option<String>> {
    match string(p, key) {
        None => Ok(None),
        Some(id) if s.projects.folder(&id).is_some() => Ok(Some(id)),
        Some(id) => Err(bad(cmd, format!("no folder {id}"))),
    }
}

/// `paths` (a list) or `path`: each one allowed to be read.
fn paths_param(cmd: &str, p: &Value) -> Result<Vec<String>> {
    let list: Vec<String> = match get(p, "paths") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(_) => return Err(bad(cmd, "`paths` must be a list of paths")),
        None => vec![req_string(cmd, p, "path")?],
    };
    if list.is_empty() || list.iter().any(|x| x.trim().is_empty()) {
        return Err(bad(cmd, "a file needs a path"));
    }
    for path in &list {
        allow(path, Access::Read)?;
    }
    Ok(list)
}

fn favorite_of(p: &Projects, fav: Favorite) -> bool {
    p.is_favorite(&fav)
}

fn folder_json(p: &Projects, id: &str) -> Value {
    let Some(f) = p.folder(id) else { return Value::Null };
    json!({
        "id": f.id,
        "name": f.name,
        "parent": f.parent,
        "path": p.folder_path(id),
        "folders": p.children(Some(id)).len(),
        "files": p.count_in(id, false),
        "favorite": favorite_of(p, Favorite::Folder(f.id.clone())),
        "created": f.created,
    })
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.list";
    let folder = folder_param(CMD, s, p, "folder")?;
    let all = boolean(p, "all").unwrap_or(folder.is_none());
    let lib = &s.projects;
    let folders: Vec<Value> = if all {
        lib.folders.iter().map(|f| folder_json(lib, &f.id)).collect()
    } else {
        lib.children(folder.as_deref()).iter().map(|f| folder_json(lib, &f.id)).collect()
    };
    // Confined to automation roots, files outside them are never named.
    let files: Vec<Value> = lib
        .files
        .iter()
        .filter(|f| all || f.folder == folder)
        .filter(|f| allowed(&f.path, Access::Read))
        .map(|f| {
            json!({
                "path": f.path,
                "folder": f.folder,
                "added": f.added,
                "kind": f.kind.map(|k| k.id()),
                "favorite": favorite_of(lib, Favorite::File(f.path.clone())),
                "exists": crate::file_access::exists(&f.path),
            })
        })
        .collect();
    let favorites: Vec<Value> = lib
        .favorites
        .iter()
        .filter(|f| !matches!(f, Favorite::File(path) if !allowed(path, Access::Read)))
        .map(|f| serde_json::to_value(f).unwrap_or(Value::Null))
        .collect();
    Ok(json!({"folders": folders, "files": files, "favorites": favorites, "folder": folder}))
}

fn create_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.createFolder";
    let parent = folder_param(CMD, s, p, "parent")?;
    let name = string(p, "name").unwrap_or_default();
    let now = super::comment::now_secs();
    let id = s.projects.create_folder(&name, parent.as_deref(), now).map_err(|e| bad(CMD, e))?;
    Ok(folder_json(&s.projects, &id))
}

fn rename_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.renameFolder";
    let id = req_string(CMD, p, "id")?;
    let name = req_string(CMD, p, "name")?;
    s.projects.rename_folder(&id, &name).map_err(|e| bad(CMD, e))?;
    Ok(folder_json(&s.projects, &id))
}

fn move_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.moveFolder";
    let id = req_string(CMD, p, "id")?;
    let parent = folder_param(CMD, s, p, "parent")?;
    s.projects.move_folder(&id, parent.as_deref()).map_err(|e| bad(CMD, e))?;
    Ok(folder_json(&s.projects, &id))
}

fn delete_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.deleteFolder";
    let id = req_string(CMD, p, "id")?;
    let d = s.projects.delete_folder(&id).map_err(|e| bad(CMD, e))?;
    Ok(json!({"folders": d.folders, "files": d.files, "to": d.to}))
}

fn move_files(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.moveFiles";
    let paths = paths_param(CMD, p)?;
    let folder = folder_param(CMD, s, p, "folder")?;
    let now = super::comment::now_secs();
    let moved = s.projects.move_files(&paths, folder.as_deref(), now).map_err(|e| bad(CMD, e))?;
    Ok(json!({"moved": moved, "folder": folder}))
}

fn remove_files(s: &mut Session, p: &Value) -> Result<Value> {
    let paths = paths_param("projects.removeFiles", p)?;
    Ok(json!({"removed": s.projects.remove_files(&paths)}))
}

fn favorite(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.favorite";
    let fav = match (string(p, "folder"), string(p, "path")) {
        (Some(id), None) => Favorite::Folder(id),
        (None, Some(path)) => {
            allow(&path, Access::Read)?;
            Favorite::File(path)
        }
        _ => return Err(bad(CMD, "give `folder` (an id) or `path`")),
    };
    let on = boolean(p, "on").unwrap_or_else(|| !s.projects.is_favorite(&fav));
    s.projects.set_favorite(&fav, on, super::comment::now_secs()).map_err(|e| bad(CMD, e))?;
    Ok(json!({"favorite": on}))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.relink";
    let from = req_string(CMD, p, "from")?;
    let to = req_string(CMD, p, "to")?;
    allow(&to, Access::Read)?;
    if !crate::file_access::metadata(&to).is_ok_and(|m| m.is_file()) {
        return Err(bad(CMD, format!("{to} isn't a file")));
    }
    s.projects.relink(&from, &to).map_err(|e| bad(CMD, e))?;
    ok()
}

/// A folder on disk as [`import_folder`] mirrors it: its name, its design files and the folders
/// in it that hold some (at any depth).
struct Scanned {
    name: String,
    files: Vec<String>,
    dirs: Vec<Scanned>,
}

/// What [`scan`] may still read.
struct Budget {
    files: usize,
    dirs: usize,
}

/// `dir` and the folders in it, `depth` levels at most; `None` when it holds no design file.
/// Hidden entries are skipped; entries come in name order.
fn scan(dir: &Path, depth: usize, budget: &mut Budget) -> Option<Scanned> {
    if budget.dirs == 0 {
        return None;
    }
    budget.dirs -= 1;
    let mut entries = crate::file_access::read_dir(dir).ok()?;
    entries.sort();
    let name = dir.file_name().map_or_else(|| dir.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
    let mut out = Scanned { name, files: Vec::new(), dirs: Vec::new() };
    for entry in entries {
        if entry.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        // Links aren't followed into folders: a link to a folder above would loop.
        let Ok(meta) = crate::file_access::link_metadata(&entry) else { continue };
        if meta.is_dir() {
            if depth > 0
                && let Some(sub) = scan(&entry, depth - 1, budget)
            {
                out.dirs.push(sub);
            }
        } else if is_project_file(&entry) && crate::file_access::metadata(&entry).is_ok_and(|m| m.is_file()) && budget.files > 0 {
            budget.files -= 1;
            out.files.push(entry.to_string_lossy().into_owned());
        }
    }
    (!out.files.is_empty() || !out.dirs.is_empty()).then_some(out)
}

/// Make `scanned` a folder in `parent`, its files in it. Known files already in a folder stay
/// where the user put them. (its id, folders made, files placed).
fn mirror(lib: &mut Projects, scanned: &Scanned, parent: Option<&str>, now: u64) -> std::result::Result<(String, usize, usize), String> {
    let id = lib.create_folder(&scanned.name, parent, now)?;
    let free: Vec<String> = scanned.files.iter().filter(|p| lib.file(p).is_none_or(|f| f.folder.is_none())).cloned().collect();
    let mut placed = lib.move_files(&free, Some(&id), now)?;
    let mut made = 1;
    for sub in &scanned.dirs {
        let (_, m, p) = mirror(lib, sub, Some(&id), now)?;
        made += m;
        placed += p;
    }
    Ok((id, made, placed))
}

fn import_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "projects.importFolder";
    let path = req_string(CMD, p, "path")?;
    let parent = folder_param(CMD, s, p, "folder")?;
    let dir = allow(&path, Access::Read)?;
    if !crate::file_access::is_dir(&dir) {
        return Err(bad(CMD, format!("{path} isn't a folder")));
    }
    let mut budget = Budget { files: IMPORT_FILES, dirs: IMPORT_DIRS };
    let Some(scanned) = scan(&dir, IMPORT_DEPTH, &mut budget) else {
        return Err(bad(CMD, format!("{path} has no .figcraft, .fig or .sketch files")));
    };
    let now = super::comment::now_secs();
    // All or nothing: a cap reached half way leaves the library as it was.
    let mut lib = s.projects.clone();
    let (id, folders, files) = mirror(&mut lib, &scanned, parent.as_deref(), now).map_err(|e| bad(CMD, e))?;
    s.projects = lib;
    Ok(json!({"folder": id, "folders": folders, "files": files, "capped": budget.files == 0 || budget.dirs == 0}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "projects.list",
            "List projects",
            "{folder? (an id: what is directly in it; missing: the top level), all? (every folder and file; default when no folder)} → {folders: [{id, name, parent, path, folders, files, favorite, created}], files: [{path, folder, added, kind, favorite, exists}], favorites: [{type: folder | file, id}], folder}: the home screen's project library. Files stay where they are on disk",
            false,
            list
        ),
        spec!(
            "projects.createFolder",
            "New folder",
            "{name, parent? (a folder id; missing: the top level)} → the folder {id, name, parent, path, …}. A name another folder beside it has gets a number (\"Branding 2\")",
            false,
            create_folder
        ),
        spec!(
            "projects.renameFolder",
            "Rename folder",
            "{id, name} → the folder. Refused when another folder beside it has the name",
            false,
            rename_folder
        ),
        spec!(
            "projects.moveFolder",
            "Move folder",
            "{id, parent? (a folder id; null or missing: the top level)} → the folder, with everything in it. Never into itself",
            false,
            move_folder
        ),
        spec!(
            "projects.deleteFolder",
            "Delete folder",
            "{id} → {folders (it and the folders inside it), files (moved), to (where they went: its parent, or null)}: deletes the folder from the library; its files stay, on disk and in the library",
            false,
            delete_folder
        ),
        spec!(
            "projects.moveFiles",
            "Move to folder",
            "{paths: [path] | path, folder? (an id; null or missing: in no folder)} → {moved, folder}: puts files in a folder of the library (added when new); the files stay where they are on disk",
            false,
            move_files
        ),
        spec!(
            "projects.removeFiles",
            "Remove from projects",
            "{paths: [path] | path} → {removed}: forgets files in the library (and their favourites); the files stay on disk",
            false,
            remove_files
        ),
        spec!(
            "projects.favorite",
            "Add to favorites",
            "{folder: id | path, on? (default: toggle)} → {favorite}: adds a folder or file to Favorites or takes it out (a shortcut: nothing is copied or moved)",
            false,
            favorite
        ),
        spec!(
            "projects.relink",
            "Locate file",
            "{from (the path the library knows), to (where the file is now)} → {ok}: a file that moved keeps its folder and favourite",
            false,
            relink
        ),
        spec!(
            "projects.importFolder",
            "Add folder from disk",
            "{path (a folder on disk), folder? (the library folder to put it in)} → {folder, folders, files, capped}: mirrors the folder and the folders in it that hold .figcraft, .fig or .sketch files as library folders (8 levels, 5000 files at most). Files stay where they are; files already in a library folder keep it",
            false,
            import_folder
        ),
    ]
}
