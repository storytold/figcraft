//! Keyboard handling: the shortcuts of [`figcraft_engine::keymap`], tool keys, nudging, opacity
//! digits and text editing keys.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Context, Event, Key, Modifiers};
use figcraft_engine::keymap::{self, Platform};
use serde_json::json;

use crate::FigApp;

/// Two opacity digits typed within this long make one exact percentage ("4" then "5": 45%).
pub const OPACITY_CHAIN_SECS: f64 = 0.6;

/// 0: not known yet (use the build's platform), 1: macOS, 2: Windows/Linux.
static PLATFORM: AtomicU8 = AtomicU8::new(0);

/// The platform whose keys apply: the build's, or in the browser the one egui detected.
pub fn platform() -> Platform {
    match PLATFORM.load(Ordering::Relaxed) {
        1 => Platform::Mac,
        2 => Platform::Other,
        _ => Platform::current(),
    }
}

fn note_platform(ctx: &Context) {
    PLATFORM.store(if ctx.os() == egui::os::OperatingSystem::Mac { 1 } else { 2 }, Ordering::Relaxed);
}

/// The keymap's view of egui's modifiers (Ctrl off macOS sets egui's `command` too).
pub fn mods(m: Modifiers) -> keymap::Mods {
    keymap::Mods { shift: m.shift, alt: m.alt, cmd: m.command || m.mac_cmd, ctrl: m.ctrl }
}

/// Symbols a US keyboard types without Shift.
const UNSHIFTED_SYMBOLS: [Key; 11] = [
    Key::Slash,
    Key::Backslash,
    Key::Minus,
    Key::Equals,
    Key::Period,
    Key::Comma,
    Key::Quote,
    Key::Semicolon,
    Key::OpenBracket,
    Key::CloseBracket,
    Key::Backtick,
];

/// The binding a key fires in `context`. Shortcuts follow the character typed, named as on a US
/// keyboard: a symbol another layout types with Shift (`/` is Shift+7 on a German keyboard, where
/// the key's position differs from the symbol) counts without the Shift. Characters egui has no key
/// for (`ü`, `ж`) already arrive as the key at that position.
pub fn lookup(context: keymap::Context, key: Key, physical: Option<Key>, m: Modifiers) -> Option<&'static keymap::Binding> {
    let mut held = mods(m);
    if held.shift && physical.is_some_and(|p| p != key) && UNSHIFTED_SYMBOLS.contains(&key) {
        held.shift = false;
    }
    keymap::lookup(platform(), context, key.name(), held)
}

/// The key a menu shows for a command.
pub fn shortcut(id: &str) -> Option<&'static str> {
    keymap::primary(id, platform())
}

/// A tooltip naming a command's key on this platform ("Align left  ⌥A"; just the label unbound).
pub fn tip(label: &'static str, id: &str) -> &'static str {
    cached_tip(label, shortcut(id))
}

/// A tooltip naming a key ("Resources  ⇧I").
pub fn key_tip(label: &'static str, key: &'static str) -> &'static str {
    cached_tip(label, Some(key))
}

/// Panels ask for their tooltips every frame: each (label, key, platform) is built once.
fn cached_tip(label: &'static str, key: Option<&'static str>) -> &'static str {
    type Entry = (Platform, &'static str, &'static str, &'static str);
    static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    let Some(key) = key else { return label };
    let p = platform();
    let Ok(mut cache) = CACHE.lock() else { return label };
    if let Some(e) = cache.iter().find(|e| e.0 == p && e.1 == label && e.2 == key) {
        return e.3;
    }
    // A bounded set: one entry per tooltip in the code.
    let text: &'static str = Box::leak(format!("{label}  {}", keymap::display(key, p)).into_boxed_str());
    cache.push((p, label, key, text));
    text
}

