//! Live sessions in the window: other people's cursors (smoothly following their pointer, with a
//! name tag) and selections on the canvas (their edits arrive through the engine's sync), their avatars at the top of the right panel (click one
//! to follow their view), the Share popover (share the file online or on the local network and
//! show its join code; join with a code; the server, file and access of a session you joined; a
//! published file's links), the join field (one component: the popover, the home screen and the
//! Join dialog), and the Publish to server dialog. Everything acts through the `collab.*` commands; the state is the engine's:
//! sessions are per document, and a window shows the session of the file in front
//! (`Session::live`), so another tab shows its own session or the way to start one. The network
//! wakes the UI when something arrives, and cursors only ask for frames while they glide, so an
//! idle session draws nothing.
//!
//! Cursor chat: `/` opens a bubble at your pointer to type in; others see it next to your cursor
//! and it fades a few seconds after the last change (frames are asked for only until then).
//! Spotlight: your avatar's menu asks everyone to follow you; a banner says so and stops it.

use std::collections::BTreeMap;

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_engine::live::{Access, PEOPLE, Role, SessionId, Share, Signal, Status, Via, initials, parse_color, presence_page};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::{self, Tip};
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::right::{menu_item, menu_item_with, menu_popup};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets;

/// In the browser: what joining needs (a page can't host, and https pages can't reach `ws://`).
const BROWSER_NOTE: &str = tr_noop(
    "In the browser you can join sessions the desktop app hosts and files on a FigCraft server. A page opened over https reaches other computers only through wss:// links (a server behind TLS).",
);

/// How undo works in a session (per person).
const UNDO_NOTE: &str = tr_noop("Undo takes back only your own changes; what others changed since stays.");

/// What a view link means for you.
const VIEW_NOTE: &str = tr_noop("View only: you see every change as it happens but can't edit. Leave to keep a copy you can edit.");

/// What a published file's links are.
const PUBLISHED_NOTE: &str = tr_noop(
    "Anyone with the edit link can change the file; the view link only lets people look. The links carry their secrets: share them like passwords. `figcraft-server links` makes more and revokes them.",
);

/// What the owner token is for (shown once, when a file is published without the server key).
const OWNER_NOTE: &str = tr_noop(
    "This computer remembers the owner token, so the file can be managed from the home screen's Online list. Copy it to manage the file from somewhere else: it is a password for this file and won't be shown again.",
);

/// What a server open to the public can see.
const PLAIN_NOTE: &str = tr_noop("Files aren't end-to-end encrypted: the server's operator can read them.");

/// How fast a remote cursor closes in on where it was last reported (per second).
const GLIDE: f64 = 20.0;
/// Most of one person's selection outlined.
const MAX_OUTLINES: usize = 500;
/// A cursor chat message shows this long after its last change, then fades out over
/// `CHAT_FADE` (seconds); your chat box closes when you haven't typed for both.
const CHAT_HOLD: f64 = 5.0;
const CHAT_FADE: f64 = 1.0;
/// Widest a chat bubble's text gets before it wraps.
const CHAT_WIDTH: f32 = 220.0;
/// A speech bubble's corners: sharp at the pointer.
const BUBBLE: CornerRadius = CornerRadius { nw: 2, ne: 14, sw: 14, se: 14 };

/// Your cursor chat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatBox {
    pub text: String,
    /// When you last typed (UI clock; `None` until the box first shows).
    typed: Option<f64>,
    /// Typing; else sent, showing until it fades.
    pub editing: bool,
}

/// UI-only live session state.
#[derive(Default)]
pub struct LiveUi {
    /// The session this window showed at its last frame (0: none): what belongs to one session
    /// (cursors, chats, the pointer handed over, your chat box, a report) goes when another shows.
    session: SessionId,
    /// Where each remote cursor is drawn (page coordinates), gliding to where it was reported.
    shown: BTreeMap<u32, (f64, f64)>,
    /// The UI clock at the last cursor frame.
    last: f64,
    /// The pointer position last handed to the session (page coordinates).
    seen: Option<(f64, f64)>,
    /// The Join dialog's link field (open when `Some`).
    pub join: Option<String>,
    /// Put the keyboard in the Join dialog's field on its next frame.
    focus_join: bool,
    /// Share on the local network (else online, through the server in Preferences).
    pub local: bool,
    /// The Share popover's Advanced part shows; what it sets for a local network share: only
    /// this computer can join, and the port (empty: the usual one).
    pub advanced: bool,
    pub this_computer: bool,
    pub port: String,
    /// What is typed in the join field of the Share popover and the home screen.
    pub code: String,
    /// The join started from the home screen: what it is doing shows under the field, and its
    /// file's tab shows when it opens.
    pub home_join: Option<SessionId>,
    /// Remove from server… was clicked: its confirmation shows.
    pub confirm_remove: bool,
    /// What the server for online sharing said about its hosting (`collab.serverInfo`), and the
    /// address asked.
    pub server_info: Option<(String, Value)>,
    /// The Publish to server dialog's fields (open when `Some`).
    pub publish: Option<PublishForm>,
    /// Put the keyboard in the Publish dialog's first field on its next frame.
    focus_publish: bool,
    /// The last server address published to (kept for this run only: it carries the server key).
    last_server: String,
    /// The session of a file published from the dialog without the key: its owner token shows
    /// once it arrives (whichever tab is in front then).
    owner: Option<SessionId>,
    /// The report being written about the file you're in (open when `Some`), and putting the
    /// keyboard in its field on its next frame.
    pub report: Option<String>,
    focus_report: bool,
    /// Others' chat messages as last seen, with when they changed (UI clock): they fade from then.
    chat_seen: BTreeMap<u32, (String, f64)>,
    /// Your cursor chat (open when `Some`).
    pub chat: Option<ChatBox>,
}

/// What the Publish to server dialog asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PublishForm {
    /// The server's address with its key (`ws://host:47900/?key=…`).
    pub server: String,
    /// The file's name on the server.
    pub name: String,
    /// What the server said about its hosting (`collab.serverInfo`), and the address asked.
    pub info: Option<(String, Value)>,
}

impl LiveUi {
    /// The remote cursors drawn last frame (`ui.inspect`).
    pub fn shown(&self) -> Vec<u32> {
        self.shown.keys().copied().collect()
    }
}

/// A person's colour (`#RRGGBB` from the session), or the accent for anything else.
pub fn color(s: &str) -> Color32 {
    parse_color(s).map_or(ACCENT, |(r, g, b)| Color32::from_rgb(r, g, b))
}

/// The palette colour `i` (comment authors use it too).
pub fn people(i: usize) -> Color32 {
    PEOPLE.get(i % PEOPLE.len()).map_or(ACCENT, |c| color(c))
}

/// Text that reads on `fill`.
pub fn text_on(fill: Color32) -> Color32 {
    let l = 0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b());
    if l > 170.0 { Color32::from_rgb(0x1E, 0x1E, 0x1E) } else { Color32::WHITE }
}

/// An avatar: the initials in a circle of the person's colour.
pub fn avatar(painter: &egui::Painter, center: Pos2, r: f32, name: &str, fill: Color32) {
    painter.circle_filled(center, r, fill);
    painter.text(center, Align2::CENTER_CENTER, initials(name), semibold(r * 0.8), text_on(fill));
}

