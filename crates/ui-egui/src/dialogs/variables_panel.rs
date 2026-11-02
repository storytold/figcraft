//! The Variables editor: collections on the left (this file's, then each enabled library's,
//! read-only under the library's name); the selected collection's variables on the right as a
//! table with one value column per mode. Double-click a name to rename it, right-click for more; a
//! value's variable button turns it into an alias of another variable (a library one too).
//! A collection (this file's or a library's) can be extended: a collection of this file with its
//! modes and variables, whose overridden values are marked (with a reset). Import and export read
//! and write the variables JSON (or CSS). Everything goes through `variable.*` / `library.*`
//! commands, so it's undoable and the same over MCP.

use egui::{Align2, Context, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use figcraft_doc::{Collection, Document, PublishedLibrary, Variable, VariableType, VariableValue, Variables, scopes_for};
use figcraft_engine::library;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::colorpicker::PickerTarget;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::right::menu_item;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FieldEdit, Lead};

const SIDEBAR_W: f32 = 200.0;
const NAME_W: f32 = 240.0;
const MODE_W: f32 = 200.0;
const ROW_H: f32 = 36.0;
const TYPES: [VariableType; 4] = [VariableType::Color, VariableType::Number, VariableType::String, VariableType::Boolean];

/// What's being renamed inline.
#[derive(Clone, Debug, PartialEq, Hash)]
enum Rename {
    Collection(String),
    /// (collection, mode)
    Mode(String, String),
    Variable(String),
    /// (collection, group path)
    Group(String, String),
}

/// The open editor's state (`UiState::variables`; None = closed).
#[derive(Clone, Default)]
pub struct Editor {
    /// The library whose collection is shown (id or name); None for this file's.
    library: Option<String>,
    /// The selected collection's id.
    collection: Option<String>,
    query: String,
    rename: Option<Rename>,
    /// The rename just started: its field takes the focus in the frame it's first drawn.
    rename_start: bool,
    /// The variable whose details (name, description, scopes) are open.
    details: Option<String>,
    /// The group the table shows ("space" or "color/brand"); None = every variable.
    group: Option<String>,
}

impl Editor {
    /// An editor showing `collection` (id or name; the first one when None or unknown).
    pub fn on(collection: Option<String>) -> Self {
        Editor { collection, ..Default::default() }
    }

    /// An editor showing a collection of enabled library `library` (id or name).
    pub fn in_library(library: String, collection: Option<String>) -> Self {
        Editor { library: Some(library), collection, ..Default::default() }
    }
}

/// What the editor lists for one source: this file's variables (editable, never library copies)
/// or an enabled library's published ones (read-only).
struct View<'a> {
    vars: &'a Variables,
    /// The library's (id, name); None for this file.
    library: Option<(&'a str, &'a str)>,
    collections: Vec<&'a Collection>,
    variables: Vec<&'a Variable>,
}

impl<'a> View<'a> {
    fn local(doc: &'a Document) -> Self {
        View {
            vars: &doc.variables,
            library: None,
            collections: library::local_collections(doc).collect(),
            variables: doc.variables.variables.iter().filter(|v| !library::is_remote_variable(doc, &v.id)).collect(),
        }
    }

    fn of(lib: &'a PublishedLibrary) -> Self {
        View {
            vars: &lib.doc.variables,
            library: Some((&lib.manifest.id, &lib.manifest.name)),
            collections: library::published_collections(lib).collect(),
            variables: library::published_variables(lib).collect(),
        }
    }

    fn id(&self) -> Option<&'a str> {
        self.library.map(|l| l.0)
    }

    fn editable(&self) -> bool {
        self.library.is_none()
    }

    /// Whether the editor's `library` (id or name) names this view.
    fn is(&self, library: Option<&str>) -> bool {
        match (self.library, library) {
            (None, None) => true,
            (Some((id, name)), Some(l)) => l == id || l == name,
            _ => false,
        }
    }

    /// Collection `c` (id or name).
    fn collection(&self, c: &str) -> Option<&'a Collection> {
        self.collections.iter().copied().find(|x| x.id == c).or_else(|| self.collections.iter().copied().find(|x| x.name == c))
    }

    fn in_collection(&self, c: &str) -> impl Iterator<Item = &'a Variable> {
        self.variables.iter().copied().filter(move |v| v.collection == c)
    }
}

/// Show the editor while it's open (call once per frame).
pub fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut st) = app.ui.variables.take() else { return };
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else { return };
    let libs = crate::variable_ui::libraries(app, &doc);
    let views: Vec<View> = std::iter::once(View::local(&doc)).chain(libs.iter().map(|l| View::of(l))).collect();
    // The selected collection (its source's first when none is), else this file's first.
    let found = views.iter().find(|v| v.is(st.library.as_deref())).and_then(|v| {
        let c = match st.collection.as_deref() {
            Some(c) => v.collection(c),
            None => v.collections.first().copied(),
        };
        Some((v, c?))
    });
    let Some((view, coll)) = found.map(|(v, c)| (v, Some(c))).or_else(|| views.first().map(|v| (v, v.collections.first().copied()))) else {
        app.ui.variables = Some(st);
        return;
    };
    st.library = view.id().map(str::to_string);
    st.collection = coll.map(|c| c.id.clone());
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let size = vec2(880.0_f32.min(screen.width() - 80.0).max(480.0), 480.0_f32.min(screen.height() - 120.0).max(240.0));
    let mut open = true;
    egui::Window::new(tr("Local variables"))
        .id(Id::unique("variables_window"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(app, ui, &mut open);
            let h = (size.y - 44.0).max(160.0);
            let top_down = egui::Layout::top_down(egui::Align::Min);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(SIDEBAR_W, h), top_down, |ui| sidebar(app, ui, &views, &doc.variables, &mut st));
                let (line, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
                ui.allocate_ui_with_layout(vec2(size.x - SIDEBAR_W - 1.0, h), top_down, |ui| table(app, ui, view, &mut st));
            });
        });
    details(app, ctx, &doc.variables, &mut st);
    app.ui.variables = open.then_some(st);
}

