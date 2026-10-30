//! Boards in the UI, drawn headless: the board toolbar (its flyouts pick a tool with a colour, a
//! shape or a stamp; the Design panel's button), the right panel hidden until asked for, the bar
//! over the selection (colours, shapes, text sizes, connector style, also with layers that aren't
//! board objects selected, never on a file you can only view), the home screen's New board card,
//! and the marks design files and boards get.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::{BoardItem, BoardShape, FileKind, NodeId, Routing, Stamp, StrokeCap, board_color};
use figcraft_engine::Session;
use figcraft_engine::tools::ToolId;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The app on a new board.
fn board() -> Ui {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u
}

/// The board toolbar pill on screen.
fn pill(u: &Ui) -> Rect {
    crate::toolbar::pill_rect(u.app.canvas.rect, crate::toolbar::bar_width(&u.app))
}

/// The left edge of board toolbar group `i` (button 32 wide, a 16-wide caret 2 after it on the
/// groups with flyouts, 8 between): move, ink, sticky, shapes, connector, text, section, stamp,
/// comment.
fn group_x(u: &Ui, i: usize) -> f32 {
    const WIDTHS: [f32; 9] = [58.0, 58.0, 58.0, 58.0, 40.0, 40.0, 40.0, 58.0, 40.0];
    pill(u).min.x + 8.0 + WIDTHS.iter().take(i).sum::<f32>()
}

/// Open group `i`'s flyout from its caret and click option `n` in it (a row of 32-wide cells, 4
/// apart, 8 in from the flyout's edge, the flyout's bottom 8 above the pill).
fn pick_option(u: &mut Ui, i: usize, n: usize) {
    let caret = pos2(group_x(u, i) + 42.0, pill(u).center().y);
    click(u, caret);
    let at = pos2(group_x(u, i) + 8.0 + n as f32 * 36.0 + 16.0, pill(u).min.y - 8.0 - 24.0);
    click(u, at);
}

/// Whether the bar over the selection showed in the last frame.
fn bar_shown(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.areas().visible_last_frame(&egui::LayerId::new(egui::Order::Middle, egui::Id::unique("board_bar"))))
}

/// The bar over the selection, when it shows.
fn bar(u: &Ui) -> Option<Rect> {
    bar_shown(&u.ctx).then(|| u.ctx.memory(|m| m.area_rect(egui::Id::unique("board_bar")))).flatten()
}

#[test]
fn the_board_toolbar_picks_tools_with_their_colours_shapes_and_stamps() {
    let mut u = board();
    pick_option(&mut u, 2, 5);
    assert_eq!(u.app.session.tool.tool, ToolId::Sticky);
    assert_eq!(Some(u.app.session.tool.board.sticky), board_color("blue"));
    pick_option(&mut u, 3, 3);
    assert_eq!((u.app.session.tool.tool, u.app.session.tool.board.shape), (ToolId::BoardShape, BoardShape::Diamond));
    pick_option(&mut u, 7, 1);
    assert_eq!((u.app.session.tool.tool, u.app.session.tool.board.stamp), (ToolId::Stamp, Stamp::Heart));
    // The connector's button picks it; the ink group's flyout lists the marker's siblings.
    let connector = pos2(group_x(&u, 4) + 16.0, pill(&u).center().y);
    click(&mut u, connector);
    assert_eq!(u.app.session.tool.tool, ToolId::Connector);
    let ink_caret = pos2(group_x(&u, 1) + 42.0, pill(&u).center().y);
    click(&mut u, ink_caret);
    assert_eq!(u.app.ui.tool_flyout, Some("ink"));
}

