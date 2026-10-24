//! The Variables editor in its two presentations: the floating panel it opens as (moved by its
//! header), and the full view the header's arrow expands it into, which takes the editor panels'
//! place. One body of drawing code serves both; `Geom` is the difference.

use egui::{Pos2, Rect, Shape, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center, texts};
use crate::variables_panel::{Columns, Geom, Presentation, Rename};

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

/// The editor open on a collection of two modes, Light then Dark, so a column can be told from
/// its neighbour.
fn with_two_modes() -> Ui {
    let mut u = Ui::new(Session::new());
    for (cmd, p) in [
        ("variable.createCollection", json!({"name": "Tokens", "modes": ["Light", "Dark"]})),
        ("variable.create", json!({"collection": "Tokens", "name": "space/sm", "type": "number", "value": 8})),
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

/// The selected collection's modes, in the order the columns show them.
fn mode_names(u: &Ui) -> Vec<String> {
    u.app
        .session
        .doc()
        .ok()
        .and_then(|d| d.variables.collections.first())
        .map(|c| c.modes.iter().map(|m| m.name.clone()).collect())
        .unwrap_or_default()
}

/// Their ids, in the same order.
fn mode_ids(u: &Ui) -> Vec<String> {
    u.app.session.doc().ok().and_then(|d| d.variables.collections.first()).map(|c| c.modes.iter().map(|m| m.id.clone()).collect()).unwrap_or_default()
}

/// Every text with the rect it was clipped to: a text clipped to a rect outside the rows' viewport
/// can still be seen, drawn over the header.
fn text_clips(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Pos2, Rect)> {
    fn walk(s: &Shape, clip: Rect, out: &mut Vec<(String, Pos2, Rect)>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos, clip)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, c.clip_rect, &mut out);
    }
    out
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
    // A mode column's edge narrows that column, not every one of them.
    let st = u.app.ui.variables.as_ref().expect("open");
    let id = mode_ids(&u).remove(0);
    let edge = Pos2::new(table_left + st.name_w + 1.0 + 200.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    drag_from(&mut u, edge, edge + vec2(-40.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.mode_width(&id) < 200.0, "the mode column is narrower: {}", st.mode_width(&id));
}

/// A double click at `p`: two presses and releases inside egui's double-click window.
fn double_click(u: &mut Ui, p: Pos2) {
    u.frame(vec![egui::Event::PointerMoved(p)]);
    click(u, p);
    click(u, p);
}

#[test]
fn double_clicking_a_mode_columns_edge_fits_it_to_its_values() {
    let mut u = with_variables();
    // The colour variable is the widest thing the column holds; without it only two numbers (8 and
    // 16) and the heading are left, far narrower than 200.
    u.app.session.execute("variable.delete", &json!({"variable": "brand"})).unwrap();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    u.frame(vec![]);
    let id = mode_ids(&u).remove(0);
    let st = u.app.ui.variables.as_ref().expect("open");
    assert_eq!(st.mode_width(&id), 200.0, "a column starts at the default width");
    // A double-click on its divider fits it down to the narrowest a column goes (MODE_MIN).
    let edge = Pos2::new(table_left + st.name_w + 1.0 + 200.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    double_click(&mut u, edge);
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!((st.mode_width(&id) - 100.0).abs() < 0.5, "the column fitted its values: {}", st.mode_width(&id));
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn double_clicking_a_mode_columns_edge_widens_it_to_a_long_value() {
    let mut u = with_variables();
    let long = "A value far wider than the column it starts in";
    u.app.session.execute("variable.create", &json!({"collection": "Tokens", "name": "note", "type": "string", "value": long})).unwrap();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    u.frame(vec![]);
    let id = mode_ids(&u).remove(0);
    let edge = Pos2::new(table_left + 240.0 + 1.0 + 200.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    double_click(&mut u, edge);
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.mode_width(&id) > 200.0, "the column grew to hold the long value: {}", st.mode_width(&id));
    assert!(st.mode_width(&id) < 420.0, "and stays within MODE_MAX: {}", st.mode_width(&id));
}

#[test]
fn dragging_one_mode_columns_edge_leaves_the_others_alone() {
    let mut u = with_two_modes();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    let st = u.app.ui.variables.as_ref().expect("open");
    let ids = mode_ids(&u);
    assert_eq!(ids.len(), 2, "two columns to tell apart");
    let edge = Pos2::new(table_left + st.name_w + 1.0 + 200.0, g.panel.min.y + Geom::HEADER_H + 16.0);
    drag_from(&mut u, edge, edge + vec2(-50.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.mode_width(&ids[0]) < 200.0, "the dragged column narrowed: {}", st.mode_width(&ids[0]));
    // They used to share one width, so narrowing one squeezed the other with it.
    assert_eq!(st.mode_width(&ids[1]), 200.0, "the column beside it kept its width");
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

#[test]
fn a_colour_variables_alpha_is_a_field_of_its_own() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "bg", "type": "color", "value": "#02DA96"})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    // The percentage at the right end of the value takes typing like the hex does.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, alpha_of_row(&shapes, "02DA96"));
    u.frame(vec![egui::Event::Text("40".into())]);
    for pressed in [true, false] {
        u.frame(vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }]);
    }
    u.frame(vec![]);
    assert_eq!(colour_of(&u, "bg").to_css_hex(), "#02DA9666", "40% of the same green");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

/// The middle of the alpha field that sits beside the hex `hex` on screen (the toolbar has
/// percentages of its own: only the one on that row is the variable's).
fn alpha_of_row(shapes: &[Shape], hex: &str) -> Pos2 {
    let row = text_center(shapes, hex).y;
    texts(shapes)
        .into_iter()
        .map(|(t, p, size)| (t, p + size / 2.0))
        .filter(|(t, p)| t.ends_with('%') && (p.y - row).abs() < 6.0)
        .map(|(_, p)| p)
        .next()
        .unwrap_or_else(|| panic!("no alpha beside {hex:?}"))
}

/// The colour variable named `name`.
fn colour_of(u: &Ui, name: &str) -> figcraft_color::Color {
    let doc = u.app.session.doc().unwrap();
    let v = doc.variables.variables.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("no variable {name:?}"));
    match v.values.values().next() {
        Some(figcraft_doc::VariableValue::Color(c)) => *c,
        other => panic!("{name:?} is not a colour: {other:?}"),
    }
}

/// The bottom of the panel's own backdrop. The floating panel's window frame is the rect that starts
/// at the panel's top-left and spans its width; it wraps the editor's content, so when the content
/// outgrows the panel the frame grows with it — its bottom then sits past `g.panel`'s. (Looking for
/// "the lowest shape inside the panel's columns" does not work: the canvas behind the panel keeps
/// drawing its own vertical rules and its bottom toolbar in that same band.)
fn panel_frame_bottom(shapes: &[Shape], panel: Rect) -> Option<f32> {
    fn walk(s: &Shape, panel: Rect, out: &mut Option<f32>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, panel, out)),
            Shape::Rect(rs) => {
                let same_origin = (rs.rect.min.x - panel.min.x).abs() < 2.0 && (rs.rect.min.y - panel.min.y).abs() < 2.0;
                let same_width = (rs.rect.width() - panel.width()).abs() < 4.0;
                if same_origin && same_width {
                    *out = Some(out.map_or(rs.rect.max.y, |y: f32| y.max(rs.rect.max.y)));
                }
            }
            _ => {}
        }
    }
    let mut out = None;
    for s in shapes {
        walk(s, panel, &mut out);
    }
    out
}

