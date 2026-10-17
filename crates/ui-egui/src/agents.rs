//! Preferences ▸ AI agents: the local MCP server agents reach the app through, the MCP clients it
//! is installed in, and what agents did. The server and the clients' settings files belong to the
//! app crate (they need sockets, files and processes): it plugs them in through [`AgentHost`], and
//! the browser build, which can't listen, has none.

use std::collections::VecDeque;

use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// What agents did that the tab lists (newest first), at most this many.
const ACTIVITY_MAX: usize = 50;
/// Rows of activity the tab shows.
const ACTIVITY_SHOWN: usize = 8;
/// The tab's content width (Preferences draws it right of its sidebar).
pub const WIDTH: f32 = 515.0;

/// The server agents connect to, as the app runs it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerStatus {
    /// The port it listens on (127.0.0.1 only), when it runs.
    pub listening: Option<u16>,
    /// Agents connected now.
    pub connections: usize,
    /// Why it didn't start, the last time it was asked to.
    pub error: Option<String>,
    /// The port that error is about.
    pub error_port: Option<u16>,
}

/// An MCP client the app can install itself in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// A stable id: "claude-code", "claude-desktop", "cursor".
    pub id: &'static str,
    /// Its product name, as the client calls itself.
    pub name: &'static str,
    pub state: ClientState,
    /// Where it was found or configured (a path), for the row's second line.
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// Not on this computer.
    Missing,
    /// On this computer, without FigCraft.
    Available,
    /// FigCraft is in its settings; `same_port` is false when they point at another port.
    Installed { same_port: bool },
}

/// The app side of the AI agents tab (desktop only).
pub trait AgentHost {
    fn status(&self) -> ServerStatus;
    /// Start listening on `port` (stopping first if it listens elsewhere).
    fn start(&mut self, port: u16) -> Result<(), String>;
    fn stop(&mut self);
    /// Close every agent connection (the server keeps listening).
    fn disconnect(&mut self);
    /// The first port from `from` up that is free to listen on.
    fn free_port(&self, from: u16) -> Option<u16>;
    /// The clients, found on this computer or not.
    fn clients(&mut self, port: u16) -> Vec<Client>;
    /// Put FigCraft into a client's settings, pointing at `port`. `Ok` says what to do next.
    fn install(&mut self, id: &str, port: u16) -> Result<String, String>;
    fn uninstall(&mut self, id: &str) -> Result<(), String>;
    /// The settings to paste into any other MCP client.
    fn config_json(&self, port: u16) -> String;
}

/// One thing an agent did.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    /// UI clock (seconds).
    pub at: f64,
    pub command: String,
    /// The command's label in the interface language, and the layer it made, when it made one.
    pub detail: String,
}

/// Who connected and what they did (kept while the app runs).
#[derive(Clone, Debug, Default)]
pub struct AgentLog {
    /// `clientInfo` of the last MCP client that said hello: name and version.
    pub client: Option<(String, String)>,
    /// When it said hello (UI clock).
    pub since: f64,
    /// Commands that changed a document since then.
    pub commands: usize,
    pub recent: VecDeque<Activity>,
    /// The port typed in the field, until it's applied.
    pub port_text: Option<String>,
    /// The tab strip indicator's popover is open.
    pub popover: bool,
    /// The clients as last read (their settings files can be large): the port they were read
    /// for, the pass they were last shown in, and the list. Read again when the tab opens, after
    /// an install or a removal, and when the port changes, never every frame.
    pub clients: Option<(u16, u64, Vec<Client>)>,
}

impl AgentLog {
    pub fn hello(&mut self, client: &str, version: &str, now: f64) {
        let name: String = client.chars().filter(|c| !c.is_control()).take(64).collect();
        let version: String = version.chars().filter(|c| !c.is_control()).take(32).collect();
        self.client = Some((name, version));
        self.since = now;
        self.commands = 0;
    }

    pub fn record(&mut self, command: &str, detail: String, now: f64) {
        self.commands += 1;
        self.recent.push_front(Activity { at: now, command: command.to_string(), detail });
        self.recent.truncate(ACTIVITY_MAX);
    }

