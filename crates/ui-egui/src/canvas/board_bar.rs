//! Boards in the UI: the small bar floating over a board's selection (colour, shape, text size,
//! connector style; boards hide the Design panel until asked for), and the swatches it and the
//! board tools' flyouts share.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{BOARD_COLORS, BoardItem, BoardShape, NodeId, NodeKind, Routing, Stamp, StrokeCap};
use figcraft_engine::board::{BoardTool, TEXT_SIZES};
use figcraft_engine::board_grid::GridStyle;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, semibold};
use crate::toolbar::Options;

/// What a swatch shows.
pub(crate) enum Mark {
    Color(figcraft_color::Color),
    Shape(BoardShape),
    Stamp(Stamp),
}

/// One choice of a board tool's flyout: its name, the tool's params, whether it is the tool's now.
pub(crate) struct Choice {
    pub label: &'static str,
    pub params: Value,
    pub on: bool,
    pub mark: Mark,
}

/// The choices a board tool's flyout offers.
pub(crate) fn choices(options: Options, board: &BoardTool) -> Vec<Choice> {
    match options {
        Options::Colors => BOARD_COLORS
            .iter()
            .map(|(n, c)| Choice { label: color_label(n), params: json!({"color": n}), on: board.sticky == *c, mark: Mark::Color(*c) })
            .collect(),
        Options::Shapes => BoardShape::ALL
            .iter()
            .map(|b| Choice { label: b.label(), params: json!({"shape": b.id()}), on: board.shape == *b, mark: Mark::Shape(*b) })
            .collect(),
        Options::Stamps => Stamp::ALL
            .iter()
            .map(|b| Choice { label: b.label(), params: json!({"stamp": b.id()}), on: board.stamp == *b, mark: Mark::Stamp(*b) })
            .collect(),
        Options::None => Vec::new(),
    }
}

/// A board colour's name as the UI says it.
fn color_label(name: &str) -> &'static str {
    match name {
        "yellow" => tr_noop("Yellow"),
        "orange" => tr_noop("Orange"),
        "red" => tr_noop("Red"),
        "pink" => tr_noop("Pink"),
        "violet" => tr_noop("Violet"),
        "blue" => tr_noop("Blue"),
        "green" => tr_noop("Green"),
        _ => tr_noop("Gray"),
    }
}

/// Paint `mark` in `r`: a colour chip, a shape's outline in `ink`, or a stamp badge.
pub(crate) fn paint_choice(p: &egui::Painter, r: Rect, mark: &Mark, ink: Color32) {
    match mark {
        Mark::Color(c) => {
            p.rect_filled(r, CornerRadius::same(4), c32(*c));
            p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, Color32::from_black_alpha(40)), StrokeKind::Inside);
        }
        Mark::Shape(s) => {
            let path = s.path(14.0, 14.0);
            let path = figcraft_geom::Affine::translate((1.0, 1.0)) * path;
            icons::paint_outlines(p, r, 16.0, &[(&path, false)], ink, 1.2);
        }
        Mark::Stamp(s) => {
            p.circle_filled(r.center(), r.width().min(r.height()) / 2.0, c32(s.color()));
            let parts: Vec<(figcraft_geom::BezPath, bool)> =
                s.mark().iter().filter_map(|m| figcraft_geom::BezPath::from_svg(m.d).ok().map(|b| (b, m.filled))).collect();
            let refs: Vec<(&figcraft_geom::BezPath, bool)> = parts.iter().map(|(b, f)| (b, *f)).collect();
            icons::paint_outlines(p, r, figcraft_doc::STAMP_SIZE, &refs, Color32::WHITE, 3.0);
        }
    }
}