/// Run the shortcuts this frame's keys press. `typing`: a field had the keyboard when the frame
/// began.
/// `swallowed`: what this frame's Cmd/Ctrl+V chords the shell swallowed ask for ([`swallowed_paste`]).
pub fn handle(app: &mut FigApp, ctx: &Context, typing: bool, swallowed: Swallowed) {
    note_platform(ctx);
    // A save asked for from a field last frame: the field has committed by now.
    if let Some(id) = app.ui.after_field.take() {
        run_binding(app, id);
    }
    // Text fields in panels own the keyboard (also the key that ends their editing), except for
    // the save shortcuts.
    if typing || crate::a11y::field_focused(ctx) || app.ui.palette.is_some() {
        if app.ui.palette.is_none() {
            save_from_field(app, ctx);
        }
        return;
    }
    let events: Vec<Event> = ctx.input(|i| i.events.clone());
    // Every frame while editing, even one without input: a frame without the IME output turns the
    // platform's IME off, which drops the composition (macOS then types plain letters).
    let editing = app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false);
    if editing {
        text_keys(app, ctx, &events);
        return;
    }
    if events.is_empty() {
        return;
    }
    // An image-only clipboard brings no paste event: paste from the key then. The browser sends
    // the press; the desktop shell swallows it and sends only the release.
    let paste_event = events.iter().any(|e| matches!(e, Event::Paste(_)));
    let paste_key = events.iter().any(|e| matches!(e, Event::Key { key: Key::V, pressed: true, repeat: false, modifiers, .. } if modifiers.command && v_binding(*modifiers) == Some("edit.paste")));
    match swallowed.run.or((paste_key && !paste_event).then_some("edit.paste")) {
        Some("edit.paste") => crate::menus::paste(app, None, None),
        Some(id) => run_binding(app, id),
        None => {}
    }
    let now = ctx.input(|i| i.time);
    for ev in &events {
        match ev {
            Event::Copy => crate::menus::invoke(app, "edit.copy"),
            Event::Cut => crate::menus::invoke(app, "edit.cut"),
            Event::Paste(text) if !swallowed.drop_text => crate::menus::paste(app, Some(text), None),
            Event::Key { key, physical_key, pressed: true, modifiers, repeat }
                if key_event(app, *key, *physical_key, *modifiers, *repeat, now, ctx) =>
            {
                // handled
            }
            _ => {}
        }
    }
}

/// Commands whose shortcut works while a panel field has the keyboard: they don't edit text, and
/// a file that can't be saved from a field looks like a file that can't be saved (#1157).
const FROM_A_FIELD: [&str; 2] = ["file.save", "file.saveAs"];

/// Cmd/Ctrl+S (or Shift+Cmd/Ctrl+S) pressed in a panel field: the field lets go of the keyboard,
/// which commits what was typed as clicking away does, and the save runs next frame, after that
/// commit.
fn save_from_field(app: &mut FigApp, ctx: &Context) {
    let id = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } if modifiers.command => {
                lookup(keymap::Context::Canvas, *key, *physical_key, *modifiers).map(|b| b.id).filter(|id| FROM_A_FIELD.contains(id))
            }
            _ => None,
        })
    });
    if let Some(id) = id {
        ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
        app.ui.after_field = Some(id);
        ctx.request_repaint();
    }
}

/// The command Cmd/Ctrl+V runs with `m` held too: `edit.paste`, `edit.pasteOver` (Shift),
/// `edit.pasteProperties` (Alt), or `None` for a chord nothing is bound to. The keymap decides,
/// whether the key press arrived (browser) or only a paste event and the release (desktop).
fn v_binding(m: Modifiers) -> Option<&'static str> {
    lookup(keymap::Context::Canvas, Key::V, Some(Key::V), Modifiers { command: true, ..m }).map(|b| b.id)
}

/// What this frame's Cmd/Ctrl+V chords ask for when the desktop shell swallowed their press
/// ([`swallowed_paste`]).
#[derive(Clone, Copy, Default)]
pub struct Swallowed {
    /// The command to run ([`v_binding`]).
    pub run: Option<&'static str>,
    /// This frame's paste event belongs to another command or an unbound chord: its text isn't
    /// pasted.
    pub drop_text: bool,
}