    /// An MCP client said hello and is still connected (a script on the control channel that
    /// never said hello isn't an agent).
    pub fn connected(&self, status: &ServerStatus) -> bool {
        status.connections > 0 && self.client.is_some()
    }

    /// Forget who said hello once nobody is connected (the next connection may be someone else).
    pub fn sync(&mut self, status: &ServerStatus) {
        if status.connections == 0 {
            self.client = None;
        }
    }

    /// The connected client's name as people know it ("claude-code" → "Claude Code").
    pub fn client_name(&self) -> String {
        let raw = self.client.as_ref().map(|(n, _)| n.as_str()).unwrap_or_default();
        pretty_client(raw).unwrap_or_else(|| tr("An AI agent").to_string())
    }
}

/// "claude-code" → "Claude Code", "cursor-vscode" → "Cursor"; unknown names as they are.
pub fn pretty_client(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let known = [("claude-code", "Claude Code"), ("claude-ai", "Claude Desktop"), ("claude desktop", "Claude Desktop"), ("cursor", "Cursor")];
    if raw.trim().is_empty() {
        return None;
    }
    Some(known.iter().find(|(k, _)| lower.starts_with(k)).map_or_else(|| raw.trim().to_string(), |(_, v)| v.to_string()))
}

/// "now", "12 s", "3 min", "2 h" (no clock: the UI clock has no time zone).
fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..5 => tr("now").to_string(),
        5..60 => trf("{n} s", &[("n", &s)]),
        60..3600 => trf("{n} min", &[("n", &(s / 60))]),
        _ => trf("{n} h", &[("n", &(s / 3600))]),
    }
}

// ---------- the tab ----------

/// Gaps between the tab's blocks.
const GAP: f32 = 16.0;

fn text(ui: &mut Ui, s: &str, font: FontId, color: Color32) {
    ui.add(egui::Label::new(RichText::new(s).font(font).color(color)).wrap());
}

fn link(ui: &mut Ui, t: &Tokens, s: &str) -> bool {
    let resp = ui.add(egui::Label::new(RichText::new(s).font(regular(12.0)).color(t.accent_text)).sense(Sense::click()));
    crate::a11y::button(&resp, s);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().circle_filled(r.center(), size / 2.0, color);
}

/// Preferences ▸ AI agents.
pub fn tab(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing.y = 0.0;
    let now = ui.input(|i| i.time);
    let Some(host) = app.services.agents() else {
        text(ui, tr("AI agents connect to the desktop app: a browser tab can't listen for them."), regular(13.0), t.text_secondary);
        return;
    };
    let status = host.status();
    app.ui.agents.sync(&status);
    let port = app.session.prefs.agents.port;
    // Agents come and go on other threads: look again now and then while the tab is open.
    crate::repaint_in(ui.ctx(), 1.0);
    if status.listening.is_none() && status.error.is_none() {
        return empty_state(app, ui, &t, port);
    }
    let clients = cached_clients(app, ui.ctx().cumulative_pass_nr(), port);
    status_header(app, ui, &t, &status, now);
    ui.add_space(GAP);
    options(app, ui, &t, &status);
    ui.add_space(GAP);
    widgets::separator(ui);
    ui.add_space(GAP);
    clients_list(app, ui, &t, &clients, port);
    ui.add_space(GAP);
    if app.ui.agents.connected(&status) || !app.ui.agents.recent.is_empty() {
        activity(app, ui, &t, &status, now);
    } else if status.listening.is_some() {
        waiting(ui, &t);
    }
}

