//! Plugins in the UI: the installed plugins in the Plugins menu (widgets in Plugins ▸ Widgets), the
//! parameters form a command with parameters asks for before it runs, the panels running plugins
//! show, and the Manage plugins window (installed plugins and their updates, plug-in catalogs and
//! GitHub repositories to install from, and a server's plugin directory to browse, install from
//! and update from). Everything runs through the engine's `plugin.*`, `plugin.catalog.*`,
//! `plugin.directory.*` and `widget.*` commands.

use egui::{Align2, Context, Id, RichText, Ui};
use figcraft_engine::plugin_directory::{PluginEntry, installed_version, newer_version};
use figcraft_engine::plugins::{Access, InstalledPlugin, Param, ParamKind};
use serde_json::{Map, Value, json};

use crate::FigApp;
use crate::colorpicker::PickerTarget;
use crate::dialogs::{floating, tab_strip, window};
use crate::i18n::{tr, trf};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FieldEdit, Lead};

/// The parameters form of a plugin command, with the values entered so far.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginForm {
    pub id: String,
    pub command: String,
    pub values: Map<String, Value>,
}

/// The Manage plugins window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluginsTab {
    #[default]
    Installed,
    /// Plug-in catalogs and GitHub repositories.
    Catalogs,
    /// A figcraft-server's plugin directory.
    Browse,
}

impl PluginsTab {
    pub const ALL: [PluginsTab; 3] = [PluginsTab::Installed, PluginsTab::Catalogs, PluginsTab::Browse];

    fn label(self) -> &'static str {
        match self {
            PluginsTab::Installed => tr("Installed"),
            PluginsTab::Catalogs => tr("Catalogs"),
            PluginsTab::Browse => tr("Browse"),
        }
    }
}

/// A plugin read for its install confirmation (`plugin.install` or `plugin.update` without
/// `confirm`): the command, what it was asked, and what it said about the plugin.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreInstall {
    pub command: String,
    pub request: Value,
    pub preview: Value,
}

/// The Manage plugins window's state: its tab, the Browse tab's fields and the plugin waiting for
/// the user to confirm its install (by id).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginsWindow {
    pub tab: PluginsTab,
    /// The server address typed (`None`: the one browsed last).
    pub server: Option<String>,
    pub query: String,
    pub confirm: Option<String>,
    /// The Catalogs tab's fields: a GitHub repository and a catalog's address.
    pub repo: String,
    pub catalog: String,
    /// The plugin from a file, a catalog or a repository waiting for the user to confirm.
    pub install: Option<StoreInstall>,
    /// The updates found by Check for updates (`plugin.update`'s `updates`).
    pub updates: Option<Vec<Value>>,
}

/// Width of the plugin windows' fields.
const FIELD_W: f32 = 220.0;
/// Tallest a panel image is drawn, and the largest picture a panel shows (pixels on a side).
const IMAGE_MAX_H: f32 = 240.0;
const IMAGE_MAX_SIDE: u32 = 4096;
/// The value field beside a slider.
const SLIDER_VALUE_W: f32 = 56.0;
/// How far apart plugin panels open, side by side (a panel and a gap).
const PANEL_STEP: f32 = 372.0;

/// What the user did to a parameter or panel control.
enum Used {
    /// Changed a field to a value, or pressed a button (null).
    Set(String, Value),
    /// Asked for the colour picker for a colour field.
    Pick(String),
}

/// The value of colour field `control` of plugin `plugin`'s parameters form (when it's open) or
/// panel, as hex.
pub(crate) fn field_value(app: &FigApp, plugin: &str, control: &str) -> Option<String> {
    let values = match &app.ui.plugin_form {
        Some(f) if f.id == plugin => &f.values,
        _ => &app.session.plugins.panel(plugin)?.values,
    };
    values.get(control)?.as_str().map(str::to_string)
}

/// Where the window holding plugin `plugin`'s colour fields is (its parameters form when it's
/// open, else its panel), as last drawn.
pub(crate) fn field_window(app: &FigApp, ctx: &Context, plugin: &str) -> Option<egui::Rect> {
    let id = match &app.ui.plugin_form {
        Some(f) if f.id == plugin => Id::unique("plugin_form"),
        _ => Id::unique(("plugin_panel", plugin)),
    };
    ctx.memory(|m| m.area_rect(id))
}

/// Set colour field `control` of plugin `plugin`'s parameters form (when it's open), or of its
/// panel (calling the plugin back), to `hex`.
pub(crate) fn set_field(app: &mut FigApp, plugin: &str, control: &str, hex: &str) {
    match &mut app.ui.plugin_form {
        Some(f) if f.id == plugin => {
            f.values.insert(control.to_string(), json!(hex));
        }
        _ => run(app, "plugin.event", json!({"id": plugin, "control": control, "value": hex})),
    }
}

/// Start plugin `id`'s `command` (empty: its first): ask for its parameters first when it has any.
pub fn start(app: &mut FigApp, id: &str, command: &str) {
    let found = app.session.prefs.plugins.iter().find(|p| p.manifest.id == id).and_then(|p| p.manifest.command(Some(command)));
    match found {
        Some(c) if !c.params.is_empty() => {
            let values = c.params.iter().map(|p| (p.name.clone(), p.default_value())).collect();
            app.ui.plugin_form = Some(PluginForm { id: id.to_string(), command: c.id.clone(), values });
        }
        _ => run(app, "plugin.run", json!({"id": id, "command": command})),
    }
}

