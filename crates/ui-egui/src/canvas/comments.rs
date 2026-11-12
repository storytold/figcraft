//! Comments: pins on the canvas (gathered into clusters when they crowd together; yours can be
//! dragged to another spot or frame), the thread popover (edit and delete your messages, emoji
//! reactions, @mentions, read state, copy link), the new-comment composer, the Comments panel shown
//! in place of the Design panel while the Comment tool is active (search, filter, sort, keyboard
//! navigation) and pins on a presented frame. Every change goes through a `comment.*` command.

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use figcraft_doc::{Comment, Message, NodeId, mention_ranges};
use figcraft_engine::ToolId;
use figcraft_engine::cmd::comment::{self as cc, Filter, Query, Sort};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, Tokens, regular, semibold};

const PIN: f32 = 28.0;
const POPOVER_W: f32 = 300.0;
/// Pins whose centres come closer than this (screen points) gather into a cluster.
pub const CLUSTER_R: f32 = 24.0;
/// A press on your pin that moves further than this drags it.
const DRAG_SLOP: f32 = 3.0;
/// The reactions on offer (text emoji).
pub const REACTIONS: [&str; 8] = ["👍", "❤", "😂", "🎉", "👀", "🙏", "🔥", "✅"];
/// Names the @mention picker lists at most.
const MENTION_ROWS: usize = 6;
/// A speech bubble: round on three corners, pointed at its spot.
const BUBBLE: CornerRadius = CornerRadius { nw: 14, ne: 14, se: 14, sw: 0 };

/// Comment UI state.
pub struct CommentUi {
    /// The open thread.
    pub open: Option<u64>,
    /// A new comment being written at a page point.
    pub draft: Option<(f64, f64, String)>,
    pub reply: String,
    /// What the panel (and the pins) show, and in what order.
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    /// A message being edited: (thread, message index, text).
    pub editing: Option<(u64, usize, String)>,
    pub mention: MentionPick,
    drag: Option<PinDrag>,
    /// Scroll the panel to the open thread (after keyboard navigation).
    scroll_to_open: bool,
}

impl Default for CommentUi {
    fn default() -> Self {
        Self {
            open: None,
            draft: None,
            reply: String::new(),
            filter: Filter::Open,
            sort: Sort::Date,
            search: String::new(),
            editing: None,
            mention: MentionPick::default(),
            drag: None,
            scroll_to_open: false,
        }
    }
}

impl CommentUi {
    /// A pin is being pressed or dragged.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Show a thread (a pin click, a link, the panel, keyboard navigation).
    pub fn show(&mut self, id: u64) {
        if self.open != Some(id) {
            self.reply.clear();
        }
        self.open = Some(id);
        self.draft = None;
        self.editing = None;
        self.mention = MentionPick::default();
    }

    /// Close the open thread and the draft.
    pub fn close(&mut self) {
        self.open = None;
        self.draft = None;
        self.editing = None;
        self.mention = MentionPick::default();
    }
}

/// The @mention picker: the names it offers for the `@query` being typed in one text field.
#[derive(Clone, Debug, Default)]
pub struct MentionPick {
    field: Option<Id>,
    /// The `@query`'s byte range in the field's text.
    range: (usize, usize),
    names: Vec<String>,
    pub sel: usize,
    /// Dismissed with Escape: (field, where its `@` is).
    dismissed: Option<(Id, usize)>,
}

impl MentionPick {
    /// It offers names (in some field).
    pub fn active(&self) -> bool {
        self.field.is_some() && !self.names.is_empty()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// A pin being pressed: dragged when it's yours and the pointer moves.
#[derive(Clone, Copy, Debug)]
struct PinDrag {
    id: u64,
    from: Pos2,
    /// The pointer's offset from the pin's spot.
    grab: Vec2,
    /// Where the pin's spot is now (screen).
    at: Pos2,
    mine: bool,
    moved: bool,
}

// ---------------------------------------------------------------------------------------------
// Pure helpers (tested in `comments_tests.rs`).

/// The `@query` being typed just before the cursor (`cursor` in chars): its byte range, from the
/// `@` to the cursor. None when the cursor isn't in one (a mention starts the text or follows a
/// non-alphanumeric character, stays on one line and doesn't end in a space).
pub fn mention_query(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let end = text.char_indices().nth(cursor).map_or(text.len(), |(b, _)| b);
    let before = text.get(..end)?;
    let at = before.rfind('@')?;
    let q = before.get(at + 1..)?;
    if q.contains('\n') || q.chars().count() > 40 || q.ends_with(char::is_whitespace) {
        return None;
    }
    let starts = before.get(..at)?.chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    starts.then_some((at, end))
}

/// The people (not `me`) whose name, or a word in it, starts with `query` (any case).
pub fn mention_matches(people: &[String], query: &str, me: &str) -> Vec<String> {
    let q = query.to_lowercase();
    people
        .iter()
        .filter(|n| n.as_str() != me)
        .filter(|n| {
            let l = n.to_lowercase();
            l.starts_with(&q) || l.split_whitespace().any(|w| w.starts_with(&q))
        })
        .take(MENTION_ROWS)
        .cloned()
        .collect()
}

/// Replace the `@query` at `range` with `@name ` and return the cursor after it (chars).
pub fn insert_mention(text: &mut String, range: (usize, usize), name: &str) -> usize {
    let (a, b) = range;
    if a > b || b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return text.chars().count();
    }
    let ins = format!("@{name} ");
    text.replace_range(a..b, &ins);
    text.get(..a).map_or(0, |t| t.chars().count()) + ins.chars().count()
}

/// Group points closer than `r` to a group's centre (in order: each joins the first group near
/// enough, else starts one). Returns the groups as indexes.
pub fn cluster(points: &[Pos2], r: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<(Vec<usize>, Vec2)> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let near = groups.iter_mut().find(|(g, sum)| (*sum / g.len() as f32).to_pos2().distance(*p) < r);
        match near {
            Some((g, sum)) => {
                g.push(i);
                *sum += p.to_vec2();
            }
            None => groups.push((vec![i], p.to_vec2())),
        }
    }
    groups.into_iter().map(|(g, _)| g).collect()
}

