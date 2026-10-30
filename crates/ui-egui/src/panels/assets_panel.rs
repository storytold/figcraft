//! The Assets tab: components as thumbnail tiles filed in folders (their page, the frames holding
//! them, then the slashes in their names; a folder folds away), searched, clicked to place or
//! dragged onto the canvas to drop an instance into the frame or auto layout slot under the pointer
//! (with Alt held, onto an instance to swap it), and styles (click to apply to the selection), for
//! this file and for every enabled library, with a "Library updates available" banner. The
//! libraries view publishes this file as a library (to the file, or to its server for the team,
//! with how other files use it), enables library files and team libraries (by a share link), and
//! swaps one library for another.
//!
//! Thousands of assets stay smooth: only the tiles in view are laid out, and thumbnails render
//! when their tile first shows (a few per frame), are cached, and re-render only when their
//! component's layers change (compared by the document's shared layer snapshots).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind, Style, StyleKind};
use figcraft_engine::library;
use figcraft_engine::library_server::{is_server_link, shown_path};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{COMPONENT, Tokens, regular, semibold};

const TILE_LABEL: f32 = 22.0;
const TILE_GAP: f32 = 8.0;
/// Re-check thumbnails at most this often (seconds) while the document changes.
const THUMB_REFRESH: f64 = 0.4;
/// Thumbnails kept (those out of view go first beyond this).
const MAX_THUMBS: usize = 600;
/// Thumbnails rendered per frame (the rest follow on the next frames).
const THUMBS_PER_FRAME: usize = 8;
/// Layers compared per component when checking whether its thumbnail is stale.
const MAX_COMPARED: usize = 20_000;
/// Look for changed library files this often (seconds) while the panel shows.
const LIBRARY_REFRESH: f64 = 2.0;
/// Ask team libraries' servers for newer versions this often (seconds) while the panel shows.
// shortcut: polled (a few bytes when nothing changed); a server push needs a connection per
// library, worth it once libraries are used beyond the Assets panel.
const SERVER_REFRESH: f64 = 30.0;
/// Components listed in a team library's usage.
const USAGE_ROWS: usize = 8;
const PAD: f32 = 16.0;
/// The team library commands the panel runs without waiting for the server.
const TO_SERVER: &str = "library.publishToServer";
const USAGE: &str = "library.usage";
/// The periods Show usage counts insertions and detachments over: days, the choice's label and
/// the summary's.
const USAGE_PERIODS: [(u64, &str, &str); 3] = [
    (30, tr_noop("30 days"), tr_noop("Past 30 days")),
    (90, tr_noop("90 days"), tr_noop("Past 90 days")),
    (365, tr_noop("1 year"), tr_noop("Past year")),
];
const ADD: &str = "library.add";
const CHECK: &str = "library.check";

/// Where an asset comes from: this file or an enabled library (its id).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Source {
    Local,
    Library(String),
}

#[derive(Clone)]
struct Tile {
    id: NodeId,
    /// The full name (the tooltip's).
    name: String,
    /// The name on the tile: its last slash part.
    label: String,
    set: bool,
    /// Kept out of the library this file publishes (its name starts with `.` or `_`).
    hidden: bool,
}

impl Tile {
    /// Asset `id` of `doc` with its folder (the folders joined).
    fn new(doc: &Document, id: NodeId, name: &str, set: bool) -> (String, Self) {
        let (folders, label) = library::asset_folder(doc, id);
        let label = if label.is_empty() { name.to_string() } else { label };
        (folders.join(" / "), Self { id, name: name.to_string(), label, set, hidden: library::hidden_name(name) })
    }
}

/// Tiles under one folder heading (an empty name: no heading).
#[derive(Clone)]
struct Folder {
    /// Remembers whether the folder is folded away.
    key: String,
    name: String,
    tiles: Vec<Tile>,
}

/// `tiles` (with their folders) sorted and gathered by folder, for the source keyed `src`.
fn foldered(src: &str, mut tiles: Vec<(String, Tile)>) -> Vec<Folder> {
    tiles.sort_by(|(fa, a), (fb, b)| {
        fa.to_lowercase().cmp(&fb.to_lowercase()).then(a.label.to_lowercase().cmp(&b.label.to_lowercase())).then(a.id.cmp(&b.id))
    });
    let mut out: Vec<Folder> = Vec::new();
    for (folder, t) in tiles {
        match out.last_mut() {
            Some(f) if f.name == folder => f.tiles.push(t),
            _ => out.push(Folder { key: format!("{src}/{folder}"), name: folder, tiles: vec![t] }),
        }
    }
    out
}

#[derive(Clone)]
struct StyleRow {
    id: String,
    name: String,
    kind: StyleKind,
}

impl StyleRow {
    fn of(s: &Style) -> Self {
        Self { id: s.id.clone(), name: s.name.clone(), kind: s.kind.clone() }
    }
}

/// An enabled library as listed: its snapshot (when it could be read) and matching assets.
#[derive(Clone)]
struct Section {
    id: String,
    name: String,
    doc: Option<Arc<Document>>,
    error: Option<String>,
    tiles: Vec<Folder>,
    styles: Vec<StyleRow>,
}

/// What the panel lists; rebuilt only when the document, the search or a library changes.
struct Listing {
    key: (u64, String, u64),
    local: Vec<Folder>,
    local_styles: Vec<StyleRow>,
    libraries: Vec<Section>,
    /// Library components and styles in this file with a newer published version.
    updates: usize,
}

/// Rendered thumbnails keyed by source and component (None: it renders to nothing), and the
/// documents they were rendered from (this file's, each library's snapshot).
#[derive(Clone, Default)]
struct Thumbs {
    map: HashMap<(Source, NodeId), Option<egui::TextureHandle>>,
    docs: HashMap<Source, Arc<Document>>,
    checked_at: f64,
}

/// A component being dragged out of the panel.
#[derive(Clone)]
struct AssetDrag {
    src: Source,
    component: NodeId,
    set: bool,
}

/// What the user did in the list this frame.
enum Act {
    Place(Source, NodeId),
    Drag(AssetDrag),
    Style(Source, String),
    Toggle(String),
    /// Hide the local component when publishing, or show it again.
    Hide(NodeId, bool),
}

/// Stable egui id for the assets drag payload.
fn drag_id() -> Id {
    Id::unique("asset_drag")
}

/// Whether component `id` has the same layers in two versions of a document (layers an edit
/// didn't touch stay shared between the versions, so this compares pointers, not contents).
pub(crate) fn same_layers(a: &Document, b: &Document, id: NodeId) -> bool {
    let mut stack = vec![id];
    let mut seen = 0;
    while let Some(i) = stack.pop() {
        seen += 1;
        if seen > MAX_COMPARED {
            return false;
        }
        match (a.node_arc(i), b.node_arc(i)) {
            (Some(x), Some(y)) if Arc::ptr_eq(x, y) => {}
            _ => return false,
        }
        if let Some(n) = b.node(i) {
            stack.extend(n.children.iter().copied());
        }
    }
    true
}

