//! The Design panel's layout section: auto layout direction and wrap, W/H with their sizing (Fixed /
//! Hug / Fill) and min/max limits, alignment (including text baselines), fixed or auto gaps,
//! padding, strokes in layout and canvas stacking, grid tracks, a grid child's cell, spans and
//! alignment in its cell, and clip content. Everything goes through `node.set` props or `layout.*`
//! commands.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Ui, pos2, vec2};
use figcraft_doc::*;
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::right::{block, common, menu_item, menu_popup, row};
use crate::theme::{Tokens, regular};
use crate::widgets::{self, FIELD_H, Lead};

/// Most tracks listed in the panel (the rest are still laid out; edit them through commands).
const MAX_TRACK_ROWS: usize = 24;

fn parent_of(doc: &Document, id: NodeId) -> Option<&Node> {
    doc.node(id).and_then(|n| n.parent).and_then(|p| doc.node(p))
}

pub fn layout_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let frames = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like()));
    let al = common(doc, sel, |n| n.layout.clone());
    let has_al = matches!(al, Some(Some(_)));
    let title = if has_al { tr("Auto layout") } else { tr("Layout") };
    // An instance's auto layout is its main's (only padding, gap and alignment are its own); the
    // layers inside it override their size and resizing, not their place in the layout.
    let in_instance = sel.iter().any(|i| figcraft_engine::components::owning_instance(doc, *i).is_some());
    let nested = sel.iter().any(|i| figcraft_engine::components::inside_instance(doc, *i));
    let btns: Vec<(Icon, &str)> = if in_instance {
        vec![]
    } else if frames && !has_al {
        vec![(Icon::Plus, crate::shortcuts::tip(tr("Add auto layout"), "layout.add"))]
    } else if has_al {
        vec![(Icon::Minus, crate::shortcuts::tip(tr("Remove auto layout"), "layout.remove"))]
    } else {
        vec![]
    };
    if widgets::section_header(ui, title, &btns, false).is_some() {
        app.run(if has_al { "layout.remove" } else { "layout.add" }, json!({"ids": ids}));
    }
    let from_main = tr("In an instance, this comes from the main component");
    if let Some(Some(l)) = &al {
        // The direction switch, like the position fields below, carries a small caption above it.
        widgets::sublabels(ui, tr("Layout"), None);
        ui.add_enabled_ui(!in_instance, |ui| direction(app, ui, &ids, l)).response.on_disabled_hover_text(from_main);
    }
    dimensions(app, ui, doc, sel, has_al, nested, from_main);
    if let Some(Some(l)) = &al {
        alignment(app, ui, &ids, l);
        spacing(app, ui, &ids, l);
        ui.add_enabled_ui(!in_instance, |ui| {
            if l.mode == LayoutMode::Grid && sel.len() == 1 {
                tracks(app, ui, doc, &ids);
            }
        })
        .response
        .on_disabled_hover_text(from_main);
    }
    crate::selection_spacing::panel(app, ui, sel);
    let grid_parent = sel.iter().all(|i| parent_of(doc, *i).and_then(|p| p.layout.as_ref()).is_some_and(|l| l.mode == LayoutMode::Grid));
    if grid_parent && common(doc, sel, |n| n.child_layout.absolute) == Some(false) {
        ui.add_enabled_ui(!nested, |ui| grid_cell(app, ui, doc, sel)).response.on_disabled_hover_text(from_main);
    }
    if frames {
        let clip = common(doc, sel, |n| n.clip);
        row(ui, 32.0, |ui| {
            if widgets::checkbox(ui, clip.unwrap_or(false), tr("Clip content")) {
                app.set_props(&ids, json!({"clip": !clip.unwrap_or(false)}));
            }
        });
    }
}

fn direction(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout) {
    row(ui, 32.0, |ui| {
        let items = [(Icon::LayoutV, tr("Vertical")), (Icon::LayoutH, tr("Horizontal")), (Icon::LayoutGrid, tr("Grid"))];
        let active = match l.mode {
            LayoutMode::Vertical => 0,
            LayoutMode::Horizontal => 1,
            LayoutMode::Grid => 2,
        };
        // The switch fills the row: its three cells share what the wrap button (or, for a grid,
        // its placeholder) leaves, so the last cell and that button end at the panel's margin and
        // the switch keeps the same width in every mode.
        let wrap = 24.0 + widgets::GUTTER;
        let cell = (ui.available_width() - wrap) / items.len() as f32;
        if let Some(i) = widgets::segmented(ui, &items, Some(active), cell) {
            // Wrap carries over between the flows: rows of a horizontal one, columns of a vertical one.
            let mode = ["vertical", "horizontal", "grid"].get(i).copied().unwrap_or("grid");
            app.set_props(ids, json!({"layout": mode}));
        }
        if l.mode != LayoutMode::Grid {
            let resp = icons::button(ui, Icon::LayoutWrap, 24.0, l.wrap, tr("Wrap"));
            crate::a11y::toggle(&resp, tr("Wrap"), l.wrap);
            if resp.clicked() {
                app.set_props(ids, json!({"wrap": !l.wrap}));
            }
        } else {
            // A grid never wraps: keep the wrap button's slot as a dimmed placeholder so the
            // switch doesn't stretch to the panel's edge.
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            let t = Tokens::get(ui.ctx());
            icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), Icon::LayoutWrap, t.icon_secondary);
        }
    });
}

/// One axis' sizing, min/max keys and labels (the add-min and add-max menu items, the remove
/// button, the min and max field leads).
struct Axis {
    sizing: &'static str,
    min: &'static str,
    max: &'static str,
    add_min: &'static str,
    add_max: &'static str,
    remove: &'static str,
    /// The hint in a limit field with no value yet ("Min W").
    min_empty: &'static str,
    max_empty: &'static str,
    /// The caption above a limit row ("Min width").
    min_label: &'static str,
    max_label: &'static str,
}