/// Draw the variables header: the title at the left, and the import/export/Close controls at the
/// right.
fn header(app: &mut FigApp, ui: &mut Ui, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    // Import and export, left of Close.
    let bar = Rect::from_min_max(pos2(r.max.x - 280.0, r.min.y + 10.0), pos2(r.max.x - 40.0, r.max.y - 10.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let export = widgets::button(&mut bar, tr("Export…"), false, true);
    crate::right::menu_popup(&export).show(|ui| {
        for (label, format) in [(tr("Variables JSON…"), "json"), (tr("CSS custom properties…"), "css"), (tr("Dart constants for Flutter…"), "dart")]
        {
            if menu_item(ui, label, "") {
                export_dialog(app, format);
                ui.close();
            }
        }
    });
    if widgets::button(&mut bar, tr("Import…"), false, true).on_hover_text(tr("Add or update collections from a variables JSON")).clicked() {
        import_dialog(app);
    }
    ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Local variables"), semibold(13.0), t.text);
    if widgets::icon_button_at(
        ui,
        Id::unique("vars_close"),
        Rect::from_center_size(pos2(r.max.x - 22.0, r.center().y), vec2(24.0, 24.0)),
        Icon::Close,
        tr("Close"),
    ) {
        *open = false;
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

/// Export this file's collections through a save dialog, or as a download where the platform has
/// no save dialogs (the browser).
pub(crate) fn export_dialog(app: &mut FigApp, format: &str) {
    let name = app.session.doc().map(|d| format!("{}.variables.{format}", d.name)).unwrap_or_else(|_| format!("variables.{format}"));
    if app.services.downloads() {
        let Some(r) = app.run("variable.export", json!({"format": format})) else { return };
        let text = r["text"].as_str().unwrap_or_default();
        if app.services.download(&name, text.as_bytes()) {
            app.toast(trf("Exported {file}", &[("file", &name)]));
        } else {
            app.toast(tr("Couldn't download the variables"));
        }
        return;
    }
    let format = format.to_owned();
    let ext = format.clone();
    app.pick_save(tr("Export variables"), &name, &[&ext], move |app, p| {
        if app.run("variable.export", json!({"format": format, "path": p.to_string_lossy()})).is_some() {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            app.toast(trf("Exported {file}", &[("file", &file)]));
        }
    });
}

/// Import a variables JSON picked in an open dialog.
fn import_dialog(app: &mut FigApp) {
    app.pick_open(tr("Import variables"), &["json"], |app, p| {
        let Some(r) = app.run("variable.import", json!({"variables": p.to_string_lossy()})) else { return };
        let (added, updated) = (r["variables"].as_u64().unwrap_or(0), r["updated"].as_u64().unwrap_or(0));
        let skipped = r["warnings"].as_array().map_or(0, Vec::len);
        let mut msg = trn("Imported {n} new variables, updated {updated}", added as usize, &[("updated", &updated)]);
        if skipped > 0 {
            msg = format!("{msg} {}", trn("({n} warnings: some entries were skipped)", skipped, &[]));
        }
        app.toast(msg);
    });
}

/// Extend collection `c` (of library `lib`, else of this file) and show the new collection.
fn extend(app: &mut FigApp, lib: Option<&str>, c: &str, st: &mut Editor) {
    let run = match lib {
        Some(lib) => app.run("library.extendCollection", json!({"library": lib, "collection": c})),
        None => app.run("variable.extendCollection", json!({"collection": c})),
    };
    if let Some(id) = run.and_then(|r| r["id"].as_str().map(str::to_string)) {
        st.library = None;
        st.collection = Some(id);
        st.group = None;
        st.query.clear();
    }
}

/// Stable egui id for the rename field of a `Rename` target.
fn rename_id(target: &Rename) -> Id {
    Id::unique(("var_rename", target))
}

/// Start renaming `target`. The field isn't drawn yet (a new collection, mode or variable shows next
/// frame), and focusing it now would put a node that isn't there in the screen reader tree, which
/// AccessKit aborts on (#224): the field takes the focus when drawn.
fn start_rename(st: &mut Editor, target: Rename) {
    st.rename = Some(target);
    st.rename_start = true;
}

/// The inline editor of a rename in progress: the new name once committed. The rename ends when
/// the editor closes.
fn rename_field(ui: &mut Ui, st: &mut Editor, target: &Rename, value: &str, rect: Rect, font: egui::FontId) -> Option<String> {
    let start = std::mem::take(&mut st.rename_start);
    let (out, open) = widgets::rename_field(ui, rename_id(target), value, rect, font, start);
    if !open {
        st.rename = None;
    }
    out
}

/// The first of `base`, `base 2`, `base 3`… that `taken` doesn't hold.
pub(crate) fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..10_000).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Draw the collections sidebar with library headings, collection rows and a create-collection
/// button.
fn sidebar(app: &mut FigApp, ui: &mut Ui, views: &[View], local: &Variables, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (hr, _) = ui.allocate_exact_size(vec2(SIDEBAR_W, 40.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Collections"), semibold(11.0), t.text_secondary);
    if widgets::icon_button_at(
        ui,
        Id::unique("vars_add_coll"),
        Rect::from_center_size(pos2(hr.max.x - 20.0, hr.center().y), vec2(24.0, 24.0)),
        Icon::Plus,
        tr("Create collection"),
    ) {
        create_collection(app, local, st);
    }
    egui::ScrollArea::vertical().id_salt("vars_collections").auto_shrink([false, false]).show(ui, |ui| {
        for view in views {
            if let Some((_, name)) = view.library
                && !view.collections.is_empty()
            {
                library_title(ui, name);
            }
            for c in &view.collections {
                collection_row(app, ui, view, c, st);
            }
        }
    });
}

/// A library's heading in the sidebar: its collections follow, read-only.
fn library_title(ui: &mut Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    let (r, resp) = ui.allocate_exact_size(vec2(SIDEBAR_W, 26.0), Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 22.0, r.center().y), vec2(12.0, 12.0)), Icon::Library, t.icon_secondary);
    let name_r = Rect::from_min_max(pos2(r.min.x + 34.0, r.min.y), pos2(r.max.x - 8.0, r.max.y));
    ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, name, semibold(11.0), t.text_secondary);
    resp.on_hover_text(tr("Library: its variables are read-only here; edit them in the library file and publish it"));
}

/// One collection in the sidebar (with its groups while selected).
fn collection_row(app: &mut FigApp, ui: &mut Ui, view: &View, c: &Collection, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(SIDEBAR_W, 30.0), Sense::click());
    let r = r.shrink2(vec2(8.0, 1.0));
    let selected = view.is(st.library.as_deref()) && st.collection.as_deref() == Some(c.id.as_str());
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.selected_row);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    let target = Rename::Collection(c.id.clone());
    let count = view.in_collection(&c.id).count();
    ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, count.to_string(), regular(11.0), t.text_tertiary);
    let indent = if view.editable() { 8.0 } else { 26.0 };
    let name_r = Rect::from_min_max(pos2(r.min.x + indent, r.min.y + 4.0), pos2(r.max.x - 32.0, r.max.y - 4.0));
    if view.editable() && st.rename.as_ref() == Some(&target) {
        if let Some(name) = rename_field(ui, st, &target, &c.name, name_r, regular(11.0)) {
            app.run("variable.renameCollection", json!({"collection": c.id, "name": name}));
        }
    } else {
        let font = if selected { semibold(11.0) } else { regular(11.0) };
        ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, &c.name, font, t.text);
    }
    if resp.clicked() {
        if !selected {
            st.group = None;
            st.query.clear();
        }
        st.library = view.id().map(str::to_string);
        st.collection = Some(c.id.clone());
    }
    if view.editable() && resp.double_clicked() {
        start_rename(st, target.clone());
    }
    // A library's collections only extend; this file's also rename and delete.
    crate::right::context_popup(&resp).show(|ui| {
        if view.editable() && menu_item(ui, tr("Rename"), "") {
            start_rename(st, target.clone());
            ui.close();
        }
        if menu_item(ui, tr("Extend collection"), "") {
            extend(app, view.id(), &c.id, st);
            ui.close();
        }
        if view.editable() && menu_item(ui, tr("Delete collection"), "") {
            app.run("variable.deleteCollection", json!({"collection": c.id}));
            ui.close();
        }
    });
    if selected {
        group_rows(app, ui, view, c, st);
    }
}