pub(crate) fn thumbnail(ctx: &egui::Context, doc: &Document, id: NodeId, px: f32) -> Option<egui::TextureHandle> {
    let pix = thumbnail_pixmap(doc, id, px)?;
    let img = crate::render_worker::screen_image(&pix, doc.color_profile);
    Some(ctx.load_texture(format!("asset-thumb-{}", id.0), img, egui::TextureOptions::LINEAR))
}

/// The pixels a thumbnail shows. A slice paints nothing itself, so it shows the content under it
/// that exporting it with its first export setting writes (`figcraft_engine::cmd::file::drawn_layers`):
/// without this a slice's preview is a blank checkerboard next to a correct export.
pub(crate) fn thumbnail_pixmap(doc: &Document, id: NodeId, px: f32) -> Option<figcraft_render::Pixmap> {
    let slice = doc.node(id).filter(|n| matches!(n.kind, NodeKind::Slice));
    let b = if slice.is_some() { doc.world_bounds(id)? } else { figcraft_render::export_bounds(doc, id)? };
    let side = b.width().max(b.height());
    if !(side.is_finite() && side > 0.0) {
        return None;
    }
    let scale = (f64::from(px) / side).min(2.0);
    if let Some(n) = slice {
        let setting = n.exports.first().cloned().unwrap_or_default();
        let layers = figcraft_engine::cmd::file::drawn_layers(doc, &[id], &setting);
        return figcraft_render::render_area(doc, &layers, b, scale);
    }
    figcraft_render::render_node(doc, id, scale)
}

fn matches(name: &str, q: &str) -> bool {
    q.is_empty() || name.to_lowercase().contains(q)
}

/// A fingerprint of what the session read for the enabled libraries.
fn libraries_key(app: &FigApp, doc: &Document) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in &doc.libraries.enabled {
        l.id.hash(&mut h);
        match app.session.libraries.cached(&l.path) {
            Some(Ok(lib)) => (Arc::as_ptr(lib) as usize).hash(&mut h),
            Some(Err(e)) => e.hash(&mut h),
            None => 0u8.hash(&mut h),
        }
    }
    h.finish()
}

fn build_listing(app: &FigApp, doc: &Document, q: &str, key: (u64, String, u64)) -> Listing {
    let local = library::local_components(doc)
        .into_iter()
        .filter_map(|id| {
            doc.node(id).filter(|n| matches(&n.name, q)).map(|n| Tile::new(doc, id, &n.name, matches!(n.kind, NodeKind::ComponentSet { .. })))
        })
        .collect();
    let local_styles = library::local_styles(doc).filter(|s| matches(&s.name, q)).map(StyleRow::of).collect();
    let mut updates = 0;
    let mut libraries = Vec::new();
    for l in &doc.libraries.enabled {
        let mut sec = Section { id: l.id.clone(), name: l.name.clone(), doc: None, error: None, tiles: Vec::new(), styles: Vec::new() };
        match app.session.libraries.cached(&l.path) {
            Some(Ok(lib)) if lib.manifest.id == l.id => {
                updates += library::pending(doc, lib).count();
                let m = &lib.manifest;
                sec.name = m.name.clone();
                sec.tiles = foldered(
                    &l.id,
                    m.components.iter().filter(|a| a.published && matches(&a.name, q)).map(|a| Tile::new(&lib.doc, a.id, &a.name, a.set)).collect(),
                );
                sec.styles =
                    lib.doc.styles.iter().filter(|s| m.style(&s.id).is_some_and(|x| x.published) && matches(&s.name, q)).map(StyleRow::of).collect();
                sec.doc = Some(lib.doc.clone());
            }
            Some(Ok(lib)) => sec.error = Some(trf("The file now holds another library (\"{name}\")", &[("name", &lib.manifest.name)])),
            Some(Err(e)) => sec.error = Some(e.to_string()),
            None => {}
        }
        // While searching, libraries without a match stay out of the way.
        if q.is_empty() || !sec.tiles.is_empty() || !sec.styles.is_empty() {
            libraries.push(sec);
        }
    }
    Listing { key, local: foldered("", local), local_styles, libraries, updates }
}

/// Re-read enabled library files that changed on disk (throttled).
fn refresh_libraries(app: &mut FigApp, ui: &Ui) {
    let Ok(doc) = app.session.doc() else { return };
    if doc.libraries.enabled.is_empty() {
        return;
    }
    let paths: Vec<String> = doc.libraries.enabled.iter().map(|l| l.path.clone()).collect();
    let id = Id::unique("asset_libraries_checked");
    let now = ui.input(|i| i.time);
    let last: Option<f64> = ui.data(|d| d.get_temp(id));
    if last.is_none_or(|l| now - l >= LIBRARY_REFRESH) {
        for p in &paths {
            let _ = app.session.libraries.load(p);
        }
        ui.data_mut(|d| d.insert_temp(id, now));
        // Team libraries: in the background, at once for one not fetched yet.
        let sid = Id::unique("asset_server_libraries_checked");
        let fetched: Option<f64> = ui.data(|d| d.get_temp(sid));
        // Only the ones you added or checked: a file can't make the app contact a server (#1097).
        let team: Vec<&String> = paths.iter().filter(|p| is_server_link(p) && app.session.library_allowed(p)).collect();
        let unread = team.iter().any(|p| app.session.libraries.cached(p).is_none());
        if !team.is_empty() && (unread || fetched.is_none_or(|l| now - l >= SERVER_REFRESH)) {
            app.session.refresh_server_libraries();
            ui.data_mut(|d| d.insert_temp(sid, now));
        }
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(LIBRARY_REFRESH));
}

/// The search field with the libraries button; returns the lowercase query.
fn search_row(app: &mut FigApp, ui: &mut Ui, w: f32) -> String {
    let t = Tokens::get(ui.ctx());
    let filter_id = Id::unique("asset_filter");
    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    let (fr, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let field = Rect::from_min_max(pos2(fr.min.x + 12.0, fr.min.y + 8.0), pos2(fr.max.x - 44.0, fr.max.y - 4.0));
    let r = crate::widgets::search_box(ui, field, Id::unique("asset_filter_field"), &mut filter, tr("Search assets"));
    if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        filter.clear();
    }
    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    let br = Rect::from_center_size(pos2(fr.max.x - 26.0, field.center().y), vec2(24.0, 24.0));
    let bresp = ui.interact(br, Id::unique("asset_libraries_btn"), Sense::click()).tip(tr("Libraries"));
    if bresp.hovered() {
        ui.painter().rect_filled(br, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), br.shrink(4.0), Icon::Library, t.icon);
    if bresp.clicked() {
        app.ui.libraries = true;
    }
    filter.trim().to_lowercase()
}

