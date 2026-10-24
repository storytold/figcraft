//! The style browser: the popover a Fill, Stroke, Effects, Layout grid or Typography section's
//! Styles button opens, laid out like the variable picker. Search; this file's styles, then each
//! enabled library's published ones, grouped by the folders of their `/`-separated names (folders
//! collapse); a list or, for colour and text styles, a grid. Text styles show in their own font,
//! and while the pointer is over one the canvas shows it on the selected text (or the selected
//! chars of the text being edited). Click applies a style (a library's is copied in, linked); a
//! local style's edit button (or a right-click) edits its name, description and values; `+`
//! creates a style from the selection (for text, the selected chars); the detach button unlinks
//! it. Colour slots also list colour variables to bind (this file's, then each enabled library's).

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, GridKind, Node, NodeId, PublishedLibrary, Style, StyleKind, TextStyle, VariableType, trim_num};
use figcraft_engine::library;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::colorpicker::PickerTarget;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// Content width of the popover.
const W: f32 = 240.0;
/// Height of a list row.
const ROW_H: f32 = 26.0;
/// Height of a text style's list row (its name in its own font).
const TEXT_ROW_H: f32 = 32.0;
/// Side of a swatch in the grid view.
const SWATCH: f32 = 24.0;
/// A text style's tile in the grid view.
const TILE: egui::Vec2 = egui::vec2(54.0, 44.0);
/// Largest size a text style's name previews at in the list (the grid's "Ag" goes to 24).
const LIST_PREVIEW_MAX: f64 = 16.0;

/// A style slot on a layer: what a style picker lists and what a style chip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleSlot {
    Fill,
    Stroke,
    Effect,
    Grid,
    Text,
}

impl StyleSlot {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fill" => Some(StyleSlot::Fill),
            "stroke" => Some(StyleSlot::Stroke),
            "effect" => Some(StyleSlot::Effect),
            "grid" => Some(StyleSlot::Grid),
            "text" => Some(StyleSlot::Text),
            _ => None,
        }
    }

    /// The slot's name in `style.apply {target}`, `style.detach {which}` and `variable.bind {field}`.
    pub fn key(self) -> &'static str {
        match self {
            StyleSlot::Fill => "fill",
            StyleSlot::Stroke => "stroke",
            StyleSlot::Effect => "effect",
            StyleSlot::Grid => "grid",
            StyleSlot::Text => "text",
        }
    }

    fn linked(self, n: &Node) -> Option<String> {
        match self {
            StyleSlot::Fill => n.styles.fill.clone(),
            StyleSlot::Stroke => n.styles.stroke.clone(),
            StyleSlot::Effect => n.styles.effect.clone(),
            StyleSlot::Grid => n.styles.grid.clone(),
            StyleSlot::Text => n.styles.text.clone(),
        }
    }

    fn paints(self) -> bool {
        matches!(self, StyleSlot::Fill | StyleSlot::Stroke)
    }

    /// Whether the slot's styles also show as a grid (swatches, "Ag" tiles).
    fn has_grid(self) -> bool {
        self.paints() || self == StyleSlot::Text
    }

    fn title(self) -> &'static str {
        match self {
            StyleSlot::Fill | StyleSlot::Stroke => tr("Color styles"),
            StyleSlot::Effect => tr("Effect styles"),
            StyleSlot::Grid => tr("Grid styles"),
            StyleSlot::Text => tr("Text styles"),
        }
    }

    /// `style.create {type}` for this slot (taken from the layer's fills, strokes, effects, grids
    /// or text).
    fn create_type(self) -> &'static str {
        match self {
            StyleSlot::Fill => "paint",
            StyleSlot::Stroke => "stroke",
            StyleSlot::Effect => "effect",
            StyleSlot::Grid => "grid",
            StyleSlot::Text => "text",
        }
    }

    /// The base of a new style's name ("Color 3").
    fn default_name(self) -> &'static str {
        match self {
            StyleSlot::Fill | StyleSlot::Stroke => "Color",
            StyleSlot::Effect => "Effect",
            StyleSlot::Grid => "Grid",
            StyleSlot::Text => "Text",
        }
    }

    fn holds(self, k: &StyleKind) -> bool {
        matches!(
            (k, self),
            (StyleKind::Paint { .. }, StyleSlot::Fill | StyleSlot::Stroke)
                | (StyleKind::Effect { .. }, StyleSlot::Effect)
                | (StyleKind::Grid { .. }, StyleSlot::Grid)
                | (StyleKind::Text { .. }, StyleSlot::Text)
        )
    }

    fn icon(self) -> Icon {
        match self {
            StyleSlot::Grid => Icon::Grid,
            StyleSlot::Text => Icon::Text,
            _ => Icon::Effects,
        }
    }
}

/// The style being edited, with its name and description as typed.
#[derive(Clone, Debug)]
struct Editing {
    id: String,
    name: String,
    description: String,
    /// Focus the name (and select it) on the next frame: a style that was just created.
    focus: bool,
}

/// The open picker.
#[derive(Clone, Debug)]
pub struct Picker {
    slot: StyleSlot,
    ids: Vec<NodeId>,
    /// Screen y of the button that opened it.
    y: f32,
    query: String,
    edit: Option<Editing>,
}

impl Picker {
    pub fn slot(&self) -> StyleSlot {
        self.slot
    }
}

/// Open the picker for `slot` of `ids`, level with the pointer.
pub fn open(app: &mut FigApp, ui: &Ui, ids: &[NodeId], slot: StyleSlot) {
    let y = ui.input(|i| i.pointer.interact_pos()).map_or(140.0, |p| p.y - 12.0);
    app.ui.style_picker = Some(Picker { slot, ids: ids.to_vec(), y, query: String::new(), edit: None });
}

/// Open the picker for `slot` of the selection (the control channel), optionally editing `style`.
pub fn open_for_selection(app: &mut FigApp, slot: StyleSlot, style: Option<&str>) {
    let ids = app.session.selection();
    let edit = style.and_then(|s| app.session.doc().ok()?.style(s).map(editing));
    app.ui.style_picker = Some(Picker { slot, ids, y: 140.0, query: String::new(), edit });
}