/// Every group path in `names` with its ancestors ("a/b/c" → "a", "a/b"), sorted.
fn groups<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let mut path = group_of(name);
        while !path.is_empty() {
            if !out.iter().any(|g| g == path) {
                out.push(path.to_string());
            }
            path = group_of(path);
        }
    }
    out.sort();
    out
}

/// "All variables" and the collection's groups (indented by depth): picking one filters the
/// table; this file's groups can be renamed (every variable in them follows) or deleted.
fn group_rows(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let members: Vec<&Variable> = view.in_collection(&coll.id).collect();
    let names: Vec<&str> = members.iter().map(|v| v.name.as_str()).collect();
    let list = groups(names.iter().copied());
    if list.is_empty() {
        return;
    }
    let rows = std::iter::once(None).chain(list.iter().map(Some));
    for g in rows {
        let (r, resp) = ui.allocate_exact_size(vec2(SIDEBAR_W, 26.0), Sense::click());
        let r = r.shrink2(vec2(8.0, 0.0));
        let on = st.group.as_ref() == g;
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        } else if resp.hovered() {
            ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.border), StrokeKind::Inside);
        }
        let depth = g.map_or(0, |g| g.matches('/').count());
        let x = r.min.x + 20.0 + depth as f32 * 12.0;
        let label = g.map_or(tr("All variables"), |g| g.rsplit('/').next().unwrap_or(g));
        let count = names.iter().filter(|n| g.is_none_or(|g| n.starts_with(&format!("{g}/")))).count();
        ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, count.to_string(), regular(10.0), t.text_tertiary);
        let name_r = Rect::from_min_max(pos2(x, r.min.y + 3.0), pos2(r.max.x - 28.0, r.max.y - 3.0));
        let Some(g) = g else {
            ui.painter().text(name_r.left_center(), Align2::LEFT_CENTER, label, regular(11.0), t.text_secondary);
            if resp.clicked() {
                st.group = None;
            }
            continue;
        };
        let target = Rename::Group(coll.id.clone(), g.clone());
        if view.editable() && st.rename.as_ref() == Some(&target) {
            if let Some(new) = rename_field(ui, st, &target, label, name_r, regular(11.0)) {
                rename_group(app, &members, g, &new, st);
            }
        } else {
            ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, label, regular(11.0), t.text);
        }
        if resp.clicked() {
            st.group = Some(g.clone());
        }
        if !view.editable() {
            continue;
        }
        if resp.double_clicked() {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Rename group"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if menu_item(ui, tr("Delete group"), "") {
                let steps: Vec<serde_json::Value> = members
                    .iter()
                    .filter(|v| v.name.starts_with(&format!("{g}/")))
                    .map(|v| json!({"command": "variable.delete", "params": {"variable": v.id}}))
                    .collect();
                app.run("command.batch", json!({"label": "Delete group", "commands": steps}));
                if st.group.as_ref().is_some_and(|s| s == g || s.starts_with(&format!("{g}/"))) {
                    st.group = None;
                }
                ui.close();
            }
        });
    }
}

