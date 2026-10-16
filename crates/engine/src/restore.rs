//! Session restore (milestone M12): the app comes back the way it was left.
//!
//! The interface keeps a [`SavedSession`] with the preferences ([`Prefs::session`](crate::Prefs)),
//! written whenever it changes: each window's place on screen and panels, its tabs in order and
//! the one in front. A tab is a saved file (its path) or one of your online files (its server and
//! id, never a token: it is joined again through the record, `collab.openFile`). Documents that
//! were never saved aren't in it: unsaved work is the recovery copies' job.
//!
//! `session.restore` opens the tabs again and asks the interface to put them in their windows.
//! Anything may be in a preferences file: [`SavedSession::read`] keeps what is usable, within
//! limits, and never fails.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::{DocState, Result, Session, ViewState, bad, spec};

/// Most windows and most tabs in a window a saved session holds.
pub const MAX_WINDOWS: usize = 16;
pub const MAX_TABS: usize = 100;

/// A window's sides, in screen points: anything outside isn't a window anyone left.
const SIDE: std::ops::RangeInclusive<f32> = 100.0..=16_384.0;
const FAR: f32 = 1_000_000.0;

/// The windows the app had open, the main window first.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SavedSession {
    pub windows: Vec<SavedWindow>,
}

/// A window: where it was (the interface checks it against the screens there are now), its
/// panels and its tabs.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWindow {
    /// `[x, y, width, height]` while it was neither maximized nor full screen (screen points).
    pub rect: Option<[f32; 4]>,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The screen it was on, `[x, y, width, height]`.
    pub monitor: Option<[f32; 4]>,
    pub tabs: Vec<SavedTab>,
    /// Which of `tabs` was in front.
    pub front: usize,
    /// The side panels' tabs (the interface's own names) and widths.
    pub left_tab: String,
    pub right_tab: String,
    pub left_collapsed: bool,
    pub panel_widths: Option<[f32; 2]>,
}

/// A tab: a saved file, or one of your online files.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SavedTab {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// An online file's server (`wss://host/`, never with a key) and id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The page shown (its place among the file's pages).
    pub page: usize,
    /// The view: the page point at the centre, and the zoom.
    pub view: Option<[f64; 3]>,
}

