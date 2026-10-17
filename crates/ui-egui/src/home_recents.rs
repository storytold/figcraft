//! The home screen's recent files: a grid of cards, each with the picture its file carries on the
//! colour of its page, or a table (name, last modified, created). Sorted by name, creation or last
//! change either way round; a click selects a file, a double-click or Enter opens it, the arrow
//! keys move the selection, and a right-click offers Open, Show in folder and Remove from recents.
//!
//! What the files on disk say (whether they exist, their dates, their pictures) is read off the
//! frame's thread: only the cards in view read their pictures, each cached by path and modification
//! time, and a frame never waits for a file.

use std::collections::HashMap;
use std::path::Path;

use egui::{Align2, Color32, CornerRadius, Id, Key, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::Prefs;
use figcraft_format::thumbnail::{FileKind, Preview};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets;

/// The gap between cards, across and down.
pub const GAP: f32 = 32.0;
/// The narrowest and widest a card gets (columns come and go to keep cards within these).
pub const CARD_MIN: f32 = 260.0;
pub const CARD_MAX: f32 = 360.0;
/// The footer under a card's picture: its icon, name and time.
const FOOTER_H: f32 = 58.0;
/// A table row, and the table's header.
const ROW_H: f32 = 44.0;
const HEADER_H: f32 = 32.0;
/// Pictures are decoded at most this many pixels on their longer side (the widest card on a
/// double-density screen).
const PICTURE_PX: u32 = 720;
/// How long a picture takes to fade in (seconds).
const FADE_SECS: f64 = 0.2;
/// Most recent files shown.
pub const SHOWN: usize = 12;
/// The section whose view this is (`Prefs::home_views`).
const SECTION: &str = "recents";

/// Cards or a table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Grid,
    List,
}

impl View {
    /// The id `ui.set {homeRecentsView}`, `ui.inspect` and the preferences use.
    pub fn id(self) -> &'static str {
        match self {
            View::Grid => "grid",
            View::List => "list",
        }
    }

    pub fn parse(id: &str) -> Option<View> {
        [View::Grid, View::List].into_iter().find(|v| v.id() == id)
    }

    /// The view the preferences keep for the recent files.
    pub fn of(prefs: &Prefs) -> View {
        prefs.home_views.get(SECTION).and_then(|v| View::parse(v)).unwrap_or_default()
    }

    pub fn store(self, prefs: &mut Prefs) {
        prefs.home_views.insert(SECTION.into(), self.id().into());
    }
}

/// What the files are ordered by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Created,
    Modified,
}

impl SortKey {
    /// The sort menu, top to bottom.
    pub const ALL: [SortKey; 3] = [SortKey::Name, SortKey::Created, SortKey::Modified];

    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Created => "created",
            SortKey::Modified => "modified",
        }
    }

    pub fn parse(id: &str) -> Option<SortKey> {
        SortKey::ALL.into_iter().find(|k| k.id() == id)
    }

    fn label(self) -> &'static str {
        match self {
            SortKey::Name => tr_noop("Alphabetical"),
            SortKey::Created => tr_noop("Date created"),
            SortKey::Modified => tr_noop("Last modified"),
        }
    }
}

/// The order of the recent files: by what, and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    /// Newest first (Z to A by name).
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort::by(SortKey::Modified)
    }
}

impl Sort {
    /// `key` its usual way round: names A to Z, dates newest first.
    pub fn by(key: SortKey) -> Sort {
        Sort { key, descending: key != SortKey::Name }
    }

    /// The order the preferences keep (the default for anything they don't know).
    pub fn of(prefs: &Prefs) -> Sort {
        match SortKey::parse(&prefs.home_sort) {
            Some(key) => Sort { key, descending: prefs.home_order != "asc" },
            None => Sort::default(),
        }
    }

    pub fn store(self, prefs: &mut Prefs) {
        prefs.home_sort = self.key.id().into();
        prefs.home_order = if self.descending { "desc" } else { "asc" }.into();
    }

    /// A header click on `key`: the other way round when it is the order already, else `key`'s
    /// usual way.
    pub fn clicked(self, key: SortKey) -> Sort {
        if self.key == key { Sort { key, descending: !self.descending } } else { Sort::by(key) }
    }
}