fn c32(c: figcraft_color::Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// What the selection has that the bar can change.
#[derive(Default)]
struct Has {
    color: bool,
    shape: Option<BoardShape>,
    text: bool,
    connector: Option<Line>,
    /// Every selected board object is a connector: the colour part offers any colour too.
    only_lines: bool,
    table: bool,
}

/// The selected connectors' line as the bar shows it (the first one's), and their ids.
struct Line {
    routing: Routing,
    start: StrokeCap,
    end: StrokeCap,
    weight: f64,
    dashed: bool,
    ids: Vec<NodeId>,
}

/// What the selected board objects take from the bar (layers that aren't board objects, a section
/// say, are left out); `None` when none takes anything.
fn has(app: &FigApp) -> Option<Has> {
    let ds = app.session.ds().ok()?;
    let mut h = Has::default();
    let mut others = false;
    for n in ds.selection.iter().filter_map(|id| ds.doc.node(*id)) {
        others |= n.board.is_some() && !matches!(n.board, Some(BoardItem::Connector(_)));
        match &n.board {
            Some(BoardItem::Sticky { .. } | BoardItem::Ink) => h.color = true,
            Some(BoardItem::Shape { shape }) => {
                h.color = true;
                h.text = true;
                h.shape = Some(*shape);
            }
            Some(BoardItem::Connector(c)) => {
                h.color = true;
                let s = &n.stroke;
                let line = h.connector.get_or_insert_with(|| Line {
                    routing: c.routing,
                    start: s.cap_start,
                    end: s.cap_end,
                    weight: s.weight,
                    dashed: !s.dashes.is_empty(),
                    ids: Vec::new(),
                });
                line.ids.push(n.id);
            }
            Some(BoardItem::Label { .. }) => h.text = true,
            Some(BoardItem::Table) => h.table = true,
            Some(BoardItem::Stamp { .. }) | None => {}
        }
    }
    h.only_lines = h.connector.is_some() && !others;
    (h.color || h.text || h.connector.is_some() || h.table).then_some(h)
}

const CELL: f32 = 28.0;
const SWATCH: f32 = 20.0;
const GAP: f32 = 4.0;
/// The room each part takes after its controls (the divider and the gap around it).
const PART_END: f32 = 8.0;

impl Has {
    /// The bar's width for these parts.
    fn width(&self) -> f32 {
        let mut w = 8.0;
        if self.color {
            w += BOARD_COLORS.len() as f32 * (SWATCH + GAP) + PART_END;
            if self.only_lines {
                w += SWATCH + GAP;
            }
        }
        if self.shape.is_some() {
            w += CELL + 12.0 + GAP + PART_END;
        }
        if self.text {
            w += TEXT_SIZES.len() as f32 * (CELL + 2.0) + PART_END;
        }
        if self.connector.is_some() {
            w += LINE_W;
        }
        if self.table {
            w += crate::board_picker::TABLE_PART_W;
        }
        w
    }
}

/// Where a bar `size` big goes for the selection's screen box `sel`: centred above it, below it
/// when there is no room above, inside the canvas.
pub(crate) fn place(sel: Rect, size: egui::Vec2, canvas: Rect) -> Pos2 {
    let w = size.x;
    let cx = sel.center().x.clamp(canvas.min.x + w / 2.0 + 8.0, (canvas.max.x - w / 2.0 - 8.0).max(canvas.min.x + w / 2.0 + 8.0));
    let above = sel.min.y - 14.0 - size.y;
    let y = if above > canvas.min.y + 8.0 { above } else { (sel.max.y + 14.0).min(canvas.max.y - size.y - 80.0) };
    pos2(cx - w / 2.0, y)
}

/// The bar above a board's selection, while nothing is being dragged and the file can be edited.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let editable = app.session.active_read_only().is_none();
    if app.session.tool.busy() || !editable || !app.session.doc().is_ok_and(figcraft_engine::board::is_board) {
        return;
    }
    let Some(h) = has(app) else { return };
    let Some(b) = app.session.ds().ok().and_then(|ds| ds.doc.bounds_of(&ds.selection)) else { return };
    let mut sel = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1));
    // Clear of the "+" handles above and below a sticky or shape.
    if crate::board_connect::shown(app) {
        sel = sel.expand2(vec2(0.0, crate::board_connect::REACH));
    }
    let size = vec2(h.width(), 40.0);
    let at = place(sel, size, app.canvas.rect);
    let t = Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("board_bar")).fixed_pos(at).order(egui::Order::Middle).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::click());
        a11y::container_bounds(ui, rect);
        let p = ui.painter();
        p.rect_filled(rect.expand(2.0).translate(vec2(0.0, 1.0)), CornerRadius::same(12), Color32::from_black_alpha(18));
        p.rect_filled(rect, CornerRadius::same(10), t.panel);
        p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
        let cy = rect.center().y;
        let mut x = rect.min.x + 8.0;
        if h.color {
            let lines = h.connector.as_ref().filter(|_| h.only_lines).map(|l| l.ids.as_slice());
            x = color_part(app, ui, x, cy, &t, lines);
        }
        if let Some(shape) = h.shape {
            x = shape_part(app, ui, shape, pos2(x, cy), &t);
        }
        if h.text {
            x = size_part(app, ui, x, cy, &t);
        }
        if h.table {
            x = crate::board_picker::table_part(app, ui, x, cy, &t);
        }
        if let Some(line) = &h.connector {
            line_part(app, ui, line, pos2(x, cy), &t);
        }
    });
}