/// A message's text with its @mentions of `people` picked out, wrapped at `width`.
pub fn rich_text(text: &str, people: &[&str], color: Color32, mention_color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let plain = TextFormat { font_id: regular(12.0), color, ..Default::default() };
    let mention = TextFormat { font_id: semibold(12.0), color: mention_color, ..Default::default() };
    let mut at = 0;
    for r in mention_ranges(text, people) {
        job.append(text.get(at..r.start).unwrap_or(""), 0.0, plain.clone());
        job.append(text.get(r.clone()).unwrap_or(""), 0.0, mention.clone());
        at = r.end;
    }
    job.append(text.get(at..).unwrap_or(""), 0.0, plain);
    job.wrap.max_width = width;
    job
}

// ---------------------------------------------------------------------------------------------
// Pins.

/// The user's name (comments are signed with it).
fn me(app: &FigApp) -> String {
    cc::user_name(&app.session)
}

/// Pins are visible with the Comment tool, or when View › Comments is on (Shift+C).
fn visible(app: &FigApp) -> bool {
    app.session.tool.tool == ToolId::Comment
        || app.session.ds().is_ok_and(|ds| ds.view.comments)
        || app.ui.comments.open.is_some()
        || app.ui.comments.draft.is_some()
}

/// A pin: its spot is the bubble's bottom-left corner (screen).
#[derive(Clone, Debug)]
struct Pin {
    id: u64,
    author: String,
    resolved: bool,
    unread: bool,
    mine: bool,
    at: Pos2,
}

impl Pin {
    fn of(c: &Comment, me: &str, at: Pos2) -> Self {
        Pin { id: c.id, author: c.first.author.clone(), resolved: c.resolved, unread: c.is_unread(me), mine: c.first.author == me, at }
    }
}

/// The current page's pins (those the panel's filter keeps, and the open thread's) in drawing
/// order: oldest first, the open one on top; a dragged pin sits under the pointer.
fn pins(app: &FigApp) -> Vec<Pin> {
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let me = me(app);
    let cu = &app.ui.comments;
    let q = Query { page: Some(ds.page), filter: cu.filter, ..Query::default() };
    let mut list = cc::query(&ds.doc, &q, &me);
    list.reverse();
    if let Some(c) = cu.open.and_then(|o| ds.doc.comment(o)).filter(|c| c.page == ds.page && !list.iter().any(|x| x.id == c.id)) {
        list.push(c);
    }
    list.sort_by_key(|c| cu.open == Some(c.id));
    let drag = cu.drag.filter(|d| d.moved);
    list.into_iter()
        .map(|c| {
            let at = match drag {
                Some(d) if d.id == c.id => d.at,
                _ => {
                    let (x, y) = c.position(&ds.doc);
                    app.to_screen(x, y)
                }
            };
            Pin::of(c, &me, at)
        })
        .collect()
}

/// What the canvas shows: single pins, and clusters of crowded ones (their threads, spot).
enum Spot {
    One(Pin),
    Many(Vec<u64>, Pos2),
}

/// Pins, crowded ones gathered into clusters (the open and the dragged pin always stand alone, on
/// top).
fn spots(app: &FigApp) -> Vec<Spot> {
    let cu = &app.ui.comments;
    let alone = |p: &Pin| cu.open == Some(p.id) || cu.drag.is_some_and(|d| d.id == p.id);
    let (solo, rest): (Vec<Pin>, Vec<Pin>) = pins(app).into_iter().partition(alone);
    let centres: Vec<Pos2> = rest.iter().map(|p| pin_rect(p.at).center()).collect();
    let mut out: Vec<Spot> = Vec::new();
    for g in cluster(&centres, CLUSTER_R) {
        let members: Vec<&Pin> = g.iter().filter_map(|i| rest.get(*i)).collect();
        match members.as_slice() {
            [one] => out.push(Spot::One((*one).clone())),
            many => {
                let sum = many.iter().fold(Vec2::ZERO, |s, p| s + p.at.to_vec2());
                out.push(Spot::Many(many.iter().map(|p| p.id).collect(), (sum / many.len().max(1) as f32).to_pos2()));
            }
        }
    }
    out.extend(solo.into_iter().map(Spot::One));
    out
}

fn pin_rect(at: Pos2) -> Rect {
    Rect::from_min_max(pos2(at.x, at.y - PIN), pos2(at.x + PIN, at.y))
}

/// A cluster's bubble: wide enough for its count.
fn cluster_rect(at: Pos2, n: usize) -> Rect {
    let digits = n.to_string().len() as f32;
    Rect::from_min_max(pos2(at.x, at.y - PIN), pos2(at.x + PIN + 7.0 * (digits - 1.0), at.y))
}

/// A stable colour per author.
pub fn author_color(name: &str) -> Color32 {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    crate::live::people(h as usize)
}

fn avatar(painter: &egui::Painter, center: Pos2, r: f32, name: &str) {
    crate::live::avatar(painter, center, r, name, author_color(name));
}

/// Seconds since `t` (Unix seconds); `None` for unknown or future times.
fn age(t: u64) -> Option<u64> {
    let now = cc::now_secs();
    (t != 0 && now >= t).then(|| now - t)
}

/// "now", "5m", "3h", "2d", or a week count.
pub fn ago(created: u64) -> String {
    let Some(d) = age(created) else { return String::new() };
    match d {
        0..60 => tr("now").into(),
        60..3600 => trf("{n}m", &[("n", &(d / 60))]),
        3600..86_400 => trf("{n}h", &[("n", &(d / 3600))]),
        86_400..604_800 => trf("{n}d", &[("n", &(d / 86_400))]),
        _ => trf("{n}w", &[("n", &(d / 604_800))]),
    }
}