/// Columns for a grid `width` wide, and the cards' width: as many columns as keep each card
/// between [`CARD_MIN`] and [`CARD_MAX`], at least one (narrower than [`CARD_MIN`] when the width
/// is).
pub fn grid_columns(width: f32) -> (usize, f32) {
    let w = if width.is_finite() { width.max(1.0) } else { CARD_MIN };
    let card = |n: usize| (w - GAP * (n as f32 - 1.0)) / n as f32;
    let mut n = ((w + GAP) / (CARD_MAX + GAP)).ceil().clamp(1.0, 64.0) as usize;
    while n > 1 && card(n) < CARD_MIN {
        n -= 1;
    }
    (n, card(n).min(CARD_MAX))
}

/// A card `width` wide: its picture's height (16:9) and its whole height.
pub fn card_heights(width: f32) -> (f32, f32) {
    let picture = (width * 9.0 / 16.0).round();
    (picture, picture + FOOTER_H)
}

/// How long ago `then` was at `now` (seconds since 1970): "just now", "5 minutes ago",
/// "3 hours ago", "yesterday", "4 days ago", then the date ("Mar 4, 2026", in UTC).
pub fn ago(now: u64, then: u64) -> String {
    let d = now.saturating_sub(then);
    let n = |div: u64| usize::try_from(d / div).unwrap_or(usize::MAX);
    match d {
        0..60 => tr("just now").into(),
        60..3600 => trn("{n} minutes ago", n(60), &[]),
        3600..86_400 => trn("{n} hours ago", n(3600), &[]),
        86_400..172_800 => tr("yesterday").into(),
        172_800..604_800 => trn("{n} days ago", n(86_400), &[]),
        _ => date(then),
    }
}

/// "Edited …" for a file last changed at `then`.
pub fn edited(now: u64, then: u64) -> String {
    trf("Edited {when}", &[("when", &ago(now, then))])
}

const MONTHS: [&str; 12] = [
    tr_noop("Jan"),
    tr_noop("Feb"),
    tr_noop("Mar"),
    tr_noop("Apr"),
    tr_noop("May"),
    tr_noop("Jun"),
    tr_noop("Jul"),
    tr_noop("Aug"),
    tr_noop("Sep"),
    tr_noop("Oct"),
    tr_noop("Nov"),
    tr_noop("Dec"),
];

/// `secs` since 1970 as a date in UTC: "Mar 4, 2026".
fn date(secs: u64) -> String {
    // Days to a civil date (Howard Hinnant's days_from_civil, inverted).
    let z = i64::try_from(secs / 86_400).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let name = usize::try_from(month - 1).ok().and_then(|m| MONTHS.get(m)).map_or("", |m| tr(m));
    trf("{month} {day}, {year}", &[("month", &name), ("day", &day), ("year", &year)])
}

/// What a file on disk says about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub exists: bool,
    /// Seconds since 1970.
    pub modified: Option<u64>,
    /// Where the platform keeps it.
    pub created: Option<u64>,
}

/// A recent file as the screen shows it.
#[derive(Clone, Debug)]
pub struct Shown {
    pub path: String,
    /// The file name without its folder.
    pub name: String,
    /// `None` until the file has been looked at.
    pub stat: Option<Stat>,
}

impl Shown {
    fn new(path: &str, stat: Option<Stat>) -> Shown {
        // Shown without its extension, like a document's name; the full path is the tooltip.
        let p = Path::new(path);
        let name = p.file_stem().or_else(|| p.file_name()).map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned());
        Shown { path: path.to_string(), name, stat }
    }

    fn date(&self, key: SortKey) -> Option<u64> {
        let s = self.stat?;
        match key {
            SortKey::Modified => s.modified,
            SortKey::Created => s.created,
            SortKey::Name => None,
        }
    }

    /// The line under the name: "Edited 3 hours ago", "File not found", nothing yet.
    fn detail(&self, now: u64) -> String {
        match self.stat {
            Some(Stat { exists: false, .. }) => tr("File not found").into(),
            Some(Stat { modified: Some(m), .. }) => edited(now, m),
            _ => String::new(),
        }
    }
}

/// `files` in `sort` order. Files without the date sorted by come last either way round, in the
/// order they had; names compare without regard to case.
pub fn sorted(mut files: Vec<Shown>, sort: Sort) -> Vec<Shown> {
    match sort.key {
        SortKey::Name => files.sort_by(|a, b| {
            let o = a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.path.cmp(&b.path));
            if sort.descending { o.reverse() } else { o }
        }),
        key => files.sort_by(|a, b| match (a.date(key), b.date(key)) {
            (Some(x), Some(y)) => {
                if sort.descending {
                    y.cmp(&x)
                } else {
                    x.cmp(&y)
                }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }),
    }
    files
}

