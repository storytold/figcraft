//! The home screen's recent files, drawn headless: cards with the picture each file carries on its
//! page's colour (read off the frame's thread, only for cards in view), "Edited …" and "File not
//! found", the sort menu and the table's headers, grid and list kept in the preferences and set
//! through `ui.set`, a click that selects, a double-click or Enter that opens, the arrow keys and
//! Tab, the context menu, and the pure parts: relative times, columns and sorting.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Shape};
use figcraft_engine::{Prefs, Session};
use serde_json::{Value, json};

use crate::Services;
use crate::home_recents::{CARD_MAX, CARD_MIN, GAP, Shown, Sort, SortKey, Stat, View, ago, edited, grid_columns, sorted};
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

const HOUR: u64 = 3600;
const DAY: u64 = 86_400;

#[test]
fn relative_times_at_each_boundary() {
    let now = 1_800_000_000; // 2027-01-15 08:00 UTC
    let cases = [
        (0, "just now"),
        (59, "just now"),
        (60, "1 minute ago"),
        (119, "1 minute ago"),
        (120, "2 minutes ago"),
        (HOUR - 1, "59 minutes ago"),
        (HOUR, "1 hour ago"),
        (2 * HOUR, "2 hours ago"),
        (DAY - 1, "23 hours ago"),
        (DAY, "yesterday"),
        (2 * DAY - 1, "yesterday"),
        (2 * DAY, "2 days ago"),
        (7 * DAY - 1, "6 days ago"),
        (7 * DAY, "Jan 8, 2027"),
    ];
    for (d, want) in cases {
        assert_eq!(ago(now, now - d), want, "{d} seconds");
    }
    // A clock that went backwards reads as now; dates far back and the epoch itself.
    assert_eq!(ago(now, now + 500), "just now");
    assert_eq!(ago(now, 951_782_400), "Feb 29, 2000");
    assert_eq!(ago(now, 0), "Jan 1, 1970");
    assert_eq!(ago(now, 1_709_251_199), "Feb 29, 2024", "the last second of a leap day");
    assert_eq!(edited(now, now - 3 * HOUR), "Edited 3 hours ago");
}

#[test]
fn columns_follow_the_width() {
    for (width, columns) in
        [(100.0, 1), (259.0, 1), (300.0, 1), (551.0, 1), (552.0, 2), (700.0, 2), (850.0, 3), (1120.0, 3), (1600.0, 5), (3000.0, 8)]
    {
        let (n, card) = grid_columns(width);
        assert_eq!(n, columns, "{width}");
        assert!(card <= CARD_MAX, "{width}: {card}");
        if n > 1 {
            assert!(card >= CARD_MIN, "{width}: {card}");
            assert!(n as f32 * card + (n as f32 - 1.0) * GAP <= width + 0.01, "{width}: fits");
        }
    }
    // Nonsense widths still give one column.
    assert_eq!(grid_columns(f32::NAN).0, 1);
    assert_eq!(grid_columns(-5.0).0, 1);
    // A card's picture is 16:9 over a 58-point footer.
    let (picture, card) = crate::home_recents::card_heights(352.0);
    assert_eq!((picture, card), (198.0, 256.0));
}

fn shown(path: &str, modified: Option<u64>, created: Option<u64>) -> Shown {
    let stat = Some(Stat { exists: modified.is_some(), modified, created });
    Shown { path: path.into(), name: Path::new(path).file_stem().unwrap().to_string_lossy().into(), stat, kind: Default::default() }
}

#[test]
fn sorting_by_modified_created_and_name_either_way() {
    let files = || {
        vec![
            shown("/a/beta.fig", Some(300), Some(10)),
            shown("/a/Alpha.figcraft", Some(100), Some(30)),
            shown("/b/gone.figcraft", None, None),
            shown("/a/charlie.svg", Some(200), None),
        ]
    };
    let names = |sort: Sort| sorted(files(), sort).into_iter().map(|f| f.name).collect::<Vec<_>>();
    assert_eq!(Sort::default(), Sort { key: SortKey::Modified, descending: true });
    assert_eq!(names(Sort::default()), ["beta", "charlie", "Alpha", "gone"]);
    assert_eq!(names(Sort { key: SortKey::Modified, descending: false }), ["Alpha", "charlie", "beta", "gone"], "unknown dates last either way");
    assert_eq!(names(Sort::by(SortKey::Name)), ["Alpha", "beta", "charlie", "gone"], "A to Z, whatever the case");
    assert_eq!(names(Sort { key: SortKey::Name, descending: true }), ["gone", "charlie", "beta", "Alpha"]);
    assert_eq!(names(Sort::by(SortKey::Created)), ["Alpha", "beta", "gone", "charlie"], "newest first, then the list's order");
    // A header click: the other way round on the current column, else that column's usual way.
    let s = Sort::default();
    assert_eq!(s.clicked(SortKey::Modified), Sort { key: SortKey::Modified, descending: false });
    assert_eq!(s.clicked(SortKey::Name), Sort { key: SortKey::Name, descending: false });
    assert_eq!(s.clicked(SortKey::Name).clicked(SortKey::Name), Sort { key: SortKey::Name, descending: true });
}

