//! The project library: folders the home screen organises files into, and favourites.
//!
//! An organisation laid over files that stay where they are: putting a file in a folder, taking
//! it out, renaming a folder or deleting one never moves, renames or deletes a file on disk.
//! Folders have stable ids (their names can repeat under other parents and change); files are
//! known by their path. A file whose path no longer leads anywhere keeps its place, so it can be
//! found again ([`Projects::relink`]) or removed by hand. The desktop app keeps the library next
//! to the preferences in a file of its own, so a build that rewrites the preferences without
//! knowing about it leaves it alone.

use std::collections::{BTreeSet, HashMap, HashSet};

use figcraft_doc::FileKind;
use serde::{Deserialize, Serialize};

/// The library format written now.
pub const VERSION: u32 = 1;
/// Caps that keep a damaged or hostile library file from costing the app its memory.
pub const MAX_FOLDERS: usize = 5_000;
pub const MAX_FILES: usize = 50_000;
/// Folders nest at most this deep (a top-level folder is depth 1).
pub const MAX_DEPTH: usize = 32;
/// A folder name's length, in characters.
pub const MAX_NAME: usize = 120;
/// The name a folder gets when it has none.
pub const UNTITLED: &str = "Untitled folder";

/// Folders and files the user organised, and their favourites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Projects {
    pub version: u32,
    /// The number the next folder's id takes (`f1`, `f2`…); ids are never reused.
    pub next_id: u64,
    pub folders: Vec<Folder>,
    pub files: Vec<ProjectFile>,
    /// Shortcuts in the order they were added (the sidebar's Favorites).
    pub favorites: Vec<Favorite>,
    /// Changed since it was last kept: the app saves it and clears this.
    #[serde(skip)]
    pub unsaved: bool,
}

