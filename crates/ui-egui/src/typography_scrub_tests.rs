//! Regression for #373: actual pointer gestures on the Typography section's three leading icons.
use egui::{Event, Id, Modifiers, Pos2, RawInput, Rect, vec2};
use figcraft_doc::{LetterSpacing, LineHeight, NodeId};
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

struct Fields {
    app: FigApp,
    ctx: egui::Context,
    time: f64,
    text: NodeId,
}
impl Fields {
    fn new() -> Self {
        let mut s = Session::new();
        let text = NodeId(
            s.execute("text.create", &json!({"text":"Typography","props":{"lineHeight":24,"letterSpacing":0,"paragraphSpacing":12}})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("select.set", &json!({"ids":[text]})).unwrap();
        let mut f = Self { app: FigApp::new(s, Box::new(NoServices)), ctx: egui::Context::default(), time: 0.0, text };
        crate::theme::install_fonts(&f.ctx, false, 0);
        crate::theme::apply(&f.ctx, crate::theme::Tokens::light());
        for _ in 0..3 {
            f.frame(vec![], Modifiers::NONE);
        }
        f
    }
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) -> egui::CursorIcon {
        self.time += 1.0 / 60.0;
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 600.0))), time: Some(self.time), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| {
            let ds = self.app.session.ds().unwrap();
            let doc = ds.doc.clone();
            let sel = ds.selection.clone();
            crate::typography::typography(&mut self.app, ui, &doc, &sel);
        });
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    }
    fn lead(&self, key: &str) -> Pos2 {
        let rect = self.ctx.read_response(Id::new(key).with("click")).unwrap().rect;
        Pos2::new(rect.min.x + 12.0, rect.center().y)
    }
    fn drag(&mut self, key: &str, by: f32, modifiers: Modifiers) {
        let p = self.lead(key);
        self.frame(vec![Event::PointerMoved(p)], modifiers);
        self.frame(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers }], modifiers);
        for step in 1..=4 {
            self.frame(vec![Event::PointerMoved(p + vec2(by * step as f32 / 4.0, 0.0))], modifiers);
        }
        self.frame(vec![Event::PointerButton { pos: p + vec2(by, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers }], modifiers);
        self.frame(vec![], Modifiers::NONE);
    }
    fn value(&self, key: &str) -> f64 {
        let t = self.app.session.doc().unwrap().node(self.text).unwrap().kind.text().unwrap();
        match key {
            "ty_lh" => match t.style.line_height {
                LineHeight::Pixels(v) | LineHeight::Percent(v) => v,
                _ => -1.0,
            },
            "ty_ls" => match t.style.letter_spacing {
                LetterSpacing::Pixels(v) | LetterSpacing::Percent(v) => v,
            },
            _ => t.paragraph_spacing,
        }
    }
}

#[test]
fn typography_icon_drag_changes_each_spacing_field_in_one_undo_step() {
    for (key, before) in [("ty_lh", 24.0), ("ty_ls", 0.0), ("ty_ps", 12.0)] {
        let mut f = Fields::new();
        let undo = f.app.session.undo_depth().0;
        f.drag(key, 16.0, Modifiers::NONE);
        assert_eq!(f.value(key), before + 16.0, "{key}: dragging the icon must change the value");
        assert_eq!(f.app.session.undo_depth().0, undo + 1, "{key}: one gesture, one undo step");
        f.app.session.undo().unwrap();
        assert_eq!(f.value(key), before);
        f.app.session.redo().unwrap();
        assert_eq!(f.value(key), before + 16.0);
    }
}

#[test]
fn typography_icon_scrub_uses_shift_and_alt_rates_and_the_resize_cursor() {
    for (modifiers, delta) in [(Modifiers::SHIFT, 160.0), (Modifiers::ALT, 1.6)] {
        for (key, start) in [("ty_lh", 24.0), ("ty_ls", 0.0), ("ty_ps", 12.0)] {
            let mut f = Fields::new();
            let p = f.lead(key);
            assert_eq!(f.frame(vec![Event::PointerMoved(p)], modifiers), egui::CursorIcon::ResizeHorizontal);
            f.drag(key, 16.0, modifiers);
            assert!((f.value(key) - start - delta).abs() < 1e-8, "{key}: {}", f.value(key));
        }
    }
}

