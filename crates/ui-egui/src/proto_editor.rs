//! The interaction editor's actions: an interaction's action list (add, reorder by dragging a row's
//! grip or from its menu, remove, open one to edit it), conditional actions built in place (if / else if / else blocks, each with a
//! condition from the expression builder or typed as an expression, and actions of its own), and
//! set variable values given as expressions. Every edit runs a `proto.action.*` or
//! `proto.conditional.*` command (one undo step each).

use egui::{Align2, CornerRadius, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp};
use figcraft_doc::{Document, NodeId, VariableType, VariableValue, Variables};
use figcraft_engine::cmd::proto_actions::{action_at, start_value};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::list_drag::ListDrag;
use crate::proto_panel::{action_fields, action_of, action_summary, targets_for, value_json};
use crate::right::{menu_check, menu_item, menu_popup, row, row_rect};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, BindEvent, FIELD_H, FieldEdit, Lead, PAD};

/// An action of an interaction being edited: the layer, the interaction, the action's path
/// (`[i]`, or `[i, block, j, …]` inside conditional actions).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Slot {
    pub node: NodeId,
    pub k: usize,
    pub path: Vec<usize>,
}

impl Slot {
    /// A widget id for `what` of this action.
    pub fn id(&self, what: &'static str) -> Id {
        Id::new((what, self.node, self.k, &self.path))
    }
}

/// Where a variable picked in the variable picker goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarPart {
    /// A set variable action's variable.
    SetVariable,
    /// The variable the condition of block `.0` tests.
    Lhs(usize),
    /// What the condition of block `.0` compares it with.
    Rhs(usize),
}

/// A variable picker target in a prototype action.
#[derive(Clone, Debug, PartialEq)]
pub struct VarSlot {
    pub slot: Slot,
    pub part: VarPart,
}

/// Run `cmd` on the action at `slot` (for every layer edited together), with `extra` params.
pub fn run_at(app: &mut FigApp, slot: &Slot, cmd: &str, extra: Value) -> Option<Value> {
    let mut p = json!({"ids": targets_for(app, slot.node), "index": slot.k, "path": slot.path});
    if let (Some(o), Value::Object(x)) = (p.as_object_mut(), extra) {
        o.extend(x);
    }
    app.run(cmd, p)
}

/// Replace the action at `slot`.
pub fn set_action(app: &mut FigApp, slot: &Slot, a: Action) {
    run_at(app, slot, "proto.action.set", json!({ "action": a }));
}

fn set_condition(app: &mut FigApp, slot: &Slot, block: usize, e: &Expr) {
    run_at(app, slot, "proto.conditional.setCondition", json!({"block": block, "condition": e}));
}

/// What the action editors read: the document, its page and the interaction being edited.
pub struct Cx<'a> {
    pub doc: &'a Document,
    pub page: NodeId,
    pub node: NodeId,
    pub k: usize,
}

impl Cx<'_> {
    fn slot(&self, path: Vec<usize>) -> Slot {
        Slot { node: self.node, k: self.k, path }
    }
}

/// Indent of each level of actions inside a conditional block.
const INDENT: f32 = 12.0;

/// The kinds an action can be, as its menus list them (a separator before each in `GROUPS`).
const ACTIONS: [&str; 11] = [
    tr_noop("Navigate to"),
    tr_noop("Change to"),
    tr_noop("Back"),
    tr_noop("Scroll to"),
    tr_noop("Open link"),
    tr_noop("Open overlay"),
    tr_noop("Swap overlay"),
    tr_noop("Close overlay"),
    tr_noop("Set variable"),
    tr_noop("Set variable mode"),
    tr_noop("Conditional"),
];
const GROUPS: [usize; 5] = [2, 4, 5, 8, 10];