#[test]
fn view_and_sort_are_kept_in_the_preferences_and_come_back() {
    let mut prefs = Prefs::default();
    assert_eq!((View::of(&prefs), Sort::of(&prefs)), (View::Grid, Sort::default()));
    View::List.store(&mut prefs);
    Sort { key: SortKey::Name, descending: true }.store(&mut prefs);
    let back: Prefs = serde_json::from_value(serde_json::to_value(&prefs).unwrap()).unwrap();
    assert_eq!((View::of(&back), Sort::of(&back)), (View::List, Sort { key: SortKey::Name, descending: true }));
    assert_eq!(back.home_views.get("recents").map(String::as_str), Some("list"), "kept per section");
    // Old preferences without them, and junk in them, read as the defaults.
    let old: Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert_eq!((View::of(&old), Sort::of(&old)), (View::Grid, Sort::default()));
    let junk: Prefs = serde_json::from_value(json!({"home_views": {"recents": "tiles"}, "home_sort": "size"})).unwrap();
    assert_eq!((View::of(&junk), Sort::of(&junk)), (View::Grid, Sort::default()));
}

/// What the services were asked to do.
#[derive(Default)]
struct Log {
    saved: Vec<Prefs>,
    revealed: Vec<PathBuf>,
}

struct Watch(Arc<Mutex<Log>>);
impl Services for Watch {
    fn save_prefs(&mut self, prefs: &Prefs) {
        self.0.lock().unwrap().saved.push(prefs.clone());
    }
    fn reveal_path(&mut self, path: &Path) {
        self.0.lock().unwrap().revealed.push(path.to_path_buf());
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A folder of recent files: `pic.figcraft` (a picture on a dark page, edited 3 hours ago),
/// `empty.figcraft` (nothing on a light grey page, edited 2 days ago), `art.fig` (a picture, edited
/// just now), `icon.svg`, and `gone.figcraft` (missing).
fn files(name: &str) -> (PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("figcraft-home-recents-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = |rgb: f32| {
        let mut d = figcraft_doc::Document::new("File");
        let p = d.first_page().unwrap();
        if let Some(figcraft_doc::NodeKind::Page { background, .. }) = d.node_mut(p).map(|n| &mut n.kind) {
            *background = figcraft_color::Color { r: rgb, g: rgb, b: rgb, a: 1.0 };
        }
        d
    };
    let write = |file: &str, bytes: Vec<u8>, ago: u64| {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap();
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(ago)).unwrap();
        path.to_string_lossy().into_owned()
    };
    let paths = vec![
        write("pic.figcraft", figcraft_format::save_bytes(&doc(0.1), Some(&png(64, 32))).unwrap(), 3 * HOUR + 60),
        write("empty.figcraft", figcraft_format::save_bytes(&doc(0.9), None).unwrap(), 2 * DAY + 60),
        write("art.fig", figcraft_format::fig::export(&doc(0.2), Some(&png(20, 40))).unwrap().0, 0),
        write("icon.svg", b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(), 5 * 60),
        dir.join("gone.figcraft").to_string_lossy().into_owned(),
    ];
    (dir, paths)
}

fn home(name: &str) -> (Ui, Vec<String>, Arc<Mutex<Log>>) {
    let (_, paths) = files(name);
    let mut s = Session::empty();
    s.prefs.recent_files = paths.clone();
    s.prefs.getting_started_dismissed = true;
    let mut u = Ui::new(s);
    u.app.ui.home = true;
    let log = Arc::new(Mutex::new(Log::default()));
    u.app.services = Box::new(Watch(log.clone()));
    settle(&mut u);
    (u, paths, log)
}

fn recents(u: &Ui) -> Value {
    u.app.inspect()["homeRecents"].clone()
}

/// Frames until every file has been looked at and every picture in view is in (the reader is a
/// thread of its own) and the fades are over.
fn settle(u: &mut Ui) -> Vec<Shape> {
    for _ in 0..400 {
        u.frame(vec![]);
        let r = recents(u);
        let files = r["files"].as_array().cloned().unwrap_or_default();
        if files.iter().all(|f| !f["exists"].is_null() && f["picture"] != "loading" && (f["exists"] == false || f["picture"] != "none")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..20 {
        u.frame(vec![]);
    }
    flat(&u.frame_output(vec![]).shapes)
}

fn file<'a>(r: &'a Value, name: &str) -> &'a Value {
    r["files"].as_array().unwrap().iter().find(|f| f["name"] == name).unwrap_or_else(|| panic!("no {name}: {r}"))
}

fn click_with(u: &mut Ui, p: Pos2, button: PointerButton) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn click(u: &mut Ui, p: Pos2) {
    click_with(u, p, PointerButton::Primary);
}

fn double_click(u: &mut Ui, p: Pos2) {
    let mut events = vec![Event::PointerMoved(p)];
    for _ in 0..2 {
        for pressed in [true, false] {
            events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
    }
    for e in events {
        u.frame(vec![e]);
    }
    u.frame(vec![]);
}

fn press(u: &mut Ui, key: Key) {
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
}

fn texts(s: &[Shape]) -> Vec<String> {
    s.iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// Filled rectangles in exactly `c`.
fn rects_of(s: &[Shape], c: egui::Color32) -> usize {
    s.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == c)).count()
}

/// Preview images in the main content area, excluding the branded sidebar icon.
fn images(s: &[Shape]) -> usize {
    s.iter()
        .filter(|s| matches!(s, Shape::Mesh(m) if m.texture_id != egui::TextureId::default() && m.vertices.iter().any(|v| v.pos.x > 240.0)))
        .count()
}

#[test]
fn cards_show_pictures_on_their_page_colour_with_name_and_time() {
    let (mut u, _, _) = home("cards");
    let s = settle(&mut u);
    let r = recents(&u);
    assert_eq!(r["view"], "grid");
    assert_eq!(r["sort"], "modified");
    // Last modified, newest first; the missing file last.
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["art", "icon", "pic", "empty", "gone"]);
    assert_eq!(file(&r, "pic")["picture"], "picture");
    assert_eq!(file(&r, "art")["picture"], "picture");
    assert_eq!(file(&r, "empty")["picture"], "page", "nothing on it: its page colour alone");
    assert_eq!(file(&r, "icon")["picture"], "placeholder");
    assert_eq!(file(&r, "gone")["exists"], false);
    assert_eq!(images(&s), 2, "two pictures drawn");
    // Each picture sits on its file's page colour (not one fixed colour).
    let page = |v: f32| crate::theme::c32(figcraft_color::Color { r: v, g: v, b: v, a: 1.0 });
    assert!(rects_of(&s, page(0.1)) >= 1 && rects_of(&s, page(0.9)) >= 1 && rects_of(&s, page(0.2)) >= 1);
    // The footer: the file's name and when it was edited, or that it's gone.
    for want in
        ["pic", "Edited 3 hours ago", "empty", "Edited 2 days ago", "art", "Edited just now", "Edited 5 minutes ago", "gone", "File not found"]
    {
        assert!(has_text(&s, want), "{want}: {:?}", texts(&s));
    }
    // The toolbar.
    assert!(has_text(&s, "Recent files") && has_text(&s, "Last modified"));
}

