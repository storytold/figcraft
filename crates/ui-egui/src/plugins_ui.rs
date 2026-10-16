//! Plugins in the UI: the installed plugins in the Plugins menu (widgets in Plugins ▸ Widgets), the
//! parameters form a command with parameters asks for before it runs, the panels running plugins
//! show, and the Manage plugins window (installed plugins, and a server's plugin directory to
//! browse, install from and update from). Everything runs through the engine's `plugin.*`,
//! `plugin.directory.*` and `widget.*` commands.

use egui::{Align2, Context, Id, RichText, Ui};
use figcraft_engine::plugin_directory::{PluginEntry, installed_version, newer_version};
use figcraft_engine::plugins::{Access, InstalledPlugin, Param, ParamKind, WidgetFit};
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
    Browse,
}

impl PluginsTab {
    pub const ALL: [PluginsTab; 2] = [PluginsTab::Installed, PluginsTab::Browse];

    fn label(self) -> &'static str {
        match self {
            PluginsTab::Installed => tr("Installed"),
            PluginsTab::Browse => tr("Browse"),
        }
    }
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
}

/// Width of the plugin windows' fields.
const FIELD_W: f32 = 220.0;
/// While a slider is dragged its plugin hears of it at most this often (seconds), and once more
/// when the drag ends.
const SLIDER_EVERY: f64 = 0.1;
/// Width of the number field beside a slider.
const SLIDER_FIELD_W: f32 = 52.0;
/// A panel's picture is decoded only up to this many pixels on a side.
const MAX_IMAGE_SIDE: u32 = 2048;
/// The pixels a grid icon is rasterised at (crisp on retina).
const GRID_ICON_PX: u32 = 72;
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
        Some(f) if f.id == plugin => Id::new("plugin_form"),
        _ => Id::new(("plugin_panel", plugin)),
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

