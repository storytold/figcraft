//! The Variables editor in its two presentations: the floating panel it opens as (moved by its
//! header), and the full view the header's arrow expands it into, which takes the editor panels'
//! place. One body of drawing code serves both; `Geom` is the difference.

use egui::{Pos2, Rect, Shape, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};
use crate::variables_panel::{Geom, Presentation, Rename};

/// The test window's screen.
fn screen() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))
}

/// The editor open on a fresh document, drawn for a few frames.
fn editor() -> Ui {
    let mut u = Ui::new(Session::new());
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u
}

/// A press and a release at `p`, then a frame.
fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

#[test]
fn a_floating_panel_is_centred_until_it_is_moved_and_kept_on_screen() {
    let g = Geom::floating(screen(), None, true);
    assert!(g.floating);
    assert_eq!(g.size(), vec2(Geom::FLOAT_W, Geom::FLOAT_H));
    assert_eq!(g.panel.center(), screen().center(), "centred on its first frame");
    // A remembered position is honoured...
    let g = Geom::floating(screen(), Some(Pos2::new(120.0, 60.0)), true);
    assert_eq!(g.panel.min, Pos2::new(120.0, 60.0));
    // ...and pulled back on screen, whole.
    let g = Geom::floating(screen(), Some(Pos2::new(-400.0, 5000.0)), true);
    assert_eq!(g.panel.min, Pos2::new(0.0, screen().max.y - Geom::FLOAT_H));
    assert_eq!(g.panel.max.x, Geom::FLOAT_W);
}

#[test]
fn a_panel_bigger_than_the_screen_shrinks_to_its_smallest_size() {
    let small = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 300.0));
    let g = Geom::floating(small, None, true);
    assert_eq!(g.size(), vec2(Geom::MIN_W, Geom::MIN_H));
    assert_eq!(g.panel.min, Pos2::new(10.0, 30.0), "still centred");
}

#[test]
fn the_full_view_is_the_area_the_editor_panels_would_have_taken() {
    let area = Rect::from_min_size(Pos2::new(56.0, 40.0), vec2(900.0, 700.0));
    let g = Geom::full(area, true);
    assert!(!g.floating);
    assert_eq!(g.panel, area);
    assert_eq!(g.body_height(), area.height() - Geom::HEADER_H);
    // The collections sidebar and its 1 pt line come off the table's width.
    assert_eq!(g.table_width(), area.width() - 265.0);
}

#[test]
fn the_presentations_are_named_for_the_control_channel() {
    assert_eq!(Presentation::default(), Presentation::Floating, "the editor opens as a panel");
    assert_eq!(Presentation::from_name("full"), Some(Presentation::Full));
    assert_eq!(Presentation::from_name("floating"), Some(Presentation::Floating));
    assert_eq!(Presentation::from_name("sideways"), None);
    assert_eq!(Presentation::Full.name(), "full");
    assert_eq!(Presentation::Floating.name(), "floating");
}

#[test]
fn the_header_toggle_expands_to_the_full_view_and_back() {
    let mut u = editor();
    let st = u.app.ui.variables.clone().expect("open");
    assert_eq!(st.presentation, Presentation::Floating);
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar);
    // The panel stands over the editor, its title out to the right of the panels.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(text_center(&shapes, "Local variables").x > 280.0);
    // The arrow in the header expands it.
    click(&mut u, Pos2::new(g.panel.max.x - 52.0, g.panel.min.y + Geom::HEADER_H / 2.0));
    assert_eq!(u.app.ui.variables.as_ref().map(|e| e.presentation), Some(Presentation::Full), "open, in the full view");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let title = text_center(&shapes, "Local variables");
    assert!(title.x < 200.0, "the full view starts where the left panel did: {title:?}");
    // The header's arrow brings the panel back, still open.
    let right = u.ctx.content_rect().max.x - 52.0;
    click(&mut u, Pos2::new(right, title.y));
    assert_eq!(u.app.ui.variables.as_ref().map(|e| e.presentation), Some(Presentation::Floating));
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_floating_panel_moves_by_its_header() {
    let mut u = editor();
    assert!(u.app.ui.variables.as_ref().is_some_and(|e| e.pos.is_none()), "centred until moved");
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let from = g.panel.min + vec2(120.0, Geom::HEADER_H / 2.0);
    u.drag_to(from, from + vec2(90.0, 60.0));
    u.button(from + vec2(90.0, 60.0), false);
    u.frame(vec![]);
    let p = u.app.ui.variables.as_ref().and_then(|e| e.pos).expect("moved");
    assert!(p.x > g.panel.min.x + 60.0 && p.y > g.panel.min.y + 40.0, "{p:?}");
    // Where it landed is where the next frame draws it.
    let g = Geom::floating(u.ctx.content_rect(), Some(p), true);
    assert_eq!(g.panel.min, p);
}

