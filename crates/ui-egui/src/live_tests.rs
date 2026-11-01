//! Live sessions drawn headless: a remote cursor with its name tag gliding to where it was
//! reported, a remote selection outlined in its owner's colour, avatars at the top of the right
//! panel (click one to follow), the follow frame, the Share popover and the Join dialog, and no
//! frames while nothing moves.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Color32, Event, PointerButton, Pos2, RawInput, Rect, Shape, ViewportId, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::live::{Event as LiveEvent, HostOptions, LiveLink, Peer, Presence, Role, SessionInfo, Status};
use serde_json::json;

use crate::present_tests::id;
use crate::{FigApp, NoServices};

pub(crate) const HANA: Color32 = Color32::from_rgb(0x97, 0x47, 0xFF);

/// A link that hands out scripted events, as the session started last.
struct Mock(Arc<Mutex<Vec<LiveEvent>>>, u64);

impl LiveLink for Mock {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("not in this test".into())
    }
    fn join(&mut self, session: u64, _: &str, _: &str) -> Result<(), String> {
        self.1 = session;
        Ok(())
    }
    fn publish(&mut self, session: u64, _: figcraft_engine::live::PublishOptions) -> Result<(), String> {
        self.1 = session;
        Ok(())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: figcraft_engine::figcraft_collab::message::ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        std::mem::take(&mut *self.0.lock().unwrap()).into_iter().map(|e| (self.1, e)).collect()
    }
}

pub(crate) struct Harness {
    pub(crate) ctx: egui::Context,
    pub(crate) app: FigApp,
    time: f64,
    events: Arc<Mutex<Vec<LiveEvent>>>,
}

/// Everything painted, flattened.
pub(crate) fn flat(shapes: &[egui::epaint::ClippedShape]) -> Vec<Shape> {
    fn walk(s: &Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for c in shapes {
        walk(&c.shape, &mut out);
    }
    out
}

impl Harness {
    pub(crate) fn new() -> (Self, NodeId) {
        let mut s = Session::new();
        let rect = id(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap());
        s.execute("select.set", &json!({"ids": []})).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        s.live.install(Box::new(Mock(events.clone(), 0)));
        let h = Harness { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0, events };
        (h, rect)
    }

    /// One frame; the shapes it painted and the repaint delay it asked for.
    pub(crate) fn frame(&mut self, events: Vec<Event>) -> (Vec<Shape>, Duration) {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        let delay = out.viewport_output.get(&ViewportId::ROOT).map_or(Duration::MAX, |v| v.repaint_delay);
        (flat(&out.shapes), delay)
    }

    /// Let `secs` of UI time pass before the next frame.
    pub(crate) fn skip(&mut self, secs: f64) {
        self.time += secs;
    }

    pub(crate) fn frames(&mut self, n: usize) -> Vec<Shape> {
        let mut last = Vec::new();
        for _ in 0..n {
            last = self.frame(vec![]).0;
        }
        last
    }

    pub(crate) fn click(&mut self, p: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() },
            ]);
        }
    }

    pub(crate) fn send(&mut self, e: Vec<LiveEvent>) {
        self.events.lock().unwrap().extend(e);
    }

    /// Joined as Ada, with Hana (the host) already here.
    pub(crate) fn joined(&mut self) {
        self.app.session.execute("collab.join", &json!({"link": "figcraft://join?host=127.0.0.1:47900&file=f&token=t"})).unwrap();
        self.welcome();
    }

    /// The host's welcome for the join started last: you are Ada, with Hana (the host) here.
    pub(crate) fn welcome(&mut self) {
        let info = SessionInfo {
            role: Role::Guest,
            me: 2,
            name: "Ada Lovelace".into(),
            color: "#14AE5C".into(),
            file_name: "Shared".into(),
            pages: vec![],
            link: String::new(),
            url: String::new(),
            address: "127.0.0.1:47900".into(),
            network: false,
            access: figcraft_engine::live::Access::Edit,
        };
        self.send(vec![LiveEvent::Welcome { info, peers: vec![Peer { client_id: 1, name: "Hana".into(), color: "#9747FF".into() }] }]);
    }

    /// The host's welcome with the document (a session server seeded with this document): it opens
    /// as a new tab, which is then the synced document.
    pub(crate) fn sync(&mut self) {
        use figcraft_engine::figcraft_collab::message::{ClientMsg, ServerMsg};
        use figcraft_engine::figcraft_collab::{Client, Limits, Outgoing, Server};
        let mut server = Server::new(self.app.session.doc().unwrap().clone(), "", Limits::default());
        let ClientMsg::Hello(host) = Client::hello("f", "", "Hana", "") else { panic!("no hello") };
        server.join(host).unwrap();
        let ClientMsg::Hello(me) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
        let (_, outs) = server.join(me).unwrap();
        let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
        self.send(vec![LiveEvent::Sync(w)]);
        self.frames(2);
        assert!(self.app.session.live_shared());
    }

    pub(crate) fn page(&self) -> NodeId {
        self.app.session.page().unwrap()
    }

    /// Frames until the window stops asking for more (the canvas render finishing aside):
    /// whether it did within a few hundred frames.
    pub(crate) fn goes_quiet(&mut self) -> bool {
        let mut quiet = 0;
        for _ in 0..200 {
            let rendering = crate::idle_tests::wait_for_render(&self.app);
            let (_, d) = self.frame(vec![]);
            quiet = if d >= Duration::from_secs(1) && !rendering { quiet + 1 } else { 0 };
            if quiet >= 3 {
                return true;
            }
        }
        false
    }
}