/// The board colours from `x` (`board.setColor`), and for connectors `lines` any colour in the
/// colour picker; where the next part starts.
fn color_part(app: &mut FigApp, ui: &mut Ui, mut x: f32, cy: f32, t: &Tokens, lines: Option<&[NodeId]>) -> f32 {
    for (i, (name, c)) in BOARD_COLORS.iter().enumerate() {
        let r = Rect::from_center_size(pos2(x + SWATCH / 2.0, cy), vec2(SWATCH, SWATCH));
        let resp = ui.interact(r, ui.scope_id().with(("bb_color", i)), Sense::click()).on_hover_text(tr(color_label(name)));
        a11y::button(&resp, tr(color_label(name)));
        if resp.hovered() {
            ui.painter().rect_filled(r.expand(2.0), CornerRadius::same(6), t.hover);
        }
        paint_choice(ui.painter(), r, &Mark::Color(*c), t.icon);
        if resp.clicked() {
            app.run("board.setColor", json!({"color": name}));
        }
        x += SWATCH + GAP;
    }
    if let Some(ids) = lines {
        let r = Rect::from_center_size(pos2(x + SWATCH / 2.0, cy), vec2(SWATCH, SWATCH));
        let resp = ui.interact(r, ui.scope_id().with("bb_color_any"), Sense::click()).on_hover_text(tr("Edit colour"));
        a11y::button(&resp, tr("Edit colour"));
        if resp.hovered() {
            ui.painter().rect_filled(r.expand(2.0), CornerRadius::same(6), t.hover);
        }
        paint_wheel(ui.painter(), r.center(), SWATCH / 2.0 - 1.0);
        app.picker_control(crate::colorpicker::PickerTarget::Paint { fills: false, index: 0, ids: ids.to_vec() }, &resp);
        x += SWATCH + GAP;
    }
    divider(ui, t, x, cy)
}

/// A small colour wheel: the button for any colour.
fn paint_wheel(p: &egui::Painter, c: Pos2, r: f32) {
    const STEPS: usize = 12;
    let at = |i: usize| {
        let a = i as f32 / STEPS as f32 * std::f32::consts::TAU;
        c + vec2(a.cos(), a.sin()) * r
    };
    for i in 0..STEPS {
        let hue = egui::ecolor::Hsva::new(i as f32 / STEPS as f32, 0.75, 0.95, 1.0);
        p.add(egui::Shape::convex_polygon(vec![c, at(i), at(i + 1)], Color32::from(hue), Stroke::NONE));
    }
    p.circle_filled(c, r * 0.4, Color32::WHITE);
}

/// Shapes in a row of the shape menu.
const SHAPE_COLUMNS: usize = 6;