/// A plugin offered a file to save (`dialog.pluginSave`: `name`, `data` as base64): the user picks
/// where it goes in FigCraft's own save dialog, or the browser downloads it. The plugin never learns
/// the path.
pub(crate) fn save_file(app: &mut FigApp, data: &Value) {
    use base64::Engine as _;
    let name = data["name"].as_str().unwrap_or_default().to_string();
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data["data"].as_str().unwrap_or_default()) else {
        app.toast(tr("Couldn't save the file"));
        return;
    };
    if app.services.downloads() {
        if app.services.download(&name, &bytes) {
            app.toast(trf("Saved {name}", &[("name", &name)]));
        } else {
            app.toast(tr("Couldn't save the file"));
        }
        return;
    }
    let ext = std::path::Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
    let exts: Vec<&str> = if ext.is_empty() { Vec::new() } else { vec![ext.as_str()] };
    let shown = name.clone();
    app.pick_save(tr("Save file"), &name, &exts, move |app, path| {
        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or(shown);
        match figcraft_engine::file_access::write(&path, &bytes) {
            Ok(()) => app.toast(trf("Saved {name}", &[("name", &file)])),
            Err(_) => app.toast(tr("Couldn't save the file")),
        }
    });
}

/// Run `plugin.run` or `plugin.event`; a text output is shown as a toast unless the plugin showed
/// a message.
fn run(app: &mut FigApp, cmd: &str, params: Value) {
    let toast = app.ui.toast.clone();
    let Some(out) = app.run(cmd, params) else { return };
    if app.ui.toast == toast
        && let Some(text) = out["output"].as_str().filter(|t| !t.trim().is_empty())
    {
        app.toast(text.chars().take(200).collect::<String>());
    }
}

/// Plugins ▸ Development ▸ Import plugin from manifest…: pick a `craft-plugin.json` (or a
/// `figcraft-plugin.json`).
pub fn import_dialog(app: &mut FigApp) {
    app.pick_open(tr("Import plugin from manifest"), &["json"], |app, p| {
        if let Some(v) = app.run("plugin.import", json!({"path": p.to_string_lossy()})) {
            app.toast(trf("Imported {name}", &[("name", &v["name"].as_str().unwrap_or(tr("the plugin")))]));
        }
    });
}

/// The installed plugins at the top of the Plugins menu: a plugin with one command is an item, one
/// with several a submenu. True when one was clicked.
pub fn menu(app: &mut FigApp, ui: &mut Ui, t: &Tokens) -> bool {
    if app.session.prefs.plugins.is_empty() {
        return false;
    }
    let mut chosen: Option<(String, String)> = None;
    for p in &app.session.prefs.plugins {
        let m = &p.manifest;
        match m.commands.as_slice() {
            // A widget may have no commands: it's in Widgets.
            [] => {}
            [only] => {
                // An ellipsis: it asks for parameters first.
                let label = if only.params.is_empty() { m.name.clone() } else { format!("{}…", m.name) };
                if crate::menus::item(ui, &label, "", true, false, t) {
                    chosen = Some((m.id.clone(), only.id.clone()));
                }
            }
            commands => {
                let picked = crate::menus::submenu(ui, &format!("Plugins/plugin:{}", m.id), &m.name, t, |ui| {
                    commands.iter().find(|c| crate::menus::item(ui, &c.label, "", true, false, t)).map(|c| c.id.clone())
                });
                if let Some(c) = picked {
                    chosen = Some((m.id.clone(), c));
                }
            }
        }
    }
    let widgets: Vec<(&str, &str)> =
        app.session.prefs.plugins.iter().filter(|p| p.manifest.widget).map(|p| (p.manifest.id.as_str(), p.manifest.name.as_str())).collect();
    let widget = (!widgets.is_empty())
        .then(|| {
            crate::menus::submenu(ui, "Plugins/widgets", tr("Widgets"), t, |ui| {
                widgets.iter().find(|(_, name)| crate::menus::item(ui, name, "", true, false, t)).map(|(id, _)| id.to_string())
            })
        })
        .flatten();
    crate::menus::separator(ui);
    if let Some(plugin) = widget {
        app.run("widget.insert", json!({"plugin": plugin}));
        return true;
    }
    let Some((id, command)) = chosen else { return false };
    start(app, &id, &command);
    true
}

/// The plugin windows (Manage plugins, a command's parameters, the plugins' panels).
pub fn show(app: &mut FigApp, ctx: &Context) {
    if app.ui.plugins_window {
        manage_window(app, ctx);
    }
    if app.ui.plugin_form.is_some() {
        form_window(app, ctx);
    }
    if !app.session.plugins.panels.is_empty() {
        panel_windows(app, ctx);
    }
}

/// The panels running plugins show, at first beside the properties panel and each other (they
/// can be moved): a pressed button or a changed value calls its plugin
/// back (`plugin.event`); closing one ends its plugin's session.
fn panel_windows(app: &mut FigApp, ctx: &Context) {
    let t = Tokens::get(ctx);
    let right = ctx.content_rect().right() - 252.0;
    // (plugin, what was used, in order; None: closed)
    let mut actions: Vec<(String, Option<Used>)> = Vec::new();
    for (i, open) in app.session.plugins.panels.iter().enumerate() {
        let id = open.manifest.id.as_str();
        let mut keep = true;
        let mut used = Vec::new();
        // Side by side leftwards from the properties panel, three to a row.
        let at = egui::pos2(right - PANEL_STEP * (i % 3) as f32, 52.0 + 28.0 * (i / 3) as f32);
        floating(open.title(), ctx).id(Id::unique(("plugin_panel", id))).pivot(Align2::RIGHT_TOP).default_pos(at).open(&mut keep).show(ctx, |ui| {
            ui.set_width(320.0);
            used = controls(ui, id, &open.panel.controls, &open.values, &t);
        });
        if !keep {
            actions.push((id.to_string(), None));
        } else {
            actions.extend(used.into_iter().map(|u| (id.to_string(), Some(u))));
        }
    }
    for (id, used) in actions {
        match used {
            None => {
                app.run("plugin.close", json!({"id": id}));
            }
            Some(Used::Set(control, value)) => run(app, "plugin.event", json!({"id": id, "control": control, "value": value})),
            Some(Used::Pick(control)) => app.open_picker(PickerTarget::PluginField { plugin: id, control, pending: None }, None),
        }
    }
}

