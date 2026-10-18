//! The native menu bar's tree: built from the catalog like the in-window menu, with its shortcuts,
//! enabled and checked state, Open recent and the plugins, and its items run as the in-window
//! menu's do, in the window in front.

use std::cell::RefCell;

use craft_menu::{ItemRole, LayoutOptions, MenuBar, MenuRole, Node};
use egui::{RawInput, ViewportId};
use figcraft_engine::Session;
use figcraft_engine::plugins::{InstalledPlugin, Manifest};
use serde_json::json;

use crate::native_menu::{CLEAR_RECENT, MAC_LABELS, RECENT, front, menu_bar, menu_bar_in_front, run, run_in_front, send_keys, take_pastes};
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

#[test]
fn every_label_the_mac_layout_writes_is_translated() {
    let asked = RefCell::new(Vec::new());
    let label = |english: &'static str| {
        asked.borrow_mut().push(english);
        english.to_string()
    };
    craft_menu::mac_layout(&menu_bar(&app(), true), &LayoutOptions { label: &label, ..LayoutOptions::new("FigCraft") });
    let asked = asked.into_inner();
    assert!(asked.contains(&"About {app}") && asked.contains(&"Quit {app}"), "{asked:?}");
    for english in &asked {
        assert!(MAC_LABELS.contains(english), "{english:?} is missing from MAC_LABELS");
    }
    // Every full catalog (not a regional overlay) has them: About doesn't fall back to English.
    for lang in crate::i18n::LANGS.iter().filter(|l| l.parent == 0) {
        let missing: Vec<_> = MAC_LABELS.iter().filter(|k| lang.ui.binary_search_by(|(key, _)| key.cmp(k)).is_err()).collect();
        assert!(missing.is_empty(), "{}: untranslated menu bar labels {missing:?}", lang.code);
    }
    let de = crate::i18n::LANGS.iter().find(|l| l.code == "de").unwrap();
    let about = de.ui.iter().find(|(k, _)| *k == "About {app}").map(|(_, t)| t);
    assert!(matches!(about, Some(crate::i18n::Text::Plain("Über {app}"))));
}

fn plugin(id: &str, name: &str, commands: &[&str], widget: bool) -> InstalledPlugin {
    let commands: Vec<_> = commands.iter().map(|c| json!({"id": c, "label": format!("Run {c}")})).collect();
    let manifest: Manifest =
        serde_json::from_value(json!({"id": id, "name": name, "version": "1.0.0", "main": "main.js", "commands": commands, "widget": widget}))
            .unwrap();
    InstalledPlugin { manifest, path: None, source: None }
}

#[test]
fn the_plugins_menu_lists_commands_and_widgets_as_in_the_window() {
    let mut a = app();
    a.session.prefs.plugins = vec![plugin("one", "One", &["go"], false), plugin("two", "Two", &["a", "b"], false), plugin("pad", "Pad", &[], true)];
    let bar = menu_bar(&a, true);
    let plugins = bar.menus.iter().find(|m| m.title == "Plugins").unwrap();
    assert!(matches!(&plugins.children[0], Node::Item(i) if i.id == "plugin:one:go" && i.label == "One"));
    assert!(matches!(&plugins.children[1], Node::Submenu { label, children, .. } if label == "Two" && children.len() == 2));
    assert!(
        matches!(&plugins.children[2], Node::Submenu { label, children, .. } if label == "Widgets" && matches!(&children[0], Node::Item(i) if i.id == "widget:pad"))
    );
    assert!(matches!(plugins.children[3], Node::Separator));
}

/// Files A and B, A in a second window in front, B in the main window.
fn two_windows() -> (egui::Context, FigApp, u64) {
    let mut s = Session::empty();
    for n in ["A", "B"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut a = FigApp::new(s, Box::new(NoServices));
    let w = crate::control::handle(&mut a, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    a.windows.focused = w;
    (ctx, a, w)
}

fn rulers(a: &FigApp, name: &str) -> bool {
    a.session.docs.iter().find(|d| d.doc.name == name).unwrap().view.rulers
}

#[test]
fn with_two_windows_the_menu_acts_on_the_one_in_front() {
    let (_ctx, mut a, w) = two_windows();
    assert_eq!(front(&a), crate::windows::viewport(w));
    run_in_front(&mut a, "view.rulers");
    assert!(rulers(&a, "A") && !rulers(&a, "B"), "in the front window's file");
    assert_eq!(a.session.ds().unwrap().doc.name, "B", "the main window keeps its own file");
    assert_eq!(item(&menu_bar_in_front(&mut a, true), "view.rulers").checked, Some(true));
    assert_eq!(item(&menu_bar(&a, true), "view.rulers").checked, Some(false), "the main window's state");
    // With the main window in front, the main window's.
    a.windows.focused = 0;
    assert_eq!(front(&a), ViewportId::ROOT);
    run_in_front(&mut a, "view.rulers");
    assert!(rulers(&a, "B"));
}

#[test]
fn key_equivalents_and_pastes_go_to_the_window_in_front() {
    let (ctx, mut a, w) = two_windows();
    let key = egui::Event::Key { key: egui::Key::S, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    send_keys(&mut a, &ctx, vec![key.clone()]);
    assert!(crate::windows::take_input(&mut a, ViewportId::ROOT).is_empty(), "not the main window's");
    assert_eq!(crate::windows::take_input(&mut a, crate::windows::viewport(w)), vec![key.clone()]);
    // The main window's input loses the paste it was asked for, and keeps the rest.
    let mut raw = RawInput { events: vec![key.clone(), egui::Event::Paste("text".into())], ..Default::default() };
    assert_eq!(take_pastes(&mut raw.events), vec![egui::Event::Paste("text".into())]);
    assert_eq!(raw.events, vec![key]);
}