fn editing(s: &Style) -> Editing {
    Editing { id: s.id.clone(), name: s.name.clone(), description: s.description.clone(), focus: false }
}

/// The style `slot` of `ids` links to: `Some(Some(id))` one style, `Some(None)` none, None when
/// they differ. Text goes by chars: the selected chars of the text being edited, else every char.
fn link_of(app: &FigApp, doc: &Document, ids: &[NodeId], slot: StyleSlot) -> Option<Option<String>> {
    if slot != StyleSlot::Text {
        return crate::right::common(doc, ids, |n| slot.linked(n));
    }
    let range = figcraft_engine::textedit::selected_range(&app.session);
    let styles = crate::typography::target_styles(doc, ids, range);
    let mut links = styles.iter().map(|s| s.style_id.clone());
    let Some(first) = links.next() else { return Some(None) };
    links.all(|l| l == first).then_some(first)
}

/// The text style the targeted chars of `ids` link to (see [`link_of`]).
pub fn text_link(app: &FigApp, doc: &Document, ids: &[NodeId]) -> Option<Option<String>> {
    link_of(app, doc, ids, StyleSlot::Text)
}

/// The applied style's name (click opens the picker) with a detach button; for text, "Mixed"
/// when the chars link to different styles (or some to none).
pub fn chip(app: &mut FigApp, ui: &mut Ui, doc: &Document, ids: &[NodeId], slot: StyleSlot) {
    let t = Tokens::get(ui.ctx());
    let (name, mixed) = match link_of(app, doc, ids, slot) {
        Some(Some(id)) => (doc.style(&id).map_or_else(|| tr("Missing style").to_string(), |st| st.name.clone()), false),
        None if slot == StyleSlot::Text => (tr("Mixed").to_string(), true),
        _ => return,
    };
    let mut open_it = false;
    crate::right::row(ui, 32.0, |ui| {
        let w = (ui.available_width() - 32.0).max(40.0);
        let (r, resp) = ui.allocate_exact_size(vec2(w, widgets::FIELD_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        let icon = if slot == StyleSlot::Text { Icon::Text } else { Icon::Styles };
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0)), icon, t.icon_secondary);
        let col = if mixed { t.text_secondary } else { t.text };
        crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 26.0, r.center().y), &name, regular(11.0), col, r.width() - 32.0);
        let tip = if mixed { tr("The selection uses several text styles: click to apply one") } else { tr("Change style") };
        open_it = resp.on_hover_text(tip).clicked();
        let detach = if mixed { tr("Detach styles") } else { tr("Detach style") };
        if icons::button(ui, Icon::Detach, 24.0, false, detach).clicked() {
            app.run("style.detach", json!({"ids": ids, "which": slot.key()}));
        }
    });
    if open_it {
        open(app, ui, ids, slot);
    }
}

/// A listed style: by its library's id (None: this file's) and its id there.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    library: Option<String>,
    style: String,
}

impl Key {
    /// What `local` (a style in this document) is: a library style's copy, or this file's own.
    fn of_local(doc: &Document, local: &str) -> Key {
        match doc.libraries.style_import(local) {
            Some(r) => Key { library: Some(r.library.clone()), style: r.key.clone() },
            None => Key { library: None, style: local.to_string() },
        }
    }

    /// The command (and its params) applying this style to `ids`.
    fn apply(&self, ids: &[NodeId], slot: StyleSlot) -> (&'static str, Value) {
        let mut q = match &self.library {
            Some(lib) => json!({"library": lib, "style": self.style, "ids": ids}),
            None => json!({"ids": ids, "style": self.style}),
        };
        if slot.paints() {
            q["target"] = json!(slot.key());
        }
        (if self.library.is_some() { "library.applyStyle" } else { "style.apply" }, q)
    }
}

/// What the user asked the picker to do this frame.
enum Act {
    Apply(Key),
    Bind(crate::variable_ui::Pick),
    Create,
    Detach,
    Edit(String),
    Back,
    EditPaint(String, usize),
    /// Unbind the variable on a field of a style.
    Unbind(String, String),
    /// A text style's new definition (from the editor's fields).
    SetText(String, TextStyle),
    UpdateFromSelection(String),
    Delete(String),
    ToggleFolder(String),
    ToggleView,
}

/// What a listing reports besides acts: the style under the pointer, and the preview budget.
struct Frame {
    act: Option<Act>,
    hover: Option<Key>,
    /// Text previews still allowed to render this frame.
    budget: usize,
}

/// An icon button on the dark popover.
fn menu_icon(ui: &mut Ui, rect: Rect, icon: Icon, tip: &str, on: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, ui.id().with(("style_picker_btn", tip)), Sense::click()).tip(tip);
    if on || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.menu_hover);
    }
    icons::paint(ui.painter(), rect.shrink(5.0), icon, t.menu_text);
    resp
}

/// A text button on the dark popover.
fn menu_button(ui: &mut Ui, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.menu_text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, widgets::FIELD_H), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(5), if resp.hovered() { t.menu_hover } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.menu_dim), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(11.0), t.menu_text);
    resp
}

/// A dim caption on the popover.
fn caption(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).font(regular(10.0)).color(t.menu_dim));
}

/// The folder part of a style name ("Brand/Blue/500" → "Brand/Blue"), empty at the top level.
fn folder(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(f, _)| f.trim())
}

/// The last part of a style name ("Brand/Blue/500" → "500").
fn leaf(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, l)| l.trim())
}

/// `base N` with the smallest N no style of the document is named.
fn next_name(doc: &Document, base: &str) -> String {
    (1..).map(|n| format!("{base} {n}")).find(|n| doc.styles.iter().all(|s| &s.name != n)).unwrap_or_else(|| base.to_string())
}

/// Styles by folder, folders in order of first appearance, top-level styles first.
fn by_folder<'a>(styles: impl Iterator<Item = &'a Style>) -> Vec<(&'a str, Vec<&'a Style>)> {
    let mut groups: Vec<(&str, Vec<&Style>)> = Vec::new();
    for s in styles {
        let f = folder(&s.name);
        match groups.iter_mut().find(|(g, _)| *g == f) {
            Some((_, list)) => list.push(s),
            None => groups.push((f, vec![s])),
        }
    }
    groups.sort_by_key(|(f, _)| !f.is_empty());
    groups
}

