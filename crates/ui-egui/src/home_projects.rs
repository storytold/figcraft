//! The home screen's project library ([`figcraft_engine::projects`]): All projects, All folders,
//! Favorites and each folder's own page (its folders and files as cards or a table, under a
//! breadcrumb), the sidebar's Favorites, the dialogs that name, rename and delete folders, the
//! library in the home search, and dragging files onto folders. Everything acts through the
//! `projects.*` commands: a file on disk is never moved, renamed or deleted from here.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use egui::{Align2, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::projects::{Favorite, Folder, Projects};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::home::{Section, focus_ring};
use crate::home_recents::{Action, Menu, Shown, Sort, SortKey, View, card_heights, grid_columns};
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The preferences' key for the library pages' grid or list (one for all of them).
const VIEW: &str = "projects";
/// The design files Locate and Add file pick.
const PROJECT_EXTENSIONS: &[&str] = &["figcraft", "fig", "sketch"];
/// How long a failed save waits before it is tried again (seconds).
const RETRY_SECS: f64 = 10.0;

/// The library has files to organise only where there are paths (not in the browser).
pub fn available() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// What a library page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KindFilter {
    #[default]
    All,
    Folders,
    Design,
    Board,
}

impl KindFilter {
    pub const ALL: [KindFilter; 4] = [KindFilter::All, KindFilter::Folders, KindFilter::Design, KindFilter::Board];

    pub fn id(self) -> &'static str {
        match self {
            KindFilter::All => "all",
            KindFilter::Folders => "folders",
            KindFilter::Design => "design",
            KindFilter::Board => "board",
        }
    }

    pub fn parse(id: &str) -> Option<KindFilter> {
        KindFilter::ALL.into_iter().find(|k| k.id() == id)
    }

    fn label(self) -> &'static str {
        match self {
            KindFilter::All => tr_noop("All types"),
            KindFilter::Folders => tr_noop("Folders"),
            KindFilter::Design => tr_noop("Design files"),
            KindFilter::Board => tr_noop("Boards"),
        }
    }

    fn folders(self) -> bool {
        matches!(self, KindFilter::All | KindFilter::Folders)
    }

    fn files(self) -> bool {
        self != KindFilter::Folders
    }
}

/// A dialog of the library open on the home screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    /// A new folder in `parent`; `files` go in it once it's made.
    NewFolder {
        parent: Option<String>,
        name: String,
        files: Vec<String>,
    },
    Rename {
        id: String,
        name: String,
    },
    Delete {
        id: String,
    },
    /// Remove files from the library (they stay on disk).
    Forget {
        paths: Vec<String>,
    },
}

/// The library's own state on the home screen.
#[derive(Default)]
pub struct ProjectsUi {
    /// What is known about the files the library pages show (their pictures, their dates).
    pub files: crate::home_recents::Recents,
    pub kind: KindFilter,
    /// The filter typed on a library page (the page's own items only).
    pub filter: String,
    pub dialog: Option<Dialog>,
    /// The dialog's field takes the keyboard when it opens.
    focus_dialog: bool,
    /// The sidebar's Favorites folded away.
    pub favorites_closed: bool,
    /// Favourite folders unfolded in the sidebar (their folders show under them).
    pub expanded: BTreeSet<String>,
    /// When saving the library last failed (input time), so it isn't tried every frame.
    failed: Option<f64>,
}

/// What reading the library at startup found (the desktop app tells the user).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectsNotice {
    /// References that pointed nowhere were repaired; the file as it was is kept at `kept`.
    Repaired { fixes: usize, kept: Option<String> },
    /// The library came back from its last good copy (the damaged file is kept at `kept`).
    Restored { kept: Option<String> },
    /// The library was damaged and had no good copy: it starts empty (the file is kept at `kept`).
    Damaged { kept: Option<String> },
    /// The library's file couldn't be read: it is left alone and nothing is saved over it.
    Unreadable { error: String },
}

impl FigApp {
    /// Tell the user what reading the library found.
    pub fn note_projects(&mut self, notice: &ProjectsNotice) {
        let kept = |k: &Option<String>| k.clone().unwrap_or_default();
        let msg = match notice {
            ProjectsNotice::Repaired { fixes, kept: k } => {
                trn("Your folders had {n} broken references: they were repaired (the original file is kept at {path})", *fixes, &[("path", &kept(k))])
            }
            ProjectsNotice::Restored { kept: k } => {
                trf("Your folders and favorites were restored from their last good copy (the damaged file is kept at {path})", &[("path", &kept(k))])
            }
            ProjectsNotice::Damaged { kept: k } => trf(
                "Your folders and favorites couldn't be read and start empty; no file was deleted (the damaged file is kept at {path})",
                &[("path", &kept(k))],
            ),
            ProjectsNotice::Unreadable { error } => {
                trf("Your folders and favorites couldn't be read, so changes to them won't be saved: {error}", &[("error", error)])
            }
        };
        self.toast(msg);
    }

    /// Keep the library when it changed. A failure is said once and tried again a little later.
    pub(crate) fn save_projects(&mut self) {
        if !self.session.projects.unsaved {
            return;
        }
        let now = self.ui_context.as_ref().map_or(0.0, |c| c.input(|i| i.time));
        let state = &mut self.ui.home_view.projects;
        if state.failed.is_some_and(|at| now - at < RETRY_SECS && now >= at) {
            return;
        }
        self.session.projects.unsaved = false;
        match self.services.save_projects(&self.session.projects) {
            Ok(()) => {
                if self.ui.home_view.projects.failed.take().is_some() {
                    self.toast(tr("Your folders and favorites are saved again"));
                }
            }
            Err(e) => {
                log::warn!("projects: couldn't save: {e}");
                self.session.projects.unsaved = true;
                let first = self.ui.home_view.projects.failed.replace(now).is_none();
                if first {
                    self.toast(trf("Couldn't save your folders and favorites: {error}", &[("error", &e)]));
                }
            }
        }
    }
}

/// The folders as a tree, depth first, each with its depth (0 at the top), by name.
pub(crate) fn folder_tree(p: &Projects) -> Vec<(usize, &Folder)> {
    fn walk<'a>(p: &'a Projects, parent: Option<&str>, depth: usize, out: &mut Vec<(usize, &'a Folder)>, seen: &mut HashSet<&'a str>) {
        for f in p.children(parent) {
            if seen.insert(f.id.as_str()) {
                out.push((depth, f));
                walk(p, Some(&f.id), depth + 1, out, seen);
            }
        }
    }
    let mut out = Vec::new();
    walk(p, None, 0, &mut out, &mut HashSet::new());
    out
}

/// A file dragged from a card or row.
#[derive(Clone, Debug)]
struct DraggedFile(String);

