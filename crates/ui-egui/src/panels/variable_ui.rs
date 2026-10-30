//! Binding variables to properties from the Design panel: number fields show a variable button on
//! hover and the bound variable's name once bound; colour fills bind through the style picker. The
//! picker popover lists the document's variables of the matching type (and scope), by collection,
//! then those of each enabled library (picking one copies it in, linked and read-only); the
//! Variables editor uses it to pick alias targets too, and the interaction editor the variables of
//! set variable actions and conditions.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Collection, Document, ModeId, NodeId, PublishedLibrary, Variable, VariableId, VariableType, VariableValue, Variables};
use figcraft_engine::library;
use serde_json::json;

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, BindEvent, Edit, Empty, FieldEdit, Lead};

/// What picking a variable does.
#[derive(Clone)]
enum Target {
    /// Bind it to `field` of `ids`.
    Bind { ids: Vec<NodeId>, field: String },
    /// Make `variable`'s value in `mode` an alias of it.
    Alias { variable: VariableId, mode: ModeId },
    /// Use it in a prototype action (a set variable's variable, a condition's operand).
    Proto(crate::proto_editor::VarSlot),
    /// Bind it to `field` of the shared style `style`.
    Style { style: String, field: String },
}

/// The open variable picker: what it does, the variable type (None: any), where it opens.
#[derive(Clone)]
struct Picker {
    target: Target,
    kind: Option<VariableType>,
    at: Pos2,
}

/// Stable egui id for the variable picker.
fn picker_id() -> Id {
    Id::unique("variable_picker")
}

/// The variable `field` is bound to on every one of `ids` (its name), if they agree.
pub fn bound_name(doc: &Document, ids: &[NodeId], field: &str) -> Option<String> {
    let mut it = ids.iter().map(|i| doc.node(*i).and_then(|n| n.bound.get(field)).cloned());
    let first = it.next().flatten()?;
    if !it.all(|v| v.as_ref() == Some(&first)) {
        return None;
    }
    doc.variables.get_by_id(&first).map(|v| v.name.clone())
}

/// A variable a picker offers: one of this file's, or a published one of an enabled library.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    /// The library's id; None for this file's variables.
    pub library: Option<String>,
    /// The variable's id (in the library for a library variable).
    pub id: VariableId,
}

/// Bind `field` of `ids` to `pick` (a library variable is copied in first).
pub fn bind(app: &mut FigApp, ids: &[NodeId], field: &str, pick: &Pick) {
    match &pick.library {
        Some(lib) => app.run("library.bindVariable", json!({"library": lib, "variable": pick.id, "ids": ids, "field": field})),
        None => app.run("variable.bind", json!({"ids": ids, "field": field, "variable": pick.id})),
    };
}

/// The enabled libraries this session has read, for pickers and the Variables editor.
pub fn libraries(app: &FigApp, doc: &Document) -> Vec<Arc<PublishedLibrary>> {
    library::read_libraries(&app.session.libraries, doc).map(|(_, lib)| lib.clone()).collect()
}

/// A variable group a picker lists: its library (None: this file), its collection and the
/// variables offered.
pub type Group<'a> = (Option<&'a PublishedLibrary>, &'a Collection, Vec<&'a Variable>);

/// Every group a picker lists, with the variables passing `keep`: this file's collections first
/// (never library copies), then each library's published ones. Empty groups are left out.
pub fn groups<'a>(
    doc: &'a Document,
    libs: &'a [Arc<PublishedLibrary>],
    keep: impl Fn(Option<&PublishedLibrary>, &Variable) -> bool,
) -> Vec<Group<'a>> {
    let mut out: Vec<Group<'a>> = Vec::new();
    let local: Vec<&Variable> = doc.variables.variables.iter().filter(|v| !library::is_remote_variable(doc, &v.id)).collect();
    for c in library::local_collections(doc) {
        out.push((None, c, local.iter().copied().filter(|v| v.collection == c.id && keep(None, v)).collect()));
    }
    for lib in libs {
        let offered: Vec<&Variable> = library::published_variables(lib).collect();
        for c in library::published_collections(lib) {
            out.push((Some(lib.as_ref()), c, offered.iter().copied().filter(|v| v.collection == c.id && keep(Some(lib), v)).collect()));
        }
    }
    out.retain(|g| !g.2.is_empty());
    out
}