/// What the Manage plugins window asks for.
enum Manage {
    Remove(String),
    Import,
    /// Install a `.craftplugin` file.
    InstallFile,
    /// Read a plugin for its install confirmation: `plugin.install` (or `plugin.update`) with these
    /// parameters.
    Get(&'static str, Value),
    AddCatalog(String),
    RemoveCatalog(String),
    ListCatalogs,
    CheckUpdates,
    Browse(String),
    Confirm(String),
    /// Stop waiting for the directory's listing.
    StopBrowsing,
}

/// The directory requests the window sends without waiting (the server answers later).
const LIST: &str = "plugin.directory.list";
const INSTALL: &str = "plugin.directory.install";
/// The plug-in store's requests (catalogs and GitHub), also sent without waiting.
const STORE_INSTALL: &str = "plugin.install";
const STORE_UPDATE: &str = "plugin.update";
const CATALOGS: &str = "plugin.catalog.list";
const ADD_CATALOG: &str = "plugin.catalog.add";

/// Width of the Manage plugins window, and of a plugin's text beside its button.
const MANAGE_W: f32 = 400.0;
const ROW_TEXT_W: f32 = 300.0;

/// Draw the Plugins window with installed and browse tabs, dispatching remove, import,
/// browse, confirm and stop actions; closing the window cancels its directory requests and
/// clears confirmation.
fn manage_window(app: &mut FigApp, ctx: &Context) {
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut asked = None;
    let mut tab = app.ui.plugins_manage.tab;
    window(tr("Plugins"), ctx).id(Id::unique("Plugins")).open(&mut open).show(ctx, |ui| {
        ui.set_width(MANAGE_W);
        tab_strip(ui, &t, &PluginsTab::ALL, PluginsTab::label, &mut tab);
        ui.add_space(12.0);
        asked = match tab {
            PluginsTab::Installed => installed_tab(app, ui, &t),
            PluginsTab::Catalogs => catalogs_tab(app, ui, &t),
            PluginsTab::Browse => browse_tab(app, ui, &t),
        };
    });
    app.ui.plugins_manage.tab = tab;
    app.ui.plugins_window = open;
    match asked {
        Some(Manage::Remove(id)) => {
            app.run("plugin.remove", json!({"id": id}));
        }
        Some(Manage::Import) => import_dialog(app),
        Some(Manage::InstallFile) => install_file_dialog(app),
        Some(Manage::Get(command, request)) => get(app, ctx, command, request),
        Some(Manage::AddCatalog(url)) => app.run_background(
            ctx,
            ADD_CATALOG,
            json!({"url": url}),
            Box::new(|app, ctx, _| {
                app.ui.plugins_manage.catalog.clear();
                app.run_background(ctx, CATALOGS, json!({}), Box::new(|_, _, _| {}));
            }),
        ),
        Some(Manage::RemoveCatalog(url)) => {
            app.run("plugin.catalog.remove", json!({"url": url}));
        }
        Some(Manage::ListCatalogs) => app.run_background(ctx, CATALOGS, json!({}), Box::new(|_, _, _| {})),
        Some(Manage::CheckUpdates) => app.run_background(
            ctx,
            STORE_UPDATE,
            json!({}),
            Box::new(|app, _, v| {
                let updates = v["updates"].as_array().cloned().unwrap_or_default();
                if updates.is_empty() {
                    app.toast(tr("Every plugin is up to date"));
                }
                app.ui.plugins_manage.updates = Some(updates);
            }),
        ),
        Some(Manage::Browse(server)) => {
            app.ui.plugins_manage.confirm = None;
            app.cancel_background(INSTALL);
            app.run_background(ctx, LIST, json!({"server": server}), Box::new(|_, _, _| {}));
        }
        Some(Manage::Confirm(id)) => app.ui.plugins_manage.confirm = Some(id),
        Some(Manage::StopBrowsing) => app.cancel_background(LIST),
        None => {}
    }
    if !open {
        // Closing the window stops waiting for its server.
        app.cancel_background(LIST);
        app.cancel_background(INSTALL);
        for id in [STORE_INSTALL, STORE_UPDATE, CATALOGS, ADD_CATALOG] {
            app.cancel_background(id);
        }
        app.ui.plugins_manage.confirm = None;
        app.ui.plugins_manage.install = None;
    }
    if app.ui.plugins_manage.confirm.is_some() {
        confirm_window(app, ctx, &t);
    }
    if app.ui.plugins_manage.install.is_some() {
        store_confirm_window(app, ctx, &t);
    }
}

/// Read a plugin (`command` is `plugin.install` or `plugin.update`, without `confirm`) and show
/// its install confirmation when it's in.
fn get(app: &mut FigApp, ctx: &Context, command: &'static str, request: Value) {
    app.ui.plugins_manage.install = None;
    let req = request.clone();
    app.run_background(
        ctx,
        command,
        request,
        Box::new(move |app, _, v| {
            app.ui.plugins_manage.install = Some(StoreInstall { command: command.to_string(), request: req, preview: v });
        }),
    );
}

/// Install plugin ▸ From file…: pick a `.craftplugin` bundle; its confirmation follows.
pub fn install_file_dialog(app: &mut FigApp) {
    app.pick_open(tr("Install plugin from file"), &["craftplugin"], |app, p| {
        let request = json!({"path": p.to_string_lossy()});
        if let Some(v) = app.run(STORE_INSTALL, request.clone()) {
            app.ui.plugins_manage.install = Some(StoreInstall { command: STORE_INSTALL.into(), request, preview: v });
        }
    });
}

fn installed_tab(app: &FigApp, ui: &mut Ui, t: &Tokens) -> Option<Manage> {
    let mut asked = None;
    let plugins = &app.session.prefs.plugins;
    if plugins.is_empty() {
        widgets::label(ui, tr("No plugins yet. Import a development plugin from its figcraft-plugin.json."), t.text_secondary);
    }
    for (i, p) in plugins.iter().enumerate() {
        if i > 0 {
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ROW_TEXT_W);
                ui.label(RichText::new(&p.manifest.name).font(semibold(12.0)).color(t.text));
                widgets::label(ui, &summary(p), t.text_secondary);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if widgets::button(ui, tr("Remove"), false, true).clicked() {
                    asked = Some(Manage::Remove(p.manifest.id.clone()));
                }
            });
        });
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if widgets::button(ui, tr("Install from file…"), true, true).clicked() {
            asked = Some(Manage::InstallFile);
        }
        if widgets::button(ui, tr("Import plugin from manifest…"), false, true).clicked() {
            asked = Some(Manage::Import);
        }
    });
    // Updates of plugins from catalogs and GitHub: checked only when asked.
    if plugins.iter().any(|p| p.store) {
        ui.add_space(8.0);
        let checking = app.session.waiting_for(STORE_UPDATE);
        if widgets::button(ui, if checking { tr("Checking…") } else { tr("Check for updates") }, false, !checking).clicked() {
            asked = Some(Manage::CheckUpdates);
        }
        for u in app.ui.plugins_manage.updates.iter().flatten() {
            let (id, name) = (u["id"].as_str().unwrap_or_default(), u["name"].as_str().unwrap_or_default());
            let to = u["version"].as_str().or(u["tag"].as_str()).unwrap_or_default();
            ui.horizontal(|ui| {
                widgets::label(ui, &trf("{name}: {version} is out", &[("name", &name), ("version", &to)]), t.text);
                if widgets::button(ui, tr("Update"), true, !app.session.waiting_for(STORE_INSTALL)).clicked() {
                    asked = Some(Manage::Get(STORE_UPDATE, json!({"id": id})));
                }
            });
        }
    }
    asked
}