/// The action kinds as menu items; the one picked.
fn action_menu(ui: &mut Ui) -> Option<&'static str> {
    let mut picked = None;
    for (i, l) in ACTIONS.iter().enumerate() {
        if GROUPS.contains(&i) {
            ui.separator();
        }
        if menu_item(ui, tr(l), "") {
            picked = Some(*l);
        }
    }
    picked
}

/// Why an action of kind `label` can't be made here.
fn unavailable(label: &str) -> &'static str {
    match label {
        "Change to" => tr("Change to needs a layer inside a variant"),
        "Set variable" => tr("Create a variable first"),
        "Set variable mode" => tr("Create a variable collection first"),
        _ => tr("There's no destination for that action"),
    }
}

fn open_id(cx: &Cx) -> Id {
    Id::new(("proto_open_action", cx.node, cx.k))
}

/// The open action (the conditionals around it are open too); the first one by default.
fn open_path(ctx: &egui::Context, cx: &Cx) -> Vec<usize> {
    ctx.data(|d| d.get_temp(open_id(cx))).unwrap_or_else(|| vec![0])
}

fn set_open(ctx: &egui::Context, cx: &Cx, path: Vec<usize>) {
    ctx.data_mut(|d| d.insert_temp(open_id(cx), path));
}

/// `add` in a column `indent` further right; the rect it took.
fn nested(ui: &mut Ui, indent: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    let avail = ui.available_rect_before_wrap();
    let max = Rect::from_min_max(pos2(avail.min.x + indent, avail.min.y), avail.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(max).layout(egui::Layout::top_down(egui::Align::Min)), add).response.rect
}

/// The drag-to-reorder id of the action list at `prefix` (tests find its rows' grips by it).
pub(crate) fn list_key(node: NodeId, k: usize, prefix: &[usize]) -> (&'static str, NodeId, usize, &[usize]) {
    ("proto_actions", node, k, prefix)
}

/// An action list: the interaction's (`prefix` empty) or a conditional block's (`prefix` =
/// `[action, block, …]`), then "Add action". Rows drag by their grip to reorder the list.
pub fn action_list(app: &mut FigApp, ui: &mut Ui, cx: &Cx, list: &[Action], prefix: &[usize]) {
    let open = open_path(ui.ctx(), cx);
    // An interaction with one action shows its editor as it is.
    let single = prefix.is_empty() && list.len() == 1;
    let mut drag = (!single).then(|| ListDrag::in_order(ui, list_key(cx.node, cx.k, prefix)));
    for (i, a) in list.iter().enumerate() {
        let path = [prefix, &[i]].concat();
        let is_open = single || open.starts_with(&path);
        let (header, extent) = action_item(app, ui, cx, a, path, list.len(), is_open, single);
        if let Some(d) = &mut drag {
            d.row(ui, i, header, extent, || action_summary(cx.doc, a));
        }
    }
    if let Some((from, to)) = drag.and_then(|d| d.finish(ui)) {
        let moved = cx.slot([prefix, &[from]].concat());
        if run_at(app, &moved, "proto.action.move", json!({ "to": to })).is_some() {
            // The open action (or the one inside it) stays open where it went.
            let mut next = open;
            if next.len() > prefix.len()
                && next.starts_with(prefix)
                && let Some(at) = next.get_mut(prefix.len())
            {
                *at = figcraft_engine::cmd::lists::moved_index(*at, from, to);
                set_open(ui.ctx(), cx, next);
            }
        }
    }
    add_action_row(app, ui, cx, [prefix, &[list.len()]].concat());
}

/// A text button with an icon ("+ Add action").
fn link_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.text_secondary);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, FIELD_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    let col = if resp.hovered() { t.text } else { t.text_secondary };
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 11.0, r.center().y), vec2(12.0, 12.0)), icon, col);
    ui.painter().text(pos2(r.min.x + 22.0, r.center().y), Align2::LEFT_CENTER, label, regular(11.0), col);
    resp
}