/// FigCraft's remote pointer: a notched dart pointing up-left from `tip`, filled with the
/// person's colour and edged in white so it shows on any canvas.
pub fn pointer(painter: &egui::Painter, tip: Pos2, fill: Color32) {
    let at = |x: f32, y: f32| tip + vec2(x, y);
    let (left, notch, right) = (at(3.5, 15.5), at(7.5, 8.5), at(15.5, 4.0));
    painter.add(Shape::convex_polygon(vec![tip, left, notch], fill, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![tip, notch, right], fill, Stroke::NONE));
    painter.add(Shape::closed_line(vec![tip, left, notch, right], Stroke::new(1.5, Color32::WHITE)));
}

/// A name tag in a person's colour, placed by `align` at `anchor` (next to their pointer, above
/// their text caret).
pub(crate) fn name_tag(painter: &egui::Painter, anchor: Pos2, align: Align2, name: &str, fill: Color32) {
    let galley = painter.layout_no_wrap(name.to_string(), semibold(11.0), text_on(fill));
    let r = align.anchor_size(anchor, galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(r, CornerRadius::same(4), fill);
    painter.galley(r.min + vec2(6.0, 3.0), galley, text_on(fill));
}

/// How opaque a chat message `age` seconds after its last change is (0: gone); asks for the frame
/// where it next looks different.
fn chat_fade(ctx: &egui::Context, age: f64, reduce: bool) -> f32 {
    let after = |s: f64| ctx.request_repaint_after(std::time::Duration::from_secs_f64(s.max(0.0)));
    if age < CHAT_HOLD {
        after(CHAT_HOLD - age);
        1.0
    } else if age >= CHAT_HOLD + CHAT_FADE {
        0.0
    } else if reduce {
        after(CHAT_HOLD + CHAT_FADE - age);
        1.0
    } else {
        ctx.request_repaint();
        (1.0 - (age - CHAT_HOLD) / CHAT_FADE) as f32
    }
}

/// A cursor chat bubble next to the pointer at `tip`: the person's name (when given) over their
/// message, in their colour, `alpha` opaque.
fn chat_bubble(painter: &egui::Painter, tip: Pos2, name: Option<&str>, text: &str, fill: Color32, alpha: f32) {
    let fg = text_on(fill).gamma_multiply(alpha);
    let body = painter.layout(text.to_string(), regular(12.0), fg, CHAT_WIDTH);
    let head = name.map(|n| painter.layout_no_wrap(n.to_string(), semibold(10.0), fg.gamma_multiply(0.8)));
    let head_h = head.as_ref().map_or(0.0, |g| g.size().y + 2.0);
    let w = body.size().x.max(head.as_ref().map_or(0.0, |g| g.size().x));
    let r = Rect::from_min_size(tip + vec2(12.0, 16.0), vec2(w + 20.0, head_h + body.size().y + 12.0));
    painter.rect_filled(r, BUBBLE, fill.gamma_multiply(alpha));
    let x = r.min.x + 10.0;
    if let Some(g) = head {
        painter.galley(pos2(x, r.min.y + 6.0), g, fg);
    }
    painter.galley(pos2(x, r.min.y + 6.0 + head_h), body, fg);
}

/// Hand the pointer's page position to the session when it moved (or the view under it did).
pub fn track_pointer(app: &mut FigApp, ctx: &egui::Context) {
    let (id, live) = (app.session.live().id, app.session.live().status == Status::Live);
    let ui = &mut app.ui.live;
    // Another tab came to the front, or this file's session started or ended.
    if std::mem::replace(&mut ui.session, id) != id {
        ui.shown.clear();
        ui.chat_seen.clear();
        ui.seen = None;
        ui.chat = None;
        ui.report = None;
    }
    if !live {
        ui.seen = None;
        return;
    }
    let at = ctx.input(|i| i.pointer.hover_pos()).filter(|p| app.canvas.rect.contains(*p)).map(|p| app.to_page(p)).map(|p| (p.x, p.y));
    if at != app.ui.live.seen {
        app.ui.live.seen = at;
        if let Some(l) = app.session.live_mut() {
            l.cursor = at;
        }
    }
}

/// Others' selections and cursors on the canvas, and the frame around it while following.
pub fn draw(app: &mut FigApp, painter: &egui::Painter, ctx: &egui::Context) {
    let live = app.session.live();
    if live.peers.is_empty() {
        app.ui.live.shown.clear();
        app.ui.live.chat_seen.clear();
        return;
    }
    let now = ctx.input(|i| i.time);
    // A chat message fades from its last change.
    let seen = &mut app.ui.live.chat_seen;
    seen.retain(|id, _| live.chats.contains_key(id));
    for (id, text) in &live.chats {
        if seen.get(id).is_none_or(|(t, _)| t != text) {
            seen.insert(*id, (text.clone(), now));
        }
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, shared) = (ds.doc.clone(), ds.page, app.session.live_shared());
    let mut cursors = Vec::new();
    for (id, p) in &live.presence {
        let Some(peer) = live.peers.get(id) else { continue };
        let fill = color(&peer.color);
        // Selections only mean something on the same page of the same (synced) document.
        if shared
            && presence_page(p) == Some(page)
            && let Some(sel) = &p.selection
        {
            for n in sel.iter().filter(|n| doc.page_of(**n) == Some(page)).take(MAX_OUTLINES) {
                if let Some(q) = crate::canvas::node_quad(app, &doc, *n) {
                    painter.add(Shape::closed_line(q.to_vec(), Stroke::new(1.5, fill)));
                }
            }
        }
        // Cursors (unless View ▸ Multiplayer cursors is off): on your page of the session's
        // document; before its welcome arrives they show in page coordinates on whatever page
        // you look at.
        if let Some((pg, x, y)) = p.cursor
            && ds.view.multiplayer_cursors
            && (pg == page || !shared)
        {
            cursors.push((*id, peer.name.clone(), fill, (x, y)));
        }
    }
    if let Some(peer) = live.follow.and_then(|f| live.peers.get(&f)) {
        follow_frame(painter, app.canvas.rect, &peer.name, color(&peer.color));
    }
    let dt = (now - app.ui.live.last).clamp(0.0, 0.1);
    app.ui.live.last = now;
    let reduce = app.reduce_motion();
    let k = if reduce { 1.0 } else { 1.0 - (-dt * GLIDE).exp() };
    let zoom = ds.view.zoom.max(1e-9);
    let mut gliding = false;
    let mut shown = BTreeMap::new();
    for (id, name, fill, (x, y)) in cursors {
        let (mut sx, mut sy) = app.ui.live.shown.get(&id).copied().unwrap_or((x, y));
        sx += (x - sx) * k;
        sy += (y - sy) * k;
        // Within half a pixel: there.
        if ((x - sx) * zoom).hypot((y - sy) * zoom) < 0.5 {
            (sx, sy) = (x, y);
        } else {
            gliding = true;
        }
        shown.insert(id, (sx, sy));
        let tip = app.to_screen(sx, sy);
        pointer(painter, tip, fill);
        // Their chat message takes the name tag's place while it shows.
        let chat = app.ui.live.chat_seen.get(&id).map(|(text, at)| (text, chat_fade(ctx, now - at, reduce))).filter(|(_, a)| *a > 0.0);
        match chat {
            Some((text, alpha)) => chat_bubble(painter, tip, Some(&name), text, fill, alpha),
            None => name_tag(painter, tip + vec2(12.0, 16.0), Align2::LEFT_TOP, &name, fill),
        }
    }
    app.ui.live.shown = shown;
    if gliding {
        ctx.request_repaint();
    }
}

/// Open your cursor chat (`/`, `collab.cursorChat` without text), with your current message.
pub fn open_chat(app: &mut FigApp) {
    if app.ui.live.chat.as_ref().is_none_or(|c| !c.editing) {
        app.ui.live.chat = Some(ChatBox { text: app.session.live().chat.clone(), typed: None, editing: true });
    }
}

/// Your cursor chat at your pointer: a bubble to type in. What you type shows to everyone as you
/// type; Enter (or clicking away) sends it and closes the box, and the message fades; Escape takes
/// it back at once. The box closes by itself when you haven't typed for a while.
pub fn chat_box(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut chat) = app.ui.live.chat.take() else { return };
    if app.session.live().status != Status::Live {
        return;
    }
    let now = ctx.input(|i| i.time);
    let first = chat.typed.is_none();
    let typed = *chat.typed.get_or_insert(now);
    let live = app.session.live();
    let fill = live.info.as_ref().map_or(ACCENT, |i| color(&i.color));
    let tip = live.cursor.map_or(app.canvas.rect.center(), |(x, y)| app.to_screen(x, y));
    let reduce = app.reduce_motion();
    if !chat.editing {
        let alpha = chat_fade(ctx, now - typed, reduce);
        if alpha <= 0.0 || app.session.live().chat.is_empty() {
            let _ = app.session.live_chat("");
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("live_chat")));
        chat_bubble(&painter, tip, None, &chat.text, fill, alpha);
        app.ui.live.chat = Some(chat);
        return;
    }
    let idle = now - typed;
    if idle >= CHAT_HOLD + CHAT_FADE {
        let _ = app.session.live_chat("");
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(CHAT_HOLD + CHAT_FADE - idle));
    let fg = text_on(fill);
    let resp = egui::Area::new(egui::Id::new("live_chat_box"))
        .order(egui::Order::Foreground)
        .fixed_pos(tip + vec2(12.0, 16.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(fill)
                .corner_radius(BUBBLE)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    let w = ui.painter().layout_no_wrap(chat.text.clone(), regular(12.0), fg).size().x + 8.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut chat.text)
                            .id(egui::Id::new("live_chat_text"))
                            .frame(egui::Frame::NONE)
                            .font(regular(12.0))
                            .text_color(fg)
                            .hint_text(egui::RichText::new(tr("Say something")).color(fg.gamma_multiply(0.7)))
                            .char_limit(Signal::MAX_CHAT)
                            .desired_width(w.clamp(110.0, CHAT_WIDTH)),
                    )
                })
                .inner
        })
        .inner;
    a11y::text(&resp, tr("Cursor chat"), &chat.text);
    if first {
        resp.request_focus();
    }
    if resp.changed() {
        chat.typed = Some(now);
        if let Err(e) = app.session.live_chat(&chat.text) {
            app.toast(e.to_string());
            return;
        }
    }
    if resp.lost_focus() {
        // Escape takes it back (before the canvas sees the key: it doesn't deselect).
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) || chat.text.trim().is_empty() {
            let _ = app.session.live_chat("");
            return;
        }
        chat.editing = false;
        chat.typed = Some(now);
    }
    app.ui.live.chat = Some(chat);
}