/// The shape menu at `at` (its left edge and middle), showing `shape`; where the next part starts.
fn shape_part(app: &mut FigApp, ui: &mut Ui, shape: BoardShape, at: Pos2, t: &Tokens) -> f32 {
    let r = Rect::from_min_size(pos2(at.x, at.y - CELL / 2.0), vec2(CELL + 12.0, CELL));
    let resp = ui.interact(r, ui.scope_id().with("bb_shape"), Sense::click()).on_hover_text(tr("Shape"));
    a11y::menu_button(&resp, tr("Shape"), a11y::menu_open(&resp));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    paint_choice(ui.painter(), Rect::from_center_size(pos2(r.min.x + CELL / 2.0, at.y), vec2(18.0, 18.0)), &Mark::Shape(shape), t.icon);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 7.0, at.y), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    crate::right::menu_popup(&resp).show(|ui| {
        // A grid of outlines, named on hover (a list of them all would run off the screen).
        egui::Grid::new("bb_shapes").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for (i, b) in BoardShape::ALL.into_iter().enumerate() {
                let (r, cell) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                let cell = cell.on_hover_text(tr(b.label()));
                a11y::menu_item(&cell, tr(b.label()), "", Some(b == shape));
                if b == shape || cell.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
                }
                paint_choice(ui.painter(), r.shrink(6.0), &Mark::Shape(b), t.menu_text);
                if cell.clicked() {
                    app.run("board.setStyle", json!({"shape": b.id()}));
                    ui.close();
                }
                if i % SHAPE_COLUMNS == SHAPE_COLUMNS - 1 {
                    ui.end_row();
                }
            }
        });
    });
    divider(ui, t, at.x + CELL + 12.0 + GAP, at.y)
}

/// The text sizes from `x` (`board.setStyle {fontSize}`); where the next part starts.
fn size_part(app: &mut FigApp, ui: &mut Ui, mut x: f32, cy: f32, t: &Tokens) -> f32 {
    for (label, size) in TEXT_SIZES {
        let r = Rect::from_center_size(pos2(x + CELL / 2.0, cy), vec2(CELL, CELL));
        let resp = ui.interact(r, ui.scope_id().with(("bb_size", label)), Sense::click()).on_hover_text(tr("Text size"));
        a11y::button(&resp, &format!("{} {label}", tr("Text size")));
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, semibold(11.0), t.text);
        if resp.clicked() {
            app.run("board.setStyle", json!({"fontSize": size}));
        }
        x += CELL + 2.0;
    }
    divider(ui, t, x, cy)
}

/// A start or end point menu's width: the end's picture and the menu's chevron.
const CAP_W: f32 = 62.0;
/// The line style menu's width.
const STYLE_W: f32 = CELL + 12.0;
/// The connector part's width: the three routings, the two ends and the line style.
const LINE_W: f32 = 3.0 * (CELL + 2.0) + 8.0 + 2.0 * CAP_W + 2.0 + 8.0 + STYLE_W + PART_END;
/// The weights the line style menu offers.
const WEIGHTS: [f64; 4] = [1.0, 2.0, 4.0, 6.0];
/// A dashed connector's dashes and gaps.
const DASHES: [f64; 2] = [8.0, 6.0];

/// A connector's routing (straight, elbow, curved), its start and end points and its line style at
/// `at`, as they are now (`line`): `board.setStyle`, and the stroke's dashes and weight.
fn line_part(app: &mut FigApp, ui: &mut Ui, line: &Line, at: Pos2, t: &Tokens) {
    let cells = [
        (Icon::Line, tr("Straight"), Routing::Straight, "straight"),
        (Icon::Connector, tr("Elbow"), Routing::Elbow, "elbow"),
        (Icon::CurvedConnector, tr("Curved"), Routing::Curved, "curved"),
    ];
    let mut x = at.x;
    for (i, (icon, tip, routing, id)) in cells.into_iter().enumerate() {
        let r = Rect::from_center_size(pos2(x + CELL / 2.0, at.y), vec2(CELL, CELL));
        if crate::toolbar::tool_cell(ui, t, r, ("bb_line", i), icon, line.routing == routing, tip).clicked() {
            app.run("board.setStyle", json!({"style": id}));
        }
        x += CELL + 2.0;
    }
    x = divider(ui, t, x - 2.0, at.y);
    for (start, cap) in [(true, line.start), (false, line.end)] {
        cap_menu(app, ui, t, pos2(x, at.y), start, cap);
        x += CAP_W + 2.0;
    }
    x = divider(ui, t, x - 2.0, at.y);
    style_menu(app, ui, t, pos2(x, at.y), line);
}

