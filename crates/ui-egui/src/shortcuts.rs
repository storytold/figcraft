//! Keyboard handling: the shortcuts of [`figcraft_engine::keymap`], tool keys, nudging, opacity
//! digits and text editing keys.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::ime_context::{CommittedText, ImeMutationStamp, ImeObservation, ImeTarget};
use egui::{Context, Event, Key, Modifiers};
use figcraft_engine::keymap::{self, Platform};
use figcraft_engine::textedit::ImeInsertion;
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
/// `paste`: what this frame's paste events ask for ([`paste_events`]).
pub fn handle(app: &mut FigApp, ctx: &Context, typing: bool, paste: PasteEvents) {
    let ime_input = ImeInputBaseline::capture(app);
    let field_save = prepare_field_save(app, ctx);
    handle_input(app, ctx, typing, paste, ime_input, field_save);
}

/// The viewport frame binds its IME batch before pointer/control handling.
pub(crate) fn handle_input(
    app: &mut FigApp,
    ctx: &Context,
    typing: bool,
    paste: PasteEvents,
    ime_input: Option<ImeInputBaseline>,
    field_save: Option<AfterFieldSave>,
) {
    note_platform(ctx);
    // egui reruns a discarded pass with the same `RawInput`. The first pass already applied its
    // keys and text (to our own editor state, not to a widget that re-reads the event): applying
    // them again would insert text twice or run a command twice. The rerun is layout only, so it
    // keeps just the IME's caret/output of the pass that will be presented.
    if ctx.current_pass_index() > 0 {
        if app.session.ds().is_ok_and(|d| d.text_edit.is_some()) {
            // The drawn pass still needs the composition preview; the IME descriptor is observed
            // once at the end of the pass (see `FigApp::frame`).
            app.sync_preedit();
        }
        return;
    }
    // The shared field helpers already processed this batch and committed their buffers. An
    // editor outside those helpers retains the old late focus-release behavior; never drop its
    // current Text events by releasing it before it draws. Always clear a disappeared marker.
    let marker = field_save_marker(ctx);
    let late_editor = ctx.data_mut(|d| {
        let pending = d.get_temp::<(egui::Id, u64)>(marker);
        d.remove::<(egui::Id, u64)>(marker);
        pending
    });
    if let Some((editor, frame)) = late_editor
        && frame == ctx.cumulative_frame_nr()
    {
        ctx.memory_mut(|m| m.surrender_focus(editor));
    }
    // A save asked for from a field last frame: the field committed in that same pass, before
    // this frame's tab/pointer input could switch documents. New requests are queued below.
    if let Some(save) = app.ui.after_field.take() {
        save.run(app);
    }
    // Text fields in panels own the keyboard (also the key that ends their editing), except for
    // the save shortcuts.
    // The IME is observed once at the end of the pass; `FigApp::frame` marks this editor idle there.
    if typing || crate::a11y::field_focused(ctx) || app.ui.palette.is_some() || field_save.is_some() {
        if app.ui.palette.is_none()
            && let Some(save) = field_save
        {
            app.ui.after_field = Some(save);
            ctx.request_repaint();
        }
        return;
    }
    let events: Vec<Event> = ctx.input(|i| i.events.clone());
    // Every frame while editing, even one without input: a frame without the IME output turns the
    // platform's IME off, which drops the composition (macOS then types plain letters).
    let editing = app.session.ds().map(|d| d.text_edit.is_some()).unwrap_or(false);
    if editing {
        text_keys(app, ctx, &events, ime_input);
        return;
    }
    if events.is_empty() {
        return;
    }
    // An image-only clipboard brings no paste event with text: the desktop shell sends an image
    // paste event instead, and in the browser it's the key press that pastes.
    let paste_event = events.iter().any(|e| matches!(e, Event::Paste(_)));
    let paste_key = events.iter().any(|e| matches!(e, Event::Key { key: Key::V, pressed: true, repeat: false, modifiers, .. } if modifiers.command && v_binding(*modifiers) == Some("edit.paste")));
    if paste.image || (paste_key && !paste_event) {
        crate::menus::paste(app, None, None);
    }
    let now = ctx.input(|i| i.time);
    for ev in &events {
        match ev {
            // egui-winit sends Cmd/Ctrl+C as a copy event whatever Alt does, so Copy properties
            // (Alt+Cmd/Ctrl+C) arrives here and never as a key.
            Event::Copy => {
                let props = ctx.input(|i| i.modifiers.alt);
                crate::menus::invoke(app, if props { "edit.copyProperties" } else { "edit.copy" });
            }
            Event::Cut => cut(app),
            Event::Paste(text) if !paste.drop_text => crate::menus::paste(app, Some(text), None),
            Event::Key { key, physical_key, pressed: true, modifiers, repeat }
                if key_event(app, *key, *physical_key, *modifiers, *repeat, now, ctx) =>
            {
                // handled
            }
            _ => {}
        }
    }
}