/// While you spotlight, or someone does and you stopped following them: a banner on the canvas to
/// stop, or to follow again.
pub fn spotlight_banner(app: &mut FigApp, ctx: &egui::Context, canvas: Rect) {
    let live = app.session.live();
    let (text, button, cmd, params) = if live.spotlighting {
        (tr("You're spotlighting: everyone follows your view").to_string(), tr("Stop"), "collab.spotlight", json!({"on": false}))
    } else if let Some(p) = live.spotlight.filter(|p| live.follow != Some(*p)).and_then(|p| live.peers.get(&p)) {
        (trf("{name} is spotlighting", &[("name", &p.name)]), tr("Follow"), "collab.follow", json!({"client": p.client_id}))
    } else {
        return;
    };
    let t = Tokens::get(ctx);
    let clicked = widgets::canvas_banner(ctx, egui::Id::new("live_spotlight"), canvas, |ui| {
        ui.label(egui::RichText::new(text).font(regular(11.0)).color(t.text));
        widgets::button(ui, button, true, true).clicked()
    });
    if clicked {
        app.run(cmd, params);
    }
}

/// A frame around the canvas in the followed person's colour, with their name at the top.
fn follow_frame(painter: &egui::Painter, canvas: Rect, name: &str, fill: Color32) {
    painter.rect_stroke(canvas.shrink(1.0), CornerRadius::ZERO, Stroke::new(2.0, fill), egui::StrokeKind::Inside);
    let text = trf("Following {name} · {key} to stop", &[("name", &name), ("key", &crate::toolbar::pretty_shortcut("Escape"))]);
    let galley = painter.layout_no_wrap(text, semibold(11.0), text_on(fill));
    let r = Rect::from_center_size(pos2(canvas.center().x, canvas.min.y + 2.0 + (galley.size().y + 6.0) / 2.0), galley.size() + vec2(16.0, 6.0));
    painter.rect_filled(r, CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 }, fill);
    painter.galley(r.min + vec2(8.0, 3.0), galley, text_on(fill));
}