/// The menu of a connector's start (`start`) or end point at `at` (its left edge and middle),
/// showing `cap`.
fn cap_menu(app: &mut FigApp, ui: &mut Ui, t: &Tokens, at: Pos2, start: bool, cap: StrokeCap) {
    let r = Rect::from_min_size(pos2(at.x, at.y - CELL / 2.0), vec2(CAP_W, CELL));
    let label = if start { tr("Start point") } else { tr("End point") };
    let resp = ui.interact(r, ui.scope_id().with(("bb_cap", start)), Sense::click()).on_hover_text(label);
    a11y::menu_button(&resp, label, a11y::menu_open(&resp));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    // The picture ends 14 before the rectangle it is given: 4 in from this cell's left edge.
    let art = Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(r.min.x + 62.0, r.max.y));
    crate::right::cap_art(ui.painter(), art, t.icon, cap, start);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 7.0, at.y), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    let key = if start { "startCap" } else { "endCap" };
    crate::right::menu_popup(&resp).show(|ui| {
        for c in StrokeCap::ALL {
            let art = |p: &egui::Painter, row: Rect, ink: Color32| crate::right::cap_art(p, row, ink, c, start);
            if crate::right::menu_item_with(ui, tr(c.label()), "", crate::right::CAP_ART_W, art) {
                app.run("board.setStyle", json!({key: c}));
                ui.close();
            }
        }
    });
}

/// A line `weight` thick, dashed or not, across `r`'s middle.
fn paint_line(p: &egui::Painter, r: Rect, ink: Color32, weight: f64, dashed: bool) {
    let w = (weight as f32).clamp(1.0, 5.0);
    let ends = [pos2(r.min.x, r.center().y), pos2(r.max.x, r.center().y)];
    if dashed {
        p.extend(egui::Shape::dashed_line(&ends, Stroke::new(w, ink), 5.0, 3.0));
    } else {
        p.line_segment(ends, Stroke::new(w, ink));
    }
}

/// The menu of the selected connectors' line style at `at`: solid or dashed, and its weight.
fn style_menu(app: &mut FigApp, ui: &mut Ui, t: &Tokens, at: Pos2, line: &Line) {
    let r = Rect::from_min_size(pos2(at.x, at.y - CELL / 2.0), vec2(STYLE_W, CELL));
    let label = tr("Stroke style");
    let resp = ui.interact(r, ui.scope_id().with("bb_line_style"), Sense::click()).on_hover_text(label);
    a11y::menu_button(&resp, label, a11y::menu_open(&resp));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    let sample = Rect::from_min_max(pos2(r.min.x + 5.0, at.y), pos2(r.max.x - 15.0, at.y));
    paint_line(ui.painter(), sample, t.icon, line.weight, line.dashed);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 7.0, at.y), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    let ids = line.ids.clone();
    crate::right::menu_popup(&resp).show(|ui| {
        let art = |dashed: bool, weight: f64| {
            move |p: &egui::Painter, row: Rect, ink: Color32| {
                let r = Rect::from_min_max(pos2(row.max.x - 44.0, row.center().y), pos2(row.max.x - 12.0, row.center().y));
                paint_line(p, r, ink, weight, dashed);
            }
        };
        for (dashed, name) in [(false, tr("Solid")), (true, tr("Dash"))] {
            if crate::right::menu_item_with(ui, name, "", 48.0, art(dashed, 2.0)) {
                let dashes: &[f64] = if dashed { &DASHES } else { &[] };
                app.set_props(&ids, json!({"dashes": dashes}));
                ui.close();
            }
        }
        ui.separator();
        for w in WEIGHTS {
            if crate::right::menu_item_with(ui, &format!("{} {w}", tr("Stroke weight")), "", 48.0, art(false, w)) {
                app.set_props(&ids, json!({"strokeWeight": w}));
                ui.close();
            }
        }
    });
}

