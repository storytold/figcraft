//! The credits tabs as drawn, through egui headless with FigCraft's fonts and theme: every column
//! keeps its measured width and scrolls when needed, switching the Contributors view keeps its size, clicking a header sorts
//! by it, and empty tables (no contributors.json) draw a note instead.

use egui::{Event, Pos2, RawInput, Rect, vec2};

use super::*;
use crate::dialogs::ABOUT_WIDTH;

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// `n` contributors with distinct, realistic-sized counts (six-digit line counts, a long login).
fn people(n: u64) -> Vec<Contributor> {
    (0..n)
        .map(|i| Contributor {
            login: if i == 0 { "nullpointerexception" } else { leak(format!("user{i:02}")) },
            display_name: None,
            real_name: None,
            prs: 100 + i,
            commits: 1000 + i,
            lines_added: 277_000 + i,
            lines_deleted: 22_000 + i,
            binary_added: 30 + i,
            binary_deleted: i,
            first_commit: leak(format!("2026-09-{:02}T00:00:00Z", 1 + i % 28)),
            last_commit: "2026-10-07T00:00:00Z",
        })
        .collect()
}

fn model(name: &'static str, commits: u64) -> Model {
    Model { company: "Anthropic", model: name, version: "5.5", commits, lines_added: 150_000 + commits, lines_deleted: 20_000 }
}

/// A headless egui with FigCraft's fonts and light theme; `draw` runs inside a fixed-width column.
struct Ui {
    ctx: egui::Context,
    time: f64,
}

impl Ui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        crate::theme::apply(&ctx, Tokens::light());
        let mut u = Self { ctx, time: 0.0 };
        // Fonts set now take effect on the next frame.
        u.frame(vec![], |_| {});
        u
    }

    fn set_view(&self, v: View) {
        self.ctx.data_mut(|d| d.insert_temp(egui::Id::new("credits_view"), v));
    }

    fn view(&self) -> View {
        self.ctx.data(|d| d.get_temp::<View>(egui::Id::new("credits_view"))).unwrap_or_default()
    }

    /// One frame; returns the rect `draw` filled.
    fn frame(&mut self, events: Vec<Event>, mut draw: impl FnMut(&mut egui::Ui)) -> Rect {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(ABOUT_WIDTH + 40.0, 600.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut drawn = Rect::NOTHING;
        self.ctx
            .run_ui(input, |ui| {
                ui.set_width(ABOUT_WIDTH);
                drawn = ui.scope(|ui| draw(ui)).response.rect;
            })
            .textures_delta
            .clear();
        drawn
    }

    fn click(&mut self, at: Pos2, draw: impl Fn(&mut egui::Ui)) {
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.frame(vec![Event::PointerMoved(at)], &draw);
        self.frame(vec![press(true)], &draw);
        self.frame(vec![press(false)], &draw);
        self.frame(vec![], &draw);
    }

    /// Run `f` with a `Ui` from a frame (for measuring text).
    fn measure<R>(&mut self, f: impl FnOnce(&egui::Ui) -> R) -> R {
        let mut f = Some(f);
        let mut out = None;
        self.frame(vec![], |ui| {
            if let Some(f) = f.take() {
                out = Some(f(ui));
            }
        });
        out.unwrap_or_else(|| panic!("no frame"))
    }
}

/// Keep measured columns and the name minimum, using horizontal scrolling when the font
/// needs more than the About window's width. No font is required to have Inter's metrics.
fn assert_table_columns_fit_or_scroll(u: &mut Ui, widths: &[Option<f32>]) {
    let mut columns = Vec::new();
    let mut minimum = 0.0;
    let mut scroll = 0.0;
    let mut drawn = Rect::NOTHING;
    for _ in 0..3 {
        columns.clear();
        drawn = u.frame(vec![], |ui| {
            scroll = ui.spacing().scroll.allocated_width();
            minimum = widths.iter().flatten().sum::<f32>() + scroll + NAME_MIN_W;
            credits_table(
                ui,
                "credits_width_test",
                widths,
                NAME_MIN_W,
                LIST_HEIGHT,
                |mut header| {
                    for i in 0..widths.len() {
                        header.col(|ui| {
                            columns.push((ui.max_rect(), ui.clip_rect()));
                            if i + 1 == widths.len() {
                                ui.scroll_to_rect_animation(ui.max_rect(), Some(egui::Align::Max), egui::style::ScrollAnimation::none());
                            }
                        });
                    }
                },
                |_| {},
            );
        });
    }
    assert!(drawn.width() <= ABOUT_WIDTH + 0.5, "table viewport grew to {} pt", drawn.width());
    assert_eq!(columns.len(), widths.len());
    for ((rect, _), width) in columns.iter().zip(widths) {
        if let Some(width) = width {
            assert!((rect.width() - width).abs() < 0.5, "measured column {width} became {} pt", rect.width());
        } else {
            assert!(rect.width() >= NAME_MIN_W - 0.5, "name column shrank to {} pt", rect.width());
        }
    }
    let content_width = columns.iter().map(|(rect, _)| rect.width()).sum::<f32>() + scroll;
    assert!((content_width - ABOUT_WIDTH.max(minimum)).abs() < 0.5, "content needs {minimum} pt, got {content_width}");
    let (last, clip) = columns.last().unwrap();
    assert!((last.intersect(*clip).width() - last.width()).abs() < 0.5, "last column cannot be scrolled fully into view: {last:?} in {clip:?}");
    if minimum <= ABOUT_WIDTH {
        let (first, clip) = columns.first().unwrap();
        assert!((first.intersect(*clip).width() - first.width()).abs() < 0.5, "a fitting table scrolled its first column out of view");
    }
}