fn add_action_row(app: &mut FigApp, ui: &mut Ui, cx: &Cx, path: Vec<usize>) {
    row(ui, 28.0, |ui| {
        let resp = link_button(ui, Icon::Plus, tr("Add action"));
        menu_popup(&resp).width(200.0).show(|ui| {
            if let Some(l) = action_menu(ui) {
                match action_of(l, cx.doc, cx.page, cx.node, None) {
                    Some(a) => {
                        if run_at(app, &cx.slot(path.clone()), "proto.action.add", json!({ "action": a })).is_some() {
                            set_open(ui.ctx(), cx, path.clone());
                        }
                    }
                    None => app.toast(unavailable(l)),
                }
                ui.close();
            }
        });
    });
}

/// One action: its kind (open) or a summary (closed) with a menu to move or remove it, and when
/// open its settings below. Returns its row and the area it covers with its settings.
#[allow(clippy::too_many_arguments)]
fn action_item(app: &mut FigApp, ui: &mut Ui, cx: &Cx, a: &Action, path: Vec<usize>, count: usize, open: bool, single: bool) -> (Rect, Rect) {
    let t = Tokens::get(ui.ctx());
    let i = path.last().copied().unwrap_or(0);
    let removable = path.len() > 1 || count > 1;
    let slot = cx.slot(path);
    let header = row_rect(ui, 32.0, |ui| {
        if !single {
            let (r, resp) = ui.allocate_exact_size(vec2(12.0, FIELD_H), Sense::click());
            let resp = resp.tip(if open { tr("Close") } else { tr("Open") });
            icons::paint(
                ui.painter(),
                Rect::from_center_size(r.center(), vec2(12.0, 12.0)),
                if open { Icon::Chevron } else { Icon::ChevronRight },
                t.icon_secondary,
            );
            if resp.clicked() {
                // Closing keeps the conditional around it open.
                let next = if open { slot.path.get(..slot.path.len().saturating_sub(2)).unwrap_or_default().to_vec() } else { slot.path.clone() };
                set_open(ui.ctx(), cx, next);
            }
        }
        let w = ui.available_width() - if single { 0.0 } else { 32.0 };
        if open {
            let resp = widgets::dropdown(ui, tr(a.label()), w, None);
            menu_popup(&resp).show(|ui| {
                if let Some(l) = action_menu(ui) {
                    match action_of(l, cx.doc, cx.page, cx.node, Some(a)) {
                        Some(new) => set_action(app, &slot, new),
                        None => app.toast(unavailable(l)),
                    }
                    ui.close();
                }
            });
        } else {
            let (r, resp) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
            }
            let text = action_summary(cx.doc, a);
            let shown = widgets::elide(ui, &text, w - 12.0);
            ui.painter().text(pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, &shown, regular(11.0), t.text);
            let resp = if shown != text { resp.on_hover_text(text) } else { resp };
            if resp.clicked() {
                set_open(ui.ctx(), cx, slot.path.clone());
            }
        }
        if !single {
            let resp = icons::button(ui, Icon::Dots, 24.0, false, tr("More"));
            menu_popup(&resp).width(180.0).show(|ui| {
                let mut moved = None;
                if i > 0 && menu_item(ui, tr("Move up"), "") {
                    moved = Some(i - 1);
                }
                if i + 1 < count && menu_item(ui, tr("Move down"), "") {
                    moved = Some(i + 1);
                }
                if let Some(to) = moved
                    && let Some(r) = run_at(app, &slot, "proto.action.move", json!({ "to": to }))
                {
                    if open && let Some(p) = r.get("path").and_then(|p| serde_json::from_value::<Vec<usize>>(p.clone()).ok()) {
                        set_open(ui.ctx(), cx, p);
                    }
                    ui.close();
                }
                if removable {
                    if count > 1 {
                        ui.separator();
                    }
                    if menu_item(ui, tr("Remove"), "") {
                        if run_at(app, &slot, "proto.action.remove", json!({})).is_some() {
                            set_open(ui.ctx(), cx, slot.path.get(..slot.path.len().saturating_sub(2)).unwrap_or_default().to_vec());
                        }
                        ui.close();
                    }
                }
            });
        }
    });
    if open {
        nested(ui, if single { 0.0 } else { INDENT }, |ui| match a {
            Action::Conditional { blocks } => conditional_blocks(app, ui, cx, &slot, blocks),
            Action::SetVariable { variable, value, expr } => set_variable_fields(app, ui, cx, &slot, variable, value, expr.as_ref()),
            other => action_fields(app, ui, cx.doc, cx.page, &slot, other),
        });
    }
    (header, header.with_max_y(ui.cursor().min.y.max(header.max.y)))
}