/// "Library updates available" with a button that pulls them in.
fn updates_banner(app: &mut FigApp, ui: &mut Ui, w: f32, count: usize) {
    let t = Tokens::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
    let r = Rect::from_min_max(pos2(row.min.x + 12.0, row.min.y + 4.0), pos2(row.max.x - 12.0, row.max.y - 4.0));
    ui.painter().rect_filled(r, CornerRadius::same(6), t.selected_row);
    let what = trn("{n} components and styles have newer versions", count, &[]);
    ui.interact(r, Id::unique("asset_updates"), Sense::hover()).on_hover_text(what);
    ui.painter().text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, tr("Library updates available"), semibold(11.0), t.text);
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(r.max.x - 70.0, r.min.y + 6.0), pos2(r.max.x - 6.0, r.max.y))));
    if crate::widgets::button(&mut child, tr("Update"), true, true).clicked()
        && !crate::menus::dev_mode_refuses(app, "library.acceptUpdates")
        && let Some(v) = app.run("library.acceptUpdates", json!({}))
    {
        let (c, s) = (v["components"].as_u64().unwrap_or(0), v["styles"].as_u64().unwrap_or(0));
        let (c, s) = (trn("{n} components", c as usize, &[]), trn("{n} styles", s as usize, &[]));
        app.toast(trf("Updated {components} and {styles}", &[("components", &c), ("styles", &s)]));
    }
}

/// A section title row.
fn title(ui: &mut Ui, w: f32, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (hr, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + PAD, hr.center().y), Align2::LEFT_CENTER, text, semibold(11.0), t.text);
}

/// A library's (or, with `folder`, a folder's) title row with its open/closed chevron; returns
/// whether it was clicked.
fn fold_title(ui: &mut Ui, w: f32, name: &str, open: bool, folder: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (hr, resp) = ui.allocate_exact_size(vec2(w, if folder { 26.0 } else { 32.0 }), Sense::click());
    let chevron = Rect::from_center_size(pos2(hr.min.x + PAD + 5.0, hr.center().y), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chevron, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
    let (font, col) = if folder { (regular(11.0), t.text_secondary) } else { (semibold(11.0), t.text) };
    crate::bidi::paint_name(ui.painter(), pos2(chevron.max.x + 6.0, hr.center().y), name, font, col, hr.max.x - chevron.max.x - 6.0 - PAD);
    let tip = match (folder, open) {
        (false, true) => tr("Library: click to collapse"),
        (false, false) => tr("Library: click to expand"),
        (true, true) => tr("Folder: click to collapse"),
        (true, false) => tr("Folder: click to expand"),
    };
    resp.on_hover_text(tip).clicked()
}

/// Folders of tiles under their headings; a folded folder shows only its heading (all open while
/// searching).
fn folders(
    ui: &mut Ui,
    w: f32,
    src: &Source,
    list: &[Folder],
    searching: bool,
    collapsed: &HashSet<String>,
    thumbs: &Thumbs,
    wanted: &mut Vec<(Source, NodeId)>,
    act: &mut Option<Act>,
) {
    for f in list {
        if !f.name.is_empty() {
            let open = searching || !collapsed.contains(&f.key);
            if fold_title(ui, w, &f.name, open, true) {
                *act = Some(Act::Toggle(f.key.clone()));
            }
            if !open {
                continue;
            }
        }
        tiles(ui, w, src, &f.tiles, thumbs, wanted, act);
    }
}

/// Secondary text, wrapped to the panel.
fn hint(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let galley = ui.painter().layout(text.to_string(), regular(11.0), t.text_secondary, (w - PAD * 2.0).max(40.0));
    let (r, _) = ui.allocate_exact_size(vec2(w, galley.size().y + 12.0), Sense::hover());
    ui.painter().galley(pos2(r.min.x + PAD, r.min.y + 2.0), galley, t.text_secondary);
}

/// Two-column thumbnail tiles; only the rows in view are laid out. The tiles in (and next to) the
/// view go to `wanted` for their thumbnails.
fn tiles(ui: &mut Ui, w: f32, src: &Source, list: &[Tile], thumbs: &Thumbs, wanted: &mut Vec<(Source, NodeId)>, act: &mut Option<Act>) {
    let t = Tokens::get(ui.ctx());
    let tile = ((w - PAD * 2.0 - TILE_GAP) / 2.0).max(40.0);
    let row_h = tile + TILE_LABEL + TILE_GAP;
    let rows = list.len().div_ceil(2);
    let (all, _) = ui.allocate_exact_size(vec2(w, rows as f32 * row_h), Sense::hover());
    let clip = ui.clip_rect();
    let first = (((clip.min.y - all.min.y) / row_h).floor().max(0.0) as usize).min(rows);
    let last = (((clip.max.y - all.min.y) / row_h).ceil().max(0.0) as usize).min(rows);
    // Thumbnails for one row more on each side, so scrolling finds them ready.
    let near = first.saturating_sub(1)..(last + 1).min(rows);
    wanted.extend(list.iter().skip(near.start * 2).take((near.end - near.start) * 2).map(|i| (src.clone(), i.id)));
    for row in first..last {
        let y = all.min.y + row as f32 * row_h;
        for (k, item) in list.iter().skip(row * 2).take(2).enumerate() {
            let x = all.min.x + PAD + k as f32 * (tile + TILE_GAP);
            let r = Rect::from_min_size(pos2(x, y), vec2(tile, tile));
            let resp = ui.interact(
                r.union(Rect::from_min_size(r.left_bottom(), vec2(tile, TILE_LABEL))),
                Id::unique(("asset", src, item.id.0)),
                Sense::click_and_drag(),
            );
            ui.painter().rect_filled(r, CornerRadius::same(6), t.field);
            if resp.hovered() {
                ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, COMPONENT), egui::StrokeKind::Inside);
            }
            match thumbs.map.get(&(src.clone(), item.id)).and_then(Option::as_ref) {
                Some(tex) => {
                    let size = tex.size_vec2();
                    // Fit the tile, never past 1:1 physical pixels (small components stay crisp).
                    let inner = r.shrink(14.0);
                    let fit = (inner.width() / size.x).min(inner.height() / size.y).min(1.0 / ui.ctx().pixels_per_point());
                    let img = Rect::from_center_size(r.center(), size * fit);
                    ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => icons::paint(
                    ui.painter(),
                    Rect::from_center_size(r.center(), vec2(20.0, 20.0)),
                    if item.set { Icon::ComponentSet } else { Icon::Component },
                    COMPONENT,
                ),
            }
            let label = Rect::from_min_size(pos2(x, r.max.y + 2.0), vec2(tile, TILE_LABEL - 4.0));
            // Hidden when publishing: a crossed eye on the tile and a dimmer name.
            let own_hidden = *src == Source::Local && item.hidden;
            if own_hidden {
                let badge = Rect::from_min_size(r.right_top() + vec2(-22.0, 6.0), vec2(16.0, 16.0));
                icons::paint(ui.painter(), badge, Icon::EyeOff, t.icon_secondary);
            }
            let name_colour = if own_hidden { t.text_secondary } else { t.text };
            crate::bidi::paint_name(ui.painter(), pos2(x, label.center().y), &item.label, regular(11.0), name_colour, tile);
            if *src == Source::Local {
                crate::right::context_popup(&resp).show(|ui| {
                    if crate::right::menu_check(ui, item.hidden, tr("Hide when publishing"), "") {
                        *act = Some(Act::Hide(item.id, !item.hidden));
                        ui.close();
                    }
                });
            }
            let resp = resp.on_hover_text(trf(
                "{name}\nDrag onto the canvas, or click to place in the middle of the view.\nHold Alt (Option) while dropping onto an instance to swap it.",
                &[("name", &item.name)],
            ));
            if resp.drag_started() {
                *act = Some(Act::Drag(AssetDrag { src: src.clone(), component: item.id, set: item.set }));
            }
            if resp.clicked() {
                *act = Some(Act::Place(src.clone(), item.id));
            }
        }
    }
}

