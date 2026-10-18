//! The Design tokens editor (Preferences ▸ Work with: Design tokens), laid out like Local
//! variables: the resolution order on the left — sets, and modifiers (themes, brands…) with their
//! contexts, later entries overriding earlier ones — and the selected set's or context's tokens on
//! the right: name, value as written (an alias `{group.token}` or a value of the DTCG type) and the
//! value it resolves to. The header picks the context of each modifier this file uses. Import and
//! export read and write DTCG token files and resolver documents (2025.10). Everything goes
//! through `token.*` commands, so it's undoable and the same over MCP.

use std::collections::BTreeMap;

use egui::{Align2, Context, CornerRadius, Id, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use figcraft_doc::tokens::{self as dt, OrderItem, TokenType};
use figcraft_doc::{DesignTokens, Document, ResolvedToken, TokenSource};
use serde_json::{Map, Value, json};

use crate::FigApp;
use crate::colorpicker::PickerTarget;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::right::{menu_check, menu_item};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

const SIDEBAR_W: f32 = 220.0;
const NAME_W: f32 = 230.0;
const VALUE_W: f32 = 250.0;
const ROW_H: f32 = 34.0;

/// The types in the order the editor offers them, with their labels.
const TYPES: [(TokenType, &str); 13] = [
    (TokenType::Color, tr_noop("Color")),
    (TokenType::Dimension, tr_noop("Dimension")),
    (TokenType::Number, tr_noop("Number")),
    (TokenType::FontFamily, tr_noop("Font family")),
    (TokenType::FontWeight, tr_noop("Font weight")),
    (TokenType::Duration, tr_noop("Duration")),
    (TokenType::CubicBezier, tr_noop("Cubic Bézier")),
    (TokenType::StrokeStyle, tr_noop("Stroke style")),
    (TokenType::Border, tr_noop("Border")),
    (TokenType::Shadow, tr_noop("Shadow")),
    (TokenType::Gradient, tr_noop("Gradient")),
    (TokenType::Typography, tr_noop("Typography")),
    (TokenType::Transition, tr_noop("Transition")),
];

fn type_label(k: Option<TokenType>) -> &'static str {
    k.and_then(|k| TYPES.iter().find(|(t, _)| *t == k)).map_or(tr("No type"), |(_, l)| tr(l))
}

/// What's being renamed inline.
#[derive(Clone, Debug, PartialEq, Hash)]
enum Rename {
    Set(String),
    Modifier(String),
    /// (modifier, context)
    Context(String, String),
    Token(String),
    Group(String),
}

/// The open editor's state (`UiState::tokens`; None = closed).
#[derive(Clone, Default)]
pub struct Editor {
    /// The set or context whose tokens are shown.
    source: Option<Src>,
    query: String,
    rename: Option<Rename>,
    rename_start: bool,
    /// The token whose details are open, and its value being edited as JSON.
    details: Option<String>,
    json: Option<String>,
    /// Pasted JSON waiting to be imported (the paste dialog is open).
    paste: Option<String>,
    /// A file asked for in the browser, not read yet (and the set or context it goes into).
    upload: Option<Option<Src>>,
    /// The open alias picker.
    alias: Option<AliasPick>,
    /// Composite tokens whose fields are shown (by source and path); the rest show one row.
    open_fields: std::collections::BTreeSet<String>,
}

/// The alias picker: which value it sets (a token, or one field of a composite token) to an alias
/// of a token of `kind`.
#[derive(Clone)]
struct AliasPick {
    path: String,
    /// Where in the token's value (empty: the whole value; `["fontSize"]`, `["0", "color"]`…).
    sub: Vec<String>,
    kind: TokenType,
    at: egui::Pos2,
    query: String,
}

/// What a field of a composite token holds.
#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Token(TokenType),
    Bool,
}

type Field = (&'static str, FieldKind, &'static str);

const TYPOGRAPHY: &[Field] = &[
    ("fontFamily", FieldKind::Token(TokenType::FontFamily), tr_noop("Font family")),
    ("fontSize", FieldKind::Token(TokenType::Dimension), tr_noop("Font size")),
    ("fontWeight", FieldKind::Token(TokenType::FontWeight), tr_noop("Font weight")),
    ("letterSpacing", FieldKind::Token(TokenType::Dimension), tr_noop("Letter spacing")),
    ("lineHeight", FieldKind::Token(TokenType::Number), tr_noop("Line height")),
];
const BORDER: &[Field] = &[
    ("color", FieldKind::Token(TokenType::Color), tr_noop("Color")),
    ("width", FieldKind::Token(TokenType::Dimension), tr_noop("Width")),
    ("style", FieldKind::Token(TokenType::StrokeStyle), tr_noop("Style")),
];
const TRANSITION: &[Field] = &[
    ("duration", FieldKind::Token(TokenType::Duration), tr_noop("Duration")),
    ("delay", FieldKind::Token(TokenType::Duration), tr_noop("Delay")),
    ("timingFunction", FieldKind::Token(TokenType::CubicBezier), tr_noop("Timing function")),
];
const SHADOW: &[Field] = &[
    ("color", FieldKind::Token(TokenType::Color), tr_noop("Color")),
    ("offsetX", FieldKind::Token(TokenType::Dimension), tr_noop("Offset X")),
    ("offsetY", FieldKind::Token(TokenType::Dimension), tr_noop("Offset Y")),
    ("blur", FieldKind::Token(TokenType::Dimension), tr_noop("Blur")),
    ("spread", FieldKind::Token(TokenType::Dimension), tr_noop("Spread")),
    ("inset", FieldKind::Bool, tr_noop("Inner shadow")),
];
const STOP: &[Field] =
    &[("color", FieldKind::Token(TokenType::Color), tr_noop("Color")), ("position", FieldKind::Token(TokenType::Number), tr_noop("Position"))];

/// The fields of a composite value of type `kind` (object types only), with their labels.
fn fields(kind: TokenType) -> &'static [Field] {
    match kind {
        TokenType::Typography => TYPOGRAPHY,
        TokenType::Border => BORDER,
        TokenType::Transition => TRANSITION,
        TokenType::Shadow => SHADOW,
        TokenType::Gradient => STOP,
        _ => &[],
    }
}

/// `value` with the part at `sub` replaced by `new` (made where it's missing).
fn with_part(value: &Value, sub: &[String], new: Value) -> Value {
    let Some((last, parents)) = sub.split_last() else { return new };
    let mut out = value.clone();
    let mut cur = &mut out;
    for p in parents {
        let next = match cur {
            Value::Object(o) => o.entry(p.clone()).or_insert_with(|| json!({})),
            Value::Array(a) => match p.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                Some(x) => x,
                None => return out,
            },
            _ => return out,
        };
        cur = next;
    }
    match cur {
        Value::Object(o) => {
            o.insert(last.clone(), new);
        }
        Value::Array(a) => {
            if let Some(x) = last.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                *x = new;
            }
        }
        other => *other = json!({ last.clone(): new }),
    }
    out
}

/// Set the part `sub` of token `path`'s value (as written in `src`) to `new`.
fn set_part(app: &mut FigApp, src: &Src, path: &str, sub: &[String], new: Value) {
    let raw = app
        .session
        .doc()
        .ok()
        .and_then(|d| d.tokens.tree(&src.doc_source()).and_then(|t| dt::token_at(t, path)).and_then(|t| t.get("$value")).cloned())
        .unwrap_or(Value::Null);
    app.run("token.set", src.with(json!({"path": path, "value": with_part(&raw, sub, new)})));
}