/// A text style's definition, if `s` is one.
fn text_def(s: &Style) -> Option<&TextStyle> {
    match &s.kind {
        StyleKind::Text { style } => Some(style),
        _ => None,
    }
}

/// "32/Auto": a text style's size and line height.
fn text_meta(def: &TextStyle) -> String {
    format!("{}/{}", trim_num(def.font_size), def.line_height.label())
}

/// A text style's sample (`text` in its own font at most `max` big), painted from `left_center`;
/// falls back to plain UI text until the preview renders (or when the font is missing).
fn text_sample(ui: &Ui, def: &TextStyle, text: &str, max: f64, left_center: egui::Pos2, max_w: f32, max_h: f32, budget: &mut usize) {
    let t = Tokens::get(ui.ctx());
    let shown = TextStyle { font_size: def.font_size.clamp(6.0, max), ..def.clone() };
    match crate::font_preview::texture(ui.ctx(), &shown, text, budget) {
        Some(tex) => {
            crate::font_preview::paint(ui.painter(), &tex, left_center, max_w, max_h, t.menu_text);
        }
        None => {
            crate::bidi::paint_name(ui.painter(), left_center, text, regular(11.0), t.menu_text, max_w);
        }
    }
}

/// Draw a style's preview (its first paint, or the slot's icon) centred in `r`.
fn preview(ui: &Ui, r: Rect, s: &Style, slot: StyleSlot) {
    let t = Tokens::get(ui.ctx());
    match &s.kind {
        StyleKind::Paint { paints } => {
            let p = paints.last().cloned().unwrap_or_else(|| Paint::solid(figcraft_color::Color::TRANSPARENT));
            widgets::paint_swatch(ui.painter(), r, &p);
        }
        _ => icons::paint(ui.painter(), r, slot.icon(), t.menu_text),
    }
}

/// Click, right-click and hover on a listed style become acts.
fn react(resp: &Response, key: Key, editable: bool, out: &mut Frame) {
    if resp.hovered() {
        out.hover = Some(key.clone());
    }
    if resp.secondary_clicked() && editable {
        out.act = Some(Act::Edit(key.style));
    } else if resp.clicked() {
        out.act = Some(Act::Apply(key));
    }
}

/// One style in the list: preview (a text style: its name in its own font and its size), name,
/// a check when applied and an edit button on hover.
fn style_row(ui: &mut Ui, s: &Style, key: Key, slot: StyleSlot, applied: bool, editable: bool, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let h = if slot == StyleSlot::Text { TEXT_ROW_H } else { ROW_H };
    let (r, resp) = ui.allocate_exact_size(vec2(W, h), Sense::click());
    let edit_r = Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), vec2(22.0, 22.0));
    let hovered = resp.hovered() || ui.rect_contains_pointer(edit_r);
    if hovered {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    match text_def(s) {
        Some(def) => {
            let meta = text_meta(def);
            let meta_w = ui.painter().layout_no_wrap(meta.clone(), regular(10.0), t.menu_dim).size().x;
            ui.painter().text(
                pos2(edit_r.min.x - 4.0, r.center().y),
                Align2::RIGHT_CENTER,
                meta,
                regular(10.0),
                if hovered { t.menu_text } else { t.menu_dim },
            );
            let max_w = edit_r.min.x - 12.0 - meta_w - (r.min.x + 8.0);
            text_sample(ui, def, leaf(&s.name), LIST_PREVIEW_MAX, pos2(r.min.x + 8.0, r.center().y), max_w, h - 4.0, &mut out.budget);
        }
        None => {
            preview(ui, Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0)), s, slot);
            crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 26.0, r.center().y), leaf(&s.name), regular(11.0), t.menu_text, W - 64.0);
        }
    }
    // A style with variables bound inside it is marked (its users follow their own modes).
    if !s.bound.is_empty() {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(edit_r.min.x - 10.0, r.center().y), vec2(11.0, 11.0)), Icon::Variable, t.menu_dim);
    }
    if hovered && editable {
        if menu_icon(ui, edit_r, Icon::Adjust, tr("Edit style"), false).clicked() {
            out.act = Some(Act::Edit(s.id.clone()));
            return;
        }
    } else if applied {
        icons::paint(ui.painter(), edit_r.shrink(5.0), Icon::Check, t.menu_text);
    }
    let tip = match (s.description.is_empty(), text_def(s)) {
        (true, None) if !s.bound.is_empty() => Some(format!("{}\n{}", s.name, tr("Uses variables: each layer shows them in its own mode"))),
        (true, None) => None,
        (true, Some(def)) => Some(format!("{}\n{} {} · {}", s.name, def.font_family, def.font_style, text_meta(def))),
        (false, _) => Some(s.description.clone()),
    };
    let resp = match tip {
        Some(tip) => resp.on_hover_text(tip),
        None => resp,
    };
    react(&resp, key, editable, out);
}

/// Colour styles as swatches and text styles as "Ag" tiles in their own font (names on hover).
fn style_grid(ui: &mut Ui, list: &[(&Style, Key)], applied: Option<&Key>, editable: impl Fn(&Key) -> bool, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let text = list.first().is_some_and(|(s, _)| text_def(s).is_some());
    let cell = if text { TILE } else { vec2(SWATCH, SWATCH) };
    let per_row = ((W - 8.0 + 6.0) / (cell.x + 6.0)).floor().max(1.0) as usize;
    for chunk in list.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            for (s, key) in chunk {
                let (r, resp) = ui.allocate_exact_size(cell, Sense::click());
                match text_def(s) {
                    Some(def) => {
                        let fill = if resp.hovered() { t.menu_hover } else { Color32::from_white_alpha(14) };
                        ui.painter().rect_filled(r, CornerRadius::same(5), fill);
                        let sample_w = ui.painter().layout_no_wrap("Ag".into(), regular(18.0), t.menu_text).size().x;
                        text_sample(
                            ui,
                            def,
                            "Ag",
                            24.0,
                            pos2(r.center().x - sample_w / 2.0, r.center().y),
                            r.width() - 6.0,
                            r.height() - 4.0,
                            &mut out.budget,
                        );
                    }
                    None => preview(ui, r, s, StyleSlot::Fill),
                }
                if applied == Some(key) {
                    ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(5), Stroke::new(2.0, crate::theme::ACCENT), egui::StrokeKind::Inside);
                } else if resp.hovered() && !text {
                    ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(4), Stroke::new(1.0, t.menu_text), egui::StrokeKind::Inside);
                }
                let tip = match text_def(s) {
                    Some(def) => format!("{}\n{} {} · {}", s.name, def.font_family, def.font_style, text_meta(def)),
                    None => s.name.clone(),
                };
                let resp = resp.on_hover_text(tip);
                react(&resp, key.clone(), editable(key), out);
            }
        });
        ui.add_space(2.0);
    }
}