/// A collection the Design panel's mode picker lists: this file's, or one an enabled library
/// offers (or offered: a copy whose library isn't read any more).
#[derive(Clone, Debug, PartialEq)]
pub struct ModeRow<'a> {
    /// The collection with its modes (a library's copy once it is used here).
    pub coll: &'a Collection,
    /// The library's (id, name); None for this file's collections.
    pub library: Option<(&'a str, &'a str)>,
    /// The collection's id in the library, when the library is read (modes are set through
    /// `library.setVariableMode`, which copies the collection in).
    pub key: Option<&'a str>,
    /// The id frames' modes are kept under: this file's collection or a library collection's copy
    /// (None until a library collection is used here).
    pub local: Option<&'a str>,
}

/// The collections with more than one mode the mode picker lists: this file's first, then each
/// enabled library's published ones (their copies once used), then copies of collections from
/// libraries that aren't read.
pub fn mode_rows<'a>(doc: &'a Document, libs: &'a [Arc<PublishedLibrary>]) -> Vec<ModeRow<'a>> {
    let several = |c: &Collection| c.modes.len() > 1;
    let mut out: Vec<ModeRow<'a>> =
        library::local_collections(doc).filter(|c| several(c)).map(|c| ModeRow { coll: c, library: None, key: None, local: Some(&c.id) }).collect();
    let copy = |lib: &str, key: &str| doc.libraries.collections.iter().find(|r| r.library == lib && r.key == key);
    for lib in libs {
        let (id, name) = (lib.manifest.id.as_str(), library::library_name(doc, &lib.manifest.id));
        for c in library::published_collections(lib) {
            let local = copy(id, &c.id).map(|r| r.local.as_str());
            let coll = local.and_then(|l| doc.variables.collection_by_id(l)).unwrap_or(c);
            if several(coll) {
                out.push(ModeRow { coll, library: Some((id, name)), key: Some(&c.id), local });
            }
        }
    }
    for r in &doc.libraries.collections {
        if out.iter().any(|x| x.local == Some(r.local.as_str())) {
            continue;
        }
        if let Some(c) = doc.variables.collection_by_id(&r.local).filter(|c| several(c)) {
            out.push(ModeRow { coll: c, library: Some((&r.library, library::library_name(doc, &r.library))), key: None, local: Some(&r.local) });
        }
    }
    out
}

/// A picker row: the variable's swatch (colours) or icon, its name and (other types) its value in
/// the default modes; true when clicked.
pub fn variable_row(ui: &mut Ui, vars: &Variables, v: &Variable, width: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let value = vars.resolve(&v.id, &std::collections::BTreeMap::new());
    let lead = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0));
    match &value {
        Some(VariableValue::Color(c)) => widgets::paint_swatch(ui.painter(), lead, &figcraft_color::Paint::solid(*c)),
        _ => icons::paint(ui.painter(), lead.shrink(1.0), Icon::Variable, t.menu_text),
    }
    let name_r = Rect::from_min_max(pos2(r.min.x + 26.0, r.min.y), pos2(r.max.x - 6.0, r.max.y));
    if let Some(val) = value.filter(|v| !matches!(v, VariableValue::Color(_))) {
        ui.painter().text(pos2(r.max.x - 6.0, r.center().y), Align2::RIGHT_CENTER, val.display(), regular(11.0), t.menu_dim);
    }
    ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, &v.name, regular(11.0), t.menu_text);
    resp.clicked()
}

/// A group's caption: the collection's name, after its library's name for library variables.
pub fn group_caption(ui: &mut Ui, lib: Option<&PublishedLibrary>, coll: &Collection) {
    let t = Tokens::get(ui.ctx());
    let text = match lib {
        Some(l) => format!("{} · {}", l.manifest.name, coll.name),
        None => coll.name.clone(),
    };
    ui.label(egui::RichText::new(text).font(regular(10.0)).color(t.menu_dim));
}