/// egui-winit swallows every Cmd/Ctrl+V press, Shift or Alt held or not, and sends a paste event
/// only when the clipboard holds text. So a chord brings a paste event (text) or just its release
/// (an image, or nothing). The browser sends the press as well, which runs the command itself.
/// Call at the start of every frame: it reads the raw events, which keys consumed by panels or the
/// presenter still carry.
pub fn swallowed_paste(app: &mut FigApp, ctx: &Context) -> Swallowed {
    let ui = &mut app.ui;
    let mut out = Swallowed::default();
    ctx.input(|i| {
        for ev in &i.raw.events {
            match ev {
                Event::ModifiersChanged(m) => ui.held = *m,
                // Without Cmd/Ctrl held it's another paste key (Shift+Insert, a Paste key).
                Event::Paste(_) if ui.held.command => {
                    let id = v_binding(ui.held);
                    if id != Some("edit.paste") {
                        out.drop_text = true;
                        // Held down, the chord repeats its paste event: run once, as keys do.
                        if !ui.v_pressed && !ui.paste_event {
                            out.run = out.run.or(id);
                        }
                    }
                    ui.paste_event = true;
                }
                Event::Paste(_) => ui.paste_event = true,
                Event::Key { key: Key::V, pressed: true, .. } => ui.v_pressed = true,
                Event::Key { key: Key::V, pressed: false, modifiers, .. } => {
                    // Cmd was down when V went down, maybe let go first. A V held while the window
                    // was away comes back as a release alone: without Cmd held, that's no paste.
                    if !ui.v_pressed && !ui.paste_event && (modifiers.command || !ui.refocused) {
                        out.run = out.run.or(v_binding(*modifiers));
                    }
                    ui.v_pressed = false;
                    ui.paste_event = false;
                    ui.refocused = false;
                }
                // The other keys that paste (Shift+Insert, a Paste key) end their paste event.
                Event::Key { key: Key::Insert | Key::Paste, pressed: false, .. } => ui.paste_event = false,
                // Keys let go in another window never come back as releases.
                Event::WindowFocused(false) => {
                    ui.v_pressed = false;
                    ui.paste_event = false;
                    ui.held = Modifiers::NONE;
                }
                Event::WindowFocused(true) => ui.refocused = true,
                _ => {}
            }
        }
    });
    out
}

/// Run the command a shortcut is bound to, with the keyboard's own twists (Escape steps out
/// before it deselects, Delete ends a pen path, Enter edits the selected text or vector).
pub fn run_binding(app: &mut FigApp, id: &str) {
    // Dev Mode places and edits nothing: Enter selects children.
    let dev = app.ui.right_tab == crate::RightTab::Inspect;
    match id {
        "select.none" => {
            if let Some(ctx) = &app.ui_context
                && crate::proto_panel::close_editor(ctx)
            {
                return;
            }
            if let Err(e) = app.session.escape() {
                app.toast(e.to_string());
            }
            app.ui.context_menu = None;
        }
        "edit.delete" if app.session.tool.pen.is_some() => {
            let _ = app.session.pen_finish();
        }
        // Enter with a shape or frame tool chosen places one in the middle of the view.
        "select.children" if !dev && app.session.tool.tool.draws_shape() => {
            app.run("tool.place", json!({}));
        }
        "select.children" if !dev && enter_text(app) => {}
        _ => crate::menus::invoke(app, id),
    }
}

