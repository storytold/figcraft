//! The main menu as a native menu bar (macOS, through `craft-menu`): the in-window menu's items,
//! labels, shortcuts and enabled and checked state, built from the same catalog, with Open recent
//! and the installed plugins. With several windows it acts on the one in front.

use egui::ViewportId;
use figcraft_engine::catalog::{MENU, SEP};
use figcraft_engine::keymap::{self, Platform};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{self, tr, tr_noop};
use crate::menus::TOP_MENUS;
use crate::plugins_ui::{self, MenuEntry};
use crate::windows;

/// Item ids that carry a parameter: a recent file (`file.recent:<path>`), Clear recent, and a
/// plugin's command (`plugin:<plugin id>:<command id>`). A native menu reports only the id.
pub const RECENT: &str = "file.recent:";
pub const CLEAR_RECENT: &str = "file.recent.clear";
pub const PLUGIN: &str = "plugin:";
/// A plugin's widget (`widget:<plugin id>`, Plugins ▸ Widgets).
pub const WIDGET: &str = "widget:";

/// The labels the Mac layout writes itself (the app menu, the Window menu), for
/// [`craft_menu::LayoutOptions::label`]; `{app}` is the app's name.
pub const MAC_LABELS: &[&str] = &[
    tr_noop("About {app}"),
    tr_noop("Settings…"),
    tr_noop("Services"),
    tr_noop("Hide {app}"),
    tr_noop("Hide Others"),
    tr_noop("Show All"),
    tr_noop("Quit {app}"),
    tr_noop("Window"),
    tr_noop("Minimize"),
    tr_noop("Zoom"),
    tr_noop("Bring All to Front"),
    tr_noop("Close Window"),
];

/// A label the Mac layout writes ([`MAC_LABELS`]), translated.
pub fn mac_label(english: &'static str) -> String {
    tr(english).to_string()
}

/// The main menu as a menu bar, with the Mac's shortcuts (a native menu bar is the Mac's).
/// `windows`: whether the app can open more windows (New window).
pub fn menu_bar(app: &FigApp, windows: bool) -> craft_menu::MenuBar {
    let mut rows = Vec::new();
    let path = |menu: &str| menu.split('/').map(|p| tr(p).to_string()).collect::<Vec<String>>();
    let row = |path: Vec<String>, id: String, label: String, shortcut: Option<&str>, enabled: bool, checked: Option<bool>| craft_menu::Row {
        path,
        id,
        label,
        shortcut: shortcut.map(str::to_string),
        enabled,
        checked,
    };
    let sep = |path: Vec<String>| row(path, String::new(), "---".into(), None, false, None);
    plugin_rows(app, &mut rows);
    for m in MENU {
        if m.label == SEP {
            rows.push(sep(path(m.menu)));
            continue;
        }
        if m.id == "file.openRecent" {
            recent_rows(app, path(&format!("{}/{}", m.menu, i18n::menu_item(m.id, m.label))), &mut rows);
            continue;
        }
        let enabled = crate::menus::available(m.id) && (m.id != "window.new" || windows);
        let checked = figcraft_engine::cmd::view::checked(&app.session, m.id);
        rows.push(row(
            path(m.menu),
            m.id.into(),
            i18n::menu_item(m.id, m.label).into_owned(),
            keymap::primary(m.id, Platform::Mac),
            enabled,
            checked,
        ));
    }
    let tops: Vec<&str> = TOP_MENUS.iter().map(|t| tr(t)).collect();
    let mut bar = craft_menu::from_items(&tops, &rows);
    bar.tag_item("help.about", craft_menu::ItemRole::About);
    bar.tag_item("app.preferences", craft_menu::ItemRole::Settings);
    bar.tag_menu(tr("File"), craft_menu::MenuRole::File);
    bar.tag_menu(tr("Help"), craft_menu::MenuRole::Help);
    bar
}

/// File ▸ Open recent: the recent files (greyed when the file is gone), then Clear recent.
fn recent_rows(app: &FigApp, path: Vec<String>, rows: &mut Vec<craft_menu::Row>) {
    let recents: Vec<&String> = crate::home_recents::listed(&app.session.prefs.recent_files).collect();
    for p in &recents {
        let file = std::path::Path::new(p.as_str());
        let name = file.file_name().map_or_else(|| p.to_string(), |f| f.to_string_lossy().into_owned());
        rows.push(craft_menu::Row {
            path: path.clone(),
            id: format!("{RECENT}{p}"),
            label: name,
            shortcut: None,
            enabled: file.exists(),
            checked: None,
        });
    }
    if !recents.is_empty() {
        rows.push(craft_menu::Row { path: path.clone(), id: String::new(), label: "---".into(), shortcut: None, enabled: false, checked: None });
    }
    rows.push(craft_menu::Row {
        path,
        id: CLEAR_RECENT.into(),
        label: tr("Clear recent").into(),
        shortcut: None,
        enabled: !recents.is_empty(),
        checked: None,
    });
}