/// `n` finite numbers.
fn numbers<const N: usize>(v: Option<&Value>) -> Option<[f64; N]> {
    let list = v?.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (slot, n) in out.iter_mut().zip(list) {
        *slot = n.as_f64().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// A rectangle a window or a screen can have: on a sane part of the plane, neither empty nor huge.
fn rect(v: Option<&Value>) -> Option<[f32; 4]> {
    let r = numbers::<4>(v)?.map(|n| n as f32);
    (r[0].abs() <= FAR && r[1].abs() <= FAR && SIDE.contains(&r[2]) && SIDE.contains(&r[3])).then_some(r)
}

/// A short text without control characters (`None`: not one, or longer than `max`).
fn text(v: Option<&Value>, max: usize) -> Option<String> {
    v?.as_str().filter(|s| !s.is_empty() && s.len() <= max && !s.contains(char::is_control)).map(str::to_string)
}

impl SavedTab {
    fn read(v: &Value) -> Option<Self> {
        let path = text(v.get("path"), 4096);
        let server = text(v.get("server"), 512).filter(|s| s.starts_with("ws://") || s.starts_with("wss://"));
        let file = text(v.get("file"), 64).filter(|f| f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        let (server, file) = server.zip(file).unzip();
        if path.is_none() && file.is_none() {
            return None;
        }
        let page = v.get("page").and_then(Value::as_u64).map_or(0, |p| p.min(100_000) as usize);
        let far = |n: f64| n.clamp(-1e9, 1e9);
        let view = numbers::<3>(v.get("view")).map(|[x, y, z]| [far(x), far(y), z.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM)]);
        Some(Self { path, server, file, page, view })
    }

    /// The same file as `other`.
    fn is(&self, other: &SavedTab) -> bool {
        match (&self.path, &other.path) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self.file == other.file && self.server == other.server,
            _ => false,
        }
    }

    /// The file's name, for a message.
    fn name(&self) -> String {
        let path =
            self.path.as_deref().map(|p| std::path::Path::new(p).file_name().map_or_else(|| p.to_string(), |f| f.to_string_lossy().into_owned()));
        path.or_else(|| self.file.clone()).unwrap_or_default()
    }
}

impl SavedWindow {
    fn read(v: &Value) -> Option<Self> {
        v.as_object()?;
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let mut tabs: Vec<SavedTab> = Vec::new();
        let listed = v.get("tabs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        // The tab in front keeps its place when tabs before it are dropped.
        let asked = v.get("front").and_then(Value::as_u64).unwrap_or(0);
        let mut front = 0;
        for (i, tab) in listed.iter().take(4 * MAX_TABS).enumerate() {
            let Some(tab) = SavedTab::read(tab).filter(|t| tabs.len() < MAX_TABS && !tabs.iter().any(|o| o.is(t))) else { continue };
            if i as u64 <= asked {
                front = tabs.len();
            }
            tabs.push(tab);
        }
        Some(Self {
            rect: rect(v.get("rect")),
            maximized: flag("maximized"),
            fullscreen: flag("fullscreen"),
            monitor: rect(v.get("monitor")),
            tabs,
            front,
            left_tab: text(v.get("leftTab"), 32).unwrap_or_default(),
            right_tab: text(v.get("rightTab"), 32).unwrap_or_default(),
            left_collapsed: flag("leftCollapsed"),
            panel_widths: numbers::<2>(v.get("panelWidths")).map(|w| w.map(|w| (w as f32).clamp(0.0, 10_000.0))),
        })
    }
}

impl SavedSession {
    /// What of `v` is a usable saved session (nothing, for anything else).
    pub fn read(v: &Value) -> Self {
        let listed = v.get("windows").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        Self { windows: listed.iter().take(4 * MAX_WINDOWS).filter_map(SavedWindow::read).take(MAX_WINDOWS).collect() }
    }

    /// How many tabs it would open.
    pub fn tabs(&self) -> usize {
        self.windows.iter().map(|w| w.tabs.len()).sum()
    }
}

impl<'de> Deserialize<'de> for SavedSession {
    /// Junk here never costs the other preferences.
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Value::deserialize(d).map(|v| Self::read(&v)).unwrap_or_default())
    }
}

/// One of your online files on its way back into a restored window.
struct Pending {
    /// The live session joining it.
    session: u64,
    /// Which of the restored windows it belongs to, and whether it is the tab in front there.
    window: usize,
    front: bool,
    tab: SavedTab,
    /// The tab before it in its window (`None`: it leads).
    after: Option<u64>,
}

/// A restored online file that arrived from its server ([`Session::restore_arrivals`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// The open document.
    pub uid: u64,
    /// Which of the restored windows it belongs to (as `session.restore` listed them).
    pub window: usize,
    /// It was the tab in front there.
    pub front: bool,
}

/// A restored window for the interface to place: what was saved of it, its tabs that are open
/// (documents, in order) and the one in front.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub saved: SavedWindow,
    pub docs: Vec<u64>,
    pub front: Option<u64>,
}

/// What a session knows about restoring.
#[derive(Default)]
pub struct State {
    /// The session of the last run, when this run started without restoring it: what
    /// `session.restore` brings back (once).
    pub last: Option<SavedSession>,
    /// The windows `session.restore` opened the tabs of, until the interface places them, and
    /// how many files were gone.
    pub placing: Option<(Vec<Placed>, usize)>,
    pending: Vec<Pending>,
}

impl State {
    /// The online files still on their way, with the restored window each belongs to: a session
    /// saved meanwhile keeps them.
    pub fn joining(&self) -> impl Iterator<Item = (usize, &SavedTab)> {
        self.pending.iter().map(|p| (p.window, &p.tab))
    }
}