/// The styles a source offers (this file's, or a library's published ones) that the slot holds
/// and the search matches.
fn offered<'a>(doc: &'a Document, lib: Option<&'a PublishedLibrary>, slot: StyleSlot, q: &str) -> Vec<&'a Style> {
    let matches = |s: &&Style| slot.holds(&s.kind) && (q.is_empty() || s.name.to_lowercase().contains(q));
    match lib {
        Some(l) => l.doc.styles.iter().filter(|s| l.manifest.style(&s.id).is_some_and(|x| x.published)).filter(matches).collect(),
        None => library::local_styles(doc).filter(matches).collect(),
    }
}

/// The style list: header, search, this file's and the libraries' styles by folder, and (colour
/// slots) colour variables.
fn listing(app: &FigApp, ui: &mut Ui, doc: &Document, p: &mut Picker, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let slot = p.slot;
    let link = link_of(app, doc, &p.ids, slot);
    let applied = link.clone().flatten().map(|id| Key::of_local(doc, &id));
    let grid = slot.has_grid() && app.ui.style_grid;
    // Header: title, then view toggle, detach and create.
    let (hr, _) = ui.allocate_exact_size(vec2(W, 24.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + 2.0, hr.center().y), Align2::LEFT_CENTER, slot.title(), semibold(11.0), t.menu_text);
    let mut x = hr.max.x - 24.0;
    let mut slot_rect = || {
        let r = Rect::from_min_size(pos2(x, hr.min.y), vec2(24.0, 24.0));
        x -= 26.0;
        r
    };
    let create_tip = if slot == StyleSlot::Text { tr("Create style from the selected text") } else { tr("Create style") };
    if menu_icon(ui, slot_rect(), Icon::Plus, create_tip, false).clicked() {
        out.act = Some(Act::Create);
    }
    if link != Some(None) && menu_icon(ui, slot_rect(), Icon::Detach, tr("Detach style"), false).clicked() {
        out.act = Some(Act::Detach);
    }
    if slot.has_grid() {
        let (icon, tip) = if grid { (Icon::ViewList, tr("Show as list")) } else { (Icon::ViewGrid, tr("Show as grid")) };
        if menu_icon(ui, slot_rect(), icon, tip, false).clicked() {
            out.act = Some(Act::ToggleView);
        }
    }
    ui.add_space(4.0);
    let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tr("Search")).font(regular(11.0)).desired_width(W));
    crate::widgets::select_all_on_focus(ui, &r, &p.query);
    if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
        r.request_focus();
    }
    ui.add_space(4.0);
    let q = p.query.trim().to_lowercase();
    let libs = crate::variable_ui::libraries(app, doc);
    let mut sources: Vec<(Option<&PublishedLibrary>, Vec<&Style>)> = vec![(None, offered(doc, None, slot, &q))];
    for l in &libs {
        let list = offered(doc, Some(l), slot, &q);
        if !list.is_empty() {
            sources.push((Some(l.as_ref()), list));
        }
    }
    let colors = if slot.paints() {
        crate::variable_ui::groups(doc, &libs, |_, v| v.kind == VariableType::Color && (q.is_empty() || v.name.to_lowercase().contains(&q)))
    } else {
        Vec::new()
    };
    egui::ScrollArea::vertical().id_salt("style_picker_scroll").max_height(320.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if sources.iter().all(|(_, l)| l.is_empty()) {
            let any = library::local_styles(doc).any(|s| slot.holds(&s.kind));
            let hint = match (any, slot) {
                (true, _) => tr("No matching styles"),
                (false, StyleSlot::Text) => tr("No text styles yet. Click + to create one from the selected text."),
                (false, _) => tr("No styles yet. Click + to create one from the selection."),
            };
            caption(ui, hint);
        }
        for (lib, list) in &sources {
            if let Some(l) = lib {
                ui.add_space(6.0);
                caption(ui, &l.manifest.name);
            } else if list.is_empty() {
                continue;
            } else if sources.len() > 1 {
                caption(ui, tr("This file"));
            }
            let lib_id = lib.map(|l| l.manifest.id.clone());
            for (f, styles) in by_folder(list.iter().copied()) {
                let fold = match &lib_id {
                    Some(id) => format!("{id}/{f}"),
                    None => f.to_string(),
                };
                if !f.is_empty() {
                    // Searching shows every match, folders open.
                    let closed = q.is_empty() && app.ui.style_folds.contains(&fold);
                    let (r, resp) = ui.allocate_exact_size(vec2(W, ROW_H), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
                    }
                    let chevron = Rect::from_center_size(pos2(r.min.x + 10.0, r.center().y), vec2(10.0, 10.0));
                    icons::paint(ui.painter(), chevron, if closed { Icon::ChevronRight } else { Icon::Chevron }, t.menu_dim);
                    let label = f.split('/').map(str::trim).collect::<Vec<_>>().join(" / ");
                    crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 22.0, r.center().y), &label, regular(11.0), t.menu_dim, W - 26.0);
                    if resp.clicked() && q.is_empty() {
                        out.act = Some(Act::ToggleFolder(fold.clone()));
                    }
                    if closed {
                        continue;
                    }
                }
                let keyed: Vec<(&Style, Key)> = styles.into_iter().map(|s| (s, Key { library: lib_id.clone(), style: s.id.clone() })).collect();
                // Library styles are read-only here.
                let editable = |k: &Key| k.library.is_none();
                if grid {
                    ui.add_space(4.0);
                    style_grid(ui, &keyed, applied.as_ref(), editable, out);
                } else {
                    for (s, key) in keyed {
                        let on = applied.as_ref() == Some(&key);
                        let can_edit = editable(&key);
                        style_row(ui, s, key, slot, on, can_edit, out);
                    }
                }
            }
        }
        if !colors.is_empty() {
            ui.add_space(6.0);
            caption(ui, tr("Color variables"));
        }
        for (lib, c, vars) in &colors {
            crate::variable_ui::group_caption(ui, *lib, c);
            let all = lib.map_or(&doc.variables, |l| &l.doc.variables);
            for v in vars {
                if crate::variable_ui::variable_row(ui, all, v, W) {
                    out.act = Some(Act::Bind(crate::variable_ui::Pick { library: lib.map(|l| l.manifest.id.clone()), id: v.id.clone() }));
                }
            }
        }
    });
}