/// A thin separator after `x`; where the next part starts.
pub(crate) fn divider(ui: &Ui, t: &Tokens, x: f32, cy: f32) -> f32 {
    ui.painter().line_segment([pos2(x + 2.0, cy - 10.0), pos2(x + 2.0, cy + 10.0)], Stroke::new(1.0, t.border));
    x + 8.0
}

/// The closest the grid's marks come on screen (points): zoomed out, every second, fourth… mark
/// stays (graph paper: every strong line), the ones going fading out first. Lines read at a
/// smaller gap than dots.
const DOT_MIN_GAP: f64 = 14.0;
const LINE_MIN_GAP: f64 = 10.0;

/// A board page's grid on its background `bg` as the preferences set it (`board.grid`), for the
/// canvas render to draw under the layers: dark marks on a light page, light ones on a dark page,
/// in the theme's ink. `None` on design files and with the grid off.
pub fn dots(app: &FigApp, bg: Color32, t: &Tokens) -> Option<figcraft_render::Dots> {
    let ds = app.session.ds().ok()?;
    if !figcraft_engine::board::is_board(&ds.doc) {
        return None;
    }
    let g = app.session.prefs.board_grid.clone().sanitized();
    let lines = match g.style {
        GridStyle::None => return None,
        GridStyle::Dots => false,
        GridStyle::Lines | GridStyle::Graph => true,
    };
    // Dots and plain lines thin out by halves; graph paper keeps its strong lines.
    let major = if g.style == GridStyle::Graph { g.major } else { 2 };
    let zoom = ds.view.zoom;
    let min_gap = if lines { LINE_MIN_GAP } else { DOT_MIN_GAP };
    let (step, fade) = grid_spacing(zoom, g.spacing, major, min_gap)?;
    let dark_page = 0.2126 * f32::from(bg.r()) + 0.7152 * f32::from(bg.g()) + 0.0722 * f32::from(bg.b()) < 128.0;
    let ink = if dark_page == t.dark { t.text } else { t.bg };
    // Lines cover more of the page than dots, so they start fainter.
    let base = if dark_page { 0.22 } else { 0.18 } * if lines { 0.6 } else { 1.0 };
    let alpha = (base * g.contrast as f32).clamp(0.0, 1.0);
    // Graph paper's in-between lines stay fainter than its strong ones even zoomed in.
    let faded = alpha * fade * if g.style == GridStyle::Graph { 0.5 } else { 1.0 };
    let color = |a: f32| figcraft_color::Color::from_rgba8(ink.r(), ink.g(), ink.b(), (a * 255.0).round() as u8);
    // Dots are about a point and a half across on screen, growing a little zoomed in, scaled by
    // the size setting (1, the default, keeps the usual dots).
    let k = g.size;
    let size = if lines { k.max(0.5) / zoom } else { (1.5 * zoom.sqrt()).clamp(1.0, 3.0) * k / zoom };
    Some(figcraft_render::Dots { step, size, color: color(alpha), faded: color(faded), lines, major })
}