/// Cut: usually the layers. Windows sends Shift+Delete (remove the points and heal the path)
/// as a cut command and never as a key, so in vector edit mode the selection is points: they are
/// removed, a heal joining the path through them.
fn cut(app: &mut FigApp) {
    let Some(ve) = app.session.ds().ok().and_then(|d| d.vector_edit.clone()) else {
        crate::menus::invoke(app, "edit.cut");
        return;
    };
    if ve.parts().iter().any(|p| !p.vertices.is_empty()) {
        app.run("vector.deleteHeal", json!({}));
    } else if ve.parts().iter().any(|p| !p.segments.is_empty()) {
        app.run("vector.edit.delete", json!({}));
    }
}

/// Commands whose shortcut works while a panel field has the keyboard: they don't edit text, and
/// a file that can't be saved from a field looks like a file that can't be saved (#1157).
const FROM_A_FIELD: [&str; 2] = ["file.save", "file.saveAs"];

/// A save deferred until its panel field commits, bound to the originating file and window.
#[derive(Clone, Copy)]
pub struct AfterFieldSave {
    command: &'static str,
    document: u64,
    window: u64,
}

impl AfterFieldSave {
    /// Run a deferred field-save binding only if the originating window still exists and owns
    /// the document; temporarily activate that document, run the binding, then restore the
    /// previously front tab.
    fn run(self, app: &mut FigApp) {
        // A closed file or one moved to another window cannot turn this into a save of the
        // current tab. The UiState normally stays in its window's pane; validate that too.
        if app.windows.current != self.window || !app.windows.exists(self.window) || app.windows.of(self.document) != self.window {
            return;
        }
        let Some(index) = app.session.docs.iter().position(|d| d.uid == self.document) else { return };
        let front = app.session.ds().ok().map(|d| d.uid);
        app.session.active = index;
        // Save as captures this owner in PendingDialog before the visible tab is restored.
        run_binding(app, self.command);
        if let Some(index) = front.and_then(|uid| app.session.docs.iter().position(|d| d.uid == uid)) {
            app.session.active = index;
        }
    }
}

/// The request belongs to one editor in one viewport/frame; a discarded pass never releases it
/// a second time, and a field that disappeared cannot leave a marker for another frame.
fn field_save_marker(ctx: &Context) -> egui::Id {
    egui::Id::unique(("field_save_commit", ctx.viewport_id()))
}

/// An early-returning pane did not draw its field or reach shortcut delivery.
pub(crate) fn clear_field_save_marker(ctx: &Context) {
    let marker = field_save_marker(ctx);
    ctx.data_mut(|d| d.remove::<(egui::Id, u64)>(marker));
}

/// Capture a field's save before widgets route pointer/control input. Keep its focus until its
/// TextEdit has processed this batch, including Text delivered together with the save chord.
pub(crate) fn prepare_field_save(app: &FigApp, ctx: &Context) -> Option<AfterFieldSave> {
    if ctx.current_pass_index() > 0 {
        return None;
    }
    let marker = field_save_marker(ctx);
    ctx.data_mut(|d| d.remove::<(egui::Id, u64)>(marker));
    if !ctx.text_edit_focused()
        || !crate::a11y::field_focused(ctx)
        || app.ui.palette.is_some()
        || app.pending_dialog.is_some()
        || egui::Popup::is_any_open(ctx)
        || ctx.memory(|m| m.top_modal_layer().is_some())
    {
        return None;
    }
    let editor = ctx.memory(|m| m.focused())?;
    note_platform(ctx);
    let (command, key, modifiers) = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } if modifiers.command => {
                lookup(keymap::Context::Canvas, *key, *physical_key, *modifiers)
                    .filter(|b| FROM_A_FIELD.contains(&b.id))
                    .map(|b| (b.id, *key, *modifiers))
            }
            _ => None,
        })
    })?;
    if !crate::menus::available(command) || (app.ui.right_tab == crate::RightTab::Inspect && !figcraft_engine::devinspect::dev_mode_allows(command)) {
        return None;
    }
    let document = app.session.ds().ok().filter(|d| app.shows(d.uid)).map(|d| d.uid)?;
    if !ctx.input_mut(|i| i.consume_key(modifiers, key)) {
        return None;
    }
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(marker, (editor, frame)));
    Some(AfterFieldSave { command, document, window: app.windows.current })
}

