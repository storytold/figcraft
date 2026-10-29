//! Your name, one for comments and live sessions (#1380): "Your name" in Preferences, shown and
//! editable in the Join dialog, and kept as soon as it changes.

use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers};
use figcraft_engine::Prefs;

use crate::Services;
use crate::live_tests::{Harness, has_text, text_center};

/// Keeps the name of each preferences save.
struct Kept(Arc<Mutex<Vec<String>>>);

impl Services for Kept {
    fn save_prefs(&mut self, prefs: &Prefs) {
        self.0.lock().unwrap().push(prefs.user_name.clone());
    }
}

fn harness() -> (Harness, Arc<Mutex<Vec<String>>>) {
    let (mut h, _) = Harness::new();
    let kept = Arc::new(Mutex::new(Vec::new()));
    h.app.services = Box::new(Kept(kept.clone()));
    (h, kept)
}

/// Replace the text of the field showing `old` with `new` and press Enter.
fn retype(h: &mut Harness, old: &str, new: &str) {
    let shapes = h.frames(1);
    h.click(text_center(&shapes, old));
    let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    h.frame(vec![key(Key::A, Modifiers::COMMAND)]);
    h.frame(vec![Event::Text(new.into())]);
    h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    h.frames(1);
}

#[test]
fn the_join_dialog_shows_your_name_and_changes_it() {
    let (mut h, kept) = harness();
    h.app.session.execute("pref.userName", &serde_json::json!({"name": "Ada"})).unwrap();
    crate::menus::invoke(&mut h.app, "collab.join");
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "Your name") && has_text(&shapes, "Ada"), "the name others will see");
    retype(&mut h, "Ada", "Bob");
    assert_eq!(h.app.session.prefs.user_name, "Bob");
    assert_eq!(kept.lock().unwrap().last().map(String::as_str), Some("Bob"), "kept at once");
    assert!(h.app.ui.live.join.is_some(), "the dialog stays to join");
}

#[test]
fn preferences_call_it_your_name_and_the_default_shows_empty() {
    let (mut h, kept) = harness();
    h.app.ui.preferences = true;
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "Your name") && !has_text(&shapes, "Your name in comments"));
    assert!(!has_text(&shapes, "You"), "the default isn't a name to edit");
    h.app.session.execute("pref.userName", &serde_json::json!({"name": "Ada"})).unwrap();
    retype(&mut h, "Ada", "Lin");
    assert_eq!(h.app.session.prefs.user_name, "Lin");
    assert_eq!(kept.lock().unwrap().as_slice(), ["Lin"]);
}
