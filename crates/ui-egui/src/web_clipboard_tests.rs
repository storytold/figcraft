//! Copy and paste as the browser does them: Cmd+C arrives as a copy event and the copy reaches the
//! system clipboard only through egui's output; Cmd+V arrives as the key and a paste event holding
//! whatever the system clipboard has. A copy that never reached the clipboard pasted the text that
//! was there before as a text layer instead of the copied layers.

use egui::{Event, Key, Modifiers, OutputCommand, Pos2, RawInput, Rect, vec2};
use figcraft_doc::NodeKind;
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, Services, egui_copy_png, egui_copy_text};

/// The browser shell's clipboard: writes go out as egui output commands.
struct Browser(egui::Context);

impl Services for Browser {
    fn clipboard_set_text(&mut self, s: &str) {
        egui_copy_text(&self.0, s);
    }
    fn clipboard_set_png(&mut self, w: u32, h: u32, rgba: &[u8]) {
        egui_copy_png(&self.0, w, h, rgba);
    }
}

struct Page {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
    /// The system clipboard's text.
    clipboard: String,
}

impl Page {
    fn new() -> Self {
        let mut s = Session::new();
        let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
        s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
        let ctx = egui::Context::default();
        let app = FigApp::new(s, Box::new(Browser(ctx.clone())));
        let mut p = Self { ctx, app, time: 0.0, clipboard: "text copied in another app".into() };
        for _ in 0..3 {
            p.frame(vec![]);
        }
        p
    }

    /// One frame; copies in its output land on the clipboard, as eframe writes them.
    fn frame(&mut self, events: Vec<Event>) -> Vec<OutputCommand> {
        self.frame_then(events, |_| {})
    }

    /// One frame that runs `then` after the app's own UI (a menu item clicked during the frame).
    fn frame_then(&mut self, events: Vec<Event>, then: impl FnOnce(&mut FigApp)) -> Vec<OutputCommand> {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut then = Some(then);
        let mut out = self.ctx.run_ui(input, |ui| {
            app.ui(ui);
            if let Some(f) = then.take() {
                f(app);
            }
        });
        out.textures_delta.clear();
        for c in &out.platform_output.commands {
            if let OutputCommand::CopyText(t) = c {
                self.clipboard = t.clone();
            }
        }
        out.platform_output.commands
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::MAC_CMD | Modifiers::COMMAND }
    }

    fn copy(&mut self) {
        self.frame(vec![Self::key(Key::C, true), Event::Copy]);
        self.frame(vec![Self::key(Key::C, false)]);
    }

    fn paste(&mut self) {
        let text = self.clipboard.clone();
        self.frame(vec![Self::key(Key::V, true), Event::Paste(text)]);
        self.frame(vec![Self::key(Key::V, false)]);
    }

    fn count(&self, kind: fn(&NodeKind) -> bool) -> usize {
        self.app.session.doc().unwrap().nodes().filter(|n| kind(&n.kind)).count()
    }
}

fn rect(k: &NodeKind) -> bool {
    matches!(k, NodeKind::Rectangle)
}

fn text(k: &NodeKind) -> bool {
    matches!(k, NodeKind::Text(_))
}

#[test]
fn cmd_c_then_cmd_v_pastes_the_copied_layers() {
    let mut p = Page::new();
    p.copy();
    assert_ne!(p.clipboard, "text copied in another app", "the copy reached the system clipboard");
    p.paste();
    assert_eq!(p.count(rect), 2, "the rectangle was pasted");
    assert_eq!(p.count(text), 0, "no text layer from the clipboard's old text");
    p.paste();
    assert_eq!(p.count(rect), 3, "and again");
}

#[test]
fn text_copied_elsewhere_after_our_copy_still_pastes_as_text() {
    let mut p = Page::new();
    p.copy();
    p.clipboard = "hello from another app".into();
    p.paste();
    assert_eq!(p.count(rect), 1);
    assert_eq!(p.count(text), 1, "newer text from elsewhere becomes a text layer");
}

#[test]
fn copy_as_png_puts_an_image_on_the_clipboard() {
    let mut p = Page::new();
    let out = p.frame_then(vec![], |app| crate::menus::invoke(app, "edit.copyAsPng"));
    let img = out.iter().find_map(|c| if let OutputCommand::CopyImage(i) = c { Some(i.clone()) } else { None });
    let img = img.expect("an image was copied");
    assert!(img.size[0] > 0 && img.size[1] > 0);
}

#[test]
fn a_mismatched_image_buffer_is_dropped() {
    let ctx = egui::Context::default();
    for (w, h, rgba) in [(4, 4, &[0u8; 10][..]), (u32::MAX, u32::MAX, &[][..])] {
        let mut out = ctx.run_ui(RawInput::default(), |ui| egui_copy_png(ui.ctx(), w, h, rgba));
        out.textures_delta.clear();
        assert!(out.platform_output.commands.is_empty(), "{w}x{h}");
    }
}