/// Plug-ins from GitHub repositories and catalogs: a repository to install from, the catalogs added
/// and what they offer. Nothing is fetched until a button is pressed.
fn catalogs_tab(app: &mut FigApp, ui: &mut Ui, t: &Tokens) -> Option<Manage> {
    let mut asked = None;
    let busy = app.session.waiting_for(STORE_INSTALL) || app.session.waiting_for(STORE_UPDATE);
    ui.label(RichText::new(tr("From a GitHub repository")).font(semibold(11.0)).color(t.text));
    ui.horizontal(|ui| {
        let mut enter = false;
        let repo = app.ui.plugins_manage.repo.clone();
        if let Some(r) = widgets::text_field(ui, Id::unique("plugin_repo"), &repo, MANAGE_W - 90.0, tr("owner/repository or owner/repository@tag")) {
            enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            app.ui.plugins_manage.repo = r;
        }
        let repo = app.ui.plugins_manage.repo.trim().to_string();
        if (widgets::button(ui, tr("Get"), true, !busy).clicked() || enter) && !repo.is_empty() && !busy {
            asked = Some(Manage::Get(STORE_INSTALL, json!({"repo": repo})));
        }
    });
    if busy {
        ui.add_space(4.0);
        widgets::label(ui, tr("Downloading…"), t.text_secondary);
    }
    ui.add_space(12.0);
    ui.label(RichText::new(tr("Catalogs")).font(semibold(11.0)).color(t.text));
    let catalogs = app.session.prefs.plugin_catalogs.clone();
    if catalogs.is_empty() {
        widgets::label(ui, tr("Add a catalog's address (its index.json) to see the plugins it lists."), t.text_secondary);
    }
    for url in &catalogs {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(url).font(regular(11.0)).color(t.text)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if widgets::button(ui, tr("Remove"), false, true).clicked() {
                    asked = Some(Manage::RemoveCatalog(url.clone()));
                }
            });
        });
    }
    ui.horizontal(|ui| {
        let mut enter = false;
        let url = app.ui.plugins_manage.catalog.clone();
        if let Some(u) =
            widgets::text_field(ui, Id::unique("plugin_catalog_url"), &url, MANAGE_W - 90.0, tr("Catalog address (https://…/index.json)"))
        {
            enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            app.ui.plugins_manage.catalog = u;
        }
        let url = app.ui.plugins_manage.catalog.trim().to_string();
        if (widgets::button(ui, tr("Add"), false, !app.session.waiting_for(ADD_CATALOG)).clicked() || enter) && !url.is_empty() {
            asked = Some(Manage::AddCatalog(url));
        }
    });
    if catalogs.is_empty() {
        return asked;
    }
    ui.add_space(8.0);
    let loading = app.session.waiting_for(CATALOGS);
    if widgets::button(ui, if loading { tr("Fetching…") } else { tr("Refresh") }, false, !loading).clicked() {
        asked = Some(Manage::ListCatalogs);
    }
    let Some(listing) = app.session.plugins.catalogs.as_ref() else { return asked };
    for (url, why) in &listing.errors {
        widgets::label(ui, &trf("{catalog}: {error}", &[("catalog", url), ("error", why)]), t.text_secondary);
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical().id_salt("plugin_catalog_offers").max_height(300.0).show(ui, |ui| {
        for (i, o) in listing.offers.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            let p = &o.package;
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ROW_TEXT_W);
                    ui.label(RichText::new(&p.name).font(semibold(12.0)).color(t.text));
                    let mut parts = vec![o.release.version.clone()];
                    if !p.author.is_empty() {
                        parts.push(trf("by {author}", &[("author", &p.author)]));
                    }
                    if let Some(b) = o.release.size {
                        parts.push(size(b));
                    }
                    widgets::label(ui, &parts.join(" · "), t.text_secondary);
                    if !p.description.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&p.description).font(regular(11.0)).color(t.text)).wrap());
                    }
                });
                let label = match installed_version(&app.session, &p.id) {
                    None => Some(tr("Install")),
                    Some(have) if newer_version(&o.release.version, have) => Some(tr("Update")),
                    Some(_) => None,
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if let Some(label) = label
                        && widgets::button(ui, label, true, !busy).clicked()
                    {
                        asked = Some(Manage::Get(STORE_INSTALL, json!({"catalog": o.catalog, "id": p.id})));
                    }
                });
            });
        }
    });
    asked
}