/// Show `tab`'s page and view in its document.
fn show(ds: &mut DocState, tab: &SavedTab) {
    if let Some(page) = ds.doc.pages.get(tab.page).copied().filter(|p| *p != ds.page && !ds.doc.is_divider(*p)) {
        ds.page = page;
        ds.selection.clear();
    }
    if let Some([cx, cy, zoom]) = tab.view {
        (ds.view.cx, ds.view.cy, ds.view.zoom) = (cx, cy, zoom);
        // The interface fits a document to its window the first time it sizes its view (one
        // still exactly at the default size): this one has its view already.
        ds.view.vw += 1e-6;
    }
}

impl Session {
    /// Open document `d` as a tab of a saved session (`None`: one that was never saved, or a
    /// branch's tab, which opens from its file).
    pub fn saved_tab(&self, d: &DocState) -> Option<SavedTab> {
        if d.branch_of.is_some() {
            return None;
        }
        let online = || self.online_of(d.uid).map(|(server, file, _)| (server, file));
        let (server, file) = if d.path.is_some() { (None, None) } else { online().unzip() };
        if d.path.is_none() && file.is_none() {
            return None;
        }
        let page = d.doc.pages.iter().position(|p| *p == d.page).unwrap_or(0);
        Some(SavedTab { path: d.path.clone(), server, file, page, view: Some([d.view.cx, d.view.cy, d.view.zoom]) })
    }

    /// Move open document `uid` to just after document `after` (`None`: before every other).
    fn tab_after(&mut self, uid: u64, after: Option<u64>) {
        let active = self.docs.get(self.active).map(|d| d.uid);
        let Some(from) = self.docs.iter().position(|d| d.uid == uid) else { return };
        let d = self.docs.remove(from);
        let to = match after {
            Some(a) => self.docs.iter().position(|d| d.uid == a).map_or(self.docs.len(), |i| i + 1),
            None => 0,
        };
        self.docs.insert(to, d);
        self.active = self.docs.iter().position(|d| Some(d.uid) == active).unwrap_or(self.active);
    }

    /// The restored online files that arrived from their servers since the last call, each in
    /// its place among the tabs with its page and view; one that couldn't be joined is skipped
    /// with a message. The interface puts them in their windows.
    pub fn restore_arrivals(&mut self) -> Vec<Arrival> {
        let mut out = Vec::new();
        // The queue stays in saved order while it is walked: a tab that arrived leaves it, the
        // later ones keep their place, and the ones still on their way stay for the next call.
        // Two that arrive in the same call must land in saved order, so each is placed after the
        // tab before it — which the arrival of an earlier one has just made concrete.
        let mut queue = std::mem::take(&mut self.restore.pending);
        let mut i = 0;
        while i < queue.len() {
            let (session, window, front, after) =
                { let p = &queue[i]; (p.session, p.window, p.front, p.after) };
            let live = self.live.get(session).map(|l| (l.doc_uid, l.status.clone()));
            match live {
                Some((Some(uid), _)) if self.docs.iter().any(|d| d.uid == uid) => {
                    self.tab_after(uid, after);
                    if let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) {
                        show(ds, &queue[i].tab);
                    }
                    // The tabs that follow it there come after it (the ones still unvisited).
                    for later in queue[i + 1..].iter_mut().filter(|l| l.window == window && l.after == after) {
                        later.after = Some(uid);
                    }
                    out.push(Arrival { uid, window, front });
                    queue.remove(i);
                }
                Some((_, status)) if status.active() => i += 1,
                gone => {
                    let why = gone.map(|(_, status)| format!(": {}", status.label())).unwrap_or_default();
                    self.message = Some(format!("{} couldn't be reopened from its server{why}", queue[i].tab.name()));
                    queue.remove(i);
                }
            }
        }
        self.restore.pending = queue;
        out
    }
}

/// How `tab` came back.
enum Opened {
    Doc(u64),
    Joining(u64),
    Missing,
}

