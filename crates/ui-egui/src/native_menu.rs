//! The main menu as a native menu bar (macOS, through `craft-menu`): the in-window menu's items,
//! labels, shortcuts and enabled and checked state, built from the same catalog, with Open recent
//! and the installed plugins.

use figcraft_engine::catalog::{MENU, SEP};
use figcraft_engine::keymap::{self, Platform};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{self, tr};

/// Item ids that carry a parameter: a recent file (`file.recent:<path>`), Clear recent, and a
/// plugin's command (`plugin:<plugin id>:<command id>`). A native menu reports only the id.
pub const RECENT: &str = "file.recent:";
pub const CLEAR_RECENT: &str = "file.recent.clear";
pub const PLUGIN: &str = "plugin:";

/// The menus the bar shows, in order (the in-window menu's top level).
const TOPS: &[&str] = &["File", "Edit", "View", "Object", "Text", "Arrange", "Vector", "Plugins", "Preferences", "Libraries", "Help"];

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
    let tops: Vec<&str> = TOPS.iter().map(|t| tr(t)).collect();
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

/// The installed plugins' commands, heading the Plugins menu as in the in-window menu: one item
/// for a plugin with one command, a submenu for one with several.
fn plugin_rows(app: &FigApp, rows: &mut Vec<craft_menu::Row>) {
    let top = tr("Plugins").to_string();
    let mut any = false;
    for p in &app.session.prefs.plugins {
        let m = &p.manifest;
        let item = |path: Vec<String>, command: &str, label: String| craft_menu::Row {
            path,
            id: format!("{PLUGIN}{}:{command}", m.id),
            label,
            shortcut: None,
            enabled: true,
            checked: None,
        };
        match m.commands.as_slice() {
            [] => {}
            [only] => {
                let label = if only.params.is_empty() { m.name.clone() } else { format!("{}…", m.name) };
                rows.push(item(vec![top.clone()], &only.id, label));
                any = true;
            }
            commands => {
                for c in commands {
                    rows.push(item(vec![top.clone(), m.name.clone()], &c.id, c.label.clone()));
                }
                any = true;
            }
        }
    }
    if any {
        rows.push(craft_menu::Row { path: vec![top], id: String::new(), label: "---".into(), shortcut: None, enabled: false, checked: None });
    }
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
        crate::plugins_ui::start(app, plugin, command);
    } else if crate::menus::available(id) {
        crate::menus::invoke(app, id);
    }
}