/// "Just now", "5 minutes ago", "Yesterday", "3 weeks ago"…
pub fn ago_long(t: u64) -> String {
    let Some(d) = age(t) else { return String::new() };
    let n = |div: u64| usize::try_from(d / div).unwrap_or(usize::MAX);
    match d {
        0..60 => tr("Just now").into(),
        60..3600 => trn("{n} minutes ago", n(60), &[]),
        3600..86_400 => trn("{n} hours ago", n(3600), &[]),
        86_400..172_800 => tr("Yesterday").into(),
        172_800..604_800 => trn("{n} days ago", n(86_400), &[]),
        604_800..2_592_000 => trn("{n} weeks ago", n(604_800), &[]),
        2_592_000..31_536_000 => trn("{n} months ago", n(2_592_000), &[]),
        _ => trn("{n} years ago", n(31_536_000), &[]),
    }
}

/// One pin: the author's avatar in a bubble (accent when `hot`), a dot when unread, a check when
/// resolved.
fn paint_pin(painter: &egui::Painter, r: Rect, p: &Pin, hot: bool) {
    painter.rect_filled(r.translate(vec2(0.0, 1.0)), BUBBLE, Color32::from_black_alpha(40));
    painter.rect_filled(r, BUBBLE, if hot { ACCENT } else { Color32::WHITE });
    painter.rect_stroke(r, BUBBLE, Stroke::new(1.0, Color32::from_black_alpha(30)), StrokeKind::Inside);
    let fill = if p.resolved { Color32::from_gray(150) } else { author_color(&p.author) };
    painter.circle_filled(r.center(), 11.0, fill);
    painter.text(r.center(), Align2::CENTER_CENTER, figcraft_engine::live::initials(&p.author), semibold(9.0), Color32::WHITE);
    if p.unread {
        let c = r.right_top() + vec2(-2.0, 2.0);
        painter.circle_filled(c, 4.5, Color32::WHITE);
        painter.circle_filled(c, 3.5, ACCENT);
    }
    if p.resolved {
        icons::paint(painter, Rect::from_center_size(r.right_bottom() + vec2(-3.0, -3.0), vec2(10.0, 10.0)), Icon::Check, ACCENT);
    }
}

/// A cluster: a stack of bubbles with the number of threads in it.
fn paint_cluster(painter: &egui::Painter, at: Pos2, n: usize, hot: bool) {
    let r = cluster_rect(at, n);
    let back = r.translate(vec2(3.0, -3.0));
    let edge = Stroke::new(1.0, Color32::from_black_alpha(30));
    painter.rect_filled(back, BUBBLE, Color32::WHITE);
    painter.rect_stroke(back, BUBBLE, edge, StrokeKind::Inside);
    painter.rect_filled(r.translate(vec2(0.0, 1.0)), BUBBLE, Color32::from_black_alpha(40));
    painter.rect_filled(r, BUBBLE, if hot { ACCENT } else { Color32::WHITE });
    painter.rect_stroke(r, BUBBLE, edge, StrokeKind::Inside);
    let inner = r.shrink(3.0);
    painter.rect_filled(inner, CornerRadius::same(11), Color32::from_gray(44));
    painter.text(inner.center(), Align2::CENTER_CENTER, n.to_string(), semibold(11.0), Color32::WHITE);
}

/// Draw the pins, clusters and the draft pin.
pub fn draw(app: &FigApp, painter: &egui::Painter) {
    if !visible(app) {
        return;
    }
    let hover = painter.ctx().input(|i| i.pointer.hover_pos());
    let hot = |r: Rect| hover.is_some_and(|h| r.contains(h));
    for spot in spots(app) {
        match spot {
            Spot::One(p) => {
                let r = pin_rect(p.at);
                paint_pin(painter, r, &p, app.ui.comments.open == Some(p.id) || hot(r));
            }
            Spot::Many(ids, at) => paint_cluster(painter, at, ids.len(), hot(cluster_rect(at, ids.len()))),
        }
    }
    if let Some((x, y, _)) = &app.ui.comments.draft {
        let r = pin_rect(app.to_screen(*x, *y));
        painter.rect_filled(r, BUBBLE, ACCENT);
        painter.circle_filled(r.center(), 3.0, crate::theme::ACCENT_ON);
    }
}

/// Open a thread, or close it when it's the open one.
fn toggle(app: &mut FigApp, id: u64) {
    if app.ui.comments.open == Some(id) {
        app.ui.comments.close();
    } else {
        app.ui.comments.show(id);
    }
}

/// Pointer handling: a pin click opens its thread, dragging your pin moves it (onto the frame
/// under it), a cluster click zooms in until its pins come apart. Returns true when the comments
/// own the pointer.
pub fn input(app: &mut FigApp, ctx: &egui::Context, pos: Pos2) -> bool {
    if !visible(app) {
        app.ui.comments.drag = None;
        return false;
    }
    let (pressed, down) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down()));
    if let Some(mut d) = app.ui.comments.drag.take() {
        if down {
            d.moved |= d.mine && (pos - d.from).length() > DRAG_SLOP;
            if d.moved {
                d.at = pos - d.grab;
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            app.ui.comments.drag = Some(d);
        } else if d.moved {
            let p = app.to_page(d.at);
            app.run("comment.move", json!({"id": d.id, "x": p.x, "y": p.y}));
        } else {
            toggle(app, d.id);
        }
        return true;
    }
    for spot in spots(app).iter().rev() {
        match spot {
            Spot::One(p) if pin_rect(p.at).contains(pos) => {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                if pressed {
                    app.ui.comments.drag = Some(PinDrag { id: p.id, from: pos, grab: pos - p.at, at: p.at, mine: p.mine, moved: false });
                }
                return true;
            }
            Spot::Many(ids, at) if cluster_rect(*at, ids.len()).contains(pos) => {
                ctx.set_cursor_icon(egui::CursorIcon::ZoomIn);
                if pressed {
                    app.run("comment.zoom", json!({"ids": ids}));
                }
                return true;
            }
            _ => {}
        }
    }
    // A click elsewhere on the canvas closes an open thread (the Comment tool then starts a draft).
    if pressed && app.ui.comments.open.is_some() {
        app.ui.comments.close();
        return app.session.tool.tool != ToolId::Comment;
    }
    false
}