const AXES: [Axis; 2] = [
    Axis {
        sizing: "sizingH",
        min: "minWidth",
        max: "maxWidth",
        add_min: tr_noop("Add min width"),
        add_max: tr_noop("Add max width"),
        remove: tr_noop("Remove min and max width"),
        min_empty: tr_noop("Min W"),
        max_empty: tr_noop("Max W"),
        min_label: tr_noop("Min width"),
        max_label: tr_noop("Max width"),
    },
    Axis {
        sizing: "sizingV",
        min: "minHeight",
        max: "maxHeight",
        add_min: tr_noop("Add min height"),
        add_max: tr_noop("Add max height"),
        remove: tr_noop("Remove min and max height"),
        min_empty: tr_noop("Min H"),
        max_empty: tr_noop("Max H"),
        min_label: tr_noop("Min height"),
        max_label: tr_noop("Max height"),
    },
];

/// A node's min (`is_min`) or max limit on one axis.
fn limit(n: &Node, horizontal: bool, is_min: bool) -> Option<f64> {
    let cl = &n.child_layout;
    match (horizontal, is_min) {
        (true, true) => cl.min_w,
        (true, false) => cl.max_w,
        (false, true) => cl.min_h,
        (false, false) => cl.max_h,
    }
}

/// `nested`: layers inside an instance, whose proportion lock is their main's (`from_main` says so).
fn dimensions(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], has_al: bool, nested: bool, from_main: &str) {
    let ids = sel.to_vec();
    let w_of = |n: &Node| (n.width * 100.0).round() / 100.0;
    let h_of = |n: &Node| (n.height * 100.0).round() / 100.0;
    let (w, h) = (common(doc, sel, w_of), common(doc, sel, h_of));
    let col = widgets::col_width(ui.available_width());
    widgets::sublabels(ui, tr("Dimensions"), None);
    let parent_al = sel.iter().all(|i| parent_of(doc, *i).is_some_and(|p| p.layout.is_some()));
    let is_text = sel.iter().all(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Text(_))));
    let can_fill = parent_al && sel.iter().all(|i| doc.node(*i).is_some_and(|n| !n.child_layout.absolute));
    let can_limit = has_al || can_fill;
    // Sizing applies to an auto-layout frame, a layer inside one, and text: those get the menu.
    let in_al = has_al || parent_al || is_text;
    row(ui, 32.0, |ui| {
        dim_field(
            app,
            ui,
            doc,
            &ids,
            sel,
            "width",
            Id::new("lay_w"),
            Lead::Text("W"),
            w,
            w_of,
            col,
            true,
            in_al,
            has_al,
            can_fill,
            can_limit,
            is_text,
        );
        dim_field(
            app,
            ui,
            doc,
            &ids,
            sel,
            "height",
            Id::new("lay_h"),
            Lead::Text("H"),
            h,
            h_of,
            col,
            false,
            in_al,
            has_al,
            can_fill,
            can_limit,
            is_text,
        );
        let locked = common(doc, sel, |n| n.constrain_proportions).unwrap_or(false);
        let link = ui
            .add_enabled_ui(!nested, |ui| {
                icons::button(ui, if locked { Icon::LinkOn } else { Icon::LinkOff }, 24.0, locked, tr("Constrain proportions"))
            })
            .inner
            .on_disabled_hover_text(from_main);
        if link.clicked() {
            app.set_props(&ids, json!({"constrainProportions": !locked}));
        }
    });
    // Min/max sit under their own axis: the max row first, then the min row (a vertical stack).
    if can_limit {
        limit_row_v(app, ui, doc, sel, false, col);
        limit_row_v(app, ui, doc, sel, true, col);
    }
}

/// A W/H field with a trailing control: in auto layout a dropdown that opens the sizing menu
/// (fixed / hug / fill, add min & max, apply variable); otherwise an apply-variable button. A fill
/// child's value reads at 50% ink, and typing a number fixes that axis to it. The value commits
/// through `of` like a plain number field.
#[allow(clippy::too_many_arguments)]
fn dim_field(
    app: &mut FigApp,
    ui: &mut Ui,
    doc: &Document,
    ids: &[NodeId],
    sel: &[NodeId],
    field: &str,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    of: impl Fn(&Node) -> f64,
    col: f32,
    horizontal: bool,
    in_al: bool,
    has_al: bool,
    can_fill: bool,
    can_limit: bool,
    is_text: bool,
) {
    let first = sel.first().and_then(|i| doc.node(*i));
    let axis = &AXES[if horizontal { 0 } else { 1 }];
    let cur = first.map(|n| if horizontal { n.child_layout.sizing_h } else { n.child_layout.sizing_v });
    // A fill's or a hug's size is the layout's, not its own: it reads at 50% ink and its word takes
    // the chevron's place, and typing a number fixes that axis to it.
    let dim = in_al && matches!(cur, Some(Sizing::Fill) | Some(Sizing::Hug));
    let word = match cur {
        Some(Sizing::Fill) => Some(tr("Fill|sizing")),
        Some(Sizing::Hug) => Some(tr("Hug")),
        _ => None,
    };
    // The field's own right-end button is the sizing menu in auto layout (a chevron, or that word
    // while it shows), and the variable button everywhere else.
    let (e, menu) = crate::variable_ui::number_field_mixed_menu(app, ui, ids, field, id, lead, value, col, dim, in_al.then_some(word));
    if dim && matches!(&e, Some(crate::widgets::Edit::All(crate::widgets::FieldEdit::Set(_)))) {
        // A typed size on a fill or hug child: the value and the fixed sizing in one undo step.
        if let Some(crate::widgets::Edit::All(crate::widgets::FieldEdit::Set(v))) = e {
            app.set_props(ids, json!({field: v.max(0.0), axis.sizing: "fixed"}));
        }
    } else {
        let ended = matches!(&e, Some(crate::widgets::Edit::All(crate::widgets::FieldEdit::Scrub(_, true))));
        app.field_of(ids, field, e, of, |v| json!(v.max(0.0)));
        if dim && ended {
            app.set_props(ids, json!({axis.sizing: "fixed"}));
        }
    }
    if let Some(resp) = menu {
        let ids2 = ids.to_vec();
        let ctx = ui.ctx().clone();
        let open = |is_min: bool| first.is_some_and(|n| limit(n, horizontal, is_min).is_some() || limit_open(&ctx, n.id, horizontal, is_min));
        let (has_min, has_max) = (open(true), open(false));
        menu_popup(&resp).width(200.0).show(|ui| {
            let mut opts = vec![(tr("Fixed"), "fixed")];
            if has_al || is_text {
                opts.push((tr("Hug contents"), "hug"));
            }
            if can_fill {
                opts.push((tr("Fill container"), "fill"));
            }
            for (l, v) in opts {
                if menu_item(ui, l, "") {
                    app.set_props(&ids2, json!({axis.sizing: v}));
                    ui.close();
                }
            }
            if can_limit {
                ui.separator();
                if !has_min && menu_item(ui, tr(axis.add_min), "") {
                    // The row opens with no value: the field's hint names it until a number is typed.
                    for id in &ids2 {
                        ui.ctx().data_mut(|d| d.insert_temp(limit_open_id(*id, horizontal, true), true));
                    }
                    ui.close();
                }
                if !has_max && menu_item(ui, tr(axis.add_max), "") {
                    for id in &ids2 {
                        ui.ctx().data_mut(|d| d.insert_temp(limit_open_id(*id, horizontal, false), true));
                    }
                    ui.close();
                }
                if (has_min || has_max) && menu_item(ui, tr(axis.remove), "") {
                    app.set_props(&ids2, json!({axis.min: null, axis.max: null}));
                    for id in &ids2 {
                        ui.ctx().data_mut(|d| {
                            d.remove::<bool>(limit_open_id(*id, horizontal, true));
                            d.remove::<bool>(limit_open_id(*id, horizontal, false));
                        });
                    }
                    ui.close();
                }
            }
            // The last item is the same variable picker the field's own button opens.
            ui.separator();
            if menu_item(ui, tr("Apply variable"), "") {
                crate::variable_ui::open_at_pointer(ui, &ids2, field, VariableType::Number);
                ui.close();
            }
        });
    }
}