/// A file's card or row can be dragged onto a folder: a label follows the pointer.
pub(crate) fn drag_file(ui: &Ui, resp: &Response, path: &str) {
    if !available() {
        return;
    }
    resp.dnd_set_drag_payload(DraggedFile(path.to_string()));
    if resp.dragged()
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        let t = Tokens::get(ui.ctx());
        let name = Path::new(path).file_stem().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("home_drag_file")));
        let galley = painter.layout_no_wrap(name, semibold(12.0), t.text);
        let r = Rect::from_min_size(at + vec2(14.0, 10.0), galley.size() + vec2(36.0, 14.0));
        painter.rect_filled(r, CornerRadius::same(6), t.panel);
        painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
        icons::paint(&painter, Rect::from_min_size(r.min + vec2(8.0, (r.height() - 14.0) / 2.0), vec2(14.0, 14.0)), Icon::Page, t.icon);
        painter.galley(r.min + vec2(28.0, 7.0), galley, t.text);
    }
}

/// `resp` takes files dropped on it: lit while one is held over it; the file dropped.
fn dropped_on(ui: &Ui, t: &Tokens, resp: &Response, rect: Rect, radius: u8) -> Option<String> {
    if resp.dnd_hover_payload::<DraggedFile>().is_some() {
        ui.painter().rect_stroke(rect, CornerRadius::same(radius), Stroke::new(2.0, t.accent), StrokeKind::Inside);
    }
    resp.dnd_release_payload::<DraggedFile>().map(|f| f.0.clone())
}

/// The star that marks a favourite, centred on `at`.
pub(crate) fn favorite_badge(p: &egui::Painter, t: &Tokens, at: egui::Pos2) {
    icons::paint(p, Rect::from_center_size(at, vec2(14.0, 14.0)), Icon::StarFilled, t.warning);
}

/// The library part of a file's context menu: favourites, Move to folder, Remove from folder,
/// Locate file (when it is missing).
pub(crate) fn file_menu(ui: &mut Ui, p: &Projects, path: &str, missing: bool) -> Option<Action> {
    let t = Tokens::get(ui.ctx());
    let mut action = None;
    let favorite = p.is_favorite(&Favorite::File(path.to_string()));
    if crate::right::menu_item(ui, if favorite { tr("Remove from favorites") } else { tr("Add to favorites") }, "") {
        action = Some(Action::Favorite(path.to_string(), !favorite));
    }
    let current = p.file(path).and_then(|f| f.folder.clone());
    let moved = crate::menus::submenu(ui, &format!("home_move_file:{path}"), tr("Move to folder"), &t, |ui| {
        let mut picked = None;
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (depth, f) in folder_tree(p) {
                let label = format!("{}{}", "    ".repeat(depth.min(8)), f.name);
                if crate::right::menu_check(ui, current.as_deref() == Some(f.id.as_str()), &label, "") {
                    picked = Some(Action::MoveTo(vec![path.to_string()], Some(f.id.clone())));
                }
            }
        });
        if !p.folders.is_empty() {
            ui.separator();
        }
        if crate::right::menu_item(ui, tr("New folder…"), "") {
            picked = Some(Action::NewFolderFor(vec![path.to_string()]));
        }
        picked
    });
    action = action.or(moved);
    if current.is_some() && crate::right::menu_item(ui, tr("Remove from folder"), "") {
        action = Some(Action::MoveTo(vec![path.to_string()], None));
    }
    if missing && p.file(path).is_some() && crate::right::menu_item(ui, tr("Locate file…"), "") {
        action = Some(Action::Locate(path.to_string()));
    }
    action
}

/// What was asked of a folder.
enum FolderAction {
    Open(String),
    NewIn(String),
    Rename(String),
    Favorite(String, bool),
    Move(String, Option<String>),
    Delete(String),
    /// A file dropped on it.
    Drop(String, String),
}

/// A folder's menu: Open (in a context menu), New folder, Rename, Favorites, Move to, Delete.
fn folder_menu(ui: &mut Ui, t: &Tokens, p: &Projects, id: &str, with_open: bool) -> Option<FolderAction> {
    let mut action = None;
    let mut item = |ui: &mut Ui, label: &str, a: FolderAction| {
        if crate::right::menu_item(ui, label, "") {
            action = Some(a);
            ui.close();
        }
    };
    if with_open {
        item(ui, tr("Open"), FolderAction::Open(id.to_string()));
        ui.separator();
    }
    item(ui, tr("New folder inside…"), FolderAction::NewIn(id.to_string()));
    item(ui, tr("Rename…"), FolderAction::Rename(id.to_string()));
    let favorite = p.is_favorite(&Favorite::Folder(id.to_string()));
    item(ui, if favorite { tr("Remove from favorites") } else { tr("Add to favorites") }, FolderAction::Favorite(id.to_string(), !favorite));
    let inside: HashSet<String> = p.descendants(id).into_iter().collect();
    let parent = p.folder(id).and_then(|f| f.parent.clone());
    let moved = crate::menus::submenu(ui, &format!("home_move_folder:{id}"), tr("Move to"), t, |ui| {
        let mut picked = None;
        if crate::right::menu_check(ui, parent.is_none(), tr("Top level"), "") {
            picked = Some(FolderAction::Move(id.to_string(), None));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (depth, f) in folder_tree(p).into_iter().filter(|(_, f)| !inside.contains(&f.id)) {
                let label = format!("{}{}", "    ".repeat(depth.min(8)), f.name);
                if crate::right::menu_check(ui, parent.as_deref() == Some(f.id.as_str()), &label, "") {
                    picked = Some(FolderAction::Move(id.to_string(), Some(f.id.clone())));
                }
            }
        });
        picked
    });
    if moved.is_some() {
        action = moved;
        ui.close();
    }
    ui.separator();
    if crate::right::menu_item(ui, tr("Delete folder…"), "") {
        action = Some(FolderAction::Delete(id.to_string()));
        ui.close();
    }
    action
}

/// Show folder `id`'s page.
pub(crate) fn open_folder(app: &mut FigApp, id: &str) {
    app.ui.home_view.section = Section::Folder;
    app.ui.home_view.folder = Some(id.to_string());
    app.ui.home_view.file_query.clear();
    app.ui.home_view.projects.filter.clear();
}

fn folder_action(app: &mut FigApp, action: FolderAction) {
    match action {
        FolderAction::Open(id) => open_folder(app, &id),
        FolderAction::NewIn(id) => new_folder_dialog(app, Some(id), Vec::new()),
        FolderAction::Rename(id) => {
            let name = app.session.projects.folder(&id).map(|f| f.name.clone()).unwrap_or_default();
            open_dialog(app, Dialog::Rename { id, name });
        }
        FolderAction::Favorite(id, on) => {
            app.run("projects.favorite", json!({"folder": id, "on": on}));
        }
        FolderAction::Move(id, parent) => {
            if let Err(e) = app.session.execute("projects.moveFolder", &json!({"id": id, "parent": parent})) {
                app.toast(e.to_string());
            }
        }
        FolderAction::Delete(id) => open_dialog(app, Dialog::Delete { id }),
        FolderAction::Drop(path, id) => {
            let name = app.session.projects.folder(&id).map(|f| f.name.clone()).unwrap_or_default();
            if app.run("projects.moveFiles", json!({"paths": [path], "folder": id})).is_some_and(|r| r["moved"].as_u64() > Some(0)) {
                app.toast(trf("Moved to {folder}", &[("folder", &name)]));
            }
        }
    }
}