/// Installing runs a plugin's code: what a plugin from a file, a catalog or a repository asks
/// permission for, its kind, where it comes from (and when that is in no catalog the user reads)
/// and its SHA-256. It is installed only when Install is pressed.
fn store_confirm_window(app: &mut FigApp, ctx: &Context, t: &Tokens) {
    let Some(pending) = app.ui.plugins_manage.install.clone() else { return };
    let v = &pending.preview;
    let update = v["installedVersion"].is_string();
    let installing = app.session.waiting_for(&pending.command);
    let title = if update { tr("Update plugin") } else { tr("Install plugin") };
    let (mut open, mut install, mut cancel) = (true, false, false);
    window(title, ctx).id(Id::unique("plugin_store_confirm")).open(&mut open).show(ctx, |ui| {
        ui.set_width(360.0);
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        ui.label(RichText::new(format!("{} {}", s("name"), s("version"))).font(semibold(13.0)).color(t.text));
        widgets::label(ui, &trf("From {source}", &[("source", &s("from"))]), t.text_secondary);
        if v["inCatalog"] == json!(false) {
            ui.add(
                egui::Label::new(
                    RichText::new(tr("Not in any catalog you use: nobody has listed this plugin. Install it only if you trust its author."))
                        .font(semibold(11.0))
                        .color(t.text),
                )
                .wrap(),
            );
        }
        widgets::label(ui, &trf("Kind: {kind}", &[("kind", &s("kind"))]), t.text_secondary);
        if update && v["permissionsChanged"] == json!(true) {
            widgets::label(ui, tr("This version asks for different permissions."), t.text);
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr("It asks permission to:")).font(semibold(11.0)).color(t.text));
        let perms = &v["permissions"];
        let mut asks = vec![if perms["access"] == json!("write") { tr("read and edit the document") } else { tr("read the document") }.to_string()];
        let commands: Vec<&str> = perms["commands"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        asks.push(if commands.is_empty() {
            tr("call any command its access allows").to_string()
        } else {
            trf("call only: {commands}", &[("commands", &commands.join(", "))])
        });
        if v["widget"] == json!(true) {
            asks.push(tr("put a widget on the canvas").to_string());
        }
        if perms["saveFile"] == json!(true) {
            asks.push(tr("offer files to save if it asks to (you choose where each one goes)").to_string());
        }
        for a in asks {
            ui.add(egui::Label::new(RichText::new(format!("•  {a}")).font(regular(11.0)).color(t.text)).wrap());
        }
        ui.add_space(8.0);
        widgets::label(
            ui,
            tr("Plugins run in FigCraft's sandbox, without files, network or clock. Install only plugins you trust."),
            t.text_secondary,
        );
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(trf("SHA-256: {sha}", &[("sha", &s("sha256"))])).font(regular(10.0)).color(t.text_secondary)).wrap());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if installing {
                widgets::label(ui, tr("Installing…"), t.text_secondary);
            } else {
                install = widgets::button(ui, if update { tr("Update") } else { tr("Install") }, true, true).clicked();
            }
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    if !open || cancel {
        app.cancel_background(&pending.command);
        app.ui.plugins_manage.install = None;
    }
    if install {
        let mut params = pending.request.clone();
        params["confirm"] = json!(true);
        params["sha256"] = v["sha256"].clone();
        let command = pending.command.clone();
        app.run_background(
            ctx,
            &command,
            params,
            Box::new(move |app, _, v| {
                app.ui.plugins_manage.install = None;
                if let Some(list) = app.ui.plugins_manage.updates.as_mut() {
                    list.retain(|u| u["id"] != v["id"]);
                }
                let name = v["name"].as_str().unwrap_or(tr("the plugin")).to_string();
                let args: &[(&str, &dyn std::fmt::Display)] = &[("name", &name)];
                app.toast(if update { trf("Updated {name}", args) } else { trf("Installed {name}", args) });
            }),
        );
    }
}