/// The id of the UI's "this limit row is open" flag: a row the user added from the menu stays open
/// (and empty) until a number is typed, so its being shown is the UI's own, not the document's.
fn limit_open_id(id: NodeId, horizontal: bool, is_min: bool) -> Id {
    Id::new(("limit_open", id, horizontal, is_min))
}

fn limit_open(ctx: &egui::Context, id: NodeId, horizontal: bool, is_min: bool) -> bool {
    ctx.data(|d| d.get_temp::<bool>(limit_open_id(id, horizontal, is_min))).unwrap_or(false)
}

/// One min or max row for both axes (shown once either is set or opened): each shown limit's field
/// sits in its own column, under its W/H, with a caption above saying which limit it holds; an
/// unshown one leaves its column empty (added from the dropdown).
fn limit_row_v(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], is_min: bool, col: f32) {
    let nodes: Vec<&Node> = sel.iter().filter_map(|i| doc.node(*i)).collect();
    let ctx = ui.ctx().clone();
    let has = |horizontal: bool| nodes.iter().any(|n| limit(n, horizontal, is_min).is_some() || limit_open(&ctx, n.id, horizontal, is_min));
    if !has(true) && !has(false) {
        return;
    }
    // The caption names the limit under it, over that axis' column.
    let (a, b) = if is_min { (AXES[0].min_label, AXES[1].min_label) } else { (AXES[0].max_label, AXES[1].max_label) };
    widgets::sublabels(ui, tr(a), Some(tr(b)));
    let ids = sel.to_vec();
    // "m" for a minimum, "M" for a maximum, in front of the number.
    let lead = if is_min { "m" } else { "M" };
    row(ui, 32.0, |ui| {
        for (ax, axis) in AXES.iter().enumerate() {
            let horizontal = ax == 0;
            let (key, empty) = if is_min { (axis.min, axis.min_empty) } else { (axis.max, axis.max_empty) };
            if has(horizontal) {
                let v = common(doc, sel, |n| limit(n, horizontal, is_min).map(|v| (v * 100.0).round() / 100.0)).flatten();
                let e =
                    crate::variable_ui::number_field_or(app, ui, &ids, key, Id::new(("lay_limit_v", key)), v, col, tr(empty), Lead::Text(lead), "");
                app.field(&ids, key, e, |v| json!(v.max(0.0)));
            } else {
                let _ = ui.allocate_exact_size(vec2(col, FIELD_H), Sense::hover());
            }
        }
    });
}

fn alignment(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout) {
    // A grid and a wrapping flow space two axes, so they show two gaps; a plain flow shows one.
    // The picker and the gaps get a column each — half the row — and the dots button sits at the
    // row's end.
    let two_gaps = l.mode == LayoutMode::Grid || l.wrap;
    // The picker spans the two fields exactly, so its top and bottom line up with theirs.
    let h = FIELD_H * 2.0 + widgets::GUTTER;
    // The picker reads "Align" for a flow and "Grid" for a grid; the gaps beside it are "Spacing".
    widgets::sublabels(ui, if l.mode == LayoutMode::Grid { tr("Grid") } else { tr("Align") }, Some(tr("Spacing")));
    block(ui, h + 12.0, 6.0, |ui| {
        let col = ((ui.available_width() - 24.0 - widgets::GUTTER * 2.0) / 2.0).max(60.0);
        let pill = ui.allocate_exact_size(vec2(col, h), Sense::hover()).0;
        picker(app, ui, ids, l, pill);
        let gaps = ui.allocate_exact_size(vec2(col, h), Sense::hover()).0;
        let mut column = ui.new_child(egui::UiBuilder::new().max_rect(gaps).layout(egui::Layout::top_down(egui::Align::Min)));
        column.spacing_mut().item_spacing = vec2(0.0, widgets::GUTTER);
        if two_gaps {
            gap_fields(app, &mut column, ids, l, col);
        } else {
            gap_and_mode(app, &mut column, ids, l, col);
        }
        let end = ui.allocate_exact_size(vec2(24.0, h), Sense::hover()).0;
        // The advanced button's top lines up with the top gap field's, not the row's centre.
        let mut button = ui.new_child(
            egui::UiBuilder::new().max_rect(Rect::from_min_size(end.min, vec2(24.0, 24.0))).layout(egui::Layout::top_down(egui::Align::Min)),
        );
        settings_toggle(app, &mut button);
    });
}