/// You and the others at the top of the right panel, within `area`: click someone to follow
/// them (again to stop); right-click them to remove them from the session (when you host it or
/// own the file); the chevron lists everyone.
pub fn people_bar(app: &mut FigApp, ui: &mut Ui, area: Rect) {
    let t = Tokens::get(ui.ctx());
    const R: f32 = 12.0;
    const STEP: f32 = 20.0;
    let cy = area.center().y;
    let live = app.session.live();
    let me_name = match live.info.as_ref() {
        Some(i) if !i.name.is_empty() => i.name.clone(),
        _ => app.session.prefs.user_name.clone(),
    };
    let me_fill = live.info.as_ref().map_or(t.success, |i| color(&i.color));
    let peers: Vec<(u32, String, Color32)> = live.peers.values().map(|p| (p.client_id, p.name.clone(), color(&p.color))).collect();
    let follow = live.follow;
    let (in_session, spotlighting) = (live.status == Status::Live, live.spotlighting);
    let can_remove = figcraft_engine::live::can_remove_people(&app.session);
    let mut remove = None;
    // Room for the avatars after yours and the chevron.
    let fit = (((area.width() - 2.0 * R - 18.0) / STEP).floor() as usize).max(1);
    let (show, more) = if peers.len() > fit { (fit - 1, peers.len() - (fit - 1)) } else { (peers.len(), 0) };
    let mut x = area.min.x + R + 2.0;
    let ring = |ui: &Ui, c: Pos2| ui.painter().circle_stroke(c, R + 1.0, Stroke::new(2.0, t.panel));
    avatar(ui.painter(), pos2(x, cy), R, &me_name, me_fill);
    // Your avatar's menu: spotlight.
    let mut spotlight = false;
    if in_session {
        let c = pos2(x, cy);
        let resp = ui.interact(Rect::from_center_size(c, vec2(2.0 * R, 2.0 * R)), ui.id().with("live_me"), Sense::click());
        a11y::menu_button(&resp, tr("You"), a11y::menu_open(&resp));
        if spotlighting || resp.hovered() {
            ui.painter().circle_stroke(c, R + 2.5, Stroke::new(2.0, me_fill));
        }
        menu_popup(&resp).width(200.0).show(|ui| {
            if menu_item(ui, if spotlighting { tr("Stop spotlighting") } else { tr("Spotlight me") }, "") {
                spotlight = true;
                ui.close();
            }
        });
    }
    let mut clicked = None;
    for (id, name, fill) in peers.iter().take(show) {
        x += STEP;
        let c = pos2(x, cy);
        let following = follow == Some(*id);
        let tip = if following { trf("Stop following {name}", &[("name", name)]) } else { trf("Follow {name}", &[("name", name)]) };
        let resp = ui.interact(Rect::from_center_size(c, vec2(2.0 * R, 2.0 * R)), ui.id().with(("live_avatar", id)), Sense::click()).tip(tip);
        ring(ui, c);
        avatar(ui.painter(), c, R, name, *fill);
        if following || resp.hovered() {
            ui.painter().circle_stroke(c, R + 2.5, Stroke::new(2.0, *fill));
        }
        if resp.clicked() {
            clicked = Some(*id);
        }
        if can_remove {
            crate::right::context_popup(&resp).show(|ui| {
                if menu_item(ui, &trf("Remove {name} from session", &[("name", name)]), "") {
                    remove = Some(*id);
                    ui.close();
                }
            });
        }
    }
    if more > 0 {
        x += STEP;
        let c = pos2(x, cy);
        ring(ui, c);
        ui.painter().circle_filled(c, R, t.pressed);
        ui.painter().text(c, Align2::CENTER_CENTER, format!("+{more}"), semibold(10.0), t.text);
    }
    let chev = Rect::from_center_size(pos2(x + R + 10.0, cy), vec2(16.0, 24.0));
    let cresp = ui.interact(chev, ui.id().with("live_people"), Sense::click());
    a11y::menu_button(&cresp, tr("People in this file"), a11y::menu_open(&cresp));
    if cresp.hovered() {
        ui.painter().rect_filled(chev, CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(chev.center(), vec2(10.0, 10.0)), Icon::Chevron, t.icon_secondary);
    menu_popup(&cresp).width(220.0).show(|ui| {
        let row = |ui: &mut Ui, label: &str, fill: Color32, check: bool| {
            menu_item_with(ui, label, if check { tr("Following") } else { "" }, 22.0, |p, r, _| {
                avatar(p, pos2(r.max.x - 15.0, r.center().y), 8.0, label, fill)
            })
        };
        row(ui, &trf("{name} (you)", &[("name", &me_name)]), me_fill, false);
        for (id, name, fill) in &peers {
            if row(ui, name, *fill, follow == Some(*id)) {
                clicked = Some(*id);
                ui.close();
            }
        }
        if peers.is_empty() {
            widgets::label(
                ui,
                if app.session.live().status.active() { tr("Nobody else is here yet") } else { tr("Not in a live session") },
                t.text_secondary,
            );
        }
    });
    if let Some(id) = clicked {
        let params = if follow == Some(id) { json!({}) } else { json!({"client": id}) };
        app.run("collab.follow", params);
    }
    if spotlight {
        app.run("collab.spotlight", json!({"on": !spotlighting}));
    }
    // A server is asked without the window waiting for it.
    if let Some(id) = remove {
        app.run_background(&ui.ctx().clone(), "collab.removePerson", json!({"client": id}), Box::new(|_, _, _| {}));
    }
}

/// The Share button's popover: the live session (start, invite link, stop or leave) and export.
pub fn share_popover(app: &mut FigApp, anchor: &egui::Response) {
    menu_popup(anchor).width(300.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.set_max_width(300.0);
        // Menus lay out justified and don't wrap: the session's notes wrap, left-aligned, within
        // the popover's width.
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add_space(4.0);
            let status = app.session.live().status.clone();
            let role = app.session.live().role();
            let others = app.session.live().peers.len();
            let published = app.session.live().published.is_some();
            let share = app.session.live().share.clone().filter(|_| status.active());
            let title = if status.active() { tr("Live session") } else { tr("Share this file") };
            ui.label(egui::RichText::new(title).font(semibold(12.0)));
            // Connecting with a tab already chosen and nobody's welcome yet: the upload.
            let publishing = status == Status::Connecting && role.is_none() && app.session.live().doc_uid.is_some();
            let (line, col) = match (&status, role) {
                (Status::Off, _) => (String::new(), t.text_secondary),
                // On the server and not shared at the moment: the file is its owner's alone.
                (Status::Live, _) if share.as_ref().is_some_and(|s| s.links.is_none()) => (tr("Not shared").to_string(), t.text_secondary),
                (Status::Live, _) if share.as_ref().is_some_and(|s| s.via == Via::Online) => {
                    (trf("Sharing online · {people}", &[("people", &people_count(others))]), t.success)
                }
                (Status::Live, _) if share.is_some() => (trf("Sharing on your network · {people}", &[("people", &people_count(others))]), t.success),
                (Status::Connecting, _) if share.is_some() => (tr("Starting to share…").to_string(), t.text_secondary),
                (Status::Live, Some(Role::Host)) => (trf("Live · {people}", &[("people", &people_count(others))]), t.success),
                (Status::Live, _) if published => (trf("Published · {people}", &[("people", &people_count(others))]), t.success),
                (Status::Connecting, _) if publishing => (tr("Publishing…").to_string(), t.text_secondary),
                (Status::Live, _) => {
                    let file = app.session.live().info.as_ref().map(|i| i.file_name.clone()).unwrap_or_default();
                    (if file.is_empty() { tr("Joined").to_string() } else { trf("Joined {file}", &[("file", &file)]) }, t.success)
                }
                (Status::Reconnecting { attempt }, _) => {
                    (trf("Connection lost, reconnecting (attempt {attempt})…", &[("attempt", attempt)]), t.text_secondary)
                }
                (Status::Failed { .. }, _) => (status.label(), t.danger),
                _ => (tr(&status.label()).to_string(), t.text_secondary),
            };
            if !line.is_empty() {
                widgets::label(ui, &line, col);
            }
            match (&status, role, &share) {
                (Status::Off | Status::Failed { .. }, ..) => start_ui(app, ui, &t),
                (Status::Live, _, Some(share)) if share.links.is_none() => private_ui(app, ui, &t, share),
                (_, _, Some(share)) => share_ui(app, ui, &t, share),
                (Status::Live, Some(Role::Host), _) => host_ui(app, ui),
                _ => guest_ui(app, ui, &t),
            }
        });
        ui.separator();
        if menu_item(ui, tr("Export…"), "") {
            app.ui.export_preview = true;
            ui.close();
        }
    });
}

fn people_count(others: usize) -> String {
    match others {
        0 => tr("nobody else here yet").into(),
        n => trn("{n} people joined", n, &[]),
    }
}

/// The host a server address names (`wss://share.example/` is `share.example`).
fn server_host(server: &str) -> &str {
    server.split_once("://").map_or(server, |(_, rest)| rest).trim_end_matches('/')
}

/// What a server says about its hosting (`collab.serverInfo`): who runs it, its terms, how long
/// files nobody opens are kept, and that it can read them.
fn hosting_lines(ui: &mut Ui, t: &Tokens, info: &Value) {
    host_line(ui, t, info["operator"].as_str().unwrap_or_default(), info["terms"].as_str().unwrap_or_default());
    if let Some(days) = info["idleDays"].as_f64() {
        let n = days.ceil().max(1.0) as usize;
        widgets::label(ui, &trn("Files nobody opens for {n} days are deleted.", n, &[]), t.text_secondary);
    }
    widgets::label(ui, tr(PLAIN_NOTE), t.text_secondary);
}