/// One property row in the editor (a paint, an effect, a grid); paints open the colour picker.
fn property_row(ui: &mut Ui, label: &str, paint: Option<&Paint>, slot: StyleSlot) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(W, ROW_H), if paint.is_some() { Sense::click() } else { Sense::hover() });
    if paint.is_some() && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let pr = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0));
    match paint {
        Some(p) => widgets::paint_swatch(ui.painter(), pr, p),
        None => icons::paint(ui.painter(), pr, slot.icon(), t.menu_text),
    }
    ui.painter().text(pos2(r.min.x + 26.0, r.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.menu_text);
    paint.is_some() && resp.clicked()
}

/// The variable on `field` of the style a [`property_row`] just drew: its name at the row's end
/// (a click unbinds it), or, with the pointer over the row, the button that opens the variable
/// picker for it. True when the pointer is on it (the row's own click isn't meant then).
fn binding_cell(ui: &mut Ui, doc: &Document, style: &Style, field: &str, kind: figcraft_doc::VariableType, out: &mut Frame) -> bool {
    let t = Tokens::get(ui.ctx());
    let row = ui.min_rect();
    let row = Rect::from_min_max(pos2(row.min.x, row.max.y - ROW_H), row.max);
    let Some(name) = style.bound.get(field).map(|v| doc.variables.get(v).map_or(v.as_str(), |x| x.name.as_str())) else {
        let b = Rect::from_center_size(pos2(row.max.x - 13.0, row.center().y), vec2(20.0, 20.0));
        if ui.rect_contains_pointer(row) && menu_icon(ui, b, Icon::Variable, tr("Apply variable"), false).clicked() {
            crate::variable_ui::open_style(ui, &style.id, field, kind, row.left_bottom());
        }
        return ui.rect_contains_pointer(b);
    };
    let id = Id::new(("style_binding", &style.id, field));
    let shown = widgets::elide_font(ui, leaf(name), 90.0, regular(10.0));
    let w = ui.painter().layout_no_wrap(shown.clone(), regular(10.0), t.menu_dim).size().x + 22.0;
    let chip = Rect::from_min_max(pos2(row.max.x - w - 4.0, row.min.y + 4.0), pos2(row.max.x - 4.0, row.max.y - 4.0));
    let resp =
        ui.interact(chip, id, Sense::click()).on_hover_text(crate::i18n::trf("Bound to {name}. Click to detach the variable.", &[("name", &name)]));
    ui.painter().rect_filled(chip, CornerRadius::same(4), if resp.hovered() { t.menu_hover } else { t.menu });
    ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.menu_dim.gamma_multiply(0.4)), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(chip.min.x + 9.0, chip.center().y), vec2(11.0, 11.0)), Icon::Variable, t.menu_dim);
    ui.painter().text(pos2(chip.min.x + 18.0, chip.center().y), Align2::LEFT_CENTER, shown, regular(10.0), t.menu_text);
    if resp.clicked() {
        out.act = Some(Act::Unbind(style.id.clone(), field.to_string()));
    }
    resp.hovered()
}

/// The field of effect `fx` (item `i` of an effect style) a variable binds from its row, and its
/// type: a shadow's colour, a blur's radius.
fn effect_binding(fx: &figcraft_doc::Effect, i: usize) -> Option<(String, figcraft_doc::VariableType)> {
    use figcraft_doc::{EffectKind as K, VariableType as V};
    match fx.kind {
        K::DropShadow(_) | K::InnerShadow(_) => Some((format!("effects/{i}/color"), V::Color)),
        K::LayerBlur(_) | K::BackgroundBlur(_) => Some((format!("effects/{i}/radius"), V::Number)),
        _ => None,
    }
}

/// A layout grid's summary ("Columns 12", "Grid 8").
fn grid_label(g: &figcraft_doc::LayoutGrid) -> String {
    match g.kind {
        GridKind::Grid => trf("Grid {size}", &[("size", &g.size)]),
        GridKind::Columns => trf("Columns {count}", &[("count", &g.count)]),
        GridKind::Rows => trf("Rows {count}", &[("count", &g.count)]),
    }
}