/// Never turned on: what MCP does, shown rather than told, and one button.
fn empty_state(app: &mut FigApp, ui: &mut Ui, t: &Tokens, port: u16) {
    let (r, _) = ui.allocate_exact_size(vec2(WIDTH, 232.0), Sense::hover());
    picture(ui, t, r);
    ui.add_space(24.0);
    text(ui, tr("Let an AI agent work on your file"), semibold(18.0), t.text);
    ui.add_space(8.0);
    text(
        ui,
        tr(
            "Turn on MCP and assistants such as Claude Code, Claude Desktop or Cursor create layers, set up auto layout and export for you. Every change shows on the canvas as it happens and goes into the history, so you can undo it.",
        ),
        regular(13.0),
        t.text_secondary,
    );
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let label = tr("Turn on MCP");
        let font = semibold(13.0);
        let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE);
        let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, g.size().y + 16.0), Sense::click());
        crate::a11y::button(&resp, label);
        ui.painter().rect_filled(r, CornerRadius::same(5), if resp.hovered() { t.accent_hover } else { t.accent });
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            turn_on(app, port, true);
        }
        ui.label(RichText::new(tr("Only connections from this computer are accepted.")).font(regular(12.0)).color(t.text_secondary));
    });
    ui.add_space(24.0);
    widgets::separator(ui);
    ui.add_space(24.0);
    let steps = [
        ("1", tr("Turn it on").to_string(), trf("FigCraft listens on port {port}, on this computer only.", &[("port", &port)])),
        ("2", tr("Connect your client").to_string(), tr("One click installs FigCraft in Claude Code, Claude Desktop or Cursor.").to_string()),
        (
            "3",
            tr("Ask in your words").to_string(),
            tr("“Align the selection and export it as PNG at 2×.” The agent does it; you review it and undo what you don't want.").to_string(),
        ),
    ];
    let col = (WIDTH - 40.0) / 3.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        for (n, title, body) in &steps {
            ui.vertical(|ui| {
                ui.set_width(col);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(*n).font(semibold(11.0)).color(t.accent_text));
                ui.label(RichText::new(title).font(semibold(13.0)).color(t.text));
                text(ui, body, regular(12.0), t.text_secondary);
            });
        }
    });
}