/// Sharing online: where the file goes, and on request what that server says about its hosting.
fn online_note(app: &mut FigApp, ui: &mut Ui, t: &Tokens, server: &str) {
    widgets::label(
        ui,
        &trf("People join from anywhere with a code. The file is kept on {server} while you share it.", &[("server", &server_host(server))]),
        t.text_secondary,
    );
    match app.ui.live.server_info.as_ref().filter(|(at, _)| at == server) {
        Some((_, info)) if info["open"] == true => hosting_lines(ui, t, info),
        Some(_) => widgets::label(ui, tr("This server takes files only with its key: see Advanced."), t.danger),
        None if app.session.waiting_for(SERVER_INFO) => widgets::label(ui, tr("Asking the server…"), t.text_secondary),
        None => {
            let about = egui::Link::new(egui::RichText::new(tr("About this server")).font(regular(11.0)));
            if ui.add(about).on_hover_text(tr("Who runs it, its terms and how long it keeps files")).clicked() {
                ask_server(app, &ui.ctx().clone(), server.to_string());
            }
        }
    }
}

/// The join field with its Join button, on one row: what is typed stays while the popover or the
/// home screen is closed. The session of the join that started, if one did.
pub fn join_row(app: &mut FigApp, ui: &mut Ui, width: f32) -> Option<SessionId> {
    let mut typed = std::mem::take(&mut app.ui.live.code);
    let joined = ui.horizontal(|ui| join_field(app, ui, &mut typed, egui::Id::new("live_join_code"), width, false)).inner;
    app.ui.live.code = typed;
    joined
}

/// The one way to join: a field for a join code or an invite link (pasted or typed; spaces,
/// dashes and case don't matter in a code) and a Join button; Enter joins too. The session of the
/// join that started (the field is cleared); what can't be joined says why and stays to be fixed.
pub fn join_field(app: &mut FigApp, ui: &mut Ui, typed: &mut String, id: egui::Id, width: f32, focus: bool) -> Option<SessionId> {
    let resp = widgets::text_input(ui, id, typed, width, tr("Code or invite link"));
    // It may hold a link with its secret: it isn't read out.
    a11y::text(&resp, tr("Join code or invite link"), "");
    if focus {
        resp.request_focus();
    }
    let filled = !typed.trim().is_empty();
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let go = widgets::button(ui, tr("Join"), true, filled).clicked() || enter;
    let joined = if go && filled { app.run("collab.join", json!({"code": typed.trim()})) } else { None };
    let session = joined.and_then(|j| j["session"].as_u64());
    if session.is_some() {
        typed.clear();
    }
    session
}

/// What join `session` is doing, under the field it was started from (the home screen): on its
/// way, or why it failed.
pub fn join_status(app: &FigApp, ui: &mut Ui, t: &Tokens, session: SessionId) {
    match app.session.live.get(session).map(|l| &l.status) {
        Some(Status::Connecting) => widgets::label(ui, tr("Joining…"), t.text_secondary),
        Some(Status::Failed { reason }) => widgets::label(ui, reason, t.danger),
        _ => {}
    }
}

/// Not in a session: your name, how to share (online or on the local network), start; or join
/// one with a code. Everything else is under Advanced.
fn start_ui(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        widgets::label(ui, tr("Your name"), t.text);
        let name = app.session.prefs.user_name.clone();
        if let Some(n) = widgets::text_field(ui, egui::Id::new("live_name"), &name, 160.0, tr("Your name")) {
            app.run("pref.userName", json!({"name": n}));
        }
    });
    if !app.session.live.can_host() {
        widgets::label(ui, tr("Sharing a file needs the desktop app; here you can join one."), t.text_secondary);
        widgets::label(ui, tr("Join with a code"), t.text);
        join_row(app, ui, JOIN_WIDTH);
        widgets::label(ui, tr(BROWSER_NOTE), t.text_secondary);
        return;
    }
    // Without a server in Preferences only the local network is left.
    let server = app.session.prefs.publish_server.trim().to_string();
    let local = app.ui.live.local || server.is_empty();
    if server.is_empty() {
        widgets::label(ui, tr("Sharing on the local network (online sharing is off: no server is set in Preferences)."), t.text_secondary);
    } else if let Some(i) = widgets::segmented_text(ui, &[tr("Online"), tr("Local network")], usize::from(local)) {
        app.ui.live.local = i == 1;
    }
    if local {
        widgets::label(ui, tr("People on your network join with a code. Nothing goes through a server."), t.text_secondary);
    } else {
        online_note(app, ui, t, &server);
    }
    if widgets::button(ui, tr("Start sharing"), true, true).clicked() {
        let mut params = json!({"via": if local { "local" } else { "online" }});
        if local && app.ui.live.this_computer {
            params["bind"] = json!("local");
        }
        if let (true, Ok(port)) = (local, app.ui.live.port.trim().parse::<u16>()) {
            params["port"] = json!(port);
        }
        app.run("collab.share", params);
    }
    join_and_advanced(app, ui, t, |app, ui, t| {
        if local {
            if widgets::checkbox(ui, app.ui.live.this_computer, tr("This computer only")) {
                app.ui.live.this_computer = !app.ui.live.this_computer;
            }
            ui.horizontal(|ui| {
                widgets::label(ui, tr("Port"), t.text);
                if let Some(port) = widgets::text_field_named(ui, egui::Id::new("live_port"), &app.ui.live.port, 70.0, tr("Port")) {
                    // Digits only; empty: the usual port, or any free one.
                    app.ui.live.port = port.trim().parse::<u16>().map(|p| p.to_string()).unwrap_or_default();
                }
            });
        } else {
            widgets::label(ui, tr("Another server, or one that needs its key. The usual server is set in Preferences."), t.text_secondary);
            if widgets::button(ui, tr("Publish to another server…"), false, true).clicked() {
                open_publish(app);
                ui.close();
            }
        }
    });
}

/// Under what starts a share: the join field, then the Advanced part (`advanced` draws it while
/// it is open).
fn join_and_advanced(app: &mut FigApp, ui: &mut Ui, t: &Tokens, advanced: impl FnOnce(&mut FigApp, &mut Ui, &Tokens)) {
    ui.separator();
    widgets::label(ui, tr("Join with a code"), t.text);
    join_row(app, ui, JOIN_WIDTH);
    if widgets::disclosure(ui, tr("Advanced"), app.ui.live.advanced) {
        app.ui.live.advanced = !app.ui.live.advanced;
    }
    if app.ui.live.advanced {
        advanced(app, ui, t);
    }
}

