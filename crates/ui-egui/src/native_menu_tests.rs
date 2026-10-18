//! The native menu bar's tree: built from the catalog like the in-window menu, with its shortcuts,
//! enabled and checked state, Open recent and the plugins, and its items run as the in-window
//! menu's do.

use craft_menu::{ItemRole, LayoutOptions, MenuBar, MenuRole, Node};
use figcraft_engine::Session;
use serde_json::json;

use crate::native_menu::{CLEAR_RECENT, RECENT, menu_bar, run};
use crate::{FigApp, NoServices};

fn app() -> FigApp {
    FigApp::new(Session::new(), Box::new(NoServices))
}

fn item<'a>(bar: &'a MenuBar, id: &str) -> &'a craft_menu::Item {
    bar.find(id).unwrap_or_else(|| panic!("no item {id}"))
}

#[test]
fn the_bar_has_the_in_window_menus_in_order() {
    let bar = menu_bar(&app(), true);
    let titles: Vec<&str> = bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["File", "Edit", "View", "Object", "Text", "Arrange", "Vector", "Plugins", "Preferences", "Libraries", "Help"]);
    // Submenus keep their place: Edit ▸ Copy as.
    let edit = &bar.menus[1];
    assert!(edit.children.iter().any(|n| matches!(n, Node::Submenu { label, .. } if label == "Copy as")), "{:?}", edit.children);
}

#[test]
fn items_carry_their_shortcuts_and_state() {
    let mut a = app();
    let bar = menu_bar(&a, true);
    let save = item(&bar, "file.save");
    assert_eq!(save.label, "Save");
    assert_eq!(save.shortcut.as_deref(), Some("Cmd+S"));
    assert!(save.enabled);
    assert_eq!(item(&bar, "edit.redo").shortcut.as_deref(), Some("Shift+Cmd+Z"));
    // A View toggle shows its check, and follows it.
    assert_eq!(item(&bar, "view.rulers").checked, Some(false));
    a.run("view.rulers", json!({}));
    assert_eq!(item(&menu_bar(&a, true), "view.rulers").checked, Some(true));
    // New window only where the app can open windows.
    assert!(item(&bar, "window.new").enabled);
    assert!(!item(&menu_bar(&a, false), "window.new").enabled);
}

#[test]
fn open_recent_lists_the_files_and_clears_them() {
    let mut a = app();
    let dir = std::env::temp_dir().join(format!("figcraft-native-menu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let here = dir.join("here.figcraft");
    std::fs::write(&here, b"x").unwrap();
    let gone = dir.join("gone.figcraft");
    a.session.prefs.recent_files = vec![here.to_string_lossy().into_owned(), gone.to_string_lossy().into_owned()];
    let bar = menu_bar(&a, true);
    let shown = item(&bar, &format!("{RECENT}{}", here.display()));
    assert_eq!(shown.label, "here.figcraft");
    assert!(shown.enabled);
    assert!(!item(&bar, &format!("{RECENT}{}", gone.display())).enabled, "a file that is gone is greyed");
    assert!(item(&bar, CLEAR_RECENT).enabled);
    run(&mut a, CLEAR_RECENT);
    assert!(a.session.prefs.recent_files.is_empty());
    assert!(!item(&menu_bar(&a, true), CLEAR_RECENT).enabled);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chosen_items_run_their_commands() {
    let mut a = app();
    assert!(!a.session.ds().unwrap().view.rulers);
    run(&mut a, "view.rulers");
    assert!(a.session.ds().unwrap().view.rulers);
    // Unknown ids and greyed parameters do nothing.
    run(&mut a, "no.such.command");
    run(&mut a, "plugin:missing");
}

#[test]
fn the_mac_layout_moves_about_and_settings_to_the_app_menu() {
    let bar = menu_bar(&app(), true);
    let layout = craft_menu::mac_layout(&bar, &LayoutOptions::new("FigCraft"));
    let first = &layout.bar.menus[0];
    assert_eq!(first.role, MenuRole::App);
    let roles: Vec<Option<ItemRole>> = first
        .children
        .iter()
        .filter_map(|n| match n {
            Node::Item(i) => Some(i.role),
            _ => None,
        })
        .collect();
    assert!(roles.contains(&Some(ItemRole::About)) && roles.contains(&Some(ItemRole::Settings)), "{:?}", first.children);
    assert!(layout.bar.find("help.about").is_some_and(|i| i.role == Some(ItemRole::About)));
    // A Window menu is added; Help stays last.
    let titles: Vec<&str> = layout.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert!(titles.contains(&"Window"), "{titles:?}");
    assert_eq!(titles.last(), Some(&"Help"));
}
