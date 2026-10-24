//! The Variables editor: collections on the left (this file's, then each enabled library's,
//! read-only under the library's name); the selected collection's variables on the right as a
//! table with one value column per mode. Double-click a name to rename it, right-click for more; a
//! value's variable button turns it into an alias of another variable (a library one too).
//! A library collection can be extended: a collection of this file with its modes and variables,
//! whose overridden values are marked (with a reset). Import and export read and write the
//! variables JSON (or CSS). Everything goes through `variable.*` / `library.*` commands, so it's
//! undoable and the same over MCP.

use egui::{Align2, Context, CornerRadius, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};
use figcraft_doc::{Collection, Document, PublishedLibrary, Variable, VariableType, VariableValue, Variables, scopes_for};
use figcraft_engine::library;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::colorpicker::PickerTarget;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::right::{menu_check, menu_item};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FieldEdit, Lead};

const SIDEBAR_W: f32 = 264.0;
/// How far the sidebar's edge drags, and the table's column edges.
const SIDEBAR_MIN: f32 = 180.0;
const SIDEBAR_MAX: f32 = 420.0;
const NAME_W: f32 = 240.0;
const NAME_MIN: f32 = 120.0;
const NAME_MAX: f32 = 420.0;
const MODE_W: f32 = 200.0;
const MODE_MIN: f32 = 100.0;
const MODE_MAX: f32 = 420.0;
/// The column at the table's right end: the button that adds a mode. It never scrolls.
const ADD_W: f32 = 40.0;
const ROW_H: f32 = 36.0;
const HEAD_ROW_H: f32 = 32.0;
/// The horizontal scrollbar's strip, under the rows (only the mode columns scroll).
const HSCROLL_H: f32 = 12.0;
/// The bottom bar's height: "+ Create variable" and the row of the table's scrollbar.
const BOTTOM_H: f32 = 41.0;
/// The search field in the top bar, and the filter button joined to its right end.
const SEARCH_W: f32 = 174.0;
const FILTER_W: f32 = 25.0;
const TYPES: [VariableType; 4] = [VariableType::Color, VariableType::Number, VariableType::String, VariableType::Boolean];

/// What a dragged row lands on: a gap between two rows of the table, or a group (a header there,
/// or a row of the sidebar's Groups list).
#[derive(Clone, Debug, PartialEq, Eq)]
enum DropTarget {
    /// The gap above this variable.
    Before(String),
    /// The gap below this variable (the one after it, or the end of its folder).
    After(String),
    /// A folder's rows, at their end (None = "All variables": out of every folder).
    Group(Option<String>),
}

/// A row of the editor a dragged row can land on, as laid out this frame.
struct Slot {
    rect: Rect,
    /// The variable of a table row, or the folder of a group row (None = "All variables").
    var: Option<String>,
    group: Option<Option<String>>,
}

impl Slot {
    fn var(rect: Rect, id: String) -> Slot {
        Slot { rect, var: Some(id), group: None }
    }

    fn group(rect: Rect, path: Option<String>) -> Slot {
        Slot { rect, var: None, group: Some(path) }
    }
}

/// What's being renamed inline.
#[derive(Clone, Debug, PartialEq, Hash)]
pub(crate) enum Rename {
    Collection(String),
    /// (collection, mode)
    Mode(String, String),
    Variable(String),
    /// (collection, group path)
    Group(String, String),
}

/// How the editor is drawn: in a panel of its own over the editor (floating, moved by its header),
/// or filling the editor's content area (full, the left rail stays). Both draw with the same code;
/// [`Geom`] is the only difference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presentation {
    #[default]
    Floating,
    Full,
}

impl Presentation {
    /// The presentation a control-channel word names ("full", "floating").
    pub fn from_name(name: &str) -> Option<Presentation> {
        match name {
            "full" => Some(Presentation::Full),
            "floating" => Some(Presentation::Floating),
            _ => None,
        }
    }

    /// The word the control channel reads the presentation back as.
    pub fn name(self) -> &'static str {
        match self {
            Presentation::Floating => "floating",
            Presentation::Full => "full",
        }
    }
}

/// Where the editor sits and how big it is. Every offset the layout uses comes from here, so one
/// body of drawing code serves both presentations: a panel at the user's position, or the area the
/// editor panels would have taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub panel: Rect,
    pub floating: bool,
    /// The collections sidebar shows; folded, the table takes its column.
    pub sidebar: bool,
    /// Its width while it shows (its edge drags).
    pub sidebar_w: f32,
    /// The screen the panel is on (a size never grows past it).
    screen: Rect,
}

impl Geom {
    /// A floating panel's size: wider than a Variables palette's, because this editor has the
    /// collections sidebar next to a column per mode.
    pub const FLOAT_W: f32 = 880.0;
    pub const FLOAT_H: f32 = 480.0;
    /// The panel's smallest usable size on a small screen.
    pub const MIN_W: f32 = 480.0;
    pub const MIN_H: f32 = 240.0;
    /// The header's height, in both presentations.
    pub const HEADER_H: f32 = 44.0;

    /// The floating panel at `pos` (its remembered top-left; centred when it has none), kept
    /// inside `screen`.
    pub fn floating(screen: Rect, pos: Option<Pos2>, sidebar: bool) -> Geom {
        let w = Self::FLOAT_W.min((screen.width() - 80.0).max(Self::MIN_W));
        let h = Self::FLOAT_H.min((screen.height() - 120.0).max(Self::MIN_H));
        let size = vec2(w, h);
        let pos = pos.unwrap_or_else(|| screen.center() - size / 2.0);
        let x = pos.x.clamp(screen.min.x, (screen.max.x - w).max(screen.min.x));
        let y = pos.y.clamp(screen.min.y, (screen.max.y - h).max(screen.min.y));
        Geom { panel: Rect::from_min_size(pos2(x, y), size), floating: true, sidebar, sidebar_w: SIDEBAR_W, screen }
    }

    /// The full view: all of `panel`, which it takes from the editor panels.
    pub fn full(panel: Rect, sidebar: bool) -> Geom {
        Geom { panel, floating: false, sidebar, sidebar_w: SIDEBAR_W, screen: panel }
    }

    /// The sidebar at the width the user dragged it to.
    pub fn with_sidebar_width(mut self, width: f32) -> Geom {
        self.sidebar_w = width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        self
    }

    /// At the size the user dragged it to (None = the default). Never smaller than the smallest
    /// usable panel, never larger than the screen it is on.
    pub fn with_size(mut self, size: Option<Vec2>) -> Geom {
        if let Some(size) = size {
            self.panel = Rect::from_min_size(self.panel.min, self.size_of(size));
        }
        self
    }

    /// `size` within the panel's bounds, given the screen it sits on.
    fn size_of(&self, size: Vec2) -> Vec2 {
        let screen = self.screen;
        vec2(size.x.clamp(Self::MIN_W, screen.width().max(Self::MIN_W)), size.y.clamp(Self::MIN_H, screen.height().max(Self::MIN_H)))
    }

    pub fn size(&self) -> Vec2 {
        self.panel.size()
    }

    /// The body's height: the panel less its header.
    pub fn body_height(&self) -> f32 {
        (self.panel.height() - Self::HEADER_H).max(120.0)
    }

    /// The collections sidebar's column, its 1 pt line included; 0 when it is folded.
    pub fn sidebar_width(&self) -> f32 {
        if self.sidebar { self.sidebar_w + 1.0 } else { 0.0 }
    }

    /// The width the table gets: the panel less the collections sidebar and its 1 pt line.
    pub fn table_width(&self) -> f32 {
        (self.panel.width() - self.sidebar_width()).max(200.0)
    }
}

/// The open editor's state (`UiState::variables`; None = closed).
#[derive(Clone)]
pub struct Editor {
    /// The floating panel or the full view; remembered between openings, like the collection.
    pub presentation: Presentation,
    /// The floating panel's top-left (screen points); None = centred. Its header moves it.
    pub pos: Option<Pos2>,
    /// The library whose collection is shown (id or name); None for this file's.
    library: Option<String>,
    /// The selected collection's id.
    collection: Option<String>,
    query: String,
    pub(crate) rename: Option<Rename>,
    /// The rename just started: its field takes the focus in the frame it's first drawn.
    rename_start: bool,
    /// The variable whose details (name, description, scopes) are open.
    details: Option<String>,
    /// The group the table shows ("space" or "color/brand"); None = every variable.
    group: Option<String>,
    /// The collections sidebar shows (its fold button, in the header).
    pub sidebar: bool,
    /// Its width, dragged by its edge.
    pub sidebar_w: f32,
    /// The floating panel's size (None = the default); its right and bottom edges, and their
    /// corner, drag it.
    pub size: Option<Vec2>,
    /// The table's name column and one mode column, dragged by their headings' edges.
    pub name_w: f32,
    pub mode_w: f32,
    /// How far the mode columns have scrolled sideways; the name and add columns stay put.
    pub scroll_x: f32,
    /// The types the filter button hides (the funnel, right of the search field).
    hidden_kinds: Vec<VariableType>,
    /// The variable whose row is being dragged by its name.
    drag: Option<String>,
    /// While the floating panel is moved by its header: where the pointer took hold of it, from
    /// the panel's top-left. The frame after puts the panel back under the pointer, so it follows
    /// it without lagging behind — and, recomputed each frame, it doesn't stick at the screen edge
    /// the way a delta added to the last (clamped) position does.
    grab: Option<Vec2>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            presentation: Presentation::Floating,
            pos: None,
            library: None,
            collection: None,
            query: String::new(),
            rename: None,
            rename_start: false,
            details: None,
            group: None,
            sidebar: true,
            sidebar_w: SIDEBAR_W,
            size: None,
            name_w: NAME_W,
            mode_w: MODE_W,
            scroll_x: 0.0,
            hidden_kinds: Vec::new(),
            drag: None,
            grab: None,
        }
    }
}

