//! An idle window draws no frames: once renders and animations settle, the whole UI (driven
//! headless through egui) asks for no further immediate repaint, only far-off deadlines (the
//! autosave checkpoint, a toast going away). What does need a frame wakes the UI: a finished
//! background render, a screenshot on its way, a control request.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use egui::{Event, Pos2, RawInput, Rect, ViewportId, vec2};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present_tests::id;
use crate::{ControlRequest, FigApp, NoServices};

/// The shortest repaint delay that counts as idle (a blinking caret wakes twice a second; nothing
/// here edits text).
const IDLE: Duration = Duration::from_secs(1);

pub(crate) struct Harness {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Harness {
    pub(crate) fn new(s: Session) -> Self {
        // The one system font scan bumps the layout generation, which makes the canvas render
        // again: scanned here, before the first frame, rather than by whichever test happens to
        // run first while this one waits for idle (#333).
        figcraft_text::scan_system_fonts();
        Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 }
    }

    /// One frame with `events`, 1/60 s after the last; returns the repaint delay it asked for.
    fn frame(&mut self, events: Vec<Event>) -> Duration {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        out.viewport_output.get(&ViewportId::ROOT).map_or(Duration::MAX, |v| v.repaint_delay)
    }

    /// Draw frames with no input until the UI is idle: no render in flight and three frames in a
    /// row asking for no repaint sooner than [`IDLE`]. Returns the last frame's delay; fails with
    /// what kept asking for frames.
    fn settle(&mut self) -> Duration {
        let mut quiet = 0;
        for _ in 0..2000 {
            let delay = self.frame(vec![]);
            let rendering = wait_for_render(&self.app);
            quiet = if delay >= IDLE && !rendering { quiet + 1 } else { 0 };
            if quiet >= 3 {
                return delay;
            }
        }
        panic!("still asking for frames after 2000 with no input: {:?}", self.ctx.repaint_causes());
    }

    fn run(&mut self, cmd: &str, params: Value) -> Value {
        let r = self.app.session.execute(cmd, &params).unwrap();
        self.app.handle_ui_requests();
        r
    }

    /// Settles, then stays idle: more frames with no input ask for nothing sooner.
    pub(crate) fn assert_idle(&mut self, what: &str) {
        self.settle();
        for _ in 0..10 {
            let delay = self.frame(vec![]);
            assert!(delay >= IDLE, "{what}: asked for a repaint in {delay:?}: {:?}", self.ctx.repaint_causes());
        }
    }
}

/// Waits (without drawing frames, however slow the machine) until the render thread is done with
/// every job and has woken the UI for it; returns whether one was in flight.
pub(crate) fn wait_for_render(app: &FigApp) -> bool {
    let start = Instant::now();
    let mut waited = false;
    while app.worker.as_ref().is_some_and(crate::render_worker::Worker::in_flight) {
        assert!(start.elapsed() < Duration::from_secs(120), "the render thread never finished");
        std::thread::sleep(Duration::from_millis(2));
        waited = true;
    }
    waited
}

/// A page with a rectangle, an ellipse and a text layer; returns the text's id.
fn document() -> (Harness, Value) {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 120})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 300, "y": 40, "width": 120, "height": 120})).unwrap();
    let text = s.execute("text.create", &json!({"x": 40, "y": 220, "text": "Idle"})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    (Harness::new(s), text)
}

#[test]
fn an_open_document_draws_no_frames_while_idle() {
    let (mut h, _) = document();
    h.assert_idle("a document");
    // The next deadline is the autosave checkpoint, minutes away.
    assert!(h.frame(vec![]) > Duration::from_secs(60));
}

#[test]
fn a_selected_text_layer_draws_no_frames_while_idle() {
    let (mut h, text) = document();
    h.run("select.set", json!({"ids": [text]}));
    h.assert_idle("a selected text layer");
}

#[test]
fn the_comments_panel_draws_no_frames_while_idle() {
    let (mut h, _) = document();
    h.run("comment.add", json!({"x": 50, "y": 50, "text": "Looks good", "author": "Ada", "anchor": true}));
    h.run("tool.comment", json!({}));
    h.assert_idle("the comments panel");
}

