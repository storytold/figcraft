//! The home screen: Recents (new file, open, join with a code, the Getting started tips, open and
//! recent files ([`crate::home_recents`]), and templates to start from on a first run), Online (your online files:
//! [`crate::online`]) and Templates (the starter templates as cards with thumbnails rendered from
//! the documents themselves, searched and filtered by kind).

use std::collections::BTreeMap;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::templates::{Kind, TEMPLATES, Template};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::i18n::{tr, tr_noop};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets;

/// The section the home screen shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Recents,
    Online,
    Templates,
    /// The project library ([`crate::home_projects`]): every file in it.
    Projects,
    /// The top-level folders.
    Folders,
    Favorites,
    /// One folder's page ([`Home::folder`]).
    Folder,
}

impl Section {
    /// The sidebar, top to bottom (the library's sections after the first three).
    const ALL: [(Section, Icon, &str); 6] = [
        (Section::Recents, Icon::Home, tr_noop("Recents")),
        (Section::Online, Icon::Online, tr_noop("Online")),
        (Section::Templates, Icon::Assets, tr_noop("Templates")),
        (Section::Projects, Icon::Page, tr_noop("All projects")),
        (Section::Folders, Icon::Folder, tr_noop("All folders")),
        (Section::Favorites, Icon::Star, tr_noop("Favorites")),
    ];

    /// The id `ui.set {homeSection}` and `ui.inspect` use.
    pub fn id(self) -> &'static str {
        match self {
            Section::Recents => "recents",
            Section::Online => "online",
            Section::Templates => "templates",
            Section::Projects => "projects",
            Section::Folders => "folders",
            Section::Favorites => "favorites",
            Section::Folder => "folder",
        }
    }

    pub fn parse(id: &str) -> Option<Section> {
        Section::ALL.into_iter().map(|(section, ..)| section).chain([Section::Folder]).find(|section| section.id() == id)
    }

    /// One of the project library's pages.
    pub fn library(self) -> bool {
        matches!(self, Section::Projects | Section::Folders | Section::Favorites | Section::Folder)
    }
}

/// The home screen's own state.
#[derive(Default)]
pub struct Home {
    pub section: Section,
    /// Searches known files and starter templates together from the home sidebar.
    pub file_query: String,
    /// The Templates search text and kind filter.
    pub query: String,
    pub kind: Option<Kind>,
    /// The recent files: what is known about each, and the selection.
    pub recents: crate::home_recents::Recents,
    /// The folder [`Section::Folder`] shows (an id of the project library).
    pub folder: Option<String>,
    /// The project library's pages, dialogs and sidebar.
    pub projects: crate::home_projects::ProjectsUi,
}

const SIDEBAR: f32 = 240.0;
const CARD_W: f32 = 248.0;
const THUMB_H: f32 = 148.0;
const CARD_H: f32 = THUMB_H + 86.0;
const GAP: f32 = 20.0;
/// Templates offered on a first run.
const FIRST_RUN: usize = 4;

/// The Getting started tips: (title, what to do, the command whose shortcut shows).
const TIPS: [(&str, &str, &str); 6] = [
    (tr_noop("Draw a frame"), tr_noop("Pick the Frame tool, then drag on the canvas or choose a device size"), "tool.frame"),
    (tr_noop("Add auto layout"), tr_noop("Select layers and they space themselves as their content changes"), "layout.add"),
    (tr_noop("Make a component"), tr_noop("Turn a layer into a component, reuse it, and change every copy at once"), "component.create"),
    (tr_noop("Prototype"), tr_noop("Link frames on the Prototype tab, then present the flow"), "proto.present"),
    (tr_noop("Share with a code"), tr_noop("Start sharing with the Share button, then pass on the code it gives"), "collab.share"),
    (tr_noop("Export"), tr_noop("Export frames and layers as PNG, JPG, SVG or PDF"), "file.export"),
];

