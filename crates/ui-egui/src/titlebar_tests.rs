//! The frameless window's title bar, driven through egui headless: its buttons minimise, maximise
//! and close the window, its empty space drags (double-click maximises), and the window's edges
//! resize it. A system-drawn title bar leaves all of that to the system.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, ResizeDirection, ViewportCommand, ViewportId, pos2, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices, TitleBar};

const SIZE: egui::Vec2 = vec2(1440.0, 900.0);

struct Ui {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
    /// What the system reports about the window.
    maximized: bool,
}

impl Ui {
    fn new(title_bar: TitleBar) -> Self {
        let mut app = FigApp::new(Session::new(), Box::new(NoServices));
        app.ui.title_bar = title_bar;
        let mut u = Self { ctx: egui::Context::default(), app, time: 0.0, maximized: false };
        for _ in 0..3 {
            u.frame(vec![]);
        }
        u
    }

    /// Run one frame; returns the window commands it sent.
    fn frame(&mut self, events: Vec<Event>) -> Vec<ViewportCommand> {
        self.time += 1.0 / 60.0;
        let mut input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)), time: Some(self.time), events, ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().maximized = Some(self.maximized);
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
    }

    fn button(&mut self, p: Pos2, pressed: bool) -> Vec<ViewportCommand> {
        self.frame(vec![
            Event::PointerMoved(p),
            Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() },
        ])
    }

    fn click(&mut self, p: Pos2) -> Vec<ViewportCommand> {
        let mut cmds = self.button(p, true);
        cmds.extend(self.button(p, false));
        cmds
    }

    /// Press on `p` and move away from it; the commands sent while dragging.
    fn drag(&mut self, p: Pos2, by: egui::Vec2) -> Vec<ViewportCommand> {
        let mut cmds = self.button(p, true);
        for k in 1..=5 {
            cmds.extend(self.frame(vec![Event::PointerMoved(p + by * (k as f32 / 5.0))]));
        }
        cmds.extend(self.button(p + by, false));
        cmds
    }
}

/// The middle of window button `k` (0 minimise, 1 maximise, 2 close) at the bar's right end.
fn window_button(k: usize) -> Pos2 {
    pos2(SIZE.x - 46.0 * (3 - k) as f32 + 23.0, crate::tabs::HEIGHT / 2.0)
}

/// Empty title bar space, right of the tabs.
const EMPTY: Pos2 = pos2(800.0, 20.0);

#[test]
fn window_buttons_minimise_maximise_and_close() {
    let mut u = Ui::new(TitleBar::Custom);
    assert!(u.click(window_button(0)).contains(&ViewportCommand::Minimized(true)));
    assert!(u.click(window_button(1)).contains(&ViewportCommand::Maximized(true)));
    u.click(window_button(2));
    assert!(u.frame(vec![]).contains(&ViewportCommand::Close));
}

#[test]
fn titlebar_close_asks_over_the_presenter_when_a_file_is_unsaved() {
    let mut u = Ui::new(TitleBar::Custom);
    let frame = u.app.session.execute("node.create", &json!({"type": "frame", "width": 402, "height": 874})).unwrap()["id"].clone();
    u.app.session.execute("proto.present", &json!({"start": frame})).unwrap();
    u.app.handle_ui_requests();
    u.frame(vec![]);
    assert!(u.app.ui.present.is_some());

    let commands = u.click(window_button(2));
    assert!(!commands.contains(&ViewportCommand::Close));
    assert!(u.app.ui.confirm_close);
    assert!(u.ctx.memory(|memory| memory.area_rect(egui::Id::new(Some(crate::i18n::tr("Unsaved changes")))).is_some()));
}

#[test]
fn maximise_button_restores_a_maximised_window() {
    let mut u = Ui::new(TitleBar::Custom);
    u.maximized = true;
    let cmds = u.click(window_button(1));
    assert!(cmds.contains(&ViewportCommand::Maximized(false)), "{cmds:?}");
}

#[test]
fn empty_bar_drags_the_window_and_double_click_maximises() {
    let mut u = Ui::new(TitleBar::Custom);
    assert!(u.drag(EMPTY, vec2(40.0, 10.0)).contains(&ViewportCommand::StartDrag));
    let mut cmds = u.click(EMPTY);
    cmds.extend(u.click(EMPTY));
    assert!(cmds.contains(&ViewportCommand::Maximized(true)), "{cmds:?}");
    // A single click on the bar does nothing to the window.
    u.time += 1.0;
    assert!(u.click(EMPTY).is_empty());
}

#[test]
fn edges_and_corners_resize_the_window() {
    let mut u = Ui::new(TitleBar::Custom);
    // Not while maximised.
    u.maximized = true;
    assert!(u.drag(pos2(SIZE.x - 2.0, SIZE.y - 2.0), vec2(-30.0, -30.0)).iter().all(|c| !matches!(c, ViewportCommand::BeginResize(_))));
    u.maximized = false;
    u.frame(vec![]);
    for (p, dir) in [
        (pos2(SIZE.x - 2.0, SIZE.y - 2.0), ResizeDirection::SouthEast),
        (pos2(SIZE.x - 2.0, 2.0), ResizeDirection::NorthEast),
        (pos2(2.0, SIZE.y - 2.0), ResizeDirection::SouthWest),
        (pos2(2.0, 450.0), ResizeDirection::West),
        (pos2(720.0, SIZE.y - 2.0), ResizeDirection::South),
        (pos2(720.0, 2.0), ResizeDirection::North),
    ] {
        // Pulled in towards the middle.
        let inward = ((SIZE / 2.0).to_pos2() - p).normalized() * 30.0;
        assert!(u.drag(p, inward).contains(&ViewportCommand::BeginResize(dir)), "{dir:?}");
    }
}

#[test]
fn tabs_and_new_tab_keep_their_clicks() {
    let mut u = Ui::new(TitleBar::Custom);
    u.app.ui.home = true;
    // The open file's tab switches to it.
    assert!(u.click(pos2(100.0, 20.0)).is_empty());
    assert!(!u.app.ui.home);
    // The `+` right of it opens a new file (found by clicking along the bar past the tab).
    let mut opened = false;
    for x in (176..320).step_by(4) {
        u.time += 1.0;
        assert!(u.click(pos2(x as f32, 20.0)).is_empty(), "{x}");
        if u.app.session.docs.len() > 1 {
            opened = true;
            break;
        }
    }
    assert!(opened, "the new-tab button took no click");
}

#[test]
fn a_system_title_bar_leaves_the_window_alone() {
    let mut u = Ui::new(TitleBar::Native);
    assert!(!u.click(window_button(2)).contains(&ViewportCommand::Close));
    assert!(u.drag(EMPTY, vec2(40.0, 10.0)).is_empty());
    assert!(u.drag(pos2(SIZE.x - 2.0, SIZE.y - 2.0), vec2(-30.0, -30.0)).iter().all(|c| !matches!(c, ViewportCommand::BeginResize(_))));
}