/// Rename the last part of group `g` of a collection (its variables `members`) to `new`: every
/// variable under it moves along (one undo step).
fn rename_group(app: &mut FigApp, members: &[&Variable], g: &str, new: &str, st: &mut Editor) {
    let new = new.trim().replace('/', "-");
    if new.is_empty() {
        return;
    }
    let renamed = match g.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new}"),
        None => new,
    };
    let prefix = format!("{g}/");
    let steps: Vec<serde_json::Value> = members
        .iter()
        .filter(|v| v.name.starts_with(&prefix))
        .map(|v| json!({"command": "variable.update", "params": {"variable": v.id, "name": format!("{renamed}/{}", &v.name[prefix.len()..])}}))
        .collect();
    if app.run("command.batch", json!({"label": "Rename group", "commands": steps})).is_some()
        && st.group.as_deref().is_some_and(|s| s == g || s.starts_with(&prefix))
    {
        st.group = st.group.as_ref().map(|s| format!("{renamed}{}", &s[g.len()..]));
    }
}

fn create_collection(app: &mut FigApp, vars: &Variables, st: &mut Editor) {
    let name = unique("Collection", |n| vars.collections.iter().any(|c| c.name == n));
    if let Some(id) =
        app.run("variable.createCollection", json!({"name": name, "modes": ["Mode 1"]})).and_then(|r| r["id"].as_str().map(str::to_string))
    {
        st.library = None;
        st.collection = Some(id.clone());
        start_rename(st, Rename::Collection(id));
    }
}

/// The group part of a variable name ("space/md" → "space"; "" when ungrouped).
fn group_of(name: &str) -> &str {
    name.rsplit_once('/').map(|(g, _)| g).unwrap_or("")
}

/// Draw the variables table for the selected collection: search, library/extended notes,
/// grouped rows, mode columns and the create-variable row when editable.
fn table(app: &mut FigApp, ui: &mut Ui, view: &View, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    // A sidebar click this frame picked another source: shown next frame.
    if !view.is(st.library.as_deref()) {
        ui.ctx().request_repaint();
        return;
    }
    let Some(coll) = st.collection.as_deref().and_then(|c| view.collection(c)) else {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.vertical(|ui| {
                widgets::label(
                    ui,
                    tr("Variables store reusable values (colours, numbers, strings, booleans) with modes such as Light and Dark."),
                    t.text_secondary,
                );
                ui.add_space(12.0);
                if widgets::button(ui, tr("Create collection"), true, true).clicked() {
                    create_collection(app, view.vars, st);
                }
            });
        });
        return;
    };
    // Search.
    let (sr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let field = Rect::from_min_size(pos2(sr.min.x + 16.0, sr.center().y - 12.0), vec2(220.0, 24.0));
    ui.painter().rect_filled(field, CornerRadius::same(5), t.field);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(field.min.x + 13.0, field.center().y), vec2(12.0, 12.0)), Icon::Search, t.icon_secondary);
    let mut child = ui.new_child(
        UiBuilder::new().max_rect(Rect::from_min_max(pos2(field.min.x + 24.0, field.min.y + 4.0), pos2(field.max.x - 4.0, field.max.y - 4.0))),
    );
    let r = child.add(
        egui::TextEdit::singleline(&mut st.query)
            .frame(egui::Frame::NONE)
            .hint_text(tr("Search"))
            .font(regular(11.0))
            .align(egui::Align2::LEFT_CENTER),
    );
    crate::widgets::select_all_on_focus(ui, &r, &st.query);
    let note = Rect::from_min_max(pos2(field.max.x + 16.0, sr.min.y + 8.0), pos2(sr.max.x - 16.0, sr.max.y - 8.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(note).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let text = match (view.library, coll.extends.as_deref().and_then(|p| view.vars.collection_by_id(p))) {
        (Some((lib, name)), _) => {
            let tip = tr("A collection of this file with these modes and variables: override their values per mode and add variables of its own");
            if widgets::button(&mut bar, tr("Extend collection"), false, true).on_hover_text(tip).clicked() {
                extend(app, Some(lib), &coll.id, st);
            }
            bar.add_space(8.0);
            Some(trf("From the library \"{name}\": read-only", &[("name", &name)]))
        }
        (None, Some(parent)) => Some(trf("Extends \"{name}\": overridden values are marked", &[("name", &parent.name)])),
        _ => None,
    };
    if let Some(text) = text {
        bar.add(egui::Label::new(egui::RichText::new(text).font(regular(11.0)).color(t.text_secondary)).truncate());
    }
    let q = st.query.trim().to_lowercase();
    let in_group = |v: &Variable| st.group.as_ref().is_none_or(|g| v.name.starts_with(&format!("{g}/")));
    let mut list: Vec<&Variable> =
        view.in_collection(&coll.id).filter(|v| in_group(v) && (q.is_empty() || v.name.to_lowercase().contains(&q))).collect();
    list.sort_by(|a, b| group_of(&a.name).cmp(group_of(&b.name)));
    let width = (NAME_W + coll.modes.len() as f32 * MODE_W + 40.0).max(ui.available_width());
    egui::ScrollArea::both().id_salt("vars_table").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        header_row(app, ui, view, coll, st, width);
        let mut group: Option<&str> = None;
        for v in &list {
            let g = group_of(&v.name);
            if group != Some(g) {
                group = Some(g);
                if !g.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
                    ui.painter().text(pos2(r.min.x + 16.0, r.center().y + 2.0), Align2::LEFT_CENTER, g, semibold(11.0), t.text_secondary);
                }
            }
            var_row(app, ui, view, coll, v, st, width);
        }
        if list.is_empty() && !q.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("No matching variables"), regular(11.0), t.text_secondary);
        }
        if view.editable() {
            create_row(app, ui, view, coll, st);
        }
    });
}