/// A row of the home screen's file lists.
pub(crate) struct FileRow<'a> {
    /// The mark before the name, and its colour.
    pub mark: Option<(Icon, Color32)>,
    pub name: &'a str,
    pub ink: Color32,
    /// What its right end says (a path, where the file is), with its size and colour.
    pub detail: &'a str,
    pub detail_size: f32,
    pub detail_ink: Color32,
}

/// Paint `row` in `r` (highlighted while `resp` is hovered): the name first, the detail in the
/// room left after it, cut at its start; true when either is cut.
pub(crate) fn file_row(ui: &Ui, t: &Tokens, r: Rect, resp: &Response, row: &FileRow) -> bool {
    /// Between the name and the detail.
    const GAP: f32 = 16.0;
    /// A detail with less room than this is left out rather than shown as a bare ellipsis.
    const MIN_DETAIL: f32 = 48.0;
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    if let Some((icon, tint)) = row.mark {
        icons::paint(ui.painter(), Rect::from_min_size(pos2(r.min.x + 8.0, r.center().y - 8.0), vec2(16.0, 16.0)), icon, tint);
    }
    let (name_font, detail_font) = (regular(12.0), regular(row.detail_size));
    let width = |s: &str, font: &egui::FontId| ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    let room = (r.width() - 40.0).max(0.0);
    let name_w = width(row.name, &name_font);
    let detail_w = width(row.detail, &detail_font);
    // When both don't fit, the name keeps at least 60% of the row and the detail gets the rest.
    let name_max = if name_w + GAP + detail_w <= room { name_w } else { name_w.min((room - GAP - detail_w).max(room * 0.6)) };
    let name = widgets::elide_font(ui, row.name, name_max, name_font.clone());
    let detail_room = room - width(&name, &name_font) - GAP;
    let detail = if row.detail.is_empty() || detail_room < MIN_DETAIL {
        String::new()
    } else {
        widgets::elide_start_font(ui, row.detail, detail_room, detail_font.clone())
    };
    let cut = name != row.name || detail != row.detail;
    ui.painter().text(pos2(r.min.x + 32.0, r.center().y), Align2::LEFT_CENTER, name, name_font, row.ink);
    ui.painter().text(pos2(r.max.x - 8.0, r.center().y), Align2::RIGHT_CENTER, detail, detail_font, row.detail_ink);
    cut
}

/// A row of Recents' own lists: laid out in place, named for screen readers, ringed with the
/// keyboard on it; true when chosen.
pub(crate) fn listed(ui: &mut Ui, t: &Tokens, id: Id, height: f32, row: &FileRow) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(640.0), height), Sense::hover());
    let mut resp = ui.interact(r, id, Sense::click());
    a11y::button(&resp, row.name);
    if file_row(ui, t, r, &resp, row) {
        // Cut to fit: in full on hover.
        resp = resp.on_hover_text(format!("{}\n{}", row.name, row.detail));
    }
    focus_ring(ui, t, r, &resp, 5);
    resp.clicked()
}

pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    dropped(app, ui.ctx());
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.panel)).show(ui, |ui| {
        let rect = ui.max_rect();
        let left = Rect::from_min_size(rect.min, vec2(SIDEBAR, rect.height()));
        ui.painter().line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.border));
        sidebar(app, ui, &t, left);
        if app.ui.home_view.file_query.trim().is_empty() && app.ui.home_view.section == Section::Online {
            // Your online files lay themselves out in the area (and scroll their own list).
            let area = Rect::from_min_max(pos2(left.max.x + 40.0, rect.min.y + 24.0), rect.max - vec2(40.0, 24.0));
            crate::online::section(app, ui, area, &t);
            opened_join(app);
            return;
        }
        let main = Rect::from_min_max(pos2(left.max.x, rect.min.y), rect.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            egui::ScrollArea::vertical().id_salt("home").auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(40, 24)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    if app.ui.home_view.file_query.trim().is_empty() {
                        match app.ui.home_view.section {
                            Section::Templates => templates(app, ui, &t),
                            section if section.library() && crate::home_projects::available() => crate::home_projects::section(app, ui, &t),
                            _ => recents(app, ui, &t),
                        }
                    } else {
                        search_results(app, ui, &t);
                        opened_join(app);
                    }
                });
            });
        });
    });
    // The whole window is the drop target, not only the canvas.
    crate::drop_overlay(ui.ctx(), ui.ctx().content_rect(), tr("Drop to open"));
}