/// Start a new comment at a page point (the Comment tool's click).
pub fn start_draft(app: &mut FigApp, x: f64, y: f64) {
    app.ui.comments.close();
    app.ui.comments.draft = Some((x, y, String::new()));
}

// ---------------------------------------------------------------------------------------------
// Composer and thread popover.

/// The reply field's id for a thread.
fn reply_id(thread: u64) -> Id {
    Id::unique(("comment_reply", thread))
}

fn set_cursor(ctx: &egui::Context, id: Id, at: usize) {
    let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at))));
    egui::TextEdit::store_state(ctx, id, st);
}

/// Focus a thread's reply field with the cursor at `at` (chars).
pub fn focus_reply(ctx: &egui::Context, thread: u64, at: usize) {
    set_cursor(ctx, reply_id(thread), at);
    ctx.memory_mut(|m| m.request_focus(reply_id(thread)));
}

/// A text field with a send button: Enter posts (Shift+Enter starts a new line); typing `@` offers
/// the names in `people` (arrows choose, Enter or Tab or a click inserts, Escape dismisses).
/// Returns true when the text is posted.
fn composer(ui: &mut Ui, id: Id, text: &mut String, hint: &str, people: &[String], me: &str, pick: &mut MentionPick) -> bool {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut submit = false;
    // The picker's keys and Enter, before the text field sees them.
    if ui.memory(|m| m.has_focus(id)) {
        if pick.field == Some(id) && pick.active() {
            let n = pick.names.len();
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                pick.sel = (pick.sel + 1) % n;
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                pick.sel = (pick.sel + n - 1) % n;
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                pick.dismissed = Some((id, pick.range.0));
            } else if ui.input_mut(|i| (!i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter)) || i.consume_key(Modifiers::NONE, Key::Tab))
                && let Some(name) = pick.names.get(pick.sel).cloned()
            {
                let at = insert_mention(text, pick.range, &name);
                set_cursor(&ctx, id, at);
            }
        } else if ui.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter)) {
            submit = true;
        }
    }
    let frame = egui::Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(8, 6));
    let mut cursor = None;
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            let out = egui::TextEdit::multiline(text)
                .id(id)
                .hint_text(hint)
                .font(regular(12.0))
                .frame(egui::Frame::NONE)
                .desired_rows(1)
                .desired_width(ui.available_width() - 30.0)
                .show(ui);
            let resp = out.response.response;
            if !resp.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
                resp.request_focus();
            }
            if resp.has_focus() {
                cursor = out.cursor_range.map(|r| r.primary.index.0);
            }
            let (br, bresp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            let ready = !text.trim().is_empty();
            ui.painter().circle_filled(br.center(), 12.0, if ready { ACCENT } else { t.hover });
            let c = br.center();
            let col = if ready { Color32::WHITE } else { t.icon_secondary };
            ui.painter().line_segment([c + vec2(0.0, 5.0), c + vec2(0.0, -5.0)], Stroke::new(1.6, col));
            ui.painter().line_segment([c + vec2(-4.0, -1.0), c + vec2(0.0, -5.0)], Stroke::new(1.6, col));
            ui.painter().line_segment([c + vec2(4.0, -1.0), c + vec2(0.0, -5.0)], Stroke::new(1.6, col));
            if bresp.on_hover_text(tr("Post")).clicked() {
                submit = true;
            }
        });
    });
    // The @mention picker for what's typed now (kept while nothing has the focus: a click on a
    // name takes it from the field for a moment).
    let found = cursor.and_then(|c| mention_query(text, c)).filter(|r| pick.dismissed != Some((id, r.0)));
    let names = found.map(|r| mention_matches(people, text.get(r.0 + 1..r.1).unwrap_or(""), me)).unwrap_or_default();
    let elsewhere = cursor.is_none() && crate::a11y::field_focused(ui.ctx());
    match found {
        Some(r) if !names.is_empty() => {
            if pick.field != Some(id) || pick.range.0 != r.0 {
                pick.sel = 0;
            }
            pick.sel = pick.sel.min(names.len() - 1);
            pick.field = Some(id);
            pick.range = r;
            pick.names = names;
        }
        _ if (cursor.is_some() || elsewhere) && pick.field == Some(id) => {
            pick.field = None;
            pick.names.clear();
        }
        _ => {}
    }
    if pick.field == Some(id) && pick.active() {
        ui.add_space(4.0);
        let mut chosen = None;
        for (k, name) in pick.names.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            if k == pick.sel || resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), if k == pick.sel { t.selected_row } else { t.hover });
            }
            avatar(ui.painter(), pos2(r.min.x + 14.0, r.center().y), 9.0, name);
            ui.painter().text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, name, regular(12.0), t.text);
            if resp.clicked() {
                chosen = Some(name.clone());
            }
        }
        if let Some(name) = chosen {
            let at = insert_mention(text, pick.range, &name);
            set_cursor(&ctx, id, at);
            ctx.memory_mut(|m| m.request_focus(id));
            *pick = MentionPick::default();
        }
    }
    submit && !text.trim().is_empty()
}

/// What the thread popover asked for.
enum Act {
    Close,
    Run(&'static str, Value),
    CopyLink,
    CopyForAi,
    MarkUnread,
    Edit(usize, String),
}

/// `{id, reply?}` naming a message of a thread (index 0: the first message).
fn message_params(thread: u64, index: usize) -> Value {
    match index.checked_sub(1) {
        Some(r) => json!({"id": thread, "reply": r}),
        None => json!({"id": thread}),
    }
}

/// A small icon button drawn only while `show` (its message is hovered or its menu is open).
fn hover_button(ui: &mut Ui, id: Id, icon: Icon, show: bool, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    let resp = ui.interact(r, id, Sense::click());
    if !show {
        return resp;
    }
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), r.shrink(2.0), icon, t.icon_secondary);
    resp.on_hover_text(tip)
}

