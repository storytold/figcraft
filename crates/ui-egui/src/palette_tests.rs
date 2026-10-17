//! The Actions palette, driven headless: runs from it record recents (newest first, capped),
//! sections group commands by menu, a search covers every command with the best matches first,
//! Up/Down move the highlight and Enter runs it, gone entries drop out of Recents, a used plugin
//! resolves back to a row, and a press outside the panel closes it.

use egui::{Event, Key, Modifiers, Pos2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

#[test]
fn palette_runs_record_recents_newest_first_and_capped() {
    let mut u = Ui::new(Session::new());
    // Menus, shortcuts and context menus don't touch the palette's Recents.
    crate::menus::invoke(&mut u.app, "view.zoomIn");
    assert!(u.app.session.prefs.recent_actions.is_empty(), "only the palette records recents");
    for id in ["view.zoomIn", "view.zoomOut", "edit.copyAsText", "style.list", "edit.findReplace", "edit.copyAsCss", "view.zoomIn"] {
        crate::menus::run_action(&mut u.app, &format!("command:{id}"));
    }
    let r = &u.app.session.prefs.recent_actions;
    assert_eq!(r.len(), figcraft_engine::MAX_RECENT_ACTIONS, "recents are capped: {r:?}");
    assert_eq!(r[0], "command:view.zoomIn", "a re-run moves to the front: {r:?}");
    assert_eq!(r.iter().filter(|k| k.as_str() == "command:view.zoomIn").count(), 1, "no duplicates: {r:?}");
    assert!(!r.contains(&"command:view.zoomOut".to_string()), "the oldest drops out: {r:?}");
}

#[test]
fn sections_group_by_menu_in_catalog_order() {
    let groups = crate::menus::command_sections("");
    assert!(!groups.is_empty(), "the catalog has commands");
    assert_eq!(groups[0].0, "File", "catalog order starts at File");
    for (group, rows) in &groups {
        assert!(!rows.is_empty(), "{group} has no rows");
        for row in rows {
            assert!(row.key.starts_with("command:"), "unexpected key {}", row.key);
        }
    }
    let names: Vec<&str> = groups.iter().map(|(g, _)| g.as_str()).collect();
    assert!(names.contains(&"File") && names.contains(&"Edit"), "menus group: {names:?}");
    assert!(!names.iter().any(|g| g.starts_with("Plugins")), "plugins have their own tab");
}

#[test]
fn query_filters_to_matches() {
    let groups = crate::menus::command_sections("save");
    let labels: Vec<&str> = groups.iter().flat_map(|(_, rows)| rows.iter().map(|r| r.label.as_str())).collect();
    assert!(!labels.is_empty(), "something matches 'save'");
    assert!(labels.iter().all(|l| l.to_lowercase().contains("save")), "only matches: {labels:?}");
    assert!(crate::menus::command_sections("zzz-no-such-action").iter().all(|(_, r)| r.is_empty()));
}

#[test]
fn gone_entries_drop_out_of_recents() {
    let u = Ui::new(Session::new());
    assert!(crate::menus::recent_row(&u.app, "command:nope.nope", "").is_none(), "gone commands drop out");
    assert!(crate::menus::recent_row(&u.app, "plugin:nope.nope/nope", "").is_none(), "gone plugins drop out");
    assert!(crate::menus::recent_row(&u.app, "widget:nope", "").is_none(), "gone widgets drop out");
}

/// A used plugin (API 4, several commands) resolves back to a Recents row.
#[test]
fn used_plugin_resolves_to_a_recent_row() {
    let mut u = Ui::new(Session::new());
    let plugin: figcraft_engine::plugins::InstalledPlugin = serde_json::from_value(json!({
        "manifest": {
            "id": "dev.local.solar-icons",
            "name": "Solar icons",
            "main": "solar_icons.wasm",
            "api": 4,
            "commands": [
                {"id": "panel", "label": "Panel"},
                {"id": "grid", "label": "Grid"},
            ],
        },
    }))
    .unwrap();
    u.app.session.prefs.plugins.push(plugin);
    crate::menus::record_recent(&mut u.app, "plugin:dev.local.solar-icons/panel");
    let row = crate::menus::recent_row(&u.app, "plugin:dev.local.solar-icons/panel", "").expect("a used plugin stays in Recents");
    assert_eq!(row.label, "Solar icons — Panel");
    assert!(crate::menus::recent_row(&u.app, "plugin:dev.local.solar-icons/panel", "zzz").is_none(), "the query filters recents");
}

/// A press outside the panel closes it; the opening press is spared once.
#[test]
fn outside_press_closes_the_panel() {
    let mut u = Ui::new(Session::new());
    u.app.ui.palette = Some(String::new());
    u.app.ui.palette_armed = true;
    // The opening press: spared, and disarmed.
    u.button(Pos2::new(720.0, 20.0), true);
    u.button(Pos2::new(720.0, 20.0), false);
    assert!(u.app.ui.palette.is_some(), "the opening press is spared");
    // The next press outside: closed.
    u.button(Pos2::new(720.0, 20.0), true);
    u.button(Pos2::new(720.0, 20.0), false);
    assert!(u.app.ui.palette.is_none(), "an outside press closes the panel");
    // One click is enough: the spare expires on frames without a press.
    u.app.ui.palette = Some(String::new());
    u.app.ui.palette_armed = true;
    u.frame(vec![]);
    u.button(Pos2::new(720.0, 20.0), true);
    u.button(Pos2::new(720.0, 20.0), false);
    assert!(u.app.ui.palette.is_none(), "a single outside press closes the panel");
}

fn rows(groups: &[(String, Vec<crate::menus::ActionRow>)]) -> Vec<&crate::menus::ActionRow> {
    groups.iter().flat_map(|(_, rows)| rows).collect()
}

/// A search reaches commands no menu lists (in an "Other" group), not just menu items.
#[test]
fn search_covers_every_command() {
    let menu = figcraft_engine::catalog::MENU;
    let spec = figcraft_engine::all_commands().iter().find(|c| !menu.iter().any(|m| m.id == c.id)).expect("a command no menu lists");
    let groups = crate::menus::command_sections(spec.id);
    let key = format!("command:{}", spec.id);
    let group = groups.iter().find(|(_, rows)| rows.iter().any(|r| r.key == key)).map(|(g, _)| g.as_str());
    assert_eq!(group, Some("Other"), "{} is found: {:?}", spec.id, groups.iter().map(|(g, _)| g).collect::<Vec<_>>());
    // Without a query the palette lists the menus' items only.
    assert!(rows(&crate::menus::command_sections("")).iter().all(|r| r.key != key));
}

/// Labels starting with the query come first, the shortest first; the rest are capped.
#[test]
fn search_ranks_prefix_matches_first() {
    let groups = crate::menus::command_sections("zoom");
    let hits = rows(&groups);
    let first = hits.first().expect("something matches 'zoom'");
    assert!(first.label.to_lowercase().starts_with("zoom"), "a prefix match leads: {}", first.label);
    let shortest = hits.iter().filter(|r| r.label.to_lowercase().starts_with("zoom")).map(|r| r.label.chars().count()).min();
    assert_eq!(Some(first.label.chars().count()), shortest, "the shortest prefix match leads");
    assert!(rows(&crate::menus::command_sections("e")).len() <= 60, "a broad search is capped");
}

fn key(u: &mut Ui, k: Key) {
    u.frame(vec![
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
}

/// Up and Down move the highlight (it stops at the ends); Enter runs the highlighted row.
#[test]
fn arrows_move_the_highlight_and_enter_runs_it() {
    let mut u = Ui::new(Session::new());
    crate::menus::set_palette(&mut u.app.ui, true);
    u.frame(vec![]);
    u.frame(vec![Event::Text("zoom".into())]);
    assert_eq!(u.app.ui.palette.as_deref(), Some("zoom"));
    assert_eq!(u.app.ui.palette_hi, 0, "a search starts at the first row");
    key(&mut u, Key::ArrowUp);
    assert_eq!(u.app.ui.palette_hi, 0, "it stops at the top");
    key(&mut u, Key::ArrowDown);
    key(&mut u, Key::ArrowDown);
    key(&mut u, Key::ArrowUp);
    assert_eq!(u.app.ui.palette_hi, 1);
    let want = rows(&crate::menus::palette_sections(&u.app, "zoom", crate::menus::PaletteTab::All)).get(1).map(|r| r.key.clone()).expect("two rows");
    key(&mut u, Key::Enter);
    assert!(u.app.ui.palette.is_none(), "Enter runs a row and closes the palette");
    assert_eq!(u.app.session.prefs.recent_actions.first(), Some(&want), "the highlighted row ran");
}