#[test]
fn a_boards_design_panel_opens_when_asked_for_and_prototyping_is_not_offered() {
    let mut u = board();
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.frame(vec![]);
    assert_eq!(u.app.ui.right_tab, crate::RightTab::Design, "boards have no Prototype tab");
    let canvas = u.app.canvas.rect;
    assert!(canvas.max.x > 1400.0, "no right panel: the canvas reaches the window's edge ({canvas:?})");
    // The Design panel's button at the pill's right end.
    let p = pill(&u);
    click(&mut u, pos2(p.max.x - 8.0 - 16.0, p.center().y));
    assert!(u.app.ui.board_panel);
    u.frame(vec![]);
    assert!(u.app.canvas.rect.max.x < canvas.max.x - 200.0, "the panel takes its room");
    // On a design file the panel shows whatever was asked for a board.
    u.app.run("file.new", json!({}));
    u.app.run("view.design", json!({}));
    u.frame(vec![]);
    assert!(u.app.ui.board_panel, "view.design on a design file doesn't toggle the board's panel");
}

#[test]
fn the_selection_bar_recolours_reshapes_and_restyles() {
    let mut u = board();
    let shape = id(&u.app.session.execute("board.shape", &json!({"x": 0, "y": 0, "shape": "rectangle", "text": "Hi"})).unwrap());
    let section = id(&u.app.session.execute("node.create", &json!({"type": "section", "x": 0, "y": 400, "width": 200, "height": 200})).unwrap());
    // A section selected with it (no board object) leaves the bar for the shape.
    u.app.session.execute("select.set", &json!({"ids": [shape, section]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let r = bar(&u).expect("the bar shows");
    let cy = r.center().y;
    // Swatch 5 (blue): 8 in, 24 apart, 20 wide.
    click(&mut u, pos2(r.min.x + 8.0 + 5.0 * 24.0 + 10.0, cy));
    let outline = u.app.session.doc().unwrap().node(shape).unwrap().children[0];
    assert_eq!(u.app.session.doc().unwrap().node(outline).unwrap().fills[0].solid_color(), board_color("blue"));
    // Text size L: after the colours (208) and the shape menu (52), the third of the 30-wide cells.
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + 208.0 + 52.0 + 2.0 * 30.0 + 14.0, cy));
    let text = figcraft_engine::board::text_of(u.app.session.doc().unwrap(), shape).unwrap();
    assert_eq!(u.app.session.doc().unwrap().node(text).unwrap().kind.text().unwrap().style.font_size, 24.0);
    // The shape menu lists the outlines.
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + 208.0 + 20.0, cy));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "Ellipse"));
    assert_eq!(u.app.session.doc().unwrap().node(shape).unwrap().board, Some(BoardItem::Shape { shape: BoardShape::Ellipse }));
}

#[test]
fn the_selection_bar_sets_a_connectors_style_and_offers_nothing_for_a_stamp() {
    let mut u = board();
    let a = u.app.session.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
    let b = u.app.session.execute("board.sticky", &json!({"x": 400, "y": 300})).unwrap()["id"].clone();
    let c = id(&u.app.session.execute("board.connector", &json!({"from": a, "to": b, "style": "elbow"})).unwrap());
    u.frame(vec![]);
    u.frame(vec![]);
    let r = bar(&u).expect("the bar shows for a connector");
    let cy = r.center().y;
    // After the colours (208): Straight, Elbow, Curved, the end's arrow, 30 apart.
    click(&mut u, pos2(r.min.x + 208.0 + 14.0, cy));
    let routing = |u: &Ui| match &u.app.session.doc().unwrap().node(c).unwrap().board {
        Some(BoardItem::Connector(k)) => k.routing,
        _ => panic!("not a connector"),
    };
    assert_eq!(routing(&u), Routing::Straight);
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + 208.0 + 60.0 + 14.0, cy));
    assert_eq!(routing(&u), Routing::Curved);
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + 208.0 + 90.0 + 14.0, cy));
    assert_eq!(u.app.session.doc().unwrap().node(c).unwrap().stroke.cap_end, StrokeCap::None);
    // Nothing is offered while a stamp alone is selected: it takes neither colour nor style.
    let st = id(&u.app.session.execute("board.stamp", &json!({"x": 900, "y": 0, "stamp": "star"})).unwrap());
    u.app.session.execute("select.set", &json!({"ids": [st]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(bar(&u).is_none());
}

#[test]
fn a_board_you_only_view_offers_no_bar() {
    let (mut h, _) = crate::live_tests::Harness::new();
    h.app.session.execute("file.newBoard", &json!({})).unwrap();
    let st = id(&h.app.session.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap());
    h.frames(2);
    assert!(bar_shown(&h.ctx), "the bar shows for the selected sticky");
    crate::live_server_tests::joined(&mut h, figcraft_engine::live::Access::View);
    h.app.session.execute("select.set", &json!({"ids": [st]})).unwrap();
    h.frames(2);
    assert!(h.app.session.active_read_only().is_some());
    assert!(!bar_shown(&h.ctx), "view only: nothing to edit");
}

#[test]
fn the_home_screen_makes_boards_and_marks_files_by_kind() {
    let mut u = Ui::new(Session::empty());
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "New board"));
    assert_eq!(u.app.session.doc().unwrap().kind, FileKind::Board);
    assert_eq!(crate::icons::file_mark(FileKind::Board), crate::icons::Icon::Board);
    assert_eq!(crate::icons::file_mark(FileKind::Design), crate::icons::Icon::PenNib);
}