fn open_dialog(app: &mut FigApp, dialog: Dialog) {
    app.ui.home_view.projects.dialog = Some(dialog);
    app.ui.home_view.projects.focus_dialog = true;
}

/// Ask for a new folder's name (in `parent`; `files` go in it).
pub(crate) fn new_folder_dialog(app: &mut FigApp, parent: Option<String>, files: Vec<String>) {
    open_dialog(app, Dialog::NewFolder { parent, name: String::new(), files });
}

/// Ask before removing `paths` from the library.
pub(crate) fn confirm_forget(app: &mut FigApp, paths: Vec<String>) {
    open_dialog(app, Dialog::Forget { paths });
}

/// Pick where the file known as `path` is now.
pub(crate) fn locate(app: &mut FigApp, path: String) {
    app.pick_open(tr("Locate file"), PROJECT_EXTENSIONS, move |app, to| {
        let to = to.to_string_lossy().into_owned();
        match app.session.execute("projects.relink", &json!({"from": path, "to": to})) {
            Ok(_) => app.toast(tr("File located: it keeps its folder and favorite")),
            Err(e) => app.toast(e.to_string()),
        }
    });
}

/// The file name without its folder or extension.
fn stem(path: &str) -> String {
    let p = Path::new(path);
    p.file_stem().or_else(|| p.file_name()).map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------------------------
// The sidebar's Favorites.

/// A row of the sidebar's Favorites.
enum Row<'a> {
    Folder { folder: &'a Folder, depth: usize, children: bool, open: bool },
    File { path: &'a str },
}

/// The sidebar's Favorites heading (folds them away) and its rows in `area` (scrolling when they
/// don't fit).
pub(crate) fn sidebar(app: &mut FigApp, ui: &mut Ui, t: &Tokens, area: Rect) {
    if !available() || area.height() < 40.0 {
        return;
    }
    let head = Rect::from_min_size(area.min, vec2(area.width(), 28.0));
    let resp = ui.interact(head, ui.scope_id().with("home_favorites_head"), Sense::click());
    let closed = app.ui.home_view.projects.favorites_closed;
    a11y::button(&resp, tr("Favorites"));
    if resp.hovered() {
        ui.painter().rect_filled(head, CornerRadius::same(5), t.hover);
    }
    focus_ring(ui, t, head, &resp, 5);
    let chevron = if closed { Icon::ChevronRight } else { Icon::Chevron };
    icons::paint(ui.painter(), Rect::from_center_size(pos2(head.min.x + 14.0, head.center().y), vec2(10.0, 10.0)), chevron, t.icon_secondary);
    ui.painter().text(pos2(head.min.x + 26.0, head.center().y), Align2::LEFT_CENTER, tr("Favorites"), semibold(11.0), t.text_secondary);
    if resp.clicked() {
        app.ui.home_view.projects.favorites_closed = !closed;
    }
    if closed {
        return;
    }
    let list = Rect::from_min_max(pos2(area.min.x, head.max.y + 2.0), area.max);
    let mut action = None;
    let mut open_file = None;
    let mut toggle = None;
    let lib = &app.session.projects;
    if lib.favorites.is_empty() {
        let galley =
            ui.painter().layout(tr("Add folders and files here from their menus").to_string(), regular(11.0), t.text_tertiary, list.width() - 28.0);
        ui.painter().galley(pos2(list.min.x + 14.0, list.min.y + 6.0), galley, t.text_tertiary);
        return;
    }
    let expanded = &app.ui.home_view.projects.expanded;
    let mut rows = Vec::new();
    for fav in &lib.favorites {
        match fav {
            Favorite::Folder(id) => {
                let Some(folder) = lib.folder(id) else { continue };
                let mut stack = vec![(folder, 0usize)];
                while let Some((f, depth)) = stack.pop() {
                    let children = lib.children(Some(&f.id));
                    let open = expanded.contains(&f.id) && depth < 8;
                    rows.push(Row::Folder { folder: f, depth, children: !children.is_empty(), open });
                    if open {
                        stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
                    }
                }
            }
            Favorite::File(path) => {
                if figcraft_engine::file_access::allowed(path, figcraft_engine::file_access::Access::Read) {
                    rows.push(Row::File { path: path.as_str() });
                }
            }
        }
    }
    let current = (app.ui.home_view.section == Section::Folder && app.ui.home_view.file_query.trim().is_empty())
        .then(|| app.ui.home_view.folder.clone())
        .flatten();
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        egui::ScrollArea::vertical().id_salt("home_favorites").auto_shrink([false, false]).show(ui, |ui| {
            for (i, row) in rows.iter().enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(list.width(), 30.0), Sense::hover());
                let (key, name, depth) = match row {
                    Row::Folder { folder, depth, .. } => (format!("folder:{}:{i}", folder.id), folder.name.clone(), *depth),
                    Row::File { path } => (format!("file:{path}"), stem(path), 0),
                };
                let resp = ui.interact(r, Id::unique(("home_favorite", key)), Sense::click());
                let on = matches!(row, Row::Folder { folder, .. } if current.as_deref() == Some(folder.id.as_str()));
                a11y::tab(&resp, &name, on);
                if on || resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(5), if on { t.selected_row } else { t.hover });
                }
                focus_ring(ui, t, r, &resp, 5);
                let x = r.min.x + 12.0 + depth as f32 * 14.0;
                if let Row::Folder { children: true, open, folder, .. } = row {
                    let c = Rect::from_center_size(pos2(x + 4.0, r.center().y), vec2(14.0, 14.0));
                    let hit = ui.interact(c, Id::unique(("home_favorite_fold", &folder.id, i)), Sense::click());
                    a11y::button(&hit, if *open { tr("Fold") } else { tr("Unfold") });
                    icons::paint(ui.painter(), c.shrink(2.0), if *open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
                    if hit.clicked() {
                        toggle = Some(folder.id.clone());
                    }
                }
                let icon = match row {
                    Row::Folder { .. } => Icon::Folder,
                    Row::File { .. } => Icon::Page,
                };
                icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 20.0, r.center().y), vec2(16.0, 16.0)), icon, t.icon);
                let text = widgets::elide_font(ui, &name, r.max.x - x - 40.0, regular(12.0));
                ui.painter().text(pos2(x + 34.0, r.center().y), Align2::LEFT_CENTER, text, regular(12.0), t.text);
                match row {
                    Row::Folder { folder, .. } => {
                        if let Some(path) = dropped_on(ui, t, &resp, r, 5) {
                            action = Some(FolderAction::Drop(path, folder.id.clone()));
                        }
                        if resp.clicked() && toggle.is_none() {
                            action = Some(FolderAction::Open(folder.id.clone()));
                        }
                        crate::right::context_popup(&resp).show(|ui| {
                            if let Some(a) = folder_menu(ui, t, lib, &folder.id, true) {
                                action = Some(a);
                            }
                        });
                    }
                    Row::File { path } => {
                        let resp = resp.on_hover_text(*path);
                        if resp.clicked() {
                            open_file = Some(Action::Open(path.to_string()));
                        }
                        crate::right::context_popup(&resp).show(|ui| {
                            if crate::right::menu_item(ui, tr("Open"), "") {
                                open_file = Some(Action::Open(path.to_string()));
                                ui.close();
                            }
                            if crate::right::menu_item(ui, tr("Remove from favorites"), "") {
                                open_file = Some(Action::Favorite(path.to_string(), false));
                                ui.close();
                            }
                        });
                    }
                }
            }
        });
    });
    if let Some(id) = toggle {
        let set = &mut app.ui.home_view.projects.expanded;
        if !set.remove(&id) {
            set.insert(id);
        }
    }
    if let Some(a) = action {
        folder_action(app, a);
    }
    if let Some(a) = open_file {
        crate::home_recents::apply(app, a, VIEW);
    }
}