/// The file is on a server and not shared now: it is its owner's alone. Start sharing gives
/// others new links and codes for this same file; taking it off the server is a separate,
/// confirmed step under Advanced.
fn private_ui(app: &mut FigApp, ui: &mut Ui, t: &Tokens, share: &Share) {
    let host = server_host(share.server.split('?').next().unwrap_or_default()).to_string();
    widgets::label(
        ui,
        &trf("This file is on {server}, open only to you. Share it to let others in with a new code.", &[("server", &host)]),
        t.text_secondary,
    );
    if let Some(h) = app.session.live().hosting.clone() {
        host_line(ui, t, &h.operator, &h.terms);
        expiry_line(ui, t, h.expires);
    }
    if app.session.waiting_for(SHARE) {
        widgets::label(ui, tr("Starting to share…"), t.text_secondary);
    } else if widgets::button(ui, tr("Start sharing"), true, true).clicked() {
        app.run_background(&ui.ctx().clone(), SHARE, json!({"via": "online"}), Box::new(|_, _, _| {}));
    }
    // Whoever published it without the server's key holds its owner token (in this session, or
    // recorded with their online files): they can remove it.
    let removable = app.session.live().published.clone().map(|p| (share.server.clone(), p.file)).filter(|(server, file)| {
        app.session.live().published.as_ref().is_some_and(|p| p.owner.is_some())
            || app.session.prefs.online_files.iter().any(|f| f.file == *file && figcraft_engine::online::same_server(&f.server, server))
    });
    join_and_advanced(app, ui, t, |app, ui, t| {
        if widgets::button(ui, tr("Leave session"), false, true).on_hover_text(tr("Your copy stays open; the file stays on the server")).clicked() {
            app.run("collab.leave", json!({}));
        }
        let Some((server, file)) = removable else { return };
        if !std::mem::take(&mut app.ui.live.confirm_remove) {
            if widgets::button(ui, tr("Remove from server…"), false, true).clicked() {
                app.ui.live.confirm_remove = true;
            }
            return;
        }
        widgets::label(
            ui,
            tr("Remove this file from the server? It can't be shared or opened from there again. Your copy stays open here."),
            t.danger,
        );
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Remove"), true, true).clicked() {
                // The owner token comes from the session or the record; the session ends with the
                // file.
                let owner = app.session.live().published.as_ref().and_then(|p| p.owner.clone());
                let mut params = json!({"server": server, "file": file, "action": "remove"});
                if let Some(owner) = owner {
                    params["owner"] = json!(owner);
                }
                app.run_background(&ui.ctx().clone(), "collab.manageFile", params, Box::new(|app, _, _| app.toast(tr("Removed from the server"))));
            } else if !widgets::button(ui, tr("Cancel"), false, true).clicked() {
                app.ui.live.confirm_remove = true;
            }
        });
    });
}

const SHARE: &str = "collab.share";
const STOP: &str = "collab.stop";

/// The join field's width in the Share popover (the Join button fits beside it).
const JOIN_WIDTH: f32 = 196.0;

/// What sharing without a join code falls back to.
const NO_CODE_NOTE: &str =
    tr_noop("No join code here (the server is older than join codes, or this address can't be put in one): share the link instead.");

/// Sharing with a join code: the code, large; copying it or the link; what it lets people do;
/// who is here; and stop.
fn share_ui(app: &mut FigApp, ui: &mut Ui, t: &Tokens, share: &Share) {
    let live = app.session.live();
    let online = share.via == Via::Online;
    let stop = |app: &mut FigApp, ui: &mut Ui| {
        let hint = if online {
            tr("Others lose access and keep their copy; the file stays on the server for you")
        } else {
            tr("Ends the session; everyone keeps their copy")
        };
        // The server is asked without the window waiting for it.
        if widgets::button(ui, tr("Stop sharing"), false, !app.session.waiting_for(STOP)).on_hover_text(hint).clicked() {
            app.run_background(&ui.ctx().clone(), STOP, json!({}), Box::new(|_, _, _| {}));
        }
    };
    if live.status != Status::Live {
        return stop(app, ui);
    }
    let view = share.access == Access::View;
    let link = share.link().unwrap_or_default().to_string();
    let me = live.info.as_ref().map(|i| i.name.clone()).unwrap_or_default();
    let names: Vec<String> = std::iter::once(trf("{name} (you)", &[("name", &me)])).chain(live.peers.values().map(|p| p.name.clone())).collect();
    let hosting = live.hosting.clone();
    match share.code() {
        Some(code) => {
            ui.vertical_centered(|ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&code).font(egui::FontId::monospace(if online { 24.0 } else { 15.0 })).color(t.text))
                    .on_hover_text(tr("Others type this under Share ▸ Join with a code"));
            });
            ui.horizontal(|ui| {
                if widgets::button(ui, tr("Copy code"), true, true).clicked() {
                    app.services.clipboard_set_text(&code);
                    app.toast(tr("Join code copied"));
                }
                if widgets::button(ui, tr("Copy link"), false, true).on_hover_text(tr("An invite link that opens FigCraft and joins")).clicked() {
                    app.services.clipboard_set_text(&link);
                    app.toast(tr("Invite link copied"));
                }
            });
        }
        None => {
            widgets::label(ui, tr(NO_CODE_NOTE), t.text_secondary);
            link_rows(app, ui, &[(tr("Invite link"), &link, tr("Invite link copied"))]);
        }
    }
    // A server has a link (and a code) for looking only; a session this app hosts doesn't.
    if share.links.as_ref().is_some_and(|l| l.view.is_some())
        && let Some(i) = widgets::segmented_text(ui, &[tr("Can edit"), tr("Can view")], usize::from(view))
        && (i == 1) != view
    {
        app.run("collab.share", json!({"access": if i == 1 { "view" } else { "edit" }}));
    }
    let who = match (online, view) {
        (true, true) => tr("Anyone with this code can look at the file, not change it."),
        (true, false) => tr("Anyone with this code can edit the file."),
        (false, _) => tr("Anyone on your network with this code can edit the file, while this app stays open."),
    };
    widgets::label(ui, who, t.text_secondary);
    let here = trf("Here: {names}", &[("names", &names.join(", "))]);
    ui.label(egui::RichText::new(widgets::elide(ui, &here, ui.available_width())).font(regular(11.0)).color(t.text_secondary)).on_hover_text(here);
    if let Some(h) = &hosting {
        host_line(ui, t, &h.operator, &h.terms);
        expiry_line(ui, t, h.expires);
    }
    stop(app, ui);
}

/// Hosting: the invite link and URL to copy, and stop.
fn host_ui(app: &mut FigApp, ui: &mut Ui) {
    let Some(info) = app.session.live().info.clone() else { return };
    link_rows(app, ui, &[(tr("Invite link"), &info.link, tr("Invite link copied")), (tr("WebSocket URL"), &info.url, tr("WebSocket URL copied"))]);
    let t = Tokens::get(ui.ctx());
    widgets::label(ui, tr("The link carries the session's secret: share it only with people you invite."), t.text_secondary);
    widgets::label(ui, tr(UNDO_NOTE), t.text_secondary);
    if widgets::button(ui, tr("Stop session"), false, true).clicked() {
        app.run("collab.stop", json!({}));
    }
}

/// Joined, or publishing: where the file is and what you may do there, a published file's links,
/// and leave (the file stays open as your copy).
fn guest_ui(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let live = app.session.live();
    let view = live.info.as_ref().is_some_and(|i| i.access == Access::View);
    if let Some(i) = live.info.as_ref() {
        widgets::label(ui, &format!("{} · {}", i.address, if view { tr("View only") } else { tr("Can edit") }), t.text_secondary);
        if !i.pages.is_empty() {
            let pages: Vec<&str> = i.pages.iter().map(|p| p.name.as_str()).take(8).collect();
            widgets::label(ui, &trf("Pages: {pages}", &[("pages", &pages.join(", "))]), t.text_secondary);
        }
    }
    let hosting = live.hosting.clone();
    if let Some(p) = live.published.clone() {
        link_rows(app, ui, &[(tr("Edit link"), &p.edit, tr("Edit link copied")), (tr("View link"), &p.view, tr("View link copied"))]);
        widgets::label(ui, tr(PUBLISHED_NOTE), t.text_secondary);
    }
    if let Some(h) = &hosting {
        host_line(ui, t, &h.operator, &h.terms);
        expiry_line(ui, t, h.expires);
    }
    widgets::label(ui, tr(if view { VIEW_NOTE } else { UNDO_NOTE }), t.text_secondary);
    if widgets::button(ui, tr("Leave session"), false, true).on_hover_text(tr("Your copy of the file stays open")).clicked() {
        app.run("collab.leave", json!({}));
    }
    // A server that takes reports said so (its hosting).
    if hosting.is_some() && app.session.live().status == Status::Live {
        report_ui(app, ui, t);
    }
}