/// Plugins ▸ Development ▸ Import plugin from manifest…: pick a `figcraft-plugin.json`.
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
                let picked = crate::menus::submenu(ui, "Plugins", &format!("Plugins/plugin:{}", m.id), &m.name, t, |ui| {
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
            crate::menus::submenu(ui, "Plugins", "Plugins/widgets", tr("Widgets"), t, |ui| {
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
    // (plugin, what was used; None: closed)
    let mut actions: Vec<(String, Option<Used>)> = Vec::new();
    for (i, open) in app.session.plugins.panels.iter().enumerate() {
        let id = open.manifest.id.as_str();
        let mut keep = true;
        let mut used = None;
        // Side by side leftwards from the properties panel, three to a row.
        let at = egui::pos2(right - PANEL_STEP * (i % 3) as f32, 52.0 + 28.0 * (i / 3) as f32);
        floating(open.title(), ctx).id(Id::new(("plugin_panel", id))).pivot(Align2::RIGHT_TOP).default_pos(at).open(&mut keep).show(ctx, |ui| {
            ui.set_width(320.0);
            used = controls(ui, id, &open.panel.controls, &open.values, &t);
        });
        if !keep {
            actions.push((id.to_string(), None));
        } else if used.is_some() {
            actions.push((id.to_string(), used));
        }
    }
    for (id, used) in actions {
        match used {
            None => {
                app.run("plugin.close", json!({"id": id}));
            }
            Some(Used::Set(control, value)) => run(app, "plugin.event", json!({"id": id, "control": control, "value": value})),
            Some(Used::Pick(control)) => app.ui.picker = Some(PickerTarget::PluginField { plugin: id, control, pending: None }),
        }
    }
}

/// What the Manage plugins window asks for.
enum Manage {
    Remove(String),
    Import,
    Browse(String),
    Confirm(String),
    /// Stop waiting for the directory's listing.
    StopBrowsing,
}

/// The directory requests the window sends without waiting (the server answers later).
const LIST: &str = "plugin.directory.list";
const INSTALL: &str = "plugin.directory.install";

/// Width of the Manage plugins window, and of a plugin's text beside its button.
const MANAGE_W: f32 = 400.0;
const ROW_TEXT_W: f32 = 300.0;

fn manage_window(app: &mut FigApp, ctx: &Context) {
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut asked = None;
    let mut tab = app.ui.plugins_manage.tab;
    window(tr("Plugins"), ctx).id(Id::new("Plugins")).open(&mut open).show(ctx, |ui| {
        ui.set_width(MANAGE_W);
        tab_strip(ui, &t, &PluginsTab::ALL, PluginsTab::label, &mut tab);
        ui.add_space(12.0);
        asked = match tab {
            PluginsTab::Installed => installed_tab(app, ui, &t),
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
        app.ui.plugins_manage.confirm = None;
    }
    if app.ui.plugins_manage.confirm.is_some() {
        confirm_window(app, ctx, &t);
    }
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
    if widgets::button(ui, tr("Import plugin from manifest…"), true, true).clicked() {
        asked = Some(Manage::Import);
    }
    asked
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
            widgets::text_field(ui, Id::new("plugin_directory_server"), &server, MANAGE_W - 90.0, tr("Server address (ws://host:47900/)"))
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
    if let Some(q) = widgets::text_field(ui, Id::new("plugin_directory_query"), &app.ui.plugins_manage.query, MANAGE_W, tr("Search plugins")) {
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
    window(title, ctx).id(Id::new("plugin_install_confirm")).open(&mut open).show(ctx, |ui| {
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
    parts.push(match (&p.source, &p.path) {
        (Some(server), _) => trf("from {server}", &[("server", server)]),
        (None, Some(path)) => trf("development: {path}", &[("path", path)]),
        (None, None) => tr("this session only").into(),
    });
    parts.join(" · ")
}

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
    window(&title, ctx).id(Id::new("plugin_form")).open(&mut open).show(ctx, |ui| {
        ui.set_width(320.0);
        match controls(ui, &format!("{}/{}", form.id, form.command), &command.params, &form.values, &t) {
            Some(Used::Set(name, v)) => {
                form.values.insert(name, v);
            }
            Some(Used::Pick(control)) => pick = Some(control),
            None => {}
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            submit = widgets::button(ui, tr("Run"), true, true).clicked();
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    app.ui.plugin_form = (open && !submit && !cancel).then_some(form.clone());
    if let Some(control) = pick {
        app.ui.picker = Some(PickerTarget::PluginField { plugin: form.id.clone(), control, pending: None });
    }
    if submit {
        run(app, "plugin.run", json!({"id": form.id, "command": form.command, "params": form.values}));
    }
}

/// Whether a control is drawn across the panel, not as a row of the label and field columns.
fn full_width(kind: ParamKind) -> bool {
    matches!(kind, ParamKind::Section | ParamKind::Divider | ParamKind::Image | ParamKind::Progress | ParamKind::Grid)
}

/// The label in the left column: checkboxes and buttons carry their own, a note has none.
fn caption(p: &Param) -> &str {
    if matches!(p.kind, ParamKind::Boolean | ParamKind::Button | ParamKind::Note) { "" } else { p.title() }
}

/// Parameters or panel controls in two columns (label, field); buttons in a row share one line,
/// the first button filled. Dividers, pictures and progress bars span the panel, and a section's
/// heading folds the controls after it, up to the next section or divider. Returns what the user
/// did to which control.
fn controls(ui: &mut Ui, salt: &str, params: &[Param], values: &Map<String, Value>, t: &Tokens) -> Option<Used> {
    let mut changed = None;
    let mut first_button = true;
    // The label column is as wide as its widest label in every block of fields, so they line up.
    let label_w = |p: &Param| ui.painter().layout_no_wrap(caption(p).to_string(), regular(11.0), t.text_secondary).size().x;
    let caption_w = params.iter().filter(|p| !full_width(p.kind)).map(label_w).fold(0.0, f32::max);
    let mut open = true;
    for (i, block) in params.chunk_by(|a, b| !full_width(a.kind) && !full_width(b.kind)).enumerate() {
        let Some(p) = block.first() else { continue };
        match p.kind {
            ParamKind::Section => {
                let id = Id::new(("plugin_section", salt, i));
                let starts_open = p.default.as_ref().and_then(Value::as_bool).unwrap_or(true);
                open = ui.data(|d| d.get_temp(id)).unwrap_or(starts_open);
                if widgets::disclosure(ui, &p.label, open) {
                    open = !open;
                    ui.data_mut(|d| d.insert_temp(id, open));
                }
            }
            // A divider ends a section: it and what follows always show.
            ParamKind::Divider => {
                open = true;
                ui.add_space(2.0);
                widgets::separator(ui);
                ui.add_space(2.0);
            }
            _ if !open => {}
            ParamKind::Image => picture(ui, Id::new(("plugin_image", salt, i)), p, t),
            ParamKind::Grid => {
                if let Some(hit) = grid(ui, Id::new(("plugin_grid", salt, i)), p, values, t) {
                    changed = Some(hit);
                }
            }
            ParamKind::Progress => {
                if !p.label.is_empty() {
                    widgets::label(ui, &p.label, t.text_secondary);
                }
                widgets::progress_bar(ui, p.title(), p.default.as_ref().and_then(Value::as_f64).unwrap_or(0.0), ui.available_width());
            }
            _ => {
                let hit = fields(ui, (salt, i), block, values, caption_w, &mut first_button, t);
                if hit.is_some() {
                    changed = hit;
                }
            }
        }
        ui.add_space(if open || p.kind == ParamKind::Section { 8.0 } else { 0.0 });
    }
    changed
}

/// A block of fields, notes and buttons in two columns. A disabled control is greyed and can't be
/// used; a field's validation message shows under it.
fn fields(
    ui: &mut Ui,
    salt: (&str, usize),
    block: &[Param],
    values: &Map<String, Value>,
    caption_w: f32,
    first_button: &mut bool,
    t: &Tokens,
) -> Option<Used> {
    let mut changed = None;
    egui::Grid::new(("plugin_controls", salt)).num_columns(2).min_col_width(caption_w).spacing([12.0, 8.0]).show(ui, |ui| {
        for row in block.chunk_by(|a, b| a.kind == ParamKind::Button && b.kind == ParamKind::Button) {
            let Some(p) = row.first() else { continue };
            let label = RichText::new(caption(p)).font(regular(11.0)).color(t.text_secondary);
            // A list's label sits by its first item.
            if p.kind == ParamKind::List {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| ui.label(label));
            } else {
                ui.label(label);
            }
            let hit = match p.kind {
                ParamKind::Note => {
                    ui.vertical(|ui| {
                        ui.set_max_width(FIELD_W);
                        ui.add(egui::Label::new(RichText::new(&p.label).font(regular(11.0)).color(t.text)).wrap());
                    });
                    None
                }
                ParamKind::Button => {
                    ui.horizontal(|ui| {
                        let pressed = row.iter().filter(|b| widgets::button(ui, b.title(), std::mem::take(first_button), !b.disabled).clicked());
                        pressed.last().map(|b| Used::Set(b.name.clone(), Value::Null))
                    })
                    .inner
                }
                _ => {
                    let current = values.get(&p.name).cloned().unwrap_or_else(|| p.default_value());
                    let id = Id::new(("plugin_control", salt, &p.name));
                    ui.add_enabled_ui(!p.disabled, |ui| field(ui, id, p, &current)).inner
                }
            };
            if hit.is_some() {
                changed = hit;
            }
            ui.end_row();
            if !p.error.is_empty() {
                ui.label("");
                ui.vertical(|ui| {
                    ui.set_max_width(FIELD_W);
                    ui.add(egui::Label::new(RichText::new(&p.error).font(regular(11.0)).color(t.danger)).wrap());
                });
                ui.end_row();
            }
        }
    });
    changed
}

/// A panel's picture, fitted in the panel's width at its height and named for screen readers. It
/// is decoded once, and again only when the plugin shows another.
fn picture(ui: &mut Ui, id: Id, p: &Param, t: &Tokens) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    p.data.hash(&mut hasher);
    let key = hasher.finish();
    let kept = ui.data(|d| d.get_temp::<(u64, Option<egui::TextureHandle>)>(id)).filter(|(k, _)| *k == key);
    let (_, texture) = kept.unwrap_or_else(|| {
        let fresh = (key, decode(&p.data).map(|img| ui.ctx().load_texture("plugin-image", img, egui::TextureOptions::LINEAR)));
        ui.data_mut(|d| d.insert_temp(id, fresh.clone()));
        fresh
    });
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), p.height.unwrap_or(120.0) as f32), egui::Sense::hover());
    crate::a11y::image(&resp, p.title());
    let Some(texture) = texture else {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), t.field);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, tr("This picture can't be shown"), regular(11.0), t.text_secondary);
        return;
    };
    // `fit` shows all of it, `fill` stretches it to the box; `crop` and `tile` cover the box.
    let (size, mut uv) = (texture.size_vec2(), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)));
    let scale = |cover: bool| {
        let (sx, sy) = (rect.width() / size.x.max(1.0), rect.height() / size.y.max(1.0));
        if cover { sx.max(sy) } else { sx.min(sy) }
    };
    let shown = match p.fit {
        None | Some(WidgetFit::Fit) => egui::Rect::from_center_size(rect.center(), size * scale(false)),
        Some(WidgetFit::Fill) => rect,
        Some(WidgetFit::Crop | WidgetFit::Tile) => {
            let covered = size * scale(true);
            uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(rect.width() / covered.x, rect.height() / covered.y));
            rect
        }
    };
    ui.painter().with_clip_rect(rect).image(texture.id(), shown, uv, egui::Color32::WHITE);
}