/// A token tree: a set, or a modifier's context.
#[derive(Clone, Debug, PartialEq, Hash)]
enum Src {
    Set(String),
    Context(String, String),
}

impl Src {
    fn params(&self) -> Map<String, Value> {
        let mut m = Map::new();
        match self {
            Src::Set(s) => {
                m.insert("set".into(), json!(s));
            }
            Src::Context(md, c) => {
                m.insert("modifier".into(), json!(md));
                m.insert("context".into(), json!(c));
            }
        }
        m
    }

    fn with(&self, extra: Value) -> Value {
        let mut m = self.params();
        if let Value::Object(e) = extra {
            m.extend(e);
        }
        Value::Object(m)
    }

    fn doc_source(&self) -> TokenSource {
        match self {
            Src::Set(s) => TokenSource::Set(s.clone()),
            Src::Context(m, c) => TokenSource::Context { modifier: m.clone(), context: c.clone() },
        }
    }

    fn exists(&self, t: &DesignTokens) -> bool {
        t.tree(&self.doc_source()).is_some()
    }
}

/// The first token tree in resolution order.
fn first_source(t: &DesignTokens) -> Option<Src> {
    t.sources().into_iter().next().map(|s| match s {
        TokenSource::Set(s) => Src::Set(s),
        TokenSource::Context { modifier, context } => Src::Context(modifier, context),
    })
}

/// The input the table resolves with: the file's contexts, the shown context for its modifier.
fn view_input(t: &DesignTokens, src: Option<&Src>) -> BTreeMap<String, String> {
    let over = match src {
        Some(Src::Context(m, c)) => [(m.clone(), c.clone())].into_iter().collect(),
        _ => BTreeMap::new(),
    };
    t.input_for(&over)
}

/// Show the editor while it's open (call once per frame).
pub fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut st) = app.ui.tokens.take() else { return };
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else {
        app.ui.tokens = Some(st);
        return;
    };
    if !st.source.as_ref().is_some_and(|s| s.exists(&doc.tokens)) {
        st.source = first_source(&doc.tokens);
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let size = vec2(960.0_f32.min(screen.width() - 80.0).max(520.0), 540.0_f32.min(screen.height() - 120.0).max(260.0));
    let mut open = true;
    egui::Window::new(tr("Design tokens"))
        .id(Id::new("tokens_window"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(app, ui, &doc, &mut st, &mut open);
            let h = (size.y - 44.0).max(160.0);
            let top_down = egui::Layout::top_down(egui::Align::Min);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(SIDEBAR_W, h), top_down, |ui| sidebar(app, ui, &doc.tokens, &mut st));
                let (line, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
                ui.allocate_ui_with_layout(vec2(size.x - SIDEBAR_W - 1.0, h), top_down, |ui| table(app, ui, &doc, &mut st));
            });
        });
    details(app, ctx, &doc, &mut st);
    paste_dialog(app, ctx, &mut st);
    take_upload(app, ctx, &mut st);
    alias_picker(app, ctx, &doc, &mut st);
    app.ui.tokens = open.then_some(st);
}

