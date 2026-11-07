//! The floating bottom toolbar (UI3): tool groups with flyouts, comments, actions.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::ToolId;
use figcraft_engine::vectoredit::VTool;
use serde_json::json;

use crate::FigApp;
use crate::a11y::{self, Nav, Region, Tip};
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular};

/// Every mark in the toolbar row is painted in a box this size, so they share one optical size
/// and one stroke width (#737, #738).
const GLYPH: f32 = 18.0;
/// The flyout carets: smaller than the marks, but on a box whose stroke rounds to the same width
/// as theirs at 1×, 1.5× and 2× (a 10-point box drew them one physical pixel thinner).
const CARET: f32 = 14.0;
/// A flyout caret's button: wide enough to hit, a little clear of its tool's (#988).
const FLYOUT_W: f32 = 16.0;
const FLYOUT_GAP: f32 = 2.0;

// The pill's grid: every state of it (the tools, the vector and shape builder ones) is laid out on
// one 32 pt cell, PAD in from its ends and GAP between its top-level items, the divider included.
/// A toolbar button's size: the cell everything in the pill is placed on.
const TOOL_CELL: f32 = 32.0;
/// The pill's left and right padding.
const PAD: f32 = 8.0;
/// Between the pill's top-level items — the divider included.
const GAP: f32 = 8.0;
/// The divider's width.
const DIVIDER: f32 = 1.0;
/// The mode switcher's marks: smaller than the tools' (13–14 pt).
const MODE_GLYPH: f32 = 14.0;

/// `GLYPH`'s box centred in `r`.
fn glyph_rect(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), vec2(GLYPH, GLYPH))
}

struct Group {
    key: &'static str,
    /// The flyout button's name for screen readers.
    name: &'static str,
    tools: &'static [(ToolId, Icon)],
    /// What a one-tool group's flyout offers instead of tools (boards: colours, shapes, stamps).
    options: Options,
}

/// The choices a board tool's flyout shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Options {
    None,
    Colors,
    Shapes,
    Stamps,
}

impl Group {
    const fn tools(key: &'static str, name: &'static str, tools: &'static [(ToolId, Icon)]) -> Group {
        Group { key, name, tools, options: Options::None }
    }
    fn has_flyout(&self) -> bool {
        self.tools.len() > 1 || self.options != Options::None
    }
}

const GROUPS: &[Group] = &[
    Group::tools("move", tr_noop("Move tools"), &[(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand), (ToolId::Scale, Icon::Scale)]),
    Group::tools("frame", tr_noop("Frame tools"), &[(ToolId::Frame, Icon::Frame), (ToolId::Section, Icon::Section), (ToolId::Slice, Icon::Slice)]),
    Group::tools(
        "shape",
        tr_noop("Shape tools"),
        &[
            (ToolId::Rectangle, Icon::Rectangle),
            (ToolId::Line, Icon::Line),
            (ToolId::Arrow, Icon::Arrow),
            (ToolId::Ellipse, Icon::Ellipse),
            (ToolId::Polygon, Icon::Polygon),
            (ToolId::Star, Icon::Star),
        ],
    ),
    Group::tools(
        "pen",
        tr_noop("Drawing tools"),
        &[(ToolId::Pen, Icon::Pen), (ToolId::Pencil, Icon::Pencil), (ToolId::ShapeBuilder, Icon::ShapeBuilder)],
    ),
    Group::tools("text", tr_noop("Text"), &[(ToolId::Text, Icon::Text), (ToolId::TextOnPath, Icon::TextOnPath)]),
    Group::tools("comment", tr_noop("Comment"), &[(ToolId::Comment, Icon::Comment)]),
];

/// A board's toolbar: moving around, ink, stickies, shapes, connectors, text, sections, stamps.
const BOARD_GROUPS: &[Group] = &[
    Group::tools("bmove", tr_noop("Move tools"), &[(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand)]),
    Group::tools(
        "ink",
        tr_noop("Drawing tools"),
        &[(ToolId::Marker, Icon::Marker), (ToolId::Highlighter, Icon::Highlighter), (ToolId::Eraser, Icon::Eraser)],
    ),
    Group { key: "sticky", name: tr_noop("Sticky note colors"), tools: &[(ToolId::Sticky, Icon::Board)], options: Options::Colors },
    Group { key: "bshape", name: tr_noop("Shapes"), tools: &[(ToolId::BoardShape, Icon::Shapes)], options: Options::Shapes },
    Group::tools("connector", tr_noop("Connector"), &[(ToolId::Connector, Icon::Connector)]),
    Group::tools("text", tr_noop("Text"), &[(ToolId::Text, Icon::Text)]),
    Group::tools("section", tr_noop("Section"), &[(ToolId::Section, Icon::Section)]),
    Group { key: "stamp", name: tr_noop("Stamps"), tools: &[(ToolId::Stamp, Icon::Stamp)], options: Options::Stamps },
    Group::tools("comment", tr_noop("Comment"), &[(ToolId::Comment, Icon::Comment)]),
];