/// A message's author, with a small "Agent" label when an AI agent wrote it.
fn author_name(ui: &mut Ui, t: &Tokens, m: &Message) {
    ui.label(egui::RichText::new(&m.author).font(semibold(12.0)).color(t.text));
    if !m.agent {
        return;
    }
    let galley = ui.painter().layout_no_wrap(tr("Agent").to_string(), semibold(10.0), t.text_secondary);
    let (r, resp) = ui.allocate_exact_size(galley.size() + vec2(8.0, 2.0), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tr("Agent")));
    ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, t.text_secondary);
}

/// What every message of a thread needs to know.
struct ThreadCtx<'a> {
    c: &'a Comment,
    me: &'a str,
    people: &'a [String],
}

/// One message: author, time, its buttons on hover (react; edit and delete when it's yours), the
/// text (or its editor) and the reactions.
fn message(
    ui: &mut Ui,
    cx: &ThreadCtx,
    i: usize,
    m: &Message,
    editing: &mut Option<(u64, usize, String)>,
    pick: &mut MentionPick,
    act: &mut Option<Act>,
) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let id = cx.c.id;
    let hover_key = Id::unique(("comment_msg_hover", id, i));
    let hovered = ctx.data(|d| d.get_temp::<bool>(hover_key)).unwrap_or(false);
    let indent = 36.0;
    let block = ui.scope(|ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            avatar(ui.painter(), r.center(), 12.0, &m.author);
            ui.add_space(4.0);
            author_name(ui, &t, m);
            let when = if m.edited > 0 { trf("{when} · edited", &[("when", &ago(m.created))]) } else { ago(m.created) };
            ui.label(egui::RichText::new(when).font(regular(11.0)).color(t.text_tertiary));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if m.author == cx.me {
                    let menu = Id::unique(("comment_msg_more_menu", id, i));
                    let more = hover_button(
                        ui,
                        Id::unique(("comment_msg_more", id, i)),
                        Icon::Dots,
                        hovered || egui::Popup::is_id_open(&ctx, menu),
                        tr("More"),
                    );
                    crate::right::menu_popup(&more).id(menu).show(|ui| {
                        if crate::right::menu_item(ui, tr("Edit"), "") {
                            *act = Some(Act::Edit(i, m.text.clone()));
                            ui.close();
                        }
                        if crate::right::menu_item(ui, if i == 0 { tr("Delete thread") } else { tr("Delete") }, "") {
                            *act = Some(Act::Run("comment.delete", message_params(id, i)));
                            ui.close();
                        }
                    });
                }
                let menu = Id::unique(("comment_msg_react_menu", id, i));
                let react = hover_button(
                    ui,
                    Id::unique(("comment_msg_react", id, i)),
                    Icon::Reaction,
                    hovered || egui::Popup::is_id_open(&ctx, menu),
                    tr("Add reaction"),
                );
                crate::right::menu_popup(&react).id(menu).show(|ui| {
                    ui.horizontal(|ui| {
                        for e in REACTIONS {
                            let on = m.reacted(e, cx.me);
                            let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                            if on || resp.hovered() {
                                ui.painter().rect_filled(r, CornerRadius::same(6), if on { t.selected_row } else { t.hover });
                            }
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, e, regular(16.0), t.text);
                            if resp.clicked() {
                                let mut p = message_params(id, i);
                                p["emoji"] = json!(e);
                                *act = Some(Act::Run("comment.react", p));
                                ui.close();
                            }
                        }
                    });
                });
            });
        });
        match editing.as_mut().filter(|e| e.0 == id && e.1 == i) {
            Some((_, _, text)) => {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.vertical(|ui| {
                        if composer(ui, Id::unique(("comment_edit", id, i)), text, tr("Edit comment"), cx.people, cx.me, pick) {
                            let mut p = message_params(id, i);
                            p["text"] = json!(text.trim());
                            *act = Some(Act::Run("comment.edit", p));
                        }
                        ui.label(egui::RichText::new(tr("Enter to save · Esc to cancel")).font(regular(10.5)).color(t.text_tertiary));
                    });
                });
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    let mut names: Vec<&str> = cx.people.iter().map(String::as_str).collect();
                    names.push(cx.me);
                    ui.add(egui::Label::new(rich_text(&m.text, &names, t.text, t.accent_text, ui.available_width())).wrap());
                });
            }
        }
        if !m.reactions.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(indent);
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for r in &m.reactions {
                    let on = r.users.iter().any(|u| u == cx.me);
                    // The emoji a size up from its count.
                    let mut job = LayoutJob::default();
                    job.append(
                        &r.emoji,
                        0.0,
                        TextFormat { font_id: regular(14.0), color: t.text, valign: egui::Align::Center, ..Default::default() },
                    );
                    job.append(
                        &r.users.len().to_string(),
                        4.0,
                        TextFormat { font_id: regular(11.5), color: t.text, valign: egui::Align::Center, ..Default::default() },
                    );
                    let galley = ui.painter().layout_job(job);
                    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(14.0, 6.0), Sense::click());
                    let cr = CornerRadius::same((rect.height() / 2.0) as u8);
                    ui.painter().rect_filled(
                        rect,
                        cr,
                        if on {
                            t.selected_row
                        } else if resp.hovered() {
                            t.hover
                        } else {
                            t.field
                        },
                    );
                    if on {
                        ui.painter().rect_stroke(rect, cr, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
                    }
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
                    if resp.on_hover_text(r.users.join(", ")).clicked() {
                        let mut p = message_params(id, i);
                        p["emoji"] = json!(r.emoji);
                        *act = Some(Act::Run("comment.react", p));
                    }
                }
            });
        }
        ui.add_space(8.0);
    });
    ctx.data_mut(|d| d.insert_temp(hover_key, block.response.contains_pointer()));
}