/// A file's picture as the screen has it.
struct Ready {
    /// The modification time of the file it was read from.
    modified: Option<u64>,
    page: Option<Color32>,
    texture: Option<egui::TextureHandle>,
    /// When it came in (for its fade).
    arrived: f64,
}

#[derive(Default)]
struct Known {
    stat: Option<Stat>,
    /// Whether the dates were asked for since the recent files came into view, and whether that is
    /// still under way.
    asked: bool,
    pending: bool,
    /// The picture shown (an older one stays until a changed file's new one is in).
    shown: Option<Ready>,
    /// The picture being read, by the file's modification time.
    wanted: Option<Option<u64>>,
}

/// The recent files' state: what is known about each file, the reader, the selection.
#[derive(Default)]
pub struct Recents {
    files: HashMap<String, Known>,
    loader: Option<Loader>,
    /// The selected file (a click, the arrow keys, a right-click).
    pub selected: Option<String>,
    /// The pass the recent files were last drawn in: coming back to them reads the dates again.
    drawn: u64,
    /// When the dates were last read again while in view, and whether the window had the focus:
    /// files deleted, moved or saved elsewhere show up within [`RECHECK_SECS`], or at once when
    /// the window gets the focus back.
    checked: f64,
    focused: bool,
}

/// How often the dates of the recent files in view are read again (seconds).
const RECHECK_SECS: f64 = 2.0;

enum Job {
    Stat(String),
    Preview(String, Option<u64>),
}

enum Reply {
    Stat(String, Stat),
    Preview(String, Option<u64>, Preview),
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs())
}

fn run(job: Job) -> Reply {
    match job {
        Job::Stat(path) => {
            let meta = std::fs::metadata(&path).ok().filter(std::fs::Metadata::is_file);
            let stat = Stat {
                exists: meta.is_some(),
                modified: meta.as_ref().and_then(|m| secs(m.modified())),
                created: meta.as_ref().and_then(|m| secs(m.created())),
            };
            Reply::Stat(path, stat)
        }
        Job::Preview(path, modified) => {
            let preview = figcraft_format::thumbnail::file_preview(Path::new(&path), PICTURE_PX);
            Reply::Preview(path, modified, preview)
        }
    }
}

/// Reads files on a thread of its own (on the web, where there are no files to read, in place).
struct Loader {
    #[cfg(not(target_arch = "wasm32"))]
    thread: Option<(std::sync::mpsc::Sender<Job>, std::sync::mpsc::Receiver<Reply>)>,
    /// Replies read in place.
    done: Vec<Reply>,
}

impl Loader {
    fn new(ctx: &egui::Context) -> Loader {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, jobs) = std::sync::mpsc::channel::<Job>();
            let (done_tx, rx) = std::sync::mpsc::channel::<Reply>();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("figcraft-recents".into()).spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job)));
                    if let Ok(reply) = reply
                        && done_tx.send(reply).is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
            let thread = match spawned {
                Ok(_) => Some((tx, rx)),
                Err(e) => {
                    log::warn!("recent files: no reader thread ({e}); reading in place");
                    None
                }
            };
            Loader { thread, done: Vec::new() }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            Loader { done: Vec::new() }
        }
    }

    fn submit(&mut self, job: Job) {
        #[cfg(not(target_arch = "wasm32"))]
        let job = match &self.thread {
            Some((tx, _)) => match tx.send(job) {
                Ok(()) => return,
                Err(std::sync::mpsc::SendError(job)) => job,
            },
            None => job,
        };
        self.done.push(run(job));
    }

    fn poll(&mut self) -> Vec<Reply> {
        let out = std::mem::take(&mut self.done);
        #[cfg(not(target_arch = "wasm32"))]
        let out = match &self.thread {
            Some((_, rx)) => out.into_iter().chain(rx.try_iter()).collect(),
            None => out,
        };
        out
    }
}