// ---------------------------------------------------------------------------------------------
// The pages.

/// The library page the home screen shows.
#[derive(Clone, Debug, PartialEq)]
enum Page {
    All,
    Folders,
    Favorites,
    Folder(String),
}

impl Page {
    fn of(app: &FigApp) -> Page {
        match app.ui.home_view.section {
            Section::Folders => Page::Folders,
            Section::Favorites => Page::Favorites,
            Section::Folder => match app.ui.home_view.folder.as_deref().filter(|id| app.session.projects.folder(id).is_some()) {
                Some(id) => Page::Folder(id.to_string()),
                None => Page::Folders,
            },
            _ => Page::All,
        }
    }
}

/// What a page lists: its folders and the paths of its files, before the filters.
fn contents(p: &Projects, page: &Page, kind: KindFilter) -> (Vec<String>, Vec<String>) {
    let readable = |path: &&str| figcraft_engine::file_access::allowed(path, figcraft_engine::file_access::Access::Read);
    match page {
        Page::All => {
            // Folders show here only when asked for (Type ▸ Folders): every one of them.
            let folders = if kind == KindFilter::Folders { folder_tree(p).into_iter().map(|(_, f)| f.id.clone()).collect() } else { Vec::new() };
            (folders, p.files.iter().map(|f| f.path.as_str()).filter(readable).map(str::to_string).collect())
        }
        Page::Folders => (p.children(None).iter().map(|f| f.id.clone()).collect(), Vec::new()),
        Page::Favorites => {
            let folders = p.favorites.iter().filter_map(|f| if let Favorite::Folder(id) = f { Some(id.clone()) } else { None }).collect();
            let files = p
                .favorites
                .iter()
                .filter_map(|f| if let Favorite::File(path) = f { Some(path.as_str()) } else { None })
                .filter(readable)
                .map(str::to_string)
                .collect();
            (folders, files)
        }
        Page::Folder(id) => (
            p.children(Some(id)).iter().map(|f| f.id.clone()).collect(),
            p.files_in(Some(id)).map(|f| f.path.as_str()).filter(readable).map(str::to_string).collect(),
        ),
    }
}

/// `folders` in `sort` order: by name, or by when they were made.
fn sort_folders(p: &Projects, mut folders: Vec<String>, sort: Sort) -> Vec<String> {
    let name = |id: &String| p.folder(id).map(|f| f.name.to_lowercase()).unwrap_or_default();
    let created = |id: &String| p.folder(id).map_or(0, |f| f.created);
    folders.sort_by(|a, b| {
        let o = match sort.key {
            SortKey::Name => name(a).cmp(&name(b)),
            _ => created(a).cmp(&created(b)).then_with(|| name(a).cmp(&name(b))),
        };
        if sort.descending { o.reverse() } else { o }
    });
    folders
}

/// Whether a file of kind `kind` (`None`: not known yet, read as a design file) shows under `filter`.
fn kind_matches(filter: KindFilter, kind: figcraft_doc::FileKind) -> bool {
    match filter {
        KindFilter::All => true,
        KindFilter::Folders => false,
        KindFilter::Design => kind == figcraft_doc::FileKind::Design,
        KindFilter::Board => kind == figcraft_doc::FileKind::Board,
    }
}