/// The editor open on a collection of three variables: space/sm, space/md and brand, in that order.
fn with_variables() -> Ui {
    let mut u = Ui::new(Session::new());
    for (cmd, p) in [
        ("variable.createCollection", json!({"name": "Tokens", "modes": ["Light"]})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/sm", "type": "number", "value": 8})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/md", "type": "number", "value": 16})),
        ("variable.create", json!({"collection": "Tokens", "name": "brand", "type": "color", "value": "#0D99FF"})),
    ] {
        u.app.session.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e:?}"));
    }
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u
}

/// A drag from `from` to `to`: the pointer rests on the widget first (a press belongs to the
/// widget under the pointer, and that takes a frame to know).
fn drag_from(u: &mut Ui, from: Pos2, to: Pos2) {
    for _ in 0..3 {
        u.frame(vec![egui::Event::PointerMoved(from)]);
    }
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
}

/// The variables as the table shows them: grouped by folder (a folder's name sorts it), in
/// document order inside a folder — what a drop is judged by.
fn shown(u: &Ui) -> Vec<String> {
    let mut names: Vec<String> = u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.name.clone()).collect();
    names.sort_by_key(|n: &String| n.rsplit_once('/').map(|(g, _)| g).unwrap_or("").to_string());
    names
}

/// The collection's variables in document order.
fn names(u: &Ui) -> Vec<String> {
    u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.name.clone()).collect()
}

/// The editor open on two folders: color/brand, color/surface, space/sm and space/md.
fn with_two_folders() -> Ui {
    let mut u = Ui::new(Session::new());
    for (cmd, p) in [
        ("variable.createCollection", json!({"name": "Tokens", "modes": ["Light"]})),
        ("variable.create", json!({"collection": "Tokens", "name": "color/brand", "type": "color", "value": "#0D99FF"})),
        ("variable.create", json!({"collection": "Tokens", "name": "color/surface", "type": "color", "value": "#FFFFFF"})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/sm", "type": "number", "value": 8})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/md", "type": "number", "value": 16})),
    ] {
        u.app.session.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e:?}"));
    }
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u
}

/// The editor open on three folders: radius (no folder), color/brand, color/surface, space/sm and
/// space/md. The table shows them grouped: radius, then color, then space.
fn with_groups() -> Ui {
    let mut u = Ui::new(Session::new());
    for (cmd, p) in [
        ("variable.createCollection", json!({"name": "Tokens", "modes": ["Light"]})),
        ("variable.create", json!({"collection": "Tokens", "name": "color/brand", "type": "color", "value": "#0D99FF"})),
        ("variable.create", json!({"collection": "Tokens", "name": "color/surface", "type": "color", "value": "#FFFFFF"})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/sm", "type": "number", "value": 8})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/md", "type": "number", "value": 16})),
        ("variable.create", json!({"collection": "Tokens", "name": "radius", "type": "number", "value": 4})),
    ] {
        u.app.session.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e:?}"));
    }
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    u
}

/// Where a row shows on screen: its leaf name's centre, pulled left into the name column.
fn row(shapes: &[Shape], leaf: &str) -> Pos2 {
    text_center(shapes, leaf) - vec2(14.0, 0.0)
}