/// A server's plugin directory: its address, a search, and its plugins with Install, Update or
/// Remove. Nothing is fetched until Browse is pressed.
fn browse_tab(app: &mut FigApp, ui: &mut Ui, t: &Tokens) -> Option<Manage> {
    let mut asked = None;
    let mut server = app.ui.plugins_manage.server.clone().or_else(|| app.session.prefs.plugin_directory.clone()).unwrap_or_default();
    let loading = app.session.waiting_for(LIST);
    ui.horizontal(|ui| {
        // The field commits on Enter (which also browses) or when Browse takes the focus.
        let mut enter = false;
        if let Some(s) =
            widgets::text_field(ui, Id::unique("plugin_directory_server"), &server, MANAGE_W - 90.0, tr("Server address (ws://host:47900/)"))
        {
            enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            server = s.clone();
            app.ui.plugins_manage.server = Some(s);
        }
        if loading {
            if widgets::button(ui, tr("Cancel"), false, true).clicked() {
                asked = Some(Manage::StopBrowsing);
            }
        } else if (widgets::button(ui, tr("Browse"), true, true).clicked() || enter) && !server.trim().is_empty() {
            asked = Some(Manage::Browse(server.trim().to_string()));
        }
    });
    if loading {
        ui.add_space(8.0);
        widgets::label(ui, tr("Fetching from the server…"), t.text_secondary);
        return asked;
    }
    let Some(listing) = app.session.plugins.directory.as_ref() else {
        ui.add_space(8.0);
        widgets::label(ui, tr("Enter the address of a figcraft-server and press Browse to see the plugins it offers."), t.text_secondary);
        return asked;
    };
    ui.add_space(8.0);
    if let Some(q) = widgets::text_field(ui, Id::unique("plugin_directory_query"), &app.ui.plugins_manage.query, MANAGE_W, tr("Search plugins")) {
        app.ui.plugins_manage.query = q;
    }
    ui.add_space(8.0);
    let q = app.ui.plugins_manage.query.trim().to_lowercase();
    let shown: Vec<&PluginEntry> = listing
        .entries
        .iter()
        .filter(|e| q.is_empty() || [&e.name, &e.description, &e.author, &e.id].iter().any(|f| f.to_lowercase().contains(&q)))
        .collect();
    if shown.is_empty() {
        widgets::label(ui, &trf("No plugins on {server}.", &[("server", &listing.server)]), t.text_secondary);
    }
    // One install at a time: the rows wait for it.
    let installing = app.session.waiting_for(INSTALL);
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        for (i, e) in shown.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ROW_TEXT_W);
                    ui.label(RichText::new(&e.name).font(semibold(12.0)).color(t.text));
                    widgets::label(ui, &entry_summary(e), t.text_secondary);
                    if !e.description.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&e.description).font(regular(11.0)).color(t.text)).wrap());
                    }
                });
                let (label, primary, ask): (&str, bool, fn(String) -> Manage) = match installed_version(&app.session, &e.id) {
                    None => (tr("Install"), true, Manage::Confirm),
                    Some(have) if newer_version(&e.version, have) => (tr("Update"), true, Manage::Confirm),
                    Some(_) => (tr("Remove"), false, Manage::Remove),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if widgets::button(ui, label, primary, !installing).clicked() {
                        asked = Some(ask(e.id.clone()));
                    }
                });
            });
        }
    });
    asked
}

/// "1.2.0 · by Ada · 54 KB · can edit the document" under a directory plugin's name.
fn entry_summary(e: &PluginEntry) -> String {
    let mut parts = vec![e.version.clone()];
    if !e.author.is_empty() {
        parts.push(trf("by {author}", &[("author", &e.author)]));
    }
    parts.push(size(e.bytes));
    parts.push(access_text(&e.access).to_string());
    parts.join(" · ")
}

fn access_text(access: &str) -> &'static str {
    if access == "write" { tr("can edit the document") } else { tr("reads the document") }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{} KB", b.div_ceil(1 << 10)),
        b => format!("{b} B"),
    }
}

/// Installing runs a plugin's code: this window shows what it asks permission for, where it comes
/// from and its SHA-256, and installs only when Install is pressed.
fn confirm_window(app: &mut FigApp, ctx: &Context, t: &Tokens) {
    let id = app.ui.plugins_manage.confirm.clone().unwrap_or_default();
    let found = app.session.plugins.directory.as_ref().and_then(|l| Some((l.server.clone(), l.entries.iter().find(|e| e.id == id)?.clone())));
    let Some((server, e)) = found else {
        app.ui.plugins_manage.confirm = None;
        return;
    };
    let update = installed_version(&app.session, &e.id).is_some();
    let installing = app.session.waiting_for(INSTALL);
    let title = if update { tr("Update plugin") } else { tr("Install plugin") };
    let (mut open, mut install, mut cancel) = (true, false, false);
    window(title, ctx).id(Id::unique("plugin_install_confirm")).open(&mut open).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(format!("{} {}", e.name, e.version)).font(semibold(13.0)).color(t.text));
        widgets::label(ui, &trf("From {server}", &[("server", &server)]), t.text_secondary);
        ui.add_space(10.0);
        ui.label(RichText::new(tr("It asks permission to:")).font(semibold(11.0)).color(t.text));
        let mut asks = vec![if e.access == "write" { tr("read and edit the document") } else { tr("read the document") }.to_string()];
        asks.push(if e.commands.is_empty() {
            tr("call any command its access allows").to_string()
        } else {
            trf("call only: {commands}", &[("commands", &e.commands.join(", "))])
        });
        if e.widget {
            asks.push(tr("put a widget on the canvas").to_string());
        }
        // shortcut: a directory entry doesn't say whether the manifest asks for `saveFile` (a new
        // field would break the entries' postcard wire format), so every API 4 plugin says it may;
        // carry the permission in the entry once the directory protocol negotiates new fields.
        if e.api >= 4 {
            asks.push(tr("offer files to save if it asks to (you choose where each one goes)").to_string());
        }
        for a in asks {
            ui.add(egui::Label::new(RichText::new(format!("•  {a}")).font(regular(11.0)).color(t.text)).wrap());
        }
        ui.add_space(8.0);
        widgets::label(
            ui,
            tr("Plugins run in FigCraft's sandbox, without files, network or clock. Install only plugins you trust."),
            t.text_secondary,
        );
        ui.add_space(4.0);
        widgets::label(ui, &trf("SHA-256: {sha}…", &[("sha", &e.sha256.get(..16).unwrap_or_default())]), t.text_secondary);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if installing {
                widgets::label(ui, tr("Installing…"), t.text_secondary);
            } else {
                install = widgets::button(ui, if update { tr("Update") } else { tr("Install") }, true, true).clicked();
            }
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    if !open || cancel {
        app.cancel_background(INSTALL);
        app.ui.plugins_manage.confirm = None;
    }
    if install {
        // The window stays open (Installing…) until the download is in; an error leaves it open.
        let params = json!({"id": e.id, "version": e.version, "sha256": e.sha256, "server": server, "confirm": true});
        app.run_background(
            ctx,
            INSTALL,
            params,
            Box::new(move |app, _, v| {
                app.ui.plugins_manage.confirm = None;
                let name = v["name"].as_str().unwrap_or(tr("the plugin")).to_string();
                let args: &[(&str, &dyn std::fmt::Display)] = &[("name", &name)];
                app.toast(if update { trf("Updated {name}", args) } else { trf("Installed {name}", args) });
            }),
        );
    }
}