/// A panel's grid of clickable cells (plugin API 4): icons rasterised from SVG path data or
/// decoded pictures, `columns` across. The picked cell calls back with its value like a button;
/// the picked value draws selected. Returns what the user picked, if any.
fn grid(ui: &mut Ui, salt: Id, p: &Param, values: &Map<String, Value>, t: &Tokens) -> Option<Used> {
    let picked = values.get(&p.name).and_then(Value::as_str).unwrap_or_default().to_string();
    let columns = p.columns.unwrap_or(6.0).clamp(1.0, 12.0) as usize;
    // Cells fill the panel's width, so the grid has no lopsided padding.
    let cell = ((ui.available_width() - 6.0 * (columns - 1) as f32) / columns as f32).max(28.0);
    let mut hit = None;
    ui.add_enabled_ui(!p.disabled, |ui| {
        egui::Grid::new(("plugin_cells", salt)).num_columns(columns).spacing([6.0, 6.0]).show(ui, |ui| {
            for (n, item) in p.items.iter().enumerate() {
                let id = Id::new(("plugin_cell", salt, n));
                let texture = cell_texture(ui, id, item);
                let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(cell, cell), egui::Sense::click());
                let name = if item.label.is_empty() { item.value.as_str() } else { item.label.as_str() };
                crate::a11y::image(&resp, name);
                resp = resp.on_hover_text(name);
                if ui.is_rect_visible(rect) {
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, egui::CornerRadius::same(10), t.hover);
                    }
                    if picked == item.value {
                        ui.painter().rect_stroke(rect, egui::CornerRadius::same(10), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                    }
                    match texture {
                        Some(texture) => {
                            let inner = rect.shrink(7.0);
                            let s = texture.size_vec2();
                            let shown = egui::Rect::from_center_size(inner.center(), s * (inner.width() / s.x.max(1.0)).min(inner.height() / s.y.max(1.0)));
                            ui.painter().image(texture.id(), shown, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(rect.shrink(7.0), egui::CornerRadius::same(4), t.field);
                        }
                    }
                }
                if resp.clicked() {
                    hit = Some(Used::Set(p.name.clone(), Value::String(item.value.clone())));
                }
                if (n + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
    });
    hit
}

/// A grid cell's texture, rasterised once per path, picture, colour and stroke.
fn cell_texture(ui: &mut Ui, id: Id, item: &figcraft_engine::plugins::GridItem) -> Option<egui::TextureHandle> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    item.path.hash(&mut hasher);
    item.data.hash(&mut hasher);
    item.color.hash(&mut hasher);
    item.stroke.map(f64::to_bits).hash(&mut hasher);
    let key = hasher.finish();
    let kept = ui.data(|d| d.get_temp::<(u64, Option<egui::TextureHandle>)>(id)).filter(|(k, _)| *k == key);
    let (_, texture) = kept.unwrap_or_else(|| {
        let fresh = (key, cell_pixels(item).map(|img| ui.ctx().load_texture("plugin-cell", img, egui::TextureOptions::LINEAR)));
        ui.data_mut(|d| d.insert_temp(id, fresh.clone()));
        fresh
    });
    texture
}

/// A grid cell's pixels: a picture decoded, or SVG path data rasterised in its colour.
fn cell_pixels(item: &figcraft_engine::plugins::GridItem) -> Option<egui::ColorImage> {
    if !item.data.trim().is_empty() {
        return decode(&item.data);
    }
    let color = if item.color.is_empty() { "#1E1E1E" } else { item.color.as_str() };
    let rgba = match item.stroke {
        Some(w) => figcraft_render::rasterize_icon(&item.path, None, Some((color, w)), GRID_ICON_PX),
        None => figcraft_render::rasterize_icon(&item.path, Some(color), None, GRID_ICON_PX),
    }?;
    Some(egui::ColorImage::from_rgba_unmultiplied([GRID_ICON_PX as usize, GRID_ICON_PX as usize], &rgba))
}

/// A picture's pixels from its file's bytes as base64 (PNG, JPEG, BMP…), if it is one of at most
/// [`MAX_IMAGE_SIDE`] pixels on a side.
fn decode(data: &str) -> Option<egui::ColorImage> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    let reader = || image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok();
    let (w, h) = reader()?.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return None;
    }
    let rgba = reader()?.decode().ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
}