/// The grid's spacing on the page at `zoom`, starting from `base` and multiplied by `major` until
/// the marks are at least `min_gap` apart on screen, and how strongly the marks the next step
/// drops still show (0 to 1).
pub(crate) fn grid_spacing(zoom: f64, base: f64, major: u32, min_gap: f64) -> Option<(f64, f32)> {
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if !(ok(zoom) && ok(base) && ok(min_gap)) {
        return None;
    }
    let factor = f64::from(major.max(2));
    // Bounded: past about 1000 multiplications the step is no longer finite, and there is no grid.
    let mut step = base;
    for _ in 0..1100 {
        if step * zoom >= min_gap {
            break;
        }
        step *= factor;
    }
    let gap = step * zoom;
    // From one to two minimum gaps apart, the in-between marks fade in.
    (gap.is_finite() && gap >= min_gap).then(|| (step, ((gap - min_gap) / min_gap).clamp(0.0, 1.0) as f32))
}

/// The Preferences dialog's board grid settings (`board.grid`): its style, then (with a grid)
/// how far apart, how large and how strong its marks are, and graph paper's cells per strong line.
pub(crate) fn grid_settings(app: &mut FigApp, ui: &mut Ui) {
    let g = app.session.prefs.board_grid.clone().sanitized();
    let mut set = None;
    ui.horizontal(|ui| {
        ui.label(tr("Board grid"));
        let styles = [
            (GridStyle::Dots, tr_noop("Dots")),
            (GridStyle::Lines, tr_noop("Lines")),
            (GridStyle::Graph, tr_noop("Graph paper")),
            (GridStyle::None, tr_noop("None")),
        ];
        for (style, label) in styles {
            if ui.selectable_label(g.style == style, tr(label)).clicked() {
                set = Some(json!({"style": style.id()}));
            }
        }
    });
    if g.style != GridStyle::None {
        ui.horizontal(|ui| {
            let (mut spacing, mut major) = (g.spacing, g.major);
            ui.label(tr("Spacing"));
            let (lo, hi) = figcraft_engine::board_grid::SPACING;
            if ui.add(egui::DragValue::new(&mut spacing).range(lo..=hi)).changed() {
                set = Some(json!({"spacing": spacing}));
            }
            if g.style == GridStyle::Graph {
                ui.label(tr("Strong line every"));
                let (lo, hi) = figcraft_engine::board_grid::MAJOR;
                if ui.add(egui::DragValue::new(&mut major).range(lo..=hi)).changed() {
                    set = Some(json!({"major": major}));
                }
            }
        });
        ui.horizontal(|ui| {
            let (mut size, mut contrast) = (g.size, g.contrast);
            ui.label(tr("Size"));
            let (lo, hi) = figcraft_engine::board_grid::SIZE;
            if ui.add(egui::Slider::new(&mut size, lo..=hi).max_decimals(2)).changed() {
                set = Some(json!({"size": size}));
            }
            ui.label(tr("Contrast"));
            let (lo, hi) = figcraft_engine::board_grid::CONTRAST;
            if ui.add(egui::Slider::new(&mut contrast, lo..=hi).max_decimals(2)).changed() {
                set = Some(json!({"contrast": contrast}));
            }
        });
        if g != Default::default() && crate::widgets::button(ui, tr("Reset grid"), false, true).clicked() {
            set = Some(json!({"reset": true}));
        }
    }
    if let Some(p) = set {
        app.run("board.grid", p);
    }
}

/// The name of the person who placed the stamp under the pointer, beside it.
pub fn stamp_author(app: &FigApp, painter: &egui::Painter, pointer: Option<Pos2>) {
    let (Some(at), Ok(ds)) = (pointer, app.session.ds()) else { return };
    let Some(n) = ds.hover.and_then(|h| ds.doc.node(h)) else { return };
    let Some(BoardItem::Stamp { author, .. }) = &n.board else { return };
    if author.is_empty() || !matches!(n.kind, NodeKind::Frame) {
        return;
    }
    let t = Tokens::get(painter.ctx());
    let galley = painter.layout_no_wrap(author.clone(), semibold(11.0), t.text);
    let r = Rect::from_min_size(at + vec2(14.0, 14.0), galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(r, CornerRadius::same(4), t.menu);
    painter.galley(r.min + vec2(6.0, 3.0), galley, t.menu_text);
}