/// The if / else if / else blocks of a conditional action: each one's condition and actions,
/// then "Else if" and "Else".
fn conditional_blocks(app: &mut FigApp, ui: &mut Ui, cx: &Cx, slot: &Slot, blocks: &[ConditionalBlock]) {
    let t = Tokens::get(ui.ctx());
    for (b, block) in blocks.iter().enumerate() {
        let label = match (&block.condition, b) {
            (Some(_), 0) => tr("If"),
            (Some(_), _) => tr("Else if"),
            (None, _) => tr("Else"),
        };
        block_header(app, ui, slot, b, label, block.condition.as_ref());
        if let Some(c) = &block.condition {
            condition_editor(app, ui, cx, slot, b, c);
        }
        let prefix = [slot.path.as_slice(), &[b]].concat();
        // A line down the block's actions, under its label (and under their rows' drag handles).
        let line = ui.painter().add(egui::Shape::Noop);
        let r = nested(ui, INDENT, |ui| action_list(app, ui, cx, &block.actions, &prefix));
        let x = r.min.x - INDENT + PAD + 2.0;
        ui.painter().set(line, egui::Shape::vline(x, r.min.y + 2.0..=r.max.y - 4.0, Stroke::new(1.0, t.border)));
    }
    let has_else = blocks.last().is_some_and(|b| b.condition.is_none());
    row(ui, 28.0, |ui| {
        if link_button(ui, Icon::Plus, tr("Else if")).on_hover_text(tr("Add another condition")).clicked() {
            run_at(app, slot, "proto.conditional.addBlock", json!({}));
        }
        if !has_else && link_button(ui, Icon::Plus, tr("Else")).on_hover_text(tr("Add actions for when no condition holds")).clicked() {
            run_at(app, slot, "proto.conditional.addBlock", json!({"else": true}));
        }
    });
}

/// Whether block `b`'s condition is edited as an expression (by choice).
fn text_mode_id(slot: &Slot, b: usize) -> Id {
    slot.id("proto_cond_text").with(b)
}

fn block_header(app: &mut FigApp, ui: &mut Ui, slot: &Slot, b: usize, label: &str, cond: Option<&Expr>) {
    let t = Tokens::get(ui.ctx());
    row(ui, 28.0, |ui| {
        let buttons = if cond.is_some() { 56.0 } else { 24.0 };
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - buttons, FIELD_H), Sense::hover());
        ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, semibold(11.0), t.text);
        if let Some(c) = cond {
            let id = text_mode_id(slot, b);
            let builder = simple(c).is_some();
            let text = !builder || ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            let tip = match (builder, text) {
                (false, _) => tr("This condition is edited as an expression"),
                (true, true) => tr("Edit with the builder"),
                (true, false) => tr("Edit as an expression"),
            };
            if icons::button(ui, Icon::Code, 24.0, text, tip).clicked() && builder {
                ui.data_mut(|d| d.insert_temp(id, !text));
            }
        }
        let what = if cond.is_some() { tr("Remove this condition and its actions") } else { tr("Remove the else and its actions") };
        if icons::button(ui, Icon::Minus, 24.0, false, what).clicked() {
            run_at(app, slot, "proto.conditional.removeBlock", json!({ "block": b }));
        }
    });
}