#[test]
fn the_panel_does_not_grow_when_mode_columns_overflow() {
    // Two wide modes force the horizontal scrollbar. The editor must pay for it out of the rows'
    // area, not by making the panel taller: content taller than the panel grows the window frame
    // past `g.panel`, which leaves the bottom resize handle inside the panel instead of on its edge.
    let mut u = with_two_modes();
    u.app.ui.variables.as_mut().expect("open").mode_w = 400.0;
    u.frame(vec![]);
    let st = u.app.ui.variables.as_ref().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let modes = u.app.session.doc().ok().and_then(|d| d.variables.collections.first().map(|c| c.modes.clone())).unwrap_or_default();
    assert_eq!(modes.len(), 2);
    assert!(Columns::for_modes(g.table_width(), &modes, st).max_sx() > 0.0, "the mode columns overflow");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let bottom = panel_frame_bottom(&shapes, g.panel).expect("the panel's frame");
    // The 1 pt window stroke is painted just outside the frame's rect; a few points of slack, not
    // the 32 pt (header) or 44 pt (header + bar) the panel used to grow by.
    assert!((bottom - g.panel.max.y).abs() <= 3.0, "the panel grew: frame bottom {bottom}, panel bottom {}", g.panel.max.y);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_sidebars_divider_drag_trades_the_two_lists() {
    // The divider between the collections and groups lists drags to give one more of the sidebar's
    // body than the other.
    let mut u = with_two_modes();
    let st = u.app.ui.variables.as_ref().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w);
    let h = g.body_height();
    let av = h - 34.0;
    let split_total = (av - 7.0 - 34.0).max(80.0);
    let collections_h = (st.sidebar_split * split_total).clamp(40.0, (split_total - 40.0).max(40.0));
    // The divider sits a few px into the 7 pt strip right after the collections list.
    let divider_y = g.panel.min.y + Geom::HEADER_H + 34.0 + collections_h + 3.0;
    let x = g.panel.min.x + 100.0;
    // Dragging it down gives the collections list more of the sidebar's body.
    drag_from(&mut u, Pos2::new(x, divider_y), Pos2::new(x, divider_y + 60.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.sidebar_split > 0.5, "the collections list grew: {}", st.sidebar_split);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn dragging_the_sidebar_divider_to_the_top_keeps_the_panels_height() {
    // egui's ScrollArea is at least 64 pt tall whatever it is given. With the divider at the top the
    // collections list is 40, and that floor used to grow the panel by the 24 pt difference — the
    // window got taller the further the divider was dragged up.
    let mut u = with_two_modes();
    let st = u.app.ui.variables.clone().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let h = g.body_height();
    let av = h - 34.0;
    let split_total = (av - 7.0 - 34.0).max(80.0);
    let collections_h = (st.sidebar_split * split_total).clamp(40.0, (split_total - 40.0).max(40.0));
    let divider_y = g.panel.min.y + Geom::HEADER_H + 34.0 + collections_h + 3.0;
    let x = g.panel.min.x + 100.0;
    // Past the panel's own top edge: the drag keeps going where the list cannot.
    drag_from(&mut u, Pos2::new(x, divider_y), Pos2::new(x, g.panel.min.y - 120.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert!(st.sidebar_split < 0.2, "the collections list shrank: {}", st.sidebar_split);
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let bottom = panel_frame_bottom(&shapes, g.panel).expect("the panel's frame");
    assert!((bottom - g.panel.max.y).abs() <= 3.0, "the panel grew: frame bottom {bottom}, panel bottom {}", g.panel.max.y);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

/// The rail's selected marks (the chips filled with the selected-row colour). The rail is the 56 pt
/// column on the left, so nothing from the panels can sneak in.
fn rail_marks(u: &mut Ui) -> Vec<Rect> {
    let selected = crate::theme::Tokens::get(&u.ctx).selected_row;
    flat(&u.frame_output(vec![]).shapes)
        .iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.fill == selected && r.rect.max.x < 56.0 => Some(r.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn the_rail_marks_the_variables_button_while_the_editor_is_open() {
    // The Variables button opens the editor rather than a panel tab, so it had no selected state at
    // all — with the editor up it looked like nothing was open. It shows the mark now, for both
    // presentations, and drops it when the editor closes.
    let mut u = Ui::new(Session::new());
    let idle = rail_marks(&mut u);
    u.app.ui.variables = Some(Default::default());
    for presentation in [Presentation::Floating, Presentation::Full] {
        u.app.ui.variables.as_mut().expect("open").presentation = presentation;
        let open = rail_marks(&mut u);
        let added: Vec<Rect> = open.iter().copied().filter(|r| !idle.contains(r)).collect();
        assert_eq!(added.len(), 1, "the Variables button is the one that gained the mark ({presentation:?}): {open:?} vs {idle:?}");
    }
    // Closing it takes the mark away again: the rail is back to what it looked like before.
    u.app.ui.variables = None;
    assert_eq!(rail_marks(&mut u), idle, "the rail is idle again once the editor closes");
}

/// A vertical line (constant x) in `shapes` whose x is within `tol` of `x` and spans at least `h`.
fn vline_near(shapes: &[Shape], x: f32, tol: f32, h: f32) -> bool {
    shapes.iter().any(|s| match s {
        Shape::LineSegment { points, .. } => {
            let (a, b) = (points[0], points[1]);
            (a.x - b.x).abs() < 0.5 && (a.x - x).abs() <= tol && (b.y - a.y).abs() >= h
        }
        _ => false,
    })
}

/// A 1 px-wide vertical rect in `shapes` at screen x ≈ `x`, whose vertical span meets `[y_min, y_max]`.
fn vrect_at(shapes: &[Shape], x: f32, tol: f32, y_min: f32, y_max: f32) -> bool {
    shapes.iter().any(|s| match s {
        Shape::Rect(rs) => {
            let r = rs.rect;
            (r.width() - 1.0).abs() < 0.5 && (r.min.x - x).abs() <= tol && r.max.y > y_min && r.min.y < y_max
        }
        _ => false,
    })
}

#[test]
fn the_left_and_top_edges_resize_the_panel_without_sliding_away() {
    let mut u = editor();
    let now = |u: &Ui| {
        let st = u.app.ui.variables.as_ref().expect("open");
        Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size)
    };
    // The left edge: dragging it left grows the width and pulls the panel's left edge left with it,
    // instead of leaving the panel where it was and only shrinking the right.
    let g = now(&u);
    let from = Pos2::new(g.panel.min.x, g.panel.min.y + 120.0);
    drag_from(&mut u, from, from + vec2(-60.0, 0.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    let size = st.size.expect("resized");
    let pos = st.pos.expect("moved");
    assert!((size.x - (g.panel.width() + 60.0)).abs() < 1.0, "{size:?}");
    assert!((pos.x - (g.panel.min.x - 60.0)).abs() < 1.0, "{pos:?}");
    assert!((pos.y - g.panel.min.y).abs() < 1.0, "the top stays put: {pos:?}");
    // The top edge: dragging it up grows the height and pulls the panel's top edge up with it.
    let g = now(&u);
    let from = Pos2::new(g.panel.min.x + 200.0, g.panel.min.y - 2.0);
    drag_from(&mut u, from, from + vec2(0.0, -50.0));
    let st = u.app.ui.variables.as_ref().expect("open");
    let size = st.size.expect("resized");
    let pos = st.pos.expect("moved");
    assert!((size.y - (g.panel.height() + 52.0)).abs() < 1.0, "{size:?}");
    assert!((pos.y - (g.panel.min.y - 52.0)).abs() < 1.0, "{pos:?}");
    assert!((pos.x - g.panel.min.x).abs() < 1.0, "the left stays put: {pos:?}");
    // The top edge stays on screen and never below the smallest height.
    assert!(pos.y >= 0.0, "{pos:?}");
    assert!(size.y >= Geom::MIN_H, "{size:?}");
}

#[test]
fn the_columns_fit_their_width_even_when_the_panel_is_narrow() {
    let st = Default::default();
    let r = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 36.0));
    // A roomy table: the name and fixed columns keep the room left for the mode columns.
    let cols = Columns::of(1000.0, 3, &st);
    assert!(cols.modes_rect(r).max.x <= cols.add_rect(r).min.x, "modes clear the add column");
    assert!(cols.add_rect(r).max.x <= r.max.x, "the add column stays inside the row");
    // A panel too narrow for the minimums: the columns still add up to no more than the width, so
    // nothing is drawn past the table's right edge (a vertical scroll area clips only its axis).
    let cols = Columns::of(150.0, 5, &st);
    let name = cols.name_rect(r);
    let modes = cols.modes_rect(r);
    let add = cols.add_rect(r);
    assert!(name.min.x >= r.min.x, "name starts at the row");
    assert!(modes.min.x >= name.max.x, "modes follow the name");
    assert!(add.min.x >= modes.max.x, "the add column follows the modes");
    assert!(add.max.x <= r.max.x, "nothing runs past the row's right edge");
}

#[test]
fn the_sidebars_divider_runs_straight_through_the_header() {
    let mut u = editor();
    let st = u.app.ui.variables.as_ref().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w);
    let x = g.panel.min.x + g.sidebar_width() - 1.0;
    let shapes = flat(&u.frame_output(vec![]).shapes);
    // The sidebar's 1 px line is a rect in both the header and the body, so the two line up exactly
    // (a centred stroke in the header would sit half a pixel to the left of the body's rect).
    assert!(vrect_at(&shapes, x, 1.0, g.panel.min.y, g.panel.min.y + Geom::HEADER_H), "the line in the header at {x}");
    assert!(vrect_at(&shapes, x, 1.0, g.panel.min.y + Geom::HEADER_H, g.panel.min.y + Geom::HEADER_H + 200.0), "the line in the body");
}

#[test]
fn the_modes_have_a_divider_between_them() {
    let mut u = with_variables();
    let st = u.app.ui.variables.as_ref().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let table_left = g.panel.min.x + g.sidebar_width();
    let table_w = g.table_width();
    let cols = Columns::of(table_w, 1, st);
    let r = Rect::from_min_size(Pos2::ZERO, vec2(table_w, 36.0));
    // The divider to the right of the (single) mode column: the header and rows draw a line there,
    // so the columns read as a grid. Before the fix this line was never drawn.
    let want = table_left + cols.mode_rect(r, 0).max.x;
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(vline_near(&shapes, want, 2.0, 20.0), "a vertical divider at {want} between the mode and the add column");
}

#[test]
fn dragging_a_mode_heading_moves_that_column() {
    let mut u = with_two_modes();
    // Narrow enough that both columns sit inside the strip.
    u.app.ui.variables.as_mut().expect("open").mode_w = 150.0;
    u.frame(vec![]);
    let st = u.app.ui.variables.as_ref().expect("open");
    let g = Geom::floating(u.ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let table_left = g.panel.min.x + g.sidebar_width();
    let table_w = g.table_width();
    let cols = Columns::of(table_w, 2, st);
    let head = Rect::from_min_size(Pos2::new(table_left, g.panel.min.y + Geom::HEADER_H), vec2(table_w, 40.0));
    assert_eq!(mode_names(&u), ["Light", "Dark"]);
    // Carry "Dark" (the second column) to the left of "Light" by its heading.
    let from = cols.mode_rect(head, 1).center();
    let to = cols.mode_rect(head, 0).left_center() + vec2(10.0, 0.0);
    drag_from(&mut u, from, to);
    assert_eq!(mode_names(&u), ["Dark", "Light"], "the columns changed places");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn a_row_scrolled_out_of_view_is_clipped_away() {
    let mut u = Ui::new(Session::new());
    u.app.session.execute("variable.createCollection", &json!({"name": "Tokens", "modes": ["Light", "Dark"]})).unwrap();
    for i in 0..40 {
        u.app.session.execute("variable.create", &json!({"collection": "Tokens", "name": format!("v{i}"), "type": "number", "value": i})).unwrap();
    }
    u.app.ui.variables = Some(Default::default());
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    let head_bottom = g.panel.min.y + Geom::HEADER_H + 40.0;
    // Scroll the rows to the bottom.
    let over = Pos2::new(table_left + 300.0, g.panel.min.y + Geom::HEADER_H + 120.0);
    u.frame(vec![egui::Event::PointerMoved(over)]);
    for _ in 0..6 {
        u.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    u.frame(vec![]);
    // A row's cells are clipped to the rows' viewport *and* to their own strip. The strip alone was
    // the clip before, so a row scrolled out of view kept painting — over the header and past the
    // panel's edges. Every value cell now stops at the viewport's top.
    let mut seen = 0;
    for (text, at, clip) in text_clips(&u.frame_output(vec![]).shapes) {
        if at.x > table_left && text.parse::<u32>().is_ok() {
            seen += 1;
            assert!(clip.min.y >= head_bottom - 1.0, "the value {text} at {at:?} can be seen above the header (clip {clip:?})");
        }
    }
    assert!(seen > 0, "the visible rows still draw their values");
}

/// A variable's id by name.
fn var_id(u: &Ui, name: &str) -> String {
    u.app
        .session
        .doc()
        .unwrap()
        .variables
        .variables
        .iter()
        .find(|v| v.name == name)
        .map(|v| v.id.clone())
        .unwrap_or_else(|| panic!("no variable {name:?}"))
}

/// The variables in document order.
fn order(u: &Ui) -> Vec<String> {
    u.app.session.doc().unwrap().variables.variables.iter().map(|v| v.name.clone()).collect()
}

/// Whether a rect filled with `fill` covers `p` (a row's own background).
fn filled_at(shapes: &[Shape], p: Pos2, fill: egui::Color32) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == fill && r.rect.contains(p)))
}

#[test]
fn a_clicked_row_is_marked_and_a_copy_of_it_pastes_under_it() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let row = text_center(&shapes, "sm");
    click(&mut u, row);
    let sm = var_id(&u, "space/sm");
    let st = u.app.ui.variables.as_ref().expect("open");
    assert_eq!(st.selected.as_deref(), Some(sm.as_str()), "the clicked row is the selected one");
    // The selection shows: the row is painted in the colour the layers panel marks a selected row with.
    let fill = crate::theme::Tokens::get(&u.ctx).selected_row;
    // The panel fades in as it opens (egui Areas do), so let it settle before reading its colours.
    for _ in 0..12 {
        u.frame(vec![]);
    }
    assert!(filled_at(&flat(&u.frame_output(vec![]).shapes), row, fill), "the selected row is marked");
    // Cmd/Ctrl+C puts the row on the editor's clipboard...
    u.frame(vec![egui::Event::Copy]);
    assert!(u.app.ui.variables.as_ref().expect("open").copied.is_some(), "the row is copied");
    // ...and Cmd/Ctrl+V makes a copy of it right under the row it came from.
    u.frame(vec![egui::Event::Paste(String::new())]);
    let names = order(&u);
    let at = names.iter().position(|n| n == "space/sm copy").unwrap_or_else(|| panic!("no copy: {names:?}"));
    assert_eq!(names[at - 1], "space/sm", "right under the row it was copied from: {names:?}");
    let st = u.app.ui.variables.as_ref().expect("open");
    assert_eq!(st.selected.as_deref(), Some(var_id(&u, "space/sm copy").as_str()), "the pasted row is selected");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn a_paste_with_no_row_selected_lands_at_the_end_of_the_list() {
    let mut u = with_variables();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "sm"));
    u.frame(vec![egui::Event::Copy]);
    // Escape lets the row go: the paste then goes to bottom of the whole list, not under a row.
    u.frame(vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    assert!(u.app.ui.variables.as_ref().expect("open").selected.is_none(), "nothing is selected");
    u.frame(vec![egui::Event::Paste(String::new())]);
    let names = order(&u);
    assert_eq!(names.last().map(String::as_str), Some("space/sm copy"), "at the bottom: {names:?}");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn each_rows_button_at_the_tables_right_end_opens_its_details() {
    let mut u = with_variables();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let row = text_center(&shapes, "sm");
    // Under the add-mode column's heading: the row's own edit button, at its far right.
    click(&mut u, Pos2::new(g.panel.max.x - 20.0, row.y));
    let st = u.app.ui.variables.as_ref().expect("open");
    assert_eq!(st.details.as_deref(), Some(var_id(&u, "space/sm").as_str()), "the row's button opens its details");
    assert_eq!(st.selected.as_deref(), Some(var_id(&u, "space/sm").as_str()), "and marks the row");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn a_colour_cells_swatch_opens_the_picker_and_a_second_click_closes_it() {
    let mut u = with_variables();
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    // "brand" has no folder, so it is the table's first row; one mode, so one value column.
    let st = u.app.ui.variables.as_ref().expect("open");
    let cols = Columns::of(g.table_width(), 1, st);
    let table = Pos2::new(g.panel.min.x + 265.0, g.panel.min.y + Geom::HEADER_H + 40.0);
    let cell = cols.mode_rect(Rect::from_min_size(table, vec2(g.table_width(), 40.0)), 0);
    // The swatch at the cell's left edge, as a fill's row has it.
    let swatch = Pos2::new(cell.min.x + 16.0, cell.center().y);
    click(&mut u, swatch);
    let want = crate::colorpicker::PickerTarget::Variable { id: var_id(&u, "brand"), mode: mode_ids(&u).remove(0) };
    assert_eq!(u.app.ui.picker, Some(want), "the swatch opens the colour picker for that value");
    // A second click on it closes the picker again (the picker holds the press that opened it).
    click(&mut u, swatch);
    assert_eq!(u.app.ui.picker, None, "a second click closes it");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

/// The centre y of text `s` drawn in the table (x at or right of the sidebar's divider), so the
/// sidebar's own group labels don't get mistaken for the table's. Row names are centred in the row,
/// so a row's text centre is the row's centre exactly; a group header's text is drawn 2px below the
/// header's centre, so its text centre is `header_centre + 2`.
fn table_text_y(shapes: &[Shape], s: &str, table_left: f32) -> f32 {
    crate::live_tests::texts(shapes)
        .into_iter()
        .find(|(t, p, _)| t == s && p.x >= table_left)
        .unwrap_or_else(|| panic!("no table text {s:?} at x≥{table_left}"))
        .1
        .y
}

#[test]
fn the_group_rows_and_headers_keep_the_forty_pixel_rhythm() {
    let mut u = with_groups();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let g = Geom::floating(u.ctx.content_rect(), None, true);
    let table_left = g.panel.min.x + 265.0;
    let cy = |s: &str| table_text_y(&shapes, s, table_left);
    // Rows in the same group are ROW_H = 40 apart: their names are centred, so text centres are
    // exact row centres.
    assert!((cy("surface") - cy("brand") - 40.0).abs() <= 0.5, "ROW_H = {}", cy("surface") - cy("brand"));
    // A group header's first member starts right under it (no gap), so the header's centre is 40px
    // above its first row's centre — GROUP_H. The header's text sits 2px low, so the text-centre
    // difference is 40 − 2 = 38.
    assert!((cy("brand") - cy("color") - 38.0).abs() <= 0.5, "GROUP_H = {}", cy("brand") - cy("color"));
    // A group header's centre is ROW_H/2 + GROUP_GAP + GROUP_H/2 below the centre of the row above
    // it; with the header's 2px text offset that is 20 + 16 + 20 + 2 = 58. This pins GROUP_GAP = 16
    // once ROW_H and GROUP_H are fixed above.
    assert!((cy("space") - cy("surface") - 58.0).abs() <= 0.5, "GROUP_GAP = {}", cy("space") - cy("surface"));
}