pub(crate) fn texts(shapes: &[Shape]) -> Vec<(String, Pos2, egui::Vec2)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some((t.galley.text().to_string(), t.pos, t.galley.size())),
            _ => None,
        })
        .collect()
}

pub(crate) fn has_text(shapes: &[Shape], s: &str) -> bool {
    texts(shapes).iter().any(|(t, _, _)| t == s)
}

/// The middle of the text `s` on screen.
pub(crate) fn text_center(shapes: &[Shape], s: &str) -> Pos2 {
    let (_, p, size) = texts(shapes).into_iter().find(|(t, _, _)| t == s).unwrap_or_else(|| panic!("no text {s:?}"));
    p + size / 2.0
}

/// Filled polygons and outlines in `c`.
pub(crate) fn filled(shapes: &[Shape], c: Color32) -> usize {
    shapes.iter().filter(|s| matches!(s, Shape::Path(p) if p.fill == c)).count()
}

fn outlined(shapes: &[Shape], c: Color32) -> usize {
    shapes
        .iter()
        .filter(|s| matches!(s, Shape::Path(p) if p.closed && p.fill == Color32::TRANSPARENT && p.stroke.color == egui::epaint::ColorMode::Solid(c)))
        .count()
}

pub(crate) fn cursor(page: NodeId, x: f64, y: f64) -> Presence {
    Presence { cursor: Some((page, x, y)), ..Presence::default() }
}

#[test]
fn remote_cursors_selections_and_avatars_draw() {
    let (mut h, rect) = Harness::new();
    h.frames(3);
    h.joined();
    let page = h.page();
    h.send(vec![LiveEvent::Presence(
        1,
        Presence { selection: Some(vec![rect]), viewport: Some((page, 0.0, 0.0, 600.0, 400.0)), ..cursor(page, 150.0, 150.0) },
    )]);
    // Before the document arrives, the layer ids in her selection aren't this document's.
    let shapes = h.frames(3);
    assert_eq!(outlined(&shapes, HANA), 0, "no outline before the document syncs");
    h.sync();
    assert_eq!(h.app.session.docs.len(), 2, "the host's file opened as a tab");
    let shapes = h.frames(2);
    assert_eq!(h.app.session.live().status, Status::Live);
    // The cursor: the dart in Hana's colour, her name beside it, at her page position.
    assert!(has_text(&shapes, "Hana"), "the name tag");
    assert_eq!(filled(&shapes, HANA), 2, "the pointer's two halves");
    assert_eq!(crate::live::shown_at(&h.app), vec![(1, (150.0, 150.0))]);
    // Her selection, outlined in her colour (the pointer's white edge aside).
    assert_eq!(outlined(&shapes, HANA), 1, "the selected rectangle");
    // Avatars: you and Hana; the peers popover lists both.
    assert!(has_text(&shapes, "AL") && has_text(&shapes, "H"));
    assert_eq!(h.app.inspect()["live"]["cursorsShown"], json!([1]));
}

#[test]
fn a_cursor_glides_then_rests() {
    let (mut h, _) = Harness::new();
    h.frames(3);
    h.joined();
    let page = h.page();
    h.send(vec![LiveEvent::Presence(1, cursor(page, 0.0, 0.0))]);
    h.frames(2);
    h.send(vec![LiveEvent::Presence(1, cursor(page, 400.0, 0.0))]);
    let (_, delay) = h.frame(vec![]);
    let x = crate::live::shown_at(&h.app)[0].1.0;
    assert!(x > 0.0 && x < 400.0, "on its way: {x}");
    assert!(delay < Duration::from_millis(50), "gliding asks for frames");
    h.frames(60);
    assert_eq!(crate::live::shown_at(&h.app)[0].1, (400.0, 0.0), "arrived");
    // Settled: no more frames until something happens.
    assert!(h.goes_quiet(), "a resting cursor draws no frames: {:?}", h.ctx.repaint_causes());
    // Reduced motion: it jumps.
    h.app.session.prefs.reduce_motion = true;
    h.send(vec![LiveEvent::Presence(1, cursor(page, -50.0, 20.0))]);
    h.frame(vec![]);
    assert_eq!(crate::live::shown_at(&h.app)[0].1, (-50.0, 20.0));
}