/// Open the picker for `field` of `ids` at `at`.
pub fn open(ui: &Ui, ids: &[NodeId], field: &str, kind: VariableType, at: Pos2) {
    let target = Target::Bind { ids: ids.to_vec(), field: field.to_string() };
    ui.data_mut(|d| d.insert_temp(picker_id(), Picker { target, kind: Some(kind), at }));
}

/// Open the picker at `at` to pick a variable (of `kind`, or any) for a prototype action.
pub fn open_proto(ui: &Ui, slot: crate::proto_editor::VarSlot, kind: Option<VariableType>, at: Pos2) {
    ui.data_mut(|d| d.insert_temp(picker_id(), Picker { target: Target::Proto(slot), kind, at }));
}

/// Open the picker at `at` to bind a variable of `kind` to `field` of the shared style `style`.
pub fn open_style(ui: &Ui, style: &str, field: &str, kind: VariableType, at: Pos2) {
    let target = Target::Style { style: style.to_string(), field: field.to_string() };
    ui.data_mut(|d| d.insert_temp(picker_id(), Picker { target, kind: Some(kind), at }));
}

/// Where the open picker is on screen (clicks in it don't close what opened it).
pub fn picker_rect(ctx: &egui::Context) -> Option<Rect> {
    picker_open(ctx).then(|| ctx.memory(|m| m.area_rect(Id::unique("variable_picker_area")))).flatten()
}

/// Whether the picker is open (Escape closes it before what opened it).
pub fn picker_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Picker>(picker_id()).is_some())
}

/// Open the picker at `at` to alias `variable`'s value in `mode` to another variable.
pub fn open_alias(ui: &Ui, variable: &str, mode: &str, kind: VariableType, at: Pos2) {
    let target = Target::Alias { variable: variable.to_string(), mode: mode.to_string() };
    ui.data_mut(|d| d.insert_temp(picker_id(), Picker { target, kind: Some(kind), at }));
}

/// Open the picker for `field` of `ids` under the pointer (from a section header button).
pub fn open_at_pointer(ui: &Ui, ids: &[NodeId], field: &str, kind: VariableType) {
    let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
    open(ui, ids, field, kind, at);
}

/// Act on a bound field's event: change the variable, or detach it.
fn handle(app: &mut FigApp, ui: &Ui, ids: &[NodeId], field: &str, kind: VariableType, ev: Option<BindEvent>) {
    match ev {
        Some(BindEvent::Open(at)) => open(ui, ids, field, kind, at),
        Some(BindEvent::Detach) => {
            app.run("variable.bind", json!({"ids": ids, "field": field, "variable": null}));
        }
        None => {}
    }
}

/// A number field whose value can be bound to a number variable. `suffix` follows the number
/// ("%" for opacity, whose variables hold percentages).
pub fn number_field(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
) -> Option<FieldEdit> {
    number_field_or(app, ui, ids, field, id, value, width, tr("Mixed"), lead, suffix)
}

/// [`number_field`] showing `empty` (e.g. "Auto") for no value.
pub fn number_field_or(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    id: Id,
    value: Option<f64>,
    width: f32,
    empty: &str,
    lead: Lead,
    suffix: &str,
) -> Option<FieldEdit> {
    number_field_edit(app, ui, ids, field, id, value, width, Empty::Text(empty), lead, suffix)?.settled()
}

/// [`number_field_or`] for a field where 0 is its automatic value (a layout grid's width or
/// count): typing the word for Auto sets 0 ([`Empty::Auto`]).
pub fn number_field_auto(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    id: Id,
    value: Option<f64>,
    width: f32,
    lead: Lead,
    suffix: &str,
) -> Option<FieldEdit> {
    number_field_edit(app, ui, ids, field, id, value, width, Empty::Auto, lead, suffix)?.settled()
}

/// [`number_field`] over several layers: a relative edit of mixed values comes back as one for
/// each layer ([`Edit::Each`]; see `FigApp::field_of`).
pub fn number_field_mixed(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
) -> Option<Edit> {
    number_field_edit(app, ui, ids, field, id, value, width, Empty::Text(tr("Mixed")), lead, suffix)
}