/// Style rows (a swatch for paint styles); a click applies the style to the selection.
fn style_rows(ui: &mut Ui, w: f32, src: &Source, list: &[StyleRow], act: &mut Option<Act>) {
    let t = Tokens::get(ui.ctx());
    for s in list {
        let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.hover);
        }
        let icon_rect = Rect::from_center_size(pos2(r.min.x + 22.0, r.center().y), vec2(14.0, 14.0));
        let icon = match &s.kind {
            StyleKind::Paint { paints } => {
                if let Some(p) = paints.first() {
                    crate::widgets::paint_swatch(ui.painter(), icon_rect, p);
                }
                None
            }
            StyleKind::Text { .. } => Some(Icon::Text),
            StyleKind::Effect { .. } => Some(Icon::Effects),
            StyleKind::Grid { .. } => Some(Icon::Grid),
        };
        if let Some(i) = icon {
            icons::paint(ui.painter(), icon_rect, i, t.icon_secondary);
        }
        crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 40.0, r.center().y), &s.name, regular(11.0), t.text, w - 40.0 - PAD);
        if resp.on_hover_text(trf("{name}\nClick to apply to the selection", &[("name", &s.name)])).clicked() {
            *act = Some(Act::Style(src.clone(), s.id.clone()));
        }
    }
}

/// Drop the thumbnails whose components changed since they were rendered: this file's are
/// re-checked at most every [`THUMB_REFRESH`] seconds while it changes, a library's all go when its
/// file is read again.
fn invalidate_thumbs(ui: &Ui, thumbs: &mut Thumbs, current: &[(Source, Arc<Document>)]) {
    let now = ui.input(|i| i.time);
    for (src, doc) in current {
        let Some(old) = thumbs.docs.get(src).cloned() else {
            thumbs.docs.insert(src.clone(), doc.clone());
            continue;
        };
        if Arc::ptr_eq(&old, doc) {
            continue;
        }
        match src {
            Source::Local if now - thumbs.checked_at < THUMB_REFRESH => {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(THUMB_REFRESH));
                continue;
            }
            Source::Local => {
                thumbs.checked_at = now;
                thumbs.map.retain(|(s, id), _| *s != Source::Local || same_layers(&old, doc, *id));
            }
            Source::Library(_) => thumbs.map.retain(|(s, _), _| s != src),
        }
        thumbs.docs.insert(src.clone(), doc.clone());
    }
}

/// Render the missing thumbnails of `wanted` (a few per frame; the rest follow), and keep the
/// cache bounded, dropping thumbnails out of view first.
fn render_thumbs(ui: &Ui, w: f32, thumbs: &mut Thumbs, wanted: &[(Source, NodeId)]) {
    let px = ((w - PAD * 2.0 - TILE_GAP) / 2.0).max(32.0) * ui.ctx().pixels_per_point();
    let mut rendered = 0;
    for key in wanted {
        if thumbs.map.contains_key(key) {
            continue;
        }
        if rendered >= THUMBS_PER_FRAME {
            ui.ctx().request_repaint();
            break;
        }
        let Some(doc) = thumbs.docs.get(&key.0).cloned() else { continue };
        // A component set shows its default (top-left) variant.
        let shown = match doc.node(key.1).map(|n| &n.kind) {
            Some(NodeKind::ComponentSet { .. }) => figcraft_engine::components::default_variant(&doc, key.1).unwrap_or(key.1),
            _ => key.1,
        };
        thumbs.map.insert(key.clone(), thumbnail(ui.ctx(), &doc, shown, px));
        rendered += 1;
    }
    if thumbs.map.len() > MAX_THUMBS {
        let keep: HashSet<&(Source, NodeId)> = wanted.iter().collect();
        let extra: Vec<(Source, NodeId)> = thumbs.map.keys().filter(|k| !keep.contains(k)).take(thumbs.map.len() - MAX_THUMBS).cloned().collect();
        for k in extra {
            thumbs.map.remove(&k);
        }
    }
}