fn open(s: &mut Session, tab: &SavedTab) -> Opened {
    let uid_at =
        |s: &Session, r: &Value| r.get("index").or(r.get("document")).and_then(Value::as_u64).and_then(|i| s.docs.get(i as usize)).map(|d| d.uid);
    if let Some(path) = &tab.path {
        return match crate::cmd::file::open(s, &json!({"path": path})) {
            Ok(r) => uid_at(s, &r).map_or(Opened::Missing, Opened::Doc),
            Err(e) => {
                log::info!("session restore: {e}");
                Opened::Missing
            }
        };
    }
    match crate::online::open_file(s, &json!({"file": tab.file, "server": tab.server})) {
        Ok(r) => match (uid_at(s, &r), r.get("session").and_then(Value::as_u64)) {
            (Some(uid), _) => Opened::Doc(uid),
            (None, Some(session)) => Opened::Joining(session),
            (None, None) => Opened::Missing,
        },
        Err(e) => {
            log::info!("session restore: {e}");
            Opened::Missing
        }
    }
}

fn restore(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "session.restore";
    let saved = match p.get("session") {
        None | Some(Value::Null) => s.restore.last.take().unwrap_or_else(|| s.prefs.session.clone()),
        Some(v) if v.is_object() => SavedSession::read(v),
        Some(_) => return Err(bad(CMD, "`session` must be a saved session: {windows: [{tabs: [{path} | {server, file}]}]}")),
    };
    let was_open: Vec<u64> = s.docs.iter().map(|d| d.uid).collect();
    let (mut windows, mut missing, mut joining, mut opened) = (Vec::new(), Vec::new(), 0, 0);
    // The tabs take the saved order: each after the one placed before it.
    let (mut front_doc, mut last) = (None, None);
    for (k, w) in saved.windows.iter().enumerate() {
        let (mut docs, mut front) = (Vec::new(), None);
        for (i, tab) in w.tabs.iter().enumerate() {
            match open(s, tab) {
                // A file is a tab of one window: the first that lists it.
                Opened::Doc(uid) if docs.contains(&uid) || windows.iter().any(|w: &Placed| w.docs.contains(&uid)) => {}
                Opened::Doc(uid) => {
                    if !was_open.contains(&uid) {
                        opened += 1;
                        if let Some(ds) = s.docs.iter_mut().find(|d| d.uid == uid) {
                            show(ds, tab);
                        }
                    }
                    s.tab_after(uid, last);
                    docs.push(uid);
                    last = Some(uid);
                    if i == w.front {
                        front = Some(uid);
                    }
                }
                Opened::Joining(session) => {
                    joining += 1;
                    s.restore.pending.retain(|p| p.session != session);
                    s.restore.pending.push(Pending { session, window: k, front: i == w.front, tab: tab.clone(), after: docs.last().copied() });
                }
                Opened::Missing => missing.push(tab.name()),
            }
        }
        front_doc = front_doc.or(front).or(docs.first().copied());
        windows.push(Placed { saved: w.clone(), docs, front });
    }
    if let Some(i) = front_doc.and_then(|u| s.docs.iter().position(|d| d.uid == u)) {
        s.active = i;
    }
    let out = json!({"windows": windows.len(), "opened": opened, "joining": joining, "missing": missing});
    if !windows.is_empty() {
        s.restore.placing = Some((windows, missing.len()));
        s.request_ui(CMD, json!({}));
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "session.restore",
            "Restore last session",
            "{session? (a saved session, as the control channel's `ui.inspect` gives it under `session.saved`: {windows: [{tabs: [{path} | {server, file}, page?, view?: [cx, cy, zoom]], front?, rect?: [x, y, width, height], maximized?, fullscreen?}]}; default: the one kept with the preferences, or the last run's when this run started without it)} → {windows, opened (tabs opened now), joining (your online files on their way from their servers), missing: [names of files that are gone]}: opens the saved windows' tabs again in order, with their page and view (a file that is open already is used as it is), and the app puts them in their windows where they were (a window that would be off every screen moves onto one). Headless (CLI, MCP): the tabs only",
            false,
            restore
        ),
        spec!("pref.restoreSession", "Reopen windows and tabs from last time", "{on?}", false, |s, p| {
            s.prefs.restore_session = crate::params::boolean(p, "on").unwrap_or(!s.prefs.restore_session);
            Ok(json!({"on": s.prefs.restore_session}))
        }),
    ]
}