/// The library pages: a heading (a breadcrumb for a folder), their toolbar, their folders and their
/// files, and what to do when there is nothing to show.
pub(crate) fn section(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let page = Page::of(app);
    if app.ui.home_view.section == Section::Folder && !matches!(page, Page::Folder(_)) {
        // The folder was deleted: back to the folders.
        app.ui.home_view.section = Section::Folders;
        app.ui.home_view.folder = None;
    }
    let mut folder_act = header(app, ui, t, &page);
    let time = ui.input(|i| i.time);
    let now = figcraft_engine::cmd::comment::now_secs();
    let reduce_motion = app.reduce_motion();
    let (view, sort) = (View::of_section(&app.session.prefs, VIEW), Sort::of(&app.session.prefs));
    let kind = app.ui.home_view.projects.kind;
    let query = app.ui.home_view.projects.filter.trim().to_lowercase();
    let lib = &app.session.projects;
    let (folders, paths) = contents(lib, &page, kind);
    let state = &mut app.ui.home_view.projects.files;
    state.refresh(ui.ctx(), &paths, time);
    let mut files: Vec<Shown> = crate::home_recents::sorted(state.shown(&paths), sort);
    // The library remembers what each file turned out to be (a board).
    let learned: Vec<(String, figcraft_doc::FileKind)> = files
        .iter()
        .filter(|f| f.kind == figcraft_doc::FileKind::Board && lib.file(&f.path).is_some_and(|x| x.kind != Some(f.kind)))
        .map(|f| (f.path.clone(), f.kind))
        .collect();
    for f in &mut files {
        if let Some(k) = lib.file(&f.path).and_then(|x| x.kind) {
            f.kind = k;
        }
    }
    let folder_hit = |id: &String| query.is_empty() || lib.folder(id).is_some_and(|f| f.name.to_lowercase().contains(&query));
    let folders: Vec<String> = if kind.folders() { sort_folders(lib, folders.into_iter().filter(folder_hit).collect(), sort) } else { Vec::new() };
    files.retain(|f| {
        kind.files()
            && kind_matches(kind, f.kind)
            && (query.is_empty()
                || f.name.to_lowercase().contains(&query)
                || lib.file(&f.path).and_then(|x| x.folder.as_deref()).is_some_and(|id| lib.folder_path(id).to_lowercase().contains(&query)))
    });
    let with_created = files.iter().any(|f| f.stat.is_some_and(|s| s.created.is_some()));
    let count = match (folders.len(), files.len()) {
        (0, n) => trn("{n} files", n, &[]),
        (n, 0) => trn("{n} folders", n, &[]),
        (d, f) => format!("{} · {}", trn("{n} folders", d, &[]), trn("{n} files", f, &[])),
    };
    let (mut action, free) = crate::home_recents::toolbar(ui, t, &count, view, sort, with_created);
    filters(&mut app.ui.home_view.projects, ui, t, free);
    ui.add_space(16.0);
    let state = &mut app.ui.home_view.projects.files;
    let lib = &app.session.projects;
    if !folders.is_empty() {
        if let Some(a) = match view {
            View::Grid => folder_grid(ui, t, lib, &folders),
            View::List => folder_list(ui, t, lib, &folders, now),
        } {
            folder_act = Some(a);
        }
        if !files.is_empty() {
            ui.add_space(12.0);
        }
    }
    if !files.is_empty() {
        let menu = Menu { projects: lib, library: true };
        let chosen = match view {
            View::Grid => crate::home_recents::grid(ui, t, state, &files, now, time, reduce_motion, &menu),
            View::List => crate::home_recents::list(ui, t, state, &files, now, sort, with_created, &menu),
        };
        action = action.or(chosen);
    }
    if folders.is_empty()
        && files.is_empty()
        && let Some(a) = empty(app, ui, t, &page)
    {
        folder_act = Some(a);
    }
    for (path, kind) in learned {
        app.session.projects.touch(&path, Some(kind), now);
    }
    if let Some(a) = action {
        crate::home_recents::apply(app, a, VIEW);
    }
    if let Some(a) = folder_act {
        folder_action(app, a);
    }
}

/// The page's title (a breadcrumb and the folder's menu for a folder) and its buttons: New folder
/// and Add from disk.
fn header(app: &mut FigApp, ui: &mut Ui, t: &Tokens, page: &Page) -> Option<FolderAction> {
    let mut action = None;
    let lib = &app.session.projects;
    if let Page::Folder(id) = page {
        // All folders › Northwind › Social: each step goes back there.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
            if link(ui, t, tr("All folders"), "home_crumb_root") {
                app.ui.home_view.section = Section::Folders;
            }
            for f in lib.ancestors(id).iter().rev().skip(1).rev() {
                ui.label(egui::RichText::new("›").font(regular(12.0)).color(t.text_tertiary));
                if link(ui, t, &f.name, &format!("home_crumb_{}", f.id)) {
                    action = Some(FolderAction::Open(f.id.clone()));
                }
            }
        });
        ui.add_space(6.0);
    }
    let title = match page {
        Page::All => tr("All projects").to_string(),
        Page::Folders => tr("All folders").to_string(),
        Page::Favorites => tr("Favorites").to_string(),
        Page::Folder(id) => lib.folder(id).map(|f| f.name.clone()).unwrap_or_default(),
    };
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let end = ui.painter().text(pos2(row.min.x, row.center().y), Align2::LEFT_CENTER, &title, semibold(18.0), t.text).max.x;
    if let Page::Folder(id) = page {
        // The folder's own menu, under the chevron beside its name.
        let menu = Rect::from_center_size(pos2(end + 16.0, row.center().y), vec2(24.0, 24.0));
        let resp = ui.interact(menu, Id::unique(("home_folder_menu", id)), Sense::click());
        a11y::menu_button(&resp, &trf("Actions for {name}", &[("name", &title)]), a11y::menu_open(&resp));
        if resp.hovered() {
            ui.painter().rect_filled(menu, CornerRadius::same(5), t.hover);
        }
        focus_ring(ui, t, menu, &resp, 5);
        icons::paint(ui.painter(), menu.shrink(6.0), Icon::Chevron, t.icon);
        if lib.is_favorite(&Favorite::Folder(id.clone())) {
            favorite_badge(ui.painter(), t, pos2(menu.max.x + 12.0, row.center().y));
        }
        crate::right::menu_popup(&resp).show(|ui| {
            if let Some(a) = folder_menu(ui, t, lib, id, false) {
                action = Some(a);
            }
        });
    }
    // At the right: Add from disk and New folder.
    let parent = if let Page::Folder(id) = page { Some(id.clone()) } else { None };
    let mut buttons = Rect::from_min_max(pos2(row.max.x - 340.0, row.min.y), row.max);
    buttons.min.x = buttons.min.x.max(end + 48.0);
    let (mut new, mut add_dir, mut add_file) = (false, false, false);
    ui.scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        if !matches!(page, Page::Favorites) {
            new = widgets::button(ui, tr("New folder"), true, true).clicked();
        }
        let add_w = ui.painter().layout_no_wrap(tr("Add").to_string(), regular(11.0), t.text).size().x + 40.0;
        let resp = widgets::dropdown(ui, tr("Add"), add_w.max(72.0), None);
        crate::right::menu_popup(&resp).show(|ui| {
            if crate::right::menu_item(ui, tr("Add folder from disk…"), "") {
                add_dir = true;
                ui.close();
            }
            if crate::right::menu_item(ui, tr("Add file…"), "") {
                add_file = true;
                ui.close();
            }
        });
    });
    if new {
        new_folder_dialog(app, parent.clone(), Vec::new());
    }
    if add_dir {
        add_folder(app, parent.clone());
    }
    if add_file {
        add_file_dialog(app, parent);
    }
    action
}

/// A line of text that acts like a link; true when chosen.
fn link(ui: &mut Ui, t: &Tokens, text: &str, key: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), regular(12.0), t.text_secondary);
    let (r, _) = ui.allocate_exact_size(galley.size() + vec2(4.0, 6.0), Sense::hover());
    let resp = ui.interact(r, Id::unique(("home_link", key)), Sense::click());
    a11y::button(&resp, text);
    let ink = if resp.hovered() { t.text } else { t.text_secondary };
    ui.painter().text(r.left_center() + vec2(2.0, 0.0), Align2::LEFT_CENTER, text, regular(12.0), ink);
    if resp.hovered() {
        ui.painter().line_segment([pos2(r.min.x + 2.0, r.max.y - 2.0), pos2(r.max.x - 2.0, r.max.y - 2.0)], Stroke::new(1.0, ink));
    }
    focus_ring(ui, t, r, &resp, 3);
    resp.clicked()
}