impl Editor {
    /// An editor showing `collection` (id or name; the first one when None or unknown).
    pub fn on(collection: Option<String>) -> Self {
        Editor { collection, ..Default::default() }
    }

    /// An editor showing a collection of enabled library `library` (id or name).
    pub fn in_library(library: String, collection: Option<String>) -> Self {
        Editor { library: Some(library), collection, ..Default::default() }
    }
}

/// What the editor lists for one source: this file's variables (editable, never library copies)
/// or an enabled library's published ones (read-only).
struct View<'a> {
    vars: &'a Variables,
    /// The library's (id, name); None for this file.
    library: Option<(&'a str, &'a str)>,
    collections: Vec<&'a Collection>,
    variables: Vec<&'a Variable>,
}

impl<'a> View<'a> {
    fn local(doc: &'a Document) -> Self {
        View {
            vars: &doc.variables,
            library: None,
            collections: library::local_collections(doc).collect(),
            variables: doc.variables.variables.iter().filter(|v| !library::is_remote_variable(doc, &v.id)).collect(),
        }
    }

    fn of(lib: &'a PublishedLibrary) -> Self {
        View {
            vars: &lib.doc.variables,
            library: Some((&lib.manifest.id, &lib.manifest.name)),
            collections: library::published_collections(lib).collect(),
            variables: library::published_variables(lib).collect(),
        }
    }

    fn id(&self) -> Option<&'a str> {
        self.library.map(|l| l.0)
    }

    fn editable(&self) -> bool {
        self.library.is_none()
    }

    /// Whether the editor's `library` (id or name) names this view.
    fn is(&self, library: Option<&str>) -> bool {
        match (self.library, library) {
            (None, None) => true,
            (Some((id, name)), Some(l)) => l == id || l == name,
            _ => false,
        }
    }

    /// Collection `c` (id or name).
    fn collection(&self, c: &str) -> Option<&'a Collection> {
        self.collections.iter().copied().find(|x| x.id == c).or_else(|| self.collections.iter().copied().find(|x| x.name == c))
    }

    fn in_collection(&self, c: &str) -> impl Iterator<Item = &'a Variable> {
        self.variables.iter().copied().filter(move |v| v.collection == c)
    }
}

/// Show the editor while it's open (call once per frame). The floating panel draws here; the full
/// view is drawn by [`full`], in the place the editor panels would have taken.
pub fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut st) = app.ui.variables.take() else { return };
    if st.presentation == Presentation::Full {
        app.ui.variables = Some(st);
        return;
    }
    // A drag the header started puts the panel under the pointer before it is drawn, so it follows
    // the pointer in this frame rather than the next one.
    if let Some(grab) = st.grab {
        match ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down())) {
            (Some(p), true) => st.pos = Some(p - grab),
            _ => st.grab = None,
        }
    }
    let g = Geom::floating(ctx.content_rect(), st.pos, st.sidebar).with_sidebar_width(st.sidebar_w).with_size(st.size);
    let open = draw(app, ctx, &mut st, g);
    app.ui.variables = open.then_some(st);
}

/// The full view: the editor in `panel`, which the window gives it in place of the panels and the
/// canvas (the rail stays).
pub fn full(app: &mut FigApp, ctx: &Context, panel: Rect) {
    let Some(mut st) = app.ui.variables.take() else { return };
    if st.presentation != Presentation::Full {
        app.ui.variables = Some(st);
        return;
    }
    let g = Geom::full(panel, st.sidebar).with_sidebar_width(st.sidebar_w);
    let open = draw(app, ctx, &mut st, g);
    app.ui.variables = open.then_some(st);
}

/// The editor in `g`'s geometry; false when it was closed. One area draws both presentations, so
/// the controls keep their ids (and the keyboard) as the panel grows into the full view.
fn draw(app: &mut FigApp, ctx: &Context, st: &mut Editor, g: Geom) -> bool {
    let t = Tokens::get(ctx);
    let mut open = true;
    let frame = if g.floating { egui::Frame::window(&ctx.global_style()).fill(t.panel) } else { egui::Frame::NONE.fill(t.panel) }
        .inner_margin(egui::Margin::same(0));
    egui::Area::new(Id::new("variables_panel")).order(egui::Order::Middle).fixed_pos(g.panel.min).constrain(g.floating).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(g.size().x);
            ui.set_min_height(g.size().y);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            editor(app, ui, st, g, &mut open);
        });
    });
    open
}

