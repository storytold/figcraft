//! Shift+I opens the Assets panel (#710), from a collapsed sidebar and from the version history too.

use egui::{Event, Key, Modifiers};
use figcraft_engine::Session;

use crate::LeftTab;
use crate::layers_drag_tests::Ui;

fn shift_i(u: &mut Ui) {
    let modifiers = Modifiers::SHIFT;
    u.frame(vec![
        Event::Key { key: Key::I, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key: Key::I, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
    u.frame(vec![]);
}

fn assets_shown(u: &Ui) -> bool {
    u.app.ui.left_tab == LeftTab::Assets && !u.app.ui.left_collapsed && !u.app.ui.versions.open
}

/// The three states the sidebar can be in when Shift+I is pressed.
fn starts() -> Vec<(&'static str, Ui)> {
    let shown = Ui::new(Session::new());
    let mut collapsed = Ui::new(Session::new());
    collapsed.app.ui.left_collapsed = true;
    let mut versions = Ui::new(Session::new());
    crate::versions::open(&mut versions.app);
    vec![("layers", shown), ("collapsed", collapsed), ("version history", versions)]
}

#[test]
fn shift_i_opens_the_assets_panel() {
    for (name, mut u) in starts() {
        u.frame(vec![]);
        shift_i(&mut u);
        assert!(assets_shown(&u), "{name}: the Assets panel is open");
    }
}