/// The popover's place next to a pin's spot, inside `bounds`.
fn popover_pos(anchor: Pos2, bounds: Rect) -> Pos2 {
    let mut p = anchor + vec2(PIN + 8.0, -PIN);
    if p.x + POPOVER_W > bounds.max.x - 8.0 {
        p.x = anchor.x - POPOVER_W - 8.0;
    }
    p.y = p.y.clamp(bounds.min.y + 8.0, (bounds.max.y - 200.0).max(bounds.min.y + 8.0));
    p
}

fn popover_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.panel)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(12))
        .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(40) })
        .inner_margin(egui::Margin::same(12))
}

/// Escape: ends an edit, else closes the open thread or the draft (an open @mention picker takes
/// it first). True when they closed.
fn escape(app: &mut FigApp, ctx: &egui::Context) -> bool {
    let cu = &mut app.ui.comments;
    if (cu.open.is_none() && cu.draft.is_none()) || cu.mention.active() || !ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return false;
    }
    if cu.editing.take().is_some() {
        return false;
    }
    cu.close();
    true
}

/// The thread popover and the draft composer, floating next to their pins on the canvas.
pub fn popovers(app: &mut FigApp, ctx: &egui::Context) {
    if escape(app, ctx) {
        return;
    }
    let canvas = app.canvas.rect;
    if let Some((x, y, mut text)) = app.ui.comments.draft.clone() {
        let t = Tokens::get(ctx);
        let people = app.session.doc().map(|d| d.comment_people()).unwrap_or_default();
        let me = me(app);
        let mut pick = std::mem::take(&mut app.ui.comments.mention);
        let mut post = false;
        egui::Area::new(Id::unique("comment_draft")).order(egui::Order::Foreground).fixed_pos(popover_pos(app.to_screen(x, y), canvas)).show(
            ctx,
            |ui| {
                popover_frame(&t).show(ui, |ui| {
                    ui.set_width(POPOVER_W - 24.0);
                    post = composer(ui, Id::unique("comment_draft_text"), &mut text, tr("Add a comment"), &people, &me, &mut pick);
                });
            },
        );
        app.ui.comments.mention = pick;
        if post {
            let r = app.run("comment.add", json!({"x": x, "y": y, "text": text.trim(), "anchor": true}));
            app.ui.comments.draft = None;
            if let Some(id) = r.and_then(|v| v.get("id").and_then(Value::as_u64)) {
                app.ui.comments.show(id);
            }
        } else if let Some(d) = &mut app.ui.comments.draft {
            d.2 = text;
        }
        return;
    }
    let Some(open) = app.ui.comments.open else { return };
    let Some(pin) = pins(app).into_iter().find(|p| p.id == open) else {
        app.ui.comments.close();
        return;
    };
    thread(app, ctx, open, pin.at, canvas);
}

/// A thread's popover next to the spot `anchor` (screen), kept inside `bounds`: its messages and a
/// reply field. Showing it reads the thread.
pub fn thread(app: &mut FigApp, ctx: &egui::Context, id: u64, anchor: Pos2, bounds: Rect) {
    let t = Tokens::get(ctx);
    let Ok(ds) = app.session.ds() else { return };
    let previewing = ds.preview.is_some();
    let Some(c) = ds.doc.comment(id).cloned() else {
        app.ui.comments.close();
        return;
    };
    let people = ds.doc.comment_people();
    let me = me(app);
    if c.is_unread(&me) && !previewing {
        app.run("comment.markRead", json!({"id": id}));
    }
    let mut reply = std::mem::take(&mut app.ui.comments.reply);
    let mut editing = app.ui.comments.editing.take().filter(|e| e.0 == id && c.message(e.1).is_some());
    let mut pick = std::mem::take(&mut app.ui.comments.mention);
    let mut act: Option<Act> = None;
    let cx = ThreadCtx { c: &c, me: &me, people: &people };
    egui::Area::new(Id::unique("comment_thread")).order(egui::Order::Foreground).fixed_pos(popover_pos(anchor, bounds)).show(ctx, |ui| {
        popover_frame(&t).show(ui, |ui| {
            ui.set_width(POPOVER_W - 24.0);
            // Header: resolve, more, close.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr("Comment")).font(semibold(12.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, Icon::Close, 24.0, false, tr("Close")).clicked() {
                        act = Some(Act::Close);
                    }
                    let more = icons::button(ui, Icon::Dots, 24.0, false, tr("More"));
                    crate::right::menu_popup(&more).show(|ui| {
                        if crate::right::menu_item(ui, tr("Mark as unread"), "") {
                            act = Some(Act::MarkUnread);
                            ui.close();
                        }
                        if crate::right::menu_item(ui, tr("Copy link"), "") {
                            act = Some(Act::CopyLink);
                            ui.close();
                        }
                        if crate::right::menu_item(ui, tr("Copy for AI"), "") {
                            act = Some(Act::CopyForAi);
                            ui.close();
                        }
                        if crate::right::menu_item(ui, if c.resolved { tr("Mark as unresolved") } else { tr("Mark as resolved") }, "") {
                            act = Some(Act::Run("comment.resolve", json!({"id": id, "resolved": !c.resolved})));
                            ui.close();
                        }
                        if c.first.author == me && crate::right::menu_item(ui, tr("Delete thread"), "") {
                            act = Some(Act::Run("comment.delete", json!({"id": id})));
                            ui.close();
                        }
                    });
                    let tip = if c.resolved { tr("Unresolve") } else { tr("Resolve") };
                    if icons::button(ui, Icon::Check, 24.0, c.resolved, tip).clicked() {
                        act = Some(Act::Run("comment.resolve", json!({"id": id, "resolved": !c.resolved})));
                    }
                });
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, m) in c.messages().enumerate() {
                    message(ui, &cx, i, m, &mut editing, &mut pick, &mut act);
                }
            });
            if composer(ui, reply_id(id), &mut reply, tr("Reply"), &people, &me, &mut pick) {
                act = Some(Act::Run("comment.reply", json!({"id": id, "text": reply.trim()})));
            }
        });
    });
    app.ui.comments.reply = reply;
    app.ui.comments.editing = editing;
    app.ui.comments.mention = pick;
    match act {
        Some(Act::Close) => app.ui.comments.close(),
        Some(Act::Edit(i, text)) => {
            app.ui.comments.editing = Some((id, i, text));
            app.ui.comments.mention = MentionPick::default();
        }
        Some(Act::CopyLink) => {
            if let Some(link) = app.run("comment.link", json!({"id": id})).and_then(|v| v.get("link").and_then(Value::as_str).map(str::to_string)) {
                app.clipboard_set_text(&link);
                app.toast(tr("Link copied"));
            }
        }
        Some(Act::CopyForAi) => copy_for_ai(app, Some(id)),
        // It closes: an open thread reads itself again.
        Some(Act::MarkUnread) => {
            if app.run("comment.markRead", json!({"id": id, "read": false})).is_some() {
                app.ui.comments.close();
            }
        }
        Some(Act::Run(cmd, p)) => {
            let thread_deleted = cmd == "comment.delete" && p.get("reply").is_none();
            if app.run(cmd, p).is_some() {
                match cmd {
                    "comment.reply" => app.ui.comments.reply.clear(),
                    "comment.edit" => {
                        app.ui.comments.editing = None;
                        app.ui.comments.mention = MentionPick::default();
                    }
                    _ if thread_deleted => app.ui.comments.close(),
                    _ => {}
                }
            }
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------------------------
// The Comments panel and keyboard navigation.

/// `comment.next` params for the panel's list (every page, its filter, sort and search).
fn panel_params(cu: &CommentUi, dir: i32) -> Value {
    json!({"id": cu.open, "dir": dir, "page": "all", "filter": cu.filter.id(), "sort": cu.sort.id(), "query": cu.search})
}

/// Open the next (`dir` 1) or previous (-1) thread in the panel's list, bring it into view and
/// put the keyboard in its reply field (where the arrows go on stepping while it's empty).
pub fn step(app: &mut FigApp, ctx: &egui::Context, dir: i32) -> Option<Value> {
    let p = panel_params(&app.ui.comments, dir);
    let r = app.session.execute("comment.next", &p).ok();
    app.handle_ui_requests();
    if r.is_some()
        && let Some(open) = app.ui.comments.open
    {
        app.ui.comments.scroll_to_open = true;
        ctx.memory_mut(|m| m.request_focus(reply_id(open)));
    }
    r
}

/// Up and Down step through the panel's threads while the Comment tool is active and nothing is
/// being typed (an empty reply field doesn't count).
pub fn keys(app: &mut FigApp, ctx: &egui::Context) {
    let cu = &app.ui.comments;
    if app.session.tool.tool != ToolId::Comment || cu.draft.is_some() || cu.editing.is_some() || cu.mention.active() {
        return;
    }
    let free = match ctx.memory(|m| m.focused()).filter(|_| crate::a11y::field_focused(ctx)) {
        None => true,
        Some(f) => cu.open.is_some_and(|o| f == reply_id(o)) && cu.reply.is_empty(),
    };
    if !free {
        return;
    }
    for (key, dir) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
            step(app, ctx, dir);
        }
    }
}