/// The alignment picker: nine dots on the field colour, the chosen cell its glyph in the accent
/// blue with no well behind it, and the hovered cell the same glyph in the dots' grey — a preview
/// of what a click there picks. The glyph is three horizontal bars for a horizontal flow (the main
/// axis along the columns) and three vertical bars for a vertical one (the main axis along the
/// rows): the bars pack along the main axis to read the main alignment, and the two outer bars'
/// lengths read the cross one. An auto gap lights the whole main axis: the middle cell keeps the
/// glyph and the ends keep a blue dot.
fn picker(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout, pill: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(pill, CornerRadius::same(5), t.field);
    // A grid shows the grid itself, not the alignment dots.
    if l.mode == LayoutMode::Grid {
        grid_preview(ui, l, pill, &t);
        return;
    }
    // The grid is five sixths of its well's height, a twelfth in from the top and the
    // bottom and as wide as the well; the well here spans the two gap fields, so the grid keeps
    // that share of its height.
    let inset = pill.height() / 12.0;
    let grid = Rect::from_min_max(pos2(pill.min.x, pill.min.y + inset), pos2(pill.max.x, pill.max.y - inset));
    let cell = vec2(grid.width() / 3.0, grid.height() / 3.0);
    let grid_mode = l.mode == LayoutMode::Grid;
    let horizontal = l.mode != LayoutMode::Vertical;
    let cur_main = match l.main_align {
        MainAlign::Min => 0,
        MainAlign::Center => 1,
        MainAlign::Max => 2,
        MainAlign::SpaceBetween | MainAlign::SpaceAround | MainAlign::SpaceEvenly => 9,
    };
    // Wrapped lines spread across the flow light the whole cross axis, like an auto gap does the
    // main one.
    let spread = !grid_mode && l.wrap && l.content_align == ContentAlign::SpaceBetween;
    let cur_cross = match l.cross_align {
        _ if spread => 9,
        CrossAlign::Min | CrossAlign::Baseline => 0,
        CrossAlign::Center => 1,
        CrossAlign::Max => 2,
    };
    // The dimmed grey: pure black at 50 % on the light theme's field, pure white at 50 % on the
    // dark one's, where black would disappear into the well.
    let grey = if t.dark { Color32::from_white_alpha(128) } else { Color32::from_black_alpha(128) };
    for yi in 0..3 {
        for xi in 0..3 {
            let r = Rect::from_min_size(pos2(grid.min.x + xi as f32 * cell.x, grid.min.y + yi as f32 * cell.y), cell);
            let c = r.center();
            let resp = ui.interact(r, Id::new(("al_grid", xi, yi)), Sense::click());
            let (main_i, cross_i) = axes(xi, yi, horizontal);
            let on = lit(main_i, cross_i, cur_main, cur_cross);
            // A spread axis keeps one glyph, in its middle cell, and marks the ends in blue.
            if on && (cur_main != 9 || main_i == 1) {
                glyph(ui.painter(), c, horizontal, cur_main, cur_cross, t.accent_text);
            } else if on {
                ui.painter().circle_filled(c, 1.0, t.accent_text);
            } else if resp.hovered() {
                glyph(ui.painter(), c, horizontal, main_i, cross_i, grey);
            } else {
                ui.painter().circle_filled(c, 1.0, grey);
            }
            if resp.clicked() {
                let m = ["min", "center", "max"][main_i];
                let cr = ["min", "center", "max"][cross_i];
                // An auto gap keeps its mode: the cells pick the cross alignment.
                let main = if l.main_align.is_auto_gap() && !grid_mode { json!(l.main_align) } else { json!(m) };
                app.set_props(ids, json!({"mainAlign": main, "crossAlign": cr}));
            }
        }
    }
}

/// The grid picker: the frame's grid drawn as white cells (2 pt corners) in its rows × columns, a
/// 2 pt gap between them and 3 pt in from the well, with the count ("rows × cols") centred in
/// 12 pt. The cells cap at 12 × 8 so a large grid stays legible; the count still reads the real
/// size.
fn grid_preview(ui: &Ui, l: &AutoLayout, pill: Rect, t: &Tokens) {
    let painter = ui.painter();
    let inner = pill.shrink(3.0);
    let cols = l.grid_columns.clamp(1, 12) as usize;
    let rows = l.grid_rows.clamp(1, 8) as usize;
    let gap = 2.0;
    let cw = (inner.width() - (cols - 1) as f32 * gap) / cols as f32;
    let ch = (inner.height() - (rows - 1) as f32 * gap) / rows as f32;
    for r in 0..rows {
        for c in 0..cols {
            let at = pos2(inner.min.x + c as f32 * (cw + gap), inner.min.y + r as f32 * (ch + gap));
            painter.rect_filled(Rect::from_min_size(at, vec2(cw, ch)), CornerRadius::same(2), Color32::WHITE);
        }
    }
    let label = format!("{} × {}", l.grid_columns, l.grid_rows);
    painter.text(inner.center(), Align2::CENTER_CENTER, label, regular(12.0), t.text);
}

/// A cell's `(main, cross)` indices for a flow: a horizontal one runs
/// the main axis along the columns, a vertical one along the rows.
fn axes(xi: usize, yi: usize, horizontal: bool) -> (usize, usize) {
    if horizontal { (xi, yi) } else { (yi, xi) }
}

/// Whether the cell at (`main_i`, `cross_i`) is lit: the chosen alignment, or every cell of an
/// axis that spreads — an auto gap along the main one, wrapped lines across the other.
fn lit(main_i: usize, cross_i: usize, cur_main: usize, cur_cross: usize) -> bool {
    (cur_main == main_i || cur_main == 9) && (cur_cross == cross_i || cur_cross == 9)
}

/// The glyph's box: 10 pt square.
const GLYPH_BOX: f32 = 10.0;