/// "Hosted by …" and a link to the terms, when the server names them.
fn host_line(ui: &mut Ui, t: &Tokens, operator: &str, terms: &str) {
    if operator.is_empty() && terms.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if !operator.is_empty() {
            widgets::label(ui, &trf("Hosted by {operator}", &[("operator", &operator)]), t.text_secondary);
        }
        if !terms.is_empty() {
            if !operator.is_empty() {
                widgets::label(ui, "·", t.text_secondary);
            }
            ui.hyperlink_to(egui::RichText::new(tr("Terms")).font(regular(11.0)), terms).on_hover_text(terms);
        }
    });
}

/// When the server deletes the file unless someone opens it.
fn expiry_line(ui: &mut Ui, t: &Tokens, expires: Option<u64>) {
    if let Some(e) = expires {
        let date = figcraft_engine::live::utc_date(e);
        widgets::label(ui, &trf("The server deletes this file if nobody opens it before {date}.", &[("date", &date)]), t.text_secondary);
    }
}

/// Report the file you're in to the server's operator: a button, then a reason to send.
fn report_ui(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let Some(mut reason) = app.ui.live.report.take() else {
        if widgets::button(ui, tr("Report this file…"), false, true).on_hover_text(tr("Tell the server's operator what is wrong with it")).clicked()
        {
            app.ui.live.report = Some(String::new());
            app.ui.live.focus_report = true;
        }
        return;
    };
    widgets::label(ui, tr("What is wrong with this file? The server's operator gets your report."), t.text);
    let resp = ui.add(
        egui::TextEdit::singleline(&mut reason)
            .hint_text(tr("Spam, stolen work, abuse…"))
            .char_limit(figcraft_engine::live::MAX_REPORT)
            .desired_width(ui.available_width())
            .id(egui::Id::new("live_report_reason")),
    );
    a11y::text(&resp, tr("Report"), &reason);
    crate::widgets::select_all_on_focus(ui, &resp, &reason);
    if std::mem::take(&mut app.ui.live.focus_report) {
        resp.request_focus();
    }
    let mut keep = true;
    ui.horizontal(|ui| {
        let send = widgets::button(ui, tr("Send report"), true, !reason.trim().is_empty()).clicked()
            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if send && !reason.trim().is_empty() && app.run("collab.report", json!({"reason": reason.trim()})).is_some() {
            keep = false;
        }
        if widgets::button(ui, tr("Cancel"), false, true).clicked() {
            keep = false;
        }
    });
    if keep {
        app.ui.live.report = Some(reason);
    }
}

/// Links to copy: a label, the link (elided, monospace) and a Copy button each.
fn link_rows(app: &mut FigApp, ui: &mut Ui, rows: &[(&str, &String, &str)]) {
    for (label, value, what) in rows {
        ui.horizontal(|ui| {
            let t = Tokens::get(ui.ctx());
            let w = ui.available_width() - 64.0;
            ui.vertical(|ui| {
                ui.set_width(w);
                widgets::label(ui, label, t.text_secondary);
                ui.label(egui::RichText::new(widgets::elide(ui, value, w)).font(regular(10.5)).monospace());
            });
            if widgets::button(ui, tr("Copy"), false, true).on_hover_text(trf("Copy the {label}", &[("label", &label.to_lowercase())])).clicked() {
                app.services.clipboard_set_text(value);
                app.toast(*what);
            }
        });
    }
}

/// Open the Join dialog, with `link` in its field (focused).
pub fn open_join(app: &mut FigApp, link: &str) {
    app.ui.live.join = Some(link.to_string());
    app.ui.live.focus_join = true;
}

/// An invite link the app was opened with (`figcraft figcraft://join?…`): the Join dialog opens
/// with the link filled in, so joining is the person's own choice.
///
/// The system hands the app any `figcraft://` link that is followed, which may be a page, a
/// message or a document rather than something the person meant to act on, so the link is shown
/// and confirmed rather than acted on.
pub fn open_invite(app: &mut FigApp, link: &str) {
    open_join(app, link.trim());
}

/// Escape closes the Join or Publish dialog (before the canvas's shortcuts see it: it doesn't
/// deselect).
pub fn dialog_keys(app: &mut FigApp, ctx: &egui::Context) {
    let live = &mut app.ui.live;
    if (live.join.is_some() || live.publish.is_some()) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        live.join = None;
        live.publish = None;
    }
}