impl Recents {
    /// Take in what the reader found, forget files no longer listed, and ask for the dates of
    /// files not looked at since the recent files came into view. `time`: the input time.
    fn refresh(&mut self, ctx: &egui::Context, paths: &[String], time: f64) {
        let pass = ctx.cumulative_pass_nr();
        let (focused, refocused) = ctx.input(|i| (i.focused, i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))));
        let back = pass > self.drawn.saturating_add(1) || refocused || (focused && !self.focused);
        if back || time - self.checked >= RECHECK_SECS {
            self.files.values_mut().for_each(|k| k.asked = false);
            self.checked = time;
        }
        self.drawn = pass;
        self.focused = focused;
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(RECHECK_SECS));
        let loader = self.loader.get_or_insert_with(|| Loader::new(ctx));
        for reply in loader.poll() {
            match reply {
                Reply::Stat(path, stat) => {
                    if let Some(k) = self.files.get_mut(&path) {
                        k.stat = Some(stat);
                        k.pending = false;
                    }
                }
                Reply::Preview(path, modified, preview) => {
                    if let Some(k) = self.files.get_mut(&path)
                        && k.wanted == Some(modified)
                    {
                        let texture = preview.picture.map(|p| {
                            let size = [p.width as usize, p.height as usize];
                            let image = egui::ColorImage::from_rgba_unmultiplied(size, &p.rgba);
                            ctx.load_texture(format!("recent-file:{path}"), image, egui::TextureOptions::LINEAR)
                        });
                        let page = preview.page.map(|[r, g, b, a]| crate::theme::c32(figcraft_color::Color { r, g, b, a }));
                        // A changed file's new picture shows at once; a first one fades in.
                        let arrived = if k.shown.is_some() { f64::NEG_INFINITY } else { time };
                        k.shown = Some(Ready { modified, page, texture, arrived });
                        k.wanted = None;
                    }
                }
            }
        }
        self.files.retain(|p, _| paths.contains(p));
        if self.selected.as_ref().is_some_and(|s| !paths.contains(s)) {
            self.selected = None;
        }
        for p in paths {
            let k = self.files.entry(p.clone()).or_default();
            if !k.pending && !k.asked {
                k.pending = true;
                k.asked = true;
                loader.submit(Job::Stat(p.clone()));
            }
        }
    }

    /// The picture for `path` (asking for it when the card is in view and the file has changed
    /// since): its page colour, its texture and its opacity while it fades in. `None` while there is
    /// nothing to show yet, and for a file that isn't there.
    fn picture(&mut self, path: &str, visible: bool, time: f64) -> Option<(Option<Color32>, Option<egui::TextureHandle>, f32)> {
        let k = self.files.get_mut(path)?;
        let stat = k.stat.filter(|s| s.exists)?;
        let current = k.shown.as_ref().is_some_and(|r| r.modified == stat.modified) || k.wanted == Some(stat.modified);
        if visible
            && !current
            && let Some(loader) = self.loader.as_mut()
        {
            loader.submit(Job::Preview(path.to_string(), stat.modified));
            k.wanted = Some(stat.modified);
        }
        let r = k.shown.as_ref()?;
        Some((r.page, r.texture.clone(), (((time - r.arrived) / FADE_SECS) as f32).clamp(0.0, 1.0)))
    }

    /// The files as listed, with what is known about them.
    fn shown(&self, paths: &[String]) -> Vec<Shown> {
        paths.iter().map(|p| Shown::new(p, self.files.get(p).and_then(|k| k.stat))).collect()
    }

    /// For `ui.inspect`: each file listed, whether it is there and whether its picture is in.
    pub fn inspect(&self, prefs: &Prefs) -> Value {
        let paths: Vec<String> = listed(&prefs.recent_files).take(SHOWN).cloned().collect();
        let files: Vec<Value> = sorted(self.shown(&paths), Sort::of(prefs))
            .iter()
            .map(|f| {
                let k = self.files.get(&f.path);
                let picture = match k.map(|k| (k.shown.as_ref(), k.wanted.is_some())) {
                    Some((Some(Ready { texture: Some(_), .. }), _)) => "picture",
                    Some((Some(Ready { page: Some(_), .. }), _)) => "page",
                    Some((Some(_), _)) => "placeholder",
                    Some((None, true)) => "loading",
                    _ => "none",
                };
                json!({"path": f.path, "name": f.name, "exists": f.stat.map(|s| s.exists), "picture": picture, "selected": self.selected.as_deref() == Some(f.path.as_str())})
            })
            .collect();
        json!(files)
    }
}

/// The recent files that may be listed: all of them, or, when the app runs confined to automation
/// roots, those inside them (no name or picture of another file is shown).
pub(crate) fn listed(recents: &[String]) -> impl Iterator<Item = &String> {
    recents.iter().filter(|p| figcraft_engine::file_access::allowed(p, figcraft_engine::file_access::Access::Read))
}