#[test]
fn the_prototype_curve_preview_plays_then_rests() {
    let mut s = Session::new();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    let b = id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 200, "height": 300})).unwrap());
    s.execute("proto.interaction.add", &json!({"id": a, "dest": b, "transition": {"type": "dissolve"}, "durationMs": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut h = Harness::new(s);
    h.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&h.ctx, Some((a, 0)));
    // It plays when it shows (the dot moves: frames at animation pace) …
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.frame(vec![]) < Duration::from_millis(50), "the curve preview plays when it opens");
    // … then rests …
    h.assert_idle("the interaction editor");
    // … and plays again when it shows again.
    crate::proto_panel::open_editor(&h.ctx, None);
    h.frame(vec![]);
    crate::proto_panel::open_editor(&h.ctx, Some((a, 0)));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.frame(vec![]) < Duration::from_millis(50), "the curve preview plays when it shows again");
    h.assert_idle("the interaction editor, again");
}

#[test]
fn presenting_a_still_frame_draws_no_frames_while_idle() {
    let mut s = Session::new();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    let mut h = Harness::new(s);
    h.run("proto.present", json!({"start": a}));
    assert!(h.app.ui.present.is_some());
    h.assert_idle("the presenter");
}

#[test]
fn an_after_delay_wakes_the_presenter_when_due_even_under_an_overlay() {
    let mut s = Session::new();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    let b = id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 200, "height": 300})).unwrap());
    let o = id(&s.execute("frame.create", &json!({"x": 600, "y": 0, "width": 100, "height": 100})).unwrap());
    let delay = |ms: u64, action: Value| json!({"id": a, "trigger": {"type": "afterDelay", "ms": ms}, "action": action});
    s.execute("proto.interaction.add", &delay(2000, json!({"type": "navigate", "dest": b}))).unwrap();
    s.execute("proto.interaction.add", &delay(100, json!({"type": "openOverlay", "dest": o}))).unwrap();
    let mut h = Harness::new(s);
    h.run("proto.present", json!({"start": a}));
    h.settle();
    let presenter = || h.app.ui.present.as_ref().map(crate::present::Presenter::inspect).unwrap();
    assert_eq!(presenter()["overlays"], json!([o]), "the overlay opened after 100 ms");
    // The 2 s timer belongs to the frame under the overlay, which can't fire it while the overlay
    // is open: once it's past due the presenter doesn't spin waiting for it.
    h.time += 3.0;
    h.assert_idle("an overlay over a frame with a pending delay");
    let presenter = h.app.ui.present.as_ref().map(crate::present::Presenter::inspect).unwrap();
    assert_eq!(presenter["current"], json!(a), "{presenter}");
}

#[test]
fn a_toast_wakes_the_ui_only_to_go_away() {
    let (mut h, _) = document();
    h.settle();
    h.app.toast("Saved");
    // Once it has faded in, the next frame it asks for is the one that hides it.
    let delay = h.settle();
    assert!(delay > Duration::from_secs(2) && delay <= Duration::from_secs(3), "{delay:?}");
    assert!(h.app.ui.toast.is_some());
    h.time += 3.0;
    h.frame(vec![]);
    assert!(h.app.ui.toast.is_none(), "gone after 3 s");
}

#[test]
fn a_toast_raised_by_a_slow_frame_still_shows_for_3_s() {
    let (mut h, _) = document();
    h.settle();
    // The Open dialog and a failed open block the frame that raises the error: the next frame
    // comes 5 s after it started.
    h.app.toast("can't open file.fig");
    h.frame(vec![]);
    h.time += 5.0;
    h.frame(vec![]);
    assert!(h.app.ui.toast.is_some(), "shown, not expired by the wait");
    h.time += 2.0;
    h.frame(vec![]);
    assert!(h.app.ui.toast.is_some());
    h.time += 1.5;
    h.frame(vec![]);
    assert!(h.app.ui.toast.is_none(), "gone after 3 s");
}

#[test]
fn a_finished_background_render_wakes_the_ui() {
    let ctx = egui::Context::default();
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    ctx.set_request_repaint_callback(move |_| {
        w.fetch_add(1, Ordering::SeqCst);
    });
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    let ds = s.ds().unwrap();
    let mut worker = crate::render_worker::Worker::new(ctx.clone());
    worker.submit(crate::render_worker::Job {
        doc: ds.doc.clone(),
        page: ds.page,
        view: figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 120, height: 120 },
        outline: false,
        outline_extras: Default::default(),
        outlined: vec![],
        skip: vec![],
        solo: None,
        proof: false,
        uid: 0,
        zoom_preview: false,
        at: web_time::Instant::now(),
    });
    let start = Instant::now();
    while wakes.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "the render thread never woke the UI");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(worker.poll().is_some(), "the woken frame finds the image");
}