fn header(app: &mut FigApp, ui: &mut Ui, doc: &Document, st: &mut Editor, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Design tokens"), semibold(13.0), t.text);
    // The context of each modifier this file uses.
    let ctx_r = Rect::from_min_max(pos2(r.min.x + 130.0, r.min.y + 10.0), pos2(r.max.x - 250.0, r.max.y - 10.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(ctx_r).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let active = doc.tokens.input_for(&BTreeMap::new());
    for m in &doc.tokens.modifiers {
        let cur = active.get(&m.name).map_or("", String::as_str);
        let label = format!("{}: {cur}", m.name);
        let resp = widgets::dropdown(&mut bar, &label, 150.0, None).on_hover_text(tr("The context this file uses"));
        crate::right::menu_popup(&resp).show(|ui| {
            for c in &m.contexts {
                if menu_check(ui, c.name == cur, &c.name, "") {
                    app.run("token.setContext", json!({"modifier": m.name, "context": c.name}));
                    ui.close();
                }
            }
        });
        bar.add_space(6.0);
    }
    let right = Rect::from_min_max(pos2(r.max.x - 240.0, r.min.y + 10.0), pos2(r.max.x - 40.0, r.max.y - 10.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let export = widgets::button(&mut bar, tr("Export…"), false, true);
    crate::right::menu_popup(&export).show(|ui| {
        if menu_item(ui, tr("ZIP: resolver and a file per set and context…"), "") {
            export_zip(app);
            ui.close();
        }
        if let Some(src) = &st.source
            && menu_item(ui, tr("Token file of this set or context…"), "")
        {
            export_dialog(app, src.with(json!({"format": "tokens"})), "tokens");
            ui.close();
        }
        if menu_item(ui, tr("Resolved token file (this file's contexts)…"), "") {
            export_dialog(app, json!({"format": "tokens", "input": active, "resolveAliases": true}), "tokens");
            ui.close();
        }
    });
    bar.add_space(6.0);
    let import = widgets::button(&mut bar, tr("Import…"), false, true)
        .on_hover_text(tr("A ZIP of sets and modifiers, a resolver document or a token file. Files can also be dropped here."));
    crate::right::menu_popup(&import).show(|ui| {
        if menu_item(ui, tr("ZIP or JSON file…"), "") {
            import_file(app, st, None);
            ui.close();
        }
        if let Some(src) = st.source.clone()
            && menu_item(ui, tr("JSON file into this set or context…"), "")
        {
            import_file(app, st, Some(src));
            ui.close();
        }
        if menu_item(ui, tr("Paste JSON…"), "") {
            st.paste = Some(String::new());
            ui.close();
        }
    });
    if widgets::icon_button_at(
        ui,
        Id::new("tokens_close"),
        Rect::from_center_size(pos2(r.max.x - 22.0, r.center().y), vec2(24.0, 24.0)),
        Icon::Close,
        tr("Close"),
    ) {
        *open = false;
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

/// Export through a save dialog, or as a download where the platform has no save dialogs.
fn export_dialog(app: &mut FigApp, params: Value, kind: &str) {
    let base = app.session.doc().map(|d| d.name.clone()).unwrap_or_else(|_| "tokens".into());
    let name = format!("{base}.{kind}.json");
    if app.services.downloads() {
        let Some(r) = app.run("token.export", params) else { return };
        let text = r["text"].as_str().unwrap_or_default();
        if app.services.download(&name, text.as_bytes()) {
            app.toast(trf("Exported {file}", &[("file", &name)]));
        } else {
            app.toast(tr("Couldn't download the design tokens"));
        }
        return;
    }
    app.pick_save(tr("Export design tokens"), &name, &["json"], move |app, p| {
        let mut params = params;
        if let Some(o) = params.as_object_mut() {
            o.insert("path".into(), json!(p.to_string_lossy()));
        }
        if app.run("token.export", params).is_some() {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            app.toast(trf("Exported {file}", &[("file", &file)]));
        }
    });
}

/// Export every set and modifier as a ZIP: the resolver document and a token file each.
fn export_zip(app: &mut FigApp) {
    let base = app.session.doc().map(|d| d.name.clone()).unwrap_or_else(|_| "tokens".into());
    let name = format!("{base}.tokens.zip");
    if app.services.downloads() {
        use base64::Engine as _;
        let Some(r) = app.run("token.export", json!({"format": "zip"})) else { return };
        let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or_default()).unwrap_or_default();
        if !bytes.is_empty() && app.services.download(&name, &bytes) {
            app.toast(trf("Exported {file}", &[("file", &name)]));
        } else {
            app.toast(tr("Couldn't download the design tokens"));
        }
        return;
    }
    app.pick_save(tr("Export design tokens"), &name, &["zip"], move |app, p| {
        if app.run("token.export", json!({"format": "zip", "path": p.to_string_lossy()})).is_some() {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            app.toast(trf("Exported {file}", &[("file", &file)]));
        }
    });
}

/// Import a ZIP or JSON file (`into`: a token file goes into that set or context). The browser
/// has no paths: the file's bytes arrive in a later frame ([`take_upload`]).
fn import_file(app: &mut FigApp, st: &mut Editor, into: Option<Src>) {
    let exts: &[&str] = if into.is_some() { &["json"] } else { &["zip", "json"] };
    if app.services.downloads() {
        if app.services.pick_open_bytes(exts) {
            st.upload = Some(into);
        }
        return;
    }
    app.pick_open(tr("Import design tokens"), exts, move |app, p| {
        let mut params = into.as_ref().map(Src::params).unwrap_or_default();
        params.insert("tokens".into(), json!(p.to_string_lossy()));
        let r = app.run("token.import", Value::Object(params));
        import_toast(app, r);
    });
}

/// Import a file read in the browser, or dropped on the editor, as bytes.
fn import_bytes(app: &mut FigApp, into: Option<&Src>, bytes: &[u8]) {
    use base64::Engine as _;
    let mut params = into.map(Src::params).unwrap_or_default();
    params.insert("data".into(), json!(base64::engine::general_purpose::STANDARD.encode(bytes)));
    let r = app.run("token.import", Value::Object(params));
    import_toast(app, r);
}

/// A picked file the browser has finished reading, and files dropped while the editor is open.
fn take_upload(app: &mut FigApp, ctx: &Context, st: &mut Editor) {
    if let Some(into) = st.upload.clone()
        && let Some((_, bytes)) = app.services.take_opened_bytes()
    {
        st.upload = None;
        import_bytes(app, into.as_ref(), &bytes);
    }
    for f in ctx.input(|i| i.raw.dropped_files.clone()) {
        let path = f.path().to_path_buf();
        if path.exists() {
            let r = app.run("token.import", json!({"tokens": path.to_string_lossy()}));
            import_toast(app, r);
        } else if let Some(bytes) = crate::dropped_bytes(&f) {
            import_bytes(app, None, &bytes);
        }
    }
}

fn import_toast(app: &mut FigApp, r: Option<Value>) {
    let Some(r) = r else { return };
    let n = r["tokens"].as_u64().unwrap_or(0) as usize;
    let warnings = r["warnings"].as_array().map_or(0, Vec::len);
    let mut msg = trn("{n} design tokens in this file", n, &[]);
    if warnings > 0 {
        msg = format!("{msg} {}", trn("({n} warnings: some entries were skipped)", warnings, &[]));
    }
    app.toast(msg);
}

/// The dialog JSON is pasted into: a token file goes into the shown set or context.
fn paste_dialog(app: &mut FigApp, ctx: &Context, st: &mut Editor) {
    let Some(mut text) = st.paste.take() else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut done = false;
    egui::Window::new(tr("Paste JSON"))
        .id(Id::new("tokens_paste"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(400.0);
            widgets::label(ui, tr("A DTCG token file or resolver document. A token file joins the set or context shown."), t.text_secondary);
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(400.0).desired_rows(12));
            });
            ui.add_space(8.0);
            if widgets::button(ui, tr("Import"), true, !text.trim().is_empty()).clicked() {
                let mut p = st.source.as_ref().map(Src::params).unwrap_or_default();
                p.insert("tokens".into(), json!(text));
                let r = app.run("token.import", Value::Object(p));
                if r.is_some() {
                    done = true;
                }
                import_toast(app, r);
            }
        });
    if open && !done {
        st.paste = Some(text);
    }
}

fn rename_id(target: &Rename) -> Id {
    Id::new(("token_rename", target))
}

fn start_rename(st: &mut Editor, target: Rename) {
    st.rename = Some(target);
    st.rename_start = true;
}

fn rename_field(ui: &mut Ui, st: &mut Editor, target: &Rename, value: &str, rect: Rect, font: egui::FontId) -> Option<String> {
    let start = std::mem::take(&mut st.rename_start);
    let (out, open) = widgets::rename_field(ui, rename_id(target), value, rect, font, start);
    if !open {
        st.rename = None;
    }
    out
}

fn sidebar(app: &mut FigApp, ui: &mut Ui, tokens: &DesignTokens, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (hr, _) = ui.allocate_exact_size(vec2(SIDEBAR_W, 40.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Resolution order"), semibold(11.0), t.text_secondary);
    ui.interact(hr, Id::new("tokens_order_tip"), Sense::hover()).on_hover_text(tr("Later sets and modifiers override earlier ones"));
    let add = Rect::from_center_size(pos2(hr.max.x - 20.0, hr.center().y), vec2(24.0, 24.0));
    let resp = ui.interact(add, Id::new("tokens_add"), Sense::click()).on_hover_text(tr("Create set or modifier"));
    if resp.hovered() {
        ui.painter().rect_filled(add, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), add.shrink(5.0), Icon::Plus, t.icon);
    crate::right::menu_popup(&resp).show(|ui| {
        if menu_item(ui, tr("New set"), "") {
            if let Some(n) = app.run("token.createSet", json!({})).and_then(|r| r["name"].as_str().map(str::to_string)) {
                st.source = Some(Src::Set(n.clone()));
                start_rename(st, Rename::Set(n));
            }
            ui.close();
        }
        if menu_item(ui, tr("New modifier (theme, brand…)"), "") {
            if let Some(n) = app.run("token.createModifier", json!({})).and_then(|r| r["name"].as_str().map(str::to_string)) {
                start_rename(st, Rename::Modifier(n));
            }
            ui.close();
        }
    });
    egui::ScrollArea::vertical().id_salt("tokens_sources").auto_shrink([false, false]).show(ui, |ui| {
        let order = tokens.full_order();
        if order.is_empty() {
            ui.add_space(8.0);
            widgets::note(ui, tr("Create a set for your tokens, and modifiers for themes or brands."), t.text_secondary);
        }
        let n = order.len();
        for (i, item) in order.iter().enumerate() {
            match item {
                OrderItem::Set(s) => set_row(app, ui, tokens, s, i, n, st),
                OrderItem::Modifier(m) => modifier_rows(app, ui, tokens, m, i, n, st),
            }
        }
    });
}

/// Move up and down in the resolution order.
fn order_items(app: &mut FigApp, ui: &mut Ui, key: (&str, &str), i: usize, n: usize) {
    if i > 0 && menu_item(ui, tr("Move up"), "") {
        app.run("token.moveOrder", json!({key.0: key.1, "to": i - 1}));
        ui.close();
    }
    if i + 1 < n && menu_item(ui, tr("Move down"), "") {
        app.run("token.moveOrder", json!({key.0: key.1, "to": i + 1}));
        ui.close();
    }
}

fn source_row(ui: &mut Ui, label: &str, count: usize, indent: f32, selected: bool, bold: bool, icon: Option<Icon>) -> (egui::Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(SIDEBAR_W, 28.0), Sense::click());
    let r = r.shrink2(vec2(8.0, 1.0));
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.selected_row);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    if let Some(icon) = icon {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + indent + 6.0, r.center().y), vec2(12.0, 12.0)), icon, t.icon_secondary);
    }
    let name_r = Rect::from_min_max(pos2(r.min.x + indent + 18.0, r.min.y + 3.0), pos2(r.max.x - 30.0, r.max.y - 3.0));
    if count > 0 {
        ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, count.to_string(), regular(11.0), t.text_tertiary);
    }
    let font = if bold || selected { semibold(11.0) } else { regular(11.0) };
    ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, label, font, t.text);
    (resp, name_r)
}

