//! The control channel's UI-thread side. The app crate accepts JSON lines on a localhost socket
//! and queues [`ControlRequest`]s; [`drain`] runs them at the start of a frame so replies reflect
//! the real UI. Methods:
//!
//! - `engine.execute {command, params}` — run any command (no dialogs open)
//! - `engine.commands {}` — every command with its params doc
//! - `document.inspect {depth?, allPages?}` / `node.get {id, depth?}`
//! - `ui.inspect {}` — panel/tab/tool state, canvas rect, timings
//! - `ui.pointer {kind: down|move|up|click|double|hover|drag, x, y (page) | sx, sy (screen), to?, shift?, alt?, cmd?}`
//! - `ui.key {key, shift?, cmd?, alt?}` — a shortcut as if typed on the canvas
//! - `ui.type {text}` — type into the text being edited
//! - `ui.input {events: [{move|down|up: [x, y], button?} | {key, shift?, alt?, cmd?} | {text} | {leave}
//!   | {hold: {alt?, shift?, cmd?}} | {gamepad: "A" | "Gamepad A" | 0}]}` — raw pointer and key events
//!   for the next frame, as if from the window (panels, popovers); `hold` keeps modifier keys down
//!   for the frames that follow (all false: released); `gamepad` presses a controller button in the
//!   prototype playing (presenter or preview pane)
//! - `ui.menu.invoke {id}` — a menu item exactly like clicking it (may open a dialog)
//! - `ui.set {rightTab?, leftTab?, theme?, home?, homeSection?, homeSearch?, homeRecentsView?, homeRecentsSort?, homeRecentsOrder?, palette?, focusRegion?, preview?, presentScroll?, …}`
//! - `ui.screenshot {path?}` — PNG of the window (needs a visible window), written to `path` or
//!   returned as base64 `data`
//! - `ui.render {path?, scale?, ids?}` — PNG of the document (no UI chrome; works without a
//!   window), written to `path` or returned as base64 `data`
//! - `window.new {path?, index?}` → `{window}` / `window.list {}` / `window.focus {id}` — several
//!   windows (desktop)
//! - `app.quit {}`
//!
//! Any method takes `window` (an id from `window.list`) to run in that window, with its tabs and
//! panels; without it, requests go to the window last in front.

use egui::Context;
use figcraft_engine::cmd::comment::{Filter, Sort};
use figcraft_engine::{PointerEvent, PointerKind};
use serde_json::{Value, json};

use crate::FigApp;

pub struct ControlReply {
    pub id: Value,
    pub result: Result<Value, String>,
}

pub struct ControlRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
    pub reply: std::sync::mpsc::Sender<ControlReply>,
    /// The transport stops waiting after its timeout; discard requests still queued then.
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ControlRequest {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Pending screenshot replies.
pub struct PendingShot {
    pub id: Value,
    /// Where to write the PNG; `None`: return it as data.
    pub path: Option<String>,
    pub reply: std::sync::mpsc::Sender<ControlReply>,
    /// When it was asked for (UI clock, seconds).
    pub since: f64,
    /// The window it captures.
    pub viewport: egui::ViewportId,
}

/// How long a screenshot waits for the window to deliver it (the control server gives up then too).
const SHOT_TIMEOUT: f64 = 60.0;

fn pending(ctx: &Context) -> std::sync::Arc<std::sync::Mutex<Vec<PendingShot>>> {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("pending_shots"), || std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))).clone())
}

pub fn drain(app: &mut FigApp, ctx: &Context) {
    let now = ctx.input(|i| i.time);
    let list = pending(ctx);
    // Finish screenshots that arrived (a window's capture can arrive in any window's input).
    let shots: Vec<(egui::ViewportId, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Screenshot { viewport_id, image, .. } => Some((*viewport_id, image.clone())),
                _ => None,
            })
            .collect()
    });
    for (vp, img) in &shots {
        list.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|p| {
            if p.viewport != *vp {
                return true;
            }
            let res = shot_reply(img, p.path.as_deref());
            let _ = p.reply.send(ControlReply { id: p.id.clone(), result: res });
            false
        });
    }
    let reqs = std::mem::take(&mut app.control_queue);
    for r in reqs {
        if r.is_cancelled() {
            continue;
        }
        // Requests for another window wait for its frame.
        let target = r.params.get("window").and_then(Value::as_u64).unwrap_or(app.windows.focused);
        if target != app.windows.current {
            if app.windows.exists(target) {
                ctx.request_repaint_of(crate::windows::viewport(target));
                app.control_queue.push(r);
            } else {
                let _ = r.reply.send(ControlReply { id: r.id, result: Err(format!("no window {target}")) });
            }
            continue;
        }
        if r.method == "ui.screenshot" {
            let path = r.params.get("path").and_then(Value::as_str).map(str::to_string);
            let viewport = ctx.viewport_id();
            list.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(PendingShot { id: r.id, path, reply: r.reply, since: now, viewport });
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            continue;
        }
        let res = handle(app, ctx, &r.method, &r.params);
        let _ = r.reply.send(ControlReply { id: r.id, result: res });
        ctx.request_repaint();
    }
    // The window delivers a screenshot some frames after it was asked for (once the GPU has copied
    // it back), and only while frames are drawn: keep drawing until it arrives (a hidden window
    // never delivers; its requests expire).
    let mut list = list.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    list.retain(|p| {
        let live = now - p.since < SHOT_TIMEOUT;
        if !live {
            let _ = p.reply.send(ControlReply { id: p.id.clone(), result: Err("the window didn't deliver a screenshot (is it visible?)".into()) });
        }
        live
    });
    if !list.is_empty() {
        ctx.request_repaint();
    }
}