/// What a simple condition compares its variable with.
#[derive(Clone, Debug, PartialEq)]
pub enum Rhs {
    Value(VariableValue),
    Variable(String),
    /// `not variable`.
    Nothing,
}

/// A condition the builder edits: a variable, an operator, and a value or another variable.
#[derive(Clone, Debug, PartialEq)]
pub struct Simple {
    pub var: String,
    pub op: ExprOp,
    pub rhs: Rhs,
}

impl Simple {
    pub fn expr(&self) -> Expr {
        let lhs = Expr::var(self.var.clone());
        match &self.rhs {
            Rhs::Nothing => Expr::op(ExprOp::Not, vec![lhs]),
            Rhs::Value(v) => Expr::op(self.op, vec![lhs, Expr::Value(v.clone())]),
            Rhs::Variable(w) => Expr::op(self.op, vec![lhs, Expr::var(w.clone())]),
        }
    }
}

/// `e` as a builder condition, if it is one (anything else is edited as an expression).
pub fn simple(e: &Expr) -> Option<Simple> {
    let Expr::Op { op, args } = e else { return None };
    match (op, args.as_slice()) {
        (ExprOp::Not, [Expr::Value(VariableValue::Alias(v))]) => Some(Simple { var: v.clone(), op: ExprOp::Not, rhs: Rhs::Nothing }),
        (_, [Expr::Value(VariableValue::Alias(v)), rhs]) if BINARY.contains(op) => {
            let rhs = match rhs {
                Expr::Value(VariableValue::Alias(w)) => Rhs::Variable(w.clone()),
                Expr::Value(x) => Rhs::Value(x.clone()),
                _ => return None,
            };
            Some(Simple { var: v.clone(), op: *op, rhs })
        }
        _ => None,
    }
}

const BINARY: [ExprOp; 8] =
    [ExprOp::Equals, ExprOp::NotEqual, ExprOp::Less, ExprOp::LessOrEqual, ExprOp::Greater, ExprOp::GreaterOrEqual, ExprOp::And, ExprOp::Or];

/// The operators the builder offers for a variable of `kind`.
pub fn ops_for(kind: Option<VariableType>) -> &'static [ExprOp] {
    use ExprOp::*;
    match kind {
        Some(VariableType::Number) => &[Equals, NotEqual, Less, LessOrEqual, Greater, GreaterOrEqual],
        Some(VariableType::Boolean) => &[Equals, NotEqual, And, Or, Not],
        _ => &[Equals, NotEqual],
    }
}

/// `s` with operator `op`; what it compares with follows (`not` has nothing, the others a value).
fn with_op(s: &Simple, op: ExprOp, vars: &Variables) -> Simple {
    let rhs = match (op, &s.rhs) {
        (ExprOp::Not, _) => Rhs::Nothing,
        (_, Rhs::Nothing) => Rhs::Value(vars.get(&s.var).map_or(VariableValue::Boolean(true), |v| start_value(vars, v))),
        (_, r) => r.clone(),
    };
    Simple { var: s.var.clone(), op, rhs }
}

/// Block `b`'s condition: the builder (variable, operator, value or variable) when it fits,
/// otherwise (or by choice) an expression field.
fn condition_editor(app: &mut FigApp, ui: &mut Ui, cx: &Cx, slot: &Slot, b: usize, c: &Expr) {
    let vars = &cx.doc.variables;
    let text = ui.data(|d| d.get_temp::<bool>(text_mode_id(slot, b))).unwrap_or(false);
    match simple(c).filter(|_| !text) {
        Some(s) => builder_row(app, ui, cx, slot, b, &s),
        None => {
            let id = slot.id("proto_cond_expr").with(b);
            if let Some(e) = expr_field(ui, id, vars, Some(c), tr("e.g. count >= 3 and not done"), |e| e.check_condition(vars), None::<fn(&mut Ui)>) {
                set_condition(app, slot, b, &e);
            }
        }
    }
}