/// Files dropped on the home screen: each one the app opens becomes a document of its own (the
/// editor then shows it), behind an "Opening as a new file" card ([`crate::pending_open`]); anything
/// else is turned away with a toast. A file that fails to open keeps the engine's own error.
fn dropped(app: &mut FigApp, ctx: &egui::Context) {
    let files = ctx.input(|i| i.raw.dropped_files.clone());
    for f in files {
        if !crate::opens(f.path()) {
            app.toast(tr("Drop a .figcraft, .fig, .sketch, .json, .svg or image file."));
            continue;
        }
        crate::pending_open::open_dropped(app, &f);
    }
}

/// Draw the Home sidebar: app icon and wordmark, file/template search (Escape clears it)
/// and section tabs; choosing a section clears the file query.
fn sidebar(app: &mut FigApp, ui: &mut Ui, t: &Tokens, left: Rect) {
    let mut y = left.min.y + 20.0;
    crate::theme::paint_app_icon(ui, Rect::from_min_size(pos2(left.min.x + 12.0, y - 4.0), vec2(32.0, 32.0)));
    crate::theme::paint_wordmark(ui.painter(), pos2(left.min.x + 52.0, y + 12.0), 16.0, t);
    y += 52.0;
    let search = Rect::from_min_size(pos2(left.min.x + 8.0, y), vec2(SIDEBAR - 16.0, widgets::FIELD_H));
    ui.scope_builder(egui::UiBuilder::new().max_rect(search), |ui| {
        let response = widgets::text_input(
            ui,
            Id::unique("home_file_search"),
            &mut app.ui.home_view.file_query,
            search.width(),
            tr("Search files and templates"),
        );
        a11y::text(&response, tr("Search files and templates"), &app.ui.home_view.file_query);
        if (response.has_focus() || response.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.ui.home_view.file_query.clear();
        }
    });
    y += widgets::FIELD_H + 12.0;
    for (i, (section, icon, label)) in Section::ALL.into_iter().enumerate() {
        if section.library() && !crate::home_projects::available() {
            continue;
        }
        if i == 3 {
            // The project library below the files and templates.
            y += 6.0;
            ui.painter().line_segment([pos2(left.min.x + 16.0, y), pos2(left.max.x - 16.0, y)], Stroke::new(1.0, t.border));
            y += 8.0;
        }
        let r = Rect::from_min_size(pos2(left.min.x + 8.0, y), vec2(SIDEBAR - 16.0, 32.0));
        let resp = ui.interact(r, ui.scope_id().with(("home_section", label)), Sense::click());
        let current = app.ui.home_view.file_query.trim().is_empty() && app.ui.home_view.section == section;
        a11y::tab(&resp, tr(label), current);
        if current || resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), if current { t.selected_row } else { t.hover });
        }
        focus_ring(ui, t, r, &resp, 5);
        icons::paint(ui.painter(), Rect::from_min_size(pos2(r.min.x + 10.0, r.center().y - 8.0), vec2(16.0, 16.0)), icon, t.icon);
        ui.painter().text(pos2(r.min.x + 36.0, r.center().y), Align2::LEFT_CENTER, tr(label), regular(12.0), t.text);
        if resp.clicked() {
            app.ui.home_view.section = section;
            app.ui.home_view.file_query.clear();
            app.ui.home_view.projects.filter.clear();
        }
        y += 34.0;
    }
    // The favourites under them, to the bottom of the sidebar.
    let rest = Rect::from_min_max(pos2(left.min.x + 8.0, y + 14.0), pos2(left.max.x - 8.0, left.max.y - 12.0));
    crate::home_projects::sidebar(app, ui, t, rest);
}