/// The centre of the text `s` whose left edge is left of `x`.
fn text_left_of(shapes: &[Shape], s: &str, x: f32) -> Pos2 {
    let (_, p, size) =
        crate::live_tests::texts(shapes).into_iter().find(|(t, p, _)| t == s && p.x < x).unwrap_or_else(|| panic!("no text {s:?} left of {x}"));
    p + size / 2.0
}

#[test]
fn dragging_a_row_by_its_name_moves_it_to_the_gap_it_is_dropped_in() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // From "brand" to the top half of "space/sm" (its row shows the leaf name).
    let from = text_left_of(&shapes, "brand", 900.0) - vec2(14.0, 0.0);
    let to = text_left_of(&shapes, "sm", 900.0) - vec2(14.0, 6.0);
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    // It lands above space/sm and takes its folder: the table groups rows by name.
    assert_eq!(names(&u), ["space/brand", "space/sm", "space/md"]);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn dragging_a_row_onto_the_sidebar_group_moves_it_into_that_group() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // The sidebar's "space" row (the table's group header is right of the sidebar's edge).
    let group = text_left_of(&shapes, "space", 500.0);
    let from = text_left_of(&shapes, "brand", 900.0) - vec2(14.0, 0.0);
    u.drag_to(from, group);
    u.button(group, false);
    u.frame(vec![]);
    assert_eq!(names(&u), ["space/sm", "space/md", "space/brand"]);
}

#[test]
fn dropping_a_row_back_where_it_was_is_no_move_at_all() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let ids = |u: &Ui| -> Vec<String> { u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.id.clone()).collect() };
    let before = ids(&u);
    let from = text_left_of(&shapes, "md", 900.0) - vec2(14.0, 0.0);
    // The gap right under space/md is the one it already fills.
    u.drag_to(from, from + vec2(0.0, 8.0));
    u.button(from + vec2(0.0, 8.0), false);
    u.frame(vec![]);
    assert_eq!(ids(&u), before, "the same variables, in the same order, untouched");
    assert_eq!(names(&u), ["space/sm", "space/md", "brand"]);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_panel_follows_the_pointer_without_a_frame_of_lag() {
    let mut u = editor();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let from = g.panel.min + vec2(80.0, Geom::HEADER_H / 2.0);
    let to = from + vec2(70.0, 50.0);
    u.drag_to(from, to);
    // The frame that moved the pointer last already has the panel under it: a delta applied after
    // the fact would leave it a step (here 7 pt and 5 pt) behind.
    let p = u.app.ui.variables.as_ref().and_then(|e| e.pos).expect("moved");
    assert_eq!(p, g.panel.min + vec2(70.0, 50.0));
    u.button(to, false);
}

#[test]
fn dropping_below_a_folders_last_row_lands_at_the_end_of_that_folder() {
    let mut u = with_two_folders();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // "md" (space's last row) dropped under "surface" (color's last row): the gap is color's end,
    // though a group header for space follows it in the table.
    let from = text_left_of(&shapes, "md", 900.0) - vec2(14.0, 0.0);
    let to = text_left_of(&shapes, "surface", 900.0) - vec2(14.0, -8.0);
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    assert_eq!(names(&u), ["color/brand", "color/surface", "space/sm", "color/md"], "it joins the folder whose end it landed on");
}

#[test]
fn dragging_the_last_row_of_a_folder_onto_its_own_end_does_nothing() {
    let mut u = with_two_folders();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let ids = |u: &Ui| -> Vec<String> { u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.id.clone()).collect() };
    let before = ids(&u);
    // The gap under space's last row is where it already is.
    let from = text_left_of(&shapes, "md", 900.0) - vec2(14.0, 0.0);
    u.drag_to(from, from + vec2(0.0, 8.0));
    u.button(from + vec2(0.0, 8.0), false);
    u.frame(vec![]);
    assert_eq!(ids(&u), before);
    assert_eq!(names(&u), ["color/brand", "color/surface", "space/sm", "space/md"]);
}