/// Draw the Assets panel: refresh libraries, switch to the libraries view when asked,
/// otherwise cache a listing keyed by revision, query and library state, render local and
/// enabled-library assets with reused thumbnails, and dispatch place, style, hide, toggle
/// and drag actions.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    refresh_libraries(app, ui);
    if app.ui.libraries {
        libraries_view(app, ui);
        return;
    }
    let w = ui.available_width();
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let revision = ds.revision;
    let q = search_row(app, ui, w);
    let key = (revision, q.clone(), libraries_key(app, &doc));
    let listing_id = Id::unique("asset_listing");
    let listing = match ui.data(|d| d.get_temp::<Arc<Listing>>(listing_id)) {
        Some(l) if l.key == key => l,
        _ => {
            let l = Arc::new(build_listing(app, &doc, &q, key));
            ui.data_mut(|d| d.insert_temp(listing_id, l.clone()));
            l
        }
    };
    if listing.updates > 0 {
        updates_banner(app, ui, w, listing.updates);
    }
    let mut thumbs: Thumbs = ui.data(|d| d.get_temp(Id::unique("asset_thumbs"))).unwrap_or_default();
    let mut sources: Vec<(Source, Arc<Document>)> = vec![(Source::Local, doc.clone())];
    sources.extend(listing.libraries.iter().filter_map(|l| l.doc.clone().map(|d| (Source::Library(l.id.clone()), d))));
    invalidate_thumbs(ui, &mut thumbs, &sources);
    let mut wanted = Vec::new();
    let collapsed_id = Id::unique("asset_collapsed_libraries");
    let collapsed: HashSet<String> = ui.data(|d| d.get_temp(collapsed_id)).unwrap_or_default();
    let mut act = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        title(ui, w, tr("Local components"));
        if listing.local.is_empty() {
            if q.is_empty() {
                let keys = crate::toolbar::pretty_shortcut(crate::shortcuts::shortcut("component.create").unwrap_or(""));
                hint(ui, &trf("No components yet. Select layers and press {keys}.", &[("keys", &keys)]));
            } else {
                hint(ui, tr("No matching components"));
            }
        }
        folders(ui, w, &Source::Local, &listing.local, !q.is_empty(), &collapsed, &thumbs, &mut wanted, &mut act);
        crate::widgets::separator(ui);
        ui.add_space(4.0);
        title(ui, w, tr("Local styles"));
        if listing.local_styles.is_empty() {
            hint(ui, if q.is_empty() { tr("No styles yet.") } else { tr("No matching styles") });
        }
        style_rows(ui, w, &Source::Local, &listing.local_styles, &mut act);
        for sec in &listing.libraries {
            crate::widgets::separator(ui);
            ui.add_space(4.0);
            let open = !q.is_empty() || !collapsed.contains(&sec.id);
            if fold_title(ui, w, &sec.name, open, false) {
                act = Some(Act::Toggle(sec.id.clone()));
            }
            if !open {
                continue;
            }
            if let Some(e) = &sec.error {
                hint(ui, &trf("Can't read this library: {e}", &[("e", e)]));
                continue;
            }
            if sec.doc.is_none() {
                hint(ui, tr("Reading the library…"));
                continue;
            }
            let src = Source::Library(sec.id.clone());
            folders(ui, w, &src, &sec.tiles, !q.is_empty(), &collapsed, &thumbs, &mut wanted, &mut act);
            style_rows(ui, w, &src, &sec.styles, &mut act);
        }
    });
    render_thumbs(ui, w, &mut thumbs, &wanted);
    let drag = ui.data(|d| d.get_temp::<AssetDrag>(drag_id()));
    let tex = drag.as_ref().and_then(|d| thumbs.map.get(&(d.src.clone(), d.component)).cloned().flatten());
    ui.data_mut(|d| d.insert_temp(Id::unique("asset_thumbs"), thumbs));
    match act {
        Some(Act::Drag(d)) => ui.data_mut(|m| {
            m.insert_temp(drag_id(), d);
        }),
        Some(Act::Place(src, id)) => {
            let (cx, cy) = app.session.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
            place(app, &src, id, cx, cy);
        }
        Some(Act::Style(src, style)) => apply_style(app, &src, &style),
        Some(Act::Hide(id, on)) => {
            if !crate::menus::dev_mode_refuses(app, "component.hideWhenPublishing") {
                app.run("component.hideWhenPublishing", json!({"ids": [id], "on": on}));
            }
        }
        Some(Act::Toggle(id)) => {
            let mut c = collapsed;
            if !c.remove(&id) {
                c.insert(id);
            }
            ui.data_mut(|d| d.insert_temp(collapsed_id, c));
        }
        None => {}
    }
    let libs: HashMap<String, Arc<Document>> = listing.libraries.iter().filter_map(|l| l.doc.clone().map(|d| (l.id.clone(), d))).collect();
    drag_feedback(app, ui, &doc, &libs, tex);
}

/// Place component `id` of `src` centred on page point (`x`, `y`).
fn place(app: &mut FigApp, src: &Source, id: NodeId, x: f64, y: f64) {
    let (cmd, params) = match src {
        Source::Local => ("instance.place", json!({"component": id, "x": x, "y": y})),
        Source::Library(lib) => ("library.place", json!({"library": lib, "component": id, "x": x, "y": y})),
    };
    if !crate::menus::dev_mode_refuses(app, cmd) {
        app.run(cmd, params);
    }
}

fn apply_style(app: &mut FigApp, src: &Source, style: &str) {
    let (cmd, params) = match src {
        Source::Local => ("style.apply", json!({"style": style})),
        Source::Library(lib) => ("library.applyStyle", json!({"library": lib, "style": style})),
    };
    if crate::menus::dev_mode_refuses(app, cmd) {
        return;
    }
    if app.session.selection().is_empty() {
        app.toast(tr("Select layers to apply the style to"));
        return;
    }
    app.run(cmd, params);
}

/// Swap the instance at page point (`x`, `y`) to component `id` of `src`.
fn swap_at(app: &mut FigApp, src: &Source, id: NodeId, x: f64, y: f64) {
    let (cmd, params) = match src {
        Source::Local => ("instance.swapAt", json!({"component": id, "x": x, "y": y})),
        Source::Library(lib) => ("library.swapAt", json!({"library": lib, "component": id, "x": x, "y": y})),
    };
    if !crate::menus::dev_mode_refuses(app, cmd) {
        app.run(cmd, params);
    }
}

/// The component set (by name) a dragged asset belongs to, in the document it comes from: the
/// instances an Alt-drop prefers to swap. A set's tile is its own family.
fn drag_family(doc: &Document, libs: &HashMap<String, Arc<Document>>, drag: &AssetDrag) -> Option<String> {
    let d = match &drag.src {
        Source::Local => doc,
        Source::Library(lib) => libs.get(lib)?.as_ref(),
    };
    let n = d.node(drag.component)?;
    if matches!(n.kind, NodeKind::ComponentSet { .. }) {
        return Some(n.name.clone());
    }
    figcraft_engine::drop::swap_family(d, drag.component)
}

/// While a component is dragged: a ghost at the pointer and, over the canvas, the frame or auto
/// layout slot it would drop into, or with Alt held the instance it would swap. Releasing over the
/// canvas places the instance (or swaps that one).
fn drag_feedback(app: &mut FigApp, ui: &mut Ui, doc: &Document, libs: &HashMap<String, Arc<Document>>, tex: Option<egui::TextureHandle>) {
    let Some(drag) = ui.data(|d| d.get_temp::<AssetDrag>(drag_id())) else { return };
    let (pointer, released, alt) = ui.input(|i| (i.pointer.hover_pos(), i.pointer.any_released(), i.modifiers.alt));
    let over_canvas = pointer.filter(|p| app.canvas.rect.contains(*p));
    let page = app.session.ds().map(|d| d.page).ok();
    let at = over_canvas.zip(page).map(|(p, page)| (app.to_page(p), page));
    // With Alt: the instance under the pointer (never one inside the dragged component itself).
    let swap = at.filter(|_| alt).and_then(|(pt, page)| {
        let own = matches!(drag.src, Source::Local).then_some(drag.component);
        figcraft_engine::drop::swap_target(doc, page, pt, drag_family(doc, libs, &drag).as_deref(), own)
    });
    let slot = at.filter(|_| swap.is_none()).map(|(pt, page)| (pt, figcraft_engine::drop::slot(doc, page, pt)));
    let ov = &mut app.session.tool.overlay;
    ov.swap_target = swap;
    ov.drop_target = slot.as_ref().and_then(|(_, s)| (Some(s.parent) != page).then_some(s.parent));
    ov.insert_line = slot.as_ref().and_then(|(_, s)| s.line);
    if let Some(p) = pointer {
        ghost(ui, p, drag.set, tex);
    }
    if released {
        ui.data_mut(|d| d.remove::<AssetDrag>(drag_id()));
        let ov = &mut app.session.tool.overlay;
        ov.drop_target = None;
        ov.insert_line = None;
        ov.swap_target = None;
        match (swap, at, slot) {
            (Some(_), Some((pt, _)), _) => swap_at(app, &drag.src, drag.component, pt.x, pt.y),
            (None, _, Some((pt, _))) => place(app, &drag.src, drag.component, pt.x, pt.y),
            _ => {}
        }
    }
    ui.ctx().request_repaint();
}