/// The editor's body in `g`'s geometry: the collections sidebar, then the table, then the details
/// popover. Both presentations call this.
fn editor(app: &mut FigApp, ui: &mut Ui, st: &mut Editor, g: Geom, open: &mut bool) {
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else { return };
    let libs = crate::variable_ui::libraries(app, &doc);
    let views: Vec<View> = std::iter::once(View::local(&doc)).chain(libs.iter().map(|l| View::of(l))).collect();
    // The selected collection (its source's first when none is), else this file's first.
    let found = views.iter().find(|v| v.is(st.library.as_deref())).and_then(|v| {
        let c = match st.collection.as_deref() {
            Some(c) => v.collection(c),
            None => v.collections.first().copied(),
        };
        Some((v, c?))
    });
    let Some((view, coll)) = found.map(|(v, c)| (v, Some(c))).or_else(|| views.first().map(|v| (v, v.collections.first().copied()))) else { return };
    st.library = view.id().map(str::to_string);
    st.collection = coll.map(|c| c.id.clone());
    let t = Tokens::get(ui.ctx());
    let mut slots: Vec<Slot> = Vec::new();
    header(app, ui, st, g, open);
    let h = g.body_height();
    let top_down = egui::Layout::top_down(egui::Align::Min);
    ui.horizontal_top(|ui| {
        if g.sidebar {
            ui.allocate_ui_with_layout(vec2(g.sidebar_width() - 1.0, h), top_down, |ui| {
                sidebar(app, ui, st, &views, &doc.variables, view, coll, &mut slots)
            });
            let (line, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
            ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
        }
        let w = g.table_width();
        ui.allocate_ui_with_layout(vec2(w, h), top_down, |ui| {
            ui.allocate_ui_with_layout(vec2(w, (h - BOTTOM_H).max(80.0)), top_down, |ui| table(app, ui, view, st, &mut slots));
            if let Some(c) = coll {
                bottom_bar(app, ui, st, view, c);
            }
        });
    });
    // The sidebar's edge drags its width.
    if g.sidebar {
        let x = g.panel.min.x + g.sidebar_width() - 1.0;
        let hit = Rect::from_min_max(pos2(x - 3.0, g.panel.min.y + Geom::HEADER_H), pos2(x + 3.0, g.panel.max.y));
        let resp = ui.interact(hit, Id::new("vars_sidebar_edge"), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.dragged()
            && let Some(p) = ui.input(|i| i.pointer.interact_pos())
        {
            st.sidebar_w = (p.x - g.panel.min.x).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        }
    }
    // The floating panel's right and bottom edges, and their corner, drag its size.
    if g.floating {
        let p = g.panel;
        let corner = Rect::from_min_max(pos2(p.max.x - 16.0, p.max.y - 16.0), p.max);
        let right = Rect::from_min_max(pos2(p.max.x - 3.0, p.min.y + Geom::HEADER_H), pos2(p.max.x + 3.0, p.max.y - 16.0));
        let bottom = Rect::from_min_max(pos2(p.min.x + 40.0, p.max.y - 3.0), pos2(p.max.x - 16.0, p.max.y + 3.0));
        let (w_id, h_id, c_id) = (Id::new("vars_size_w"), Id::new("vars_size_h"), Id::new("vars_size"));
        for (rect, id, cursor) in [
            (corner, c_id, egui::CursorIcon::ResizeNwSe),
            (right, w_id, egui::CursorIcon::ResizeHorizontal),
            (bottom, h_id, egui::CursorIcon::ResizeVertical),
        ] {
            let resp = ui.interact(rect, id, Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(cursor);
            }
            if resp.dragged()
                && let Some(ptr) = ui.input(|i| i.pointer.interact_pos())
            {
                // Recomputed from the pointer every frame, so it never drifts or sticks.
                let mut size = st.size.unwrap_or_else(|| p.size());
                if id == w_id || id == c_id {
                    size.x = (ptr.x - p.min.x).max(Geom::MIN_W);
                }
                if id == h_id || id == c_id {
                    size.y = (ptr.y - p.min.y).max(Geom::MIN_H);
                }
                st.size = Some(size);
            }
        }
        // The corner's grip shows while the pointer is on it.
        if ui.rect_contains_pointer(corner) {
            let grip = ui.painter();
            for i in 0..3 {
                let x = p.max.x - 5.0 - i as f32 * 4.5;
                grip.line_segment([pos2(x - 3.5, p.max.y - 5.0), pos2(x, p.max.y - 8.5)], Stroke::new(1.2, t.text_secondary));
            }
        }
    }
    drag_and_drop(app, ui, st, &slots);
    details(app, ui.ctx(), &doc.variables, st);
}

fn header(app: &mut FigApp, ui: &mut Ui, st: &mut Editor, g: Geom, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), Geom::HEADER_H), Sense::hover());
    // The floating panel's handle: everything left of the buttons (they take their own presses).
    if g.floating {
        let handle = Rect::from_min_max(r.min, pos2((r.max.x - 340.0).max(r.min.x + 40.0), r.max.y));
        let resp = ui.interact(handle, Id::new("vars_drag"), Sense::click_and_drag());
        if resp.drag_started() {
            // Where the pointer took hold of it: the frame after puts the panel back under it, so
            // it follows the pointer instead of a frame behind it.
            st.grab = ui.input(|i| i.pointer.press_origin()).map(|p| p - g.panel.min);
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    let side = g.sidebar_width();
    let left_bound = r.min.x + side + 12.0;
    // The right end: Close, the presentation toggle, the search field with its filter button, and
    // Import and Export left of those.
    if widgets::icon_button_at(
        ui,
        Id::new("vars_close"),
        Rect::from_center_size(pos2(r.max.x - 24.0, r.center().y), vec2(24.0, 24.0)),
        Icon::Close,
        tr("Close"),
    ) {
        *open = false;
    }
    // The presentation toggle: out to the whole window, or back into a panel of its own.
    let (icon, tip) = if g.floating { (Icon::Expand, tr("Expand to the full view")) } else { (Icon::Collapse, tr("Collapse to a floating panel")) };
    if widgets::icon_button_at(
        ui,
        Id::new("vars_presentation"),
        Rect::from_center_size(pos2(r.max.x - 52.0, r.center().y), vec2(24.0, 24.0)),
        icon,
        tip,
    ) {
        st.presentation = if g.floating { Presentation::Full } else { Presentation::Floating };
    }
    let search_left = (r.max.x - 74.0 - SEARCH_W - FILTER_W).max(left_bound);
    let search = Rect::from_min_size(pos2(search_left, r.center().y - 12.0), vec2(SEARCH_W + FILTER_W, 24.0));
    if search.width() >= SEARCH_W + FILTER_W {
        search_box(ui, st, search);
    }
    let bar = Rect::from_min_max(pos2((search_left - 8.0 - 240.0).max(left_bound), r.min.y + 10.0), pos2(search_left - 8.0, r.max.y - 10.0));
    if bar.width() >= 120.0 {
        let mut bar = ui.new_child(UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let export = widgets::button(&mut bar, tr("Export…"), false, true);
        crate::right::menu_popup(&export).show(|ui| {
            for (label, format) in [(tr("Variables JSON…"), "json"), (tr("CSS custom properties…"), "css")] {
                if menu_item(ui, label, "") {
                    export_dialog(app, format);
                    ui.close();
                }
            }
        });
        if widgets::button(&mut bar, tr("Import…"), false, true).on_hover_text(tr("Add or update collections from a variables JSON")).clicked() {
            import_dialog(app);
        }
    }
    // The sidebar's half: its name and the button that folds it away; folded, the button that
    // brings it back, at the header's left end where the sidebar was.
    let title_font = semibold(13.0);
    if g.sidebar {
        let title_r = Rect::from_min_max(r.min, pos2((r.min.x + side - 40.0).min(r.max.x), r.max.y));
        ui.painter().with_clip_rect(title_r).text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Local variables"), title_font, t.text);
        if widgets::icon_button_at(
            ui,
            Id::new("vars_sidebar"),
            Rect::from_center_size(pos2(r.min.x + side - 24.0, r.center().y), vec2(24.0, 24.0)),
            Icon::Sidebar,
            tr("Hide sidebar"),
        ) {
            st.sidebar = false;
        }
        // The sidebar's edge runs on as the line between it and the table.
        let x = r.min.x + side - 1.0;
        ui.painter().line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0, t.border));
    } else {
        if widgets::icon_button_at(
            ui,
            Id::new("vars_sidebar"),
            Rect::from_center_size(pos2(r.min.x + 30.0, r.center().y), vec2(24.0, 24.0)),
            Icon::Sidebar,
            tr("Show sidebar"),
        ) {
            st.sidebar = true;
        }
        let title_r = Rect::from_min_max(pos2(r.min.x + 52.0, r.min.y), pos2((bar.min.x - 8.0).max(r.min.x + 64.0), r.max.y));
        ui.painter().with_clip_rect(title_r).text(pos2(r.min.x + 52.0, r.center().y), Align2::LEFT_CENTER, tr("Local variables"), title_font, t.text);
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

/// The search field, with the button that filters by type joined to its right end.
fn search_box(ui: &mut Ui, st: &mut Editor, field: Rect) {
    let t = Tokens::get(ui.ctx());
    let filter = Rect::from_min_max(pos2(field.max.x - FILTER_W, field.min.y), field.max);
    let body = Rect::from_min_max(field.min, pos2(field.max.x - FILTER_W, field.max.y));
    ui.painter().rect_filled(body, CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 }, t.field);
    let f = ui.interact(filter, Id::new("vars_filter"), Sense::click());
    ui.painter().rect_filled(filter, CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 }, if f.hovered() { t.hover } else { t.field });
    let hidden = !st.hidden_kinds.is_empty();
    let glyph = Rect::from_center_size(filter.center(), vec2(13.0, 13.0));
    icons::paint(ui.painter(), glyph, Icon::Filter, if hidden { t.accent } else { t.icon_secondary });
    let mag = Rect::from_center_size(pos2(body.min.x + 12.0, body.center().y), vec2(12.0, 12.0));
    icons::paint(ui.painter(), mag, Icon::Search, t.icon_secondary);
    let text_r = Rect::from_min_max(pos2(body.min.x + 24.0, body.min.y + 3.0), pos2(body.max.x - 4.0, body.max.y - 3.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(text_r));
    let r = child.add(
        egui::TextEdit::singleline(&mut st.query)
            .frame(egui::Frame::NONE)
            .hint_text(tr("Search"))
            .font(regular(11.0))
            .vertical_align(egui::Align::Center),
    );
    crate::widgets::select_all_on_focus(ui, &r, &st.query);
    crate::right::menu_popup(&f).show(|ui| {
        for kind in TYPES {
            // The English label also names the new variable (document content).
            let label = match kind {
                VariableType::Color => tr_noop("Color"),
                VariableType::Number => tr_noop("Number"),
                VariableType::String => tr_noop("String"),
                VariableType::Boolean => tr_noop("Boolean"),
            };
            let on = !st.hidden_kinds.contains(&kind);
            if menu_check(ui, on, tr(label), "") {
                if on {
                    st.hidden_kinds.push(kind);
                } else {
                    st.hidden_kinds.retain(|k| *k != kind);
                }
            }
        }
        if hidden {
            ui.separator();
            if menu_item(ui, tr("Show every type"), "") {
                st.hidden_kinds.clear();
                ui.close();
            }
        }
    });
    f.on_hover_text(tr("Filter by type"));
}

/// Export this file's collections through a save dialog, or as a download where the platform has
/// no save dialogs (the browser).
pub(crate) fn export_dialog(app: &mut FigApp, format: &str) {
    let name = app.session.doc().map(|d| format!("{}.variables.{format}", d.name)).unwrap_or_else(|_| format!("variables.{format}"));
    if app.services.downloads() {
        let Some(r) = app.run("variable.export", json!({"format": format})) else { return };
        let text = r["text"].as_str().unwrap_or_default();
        if app.services.download(&name, text.as_bytes()) {
            app.toast(trf("Exported {file}", &[("file", &name)]));
        } else {
            app.toast(tr("Couldn't download the variables"));
        }
        return;
    }
    let format = format.to_owned();
    let ext = format.clone();
    app.pick_save(tr("Export variables"), &name, &[&ext], move |app, p| {
        if app.run("variable.export", json!({"format": format, "path": p.to_string_lossy()})).is_some() {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            app.toast(trf("Exported {file}", &[("file", &file)]));
        }
    });
}

/// Import a variables JSON picked in an open dialog.
fn import_dialog(app: &mut FigApp) {
    app.pick_open(tr("Import variables"), &["json"], |app, p| {
        let Some(r) = app.run("variable.import", json!({"variables": p.to_string_lossy()})) else { return };
        let (added, updated) = (r["variables"].as_u64().unwrap_or(0), r["updated"].as_u64().unwrap_or(0));
        let skipped = r["warnings"].as_array().map_or(0, Vec::len);
        let mut msg = trn("Imported {n} new variables, updated {updated}", added as usize, &[("updated", &updated)]);
        if skipped > 0 {
            msg = format!("{msg} {}", trn("({n} warnings: some entries were skipped)", skipped, &[]));
        }
        app.toast(msg);
    });
}

/// Extend library `lib`'s collection `c` and show the new collection.
fn extend(app: &mut FigApp, lib: &str, c: &str, st: &mut Editor) {
    if let Some(id) = app.run("library.extendCollection", json!({"library": lib, "collection": c})).and_then(|r| r["id"].as_str().map(str::to_string))
    {
        st.library = None;
        st.collection = Some(id);
        st.group = None;
        st.query.clear();
    }
}

fn rename_id(target: &Rename) -> Id {
    Id::new(("var_rename", target))
}

/// Start renaming `target`. The field isn't drawn yet (a new collection, mode or variable shows next
/// frame), and focusing it now would put a node that isn't there in the screen reader tree, which
/// AccessKit aborts on (#224): the field takes the focus when drawn.
fn start_rename(st: &mut Editor, target: Rename) {
    st.rename = Some(target);
    st.rename_start = true;
}

/// The inline editor of a rename in progress: the new name once committed. The rename ends when
/// the editor closes.
fn rename_field(ui: &mut Ui, st: &mut Editor, target: &Rename, value: &str, rect: Rect, font: egui::FontId) -> Option<String> {
    let start = std::mem::take(&mut st.rename_start);
    let (out, open) = widgets::rename_field(ui, rename_id(target), value, rect, font, start);
    if !open {
        st.rename = None;
    }
    out
}

/// The first of `base`, `base 2`, `base 3`… that `taken` doesn't hold.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..10_000).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn sidebar(
    app: &mut FigApp,
    ui: &mut Ui,
    st: &mut Editor,
    views: &[View],
    local: &Variables,
    view: &View,
    coll: Option<&Collection>,
    slots: &mut Vec<Slot>,
) {
    let t = Tokens::get(ui.ctx());
    // Collections: the heading with the button that makes one, then this file's and each library's.
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Collections"), semibold(11.0), t.text_secondary);
    if widgets::icon_button_at(
        ui,
        Id::new("vars_add_coll"),
        Rect::from_center_size(pos2(hr.max.x - 20.0, hr.center().y), vec2(24.0, 24.0)),
        Icon::Plus,
        tr("Create collection"),
    ) {
        create_collection(app, local, st);
    }
    egui::ScrollArea::vertical().id_salt("vars_sidebar").auto_shrink([false, false]).show(ui, |ui| {
        for v in views {
            if let Some((_, name)) = v.library
                && !v.collections.is_empty()
            {
                library_title(ui, name);
            }
            for c in &v.collections {
                collection_row(app, ui, v, c, st);
            }
        }
        // The groups of the selected collection: a section of their own under a line, as in the
        // Variables palette's sidebar (not nested under the collection's row).
        if let Some(c) = coll {
            ui.add_space(6.0);
            let (sep, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(sep, CornerRadius::ZERO, t.border);
            let (gr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
            ui.painter().text(pos2(gr.min.x + 16.0, gr.center().y), Align2::LEFT_CENTER, tr("Groups"), semibold(11.0), t.text_secondary);
            group_rows(app, ui, view, c, st, slots);
        }
    });
}

/// A library's heading in the sidebar: its collections follow, read-only.
fn library_title(ui: &mut Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 22.0, r.center().y), vec2(12.0, 12.0)), Icon::Library, t.icon_secondary);
    let name_r = Rect::from_min_max(pos2(r.min.x + 34.0, r.min.y), pos2(r.max.x - 8.0, r.max.y));
    ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, name, semibold(11.0), t.text_secondary);
    resp.on_hover_text(tr("Library: its variables are read-only here; edit them in the library file and publish it"));
}