#[test]
fn folding_the_sidebar_gives_its_column_to_the_table() {
    let mut u = with_variables();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    assert_eq!(g.table_width(), g.panel.width() - 265.0);
    // The button at the end of the sidebar's half folds it away.
    click(&mut u, Pos2::new(g.panel.min.x + 265.0 - 24.0, g.panel.min.y + Geom::HEADER_H / 2.0));
    let st = u.app.ui.variables.clone().expect("open");
    assert!(!st.sidebar, "folded");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // The table now starts where the sidebar did, and its heading is gone.
    let g = Geom::floating(u.ctx.content_rect(), st.pos, false);
    assert_eq!(g.table_width(), g.panel.width(), "the table takes the whole panel");
    let heading = crate::i18n::tr("Collections");
    assert!(crate::live_tests::texts(&shapes).iter().all(|(t, _, _)| *t != heading), "no sidebar left");
    let name = text_center(&shapes, crate::i18n::tr("Name"));
    assert!(name.x < g.panel.min.x + 60.0, "the table starts at the panel's edge: {name:?}");
    // The button at the header's left end brings it back.
    click(&mut u, Pos2::new(g.panel.min.x + 30.0, g.panel.min.y + Geom::HEADER_H / 2.0));
    assert!(u.app.ui.variables.as_ref().is_some_and(|e| e.sidebar), "shown again");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_sidebars_edge_drags_its_width() {
    let mut u = with_variables();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let edge = Pos2::new(g.panel.min.x + 264.0, g.panel.min.y + 120.0);
    drag_from(&mut u, edge, edge + vec2(40.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.sidebar_w > 264.0, "wider: {}", st.sidebar_w);
    // The table's own edge follows it.
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w);
    assert!(g.table_width() < g.panel.width() - 264.0, "the table gives the sidebar its column");
}

#[test]
fn the_columns_edges_drag_their_widths() {
    let mut u = with_variables();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    // The name column's edge, between the names and the modes.
    let edge = Pos2::new(table_left + 240.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    drag_from(&mut u, edge, edge + vec2(30.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.name_w > 240.0, "the name column is wider: {}", st.name_w);
    // A mode column's edge: every mode column is that wide.
    let st = u.app.ui.variables.as_ref().expect("open");
    let edge = Pos2::new(table_left + st.name_w + 1.0 + 200.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    drag_from(&mut u, edge, edge + vec2(-40.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.mode_w < 200.0, "the mode columns are narrower: {}", st.mode_w);
}

#[test]
fn only_the_mode_columns_scroll_sideways() {
    let mut u = with_variables();
    // Two modes that are too wide for the table: the mode columns overflow.
    u.app.ui.variables.as_mut().expect("open").mode_w = 400.0;
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let name_before = text_center(&shapes, "sm");
    let mode_before = text_center(&shapes, "Light");
    // A sideways scroll over the mode columns.
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let over = Pos2::new(g.panel.min.x + 265.0 + 240.0 + 30.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    u.frame(vec![egui::Event::PointerMoved(over)]);
    u.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(60.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert_eq!(text_center(&shapes, "sm"), name_before, "the names stay put");
    assert!(text_center(&shapes, "Light").x < mode_before.x, "the modes moved left");
}

#[test]
fn double_clicking_a_variables_name_puts_it_in_edit() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let name = text_left_of(&shapes, "sm", 900.0) - vec2(14.0, 0.0);
    for _ in 0..3 {
        u.frame(vec![egui::Event::PointerMoved(name)]);
    }
    for _ in 0..2 {
        u.button(name, true);
        u.button(name, false);
    }
    u.frame(vec![]);
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(matches!(&st.rename, Some(Rename::Variable(_))), "the name is being edited: {:?}", st.rename);
}

#[test]
fn reordering_a_row_to_the_end_of_the_list_works() {
    let mut u = with_two_folders();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // space/sm (the group's first) dropped under space/md (its last): it used to be refused.
    let from = text_left_of(&shapes, "sm", 900.0) - vec2(14.0, 0.0);
    let to = text_left_of(&shapes, "md", 900.0) - vec2(14.0, -8.0);
    drag_from(&mut u, from, to);
    assert_eq!(names(&u), ["color/brand", "color/surface", "space/md", "space/sm"], "it lands at the end");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_panels_edges_drag_its_size() {
    let mut u = editor();
    // The panel as it is drawn now (it grows and stays centred, so its edges move).
    let now = |u: &Ui| {
        let st = u.app.ui.variables.as_ref().expect("open");
        Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size)
    };
    // The corner grows both ways (the pointer takes hold 8 pt in from it).
    let g = now(&u);
    let from = Pos2::new(g.panel.max.x - 8.0, g.panel.max.y - 8.0);
    drag_from(&mut u, from, from + vec2(60.0, 40.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    let size = st.size.expect("resized");
    assert!((size.x - (Geom::FLOAT_W + 52.0)).abs() < 1.0, "{size:?}");
    assert!((size.y - (Geom::FLOAT_H + 32.0)).abs() < 1.0, "{size:?}");
    // The right edge alone: the width, the height stays.
    let g = now(&u);
    let from = Pos2::new(g.panel.max.x, g.panel.min.y + 120.0);
    drag_from(&mut u, from, from + vec2(-100.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    let size = st.size.expect("resized");
    assert!((size.x - (g.panel.width() - 100.0)).abs() < 1.0, "{size:?}");
    assert!((size.y - g.panel.height()).abs() < 1.0, "the height is untouched: {size:?}");
    // The bottom edge alone.
    let g = now(&u);
    let from = Pos2::new(g.panel.min.x + 200.0, g.panel.max.y);
    drag_from(&mut u, from, from + vec2(0.0, 50.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    let size = st.size.expect("resized");
    assert!((size.x - g.panel.width()).abs() < 1.0, "the width is untouched: {size:?}");
    assert!((size.y - (g.panel.height() + 50.0)).abs() < 1.0, "{size:?}");
    // Never below the smallest usable panel.
    let g = now(&u);
    let from = Pos2::new(g.panel.max.x, g.panel.min.y + 120.0);
    drag_from(&mut u, from, from + vec2(-5000.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.size.expect("resized").x >= Geom::MIN_W, "{:?}", st.size);
}

#[test]
fn dragging_between_two_folders_lands_in_the_folder_of_the_row_under_the_pointer() {
    // color/surface dropped under space/md (space's last row): it joins space, at its end.
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let from = row(&shapes, "surface");
    let to = row(&shapes, "md") + vec2(0.0, 8.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["radius", "color/brand", "space/sm", "space/md", "space/surface"], "it joins space, not color");

    // space/sm dropped above color/brand (color's first row): it joins color, at its start.
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let from = row(&shapes, "sm");
    let to = row(&shapes, "brand") - vec2(0.0, 8.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["radius", "color/sm", "color/brand", "color/surface", "space/md"], "it lands above brand, inside color");
}

#[test]
fn dragging_just_past_the_ends_of_the_list_lands_on_the_end() {
    // Below the last row, in the empty strip under it: the end of that row's folder.
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let from = row(&shapes, "brand");
    let to = row(&shapes, "md") + vec2(0.0, 26.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["radius", "color/surface", "space/sm", "space/md", "space/brand"], "brand joins space at its end");

    // Above the first row, in the strip above it: the front of the list, out of its folder.
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let from = row(&shapes, "md");
    let to = row(&shapes, "radius") - vec2(0.0, 26.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["md", "radius", "color/brand", "color/surface", "space/sm"], "md moves to the front, out of space");
}

#[test]
fn dragging_within_a_folder_moves_one_place() {
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // space/md above space/sm: one place up.
    let from = row(&shapes, "md");
    let to = row(&shapes, "sm") - vec2(0.0, 8.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["radius", "color/brand", "color/surface", "space/md", "space/sm"]);
    // And back down again: under space/sm is where it started.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let from = row(&shapes, "md");
    let to = row(&shapes, "sm") + vec2(0.0, 8.0);
    drag_from(&mut u, from, to);
    assert_eq!(shown(&u), ["radius", "color/brand", "color/surface", "space/sm", "space/md"]);
}