/// Draw the 64×64 component ghost on the tooltip layer, using the thumbnail when available
/// and the component or component-set icon otherwise.
fn ghost(ui: &Ui, p: Pos2, set: bool, tex: Option<egui::TextureHandle>) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::unique("asset_ghost")));
    let r = Rect::from_center_size(p, vec2(64.0, 64.0));
    painter.rect_filled(r, CornerRadius::same(6), t.panel.gamma_multiply(0.9));
    painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, COMPONENT), egui::StrokeKind::Inside);
    match tex {
        Some(tex) => {
            let size = tex.size_vec2();
            let fit = (56.0 / size.x).min(56.0 / size.y);
            painter.image(
                tex.id(),
                Rect::from_center_size(r.center(), size * fit),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(0.85),
            );
        }
        None => icons::paint(
            &painter,
            Rect::from_center_size(r.center(), vec2(20.0, 20.0)),
            if set { Icon::ComponentSet } else { Icon::Component },
            COMPONENT,
        ),
    }
}

/// Publish choices the user changed (`c<id>` / `s<id>` → offered) and the description.
#[derive(Clone, Default)]
struct PublishForm {
    picked: HashMap<String, bool>,
    description: Option<String>,
}

/// The libraries view: publish this file, and the library files this file uses.
fn libraries_view(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (hr, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let back = Rect::from_center_size(pos2(hr.min.x + 24.0, hr.center().y), vec2(24.0, 24.0));
    let bresp = ui.interact(back, Id::unique("asset_libraries_back"), Sense::click()).tip(tr("Back to assets"));
    if bresp.hovered() {
        ui.painter().rect_filled(back, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), back.shrink(5.0), Icon::ChevronLeft, t.icon);
    ui.painter().text(pos2(back.max.x + 6.0, hr.center().y), Align2::LEFT_CENTER, tr("Libraries"), semibold(12.0), t.text);
    if bresp.clicked() {
        app.ui.libraries = false;
        return;
    }
    crate::widgets::separator(ui);
    let Ok(doc) = app.session.doc().cloned() else { return };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        publish_form(app, ui, w, &doc);
        crate::widgets::separator(ui);
        enabled_libraries(app, ui, w, &doc);
    });
}

/// Draw the publish form: status, publishable components/styles/collections, description and
/// publish buttons. Run `library.publish` locally or publish to the server in the
/// background, reset the form on success, and report when the file is unsaved so other files
/// cannot yet use it.
fn publish_form(app: &mut FigApp, ui: &mut Ui, w: f32, doc: &Document) {
    let form_id = Id::unique("asset_publish_form");
    let mut form: PublishForm = ui.data(|d| d.get_temp(form_id)).unwrap_or_default();
    title(ui, w, tr("This file"));
    let m = doc.published.as_ref().map(|p| &p.manifest);
    // A file on a server publishes its library there (a live file doesn't keep a local one).
    let server = app.session.server_link();
    let on_server = server.as_deref().and_then(|l| match app.session.libraries.cached(l) {
        Some(Ok(lib)) => Some(lib.manifest.version),
        _ => None,
    });
    let status = match (&server, on_server, m) {
        (Some(_), Some(v), _) => trf("{name} · version {v} on the server", &[("name", &doc.name), ("v", &v)]),
        (Some(_), None, _) => trf("{name} · on a server", &[("name", &doc.name)]),
        (None, _, Some(m)) => trf("{name} · published version {v}", &[("name", &doc.name), ("v", &m.version)]),
        (None, _, None) => trf("{name} · not published", &[("name", &doc.name)]),
    };
    hint(ui, &status);
    let all = library::publishable(doc);
    if all.is_empty() {
        hint(ui, tr("Create components, styles or variables to publish them as a library."));
        return;
    }
    let def = library::default_selection(doc, m);
    let mut include: Vec<Value> = Vec::new();
    let mut check = |ui: &mut Ui, key: String, default: bool, label: &str| -> bool {
        let on = form.picked.get(&key).copied().unwrap_or(default);
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(PAD);
            clicked = crate::widgets::checkbox(ui, on, label);
        });
        let on = if clicked { !on } else { on };
        if clicked {
            form.picked.insert(key, on);
        }
        on
    };
    if !all.components.is_empty() {
        crate::widgets::sublabels(ui, tr("Components"), None);
        for c in &all.components {
            let name = doc.node(*c).map(|n| n.name.as_str()).unwrap_or_default();
            if check(ui, format!("c{}", c.0), def.components.contains(c), name) {
                include.push(json!(c.0));
            }
        }
    }
    if !all.styles.is_empty() {
        crate::widgets::sublabels(ui, tr("Styles"), None);
        for s in &all.styles {
            let name = doc.style(s).map(|x| x.name.as_str()).unwrap_or_default();
            if check(ui, format!("s{s}"), def.styles.contains(s), name) {
                include.push(json!(s));
            }
        }
    }
    if !all.collections.is_empty() {
        crate::widgets::sublabels(ui, tr("Variable collections"), None);
        for c in &all.collections {
            let name = doc.variables.collection_by_id(c).map(|x| x.name.as_str()).unwrap_or_default();
            if check(ui, format!("v{c}"), def.collections.contains(c), name) {
                include.push(json!(c));
            }
        }
    }
    ui.add_space(8.0);
    let description = form.description.clone().unwrap_or_else(|| m.map(|m| m.description.clone()).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        if let Some(d) =
            crate::widgets::text_field(ui, Id::unique("asset_publish_description"), &description, w - PAD * 2.0, tr("Description (optional)"))
        {
            form.description = Some(d);
        }
    });
    ui.add_space(8.0);
    let any = !include.is_empty();
    let (mut publish, mut to_server) = (false, false);
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        if server.is_some() && app.session.waiting_for(TO_SERVER) {
            crate::widgets::label(ui, tr("Publishing…"), Tokens::get(ui.ctx()).text_secondary);
            if crate::widgets::button(ui, tr("Cancel"), false, true).clicked() {
                app.cancel_background(TO_SERVER);
            }
        } else if server.is_some() {
            to_server = crate::widgets::button(ui, tr("Publish to server"), true, any).clicked();
        } else {
            publish = crate::widgets::button(ui, if m.is_some() { tr("Publish changes") } else { tr("Publish") }, true, any).clicked();
        }
    });
    ui.add_space(12.0);
    match &server {
        Some(link) => usage(app, ui, w, link),
        None if app.session.live.can_host() => {
            hint(
                ui,
                tr(
                    "To share this library with your team, publish the file to a server (Share ▸ Publish to server…), then publish the library there.",
                ),
            );
        }
        None => {}
    }
    let params = (publish || to_server).then(|| json!({"description": form.description.clone().unwrap_or(description), "include": include}));
    if publish
        && !crate::menus::dev_mode_refuses(app, "library.publish")
        && let Some(r) = params.clone().and_then(|p| app.run("library.publish", p))
    {
        form = PublishForm::default();
        // Other files read the saved file.
        app.save();
        let saved = app.session.ds().is_ok_and(|d| d.path.is_some() && !d.is_dirty());
        let v = r["version"].as_u64().unwrap_or(0);
        app.toast(if saved {
            trf("Published version {v}", &[("v", &v)])
        } else {
            trf("Published version {v}: save the file so other files can use it", &[("v", &v)])
        });
    }
    ui.data_mut(|d| d.insert_temp(form_id, form));
    // After the form is kept: a publish that finishes at once clears it.
    if to_server && let Some(params) = params {
        let done: crate::Done = Box::new(move |app, ctx, r| {
            ctx.data_mut(|d| d.remove::<PublishForm>(form_id));
            app.toast(trf("Published version {v} to the server", &[("v", &r["version"].as_u64().unwrap_or(0))]));
        });
        app.run_background(&ui.ctx().clone(), TO_SERVER, params, done);
    }
}

