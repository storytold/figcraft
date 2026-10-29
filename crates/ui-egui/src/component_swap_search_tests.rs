//! The search at the top of an instance's swap property menu, driven through egui headless
//! (#1338): typing filters the components by name, an empty result says so, Escape clears the
//! search and then closes the menu, and Up, Down and Enter pick a match.

use egui::{Event, Key, Modifiers, Pos2};
use figcraft_doc::{NodeId, NodeKind};
use serde_json::json;

use crate::component_swap_default_tests::{centre, click, default_combo, setup, tree};
use crate::layers_drag_tests::Ui;

fn key(u: &mut Ui, key: Key) {
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
}

fn typed(u: &mut Ui, text: &str) {
    u.frame(vec![Event::Text(text.into())]);
    u.frame(vec![]);
}

/// Whether the menu shows a row or text `name` (one that wasn't on screen, at `before`, before it
/// opened).
fn shown(u: &mut Ui, before: &[Pos2], name: &str) -> bool {
    tree(u).iter().any(|n| n.label().or(n.value()) == Some(name) && n.bounds().is_some() && !before.contains(&centre(n)))
}

fn component(u: &Ui, name: &str) -> NodeId {
    let doc = &u.app.session.ds().unwrap().doc;
    doc.nodes().find(|n| n.name == name && matches!(n.kind, NodeKind::Component { .. })).unwrap().id
}

#[test]
fn the_swap_menu_filters_as_you_type_and_picks_with_the_keyboard() {
    let (mut u, button, _) = setup();
    let r = u.app.session.execute("instance.create", &json!({"component": button.0, "x": 900, "y": 0})).unwrap();
    let inst = r["id"].as_u64().unwrap();
    u.app.session.execute("select.set", &json!({"ids": [inst]})).unwrap();
    let combo = default_combo(&mut u).expect("the instance's Icon property has a dropdown");
    assert_eq!(combo.value(), Some("Heart"));
    let before: Vec<Pos2> = tree(&mut u).iter().filter(|n| n.label().is_some()).map(centre).collect();
    // Open it: the search field has the keyboard, so typing filters straight away.
    click(&mut u, centre(&combo));
    assert!(shown(&mut u, &before, "Glyph 1") && shown(&mut u, &before, "Zebra"));
    typed(&mut u, "zEB");
    assert!(shown(&mut u, &before, "Zebra"), "a match, in any case");
    assert!(!shown(&mut u, &before, "Glyph 1"), "the rest are filtered out");
    typed(&mut u, "x");
    assert!(shown(&mut u, &before, "No matching components"));
    assert!(!shown(&mut u, &before, "Zebra"));
    // Escape clears the search; the menu stays open with everything again, still taking the keyboard.
    key(&mut u, Key::Escape);
    assert!(shown(&mut u, &before, "Glyph 1") && shown(&mut u, &before, "Zebra"));
    // Down twice to the second match, Enter picks it and closes the menu.
    typed(&mut u, "glyph 7");
    key(&mut u, Key::ArrowDown);
    key(&mut u, Key::ArrowDown);
    key(&mut u, Key::Enter);
    let props = u.app.session.execute("component.properties", &json!({"id": inst})).unwrap();
    assert_eq!(props["properties"][0]["value"], json!(component(&u, "Glyph 70").0), "the second of Glyph 7, Glyph 70…");
    assert!(!shown(&mut u, &before, "Glyph 7"), "picking closes the menu");
    // With the search empty, Escape closes the menu.
    let combo = default_combo(&mut u).unwrap();
    click(&mut u, centre(&combo));
    assert!(shown(&mut u, &before, "Glyph 1"));
    key(&mut u, Key::Escape);
    assert!(!shown(&mut u, &before, "Glyph 1"), "closed");
}