#[test]
fn typography_icon_scrub_preserves_units_and_clamps_only_nonnegative_fields() {
    let mut f = Fields::new();
    f.app.run("node.set", json!({"id":f.text,"props":{"lineHeight":"150%","letterSpacing":"2px"}}));
    f.frame(vec![], Modifiers::NONE);
    f.drag("ty_lh", 16.0, Modifiers::NONE);
    f.drag("ty_ls", -16.0, Modifiers::NONE);
    let t = f.app.session.doc().unwrap().node(f.text).unwrap().kind.text().unwrap();
    assert_eq!(t.style.line_height, LineHeight::Percent(166.0));
    assert_eq!(t.style.letter_spacing, LetterSpacing::Pixels(-14.0));
    f.drag("ty_lh", -200.0, Modifiers::NONE);
    f.drag("ty_ps", -200.0, Modifiers::NONE);
    assert_eq!(f.value("ty_lh"), 0.0);
    assert_eq!(f.value("ty_ps"), 0.0);
    assert!(f.app.ui.last_error.is_none(), "{:?}", f.app.ui.last_error);
}

#[test]
fn typography_icon_scrub_does_not_replace_auto_or_mixed_values() {
    let mut f = Fields::new();
    f.app.run("node.set", json!({"id":f.text,"props":{"lineHeight":"auto"}}));
    f.frame(vec![], Modifiers::NONE);
    let undo = f.app.session.undo_depth().0;
    f.drag("ty_lh", 16.0, Modifiers::NONE);
    assert_eq!(f.app.session.doc().unwrap().node(f.text).unwrap().kind.text().unwrap().style.line_height, LineHeight::Auto);
    assert_eq!(f.app.session.undo_depth().0, undo);
    let other = f.app.run("text.create", json!({"text":"Other","props":{"lineHeight":36}})).unwrap()["id"].clone();
    f.app.run("select.set", json!({"ids":[f.text,other]}));
    f.frame(vec![], Modifiers::NONE);
    let before = f.app.session.doc().unwrap().clone();
    let undo = f.app.session.undo_depth().0;
    f.drag("ty_lh", 16.0, Modifiers::NONE);
    assert_eq!(f.app.session.doc().unwrap(), &before);
    assert_eq!(f.app.session.undo_depth().0, undo);
}

#[test]
fn typography_icon_scrub_changes_only_the_selected_characters_without_ending_text_editing() {
    let mut f = Fields::new();
    let undo = f.app.session.undo_depth().0;
    figcraft_engine::textedit::begin(&mut f.app.session, f.text, Some(1), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut f.app.session, 4, true).unwrap();
    f.frame(vec![], Modifiers::NONE);
    f.drag("ty_ls", 16.0, Modifiers::NONE);
    let t = f.app.session.doc().unwrap().node(f.text).unwrap().kind.text().unwrap();
    assert_eq!(t.style_at(0).letter_spacing, LetterSpacing::Pixels(0.0));
    assert_eq!(t.style_at(1).letter_spacing, LetterSpacing::Pixels(16.0));
    assert_eq!(t.style_at(4).letter_spacing, LetterSpacing::Pixels(0.0));
    assert!(f.app.session.ds().unwrap().text_edit.is_some());
    assert_eq!(f.app.session.undo_depth().0, undo);
    figcraft_engine::textedit::end(&mut f.app.session).unwrap();
    assert_eq!(f.app.session.undo_depth().0, undo + 1);
    f.app.session.undo().unwrap();
    assert_eq!(f.value("ty_ls"), 0.0);
}

#[test]
fn typography_spacing_fields_still_accept_typed_units_and_escape_cancels() {
    let mut f = Fields::new();
    let p = f.lead("ty_lh") + vec2(36.0, 0.0);
    for pressed in [true, false] {
        f.frame(
            vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }],
            Modifiers::NONE,
        );
    }
    f.frame(vec![], Modifiers::NONE);
    assert!(f.ctx.data(|d| d.get_temp::<String>(Id::new("ty_lh").with("editing"))).is_some(), "click starts typing");
    f.ctx.memory_mut(|m| m.request_focus(Id::new("ty_lh").with("te")));
    f.frame(
        vec![Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }],
        Modifiers::COMMAND,
    );
    f.frame(vec![Event::Text("150%".into())], Modifiers::NONE);
    assert_eq!(
        f.ctx.data(|d| d.get_temp::<String>(Id::new("ty_lh").with("editing"))).as_deref(),
        Some("150%"),
        "keyboard selects and replaces the field text"
    );
    f.frame(
        vec![Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        Modifiers::NONE,
    );
    assert_eq!(f.app.session.doc().unwrap().node(f.text).unwrap().kind.text().unwrap().style.line_height, LineHeight::Percent(150.0));
    let undo = f.app.session.undo_depth().0;
    for pressed in [true, false] {
        f.frame(
            vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }],
            Modifiers::NONE,
        );
    }
    f.ctx.memory_mut(|m| m.request_focus(Id::new("ty_lh").with("te")));
    f.frame(vec![Event::Text("999".into())], Modifiers::NONE);
    f.frame(
        vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        Modifiers::NONE,
    );
    assert_eq!(f.value("ty_lh"), 150.0);
    assert_eq!(f.app.session.undo_depth().0, undo);
}