/// The choice of period Show usage counts over, `days` chosen now; the days of another one clicked.
fn usage_period(ui: &mut Ui, w: f32, days: u64) -> Option<u64> {
    let at = USAGE_PERIODS.iter().position(|p| p.0 == days).unwrap_or(0);
    let labels: Vec<&str> = USAGE_PERIODS.iter().map(|p| tr(p.1)).collect();
    let (r, _) = ui.allocate_exact_size(vec2(w, crate::widgets::FIELD_H + 8.0), Sense::hover());
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(PAD, 4.0))));
    crate::widgets::segmented_text(&mut row, &labels, at).filter(|i| *i != at).and_then(|i| USAGE_PERIODS.get(i)).map(|p| p.0)
}

/// How other files use this file's library on its server (asked for with a button: it is a
/// request to the server).
fn usage(app: &mut FigApp, ui: &mut Ui, w: f32, link: &str) {
    let usage_id = Id::unique(("asset_library_usage", link));
    let loading = app.session.waiting_for(USAGE);
    let (mut ask, mut stop) = (false, false);
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        if loading {
            stop = crate::widgets::button(ui, tr("Cancel"), false, true).clicked();
        } else {
            ask = crate::widgets::button(ui, tr("Show usage"), false, true).clicked();
        }
    });
    if loading {
        hint(ui, tr("Fetching from the server…"));
    }
    if stop {
        app.cancel_background(USAGE);
    }
    let shown = ui.data(|d| d.get_temp::<Arc<Value>>(usage_id));
    // The period insertions and detachments are counted over, as the last answer had it.
    let days = shown.as_ref().and_then(|v| v["days"].as_u64()).unwrap_or(USAGE_PERIODS[0].0);
    let chooses = !loading && shown.as_ref().is_some_and(|v| v["actionsKnown"].as_bool() == Some(true));
    let period = chooses.then(|| usage_period(ui, w, days)).flatten();
    if ask || period.is_some() {
        let done: crate::Done = Box::new(move |_, ctx, v| {
            ctx.data_mut(|d| d.insert_temp(usage_id, Arc::new(v)));
        });
        app.run_background(&ui.ctx().clone(), USAGE, json!({"days": period.unwrap_or(days)}), done);
    }
    let Some(v) = shown else {
        ui.add_space(8.0);
        return;
    };
    let files = v["files"].as_array().map_or(0, Vec::len);
    let instances = v["instances"].as_u64().unwrap_or(0);
    let files_n = |n: u64| trn("{n} files", n as usize, &[]);
    let instances_n = |n: u64| trn("{n} instances", n as usize, &[]);
    // Detached instances, beside what they are counted with (a server from before them keeps none).
    let known = v["detachesKnown"].as_bool().unwrap_or(false);
    let with_detached = |used: String, n: u64| {
        if known && n > 0 { trf("{used} · {detached}", &[("used", &used), ("detached", &trn("{n} detached", n as usize, &[]))]) } else { used }
    };
    let used = trf("Used in {files} · {instances}", &[("files", &files_n(files as u64)), ("instances", &instances_n(instances))]);
    hint(ui, &with_detached(used, v["detached"].as_u64().unwrap_or(0)));
    if !known {
        hint(ui, tr("This server doesn't count detached instances"));
    }
    let actions = v["actionsKnown"].as_bool().unwrap_or(false);
    let inserted_n = |n: u64| trn("{n} inserted", n as usize, &[]);
    if actions {
        let label = USAGE_PERIODS.iter().find(|p| p.0 == days).map_or(USAGE_PERIODS[0].2, |p| p.2);
        let (i, d) = (v["insertions"].as_u64().unwrap_or(0), v["detachments"].as_u64().unwrap_or(0));
        let counts = trf("{insertions} · {detachments}", &[("insertions", &inserted_n(i)), ("detachments", &trn("{n} detached", d as usize, &[]))]);
        hint(ui, &trf("{period}: {counts}", &[("period", &tr(label)), ("counts", &counts)]));
    } else {
        hint(ui, tr("This server doesn't count insertions"));
    }
    let t = Tokens::get(ui.ctx());
    for c in v["components"].as_array().into_iter().flatten().take(USAGE_ROWS) {
        let name = c["name"].as_str().unwrap_or(tr("(no longer published)"));
        let (n, f) = (c["instances"].as_u64().unwrap_or(0), c["files"].as_u64().unwrap_or(0));
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
        let gone = c["detached"].as_u64().unwrap_or(0);
        // A component only detached (no instances left anywhere) says just that.
        let right = if n == 0 && known && gone > 0 {
            trn("{n} detached", gone as usize, &[])
        } else {
            with_detached(trf("{instances} in {files}", &[("instances", &instances_n(n)), ("files", &files_n(f))]), gone)
        };
        // Inserted in the period, after what the component counts in all.
        let added = c["insertions"].as_u64().unwrap_or(0);
        let right = if actions && added > 0 { trf("{used} · {inserted}", &[("used", &right), ("inserted", &inserted_n(added))]) } else { right };
        // Two lines: the counts don't leave a name room on one.
        crate::bidi::paint_name(ui.painter(), pos2(r.min.x + PAD, r.min.y + 10.0), name, regular(11.0), t.text, w - PAD * 2.0);
        let counts = crate::widgets::elide_font(ui, &right, w - PAD * 2.0, regular(10.0));
        ui.painter().text(pos2(r.min.x + PAD, r.min.y + 25.0), Align2::LEFT_CENTER, counts, regular(10.0), t.text_secondary);
    }
    ui.add_space(8.0);
}