#[test]
fn cursors_follow_pages_in_a_shared_document() {
    let (mut h, rect) = Harness::new();
    h.frames(3);
    h.joined();
    let other = id(&h.app.session.execute("page.add", &json!({"name": "Two"})).unwrap());
    h.app.session.execute("page.select", &json!({"index": 0})).unwrap();
    h.sync();
    h.app.session.execute("page.select", &json!({"index": 0})).unwrap();
    h.send(vec![LiveEvent::Presence(
        1,
        Presence { selection: Some(vec![rect]), viewport: Some((other, 0.0, 0.0, 10.0, 10.0)), ..cursor(other, 1.0, 1.0) },
    )]);
    let shapes = h.frames(2);
    assert!(crate::live::shown_at(&h.app).is_empty(), "on another page of this file");
    assert_eq!(outlined(&shapes, HANA), 0, "a selection on another page isn't outlined here");
    // Another document in front shows its own session (none here): no cursor from this one.
    let page = h.page();
    h.app.session.execute("doc.switch", &json!({"index": 0})).unwrap();
    assert!(!h.app.session.live_shared());
    h.send(vec![LiveEvent::Presence(1, cursor(page, 5.0, 5.0))]);
    let shapes = h.frames(2);
    assert!(crate::live::shown_at(&h.app).is_empty());
    assert!(!has_text(&shapes, "Hana"));
    // Back in the session's file, on their page, it shows.
    h.app.session.execute("doc.switch", &json!({"index": 1})).unwrap();
    h.frames(2);
    assert_eq!(crate::live::shown_at(&h.app), vec![(1, (5.0, 5.0))]);
    // Gone when they leave.
    h.send(vec![LiveEvent::PeerLeft(1)]);
    let shapes = h.frames(2);
    assert!(crate::live::shown_at(&h.app).is_empty());
    assert!(!has_text(&shapes, "Hana"));
    assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Hana left"));
}

/// Asserts clicking a peer's avatar follows that peer, and Escape or scrolling the canvas
/// stops following.
#[test]
fn clicking_an_avatar_follows_and_escape_stops() {
    let (mut h, _) = Harness::new();
    h.frames(3);
    h.joined();
    let page = h.page();
    h.send(vec![LiveEvent::Presence(1, Presence { viewport: Some((page, 1000.0, 1000.0, 600.0, 400.0)), ..Presence::default() })]);
    let shapes = h.frames(2);
    let at = text_center(&shapes, "H");
    h.click(at);
    assert_eq!(h.app.session.live().follow, Some(1));
    let shapes = h.frames(2);
    let view = h.app.session.ds().unwrap().view.clone();
    assert_eq!((view.cx, view.cy), (1300.0, 1200.0), "her view");
    assert!(texts(&shapes).iter().any(|(t, _, _)| t.starts_with("Following Hana")), "the follow banner");
    // Escape on the canvas stops following.
    h.click(pos2(700.0, 500.0));
    h.app.session.execute("collab.follow", &json!({"client": 1})).unwrap();
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
    assert_eq!(h.app.session.live().follow, None);
    // Scrolling the canvas stops it too.
    h.app.session.execute("collab.follow", &json!({"client": 1})).unwrap();
    h.frame(vec![Event::PointerMoved(pos2(700.0, 500.0))]);
    h.frame(vec![Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: vec2(0.0, -40.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    }]);
    h.frames(3);
    assert_eq!(h.app.session.live().follow, None);
}

#[test]
fn share_popover_and_join_dialog() {
    let (mut h, _) = Harness::new();
    let shapes = h.frames(3);
    let share = text_center(&shapes, "Share");
    h.click(share);
    let shapes = h.frames(2);
    for t in ["Share this file", "Online", "Local network", "Start sharing", "Join with a code", "Export…"] {
        assert!(has_text(&shapes, t), "the popover shows {t:?}");
    }
    // Starting fails with this link: the error shows, nothing else changes.
    h.click(text_center(&shapes, "Local network"));
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Start sharing"));
    assert_eq!(h.app.session.live().status, Status::Off);
    assert!(h.app.ui.last_error.as_deref().is_some_and(|e| e.contains("not in this test")));
    // File ▸ Join live session opens the dialog.
    h.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    crate::menus::invoke(&mut h.app, "collab.join");
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Join live session") && has_text(&shapes, "Type the join code you were given, or paste an invite link."));
    assert_eq!(h.app.inspect()["live"]["joinDialog"], true);
    h.click(text_center(&shapes, "Cancel"));
    h.frames(1);
    assert!(h.app.ui.live.join.is_none());
}