/// Methods only a drawn frame can answer: a capture of the window, raw window input and a new
/// window (the main window's frame opens it). Everything else acts on the app's state, which a
/// hidden window answers without a frame (#269, #720).
const NEEDS_FRAME: &[&str] = &["ui.screenshot", "ui.input", "window.new"];

/// Whether window `id` draws no frames now: minimised, or covered by other windows (on macOS also
/// on another Space).
fn hidden(ctx: &Context, id: u64) -> bool {
    ctx.input(|i| i.raw.viewports.get(&crate::windows::viewport(id)).and_then(egui::ViewportInfo::visible)) == Some(false)
}

/// A hidden window (minimised, or covered by other windows) draws no frames, but the desktop app's
/// `logic` still runs, for the main window even while every window is hidden: it answers the
/// requests for any hidden window, in that window's file, so an attached agent can edit, inspect,
/// type and save while the app is out of sight. What needs a frame ([`NEEDS_FRAME`]) is an error at
/// once instead of waiting for the window to show. A request for a window on screen waits for its
/// frame; another window on screen is asked to draw, which the main window's frame would do but
/// doesn't while it is hidden. A request for a window that doesn't exist is an error at once.
pub fn drain_hidden(app: &mut FigApp, ctx: &Context) {
    for r in std::mem::take(&mut app.control_queue) {
        if r.is_cancelled() {
            continue;
        }
        let target = r.params.get("window").and_then(Value::as_u64).unwrap_or(app.windows.focused);
        if !app.windows.exists(target) {
            let _ = r.reply.send(ControlReply { id: r.id, result: Err(format!("no window {target}")) });
            continue;
        }
        if !hidden(ctx, target) {
            // Another window on screen draws only when asked: while the main window is hidden no
            // frame of it does that (`drain`), so ask here.
            if target != app.windows.current {
                ctx.request_repaint_of(crate::windows::viewport(target));
            }
            app.control_queue.push(r);
            continue;
        }
        if NEEDS_FRAME.contains(&r.method.as_str()) {
            let error =
                format!("window {target} is minimised or hidden: `{}` needs it on screen (`ui.render` draws the document without it)", r.method);
            let _ = r.reply.send(ControlReply { id: r.id, result: Err(error) });
            continue;
        }
        // Outside a frame, so outside the frame's guard: within the window's swap, a panic still
        // ends in recovery copies and leaves every window its own state.
        let res = crate::windows::as_window(app, target, |app| {
            let mut res = Err("internal error".to_string());
            crate::windows::guard(app, ctx, |app| res = handle(app, ctx, &r.method, &r.params));
            res
        });
        let _ = r.reply.send(ControlReply { id: r.id, result: res.unwrap_or_else(|| Err(format!("no window {target}"))) });
    }
}

/// A screenshot's reply: the PNG written to `path` (through the automation roots, when the app
/// has them), or returned as base64 `data`.
fn shot_reply(img: &egui::ColorImage, path: Option<&str>) -> Result<Value, String> {
    let [w, h] = img.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        rgba.extend_from_slice(&p.to_srgba_unmultiplied());
    }
    let buf = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("bad screenshot buffer")?;
    let mut png = std::io::Cursor::new(Vec::new());
    buf.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let png = png.into_inner();
    match path {
        Some(path) => {
            figcraft_engine::file_access::write(path, &png).map_err(|e| e.to_string())?;
            Ok(json!({"path": path, "width": w, "height": h}))
        }
        None => {
            use base64::Engine as _;
            Ok(json!({"format": "png", "width": w, "height": h, "data": base64::engine::general_purpose::STANDARD.encode(&png)}))
        }
    }
}