#[test]
fn a_stamp_shows_who_placed_it_on_hover() {
    let mut s = Session::new();
    s.prefs.user_name = "Grace".into();
    s.execute("file.newBoard", &json!({})).unwrap();
    s.execute("board.stamp", &json!({"x": 100, "y": 100, "stamp": "check"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let at = u.app.to_screen(100.0, 100.0);
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![Event::PointerMoved(at + egui::vec2(1.0, 0.0))]);
    let shapes = flat(&u.frame_output(vec![Event::PointerMoved(at + egui::vec2(1.0, 1.0))]).shapes);
    assert!(has_text(&shapes, "Grace"), "the placer's name beside the pointer");
}

#[test]
fn the_next_sticky_from_the_keyboard_goes_on_typing() {
    let mut u = board();
    u.app.run("board.sticky", json!({"x": 0, "y": 0}));
    // Shift+Cmd/Ctrl+Enter runs it from the menu layer, asking to type in the new one.
    crate::menus::invoke(&mut u.app, "board.nextSticky");
    let doc = u.app.session.doc().unwrap();
    let te = u.app.session.ds().unwrap().text_edit.clone().expect("typing in the new sticky");
    let owner = figcraft_engine::board::owner_of_text(doc, te.id).unwrap();
    assert_eq!(doc.node(owner).unwrap().transform.translation().x, 224.0);
}

#[test]
fn the_next_shape_from_the_keyboard_comes_connected_and_goes_on_typing() {
    let mut u = board();
    let a = id(&u.app.run("board.shape", json!({"shape": "diamond", "x": 0, "y": 0, "width": 200, "height": 120})).unwrap());
    // Shift+Cmd/Ctrl+Enter on a shape: the next shape, joined to it (#1597).
    crate::menus::invoke(&mut u.app, "board.nextSticky");
    let doc = u.app.session.doc().unwrap();
    let te = u.app.session.ds().unwrap().text_edit.clone().expect("typing in the new shape");
    let b = figcraft_engine::board::owner_of_text(doc, te.id).unwrap();
    assert!(matches!(doc.node(b).unwrap().board, Some(BoardItem::Shape { shape: BoardShape::Diamond })));
    let joined = doc.nodes().any(|n| {
        matches!(&n.board, Some(BoardItem::Connector(k)) if k.from == figcraft_doc::End::Node { node: a, side: figcraft_doc::Side::Right }
            && k.to == figcraft_doc::End::Node { node: b, side: figcraft_doc::Side::Left })
    });
    assert!(joined, "a connector from the first shape's right side to the new one's left");
}

#[test]
fn a_board_page_has_a_dot_grid_under_its_layers_thinned_out_zoomed_out() {
    let mut u = board();
    let sticky = id(&u.app.session.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap());
    u.app.session.execute("view.zoom100", &json!({})).unwrap();
    u.frame(vec![]);
    let t = crate::theme::Tokens::get(&u.ctx);
    let page = figcraft_doc::BOARD_PAGE;
    let bg = egui::Color32::from_rgb((page.r * 255.0).round() as u8, (page.g * 255.0).round() as u8, (page.b * 255.0).round() as u8);
    let dots = crate::board_bar::dots(&u.app, bg, &t).expect("a board has dots");
    assert!(dots.color.a > 0.0 && dots.color.r < 0.5, "dark dots on the light page: {:?}", dots.color);
    // The canvas render draws them on the page, and the layers over them.
    let ds = u.app.session.ds().unwrap();
    let view = figcraft_render::View { origin: (-60.0, -60.0), scale: 1.0, width: 400, height: 400 };
    let opts = figcraft_render::Options { dots: Some(dots), ..Default::default() };
    let pix = figcraft_render::render_view(&ds.doc, ds.page, &view, &opts).unwrap();
    let at = |x: f64, y: f64| {
        let p = pix.pixel((x + 60.0) as u32, (y + 60.0) as u32).unwrap();
        [p.red(), p.green(), p.blue()]
    };
    let plain = [bg.r(), bg.g(), bg.b()];
    let step = dots.step;
    assert_ne!(at(-2.0 * step, -2.0 * step), plain, "a dot on the page");
    assert_eq!(at(-2.0 * step + step / 2.0, -2.0 * step), plain, "between the dots");
    let fill = u.app.session.doc().unwrap().node(sticky).unwrap().fills[0].solid_color().unwrap().to_rgba8();
    assert_eq!(at(2.0 * step, 2.0 * step), [fill[0], fill[1], fill[2]], "the sticky covers the dot under it");
    // Zoomed out, the dots space out: every second, fourth… stays, the next to go fading.
    let dot_spacing = |z: f64| crate::board_bar::grid_spacing(z, 24.0, 2, 14.0);
    assert_eq!(dot_spacing(1.0), Some((24.0, ((24.0 - 14.0) / 14.0) as f32)));
    assert_eq!(dot_spacing(0.25).map(|d| d.0), Some(96.0));
    assert_eq!(dot_spacing(4.0), Some((24.0, 1.0)));
    for z in [1e-300, 1e-9, 0.01, 0.37, 1.0, 64.0, 1e300] {
        let (step, fade) = dot_spacing(z).unwrap();
        assert!(step * z >= 14.0 && step * z < 28.0 || step == 24.0, "{z}: {step}");
        assert!((0.0..=1.0).contains(&fade));
    }
    assert_eq!(dot_spacing(0.0), None);
    assert_eq!(dot_spacing(f64::NAN), None);
    // Design files have none.
    let d = Ui::new(Session::new());
    assert!(crate::board_bar::dots(&d.app, bg, &t).is_none());
}

#[test]
fn a_selected_connector_shows_its_ends_as_handles() {
    let mut u = board();
    let a = u.app.session.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
    let b = u.app.session.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap()["id"].clone();
    u.app.session.execute("board.connector", &json!({"from": a, "to": b, "style": "straight"})).unwrap();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let ends = [u.app.to_screen(200.0, 100.0), u.app.to_screen(400.0, 100.0)];
    for end in ends {
        let ring = shapes.iter().any(|s| matches!(s, egui::Shape::Circle(c) if (c.center - end).length() < 0.5 && c.radius == 5.0));
        assert!(ring, "a handle at {end:?}");
    }
}

#[test]
fn the_board_grid_follows_its_settings() {
    let mut u = board();
    u.app.session.execute("view.zoom100", &json!({})).unwrap();
    u.frame(vec![]);
    let t = crate::theme::Tokens::get(&u.ctx);
    let bg = egui::Color32::from_rgb(240, 240, 240);
    let usual = crate::board_bar::dots(&u.app, bg, &t).unwrap();
    assert!(!usual.lines && usual.major == 2);
    // Lines, as fine at any zoom.
    u.app.run("board.grid", json!({"style": "lines"}));
    let lines = crate::board_bar::dots(&u.app, bg, &t).unwrap();
    assert!(lines.lines && lines.major == 2 && lines.color.a < usual.color.a, "lines start fainter than dots");
    // Graph paper: a strong line every 5 cells of 8, the cells in between fainter even zoomed in.
    u.app.run("board.grid", json!({"style": "graph", "spacing": 8, "major": 5}));
    u.app.session.ds_mut().unwrap().view.zoom = 4.0;
    let graph = crate::board_bar::dots(&u.app, bg, &t).unwrap();
    assert_eq!((graph.lines, graph.major, graph.step), (true, 5, 8.0));
    assert!(graph.faded.a < graph.color.a);
    // Zoomed out, the fine cells go and the strong lines stay: the step grows by fives.
    u.app.session.ds_mut().unwrap().view.zoom = 0.25;
    let far = crate::board_bar::dots(&u.app, bg, &t).unwrap();
    assert_eq!(far.step, 8.0 * 5.0, "40 × 0.25 = 10 points apart, the closest lines come");
    assert_eq!(far.faded.a, 0.0, "the cells the next step drops have faded out");
    // More contrast: stronger marks; larger: larger.
    u.app.run("board.grid", json!({"style": "dots", "contrast": 3, "size": 2}));
    let strong = crate::board_bar::dots(&u.app, bg, &t).unwrap();
    assert!(strong.color.a > usual.color.a * 2.5 && strong.size > usual.size * 0.25 * 1.5, "{strong:?}");
    // None: no grid.
    u.app.run("board.grid", json!({"style": "none"}));
    assert!(crate::board_bar::dots(&u.app, bg, &t).is_none());
    use crate::board_bar::grid_spacing;
    assert_eq!(grid_spacing(1.0, 24.0, 5, 0.0), None);
    assert_eq!(grid_spacing(1e-320, 512.0, 2, 14.0), None, "no finite step that far out");
    assert!(grid_spacing(1e-300, 4.0, 12, 10.0).is_some_and(|(step, _)| step.is_finite()));
}

#[test]
fn preferences_set_the_board_grid_and_the_default_dots_keep_their_size() {
    let mut u = board();
    let bg = egui::Color32::from_rgb(240, 240, 240);
    // The default grid: dots a point and a half across at 100 %, one to three points on screen.
    for (zoom, on_screen) in [(1.0, 1.5), (0.25, 1.0), (4.0, 3.0)] {
        u.app.session.ds_mut().unwrap().view.zoom = zoom;
        let t = crate::theme::Tokens::get(&u.ctx);
        let size = crate::board_bar::dots(&u.app, bg, &t).unwrap().size;
        assert!((size * zoom - on_screen).abs() < 1e-9, "at {zoom}: {size}");
    }
    u.app.session.ds_mut().unwrap().view.zoom = 1.0;
    u.app.ui.preferences = true;
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Board grid") && has_text(&shapes, "Spacing"));
    assert!(!has_text(&shapes, "Strong line every") && !has_text(&shapes, "Reset grid"), "dots, as usual");
    click(&mut u, text_center(&shapes, "Graph paper"));
    assert_eq!(u.app.session.prefs.board_grid.style, figcraft_engine::board_grid::GridStyle::Graph);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Strong line every"), "graph paper's cells per strong line");
    click(&mut u, text_center(&shapes, "Reset grid"));
    assert_eq!(u.app.session.prefs.board_grid, Default::default());
    // No grid: nothing to set but the style.
    u.app.run("board.grid", json!({"style": "none"}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Board grid") && !has_text(&shapes, "Spacing"));
}