/// Draw a collection's mode header row with rename, duplicate, reorder and delete actions
/// and an add-mode button when the collection is editable and not extended.
fn header_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor, width: f32) {
    let t = Tokens::get(ui.ctx());
    // An extended collection has the modes of the one it extends.
    let editable = view.editable() && coll.extends.is_none();
    let (r, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Name"), regular(11.0), t.text_secondary);
    for (i, m) in coll.modes.iter().enumerate() {
        let x = r.min.x + NAME_W + i as f32 * MODE_W;
        let cell = Rect::from_min_max(pos2(x + 4.0, r.min.y + 4.0), pos2(x + MODE_W - 8.0, r.max.y - 4.0));
        let target = Rename::Mode(coll.id.clone(), m.id.clone());
        if editable && st.rename.as_ref() == Some(&target) {
            if let Some(name) = rename_field(ui, st, &target, &m.name, cell.shrink2(vec2(4.0, 2.0)), regular(11.0)) {
                app.run("variable.renameMode", json!({"collection": coll.id, "mode": m.id, "name": name}));
            }
            continue;
        }
        let name_pos = pos2(cell.min.x + 4.0, cell.center().y);
        if !editable {
            ui.painter().with_clip_rect(cell).text(name_pos, Align2::LEFT_CENTER, &m.name, regular(11.0), t.text_secondary);
            continue;
        }
        let resp = ui.interact(cell, Id::unique(("vars_mode", &m.id)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(cell, CornerRadius::same(4), t.hover);
        }
        ui.painter().with_clip_rect(cell).text(name_pos, Align2::LEFT_CENTER, &m.name, regular(11.0), t.text_secondary);
        if resp.double_clicked() {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Rename mode"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if menu_item(ui, tr("Duplicate mode"), "") {
                let name = unique(&format!("{} copy", m.name), |n| coll.modes.iter().any(|x| x.name == n));
                let made = app.run("variable.addMode", json!({"collection": coll.id, "name": name, "from": m.id}));
                if let Some(id) = made.and_then(|r| r["id"].as_str().map(str::to_string)) {
                    start_rename(st, Rename::Mode(coll.id.clone(), id));
                }
                ui.close();
            }
            // The default mode is the first column: moving a column changes it.
            let last = coll.modes.len() - 1;
            for (show, label, to) in [
                (i > 0, tr_noop("Set as default"), 0),
                (i > 0, tr_noop("Move column left"), i.saturating_sub(1)),
                (i < last, tr_noop("Move column right"), i + 1),
            ] {
                if show && menu_item(ui, tr(label), "") {
                    app.run("variable.moveMode", json!({"collection": coll.id, "mode": m.id, "to": to}));
                    ui.close();
                }
            }
            if coll.modes.len() > 1 && menu_item(ui, tr("Delete mode"), "") {
                app.run("variable.removeMode", json!({"collection": coll.id, "mode": m.id}));
                ui.close();
            }
        });
    }
    let x = r.min.x + NAME_W + coll.modes.len() as f32 * MODE_W;
    if editable
        && widgets::icon_button_at(
            ui,
            Id::unique("vars_add_mode"),
            Rect::from_center_size(pos2(x + 14.0, r.center().y), vec2(24.0, 24.0)),
            Icon::Plus,
            tr("Add mode"),
        )
    {
        let name = unique("Mode", |n| coll.modes.iter().any(|m| m.name == n));
        if let Some(id) = app.run("variable.addMode", json!({"collection": coll.id, "name": name})).and_then(|r| r["id"].as_str().map(str::to_string))
        {
            start_rename(st, Rename::Mode(coll.id.clone(), id));
        }
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

fn var_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, v: &Variable, st: &mut Editor, width: f32) {
    let t = Tokens::get(ui.ctx());
    let editable = view.editable();
    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    if resp.hovered() || (editable && st.details.as_deref() == Some(v.id.as_str())) {
        ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
    }
    kind_glyph(ui, view.vars, v, coll, Rect::from_center_size(pos2(r.min.x + 24.0, r.center().y), vec2(14.0, 14.0)));
    let name_r = Rect::from_min_max(pos2(r.min.x + 40.0, r.min.y + 8.0), pos2(r.min.x + NAME_W - 8.0, r.max.y - 8.0));
    let target = Rename::Variable(v.id.clone());
    // An inherited variable is named (and deleted) in the collection it extends.
    let inherited = v.extends.is_some();
    if editable && st.rename.as_ref() == Some(&target) {
        if let Some(name) = rename_field(ui, st, &target, &v.name, name_r, regular(11.0)) {
            app.run("variable.update", json!({"variable": v.id, "name": name}));
        }
    } else {
        // Grouped names show their last part under the group's header.
        let short = v.name.rsplit('/').next().unwrap_or(&v.name);
        ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, short, regular(11.0), t.text);
    }
    for (i, m) in coll.modes.iter().enumerate() {
        let x = r.min.x + NAME_W + i as f32 * MODE_W;
        let cell = Rect::from_min_max(pos2(x + 4.0, r.center().y - 12.0), pos2(x + MODE_W - 8.0, r.center().y + 12.0));
        value_cell(app, ui, view, v, coll, &m.id, cell);
    }
    if editable {
        if !inherited && resp.double_clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < name_r.max.x) {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Edit variable"), "") {
                st.details = Some(v.id.clone());
                ui.close();
            }
            if !inherited && menu_item(ui, tr("Rename"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if !v.overrides.is_empty() && menu_item(ui, tr("Reset overrides"), "") {
                app.run("variable.resetOverride", json!({"variable": v.id}));
                ui.close();
            }
            if menu_item(ui, tr("Duplicate"), "") {
                app.run("variable.duplicate", json!({"variable": v.id}));
                ui.close();
            }
            if !inherited && menu_item(ui, tr("Delete variable"), "") {
                app.run("variable.delete", json!({"variable": v.id}));
                ui.close();
            }
        });
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

/// The type marker before a variable's name: its colour (default mode), or a type glyph.
fn kind_glyph(ui: &Ui, vars: &Variables, v: &Variable, coll: &Collection, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    match v.kind {
        VariableType::Color => {
            let modes = coll.modes.first().map(|m| [(coll.id.clone(), m.id.clone())].into_iter().collect()).unwrap_or_default();
            if let Some(VariableValue::Color(c)) = vars.resolve(&v.id, &modes) {
                widgets::paint_swatch(p, rect, &figcraft_color::Paint::solid(c));
            }
        }
        VariableType::Number => {
            p.text(rect.center(), Align2::CENTER_CENTER, "#", semibold(12.0), t.icon_secondary);
        }
        VariableType::String => icons::paint(p, rect, Icon::Text, t.icon_secondary),
        VariableType::Boolean => {
            let track = Rect::from_center_size(rect.center(), vec2(14.0, 8.0));
            p.rect_stroke(track, CornerRadius::same(4), Stroke::new(1.2, t.icon_secondary), StrokeKind::Inside);
            p.circle_filled(pos2(track.max.x - 4.0, track.center().y), 2.0, t.icon_secondary);
        }
    }
}

/// The button that replaces the reference or calculation in `mode` with the value it shows now.
fn detach(app: &mut FigApp, ui: &mut Ui, view: &View, v: &Variable, coll: &Collection, mode: &str, (key, at): (Id, Rect), label: &str) {
    if widgets::icon_button_at(ui, key, at, Icon::Detach, label) {
        let modes = [(coll.id.clone(), mode.to_string())].into_iter().collect();
        if let Some(resolved) = view.vars.resolve(&v.id, &modes) {
            set(app, v, mode, crate::panels::proto_panel::value_json(&resolved));
        }
    }
}

fn set(app: &mut FigApp, v: &Variable, mode: &str, value: Value) {
    app.run("variable.set", json!({"variable": v.id, "mode": mode, "value": value}));
}

/// One value cell: an editor for the value, or the aliased variable's chip; the variable button
/// on the right makes it an alias. A library's values only show.
fn value_cell(app: &mut FigApp, ui: &mut Ui, view: &View, v: &Variable, coll: &Collection, mode: &str, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let Some(val) = v.values.get(mode) else { return };
    let editable = view.editable();
    let id = Id::unique(("var_cell", &v.id, mode));
    // The value editor leaves room for the alias button (and in extended collections an
    // override's reset button, so the column stays even).
    let overridden = v.overrides.contains(mode);
    let body = Rect::from_min_max(rect.min, pos2(rect.max.x - if coll.extends.is_some() { 50.0 } else { 26.0 }, rect.max.y));
    let alias_btn = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(22.0, 22.0));
    let hovered = editable && ui.rect_contains_pointer(rect.expand2(vec2(4.0, 6.0)));
    if overridden {
        let mark = Rect::from_min_size(pos2(rect.min.x - 4.0, rect.min.y + 3.0), vec2(2.0, rect.height() - 6.0));
        ui.painter().rect_filled(mark, CornerRadius::same(1), t.accent);
        ui.interact(mark.expand2(vec2(2.0, 0.0)), id.with("mark"), Sense::hover())
            .tip(tr("Overridden here (the collection it extends has another value)"));
        if hovered
            && widgets::icon_button_at(ui, id.with("reset"), alias_btn.translate(vec2(-24.0, 0.0)), Icon::Reset, tr("Reset to the inherited value"))
        {
            app.run("variable.resetOverride", json!({"variable": v.id, "mode": mode}));
        }
    }
    if let VariableValue::Alias(target) = val {
        let full = view.vars.get_by_id(target).map(|x| x.name.as_str()).unwrap_or(tr("Missing variable"));
        let resp = ui.interact(body, id.with("alias"), if editable { Sense::click() } else { Sense::hover() });
        let resp = if editable { resp.on_hover_text(tr("Change alias")) } else { resp };
        ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
        let chip = body.shrink2(vec2(3.0, 3.0));
        ui.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
        let border = if editable && resp.hovered() { t.border_strong } else { t.border };
        ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(chip.min.x + 11.0, chip.center().y), vec2(12.0, 12.0)),
            Icon::Variable,
            t.icon_secondary,
        );
        let text_r = Rect::from_min_max(pos2(chip.min.x + 21.0, chip.min.y), pos2(chip.max.x - 4.0, chip.max.y));
        ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, full, regular(11.0), t.text);
        if resp.clicked() {
            crate::variable_ui::open_alias(ui, &v.id, mode, v.kind, body.left_bottom());
        }
        if hovered {
            detach(app, ui, view, v, coll, mode, (id.with("detach"), alias_btn), tr("Detach alias"));
        }
        return;
    }
    // A calculation shows what it resolves to and names its operands. It is set through the
    // commands; here, like an alias, it can be swapped for an alias or detached to its colour.
    if matches!(val, VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. }) {
        let name = |id: &str| view.vars.get_by_id(id).map_or_else(|| tr("Missing variable").to_string(), |x| x.name.clone());
        let formula = val.display_with(&name);
        let resp = ui.interact(body, id.with("calc"), if editable { Sense::click() } else { Sense::hover() });
        let resp = resp.tip(format!("{formula}\n{}", tr("Calculated from other variables; it follows them")));
        let shown = match view.vars.flattened(v, mode, val) {
            Some(VariableValue::Color(c)) => Some(c),
            _ => None,
        };
        // The formula is the point of the cell: it keeps the width an alpha readout would take.
        read_only_cell(ui, body, shown, formula, false);
        if resp.clicked() {
            crate::variable_ui::open_alias(ui, &v.id, mode, v.kind, body.left_bottom());
        }
        if hovered {
            detach(app, ui, view, v, coll, mode, (id.with("detach"), alias_btn), tr("Detach calculation"));
        }
        return;
    }
    if !editable {
        read_only_value(ui, val, body);
        return;
    }
    match val {
        VariableValue::Color(c) => {
            ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
            let sw = Rect::from_center_size(pos2(body.min.x + 13.0, body.center().y), vec2(14.0, 14.0));
            widgets::paint_swatch(ui.painter(), sw, &figcraft_color::Paint::solid(*c));
            let response = ui.interact(sw.expand(3.0), id.with("swatch"), Sense::click()).tip(tr("Edit colour"));
            app.picker_control(PickerTarget::Variable { id: v.id.clone(), mode: mode.to_string() }, &response);
            let hex_r = Rect::from_min_max(pos2(sw.max.x + 8.0, body.min.y + 4.0), pos2(body.max.x - 44.0, body.max.y - 4.0));
            if let Some(h) = widgets::text_field_inline(ui, id.with("hex"), &c.to_hex(), hex_r, regular(11.0))
                && let Some(nc) = figcraft_color::Color::from_hex_or_name(&h)
            {
                // Six digits (or a colour name) keep the alpha; eight digits set it.
                let digits = h.trim().trim_start_matches('#');
                let nc = if digits.len() == 8 && digits.chars().all(|ch| ch.is_ascii_hexdigit()) { nc } else { nc.with_alpha(c.a) };
                set(app, v, mode, json!(nc.to_css_hex()));
            }
            alpha_text(ui, c, body);
        }
        VariableValue::Number(x) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body));
            match widgets::number_field(&mut child, id.with("num"), Lead::None, Some(*x), "", body.width(), 1.0) {
                Some(FieldEdit::Set(n)) => set(app, v, mode, json!(n)),
                Some(FieldEdit::Scrub(n, done)) => {
                    crate::typography::live(app, "Edit variable", "variable.set", json!({"variable": v.id, "mode": mode, "value": n}), done)
                }
                None => {}
            }
        }
        VariableValue::String(s) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body));
            if let Some(n) = widgets::text_field(&mut child, id.with("str"), s, body.width(), "") {
                set(app, v, mode, json!(n));
            }
        }
        VariableValue::Boolean(b) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body.shrink2(vec2(6.0, 0.0))));
            if widgets::checkbox(&mut child, *b, if *b { tr("True") } else { tr("False") }) {
                set(app, v, mode, json!(!b));
            }
        }
        VariableValue::Alias(_) | VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => {}
    }
    if hovered && widgets::icon_button_at(ui, id.with("mkalias"), alias_btn, Icon::Variable, tr("Create alias")) {
        crate::variable_ui::open_alias(ui, &v.id, mode, v.kind, body.left_bottom());
    }
}