/// Return the last tool index remembered for a toolbar group, falling back to `default`.
fn remembered(ui: &Ui, key: &str, default: ToolId) -> ToolId {
    ui.data(|d| d.get_temp::<u8>(egui::Id::unique(("tb_last", key)))).and_then(|i| ToolId::ALL.get(i as usize).copied()).unwrap_or(default)
}

pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let canvas = app.canvas.rect;
    app.ui.actions_rect = None;
    // A file you can't edit now (view only, or on its way to a server): moving around only.
    if let Some(why) = app.session.active_read_only() {
        view_only_toolbar(app, ui, why);
        return;
    }
    // Vector edit mode swaps the tools for the edit tools.
    if let Some(tool) = app.session.ds().ok().and_then(|d| d.vector_edit.as_ref()).map(|v| v.tool) {
        edit_toolbar(app, ui, tool);
        return;
    }
    // So does the shape builder, for its merge/remove modes.
    if app.session.tool.tool == ToolId::ShapeBuilder {
        builder_toolbar(app, ui);
        return;
    }
    let board = app.session.doc().is_ok_and(figcraft_engine::board::is_board);
    main_toolbar(app, ui, canvas, &t, board);
}

/// The main toolbar pill's width for the active file: the Actions palette matches it so the two
/// read as one bar.
pub(crate) fn bar_width(app: &FigApp) -> f32 {
    let board = app.session.doc().is_ok_and(figcraft_engine::board::is_board);
    main_width(if board { BOARD_GROUPS } else { GROUPS }, board)
}

/// The main toolbar pill's width: the tool groups with their flyout carets, Actions, the divider
/// and the mode switcher (boards: the templates and the Design panel's buttons instead).
fn main_width(groups: &[Group], board: bool) -> f32 {
    let tools: f32 = groups.iter().map(|g| TOOL_CELL + if g.has_flyout() { FLYOUT_GAP + FLYOUT_W } else { 0.0 }).sum();
    // The end, and the gaps that aren't a group's own: after Actions and after the divider (and
    // between a board's two buttons, which the end counts as one item).
    let (end, gaps) = if board { (2.0 * TOOL_CELL, 3.0) } else { (4.0 * TOOL_CELL, 2.0) };
    PAD * 2.0 + tools + (groups.len() as f32 + gaps) * GAP + TOOL_CELL + DIVIDER + end
}

/// The main toolbar pill: the tool groups with their flyouts, Actions, then the
/// Draw · Design · Prototype · Inspect mode switcher (on a board, its two buttons).
fn main_toolbar(app: &mut FigApp, ui: &mut Ui, canvas: Rect, t: &Tokens, board: bool) {
    let groups = if board { BOARD_GROUPS } else { GROUPS };
    let width = main_width(groups, board);
    let rect = pill(ui, &mut app.ui.nav, canvas, width, t);
    let cy = rect.center().y;
    let scroll = pill_scroll(ui, rect, width);
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(rect.intersect(outer_clip));
    let mut x = rect.min.x + PAD - scroll;
    for g in groups {
        x = group_cell(app, ui, g, pos2(x, cy), rect.min.y, t);
    }
    x = actions_button(app, ui, x, cy, t);
    divider(ui, t, x, cy);
    x += DIVIDER + GAP;
    if board {
        // Boards have no prototypes or Dev Mode; the Design panel is hidden until asked for.
        x = crate::board_picker::button(app, ui, pos2(x, cy), rect.min.y, t) + GAP;
        let r = Rect::from_center_size(pos2(x + TOOL_CELL / 2.0, cy), vec2(TOOL_CELL, TOOL_CELL));
        let tip = crate::shortcuts::tip(tr("Design panel"), "view.design");
        if tool_cell(ui, t, r, "board_panel", Icon::Panels, app.ui.board_panel, tip).clicked() {
            app.run("view.design", json!({}));
        }
    } else {
        mode_switcher(app, ui, x, cy, t);
    }
    ui.set_clip_rect(outer_clip);
    help_button(app, ui, canvas, rect, t);
}

