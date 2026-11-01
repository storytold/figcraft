//! The font family menu follows its list's height while it is open: filtering to nothing shrinks
//! it, clearing the filter grows it back, and the search field stays where it is (egui#5225).
use egui::accesskit::{Node, Role};
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

struct Panel {
    app: FigApp,
    ctx: egui::Context,
    time: f64,
}

impl Panel {
    fn new() -> Self {
        let mut s = Session::new();
        let text = s.execute("text.create", &json!({"text": "Typography"})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [text]})).unwrap();
        let mut p = Self { app: FigApp::new(s, Box::new(NoServices)), ctx: egui::Context::default(), time: 0.0 };
        crate::theme::install_fonts(&p.ctx, false, 0);
        crate::theme::apply(&p.ctx, crate::theme::Tokens::light());
        p.ctx.enable_accesskit();
        for _ in 0..3 {
            p.frame(vec![]);
        }
        p
    }

    fn frame(&mut self, events: Vec<Event>) -> Vec<Node> {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 800.0))), time: Some(self.time), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| {
            let ds = self.app.session.ds().unwrap();
            let doc = ds.doc.clone();
            let sel = ds.selection.clone();
            crate::typography::typography(&mut self.app, ui, &doc, &sel);
        });
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
    }

    /// Settled frames; returns the last frame's accessibility tree.
    fn settle(&mut self, events: Vec<Event>) -> Vec<Node> {
        let mut nodes = self.frame(events);
        for _ in 0..4 {
            nodes = self.frame(vec![]);
        }
        nodes
    }

    fn click(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.frame(vec![button(true)]);
        self.settle(vec![button(false)]);
    }

    /// The popup's rectangle: the one foreground area that wasn't there before the menu opened.
    fn menu_rect(&self, before: &[egui::LayerId]) -> Rect {
        let layers: Vec<egui::LayerId> = self.ctx.memory(|m| m.layer_ids().collect());
        layers
            .into_iter()
            .filter(|l| l.order == egui::Order::Foreground && !before.contains(l))
            .filter_map(|l| self.ctx.memory(|m| m.area_rect(l.id)))
            .max_by(|a, b| a.height().total_cmp(&b.height()))
            .expect("the font menu is open")
    }
}

fn centre(n: &Node) -> Pos2 {
    let b = n.bounds().unwrap();
    Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// The top of the search field: the text input as wide as `menu` (the panel's own fields are narrower).
fn search_top(nodes: &[Node], menu: Rect) -> f32 {
    let field = nodes
        .iter()
        .filter(|n| n.role() == Role::TextInput)
        .filter_map(|n| n.bounds())
        .find(|b| menu.contains(Pos2::new(b.x0 as f32 + 1.0, b.y0 as f32 + 1.0)) && b.x1 - b.x0 > f64::from(menu.width()) - 40.0)
        .expect("the menu has a search field");
    field.y0 as f32
}

#[test]
fn the_font_menu_regrows_after_a_search_matches_nothing() {
    figcraft_text::scan_system_fonts();
    if figcraft_text::family_names().len() < 3 {
        eprintln!("fewer than three font families; skipping");
        return;
    }
    let mut p = Panel::new();
    let nodes = p.settle(vec![]);
    let family =
        nodes.iter().find(|n| n.role() == Role::ComboBox && n.value() == Some("Inter")).expect("the Typography section has a font family field");
    let before: Vec<egui::LayerId> = p.ctx.memory(|m| m.layer_ids().collect());
    p.click(centre(family));
    let nodes = p.settle(vec![]);
    let open = p.menu_rect(&before);
    let top = search_top(&nodes, open);
    assert!(open.height() > 100.0, "the menu lists fonts: {open:?}");
    assert!(open.width() >= 240.0, "the menu keeps the 240 minimum width under a narrower field: {open:?}");

    let type_text = |p: &mut Panel, s: &str| p.settle(vec![Event::Text(s.into())]);
    let nodes = type_text(&mut p, "zzzzqqqq");
    let none = p.menu_rect(&before);
    assert!(none.height() < open.height() - 20.0, "no match shrinks the menu: {open:?} → {none:?}");
    assert_eq!(search_top(&nodes, none), top, "the search field stays put while the list is empty");

    let mut nodes = nodes;
    for _ in 0.."zzzzqqqq".len() {
        let key = Event::Key { key: Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        nodes = p.settle(vec![key]);
    }
    let again = p.menu_rect(&before);
    assert!((again.height() - open.height()).abs() < 1.0, "clearing the search grows the menu back: {open:?} → {again:?}");
    assert_eq!(search_top(&nodes, again), top, "the search field stays put when the list grows back");
}