/// A colour's alpha as a percentage at the right end of `body`.
fn alpha_text(ui: &Ui, c: &figcraft_color::Color, body: Rect) {
    let t = Tokens::get(ui.ctx());
    let pct = format!("{}%", (c.a * 100.0).round());
    ui.painter().text(pos2(body.max.x - 8.0, body.center().y), Align2::RIGHT_CENTER, pct, regular(11.0), t.text_secondary);
}

/// A value that only shows (a library's): its swatch and hex for colours, else its text.
fn read_only_value(ui: &Ui, val: &VariableValue, body: Rect) {
    let swatch = match val {
        VariableValue::Color(c) => Some(*c),
        _ => None,
    };
    let text = match val {
        VariableValue::Boolean(b) => (if *b { tr("True") } else { tr("False") }).to_string(),
        other => other.display(),
    };
    read_only_cell(ui, body, swatch, text, true);
}

/// A cell that only shows: the colour's swatch (and its alpha at the right end when `alpha`), then
/// `text`.
fn read_only_cell(ui: &Ui, body: Rect, swatch: Option<figcraft_color::Color>, text: String, alpha: bool) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
    let mut x = body.min.x + 8.0;
    if let Some(c) = swatch {
        let sw = Rect::from_center_size(pos2(body.min.x + 13.0, body.center().y), vec2(14.0, 14.0));
        widgets::paint_swatch(ui.painter(), sw, &figcraft_color::Paint::solid(c));
        x = sw.max.x + 8.0;
        if alpha {
            alpha_text(ui, &c, body);
        }
    }
    let right = if alpha && swatch.is_some() { 44.0 } else { 4.0 };
    let text_r = Rect::from_min_max(pos2(x, body.min.y), pos2(body.max.x - right, body.max.y));
    ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, text, regular(11.0), t.text);
}