/// A request in words turning into a layer on the canvas, selected: what an agent does (the
/// design's empty-state picture, drawn at its coordinates).
fn picture(ui: &Ui, t: &Tokens, r: Rect) {
    let p = ui.painter();
    let at = |x: f32, y: f32| r.min + vec2(x, y);
    p.rect_filled(r, CornerRadius::same(8), t.canvas);
    // The request.
    let bubble = Rect::from_min_size(at(24.0, 44.0), vec2(196.0, 84.0));
    p.rect_filled(bubble, CornerRadius::same(8), t.panel);
    p.rect_stroke(bubble, CornerRadius::same(8), Stroke::new(1.0, t.border), StrokeKind::Inside);
    let words = p.layout(tr("“Make a profile card with auto layout and a Follow button.”").to_string(), regular(12.0), t.text, 168.0);
    p.galley(bubble.min + vec2(14.0, 14.0), words, t.text);
    p.text(at(24.0, 140.0), Align2::LEFT_TOP, tr("Your agent"), regular(11.0), t.text_secondary);
    // MCP, between the two.
    let (x0, x1, y) = (r.min.x + 228.0, r.min.x + 304.0, r.min.y + 86.0);
    let mut x = x0;
    while x < x1 {
        p.line_segment([pos2(x, y), pos2((x + 4.0).min(x1), y)], Stroke::new(1.5, t.accent));
        x += 8.0;
    }
    let chip = Rect::from_min_size(at(246.0, 77.0), vec2(40.0, 18.0));
    p.rect_filled(chip, CornerRadius::same(9), t.panel);
    p.rect_stroke(chip, CornerRadius::same(9), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    p.text(chip.center(), Align2::CENTER_CENTER, "MCP", semibold(10.0), t.accent_text);
    // The layer it made, selected, with its size.
    let card = Rect::from_min_size(at(316.0, 36.0), vec2(176.0, 132.0));
    p.rect_filled(card, CornerRadius::same(8), t.panel);
    p.rect_stroke(card, CornerRadius::same(8), Stroke::new(1.0, t.border), StrokeKind::Inside);
    p.circle_filled(card.min + vec2(32.0, 32.0), 16.0, t.selected_row);
    let bar = |x: f32, y: f32, w: f32, h: f32, c: Color32| {
        p.rect_filled(Rect::from_min_size(card.min + vec2(x, y), vec2(w, h)), CornerRadius::same((h / 2.0) as u8), c);
    };
    bar(58.0, 21.0, 92.0, 8.0, t.text);
    bar(58.0, 35.0, 60.0, 6.0, t.text_tertiary);
    bar(16.0, 62.0, 144.0, 6.0, t.border);
    let button = Rect::from_min_size(card.min + vec2(16.0, 92.0), vec2(144.0, 24.0));
    p.rect_filled(button, CornerRadius::same(5), t.accent);
    p.text(button.center(), Align2::CENTER_CENTER, tr("Follow"), semibold(11.0), Color32::WHITE);
    p.rect_stroke(card, CornerRadius::ZERO, Stroke::new(1.0, t.accent), StrokeKind::Middle);
    for c in [card.left_top(), card.right_top(), card.left_bottom(), card.right_bottom()] {
        let h = Rect::from_center_size(c, vec2(6.0, 6.0));
        p.rect_filled(h, CornerRadius::ZERO, Color32::WHITE);
        p.rect_stroke(h, CornerRadius::ZERO, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    let size = Rect::from_min_size(at(370.0, 178.0), vec2(68.0, 18.0));
    p.rect_filled(size, CornerRadius::same(3), t.accent);
    p.text(size.center(), Align2::CENTER_CENTER, "176 × 132", semibold(10.0), Color32::WHITE);
    p.text(at(316.0, 206.0), Align2::LEFT_TOP, tr("On your canvas, live"), regular(11.0), t.text_secondary);
}

fn turn_on(app: &mut FigApp, port: u16, remember: bool) {
    let started = app.services.agents().map(|h| h.start(port));
    if remember && matches!(started, Some(Ok(()))) {
        app.run("pref.agents", json!({"enabled": true}));
    }
}

fn status_header(app: &mut FigApp, ui: &mut Ui, t: &Tokens, status: &ServerStatus, now: f64) {
    let (color, title, sub) = match (&status.error, status.listening) {
        (Some(e), Some(still)) => {
            let port = status.error_port.unwrap_or(app.session.prefs.agents.port);
            let sub = format!("{} {}", port_error(e, port), trf("Still listening on 127.0.0.1:{port}.", &[("port", &still)]));
            (t.danger, trf("Couldn't switch to port {port}", &[("port", &port)]), sub)
        }
        (Some(e), None) => {
            let port = status.error_port.unwrap_or(app.session.prefs.agents.port);
            (t.danger, tr("Couldn't turn on MCP").to_string(), port_error(e, port))
        }
        (None, Some(port)) if app.ui.agents.connected(status) => {
            let n = trn("{n} commands", app.ui.agents.commands, &[]);
            let secs = now - app.ui.agents.since;
            let since = if secs < 5.0 { tr("Connected just now").to_string() } else { trf("Connected {time} ago", &[("time", &ago(secs))]) };
            (t.success, trf("{client} is connected", &[("client", &app.ui.agents.client_name())]), format!("{since} · {n} · 127.0.0.1:{port}"))
        }
        (None, Some(port)) => {
            (t.success, tr("MCP is on").to_string(), trf("Listening on 127.0.0.1:{port}, for this computer only.", &[("port", &port)]))
        }
        (None, None) => (t.text_tertiary, tr("MCP is off").to_string(), String::new()),
    };
    let failed = status.error.is_some();
    let (fill, margin) =
        if failed { (t.danger.gamma_multiply(0.08), egui::Margin::symmetric(14, 12)) } else { (Color32::TRANSPARENT, egui::Margin::ZERO) };
    egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(6)).inner_margin(margin).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let inner = ui.available_width() - 48.0;
            ui.vertical(|ui| {
                ui.set_width(inner);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    dot(ui, color, 8.0);
                    ui.label(RichText::new(&title).font(semibold(15.0)).color(t.text));
                });
                if !sub.is_empty() {
                    text(ui, &sub, regular(12.0), t.text_secondary);
                }
                if failed {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let from = status.error_port.unwrap_or(app.session.prefs.agents.port);
                        let free = app.services.agents().and_then(|h| h.free_port(from.saturating_add(1)));
                        if let Some(free) = free
                            && widgets::button(ui, &trf("Use port {port}", &[("port", &free)]), true, true).clicked()
                        {
                            app.run("pref.agents", json!({"port": free}));
                            app.ui.agents.port_text = None;
                            turn_on(app, free, false);
                        }
                        if widgets::button(ui, tr("Try again"), false, true).clicked() {
                            turn_on(app, from, false);
                        }
                    });
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let on = status.listening.is_some();
                if widgets::switch(ui, on, tr("MCP")).clicked() {
                    if on {
                        if let Some(h) = app.services.agents() {
                            h.stop();
                        }
                    } else {
                        turn_on(app, app.session.prefs.agents.port, false);
                    }
                }
            });
        });
    });
}