impl Default for Projects {
    fn default() -> Self {
        Projects { version: VERSION, next_id: 1, folders: Vec::new(), files: Vec::new(), favorites: Vec::new(), unsaved: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// The folder it is in (`None`: at the top).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Seconds since 1970.
    pub created: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectFile {
    pub path: String,
    /// The folder it is in (`None`: in no folder).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// When it joined the library (seconds since 1970).
    pub added: u64,
    /// A design file or a board, once known (from opening it or reading its picture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<FileKind>,
}

/// A shortcut in Favorites: a folder by its id, or a file by its path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "lowercase")]
pub enum Favorite {
    Folder(String),
    File(String),
}

/// What deleting a folder did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deleted {
    /// The folders removed: it and every folder inside it.
    pub folders: usize,
    /// The files that were in them, now in `to`.
    pub files: usize,
    /// Where those files went: the deleted folder's parent (`None`: in no folder).
    pub to: Option<String>,
}

/// A folder name as kept: control characters become spaces, the ends are trimmed and it is cut to
/// [`MAX_NAME`] characters. Empty when nothing is left.
pub fn clean_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// Names compare without regard to case or surrounding space.
fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl Projects {
    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    fn folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    /// The folders directly in `parent` (`None`: the top level), by name.
    pub fn children(&self, parent: Option<&str>) -> Vec<&Folder> {
        let mut out: Vec<&Folder> = self.folders.iter().filter(|f| f.parent.as_deref() == parent).collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// `id` and the folders it is in, the top-level one first (empty for an unknown id).
    pub fn ancestors(&self, id: &str) -> Vec<&Folder> {
        let mut out = Vec::new();
        let mut at = self.folder(id);
        while let Some(f) = at {
            if out.len() > MAX_DEPTH || out.iter().any(|o: &&Folder| o.id == f.id) {
                break;
            }
            out.push(f);
            at = f.parent.as_deref().and_then(|p| self.folder(p));
        }
        out.reverse();
        out
    }

    /// How deep `id` is (a top-level folder: 1).
    pub fn depth(&self, id: &str) -> usize {
        self.ancestors(id).len()
    }

    /// `id` and every folder inside it, at any depth.
    pub fn descendants(&self, id: &str) -> Vec<String> {
        if self.folder(id).is_none() {
            return Vec::new();
        }
        let mut out = vec![id.to_string()];
        let mut seen: HashSet<String> = out.iter().cloned().collect();
        let mut i = 0;
        while let Some(at) = out.get(i).cloned() {
            for f in &self.folders {
                if f.parent.as_deref() == Some(at.as_str()) && seen.insert(f.id.clone()) {
                    out.push(f.id.clone());
                }
            }
            i += 1;
        }
        out
    }

    /// The deepest a folder inside `id` sits below it (0: it has none).
    fn height(&self, id: &str) -> usize {
        self.descendants(id).iter().map(|d| self.depth(d).saturating_sub(self.depth(id))).max().unwrap_or(0)
    }

    pub fn file(&self, path: &str) -> Option<&ProjectFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// The files directly in `folder` (`None`: in no folder).
    pub fn files_in<'a>(&'a self, folder: Option<&'a str>) -> impl Iterator<Item = &'a ProjectFile> + 'a {
        self.files.iter().filter(move |f| f.folder.as_deref() == folder)
    }

    /// How many files are in `id`, and in the folders inside it when `deep`.
    pub fn count_in(&self, id: &str, deep: bool) -> usize {
        if deep {
            let all: HashSet<String> = self.descendants(id).into_iter().collect();
            self.files.iter().filter(|f| f.folder.as_ref().is_some_and(|x| all.contains(x))).count()
        } else {
            self.files_in(Some(id)).count()
        }
    }

    /// "Northwind / Social media": where `id` is, for a file's detail line and search.
    pub fn folder_path(&self, id: &str) -> String {
        self.ancestors(id).iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" / ")
    }

    pub fn is_favorite(&self, fav: &Favorite) -> bool {
        self.favorites.contains(fav)
    }

    /// Whether a folder in `parent` other than `except` is called `name` (ignoring case).
    pub fn name_taken(&self, parent: Option<&str>, name: &str, except: Option<&str>) -> bool {
        self.folders.iter().any(|f| f.parent.as_deref() == parent && Some(f.id.as_str()) != except && same_name(&f.name, name))
    }

    /// `name`, or `name 2`, `name 3`… when another folder in `parent` has it.
    fn unique_name(&self, parent: Option<&str>, name: &str, except: Option<&str>) -> String {
        if !self.name_taken(parent, name, except) {
            return name.to_string();
        }
        (2..)
            .map(|n| {
                let suffix = format!(" {n}");
                let room = MAX_NAME.saturating_sub(suffix.chars().count());
                format!("{}{suffix}", name.chars().take(room).collect::<String>().trim_end())
            })
            .find(|candidate| !self.name_taken(parent, candidate, except))
            .unwrap_or_else(|| name.to_string())
    }

    /// A new folder called `name` in `parent` (another folder there with that name: `name 2`…).
    /// Its id.
    pub fn create_folder(&mut self, name: &str, parent: Option<&str>, now: u64) -> Result<String, String> {
        if self.folders.len() >= MAX_FOLDERS {
            return Err(format!("the library holds at most {MAX_FOLDERS} folders"));
        }
        if let Some(p) = parent {
            if self.folder(p).is_none() {
                return Err(format!("no folder {p}"));
            }
            if self.depth(p) >= MAX_DEPTH {
                return Err(format!("folders nest at most {MAX_DEPTH} deep"));
            }
        }
        let clean = clean_name(name);
        let name = self.unique_name(parent, if clean.is_empty() { UNTITLED } else { &clean }, None);
        let id = self.new_id();
        self.folders.push(Folder { id: id.clone(), name, parent: parent.map(str::to_string), created: now });
        self.unsaved = true;
        Ok(id)
    }

    fn new_id(&mut self) -> String {
        loop {
            let id = format!("f{}", self.next_id);
            self.next_id = self.next_id.saturating_add(1);
            if self.folder(&id).is_none() {
                return id;
            }
        }
    }

    /// Call `id` `name`. Refused when the name is empty or another folder beside it has it.
    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err("a folder needs a name".into());
        }
        let parent = self.folder(id).ok_or_else(|| format!("no folder {id}"))?.parent.clone();
        if self.name_taken(parent.as_deref(), &name, Some(id)) {
            return Err(format!("another folder here is called {name}"));
        }
        if let Some(f) = self.folder_mut(id)
            && f.name != name
        {
            f.name = name;
            self.unsaved = true;
        }
        Ok(())
    }

    /// Put folder `id` in `parent` (`None`: the top level), with everything in it. Refused into
    /// itself or a folder inside it, or past [`MAX_DEPTH`]; a folder already there with its name
    /// gives it a number.
    pub fn move_folder(&mut self, id: &str, parent: Option<&str>) -> Result<(), String> {
        let current = self.folder(id).ok_or_else(|| format!("no folder {id}"))?.parent.clone();
        if current.as_deref() == parent {
            return Ok(());
        }
        if let Some(p) = parent {
            if self.folder(p).is_none() {
                return Err(format!("no folder {p}"));
            }
            if self.descendants(id).iter().any(|d| d == p) {
                return Err("a folder can't go inside itself".into());
            }
            if self.depth(p) + 1 + self.height(id) > MAX_DEPTH {
                return Err(format!("folders nest at most {MAX_DEPTH} deep"));
            }
        }
        let name = self.folder(id).map(|f| f.name.clone()).unwrap_or_default();
        let name = self.unique_name(parent, &name, Some(id));
        if let Some(f) = self.folder_mut(id) {
            f.parent = parent.map(str::to_string);
            f.name = name;
        }
        self.unsaved = true;
        Ok(())
    }

    /// Delete folder `id` and the folders inside it. Their files are kept: they go to the deleted
    /// folder's parent (in no folder at the top level). Favourites of the deleted folders go.
    pub fn delete_folder(&mut self, id: &str) -> Result<Deleted, String> {
        let to = self.folder(id).ok_or_else(|| format!("no folder {id}"))?.parent.clone();
        let gone: HashSet<String> = self.descendants(id).into_iter().collect();
        let mut files = 0;
        for f in &mut self.files {
            if f.folder.as_ref().is_some_and(|x| gone.contains(x)) {
                f.folder = to.clone();
                files += 1;
            }
        }
        self.folders.retain(|f| !gone.contains(&f.id));
        self.favorites.retain(|fav| !matches!(fav, Favorite::Folder(x) if gone.contains(x)));
        self.unsaved = true;
        Ok(Deleted { folders: gone.len(), files, to })
    }

    /// Know `path` (a file opened or saved): added in no folder when new; `kind` noted when given.
    /// Whether anything changed.
    pub fn touch(&mut self, path: &str, kind: Option<FileKind>, now: u64) -> bool {
        if path.is_empty() {
            return false;
        }
        if let Some(f) = self.files.iter_mut().find(|f| f.path == path) {
            if kind.is_some() && f.kind != kind {
                f.kind = kind;
                self.unsaved = true;
                return true;
            }
            return false;
        }
        if self.files.len() >= MAX_FILES {
            return false;
        }
        self.files.push(ProjectFile { path: path.to_string(), folder: None, added: now, kind });
        self.unsaved = true;
        true
    }

    /// Put the files at `paths` in `folder` (`None`: take them out of their folders), adding the
    /// ones not known yet. How many changed place.
    pub fn move_files(&mut self, paths: &[String], folder: Option<&str>, now: u64) -> Result<usize, String> {
        if let Some(id) = folder
            && self.folder(id).is_none()
        {
            return Err(format!("no folder {id}"));
        }
        if paths.iter().any(String::is_empty) {
            return Err("a file needs a path".into());
        }
        let new = paths.iter().filter(|p| self.file(p).is_none()).collect::<BTreeSet<_>>().len();
        if self.files.len() + new > MAX_FILES {
            return Err(format!("the library holds at most {MAX_FILES} files"));
        }
        let mut changed = 0;
        for path in paths {
            match self.files.iter_mut().find(|f| f.path == *path) {
                Some(f) if f.folder.as_deref() == folder => {}
                Some(f) => {
                    f.folder = folder.map(str::to_string);
                    changed += 1;
                }
                None => {
                    self.files.push(ProjectFile { path: path.clone(), folder: folder.map(str::to_string), added: now, kind: None });
                    changed += 1;
                }
            }
        }
        if changed > 0 {
            self.unsaved = true;
        }
        Ok(changed)
    }

    /// Forget the files at `paths` (the files themselves stay on disk), and their favourites.
    /// How many were known.
    pub fn remove_files(&mut self, paths: &[String]) -> usize {
        let before = self.files.len();
        self.files.retain(|f| !paths.contains(&f.path));
        self.favorites.retain(|fav| !matches!(fav, Favorite::File(p) if paths.contains(p)));
        let removed = before - self.files.len();
        if removed > 0 {
            self.unsaved = true;
        }
        removed
    }

    /// Add `fav` to Favorites (at the end) or take it out. A file favourited joins the library.
    /// Whether anything changed.
    pub fn set_favorite(&mut self, fav: &Favorite, on: bool, now: u64) -> Result<bool, String> {
        match fav {
            Favorite::Folder(id) if self.folder(id).is_none() => return Err(format!("no folder {id}")),
            Favorite::File(path) if path.is_empty() => return Err("a file needs a path".into()),
            _ => {}
        }
        if self.is_favorite(fav) == on {
            return Ok(false);
        }
        if on {
            if let Favorite::File(path) = fav {
                self.touch(path, None, now);
            }
            self.favorites.push(fav.clone());
        } else {
            self.favorites.retain(|f| f != fav);
        }
        self.unsaved = true;
        Ok(true)
    }

    /// The file known as `from` is at `to` now (found again after it moved): its folder and
    /// favourite follow. Refused when `to` is known already.
    pub fn relink(&mut self, from: &str, to: &str) -> Result<(), String> {
        if to.is_empty() {
            return Err("a file needs a path".into());
        }
        if from == to {
            return Ok(());
        }
        if self.file(to).is_some() {
            return Err(format!("{to} is in the library already"));
        }
        let f = self.files.iter_mut().find(|f| f.path == from).ok_or_else(|| format!("{from} isn't in the library"))?;
        f.path = to.to_string();
        for fav in &mut self.favorites {
            if *fav == Favorite::File(from.to_string()) {
                *fav = Favorite::File(to.to_string());
            }
        }
        self.unsaved = true;
        Ok(())
    }

    /// Make every reference good, whatever a file on disk said: ids and paths unique, names clean,
    /// parents that exist without cycles or more than [`MAX_DEPTH`] levels, files in folders that
    /// exist, favourites of things that exist (a favourite file joins the library), the caps kept.
    /// How many things it changed.
    pub fn repair(&mut self) -> usize {
        let mut fixes = 0;
        // Unique folder ids and clean names.
        let mut ids = HashSet::new();
        let before = self.folders.len();
        self.folders.retain(|f| !f.id.is_empty() && ids.insert(f.id.clone()));
        self.folders.truncate(MAX_FOLDERS);
        fixes += before - self.folders.len();
        for f in &mut self.folders {
            let clean = clean_name(&f.name);
            let clean = if clean.is_empty() { UNTITLED.to_string() } else { clean };
            if clean != f.name {
                f.name = clean;
                fixes += 1;
            }
        }
        // Parents that exist, no cycles, not too deep: a folder that breaks this goes to the top.
        let known: HashSet<String> = self.folders.iter().map(|f| f.id.clone()).collect();
        for f in &mut self.folders {
            if f.parent.as_ref().is_some_and(|p| !known.contains(p) || *p == f.id) {
                f.parent = None;
                fixes += 1;
            }
        }
        let parents: HashMap<String, Option<String>> = self.folders.iter().map(|f| (f.id.clone(), f.parent.clone())).collect();
        let mut lifted = HashSet::new();
        for f in &self.folders {
            let mut seen = HashSet::from([f.id.clone()]);
            let mut at = f.parent.clone();
            while let Some(p) = at {
                if !seen.insert(p.clone()) || seen.len() > MAX_DEPTH {
                    lifted.insert(f.id.clone());
                    break;
                }
                at = parents.get(&p).cloned().flatten();
            }
        }
        for f in &mut self.folders {
            if lifted.contains(&f.id) {
                f.parent = None;
                fixes += 1;
            }
        }
        // Unique paths, in folders that exist.
        let mut paths = HashSet::new();
        let before = self.files.len();
        self.files.retain(|f| !f.path.is_empty() && paths.insert(f.path.clone()));
        self.files.truncate(MAX_FILES);
        fixes += before - self.files.len();
        for f in &mut self.files {
            if f.folder.as_ref().is_some_and(|x| !known.contains(x)) {
                f.folder = None;
                fixes += 1;
            }
        }
        // Favourites of things that exist, once each.
        let mut seen = HashSet::new();
        let before = self.favorites.len();
        self.favorites.retain(|fav| {
            seen.insert(fav.clone())
                && match fav {
                    Favorite::Folder(id) => known.contains(id),
                    Favorite::File(path) => !path.is_empty(),
                }
        });
        fixes += before - self.favorites.len();
        let missing: Vec<String> = self
            .favorites
            .iter()
            .filter_map(|f| if let Favorite::File(p) = f { Some(p.clone()) } else { None })
            .filter(|p| self.file(p).is_none())
            .collect();
        for path in missing {
            if self.touch(&path, None, 0) {
                fixes += 1;
            } else {
                self.favorites.retain(|f| *f != Favorite::File(path.clone()));
                fixes += 1;
            }
        }
        // New ids never take an old one.
        let highest = self.folders.iter().filter_map(|f| f.id.strip_prefix('f')?.parse::<u64>().ok()).max().unwrap_or(0);
        if self.next_id <= highest {
            self.next_id = highest.saturating_add(1);
            fixes += 1;
        }
        self.next_id = self.next_id.max(1);
        if self.version != VERSION {
            self.version = VERSION;
        }
        fixes
    }

    /// A library read from its file's text, repaired ([`Projects::repair`]), with how many things
    /// the repair changed. `Err` when the text isn't a library at all.
    pub fn from_json(text: &str) -> Result<(Projects, usize), String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if !value.is_object() {
            return Err("not a project library".into());
        }
        let mut projects: Projects = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let fixes = projects.repair();
        projects.unsaved = false;
        Ok((projects, fixes))
    }

    /// The library as its file keeps it.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}