/// Pick a folder on disk and mirror it in the library (in `parent`), then show it.
fn add_folder(app: &mut FigApp, parent: Option<String>) {
    app.pick_folder(tr("Add folder from disk"), move |app, dir| {
        match app.session.execute("projects.importFolder", &json!({"path": dir.to_string_lossy(), "folder": parent})) {
            Ok(r) => {
                let files = usize::try_from(r["files"].as_u64().unwrap_or(0)).unwrap_or(0);
                app.toast(trn("Added {n} files: they stay where they are on disk", files, &[]));
                if r["capped"] == true {
                    app.toast(tr("Only part of that folder was added: it holds more files or folders than one go reads"));
                }
                if let Some(id) = r["folder"].as_str() {
                    open_folder(app, id);
                }
            }
            Err(e) => app.toast(e.to_string()),
        }
    });
}

/// Pick a design file and put it in `folder` (in no folder at the top level).
fn add_file_dialog(app: &mut FigApp, folder: Option<String>) {
    app.pick_open(tr("Add file"), PROJECT_EXTENSIONS, move |app, path| {
        if let Err(e) = app.session.execute("projects.moveFiles", &json!({"paths": [path.to_string_lossy()], "folder": folder})) {
            app.toast(e.to_string());
        }
    });
}

/// The search field and the Type menu, at the right end of `free` (the toolbar's room).
fn filters(state: &mut ProjectsUi, ui: &mut Ui, t: &Tokens, free: Rect) {
    let label = tr(state.kind.label());
    let w = (ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.text).size().x + 40.0).max(100.0);
    let menu = Rect::from_min_size(pos2(free.max.x - w, free.min.y), vec2(w, widgets::FIELD_H));
    ui.scope_builder(egui::UiBuilder::new().max_rect(menu), |ui| {
        let resp = widgets::dropdown(ui, label, w, None);
        crate::right::menu_popup(&resp).show(|ui| {
            for k in KindFilter::ALL {
                if crate::right::menu_check(ui, state.kind == k, tr(k.label()), "") {
                    state.kind = k;
                    ui.close();
                }
            }
        });
    });
    let field_w = 180.0_f32.min(menu.min.x - free.min.x - 140.0);
    if field_w >= 80.0 {
        let field = Rect::from_min_size(pos2(menu.min.x - 8.0 - field_w, free.min.y), vec2(field_w, widgets::FIELD_H));
        ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
            let resp = widgets::text_input(ui, Id::unique("home_projects_filter"), &mut state.filter, field_w, tr("Filter this page"));
            a11y::text(&resp, tr("Filter this page"), &state.filter);
            if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                state.filter.clear();
            }
        });
    }
}

/// "3 files · 2 folders" under a folder's name.
fn folder_detail(p: &Projects, id: &str) -> String {
    let (folders, files) = (p.children(Some(id)).len(), p.count_in(id, false));
    match (folders, files) {
        (0, n) => trn("{n} files", n, &[]),
        (d, 0) => trn("{n} folders", d, &[]),
        (d, n) => format!("{} · {}", trn("{n} folders", d, &[]), trn("{n} files", n, &[])),
    }
}

/// A folder's card or row: a click opens it, a right-click offers its menu, files drop on it.
fn folder_interact(ui: &mut Ui, t: &Tokens, p: &Projects, id: &str, resp: &Response, rect: Rect, radius: u8) -> Option<FolderAction> {
    let mut action = None;
    if let Some(path) = dropped_on(ui, t, resp, rect, radius) {
        action = Some(FolderAction::Drop(path, id.to_string()));
    }
    if resp.clicked() {
        action = Some(FolderAction::Open(id.to_string()));
    }
    crate::right::context_popup(resp).show(|ui| {
        if let Some(a) = folder_menu(ui, t, p, id, true) {
            action = Some(a);
        }
    });
    action
}

fn folder_grid(ui: &mut Ui, t: &Tokens, p: &Projects, folders: &[String]) -> Option<FolderAction> {
    let (columns, card_w) = grid_columns(ui.available_width());
    let (picture_h, card_h) = card_heights(card_w);
    // Folders take a shorter card: their picture is a mark, not a page.
    let (picture_h, card_h) = ((picture_h * 0.55).round(), card_h - picture_h + (picture_h * 0.55).round());
    let mut action = None;
    for row in folders.chunks(columns) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, id) in row.iter().enumerate() {
            let Some(f) = p.folder(id) else { continue };
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + crate::home_recents::GAP), r.min.y), vec2(card_w, card_h));
            let resp = ui.interact(rect, Id::unique(("home_folder_card", id)), Sense::click());
            let detail = folder_detail(p, id);
            a11y::named_button(&resp, &format!("{}, {detail}", f.name));
            let resp = resp.on_hover_text(p.folder_path(id));
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(8), if resp.hovered() { t.hover } else { t.panel });
            let area = Rect::from_min_size(rect.min, vec2(rect.width(), picture_h)).shrink(1.0);
            painter.rect_filled(area, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, t.field);
            icons::paint(painter, Rect::from_center_size(area.center(), vec2(40.0, 40.0)), Icon::Folder, t.icon_secondary);
            if p.is_favorite(&Favorite::Folder(id.clone())) {
                favorite_badge(painter, t, pos2(area.max.x - 18.0, area.min.y + 18.0));
            }
            painter.line_segment([pos2(rect.min.x, area.max.y + 0.5), pos2(rect.max.x, area.max.y + 0.5)], Stroke::new(1.0, t.border));
            let y = area.max.y + 1.0 + (card_h - picture_h) / 2.0;
            icons::paint(painter, Rect::from_center_size(pos2(rect.min.x + 24.0, y), vec2(16.0, 16.0)), Icon::Folder, t.icon);
            let w = rect.width() - 56.0;
            painter.text(
                pos2(rect.min.x + 44.0, y - 8.0),
                Align2::LEFT_CENTER,
                widgets::elide_font(ui, &f.name, w, semibold(12.5)),
                semibold(12.5),
                t.text,
            );
            painter.text(
                pos2(rect.min.x + 44.0, y + 9.0),
                Align2::LEFT_CENTER,
                widgets::elide_font(ui, &detail, w, regular(11.5)),
                regular(11.5),
                t.text_secondary,
            );
            painter.rect_stroke(
                rect,
                CornerRadius::same(8),
                Stroke::new(1.0, if resp.hovered() { t.border_strong } else { t.border }),
                StrokeKind::Inside,
            );
            focus_ring(ui, t, rect, &resp, 8);
            if let Some(a) = folder_interact(ui, t, p, id, &resp, rect, 8) {
                action = Some(a);
            }
        }
        ui.add_space(crate::home_recents::GAP);
    }
    action
}