fn builder_row(app: &mut FigApp, ui: &mut Ui, cx: &Cx, slot: &Slot, b: usize, s: &Simple) {
    let vars = &cx.doc.variables;
    let var = vars.get(&s.var);
    let kind = var.map(|v| v.kind);
    row(ui, 32.0, |ui| {
        let w = ui.available_width();
        let op_w = 48.0;
        let side = ((w - op_w - 16.0) / 2.0).max(40.0);
        // The variable tested, from the variable picker.
        let name = var.map_or(tr("Missing variable"), |v| v.name.as_str());
        let shown = widgets::elide(ui, name, side - 48.0);
        let resp = widgets::dropdown(ui, &shown, side, Some(Icon::Variable)).on_hover_text(name);
        if resp.clicked() {
            crate::variable_ui::open_proto(ui, VarSlot { slot: slot.clone(), part: VarPart::Lhs(b) }, None, resp.rect.left_bottom());
        }
        let resp = widgets::dropdown(ui, s.op.symbol(), op_w, None);
        menu_popup(&resp).show(|ui| {
            for op in ops_for(kind) {
                if menu_check(ui, *op == s.op, op.symbol(), "") {
                    set_condition(app, slot, b, &with_op(s, *op, vars).expr());
                    ui.close();
                }
            }
        });
        let field = Rect::from_min_size(ui.cursor().min, vec2(side, FIELD_H));
        let commit = |app: &mut FigApp, rhs: Rhs| set_condition(app, slot, b, &Simple { rhs, ..s.clone() }.expr());
        match &s.rhs {
            Rhs::Nothing => {}
            Rhs::Variable(other) => {
                let (rect, _) = ui.allocate_exact_size(vec2(side, FIELD_H), Sense::hover());
                let name = vars.get(other).map_or(tr("Missing variable"), |v| v.name.as_str());
                match widgets::bound_field(ui, slot.id("proto_cond_rhs_var").with(b), Lead::None, rect, name) {
                    Some(BindEvent::Open(at)) => crate::variable_ui::open_proto(ui, VarSlot { slot: slot.clone(), part: VarPart::Rhs(b) }, kind, at),
                    Some(BindEvent::Detach) => {
                        let v = var.map_or(VariableValue::Boolean(true), |v| start_value(vars, v));
                        commit(app, Rhs::Value(v));
                    }
                    None => {}
                }
            }
            Rhs::Value(v) => {
                if let Some(nv) = literal_field(ui, slot.id("proto_cond_rhs").with(b), v, side) {
                    commit(app, Rhs::Value(nv));
                }
                if crate::variable_ui::variable_button(ui, field, slot.id("proto_cond_rhs_bind").with(b)) {
                    crate::variable_ui::open_proto(ui, VarSlot { slot: slot.clone(), part: VarPart::Rhs(b) }, kind, field.left_bottom());
                }
            }
        }
    });
}

/// A field for a value of `cur`'s type `w` wide: a number, text, true / false or a colour; the
/// new value when edited.
fn literal_field(ui: &mut Ui, id: Id, cur: &VariableValue, w: f32) -> Option<VariableValue> {
    match cur {
        VariableValue::Number(n) => match widgets::number_field(ui, id, Lead::None, Some(*n), "", w, 1.0) {
            Some(FieldEdit::Set(x) | FieldEdit::Scrub(x, true)) if x.is_finite() && x != *n => Some(VariableValue::Number(x)),
            _ => None,
        },
        VariableValue::Boolean(on) => {
            let resp = widgets::dropdown(ui, if *on { "true" } else { "false" }, w, None);
            let mut out = None;
            menu_popup(&resp).show(|ui| {
                for v in [true, false] {
                    if menu_check(ui, v == *on, if v { "true" } else { "false" }, "") {
                        out = (v != *on).then_some(VariableValue::Boolean(v));
                        ui.close();
                    }
                }
            });
            out
        }
        VariableValue::Color(c) => {
            let swatch = 16.0;
            widgets::swatch(ui, &figcraft_color::Paint::solid(*c), swatch);
            let hex = c.to_hex();
            widgets::text_field(ui, id, &hex, (w - swatch - 8.0).max(32.0), "")
                .and_then(|h| figcraft_color::Color::from_hex_or_name(&h))
                .filter(|n| n.to_hex() != hex)
                .map(VariableValue::Color)
        }
        VariableValue::String(s) => widgets::text_field(ui, id, s, w, tr("Text")).filter(|n| n != s).map(VariableValue::String),
        VariableValue::Alias(_) => None,
    }
}