#[test]
fn control_requests_and_screenshots_wake_an_idle_ui() {
    let (mut h, _) = document();
    h.settle();
    // A request is answered in the next frame, which asks for one more to show its effect.
    let (tx, rx) = std::sync::mpsc::channel();
    h.app.control_queue.push(ControlRequest {
        id: json!(1),
        method: "ui.inspect".into(),
        params: json!({}),
        reply: tx.clone(),
        cancelled: Default::default(),
    });
    assert_eq!(h.frame(vec![]), Duration::ZERO);
    assert!(rx.try_recv().unwrap().result.is_ok());
    h.assert_idle("after a control request");
    // A screenshot keeps frames coming until the window delivers it, then the UI rests again.
    let path = std::env::temp_dir().join(format!("figcraft-idle-shot-{}.png", std::process::id()));
    let params = json!({"path": path.to_string_lossy()});
    h.app.control_queue.push(ControlRequest { id: json!(2), method: "ui.screenshot".into(), params, reply: tx, cancelled: Default::default() });
    for _ in 0..5 {
        assert_eq!(h.frame(vec![]), Duration::ZERO, "waiting for the screenshot");
    }
    assert!(rx.try_recv().is_err(), "not delivered yet");
    let image = Arc::new(egui::ColorImage::filled([4, 4], egui::Color32::RED));
    h.frame(vec![Event::Screenshot { viewport_id: ViewportId::ROOT, user_data: egui::UserData::default(), image }]);
    let reply = rx.try_recv().unwrap();
    assert_eq!(reply.result.unwrap()["width"], json!(4));
    let _ = std::fs::remove_file(&path);
    h.assert_idle("after a screenshot");
}

/// A synthetic animated GIF: red, then green, 100 ms each.
fn animated_gif() -> String {
    use base64::Engine as _;
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        let frame = |c: [u8; 4]| {
            image::Frame::from_parts(image::RgbaImage::from_pixel(8, 8, image::Rgba(c)), 0, 0, image::Delay::from_numer_denom_ms(100, 1))
        };
        enc.encode_frames([frame([255, 0, 0, 255]), frame([0, 255, 0, 255])]).unwrap();
    }
    base64::engine::general_purpose::STANDARD.encode(out)
}