/// One collection in the sidebar (with its groups while selected).
fn collection_row(app: &mut FigApp, ui: &mut Ui, view: &View, c: &Collection, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let r = r.shrink2(vec2(8.0, 1.0));
    let selected = view.is(st.library.as_deref()) && st.collection.as_deref() == Some(c.id.as_str());
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.selected_row);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    let target = Rename::Collection(c.id.clone());
    let count = view.in_collection(&c.id).count();
    ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, count.to_string(), regular(11.0), t.text_tertiary);
    let indent = if view.editable() { 8.0 } else { 26.0 };
    let name_r = Rect::from_min_max(pos2(r.min.x + indent, r.min.y + 4.0), pos2(r.max.x - 32.0, r.max.y - 4.0));
    if view.editable() && st.rename.as_ref() == Some(&target) {
        if let Some(name) = rename_field(ui, st, &target, &c.name, name_r, regular(11.0)) {
            app.run("variable.renameCollection", json!({"collection": c.id, "name": name}));
        }
    } else {
        let font = if selected { semibold(11.0) } else { regular(11.0) };
        ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, &c.name, font, t.text);
    }
    if resp.clicked() {
        if !selected {
            st.group = None;
            st.query.clear();
        }
        st.library = view.id().map(str::to_string);
        st.collection = Some(c.id.clone());
    }
    if view.editable() {
        if resp.double_clicked() {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Rename"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if menu_item(ui, tr("Delete collection"), "") {
                app.run("variable.deleteCollection", json!({"collection": c.id}));
                ui.close();
            }
        });
    } else if let Some(lib) = view.id() {
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Extend collection"), "") {
                extend(app, lib, &c.id, st);
                ui.close();
            }
        });
    }
}

/// Every group path in `names` with its ancestors ("a/b/c" → "a", "a/b"), sorted.
fn groups<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let mut path = group_of(name);
        while !path.is_empty() {
            if !out.iter().any(|g| g == path) {
                out.push(path.to_string());
            }
            path = group_of(path);
        }
    }
    out.sort();
    out
}

/// "All variables" and the collection's groups (indented by depth): picking one filters the
/// table; this file's groups can be renamed (every variable in them follows) or deleted.
fn group_rows(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor, slots: &mut Vec<Slot>) {
    let t = Tokens::get(ui.ctx());
    let members: Vec<&Variable> = view.in_collection(&coll.id).collect();
    let names: Vec<&str> = members.iter().map(|v| v.name.as_str()).collect();
    let list = groups(names.iter().copied());
    let rows = std::iter::once(None).chain(list.iter().map(Some));
    for g in rows {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        let r = r.shrink2(vec2(8.0, 0.0));
        slots.push(Slot::group(r, g.cloned()));
        let on = st.group.as_ref() == g;
        // A dragged row over this folder: dropping it there moves it in.
        let hot = st.drag.is_some() && ui.rect_contains_pointer(r);
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        } else if resp.hovered() || hot {
            ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.border), StrokeKind::Inside);
        }
        if hot {
            ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
        }
        let depth = g.map_or(0, |g| g.matches('/').count());
        let x = r.min.x + 20.0 + depth as f32 * 12.0;
        let label = g.map_or(tr("All variables"), |g| g.rsplit('/').next().unwrap_or(g));
        let count = names.iter().filter(|n| g.is_none_or(|g| n.starts_with(&format!("{g}/")))).count();
        ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, count.to_string(), regular(10.0), t.text_tertiary);
        let name_r = Rect::from_min_max(pos2(x, r.min.y + 3.0), pos2(r.max.x - 28.0, r.max.y - 3.0));
        let Some(g) = g else {
            ui.painter().text(name_r.left_center(), Align2::LEFT_CENTER, label, regular(11.0), t.text_secondary);
            if resp.clicked() {
                st.group = None;
            }
            continue;
        };
        let target = Rename::Group(coll.id.clone(), g.clone());
        if view.editable() && st.rename.as_ref() == Some(&target) {
            if let Some(new) = rename_field(ui, st, &target, label, name_r, regular(11.0)) {
                rename_group(app, &members, g, &new, st);
            }
        } else {
            ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, label, regular(11.0), t.text);
        }
        if resp.clicked() {
            st.group = Some(g.clone());
        }
        if !view.editable() {
            continue;
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
                let steps: Vec<serde_json::Value> = members
                    .iter()
                    .filter(|v| v.name.starts_with(&format!("{g}/")))
                    .map(|v| json!({"command": "variable.delete", "params": {"variable": v.id}}))
                    .collect();
                app.run("command.batch", json!({"label": "Delete group", "commands": steps}));
                if st.group.as_ref().is_some_and(|s| s == g || s.starts_with(&format!("{g}/"))) {
                    st.group = None;
                }
                ui.close();
            }
        });
    }
}

/// Rename the last part of group `g` of a collection (its variables `members`) to `new`: every
/// variable under it moves along (one undo step).
fn rename_group(app: &mut FigApp, members: &[&Variable], g: &str, new: &str, st: &mut Editor) {
    let new = new.trim().replace('/', "-");
    if new.is_empty() {
        return;
    }
    let renamed = match g.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new}"),
        None => new,
    };
    let prefix = format!("{g}/");
    let steps: Vec<serde_json::Value> = members
        .iter()
        .filter(|v| v.name.starts_with(&prefix))
        .map(|v| json!({"command": "variable.update", "params": {"variable": v.id, "name": format!("{renamed}/{}", &v.name[prefix.len()..])}}))
        .collect();
    if app.run("command.batch", json!({"label": "Rename group", "commands": steps})).is_some()
        && st.group.as_deref().is_some_and(|s| s == g || s.starts_with(&prefix))
    {
        st.group = st.group.as_ref().map(|s| format!("{renamed}{}", &s[g.len()..]));
    }
}

fn create_collection(app: &mut FigApp, vars: &Variables, st: &mut Editor) {
    let name = unique("Collection", |n| vars.collections.iter().any(|c| c.name == n));
    if let Some(id) =
        app.run("variable.createCollection", json!({"name": name, "modes": ["Mode 1"]})).and_then(|r| r["id"].as_str().map(str::to_string))
    {
        st.library = None;
        st.collection = Some(id.clone());
        start_rename(st, Rename::Collection(id));
    }
}

/// The group part of a variable name ("space/md" → "space"; "" when ungrouped).
fn group_of(name: &str) -> &str {
    name.rsplit_once('/').map(|(g, _)| g).unwrap_or("")
}