fn file_matches(name: &str, location: &str, query: &str) -> bool {
    name.to_lowercase().contains(query) || location.to_lowercase().contains(query)
}

/// One search over known files and starter templates. It never sends a query or owner token to a
/// server; online files are opened through their existing owner-token command only on click.
fn search_results(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    heading(ui, t, tr("Search results"));
    let query = app.ui.home_view.file_query.trim().to_lowercase();
    let mut count = open_files(app, ui, t, &query);
    count += crate::home_recents::section(app, ui, t, &query);
    count += crate::home_projects::search(app, ui, t, &query);

    let online: Vec<_> = app.session.prefs.online_files.iter().filter(|f| file_matches(&f.name, &f.server, &query)).cloned().collect();
    let mut open_online = None;
    if !online.is_empty() {
        subheading(ui, t, tr("Online files"));
        for f in &online {
            count += 1;
            let host = figcraft_engine::online::server_host(&f.server);
            let icon = crate::online::mark(f.shared.is_some());
            let row = FileRow {
                mark: Some((icon, if f.shared.is_some() { t.success } else { t.icon_secondary })),
                name: &f.name,
                ink: t.text,
                detail: host,
                detail_size: 11.0,
                detail_ink: t.text_secondary,
            };
            if listed(ui, t, ui.scope_id().with(("search_online", &f.server, &f.file)), 36.0, &row) {
                open_online = Some(f.clone());
            }
        }
    }
    let templates: Vec<&Template> = TEMPLATES.iter().filter(|template| matches(template, &query, None)).collect();
    if !templates.is_empty() {
        subheading(ui, t, tr("Templates"));
        template_grid(app, ui, t, &templates);
    }
    if count == 0 && templates.is_empty() {
        ui.label(egui::RichText::new(tr("No files or templates match your search")).font(semibold(13.0)).color(t.text));
        ui.add_space(6.0);
        widgets::label(ui, tr("Try another search, or browse templates."), t.text_secondary);
        ui.add_space(16.0);
        if widgets::button(ui, tr("Show all templates"), false, true).clicked() {
            app.ui.home_view.file_query.clear();
            app.ui.home_view.section = Section::Templates;
        }
    }
    if let Some(file) = open_online {
        crate::online::open(app, &file);
    }
}

/// A ring around a card or row that has the keyboard.
pub(crate) fn focus_ring(ui: &Ui, t: &Tokens, rect: Rect, resp: &Response, radius: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(radius), Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
}

/// A card's background and border (lit on hover, ringed with the keyboard on it).
fn card_frame(ui: &Ui, t: &Tokens, rect: Rect, resp: &Response) {
    ui.painter().rect_filled(rect, CornerRadius::same(8), if resp.hovered() { t.hover } else { t.panel });
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, t.border), StrokeKind::Inside);
    focus_ring(ui, t, rect, resp, 8);
}

fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(egui::RichText::new(text).font(semibold(18.0)).color(t.text));
    ui.add_space(24.0);
}

pub(crate) fn subheading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add_space(28.0);
    ui.label(egui::RichText::new(text).font(semibold(12.0)).color(t.text_secondary));
    ui.add_space(12.0);
}