fn folder_list(ui: &mut Ui, t: &Tokens, p: &Projects, folders: &[String], now: u64) -> Option<FolderAction> {
    let width = ui.available_width();
    let mut action = None;
    for id in folders {
        let Some(f) = p.folder(id) else { continue };
        let (r, _) = ui.allocate_exact_size(vec2(width, 44.0), Sense::hover());
        let resp = ui.interact(r, Id::unique(("home_folder_row", id)), Sense::click());
        let detail = folder_detail(p, id);
        a11y::named_button(&resp, &format!("{}, {detail}", f.name));
        if !ui.is_rect_visible(r) {
            continue;
        }
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        focus_ring(ui, t, r, &resp, 5);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(16.0, 16.0)), Icon::Folder, t.icon);
        let name_end = ui
            .painter()
            .text(
                pos2(r.min.x + 40.0, r.center().y),
                Align2::LEFT_CENTER,
                widgets::elide_font(ui, &f.name, width * 0.45, regular(12.0)),
                regular(12.0),
                t.text,
            )
            .max
            .x;
        if p.is_favorite(&Favorite::Folder(id.clone())) {
            favorite_badge(ui.painter(), t, pos2(name_end + 12.0, r.center().y));
        }
        ui.painter().text(pos2(r.min.x + width * 0.55, r.center().y), Align2::LEFT_CENTER, &detail, regular(11.5), t.text_secondary);
        if f.created > 0 {
            let made = trf("Created {when}", &[("when", &crate::home_recents::ago(now, f.created))]);
            ui.painter().text(pos2(r.max.x - 12.0, r.center().y), Align2::RIGHT_CENTER, made, regular(11.5), t.text_secondary);
        }
        ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        if let Some(a) = folder_interact(ui, t, p, id, &resp, r, 5) {
            action = Some(a);
        }
    }
    action
}

/// What a page with nothing to show says, and what it offers.
fn empty(app: &mut FigApp, ui: &mut Ui, t: &Tokens, page: &Page) -> Option<FolderAction> {
    let filtered = !app.ui.home_view.projects.filter.trim().is_empty() || app.ui.home_view.projects.kind != KindFilter::All;
    ui.add_space(24.0);
    let (title, line) = if filtered {
        (tr("Nothing here matches"), tr("Try another filter, or show all types."))
    } else {
        match page {
            Page::All => (tr("No projects yet"), tr("Files you open or save show up here, and stay here when they leave Recents.")),
            Page::Folders => (tr("No folders yet"), tr("Make a folder for each client, or add a folder from disk as it is.")),
            Page::Favorites => (tr("No favorites yet"), tr("Add folders and files here from their menus.")),
            Page::Folder(_) => (tr("This folder is empty"), tr("Drag files here, or choose Move to folder in a file's menu.")),
        }
    };
    ui.label(egui::RichText::new(title).font(semibold(13.0)).color(t.text));
    ui.add_space(6.0);
    widgets::label(ui, line, t.text_secondary);
    ui.add_space(14.0);
    if filtered {
        if widgets::button(ui, tr("Clear filters"), false, true).clicked() {
            app.ui.home_view.projects.filter.clear();
            app.ui.home_view.projects.kind = KindFilter::All;
        }
    } else if let Page::Folder(id) = page
        && widgets::button(ui, tr("New folder inside"), false, true).clicked()
    {
        return Some(FolderAction::NewIn(id.clone()));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// The home search.

/// The folders whose name has `query` and the library's files (not open, not among the recent
/// files) whose name or folder has it, a row each. How many are listed.
pub(crate) fn search(app: &mut FigApp, ui: &mut Ui, t: &Tokens, query: &str) -> usize {
    if !available() {
        return 0;
    }
    let lib = &app.session.projects;
    let folders: Vec<&Folder> = folder_tree(lib).into_iter().map(|(_, f)| f).filter(|f| f.name.to_lowercase().contains(query)).collect();
    let recents: HashSet<&String> = crate::home_recents::listed(&app.session.prefs.recent_files).collect();
    let files: Vec<(&str, String)> = lib
        .files
        .iter()
        .filter(|f| !recents.contains(&f.path) && !app.session.docs.iter().any(|d| d.path.as_deref() == Some(f.path.as_str())))
        .filter(|f| figcraft_engine::file_access::allowed(&f.path, figcraft_engine::file_access::Access::Read))
        .map(|f| (f.path.as_str(), f.folder.as_deref().map(|id| lib.folder_path(id)).unwrap_or_default()))
        .filter(|(path, folder)| stem(path).to_lowercase().contains(query) || folder.to_lowercase().contains(query))
        .collect();
    let mut open = None;
    let mut show = None;
    if !folders.is_empty() {
        crate::home::subheading(ui, t, tr("Folders"));
        for f in &folders {
            let parent = f.parent.as_deref().map(|p| lib.folder_path(p)).unwrap_or_default();
            let row = crate::home::FileRow {
                mark: Some((Icon::Folder, t.icon)),
                name: &f.name,
                ink: t.text,
                detail: &parent,
                detail_size: 11.0,
                detail_ink: t.text_secondary,
            };
            if crate::home::listed(ui, t, Id::unique(("search_folder", &f.id)), 36.0, &row) {
                show = Some(f.id.clone());
            }
        }
    }
    if !files.is_empty() {
        crate::home::subheading(ui, t, tr("Projects"));
        for (path, folder) in &files {
            let name = stem(path);
            let detail = if folder.is_empty() { path.to_string() } else { folder.clone() };
            let row = crate::home::FileRow {
                mark: Some((Icon::Page, t.icon)),
                name: &name,
                ink: t.text,
                detail: &detail,
                detail_size: 11.0,
                detail_ink: t.text_secondary,
            };
            if crate::home::listed(ui, t, Id::unique(("search_project", path)), 36.0, &row) {
                open = Some(path.to_string());
            }
        }
    }
    let count = folders.len() + files.len();
    if let Some(id) = show {
        open_folder(app, &id);
    }
    if let Some(path) = open {
        crate::home_recents::apply(app, Action::Open(path), VIEW);
    }
    count
}

// ---------------------------------------------------------------------------------------------
// Dialogs.

/// The library's dialogs: New folder, Rename folder, Delete folder, Remove from projects.
pub fn dialogs(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.ui.home_view.projects.dialog.take() else { return };
    let t = Tokens::get(ctx);
    let focus = std::mem::take(&mut app.ui.home_view.projects.focus_dialog);
    let (mut open, mut cancel, mut go) = (true, false, false);
    let lib = &app.session.projects;
    let title = match &dialog {
        Dialog::NewFolder { .. } => tr("New folder"),
        Dialog::Rename { .. } => tr("Rename folder"),
        Dialog::Delete { .. } => tr("Delete folder"),
        Dialog::Forget { .. } => tr("Remove from projects"),
    };
    let mut problem: Option<String> = None;
    crate::dialogs::window(title, ctx).id(Id::unique("home_projects_dialog")).open(&mut open).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        match &mut dialog {
            Dialog::NewFolder { parent, name, files } => {
                let place =
                    parent.as_deref().map_or_else(|| tr("At the top level").to_string(), |p| trf("In {folder}", &[("folder", &lib.folder_path(p))]));
                widgets::label(ui, &place, t.text_secondary);
                if !files.is_empty() {
                    widgets::label(ui, &trn("{n} files go in it", files.len(), &[]), t.text_secondary);
                }
                go = name_field(ui, name, focus);
                if lib.name_taken(parent.as_deref(), name, None) {
                    widgets::label(ui, tr("Another folder here has this name: the new one gets a number."), t.text_secondary);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    go |= widgets::button(ui, tr("Create"), true, true).clicked();
                    cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
                });
            }
            Dialog::Rename { id, name } => {
                go = name_field(ui, name, focus);
                let parent = lib.folder(id).and_then(|f| f.parent.clone());
                if figcraft_engine::projects::clean_name(name).is_empty() {
                    problem = Some(tr("A folder needs a name.").into());
                } else if lib.name_taken(parent.as_deref(), name, Some(id)) {
                    problem = Some(tr("Another folder here has this name.").into());
                }
                if let Some(why) = &problem {
                    widgets::label(ui, why, t.danger);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    go |= widgets::button(ui, tr("Rename"), true, problem.is_none()).clicked();
                    cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
                });
                go &= problem.is_none();
            }
            Dialog::Delete { id } => {
                let name = lib.folder(id).map(|f| f.name.clone()).unwrap_or_default();
                let inside = lib.descendants(id).len().saturating_sub(1);
                let files = lib.count_in(id, true);
                let to = lib.folder(id).and_then(|f| f.parent.as_deref()).map(|p| lib.folder_path(p));
                widgets::label(ui, &trf("Delete the folder {name}?", &[("name", &name)]), t.text);
                if inside > 0 {
                    widgets::label(ui, &trn("The {n} folders inside it are deleted too.", inside, &[]), t.text_secondary);
                }
                let line = match to {
                    Some(to) => trn("Its {n} files move to {folder}.", files, &[("folder", &to)]),
                    None => trn("Its {n} files stay in All projects, in no folder.", files, &[]),
                };
                widgets::label(ui, &line, t.text_secondary);
                widgets::label(ui, tr("No file is deleted from your disk."), t.text_secondary);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    go = widgets::button(ui, tr("Delete folder"), true, true).clicked();
                    cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
                });
            }
            Dialog::Forget { paths } => {
                let line = match paths.as_slice() {
                    [one] => trf("Remove {name} from your projects?", &[("name", &stem(one))]),
                    many => trn("Remove {n} files from your projects?", many.len(), &[]),
                };
                widgets::label(ui, &line, t.text);
                widgets::label(
                    ui,
                    tr("Its folder and favorite are forgotten; the file stays on your disk, and opening it again brings it back."),
                    t.text_secondary,
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    go = widgets::button(ui, tr("Remove"), true, true).clicked();
                    cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
                });
            }
        }
    });
    if go {
        finish(app, dialog);
    } else if open && !cancel {
        app.ui.home_view.projects.dialog = Some(dialog);
    }
}