fn table(app: &mut FigApp, ui: &mut Ui, view: &View, st: &mut Editor, slots: &mut Vec<Slot>) {
    let t = Tokens::get(ui.ctx());
    // A sidebar click this frame picked another source: shown next frame.
    if !view.is(st.library.as_deref()) {
        ui.ctx().request_repaint();
        return;
    }
    let Some(coll) = st.collection.as_deref().and_then(|c| view.collection(c)) else {
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.vertical(|ui| {
                widgets::label(
                    ui,
                    tr("Variables store reusable values (colours, numbers, strings, booleans) with modes such as Light and Dark."),
                    t.text_secondary,
                );
                ui.add_space(12.0);
                if widgets::button(ui, tr("Create collection"), true, true).clicked() {
                    create_collection(app, view.vars, st);
                }
            });
        });
        return;
    };
    // A library's collection, or one that extends another: a line about it and the way to extend it.
    notice_row(app, ui, view, coll, st);
    let q = st.query.trim().to_lowercase();
    let hidden = &st.hidden_kinds;
    let in_group = |v: &Variable| st.group.as_ref().is_none_or(|g| v.name.starts_with(&format!("{g}/")));
    let mut list: Vec<&Variable> = view
        .in_collection(&coll.id)
        .filter(|v| in_group(v) && !hidden.contains(&v.kind) && (q.is_empty() || v.name.to_lowercase().contains(&q)))
        .collect();
    list.sort_by(|a, b| group_of(&a.name).cmp(group_of(&b.name)));
    // The rows keep clear of the vertical scrollbar, so the header and they line up whether it
    // shows or not; the header stays put while they scroll under it.
    let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
    let width = (ui.available_width() - bar).max(240.0);
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEAD_ROW_H), Sense::hover());
    let head = Rect::from_min_size(head.min, vec2(width, head.height()));
    // Sideways: only the mode columns, under the pointer over them (Shift + wheel too).
    let cols = Columns::of(width, coll.modes.len(), st);
    if ui.rect_contains_pointer(cols.modes_rect(head)) {
        let (dx, shift, dy) = ui.input(|i| (i.smooth_scroll_delta.x, i.modifiers.shift, i.smooth_scroll_delta.y));
        // egui: a positive delta moves the content right, so the columns' offset grows.
        st.scroll_x += dx + if shift { dy } else { 0.0 };
    }
    st.scroll_x = st.scroll_x.clamp(0.0, cols.max_sx());
    let cols = Columns { sx: st.scroll_x, ..cols };
    header_row(app, ui, view, coll, st, &cols, head);
    let rows_h = (ui.available_height() - HSCROLL_H).max(60.0);
    egui::ScrollArea::vertical().id_salt("vars_rows").auto_shrink([false, false]).max_height(rows_h).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mut group: Option<&str> = None;
        for v in &list {
            let g = group_of(&v.name);
            if group != Some(g) {
                group = Some(g);
                if !g.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
                    ui.painter().text(pos2(r.min.x + 16.0, r.center().y + 2.0), Align2::LEFT_CENTER, g, semibold(11.0), t.text_secondary);
                    slots.push(Slot::group(r, Some(g.to_string())));
                }
            }
            var_row(app, ui, view, coll, v, st, &cols, slots);
        }
        if list.is_empty() && (!q.is_empty() || !st.hidden_kinds.is_empty()) {
            let (r, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
            ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("No matching variables"), regular(11.0), t.text_secondary);
        }
    });
    if cols.max_sx() > 0.0 {
        hscroll(ui, width, &cols, st);
    }
}

/// The table's columns as laid out this frame: the names on the left and the add-mode column on the
/// right stay put, and only the mode columns between them scroll sideways.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Columns {
    /// A row's width.
    width: f32,
    name: f32,
    /// The room the mode columns have between the two lines.
    modes: f32,
    add: f32,
    /// One mode column, and how many there are.
    mode: f32,
    count: usize,
    /// How far they have scrolled.
    sx: f32,
}

impl Columns {
    fn of(width: f32, count: usize, st: &Editor) -> Columns {
        let name = st.name_w.clamp(NAME_MIN, NAME_MAX).min((width * 0.5).max(NAME_MIN));
        Columns {
            width,
            name,
            modes: (width - name - ADD_W - 2.0).max(60.0),
            add: ADD_W,
            mode: st.mode_w.clamp(MODE_MIN, MODE_MAX),
            count: count.max(1),
            sx: st.scroll_x,
        }
    }

    /// How far the mode columns can scroll: their full width less the room they have.
    fn max_sx(&self) -> f32 {
        (self.count as f32 * (self.mode + 1.0) - (self.modes + 1.0)).max(0.0)
    }

    fn name_rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(r.min, pos2(r.min.x + self.name, r.max.y))
    }

    fn modes_rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(pos2(r.min.x + self.name + 1.0, r.min.y), pos2(r.max.x - self.add - 1.0, r.max.y))
    }

    fn add_rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(pos2(r.max.x - self.add, r.min.y), r.max)
    }

    /// Mode column `i` of `count`, scrolled by `sx`.
    fn mode_rect(&self, r: Rect, i: usize) -> Rect {
        let x = r.min.x + self.name + 1.0 + i as f32 * (self.mode + 1.0) - self.sx;
        Rect::from_min_size(pos2(x, r.min.y), vec2(self.mode, r.height()))
    }
}

/// The sideways scrollbar under the rows: it moves the mode columns only.
fn hscroll(ui: &mut Ui, width: f32, cols: &Columns, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, HSCROLL_H), Sense::hover());
    let track = Rect::from_min_max(pos2(r.min.x + cols.name + 9.0, r.center().y - 2.0), pos2(r.max.x - cols.add - 9.0, r.center().y + 2.0));
    if track.width() <= 12.0 {
        return;
    }
    ui.painter().rect_filled(track, CornerRadius::same(2), t.hover);
    let full = cols.count as f32 * (cols.mode + 1.0);
    let max_sx = cols.max_sx();
    let thumb_w = (track.width() * (cols.modes / full).clamp(0.12, 1.0)).min(track.width());
    let along = if max_sx > 0.0 { (st.scroll_x / max_sx).clamp(0.0, 1.0) } else { 0.0 };
    let thumb = Rect::from_min_size(pos2(track.left() + (track.width() - thumb_w) * along, track.top() - 1.0), vec2(thumb_w, track.height() + 2.0));
    let resp = ui.interact(track.expand2(vec2(0.0, 4.0)), Id::new("vars_hscroll"), Sense::click_and_drag());
    let lit = resp.hovered() || resp.dragged();
    ui.painter().rect_filled(thumb, CornerRadius::same(3), if lit { t.text_secondary } else { t.border_strong });
    if (resp.dragged() || resp.clicked())
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
    {
        let at = ((p.x - thumb_w / 2.0 - track.left()) / (track.width() - thumb_w).max(1.0)).clamp(0.0, 1.0);
        st.scroll_x = at * max_sx;
    }
}