/// The glyph's three bars as `(x, y, w, h)` in the 10 pt box. `horizontal` picks three horizontal
/// bars (a quarter of the box apart), otherwise three vertical ones; the
/// middle bar is always full length, the two outer ones are shorter and the longer of the two
/// sits on the chosen cross side (min → the first is longer, max → the second). Where each bar
/// packs along the main axis reads the main alignment — flush at the start, sharing the slack, or
/// flush at the end. `main`/`cross` are 0/1/2 (or 9 for a spreading axis, which centres).
fn bars(horizontal: bool, main: usize, cross: usize) -> [(f32, f32, f32, f32); 3] {
    let (first, last) = match cross {
        0 => (7.0, 5.0),
        2 => (5.0, 7.0),
        _ => (5.0, 5.0),
    };
    let at = |len: f32| match main {
        0 => 0.0,
        2 => GLYPH_BOX - len,
        _ => (GLYPH_BOX - len) / 2.0,
    };
    if horizontal {
        [(at(first), 0.0, first, 2.0), (0.0, 4.0, GLYPH_BOX, 2.0), (at(last), 8.0, last, 2.0)]
    } else {
        [(0.0, at(first), 2.0, first), (4.0, 0.0, 2.0, GLYPH_BOX), (8.0, at(last), 2.0, last)]
    }
}

/// The glyph centred on `centre` in `col`: the accent blue on the chosen cell, the dots' grey on
/// a hover preview.
fn glyph(painter: &egui::Painter, centre: Pos2, horizontal: bool, main: usize, cross: usize, col: Color32) {
    let g = Rect::from_center_size(centre, vec2(GLYPH_BOX, GLYPH_BOX));
    for (x, y, w, h) in bars(horizontal, main, cross) {
        painter.rect_filled(Rect::from_min_size(pos2(g.min.x + x, g.min.y + y), vec2(w, h)), CornerRadius::same(1), col);
    }
}

/// The two gaps beside the picker, the vertical one above the horizontal one. They are named by
/// the axis rather than by main and cross, so a vertical flow's own gap is the vertical one.
fn gap_fields(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout, col: f32) {
    let (v_tip, h_tip) = (tr("Gap between rows"), tr("Gap between columns"));
    if l.mode == LayoutMode::Grid {
        let e = crate::variable_ui::number_field(
            app,
            ui,
            ids,
            "rowGap",
            Id::new("al_rowgap"),
            Lead::Icon(Icon::DistributeV, v_tip),
            Some(l.cross_gap),
            "",
            col,
        );
        app.field(ids, "rowGap", e, |v| json!(v.max(0.0)));
        let e = crate::variable_ui::number_field(app, ui, ids, "columnGap", Id::new("al_gap"), Lead::Icon(Icon::Gap, h_tip), Some(l.gap), "", col);
        app.field(ids, "columnGap", e, |v| json!(v.max(0.0)));
        return;
    }
    let vertical = l.mode == LayoutMode::Vertical;
    // An automatic gap and a spread between lines have no number to show.
    let auto_main = l.main_align.is_auto_gap();
    let spread = l.content_align == ContentAlign::SpaceBetween;
    let (v_key, v_id, v_val) =
        if vertical { ("gap", "al_gap", (!auto_main).then_some(l.gap)) } else { ("crossGap", "al_rowgap", (!spread).then_some(l.cross_gap)) };
    let (h_key, h_id, h_val) =
        if vertical { ("crossGap", "al_rowgap", (!spread).then_some(l.cross_gap)) } else { ("gap", "al_gap", (!auto_main).then_some(l.gap)) };
    for (key, id, value, icon, tip) in [(v_key, v_id, v_val, Icon::DistributeV, v_tip), (h_key, h_id, h_val, Icon::Gap, h_tip)] {
        let e = crate::variable_ui::number_field_or(app, ui, ids, key, Id::new(id), value, col, tr("Auto"), Lead::Icon(icon, tip), "");
        if key == "crossGap" {
            // A typed gap between the lines is a fixed one.
            app.field_run(e, "node.set", |v| json!({"ids": ids, "props": {"crossGap": v.max(0.0), "contentAlign": "auto"}}));
        } else {
            app.field(ids, key, e, |v| json!(v));
        }
    }
}

/// A plain flow's single gap and its mode (fixed, or an automatic spread).
fn gap_and_mode(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout, col: f32) {
    let auto = l.main_align.is_auto_gap();
    let gap_icon = if l.mode == LayoutMode::Vertical { Icon::DistributeV } else { Icon::Gap };
    let (shown, lead) = ((!auto).then_some(l.gap), Lead::Icon(gap_icon, tr("Gap between items")));
    let e = crate::variable_ui::number_field_or(app, ui, ids, "gap", Id::new("al_gap"), shown, col, tr("Auto"), lead, "");
    app.field(ids, "gap", e, |v| json!(v));
    let label = match l.main_align {
        MainAlign::SpaceBetween => tr("Space between"),
        MainAlign::SpaceAround => tr("Space around"),
        MainAlign::SpaceEvenly => tr("Space evenly"),
        _ => tr("Fixed gap"),
    };
    let resp = widgets::dropdown(ui, label, col, None);
    crate::a11y::combo(&resp, tr("Auto gap"), label, crate::a11y::menu_open(&resp));
    menu_popup(&resp).show(|ui| {
        let modes = [
            (tr("Fixed"), "min"),
            (tr("Auto (space between)"), "spaceBetween"),
            (tr("Auto (space evenly)"), "spaceEvenly"),
            (tr("Auto (space around)"), "spaceAround"),
        ];
        for (l, v) in modes {
            if menu_item(ui, l, "") {
                app.set_props(ids, json!({"mainAlign": v}));
                ui.close();
            }
        }
    });
}

/// The layout settings' dots button: toggles the popover, holding the button's place while it is
/// open. It stays enabled inside an instance — the text baselines are the instance's own to set
/// (the strokes and stacking rows inside the popover are the ones that go disabled).
fn settings_toggle(app: &mut FigApp, ui: &mut Ui) {
    let open = app.ui.layout_settings.is_some();
    let more = icons::button(ui, Icon::Dots, 24.0, open, tr("Advanced"));
    // The button's rect is live every frame: the outside-press hold follows the button when the
    // panel scrolls, so a click on it at its new place toggles the popover shut. The opening
    // anchor is taken once, on the press, so the popover keeps its place as the panel scrolls.
    app.ui.layout_settings_button = Some(more.rect);
    if more.clicked() {
        app.ui.layout_settings = match app.ui.layout_settings.take() {
            Some(_) => None,
            None => {
                app.ui.layout_settings_anchor = Some(more.rect.min.y);
                crate::colorpicker::close(app);
                Some(crate::popover::Popover::new("layout_settings", tr("Layout")).pad(12.0))
            }
        };
    }
}