#[test]
fn only_cards_in_view_read_their_pictures() {
    let (dir, mut paths) = files("visible");
    // Twelve files: the last rows are below the window.
    let bytes = std::fs::read(&paths[0]).unwrap();
    for i in 0..8 {
        let p = dir.join(format!("more{i}.figcraft"));
        std::fs::write(&p, &bytes).unwrap();
        paths.push(p.to_string_lossy().into_owned());
    }
    let mut s = Session::empty();
    s.prefs.recent_files = paths;
    s.prefs.getting_started_dismissed = true;
    s.prefs.home_sort = "name".into();
    s.prefs.home_order = "asc".into();
    let mut u = Ui::new(s);
    u.app.ui.home = true;
    settle(&mut u);
    let r = recents(&u);
    let pictures: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["picture"].as_str().unwrap()).collect();
    assert_eq!(pictures.len(), 12);
    // A to Z: art.fig, empty.figcraft, gone.figcraft (missing: nothing to read) on the first row.
    assert_eq!(pictures.get(..3), Some(["picture", "page", "none"].as_slice()), "the first row is read: {pictures:?}");
    assert!(pictures.iter().skip(9).all(|p| *p == "none"), "the last row is not: {pictures:?}");
}

#[test]
fn a_click_selects_a_double_click_opens() {
    let (mut u, paths, _) = home("open");
    let s = settle(&mut u);
    let at = text_center(&s, "pic");
    click(&mut u, at);
    assert!(u.app.ui.home, "a click doesn't open");
    assert!(u.app.session.docs.is_empty());
    assert_eq!(file(&recents(&u), "pic")["selected"], true);
    // The selection is drawn as an accent border.
    let s = settle(&mut u);
    let border = s.iter().any(|s| matches!(s, Shape::Rect(r) if r.stroke.width == 2.0 && r.stroke.color == crate::theme::ACCENT));
    assert!(border, "a 2-point accent border");
    double_click(&mut u, at);
    assert!(!u.app.ui.home, "a double-click opens");
    assert_eq!(u.app.session.ds().unwrap().path.as_deref(), Some(paths[0].as_str()));
}