fn set_row(app: &mut FigApp, ui: &mut Ui, tokens: &DesignTokens, s: &str, i: usize, n: usize, st: &mut Editor) {
    let count = tokens.set(s).map_or(0, |x| dt::flatten(&x.tokens).len());
    let selected = st.source == Some(Src::Set(s.to_string()));
    let target = Rename::Set(s.to_string());
    let renaming = st.rename.as_ref() == Some(&target);
    let (resp, name_r) = source_row(ui, if renaming { "" } else { s }, count, 0.0, selected, false, Some(Icon::Styles));
    if renaming
        && let Some(name) = rename_field(ui, st, &target, s, name_r, regular(11.0))
        && app.run("token.renameSet", json!({"set": s, "name": name})).is_some()
    {
        st.source = Some(Src::Set(name));
    }
    if resp.clicked() {
        st.source = Some(Src::Set(s.to_string()));
    }
    if resp.double_clicked() {
        start_rename(st, target.clone());
    }
    crate::right::context_popup(&resp).show(|ui| {
        if menu_item(ui, tr("Rename"), "") {
            start_rename(st, target.clone());
            ui.close();
        }
        order_items(app, ui, ("set", s), i, n);
        if menu_item(ui, tr("Delete set"), "") {
            app.run("token.removeSet", json!({"set": s}));
            ui.close();
        }
    });
}

fn modifier_rows(app: &mut FigApp, ui: &mut Ui, tokens: &DesignTokens, m: &str, i: usize, n: usize, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let Some(modi) = tokens.modifier(m) else { return };
    let target = Rename::Modifier(m.to_string());
    let renaming = st.rename.as_ref() == Some(&target);
    let (resp, name_r) = source_row(ui, if renaming { "" } else { m }, 0, 0.0, false, true, Some(Icon::Variable));
    if renaming && let Some(name) = rename_field(ui, st, &target, m, name_r, semibold(11.0)) {
        app.run("token.renameModifier", json!({"modifier": m, "name": name}));
    }
    resp.clone().on_hover_text(tr("A modifier: one of its contexts applies at a time"));
    if resp.double_clicked() {
        start_rename(st, target.clone());
    }
    crate::right::context_popup(&resp).show(|ui| {
        if menu_item(ui, tr("Rename"), "") {
            start_rename(st, target.clone());
            ui.close();
        }
        if menu_item(ui, tr("Add context"), "") {
            if let Some(c) = app.run("token.addContext", json!({"modifier": m})).and_then(|r| r["name"].as_str().map(str::to_string)) {
                st.source = Some(Src::Context(m.to_string(), c.clone()));
                start_rename(st, Rename::Context(m.to_string(), c));
            }
            ui.close();
        }
        order_items(app, ui, ("modifier", m), i, n);
        if menu_item(ui, tr("Delete modifier"), "") {
            app.run("token.removeModifier", json!({"modifier": m}));
            ui.close();
        }
    });
    let active = tokens.input_for(&BTreeMap::new());
    for c in &modi.contexts {
        let selected = st.source == Some(Src::Context(m.to_string(), c.name.clone()));
        let target = Rename::Context(m.to_string(), c.name.clone());
        let renaming = st.rename.as_ref() == Some(&target);
        let count = dt::flatten(&c.tokens).len();
        let (resp, name_r) = source_row(ui, if renaming { "" } else { &c.name }, count, 14.0, selected, false, None);
        // The file's context is marked, the default one named.
        let on = active.get(m) == Some(&c.name);
        let dot = pos2(name_r.min.x - 10.0, name_r.center().y);
        if on {
            ui.painter().circle_filled(dot, 3.0, t.accent);
        } else {
            ui.painter().circle_stroke(dot, 3.0, Stroke::new(1.0, t.border_strong));
        }
        if modi.default.as_deref() == Some(c.name.as_str()) && !renaming {
            let w = egui::WidgetText::from(c.name.as_str()).into_galley(ui, None, f32::INFINITY, regular(11.0)).size().x;
            ui.painter().text(pos2(name_r.min.x + w + 6.0, name_r.center().y), Align2::LEFT_CENTER, tr("default"), regular(10.0), t.text_tertiary);
        }
        if renaming
            && let Some(name) = rename_field(ui, st, &target, &c.name, name_r, regular(11.0))
            && app.run("token.renameContext", json!({"modifier": m, "context": c.name, "name": name})).is_some()
        {
            st.source = Some(Src::Context(m.to_string(), name));
        }
        if resp.clicked() {
            st.source = Some(Src::Context(m.to_string(), c.name.clone()));
        }
        if resp.double_clicked() {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Use in this file"), "") {
                app.run("token.setContext", json!({"modifier": m, "context": c.name}));
                ui.close();
            }
            if menu_item(ui, tr("Set as default"), "") {
                app.run("token.setDefaultContext", json!({"modifier": m, "context": c.name}));
                ui.close();
            }
            if menu_item(ui, tr("Rename"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if menu_item(ui, tr("Duplicate"), "") {
                app.run(
                    "token.addContext",
                    json!({"modifier": m, "copy": c.name, "name": unique(&format!("{} copy", c.name), |x| modi.context(x).is_some())}),
                );
                ui.close();
            }
            if modi.contexts.len() > 1 && menu_item(ui, tr("Delete context"), "") {
                app.run("token.removeContext", json!({"modifier": m, "context": c.name}));
                ui.close();
            }
        });
    }
}