fn number_field_edit(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    id: Id,
    value: Option<f64>,
    width: f32,
    empty: Empty<'_>,
    lead: Lead,
    suffix: &str,
) -> Option<Edit> {
    let name = app.session.doc().ok().and_then(|d| bound_name(d, ids, field));
    let (edit, ev) = widgets::number_field_bindable(ui, id, lead, value, suffix, width, 1.0, Some(name.as_deref()), empty);
    handle(app, ui, ids, field, VariableType::Number, ev);
    edit
}

/// When `field` of `ids` is bound: the variable's chip in `rect` (change or detach it) and true;
/// otherwise nothing and false.
pub fn bound_chip(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], field: &str, kind: VariableType, rect: Rect) -> bool {
    let Some(name) = app.session.doc().ok().and_then(|d| bound_name(d, ids, field)) else { return false };
    let ev = widgets::bound_field(ui, Id::unique(("bound_chip", field, ids)), Lead::None, rect, &name);
    handle(app, ui, ids, field, kind, ev);
    true
}

/// A control whose value can come from a variable: while `field` of `ids` is bound, the
/// variable's chip (`width` wide) stands in for it; otherwise `control` lays it out in the current
/// row and a variable button shows over that area on hover.
pub fn bindable(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    field: &str,
    kind: VariableType,
    width: f32,
    control: impl FnOnce(&mut FigApp, &mut Ui),
) {
    let area = Rect::from_min_size(ui.cursor().min, vec2(width, widgets::FIELD_H));
    if app.session.doc().ok().and_then(|d| bound_name(d, ids, field)).is_some() {
        let (r, _) = ui.allocate_exact_size(area.size(), Sense::hover());
        bound_chip(app, ui, ids, field, kind, r);
        return;
    }
    control(app, ui);
    bind_button(ui, area, ids, field, kind);
}

/// A variable button at the right end of `rect` while the pointer is over it: opens the picker
/// to bind `field` of `ids`.
pub fn bind_button(ui: &mut Ui, rect: Rect, ids: &[NodeId], field: &str, kind: VariableType) {
    if variable_button(ui, rect, Id::unique(("bind_button", field, ids))) {
        open(ui, ids, field, kind, rect.left_bottom());
    }
}

/// The "Apply variable" button at the right end of `rect` while the pointer is over it; true
/// when clicked.
pub fn variable_button(ui: &mut Ui, rect: Rect, id: Id) -> bool {
    if !ui.rect_contains_pointer(rect) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let b = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(18.0, 18.0));
    let resp = ui.interact(b, id, Sense::click()).tip(tr("Apply variable"));
    if resp.hovered() {
        ui.painter().rect_filled(b, CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), b.shrink(3.0), Icon::Variable, t.icon_secondary);
    resp.clicked()
}

/// A field showing the variable `field` of `ids` is bound to, for bindings without a value field
/// of their own (text content, visibility): `width` wide, at most the row. Nothing when unbound.
pub fn bound_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], field: &str, kind: VariableType, lead: Lead, width: f32) {
    let Some(name) = app.session.doc().ok().and_then(|d| bound_name(d, ids, field)) else { return };
    crate::right::row(ui, 32.0, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width()), widgets::FIELD_H), Sense::hover());
        let ev = widgets::bound_field(ui, Id::unique(("bound_row", field)), lead, rect, &name);
        handle(app, ui, ids, field, kind, ev);
    });
}

/// Whether a variable of type `have` belongs in a picker for `want` on `field`: its own type, and
/// numbers for text content too (shown as text).
fn kind_fits(want: Option<VariableType>, field: Option<&str>, have: VariableType) -> bool {
    want.is_none_or(|k| have == k || (field == Some("characters") && have == VariableType::Number))
}