/// A set variable action: the variable (from the variable picker) and the value it gets, fixed or
/// computed by an expression when the action runs (`count + 1`, `"Hi " + name`).
fn set_variable_fields(app: &mut FigApp, ui: &mut Ui, cx: &Cx, slot: &Slot, variable: &str, value: &Value, expr: Option<&Expr>) {
    let vars = &cx.doc.variables;
    let var = vars.get(variable);
    widgets::sublabels(ui, tr("Variable"), None);
    row(ui, 32.0, |ui| {
        let name = var.map_or(tr("Choose variable"), |v| v.name.as_str());
        let w = ui.available_width();
        let shown = widgets::elide(ui, name, w - 48.0);
        let resp = widgets::dropdown(ui, &shown, w, Some(Icon::Variable));
        if resp.clicked() {
            crate::variable_ui::open_proto(ui, VarSlot { slot: slot.clone(), part: VarPart::SetVariable }, None, resp.rect.left_bottom());
        }
    });
    let Some(var) = var else { return };
    let set =
        |app: &mut FigApp, value: Value, expr: Option<Expr>| set_action(app, slot, Action::SetVariable { variable: var.id.clone(), value, expr });
    // The toggle between a fixed value and an expression ends the value's row.
    let toggle = |ui: &mut Ui| {
        let on = expr.is_some();
        let tip = if on { tr("Use a fixed value") } else { tr("Use an expression, e.g. count + 1") };
        icons::button(ui, Icon::Code, 24.0, on, tip).clicked()
    };
    widgets::sublabels(ui, if expr.is_some() { tr("Expression") } else { tr("Value") }, None);
    let mut toggled = false;
    match expr {
        Some(e) => {
            let edited = expr_field(
                ui,
                slot.id("proto_set_expr"),
                vars,
                Some(e),
                tr("e.g. count + 1"),
                |e| e.check_value(vars, var.kind),
                Some(|ui: &mut Ui| {
                    toggled = toggle(ui);
                }),
            );
            if let Some(e) = edited {
                set(app, value.clone(), Some(e));
            }
            if toggled {
                // Back to a fixed value: the expression's when it is one, else the value it had.
                let fixed = match e {
                    Expr::Value(v) if v.type_of() == Some(var.kind) => value_json(v),
                    _ if !value.is_null() => value.clone(),
                    _ => value_json(&start_value(vars, var)),
                };
                set(app, fixed, None);
            }
        }
        None => {
            let cur = figcraft_engine::proto_run::literal(var.kind, value).unwrap_or_else(|| start_value(vars, var));
            row(ui, 32.0, |ui| {
                let w = ui.available_width() - 32.0;
                if let Some(v) = literal_field(ui, slot.id("proto_set_value"), &cur, w) {
                    set(app, value_json(&v), None);
                }
                toggled = toggle(ui);
            });
            if toggled {
                // An expression starts from the variable itself: type "+ 1" after it.
                set(app, value.clone(), Some(Expr::var(var.id.clone())));
            }
        }
    }
}

