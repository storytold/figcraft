//! #1029, through the whole app: in a language whose labels run past Type settings' label column,
//! the popover keeps its width while it stays open. French does with Inter (release builds, the
//! report); Georgian does with egui's own fonts too (CI, which builds without craft-fonts). Its own
//! test binary, since the UI language is one setting for the whole process.
#![allow(clippy::unwrap_used)]

use egui::{Id, Popup, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

/// The Type settings popover's width over the frames it is open, the UI in `language`.
fn widths(language: &str) -> Vec<f32> {
    let mut s = Session::new();
    s.execute("pref.language", &json!({"language": language})).unwrap();
    // Truncating text also shows Max lines.
    let text = s.execute("text.create", &json!({"text": "Type", "props": {"textAutoResize": "truncate"}})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [text]})).unwrap();
    let ctx = egui::Context::default();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let open = Id::unique("ty_settings_open");
    let mut widths = Vec::new();
    for i in 0..40 {
        if i == 3 {
            Popup::open_id(&ctx, open);
        }
        let raw =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(i as f64 / 60.0), ..Default::default() };
        ctx.run_ui(raw, |ui| app.ui(ui)).textures_delta.clear();
        // From the frame after the one it opened on (egui's sizing pass).
        if i > 3 {
            assert!(Popup::is_id_open(&ctx, open), "{language}: Type settings closed on frame {i}");
            widths.push(ctx.memory(|m| m.area_rect(open)).unwrap().width());
        }
    }
    widths
}

#[test]
fn type_settings_keeps_its_width_with_labels_longer_than_their_column() {
    for language in ["ka", "fr"] {
        let w = widths(language);
        assert_ne!(figcraft_ui_egui::i18n::tr("Paragraph indent"), "Paragraph indent", "{language} is shown");
        // The width it asks for (`Popup::width`), every frame.
        assert!(w.iter().all(|x| (x - 264.0).abs() < 0.01), "{language}: Type settings widens: {} → {}", w[0], w[w.len() - 1]);
    }
}