/// Copy the panel's current list (or one thread) as a task brief for an AI agent.
pub(crate) fn copy_for_ai(app: &mut FigApp, id: Option<u64>) {
    let params = match id {
        Some(id) => json!({"ids": [id]}),
        None => {
            let cu = &app.ui.comments;
            json!({"page": "all", "filter": cu.filter.id(), "sort": cu.sort.id(), "query": cu.search})
        }
    };
    let Some(r) = app.run("comment.prompt", params) else { return };
    if r.get("count").and_then(Value::as_u64).unwrap_or(0) == 0 {
        app.toast(tr("No comments to copy"));
        return;
    }
    if let Some(text) = r.get("text").and_then(Value::as_str) {
        app.clipboard_set_text(text);
        app.toast(tr("Copied comments for AI"));
    }
}

/// The Comments panel (right side, Comment tool).
pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Ok(ds) = app.session.ds() else { return };
    let page = ds.page;
    let doc = ds.doc.clone();
    let me = me(app);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(egui::RichText::new(tr("Comments")).font(semibold(12.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let menu = icons::button(ui, Icon::Adjust, 24.0, app.ui.comments.filter != Filter::Open, tr("Filter and sort"));
            crate::right::menu_popup(&menu).width(200.0).show(|ui| {
                for f in Filter::ALL {
                    if crate::right::menu_check(
                        ui,
                        app.ui.comments.filter == f,
                        if f == Filter::Open { tr("Open|comments") } else { tr(f.label()) },
                        "",
                    ) {
                        app.ui.comments.filter = f;
                        ui.close();
                    }
                }
                ui.separator();
                for s in Sort::ALL {
                    if crate::right::menu_check(ui, app.ui.comments.sort == s, tr(s.label()), "") {
                        app.ui.comments.sort = s;
                        ui.close();
                    }
                }
                ui.separator();
                if crate::right::menu_item(ui, tr("Mark all as read"), "") {
                    app.run("comment.markRead", json!({}));
                    ui.close();
                }
            });
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let w = ui.available_width() - 16.0;
        if let Some(s) = crate::widgets::text_field(ui, Id::unique("comment_search"), &app.ui.comments.search, w, tr("Search comments")) {
            app.ui.comments.search = s;
        }
    });
    // Another filter than the usual one says so, with a way back.
    if app.ui.comments.filter != Filter::Open {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(egui::RichText::new(tr(app.ui.comments.filter.label())).font(regular(11.0)).color(t.text_secondary));
            ui.add_space(4.0);
            if ui.add(egui::Label::new(egui::RichText::new(tr("Show open")).font(regular(11.0)).color(t.accent_text)).sense(Sense::click())).clicked()
            {
                app.ui.comments.filter = Filter::Open;
            }
        });
    }
    ui.add_space(8.0);
    let search = app.ui.comments.search.clone();
    let q = Query { page: None, frame: None, filter: app.ui.comments.filter, sort: app.ui.comments.sort, text: &search };
    let list = cc::query(&doc, &q, &me);
    if list.is_empty() {
        let (title, sub) = if doc.comments.iter().any(|c| doc.pages.contains(&c.page)) {
            (tr("No matching comments"), tr("Try another search or filter."))
        } else {
            (tr("No comments yet"), tr("Click anywhere on the canvas to leave one."))
        };
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(title).font(semibold(12.0)).color(t.text_secondary));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(sub).font(regular(11.0)).color(t.text_tertiary));
        });
        return;
    }
    let by_page = app.ui.comments.sort == Sort::Page;
    let scroll = std::mem::take(&mut app.ui.comments.scroll_to_open);
    let mut go: Option<u64> = None;
    let mut last_page: Option<NodeId> = None;
    for c in list {
        if by_page && last_page != Some(c.page) {
            last_page = Some(c.page);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let name = doc.node(c.page).map(|n| n.name.clone()).unwrap_or_default();
                ui.label(egui::RichText::new(name).font(semibold(11.0)).color(t.text_secondary));
            });
            ui.add_space(2.0);
        }
        let w = ui.available_width();
        let body = ui.painter().layout(c.first.text.clone(), regular(12.0), t.text, w - 64.0);
        let lines = body.rows.len().clamp(1, 3) as f32;
        let (r, resp) = ui.allocate_exact_size(vec2(w, 52.0 + 16.0 * lines), Sense::click());
        if app.ui.comments.open == Some(c.id) {
            ui.painter().rect_filled(r, CornerRadius::ZERO, t.selected_row);
            if scroll {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
        }
        let unread = c.is_unread(&me);
        if unread {
            ui.painter().circle_filled(pos2(r.min.x + 8.0, r.min.y + 22.0), 3.5, ACCENT);
        }
        avatar(ui.painter(), pos2(r.min.x + 28.0, r.min.y + 22.0), 12.0, &c.first.author);
        let name_font = if unread { semibold(12.0) } else { regular(12.0) };
        ui.painter().text(pos2(r.min.x + 48.0, r.min.y + 22.0), Align2::LEFT_CENTER, &c.first.author, name_font, t.text);
        ui.painter().text(pos2(r.max.x - 16.0, r.min.y + 22.0), Align2::RIGHT_CENTER, ago(c.last_activity()), regular(11.0), t.text_tertiary);
        let clip = Rect::from_min_size(pos2(r.min.x + 48.0, r.min.y + 38.0), vec2(w - 64.0, 16.0 * lines));
        ui.painter().with_clip_rect(clip).galley(clip.min, body, t.text);
        // The thread's frame and page, replies, state.
        let mut meta = Vec::new();
        if let Some(f) = cc::frame_of(&doc, c).and_then(|f| doc.node(f)) {
            meta.push(f.name.clone());
        }
        if !by_page && c.page != page {
            meta.push(doc.node(c.page).map(|n| n.name.clone()).unwrap_or_default());
        }
        if !c.replies.is_empty() {
            meta.push(trn("{n} replies", c.replies.len(), &[]));
        }
        if c.resolved {
            meta.push(tr("Resolved").into());
        }
        if !meta.is_empty() {
            let galley = ui.painter().layout_no_wrap(meta.join(" · "), regular(11.0), t.text_secondary);
            let at = pos2(r.min.x + 48.0, r.max.y - 12.0 - galley.size().y / 2.0);
            ui.painter().with_clip_rect(Rect::from_min_max(pos2(at.x, r.min.y), pos2(r.max.x - 16.0, r.max.y))).galley(at, galley, t.text_secondary);
        }
        ui.painter().line_segment([pos2(r.min.x + 16.0, r.max.y), pos2(r.max.x, r.max.y)], Stroke::new(1.0, t.border));
        if resp.clicked() {
            go = Some(c.id);
        }
    }
    if let Some(id) = go {
        app.run("comment.open", json!({"id": id, "view": "center"}));
    }
}