/// Escape with the settings popover open closes it (the selection and the opener's place stay).
pub fn close_settings(app: &mut FigApp) -> bool {
    app.ui.layout_settings.take().is_some()
}

/// The Spacing row: the H/V padding pair, or, with Individual padding on, two rows of one field
/// per side (like Individual corners).
fn spacing(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout) {
    widgets::sublabels(ui, tr("Padding"), None);
    let col = widgets::col_width(ui.available_width());
    let [pt, pr, pb, pl] = l.padding;
    let open = ui.data(|d| d.get_temp::<bool>(Id::new("indep_padding"))).unwrap_or(false);
    let toggle = |ui: &mut Ui| {
        if icons::button(ui, Icon::IndependentPadding, 24.0, open, tr("Individual padding")).clicked() {
            ui.data_mut(|d| d.insert_temp(Id::new("indep_padding"), !open));
        }
    };
    if open {
        // `l` is the selection's common layout, so its sides are common too. `k` indexes
        // [top, right, bottom, left].
        let sides = [
            ("paddingLeft", tr("Left padding"), Icon::PaddingLeft, pl, 3),
            ("paddingTop", tr("Top padding"), Icon::PaddingTop, pt, 0),
            ("paddingRight", tr("Right padding"), Icon::PaddingRight, pr, 1),
            ("paddingBottom", tr("Bottom padding"), Icon::PaddingBottom, pb, 2),
        ];
        for (r, pair) in sides.chunks(2).enumerate() {
            row(ui, 32.0, |ui| {
                for (key, name, icon, v, k) in pair {
                    let e = crate::variable_ui::number_field(app, ui, ids, key, Id::new(("padding", *k)), Lead::Icon(*icon, name), Some(*v), "", col);
                    app.field(ids, key, e, |v| json!(v.max(0.0)));
                }
                if r == 0 {
                    toggle(ui);
                }
            });
        }
    } else {
        row(ui, 32.0, |ui| {
            let e = crate::variable_ui::number_field(
                app,
                ui,
                ids,
                "paddingHorizontal",
                Id::new("al_padh"),
                Lead::Icon(Icon::PaddingH, tr("Horizontal padding")),
                (pl == pr).then_some(pl),
                "",
                col,
            );
            app.field(ids, "paddingHorizontal", e, |v| json!(v.max(0.0)));
            let e = crate::variable_ui::number_field(
                app,
                ui,
                ids,
                "paddingVertical",
                Id::new("al_padv"),
                Lead::Icon(Icon::PaddingV, tr("Vertical padding")),
                (pt == pb).then_some(pt),
                "",
                col,
            );
            app.field(ids, "paddingVertical", e, |v| json!(v.max(0.0)));
            toggle(ui);
        });
    }
    if l.mode != LayoutMode::Grid && l.wrap {
        // The gap between the wrapped rows (horizontal) or columns (vertical) is the alignment
        // block's vertical field, so this row carries only its mode.
        let vertical = l.mode == LayoutMode::Vertical;
        let name = if vertical { tr("Column gap") } else { tr("Row gap") };
        widgets::sublabels(ui, name, None);
        row(ui, 32.0, |ui| {
            let spread = l.content_align == ContentAlign::SpaceBetween;
            let label = if spread { tr("Space between") } else { tr("Fixed gap") };
            let resp = widgets::dropdown(ui, label, col, None);
            crate::a11y::combo(&resp, name, label, crate::a11y::menu_open(&resp));
            menu_popup(&resp).show(|ui| {
                for (l, v) in [(tr("Fixed"), "auto"), (tr("Auto (space between)"), "spaceBetween")] {
                    if menu_item(ui, l, "") {
                        app.set_props(ids, json!({"contentAlign": v}));
                        ui.close();
                    }
                }
            });
        });
    }
    if l.mode == LayoutMode::Grid {
        widgets::sublabels(ui, tr("Columns"), Some(tr("Rows")));
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::new("al_cols"), Lead::None, Some(f64::from(l.grid_columns)), "", col, 1.0);
            app.field(ids, "gridColumns", e, |v| json!(v.max(1.0)));
            let e = widgets::number_field(ui, Id::new("al_rows"), Lead::None, Some(f64::from(l.grid_rows)), "", col, 1.0);
            app.field(ids, "gridRows", e, |v| json!(v.max(1.0)));
        });
    }
}

/// The advanced layout settings, in one popover: whether the frame's inside stroke takes up
/// space, which child paints on top where children overlap (canvas stacking; the layer order
/// stays), and text baselines on horizontal stacks — in that order. Inside an instance the first
/// two come from the main component, so they read disabled; the baselines stay the instance's own.
fn settings(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], l: &AutoLayout, in_instance: bool) {
    let from_main = tr("In an instance, this comes from the main component");
    ui.add_enabled_ui(!in_instance, |ui| {
        let strokes = l.strokes_included;
        let label = if strokes { tr("Included") } else { tr("Excluded") };
        widgets::setting_row(ui, tr("Strokes"), |ui| {
            let resp = widgets::dropdown(ui, label, widgets::SETTING_CONTROL, None);
            crate::a11y::combo(&resp, tr("Strokes"), label, crate::a11y::menu_open(&resp));
            menu_popup(&resp).show(|ui| {
                for (v, l, _) in [(true, tr("Included in layout"), tr("Included")), (false, tr("Excluded from layout"), tr("Excluded"))] {
                    if menu_item(ui, l, "") {
                        app.set_props(ids, json!({"strokesIncluded": v}));
                        ui.close();
                    }
                }
            });
        });
        let first = l.reverse_z;
        let label = if first { tr("First on top") } else { tr("Last on top") };
        widgets::setting_row(ui, tr("Canvas stacking"), |ui| {
            let resp = widgets::dropdown(ui, label, widgets::SETTING_CONTROL, None);
            crate::a11y::combo(&resp, tr("Canvas stacking"), label, crate::a11y::menu_open(&resp));
            menu_popup(&resp).show(|ui| {
                for (v, l) in [(false, tr("Last on top")), (true, tr("First on top"))] {
                    if menu_item(ui, l, "") {
                        app.set_props(ids, json!({"reverseZ": v}));
                        ui.close();
                    }
                }
            });
        });
    })
    .response
    .on_disabled_hover_text(from_main);
    // Text baselines only mean something on a horizontal stack.
    if l.mode == LayoutMode::Horizontal {
        let on = l.cross_align == CrossAlign::Baseline;
        ui.horizontal(|ui| {
            if widgets::checkbox(ui, on, tr("Align text baselines")) {
                app.set_props(ids, json!({"crossAlign": if on { "min" } else { "baseline" }}));
            }
        });
    }
}