/// The OS's words for a port someone else listens on, in the interface language.
fn port_error(e: &str, port: u16) -> String {
    let lower = e.to_ascii_lowercase();
    if lower.contains("in use")
        || lower.contains("addrinuse")
        || lower.contains("os error 48")
        || lower.contains("os error 98")
        || lower.contains("10048")
    {
        trf("Port {port} is already in use by another app on this computer.", &[("port", &port)])
    } else {
        trf("Port {port}: {error}", &[("port", &port), ("error", &e)])
    }
}

fn options(app: &mut FigApp, ui: &mut Ui, t: &Tokens, status: &ServerStatus) {
    let prefs = app.session.prefs.agents.clone();
    if widgets::checkbox(ui, prefs.enabled, tr("Turn on when FigCraft starts")) {
        app.run("pref.agents", json!({"enabled": !prefs.enabled}));
    }
    ui.add_space(12.0);
    // The same preference as General's: files with a path save themselves a few seconds after any
    // change, the agent's included (the browser has no files to save to).
    if !cfg!(target_arch = "wasm32") {
        let hint = tr(
            "Files with a path save themselves a few seconds after any change, the agent's included, so the file on disk always has its latest work. A file that was never saved stays unsaved until you save it once.",
        );
        if widgets::checkbox(ui, app.session.prefs.auto_save, tr("Save files automatically")) {
            app.run("pref.autoSave", json!({}));
        }
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            text(ui, hint, regular(12.0), t.text_secondary);
        });
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(tr("Port")).font(regular(13.0)).color(t.text));
        let shown = app.ui.agents.port_text.clone().unwrap_or_else(|| prefs.port.to_string());
        let busy = status.error.is_some() && status.error_port == Some(prefs.port);
        if let Some(typed) = widgets::text_field(ui, egui::Id::new("agents_port"), &shown, 64.0, "7979") {
            match typed.trim().parse::<u16>() {
                Ok(p) if p >= 1024 => {
                    app.run("pref.agents", json!({"port": p}));
                    app.ui.agents.port_text = None;
                    if status.listening.is_some_and(|l| l != p) {
                        turn_on(app, p, false);
                    }
                }
                _ => app.ui.agents.port_text = Some(typed),
            }
        }
        let (hint, color) = if app.ui.agents.port_text.is_some() {
            (tr("A port is a whole number from 1024 to 65535.").to_string(), t.danger)
        } else if busy {
            (tr("Taken: pick another port.").to_string(), t.danger)
        } else {
            (trf("Change it if another app already uses {port}.", &[("port", &prefs.port)]), t.text_secondary)
        };
        ui.label(RichText::new(hint).font(regular(12.0)).color(color));
    });
}

/// The clients, read again only when the tab wasn't shown in the previous pass (it just opened),
/// or after [`AgentLog::clients`] was cleared or the port changed.
fn cached_clients(app: &mut FigApp, pass: u64, port: u16) -> Vec<Client> {
    let fresh = match &app.ui.agents.clients {
        Some((p, last, _)) => *p == port && pass <= last + 1,
        None => false,
    };
    if !fresh {
        let list = app.services.agents().map(|h| h.clients(port)).unwrap_or_default();
        app.ui.agents.clients = Some((port, pass, list));
    }
    let Some((_, last, list)) = &mut app.ui.agents.clients else { return Vec::new() };
    *last = pass;
    list.clone()
}