/// A column's edge at `x` over `r`: dragging it sets `value` to the pointer's distance from `base`.
/// True while the pointer is on it (so the caller can light the edge).
fn column_edge(ui: &mut Ui, id: Id, x: f32, base: f32, r: Rect, value: &mut f32, min: f32, max: f32) -> bool {
    let hit = Rect::from_min_max(pos2(x - 3.0, r.min.y), pos2(x + 3.0, r.max.y));
    let resp = ui.interact(hit, id, Sense::drag());
    let on = resp.hovered() || resp.dragged();
    if on {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if resp.dragged()
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
    {
        *value = (p.x - base).clamp(min, max);
    }
    on
}

/// The line above a library's collection (or an extended one): where it comes from, and the button
/// that extends it here.
fn notice_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor) {
    let t = Tokens::get(ui.ctx());
    let mut text = match (view.library, coll.extends.as_deref().and_then(|p| view.vars.collection(p))) {
        (Some((_, name)), _) => Some(trf("From the library \"{name}\": read-only", &[("name", &name)])),
        (None, Some(parent)) => Some(trf("Extends \"{name}\": overridden values are marked", &[("name", &parent.name)])),
        _ => None,
    };
    if text.is_none() && view.library.is_none() {
        return;
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    if let Some((lib, _)) = view.library {
        let tip = tr("A collection of this file with these modes and variables: override their values per mode and add variables of its own");
        let b = Rect::from_min_max(pos2(r.max.x - 148.0, r.center().y - 12.0), pos2(r.max.x - 16.0, r.center().y + 12.0));
        let mut bar = ui.new_child(UiBuilder::new().max_rect(b).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if widgets::button(&mut bar, tr("Extend collection"), false, true).on_hover_text(tip).clicked() {
            extend(app, lib, &coll.id, st);
        }
    }
    if let Some(text) = text.take() {
        let text_r = Rect::from_min_max(pos2(r.min.x + 16.0, r.min.y), pos2((r.max.x - 160.0).max(r.min.x + 24.0), r.max.y));
        ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, text, regular(11.0), t.text_secondary);
    }
}

fn header_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, st: &mut Editor, cols: &Columns, r: Rect) {
    let t = Tokens::get(ui.ctx());
    // An extended collection has the modes of the one it extends.
    let editable = view.editable() && coll.extends.is_none();
    // Opaque: the rows scroll under it.
    ui.painter().rect_filled(r, CornerRadius::ZERO, t.panel);
    let name_r = cols.name_rect(r);
    ui.painter().text(pos2(name_r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, tr("Name"), regular(11.0), t.text_secondary);
    // The mode columns scroll between the two fixed ones, so they get a ui of their own, clipped.
    let modes = cols.modes_rect(r);
    let mut wide = ui.new_child(UiBuilder::new().max_rect(modes));
    wide.set_clip_rect(modes);
    for (i, m) in coll.modes.iter().enumerate() {
        let cell = cols.mode_rect(r, i);
        if cell.max.x < modes.min.x || cell.min.x > modes.max.x {
            continue;
        }
        let cell = cell.shrink2(vec2(4.0, 4.0));
        let target = Rename::Mode(coll.id.clone(), m.id.clone());
        if editable && st.rename.as_ref() == Some(&target) {
            if let Some(name) = rename_field(&mut wide, st, &target, &m.name, cell.shrink2(vec2(4.0, 2.0)), regular(11.0)) {
                app.run("variable.renameMode", json!({"collection": coll.id, "mode": m.id, "name": name}));
            }
            continue;
        }
        let name_pos = pos2(cell.min.x + 4.0, cell.center().y);
        if !editable {
            wide.painter().with_clip_rect(cell).text(name_pos, Align2::LEFT_CENTER, &m.name, regular(11.0), t.text_secondary);
            continue;
        }
        let resp = wide.interact(cell, Id::new(("vars_mode", &m.id)), Sense::click());
        if resp.hovered() {
            wide.painter().rect_filled(cell, CornerRadius::same(4), t.hover);
        }
        wide.painter().with_clip_rect(cell).text(name_pos, Align2::LEFT_CENTER, &m.name, regular(11.0), t.text_secondary);
        if resp.double_clicked() {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Rename mode"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if menu_item(ui, tr("Duplicate mode"), "") {
                let name = unique(&format!("{} copy", m.name), |n| coll.modes.iter().any(|x| x.name == n));
                let made = app.run("variable.addMode", json!({"collection": coll.id, "name": name, "from": m.id}));
                if let Some(id) = made.and_then(|r| r["id"].as_str().map(str::to_string)) {
                    start_rename(st, Rename::Mode(coll.id.clone(), id));
                }
                ui.close();
            }
            // The default mode is the first column: moving a column changes it.
            let last = coll.modes.len() - 1;
            for (show, label, to) in [
                (i > 0, tr_noop("Set as default"), 0),
                (i > 0, tr_noop("Move column left"), i.saturating_sub(1)),
                (i < last, tr_noop("Move column right"), i + 1),
            ] {
                if show && menu_item(ui, tr(label), "") {
                    app.run("variable.moveMode", json!({"collection": coll.id, "mode": m.id, "to": to}));
                    ui.close();
                }
            }
            if coll.modes.len() > 1 && menu_item(ui, tr("Delete mode"), "") {
                app.run("variable.removeMode", json!({"collection": coll.id, "mode": m.id}));
                ui.close();
            }
        });
    }
    // The button that adds a mode, at the table's right end where nothing scrolls.
    let add = cols.add_rect(r);
    if editable
        && widgets::icon_button_at(ui, Id::new("vars_add_mode"), Rect::from_center_size(add.center(), vec2(24.0, 24.0)), Icon::Plus, tr("Add mode"))
    {
        let name = unique("Mode", |n| coll.modes.iter().any(|m| m.name == n));
        if let Some(id) = app.run("variable.addMode", json!({"collection": coll.id, "name": name})).and_then(|r| r["id"].as_str().map(str::to_string))
        {
            start_rename(st, Rename::Mode(coll.id.clone(), id));
        }
    }
    // The two fixed lines, then the edges that drag the columns' widths.
    ui.painter().line_segment([pos2(name_r.max.x, r.min.y), pos2(name_r.max.x, r.max.y)], Stroke::new(1.0, t.border));
    ui.painter().line_segment([pos2(add.min.x, r.min.y), pos2(add.min.x, r.max.y)], Stroke::new(1.0, t.border));
    column_edge(ui, Id::new("vars_name_edge"), name_r.max.x, r.min.x, r, &mut st.name_w, NAME_MIN, NAME_MAX);
    for i in 0..cols.count {
        let cell = cols.mode_rect(r, i);
        if cell.max.x <= modes.min.x + 3.0 || cell.max.x >= modes.max.x - 3.0 {
            continue;
        }
        column_edge(ui, Id::new(("vars_mode_edge", i)), cell.max.x, cell.min.x, r, &mut st.mode_w, MODE_MIN, MODE_MAX);
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

fn var_row(app: &mut FigApp, ui: &mut Ui, view: &View, coll: &Collection, v: &Variable, st: &mut Editor, cols: &Columns, slots: &mut Vec<Slot>) {
    let t = Tokens::get(ui.ctx());
    let editable = view.editable();
    let (r, resp) = ui.allocate_exact_size(vec2(cols.width, ROW_H), Sense::click_and_drag());
    slots.push(Slot::var(r, v.id.clone()));
    if resp.hovered() || (editable && st.details.as_deref() == Some(v.id.as_str())) {
        ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
    }
    // The name column: it stays put while the modes scroll beside it.
    let name_r = cols.name_rect(r);
    kind_glyph(ui, view.vars, v, coll, Rect::from_center_size(pos2(name_r.min.x + 24.0, r.center().y), vec2(14.0, 14.0)));
    let text_r = Rect::from_min_max(pos2(name_r.min.x + 40.0, r.min.y + 8.0), pos2(name_r.max.x - 8.0, r.max.y - 8.0));
    let target = Rename::Variable(v.id.clone());
    // An inherited variable is named (and deleted) in the collection it extends.
    let inherited = v.extends.is_some();
    // Dragged by its name column: the row carries the drag and the double-click that renames it
    // both (the values keep their own drags: scrubbing a number, selecting a hex field). An
    // inherited variable's place is the collection it extends's to give.
    if editable && !inherited {
        let on_name = |ui: &Ui| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < text_r.max.x);
        if resp.drag_started() && ui.input(|i| i.pointer.press_origin()).is_some_and(|p| p.x < text_r.max.x) {
            st.drag = Some(v.id.clone());
        } else if resp.hovered() && st.drag.is_none() && on_name(ui) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if st.drag.as_deref() == Some(v.id.as_str()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }
    if editable && st.rename.as_ref() == Some(&target) {
        if let Some(name) = rename_field(ui, st, &target, &v.name, text_r, regular(11.0)) {
            app.run("variable.update", json!({"variable": v.id, "name": name}));
        }
    } else {
        // Grouped names show their last part under the group's header.
        let short = v.name.rsplit('/').next().unwrap_or(&v.name);
        ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, short, regular(11.0), t.text);
    }
    // The mode cells, clipped to their strip so the ones scrolled out of it can't be hit.
    let modes = cols.modes_rect(r);
    let mut wide = ui.new_child(UiBuilder::new().max_rect(modes));
    wide.set_clip_rect(modes);
    for (i, m) in coll.modes.iter().enumerate() {
        let cell = cols.mode_rect(r, i);
        if cell.max.x < modes.min.x || cell.min.x > modes.max.x {
            continue;
        }
        let cell = Rect::from_min_max(pos2(cell.min.x + 4.0, r.center().y - 12.0), pos2(cell.max.x - 8.0, r.center().y + 12.0));
        value_cell(app, &mut wide, view, v, coll, &m.id, cell);
    }
    if editable {
        if !inherited && resp.double_clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x < text_r.max.x) {
            start_rename(st, target.clone());
        }
        crate::right::context_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Edit variable"), "") {
                st.details = Some(v.id.clone());
                ui.close();
            }
            if !inherited && menu_item(ui, tr("Rename"), "") {
                start_rename(st, target.clone());
                ui.close();
            }
            if !v.overrides.is_empty() && menu_item(ui, tr("Reset overrides"), "") {
                app.run("variable.resetOverride", json!({"variable": v.id}));
                ui.close();
            }
            if menu_item(ui, tr("Duplicate"), "") {
                app.run("variable.duplicate", json!({"variable": v.id}));
                ui.close();
            }
            if !inherited && menu_item(ui, tr("Delete variable"), "") {
                app.run("variable.delete", json!({"variable": v.id}));
                ui.close();
            }
        });
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.border));
}

/// The type marker before a variable's name: its colour (default mode), or a type glyph.
fn kind_glyph(ui: &Ui, vars: &Variables, v: &Variable, coll: &Collection, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    match v.kind {
        VariableType::Color => {
            let modes = coll.modes.first().map(|m| [(coll.id.clone(), m.id.clone())].into_iter().collect()).unwrap_or_default();
            if let Some(VariableValue::Color(c)) = vars.resolve(&v.id, &modes) {
                widgets::paint_swatch(p, rect, &figcraft_color::Paint::solid(c));
            }
        }
        VariableType::Number => {
            p.text(rect.center(), Align2::CENTER_CENTER, "#", semibold(12.0), t.icon_secondary);
        }
        VariableType::String => icons::paint(p, rect, Icon::Text, t.icon_secondary),
        VariableType::Boolean => {
            let track = Rect::from_center_size(rect.center(), vec2(14.0, 8.0));
            p.rect_stroke(track, CornerRadius::same(4), Stroke::new(1.2, t.icon_secondary), StrokeKind::Inside);
            p.circle_filled(pos2(track.max.x - 4.0, track.center().y), 2.0, t.icon_secondary);
        }
    }
}

/// `val` as a `variable.set` value.
fn value_json(val: &VariableValue) -> Value {
    match val {
        VariableValue::Color(c) => json!(c.to_css_hex()),
        VariableValue::Number(x) => json!(x),
        VariableValue::String(s) => json!(s),
        VariableValue::Boolean(b) => json!(b),
        VariableValue::Alias(a) => json!({"alias": a}),
    }
}