/// How far the buttons of a pill `width` wide drawn at `rect` are scrolled: wider than the pill (a
/// phone), they slide sideways under a drag or the wheel.
fn pill_scroll(ui: &Ui, rect: Rect, width: f32) -> f32 {
    let scroll_id = ui.scope_id().with("tb_scroll");
    let max_scroll = (width - rect.width()).max(0.0);
    let mut scroll = 0.0;
    if max_scroll > 0.0 {
        let drag = ui.ctx().read_response(ui.scope_id().with("pill")).map_or(0.0, |r| r.drag_delta().x);
        let wheel = if ui.rect_contains_pointer(rect) { ui.input(|i| i.smooth_scroll_delta.x + i.smooth_scroll_delta.y) } else { 0.0 };
        scroll = (ui.data(|d| d.get_temp::<f32>(scroll_id)).unwrap_or(0.0) - drag - wheel).clamp(0.0, max_scroll);
    }
    ui.data_mut(|d| d.insert_temp(scroll_id, scroll));
    scroll
}

/// Tool group `g` at `at` (its left edge, the pill's middle): the shown tool's button, its flyout
/// caret and, while open, its flyout above the pill (whose top is `top`). Where the next starts.
fn group_cell(app: &mut FigApp, ui: &mut Ui, g: &Group, at: egui::Pos2, top: f32, t: &Tokens) -> f32 {
    let current = app.session.tool.tool;
    let in_group = g.tools.iter().any(|(tid, _)| *tid == current);
    let shown = if in_group { current } else { remembered(ui, g.key, g.tools.first().map(|x| x.0).unwrap_or(ToolId::Move)) };
    let icon = g.tools.iter().find(|(tid, _)| *tid == shown).map(|(_, i)| *i).unwrap_or(Icon::Move);
    let r = Rect::from_center_size(pos2(at.x + TOOL_CELL / 2.0, at.y), vec2(TOOL_CELL, TOOL_CELL));
    let resp = tool_cell(ui, t, r, ("tool", g.key), icon, in_group, &format!("{}  {}", tr(shown.label()), pretty_shortcut(shown.shortcut())));
    if in_group {
        a11y::prefer(&mut app.ui.nav, Region::Toolbar, resp.id);
    }
    if resp.clicked() {
        // Enter (a keyboard click) on the shape or frame tool already chosen places one; the
        // canvas takes the keyboard.
        // In Dev Mode the tool goes through `choose`, which returns to the design first.
        if in_group && shown.draws_shape() && app.ui.right_tab != crate::RightTab::Inspect && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            app.run("tool.place", json!({}));
            app.ui.nav.go(Region::Canvas);
        } else {
            choose(app, shown);
        }
    }
    let mut x = at.x + TOOL_CELL;
    if g.has_flyout() {
        flyout_caret(app, ui, g, pos2(x, at.y), t);
        x += FLYOUT_GAP + FLYOUT_W;
    }
    if app.ui.tool_flyout == Some(g.key) {
        if g.options == Options::None {
            flyout(app, ui, g, pos2(r.min.x, top - 8.0));
        } else {
            options_flyout(app, ui, g, pos2(r.min.x, top - 8.0));
        }
    }
    x + GAP
}

/// The caret after group `g`'s button (at `at`, its left edge and middle): opens or closes its
/// flyout.
fn flyout_caret(app: &mut FigApp, ui: &mut Ui, g: &Group, at: egui::Pos2, t: &Tokens) {
    let cr = Rect::from_min_size(pos2(at.x + FLYOUT_GAP, at.y - TOOL_CELL / 2.0), vec2(FLYOUT_W, TOOL_CELL));
    let cresp = ui.interact(cr, ui.scope_id().with(("flyout", g.key)), Sense::click());
    let open = app.ui.tool_flyout == Some(g.key);
    a11y::menu_button(&cresp, tr(g.name), open);
    if open {
        a11y::popup(&mut app.ui.nav, cresp.id, flyout_id(g.key));
    }
    if cresp.hovered() {
        ui.painter().rect_filled(cr, CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(cr.center(), vec2(CARET, CARET)), Icon::Chevron, t.icon_secondary);
    if cresp.clicked() {
        app.ui.tool_flyout = if open { None } else { Some(g.key) };
    }
}

/// The Actions palette's button, from `x`. Where the next part starts.
fn actions_button(app: &mut FigApp, ui: &mut Ui, x: f32, cy: f32, t: &Tokens) -> f32 {
    let r = Rect::from_center_size(pos2(x + TOOL_CELL / 2.0, cy), vec2(TOOL_CELL, TOOL_CELL));
    let resp = ui.interact(r, ui.scope_id().with("actions"), Sense::click()).tip(crate::shortcuts::tip(tr("Actions"), "app.actions"));
    app.ui.actions_rect = Some(r);
    // The Actions button reads on while its panel is open; clicking it again closes it.
    let open = app.ui.palette.is_some();
    if open {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.accent);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), glyph_rect(r), Icon::Actions, if open { t.accent_on } else { t.icon });
    if resp.clicked() {
        crate::menus::toggle_palette(&mut app.ui);
    }
    x + TOOL_CELL + GAP
}