/// Whether a slider that moved tells its plugin now: when the drag ends, and while it goes on at
/// most every [`SLIDER_EVERY`] seconds (`last`: when the plugin last heard of this drag).
pub(crate) fn slider_due(done: bool, now: f64, last: Option<f64>) -> bool {
    done || last.is_none_or(|l| now - l >= SLIDER_EVERY)
}

/// A slider with its value in a number field beside it (which also takes the keyboard). While it
/// is dragged it shows the dragged value at once and tells the plugin now and then.
fn slider(ui: &mut Ui, id: Id, p: &Param, current: &Value) -> Option<Value> {
    let name = p.title();
    // (the value being dragged, when the plugin last heard of the drag)
    let live = ui.data(|d| d.get_temp::<(f64, Option<f64>)>(id));
    let shown = live.map(|l| l.0).or(current.as_f64());
    let range = (p.min.unwrap_or(0.0), p.max.unwrap_or(100.0));
    ui.horizontal(|ui| {
        let (_, moved) = widgets::slider(ui, id, name, range, None, shown, FIELD_W - SLIDER_FIELD_W - 8.0);
        let typed = match widgets::number_field(ui, id.with("value"), Lead::Named(name), shown, "", SLIDER_FIELD_W, p.step.unwrap_or(1.0)) {
            Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) => p.coerce(&json!(v)).ok(),
            _ => None,
        };
        let Some((v, done)) = moved else { return typed };
        let value = p.coerce(&json!(v)).ok()?;
        let (now, last) = (ui.input(|i| i.time), live.and_then(|l| l.1));
        // Nothing to tell when the plugin already has this value.
        let due = slider_due(done, now, last) && value.as_f64() != current.as_f64();
        let told = if due { Some(now) } else { last };
        ui.data_mut(|d| match done {
            true => d.remove::<(f64, Option<f64>)>(id),
            false => drop(d.insert_temp(id, (value.as_f64().unwrap_or(v), told))),
        });
        due.then_some(value)
    })
    .inner
}