/// Draw the libraries enabled in this file with version/count/status, remove and swap
/// actions, add-file and pasted-link controls, and background fetch/cancel handling.
fn enabled_libraries(app: &mut FigApp, ui: &mut Ui, w: f32, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    title(ui, w, tr("Libraries in this file"));
    if doc.libraries.enabled.is_empty() {
        hint(ui, tr("No libraries yet. Add a .figcraft file that was published as a library, or paste a team library's link."));
    }
    let (mut remove, mut swap) = (None, None);
    let swappable = doc.libraries.enabled.len() > 1;
    for l in &doc.libraries.enabled {
        let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
        let status = match app.session.libraries.cached(&l.path) {
            Some(Ok(lib)) if lib.manifest.id == l.id => {
                let m = &lib.manifest;
                let n = m.components.iter().filter(|a| a.published).count();
                let s = m.styles.iter().filter(|a| a.published).count();
                let v = m.variables.iter().filter(|a| a.published).count();
                (
                    trf(
                        "Version {v} · {components} · {styles} · {variables}",
                        &[
                            ("v", &m.version),
                            ("components", &trn("{n} components", n, &[])),
                            ("styles", &trn("{n} styles", s, &[])),
                            ("variables", &trn("{n} variables", v, &[])),
                        ],
                    ),
                    t.text_secondary,
                )
            }
            Some(Ok(_)) => (tr("The file now holds another library").into(), t.danger),
            Some(Err(e)) => (e.to_string(), t.danger),
            None if is_server_link(&l.path) && app.session.library_allowed(&l.path) => (tr("Fetching from the server…").into(), t.text_secondary),
            None if is_server_link(&l.path) => (tr("Not checked yet").into(), t.text_secondary),
            None => (tr("Not read yet").into(), t.text_secondary),
        };
        let text_w = w - PAD - if swappable { 72.0 } else { 44.0 };
        let name = ui.painter().layout_no_wrap(l.name.clone(), semibold(11.0), t.text);
        let line = ui.painter().layout(status.0, regular(10.0), status.1, text_w);
        let clip = Rect::from_min_size(pos2(r.min.x + PAD, r.min.y), vec2(text_w, r.height()));
        ui.painter().with_clip_rect(clip).galley(pos2(r.min.x + PAD, r.min.y + 6.0), name, t.text);
        ui.painter().with_clip_rect(clip).galley(pos2(r.min.x + PAD, r.min.y + 23.0), line, t.text_secondary);
        ui.interact(clip, Id::unique(("asset_lib_row", &l.id)), Sense::hover()).on_hover_text(shown_path(&l.path));
        let x = Rect::from_center_size(pos2(r.max.x - 24.0, r.center().y), vec2(24.0, 24.0));
        let xr = ui.interact(x, Id::unique(("asset_lib_remove", &l.id)), Sense::click()).tip(tr("Remove library from this file"));
        if xr.hovered() {
            ui.painter().rect_filled(x, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), x.shrink(6.0), Icon::Close, t.icon);
        if xr.clicked() {
            remove = Some(l.id.clone());
        }
        if swappable {
            let sw = Rect::from_center_size(pos2(r.max.x - 52.0, r.center().y), vec2(24.0, 24.0));
            let sr = ui
                .interact(sw, Id::unique(("asset_lib_swap", &l.id)), Sense::click())
                .tip(tr("Swap library: use another library's assets of the same names"));
            if sr.hovered() {
                ui.painter().rect_filled(sw, CornerRadius::same(5), t.hover);
            }
            icons::paint(ui.painter(), sw.shrink(6.0), Icon::Swap, t.icon);
            crate::right::menu_popup(&sr).show(|ui| {
                for other in doc.libraries.enabled.iter().filter(|o| o.id != l.id) {
                    if crate::right::menu_item(ui, &trf("Swap to {name}", &[("name", &other.name)]), "") {
                        swap = Some((l.id.clone(), other.id.clone()));
                        ui.close();
                    }
                }
            });
        }
    }
    // A file names its team libraries' servers: they are contacted only once you say so (#1097).
    let unchecked = doc.libraries.enabled.iter().any(|l| is_server_link(&l.path) && !app.session.library_allowed(&l.path));
    if unchecked {
        hint(ui, tr("This file uses team libraries on servers you haven't used here. They aren't contacted until you check them."));
        ui.horizontal(|ui| {
            ui.add_space(PAD);
            if crate::widgets::button(ui, tr("Check for updates"), false, true).clicked() && !crate::menus::dev_mode_refuses(app, CHECK) {
                app.run(CHECK, json!({}));
            }
        });
    }
    ui.add_space(4.0);
    let mut add = false;
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        add = crate::widgets::button(ui, tr("Add library file…"), false, true).clicked();
    });
    ui.add_space(4.0);
    let pasted = ui
        .horizontal(|ui| {
            ui.add_space(PAD);
            crate::widgets::text_field(ui, Id::unique("asset_add_link"), "", w - PAD * 2.0, tr("Paste a team library link (a view link)"))
        })
        .inner;
    if let Some(link) = pasted.map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
        && !crate::menus::dev_mode_refuses(app, ADD)
    {
        app.run_background(&ui.ctx().clone(), ADD, json!({"path": link}), Box::new(|app, _, r| added(app, &r)));
    }
    if app.session.waiting_for(ADD) {
        hint(ui, tr("Fetching from the server…"));
        ui.horizontal(|ui| {
            ui.add_space(PAD);
            if crate::widgets::button(ui, tr("Cancel"), false, true).clicked() {
                app.cancel_background(ADD);
            }
        });
    }
    if let Some(id) = remove
        && !crate::menus::dev_mode_refuses(app, "library.remove")
    {
        app.run("library.remove", json!({"id": id}));
    }
    if let Some((from, to)) = swap
        && !crate::menus::dev_mode_refuses(app, "library.swap")
        && let Some(r) = app.run("library.swap", json!({"from": from, "to": to}))
    {
        let moved = ["components", "styles", "variables"].iter().map(|k| r[*k].as_u64().unwrap_or(0)).sum::<u64>();
        let missed = r["unmatched"].as_array().map_or(0, Vec::len);
        app.toast(if missed == 0 {
            trn("Swapped {n} assets", moved as usize, &[])
        } else {
            trn("Swapped {n} assets; {missed} had no match and still use the old library", moved as usize, &[("missed", &missed)])
        });
    }
    if add && !crate::menus::dev_mode_refuses(app, ADD) {
        app.pick_open(tr("Add library"), &["figcraft"], |app, p| {
            if let Some(r) = app.run(ADD, json!({"path": p.to_string_lossy()})) {
                added(app, &r);
            }
        });
    }
}

/// "Added the library …" after `library.add` answered `r`.
fn added(app: &mut FigApp, r: &Value) {
    app.toast(trf("Added the library \"{name}\"", &[("name", &r["name"].as_str().unwrap_or_default())]));
}

/// Tests: the components whose thumbnails are cached (for this file's assets).
#[cfg(test)]
pub(crate) fn cached_thumbnails(ctx: &egui::Context) -> Vec<NodeId> {
    let thumbs: Thumbs = ctx.data(|d| d.get_temp(Id::unique("asset_thumbs"))).unwrap_or_default();
    let mut out: Vec<NodeId> = thumbs.map.keys().filter(|(s, _)| *s == Source::Local).map(|(_, id)| *id).collect();
    out.sort();
    out
}