/// The picker popover (call once per frame).
pub fn picker(app: &mut FigApp, ctx: &egui::Context) {
    let Some(p) = ctx.data(|d| d.get_temp::<Picker>(picker_id())) else { return };
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else { return };
    let libs = libraries(app, &doc);
    let t = Tokens::get(ctx);
    let query_id = Id::unique("variable_picker_query");
    let mut query: String = ctx.data(|d| d.get_temp(query_id)).unwrap_or_default();
    let q = query.trim().to_lowercase();
    // Bindings offer variables scoped to the property; aliases, any that wouldn't loop (library
    // variables never lead back to this file's).
    let (scope, own) = match &p.target {
        Target::Bind { ids, field } => (ids.first().and_then(|id| figcraft_engine::cmd::variables::field_scope(&doc, *id, field)), None),
        Target::Alias { variable, .. } => (None, doc.variables.get_by_id(variable)),
        Target::Proto(_) | Target::Style { .. } => (None, None),
    };
    // A style's fields offer variables scoped to one of the properties the field sets.
    let style_scopes = match &p.target {
        Target::Style { style, field } => doc.style(style).and_then(|st| figcraft_engine::style_vars::field_scopes(&st.kind, field)),
        _ => None,
    };
    // Prototypes and styles use this file's variables.
    let local_only = matches!(p.target, Target::Proto(_) | Target::Style { .. });
    let field = match &p.target {
        Target::Bind { field, .. } => Some(field.as_str()),
        _ => None,
    };
    let keep = |lib: Option<&PublishedLibrary>, v: &Variable| {
        kind_fits(p.kind, field, v.kind)
            && !(local_only && lib.is_some())
            && (q.is_empty() || v.name.to_lowercase().contains(&q))
            && scope.is_none_or(|s| v.in_scope(s))
            && style_scopes.is_none_or(|list| list.iter().any(|s| v.in_scope(s)))
            && (lib.is_some() || own.is_none_or(|o| figcraft_engine::cmd::variables::alias_target(&doc.variables, o, &v.id).is_ok()))
    };
    let groups = groups(&doc, &libs, keep);
    let mut chosen: Option<Pick> = None;
    let mut close = false;
    let area = egui::Area::new(Id::unique("variable_picker_area")).order(egui::Order::Foreground).fixed_pos(p.at + vec2(0.0, 4.0)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.menu).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(240.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr("Variables")).font(semibold(11.0)).color(t.menu_text));
            });
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tr("Search")).font(regular(11.0)).desired_width(224.0));
            crate::widgets::select_all_on_focus(ui, &r, &query);
            if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (lib, c, vars) in &groups {
                    group_caption(ui, *lib, c);
                    let all = lib.map_or(&doc.variables, |l| &l.doc.variables);
                    for v in vars {
                        if variable_row(ui, all, v, 224.0) {
                            chosen = Some(Pick { library: lib.map(|l| l.manifest.id.clone()), id: v.id.clone() });
                        }
                    }
                }
            });
            if groups.is_empty() {
                // Whole sentences per type: languages place the type differently.
                let (fit, none) = match p.kind {
                    Some(VariableType::Color) => {
                        (tr("No colour variables fit here."), tr("No colour variables yet. Create them in Local variables."))
                    }
                    Some(VariableType::Number) => {
                        (tr("No number variables fit here."), tr("No number variables yet. Create them in Local variables."))
                    }
                    Some(VariableType::String) => {
                        (tr("No string variables fit here."), tr("No string variables yet. Create them in Local variables."))
                    }
                    Some(VariableType::Boolean) => {
                        (tr("No boolean variables fit here."), tr("No boolean variables yet. Create them in Local variables."))
                    }
                    None => (tr("No variables fit here."), tr("No variables yet. Create them in Local variables.")),
                };
                let msg = if groups_any_kind(&doc, &libs, p.kind) { fit } else { none };
                ui.label(egui::RichText::new(msg).font(regular(11.0)).color(t.menu_dim));
            }
        });
    });
    let rect = area.response.rect;
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("variable_picker_border"))).rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, Color32::from_black_alpha(40)),
        egui::StrokeKind::Inside,
    );
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos)));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_outside {
        close = true;
    }
    if let Some(pick) = chosen {
        match (&p.target, &pick.library) {
            (Target::Bind { ids, field }, _) => bind(app, ids, field, &pick),
            (Target::Alias { variable, mode }, None) => {
                app.run("variable.set", json!({"variable": variable, "mode": mode, "value": {"alias": pick.id}}));
            }
            (Target::Alias { variable, mode }, Some(lib)) => {
                app.run("library.aliasVariable", json!({"library": lib, "alias": pick.id, "variable": variable, "mode": mode}));
            }
            (Target::Proto(slot), _) => crate::proto_editor::pick_variable(app, slot, &pick.id),
            (Target::Style { style, field }, _) => {
                app.run("style.bind", json!({"style": style, "field": field, "variable": pick.id}));
            }
        }
        close = true;
    }
    if close {
        ctx.data_mut(|d| {
            d.remove::<Picker>(picker_id());
            d.remove::<String>(query_id);
        });
    } else {
        ctx.data_mut(|d| d.insert_temp(query_id, query));
    }
}