/// The settings popover, drawn once a frame while it is open (it closes on its X, Escape and a
/// press outside, the gap row's button held out so that button toggles it).
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    use crate::popover::{Hold, Open};
    let Some(mut pop) = app.ui.layout_settings.take() else { return };
    let ids = app.session.selection();
    let doc = app.session.ds().ok().map(|d| d.doc.clone());
    let l = doc.as_ref().and_then(|d| common(d, &ids, |n| n.layout.clone())).flatten();
    // The same test the section uses: an instance (or a layer in one) has its layout from the
    // main component — the popover stays, with its strokes and stacking rows disabled.
    let in_instance = doc.as_ref().is_some_and(|d| ids.iter().any(|i| figcraft_engine::components::owning_instance(d, *i).is_some()));
    let Some(l) = l else {
        app.ui.layout_settings_button = None;
        return;
    };
    let screen = ctx.content_rect();
    let anchor = pos2(screen.max.x - app.ui.right_width, app.ui.layout_settings_anchor.unwrap_or(screen.min.y + 160.0));
    let holds: Vec<Rect> = app.ui.layout_settings_button.into_iter().collect();
    let closed = pop.show(ctx, Open::Anchor(anchor), Hold { rects: &holds, escape: false }, |ui| settings(app, ui, &ids, &l, in_instance));
    if closed {
        app.ui.layout_settings = None;
    } else {
        app.ui.layout_settings = Some(pop);
    }
}

/// Track sizes of a single grid frame: one row per column / row with its size and sizing.
fn tracks(app: &mut FigApp, ui: &mut Ui, doc: &Document, ids: &[NodeId]) {
    let Some(id) = ids.first().copied() else { return };
    let Some(l) = doc.node(id).and_then(|n| n.layout.clone()) else { return };
    let Some(g) = figcraft_layout::grid_geometry(doc, id) else { return };
    let col = widgets::col_width(ui.available_width());
    for (axis, specs, ranges) in [("columns", &l.column_tracks, &g.columns), ("rows", &l.row_tracks, &g.rows)] {
        widgets::sublabels(ui, if axis == "columns" { tr("Column sizes") } else { tr("Row sizes") }, None);
        for (i, (a, b)) in ranges.iter().enumerate().take(MAX_TRACK_ROWS) {
            let track = specs.get(i).copied().unwrap_or_default();
            let size = ((b - a) * 100.0).round() / 100.0;
            let base = json!({"ids": [id], "axis": axis, "index": i});
            let with = |sizing: &str, value: Option<f64>| {
                let mut p = base.clone();
                p["sizing"] = json!(sizing);
                if let Some(v) = value {
                    p["value"] = json!(v);
                }
                p
            };
            row(ui, 32.0, |ui| {
                let label = (i + 1).to_string();
                // Fill tracks edit their fr; fixed and hug tracks show (and set) a px size.
                let (value, suffix) = if track.sizing == Sizing::Fill { (track.value, "fr") } else { (size, "") };
                let e = widgets::number_field(
                    ui,
                    Id::new(("al_track", axis, i)),
                    Lead::Text(&label),
                    Some(value),
                    suffix,
                    col,
                    if suffix.is_empty() { 1.0 } else { 0.1 },
                );
                let fill = track.sizing == Sizing::Fill;
                app.field_run(e, "layout.setTrack", |v| with(if fill { "fill" } else { "fixed" }, Some(v.max(0.0))));
                let (label, icon) = match track.sizing {
                    Sizing::Fixed => (tr("Fixed"), Icon::Fixed),
                    Sizing::Hug => (tr("Hug"), Icon::Hug),
                    Sizing::Fill => (tr("Fill|sizing"), Icon::FillContainer),
                };
                let resp = widgets::dropdown(ui, label, col, Some(icon));
                menu_popup(&resp).show(|ui| {
                    for (l, v) in [(tr("Fixed"), "fixed"), (tr("Hug contents"), "hug"), (tr("Fill (fr)"), "fill")] {
                        if menu_item(ui, l, "") {
                            app.run("layout.setTrack", with(v, None));
                            ui.close();
                        }
                    }
                });
            });
        }
    }
}

