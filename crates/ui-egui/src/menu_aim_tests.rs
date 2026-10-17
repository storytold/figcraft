//! Which submenu the main and context menus show, driven through egui headless: a closed menu
//! reopens with no submenu open (also one level down), hovering another row closes the open
//! submenu at once while a submenu opens only once the pointer stays on its row a moment
//! (sweeping across rows doesn't flash each one), and on the way from a
//! submenu's row to its submenu the rows the pointer crosses don't take over (the safe triangle),
//! unless it rests on one or turns away.

use egui::{Event, Id, LayerId, Order, Pos2, RawInput, Rect, pos2, vec2};
use figcraft_engine::Session;

use crate::menus::{AIM_REST, HOVER_DELAY, heading_for, in_triangle, main_menu_id};
use crate::{FigApp, NoServices};

const SIZE: egui::Vec2 = vec2(1440.0, 900.0);
const ROW: f32 = 28.0;

struct Ui {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Ui {
    fn new() -> Self {
        let app = FigApp::new(Session::new(), Box::new(NoServices));
        let mut u = Self { ctx: egui::Context::default(), app, time: 0.0 };
        for _ in 0..3 {
            u.frame(vec![]);
        }
        u
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    fn hover(&mut self, p: Pos2) {
        self.frame(vec![Event::PointerMoved(p)]);
    }

    /// Hover `p` and stay there until a submenu could open.
    fn settle(&mut self, p: Pos2) {
        self.hover(p);
        self.wait(HOVER_DELAY + 0.05);
    }

    /// Frames without input for `secs`.
    fn wait(&mut self, secs: f64) {
        let until = self.time + secs;
        while self.time < until {
            self.frame(vec![]);
        }
    }

    fn open_main_menu(&mut self) {
        self.app.ui.main_menu = true;
        // Somewhere off the menu: nothing hovered.
        self.hover(pos2(900.0, 700.0));
        self.frame(vec![]);
    }

    fn shown(&self, area: Id) -> bool {
        self.ctx.memory(|m| m.areas().visible_layer_ids().contains(&LayerId::new(Order::Foreground, area)))
    }

    fn rect(&self, area: Id) -> Rect {
        self.ctx.memory(|m| m.area_rect(area)).expect("area drawn")
    }

    /// The main menu's row for top-level menu `i` (File 0, Edit 1, View 2, …).
    fn top_row(&self, i: usize) -> Rect {
        let m = self.rect(main_menu_id());
        // Frame margin, the Actions row and the space under it.
        let y = m.min.y + 8.0 + 30.0 + 6.0 + ROW * i as f32;
        Rect::from_min_max(pos2(m.min.x + 8.0, y), pos2(m.max.x - 8.0, y + ROW))
    }

    /// Which top-level menus' panels show.
    fn tops_shown(&self) -> Vec<&'static str> {
        TOPS.iter().copied().filter(|t| self.shown(top(t))).collect()
    }
}

const TOPS: [&str; 6] = ["File", "Edit", "View", "Object", "Text", "Arrange"];

fn top(name: &str) -> Id {
    Id::new(("menu", name))
}

/// The submenus of top-level menu `menu` (their keys), from the menu catalog.
fn submenu_keys(menu: &str) -> Vec<String> {
    let mut keys: Vec<String> = figcraft_engine::catalog::MENU
        .iter()
        .filter_map(|m| m.menu.strip_prefix(menu).and_then(|r| r.strip_prefix('/')).map(|r| format!("{menu}/{}", r.split('/').next().unwrap_or(r))))
        .collect();
    keys.extend(figcraft_engine::catalog::MENU.iter().filter(|m| m.menu == menu && m.id == "file.openRecent").map(|m| format!("{menu}/{}", m.label)));
    keys.dedup();
    keys
}

#[test]
fn the_triangle_takes_the_pointer_from_the_row_toward_the_panel() {
    let panel = Rect::from_min_max(pos2(300.0, 50.0), pos2(500.0, 650.0));
    let from = pos2(250.0, 100.0);
    assert!(in_triangle(pos2(1.0, 1.0), pos2(0.0, 0.0), pos2(4.0, 0.0), pos2(0.0, 4.0)));
    assert!(!in_triangle(pos2(3.0, 3.0), pos2(0.0, 0.0), pos2(4.0, 0.0), pos2(0.0, 4.0)));
    // Diagonally down toward the panel: heading for it.
    assert!(heading_for(from, pos2(260.0, 130.0), panel));
    // Straight down, straight up, back left, or far below the panel's reach: not.
    assert!(!heading_for(from, pos2(250.0, 130.0), panel));
    assert!(!heading_for(from, pos2(250.0, 80.0), panel));
    assert!(!heading_for(from, pos2(240.0, 105.0), panel));
    assert!(!heading_for(from, pos2(260.0, 400.0), Rect::from_min_max(pos2(300.0, 50.0), pos2(500.0, 120.0))));
    // A panel on the left works the same way, mirrored.
    let left = Rect::from_min_max(pos2(0.0, 50.0), pos2(200.0, 650.0));
    assert!(heading_for(from, pos2(240.0, 130.0), left));
    assert!(!heading_for(from, pos2(260.0, 130.0), left));
    // Junk positions never count.
    assert!(!heading_for(pos2(f32::NAN, 0.0), from, panel));
}

#[test]
fn a_reopened_main_menu_shows_no_stale_submenu() {
    let mut u = Ui::new();
    u.open_main_menu();
    assert!(u.tops_shown().is_empty(), "nothing open before hovering");
    u.settle(u.top_row(0).center());
    assert_eq!(u.tops_shown(), ["File"]);
    // Escape closes it; it reopens without File open.
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    u.frame(vec![]);
    assert!(!u.app.ui.main_menu);
    u.open_main_menu();
    assert!(u.tops_shown().is_empty(), "reopened with {:?} open", u.tops_shown());
    // Closed from outside (the menu button toggling it) it forgets too.
    u.settle(u.top_row(1).center());
    assert_eq!(u.tops_shown(), ["Edit"]);
    u.app.ui.main_menu = false;
    u.frame(vec![]);
    u.open_main_menu();
    assert!(u.tops_shown().is_empty(), "reopened with {:?} open", u.tops_shown());
}

#[test]
fn a_submenu_one_level_down_reopens_closed() {
    let mut u = Ui::new();
    u.open_main_menu();
    u.settle(u.top_row(0).center());
    let keys = submenu_keys("File");
    assert!(!keys.is_empty(), "File has submenus");
    let sub_shown = |u: &Ui| keys.iter().find(|k| u.shown(Id::new(("submenu", k.as_str())))).cloned();
    // Down the middle of File's panel until one of its submenus opens.
    let panel = u.rect(top("File"));
    let mut y = panel.min.y + 10.0;
    while sub_shown(&u).is_none() && y < panel.max.y {
        u.settle(pos2(panel.center().x, y));
        y += 7.0;
    }
    let Some(open) = sub_shown(&u) else { panic!("no File submenu opened (looked for {keys:?})") };
    // Over to Edit and back to File: File shows, without its submenu.
    u.hover(u.top_row(1).center());
    u.wait(AIM_REST + 0.1);
    assert_eq!(u.tops_shown(), ["Edit"]);
    u.hover(u.top_row(0).center());
    u.wait(AIM_REST + 0.1);
    assert_eq!(u.tops_shown(), ["File"]);
    assert_eq!(sub_shown(&u), None, "{open} came back open");
}

#[test]
fn crossing_rows_toward_the_open_submenu_keeps_it_open() {
    let mut u = Ui::new();
    u.open_main_menu();
    let file = u.top_row(0);
    let start = pos2(file.max.x - 30.0, file.center().y);
    u.settle(start);
    assert_eq!(u.tops_shown(), ["File"]);
    // Diagonally across Edit and View into File's panel, a few rows down.
    let panel = u.rect(top("File"));
    let end = pos2(panel.min.x + 20.0, u.top_row(3).center().y);
    for k in 1..=8 {
        let p = start + (end - start) * (k as f32 / 8.0);
        u.hover(p);
        assert_eq!(u.tops_shown(), ["File"], "at {p:?}");
    }
    assert!(panel.contains(end));
    u.wait(AIM_REST + 0.2);
    assert_eq!(u.tops_shown(), ["File"]);
}

#[test]
fn resting_on_a_crossed_row_hands_over_to_it() {
    let mut u = Ui::new();
    u.open_main_menu();
    let file = u.top_row(0);
    let start = pos2(file.max.x - 30.0, file.center().y);
    u.settle(start);
    // Toward the panel, stopping on View.
    let view = u.top_row(2);
    let stop = pos2(view.max.x - 4.0, view.center().y);
    for k in 1..=4 {
        u.hover(start + (stop - start) * (k as f32 / 4.0));
    }
    assert_eq!(u.tops_shown(), ["File"], "still on the way");
    u.wait(AIM_REST + 0.1);
    assert_eq!(u.tops_shown(), ["View"]);
}

#[test]
fn moving_straight_down_switches_after_a_short_hover() {
    let mut u = Ui::new();
    u.open_main_menu();
    let file = u.top_row(0);
    u.settle(file.center());
    assert_eq!(u.tops_shown(), ["File"]);
    // Down to Edit in small steps: no progress toward File's panel, so no triangle.
    let edit = u.top_row(1).center();
    let mut p = file.center();
    while p.y < edit.y {
        p.y = (p.y + 4.0).min(edit.y);
        u.hover(p);
    }
    assert!(u.tops_shown().is_empty(), "File closes at once, Edit waits: {:?}", u.tops_shown());
    u.wait(HOVER_DELAY + 0.05);
    assert_eq!(u.tops_shown(), ["Edit"]);
}

#[test]
fn the_first_submenu_opens_after_a_short_hover() {
    let mut u = Ui::new();
    u.open_main_menu();
    u.hover(u.top_row(0).center());
    u.frame(vec![]);
    assert!(u.tops_shown().is_empty(), "opened at once");
    u.wait(HOVER_DELAY + 0.05);
    assert_eq!(u.tops_shown(), ["File"]);
}

#[test]
fn sweeping_across_rows_opens_only_where_the_pointer_stops() {
    let mut u = Ui::new();
    u.open_main_menu();
    u.settle(u.top_row(0).center());
    // Quickly down the left of the rows, a row a frame, to Arrange.
    for i in 1..=5 {
        let r = u.top_row(i);
        u.hover(pos2(r.min.x + 20.0, r.center().y));
        // (A panel drawn above the row the pointer is on goes the next frame.)
        u.frame(vec![]);
        assert!(u.tops_shown().is_empty(), "{:?} open on row {i}", u.tops_shown());
    }
    u.wait(HOVER_DELAY + 0.05);
    assert_eq!(u.tops_shown(), ["Arrange"]);
}

#[test]
fn hovering_a_plain_row_closes_the_open_submenu_at_once() {
    let mut u = Ui::new();
    u.open_main_menu();
    u.settle(u.top_row(0).center());
    assert_eq!(u.tops_shown(), ["File"]);
    // Up onto the Actions row.
    let m = u.rect(main_menu_id());
    u.hover(pos2(u.top_row(0).center().x, m.min.y + 8.0 + 15.0));
    assert!(u.tops_shown().is_empty(), "{:?} still open", u.tops_shown());
}

#[test]
fn leaving_the_menu_keeps_the_last_submenu() {
    let mut u = Ui::new();
    u.open_main_menu();
    u.settle(u.top_row(1).center());
    // Off the menus onto the canvas.
    u.hover(pos2(900.0, 700.0));
    u.wait(AIM_REST + 0.1);
    assert_eq!(u.tops_shown(), ["Edit"]);
}
