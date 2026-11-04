//! Alt+Enter while editing text opens the link at the caret through the platform ("When editing
//! text, press ⌥Option Return/Alt Enter to activate a focused link", Accessibility at Figma),
//! driven through egui headless with a recording opener; editing goes on.

use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::Services;
use crate::layers_drag_tests::Ui;

/// Records every URL the app asks the platform to open.
#[derive(Default, Clone)]
struct Opened(Arc<Mutex<Vec<String>>>);

impl Services for Opened {
    fn open_url(&mut self, url: &str) {
        if let Ok(mut v) = self.0.lock() {
            v.push(url.to_string());
        }
    }
}

fn alt_enter(u: &mut Ui) {
    let modifiers = Modifiers::ALT;
    u.frame_with(
        vec![
            Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers },
            Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers },
        ],
        modifiers,
    );
    u.frame(vec![]);
}

#[test]
fn alt_enter_opens_the_link_at_the_caret_while_editing() {
    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "See the docs now", "x": 20, "y": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.style", &json!({"id": t, "start": 4, "end": 12, "hyperlink": "https://example.com/docs"})).unwrap();
    s.execute("text.edit.begin", &json!({"id": t, "caret": 2})).unwrap();
    let opened = Opened::default();
    let mut u = Ui::new(s);
    u.app.services = Box::new(opened.clone());
    u.frame(vec![]);
    alt_enter(&mut u);
    assert!(opened.0.lock().unwrap().is_empty(), "no link at the caret: nothing opens");
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "still editing");
    u.app.session.execute("text.edit.caret", &json!({"index": 6})).unwrap();
    alt_enter(&mut u);
    assert_eq!(*opened.0.lock().unwrap(), vec!["https://example.com/docs".to_string()]);
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "still editing");
    // Not a web or mail link: nothing opens.
    u.app.session.execute("text.style", &json!({"id": t, "start": 4, "end": 12, "hyperlink": "file:///etc/passwd"})).unwrap();
    alt_enter(&mut u);
    assert_eq!(opened.0.lock().unwrap().len(), 1);
}