fn create_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let resp = widgets::button(ui, tr("+ Create variable"), false, true);
        crate::right::menu_popup(&resp).show(|ui| {
            for kind in TYPES {
                let label = match kind {
                    // The English label also names the new variable (document content).
                    VariableType::Color => tr_noop("Color"),
                    VariableType::Number => tr_noop("Number"),
                    VariableType::String => tr_noop("String"),
                    VariableType::Boolean => tr_noop("Boolean"),
                };
                if menu_item(ui, tr(label), "") {
                    let name = unique(label, |n| view.in_collection(&coll.id).any(|v| v.name == n));
                    let params = json!({"collection": coll.id, "name": name, "type": kind.id()});
                    if let Some(id) = app.run("variable.create", params).and_then(|r| r["id"].as_str().map(str::to_string)) {
                        st.query.clear();
                        start_rename(st, Rename::Variable(id));
                    }
                    ui.close();
                }
            }
        });
    });
    ui.add_space(12.0);
}

/// The details of one of this file's variables: name, description, where pickers offer it and
/// whether publishing the file as a library leaves it out.
fn details(app: &mut FigApp, ctx: &Context, vars: &Variables, st: &mut Editor) {
    let Some(v) = st.details.as_ref().filter(|_| st.library.is_none()).and_then(|id| vars.get_by_id(id)).cloned() else {
        st.details = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    egui::Window::new(tr("Edit variable"))
        .id(Id::unique("variable_details"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(280.0)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(260.0);
            widgets::label(ui, tr("Name"), t.text_secondary);
            if let Some(n) = widgets::text_field(ui, Id::unique(("var_detail_name", &v.id)), &v.name, 260.0, "") {
                app.run("variable.update", json!({"variable": v.id, "name": n}));
            }
            ui.add_space(6.0);
            widgets::label(ui, tr("Description"), t.text_secondary);
            if let Some(d) = widgets::text_field(ui, Id::unique(("var_detail_desc", &v.id)), &v.description, 260.0, tr("What it's for")) {
                app.run("variable.update", json!({"variable": v.id, "description": d}));
            }
            ui.add_space(6.0);
            if widgets::checkbox(ui, v.hidden_from_publishing, tr("Hide from publishing")) {
                app.run("variable.update", json!({"variable": v.id, "hiddenFromPublishing": !v.hidden_from_publishing}));
            }
            // The names code uses for it per platform (generated CSS uses the Web one).
            ui.add_space(10.0);
            widgets::label(ui, tr("Code syntax"), t.text_secondary);
            for (key, label) in [("WEB", tr_noop("Web")), ("iOS", tr_noop("iOS")), ("ANDROID", tr_noop("Android"))] {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(vec2(60.0, 24.0), Sense::hover());
                    ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, tr(label), regular(11.0), t.text);
                    let cur = v.code_syntax.get(key).map_or("", String::as_str);
                    if let Some(n) = widgets::text_field_named(ui, Id::unique(("var_detail_code", key, &v.id)), cur, 192.0, tr(label)) {
                        app.run("variable.update", json!({"variable": v.id, "codeSyntax": {key: n}}));
                    }
                });
            }
            let scopes = scopes_for(v.kind);
            if scopes.is_empty() {
                return;
            }
            ui.add_space(10.0);
            widgets::label(ui, tr("Scoping: where pickers offer it"), t.text_secondary);
            // "All" off hides it everywhere; on, from everywhere it fits.
            if widgets::checkbox(ui, v.scopes.is_none(), tr("All supported properties")) {
                app.run("variable.update", json!({"variable": v.id, "scopes": if v.scopes.is_none() { json!([]) } else { Value::Null }}));
            }
            let on: Vec<&str> = scopes.iter().map(|(id, _)| *id).filter(|id| v.in_scope(id)).collect();
            for (sid, label) in scopes {
                let checked = on.contains(sid);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if widgets::checkbox(ui, checked, tr(label)) {
                        let mut list = on.clone();
                        if checked {
                            list.retain(|x| x != sid);
                        } else {
                            list.push(sid);
                        }
                        app.run("variable.update", json!({"variable": v.id, "scopes": list}));
                    }
                });
            }
        });
    if !open {
        st.details = None;
    }
}

#[cfg(test)]
mod tests {
    use super::groups;

    #[test]
    fn groups_include_parents_once_and_sorted() {
        let names = ["space/sm", "space/md", "color/brand/blue", "color/text", "plain", "color/brand/red"];
        assert_eq!(groups(names.into_iter()), ["color", "color/brand", "space"]);
        assert!(groups(["a", "b"].into_iter()).is_empty());
    }
}