/// A card with an icon tile, a title and a line under it; true when chosen.
fn action_card(ui: &mut Ui, t: &Tokens, icon: Icon, title: &str, sub: &str) -> bool {
    let (r, mut resp) = ui.allocate_exact_size(vec2(260.0, 72.0), Sense::click());
    a11y::button(&resp, title);
    // One line each inside the card; a long translation ends with an ellipsis, in full on hover.
    let text_w = r.width() - 72.0;
    let (title_shown, sub_shown) = (widgets::elide_font(ui, title, text_w, semibold(12.0)), widgets::elide(ui, sub, text_w));
    if title_shown != title || sub_shown != sub {
        resp = resp.on_hover_text(format!("{title}\n{sub}"));
    }
    card_frame(ui, t, r, &resp);
    let ir = Rect::from_min_size(pos2(r.min.x + 16.0, r.center().y - 16.0), vec2(32.0, 32.0));
    ui.painter().rect_filled(ir, CornerRadius::same(6), t.accent);
    icons::paint(ui.painter(), ir.shrink(8.0), icon, t.accent_on);
    ui.painter().text(pos2(r.min.x + 60.0, r.center().y - 8.0), Align2::LEFT_CENTER, title_shown, semibold(12.0), t.text);
    ui.painter().text(pos2(r.min.x + 60.0, r.center().y + 9.0), Align2::LEFT_CENTER, sub_shown, regular(11.0), t.text_secondary);
    resp.clicked()
}

fn recents(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    heading(ui, t, tr("Recents"));
    let (mut new_file, mut new_board, mut open) = (false, false, false);
    ui.horizontal_wrapped(|ui| {
        new_file = action_card(ui, t, Icon::PenNib, tr("New design file"), tr("Start from a blank canvas"));
        new_board = action_card(ui, t, Icon::Board, tr("New board"), tr("Sticky notes, shapes and connectors"));
        open = action_card(ui, t, Icon::Page, tr("Open file"), tr("Open .figcraft, .svg or images"));
    });
    if new_file || new_board {
        app.run(if new_board { "file.newBoard" } else { "file.new" }, json!({}));
        app.ui.home = false;
    }
    if open {
        app.open_dialog();
    }
    // Join a file someone shared: the join field, and what the join is doing.
    subheading(ui, t, tr("Join with a code"));
    ui.scope(|ui| {
        ui.set_max_width(540.0);
        if let Some(session) = crate::live::join_row(app, ui, 260.0) {
            app.ui.live.home_join = Some(session);
        }
        if let Some(session) = app.ui.live.home_join {
            crate::live::join_status(app, ui, t, session);
        }
    });
    opened_join(app);
    if !app.session.prefs.getting_started_dismissed {
        ui.add_space(28.0);
        getting_started(app, ui, t);
    }
    open_files(app, ui, t, "");
    if crate::home_recents::listed(&app.session.prefs.recent_files).next().is_none() {
        // A first run: nothing to reopen yet, so offer something to start from.
        subheading(ui, t, tr("Start from a template"));
        let shown: Vec<&Template> = TEMPLATES.iter().take(FIRST_RUN).collect();
        template_grid(app, ui, t, &shown);
        ui.add_space(12.0);
        if widgets::button(ui, tr("See all templates"), false, true).clicked() {
            app.ui.home_view.section = Section::Templates;
        }
        return;
    }
    crate::home_recents::section(app, ui, t, "");
}

/// The Open files list: the documents whose name or path has `query` (lower case; all for ""), a
/// row each; choosing one shows its tab. How many are listed.
fn open_files(app: &mut FigApp, ui: &mut Ui, t: &Tokens, query: &str) -> usize {
    let hit = |d: &&figcraft_engine::DocState| file_matches(&d.doc.name, d.path.as_deref().unwrap_or_default(), query);
    let count = app.session.docs.iter().filter(hit).count();
    if count == 0 {
        return 0;
    }
    subheading(ui, t, tr("Open files"));
    let mut switch = None;
    for (i, d) in app.session.docs.iter().enumerate().filter(|(_, d)| hit(d)) {
        let detail = d.path.as_deref().unwrap_or_default();
        let row = FileRow {
            mark: Some((icons::file_mark(d.doc.kind), ACCENT)),
            name: &d.doc.name,
            ink: t.text,
            detail,
            detail_size: 11.0,
            detail_ink: t.text_secondary,
        };
        if listed(ui, t, ui.scope_id().with(("open", d.uid)), 36.0, &row) {
            switch = Some(i);
        }
    }
    if let Some(i) = switch {
        app.run("doc.switch", json!({"index": i}));
        app.ui.home = false;
    }
    count
}