/// The first of `base`, `base 2`, `base 3`… that `taken` doesn't hold.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..10_000).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn table(app: &mut FigApp, ui: &mut Ui, doc: &Document, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let Some(src) = st.source.clone() else {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.set_max_width(420.0);
                widgets::label(
                    ui,
                    tr("Design tokens follow the Design Tokens Community Group format: sets of tokens, and modifiers whose contexts (light and dark, brands…) override them."),
                    t.text_secondary,
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, tr("New set"), true, true).clicked() {
                        app.run("token.createSet", json!({"name": "base"}));
                    }
                    ui.add_space(8.0);
                    if widgets::button(ui, tr("New modifier (theme, brand…)"), false, true).clicked() {
                        app.run("token.createModifier", json!({"name": "theme", "contexts": ["light", "dark"]}));
                    }
                });
            });
        });
        return;
    };
    let Some(tree) = doc.tokens.tree(&src.doc_source()) else { return };
    let input = view_input(&doc.tokens, Some(&src));
    let resolved = doc.tokens.resolve(&input);
    // Search.
    let (sr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let field = Rect::from_min_size(pos2(sr.min.x + 16.0, sr.center().y - 12.0), vec2(220.0, 24.0));
    ui.painter().rect_filled(field, CornerRadius::same(5), t.field);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(field.min.x + 13.0, field.center().y), vec2(12.0, 12.0)), Icon::Search, t.icon_secondary);
    let mut child = ui.new_child(
        UiBuilder::new().max_rect(Rect::from_min_max(pos2(field.min.x + 24.0, field.min.y + 4.0), pos2(field.max.x - 4.0, field.max.y - 4.0))),
    );
    let r = child.add(
        egui::TextEdit::singleline(&mut st.query)
            .frame(egui::Frame::NONE)
            .hint_text(tr("Search"))
            .font(regular(11.0))
            .vertical_align(egui::Align::Center),
    );
    widgets::select_all_on_focus(ui, &r, &st.query);
    let where_ = match &src {
        Src::Set(s) => trf("Set \"{name}\"", &[("name", s)]),
        Src::Context(m, c) => trf("{modifier} · {context}: overrides the sets before it", &[("modifier", m), ("context", c)]),
    };
    ui.painter().text(pos2(field.max.x + 16.0, sr.center().y), Align2::LEFT_CENTER, where_, regular(11.0), t.text_secondary);
    let q = st.query.trim().to_lowercase();
    let list: Vec<dt::RawToken> = dt::flatten(tree).into_iter().filter(|x| q.is_empty() || x.path.to_lowercase().contains(&q)).collect();
    let width = (NAME_W + VALUE_W + 260.0).max(ui.available_width());
    egui::ScrollArea::both().id_salt("tokens_table").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        header_row(ui, width);
        let mut group: Option<&str> = None;
        for tok in &list {
            let g = crate::tokens_ui::group_of(&tok.path);
            if group != Some(g) {
                group = Some(g);
                if !g.is_empty() {
                    group_row(app, ui, &src, g, width, st);
                }
            }
            token_row(app, ui, &src, tok, resolved.tokens.get(&tok.path), width, st);
        }
        if list.is_empty() && !q.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("No matching tokens"), regular(11.0), t.text_secondary);
        }
        create_row(app, ui, &src, tree, st);
    });
}