/// The mode switcher from `x`: Draw · Design · Prototype · Inspect (code).
fn mode_switcher(app: &mut FigApp, ui: &mut Ui, x: f32, cy: f32, t: &Tokens) {
    let modes = [(Icon::Pencil, tr("Draw")), (Icon::Frame, tr("Design")), (Icon::Prototype, tr("Prototype")), (Icon::Code, tr("Inspect"))];
    let active = match app.ui.right_tab {
        crate::RightTab::Design if app.session.tool.tool == ToolId::Pencil => 0,
        crate::RightTab::Design => 1,
        crate::RightTab::Prototype => 2,
        crate::RightTab::Inspect => 3,
    };
    let seg = Rect::from_min_size(pos2(x, cy - TOOL_CELL / 2.0), vec2(4.0 * TOOL_CELL, TOOL_CELL));
    ui.painter().rect_filled(seg, CornerRadius::same(5), t.field);
    for (i, (icon, tip)) in modes.iter().enumerate() {
        let r = Rect::from_min_size(pos2(seg.min.x + i as f32 * TOOL_CELL, seg.min.y), vec2(TOOL_CELL, TOOL_CELL));
        let resp = ui.interact(r, ui.scope_id().with(("mode", i)), Sense::click()).on_hover_text(*tip);
        a11y::toggle(&resp, tip, i == active);
        if i == active {
            // The current mode lifts with a plain inset, no outline.
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(3), t.panel);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(r.center(), vec2(MODE_GLYPH, MODE_GLYPH)),
            *icon,
            if i == active { t.accent_text } else { t.icon_secondary },
        );
        if resp.clicked() {
            pick_mode(app, i);
        }
    }
}

/// Mode `i` of the switcher chosen.
fn pick_mode(app: &mut FigApp, i: usize) {
    match i {
        0 => {
            app.ui.right_tab = crate::RightTab::Design;
            app.run("tool.set", json!({"tool": "pencil"}));
        }
        1 => {
            app.ui.right_tab = crate::RightTab::Design;
            if app.session.tool.tool == ToolId::Pencil {
                app.run("tool.set", json!({"tool": "move"}));
            }
        }
        2 => app.ui.right_tab = crate::RightTab::Prototype,
        _ => app.ui.right_tab = crate::RightTab::Inspect,
    }
}

/// The floating help button at the bottom right of the canvas.
fn help_button(app: &mut FigApp, ui: &mut Ui, canvas: Rect, pill: Rect, t: &Tokens) {
    let mut hr = Rect::from_center_size(pos2(canvas.max.x - 32.0, canvas.max.y - 32.0), vec2(32.0, 32.0));
    // Above the pill when they'd overlap (a narrow window).
    if hr.intersects(pill) {
        hr = hr.translate(vec2(0.0, -(pill.height() + 12.0)));
    }
    let hresp = ui.interact(hr, ui.scope_id().with("help"), Sense::click()).tip(tr("Help and resources"));
    ui.painter().circle_filled(hr.center() + vec2(0.0, 1.0), 17.0, Color32::from_black_alpha(20));
    ui.painter().circle_filled(hr.center(), 16.0, t.panel);
    ui.painter().text(hr.center(), Align2::CENTER_CENTER, "?", crate::theme::regular(15.0), t.text);
    crate::right::menu_popup(&hresp).show(|ui| {
        for (l, c) in [
            (tr("Getting started"), "help.gettingStarted"),
            (tr("Keyboard shortcuts"), "help.shortcuts"),
            (tr("Help page"), "help.page"),
            (tr("Check for updates…"), "help.checkForUpdates"),
            (tr("About FigCraft"), "help.about"),
        ] {
            if crate::right::menu_item(ui, l, crate::shortcuts::shortcut(c).unwrap_or("")) {
                crate::menus::invoke(app, c);
                ui.close();
            }
        }
    });
}