fn clients_list(app: &mut FigApp, ui: &mut Ui, t: &Tokens, clients: &[Client], port: u16) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(RichText::new(tr("Connect a client")).font(semibold(13.0)).color(t.text));
    text(ui, tr("Install writes FigCraft's settings into the client. No terminal needed."), regular(12.0), t.text_secondary);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(GAP);
    let mut first_available = true;
    egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for c in clients {
            let detail = match &c.state {
                ClientState::Missing => tr("Not found on this computer").to_string(),
                _ => c.detail.clone(),
            };
            let primary = matches!(c.state, ClientState::Available) && std::mem::take(&mut first_available);
            if let Some(act) = client_row(ui, t, c.name, &detail, Row::Client(&c.state, primary)) {
                act_on_client(app, c, act, port);
            }
            row_rule(ui, t);
        }
        if client_row(ui, t, tr("Another MCP client"), tr("Paste the settings into any MCP client."), Row::Copy).is_some() {
            let json = app.services.agents().map(|h| h.config_json(port)).unwrap_or_default();
            app.services.clipboard_set_text(&json);
            app.toast(tr("Settings copied: paste them into your client's MCP settings"));
        }
    });
}

fn row_rule(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, t.border);
}

#[derive(Clone, Copy)]
enum ClientAction {
    Install,
    Remove,
}

enum Row<'a> {
    Client(&'a ClientState, bool),
    Copy,
}

/// One client: its name and where it is, and what you can do with it.
fn client_row(ui: &mut Ui, t: &Tokens, name: &str, detail: &str, row: Row<'_>) -> Option<ClientAction> {
    let mut out = None;
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(name).font(semibold(13.0)).color(t.text));
                ui.label(RichText::new(detail).font(regular(12.0)).color(t.text_secondary));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                match row {
                    Row::Copy => {
                        if widgets::button(ui, tr("Copy settings"), false, true).clicked() {
                            out = Some(ClientAction::Install);
                        }
                    }
                    Row::Client(ClientState::Missing, _) => {
                        widgets::button(ui, tr("Install"), false, false);
                    }
                    Row::Client(ClientState::Available, primary) => {
                        if widgets::button(ui, tr("Install"), primary, true).clicked() {
                            out = Some(ClientAction::Install);
                        }
                    }
                    Row::Client(ClientState::Installed { same_port }, _) => {
                        if link(ui, t, tr("Remove")) {
                            out = Some(ClientAction::Remove);
                        }
                        if *same_port {
                            ui.label(RichText::new(tr("FigCraft installed")).font(semibold(12.0)).color(t.success_text));
                            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                            icons::paint(ui.painter(), r, Icon::Check, t.success);
                        } else if widgets::button(ui, tr("Update port"), true, true).clicked() {
                            out = Some(ClientAction::Install);
                        }
                    }
                }
            });
        });
    });
    out
}

fn act_on_client(app: &mut FigApp, c: &Client, act: ClientAction, port: u16) {
    let Some(host) = app.services.agents() else { return };
    let msg = match act {
        ClientAction::Install => match host.install(c.id, port) {
            Ok(next) => next,
            Err(e) => trf("Couldn't install FigCraft in {client}: {error}", &[("client", &c.name), ("error", &e)]),
        },
        ClientAction::Remove => match host.uninstall(c.id) {
            Ok(()) => trf("FigCraft was removed from {client}", &[("client", &c.name)]),
            Err(e) => trf("Couldn't remove FigCraft from {client}: {error}", &[("client", &c.name), ("error", &e)]),
        },
    };
    app.ui.agents.clients = None;
    app.toast(msg);
}

fn waiting(ui: &mut Ui, t: &Tokens) {
    egui::Frame::NONE.fill(t.canvas).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(0.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                dot(ui, t.text_tertiary, 8.0);
            });
            text(
                ui,
                tr("Waiting for an agent. After installing, open the client and ask: “Use FigCraft to create a 1440 × 900 frame.”"),
                regular(12.0),
                t.text,
            );
        });
    });
}