fn header_row(ui: &mut Ui, width: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let p = ui.painter();
    p.text(pos2(r.min.x + 40.0, r.center().y), Align2::LEFT_CENTER, tr("Name"), semibold(11.0), t.text_secondary);
    p.text(pos2(r.min.x + NAME_W, r.center().y), Align2::LEFT_CENTER, tr("Value"), semibold(11.0), t.text_secondary);
    p.text(pos2(r.min.x + NAME_W + VALUE_W + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Resolved"), semibold(11.0), t.text_secondary);
    p.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

fn group_row(app: &mut FigApp, ui: &mut Ui, src: &Src, g: &str, width: f32, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let target = Rename::Group(g.to_string());
    let name_r = Rect::from_min_max(pos2(r.min.x + 16.0, r.min.y + 4.0), pos2(r.min.x + NAME_W + VALUE_W, r.max.y - 2.0));
    if st.rename.as_ref() == Some(&target) {
        if let Some(to) = rename_field(ui, st, &target, g, name_r, semibold(11.0)) {
            app.run("token.rename", src.with(json!({"path": g, "to": to})));
        }
    } else {
        ui.painter().text(pos2(r.min.x + 16.0, r.center().y + 2.0), Align2::LEFT_CENTER, g.replace('.', " / "), semibold(11.0), t.text_secondary);
    }
    if resp.double_clicked() {
        start_rename(st, target.clone());
    }
    crate::right::context_popup(&resp).show(|ui| {
        if menu_item(ui, tr("Rename group"), "") {
            start_rename(st, target.clone());
            ui.close();
        }
        if menu_item(ui, tr("Delete group"), "") {
            app.run("token.remove", src.with(json!({"path": g})));
            ui.close();
        }
    });
}

/// The text a token's written value is edited as: an alias as written, simple values as the
/// editor shows them, composite ones as JSON.
fn edit_text(kind: Option<TokenType>, v: &Value) -> String {
    if let Some(a) = dt::alias_of(v) {
        return format!("{{{a}}}");
    }
    match kind {
        Some(k) if k.is_composite() => v.to_string(),
        Some(TokenType::Color) => {
            dt::color_of(v).map(|c| if c.a < 1.0 { c.to_css_hex() } else { c.with_alpha(1.0).to_css_hex() }).unwrap_or_else(|| v.to_string())
        }
        Some(TokenType::CubicBezier) => {
            v.as_array().map(|a| a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")).unwrap_or_else(|| v.to_string())
        }
        Some(TokenType::FontFamily) => match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            _ => v.as_str().map_or_else(|| v.to_string(), str::to_string),
        },
        Some(TokenType::FontWeight | TokenType::StrokeStyle) => v.as_str().map_or_else(|| v.to_string(), str::to_string),
        k => dt::display(k, v),
    }
}

/// A typed edit as the value `token.set` takes.
fn parse_edit(kind: Option<TokenType>, text: &str) -> Value {
    let s = text.trim();
    if s.starts_with('{') && s.ends_with('}') && dt::alias_of(&json!(s)).is_some() {
        return json!(s);
    }
    if (s.starts_with('{') || s.starts_with('['))
        && let Ok(v) = serde_json::from_str::<Value>(s)
    {
        return v;
    }
    match kind {
        Some(TokenType::Number) => s.parse::<f64>().ok().filter(|x| x.is_finite()).map_or_else(|| json!(s), |x| json!(x)),
        Some(TokenType::FontWeight) => s.parse::<f64>().ok().filter(|x| x.is_finite()).map_or_else(|| json!(s), |x| json!(x)),
        Some(TokenType::Duration) => {
            let (n, unit) = match s.strip_suffix("ms") {
                Some(n) => (n, "ms"),
                None => match s.strip_suffix('s') {
                    Some(n) => (n, "s"),
                    None => (s, "ms"),
                },
            };
            n.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map_or_else(|| json!(s), |x| json!({"value": x, "unit": unit}))
        }
        Some(TokenType::CubicBezier) => {
            let nums: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if nums.len() == 4 { json!(nums) } else { json!(s) }
        }
        Some(TokenType::FontFamily) if s.contains(',') => {
            json!(s.split(',').map(|x| x.trim().trim_matches(['"', '\''])).filter(|x| !x.is_empty()).collect::<Vec<_>>())
        }
        _ => json!(s),
    }
}

fn id_of_fold(key: &str) -> Id {
    Id::new(("token_fold", key))
}

fn token_row(app: &mut FigApp, ui: &mut Ui, src: &Src, tok: &dt::RawToken, res: Option<&ResolvedToken>, width: f32, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    if resp.hovered() || st.details.as_deref() == Some(tok.path.as_str()) {
        ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
    }
    // The resolved value's preview.
    let sw = Rect::from_center_size(pos2(r.min.x + 24.0, r.center().y), vec2(14.0, 14.0));
    if let Some(res) = res {
        crate::tokens_ui::swatch(ui, sw, res);
    }
    let name_r = Rect::from_min_max(pos2(r.min.x + 40.0, r.min.y + 6.0), pos2(r.min.x + NAME_W - 8.0, r.max.y - 6.0));
    let target = Rename::Token(tok.path.clone());
    let short = tok.path.rsplit('.').next().unwrap_or(&tok.path);
    if st.rename.as_ref() == Some(&target) {
        if let Some(name) = rename_field(ui, st, &target, short, name_r, regular(11.0)) {
            // A name with dots is a whole new path; otherwise it renames within the group.
            let g = crate::tokens_ui::group_of(&tok.path);
            let to = if name.contains('.') || g.is_empty() { name } else { format!("{g}.{name}") };
            app.run("token.rename", src.with(json!({"path": tok.path, "to": to})));
        }
    } else {
        let p = ui.painter().with_clip_rect(name_r);
        p.text(name_r.left_center(), Align2::LEFT_CENTER, short, regular(11.0), if tok.deprecated { t.text_tertiary } else { t.text });
    }
    // The value as written: composite values show their fields on rows of their own.
    let body = Rect::from_min_max(pos2(r.min.x + NAME_W, r.center().y - 12.0), pos2(r.min.x + NAME_W + VALUE_W - 8.0, r.center().y + 12.0));
    let id = Id::new(("token_cell", &tok.path, format!("{src:?}")));
    let expanded = tok.kind.is_some_and(|k| k.is_composite() && k != TokenType::StrokeStyle) && dt::alias_of(&tok.value).is_none();
    // Composite tokens fold their fields away; the chevron left of the preview shows them.
    let fold_key = format!("{src:?}/{}", tok.path);
    let unfolded = st.open_fields.contains(&fold_key);
    if expanded {
        let chev = Rect::from_center_size(pos2(r.min.x + 9.0, r.center().y), vec2(14.0, 14.0));
        let tip = if unfolded { tr("Hide fields") } else { tr("Show fields") };
        let cresp = ui.interact(chev.expand(3.0), id_of_fold(&fold_key), Sense::click()).on_hover_text(tip);
        let icon = if unfolded { Icon::Chevron } else { Icon::ChevronRight };
        icons::paint(ui.painter(), chev.shrink(2.0), icon, if cresp.hovered() { t.icon } else { t.icon_secondary });
        if cresp.clicked() {
            if unfolded {
                st.open_fields.remove(&fold_key);
            } else {
                st.open_fields.insert(fold_key.clone());
            }
        }
    }
    match tok.kind {
        Some(k) if expanded => {
            ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
            let text_r = body.shrink2(vec2(8.0, 4.0));
            let summary = res.map(|x| dt::display(x.kind, &x.value)).unwrap_or_default();
            ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, summary, regular(11.0), t.text_secondary);
            alias_button(ui, st, id, body, &tok.path, &[], k);
        }
        Some(k) => value_cell(app, ui, st, src, &tok.path, &[], FieldKind::Token(k), &tok.value, res.map(|x| &x.value), body, id),
        None => {
            if let Some(text) =
                widgets::text_field_inline(ui, id.with("value"), &edit_text(None, &tok.value), body.shrink2(vec2(8.0, 4.0)), regular(11.0))
            {
                app.run("token.set", src.with(json!({"path": tok.path, "value": parse_edit(None, &text)})));
            }
        }
    }
    // What it resolves to here, or why it doesn't.
    let res_r = Rect::from_min_max(pos2(r.min.x + NAME_W + VALUE_W + 16.0, r.min.y), pos2(r.max.x - 12.0, r.max.y));
    let (text, color) = match res {
        Some(x) if x.error.is_some() => (x.error.clone().unwrap_or_default(), t.danger),
        Some(x) => (format!("{} · {}", type_label(x.kind), dt::display(x.kind, &x.value)), t.text_secondary),
        None => (String::new(), t.text_secondary),
    };
    ui.painter().with_clip_rect(res_r).text(res_r.left_center(), Align2::LEFT_CENTER, &text, regular(11.0), color);
    let resp = if tok.description.is_empty() { resp } else { resp.on_hover_text(&tok.description) };
    if resp.double_clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < name_r.max.x) {
        start_rename(st, target.clone());
    }
    crate::right::context_popup(&resp).show(|ui| {
        if menu_item(ui, tr("Edit token"), "") {
            st.details = Some(tok.path.clone());
            st.json = None;
            ui.close();
        }
        if menu_item(ui, tr("Rename"), "") {
            start_rename(st, target.clone());
            ui.close();
        }
        if menu_item(ui, tr("Copy alias"), "") {
            app.services.clipboard_set_text(&format!("{{{}}}", tok.path));
            ui.close();
        }
        if menu_item(ui, tr("Delete token"), "") {
            app.run("token.remove", src.with(json!({"path": tok.path})));
            ui.close();
        }
    });
    if expanded
        && unfolded
        && let Some(k) = tok.kind
    {
        field_rows(app, ui, st, src, tok, res, k, width);
    }
    let bottom = ui.cursor().min.y;
    ui.painter().line_segment([pos2(r.min.x, bottom), pos2(r.max.x, bottom)], Stroke::new(1.0, t.border));
}

/// A composite token's fields: one row each (shadow layers and gradient stops in turn, with
/// buttons to add and remove them).
#[allow(clippy::too_many_arguments)]
fn field_rows(
    app: &mut FigApp,
    ui: &mut Ui,
    st: &mut Editor,
    src: &Src,
    tok: &dt::RawToken,
    res: Option<&ResolvedToken>,
    kind: TokenType,
    width: f32,
) {
    let t = Tokens::get(ui.ctx());
    let resolved = res.map(|x| &x.value);
    // Lists: a shadow's layers (one object is one layer), a gradient's stops.
    let list = match (kind, &tok.value) {
        (TokenType::Shadow | TokenType::Gradient, Value::Array(a)) => Some(a.len()),
        (TokenType::Shadow, _) => None,
        (TokenType::Gradient, _) => Some(0),
        _ => None,
    };
    let row = |ui: &mut Ui, label: &str, indent: f32| -> Rect {
        let (r, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
        ui.painter().text(pos2(r.min.x + 40.0 + indent, r.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.text_secondary);
        r
    };
    let one = |app: &mut FigApp, ui: &mut Ui, st: &mut Editor, prefix: &[String], indent: f32| {
        for (key, fk, label) in fields(kind) {
            let r = row(ui, tr(label), indent);
            let sub: Vec<String> = prefix.iter().cloned().chain(std::iter::once((*key).to_string())).collect();
            let body = Rect::from_min_max(pos2(r.min.x + NAME_W, r.center().y - 11.0), pos2(r.min.x + NAME_W + VALUE_W - 8.0, r.center().y + 11.0));
            let id = Id::new(("token_field", &tok.path, &sub, format!("{src:?}")));
            let raw = dt::at_path(&tok.value, &sub).cloned().unwrap_or(Value::Null);
            let res_part = resolved.and_then(|v| dt::at_path(v, &sub));
            value_cell(app, ui, st, src, &tok.path, &sub, *fk, &raw, res_part, body, id);
            if let (FieldKind::Token(k), Some(v)) = (fk, res_part) {
                let res_r = Rect::from_min_max(pos2(r.min.x + NAME_W + VALUE_W + 16.0, r.min.y), pos2(r.max.x - 12.0, r.max.y));
                ui.painter().with_clip_rect(res_r).text(
                    res_r.left_center(),
                    Align2::LEFT_CENTER,
                    dt::display(Some(*k), v),
                    regular(11.0),
                    t.text_tertiary,
                );
            }
        }
    };
    match list {
        None => one(app, ui, st, &[], 0.0),
        Some(n) => {
            let item = if kind == TokenType::Shadow { tr_noop("Shadow {n}") } else { tr_noop("Stop {n}") };
            for i in 0..n {
                let r = row(ui, &crate::i18n::trf(item, &[("n", &(i + 1))]), 0.0);
                let del = Rect::from_center_size(pos2(r.min.x + NAME_W - 20.0, r.center().y), vec2(20.0, 20.0));
                let min = if kind == TokenType::Gradient { 2 } else { 1 };
                if n > min
                    && widgets::icon_button_at(ui, Id::new(("token_item_del", &tok.path, i)), del, Icon::Minus, tr("Remove"))
                    && let Value::Array(mut a) = tok.value.clone()
                {
                    a.remove(i);
                    app.run("token.set", src.with(json!({"path": tok.path, "value": a})));
                }
                one(app, ui, st, &[i.to_string()], 12.0);
            }
            let r = row(ui, "", 0.0);
            let b = Rect::from_min_size(pos2(r.min.x + 40.0, r.min.y + 4.0), vec2(150.0, 22.0));
            let label = if kind == TokenType::Shadow { tr("+ Add shadow") } else { tr("+ Add stop") };
            if ui.put(b, egui::Button::new(egui::RichText::new(label).font(regular(11.0))).frame(false)).clicked() {
                let mut a = tok.value.as_array().cloned().unwrap_or_default();
                let next = match (kind, a.last()) {
                    (_, Some(last)) => last.clone(),
                    (TokenType::Shadow, None) => TokenType::Shadow.default_value(),
                    _ => json!({"color": dt::color_value(figcraft_color::Color::BLACK), "position": 1}),
                };
                a.push(next);
                app.run("token.set", src.with(json!({"path": tok.path, "value": a})));
            }
        }
    }
    // One shadow becomes a list when another is added.
    if kind == TokenType::Shadow && list.is_none() {
        let r = row(ui, "", 0.0);
        let b = Rect::from_min_size(pos2(r.min.x + 40.0, r.min.y + 4.0), vec2(150.0, 22.0));
        if ui.put(b, egui::Button::new(egui::RichText::new(tr("+ Add shadow")).font(regular(11.0))).frame(false)).clicked() {
            app.run("token.set", src.with(json!({"path": tok.path, "value": [tok.value.clone(), tok.value.clone()]})));
        }
    }
}

/// One value: an alias as a chip (click: pick another; detach: keep what it resolves to), else
/// its field (a colour with its swatch, a checkbox for booleans), with the alias button on hover.
#[allow(clippy::too_many_arguments)]
fn value_cell(
    app: &mut FigApp,
    ui: &mut Ui,
    st: &mut Editor,
    src: &Src,
    path: &str,
    sub: &[String],
    kind: FieldKind,
    raw: &Value,
    resolved: Option<&Value>,
    body: Rect,
    id: Id,
) {
    let t = Tokens::get(ui.ctx());
    let FieldKind::Token(k) = kind else {
        let on = raw.as_bool().unwrap_or(false);
        let mut child = ui.new_child(UiBuilder::new().max_rect(body.shrink2(vec2(4.0, 0.0))));
        if widgets::checkbox(&mut child, on, if on { tr("True") } else { tr("False") }) {
            set_part(app, src, path, sub, json!(!on));
        }
        return;
    };
    let set = |app: &mut FigApp, v: Value| {
        if sub.is_empty() {
            app.run("token.set", src.with(json!({"path": path, "value": v})));
        } else {
            set_part(app, src, path, sub, v);
        }
    };
    ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
    if let Some(target) = dt::alias_of(raw) {
        let chip = body.shrink2(vec2(3.0, 3.0));
        let resp = ui.interact(chip, id.with("alias"), Sense::click()).on_hover_text(tr("Change alias"));
        ui.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
        let border = if resp.hovered() { t.border_strong } else { t.border };
        ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, border), egui::StrokeKind::Inside);
        let mut x = chip.min.x + 6.0;
        if k == TokenType::Color
            && let Some(c) = resolved.and_then(dt::color_of)
        {
            let sw = Rect::from_min_size(pos2(x, chip.center().y - 6.0), vec2(12.0, 12.0));
            widgets::paint_swatch(ui.painter(), sw, &figcraft_color::Paint::solid(c));
            x = sw.max.x + 6.0;
        } else {
            icons::paint(ui.painter(), Rect::from_min_size(pos2(x, chip.center().y - 6.0), vec2(12.0, 12.0)), Icon::Variable, t.icon_secondary);
            x += 18.0;
        }
        let text_r = Rect::from_min_max(pos2(x, chip.min.y), pos2(chip.max.x - 24.0, chip.max.y));
        ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, &target, regular(11.0), t.text);
        if resp.clicked() {
            st.alias = Some(AliasPick { path: path.to_string(), sub: sub.to_vec(), kind: k, at: body.left_bottom(), query: String::new() });
        }
        // Detach: keep the value it resolves to now.
        let det = Rect::from_center_size(pos2(chip.max.x - 11.0, chip.center().y), vec2(18.0, 18.0));
        if let Some(v) = resolved
            && widgets::icon_button_at(ui, id.with("detach"), det, Icon::Detach, tr("Detach alias"))
        {
            set(app, v.clone());
        }
        return;
    }
    let mut text_r = body.shrink2(vec2(8.0, 3.0));
    text_r.max.x -= 22.0;
    if k == TokenType::Color
        && let Some(c) = dt::color_of(raw)
    {
        let cs = Rect::from_center_size(pos2(body.min.x + 13.0, body.center().y), vec2(14.0, 14.0));
        widgets::paint_swatch(ui.painter(), cs, &figcraft_color::Paint::solid(c));
        // The colour picker edits whole colour tokens.
        if sub.is_empty() && ui.interact(cs.expand(3.0), id.with("swatch"), Sense::click()).on_hover_text(tr("Edit colour")).clicked() {
            let (set, modifier, context) = match src {
                Src::Set(s) => (Some(s.clone()), None, None),
                Src::Context(m, c) => (None, Some(m.clone()), Some(c.clone())),
            };
            app.ui.picker = Some(PickerTarget::Token { set, modifier, context, path: path.to_string() });
        }
        text_r.min.x = cs.max.x + 8.0;
    }
    let text = if raw.is_null() { String::new() } else { edit_text(Some(k), raw) };
    if let Some(edit) = widgets::text_field_inline(ui, id.with("value"), &text, text_r, regular(11.0)) {
        set(app, parse_edit(Some(k), &edit));
    }
    alias_button(ui, st, id, body, path, sub, k);
}