/// The recent files for `ui.inspect`: `{view, sort, order, files: [{path, name, exists, picture,
/// selected}]}` in the order shown (`picture`: `none`, `loading`, `picture`, `page` or
/// `placeholder`).
pub fn inspect(app: &FigApp) -> Value {
    let prefs = &app.session.prefs;
    let sort = Sort::of(prefs);
    json!({
        "view": View::of(prefs).id(),
        "sort": sort.key.id(),
        "order": if sort.descending { "desc" } else { "asc" },
        "files": app.ui.home_view.recents.inspect(prefs),
    })
}

/// What the user asked for on the recent files this frame.
enum Action {
    Open(String),
    Reveal(String),
    Remove(String),
    Sort(Sort),
    View(View),
}

/// The Recent files heading, its toolbar and the files as cards or a table.
pub fn section(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let paths: Vec<String> = listed(&app.session.prefs.recent_files).take(SHOWN).cloned().collect();
    let time = ui.input(|i| i.time);
    let now = figcraft_engine::cmd::comment::now_secs();
    let reduce_motion = app.reduce_motion();
    let (view, sort) = (View::of(&app.session.prefs), Sort::of(&app.session.prefs));
    let state = &mut app.ui.home_view.recents;
    state.refresh(ui.ctx(), &paths, time);
    let files = sorted(state.shown(&paths), sort);
    let with_created = files.iter().any(|f| f.stat.is_some_and(|s| s.created.is_some()));
    let mut action = toolbar(ui, t, view, sort, with_created);
    ui.add_space(16.0);
    let chosen = match view {
        View::Grid => grid(ui, t, state, &files, now, time, reduce_motion),
        View::List => list(ui, t, state, &files, now, sort, with_created),
    };
    action = action.or(chosen);
    match action {
        Some(Action::Open(path)) => {
            if app.run_user("file.open", json!({"path": path})).is_some() {
                app.ui.home = false;
            }
        }
        Some(Action::Reveal(path)) => app.services.reveal_path(Path::new(&path)),
        Some(Action::Remove(path)) => {
            app.run("file.openRecent", json!({"remove": path}));
            app.services.save_prefs(&app.session.prefs);
        }
        Some(Action::Sort(sort)) => {
            sort.store(&mut app.session.prefs);
            app.services.save_prefs(&app.session.prefs);
        }
        Some(Action::View(view)) => {
            view.store(&mut app.session.prefs);
            app.services.save_prefs(&app.session.prefs);
        }
        None => {}
    }
}

/// "Recent files", and at the right the sort menu and the grid and list buttons.
fn toolbar(ui: &mut Ui, t: &Tokens, view: View, sort: Sort, with_created: bool) -> Option<Action> {
    let mut action = None;
    ui.add_space(28.0);
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, widgets::FIELD_H), Sense::hover());
    ui.painter().text(pos2(row.min.x, row.center().y), Align2::LEFT_CENTER, tr("Recent files"), semibold(12.0), t.text_secondary);
    let toggle = Rect::from_min_size(pos2(row.max.x - 56.0, row.min.y), vec2(56.0, widgets::FIELD_H));
    ui.scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
        let active = Some(if view == View::Grid { 0 } else { 1 });
        match widgets::segmented(ui, &[(Icon::ViewGrid, tr("Show as grid")), (Icon::ViewList, tr("Show as list"))], active, 28.0) {
            Some(0) if view != View::Grid => action = Some(Action::View(View::Grid)),
            Some(1) if view != View::List => action = Some(Action::View(View::List)),
            _ => {}
        }
    });
    let label = tr(sort.key.label());
    let w = (ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.text).size().x + 40.0).max(110.0);
    let menu = Rect::from_min_size(pos2(toggle.min.x - 8.0 - w, row.min.y), vec2(w, widgets::FIELD_H));
    ui.scope_builder(egui::UiBuilder::new().max_rect(menu), |ui| {
        let resp = widgets::dropdown(ui, label, w, None);
        crate::right::menu_popup(&resp).show(|ui| {
            heading(ui, t, tr("Sort by"));
            for key in SortKey::ALL.into_iter().filter(|k| *k != SortKey::Created || with_created) {
                if crate::right::menu_check(ui, sort.key == key, tr(key.label()), "") {
                    action = Some(Action::Sort(Sort { key, ..Sort::by(key) }));
                    ui.close();
                }
            }
            ui.separator();
            heading(ui, t, tr("Order"));
            let (first, second) = if sort.key == SortKey::Name { (tr("A to Z"), tr("Z to A")) } else { (tr("Oldest first"), tr("Newest first")) };
            for (descending, label) in [(false, first), (true, second)] {
                if crate::right::menu_check(ui, sort.descending == descending, label, "") {
                    action = Some(Action::Sort(Sort { descending, ..sort }));
                    ui.close();
                }
            }
        });
    });
    action
}