fn activity(app: &mut FigApp, ui: &mut Ui, t: &Tokens, status: &ServerStatus, now: f64) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Agent activity")).font(semibold(13.0)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.ui.agents.connected(status)
                && link(ui, t, tr("Disconnect"))
                && let Some(h) = app.services.agents()
            {
                h.disconnect();
            }
        });
    });
    ui.add_space(12.0);
    egui::Frame::NONE
        .fill(t.panel)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(0, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if app.ui.agents.recent.is_empty() {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 7)).show(ui, |ui| {
                    text(ui, tr("Connected. Nothing changed yet: what the agent does to your files shows here."), regular(12.0), t.text_secondary);
                });
            }
            for (i, a) in app.ui.agents.recent.iter().take(ACTIVITY_SHOWN).enumerate() {
                let fill = if i == 0 { t.selected_row_child } else { Color32::TRANSPARENT };
                egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(12, 7)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let cell = |ui: &mut Ui, w: f32, s: &str, font: FontId, c: Color32| {
                            let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
                            let shown = widgets::elide_font(ui, s, w, font.clone());
                            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, shown, font, c);
                        };
                        cell(ui, 56.0, &ago(now - a.at), regular(12.0), t.text_secondary);
                        cell(ui, 110.0, &a.command, FontId::monospace(12.0), t.text);
                        let rest = ui.available_width();
                        cell(ui, rest, &a.detail, regular(12.0), t.text);
                    });
                });
            }
        });
}

// ---------- the tab strip indicator ----------

/// The width [`indicator`] takes, or 0 when no agent is connected.
pub fn indicator_width(app: &mut FigApp, ui: &Ui) -> f32 {
    let Some(status) = app.services.agents().map(|h| h.status()) else { return 0.0 };
    app.ui.agents.sync(&status);
    if !app.ui.agents.connected(&status) {
        app.ui.agents.popover = false;
        return 0.0;
    }
    let name = app.ui.agents.client_name();
    ui.painter().layout_no_wrap(name, semibold(12.0), Color32::WHITE).size().x + 37.0
}

/// A pill in the tab strip while an agent is connected: who it is. A click opens a popover with
/// what it did, AI agents settings and Disconnect.
pub fn indicator(app: &mut FigApp, ui: &mut Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let name = app.ui.agents.client_name();
    let resp = ui.interact(rect, ui.id().with("agent_indicator"), Sense::click());
    crate::a11y::button(&resp, &name);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), if resp.hovered() { t.hover } else { t.panel });
    p.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    p.circle_filled(pos2(rect.min.x + 14.0, rect.center().y), 3.5, t.success);
    p.text(pos2(rect.min.x + 24.0, rect.center().y), Align2::LEFT_CENTER, &name, semibold(12.0), t.text);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        app.ui.agents.popover = !app.ui.agents.popover;
    }
    if app.ui.agents.popover {
        popover(app, ui, &t, rect, &name);
    }
}

fn popover(app: &mut FigApp, ui: &mut Ui, t: &Tokens, pill: Rect, name: &str) {
    let now = ui.input(|i| i.time);
    let width = 316.0;
    let pos = pos2((pill.max.x - width).max(8.0), pill.max.y + 8.0);
    let area = egui::Area::new(egui::Id::new("agent_popover")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(8))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: t.shadow })
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(width - 28.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                text(ui, &trf("{client} is editing this file", &[("client", &name)]), semibold(13.0), t.text);
                let n = trn("{n} commands", app.ui.agents.commands, &[]);
                let mut sub = match app.ui.agents.recent.front() {
                    Some(last) => trf("{n} this session · the last one {time} ago.", &[("n", &n), ("time", &ago(now - last.at))]),
                    None => trf("{n} this session.", &[("n", &n)]),
                };
                if app.session.prefs.auto_save {
                    sub.push(' ');
                    sub.push_str(tr("Saving automatically."));
                }
                text(ui, &sub, regular(12.0), t.text_secondary);
                ui.horizontal(|ui| {
                    if link(ui, t, tr("Open AI agents")) {
                        app.ui.preferences = true;
                        app.ui.prefs_tab = crate::dialogs::PrefsTab::Agents;
                        app.ui.agents.popover = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, tr("Disconnect"), false, true).clicked() {
                            if let Some(h) = app.services.agents() {
                                h.disconnect();
                            }
                            app.ui.agents.popover = false;
                        }
                    });
                });
            });
    });
    // A click anywhere else closes it.
    if ui.input(|i| i.pointer.any_click()) && !area.response.contains_pointer() && !ui.rect_contains_pointer(pill) {
        app.ui.agents.popover = false;
    }
}