/// The alias button at the right end of `body`: opens the alias picker for that value.
fn alias_button(ui: &mut Ui, st: &mut Editor, id: Id, body: Rect, path: &str, sub: &[String], kind: TokenType) {
    // Shown while the pointer is over the value, as on Local variables.
    if !ui.rect_contains_pointer(body.expand(4.0)) {
        return;
    }
    let b = Rect::from_center_size(pos2(body.max.x - 12.0, body.center().y), vec2(20.0, 20.0));
    if widgets::icon_button_at(ui, id.with("mkalias"), b, Icon::Variable, tr("Use another token (alias)")) {
        st.alias = Some(AliasPick { path: path.to_string(), sub: sub.to_vec(), kind, at: body.left_bottom(), query: String::new() });
    }
}

/// The alias picker: tokens of the value's type (resolved as the table shows them), searched by
/// path; picking one makes the value `{its.path}`.
fn alias_picker(app: &mut FigApp, ctx: &Context, doc: &Document, st: &mut Editor) {
    let (Some(mut p), Some(src)) = (st.alias.take(), st.source.clone()) else { return };
    let t = Tokens::get(ctx);
    let input = view_input(&doc.tokens, Some(&src));
    let resolved = doc.tokens.resolve(&input);
    let q = p.query.trim().to_lowercase();
    let fits: Vec<&ResolvedToken> = resolved
        .tokens
        .values()
        .filter(|x| x.path != p.path && x.error.is_none() && x.kind == Some(p.kind) && (q.is_empty() || x.path.to_lowercase().contains(&q)))
        .collect();
    let mut chosen = None;
    let area = egui::Area::new(Id::new("token_alias_area")).order(egui::Order::Foreground).fixed_pos(p.at + vec2(0.0, 4.0)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.menu).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(280.0);
            let title = crate::i18n::trf("Alias: a {type} token", &[("type", &type_label(Some(p.kind)))]);
            ui.label(egui::RichText::new(title).font(semibold(11.0)).color(t.menu_text));
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(tr("Search")).font(regular(11.0)).desired_width(264.0));
            widgets::select_all_on_focus(ui, &r, &p.query);
            if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut group: Option<&str> = None;
                for tok in &fits {
                    let g = crate::tokens_ui::group_of(&tok.path);
                    if group != Some(g) {
                        group = Some(g);
                        if !g.is_empty() {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(g.replace('.', " / ")).font(regular(10.0)).color(t.menu_dim));
                        }
                    }
                    if crate::tokens_ui::token_row(ui, tok, 264.0, false) {
                        chosen = Some(tok.path.clone());
                    }
                }
            });
            if fits.is_empty() {
                ui.label(egui::RichText::new(tr("No tokens fit here.")).font(regular(11.0)).color(t.menu_dim));
            }
        });
    });
    let rect = area.response.rect;
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos)));
    if let Some(target) = chosen {
        let v = json!(format!("{{{target}}}"));
        if p.sub.is_empty() {
            app.run("token.set", src.with(json!({"path": p.path, "value": v})));
        } else {
            set_part(app, &src, &p.path, &p.sub, v);
        }
        return;
    }
    if !outside && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.alias = Some(p);
    }
}