/// A group's heading in a menu.
fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, text, regular(10.5), t.text_secondary);
}

/// The id of `path`'s card or row (stable, so the keyboard stays on it as the order changes).
fn card_id(path: &str) -> Id {
    Id::new(("home_recent_file", path))
}

/// Clicks, keys and the context menu on a card or row: a click selects, a double-click or Enter
/// opens, a right-click selects and offers the menu.
fn interact(ui: &Ui, resp: &Response, recents: &mut Recents, file: &Shown) -> Option<Action> {
    let mut action = None;
    if resp.gained_focus() {
        recents.selected = Some(file.path.clone());
    }
    // The arrow keys move the selection here, not egui's focus.
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
        });
    }
    let enter = resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    // A click soon after another counts as a triple: it opens too.
    if resp.double_clicked() || resp.triple_clicked() || enter {
        action = Some(Action::Open(file.path.clone()));
    } else if resp.clicked() || resp.secondary_clicked() {
        recents.selected = Some(file.path.clone());
        resp.request_focus();
    }
    crate::right::context_popup(resp).show(|ui| {
        if crate::right::menu_item(ui, tr("Open"), "") {
            action = Some(Action::Open(file.path.clone()));
            ui.close();
        }
        if cfg!(not(target_arch = "wasm32")) && file.stat.is_some_and(|s| s.exists) && crate::right::menu_item(ui, reveal_label(), "") {
            action = Some(Action::Reveal(file.path.clone()));
            ui.close();
        }
        ui.separator();
        if crate::right::menu_item(ui, tr("Remove from recents"), "") {
            action = Some(Action::Remove(file.path.clone()));
            ui.close();
        }
    });
    action
}

/// The platform's name for showing a file in its folder.
fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        tr("Show in Finder")
    } else if cfg!(windows) {
        tr("Show in Explorer")
    } else {
        tr("Show in folder")
    }
}

/// The arrow keys on the selected file: the one `step` away in `files` (wrapping at row ends,
/// stopping at the first and last), selected and given the keyboard.
fn arrows(ui: &Ui, recents: &mut Recents, files: &[Shown], across: usize, down: usize) {
    let Some(at) = files.iter().position(|f| ui.memory(|m| m.has_focus(card_id(&f.path)))) else { return };
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k) && !i.modifiers.any());
    let to = if pressed(Key::ArrowRight) {
        (across > 0).then(|| at + across)
    } else if pressed(Key::ArrowLeft) {
        (across > 0).then(|| at.checked_sub(across)).flatten()
    } else if pressed(Key::ArrowDown) {
        Some(at + down)
    } else if pressed(Key::ArrowUp) {
        at.checked_sub(down)
    } else {
        None
    };
    if let Some(f) = to.and_then(|i| files.get(i)) {
        recents.selected = Some(f.path.clone());
        ui.memory_mut(|m| m.request_focus(card_id(&f.path)));
    }
}

/// The square file-type mark before a name.
fn file_icon(ui: &Ui, t: &Tokens, center: egui::Pos2, path: &str) {
    let r = Rect::from_center_size(center, vec2(16.0, 16.0));
    match FileKind::of(Path::new(path)) {
        FileKind::FigCraft | FileKind::Fig => {
            ui.painter().rect_filled(r, CornerRadius::same(4), ACCENT);
            icons::paint(ui.painter(), r.shrink(3.0), Icon::Logo, Color32::WHITE);
        }
        kind => {
            ui.painter().rect_filled(r, CornerRadius::same(4), t.field);
            let glyph = match kind {
                FileKind::Svg => Icon::Vector,
                FileKind::Image => Icon::Image,
                _ => Icon::Page,
            };
            icons::paint(ui.painter(), r.shrink(2.0), glyph, t.icon_secondary);
        }
    }
}

/// The name a card or row has for screen readers: "name, Edited 3 hours ago".
fn spoken(file: &Shown, now: u64) -> String {
    let detail = file.detail(now);
    if detail.is_empty() { file.name.clone() } else { format!("{}, {detail}", file.name) }
}