/// The file joined from the home screen (a code, or one of your online files) opened in a tab of
/// its own: show it. (A join that was left is forgotten.)
fn opened_join(app: &mut FigApp) {
    match app.ui.live.home_join.map(|id| app.session.live.get(id).map(figcraft_engine::live::Live::syncing)) {
        Some(Some(true)) => {
            app.ui.live.home_join = None;
            app.ui.home = false;
        }
        Some(None) => app.ui.live.home_join = None,
        _ => {}
    }
}

/// The Getting started card: short tips with their shortcuts as the keymap has them now, and a
/// button that puts the card away (Help ▸ Getting started brings it back).
fn getting_started(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let width = ui.available_width().min(3.0 * CARD_W + 2.0 * GAP);
    let columns = if width >= 720.0 { 3 } else { 1 };
    let (cell_w, cell_h) = ((width - 32.0) / columns as f32, 62.0);
    let rows = TIPS.len().div_ceil(columns);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 52.0 + rows as f32 * cell_h + 8.0), Sense::hover());
    a11y::container_bounds(ui, rect);
    ui.painter().rect_filled(rect, CornerRadius::same(8), t.field);
    ui.painter().text(rect.min + vec2(16.0, 24.0), Align2::LEFT_CENTER, tr("Getting started"), semibold(13.0), t.text);
    let close = Rect::from_min_size(pos2(rect.max.x - 36.0, rect.min.y + 12.0), vec2(24.0, 24.0));
    if widgets::icon_button_at(
        ui,
        ui.scope_id().with("tips_close"),
        close,
        Icon::Close,
        tr("Hide these tips (Getting started in the Help menu shows them again)"),
    ) {
        app.run("help.gettingStarted", json!({"show": false}));
        app.services.save_prefs(&app.session.prefs);
    }
    for (i, (title, line, command)) in TIPS.into_iter().enumerate() {
        let cell = Rect::from_min_size(
            rect.min + vec2(16.0 + (i % columns) as f32 * cell_w, 48.0 + (i / columns) as f32 * cell_h),
            vec2(cell_w - 16.0, cell_h),
        );
        let badge = Rect::from_min_size(cell.min + vec2(0.0, 2.0), vec2(20.0, 20.0));
        ui.painter().rect_filled(badge, CornerRadius::same(10), t.panel);
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, (i + 1).to_string(), semibold(10.0), t.text_secondary);
        let x = cell.min.x + 30.0;
        let title_end = ui.painter().text(pos2(x, cell.min.y + 12.0), Align2::LEFT_CENTER, tr(title), semibold(12.0), t.text).max.x;
        if let Some(key) = shortcut(command) {
            ui.painter().text(pos2(title_end + 8.0, cell.min.y + 12.0), Align2::LEFT_CENTER, key, regular(11.0), t.text_tertiary);
        }
        let galley = lines(ui, tr(line), regular(11.0), t.text_secondary, cell.max.x - x, 2);
        ui.painter().galley(pos2(x, cell.min.y + 24.0), galley, t.text_secondary);
    }
}

/// A command's shortcut as this platform writes it, as the keymap has it now.
pub(crate) fn shortcut(command: &str) -> Option<String> {
    crate::shortcuts::shortcut(command).map(|key| figcraft_engine::keymap::display(key, crate::shortcuts::platform()))
}