/// "1.0 · can edit · development: <path>" under a plugin's name.
fn summary(p: &InstalledPlugin) -> String {
    let m = &p.manifest;
    let access = match m.permissions.access {
        Access::Read => tr("reads the document"),
        Access::Write => tr("can edit the document"),
    };
    let mut parts: Vec<String> = Vec::new();
    if !m.version.is_empty() {
        parts.push(m.version.clone());
    }
    parts.push(access.to_string());
    if m.permissions.save_file {
        parts.push(tr("can save files").to_string());
    }
    parts.push(match (&p.source, &p.path) {
        (Some(from), _) if p.store => trf("from {source}", &[("source", from)]),
        (Some(server), _) => trf("from {server}", &[("server", server)]),
        (None, Some(path)) => trf("development: {path}", &[("path", path)]),
        (None, None) => tr("this session only").into(),
    });
    parts.join(" · ")
}

/// Draw a plugin command's parameter form: close it when the plugin or command disappeared,
/// edit values and colour picks, run `plugin.run` on Run, and close on Cancel.
fn form_window(app: &mut FigApp, ctx: &Context) {
    let Some(mut form) = app.ui.plugin_form.clone() else { return };
    let Some((name, command)) = app
        .session
        .prefs
        .plugins
        .iter()
        .find(|p| p.manifest.id == form.id)
        .and_then(|p| Some((p.manifest.name.clone(), p.manifest.command(Some(&form.command))?.clone())))
    else {
        app.ui.plugin_form = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut submit = false;
    let mut cancel = false;
    let mut pick = None;
    let label = command.label.trim_end_matches(['…', '.']).trim();
    let title = if label.contains(name.as_str()) { label.to_string() } else { format!("{name}: {label}") };
    window(&title, ctx).id(Id::unique("plugin_form")).open(&mut open).show(ctx, |ui| {
        ui.set_width(320.0);
        for used in controls(ui, &format!("{}/{}", form.id, form.command), &command.params, &form.values, &t) {
            match used {
                Used::Set(name, v) => {
                    form.values.insert(name, v);
                }
                Used::Pick(control) => pick = Some(control),
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            submit = widgets::button(ui, tr("Run"), true, true).clicked();
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    app.ui.plugin_form = (open && !submit && !cancel).then_some(form.clone());
    if let Some(control) = pick {
        app.open_picker(PickerTarget::PluginField { plugin: form.id.clone(), control, pending: None }, None);
    }
    if submit {
        run(app, "plugin.run", json!({"id": form.id, "command": form.command, "params": form.values}));
    }
}

/// Parameters or panel controls in two columns (label, field); buttons in a row share one line,
/// the first button filled. Returns what the user did to which control.
fn controls(ui: &mut Ui, salt: &str, params: &[Param], values: &Map<String, Value>, t: &Tokens) -> Vec<Used> {
    // In order: a tap's press and release share a frame, so a field that loses the keyboard
    // commits in the same frame as the button that took it.
    let mut used = Vec::new();
    let mut first_button = true;
    egui::Grid::new(("plugin_controls", salt)).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        for row in params.chunk_by(|a, b| a.kind == ParamKind::Button && b.kind == ParamKind::Button) {
            let Some(p) = row.first() else { continue };
            // Checkboxes and buttons carry their own label; a list's sits by its first item.
            let caption = if matches!(p.kind, ParamKind::Boolean | ParamKind::Button | ParamKind::Note | ParamKind::Image) { "" } else { p.title() };
            // Unwrapped: the column is as wide as its widest caption ("Palette" wrapped otherwise).
            let caption = egui::Label::new(RichText::new(caption).font(regular(11.0)).color(t.text_secondary)).extend();
            if p.kind == ParamKind::List {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| ui.add(caption));
            } else {
                ui.add(caption);
            }
            let hit = match p.kind {
                ParamKind::Note => {
                    ui.vertical(|ui| {
                        ui.set_max_width(FIELD_W);
                        ui.add(egui::Label::new(RichText::new(&p.label).font(regular(11.0)).color(t.text)).wrap());
                    });
                    None
                }
                ParamKind::Image => {
                    image(ui, p, t);
                    None
                }
                ParamKind::Button => {
                    ui.horizontal(|ui| {
                        let pressed = row.iter().filter(|b| widgets::button(ui, b.title(), std::mem::take(&mut first_button), true).clicked());
                        pressed.last().map(|b| Used::Set(b.name.clone(), Value::Null))
                    })
                    .inner
                }
                _ => {
                    let current = values.get(&p.name).cloned().unwrap_or_else(|| p.default_value());
                    field(ui, Id::unique(("plugin_control", salt, &p.name)), p, &current)
                }
            };
            used.extend(hit);
            ui.end_row();
        }
    });
    used
}

/// Panel images decoded lately, by their `data` (None: it couldn't be shown); the oldest go past
/// [`IMAGE_CACHE`], so a plugin that keeps changing its pictures doesn't keep every texture.
type ImageCache = Vec<(Id, Option<egui::TextureHandle>)>;
const IMAGE_CACHE: usize = 32;

/// A panel's picture, at most as wide as a field and [`IMAGE_MAX_H`] tall (never scaled up), named
/// for screen readers by its label; its label (or "Image") as text when it can't be shown.
fn image(ui: &mut Ui, p: &Param, t: &Tokens) {
    let (cache, key) = (Id::unique("plugin_images"), Id::unique(&p.data));
    let cached = ui.data(|d| d.get_temp::<ImageCache>(cache).and_then(|c| c.into_iter().find(|(k, _)| *k == key)));
    let tex = match cached {
        Some((_, tex)) => tex,
        None => {
            let tex = decode_image(ui.ctx(), &p.data);
            ui.data_mut(|d| {
                let c = d.get_temp_mut_or_default::<ImageCache>(cache);
                c.push((key, tex.clone()));
                let over = c.len().saturating_sub(IMAGE_CACHE);
                c.drain(..over);
            });
            tex
        }
    };
    let Some(tex) = tex else {
        let text: &str = if p.label.is_empty() { tr("Image") } else { &p.label };
        ui.label(RichText::new(text).font(regular(11.0)).color(t.text_secondary));
        return;
    };
    let size = tex.size_vec2();
    let size = size * (FIELD_W / size.x).min(IMAGE_MAX_H / size.y).min(1.0);
    let resp = ui.add(egui::Image::new(&tex).fit_to_exact_size(size));
    crate::a11y::image(&resp, &p.label);
}

/// A panel image's texture: its base64 decoded, then the picture (at most [`IMAGE_MAX_SIDE`]
/// pixels on a side, checked before it is decoded), scaled to twice the field's width.
fn decode_image(ctx: &Context, data: &str) -> Option<egui::TextureHandle> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    let (w, h) = figcraft_render::images::dimensions(&bytes)?;
    if w > IMAGE_MAX_SIDE || h > IMAGE_MAX_SIDE {
        return None;
    }
    let pic = figcraft_format::thumbnail::decode_thumbnail(&bytes, (FIELD_W * 2.0) as u32)?;
    let image = egui::ColorImage::from_rgba_unmultiplied([pic.width as usize, pic.height as usize], &pic.rgba);
    Some(ctx.load_texture("plugin-panel-image", image, egui::TextureOptions::LINEAR))
}

/// One parameter's field, named for screen readers: its new value when it changed.
fn field(ui: &mut Ui, id: Id, p: &Param, current: &Value) -> Option<Used> {
    let name = p.title();
    let v = match p.kind {
        ParamKind::Text => widgets::text_field_named(ui, id, current.as_str().unwrap_or_default(), FIELD_W, name).map(Value::String),
        ParamKind::Number => match widgets::number_field(ui, id, Lead::Named(name), current.as_f64(), "", FIELD_W, 1.0)? {
            FieldEdit::Set(v) | FieldEdit::Scrub(v, true) => p.coerce(&json!(v)).ok(),
            FieldEdit::Scrub(_, false) => None,
        },
        // A slider and its value: dragging shows the value live and calls the plugin back once,
        // when let go (each call is an undo step).
        ParamKind::Slider => {
            let (min, max) = (p.min.unwrap_or(0.0), p.max.unwrap_or(1.0));
            let live = id.with("live");
            let shown = ui.data(|d| d.get_temp::<f64>(live)).or(current.as_f64());
            let (slid, typed) = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let slid = widgets::slider(ui, id.with("slider"), min..=max, None, shown, FIELD_W - SLIDER_VALUE_W - 6.0, name);
                    (slid, widgets::number_field(ui, id, Lead::Named(name), shown, "", SLIDER_VALUE_W, (max - min) / 100.0))
                })
                .inner;
            let done = match (slid, typed) {
                (Some((Some(v), false)), _) => {
                    ui.data_mut(|d| d.insert_temp(live, v));
                    None
                }
                (Some((v, true)), _) => Some(v.map_or_else(|| p.default_value(), |v| json!(v))),
                (_, Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true))) => Some(json!(v)),
                _ => None,
            };
            if done.is_some() {
                ui.data_mut(|d| d.remove::<f64>(live));
            }
            done.and_then(|v| p.coerce(&v).ok())
        }
        ParamKind::Boolean => {
            let on = current.as_bool().unwrap_or(false);
            widgets::checkbox(ui, on, name).then_some(Value::Bool(!on))
        }
        ParamKind::Select => {
            let value = current.as_str().unwrap_or_default();
            let resp = widgets::dropdown(ui, value, FIELD_W, None);
            crate::a11y::combo(&resp, name, value, crate::a11y::menu_open(&resp));
            let mut picked = None;
            crate::right::menu_popup(&resp).show(|ui| {
                for o in &p.options {
                    if crate::right::menu_item(ui, o, "") {
                        picked = Some(Value::String(o.clone()));
                        ui.close();
                    }
                }
            });
            picked
        }
        ParamKind::List => {
            let value = current.as_str();
            ui.vertical(|ui| {
                p.options
                    .iter()
                    .filter(|o| widgets::radio(ui, value == Some(o.as_str()), o) && value != Some(o.as_str()))
                    .last()
                    .map(|o| Value::String(o.clone()))
            })
            .inner
        }
        // A swatch that opens the colour picker, and the hex.
        ParamKind::Color => {
            let hex = current.as_str().unwrap_or_default();
            let (picked, typed) = ui
                .horizontal(|ui| {
                    let color = figcraft_color::Color::from_hex(hex).unwrap_or(figcraft_color::Color::BLACK);
                    let label = format!("{name} {}", tr("Color"));
                    let picked = widgets::swatch_named(ui, &figcraft_color::Paint::solid(color), widgets::FIELD_H, &label).clicked();
                    (picked, widgets::text_field_named(ui, id, hex, FIELD_W - widgets::FIELD_H - 8.0, name).and_then(|s| p.coerce(&json!(s)).ok()))
                })
                .inner;
            if picked {
                return Some(Used::Pick(p.name.clone()));
            }
            typed
        }
        ParamKind::Button | ParamKind::Note | ParamKind::Image => None,
    };
    v.map(|v| Used::Set(p.name.clone(), v))
}