/// A text style's typography in the editor: a sample, then font, style, size, line height and
/// letter spacing; a change redefines the style (every linked char follows).
fn text_properties(ui: &mut Ui, id: &str, def: &TextStyle, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(W, 40.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(5), Color32::from_white_alpha(14));
    text_sample(ui, def, "Ag", 28.0, pos2(r.min.x + 10.0, r.center().y), 60.0, r.height() - 4.0, &mut out.budget);
    let summary = format!("{} {}\n{}", def.font_family, def.font_style, text_meta(def));
    let g = ui.painter().layout(summary, regular(10.5), t.menu_dim, W - 80.0);
    ui.painter().galley(pos2(r.min.x + 72.0, r.center().y - g.size().y / 2.0), g, t.menu_dim);
    ui.add_space(6.0);
    let set = |out: &mut Frame, f: &dyn Fn(&mut TextStyle)| {
        let mut d = def.clone();
        f(&mut d);
        if d != *def {
            out.act = Some(Act::SetText(id.to_string(), d));
        }
    };
    let half = (W - 8.0) / 2.0;
    // The dropdowns' menus open outside the dark scope (they're regular popups).
    let (family_resp, style_resp) = crate::theme::on_dark(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        let family_resp = widgets::dropdown(ui, &def.font_family, W, None).on_hover_text(tr("Font"));
        let mut style_resp = None;
        ui.horizontal(|ui| {
            style_resp = Some(widgets::dropdown(ui, &def.font_style, half, None).on_hover_text(tr("Font style")));
            if let Some(widgets::FieldEdit::Set(v) | widgets::FieldEdit::Scrub(v, true)) =
                widgets::number_field(ui, Id::new(("ts_size", id)), widgets::Lead::Text(tr("Size")), Some(def.font_size), "", half, 1.0)
            {
                set(out, &|d| d.font_size = v.clamp(1.0, 5000.0));
            }
        });
        ui.horizontal(|ui| {
            if let Some(v) = widgets::text_field(ui, Id::new(("ts_lh", id)), &def.line_height.label(), half, tr("Line height"))
                && let Some(lh) = crate::typography::parse_line_height(&v, Some(def.line_height)).and_then(|v| crate::typography::line_height_of(&v))
            {
                set(out, &|d| d.line_height = lh);
            }
            if let Some(v) = widgets::text_field(ui, Id::new(("ts_ls", id)), &def.letter_spacing.label(), half, tr("Letter spacing"))
                && let Some(ls) =
                    crate::typography::parse_letter_spacing(&v, Some(def.letter_spacing)).and_then(|v| crate::typography::letter_spacing_of(&v))
            {
                set(out, &|d| d.letter_spacing = ls);
            }
        });
        (family_resp, style_resp)
    });
    crate::right::menu_popup(&family_resp).width(260.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        if let Some(f) = crate::typography::font_picker(ui, Some(&def.font_family), family_resp.clicked()).picked {
            set(out, &|d| {
                d.font_family = f.clone();
                let styles = figcraft_text::family_styles(&f);
                if !styles.is_empty() && !styles.contains(&d.font_style) {
                    d.set_font_style(styles.iter().find(|s| s.eq_ignore_ascii_case("Regular")).or(styles.first()).cloned().unwrap_or_default());
                }
            });
            ui.close();
        }
    });
    if let Some(sr) = style_resp {
        crate::right::menu_popup(&sr).width(200.0).show(|ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for s in figcraft_text::family_styles(&def.font_family) {
                    let mark = if def.font_style == s { "✓ " } else { "   " };
                    if crate::right::menu_item(ui, &format!("{mark}{s}"), "") {
                        set(out, &|d| d.set_font_style(s.clone()));
                        ui.close();
                    }
                }
            });
        });
    }
}

/// Editing one style: name, description, its values (paints open the colour picker; text styles
/// have their typography), update from the selection, delete.
fn editor(ui: &mut Ui, doc: &Document, p: &mut Picker, out: &mut Frame) {
    let t = Tokens::get(ui.ctx());
    let slot = p.slot;
    let has_selection = !p.ids.is_empty();
    let Some(e) = p.edit.as_mut() else { return };
    let Some(style) = doc.style(&e.id).cloned() else {
        out.act = Some(Act::Back);
        return;
    };
    let (hr, _) = ui.allocate_exact_size(vec2(W, 24.0), Sense::hover());
    if menu_icon(ui, Rect::from_min_size(hr.min, vec2(24.0, 24.0)), Icon::ChevronLeft, tr("Back to styles"), false).clicked() {
        out.act = Some(Act::Back);
    }
    ui.painter().text(pos2(hr.min.x + 30.0, hr.center().y), Align2::LEFT_CENTER, tr("Edit style"), semibold(11.0), t.menu_text);
    ui.add_space(6.0);
    caption(ui, tr("Name"));
    let r = ui.add(egui::TextEdit::singleline(&mut e.name).font(regular(11.0)).desired_width(W));
    if e.focus {
        r.request_focus();
        let mut state = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(e.name.chars().count()));
        state.cursor.set_char_range(Some(all));
        egui::TextEdit::store_state(ui.ctx(), r.id, state);
        e.focus = false;
    }
    crate::widgets::select_all_on_focus(ui, &r, &e.name);
    ui.add_space(6.0);
    caption(ui, tr("Description"));
    ui.add(egui::TextEdit::multiline(&mut e.description).font(regular(11.0)).desired_width(W).desired_rows(2));
    ui.add_space(8.0);
    caption(ui, tr("Properties"));
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match &style.kind {
            StyleKind::Paint { paints } if paints.is_empty() => caption(ui, tr("No paints")),
            StyleKind::Paint { paints } => {
                // Top of the list paints on top: show in reverse like the Fill section.
                for (i, paint) in paints.iter().enumerate().rev() {
                    let label = format!("{}  {:.0}%", tr(&paint.label()), paint.opacity * 100.0);
                    let clicked = property_row(ui, &label, Some(paint), slot);
                    // A solid paint's colour takes a variable.
                    let on_binding = matches!(paint.kind, PaintKind::Solid { .. })
                        && binding_cell(ui, doc, &style, &format!("fills/{i}/color"), figcraft_doc::VariableType::Color, out);
                    if clicked && !on_binding {
                        out.act = Some(Act::EditPaint(style.id.clone(), i));
                    }
                }
            }
            StyleKind::Effect { effects } => {
                for (i, fx) in effects.iter().enumerate().rev() {
                    property_row(ui, tr(fx.label()), None, slot);
                    if let Some((field, kind)) = effect_binding(fx, i) {
                        binding_cell(ui, doc, &style, &field, kind, out);
                    }
                }
            }
            StyleKind::Grid { grids } => {
                for (i, g) in grids.iter().enumerate() {
                    property_row(ui, &grid_label(g), None, slot);
                    binding_cell(ui, doc, &style, &format!("layoutGrids/{i}/size"), figcraft_doc::VariableType::Number, out);
                }
            }
            StyleKind::Text { style: def } => text_properties(ui, &style.id, def, out),
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if has_selection && menu_button(ui, tr("Update from selection")).on_hover_text(tr("Take the values from the first selected layer")).clicked()
        {
            out.act = Some(Act::UpdateFromSelection(style.id.clone()));
        }
        if menu_button(ui, tr("Delete")).on_hover_text(tr("Delete the style (layers keep their values)")).clicked() {
            out.act = Some(Act::Delete(style.id.clone()));
        }
    });
}

