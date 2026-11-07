//! #1051: where the platform can list the fonts installed on this computer only when asked (the
//! browser's Local Font Access), the font menu offers "Use fonts installed on this computer…";
//! clicking it asks the platform once. Elsewhere the row isn't there.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{Event, Key};
use serde_json::json;

use crate::Services;
use crate::live_tests::{Harness, has_text, text_center};
use crate::present_tests::id;

const ROW: &str = "Use fonts installed on this computer…";

/// A browser that can list the installed fonts: counts how often it is asked.
struct Lists(Arc<AtomicUsize>);

impl Services for Lists {
    fn can_ask_for_local_fonts(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }
    fn ask_for_local_fonts(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// The font menu of a selected text layer, opened: what it shows.
fn font_menu(h: &mut Harness) -> Vec<egui::Shape> {
    let family = h.app.session.ds().ok().and_then(|ds| ds.selection.first().copied()).and_then(|t| {
        let doc = h.app.session.doc().ok()?;
        match &doc.node(t)?.kind {
            figcraft_doc::NodeKind::Text(td) => Some(td.style.font_family.clone()),
            _ => None,
        }
    });
    let family = family.unwrap_or_else(|| panic!("no text layer selected"));
    let shapes = h.frames(3);
    h.click(text_center(&shapes, &family));
    h.frames(2)
}

fn escape(h: &mut Harness) {
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
}

#[test]
fn the_font_menu_offers_the_fonts_on_this_computer_where_they_must_be_asked_for() {
    let (mut h, _) = Harness::new();
    let t = id(&h.app.session.execute("text.create", &json!({"text": "Hello", "x": 100, "y": 300})).unwrap());
    h.app.session.execute("select.set", &json!({"ids": [t]})).unwrap();
    // The desktop sees its fonts anyway.
    assert!(!has_text(&font_menu(&mut h), ROW));
    escape(&mut h);
    let asked = Arc::new(AtomicUsize::new(0));
    h.app.services = Box::new(Lists(asked.clone()));
    let shapes = font_menu(&mut h);
    assert!(has_text(&shapes, ROW));
    h.click(text_center(&shapes, ROW));
    h.frames(2);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "asked once");
    // Once asked, the menu lists the fonts and doesn't offer the row again.
    assert!(!has_text(&font_menu(&mut h), ROW));
}