fn key_event(app: &mut FigApp, key: Key, physical: Option<Key>, m: Modifiers, repeat: bool, now: f64, ctx: &Context) -> bool {
    if key == Key::Escape && crate::proto_panel::close_editor(ctx) {
        return true;
    }
    if key == Key::Escape && crate::selection_spacing::cancel(app, ctx) {
        return true;
    }
    if key_box(app, key, m) {
        return true;
    }
    // A gradient open in the colour picker: Delete removes its selected stop.
    if matches!(key, Key::Backspace | Key::Delete) && m.is_none() && app.ui.picker.is_some() && crate::handles::remove_selected_stop(app, ctx) {
        return true;
    }
    // An interaction selected in the Prototype tab: Delete removes that interaction, not the component.
    if matches!(key, Key::Backspace | Key::Delete) && m.is_none() && crate::proto_panel::delete_selected_interaction(app, ctx) {
        return true;
    }
    // Vector edit mode and the shape builder have their own keys (tools, nudging points, delete,
    // join, Enter to finish); the rest fall through.
    match app.session.tool_key(key.name(), m.shift, m.alt, m.command) {
        Ok(true) => return true,
        Ok(false) => {}
        Err(e) => {
            app.toast(e.to_string());
            return true;
        }
    }
    if let Some(b) = lookup(keymap::Context::Canvas, key, physical, m) {
        // Copy/cut/paste arrive as clipboard events on most platforms.
        if matches!(b.id, "edit.copy" | "edit.cut" | "edit.paste") {
            return true;
        }
        if !repeat || matches!(b.id, "edit.undo" | "edit.redo" | "view.zoomIn" | "view.zoomOut") {
            if matches!(b.id, "edit.undo" | "edit.redo") && crate::selection_spacing::cancel(app, ctx) {
                return true;
            }
            run_binding(app, b.id);
        }
        return true;
    }
    // Arrows move through the layers panel while it has the keyboard.
    if app.ui.layers_focus && !m.command && !m.alt && crate::layers::key_nav(app, key, m.shift) {
        return true;
    }
    // Arrow nudges, Alt held too (measuring stays on while the selection moves); with nothing
    // selected the arrows pan the view.
    let (dx, dy) = match key {
        Key::ArrowLeft => (-1.0, 0.0),
        Key::ArrowRight => (1.0, 0.0),
        Key::ArrowUp => (0.0, -1.0),
        Key::ArrowDown => (0.0, 1.0),
        _ => (0.0, 0.0),
    };
    if (dx, dy) != (0.0, 0.0) {
        if app.session.selection().is_empty() {
            app.run("view.pan", json!({"dx": dx, "dy": dy, "big": m.shift}));
        } else if crate::menus::dev_mode_refuses(app, "transform.nudge") {
            // Dev Mode: said so, nothing moves.
        } else if m.command {
            // Cmd/Ctrl+arrows resize: right/down grow, left/up shrink.
            app.run("transform.nudgeSize", json!({"dw": dx, "dh": dy, "big": m.shift}));
        } else {
            app.run("transform.nudge", json!({"dx": dx, "dy": dy, "big": m.shift}));
        }
        return true;
    }
    if m.is_none() && !repeat && opacity_digit(app, key, now) {
        return true;
    }
    // Enter while the pen is drawing ends the path as Escape does.
    if key == Key::Enter && app.session.tool.pen.is_some() {
        let _ = app.session.pen_end();
        return true;
    }
    false
}

/// Keys while a selection box drawn from the keyboard shows (`select.area`, Alt+Space or Ctrl+Space,
/// F8): the arrows move it, Cmd/Ctrl+arrows or Shift+arrows move its far corner, sizing it (Shift
/// with Cmd/Ctrl: five times as far), by 10 screen pixels; Enter selects what it covers like a
/// marquee (Shift toggles, Cmd/Ctrl takes nested layers), Escape drops it.
fn key_box(app: &mut FigApp, key: Key, m: Modifiers) -> bool {
    let Some(mut b) = app.ui.key_box else { return false };
    let size = m.shift || m.command;
    let step = if m.shift && m.command { 50.0 } else { 10.0 } / app.session.ds().map_or(1.0, |d| d.view.zoom.max(1e-6));
    let (dx, dy) = match key {
        Key::ArrowLeft => (-step, 0.0),
        Key::ArrowRight => (step, 0.0),
        Key::ArrowUp => (0.0, -step),
        Key::ArrowDown => (0.0, step),
        _ => (0.0, 0.0),
    };
    if (dx, dy) != (0.0, 0.0) {
        if !size {
            b[0] += dx;
            b[1] += dy;
        }
        b[2] += dx;
        b[3] += dy;
        app.ui.key_box = Some(b);
        return true;
    }
    match key {
        Key::Enter => {
            app.ui.key_box = None;
            let [x0, y0, x1, y1] = b;
            app.run("select.area", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "add": m.shift, "deep": m.command}));
            true
        }
        Key::Escape => {
            app.ui.key_box = None;
            true
        }
        _ => false,
    }
}

impl FigApp {
    /// Show a selection box for the keyboard: around the selection, else in the middle of the view.
    pub(crate) fn start_key_box(&mut self) {
        let Ok(ds) = self.session.ds() else { return };
        let zoom = ds.view.zoom.max(1e-6);
        let around = ds.selection.iter().filter_map(|i| ds.doc.world_bounds(*i)).reduce(|a, b| a.union(b));
        let r = match around {
            Some(r) => r.inflate(8.0 / zoom, 8.0 / zoom),
            None => {
                let c = self.to_page(self.canvas.rect.center());
                figcraft_geom::Rect::new(c.x - 100.0 / zoom, c.y - 75.0 / zoom, c.x + 100.0 / zoom, c.y + 75.0 / zoom)
            }
        };
        self.ui.key_box = Some([r.x0, r.y0, r.x1, r.y1]);
        let cmd = crate::toolbar::pretty_shortcut("Cmd");
        self.toast(crate::i18n::trf(
            "Selection box: arrows move it, Shift+arrows or {cmd}+arrows size it, Enter selects, Escape cancels",
            &[("cmd", &cmd)],
        ));
    }
}

