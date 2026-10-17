//! The toolbar's Resources button and Shift+I open the Assets panel (#710), from a collapsed
//! sidebar and from the version history too.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Pos2, pos2};
use figcraft_engine::Session;

use crate::LeftTab;
use crate::layers_drag_tests::Ui;

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

fn shift_i(u: &mut Ui) {
    let modifiers = Modifiers::SHIFT;
    u.frame(vec![
        Event::Key { key: Key::I, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key: Key::I, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
    u.frame(vec![]);
}

/// The middle of the toolbar's Resources button, found in the AccessKit tree.
fn resources_button(u: &mut Ui) -> Pos2 {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Button && n.label() == Some("Resources")).expect("Resources");
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

fn assets_shown(u: &Ui) -> bool {
    u.app.ui.left_tab == LeftTab::Assets && !u.app.ui.left_collapsed && !u.app.ui.versions.open
}

/// The three states the sidebar can be in when Resources is asked for.
fn starts() -> Vec<(&'static str, Ui)> {
    let shown = Ui::new(Session::new());
    let mut collapsed = Ui::new(Session::new());
    collapsed.app.ui.left_collapsed = true;
    let mut versions = Ui::new(Session::new());
    crate::versions::open(&mut versions.app);
    vec![("layers", shown), ("collapsed", collapsed), ("version history", versions)]
}

#[test]
fn the_resources_button_opens_the_assets_panel() {
    for (name, mut u) in starts() {
        let at = resources_button(&mut u);
        click(&mut u, at);
        assert!(assets_shown(&u), "{name}: the Assets panel is open");
    }
}

#[test]
fn shift_i_opens_the_assets_panel() {
    for (name, mut u) in starts() {
        u.frame(vec![]);
        shift_i(&mut u);
        assert!(assets_shown(&u), "{name}: the Assets panel is open");
    }
}
