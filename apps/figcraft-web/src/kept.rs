//! What the page keeps across reloads, in the browser's local storage: your name (Preferences ▸
//! Your name), so a reload or a later visit joins live sessions and comments as you.

use figcraft_engine::Session;
use serde_json::json;

/// Put back the name the page kept, checked as one typed in Preferences: the page's
/// storage is anyone's to change.
pub fn restore_name(session: &mut Session, kept: Option<String>) {
    if let Some(name) = kept
        && let Err(e) = session.execute("pref.userName", &json!({"name": name}))
    {
        log::warn!("the kept name wasn't used: {e}");
    }
}

#[cfg(test)]
mod tests {
    use figcraft_engine::Session;
    use serde_json::json;

    use super::restore_name;

    /// What one page kept, the next one restores; junk in the storage is cleaned like typing.
    #[test]
    fn the_name_survives_a_reload() {
        let mut before = Session::new();
        before.execute("pref.userName", &json!({"name": "  Alice "})).unwrap();
        let kept = before.prefs.user_name.clone();
        let mut after = Session::new();
        restore_name(&mut after, Some(kept));
        assert_eq!(after.prefs.user_name, "Alice");
        let mut fresh = Session::new();
        restore_name(&mut fresh, None);
        assert_eq!(fresh.prefs.user_name, Session::new().prefs.user_name, "nothing kept: the default");
        restore_name(&mut fresh, Some(format!("Bob\u{7}{}", "x".repeat(500))));
        assert!(fresh.prefs.user_name.starts_with("Bobx") && fresh.prefs.user_name.chars().count() == 64);
    }
}
