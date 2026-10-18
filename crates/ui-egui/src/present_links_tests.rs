//! A prototype's Open link action carries a URL from the document and fires from whatever trigger
//! the prototype sets, including a delay, so the player hands the opener only a web link
//! ([`figcraft_engine::doc_urls`]). Without that, a saved document opens `javascript:` in the web
//! build's own origin (stored XSS) and `smb:`/`ms-…:` through the desktop's handlers.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::Services;
use crate::present_tests::{Ui as Harness, id};

/// Records every URL the player asks the platform to open.
#[derive(Default)]
struct Opened(Arc<Mutex<Vec<String>>>);

impl Services for Opened {
    fn open_url(&mut self, url: &str) {
        if let Ok(mut v) = self.0.lock() {
            v.push(url.to_string());
        }
    }
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// A presented frame whose layer opens `url` 100ms after the screen appears — no click anywhere.
fn opens_after_a_delay(url: &str) -> Vec<String> {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": f})));
    ex(
        &mut s,
        "proto.interaction.add",
        json!({"id": layer, "trigger": {"type": "afterDelay", "ms": 100}, "action": {"type": "openUrl", "url": url}, "transition": {"type": "instant"}}),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut u = Harness::new(s);
    u.app.services = Box::new(Opened(seen.clone()));
    u.run("proto.present", json!({"start": f}));
    // Well past the delay.
    u.frames(40, Vec::new);
    seen.lock().map(|v| v.clone()).unwrap_or_default()
}

#[test]
fn a_prototype_opens_web_links() {
    assert_eq!(opens_after_a_delay("https://example.com/a"), ["https://example.com/a"]);
    assert_eq!(opens_after_a_delay("mailto:a@b.c"), ["mailto:a@b.c"]);
}

#[test]
fn a_prototype_cannot_open_a_script_or_handler_url() {
    for url in [
        // In the web build the opener is `window.open`, which runs this in the app's own origin.
        "javascript:alert(document.cookie)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        // Reaches the file system; a UNC host also sends Windows credentials.
        "file:///etc/passwd",
        "smb://attacker.example/share/x",
        // Handed to an application handler by the desktop.
        "ms-msdt:/id PCWDiagnostic",
        "figcraft://join?host=127.0.0.1:47900&file=f&token=t",
    ] {
        assert!(opens_after_a_delay(url).is_empty(), "{url:?} must never reach the opener");
    }
}