/// The digit a number key types.
fn digit(key: Key) -> Option<u32> {
    let name = key.name();
    let mut chars = name.chars();
    let d = chars.next()?.to_digit(10)?;
    chars.next().is_none().then_some(d)
}

/// Number keys set the selection's opacity: one digit is tens of percent (0: 100%), a second digit
/// typed soon after makes the exact percentage.
pub fn opacity_digit(app: &mut FigApp, key: Key, now: f64) -> bool {
    let Some(d) = digit(key) else { return false };
    if !app.session.prefs.number_keys_opacity || app.session.selection().is_empty() {
        return false;
    }
    if crate::menus::dev_mode_refuses(app, "object.opacityKeys") {
        return true;
    }
    let first = app.ui.opacity_digit.take().filter(|(_, t)| now - t <= OPACITY_CHAIN_SECS).map(|(d, _)| d);
    let digits = match first {
        Some(f) => format!("{f}{d}"),
        None => {
            app.ui.opacity_digit = Some((d, now));
            d.to_string()
        }
    };
    app.run("object.opacityKeys", json!({"digits": digits}));
    true
}

/// Enter on a single selected text layer starts editing it; on shapes, edits them as vectors.
fn enter_text(app: &mut FigApp) -> bool {
    let sel = app.session.selection();
    if sel.len() > 1 {
        let ids = app.session.doc().ok().and_then(|d| figcraft_engine::vectoredit::enter_targets(d, &sel));
        return ids.is_some_and(|ids| {
            app.run("vector.edit.begin", json!({"ids": ids}));
            true
        });
    }
    if sel.len() != 1 {
        return false;
    }
    let Some(id) = sel.first().copied() else { return false };
    // A board object (a sticky, a shape with text, a label): type in its text.
    match figcraft_engine::board::edit_text(&mut app.session, id) {
        Ok(true) => return true,
        Ok(false) => {}
        Err(e) => {
            app.toast(e.to_string());
            return true;
        }
    }
    let kind = app.session.doc().ok().and_then(|d| d.node(id)).map(|n| (matches!(n.kind, figcraft_doc::NodeKind::Text(_)), n.kind.is_shape()));
    match kind {
        Some((true, _)) => {
            let _ = figcraft_engine::textedit::begin(&mut app.session, id, None, false);
            true
        }
        Some((_, true)) => {
            app.run("vector.edit.begin", json!({"id": id}));
            true
        }
        _ => false,
    }
}

