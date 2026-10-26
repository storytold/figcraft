//! The home screen's project library, drawn headless: the sidebar's sections and Favorites, a
//! folder's page and its breadcrumb, New folder, Delete folder (its files are kept), the context
//! menu's favourites and Move to folder, dragging a file onto a folder, grid and list keeping the
//! folder and the filters, empty pages, the search, a missing file, and a save that fails.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2, Shape};
use figcraft_engine::Session;
use figcraft_engine::projects::{Favorite, Projects};
use serde_json::Value;

use crate::Services;
use crate::home_projects::KindFilter;
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center, texts};

#[derive(Default)]
struct Log {
    saved: Vec<Projects>,
    /// Saving fails while this is set.
    fail: bool,
    revealed: usize,
}

struct Watch(Arc<Mutex<Log>>);
impl Services for Watch {
    fn save_projects(&mut self, projects: &Projects) -> Result<(), String> {
        let mut log = self.0.lock().unwrap();
        if log.fail {
            return Err("disk full".into());
        }
        log.saved.push(projects.clone());
        Ok(())
    }
    fn reveal_path(&mut self, _: &std::path::Path) {
        self.0.lock().unwrap().revealed += 1;
    }
}

/// A library on disk: `brand.figcraft` in NORTHWIND BUILDERS (a favourite), `post.figcraft` in no
/// folder, an empty Social Media folder in Northwind, and `missing.figcraft`, which isn't there.
struct Lib {
    u: Ui,
    log: Arc<Mutex<Log>>,
    northwind: String,
    social: String,
    brand: String,
    post: String,
    gone: String,
    dir: PathBuf,
}