/// A controller button given as its standard-layout index, its key trigger name ("Gamepad A") or
/// its short name ("A", "LB", "Up").
fn pad_name(v: &Value) -> Option<&'static str> {
    use figcraft_doc::proto::{GAMEPAD_BUTTONS, gamepad_button};
    if let Some(i) = v.as_u64() {
        return usize::try_from(i).ok().and_then(gamepad_button);
    }
    let s = v.as_str()?.trim();
    GAMEPAD_BUTTONS.iter().copied().find(|b| b.eq_ignore_ascii_case(s) || b.get(8..).is_some_and(|short| short.eq_ignore_ascii_case(s)))
}

fn b(p: &Value, k: &str) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// Handle one control method (also used by tests and the headless snapshot tool).
pub fn handle(app: &mut FigApp, ctx: &Context, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "engine.execute" => {
            let cmd = p.get("command").and_then(Value::as_str).ok_or("`command` required")?;
            let params = p.get("params").cloned().unwrap_or(json!({}));
            let r = app.session.execute(cmd, &params).map_err(|e| e.to_string());
            app.clear_comment_draft_after_tool_set(cmd, r.is_ok());
            // Programmatic calls never open dialogs.
            app.session.ui_requests.retain(|u| !u.kind.starts_with("dialog."));
            app.handle_ui_requests();
            r
        }
        "engine.commands" => Ok(Value::Array(
            figcraft_engine::all_commands()
                .iter()
                .map(|c| json!({"id": c.id, "label": c.label, "params": c.params, "menu": c.menu(), "shortcut": c.shortcut(), "undoable": c.history}))
                .collect(),
        )),
        "document.inspect" => {
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(8) as usize;
            Ok(figcraft_engine::inspect::document(&app.session, depth, b(p, "allPages")))
        }
        "node.get" => {
            let id = p.get("id").and_then(figcraft_engine::params::id_of).ok_or("`id` required")?;
            let depth = p.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            app.session.doc().map(|d| figcraft_engine::inspect::node_json(d, id, depth)).map_err(|e| e.to_string())
        }
        "window.new" => {
            let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
            crate::windows::open(app, ctx, index, p.get("path").and_then(Value::as_str), None).map(|id| json!({"window": id}))
        }
        "window.list" => Ok(crate::windows::list(app)),
        "window.focus" => crate::windows::focus(app, ctx, p.get("id").and_then(Value::as_u64).ok_or("`id` required")?),
        "ui.inspect" => {
            let mut v = app.inspect();
            // The name of the control with the keyboard (what a screen reader announces).
            if let Some(f) = v.get_mut("focus") {
                f["control"] = json!(crate::a11y::focused_name(ctx));
            }
            Ok(v)
        }
        "ui.pointer" => pointer(app, p),
        "ui.key" => {
            let key = p.get("key").and_then(Value::as_str).ok_or("`key` required")?;
            let m = figcraft_engine::keymap::Mods { shift: b(p, "shift"), alt: b(p, "alt"), cmd: b(p, "cmd"), ctrl: b(p, "ctrl") };
            let platform = crate::shortcuts::platform();
            use figcraft_engine::keymap::{Context, lookup, normalize};
            // Text editing keys go to the editor (after the shortcuts live while editing).
            if app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false) {
                if let Some(bd) = lookup(platform, Context::Text, key, m).filter(|_| m.cmd || m.alt || m.ctrl) {
                    crate::menus::invoke(app, bd.id);
                    return Ok(json!({"command": bd.id}));
                }
                let (word, line) = figcraft_engine::keymap::text_motion(platform, key, m);
                figcraft_engine::textedit::key(&mut app.session, key, m.shift, word, line).map_err(|e| e.to_string())?;
                return Ok(json!({"ok": true}));
            }
            // Vector edit mode and shape builder keys (tools, nudging points, delete, join, Enter).
            if app.session.tool_key(key, m.shift, m.alt, m.cmd).map_err(|e| e.to_string())? {
                return Ok(json!({"ok": true}));
            }
            // The Comment tool's arrows step through the comment threads.
            if app.session.tool.tool == figcraft_engine::ToolId::Comment && m.is_none() && matches!(key, "ArrowUp" | "ArrowDown" | "Up" | "Down") {
                let r = crate::comments::step(app, ctx, if matches!(key, "ArrowUp" | "Up") { -1 } else { 1 });
                return Ok(json!({"comment": r.and_then(|v| v.get("id").cloned())}));
            }
            if let Some(bd) = lookup(platform, Context::Canvas, key, m) {
                crate::shortcuts::run_binding(app, bd.id);
                return Ok(json!({"command": bd.id}));
            }
            let (name, _) = normalize(key).ok_or_else(|| format!("unknown key `{key}`"))?;
            match name {
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                    let (dx, dy) = match name {
                        "ArrowLeft" => (-1, 0),
                        "ArrowRight" => (1, 0),
                        "ArrowUp" => (0, -1),
                        _ => (0, 1),
                    };
                    let (cmd, params) = if m.cmd || m.ctrl {
                        ("transform.nudgeSize", json!({"dw": dx, "dh": dy, "big": m.shift}))
                    } else {
                        ("transform.nudge", json!({"dx": dx, "dy": dy, "big": m.shift}))
                    };
                    app.session.execute(cmd, &params).map_err(|e| e.to_string())
                }
                _ => {
                    // Number keys: opacity (two in quick succession make an exact percentage).
                    let now = ctx.input(|i| i.time);
                    match egui::Key::from_name(name) {
                        Some(k) if m.is_none() && crate::shortcuts::opacity_digit(app, k, now) => Ok(json!({"opacity": true})),
                        _ => Err(format!("no shortcut `{key}`")),
                    }
                }
            }
        }
        "ui.input" => {
            let list = p
                .get("events")
                .and_then(Value::as_array)
                .ok_or("`events` required: [{move|down|up: [x, y]} | {key, shift?, alt?, cmd?} | {text} | {leave: true}]")?;
            let pos = |v: &Value| -> Option<egui::Pos2> {
                let a = v.as_array()?;
                let p = egui::pos2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
                p.is_finite().then_some(p)
            };
            let mut events = Vec::new();
            let mut pads = Vec::new();
            for e in list.iter().take(256) {
                let button = if e.get("button").and_then(Value::as_str) == Some("secondary") {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                };
                let command = b(e, "cmd") || b(e, "ctrl");
                let mods = egui::Modifiers { alt: b(e, "alt"), ctrl: command, shift: b(e, "shift"), mac_cmd: false, command };
                if let Some(p) = e.get("move").and_then(pos) {
                    events.push(egui::Event::PointerMoved(p));
                } else if let Some((p, pressed)) =
                    e.get("down").and_then(pos).map(|p| (p, true)).or_else(|| e.get("up").and_then(pos).map(|p| (p, false)))
                {
                    events.push(egui::Event::PointerMoved(p));
                    events.push(egui::Event::PointerButton { pos: p, button, pressed, modifiers: mods });
                } else if let Some(k) = e.get("key").and_then(Value::as_str) {
                    let key = egui::Key::from_name(k).ok_or_else(|| format!("unknown key `{k}`"))?;
                    for pressed in [true, false] {
                        events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
                    }
                } else if let Some(t) = e.get("text").and_then(Value::as_str) {
                    events.push(egui::Event::Text(t.to_string()));
                } else if let Some(d) = e.get("scroll").and_then(pos) {
                    // Content moves by `d` points (negative y scrolls down, like a wheel turned towards you).
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: d.to_vec2(),
                        phase: egui::TouchPhase::Move,
                        modifiers: mods,
                    });
                } else if let Some(g) = e.get("gamepad") {
                    let names = figcraft_doc::proto::GAMEPAD_BUTTONS.join(", ");
                    pads.push(pad_name(g).ok_or_else(|| format!("unknown controller button {g} (an index 0-16 or one of {names})"))?.to_string());
                } else if b(e, "leave") {
                    events.push(egui::Event::PointerGone);
                } else if let Some(h) = e.get("hold") {
                    let held = egui::Modifiers {
                        alt: b(h, "alt"),
                        ctrl: b(h, "cmd") || b(h, "ctrl"),
                        shift: b(h, "shift"),
                        mac_cmd: false,
                        command: b(h, "cmd") || b(h, "ctrl"),
                    };
                    events.push(egui::Event::ModifiersChanged(held));
                } else {
                    return Err(format!("unknown input event {e}"));
                }
            }
            // Controller buttons go straight to the prototype playing (its next frame takes them).
            if !pads.is_empty() {
                match (app.ui.present.as_mut(), app.ui.preview.as_mut()) {
                    (Some(p), _) => p.pads.extend(pads),
                    (None, Some(pane)) => pane.player.pads.extend(pads),
                    (None, None) => return Err("controller buttons go to a prototype that is playing: run proto.present or proto.preview".into()),
                }
            }
            app.control_input.extend(events);
            ctx.request_repaint();
            Ok(json!({"queued": list.len().min(256)}))
        }
        "ui.type" => {
            let text = p.get("text").and_then(Value::as_str).ok_or("`text` required")?;
            figcraft_engine::textedit::insert(&mut app.session, text).map_err(|e| e.to_string())?;
            Ok(json!({"ok": true}))
        }
        "ui.menu.invoke" => {
            let id = p.get("id").and_then(Value::as_str).ok_or("`id` required")?;
            crate::menus::invoke(app, id);
            Ok(json!({"ok": true, "lastError": app.ui.last_error}))
        }
        "ui.set" => {
            if let Some(t) = p.get("rightTab").and_then(Value::as_str) {
                app.ui.right_tab = match t {
                    "prototype" => crate::RightTab::Prototype,
                    "inspect" => crate::RightTab::Inspect,
                    _ => crate::RightTab::Design,
                };
            }
            if let Some(t) = p.get("leftTab").and_then(Value::as_str) {
                app.ui.left_tab = if t == "assets" { crate::LeftTab::Assets } else { crate::LeftTab::File };
            }
            if let Some(t) = p.get("theme").and_then(Value::as_str) {
                app.session.prefs.theme = t.to_string();
            }
            if let Some(h) = p.get("home").and_then(Value::as_bool) {
                app.ui.home = h;
            }
            if let Some(section) = p.get("homeSection").and_then(Value::as_str).and_then(crate::home::Section::parse) {
                app.ui.home_view.section = section;
            }
            if let Some(query) = p.get("homeSearch").and_then(Value::as_str) {
                app.ui.home_view.file_query = query.chars().take(256).collect();
            }
            if let Some(view) = p.get("homeRecentsView").and_then(Value::as_str).and_then(crate::home_recents::View::parse) {
                view.store(&mut app.session.prefs);
            }
            if let Some(key) = p.get("homeRecentsSort").and_then(Value::as_str).and_then(crate::home_recents::SortKey::parse) {
                crate::home_recents::Sort::by(key).store(&mut app.session.prefs);
            }
            if let Some(order) = p.get("homeRecentsOrder").and_then(Value::as_str).filter(|o| matches!(*o, "asc" | "desc")) {
                let sort = crate::home_recents::Sort::of(&app.session.prefs);
                crate::home_recents::Sort { descending: order == "desc", ..sort }.store(&mut app.session.prefs);
            }
            if let Some(h) = p.get("palette").and_then(Value::as_bool) {
                crate::menus::set_palette(&mut app.ui, h);
            }
            // true / false, or a top-level menu to open it at ("Plugins").
            match p.get("mainMenu") {
                Some(Value::Bool(h)) => app.ui.main_menu = *h,
                Some(Value::String(top)) => {
                    app.ui.main_menu = true;
                    crate::menus::open_main_menu(ctx, top);
                }
                _ => {}
            }
            if let Some(h) = p.get("pluginsWindow").and_then(Value::as_bool) {
                app.ui.plugins_window = h;
            }
            // The Plugins window's tab ("installed", "browse"), and the plugin of the listed
            // directory whose install asks for confirmation (null: none).
            if let Some(tab) = p.get("pluginsTab").and_then(Value::as_str) {
                app.ui.plugins_manage.tab =
                    if tab == "browse" { crate::plugins_ui::PluginsTab::Browse } else { crate::plugins_ui::PluginsTab::Installed };
            }
            match p.get("pluginConfirm") {
                Some(Value::String(id)) => app.ui.plugins_manage.confirm = Some(id.clone()),
                Some(Value::Null) => app.ui.plugins_manage.confirm = None,
                _ => {}
            }
            // A plugin command's parameters form: {id, command?}; null closes it.
            match p.get("pluginForm") {
                Some(Value::Null) => app.ui.plugin_form = None,
                Some(v @ Value::Object(_)) => {
                    let id = v.get("id").and_then(Value::as_str).ok_or("pluginForm: {id, command?}")?;
                    let command = v.get("command").and_then(Value::as_str).unwrap_or_default();
                    crate::plugins_ui::start(app, id, command);
                }
                _ => {}
            }
            // Give a region the keyboard as F6 would: toolbar, layers, canvas or properties.
            if let Some(r) = p.get("focusRegion").and_then(Value::as_str) {
                app.ui.nav.go(crate::a11y::Region::parse(r).ok_or_else(|| format!("unknown region `{r}`"))?);
            }
            // A right-click on the canvas at screen [sx, sy] (null closes the menu);
            // `contextSelectLayer` opens its Select layer submenu.
            match p.get("contextMenu") {
                Some(Value::Array(at)) => {
                    let (x, y) = (at.first().and_then(Value::as_f64).unwrap_or(0.0), at.get(1).and_then(Value::as_f64).unwrap_or(0.0));
                    crate::canvas::open_context_menu(app, egui::pos2(x as f32, y as f32), b(p, "cmd"));
                }
                Some(Value::Null) => app.ui.context_menu = None,
                _ => {}
            }
            if let Some(v) = p.get("contextSelectLayer").and_then(Value::as_bool) {
                crate::menus::open_select_layer(ctx, v);
            }
            if let Some(h) = p.get("iconGallery").and_then(Value::as_bool) {
                app.ui.icon_gallery = h;
            }
            // Comment thread popover: an id opens it, null closes it.
            if let Some(v) = p.get("commentOpen") {
                match v.as_u64() {
                    Some(id) => {
                        // Pins are drawn for the current page only: reveal the thread's page first.
                        let page = app.session.doc().ok().and_then(|d| d.comment(id)).map(|c| c.page);
                        if let Some(page) = page.filter(|p| app.session.page().is_ok_and(|cur| cur != *p)) {
                            app.session.execute("page.select", &json!({"id": page.0})).map_err(|e| e.to_string())?;
                        }
                        app.ui.comments.show(id);
                    }
                    None => app.ui.comments.close(),
                }
            }
            if let Some(v) = p.get("commentDraft") {
                match (v.get("x").and_then(Value::as_f64), v.get("y").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) => crate::comments::start_draft(app, x, y),
                    _ => app.ui.comments.draft = None,
                }
            }
            if let Some(v) = p.get("showResolvedComments").and_then(Value::as_bool) {
                app.ui.comments.filter = if v { Filter::All } else { Filter::Open };
            }
            if let Some(v) = p.get("commentFilter").and_then(Value::as_str) {
                app.ui.comments.filter = Filter::parse(v).ok_or("commentFilter: open|resolved|mine|unread|all")?;
            }
            if let Some(v) = p.get("commentSort").and_then(Value::as_str) {
                app.ui.comments.sort = Sort::parse(v).ok_or("commentSort: date|page")?;
            }
            if let Some(v) = p.get("commentSearch").and_then(Value::as_str) {
                app.ui.comments.search = v.to_string();
            }
            // The open thread's reply field, with the cursor at its end (an `@` there offers names).
            if let Some(v) = p.get("commentReply").and_then(Value::as_str) {
                app.ui.comments.reply = v.to_string();
                if let Some(id) = app.ui.comments.open {
                    crate::comments::focus_reply(ctx, id, v.chars().count());
                }
            }
            // Edit a message of the open thread: its index (0: the first message); null stops.
            if let Some(v) = p.get("commentEdit") {
                app.ui.comments.editing = match (app.ui.comments.open, v.as_u64()) {
                    (Some(id), Some(i)) => {
                        let text = app.session.doc().ok().and_then(|d| d.comment(id)).and_then(|c| c.message(i as usize)).map(|m| m.text.clone());
                        Some((id, i as usize, text.ok_or("commentEdit: no such message")?))
                    }
                    _ => None,
                };
            }
            if let Some(v) = p.get("presentComments").and_then(Value::as_bool) {
                app.ui.present_view.comments = v;
                if let Some(pres) = app.ui.present.as_mut() {
                    pres.view.comments = v;
                }
            }
            // Colour picker on the selection's paint `{fills?, index?}`; null closes it.
            if let Some(v) = p.get("colorPicker") {
                app.ui.picker = v.is_object().then(|| crate::colorpicker::PickerTarget::Paint {
                    fills: v.get("fills").and_then(Value::as_bool).unwrap_or(true),
                    index: v.get("index").and_then(Value::as_u64).unwrap_or(0).min(1000) as usize,
                    ids: app.session.selection(),
                });
            }
            // The Variables editor: true / a collection (id or name) / {library, collection?} (an
            // enabled library's, by id or name) opens it, false closes it.
            match p.get("variables") {
                Some(Value::Bool(open)) => app.ui.variables = open.then(Default::default),
                Some(Value::String(c)) => app.ui.variables = Some(crate::variables_panel::Editor::on(Some(c.clone()))),
                Some(v @ Value::Object(_)) => {
                    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
                    app.ui.variables = Some(match text("library") {
                        Some(lib) => crate::variables_panel::Editor::in_library(lib, text("collection")),
                        None => crate::variables_panel::Editor::on(text("collection")),
                    });
                }
                _ => {}
            }
            // Layers panel search: a string opens it with that filter, null closes it.
            if let Some(v) = p.get("layerFilter") {
                app.ui.layer_search = v.is_string();
                app.ui.layer_filter = v.as_str().unwrap_or_default().chars().take(200).collect();
            }
            if p.get("collapseLayers").and_then(Value::as_bool) == Some(true) {
                app.ui.expanded.clear();
            }
            // The interaction editor popover in the Prototype panel: [layer id, index]; null closes it.
            match p.get("protoEditor") {
                Some(Value::Null) => crate::proto_panel::open_editor(ctx, None),
                Some(Value::Array(a)) => {
                    let id = a.first().and_then(Value::as_u64).ok_or("protoEditor: [layer id, interaction index]")?;
                    let k = a.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                    crate::proto_panel::open_editor(ctx, Some((figcraft_doc::NodeId(id), k)));
                }
                _ => {}
            }
            // The prototype preview pane: true / a frame id opens it, false or null closes it.
            match p.get("preview") {
                Some(Value::Bool(false) | Value::Null) => app.ui.preview = None,
                Some(v @ (Value::Bool(true) | Value::Number(_))) => {
                    let params = if v.is_number() { json!({"start": v}) } else { json!({}) };
                    app.session.execute("proto.preview", &params).map_err(|e| e.to_string())?;
                    app.handle_ui_requests();
                }
                _ => {}
            }
            // Scroll the presented screen (the full-window player, else the preview pane) to [x, y].
            if let Some(s) = p.get("presentScroll").and_then(Value::as_array) {
                let at = |i: usize| s.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                let player = match (app.ui.present.as_mut(), app.ui.preview.as_mut()) {
                    (Some(p), _) => Some(p),
                    (None, Some(pane)) => Some(&mut pane.player),
                    _ => None,
                };
                player.ok_or("nothing is presented")?.scroll_to(at(0), at(1));
            }
            // The style picker: a slot (fill, stroke, effect, grid, text) opens it for the selection, null
            // closes it; `stylePickerEdit` (a style id or name) opens its editor; `stylePickerView`
            // is `list` or `grid`.
            if let Some(v) = p.get("stylePicker") {
                match v.as_str().and_then(crate::style_picker::StyleSlot::parse) {
                    Some(slot) => crate::style_picker::open_for_selection(app, slot, p.get("stylePickerEdit").and_then(Value::as_str)),
                    None => app.ui.style_picker = None,
                }
            }
            if let Some(v) = p.get("stylePickerView").and_then(Value::as_str) {
                app.ui.style_grid = v == "grid";
            }
            // The Color profile dialog: a profile id opens it with that profile chosen, null closes it.
            if let Some(v) = p.get("colorProfileDialog") {
                app.ui.color_profile = v.as_str().and_then(figcraft_color::ColorProfile::parse);
            }
            if let Some(v) = p.get("exportPreview").and_then(Value::as_bool) {
                app.ui.export_thumb = v;
            }
            // Dev Mode's compare view: a frame or section opens it, null closes it; `devCompareVersion`
            // the saved version it compares with (null: when it was marked ready for dev),
            // `devCompareMode` is sideBySide or overlay, `devCompareSplit` the overlay slider (0–1).
            if let Some(v) = p.get("devCompare") {
                app.ui.dev.compare = figcraft_engine::params::id_of(v).map(|id| crate::dev_compare::CompareView::new(id, None));
            }
            if let Some(c) = app.ui.dev.compare.as_mut() {
                if let Some(v) = p.get("devCompareVersion") {
                    c.version = v.as_str().map(str::to_string);
                }
                match p.get("devCompareMode").and_then(Value::as_str) {
                    Some("overlay") => c.mode = crate::dev_compare::Mode::Overlay,
                    Some("sideBySide") => c.mode = crate::dev_compare::Mode::SideBySide,
                    Some(_) => return Err("devCompareMode: sideBySide|overlay".into()),
                    None => {}
                }
                if let Some(x) = p.get("devCompareSplit").and_then(Value::as_f64).filter(|x| x.is_finite()) {
                    c.split = x.clamp(0.0, 1.0) as f32;
                }
            }
            // The branch review dialog: a branch id opens it, null closes it; `branchReviewPicks`
            // chooses sides ({<conflict key>: "branch" | "main"}).
            if let Some(v) = p.get("branchReview") {
                match v.as_str() {
                    Some(id) => crate::branch_ui::open(app, Some(id)),
                    None => app.ui.branch_review = None,
                }
            }
            if let (Some(r), Some(picks)) = (app.ui.branch_review.as_mut(), p.get("branchReviewPicks").and_then(Value::as_object)) {
                for (k, v) in picks {
                    let side = v.as_str().and_then(figcraft_engine::branches::Side::parse).ok_or("branchReviewPicks: \"branch\" or \"main\"")?;
                    r.picks.insert(k.clone(), side);
                }
            }
            // Which annotations the canvas shows: all, none (uncategorised) or a category.
            if let Some(v) = p.get("annotationFilter").and_then(Value::as_str) {
                let doc = app.session.doc().map_err(|e| e.to_string())?;
                app.ui.dev.categories = crate::dev_mode::CategoryFilter::parse(doc, v).ok_or("annotationFilter: all|none|a category id or name")?;
            }
            if b(p, "resetFrameStats") {
                app.frame_stats.reset();
            }
            Ok(app.inspect())
        }
        "ui.render" => {
            let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
            let ids: Vec<figcraft_doc::NodeId> =
                p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(figcraft_engine::params::id_of).collect()).unwrap_or_default();
            let mut params = json!({"format": "png", "scale": scale});
            if let Some(path) = p.get("path").and_then(Value::as_str) {
                params["path"] = json!(path);
            }
            if !ids.is_empty() {
                params["ids"] = json!(ids);
            }
            app.session.execute("file.export", &params).map_err(|e| e.to_string())
        }
        "app.quit" => {
            app.ui.quit = true;
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            Ok(json!({"ok": true}))
        }
        other => Err(format!("unknown method `{other}`")),
    }
}