/// The installed plugins' commands and widgets, heading the Plugins menu as in the in-window menu
/// ([`plugins_ui::menu_entries`]).
fn plugin_rows(app: &FigApp, rows: &mut Vec<craft_menu::Row>) {
    let plugins = &app.session.prefs.plugins;
    if plugins.is_empty() {
        return;
    }
    let top = tr("Plugins").to_string();
    let item = |path: Vec<String>, id: String, label: &str| craft_menu::Row {
        path,
        id,
        label: label.to_string(),
        shortcut: None,
        enabled: true,
        checked: None,
    };
    let (entries, widgets) = plugins_ui::menu_entries(plugins);
    for e in &entries {
        match e {
            MenuEntry::Command { plugin, command, label } => rows.push(item(vec![top.clone()], format!("{PLUGIN}{plugin}:{command}"), label)),
            MenuEntry::Submenu { plugin, name, commands } => {
                for (command, label) in commands {
                    rows.push(item(vec![top.clone(), name.to_string()], format!("{PLUGIN}{plugin}:{command}"), label));
                }
            }
        }
    }
    for (plugin, name) in &widgets {
        rows.push(item(vec![top.clone(), tr("Widgets").to_string()], format!("{WIDGET}{plugin}"), name));
    }
    rows.push(craft_menu::Row { path: vec![top], id: String::new(), label: "---".into(), shortcut: None, enabled: false, checked: None });
}

/// Run the item `id` chosen in the native menu bar, as the in-window menu would.
pub fn run(app: &mut FigApp, id: &str) {
    if id == CLEAR_RECENT {
        app.run_user("file.openRecent", json!({"clear": true}));
    } else if let Some(path) = id.strip_prefix(RECENT) {
        // Through the user path, so an import report shows; `file.openRecent` also drops a file that is gone.
        if app.run_user("file.openRecent", json!({"path": path})).is_some_and(|r| r.get("index").is_some()) {
            app.ui.home = false;
        }
    } else if let Some((plugin, command)) = id.strip_prefix(PLUGIN).and_then(|rest| rest.split_once(':')) {
        plugins_ui::start(app, plugin, command);
    } else if let Some(plugin) = id.strip_prefix(WIDGET) {
        plugins_ui::insert_widget(app, plugin);
    } else if crate::menus::available(id) {
        crate::menus::invoke(app, id);
    }
}

/// The viewport of the window in front: the menu bar's key equivalents go to it.
pub fn front(app: &FigApp) -> ViewportId {
    windows::viewport(app.windows.focused)
}

/// The menu bar for the window in front (its file, selection and panels).
pub fn menu_bar_in_front(app: &mut FigApp, windows: bool) -> craft_menu::MenuBar {
    let id = app.windows.focused;
    crate::windows::as_window(app, id, |app| menu_bar(app, windows)).unwrap_or_else(|| menu_bar(app, windows))
}

/// Run the item `id` chosen in the menu bar in the window in front.
pub fn run_in_front(app: &mut FigApp, id: &str) {
    let front = app.windows.focused;
    if windows::as_window(app, front, |app| run(app, id)).is_none() {
        run(app, id);
    }
}

/// Key presses from the menu bar's key equivalents that arrived while another window drew: the
/// window in front takes them in its next frame.
pub fn send_keys(app: &mut FigApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let id = app.windows.focused;
    if windows::as_window(app, id, |app| app.control_input.extend(events)).is_some() {
        ctx.request_repaint_of(windows::viewport(id));
    }
}

/// The pastes in the main window's input while another window is in front: the menu bar's ⌘V
/// asks the main window for the clipboard ([`craft_menu::NativeMenu::take_clicks`]), so its
/// answer goes on to the window in front ([`send_keys`]).
pub fn take_pastes(events: &mut Vec<egui::Event>) -> Vec<egui::Event> {
    let (pastes, rest) = std::mem::take(events).into_iter().partition(|e| matches!(e, egui::Event::Paste(_)));
    *events = rest;
    pastes
}