fn grid(ui: &mut Ui, t: &Tokens, recents: &mut Recents, files: &[Shown], now: u64, time: f64, reduce_motion: bool) -> Option<Action> {
    let (columns, card_w) = grid_columns(ui.available_width());
    let (picture_h, card_h) = card_heights(card_w);
    let mut action = None;
    let mut fading = false;
    for row in files.chunks(columns) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, file) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + GAP), r.min.y), vec2(card_w, card_h));
            let resp = ui.interact(rect, card_id(&file.path), Sense::click());
            a11y::named_button(&resp, &spoken(file, now));
            let resp = resp.on_hover_text(&file.path);
            let visible = ui.is_rect_visible(rect);
            let picture = recents.picture(&file.path, visible, time);
            let selected = recents.selected.as_deref() == Some(file.path.as_str());
            if let Some(a) = interact(ui, &resp, recents, file) {
                action = Some(a);
            }
            if !visible {
                continue;
            }
            let Some(stat) = file.stat else {
                skeleton(ui, t, rect, picture_h);
                continue;
            };
            let alpha = picture.as_ref().map_or(1.0, |(_, _, a)| if reduce_motion { 1.0 } else { *a });
            fading |= alpha < 1.0;
            card(ui, t, rect, picture_h, &resp, selected, file, stat, picture, alpha, now);
        }
        ui.add_space(GAP);
    }
    if fading {
        ui.ctx().request_repaint();
    }
    arrows(ui, recents, files, 1, columns);
    action
}

/// A card while its file is being looked at: grey blocks where the picture, icon and text go.
fn skeleton(ui: &Ui, t: &Tokens, rect: Rect, picture_h: f32) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), t.panel);
    let picture = Rect::from_min_size(rect.min, vec2(rect.width(), picture_h)).shrink(1.0);
    p.rect_filled(picture, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, t.field);
    let y = picture.max.y + FOOTER_H / 2.0;
    p.rect_filled(Rect::from_center_size(pos2(rect.min.x + 24.0, y), vec2(16.0, 16.0)), CornerRadius::same(4), t.field);
    p.rect_filled(Rect::from_min_size(pos2(rect.min.x + 44.0, y - 11.0), vec2(rect.width() * 0.45, 9.0)), CornerRadius::same(3), t.field);
    p.rect_filled(Rect::from_min_size(pos2(rect.min.x + 44.0, y + 4.0), vec2(rect.width() * 0.3, 8.0)), CornerRadius::same(3), t.field);
    p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &Ui,
    t: &Tokens,
    rect: Rect,
    picture_h: f32,
    resp: &Response,
    selected: bool,
    file: &Shown,
    stat: Stat,
    picture: Option<(Option<Color32>, Option<egui::TextureHandle>, f32)>,
    alpha: f32,
    now: u64,
) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), if resp.hovered() { t.hover } else { t.panel });
    let area = Rect::from_min_size(rect.min, vec2(rect.width(), picture_h)).shrink(1.0);
    let top = CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 };
    p.rect_filled(area, top, t.canvas);
    match picture {
        Some((page, texture, _)) if stat.exists && (page.is_some() || texture.is_some()) => {
            if let Some(page) = page {
                p.rect_filled(area, top, page.gamma_multiply(alpha));
            }
            if let Some(texture) = texture {
                let size = texture.size_vec2();
                let inner = area.shrink(area.height() * 0.05);
                let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
                let image = Rect::from_center_size(inner.center(), size * fit);
                p.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha));
            }
        }
        // Being read: the empty area, the picture fades in.
        None if stat.exists => {}
        // Nothing to show (a missing file, an svg, a damaged picture): a file glyph.
        _ => icons::paint(p, Rect::from_center_size(area.center(), vec2(28.0, 28.0)), Icon::Page, t.icon_secondary),
    }
    p.line_segment([pos2(rect.min.x, area.max.y + 0.5), pos2(rect.max.x, area.max.y + 0.5)], Stroke::new(1.0, t.border));
    let y = area.max.y + 1.0 + FOOTER_H / 2.0;
    file_icon(ui, t, pos2(rect.min.x + 24.0, y), &file.path);
    let (x, w) = (rect.min.x + 44.0, rect.width() - 56.0);
    let name = widgets::elide_font(ui, &file.name, w, semibold(12.5));
    p.text(pos2(x, y - 8.0), Align2::LEFT_CENTER, name, semibold(12.5), t.text);
    let detail = widgets::elide_font(ui, &file.detail(now), w, regular(11.5));
    p.text(pos2(x, y + 9.0), Align2::LEFT_CENTER, detail, regular(11.5), t.text_secondary);
    if selected {
        p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, ACCENT), StrokeKind::Inside);
    } else {
        p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, if resp.hovered() { t.border_strong } else { t.border }), StrokeKind::Inside);
        if resp.has_focus() {
            p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, t.focus), StrokeKind::Outside);
        }
    }
}