/// Widget IDs are reused between documents. Routing may have replaced the prepared owner since
/// the frame began; cancel before any field can force that owner's draft into another document.
pub(crate) fn revalidate_field_save(app: &FigApp, ctx: &Context, save: Option<AfterFieldSave>) -> Option<AfterFieldSave> {
    let save = save?;
    if app.windows.current == save.window
        && app.windows.exists(save.window)
        && app.windows.of(save.document) == save.window
        && app.session.ds().is_ok_and(|d| d.uid == save.document && app.shows(d.uid))
    {
        Some(save)
    } else {
        let marker = field_save_marker(ctx);
        let rejected = ctx.data(|d| d.get_temp::<(egui::Id, u64)>(marker));
        if let Some((editor, frame)) = rejected
            && frame == ctx.cumulative_frame_nr()
        {
            let cancelled = field_save_cancellation(ctx, editor);
            ctx.data_mut(|d| d.insert_temp(cancelled, true));
        }
        clear_field_save_marker(ctx);
        None
    }
}

/// Stable id, per viewport and editor, for a rejected field-save cancellation.
fn field_save_cancellation(ctx: &Context, editor: egui::Id) -> egui::Id {
    egui::Id::unique(("field_save_cancelled", ctx.viewport_id(), editor))
}

/// Keep a rejected field's cancellation until that exact editor is drawn again. Its pane may
/// disappear in this frame; a frame-wide reset would leave its cached draft for another owner.
pub(crate) fn take_field_save_cancellation(ctx: &Context, editor: egui::Id) -> bool {
    let cancelled = field_save_cancellation(ctx, editor);
    ctx.data_mut(|d| {
        let pending = d.get_temp::<bool>(cancelled).unwrap_or(false);
        d.remove::<bool>(cancelled);
        pending
    })
}

/// A shared custom field calls this after its TextEdit has consumed the current events, before
/// its existing lost-focus commit branch. Only the original editor can consume the request.
pub(crate) fn release_field_for_save(response: &egui::Response) {
    let ctx = &response.ctx;
    if ctx.current_pass_index() > 0 {
        return;
    }
    let marker = field_save_marker(ctx);
    let requested = ctx.data(|d| d.get_temp::<(egui::Id, u64)>(marker));
    if requested == Some((response.id, ctx.cumulative_frame_nr())) {
        ctx.data_mut(|d| d.remove::<(egui::Id, u64)>(marker));
        response.surrender_focus();
    }
}

/// The command Cmd/Ctrl+V runs with `m` held too: `edit.paste`, `edit.pasteOver` (Shift),
/// `edit.pasteProperties` (Alt), or `None` for a chord nothing is bound to. The keymap decides.
fn v_binding(m: Modifiers) -> Option<&'static str> {
    lookup(keymap::Context::Canvas, Key::V, Some(Key::V), Modifiers { command: true, ..m }).map(|b| b.id)
}

/// What this frame's paste events ask for ([`paste_events`]).
#[derive(Clone, Copy, Default)]
pub struct PasteEvents {
    /// The clipboard holds an image and no text: paste it.
    pub image: bool,
    /// This frame's paste event belongs to another command or an unbound chord: its text isn't
    /// pasted.
    pub drop_text: bool,
}

