//! The Missing fonts window's Download button (#754), clicked like a user would, against a
//! scripted font library: the font is saved, the toast says so and the window closes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use egui::{Event, PointerButton, Pos2};
use figcraft_engine::Session;
use figcraft_engine::figcraft_collab::message::ClientMsg;
use figcraft_engine::fonts::{FOLDER, LIBRARY};
use figcraft_engine::live::{Event as LiveEvent, HostOptions, LiveLink, Presence, SessionInfo};
use serde_json::{Value, json};
use write_fonts::read::FontRef;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::types::NameId;

use crate::layers_drag_tests::Ui;

/// The library's files by URL, and the URLs asked for.
type Wire = (BTreeMap<String, Vec<u8>>, Vec<String>);

struct Library(Arc<Mutex<Wire>>);

impl LiveLink for Library {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        Vec::new()
    }
    fn fetch(&mut self, url: &str, _: usize) -> Result<Option<Vec<u8>>, String> {
        let mut w = self.0.lock().unwrap();
        w.1.push(url.to_string());
        Ok(w.0.get(url).cloned())
    }
}

/// A library whose answers never come: requests go out on their own thread and stay there.
struct Slow;

impl LiveLink for Slow {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        Vec::new()
    }
    fn background(&mut self, _: u64, _: figcraft_engine::live::NetRequest) -> bool {
        true
    }
}

/// An installed font file (not a collection) renamed to `family`, if this machine has fonts.
fn font_named(family: &str) -> Option<Vec<u8>> {
    figcraft_text::scan_system_fonts();
    let bytes = figcraft_text::family_names().into_iter().find_map(|f| {
        let face = figcraft_text::find_face(&f, "Regular")?;
        let bytes = face.bytes.data().to_vec();
        (face.index == 0 && FontRef::new(&bytes).is_ok()).then_some(bytes)
    })?;
    let rec = |id: NameId, s: &str| NameRecord::new(3, 1, 0x409, id, s.to_string().into());
    let name = Name::new(vec![rec(NameId::FAMILY_NAME, family), rec(NameId::SUBFAMILY_NAME, "Regular")]);
    Some(write_fonts::FontBuilder::new().add_table(&name).unwrap().copy_missing_tables(FontRef::new(&bytes).unwrap()).build())
}

/// Where `label` was drawn in the last frame.
fn drawn_at(u: &mut Ui, label: &str) -> Option<Pos2> {
    fn walk(shape: &egui::Shape, label: &str, out: &mut Option<Pos2>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => *out = Some(t.pos + t.galley.rect.center().to_vec2()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, label, out)),
            _ => {}
        }
    }
    let mut at = None;
    u.frame_output(vec![]).shapes.iter().for_each(|c| walk(&c.shape, label, &mut at));
    at
}

#[test]
fn download_saves_the_font_and_closes_the_window() {
    let family = "FigCraft UI Download";
    let Some(font) = font_named(family) else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let base = format!("{LIBRARY}ofl/figcraftuidownload/");
    let files = BTreeMap::from([
        (format!("{base}METADATA.pb"), format!("name: \"{family}\"\nlicense: \"OFL\"\nfonts {{\n  filename: \"UI-Regular.ttf\"\n}}\n").into_bytes()),
        (format!("{base}UI-Regular.ttf"), font),
        (format!("{base}OFL.txt"), b"Licence text".to_vec()),
    ]);
    let wire = Arc::new(Mutex::new((files, Vec::new())));
    let mut s = Session::new();
    s.live.install(Box::new(Library(wire.clone())));
    let root = std::env::temp_dir().join(format!("figcraft-ui-font-download-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    s.fonts_dir = Some(root.clone());
    s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0, "props": {"fontFamily": family}})).unwrap();
    let mut u = Ui::new(s);
    u.app.run("text.missingFonts", json!({}));
    u.frame(vec![]);
    assert!(wire.lock().unwrap().1.is_empty(), "nothing is sent before the click");

    let at = drawn_at(&mut u, "Download").expect("a Download button");
    let click = |down| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE };
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![click(true)]);
    u.frame(vec![click(false)]);
    u.frame(vec![]);

    assert_eq!(wire.lock().unwrap().1.len(), 3, "its entry, the font and the licence");
    assert!(root.join(FOLDER).join(family).join("UI-Regular.ttf").is_file());
    assert!(figcraft_text::has_family(family));
    assert_eq!(u.app.inspect()["missingFonts"], Value::Null, "nothing is missing any more");
    let toast = u.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert_eq!(toast, format!("Downloaded {family}"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn closing_the_window_drops_a_download_on_its_way() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    s.live.install(Box::new(Slow));
    s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0, "props": {"fontFamily": "FigCraft UI Slow"}})).unwrap();
    let mut u = Ui::new(s);
    u.app.run("text.missingFonts", json!({}));
    u.frame(vec![]);
    let at = drawn_at(&mut u, "Download").expect("a Download button");
    let click = |down| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE };
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![click(true)]);
    u.frame(vec![click(false)]);
    assert!(u.app.session.waiting_for("text.downloadFonts"));
    assert!(drawn_at(&mut u, "Downloading…").is_some(), "the button says so");
    let escape = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    u.frame(vec![escape]);
    u.frame(vec![]);
    assert!(!u.app.session.waiting_for("text.downloadFonts"), "closed, the download is dropped");
}

#[test]
fn the_actions_palette_opens_the_window_instead_of_downloading() {
    let mut s = Session::new();
    s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0, "props": {"fontFamily": "FigCraft UI Palette"}})).unwrap();
    let mut u = Ui::new(s);
    crate::menus::invoke(&mut u.app, "text.downloadFonts");
    assert!(u.app.ui.missing_fonts.is_some());
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}