fn text_keys(app: &mut FigApp, ctx: &Context, events: &[Event]) {
    // A key that ran a shortcut swallows the text it also produced (Alt+. types "≥" on macOS).
    let mut skip_text = false;
    for ev in events {
        match ev {
            Event::Text(t) | Event::Ime(egui::ImeEvent::Commit(t)) => {
                if skip_text && matches!(ev, Event::Text(_)) {
                    skip_text = false;
                    continue;
                }
                if let Err(e) = figcraft_engine::textedit::insert(&mut app.session, t) {
                    app.toast(e.to_string());
                }
            }
            // The composition shows inline at the caret; the document gets it on Commit.
            Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                let cursor = active_range_chars.as_ref().map(|r| r.start);
                if let Err(e) = figcraft_engine::textedit::preedit(&mut app.session, text, cursor) {
                    app.toast(e.to_string());
                }
            }
            Event::Paste(t) => {
                // Plain text only: the engine normalises line endings and drops control chars.
                if let Err(e) = figcraft_engine::textedit::insert(&mut app.session, t) {
                    app.toast(e.to_string());
                }
            }
            Event::Copy | Event::Cut => {
                if let Some(text) = figcraft_engine::textedit::selected_text(&app.session).filter(|t| !t.is_empty()) {
                    app.services.clipboard_set_text(&text);
                    if matches!(ev, Event::Cut) {
                        let _ = figcraft_engine::textedit::key(&mut app.session, "Delete", false, false, false);
                    }
                }
            }
            Event::Key { key, physical_key, pressed: true, modifiers: m, .. } => {
                // Keys during a composition are the IME's (Enter picks a candidate, Escape cancels).
                if app.session.ds().is_ok_and(|d| d.text_edit.as_ref().is_some_and(|te| te.preedit.is_some())) {
                    continue;
                }
                if (m.command || m.alt || m.ctrl) && text_shortcut(app, *key, *physical_key, *m) {
                    skip_text = true;
                    continue;
                }
                let name = match key {
                    Key::Backspace => "Backspace",
                    Key::Delete => "Delete",
                    Key::ArrowLeft => "Left",
                    Key::ArrowRight => "Right",
                    Key::ArrowUp => "Up",
                    Key::ArrowDown => "Down",
                    Key::Home => "Home",
                    Key::End => "End",
                    Key::Enter => "Enter",
                    Key::Escape => "Escape",
                    Key::Tab => "Tab",
                    Key::A if m.command => "SelectAll",
                    Key::Z if m.command => {
                        let _ = figcraft_engine::textedit::end(&mut app.session);
                        app.run(if m.shift { "edit.redo" } else { "edit.undo" }, json!({}));
                        continue;
                    }
                    _ => continue,
                };
                if name == "Enter" && m.command {
                    let _ = figcraft_engine::textedit::end(&mut app.session);
                    continue;
                }
                let (word, line) = keymap::text_motion(platform(), name, mods(*m));
                if let Err(e) = figcraft_engine::textedit::key(&mut app.session, name, m.shift, word, line) {
                    app.toast(e.to_string());
                }
            }
            _ => {}
        }
    }
    // Triple-click on the edited text selects its line (double-click is handled by the engine).
    if ctx.input(|i| i.pointer.button_triple_clicked(egui::PointerButton::Primary))
        && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
        && app.canvas.rect.contains(pos)
        && let Some(te) = app.session.ds().ok().and_then(|d| d.text_edit.clone())
    {
        let p = app.to_page(pos);
        let inside = app.session.doc().ok().and_then(|d| d.world_bounds(te.id)).is_some_and(|b| b.contains(p));
        if inside && let Ok(idx) = app.session.text_index_at(te.id, p) {
            app.run("text.edit.selectLine", json!({"index": idx}));
        }
    }
    app.sync_preedit();
    ime_output(app, ctx);
}

/// Shortcuts live while editing text (Bold, lists, alignment, size/weight/spacing steps, Save…).
fn text_shortcut(app: &mut FigApp, key: Key, physical: Option<Key>, m: Modifiers) -> bool {
    match lookup(keymap::Context::Text, key, physical, m) {
        Some(b) => {
            crate::menus::invoke(app, b.id);
            true
        }
        None => false,
    }
}

/// Tell the platform where the caret is (inside the composition while composing) so IME
/// candidate windows open next to it.
fn ime_output(app: &FigApp, ctx: &Context) {
    let Some(te) = app.session.ds().ok().and_then(|d| d.text_edit.clone()) else { return };
    let preedit = app.preedit_view();
    let Some(doc) = preedit.map(|p| &*p.doc).or_else(|| app.session.doc().ok()) else { return };
    let Some(node) = doc.node(te.id) else { return };
    let Some((layout, dy)) = figcraft_text::layout_node(node) else { return };
    let w = doc.world_transform(te.id);
    let (cx, li) = layout.caret(preedit.map(|p| p.cursor).unwrap_or(te.caret));
    let (top, h) = layout.lines.get(li).map(|l| (l.top, l.height)).unwrap_or((0.0, 14.0));
    let p0 = w * figcraft_geom::Point::new(cx, top + dy);
    let p1 = w * figcraft_geom::Point::new(cx, top + h + dy);
    let (a, b) = (app.to_screen(p0.x, p0.y), app.to_screen(p1.x, p1.y));
    if !(a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()) {
        return;
    }
    let cursor_rect = egui::Rect::from_two_pos(a, b + egui::vec2(1.0, 0.0));
    ctx.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            // The desktop places the candidate window by `rect` (the web by `cursor_rect`).
            rect: cursor_rect,
            cursor_rect,
            should_interrupt_composition: false,
        })
    });
}