/// Write the typed name and description of the style being edited, when they changed.
fn commit_edit(app: &mut FigApp, doc: &Document, e: &Editing) {
    let Some(s) = doc.style(&e.id) else { return };
    let name = e.name.trim();
    let mut q = json!({"style": e.id});
    if !name.is_empty() && name != s.name {
        q["name"] = json!(name);
    }
    if e.description != s.description {
        q["description"] = json!(e.description);
    }
    if q.as_object().is_some_and(|o| o.len() > 1) {
        app.run("style.update", q);
    }
}

/// The picker popover (call once per frame).
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut p) = app.ui.style_picker.take() else { return };
    let Ok(doc) = app.session.ds().map(|d| d.text_preview.clone().unwrap_or_else(|| d.doc.clone())) else { return };
    // A new selection closes the picker (it acts on the layers it was opened for).
    if app.session.selection() != p.ids {
        if let Some(e) = &p.edit {
            commit_edit(app, &doc, e);
        }
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Left of the Design panel; further left while the colour picker (which sits there) is open.
    let mut x = screen.max.x - app.ui.right_width - W - 32.0;
    if app.ui.picker.is_some() {
        x -= 260.0;
    }
    let y = p.y.clamp(screen.min.y + 48.0, (screen.max.y - 460.0).max(screen.min.y + 48.0));
    let mut out = Frame { act: None, hover: None, budget: 4 };
    let area = egui::Area::new(Id::new("style_picker_area")).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.menu).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(W);
            if p.edit.is_some() {
                editor(ui, &doc, &mut p, &mut out);
            } else {
                listing(app, ui, &doc, &mut p, &mut out);
            }
        });
    });
    let rect = area.response.rect;
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("style_picker_border"))).rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, Color32::from_black_alpha(40)),
        egui::StrokeKind::Inside,
    );
    // Clicks in the colour picker editing this picker's style, or in a menu the editor opened,
    // keep it open.
    let picker_rect =
        app.ui.picker.as_ref().filter(|t| matches!(t, PickerTarget::Style { .. })).and(ctx.memory(|m| m.area_rect(crate::colorpicker::window_id())));
    // So do clicks in the variable picker binding one of its fields (Escape closes that first).
    let variables = crate::variable_ui::picker_rect(ctx);
    let in_popup = egui::Popup::is_any_open(ctx) || variables.is_some();
    let outside = ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos) && !picker_rect.is_some_and(|r| r.contains(pos)))
    }) && !in_popup;
    let mut close = outside || (ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !in_popup);
    let ids = p.ids.clone();
    let slot = p.slot;
    // The canvas shows the text style under the pointer on the selected text.
    if slot == StyleSlot::Text
        && !close
        && out.act.is_none()
        && let Some(key) = &out.hover
    {
        let (cmd, params) = key.apply(&ids, slot);
        app.preview_text(ctx, cmd, params);
    }
    match out.act {
        Some(Act::Apply(key)) => {
            let (cmd, params) = key.apply(&ids, slot);
            app.run(cmd, params);
            close = true;
        }
        Some(Act::Bind(pick)) => {
            crate::variable_ui::bind(app, &ids, slot.key(), &pick);
            close = true;
        }
        Some(Act::Detach) => {
            app.run("style.detach", json!({"ids": ids, "which": slot.key()}));
        }
        Some(Act::Create) => {
            let name = next_name(&doc, slot.default_name());
            let made = app.run("style.create", json!({"type": slot.create_type(), "name": name, "from": ids.first()}));
            if let Some(id) = made.as_ref().and_then(|v| v["id"].as_str()) {
                p.query.clear();
                p.edit = Some(Editing { id: id.to_string(), name, description: String::new(), focus: true });
            }
        }
        Some(Act::Edit(id)) => p.edit = doc.style(&id).map(editing),
        Some(Act::Back) => {
            if let Some(e) = p.edit.take() {
                commit_edit(app, &doc, &e);
            }
        }
        Some(Act::EditPaint(id, index)) => {
            if let Some(e) = &p.edit {
                commit_edit(app, &doc, e);
            }
            app.ui.picker = Some(PickerTarget::Style { id, index });
        }
        Some(Act::Unbind(id, field)) => {
            app.run("style.bind", json!({"style": id, "field": field}));
        }
        Some(Act::SetText(id, def)) => {
            app.run("style.update", json!({"style": id, "value": def}));
        }
        Some(Act::UpdateFromSelection(id)) => {
            app.run("style.update", json!({"style": id, "from": ids.first()}));
        }
        Some(Act::Delete(id)) => {
            app.run("style.delete", json!({"style": id}));
            p.edit = None;
        }
        Some(Act::ToggleFolder(f)) => {
            if !app.ui.style_folds.remove(&f) {
                app.ui.style_folds.insert(f);
            }
        }
        Some(Act::ToggleView) => app.ui.style_grid = !app.ui.style_grid,
        None => {}
    }
    if close {
        if let Some(e) = &p.edit {
            commit_edit(app, &doc, e);
        }
        if matches!(app.ui.picker, Some(PickerTarget::Style { .. })) {
            app.ui.picker = None;
        }
    } else {
        app.ui.style_picker = Some(p);
    }
}

/// Open the style editor on this file's style `id` (from the Local styles list).
fn edit_local(app: &mut FigApp, ui: &Ui, slot: StyleSlot, id: &str) {
    let y = ui.input(|i| i.pointer.interact_pos()).map_or(140.0, |p| p.y - 12.0);
    let edit = app.session.doc().ok().and_then(|d| d.style(id).map(editing));
    app.ui.style_picker = Some(Picker { slot, ids: app.session.selection(), y, query: String::new(), edit });
}

