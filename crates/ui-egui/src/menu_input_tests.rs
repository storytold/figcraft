//! Menu and Actions entries that need an input must collect it in the UI, not send `{}` to the
//! engine (which would show a parameter error or silently use a default).

use figcraft_engine::Session;
use serde_json::json;

use crate::dialogs::MenuInput;
use crate::layers_drag_tests::Ui;
use crate::menus;

#[test]
fn preference_actions_open_preferences_without_changing_values() {
    let mut ui = Ui::new(Session::new());
    ui.app.session.prefs.language = "en".into();
    ui.app.session.prefs.publish_server = "wss://example.test/".into();
    let before = ui.app.session.prefs.clone();
    for id in ["pref.language", "pref.userName", "pref.publishServer", "pref.nudge"] {
        ui.app.ui.preferences = false;
        menus::invoke(&mut ui.app, id);
        assert!(ui.app.ui.preferences, "{id} did not open Preferences");
        assert!(ui.app.ui.last_error.is_none(), "{id} showed an error");
        assert_eq!(ui.app.session.prefs.language, before.language);
        assert_eq!(ui.app.session.prefs.publish_server, before.publish_server);
        assert_eq!(ui.app.session.prefs.nudge_small, before.nudge_small);
        assert_eq!(ui.app.session.prefs.nudge_big, before.nudge_big);
    }
    // Programmatic callers still need the actual argument.
    assert!(ui.app.session.execute("pref.language", &json!({})).is_err());
}

#[test]
fn menu_entries_collect_required_arguments_before_running() {
    let mut ui = Ui::new(Session::new());
    for (id, expected) in
        [("text.link", MenuInput::Link(String::new())), ("vector.offset", MenuInput::Offset(0.0)), ("object.moveToPage", MenuInput::MoveToPage)]
    {
        ui.app.ui.menu_input = None;
        menus::invoke(&mut ui.app, id);
        assert_eq!(ui.app.ui.menu_input, Some(expected), "{id}");
        assert!(ui.app.ui.last_error.is_none(), "{id} showed an error");
    }
    ui.frame(vec![]);
    assert!(ui.app.ui.last_error.is_none());
}

#[test]
fn palette_entries_with_existing_pickers_show_them() {
    let mut ui = Ui::new(Session::new());
    menus::invoke(&mut ui.app, "tool.set");
    assert_eq!(ui.app.ui.tool_flyout, Some("move"));
    ui.app.ui.left_collapsed = true;
    menus::invoke(&mut ui.app, "page.select");
    assert!(!ui.app.ui.left_collapsed);
    assert!(ui.app.ui.pages_open);
    menus::invoke(&mut ui.app, "file.openRecent");
    assert!(ui.app.ui.home);
    assert_eq!(ui.app.ui.home_view.section, crate::home::Section::Recents);
    assert!(ui.app.ui.last_error.is_none());
}

#[test]
fn batch_rename_opens_its_existing_dialog() {
    let mut session = Session::new();
    let id = session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone();
    session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let mut ui = Ui::new(session);
    menus::invoke(&mut ui.app, "object.batchRename");
    assert_eq!(ui.app.ui.batch_rename.as_ref().map(|d| d.ids.len()), Some(1));
    assert!(ui.app.ui.last_error.is_none());
}

/// Asserts `file.rename` opens the left panel's inline file-name editor (not a dialog),
/// focuses it, and commits the typed name on Enter.
#[test]
fn rename_file_edits_the_file_name_in_place() {
    let mut ui = Ui::new(Session::new());
    ui.app.ui.left_collapsed = true;
    menus::invoke(&mut ui.app, "file.rename");
    assert!(!ui.app.ui.left_collapsed, "the left panel shows the file name");
    assert!(ui.app.ui.menu_input.is_none(), "no dialog");
    ui.frame(vec![]);
    ui.frame(vec![]);
    assert!(!ui.app.ui.rename_file, "the rename started");
    assert!(ui.ctx.memory(|m| m.has_focus(egui::Id::unique("filename").with("te"))), "the file name's editor has the keyboard");
    ui.frame(vec![egui::Event::Text("Brand kit".into())]);
    ui.frame(vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    assert_eq!(ui.app.session.doc().unwrap().name, "Brand kit");
    assert!(ui.app.ui.last_error.is_none());
}