/// The name field of New folder and Rename; true when Enter was pressed in it.
fn name_field(ui: &mut Ui, name: &mut String, focus: bool) -> bool {
    let resp = widgets::text_input(ui, Id::unique("home_projects_name"), name, 400.0, tr("Folder name"));
    a11y::text(&resp, tr("Folder name"), name);
    if focus {
        resp.request_focus();
    }
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Do what the dialog was confirmed for.
fn finish(app: &mut FigApp, dialog: Dialog) {
    match dialog {
        Dialog::NewFolder { parent, name, files } => match app.session.execute("projects.createFolder", &json!({"name": name, "parent": parent})) {
            Ok(r) => {
                let id = r["id"].as_str().unwrap_or_default().to_string();
                if !files.is_empty() {
                    app.run("projects.moveFiles", json!({"paths": files, "folder": id}));
                } else {
                    open_folder(app, &id);
                }
            }
            Err(e) => app.toast(e.to_string()),
        },
        Dialog::Rename { id, name } => {
            if let Err(e) = app.session.execute("projects.renameFolder", &json!({"id": id, "name": name})) {
                app.toast(e.to_string());
            }
        }
        Dialog::Delete { id } => match app.session.execute("projects.deleteFolder", &json!({"id": id})) {
            Ok(r) => {
                let files = usize::try_from(r["files"].as_u64().unwrap_or(0)).unwrap_or(0);
                app.toast(trn("Folder deleted; its {n} files are kept", files, &[]));
                if app.ui.home_view.folder.as_ref().is_some_and(|f| app.session.projects.folder(f).is_none()) {
                    match r["to"].as_str() {
                        Some(to) => open_folder(app, to),
                        None => {
                            app.ui.home_view.section = Section::Folders;
                            app.ui.home_view.folder = None;
                        }
                    }
                }
            }
            Err(e) => app.toast(e.to_string()),
        },
        Dialog::Forget { paths } => {
            app.run("projects.removeFiles", json!({"paths": paths}));
        }
    }
}

/// For `ui.inspect`: the page shown, its filters, its folders and files in order, the sidebar's
/// favourites and the dialog open.
pub fn inspect(app: &FigApp) -> Value {
    let lib = &app.session.projects;
    let page = Page::of(app);
    let (folders, files) = contents(lib, &page, app.ui.home_view.projects.kind);
    let dialog = app.ui.home_view.projects.dialog.as_ref().map(|d| match d {
        Dialog::NewFolder { parent, name, files } => json!({"kind": "newFolder", "parent": parent, "name": name, "files": files}),
        Dialog::Rename { id, name } => json!({"kind": "rename", "id": id, "name": name}),
        Dialog::Delete { id } => json!({"kind": "delete", "id": id}),
        Dialog::Forget { paths } => json!({"kind": "forget", "paths": paths}),
    });
    json!({
        "folder": if let Page::Folder(id) = &page { Some(id.clone()) } else { None },
        "kind": app.ui.home_view.projects.kind.id(),
        "filter": app.ui.home_view.projects.filter,
        "view": View::of_section(&app.session.prefs, VIEW).id(),
        "folders": folders.iter().filter_map(|id| lib.folder(id)).map(|f| &f.name).collect::<Vec<_>>(),
        "files": files.iter().map(|p| stem(p)).collect::<Vec<_>>(),
        "favorites": lib.favorites.iter().map(|f| match f {
            Favorite::Folder(id) => lib.folder(id).map_or_else(String::new, |f| f.name.clone()),
            Favorite::File(path) => stem(path),
        }).collect::<Vec<_>>(),
        "favoritesOpen": !app.ui.home_view.projects.favorites_closed,
        "dialog": dialog,
        "unsaved": lib.unsaved,
    })
}