/// File ▸ Join live session: type a join code or paste an invite link.
pub fn join_dialog(app: &mut FigApp, ctx: &egui::Context) {
    // The editor takes Escape before its shortcuts; the home screen has none.
    dialog_keys(app, ctx);
    let Some(mut link) = app.ui.live.join.take() else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut cancel = false;
    let mut go = false;
    let focus = std::mem::take(&mut app.ui.live.focus_join);
    crate::dialogs::window(tr("Join live session"), ctx).id(egui::Id::new("Join live session")).open(&mut open).show(ctx, |ui| {
        ui.set_width(380.0);
        widgets::label(ui, tr("Type the join code you were given, or paste an invite link."), t.text);
        ui.horizontal(|ui| {
            go = join_field(app, ui, &mut link, egui::Id::new("live_join_link"), 300.0, focus).is_some();
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
        widgets::label(ui, tr("You'll get the shared file as a new tab and edit it together."), t.text_secondary);
        if !app.session.live.can_host() {
            widgets::label(ui, tr(BROWSER_NOTE), t.text_secondary);
        }
    });
    if go {
        return;
    }
    if open && !cancel {
        app.ui.live.join = Some(link);
    }
}

/// Open the Publish to server dialog for the open file, with the last address used this run in
/// it (the server in Preferences is what Share ▸ Online uses: this dialog is for another one).
pub fn open_publish(app: &mut FigApp) {
    if !app.session.live.can_host() {
        app.toast(tr("Publishing to a server needs the desktop app; in the browser you can join a server's files"));
        return;
    }
    let name = app.session.doc().map(|d| d.name.clone()).unwrap_or_default();
    app.ui.live.publish = Some(PublishForm { server: app.ui.live.last_server.clone(), name, info: None });
    app.ui.live.focus_publish = true;
}

/// The address carries a server key (`?key=…`).
fn has_key(server: &str) -> bool {
    server.split_once('?').is_some_and(|(_, q)| q.split('&').any(|kv| kv.strip_prefix("key=").is_some_and(|k| !k.trim().is_empty())))
}

/// Ask the server at `server` about its hosting in the background; the answer goes into the
/// Publish dialog while it still shows that address, and is kept for the Share popover.
fn ask_server(app: &mut FigApp, ctx: &egui::Context, server: String) {
    app.run_background(
        ctx,
        SERVER_INFO,
        json!({"server": server}),
        Box::new(move |app, _, v| {
            if let Some(f) = app.ui.live.publish.as_mut().filter(|f| f.server.trim() == server) {
                f.info = Some((server.clone(), v.clone()));
            }
            app.ui.live.server_info = Some((server, v));
        }),
    );
}

const SERVER_INFO: &str = "collab.serverInfo";

/// Share ▸ Publish to server…: the server's address (with its key, unless the server is open to
/// the public: it is asked first) and the file's name.
pub fn publish_dialog(app: &mut FigApp, ctx: &egui::Context) {
    published_dialog(app, ctx);
    let Some(mut form) = app.ui.live.publish.take() else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut cancel = false;
    let mut go = false;
    let mut ask = false;
    let focus = std::mem::take(&mut app.ui.live.focus_publish);
    let address = form.server.trim().to_string();
    let keyed = has_key(&address);
    let info = form.info.as_ref().filter(|(at, _)| *at == address).map(|(_, v)| v.clone());
    let asking = app.session.waiting_for(SERVER_INFO);
    let open_server = info.as_ref().is_some_and(|v| v["open"] == true);
    // A first look at an address already filled in.
    ask |= focus && !address.is_empty() && !keyed && info.is_none();
    crate::dialogs::window(tr("Publish to server"), ctx).id(egui::Id::new("Publish to server")).open(&mut open).show(ctx, |ui| {
        ui.set_width(380.0);
        widgets::label(
            ui,
            tr("Uploads this file to a FigCraft server and keeps this tab in sync with it. You get a link to edit and a link to view."),
            t.text,
        );
        widgets::label(ui, tr("Server address"), t.text_secondary);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut form.server)
                .hint_text("wss://server/ or ws://server:47900/?key=…")
                .desired_width(380.0)
                .id(egui::Id::new("live_publish_server")),
        );
        // The address may carry the server key: it isn't read out.
        a11y::text(&resp, tr("Server address"), "");
        crate::widgets::select_all_on_focus(ui, &resp, &form.server);
        if focus {
            resp.request_focus();
        }
        let enter = |ui: &Ui| ui.input(|i| i.key_pressed(egui::Key::Enter));
        // Leaving the field (or Enter) asks a server without a key whether it is open.
        let left = resp.lost_focus() && !address.is_empty() && !keyed && info.is_none() && !asking;
        go = resp.lost_focus() && enter(ui) && (keyed || open_server);
        ask |= left;
        match &info {
            _ if keyed => widgets::label(ui, tr("Publishing with the server's key."), t.text_secondary),
            _ if asking => widgets::label(ui, tr("Asking the server…"), t.text_secondary),
            Some(v) if open_server => {
                widgets::label(ui, tr("This server is open to the public: no key needed."), t.success);
                hosting_lines(ui, &t, v);
            }
            Some(_) => widgets::label(
                ui,
                tr("This server needs its key: add ?key=… to the address (`figcraft-server key` prints it on the server's machine)."),
                t.danger,
            ),
            None => widgets::label(
                ui,
                tr("Give the address of a server open to the public, or a server's address with its key (`figcraft-server key` prints it)."),
                t.text_secondary,
            ),
        }
        widgets::label(ui, tr("File name"), t.text_secondary);
        let nresp = ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(380.0).id(egui::Id::new("live_publish_name")));
        a11y::text(&nresp, tr("File name"), &form.name);
        crate::widgets::select_all_on_focus(ui, &nresp, &form.name);
        go |= nresp.lost_focus() && enter(ui) && (keyed || open_server);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            go |= widgets::button(ui, tr("Publish"), true, keyed || open_server).clicked();
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    if ask {
        ask_server(app, ctx, address.clone());
    }
    if go && app.run("collab.publish", json!({"server": address, "name": form.name.trim()})).is_some() {
        app.ui.live.last_server = address;
        // The owner token shows once the file is on the server.
        app.ui.live.owner = (!keyed).then(|| app.session.live().id);
        app.cancel_background(SERVER_INFO);
        return;
    }
    if open && !cancel {
        app.ui.live.publish = Some(form);
    } else {
        app.cancel_background(SERVER_INFO);
    }
}

/// The file just published from the dialog without the key: its links and its owner token, shown
/// this once, with where it is hosted and when it expires.
fn published_dialog(app: &mut FigApp, ctx: &egui::Context) {
    let Some(live) = app.ui.live.owner.and_then(|id| app.session.live.get(id)) else {
        // Nothing published without a key, or it didn't get there (the message says why).
        app.ui.live.owner = None;
        return;
    };
    let Some(p) = live.published.clone().filter(|p| p.owner.is_some()) else {
        // Still on its way, unless it was published with the key after all.
        if live.status != Status::Connecting {
            app.ui.live.owner = None;
        }
        return;
    };
    let hosting = live.hosting.clone();
    let owner = p.owner.clone().unwrap_or_default();
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut done = false;
    crate::dialogs::window(tr("File published"), ctx).id(egui::Id::new("File published")).open(&mut open).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        widgets::label(ui, &trf("{name} is on the server.", &[("name", &p.name)]), t.text);
        link_rows(app, ui, &[(tr("Owner token"), &owner, tr("Owner token copied"))]);
        widgets::label(ui, tr(OWNER_NOTE), t.danger);
        link_rows(app, ui, &[(tr("Edit link"), &p.edit, tr("Edit link copied")), (tr("View link"), &p.view, tr("View link copied"))]);
        expiry_line(ui, &t, p.expires);
        if let Some(h) = &hosting {
            host_line(ui, &t, &h.operator, &h.terms);
        }
        widgets::label(ui, tr(PLAIN_NOTE), t.text_secondary);
        ui.add_space(4.0);
        done = widgets::button(ui, tr("Done"), true, true).clicked();
    });
    if !open || done {
        app.ui.live.owner = None;
    }
}

/// Live session state for `ui.inspect`.
pub fn inspect(app: &FigApp) -> serde_json::Value {
    let l = app.session.live();
    json!({
        "session": (l.id != 0).then_some(l.id),
        "sessions": app.session.live.iter().count(),
        "status": l.status.id(),
        "role": l.role(),
        "peers": l.peers.len(),
        "follow": l.follow,
        "signals": l.signals,
        "spotlight": l.spotlight,
        "spotlighting": l.spotlighting,
        "chat": l.chat,
        "chats": l.chats,
        "chatBox": app.ui.live.chat.as_ref().map(|c| json!({"text": c.text, "editing": c.editing})),
        "cursorsShown": app.ui.live.shown(),
        "joinDialog": app.ui.live.join.is_some(),
        "joinField": app.ui.live.code,
        "code": l.share.as_ref().and_then(Share::code),
        "share": l.share.as_ref().map(|s| json!({"via": s.via, "access": s.access.id(), "shared": s.links.is_some()})),
        "local": app.ui.live.local,
        "advanced": app.ui.live.advanced,
        "publishDialog": app.ui.live.publish.is_some(),
        "publishInfo": app.ui.live.publish.as_ref().and_then(|f| f.info.as_ref().map(|(_, v)| v.clone())),
        "ownerDialog": app.ui.live.owner.and_then(|id| app.session.live.get(id)).and_then(|l| l.published.as_ref()).is_some_and(|p| p.owner.is_some()),
        "report": app.ui.live.report,
        "readOnly": app.session.active_read_only(),
    })
}

/// Peer ids drawn on the canvas, by what they point at (tests).
#[cfg(test)]
pub(crate) fn shown_at(app: &FigApp) -> Vec<(u32, (f64, f64))> {
    app.ui.live.shown.iter().map(|(k, v)| (*k, *v)).collect()
}