/// `style.move` for the shown style `from` dropped at shown position `to` (`shown` in display
/// order): before the style that ends up under it, into the folder of its new neighbour above
/// (below at the top).
fn drop_move(shown: &[&Style], from: usize, to: usize) -> Option<Value> {
    let moved = shown.get(from)?;
    let rest: Vec<&Style> = shown.iter().enumerate().filter(|(i, _)| *i != from).map(|(_, s)| *s).collect();
    let below = rest.get(to);
    let near = to.checked_sub(1).and_then(|i| rest.get(i)).or(below)?;
    let mut q = json!({"styles": [moved.id], "folder": folder(&near.name)});
    if let Some(b) = below {
        q["before"] = json!(b.id);
    }
    Some(q)
}

/// The id of the folder caption being renamed in the Local styles list (temp data).
fn folder_rename_id() -> Id {
    Id::new("local_style_folder_rename")
}

/// The Design panel's Local styles (shown with nothing selected): this file's styles by type
/// (text, colour, effect, grid), then by folder. A click edits a style; right-click offers edit,
/// duplicate and delete; a row dragged by its grip moves (into the folder it lands in, renamed to
/// match); right-click on a folder renames it (and the styles in it).
pub fn local_styles(app: &mut FigApp, ui: &mut Ui, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    let renaming: Option<String> = ui.data(|d| d.get_temp(folder_rename_id()));
    let mut run: Option<(&'static str, Value)> = None;
    let mut edit: Option<(StyleSlot, String)> = None;
    for slot in [StyleSlot::Text, StyleSlot::Fill, StyleSlot::Effect, StyleSlot::Grid] {
        let groups = by_folder(library::local_styles(doc).filter(|s| slot.holds(&s.kind)));
        if groups.is_empty() {
            continue;
        }
        crate::right::row(ui, 24.0, |ui| widgets::label(ui, slot.title(), t.text_secondary));
        let mut drag = crate::list_drag::ListDrag::in_order(ui, ("local_styles", slot.key()));
        let mut shown: Vec<&Style> = Vec::new();
        for (f, list) in &groups {
            let indent = if f.is_empty() { 0.0 } else { 12.0 };
            if !f.is_empty() {
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                let text_r = Rect::from_min_max(pos2(r.min.x + widgets::PAD, r.min.y + 2.0), pos2(r.max.x - widgets::PAD, r.max.y - 2.0));
                if renaming.as_deref() == Some(*f) {
                    // Focused as it is first drawn: a focus asked for before its field is drawn
                    // isn't in the screen reader tree (UI Automation aborts the app, #224).
                    let start = ui.data_mut(|d| d.remove_temp::<bool>(folder_rename_id().with("start"))).unwrap_or(false);
                    let (out, open) = widgets::rename_field(ui, folder_rename_id().with(*f), f, text_r, regular(11.0), start);
                    if let Some(name) = out {
                        run = Some(("style.renameFolder", json!({"folder": f, "name": name})));
                    }
                    if !open {
                        ui.data_mut(|d| d.remove::<String>(folder_rename_id()));
                    }
                } else {
                    let resp = resp.tip(*f);
                    crate::bidi::paint_name(ui.painter(), text_r.left_center(), f, semibold(11.0), t.text, text_r.width());
                    crate::right::context_popup(&resp).show(|ui| {
                        if crate::right::menu_item(ui, tr("Rename folder"), "") {
                            ui.data_mut(|d| d.insert_temp(folder_rename_id(), f.to_string()));
                            ui.data_mut(|d| d.insert_temp(folder_rename_id().with("start"), true));
                            ui.close();
                        }
                    });
                }
            }
            for s in list {
                let index = shown.len();
                shown.push(s);
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
                }
                let x = r.min.x + widgets::PAD + indent;
                let icon_r = Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0));
                match &s.kind {
                    StyleKind::Paint { paints } => {
                        let p = paints.last().cloned().unwrap_or_else(|| Paint::solid(figcraft_color::Color::TRANSPARENT));
                        widgets::paint_swatch(ui.painter(), icon_r, &p);
                    }
                    _ => icons::paint(ui.painter(), icon_r, slot.icon(), t.icon_secondary),
                }
                let mut name_w = r.max.x - widgets::PAD - (x + 24.0);
                if let Some(def) = text_def(s) {
                    let meta = ui.painter().text(
                        pos2(r.max.x - widgets::PAD, r.center().y),
                        Align2::RIGHT_CENTER,
                        text_meta(def),
                        regular(10.0),
                        t.text_secondary,
                    );
                    name_w -= meta.width() + 8.0;
                }
                crate::bidi::paint_name(ui.painter(), pos2(x + 24.0, r.center().y), leaf(&s.name), regular(11.0), t.text, name_w);
                let tip = if s.description.is_empty() { s.name.clone() } else { format!("{}\n{}", s.name, s.description) };
                let resp = resp.tip(tip);
                if resp.clicked() {
                    edit = Some((slot, s.id.clone()));
                }
                crate::right::context_popup(&resp).show(|ui| {
                    if crate::right::menu_item(ui, tr("Edit style"), "") {
                        edit = Some((slot, s.id.clone()));
                        ui.close();
                    }
                    if crate::right::menu_item(ui, tr("Duplicate"), "") {
                        run = Some(("style.duplicate", json!({"style": s.id})));
                        ui.close();
                    }
                    if crate::right::menu_item(ui, tr("Delete"), "") {
                        run = Some(("style.delete", json!({"style": s.id})));
                        ui.close();
                    }
                });
                drag.row(ui, index, r, r, || leaf(&s.name).to_string());
            }
        }
        if let Some(q) = drag.finish(ui).and_then(|(from, to)| drop_move(&shown, from, to)) {
            run = Some(("style.move", q));
        }
    }
    if let Some((cmd, q)) = run {
        app.run(cmd, q);
    }
    if let Some((slot, id)) = edit {
        edit_local(app, ui, slot, &id);
    }
}

#[cfg(test)]
#[path = "style_picker_tests.rs"]
mod tests;