fn pointer(app: &mut FigApp, p: &Value) -> Result<Value, String> {
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let at = |x: Option<f64>, y: Option<f64>, sx: Option<f64>, sy: Option<f64>| -> Option<(f64, f64)> {
        match (x, y, sx, sy) {
            (Some(x), Some(y), _, _) => Some((x, y)),
            (_, _, Some(sx), Some(sy)) => {
                let pt = app.to_page(egui::pos2(sx as f32, sy as f32));
                Some((pt.x, pt.y))
            }
            _ => None,
        }
    };
    let f = |k: &str| p.get(k).and_then(Value::as_f64);
    let (x, y) = at(f("x"), f("y"), f("sx"), f("sy")).ok_or("pass x, y (page) or sx, sy (screen)")?;
    let pressure = |k: &str| f(k).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
    let mk = |kind: PointerKind, x: f64, y: f64| PointerEvent {
        kind,
        x,
        y,
        sx: x * zoom,
        sy: y * zoom,
        shift: b(p, "shift"),
        alt: b(p, "alt"),
        cmd: b(p, "cmd"),
        ctrl: b(p, "ctrl"),
        zoom,
        pressure: pressure("pressure"),
    };
    // A drag's pressure goes from `pressure` to `pressureTo`.
    let (pr0, pr1) = (pressure("pressure"), pressure("pressureTo").or(pressure("pressure")));
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("click");
    let run = |app: &mut FigApp, ev: PointerEvent| app.session.pointer(&ev).map_err(|e| e.to_string());
    match kind {
        "down" => run(app, mk(PointerKind::Down, x, y))?,
        "move" => run(app, mk(PointerKind::Move, x, y))?,
        "up" => run(app, mk(PointerKind::Up, x, y))?,
        "hover" => run(app, mk(PointerKind::Hover, x, y))?,
        "double" => run(app, mk(PointerKind::DoubleClick, x, y))?,
        "click" => {
            run(app, mk(PointerKind::Down, x, y))?;
            run(app, mk(PointerKind::Up, x, y))?;
        }
        "drag" => {
            let to = p.get("to").and_then(Value::as_array).ok_or("drag needs `to: [x, y]`")?;
            let tx = to.first().and_then(Value::as_f64).ok_or("bad `to`")?;
            let ty = to.get(1).and_then(Value::as_f64).ok_or("bad `to`")?;
            let steps = p.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 200);
            run(app, mk(PointerKind::Down, x, y))?;
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let pressure = pr0.zip(pr1).map(|(a, b)| a + (b - a) * t as f32);
                run(app, PointerEvent { pressure, ..mk(PointerKind::Move, x + (tx - x) * t, y + (ty - y) * t) })?;
            }
            run(app, PointerEvent { pressure: pr1, ..mk(PointerKind::Up, tx, ty) })?;
        }
        other => return Err(format!("unknown pointer kind `{other}`")),
    }
    app.handle_ui_requests();
    Ok(json!({"selection": app.session.selection(), "tool": app.session.tool.tool.id()}))
}

#[cfg(test)]
#[path = "control_roots_tests.rs"]
mod roots_tests;