#[test]
fn an_animated_gif_wakes_only_the_presenter_and_only_at_its_frames() {
    let mut s = Session::new();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    s.execute("image.place", &json!({"data": animated_gif(), "parent": a, "x": 10, "y": 10, "width": 100, "height": 100})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let mut h = Harness::new(s);
    h.assert_idle("an animated GIF on the canvas (it shows its first frame)");
    h.run("proto.present", json!({"start": a}));
    let delays: Vec<Duration> = (0..30).map(|_| h.frame(vec![])).collect();
    // Past the first frames (the presenter opening), it wakes for the GIF's next frame: no sooner
    // than needed, never later than one GIF frame (plus the display frame egui wakes early by).
    for d in &delays[5..] {
        assert!(*d <= Duration::from_millis(100 + 17), "{delays:?}");
    }
    assert!(delays[5..].iter().any(|d| *d > Duration::from_millis(20)), "it doesn't draw every frame: {delays:?}");
    h.app.ui.present = None;
    h.assert_idle("the presenter closed");
}

#[test]
fn cjk_names_are_looked_for_in_edits_not_on_every_pan() {
    let mut s = Session::new();
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    let b = s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    s.execute("node.set", &json!({"id": a, "name": "中文"})).unwrap();
    let mut h = Harness::new(s);
    let ctx = h.ctx.clone();
    assert_eq!(h.app.cjk_needed(&ctx, 0), crate::cjk::HAN, "the first look reads every name");
    // A pan or zoom changes the revision but not the document: its hundreds of thousands of
    // names aren't read again (that took 70 ms a frame in a 900 000-layer file).
    let revision = h.app.session.ds().unwrap().revision;
    h.run("view.set", json!({"zoom": 2, "cx": 300, "cy": 200}));
    assert_ne!(h.app.session.ds().unwrap().revision, revision);
    assert_eq!(h.app.cjk_needed(&ctx, 0), 0);
    // An edit is read: the layers it changed.
    h.run("node.set", json!({"id": b, "name": "한국어"}));
    assert_eq!(h.app.cjk_needed(&ctx, 0), crate::cjk::HANGUL);
}

#[test]
fn a_hidden_window_answers_at_once() {
    // #269, #720: minimised or covered, the window draws no frames, but the app's logic still runs:
    // it answers edits, inspection, saves and the interface's state; what needs a frame (a capture
    // of the window) is an error at once instead of waiting for the window to show.
    let dir = std::env::temp_dir().join(format!("figcraft-hidden-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hidden.figcraft");
    let (mut h, _) = document();
    h.settle();
    let (tx, rx) = std::sync::mpsc::channel();
    let request = |id: u64, method: &str, params: Value| ControlRequest {
        id: json!(id),
        method: method.into(),
        params,
        reply: tx.clone(),
        cancelled: Default::default(),
    };
    h.app.control_queue.push(request(1, "engine.execute", json!({"command": "file.save", "params": {"path": path.to_string_lossy()}})));
    h.app.control_queue.push(request(2, "ui.inspect", json!({})));
    h.app.control_queue.push(request(3, "ui.screenshot", json!({"path": dir.join("shot.png").to_string_lossy()})));
    let mut hidden = RawInput::default();
    hidden.viewports.insert(ViewportId::ROOT, egui::ViewportInfo { minimized: Some(true), ..Default::default() });
    let app = &mut h.app;
    let _ = h.ctx.run_logic(&hidden, |ctx| crate::control::drain_hidden(app, ctx));
    let saved = rx.try_recv().unwrap();
    assert_eq!(saved.id, json!(1));
    assert!(saved.result.is_ok() && path.exists(), "saved while hidden: {:?}", saved.result);
    let inspected = rx.try_recv().unwrap();
    assert_eq!(inspected.id, json!(2));
    assert!(inspected.result.is_ok(), "the interface's state: {:?}", inspected.result);
    let shot = rx.try_recv().unwrap();
    assert_eq!(shot.id, json!(3));
    assert!(shot.result.unwrap_err().contains("minimised or hidden"), "a capture needs the window on screen");
    assert!(h.app.control_queue.is_empty(), "nothing waits for the window");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_save_writes_a_changed_file_then_rests() {
    let dir = std::env::temp_dir().join(format!("figcraft-auto-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("auto.figcraft").to_string_lossy().to_string();
    let (mut h, _) = document();
    h.run("file.save", json!({"path": path}));
    h.run("pref.autoSave", json!({"on": true}));
    h.assert_idle("a saved file");
    h.run("shape.rectangle", json!({"x": 500, "y": 300, "width": 40, "height": 40}));
    assert!(h.frame(vec![]) <= Duration::from_secs_f64(crate::AUTO_SAVE_SECS), "wakes up to save it");
    assert!(h.app.session.ds().unwrap().is_dirty(), "not saved at once");
    // Another change restarts the wait.
    h.time += crate::AUTO_SAVE_SECS - 0.5;
    h.run("shape.rectangle", json!({"x": 600, "y": 300, "width": 40, "height": 40}));
    h.frame(vec![]);
    h.time += 1.0;
    h.frame(vec![]);
    assert!(h.app.ui.auto_saving.is_empty() && h.app.session.ds().unwrap().is_dirty(), "waits for a pause in the changes");
    h.time += crate::AUTO_SAVE_SECS;
    // The file is encoded on a thread of its own, then written.
    let start = Instant::now();
    while h.app.session.ds().unwrap().is_dirty() {
        assert!(start.elapsed() < Duration::from_secs(20), "saved");
        let delay = h.frame(vec![]);
        assert!(h.app.ui.auto_saving.is_empty() || delay <= Duration::from_millis(100), "looks again for the file being encoded");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    let ds = t.ds().unwrap();
    assert!(ds.doc.page_content_bounds(ds.page).unwrap().x1 >= 640.0, "the new rectangles are in the file");
    h.assert_idle("an auto-saved file");
    let _ = std::fs::remove_dir_all(&dir);
}