/// The pill shown for a file you can't edit now: Move and Hand (and Comment with a comment link),
/// and a badge saying why ("View only", "Can comment", or "Publishing…" until the file is on the
/// server).
fn view_only_toolbar(app: &mut FigApp, ui: &mut Ui, why: &'static str) {
    let t = Tokens::get(ui.ctx());
    // The editing tools are off: the tool falls back to moving around.
    if !figcraft_engine::live::tool_left(why, app.session.tool.tool) {
        app.run("tool.set", json!({"tool": "move"}));
    }
    let comment = why == figcraft_engine::live::COMMENT_ONLY;
    let moving = [(ToolId::Move, Icon::Move), (ToolId::Hand, Icon::Hand), (ToolId::Comment, Icon::Comment)];
    let tools = if comment { &moving[..] } else { &moving[..2] };
    let (label, badge_icon) = match why {
        figcraft_engine::live::PUBLISHING => (tr("Publishing…"), Icon::Eye),
        _ if comment => (tr("Can comment"), Icon::Comment),
        _ => (tr("View only"), Icon::Eye),
    };
    let text = ui.painter().layout_no_wrap(label.to_string(), crate::theme::semibold(11.5), t.text);
    let badge_w = 12.0 + 14.0 + 6.0 + text.size().x + 12.0;
    let width = PAD * 2.0 + tools.len() as f32 * (TOOL_CELL + GAP) + DIVIDER + GAP + badge_w;
    let rect = pill(ui, &mut app.ui.nav, app.canvas.rect, width, &t);
    let cy = rect.center().y;
    let mut x = rect.min.x + PAD;
    for &(tool, icon) in tools {
        let r = Rect::from_center_size(pos2(x + TOOL_CELL / 2.0, cy), vec2(TOOL_CELL, TOOL_CELL));
        let active = app.session.tool.tool == tool;
        let resp = tool_cell(ui, &t, r, ("tool", tool.id()), icon, active, &format!("{}  {}", tr(tool.label()), pretty_shortcut(tool.shortcut())));
        if active {
            a11y::prefer(&mut app.ui.nav, Region::Toolbar, resp.id);
        }
        if resp.clicked() {
            app.run("tool.set", json!({"tool": tool.id()}));
        }
        x += TOOL_CELL + GAP;
    }
    divider(ui, &t, x, cy);
    let badge = Rect::from_min_size(pos2(x + DIVIDER + GAP, cy - 13.0), vec2(badge_w, 26.0));
    let resp = ui.interact(badge, ui.scope_id().with("view_only"), Sense::hover()).on_hover_text(why);
    a11y::status(&resp, label, why);
    ui.painter().rect_filled(badge, CornerRadius::same(13), t.field);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(badge.min.x + 12.0 + 7.0, cy), vec2(14.0, 14.0)), badge_icon, t.icon_secondary);
    ui.painter().galley(pos2(badge.min.x + 12.0 + 14.0 + 6.0, cy - text.size().y / 2.0), text, t.text);
}

/// The pill shown in vector edit mode: the edit tools and Done.
fn edit_toolbar(app: &mut FigApp, ui: &mut Ui, current: VTool) {
    let t = Tokens::get(ui.ctx());
    let canvas = app.canvas.rect;
    let width = VTool::ALL.len() as f32 * (TOOL_CELL + GAP) + PAD * 2.0 + DIVIDER + GAP + DONE_W;
    let rect = pill(ui, &mut app.ui.nav, canvas, width, &t);
    let cy = rect.center().y;
    let mut x = rect.min.x + PAD;
    // Holding Cmd with the Move tool bends: show it.
    let held = current == VTool::Move && ui.input(|i| i.modifiers.command);
    for tool in VTool::ALL {
        let r = Rect::from_center_size(pos2(x + TOOL_CELL / 2.0, cy), vec2(TOOL_CELL, TOOL_CELL));
        let tip = if tool == VTool::Bend {
            trf("{tool}  hold {key}", &[("tool", &tr(tool.label())), ("key", &pretty_shortcut(tool.shortcut()))])
        } else {
            format!("{}  {}", tr(tool.label()), pretty_shortcut(tool.shortcut()))
        };
        let active = tool == current && !held || held && tool == VTool::Bend;
        let icon = match tool {
            VTool::Move => Icon::Move,
            VTool::Pen => Icon::Pen,
            VTool::Bend => Icon::Bend,
            VTool::Paint => Icon::PaintBucket,
            VTool::Lasso => Icon::Lasso,
            VTool::Cut => Icon::Cut,
            VTool::Eraser => Icon::Eraser,
            VTool::Width => Icon::VariableWidth,
        };
        let resp = tool_cell(ui, &t, r, ("vtool", tool.id()), icon, active, &tip);
        if active {
            a11y::prefer(&mut app.ui.nav, Region::Toolbar, resp.id);
        }
        if resp.clicked() {
            app.run("vector.edit.tool", json!({"tool": tool.id()}));
        }
        x += TOOL_CELL + GAP;
    }
    if done_button(ui, &t, x, cy, &format!("{}  Esc", tr("Done editing"))) {
        app.run("vector.edit.end", json!({}));
    }
}