fn lib(tag: &str) -> Lib {
    let dir = std::env::temp_dir().join(format!("figcraft-home-projects-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, figcraft_format::save_bytes(&figcraft_doc::Document::new("File"), None).unwrap()).unwrap();
        path.to_string_lossy().into_owned()
    };
    let (brand, post) = (write("brand.figcraft"), write("post.figcraft"));
    let gone = dir.join("missing.figcraft").to_string_lossy().into_owned();
    let mut s = Session::empty();
    s.prefs.getting_started_dismissed = true;
    let northwind = s.projects.create_folder("NORTHWIND BUILDERS", None, 1).unwrap();
    let social = s.projects.create_folder("Social Media", Some(&northwind), 2).unwrap();
    s.projects.move_files(&[brand.clone(), gone.clone()], Some(&northwind), 3).unwrap();
    s.projects.touch(&post, None, 4);
    s.projects.set_favorite(&Favorite::Folder(northwind.clone()), true, 5).unwrap();
    s.projects.unsaved = false;
    let mut u = Ui::new(s);
    u.app.ui.home = true;
    let log = Arc::new(Mutex::new(Log::default()));
    u.app.services = Box::new(Watch(log.clone()));
    settle(&mut u);
    Lib { u, log, northwind, social, brand, post, gone, dir }
}

impl Drop for Lib {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn projects(u: &Ui) -> Value {
    u.app.inspect()["homeProjects"].clone()
}

/// Frames until the files' dates and pictures are in (read on a thread of their own).
fn settle(u: &mut Ui) -> Vec<Shape> {
    for _ in 0..60 {
        u.frame(vec![]);
        std::thread::sleep(Duration::from_millis(2));
    }
    shapes(u)
}

fn shapes(u: &mut Ui) -> Vec<Shape> {
    flat(&u.frame_output(vec![]).shapes)
}

fn click_with(u: &mut Ui, p: Pos2, button: PointerButton) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn click(u: &mut Ui, p: Pos2) {
    click_with(u, p, PointerButton::Primary);
}

/// The middle of the last text `s` drawn (a dialog's button over its title, a menu over the page).
fn last_center(s: &[Shape], text: &str) -> Pos2 {
    let (_, p, size) = texts(s).into_iter().rev().find(|(t, _, _)| t == text).unwrap_or_else(|| panic!("no text {text:?}"));
    p + size / 2.0
}

/// Click the sidebar row called `text` (the sidebar is the left 240 points).
fn sidebar(u: &mut Ui, text: &str) -> Vec<Shape> {
    let s = shapes(u);
    let (_, p, size) = texts(&s).into_iter().find(|(t, p, _)| t == text && p.x < 240.0).unwrap_or_else(|| panic!("no {text:?} in the sidebar"));
    click(u, p + size / 2.0);
    settle(u)
}

fn type_text(u: &mut Ui, text: &str) {
    u.frame(vec![Event::Text(text.into())]);
    u.frame(vec![]);
}

#[test]
fn the_sidebar_has_the_library_and_its_favourites_open_a_folder() {
    let mut l = lib("sidebar");
    let s = shapes(&mut l.u);
    for item in ["Recents", "All projects", "All folders", "Favorites", "NORTHWIND BUILDERS"] {
        assert!(has_text(&s, item), "{item}");
    }
    assert_eq!(projects(&l.u)["favorites"], serde_json::json!(["NORTHWIND BUILDERS"]));
    let s = sidebar(&mut l.u, "NORTHWIND BUILDERS");
    assert_eq!(l.u.app.inspect()["homeSection"], "folder");
    assert_eq!(projects(&l.u)["folder"], l.northwind.as_str());
    // Its folder, its files (one of them missing) and the breadcrumb back to every folder.
    for item in ["All folders", "Social Media", "brand", "missing", "File not found"] {
        assert!(has_text(&s, item), "{item}: {:?}", texts(&s).iter().map(|t| &t.0).collect::<Vec<_>>());
    }
    assert_eq!(projects(&l.u)["files"], serde_json::json!(["brand", "missing"]));
    // Folding Favorites away (its heading: the last "Favorites" in the sidebar) hides its rows.
    let s = shapes(&mut l.u);
    let (_, p, size) = texts(&s).into_iter().rfind(|(t, p, _)| t == "Favorites" && p.x < 240.0).unwrap();
    click(&mut l.u, p + size / 2.0);
    assert_eq!(projects(&l.u)["favoritesOpen"], false);
    let s = shapes(&mut l.u);
    assert!(!texts(&s).iter().any(|(t, p, _)| t == "NORTHWIND BUILDERS" && p.x < 240.0));
}

#[test]
fn a_new_folder_is_named_in_its_dialog_and_kept() {
    let mut l = lib("new");
    let s = sidebar(&mut l.u, "All folders");
    assert!(has_text(&s, "NORTHWIND BUILDERS"));
    click(&mut l.u, text_center(&s, "New folder"));
    assert_eq!(projects(&l.u)["dialog"]["kind"], "newFolder");
    type_text(&mut l.u, "LUMEN");
    let s = shapes(&mut l.u);
    click(&mut l.u, last_center(&s, "Create"));
    let lib = &l.u.app.session.projects;
    let lumen = lib.children(None).into_iter().find(|f| f.name == "LUMEN").expect("made").id.clone();
    assert_eq!(projects(&l.u)["folder"], lumen.as_str(), "its page shows");
    settle(&mut l.u);
    assert!(l.log.lock().unwrap().saved.last().is_some_and(|p| p.folder(&lumen).is_some()), "kept at once");
    // Empty, it says so.
    let s = shapes(&mut l.u);
    assert!(has_text(&s, "This folder is empty"));
}

#[test]
fn renaming_a_folder_refuses_a_name_beside_it() {
    let mut l = lib("rename");
    crate::home_projects::open_folder(&mut l.u.app, &l.social.clone());
    l.u.app.ui.home_view.projects.dialog = Some(crate::home_projects::Dialog::Rename { id: l.social.clone(), name: "Social Media".into() });
    let _ = l.u.app.session.projects.create_folder("Apresentações", Some(&l.northwind), 9);
    l.u.app.ui.home_view.projects.dialog = Some(crate::home_projects::Dialog::Rename { id: l.social.clone(), name: "apresentações".into() });
    let s = settle(&mut l.u);
    assert!(has_text(&s, "Another folder here has this name."));
    click(&mut l.u, last_center(&s, "Rename"));
    assert_eq!(l.u.app.session.projects.folder(&l.social).unwrap().name, "Social Media", "refused");
    l.u.app.ui.home_view.projects.dialog = Some(crate::home_projects::Dialog::Rename { id: l.social.clone(), name: "Social".into() });
    let s = settle(&mut l.u);
    click(&mut l.u, last_center(&s, "Rename"));
    assert_eq!(l.u.app.session.projects.folder(&l.social).unwrap().name, "Social");
}

#[test]
fn deleting_a_folder_asks_and_keeps_its_files() {
    let mut l = lib("delete");
    let s = sidebar(&mut l.u, "All folders");
    click_with(&mut l.u, text_center(&s, "NORTHWIND BUILDERS"), PointerButton::Secondary);
    let s = shapes(&mut l.u);
    click(&mut l.u, text_center(&s, "Delete folder…"));
    let s = settle(&mut l.u);
    for line in ["Delete the folder NORTHWIND BUILDERS?", "The 1 folder inside it is deleted too.", "No file is deleted from your disk."] {
        assert!(has_text(&s, line), "{line}: {:?}", texts(&s).iter().map(|t| &t.0).collect::<Vec<_>>());
    }
    click(&mut l.u, last_center(&s, "Delete folder"));
    let p = &l.u.app.session.projects;
    assert!(p.folders.is_empty());
    assert_eq!(p.files.len(), 3, "every file is still in the library");
    assert!(p.files.iter().all(|f| f.folder.is_none()));
    assert!(std::path::Path::new(&l.brand).exists(), "and on disk");
    assert!(p.favorites.is_empty());
}

#[test]
fn the_file_menu_adds_to_favorites_and_moves_to_a_folder() {
    let mut l = lib("menu");
    let s = sidebar(&mut l.u, "All projects");
    click_with(&mut l.u, text_center(&s, "post"), PointerButton::Secondary);
    let s = shapes(&mut l.u);
    for item in ["Open", "Add to favorites", "Move to folder", "Remove from projects…"] {
        assert!(has_text(&s, item), "{item}");
    }
    click(&mut l.u, text_center(&s, "Add to favorites"));
    assert!(l.u.app.session.projects.is_favorite(&Favorite::File(l.post.clone())));
    let s = settle(&mut l.u);
    assert!(texts(&s).iter().any(|(t, p, _)| t == "post" && p.x < 240.0), "in the sidebar");
    // Move to folder ▸ NORTHWIND BUILDERS.
    click_with(&mut l.u, last_center(&s, "post"), PointerButton::Secondary);
    let s = shapes(&mut l.u);
    let row = text_center(&s, "Move to folder");
    for _ in 0..30 {
        l.u.frame(vec![Event::PointerMoved(row)]);
    }
    let s = shapes(&mut l.u);
    click(&mut l.u, text_center(&s, "    NORTHWIND BUILDERS"));
    assert_eq!(l.u.app.session.projects.file(&l.post).unwrap().folder.as_deref(), Some(l.northwind.as_str()));
    // And out again.
    let s = settle(&mut l.u);
    click_with(&mut l.u, last_center(&s, "post"), PointerButton::Secondary);
    let s = shapes(&mut l.u);
    click(&mut l.u, text_center(&s, "Remove from folder"));
    assert_eq!(l.u.app.session.projects.file(&l.post).unwrap().folder, None);
    assert!(std::path::Path::new(&l.post).exists());
}

#[test]
fn a_file_dragged_onto_a_folder_goes_in_it() {
    let mut l = lib("drag");
    crate::home_projects::open_folder(&mut l.u.app, &l.northwind.clone());
    let s = settle(&mut l.u);
    let (from, to) = (text_center(&s, "brand"), text_center(&s, "Social Media"));
    l.u.drag_to(from, to);
    l.u.button(to, false);
    settle(&mut l.u);
    assert_eq!(l.u.app.session.projects.file(&l.brand).unwrap().folder.as_deref(), Some(l.social.as_str()));
}

#[test]
fn grid_and_list_keep_the_folder_and_the_filters() {
    let mut l = lib("views");
    crate::home_projects::open_folder(&mut l.u.app, &l.northwind.clone());
    l.u.app.ui.home_view.projects.kind = KindFilter::Design;
    l.u.app.ui.home_view.projects.filter = "bra".into();
    let s = settle(&mut l.u);
    assert_eq!(projects(&l.u)["view"], "grid");
    assert!(has_text(&s, "brand") && !has_text(&s, "missing"), "filtered");
    // The list button, at the right end of the toolbar row.
    let y = text_center(&s, "1 file").y;
    click(&mut l.u, Pos2::new(1400.0 - 14.0, y));
    let s = settle(&mut l.u);
    let v = projects(&l.u);
    assert_eq!(
        (v["view"].as_str(), v["folder"].as_str(), v["kind"].as_str(), v["filter"].as_str()),
        (Some("list"), Some(l.northwind.as_str()), Some("design"), Some("bra"))
    );
    for header in ["Name", "Type", "Folder", "Last modified"] {
        assert!(has_text(&s, header), "{header}");
    }
    assert!(has_text(&s, "brand") && !has_text(&s, "missing"));
    assert_eq!(l.u.app.session.prefs.home_views.get("projects").map(String::as_str), Some("list"), "kept in the preferences");
    assert_eq!(l.u.app.session.prefs.home_views.get("recents"), None, "the recent files keep their own");
    // Nothing matching: said so, with a way back.
    l.u.app.ui.home_view.projects.filter = "zzz".into();
    let s = settle(&mut l.u);
    assert!(has_text(&s, "Nothing here matches"));
    click(&mut l.u, text_center(&s, "Clear filters"));
    assert_eq!(l.u.app.ui.home_view.projects.kind, KindFilter::All);
    assert!(l.u.app.ui.home_view.projects.filter.is_empty());
}

#[test]
fn the_search_finds_folders_and_library_files() {
    let mut l = lib("search");
    l.u.app.ui.home_view.file_query = "social".into();
    let s = settle(&mut l.u);
    assert!(has_text(&s, "Folders") && has_text(&s, "Social Media"));
    click(&mut l.u, text_center(&s, "Social Media"));
    assert_eq!(projects(&l.u)["folder"], l.social.as_str());
    assert!(l.u.app.ui.home_view.file_query.is_empty());
    // A file found by its folder's name.
    l.u.app.ui.home_view.file_query = "northwind".into();
    let s = settle(&mut l.u);
    assert!(has_text(&s, "Projects") && has_text(&s, "brand"));
}

#[test]
fn a_missing_file_offers_to_be_located_and_is_never_dropped() {
    let mut l = lib("missing");
    crate::home_projects::open_folder(&mut l.u.app, &l.northwind.clone());
    let s = settle(&mut l.u);
    click_with(&mut l.u, text_center(&s, "missing"), PointerButton::Secondary);
    let s = shapes(&mut l.u);
    assert!(has_text(&s, "Locate file…"));
    for _ in 0..30 {
        l.u.frame(vec![]);
    }
    assert!(l.u.app.session.projects.file(&l.gone).is_some(), "still known");
}

#[test]
fn a_failed_save_is_said_once_and_tried_again() {
    let mut l = lib("failing");
    l.log.lock().unwrap().fail = true;
    l.u.app.run("projects.createFolder", serde_json::json!({"name": "HARBOR CONSTRUCTION"}));
    l.u.frame(vec![]);
    let toast = l.u.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert!(toast.contains("Couldn't save your folders and favorites") && toast.contains("disk full"), "{toast}");
    assert!(l.u.app.session.projects.unsaved, "still to be kept");
    l.u.app.ui.toast = None;
    for _ in 0..10 {
        l.u.frame(vec![]);
    }
    assert!(l.u.app.ui.toast.is_none(), "said once, not every frame");
    l.log.lock().unwrap().fail = false;
    l.u.advance_clock(1000.0);
    l.u.frame(vec![]);
    assert!(!l.u.app.session.projects.unsaved);
    assert!(l.log.lock().unwrap().saved.last().is_some_and(|p| p.children(None).iter().any(|f| f.name == "HARBOR CONSTRUCTION")));
    assert!(l.u.app.ui.toast.as_ref().is_some_and(|t| t.0 == "Your folders and favorites are saved again"));
}

#[test]
fn favorites_show_folders_and_files_and_go_away_without_deleting() {
    let mut l = lib("favorites");
    l.u.app.run("projects.favorite", serde_json::json!({"path": l.post.clone(), "on": true}));
    // The Favorites page.
    l.u.app.ui.home_view.section = crate::home::Section::Favorites;
    let s = settle(&mut l.u);
    assert!(has_text(&s, "NORTHWIND BUILDERS") && has_text(&s, "post"));
    l.u.app.run("projects.favorite", serde_json::json!({"folder": l.northwind.clone(), "on": false}));
    let s = settle(&mut l.u);
    assert!(!texts(&s).iter().any(|(t, p, _)| t == "NORTHWIND BUILDERS" && p.x >= 240.0), "off the page");
    assert!(l.u.app.session.projects.folder(&l.northwind).is_some(), "the folder stays");
}