/// One parameter's field, named for screen readers: its new value when it changed.
fn field(ui: &mut Ui, id: Id, p: &Param, current: &Value) -> Option<Used> {
    let name = p.title();
    let v = match p.kind {
        ParamKind::Text => widgets::text_field_named(ui, id, current.as_str().unwrap_or_default(), FIELD_W, name).map(Value::String),
        ParamKind::Number => match widgets::number_field(ui, id, Lead::Named(name), current.as_f64(), "", FIELD_W, p.step.unwrap_or(1.0))? {
            FieldEdit::Set(v) | FieldEdit::Scrub(v, true) => p.coerce(&json!(v)).ok(),
            FieldEdit::Scrub(_, false) => None,
        },
        ParamKind::Slider => slider(ui, id, p, current),
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
                    let picked = widgets::swatch(ui, &figcraft_color::Paint::solid(color), widgets::FIELD_H).clicked();
                    (picked, widgets::text_field_named(ui, id, hex, FIELD_W - widgets::FIELD_H - 8.0, name).and_then(|s| p.coerce(&json!(s)).ok()))
                })
                .inner;
            if picked {
                return Some(Used::Pick(p.name.clone()));
            }
            typed
        }
        // Buttons, notes and the controls that span the panel aren't fields.
        _ => None,
    };
    v.map(|v| Used::Set(p.name.clone(), v))
}