/// The table: Name, Last modified and Created (where the platform keeps it) headers that sort, and
/// a row per file.
fn list(ui: &mut Ui, t: &Tokens, recents: &mut Recents, files: &[Shown], now: u64, sort: Sort, with_created: bool) -> Option<Action> {
    let width = ui.available_width();
    let date_w = 180.0_f32.min(width / 4.0);
    let mut columns = vec![(SortKey::Name, tr("Name")), (SortKey::Modified, tr("Last modified"))];
    if with_created {
        columns.push((SortKey::Created, tr("Created")));
    }
    // Each column's left edge: the name takes what the dates leave.
    let date_x = |i: usize| width - date_w * (columns.len() - i) as f32;
    let mut action = None;
    let (header, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    for (i, (key, label)) in columns.iter().enumerate() {
        let x0 = if i == 0 { header.min.x } else { header.min.x + date_x(i) };
        let x1 = if i + 1 < columns.len() { header.min.x + date_x(i + 1) } else { header.max.x };
        let cell = Rect::from_min_max(pos2(x0, header.min.y), pos2(x1, header.max.y));
        let resp = ui.interact(cell, Id::new(("home_recents_header", key.id())), Sense::click());
        a11y::button(&resp, &trf("Sort by {column}", &[("column", label)]));
        let on = sort.key == *key;
        if resp.hovered() {
            ui.painter().rect_filled(cell, CornerRadius::same(4), t.hover);
        }
        let font = if on { semibold(11.0) } else { regular(11.0) };
        let end = ui
            .painter()
            .text(pos2(cell.min.x + 12.0, cell.center().y), Align2::LEFT_CENTER, *label, font, if on { t.text } else { t.text_secondary })
            .max
            .x;
        if on {
            let icon = if sort.descending { Icon::Chevron } else { Icon::ChevronUp };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(end + 10.0, cell.center().y), vec2(10.0, 10.0)), icon, t.icon_secondary);
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(cell, CornerRadius::same(4), Stroke::new(2.0, t.focus), StrokeKind::Outside);
        }
        if resp.clicked() {
            action = Some(Action::Sort(sort.clicked(*key)));
        }
    }
    ui.painter().line_segment([pos2(header.min.x, header.max.y - 0.5), pos2(header.max.x, header.max.y - 0.5)], Stroke::new(1.0, t.border));
    for file in files {
        let (r, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
        let resp = ui.interact(r, card_id(&file.path), Sense::click());
        a11y::named_button(&resp, &spoken(file, now));
        let resp = resp.on_hover_text(&file.path);
        if let Some(a) = interact(ui, &resp, recents, file) {
            action = Some(a);
        }
        if !ui.is_rect_visible(r) {
            continue;
        }
        let selected = recents.selected.as_deref() == Some(file.path.as_str());
        let p = ui.painter();
        if selected {
            p.rect_filled(r, CornerRadius::same(5), t.selected_row);
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(5), t.hover);
        }
        if resp.has_focus() && !selected {
            p.rect_stroke(r, CornerRadius::same(5), Stroke::new(2.0, t.focus), StrokeKind::Inside);
        }
        file_icon(ui, t, pos2(r.min.x + 20.0, r.center().y), &file.path);
        let name_w = date_x(1) - 52.0;
        p.text(
            pos2(r.min.x + 40.0, r.center().y),
            Align2::LEFT_CENTER,
            widgets::elide_font(ui, &file.name, name_w, regular(12.0)),
            regular(12.0),
            t.text,
        );
        let cells: Vec<String> = columns
            .iter()
            .skip(1)
            .map(|(key, _)| match file.stat {
                Some(Stat { exists: false, .. }) if *key == SortKey::Modified => tr("File not found").into(),
                Some(s) => match *key {
                    SortKey::Modified => s.modified,
                    _ => s.created,
                }
                .map(|then| ago(now, then))
                .unwrap_or_default(),
                None => String::new(),
            })
            .collect();
        for (i, text) in cells.iter().enumerate() {
            let x = r.min.x + date_x(i + 1) + 12.0;
            p.text(
                pos2(x, r.center().y),
                Align2::LEFT_CENTER,
                widgets::elide_font(ui, text, date_w - 16.0, regular(11.5)),
                regular(11.5),
                t.text_secondary,
            );
        }
        p.line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
    }
    arrows(ui, recents, files, 0, 1);
    action
}