/// `text` wrapped at `width`, at most `rows` lines (the last one ends with an ellipsis).
fn lines(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, color, width.max(1.0));
    job.wrap.max_rows = rows;
    ui.fonts_mut(|f| f.layout_job(job))
}

fn matches(template: &Template, query: &str, kind: Option<Kind>) -> bool {
    let q = query.trim().to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    kind.is_none_or(|k| k == template.kind)
        && (q.is_empty()
            || [template.name, template.description, template.kind.label()].into_iter().any(|s| has(s) || has(tr(s)))
            || has(template.size))
}

/// Draw template search and kind filter, show no-match guidance and a "Show all templates"
/// button that clears the query and kind, and lay matching templates out in a grid.
fn templates(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    heading(ui, t, tr("Templates"));
    ui.horizontal(|ui| {
        let search = widgets::text_input(ui, Id::unique("home_template_search"), &mut app.ui.home_view.query, 260.0, tr("Search templates"));
        a11y::text(&search, tr("Search templates"), &app.ui.home_view.query);
        let labels: Vec<&str> = std::iter::once(tr("All")).chain(Kind::ALL.iter().map(|k| tr(k.label()))).collect();
        let active = app.ui.home_view.kind.and_then(|k| Kind::ALL.iter().position(|x| *x == k)).map_or(0, |i| i + 1);
        ui.scope(|ui| {
            ui.set_max_width((labels.len() as f32 * 124.0).min(ui.available_width()));
            if let Some(i) = widgets::segmented_text(ui, &labels, active) {
                app.ui.home_view.kind = i.checked_sub(1).and_then(|i| Kind::ALL.get(i).copied());
            }
        });
    });
    ui.add_space(24.0);
    let (query, kind) = (app.ui.home_view.query.clone(), app.ui.home_view.kind);
    let shown: Vec<&Template> = TEMPLATES.iter().filter(|x| matches(x, &query, kind)).collect();
    if shown.is_empty() {
        ui.add_space(24.0);
        ui.label(egui::RichText::new(tr("No templates match your search")).font(semibold(13.0)).color(t.text));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr("Try another word, or look at every kind.")).font(regular(12.0)).color(t.text_secondary));
        ui.add_space(14.0);
        if widgets::button(ui, tr("Show all templates"), false, true).clicked() {
            app.ui.home_view.query.clear();
            app.ui.home_view.kind = None;
        }
        return;
    }
    template_grid(app, ui, t, &shown);
}