#[test]
fn columns_keep_their_measured_widths_with_different_font_sizes() {
    for scale in [0.75, 1.0, 1.5] {
        let mut u = Ui::new();
        u.ctx.global_style_mut(|style| {
            style.text_styles.get_mut(&egui::TextStyle::Body).unwrap().size *= scale;
        });
        let list = people(8);
        let refs: Vec<&Contributor> = list.iter().collect();
        let widths = u.measure(|ui| contributor_widths(ui, &refs));
        assert_table_columns_fit_or_scroll(&mut u, &widths);
        let models: Vec<Model> = (0..8).map(|i| model(leak(format!("Claude Model {i}")), [228, 45, 6, 2, 2, 1, 1, 1][i])).collect();
        let rows: Vec<[String; 6]> = models.iter().map(|m| model_cells(m, 482)).collect();
        let widths = u.measure(|ui| model_widths(ui, &rows));
        assert_table_columns_fit_or_scroll(&mut u, &widths);
    }
}

#[test]
fn switching_the_contributors_view_keeps_its_size() {
    let list = people(60);
    let mut u = Ui::new();
    let mut height = |table| {
        u.set_view(View { table, ..View::default() });
        let mut r = Rect::NOTHING;
        for _ in 0..3 {
            r = u.frame(vec![], |ui| contributors_in(ui, &list, 1));
        }
        r.height()
    };
    let (bag, table) = (height(false), height(true));
    assert!((bag - table).abs() < 0.5, "{bag} pt tall as a grab bag, {table} pt as a table");
}

#[test]
fn clicking_a_header_sorts_by_it() {
    let list = people(12);
    let draw = |ui: &mut egui::Ui| contributors_in(ui, &list, 1);
    let mut u = Ui::new();
    u.set_view(View { table: true, ..View::default() });
    let mut r = Rect::NOTHING;
    for _ in 0..3 {
        r = u.frame(vec![], draw);
    }
    // The table fills the bottom LIST_HEIGHT of the tab; its header row is on top.
    let header_y = r.bottom() - LIST_HEIGHT + TABLE_HEADER_H / 2.0;
    let refs: Vec<&Contributor> = list.iter().collect();
    let widths = u.measure(|ui| contributor_widths(ui, &refs));
    let fixed: f32 = widths.iter().flatten().sum();
    let scroll = u.measure(|ui| ui.spacing().scroll.allocated_width());
    let name_w = (ABOUT_WIDTH - fixed - scroll).max(NAME_MIN_W);
    // The middle of column `i` (0 is the name).
    let x = |i: usize| r.left() + name_w + widths.iter().skip(1).take(i - 1).flatten().sum::<f32>() + widths[i].unwrap_or(0.0) / 2.0;
    assert_eq!((u.view().key, u.view().ascending), (SortKey::FirstCommit, true));
    let commits = SortKey::ALL.iter().position(|k| *k == SortKey::Commits).unwrap_or(0);
    u.click(Pos2::new(x(commits), header_y), draw);
    assert_eq!((u.view().key, u.view().ascending), (SortKey::Commits, false), "a count sorts biggest first");
    u.click(Pos2::new(x(commits), header_y), draw);
    assert_eq!((u.view().key, u.view().ascending), (SortKey::Commits, true), "clicking again reverses");
    let last = SortKey::ALL.len() - 1;
    u.click(Pos2::new(x(last), header_y), draw);
    assert_eq!((u.view().key, u.view().ascending), (SortKey::LastCommit, true), "a date sorts oldest first");
}

#[test]
fn empty_tables_draw_a_note() {
    // What a build without contributors.json shows: no table, no panic, on every view.
    let mut u = Ui::new();
    for table in [false, true] {
        u.set_view(View { table, ..View::default() });
        let r = u.frame(vec![], |ui| contributors_in(ui, &[], 0));
        assert!(r.height() > 0.0 && r.height() < LIST_HEIGHT, "{r:?}");
    }
    let r = u.frame(vec![], |ui| models_in(ui, &[], 0));
    assert!(r.height() > 0.0 && r.height() < 40.0, "{r:?}");
}

#[test]
fn the_models_table_is_as_tall_as_its_rows() {
    let mut u = Ui::new();
    let few = [model("Claude Opus", 80)];
    let many: Vec<Model> = (0..40).map(|i| model(leak(format!("Model {i}")), i)).collect();
    let mut h = |models: &[Model]| {
        let mut r = Rect::NOTHING;
        for _ in 0..2 {
            r = u.frame(vec![], |ui| models_in(ui, models, 81));
        }
        r.height()
    };
    let (one, lots) = (h(&few), h(&many));
    assert!(one < 3.0 * TABLE_ROW_H + TABLE_HEADER_H, "one model takes {one} pt");
    assert!(lots <= LIST_HEIGHT + 1.0, "40 models take {lots} pt");
}

#[test]
fn the_about_window_draws_every_tab() {
    // The whole app, with the compiled-in credits (whatever contributors.json held, or none).
    let ctx = egui::Context::default();
    let mut app = crate::FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
    app.ui.about = true;
    for (i, tab) in crate::dialogs::AboutTab::ALL.into_iter().enumerate() {
        app.ui.about_tab = tab;
        for f in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                time: Some((i * 3 + f) as f64 / 60.0),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
        }
        assert!(app.ui.about, "{tab:?}: the About window closed");
        assert_eq!(app.ui.about_tab, tab);
    }
}