/// The pill shown while the shape builder is active: Merge and Remove modes, a hint and Done.
fn builder_toolbar(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let remove = app.session.tool.builder_remove;
    let has_regions = figcraft_engine::shapebuilder::current(&app.session).is_some_and(|a| !a.regions.is_empty());
    let alt = pretty_shortcut("Alt");
    let hint = if !has_regions {
        tr("Select overlapping shapes").to_string()
    } else if remove {
        trf("Click or drag to remove · {alt} merges", &[("alt", &alt)])
    } else {
        trf("Click or drag to merge · {alt} removes", &[("alt", &alt)])
    };
    let galley = ui.painter().layout_no_wrap(hint, regular(12.0), t.text_secondary);
    let width = PAD * 2.0 + 2.0 * (TOOL_CELL + GAP) + 2.0 * (DIVIDER + GAP) + galley.size().x + GAP + DONE_W;
    let rect = pill(ui, &mut app.ui.nav, app.canvas.rect, width, &t);
    let cy = rect.center().y;
    let mut x = rect.min.x + PAD;
    for (mode, icon, tip) in [("merge", Icon::ShapeBuilder, tr("Merge regions")), ("remove", Icon::ShapeBuilderRemove, tr("Remove regions"))] {
        let r = Rect::from_center_size(pos2(x + TOOL_CELL / 2.0, cy), vec2(TOOL_CELL, TOOL_CELL));
        let active = (mode == "remove") == remove;
        let resp = tool_cell(ui, &t, r, ("sbmode", mode), icon, active, tip);
        if active {
            a11y::prefer(&mut app.ui.nav, Region::Toolbar, resp.id);
        }
        if resp.clicked() {
            app.run("shapeBuilder.mode", json!({"mode": mode}));
        }
        x += TOOL_CELL + GAP;
    }
    divider(ui, &t, x, cy);
    x += DIVIDER + GAP;
    let w = galley.size().x;
    ui.painter().galley(pos2(x, cy - galley.size().y / 2.0), galley, t.text_secondary);
    x += w + GAP;
    if done_button(ui, &t, x, cy, &format!("{}  {}", tr("Done"), pretty_shortcut("Enter"))) {
        app.run("tool.set", json!({"tool": "move"}));
    }
}

/// Where a toolbar pill `width` wide sits: centred 12 px above the canvas's bottom edge.
/// (Never wider than the canvas less a margin: on a phone the main toolbar scrolls inside it.)
pub(crate) fn pill_rect(canvas: Rect, width: f32) -> Rect {
    let width = width.min((canvas.width() - 16.0).max(0.0));
    Rect::from_center_size(pos2(canvas.center().x, canvas.max.y - 12.0 - 24.0), vec2(width, 48.0))
}

/// The floating pill (shadow, panel, border) `width` wide at the bottom centre of the canvas: the
/// toolbar region keyboard navigation moves through.
fn pill(ui: &Ui, nav: &mut Nav, canvas: Rect, width: f32, t: &Tokens) -> Rect {
    let rect = pill_rect(canvas, width);
    // The pill takes the press and the drag over it: its buttons sense clicks only, and egui gives
    // a drag the canvas under them would sense to the canvas, so the pen placed an anchor there.
    // Not focusable: the keyboard walks the buttons, not the pill.
    ui.interact(rect, ui.scope_id().with("pill"), Sense::CLICK | Sense::DRAG);
    a11y::area(nav, Region::Toolbar, rect);
    a11y::container_bounds(ui, rect);
    let painter = ui.painter();
    for (i, a) in [(6.0, 10u8), (3.0, 16), (1.0, 24)] {
        painter.rect_filled(rect.expand(i).translate(vec2(0.0, i * 0.5)), CornerRadius::same((13.0 + i) as u8), Color32::from_black_alpha(a));
    }
    painter.rect_filled(rect, CornerRadius::same(13), t.panel);
    painter.rect_stroke(rect, CornerRadius::same(13), Stroke::new(1.0, t.border), StrokeKind::Inside);
    rect
}

/// A square tool button: accent when active, tinted on hover; `tip` is its tooltip and name.
pub(crate) fn tool_cell(ui: &mut Ui, t: &Tokens, r: Rect, id: impl egui::AsIdSalt, icon: Icon, active: bool, tip: &str) -> egui::Response {
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click()).on_hover_ui(|ui| {
        ui.label(tip);
    });
    a11y::toggle(&resp, tip, active);
    let bg = if active {
        t.accent
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, CornerRadius::same(5), bg);
    icons::paint(ui.painter(), glyph_rect(r), icon, if active { t.accent_on } else { t.icon });
    resp
}

/// A vertical separator just after `x`: DIVIDER wide, as tall as the pill's content.
fn divider(ui: &Ui, t: &Tokens, x: f32, cy: f32) {
    let half = TOOL_CELL / 2.0;
    ui.painter().line_segment([pos2(x + DIVIDER / 2.0, cy - half), pos2(x + DIVIDER / 2.0, cy + half)], Stroke::new(DIVIDER, t.border));
}