/// A grid child's cell (single selection: column and row, 1-based; auto placement resets them)
/// and its spans.
fn grid_cell(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let col = widgets::col_width(ui.available_width());
    if let [id] = sel
        && let Some(cell) =
            parent_of(doc, *id).and_then(|p| figcraft_layout::grid_geometry(doc, p.id)).and_then(|g| g.cells.into_iter().find(|c| c.id == *id))
    {
        let anchored = doc.node(*id).is_some_and(|n| n.child_layout.grid_column.is_some() || n.child_layout.grid_row.is_some());
        widgets::sublabels(ui, tr("Column"), Some(tr("Row")));
        row(ui, 32.0, |ui| {
            // Moving on one axis pins the other where it is, so the child doesn't jump.
            let at = |c: f64, r: f64| json!({"ids": [id], "props": {"gridColumn": (c - 1.0).max(0.0), "gridRow": (r - 1.0).max(0.0)}});
            let (c0, r0) = ((cell.column + 1) as f64, (cell.row + 1) as f64);
            let e = widgets::number_field(ui, Id::new("al_cell_col"), Lead::None, Some(c0), "", col, 1.0);
            app.field_run(e, "node.set", |v| at(v, r0));
            let e = widgets::number_field(ui, Id::new("al_cell_row"), Lead::None, Some(r0), "", col, 1.0);
            app.field_run(e, "node.set", |v| at(c0, v));
            if anchored && icons::button(ui, Icon::Reset, 24.0, false, tr("Place automatically")).clicked() {
                app.set_props(&ids, json!({"gridColumn": null, "gridRow": null}));
            }
        });
    }
    let cs = common(doc, sel, |n| f64::from(n.child_layout.column_span()));
    let rs = common(doc, sel, |n| f64::from(n.child_layout.row_span()));
    widgets::sublabels(ui, tr("Column span"), Some(tr("Row span")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::new("al_span_c"), Lead::None, cs, "", col, 1.0);
        app.field(&ids, "gridColumnSpan", e, |v| json!(v.max(1.0)));
        let e = widgets::number_field(ui, Id::new("al_span_r"), Lead::None, rs, "", col, 1.0);
        app.field(&ids, "gridRowSpan", e, |v| json!(v.max(1.0)));
    });
    // Alignment in the cell; Auto follows the frame's alignment.
    widgets::sublabels(ui, tr("Horizontal align"), Some(tr("Vertical align")));
    row(ui, 32.0, |ui| {
        let axes = [
            ("gridAlignH", tr("Horizontal align"), common(doc, sel, |n| n.child_layout.grid_align_h), [tr("Left"), tr("Center"), tr("Right")]),
            ("gridAlignV", tr("Vertical align"), common(doc, sel, |n| n.child_layout.grid_align_v), [tr("Top"), tr("Center"), tr("Bottom")]),
        ];
        for (key, name, cur, [min, center, max]) in axes {
            let options = [(None, tr("Auto")), (Some(CellAlign::Min), min), (Some(CellAlign::Center), center), (Some(CellAlign::Max), max)];
            let label = cur.map_or(tr("Mixed"), |c| options.iter().find(|(v, _)| *v == c).map_or("", |(_, l)| *l));
            let resp = widgets::dropdown(ui, label, col, None);
            crate::a11y::combo(&resp, name, label, crate::a11y::menu_open(&resp));
            menu_popup(&resp).show(|ui| {
                for (v, l) in options {
                    if menu_item(ui, l, "") {
                        app.set_props(&ids, json!({key: v}));
                        ui.close();
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_horizontal_flow_runs_the_main_axis_along_the_columns_and_a_vertical_one_along_the_rows() {
        // Horizontal flow: column 1, row 2 is the main axis centred with the cross axis at its end.
        assert_eq!(axes(1, 2, true), (1, 2));
        // Vertical flow: the same cell is the cross axis centred with the main axis at its end.
        assert_eq!(axes(1, 2, false), (2, 1));
    }

    #[test]
    fn one_cell_is_lit_for_a_plain_alignment() {
        assert!(lit(1, 2, 1, 2));
        assert!(!lit(0, 2, 1, 2));
        assert!(!lit(1, 1, 1, 2));
    }

    #[test]
    fn a_spreading_axis_lights_all_of_it() {
        // An auto gap along the main axis: every column of the chosen row, whatever the cross
        // alignment is (the ends keep a dot beside the middle glyph).
        for main_i in 0..3 {
            assert!(lit(main_i, 1, 9, 1), "main {main_i} of a spread main axis");
        }
        assert!(!lit(1, 0, 9, 1) && !lit(1, 2, 9, 1), "only the chosen row");
        // Wrapped lines spread across the flow: the whole main column, every row.
        for cross_i in 0..3 {
            assert!(lit(2, cross_i, 2, 9), "row {cross_i} of the spread cross axis");
        }
        assert!(!lit(1, 0, 2, 9) && !lit(0, 2, 2, 9), "only the chosen column");
    }

    #[test]
    fn the_bars_read_both_alignments_in_their_place_and_length() {
        // Horizontal flow, top-left (main packed left, cross at the top): the first outer bar is the
        // longer one (7) and both short bars are flush left; the middle line is full width.
        assert_eq!(bars(true, 0, 0), [(0.0, 0.0, 7.0, 2.0), (0.0, 4.0, 10.0, 2.0), (0.0, 8.0, 5.0, 2.0)]);
        // Centred main, centred cross: the outer bars are 5 and 5, sharing the slack either side.
        assert_eq!(bars(true, 1, 1), [(2.5, 0.0, 5.0, 2.0), (0.0, 4.0, 10.0, 2.0), (2.5, 8.0, 5.0, 2.0)]);
        // Packed right, cross at the bottom: the last outer bar is the longer one (7), flush right.
        assert_eq!(bars(true, 2, 2), [(5.0, 0.0, 5.0, 2.0), (0.0, 4.0, 10.0, 2.0), (3.0, 8.0, 7.0, 2.0)]);
        // A vertical flow is the horizontal one turned on its side: vertical bars, the longer one on the
        // chosen cross side (left here, so the first is 7), packed along the vertical main axis.
        assert_eq!(bars(false, 1, 0), [(0.0, 1.5, 2.0, 7.0), (4.0, 0.0, 2.0, 10.0), (8.0, 2.5, 2.0, 5.0)]);
        // A spreading axis (9) centres like a centred alignment.
        assert_eq!(bars(true, 9, 1), bars(true, 1, 1));
        assert_eq!(bars(false, 9, 2), bars(false, 1, 2));
    }

    #[test]
    fn every_bar_stays_inside_the_glyph_box() {
        for horizontal in [true, false] {
            for main in [0, 1, 2, 9] {
                for cross in [0, 1, 2] {
                    for (x, y, w, h) in bars(horizontal, main, cross) {
                        assert!(
                            x >= 0.0 && y >= 0.0 && x + w <= GLYPH_BOX && y + h <= GLYPH_BOX,
                            "the {w}x{h} bar at ({x},{y}) leaves the {GLYPH_BOX} pt box"
                        );
                    }
                }
            }
        }
    }
}