fn set(app: &mut FigApp, v: &Variable, mode: &str, value: Value) {
    app.run("variable.set", json!({"variable": v.id, "mode": mode, "value": value}));
}

/// One value cell: an editor for the value, or the aliased variable's chip; the variable button
/// on the right makes it an alias. A library's values only show.
fn value_cell(app: &mut FigApp, ui: &mut Ui, view: &View, v: &Variable, coll: &Collection, mode: &str, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let Some(val) = v.values.get(mode) else { return };
    let editable = view.editable();
    let id = Id::new(("var_cell", &v.id, mode));
    // The value editor leaves room for the alias button (and in extended collections an
    // override's reset button, so the column stays even).
    let overridden = v.overrides.contains(mode);
    let body = Rect::from_min_max(rect.min, pos2(rect.max.x - if coll.extends.is_some() { 50.0 } else { 26.0 }, rect.max.y));
    let alias_btn = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(22.0, 22.0));
    let hovered = editable && ui.rect_contains_pointer(rect.expand2(vec2(4.0, 6.0)));
    if overridden {
        let mark = Rect::from_min_size(pos2(rect.min.x - 4.0, rect.min.y + 3.0), vec2(2.0, rect.height() - 6.0));
        ui.painter().rect_filled(mark, CornerRadius::same(1), t.accent);
        ui.interact(mark.expand2(vec2(2.0, 0.0)), id.with("mark"), Sense::hover())
            .tip(tr("Overridden here (the collection it extends has another value)"));
        if hovered
            && widgets::icon_button_at(ui, id.with("reset"), alias_btn.translate(vec2(-24.0, 0.0)), Icon::Reset, tr("Reset to the inherited value"))
        {
            app.run("variable.resetOverride", json!({"variable": v.id, "mode": mode}));
        }
    }
    if let VariableValue::Alias(target) = val {
        let full = view.vars.get(target).map(|x| x.name.as_str()).unwrap_or(tr("Missing variable"));
        let resp = ui.interact(body, id.with("alias"), if editable { Sense::click() } else { Sense::hover() });
        let resp = if editable { resp.on_hover_text(tr("Change alias")) } else { resp };
        ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
        let chip = body.shrink2(vec2(3.0, 3.0));
        ui.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
        let border = if editable && resp.hovered() { t.border_strong } else { t.border };
        ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(chip.min.x + 11.0, chip.center().y), vec2(12.0, 12.0)),
            Icon::Variable,
            t.icon_secondary,
        );
        let text_r = Rect::from_min_max(pos2(chip.min.x + 21.0, chip.min.y), pos2(chip.max.x - 4.0, chip.max.y));
        ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, full, regular(11.0), t.text);
        if resp.clicked() {
            crate::variable_ui::open_alias(ui, &v.id, mode, v.kind, body.left_bottom());
        }
        // Detach: keep the value it resolves to now.
        if hovered && widgets::icon_button_at(ui, id.with("detach"), alias_btn, Icon::Detach, tr("Detach alias")) {
            let modes = [(coll.id.clone(), mode.to_string())].into_iter().collect();
            if let Some(resolved) = view.vars.resolve(&v.id, &modes) {
                set(app, v, mode, value_json(&resolved));
            }
        }
        return;
    }
    if !editable {
        read_only_value(ui, val, body);
        return;
    }
    match val {
        VariableValue::Color(c) => {
            ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
            let sw = Rect::from_center_size(pos2(body.min.x + 13.0, body.center().y), vec2(14.0, 14.0));
            widgets::paint_swatch(ui.painter(), sw, &figcraft_color::Paint::solid(*c));
            if ui.interact(sw.expand(3.0), id.with("swatch"), Sense::click()).tip(tr("Edit colour")).clicked() {
                // Its top on the swatch (a variable's paint has no place of its own to sit on).
                app.open_picker(PickerTarget::Variable { id: v.id.clone(), mode: mode.to_string() }, Some(sw.min.y));
            }
            let hex_r = Rect::from_min_max(pos2(sw.max.x + 8.0, body.min.y + 4.0), pos2(body.max.x - 44.0, body.max.y - 4.0));
            if let Some(h) = widgets::text_field_inline(ui, id.with("hex"), &c.to_hex(), hex_r, regular(11.0))
                && let Some(nc) = figcraft_color::Color::from_hex_or_name(&h)
            {
                // Six digits (or a colour name) keep the alpha; eight digits set it.
                let digits = h.trim().trim_start_matches('#');
                let nc = if digits.len() == 8 && digits.chars().all(|ch| ch.is_ascii_hexdigit()) { nc } else { nc.with_alpha(c.a) };
                set(app, v, mode, json!(nc.to_css_hex()));
            }
            alpha_text(ui, c, body);
        }
        VariableValue::Number(x) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body));
            match widgets::number_field(&mut child, id.with("num"), Lead::None, Some(*x), "", body.width(), 1.0) {
                Some(FieldEdit::Set(n)) => set(app, v, mode, json!(n)),
                Some(FieldEdit::Scrub(n, done)) => {
                    crate::typography::live(app, "Edit variable", "variable.set", json!({"variable": v.id, "mode": mode, "value": n}), done)
                }
                None => {}
            }
        }
        VariableValue::String(s) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body));
            if let Some(n) = widgets::text_field(&mut child, id.with("str"), s, body.width(), "") {
                set(app, v, mode, json!(n));
            }
        }
        VariableValue::Boolean(b) => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body.shrink2(vec2(6.0, 0.0))));
            if widgets::checkbox(&mut child, *b, if *b { tr("True") } else { tr("False") }) {
                set(app, v, mode, json!(!b));
            }
        }
        VariableValue::Alias(_) => {}
    }
    if hovered && widgets::icon_button_at(ui, id.with("mkalias"), alias_btn, Icon::Variable, tr("Create alias")) {
        crate::variable_ui::open_alias(ui, &v.id, mode, v.kind, body.left_bottom());
    }
}

/// A colour's alpha as a percentage at the right end of `body`.
fn alpha_text(ui: &Ui, c: &figcraft_color::Color, body: Rect) {
    let t = Tokens::get(ui.ctx());
    let pct = format!("{}%", (c.a * 100.0).round());
    ui.painter().text(pos2(body.max.x - 8.0, body.center().y), Align2::RIGHT_CENTER, pct, regular(11.0), t.text_secondary);
}

/// A value that only shows (a library's): its swatch and hex for colours, else its text.
fn read_only_value(ui: &Ui, val: &VariableValue, body: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(body, CornerRadius::same(5), t.field);
    let mut x = body.min.x + 8.0;
    if let VariableValue::Color(c) = val {
        let sw = Rect::from_center_size(pos2(body.min.x + 13.0, body.center().y), vec2(14.0, 14.0));
        widgets::paint_swatch(ui.painter(), sw, &figcraft_color::Paint::solid(*c));
        x = sw.max.x + 8.0;
        alpha_text(ui, c, body);
    }
    let text = match val {
        VariableValue::Boolean(b) => (if *b { tr("True") } else { tr("False") }).to_string(),
        other => other.display(),
    };
    let text_r = Rect::from_min_max(pos2(x, body.min.y), pos2(body.max.x - 44.0, body.max.y));
    ui.painter().with_clip_rect(text_r).text(text_r.left_center(), Align2::LEFT_CENTER, text, regular(11.0), t.text);
}

/// The bar under the table: the button that makes a variable, in the type its menu picks.
fn bottom_bar(app: &mut FigApp, ui: &mut Ui, st: &mut Editor, view: &View, coll: &Collection) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BOTTOM_H), Sense::hover());
    ui.painter().line_segment([pos2(r.min.x, r.min.y + 0.5), pos2(r.max.x, r.min.y + 0.5)], Stroke::new(1.0, t.border));
    let b = Rect::from_min_max(pos2(r.min.x + 8.0, r.center().y - 12.0), pos2((r.min.x + 172.0).min(r.max.x - 8.0), r.center().y + 12.0));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(b).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = widgets::button(&mut bar, tr("+ Create variable"), false, view.editable());
    if !view.editable() {
        return;
    }
    crate::right::menu_popup(&resp).show(|ui| {
        for kind in TYPES {
            let label = match kind {
                // The English label also names the new variable (document content).
                VariableType::Color => tr_noop("Color"),
                VariableType::Number => tr_noop("Number"),
                VariableType::String => tr_noop("String"),
                VariableType::Boolean => tr_noop("Boolean"),
            };
            if menu_item(ui, tr(label), "") {
                let name = unique(label, |n| view.in_collection(&coll.id).any(|v| v.name == n));
                let params = json!({"collection": coll.id, "name": name, "type": kind.id()});
                if let Some(id) = app.run("variable.create", params).and_then(|r| r["id"].as_str().map(str::to_string)) {
                    st.query.clear();
                    start_rename(st, Rename::Variable(id));
                }
                ui.close();
            }
        }
    });
}

/// Whether `from` would land in the gap right above `before`: nothing to move, nothing to undo.
fn already_there(slots: &[Slot], from: &str, before: &str) -> bool {
    let idx = |id: &str| slots.iter().position(|s| s.var.as_deref() == Some(id));
    matches!((idx(from), idx(before)), (Some(i), Some(j)) if j == i + 1)
}

/// The folder (group path) of the row at `idx`: the nearest group header at or before it
/// (`None` = the "All variables" root, out of every folder).
#[allow(dead_code)]
fn folder_of_row(slots: &[Slot], idx: usize) -> Option<String> {
    let mut path = None;
    for s in &slots[..=idx] {
        if let Some(p) = &s.group {
            path = p.clone();
        }
    }
    path
}