const DONE_W: f32 = 56.0;

/// A divider at `x` and the accent Done button after it; true when clicked.
fn done_button(ui: &mut Ui, t: &Tokens, x: f32, cy: f32, tip: &str) -> bool {
    divider(ui, t, x, cy);
    let done = Rect::from_min_size(pos2(x + DIVIDER + GAP, cy - 14.0), vec2(DONE_W, 28.0));
    let resp = ui.interact(done, ui.scope_id().with("tool_done"), Sense::click()).tip(tip);
    ui.painter().rect_filled(done, CornerRadius::same(6), if resp.hovered() { t.accent_hover } else { t.accent });
    ui.painter().text(done.center(), Align2::CENTER_CENTER, tr("Done"), crate::theme::semibold(11.5), t.accent_on);
    resp.clicked()
}

/// A board tool's flyout: the colours, shapes or stamps it places, in rows of up to eight. A pick
/// chooses the tool with it.
fn options_flyout(app: &mut FigApp, ui: &mut Ui, g: &Group, bottom_left: egui::Pos2) {
    let t = Tokens::get(ui.ctx());
    let Some(&(tool, _)) = g.tools.first() else { return };
    let items = crate::board_bar::choices(g.options, &app.session.tool.board);
    let (cell, pad) = (32.0, 8.0);
    let cols = items.len().clamp(1, 8);
    let rows = items.len().div_ceil(cols);
    let size = vec2(cols as f32 * (cell + 4.0) - 4.0 + 2.0 * pad, rows as f32 * (cell + 4.0) - 4.0 + 2.0 * pad);
    let mut chosen = None;
    egui::Area::new(flyout_id(g.key)).fixed_pos(pos2(bottom_left.x, bottom_left.y - size.y)).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(13), t.menu);
        for (i, c) in items.iter().enumerate() {
            let at = vec2((i % cols) as f32, (i / cols) as f32) * (cell + 4.0);
            let r = Rect::from_min_size(rect.min + vec2(pad, pad) + at, vec2(cell, cell));
            let resp = ui.interact(r, ui.scope_id().with(("opt", g.key, i)), Sense::click()).on_hover_text(tr(c.label));
            a11y::menu_item(&resp, tr(c.label), "", Some(c.on));
            a11y::own_focus(&resp);
            if c.on || resp.hovered() || resp.has_focus() {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
            }
            crate::board_bar::paint_choice(ui.painter(), r.shrink(5.0), &c.mark, t.menu_text);
            if resp.clicked() {
                chosen = Some(c.params.clone());
            }
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.tool_flyout = None;
    }
    if let Some(params) = chosen {
        app.run(&format!("tool.{}", tool.id()), params);
        app.ui.tool_flyout = None;
    }
}

/// Draw a tool group's flyout above its button, with keyboard/hover highlighting, remembered
/// selection when a tool is clicked, and Escape to close.
fn flyout(app: &mut FigApp, ui: &mut Ui, g: &Group, bottom_left: egui::Pos2) {
    let t = Tokens::get(ui.ctx());
    let item_h = 32.0;
    let w = 220.0;
    let h = item_h * g.tools.len() as f32 + 16.0;
    let mut chosen = None;
    egui::Area::new(flyout_id(g.key)).fixed_pos(pos2(bottom_left.x, bottom_left.y - h)).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(13), t.menu);
        let mut y = rect.min.y + 8.0;
        for (tid, icon) in g.tools {
            let r = Rect::from_min_size(pos2(rect.min.x + 8.0, y), vec2(w - 16.0, item_h));
            let resp = ui.interact(r, ui.scope_id().with(("fly", tid.id())), Sense::click());
            a11y::menu_item(&resp, tr(tid.label()), tid.shortcut(), Some(*tid == app.session.tool.tool));
            // The keyboard's item is highlighted like the hovered one.
            a11y::own_focus(&resp);
            if resp.hovered() || resp.has_focus() {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
            }
            if *tid == app.session.tool.tool {
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(12.0, 12.0)), Icon::Check, t.menu_text);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 32.0, r.center().y), vec2(16.0, 16.0)), *icon, t.menu_text);
            ui.painter().text(pos2(r.min.x + 48.0, r.center().y), Align2::LEFT_CENTER, tr(tid.label()), regular(12.0), t.menu_text);
            ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, pretty_shortcut(tid.shortcut()), regular(12.0), t.menu_dim);
            if resp.clicked() {
                chosen = Some(*tid);
            }
            y += item_h;
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.tool_flyout = None;
    }
    if let Some(tid) = chosen {
        if let Some(i) = ToolId::ALL.iter().position(|x| *x == tid) {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique(("tb_last", g.key)), i as u8));
        }
        choose(app, tid);
        app.ui.tool_flyout = None;
    }
}