/// egui-winit swallows a plain Cmd/Ctrl+V, press and release, and sends a paste event for text or
/// an image paste event for an image; with Shift or Alt held the keys arrive as any others do. The
/// browser sends the keys as well as a paste event, whatever is held, and that event's text has no
/// place in a chord bound to another command.
/// Call at the start of every frame: it reads the raw events, which keys consumed by panels or the
/// presenter still carry.
pub fn paste_events(app: &mut FigApp, ctx: &Context) -> PasteEvents {
    let held = &mut app.ui.held;
    let mut out = PasteEvents::default();
    ctx.input(|i| {
        for ev in &i.raw.events {
            match ev {
                Event::ModifiersChanged(m) => *held = *m,
                // Without Cmd/Ctrl held it's another paste key (Shift+Insert, a Paste key).
                Event::Paste(_) if held.command => out.drop_text |= v_binding(*held) != Some("edit.paste"),
                Event::PasteImage(_) => out.image = true,
                // Keys let go in another window never come back as releases.
                Event::WindowFocused(false) => *held = Modifiers::NONE,
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
    // The colour picker open: Escape closes it here, before the binding deselects — unless one of
    // its own dropdowns is open, which takes this Escape.
    if key == Key::Escape && !egui::Popup::is_any_open(ctx) && crate::colorpicker::close_if_open(app) {
        return true;
    }
    if key_box(app, key, m) {
        return true;
    }
    // Enter or Escape finishes crop mode (the crop is already applied).
    if matches!(key, Key::Enter | Key::Escape) && m.is_none() && app.ui.crop.take().is_some() {
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

/// A composition's input owner before this viewport's control/pointer handling. A normal
/// noncomposing commit needs no prior composition, so only active compositions create a guard.
pub(crate) struct ImeInputBaseline {
    target: ImeTarget,
    caret: usize,
    anchor: usize,
}

impl ImeInputBaseline {
    /// Capture an IME baseline only while a text edit and an active preedit exist, recording
    /// the IME target and the caret/anchor.
    pub(crate) fn capture(app: &FigApp) -> Option<Self> {
        let te = app.session.ds().ok()?.text_edit.as_ref()?;
        te.preedit.as_ref()?;
        Some(Self { target: ime_target(app)?, caret: te.caret, anchor: te.anchor })
    }

    /// Whether the IME target changed, editing or the preedit ended, or the caret/anchor moved
    /// since the baseline was captured.
    fn interrupted(&self, app: &FigApp) -> bool {
        ime_target(app) != Some(self.target)
            || app
                .session
                .ds()
                .ok()
                .and_then(|ds| ds.text_edit.as_ref())
                .is_none_or(|te| te.preedit.is_none() || te.caret != self.caret || te.anchor != self.anchor)
    }
}

/// Apply a batch of text, IME, paste, copy, cut and key events: reject all IME events for an
/// interrupted composition, swallow text produced by a key that ran a shortcut, then finish
/// with triple-click line selection and preedit sync.
fn text_keys(app: &mut FigApp, ctx: &Context, events: &[Event], ime_input: Option<ImeInputBaseline>) {
    // Validate once before the batch: an accepted DeleteSurrounding deliberately clears preedit,
    // but its following Commit still belongs to the same valid transaction. An interrupted
    // composition's deletes and Commit must all be rejected, not just the first stale event.
    let reject_ime = ime_input.is_some_and(|baseline| baseline.interrupted(app));
    // A key that ran a shortcut swallows the text it also produced (Alt+. types "≥" on macOS).
    let mut skip_text = false;
    for ev in events {
        if reject_ime && matches!(ev, Event::Ime(_)) {
            continue;
        }
        match ev {
            Event::Text(t) => {
                if !std::mem::take(&mut skip_text) {
                    insert_text(app, ctx, t);
                }
            }
            Event::Ime(ime) => ime_event(app, ctx, ime),
            Event::Paste(t) => insert_text(app, ctx, t),
            Event::Copy => copy_or_cut(app, ctx, false),
            Event::Cut => copy_or_cut(app, ctx, true),
            Event::Key { key, physical_key, pressed: true, modifiers: m, .. } if text_key_event(app, ctx, *key, *physical_key, *m) => {
                skip_text = true;
            }
            _ => {}
        }
    }
    // Triple-click on the edited text selects its line (double-click is handled by the engine).
    triple_click_line(app, ctx);
    app.sync_preedit();
}

/// Insert plain text (the engine normalises line endings and drops control chars) into the edit,
/// noting the application change the mutation caused.
fn insert_text(app: &mut FigApp, ctx: &Context, t: &str) {
    let before = ime_stamp(app);
    if let Err(e) = figcraft_engine::textedit::insert(&mut app.session, t) {
        app.toast(e.to_string());
    }
    if ime_stamp(app) != before {
        note_ime_application_change(app, ctx);
    }
}

/// An IME commit, preedit or surrounding-text deletion for the edit.
fn ime_event(app: &mut FigApp, ctx: &Context, ev: &egui::ImeEvent) {
    match ev {
        // An accepted IME commit earns an explicit receipt only around the exact mutation.
        // List AutoFormat inside the same command is an application transformation.
        egui::ImeEvent::Commit(t) => {
            let before = ime_stamp(app);
            match figcraft_engine::textedit::insert_ime_commit(&mut app.session, t) {
                Ok(ImeInsertion::Plain) => ime_receipt(app, ctx, before),
                Ok(ImeInsertion::Transformed) => note_ime_application_change(app, ctx),
                Ok(ImeInsertion::Nothing) => {}
                Err(e) => app.toast(e.to_string()),
            }
        }
        // The composition shows inline at the caret; the document gets it on Commit.
        egui::ImeEvent::Preedit { text, active_range_chars } => {
            let cursor = active_range_chars.as_ref().map(|r| r.start);
            if let Err(e) = figcraft_engine::textedit::preedit(&mut app.session, text, cursor) {
                app.toast(e.to_string());
            }
        }
        // An IME replacing text around the caret (0.37's incoming byte-counted deletion)
        // deletes it before the paired Commit inserts the replacement.
        egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes } => {
            let before = ime_stamp(app);
            match figcraft_engine::textedit::delete_surrounding_bytes(&mut app.session, *before_bytes, *after_bytes) {
                Ok(true) => ime_receipt(app, ctx, before),
                Ok(false) => {}
                Err(e) => app.toast(e.to_string()),
            }
        }
        egui::ImeEvent::DeleteSurrounding { before_chars, after_chars } => {
            let before = ime_stamp(app);
            match figcraft_engine::textedit::delete_surrounding_chars(&mut app.session, *before_chars, *after_chars) {
                Ok(true) => ime_receipt(app, ctx, before),
                Ok(false) => {}
                Err(e) => app.toast(e.to_string()),
            }
        }
        // Legacy Enabled/Disabled events were ignored by the original event dispatcher too.
        _ => {}
    }
}

/// Copy or cut the edit's selection. Input injected through the control channel must never read
/// or replace the user's system clipboard; it may still edit the document: a synthetic Cut
/// deletes the selection exactly like a user's Cut; only the OS clipboard write is fenced.
fn copy_or_cut(app: &mut FigApp, ctx: &Context, cut: bool) {
    let before = ime_stamp(app);
    let selected = figcraft_engine::textedit::selected_text(&app.session).filter(|t| !t.is_empty());
    if !app.control_synthetic
        && let Some(text) = &selected
    {
        app.clipboard_set_text(text);
    }
    if cut && selected.is_some() {
        let _ = figcraft_engine::textedit::key(&mut app.session, "Delete", false, false, false);
    }
    if ime_stamp(app) != before {
        note_ime_application_change(app, ctx);
    }
}

/// A text-editing key press. `true` when it ran a shortcut, whose following Text event is
/// swallowed (a shortcut that changed committed text/selection is also an application edit).
fn text_key_event(app: &mut FigApp, ctx: &Context, key: Key, physical: Option<Key>, m: Modifiers) -> bool {
    // Keys during a composition are the IME's (Enter picks a candidate, Escape cancels).
    if app.session.ds().is_ok_and(|d| d.text_edit.as_ref().is_some_and(|te| te.preedit.is_some())) {
        return false;
    }
    if m.command || m.alt || m.ctrl {
        let before = ime_stamp(app);
        if text_shortcut(app, key, physical, m) {
            // A text shortcut that changed the committed text/selection (a list,
            // Bold, alignment) is an application edit; Save and friends are not.
            if ime_stamp(app) != before {
                note_ime_application_change(app, ctx);
            }
            return true;
        }
    }
    if key == Key::Z && m.command {
        note_ime_application_change(app, ctx);
        let _ = figcraft_engine::textedit::end(&mut app.session);
        app.run(if m.shift { "edit.redo" } else { "edit.undo" }, json!({}));
        return false;
    }
    let Some(name) = text_key_name(key, m.command) else { return false };
    if name == "Enter" && m.command {
        let _ = figcraft_engine::textedit::end(&mut app.session);
        return false;
    }
    let before = ime_stamp(app);
    let (word, line) = keymap::text_motion(platform(), name, mods(m));
    if let Err(e) = figcraft_engine::textedit::key(&mut app.session, name, m.shift, word, line) {
        app.toast(e.to_string());
    }
    if ime_stamp(app) != before {
        note_ime_application_change(app, ctx);
    }
    false
}

/// The text-edit command a named key runs, if any.
fn text_key_name(key: Key, command: bool) -> Option<&'static str> {
    Some(match key {
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
        Key::A if command => "SelectAll",
        _ => return None,
    })
}

/// Triple-click on the edited text selects its line (double-click is handled by the engine).
fn triple_click_line(app: &mut FigApp, ctx: &Context) {
    if !ctx.input(|i| i.pointer.button_triple_clicked(egui::PointerButton::Primary)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()).filter(|pos| app.canvas.rect.contains(*pos)) else {
        return;
    };
    let Some(te) = app.session.ds().ok().and_then(|d| d.text_edit.clone()) else { return };
    let p = app.to_page(pos);
    if let Some(idx) = text_line_index(app, te.id, p) {
        let before = ime_stamp(app);
        app.run("text.edit.selectLine", json!({"index": idx}));
        if ime_stamp(app) != before {
            note_ime_application_change(app, ctx);
        }
    }
}

/// The text index of the line under `p`, when the edit's node contains it.
fn text_line_index(app: &FigApp, id: figcraft_doc::NodeId, p: figcraft_geom::Point) -> Option<usize> {
    let inside = app.session.doc().ok().and_then(|d| d.world_bounds(id)).is_some_and(|b| b.contains(p));
    if !inside {
        return None;
    }
    app.session.text_index_at(id, p).ok()
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
///
/// This is FigCraft's one custom IME producer. Geometry stays the actual caret (the composition
/// cursor while composing); the descriptor also carries the reviewed E21b payload: a stable
/// per-edit owner, a viewport-scoped context generation, a bounded committed-text snapshot around
/// the selection (UTF-8 byte offsets, reversed direction preserved, pre-edit excluded) and an
/// explicit `Other`/`InputMethod` change cause. The engine query is logical model text, never a
/// visually reordered string, and copies at most 4000 bytes. One observation is made per viewport pass,
/// after this pass's IME events and commands were applied. The only production caller is
/// `FigApp::observe_ime`, the pass's single observation point.
pub(crate) fn ime_output(app: &mut FigApp, ctx: &Context) {
    let viewport = ctx.viewport_id();
    let Some(target) = ime_target(app) else {
        ime_idle(app, ctx);
        return;
    };
    let Some(cursor_rect) = ime_cursor_rect(app) else {
        // No visible caret means no focused owner for this pass; a later pass starts a new
        // generation instead of inheriting a composition.
        ime_idle(app, ctx);
        return;
    };
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    let pass_nr = ctx.cumulative_pass_nr_for(viewport);
    let surrounding = figcraft_engine::textedit::ime_surrounding(&app.session);
    let committed = surrounding.as_ref().ok().map(|s| CommittedText::new(&s.text, s.cursor, s.anchor));
    // The tracker compares every receipt with the last observation's baseline. An unavailable
    // baseline therefore needs the same engine revision as `ime_stamp`; otherwise a later
    // unavailable -> available receipt (a commit replacing a too-wide selection) cannot match it.
    let unavailable_revision = if surrounding.is_err() { app.session.ds().ok().map(|ds| ds.revision) } else { None };
    let observation = ImeObservation::new(frame_nr, pass_nr, target).purpose(egui::IMEPurpose::Normal).committed(committed);
    let observation = match unavailable_revision {
        Some(revision) => observation.unavailable_revision(revision),
        None => observation,
    };
    let Some(update) = app.ime.observe(viewport, Some(observation)) else { return };
    ctx.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            owner: update.owner,
            context_generation: update.context_generation,
            purpose: update.purpose,
            // The desktop places the candidate window by `rect` (the web by `cursor_rect`).
            rect: cursor_rect,
            cursor_rect,
            should_interrupt_composition: false,
            reset_request: None,
            change_cause: update.change_cause,
            surrounding_text: update.surrounding_text,
        })
    });
}

/// Mark this viewport's custom editor as unfocused for the pass (no descriptor from it).
fn ime_idle(app: &mut FigApp, ctx: &Context) {
    let _ = app.ime.observe(ctx.viewport_id(), None);
}

/// The immutable identity of the edit being made, if any.
fn ime_target(app: &FigApp) -> Option<ImeTarget> {
    let ds = app.session.ds().ok()?;
    let te = ds.text_edit.as_ref()?;
    // A late `window.new` can move this file out of the window drawing now before the pass ends.
    // The stale session still holds the edit, but this viewport no longer shows it: observe idle
    // and let the receiving window own the platform IME on its own pass.
    if !app.shows(ds.uid) {
        return None;
    }
    // Document and node only: caret, selection, text, revision and geometry are ordinary updates.
    let owner = egui::Id::unique(("figcraft.ime.text", ds.uid, te.id.0));
    Some(ImeTarget::new(owner, te.editing_session))
}

/// The caret rectangle for the current edit (inside the composition while composing).
fn ime_cursor_rect(app: &FigApp) -> Option<egui::Rect> {
    let te = app.session.ds().ok()?.text_edit.clone()?;
    let preedit = app.preedit_view();
    let doc = preedit.map(|p| &*p.doc).or_else(|| app.session.doc().ok())?;
    let node = doc.node(te.id)?;
    let (layout, dy) = figcraft_text::layout_node(node)?;
    let w = doc.world_transform(te.id);
    let (cx, li) = layout.caret(preedit.map(|p| p.cursor).unwrap_or(te.caret));
    let (top, h) = layout.lines.get(li).map(|l| (l.top, l.height)).unwrap_or((0.0, 14.0));
    let p0 = w * figcraft_geom::Point::new(cx, top + dy);
    let p1 = w * figcraft_geom::Point::new(cx, top + h + dy);
    let (a, b) = (app.to_screen(p0.x, p0.y), app.to_screen(p1.x, p1.y));
    (a.x.is_finite() && a.y.is_finite() && b.x.is_finite() && b.y.is_finite()).then(|| egui::Rect::from_two_pos(a, b + egui::vec2(1.0, 0.0)))
}

/// The bounded committed snapshot of the current edit, for an IME receipt.
///
/// Each call walks the text once to the caret/anchor (O(n) worst case) and copies at most 4000
/// bytes; the pair before/after a text-affecting event is what receipt validation compares.
fn ime_stamp(app: &FigApp) -> Option<ImeMutationStamp> {
    let ds = app.session.ds().ok()?;
    ds.text_edit.as_ref()?;
    Some(match figcraft_engine::textedit::ime_surrounding(&app.session) {
        Ok(s) => ImeMutationStamp::from_committed(&s.text, s.cursor, s.anchor, None),
        // A bounded snapshot is unavailable (for example, a selection wider than the budget).
        // The document revision is still a trustworthy, engine-owned transition token for this
        // edit target: every accepted mutation bumps it, and the target (document uid + node +
        // editing session) scopes it. Without it the tracker cannot accept an unavailable-side
        // receipt and reports an accepted IME commit/delete as `Other`.
        Err(_) => ImeMutationStamp::unavailable_at_revision(ds.revision),
    })
}

/// Record an accepted IME commit/deletion, immediately after it succeeded.
///
/// `before` is captured immediately before the same mutation. No-op, failed, stale and
/// wrong-target receipts are rejected by the tracker, so they never earn `InputMethod`.
fn ime_receipt(app: &mut FigApp, ctx: &Context, before: Option<ImeMutationStamp>) {
    let (Some(before), Some(after), Some(target)) = (before, ime_stamp(app), ime_target(app)) else { return };
    let viewport = ctx.viewport_id();
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    let _ = app.ime.note_input_method_apply(viewport, frame_nr, target, before, after);
}

/// Record an application-origin edit for the frame (`Other` dominates it).
fn note_ime_application_change(app: &mut FigApp, ctx: &Context) {
    let Some(target) = ime_target(app) else { return };
    let viewport = ctx.viewport_id();
    let frame_nr = ctx.cumulative_frame_nr_for(viewport);
    let _ = app.ime.note_application_change(viewport, frame_nr, target);
}