#[test]
fn tab_reaches_the_cards_named_for_screen_readers_arrows_move_and_enter_opens() {
    let (mut u, paths, _) = home("keys");
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes: Vec<_> = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect();
    for name in ["art, Edited just now", "pic, Edited 3 hours ago", "gone, File not found"] {
        assert!(nodes.iter().any(|n| n.role() == Role::Button && n.label() == Some(name)), "no button named {name:?}");
    }
    let mut reached = false;
    for _ in 0..40 {
        press(&mut u, Key::Tab);
        if crate::a11y::focused_name(&u.ctx).starts_with("art, ") {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the first card");
    assert_eq!(file(&recents(&u), "art")["selected"], true, "the card with the keyboard is selected");
    // Right: the next card; Down: the one below (three columns); Left and Up back.
    press(&mut u, Key::ArrowRight);
    u.frame(vec![]);
    assert!(crate::a11y::focused_name(&u.ctx).starts_with("icon, "), "{}", crate::a11y::focused_name(&u.ctx));
    assert_eq!(file(&recents(&u), "icon")["selected"], true);
    assert_eq!(file(&recents(&u), "art")["selected"], false);
    press(&mut u, Key::ArrowRight);
    press(&mut u, Key::ArrowRight);
    u.frame(vec![]);
    assert!(crate::a11y::focused_name(&u.ctx).starts_with("empty, "), "Right wraps to the next row");
    press(&mut u, Key::ArrowUp);
    u.frame(vec![]);
    assert!(crate::a11y::focused_name(&u.ctx).starts_with("art, "));
    press(&mut u, Key::ArrowDown);
    press(&mut u, Key::ArrowLeft);
    u.frame(vec![]);
    assert!(crate::a11y::focused_name(&u.ctx).starts_with("pic, "), "{}", crate::a11y::focused_name(&u.ctx));
    assert!(u.app.ui.home, "nothing opened yet");
    press(&mut u, Key::Enter);
    u.frame(vec![]);
    assert!(!u.app.ui.home);
    assert_eq!(u.app.session.ds().unwrap().path.as_deref(), Some(paths[0].as_str()));
}

#[test]
fn the_context_menu_opens_shows_in_the_folder_and_removes() {
    let (mut u, paths, log) = home("menu");
    let s = settle(&mut u);
    click_with(&mut u, text_center(&s, "icon"), PointerButton::Secondary);
    let s = flat(&u.frame_output(vec![]).shapes);
    assert_eq!(file(&recents(&u), "icon")["selected"], true, "a right-click selects");
    let reveal = if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in folder"
    };
    for item in ["Open", reveal, "Remove from recents"] {
        assert!(has_text(&s, item), "{item}");
    }
    click(&mut u, text_center(&s, reveal));
    assert_eq!(log.lock().unwrap().revealed, vec![PathBuf::from(&paths[3])]);
    let s = settle(&mut u);
    click_with(&mut u, text_center(&s, "icon"), PointerButton::Secondary);
    let s = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&s, "Remove from recents"));
    assert!(!u.app.session.prefs.recent_files.contains(&paths[3]));
    assert_eq!(u.app.session.prefs.recent_files.len(), 4);
    assert!(log.lock().unwrap().saved.last().is_some_and(|p| p.recent_files.len() == 4), "kept");
    let s = settle(&mut u);
    assert!(!has_text(&s, "icon"));
    // Open from the menu.
    click_with(&mut u, text_center(&s, "pic"), PointerButton::Secondary);
    let s = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&s, "Open"));
    assert!(!u.app.ui.home);
    assert_eq!(u.app.session.ds().unwrap().path.as_deref(), Some(paths[0].as_str()));
}