/// Whether this file or a library offers any variable of `kind` (or any variable).
fn groups_any_kind(doc: &Document, libs: &[Arc<PublishedLibrary>], kind: Option<VariableType>) -> bool {
    !groups(doc, libs, |_, v| kind.is_none_or(|k| v.kind == k)).is_empty()
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    #[test]
    fn text_content_offers_numbers_too() {
        use VariableType::*;
        assert!(kind_fits(Some(String), Some("characters"), Number), "a number shows as text");
        assert!(kind_fits(Some(String), Some("characters"), String));
        assert!(!kind_fits(Some(String), Some("characters"), Color));
        assert!(!kind_fits(Some(String), Some("fontFamily"), Number), "only text content");
        assert!(!kind_fits(Some(Number), Some("width"), String));
        assert!(kind_fits(None, None, Boolean));
    }
}

#[cfg(test)]
mod scope_tests {
    use figcraft_engine::Session;

    use super::*;
    use crate::layers_drag_tests::Ui as TestUi;
    use crate::style_picker::StyleSlot;

    fn run(s: &mut Session, cmd: &str, p: serde_json::Value) -> serde_json::Value {
        s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
    }

    /// A rectangle (selected, with a drop shadow), a frame with a layout grid, the styles "Brand"
    /// (paint), "Shadow" (effect, from the rectangle), "Base" (grid, from the frame) and "Body"
    /// (text), and variables each scoped to one property (`any` to none).
    fn session() -> Session {
        let mut s = Session::new();
        let r = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40}))["id"].clone();
        run(&mut s, "effect.add", json!({"ids": [r], "type": "dropShadow"}));
        let f = run(&mut s, "frame.create", json!({"x": 100, "y": 0, "width": 100, "height": 100}))["id"].clone();
        run(&mut s, "grid.add", json!({"ids": [f]}));
        run(&mut s, "style.create", json!({"type": "paint", "name": "Brand", "value": [{"type": "solid", "color": "#0D99FF"}]}));
        run(&mut s, "style.create", json!({"type": "effect", "name": "Shadow", "from": r}));
        run(&mut s, "style.create", json!({"type": "grid", "name": "Base", "from": f}));
        run(&mut s, "style.create", json!({"type": "text", "name": "Body", "value": {"fontFamily": "Inter", "fontSize": 14}}));
        run(&mut s, "variable.createCollection", json!({"name": "Tokens", "modes": ["Base"]}));
        for (name, ty, value, scope) in [
            ("frame fill", "color", json!("#FF0000"), Some("FRAME_FILL")),
            ("shape fill", "color", json!("#00FF00"), Some("SHAPE_FILL")),
            ("stroke color", "color", json!("#0000FF"), Some("STROKE_COLOR")),
            ("shadow color", "color", json!("#000000"), Some("EFFECT_COLOR")),
            ("radius", "number", json!(8), Some("CORNER_RADIUS")),
            ("font size", "number", json!(18), Some("FONT_SIZE")),
            ("blur", "number", json!(4), Some("EFFECT_FLOAT")),
            ("any", "number", json!(2), None),
            ("family", "string", json!("Inter"), Some("FONT_FAMILY")),
            ("weight", "string", json!("Bold"), Some("FONT_STYLE")),
        ] {
            run(&mut s, "variable.create", json!({"collection": "Tokens", "name": name, "type": ty, "values": {"Base": value}}));
            if let Some(sc) = scope {
                run(&mut s, "variable.update", json!({"variable": name, "scopes": [sc]}));
            }
        }
        run(&mut s, "select.set", json!({"ids": [r]}));
        s
    }

    /// The texts drawn in one frame.
    fn drawn(u: &mut TestUi) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in u.frame_output(vec![]).shapes {
            walk(&c.shape, &mut out);
        }
        out
    }

    fn check(texts: &[String], shown: &[&str], hidden: &[&str], what: &str) {
        for name in shown {
            assert!(texts.iter().any(|t| t == name), "{what}: {name:?} offered, in {texts:?}");
        }
        for name in hidden {
            assert!(!texts.iter().any(|t| t == name), "{what}: {name:?} not offered, in {texts:?}");
        }
    }

    #[test]
    fn a_styles_variable_picker_offers_the_fields_scopes_only() {
        use VariableType::{Color, Number, String};
        let mut u = TestUi::new(session());
        // (style, field, type, offered, not offered)
        for (style, field, kind, shown, hidden) in [
            ("Brand", "fill", Color, &["frame fill", "shape fill", "stroke color"][..], &["shadow color"][..]),
            ("Shadow", "effects/0/color", Color, &["shadow color"], &["frame fill", "shape fill", "stroke color"]),
            ("Shadow", "effects/0/radius", Number, &["blur", "any"], &["radius", "font size"]),
            ("Base", "layoutGrids/0/size", Number, &["radius", "font size", "blur", "any"], &[]),
            ("Body", "fontSize", Number, &["font size", "any"], &["radius", "blur"]),
            ("Body", "fontFamily", String, &["family"], &["weight"]),
            ("Body", "fontStyle", String, &["weight"], &["family"]),
        ] {
            let id = u.app.session.doc().unwrap().style(style).unwrap().id.clone();
            let target = Target::Style { style: id, field: field.to_string() };
            u.ctx.data_mut(|d| d.insert_temp(picker_id(), Picker { target, kind: Some(kind), at: pos2(400.0, 100.0) }));
            u.frame(vec![]);
            check(&drawn(&mut u), shown, hidden, &format!("{style} {field}"));
        }
    }

    #[test]
    fn the_fill_and_stroke_style_pickers_offer_scoped_color_variables() {
        let mut u = TestUi::new(session());
        for (slot, shown, hidden) in [
            (StyleSlot::Fill, &["shape fill"][..], &["frame fill", "stroke color", "shadow color"][..]),
            (StyleSlot::Stroke, &["stroke color"], &["frame fill", "shape fill", "shadow color"]),
        ] {
            crate::style_picker::open_for_selection(&mut u.app, slot, None);
            u.frame(vec![]);
            let texts = drawn(&mut u);
            assert!(texts.iter().any(|t| t == "Color variables"), "{slot:?}: {texts:?}");
            check(&texts, shown, hidden, &format!("{slot:?}"));
        }
    }

    #[test]
    fn a_mixed_selections_fill_picker_offers_variables_scoped_to_every_layer() {
        let mut s = session();
        let r = s.selection()[0];
        let f = s.doc().unwrap().nodes().find(|n| matches!(n.kind, figcraft_doc::NodeKind::Frame)).unwrap().id;
        run(&mut s, "variable.create", json!({"collection": "Tokens", "name": "both fills", "type": "color", "values": {"Base": "#FFFF00"}}));
        run(&mut s, "variable.update", json!({"variable": "both fills", "scopes": ["FRAME_FILL", "SHAPE_FILL"]}));
        // The frame first: its scope alone would offer "frame fill" for the rectangle too.
        run(&mut s, "select.set", json!({"ids": [f, r]}));
        let mut u = TestUi::new(s);
        crate::style_picker::open_for_selection(&mut u.app, StyleSlot::Fill, None);
        u.frame(vec![]);
        check(&drawn(&mut u), &["both fills"], &["frame fill", "shape fill"], "frame and rectangle");
    }
}