/// Template cards in as many columns as fit; choosing one opens a new document made from it.
fn template_grid(app: &mut FigApp, ui: &mut Ui, t: &Tokens, list: &[&Template]) {
    let columns = (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let dark = ui.visuals().dark_mode;
    let mut thumbs: Thumbs = ui.data(|d| d.get_temp(Id::unique("template_thumbs"))).unwrap_or_default();
    let mut chosen = None;
    let mut rendered = false;
    for row in list.chunks(columns) {
        ui.horizontal(|ui| {
            for template in row {
                let key = (template.id, dark);
                if !thumbs.0.contains_key(&key) && ui.is_rect_visible(Rect::from_min_size(ui.cursor().min, vec2(CARD_W, CARD_H))) {
                    if rendered {
                        // One a frame keeps the screen moving; the rest follow.
                        ui.ctx().request_repaint();
                    } else {
                        thumbs.0.insert(key, thumbnail(ui.ctx(), template, app.session.new_page_background()));
                        rendered = true;
                    }
                }
                if template_card(ui, t, template, thumbs.0.get(&key).and_then(Option::as_ref)) {
                    chosen = Some(*template);
                }
            }
        });
        ui.add_space(GAP);
    }
    ui.data_mut(|d| d.insert_temp(Id::unique("template_thumbs"), thumbs));
    if let Some(template) = chosen
        && app.run("file.newFromTemplate", json!({"template": template.id, "name": tr(template.name)})).is_some()
    {
        app.ui.home = false;
    }
}

fn template_card(ui: &mut Ui, t: &Tokens, template: &Template, thumb: Option<&Thumb>) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
    let (name, description) = (tr(template.name), tr(template.description));
    a11y::button(&resp, &format!("{name}: {description}"));
    card_frame(ui, t, r, &resp);
    let picture = Rect::from_min_size(r.min, vec2(CARD_W, THUMB_H)).shrink(1.0);
    let top = CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 };
    match thumb {
        Some(thumb) => {
            ui.painter().rect_filled(picture, top, thumb.page);
            let size = thumb.texture.size_vec2();
            let inner = picture.shrink(10.0);
            let fit = (inner.width() / size.x).min(inner.height() / size.y);
            let image = Rect::from_center_size(inner.center(), size * fit);
            ui.painter().image(thumb.texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(picture, top, t.field);
            icons::paint(ui.painter(), Rect::from_center_size(picture.center(), vec2(20.0, 20.0)), Icon::Frame, t.icon_secondary);
        }
    }
    ui.painter().line_segment([picture.left_bottom(), picture.right_bottom()], Stroke::new(1.0, t.border));
    let (x, w) = (r.min.x + 14.0, CARD_W - 28.0);
    let y = picture.max.y + 20.0;
    ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, widgets::elide_font(ui, name, w, semibold(12.0)), semibold(12.0), t.text);
    let galley = lines(ui, description, regular(11.0), t.text_secondary, w, 2);
    ui.painter().galley(pos2(x, y + 11.0), galley, t.text_secondary);
    let tag = format!("{} · {}", tr(template.kind.label()), template.size);
    ui.painter().text(pos2(x, r.max.y - 15.0), Align2::LEFT_CENTER, widgets::elide_font(ui, &tag, w, regular(10.0)), regular(10.0), t.text_tertiary);
    resp.clicked()
}

/// A template's picture: its first page's frames, and the page colour they sit on.
#[derive(Clone)]
struct Thumb {
    texture: egui::TextureHandle,
    page: Color32,
}

/// Pictures by template and theme (the page colour follows the theme); `None`: nothing to show.
#[derive(Clone, Default)]
struct Thumbs(BTreeMap<(&'static str, bool), Option<Thumb>>);

/// Build `template` and render the top of its first page to fit a card.
fn thumbnail(ctx: &egui::Context, template: &Template, background: figcraft_color::Color) -> Option<Thumb> {
    let doc = template.build(background).ok()?;
    let frames = doc.node(doc.first_page()?)?.children.clone();
    let bounds = frames.iter().filter_map(|id| figcraft_render::export_bounds(&doc, *id)).reduce(|a, b| a.union(b))?;
    if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
        return None;
    }
    // A page much taller than a card shows its top, as wide as the card.
    let (w, h) = (f64::from(CARD_W - 22.0), f64::from(THUMB_H - 22.0));
    let area = figcraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1.min(bounds.y0 + bounds.width() * h / w * 1.5));
    let scale = (w / area.width()).min(h / area.height()) * f64::from(ctx.pixels_per_point());
    let pix = figcraft_render::render_area(&doc, &frames, area, scale.min(1.0))?;
    let image = crate::render_worker::screen_image(&pix, doc.color_profile);
    let texture = ctx.load_texture(format!("template-thumb-{}", template.id), image, egui::TextureOptions::LINEAR);
    Some(Thumb { texture, page: crate::theme::c32(background) })
}

/// Tests: the templates whose pictures are cached for the light or dark theme.
#[cfg(test)]
pub(crate) fn cached_thumbnails(ctx: &egui::Context, dark: bool) -> Vec<&'static str> {
    let thumbs: Thumbs = ctx.data(|d| d.get_temp(Id::unique("template_thumbs"))).unwrap_or_default();
    thumbs.0.iter().filter(|((_, d), thumb)| *d == dark && thumb.is_some()).map(|((id, _), _)| *id).collect()
}