#[test]
fn the_sort_menu_and_the_view_buttons_are_kept_in_the_preferences() {
    let (mut u, _, log) = home("toolbar");
    let s = settle(&mut u);
    click(&mut u, text_center(&s, "Last modified"));
    let s = flat(&u.frame_output(vec![]).shapes);
    for item in ["Sort by", "Order"] {
        assert!(has_text(&s, item), "{item}");
    }
    assert!(texts(&s).iter().any(|t| t.contains('✓') && t.contains("Last modified")), "the current sort is checked: {:?}", texts(&s));
    assert!(texts(&s).iter().any(|t| t.contains('✓') && t.contains("Newest first")));
    assert!(texts(&s).iter().any(|t| t.ends_with("Oldest first")));
    let alphabetical = texts(&s).into_iter().find(|t| t.ends_with("Alphabetical")).unwrap();
    click(&mut u, text_center(&s, &alphabetical));
    assert_eq!(Sort::of(&u.app.session.prefs), Sort { key: SortKey::Name, descending: false });
    assert_eq!(log.lock().unwrap().saved.last().map(|p| p.home_sort.clone()).as_deref(), Some("name"));
    let s = settle(&mut u);
    let names: Vec<String> = recents(&u)["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["art", "empty", "gone", "icon", "pic"]);
    assert!(has_text(&s, "Alphabetical"), "the button names the sort");
    // The list button: a table, no pictures.
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes: Vec<_> = out.platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect();
    let list = nodes.iter().find(|n| n.label() == Some("Show as list")).expect("a list button");
    let b = list.bounds().unwrap();
    click(&mut u, Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
    assert_eq!(View::of(&u.app.session.prefs), View::List);
    assert_eq!(log.lock().unwrap().saved.last().and_then(|p| p.home_views.get("recents").cloned()).as_deref(), Some("list"));
    let s = settle(&mut u);
    assert_eq!(images(&s), 0, "no pictures in the list");
    for want in ["Name", "Last modified", "3 hours ago", "2 days ago", "File not found", "pic"] {
        assert!(has_text(&s, want), "{want}: {:?}", texts(&s));
    }
    // A header sorts by its column, and again the other way round.
    let header = |s: &[Shape]| {
        let mut hits: Vec<Pos2> = texts_at(s, "Last modified");
        hits.sort_by(|a, b| b.y.total_cmp(&a.y));
        hits[0]
    };
    click(&mut u, header(&s));
    assert_eq!(Sort::of(&u.app.session.prefs), Sort { key: SortKey::Modified, descending: true });
    let s = settle(&mut u);
    click(&mut u, header(&s));
    assert_eq!(Sort::of(&u.app.session.prefs), Sort { key: SortKey::Modified, descending: false });
    let s = settle(&mut u);
    assert!(text_center(&s, "empty").y < text_center(&s, "art").y, "oldest first");
    assert_eq!(texts_at(&s, "Last modified").len(), 2, "the header and the dropdown read the same");
    // Kept, the preferences give the same view and sort back.
    let kept = log.lock().unwrap().saved.last().cloned().unwrap();
    let back: Prefs = serde_json::from_value(serde_json::to_value(&kept).unwrap()).unwrap();
    assert_eq!((View::of(&back), Sort::of(&back)), (View::List, Sort { key: SortKey::Modified, descending: false }));
}

/// The middle of each text reading `s`.
fn texts_at(shapes: &[Shape], s: &str) -> Vec<Pos2> {
    shapes
        .iter()
        .filter_map(|sh| match sh {
            Shape::Text(t) if t.galley.text() == s => Some(t.pos + t.galley.size() / 2.0),
            _ => None,
        })
        .collect()
}

#[test]
fn ui_set_switches_the_view_and_the_sort() {
    let (mut u, _, _) = home("uiset");
    let ctx = u.ctx.clone();
    crate::control::handle(&mut u.app, &ctx, "ui.set", &json!({"homeRecentsView": "list", "homeRecentsSort": "name", "homeRecentsOrder": "desc"}))
        .unwrap();
    let r = recents(&u);
    assert_eq!((r["view"].as_str(), r["sort"].as_str(), r["order"].as_str()), (Some("list"), Some("name"), Some("desc")));
    // Unknown values change nothing.
    crate::control::handle(&mut u.app, &ctx, "ui.set", &json!({"homeRecentsView": "tiles", "homeRecentsSort": "size", "homeRecentsOrder": "up"}))
        .unwrap();
    let r = recents(&u);
    assert_eq!((r["view"].as_str(), r["sort"].as_str(), r["order"].as_str()), (Some("list"), Some("name"), Some("desc")));
    crate::control::handle(&mut u.app, &ctx, "ui.set", &json!({"homeRecentsView": "grid", "homeRecentsSort": "modified"})).unwrap();
    let r = recents(&u);
    assert_eq!((r["view"].as_str(), r["sort"].as_str(), r["order"].as_str()), (Some("grid"), Some("modified"), Some("desc")));
}

#[test]
fn a_missing_file_still_opens_as_before() {
    let (mut u, paths, _) = home("missing");
    let s = settle(&mut u);
    double_click(&mut u, text_center(&s, "gone"));
    // `file.open` fails: the home screen stays.
    assert!(u.app.ui.home);
    assert!(u.app.session.docs.is_empty());
    assert!(u.app.session.prefs.recent_files.contains(&paths[4]), "the list is unchanged");
}

#[test]
fn the_home_screen_draws_no_frames_once_the_pictures_are_in() {
    let (_, paths) = files("idle");
    let (mut h, _) = crate::live_tests::Harness::new();
    h.app.session.prefs.recent_files = paths;
    h.app.session.prefs.getting_started_dismissed = true;
    h.app.ui.home = true;
    for _ in 0..400 {
        h.frame(vec![]);
        let r = h.app.inspect()["homeRecents"].clone();
        if r["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| !f["exists"].is_null() && f["picture"] != "loading" && (f["exists"] == false || f["picture"] != "none"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    h.frames(30);
    let (_, delay) = h.frame(vec![]);
    assert!(delay >= Duration::from_secs(1), "a frame in {delay:?}: {:?}", h.ctx.repaint_causes());
    assert_eq!(file(&h.app.inspect()["homeRecents"], "pic")["picture"], "picture");
}

#[test]
fn files_deleted_while_in_view_show_as_gone() {
    let (_, paths) = files("deleted");
    let (mut h, _) = crate::live_tests::Harness::new();
    h.app.session.prefs.recent_files = paths.clone();
    h.app.session.prefs.getting_started_dismissed = true;
    h.app.ui.home = true;
    let exists = |h: &mut crate::live_tests::Harness, name: &str| file(&h.app.inspect()["homeRecents"], name)["exists"].clone();
    // Up to `n` frames (1/60 s of UI time each), giving the reader thread time between them.
    let wait_n = |h: &mut crate::live_tests::Harness, name: &str, want: bool, n: usize| {
        for _ in 0..n {
            h.frame(vec![]);
            if exists(h, name) == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    };
    let wait = |h: &mut crate::live_tests::Harness, name: &str, want: bool| wait_n(h, name, want, 400);
    assert!(wait(&mut h, "pic", true) && wait(&mut h, "art", true));
    // Deleted while the recent files stay in view: within a couple of seconds.
    std::fs::remove_file(&paths[0]).unwrap();
    h.frames(5);
    assert_eq!(exists(&mut h, "pic"), true, "not read again on every frame");
    h.skip(2.5);
    assert!(wait(&mut h, "pic", false), "read again after a while");
    // Deleted while the window was in the background: as soon as it has the focus again.
    std::fs::remove_file(&paths[2]).unwrap();
    h.frame(vec![egui::Event::WindowFocused(false)]);
    h.frame(vec![egui::Event::WindowFocused(true)]);
    // Well under the regular interval, so only the focus can have caused it.
    assert!(wait_n(&mut h, "art", false, 30), "read again on getting the focus back");
}