/// An expression typed as text in a row (with a `trailing` button after the field): what `check`
/// accepts is returned when committed; otherwise the text stays and why shows under it.
#[allow(clippy::too_many_arguments)]
fn expr_field(
    ui: &mut Ui,
    id: Id,
    vars: &Variables,
    cur: Option<&Expr>,
    placeholder: &str,
    check: impl Fn(&Expr) -> Result<(), String>,
    trailing: Option<impl FnOnce(&mut Ui)>,
) -> Option<Expr> {
    let t = Tokens::get(ui.ctx());
    let shown = cur.map(|e| e.display(vars)).unwrap_or_default();
    let key = id.with("draft");
    // A draft that failed to read: (the expression it was typed over, the text, the error).
    let draft: Option<(String, String, String)> = ui.data(|d| d.get_temp::<(String, String, String)>(key)).filter(|d| d.0 == shown);
    let mut error = draft.as_ref().map(|d| d.2.clone());
    let mut out = None;
    row(ui, 32.0, |ui| {
        let reserve = if trailing.is_some() { 32.0 } else { 0.0 };
        let text_now = draft.as_ref().map_or(shown.as_str(), |d| d.1.as_str()).to_string();
        let w = ui.available_width() - reserve;
        if let Some(text) = widgets::text_field(ui, id, &text_now, w, placeholder) {
            match Expr::parse(&text, vars).map_err(|e| e.message).and_then(|e| check(&e).map(|()| e)) {
                Ok(e) => {
                    ui.data_mut(|d| d.remove::<(String, String, String)>(key));
                    error = None;
                    out = (Some(&e) != cur).then_some(e);
                }
                Err(m) => {
                    ui.data_mut(|d| d.insert_temp(key, (shown.clone(), text, m.clone())));
                    error = Some(m);
                }
            }
        }
        if let Some(f) = trailing {
            f(ui);
        }
    });
    if let Some(m) = error {
        let w = ui.available_width();
        let galley = ui.painter().layout(m, regular(10.5), t.danger, (w - 2.0 * PAD).max(40.0));
        let (r, _) = ui.allocate_exact_size(vec2(w, galley.size().y + 6.0), Sense::hover());
        ui.painter().galley(pos2(r.min.x + PAD, r.min.y), galley, t.danger);
    }
    out
}

/// Use the variable picked in the variable picker in the action `vs` points at.
pub fn pick_variable(app: &mut FigApp, vs: &VarSlot, id: &str) {
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else { return };
    let vars = &doc.variables;
    let Some(picked) = vars.get(id) else { return };
    let current = doc.node(vs.slot.node).and_then(|n| n.interactions.get(vs.slot.k)).and_then(|it| action_at(&it.actions, &vs.slot.path));
    match (vs.part, current) {
        (VarPart::SetVariable, Some(Action::SetVariable { variable, value, expr })) => {
            // Another type starts from the new variable's value.
            let same = vars.get(variable).is_some_and(|v| v.kind == picked.kind);
            let (value, expr) = if same { (value.clone(), expr.clone()) } else { (value_json(&start_value(vars, picked)), None) };
            set_action(app, &vs.slot, Action::SetVariable { variable: picked.id.clone(), value, expr });
        }
        (VarPart::Lhs(b), Some(Action::Conditional { blocks })) => {
            let cur = blocks.get(b).and_then(|x| x.condition.as_ref()).and_then(simple);
            let next = match cur {
                Some(s) if vars.get(&s.var).is_some_and(|v| v.kind == picked.kind) => Simple { var: picked.id.clone(), ..s },
                _ => Simple { var: picked.id.clone(), op: ExprOp::Equals, rhs: Rhs::Value(start_value(vars, picked)) },
            };
            set_condition(app, &vs.slot, b, &next.expr());
        }
        (VarPart::Rhs(b), Some(Action::Conditional { blocks })) => {
            if let Some(s) = blocks.get(b).and_then(|x| x.condition.as_ref()).and_then(simple) {
                set_condition(app, &vs.slot, b, &Simple { rhs: Rhs::Variable(picked.id.clone()), ..s }.expr());
            }
        }
        _ => {}
    }
}