// ---------------------------------------------------------------------------------------------
// The presenter.

/// Pins on the presented `frame` (placed by the player's `place`, inside `clip`) while the
/// presenter shows comments, and the open thread's popover (read and reply).
pub fn presenter(app: &mut FigApp, ctx: &egui::Context, frame: NodeId, place: crate::present::Place, clip: Rect) {
    if escape(app, ctx) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let Some(fb) = doc.world_bounds(frame) else { return };
    let me = me(app);
    let open = app.ui.comments.open;
    // The frame as designed maps onto the frame as presented.
    let to_screen = |(x, y): (f64, f64)| place.pt(place.page_min.0 + x - fb.x0, place.page_min.1 + y - fb.y0);
    let q = Query { frame: Some(frame), filter: app.ui.comments.filter, ..Query::default() };
    let mut list = cc::query(&doc, &q, &me);
    list.reverse();
    if let Some(c) = open.and_then(|o| doc.comment(o)).filter(|c| cc::frame_of(&doc, c) == Some(frame) && !list.iter().any(|x| x.id == c.id)) {
        list.push(c);
    }
    let pins: Vec<Pin> = list.iter().map(|c| Pin::of(c, &me, to_screen(c.position(&doc)))).filter(|p| clip.contains(p.at)).collect();
    let mut clicked = None;
    for p in &pins {
        let r = pin_rect(p.at);
        egui::Area::new(Id::unique(("present_comment_pin", p.id))).order(egui::Order::Foreground).fixed_pos(r.min).show(ctx, |ui| {
            let (rr, resp) = ui.allocate_exact_size(r.size(), Sense::click());
            paint_pin(ui.painter(), rr, p, open == Some(p.id) || resp.hovered());
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(p.id);
            }
        });
    }
    if let Some(id) = clicked {
        toggle(app, id);
    }
    match app.ui.comments.open.and_then(|o| pins.iter().find(|p| p.id == o)) {
        Some(p) => thread(app, ctx, p.id, p.at, ctx.content_rect()),
        // A thread on another frame (or none) closes.
        None => app.ui.comments.close(),
    }
}