fn create_row(app: &mut FigApp, ui: &mut Ui, src: &Src, tree: &Value, st: &mut Editor) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let resp = widgets::button(ui, tr("+ Create token"), false, true);
        crate::right::menu_popup(&resp).show(|ui| {
            for (kind, label) in TYPES {
                if menu_item(ui, tr(label), "") {
                    // New tokens go under a group named after their type.
                    let group = kind.id();
                    let path = unique(&format!("{group}.new"), |p| dt::split_path(p).is_some_and(|parts| dt::at_path(tree, &parts).is_some()))
                        .replace(' ', "-");
                    if app.run("token.set", src.with(json!({"path": path, "type": kind.id()}))).is_some() {
                        st.query.clear();
                        start_rename(st, Rename::Token(path));
                    }
                    ui.close();
                }
            }
        });
    });
    ui.add_space(12.0);
}

/// A token's details: type, description, deprecation and its value as JSON.
fn details(app: &mut FigApp, ctx: &Context, doc: &Document, st: &mut Editor) {
    let (Some(path), Some(src)) = (st.details.clone(), st.source.clone()) else {
        st.details = None;
        return;
    };
    let Some(tok) = doc.tokens.tree(&src.doc_source()).and_then(|tr| dt::flatten(tr).into_iter().find(|x| x.path == path)) else {
        st.details = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    egui::Window::new(tr("Edit token"))
        .id(Id::new("token_details"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(340.0)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            widgets::label(ui, tr("Path"), t.text_secondary);
            ui.label(egui::RichText::new(&tok.path).font(regular(11.0)).color(t.text));
            ui.add_space(6.0);
            widgets::label(ui, tr("Type"), t.text_secondary);
            let resp = widgets::dropdown(ui, type_label(tok.kind), 200.0, None);
            crate::right::menu_popup(&resp).show(|ui| {
                for (k, label) in TYPES {
                    if menu_check(ui, tok.kind == Some(k), tr(label), "") {
                        app.run("token.set", src.with(json!({"path": tok.path, "type": k.id()})));
                        st.json = None;
                        ui.close();
                    }
                }
            });
            ui.add_space(6.0);
            widgets::label(ui, tr("Description"), t.text_secondary);
            if let Some(d) = widgets::text_field(ui, Id::new(("token_desc", &tok.path)), &tok.description, 320.0, tr("What it's for")) {
                app.run("token.set", src.with(json!({"path": tok.path, "description": d})));
            }
            ui.add_space(6.0);
            if widgets::checkbox(ui, tok.deprecated, tr("Deprecated")) {
                app.run("token.set", src.with(json!({"path": tok.path, "deprecated": !tok.deprecated})));
            }
            ui.add_space(10.0);
            widgets::label(ui, tr("Value (DTCG JSON)"), t.text_secondary);
            let mut text = st.json.clone().unwrap_or_else(|| serde_json::to_string_pretty(&tok.value).unwrap_or_default());
            let changed = ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(320.0).desired_rows(8)).changed();
            if changed {
                st.json = Some(text.clone());
            }
            let parsed = serde_json::from_str::<Value>(&text);
            let check = match (&parsed, tok.kind) {
                (Ok(v), Some(k)) => dt::check_value(k, v).err(),
                (Ok(_), None) => None,
                (Err(e), _) => Some(e.to_string()),
            };
            if let Some(e) = &check {
                ui.label(egui::RichText::new(e).font(regular(10.0)).color(t.danger));
            }
            ui.add_space(6.0);
            if widgets::button(ui, tr("Apply"), true, st.json.is_some() && check.is_none()).clicked()
                && let Ok(v) = parsed
                && app.run("token.set", src.with(json!({"path": tok.path, "value": v}))).is_some()
            {
                st.json = None;
            }
        });
    if !open {
        st.details = None;
        st.json = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_of_composite_values_are_replaced_in_place() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let typo = json!({"fontFamily": "Inter", "fontSize": {"value": 16, "unit": "px"}});
        let out = with_part(&typo, &s(&["fontFamily"]), json!("{font.body}"));
        assert_eq!(out["fontFamily"], "{font.body}");
        assert_eq!(out["fontSize"], typo["fontSize"], "the other fields stay");
        // A missing field is added; list items by index; out of range changes nothing.
        assert_eq!(with_part(&typo, &s(&["lineHeight"]), json!(1.5))["lineHeight"], 1.5);
        let stops = json!([{"color": "#000000", "position": 0}, {"color": "#ffffff", "position": 1}]);
        assert_eq!(with_part(&stops, &s(&["1", "position"]), json!(0.5))[1]["position"], 0.5);
        assert_eq!(with_part(&stops, &s(&["9", "position"]), json!(0.5)), stops);
        assert_eq!(with_part(&stops, &[], json!("{gradient.hero}")), json!("{gradient.hero}"));
        // Every composite type that expands has fields; each field's type is one the format has.
        for k in [TokenType::Typography, TokenType::Border, TokenType::Transition, TokenType::Shadow, TokenType::Gradient] {
            assert!(!fields(k).is_empty(), "{k:?}");
        }
        // Defaults of composite types pass with every field present.
        for k in [TokenType::Typography, TokenType::Border, TokenType::Transition] {
            let v = k.default_value();
            assert!(fields(k).iter().all(|(key, _, _)| v.get(key).is_some()), "{k:?}");
        }
    }
}