/// Choose `tool` from the toolbar. Dev Mode only inspects: a tool that edits leaves it for the
/// design.
fn choose(app: &mut FigApp, tool: ToolId) {
    if app.ui.right_tab == crate::RightTab::Inspect && !matches!(tool, ToolId::Move | ToolId::Hand | ToolId::Comment) {
        app.ui.right_tab = crate::RightTab::Design;
    }
    app.run("tool.set", json!({"tool": tool.id()}));
}

/// A tool group's flyout (an egui area).
fn flyout_id(key: &str) -> egui::Id {
    egui::Id::unique(("flyout_area", key))
}

/// "Shift+Cmd+K" → "⇧⌘K" on macOS, "Ctrl+Shift+K" elsewhere.
pub fn pretty_shortcut(s: &str) -> String {
    figcraft_engine::keymap::display(s, crate::shortcuts::platform())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Shape;

    #[test]
    fn pill_never_outgrows_a_phone_canvas() {
        let full = main_width(GROUPS, false);
        let phone = Rect::from_min_size(pos2(0.0, 0.0), vec2(375.0, 700.0));
        let pill = pill_rect(phone, full);
        assert!(pill.width() <= phone.width() - 16.0 && pill.min.x >= 8.0 && pill.max.x <= phone.max.x - 8.0, "{pill:?}");
        // Narrower than its items, so it scrolls under a drag or the wheel (#1209).
        assert!(full > pill.width(), "{full} in {}", pill.width());
        // A desktop canvas keeps the full width, and a degenerate one doesn't go negative.
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0));
        assert_eq!(pill_rect(wide, full).width(), full);
        assert_eq!(pill_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(4.0, 4.0)), full).width(), 0.0);
    }

    /// The width the pill asks for is the width its items take — the groups, Actions, the divider
    /// and the end (the mode switcher, or a board's two buttons), PAD in from either end and one
    /// GAP apart — so its last item ends exactly PAD from its right edge.
    #[test]
    fn the_pill_is_as_wide_as_its_items() {
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 800.0));
        for (groups, board) in [(GROUPS, false), (BOARD_GROUPS, true)] {
            let pill = pill_rect(canvas, main_width(groups, board));
            let mut x = pill.min.x + PAD;
            for g in groups {
                x += TOOL_CELL + if g.has_flyout() { FLYOUT_GAP + FLYOUT_W } else { 0.0 } + GAP;
            }
            x += TOOL_CELL + GAP; // Actions
            x += DIVIDER + GAP; // the divider
            let end = if board { 2.0 * TOOL_CELL + GAP } else { 4.0 * TOOL_CELL };
            assert!((x + end - (pill.max.x - PAD)).abs() < 0.01, "board {board}: {} vs {}", x + end, pill.max.x - PAD);
        }
    }

    /// The stroke widths `icon` paints with in a `size` box at `ppp` pixels per point.
    fn widths(ppp: f32, icon: Icon, size: f32) -> Vec<f32> {
        let ctx = egui::Context::default();
        let input = || {
            let mut i = egui::RawInput::default();
            i.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            i
        };
        ctx.run_ui(input(), |_| {}).textures_delta.clear();
        let mut out =
            ctx.run_ui(input(), |ui| icons::paint(ui.painter(), Rect::from_center_size(pos2(50.0, 50.0), vec2(size, size)), icon, Color32::WHITE));
        out.textures_delta.clear();
        assert_eq!(ctx.pixels_per_point(), ppp);
        let mut w: Vec<f32> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::LineSegment { stroke, .. } => Some(stroke.width),
                Shape::Path(p) => Some(p.stroke.width),
                Shape::Rect(r) => Some(r.stroke.width),
                _ => None,
            })
            .filter(|w| *w > 0.0)
            .collect();
        w.dedup();
        w
    }

    /// The flyout carets and the marks beside them draw with one stroke width (#737), and the
    /// mode switcher's marks are the tool marks' size (#738).
    #[test]
    fn toolbar_marks_share_one_stroke_width() {
        for ppp in [1.0, 1.5, 2.0] {
            let mark = widths(ppp, Icon::Rectangle, GLYPH);
            assert_eq!(mark.len(), 1, "one width at {ppp}×: {mark:?}");
            assert_eq!(widths(ppp, Icon::Chevron, CARET), mark, "caret at {ppp}×");
            assert_eq!(widths(ppp, Icon::Code, GLYPH), mark, "mode mark at {ppp}×");
            for icon in [Icon::Frame, Icon::Text] {
                assert_eq!(widths(ppp, icon, GLYPH), mark, "{icon:?} at {ppp}×");
            }
        }
    }
}