/// Where a row dropped at `p` lands. The gap the insertion line is in decides it, and nothing else:
/// a folder row under the pointer (a group header, a sidebar row) joins that folder; otherwise the
/// row under the pointer decides — its top half inserts before it, its bottom half after it. Half a
/// row above the first row or below the last one counts as well, so a drop just past the end of the
/// list still lands on it instead of doing nothing.
fn pick_target(slots: &[Slot], from: &str, p: Pos2) -> Option<DropTarget> {
    // A folder row under the pointer: the dragged row joins that folder (at its end).
    if let Some(s) = slots.iter().find(|s| s.group.is_some() && s.rect.contains(p)) {
        return Some(DropTarget::Group(s.group.clone().flatten()));
    }
    let rows: Vec<&Slot> = slots.iter().filter(|s| s.var.is_some()).collect();
    let (first, last) = (rows.first()?.rect, rows.last()?.rect);
    // Over the table's columns but past either end of the list — however far: the nearest end takes
    // it, so a drop in the empty space under the last row still moves the row to the end.
    let over = p.x >= first.min.x && p.x <= first.max.x;
    let (row, below) = match rows.iter().find(|s| s.rect.contains(p)) {
        Some(s) => (s.rect, p.y >= s.rect.center().y),
        None if over && p.y > last.center().y => (last, true),
        None if over && p.y < first.center().y => (first, false),
        None => return None,
    };
    let id = rows.iter().find(|s| s.rect == row)?.var.clone()?;
    if id == from {
        return None;
    }
    Some(if below { DropTarget::After(id) } else { DropTarget::Before(id) })
}

/// The row drag: the gap (or the folder) under the pointer shows while a row's name is dragged, and
/// dropping moves the variable there — one `variable.move`, one undo step.
fn drag_and_drop(app: &mut FigApp, ui: &mut Ui, st: &mut Editor, slots: &[Slot]) {
    let Some(from) = st.drag.clone() else { return };
    // Escape gives the drag up (and goes no further).
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        st.drag = None;
        return;
    }
    let Some(pos) = ui.input(|i| i.pointer.latest_pos()) else { return };
    // The row went away (an undo, another collection): nothing to drag.
    if !slots.iter().any(|s| s.var.as_deref() == Some(from.as_str())) {
        st.drag = None;
        return;
    }
    let t = Tokens::get(ui.ctx());
    let target = pick_target(slots, &from, pos);
    let rows = slots.iter().filter(|s| s.var.is_some()).map(|s| s.rect).reduce(|a, b| a.union(b));
    match (&target, rows) {
        (Some(DropTarget::Before(id)), Some(span)) => {
            if let Some(s) = slots.iter().find(|s| s.var.as_deref() == Some(id.as_str())) {
                widgets::insertion_line(ui.painter(), span.min.x, span.max.x, s.rect.min.y);
            }
        }
        (Some(DropTarget::After(id)), Some(span)) => {
            if let Some(s) = slots.iter().find(|s| s.var.as_deref() == Some(id.as_str())) {
                widgets::insertion_line(ui.painter(), span.min.x, span.max.x, s.rect.max.y);
            }
        }
        (Some(DropTarget::Group(_)), _) => {
            if let Some(s) = slots.iter().find(|s| s.group.is_some() && s.rect.contains(pos)) {
                ui.painter().rect_stroke(s.rect, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
            }
        }
        _ => {}
    }
    if let Ok(doc) = app.session.doc()
        && let Some(v) = doc.variables.get(&from)
    {
        let galley = ui.painter().layout_no_wrap(v.name.rsplit('/').next().unwrap_or(&v.name).to_string(), regular(11.0), t.text);
        widgets::drag_ghost(ui.ctx(), pos, galley);
    }
    if !ui.input(|i| i.pointer.any_released()) {
        return;
    }
    st.drag = None;
    let Some(target) = target else { return };
    let folder_of = |id: &str| app.session.doc().ok().and_then(|d| d.variables.get(id)).map(|v| group_of(&v.name).to_string()).unwrap_or_default();
    let (before, group) = match &target {
        DropTarget::Before(id) => {
            if already_there(slots, &from, id) {
                return;
            }
            (Some(id.clone()), None)
        }
        // After a row: before the variable that follows it when there is one — a folder header
        // always sits between two rows of different folders, so that variable is in the same
        // folder, and the row simply moves down within it. A folder header (or the end of the list)
        // after it instead means it is that folder's last row, so the gap is the end of *its*
        // folder, and the dragged row joins that folder.
        DropTarget::After(id) => {
            let at = slots.iter().position(|s| s.var.as_deref() == Some(id.as_str()));
            let next = at.and_then(|i| slots.get(i + 1)).and_then(|s| s.var.clone());
            match next {
                Some(n) if n != from && !already_there(slots, &from, &n) => (Some(n), None),
                // The dragged row is the one right under it: the gap is its own spot.
                Some(_) => return,
                // This row's folder ends here. Nothing to do if the dragged row is that folder's
                // last already (it would only add an empty undo step).
                None => {
                    let folder = folder_of(id);
                    let from_at = slots.iter().position(|s| s.var.as_deref() == Some(from.as_str()));
                    let already_last =
                        from_at.is_some_and(|i| slots.iter().skip(i + 1).find_map(|s| s.var.clone()).is_none_or(|n| folder_of(&n) != folder));
                    if already_last && folder == folder_of(&from) {
                        return;
                    }
                    (None, Some(folder))
                }
            }
        }
        DropTarget::Group(g) => (None, Some(g.clone().unwrap_or_default())),
    };
    let mut params = json!({"variable": from});
    if let Some(b) = before {
        params["before"] = json!(b);
    }
    if let Some(g) = group {
        params["group"] = json!(g);
    }
    app.run("variable.move", params);
}

/// The details of one of this file's variables: name, description, where pickers offer it and
/// whether publishing the file as a library leaves it out.
fn details(app: &mut FigApp, ctx: &Context, vars: &Variables, st: &mut Editor) {
    let Some(v) = st.details.as_ref().filter(|_| st.library.is_none()).and_then(|id| vars.get(id)).cloned() else {
        st.details = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    egui::Window::new(tr("Edit variable"))
        .id(Id::new("variable_details"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(280.0)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            ui.set_width(260.0);
            widgets::label(ui, tr("Name"), t.text_secondary);
            if let Some(n) = widgets::text_field(ui, Id::new(("var_detail_name", &v.id)), &v.name, 260.0, "") {
                app.run("variable.update", json!({"variable": v.id, "name": n}));
            }
            ui.add_space(6.0);
            widgets::label(ui, tr("Description"), t.text_secondary);
            if let Some(d) = widgets::text_field(ui, Id::new(("var_detail_desc", &v.id)), &v.description, 260.0, tr("What it's for")) {
                app.run("variable.update", json!({"variable": v.id, "description": d}));
            }
            ui.add_space(6.0);
            if widgets::checkbox(ui, v.hidden_from_publishing, tr("Hide from publishing")) {
                app.run("variable.update", json!({"variable": v.id, "hiddenFromPublishing": !v.hidden_from_publishing}));
            }
            // The names code uses for it per platform (generated CSS uses the Web one).
            ui.add_space(10.0);
            widgets::label(ui, tr("Code syntax"), t.text_secondary);
            for (key, label) in [("WEB", tr_noop("Web")), ("iOS", tr_noop("iOS")), ("ANDROID", tr_noop("Android"))] {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(vec2(60.0, 24.0), Sense::hover());
                    ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, tr(label), regular(11.0), t.text);
                    let cur = v.code_syntax.get(key).map_or("", String::as_str);
                    if let Some(n) = widgets::text_field_named(ui, Id::new(("var_detail_code", key, &v.id)), cur, 192.0, tr(label)) {
                        app.run("variable.update", json!({"variable": v.id, "codeSyntax": {key: n}}));
                    }
                });
            }
            let scopes = scopes_for(v.kind);
            if scopes.is_empty() {
                return;
            }
            ui.add_space(10.0);
            widgets::label(ui, tr("Scoping: where pickers offer it"), t.text_secondary);
            // "All" off hides it everywhere; on, from everywhere it fits.
            if widgets::checkbox(ui, v.scopes.is_none(), tr("All supported properties")) {
                app.run("variable.update", json!({"variable": v.id, "scopes": if v.scopes.is_none() { json!([]) } else { Value::Null }}));
            }
            let on: Vec<&str> = scopes.iter().map(|(id, _)| *id).filter(|id| v.in_scope(id)).collect();
            for (sid, label) in scopes {
                let checked = on.contains(sid);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if widgets::checkbox(ui, checked, tr(label)) {
                        let mut list = on.clone();
                        if checked {
                            list.retain(|x| x != sid);
                        } else {
                            list.push(sid);
                        }
                        app.run("variable.update", json!({"variable": v.id, "scopes": list}));
                    }
                });
            }
        });
    if !open {
        st.details = None;
    }
}

#[cfg(test)]
mod tests {
    use super::groups;

    #[test]
    fn groups_include_parents_once_and_sorted() {
        let names = ["space/sm", "space/md", "color/brand/blue", "color/text", "plain", "color/brand/red"];
        assert_eq!(groups(names.into_iter()), ["color", "color/brand", "space"]);
        assert!(groups(["a", "b"].into_iter()).is_empty());
    }
}
