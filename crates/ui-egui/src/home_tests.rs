//! The home screen, drawn headless: the Templates grid (opened by click and by keyboard, searched,
//! filtered, its empty state, its names for screen readers, thumbnails cached per theme and no
//! frames once they are in), the first-run row (only without recent files) and the Getting
//! started card (closed, kept closed, and brought back from the Help menu).

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape};
use figcraft_engine::Session;
use figcraft_engine::templates::TEMPLATES;
use serde_json::json;

use crate::home::{Section, cached_thumbnails};
use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center, texts};

/// The app as a first run finds it: no documents, no recent files.
fn first_run() -> Ui {
    Ui::new(Session::empty())
}

fn shapes(u: &mut Ui) -> Vec<Shape> {
    flat(&u.frame_output(vec![]).shapes)
}

/// Enough frames for every thumbnail in view (they render one a frame) and what follows a click,
/// then until every recent file has been looked at: the reader is a thread of its own, and until it
/// answers a file's card is a skeleton without its name (on a loaded machine, 12 frames weren't
/// always enough, #351).
fn settle(u: &mut Ui) -> Vec<Shape> {
    for _ in 0..12 {
        u.frame(vec![]);
    }
    for _ in 0..400 {
        let files = u.app.inspect()["homeRecents"]["files"].as_array().cloned().unwrap_or_default();
        if files.iter().all(|f| !f["exists"].is_null()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        u.frame(vec![]);
    }
    shapes(u)
}

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn press(u: &mut Ui, key: Key) {
    for pressed in [true, false] {
        u.frame(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]);
    }
}

fn templates_tab(u: &mut Ui) -> Vec<Shape> {
    let s = shapes(u);
    click(u, text_center(&s, "Templates"));
    assert_eq!(u.app.ui.home_view.section, Section::Templates);
    settle(u)
}

#[test]
fn the_sidebar_has_recents_online_and_templates_and_no_placeholders() {
    let mut u = first_run();
    let s = shapes(&mut u);
    assert!(has_text(&s, "Recents") && has_text(&s, "Online") && has_text(&s, "Templates"));
    assert!(!has_text(&s, "Drafts") && !has_text(&s, "Community"));
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes: Vec<_> = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect();
    let tab = |label: &str| nodes.iter().find(|n| n.role() == Role::Tab && n.label() == Some(label)).unwrap_or_else(|| panic!("no tab {label}"));
    assert_eq!((tab("Recents").is_selected(), tab("Online").is_selected(), tab("Templates").is_selected()), (Some(true), Some(false), Some(false)));
    // In reading order: Recents, Online, Templates.
    let top = |label: &str| tab(label).bounds().unwrap().y0;
    assert!(top("Recents") < top("Online") && top("Online") < top("Templates"));
    // Each is clicked into, and `ui.inspect` and `ui.set {homeSection}` name them the same way.
    for (label, section) in [("Online", Section::Online), ("Templates", Section::Templates), ("Recents", Section::Recents)] {
        let s = shapes(&mut u);
        click(&mut u, text_center(&s, label));
        assert_eq!(u.app.ui.home_view.section, section);
        let inspect = u.app.inspect();
        assert_eq!(inspect["homeSection"], section.id());
        assert_eq!(inspect["online"]["open"], section == Section::Online);
        assert_eq!(Section::parse(section.id()), Some(section));
    }
    assert_eq!(Section::parse("drafts"), None);
    // The Online section is the online files' own.
    u.app.ui.home_view.section = Section::Online;
    let s = settle(&mut u);
    assert!(has_text(&s, "Online files") && !has_text(&s, "Getting started") && !has_text(&s, "Start from a template"));
}

#[test]
fn the_templates_grid_shows_every_template_and_a_click_opens_a_new_document() {
    let mut u = first_run();
    let s = templates_tab(&mut u);
    for t in TEMPLATES {
        assert!(has_text(&s, t.name), "{}", t.name);
    }
    assert!(has_text(&s, "Apps · 390 × 844"), "the kind and size tag");
    assert_eq!(cached_thumbnails(&u.ctx, false).len(), TEMPLATES.len(), "every card has its picture");
    click(&mut u, text_center(&s, "Dashboard"));
    assert_eq!(u.app.session.docs.len(), 1);
    let ds = u.app.session.ds().unwrap();
    assert_eq!(ds.doc.name, "Dashboard");
    assert!(ds.path.is_none() && !ds.is_dirty(), "a new, untitled-on-disk document");
    assert!(!u.app.ui.home, "and the editor shows it");
    // Again: another document, the first untouched.
    let before = u.app.session.docs[0].doc.clone();
    u.app.ui.home = true;
    let s = settle(&mut u);
    // (Its tag: the name is also on the open document's tab now.)
    click(&mut u, text_center(&s, "Apps · 1440 × 960"));
    assert_eq!(u.app.session.docs.len(), 2);
    assert!(std::sync::Arc::ptr_eq(&u.app.session.docs[0].doc, &before));
}

#[test]
fn cards_are_reached_with_tab_named_for_screen_readers_and_opened_with_enter() {
    let mut u = first_run();
    templates_tab(&mut u);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes: Vec<_> = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect();
    for t in TEMPLATES {
        let name = format!("{}: {}", t.name, t.description);
        assert!(nodes.iter().any(|n| n.role() == Role::Button && n.label() == Some(name.as_str())), "no button named {name:?}");
    }
    assert!(nodes.iter().any(|n| n.label() == Some("Search templates")));
    // Tab walks the controls; the first card it reaches opens with Enter.
    let mut reached = false;
    for _ in 0..40 {
        press(&mut u, Key::Tab);
        if crate::a11y::focused_name(&u.ctx).starts_with("Mobile app: ") {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the first card");
    press(&mut u, Key::Enter);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().name, "Mobile app");
    assert!(!u.app.ui.home);
}

#[test]
fn search_and_kind_filter_the_grid_and_nothing_found_offers_a_way_back() {
    let mut u = first_run();
    templates_tab(&mut u);
    u.app.ui.home_view.query = "icon".into();
    let s = settle(&mut u);
    assert!(has_text(&s, "Icon set starter") && !has_text(&s, "Dashboard"));
    // Descriptions and kinds match too, whatever the case.
    u.app.ui.home_view.query = "  PROTOTYPE ".into();
    let s = settle(&mut u);
    assert!(has_text(&s, "Mobile app") && !has_text(&s, "Landing page"));
    u.app.ui.home_view.query.clear();
    let s = settle(&mut u);
    click(&mut u, text_center(&s, "Websites"));
    let s = settle(&mut u);
    assert!(has_text(&s, "Landing page") && !has_text(&s, "Mobile app") && !has_text(&s, "Icon set starter"));
    u.app.ui.home_view.query = "zzz".into();
    let s = settle(&mut u);
    assert!(has_text(&s, "No templates match your search"));
    assert!(TEMPLATES.iter().all(|t| !has_text(&s, t.name)));
    click(&mut u, text_center(&s, "Show all templates"));
    let s = settle(&mut u);
    assert!(u.app.ui.home_view.query.is_empty() && u.app.ui.home_view.kind.is_none());
    assert!(TEMPLATES.iter().all(|t| has_text(&s, t.name)));
}

#[test]
fn typing_in_the_search_field_filters() {
    let mut u = first_run();
    let s = templates_tab(&mut u);
    let field = text_center(&s, "Search templates");
    click(&mut u, field);
    u.frame(vec![Event::Text("slides".into())]);
    let s = settle(&mut u);
    assert_eq!(u.app.ui.home_view.query, "slides");
    assert!(has_text(&s, "Presentation slides") && !has_text(&s, "Dashboard"));
    assert!(u.app.session.docs.is_empty(), "typing opens nothing");
}

#[test]
fn thumbnails_are_cached_per_theme() {
    let mut u = first_run();
    templates_tab(&mut u);
    assert_eq!(cached_thumbnails(&u.ctx, false).len(), TEMPLATES.len());
    assert!(cached_thumbnails(&u.ctx, true).is_empty());
    u.app.session.prefs.theme = "dark".into();
    settle(&mut u);
    assert_eq!(cached_thumbnails(&u.ctx, true).len(), TEMPLATES.len(), "the dark theme's pictures");
    assert_eq!(cached_thumbnails(&u.ctx, false).len(), TEMPLATES.len(), "the light ones are kept");
}

#[test]
fn the_home_screen_draws_no_frames_once_its_thumbnails_are_in() {
    let (mut h, _) = crate::live_tests::Harness::new();
    h.app.ui.home = true;
    for tab in [Section::Templates, Section::Online, Section::Recents] {
        h.app.ui.home_view.section = tab;
        h.frames(60);
        let (_, delay) = h.frame(vec![]);
        assert!(delay >= std::time::Duration::from_secs(1), "{tab:?} asks for a frame in {delay:?}: {:?}", h.ctx.repaint_causes());
    }
    assert_eq!(cached_thumbnails(&h.ctx, false).len(), TEMPLATES.len());
}

#[test]
fn a_first_run_offers_templates_on_recents_and_recent_files_replace_them() {
    let mut u = first_run();
    let s = settle(&mut u);
    assert!(has_text(&s, "Start from a template"));
    let offered = TEMPLATES.iter().filter(|t| has_text(&s, t.name)).count();
    assert_eq!(offered, 4);
    assert!(!has_text(&s, "Recent files") && !has_text(&s, "Open files"));
    click(&mut u, text_center(&s, "See all templates"));
    assert_eq!(u.app.ui.home_view.section, Section::Templates);
    // From the row: a click opens it.
    u.app.ui.home_view.section = Section::Recents;
    let s = settle(&mut u);
    click(&mut u, text_center(&s, "Landing page"));
    assert_eq!(u.app.session.doc().unwrap().name, "Landing page");
    // With a recent file, the list shows in the row's place.
    u.app.ui.home = true;
    u.app.session.prefs.recent_files = vec!["/tmp/ridgeline.figcraft".into()];
    let s = settle(&mut u);
    assert!(has_text(&s, "Recent files") && has_text(&s, "ridgeline"));
    assert!(!has_text(&s, "Start from a template") && !has_text(&s, "See all templates"));
    assert!(has_text(&s, "Open files"), "the document opened from the template is listed");
}

#[test]
fn an_open_files_long_name_and_path_share_their_row_without_overlapping() {
    let name = "Quarterly planning: onboarding flow review with annotations and handoff notes";
    let long = "/Users/alex/Documents/Clients/Northwind Traders/2026 Website relaunch/Design reviews/Week 42/onboarding-flow-review.figcraft";
    let short = "/tmp/wireframes.figcraft";
    let mut s = Session::new();
    s.execute("file.rename", &json!({"name": name})).unwrap();
    s.docs[0].path = Some(long.into());
    s.execute("file.new", &json!({"name": "Wireframes"})).unwrap();
    s.docs[1].path = Some(short.into());
    let mut u = Ui::new(s);
    u.app.ui.home = true;
    u.app.session.prefs.getting_started_dismissed = true;
    let s = settle(&mut u);
    assert!(has_text(&s, "Open files"));
    // A row that fits shows its name and path in full.
    assert!(has_text(&s, "Wireframes") && has_text(&s, short));
    // One that doesn't: the name, then the end of its path, side by side.
    let all = texts(&s);
    let rect = |(_, p, size): &(String, Pos2, egui::Vec2)| Rect::from_min_size(*p, *size);
    let detail = all.iter().find(|(t, _, _)| t.ends_with("/onboarding-flow-review.figcraft")).expect("the path");
    let detail_rect = rect(detail);
    let shown_name =
        all.iter().find(|e| e.0.starts_with('Q') && (rect(e).center().y - detail_rect.center().y).abs() < 2.0).expect("the name on the path's row");
    let name_rect = rect(shown_name);
    assert!(shown_name.0.starts_with("Quarterly planning: onboarding") && shown_name.0.ends_with('…'), "{:?}", shown_name.0);
    assert!(name_rect.max.x + 12.0 <= detail_rect.min.x, "{:?} {name_rect:?} overlaps {:?} {detail_rect:?}", shown_name.0, detail.0);
    assert!(detail.0.starts_with('…') && long.ends_with(detail.0.trim_start_matches('…')), "{:?}", detail.0);
    // Hovered, the row shows them in full.
    u.frame(vec![Event::PointerMoved(name_rect.center())]);
    for _ in 0..60 {
        u.frame(vec![]);
    }
    assert!(texts(&shapes(&mut u)).iter().any(|(t, _, _)| t.contains(long)), "no tooltip with the full path");
}

#[test]
fn the_getting_started_card_closes_stays_closed_and_comes_back_from_help() {
    let mut u = first_run();
    let s = settle(&mut u);
    assert!(has_text(&s, "Getting started"));
    for tip in ["Draw a frame", "Add auto layout", "Make a component", "Prototype", "Share with a code", "Export"] {
        assert!(has_text(&s, tip), "{tip}");
    }
    // Shortcuts come from the keymap.
    let key = crate::home::shortcut("layout.add").unwrap();
    assert!(has_text(&s, &key), "the auto layout shortcut {key:?}");
    // The close button is named, and closes the card.
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let nodes = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    let close = nodes.iter().map(|(_, n)| n).find(|n| n.label().is_some_and(|l| l.starts_with("Hide these tips"))).expect("a named close button");
    let b = close.bounds().unwrap();
    click(&mut u, Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
    assert!(u.app.session.prefs.getting_started_dismissed);
    let s = settle(&mut u);
    assert!(!has_text(&s, "Getting started") && !has_text(&s, "Draw a frame"));
    assert!(has_text(&s, "Start from a template"), "the rest of the screen stays");
    // The preference is kept with the others, so it stays closed next time.
    let kept: figcraft_engine::Prefs = serde_json::from_value(serde_json::to_value(&u.app.session.prefs).unwrap()).unwrap();
    assert!(kept.getting_started_dismissed);
    // Help ▸ Getting started: back on the home screen, from wherever.
    u.app.run("file.new", json!({}));
    u.app.ui.home = false;
    u.app.ui.home_view.section = Section::Templates;
    crate::menus::invoke(&mut u.app, "help.gettingStarted");
    assert!(u.app.ui.home && u.app.ui.home_view.section == Section::Recents && !u.app.session.prefs.getting_started_dismissed);
    assert!(has_text(&settle(&mut u), "Getting started"));
}

#[test]
fn the_file_menu_item_shows_the_templates() {
    let mut u = Ui::new(Session::new());
    u.app.ui.home = false;
    crate::menus::invoke(&mut u.app, "file.newFromTemplate");
    assert!(u.app.ui.home && u.app.ui.home_view.section == Section::Templates);
    assert_eq!(u.app.session.docs.len(), 1, "nothing opens until one is chosen");
    assert!(has_text(&settle(&mut u), "Wireframe kit"));
}

#[test]
fn tab_walks_the_sidebar_in_order_before_a_sections_controls() {
    let mut u = first_run();
    let mut names = Vec::new();
    for _ in 0..40 {
        press(&mut u, Key::Tab);
        let name = crate::a11y::focused_name(&u.ctx);
        if !name.is_empty() && names.last() != Some(&name) {
            names.push(name);
        }
    }
    let at = |name: &str| names.iter().position(|n| n == name).unwrap_or_else(|| panic!("Tab never reached {name:?}: {names:?}"));
    assert!(at("Recents") < at("Online") && at("Online") < at("Templates") && at("Templates") < at("Open file"), "{names:?}");
}

/// A temporary directory of its own for a test's files, removed with them when dropped.
struct Temp(std::path::PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("figcraft-home-drop-{}-{stamp}-{n}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Ok(files) = std::fs::read_dir(&self.0) {
            for file in files.flatten() {
                let _ = std::fs::remove_file(file.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// A file dropped on the window, as the windowing backend hands it over (read from its path).
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// Drag `path` over the window (one frame: what it shows), then drop it (one frame).
fn drop_file(u: &mut Ui, path: &std::path::Path) -> Vec<Shape> {
    let hovered = egui::HoveredFile { path: Some(path.to_path_buf()), mime: String::new() };
    let hovering = flat(&u.frame_raw(egui::RawInput { hovered_files: vec![hovered], ..Default::default() }).shapes);
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path.to_path_buf()));
    u.frame_raw(egui::RawInput { dropped_files: vec![file], ..Default::default() });
    hovering
}

/// A file dropped with its contents but no file on disk (a virtual file from another app).
#[derive(Debug)]
struct DroppedBytes(std::path::PathBuf, Vec<u8>);

impl egui::DroppedFile for DroppedBytes {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[test]
fn dropping_a_file_without_one_on_disk_opens_it_as_a_new_unsaved_document() {
    let dir = Temp::new();
    let path = dir.path("virtual.svg");
    assert!(!path.exists());
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="red"/></svg>"#.to_vec();
    let mut u = first_run();
    u.app.ui.home = true;
    let file: egui::DroppedFileHandle = std::sync::Arc::new(DroppedBytes(path.clone(), svg));
    u.frame_raw(egui::RawInput { dropped_files: vec![file], ..Default::default() });
    assert_eq!(u.app.ui.pending_opens.len(), 1);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    assert!(u.app.ui.pending_opens.is_empty());
    assert_eq!(u.app.ui.last_error, None);
    assert_eq!(u.app.session.docs.len(), 1, "the dropped contents open");
    assert_eq!(u.app.session.docs[0].path, None, "with no save path made up for them");
    assert!(u.app.session.docs[0].doc.nodes().any(|n| matches!(n.kind, figcraft_doc::NodeKind::Rectangle)), "the SVG's content");
    assert!(!u.app.ui.home, "and the editor shows it");
    assert!(!path.exists(), "nothing is written");
}

#[test]
fn dropping_a_native_file_on_the_home_screen_opens_it() {
    let dir = Temp::new();
    let path = dir.path("Dropped.figcraft");
    let mut saved = Session::new();
    saved.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(path.exists());
    let mut u = first_run();
    u.app.ui.home = true;
    let s = drop_file(&mut u, &path);
    assert!(has_text(&s, "Drop to open"), "dragging over the home screen says what a drop does");
    // The file waits behind its card, which is on screen before it opens.
    assert_eq!(u.app.ui.pending_opens.len(), 1);
    assert!(u.app.session.docs.is_empty(), "nothing opens in the frame of the drop");
    let s = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&s, "Opening as a new file") && has_text(&s, "Opening the file…") && has_text(&s, "Dropped.figcraft"), "the card shows");
    assert_eq!(u.app.ui.pending_opens.len(), 1);
    assert!(u.app.session.docs.is_empty(), "the card is drawn first");
    assert_eq!(u.app.inspect()["pendingOpens"], 1);
    u.frame(vec![]);
    assert!(u.app.ui.pending_opens.is_empty());
    assert_eq!(u.app.session.docs.len(), 1);
    assert_eq!(u.app.session.docs[0].path.as_deref(), Some(path.to_string_lossy().as_ref()));
    assert!(!u.app.ui.home, "and the editor shows it");
    assert!(u.app.ui.toast.as_ref().is_none_or(|t| !t.0.starts_with("Drop a")), "no turning away");
}

#[test]
fn dropping_a_native_file_on_canvas_keeps_its_receiving_document_after_tab_switch() {
    let dir = Temp::new();
    let path = dir.path("Canvas import.figcraft");
    let mut source = Session::new();
    source.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();

    let mut session = Session::new();
    let target_uid = session.ds().unwrap().uid;
    let target_page = session.page().unwrap();
    let target_name = session.doc().unwrap().name.clone();
    session.execute("file.new", &json!({"name": "Other tab"})).unwrap();
    let other_uid = session.ds().unwrap().uid;
    session.active = 0;
    let mut u = Ui::new(session);
    u.app.ui.home = false;
    u.frame(vec![]);
    let pos = u.app.canvas.rect.center();
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path.clone()));
    u.frame_raw(egui::RawInput { events: vec![Event::PointerMoved(pos)], dropped_files: vec![file], ..Default::default() });
    let pending = u.app.ui.pending_opens.first().unwrap();
    assert_eq!(pending.target.as_ref().map(|target| (target.uid, target.page)), Some((target_uid, target_page)));
    assert_eq!(pending.target.as_ref().map(|target| target.name.as_str()), Some(target_name.as_str()));

    u.frame(vec![]);
    u.app.session.active = 1;
    assert_eq!(u.app.session.ds().unwrap().uid, other_uid, "the UI switched tabs while the card was displayed");
    assert_eq!(u.app.ui.pending_opens.len(), 1, "the switch happens before dispatch");
    u.frame(vec![]);

    assert_eq!(u.app.session.ds().unwrap().uid, other_uid, "the user's active tab stays in front");
    assert_eq!(u.app.session.docs.iter().map(|doc| doc.doc.pages.len()).collect::<Vec<_>>(), [2, 1]);
}

#[test]
fn canvas_import_uses_queued_page_and_restores_the_page_selected_while_waiting() {
    let dir = Temp::new();
    let path = dir.path("Queued.svg");
    std::fs::write(&path, r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#f00"/></svg>"##)
        .unwrap();

    let mut session = Session::new();
    let target_uid = session.ds().unwrap().uid;
    let first_page = session.page().unwrap();
    session.execute("page.add", &json!({"name": "Second page"})).unwrap();
    let second_page = session.page().unwrap();
    let second_page_shape = session.execute("node.create", &json!({"type": "rectangle", "x": 30, "y": 40, "width": 12, "height": 14})).unwrap()["id"]
        .as_u64()
        .map(figcraft_doc::NodeId)
        .unwrap();
    session.execute("page.select", &json!({"id": first_page})).unwrap();
    session.execute("file.new", &json!({"name": "Other tab"})).unwrap();
    let other_uid = session.ds().unwrap().uid;
    session.active = 0;
    let mut u = Ui::new(session);
    u.app.ui.home = false;
    u.frame(vec![]);
    let pos = u.app.canvas.rect.center();
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path));
    u.frame_raw(egui::RawInput { events: vec![Event::PointerMoved(pos)], dropped_files: vec![file], ..Default::default() });
    let queued_params = u.app.ui.pending_opens[0].params.clone();
    assert_eq!(u.app.ui.pending_opens[0].target.as_ref().map(|target| (target.uid, target.page)), Some((target_uid, first_page)));

    u.frame(vec![]);
    u.app.session.execute("page.select", &json!({"id": second_page})).unwrap();
    u.app.session.execute("select.set", &json!({"ids": [second_page_shape]})).unwrap();
    u.app.session.active = 1;
    u.frame(vec![]);
    let target_index = u.app.session.docs.iter().position(|doc| doc.uid == target_uid).unwrap();
    let imported = u.app.session.docs[target_index].doc.node(first_page).unwrap().children[0];
    let doc = &u.app.session.docs[target_index].doc;
    assert_eq!(doc.page_of(imported), Some(first_page));
    let bounds = doc.world_bounds(imported).unwrap();
    assert!((bounds.x0 - queued_params["x"].as_f64().unwrap()).abs() < 1.0);
    assert!((bounds.y0 - queued_params["y"].as_f64().unwrap()).abs() < 1.0);
    assert_eq!(doc.node(second_page).unwrap().children, [second_page_shape]);
    assert_eq!(u.app.session.ds().unwrap().uid, other_uid);
    assert_eq!(u.app.session.docs[target_index].page, second_page, "the page selected during the wait is restored");
    assert_eq!(u.app.session.docs[target_index].selection, [second_page_shape], "the restored page keeps its selection");

    u.app.session.active = target_index;
    u.app.session.execute("edit.delete", &json!({})).unwrap();
    let doc = &u.app.session.docs[target_index].doc;
    assert!(doc.node(second_page).unwrap().children.is_empty(), "the next edit acts on the restored page selection");
    assert_eq!(doc.node(first_page).unwrap().children.len(), 1, "the imported object on the queued page remains");
}

#[test]
fn queued_canvas_import_is_cancelled_if_its_page_is_deleted() {
    let dir = Temp::new();
    let path = dir.path("Removed target.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#).unwrap();

    let mut session = Session::new();
    let target_uid = session.ds().unwrap().uid;
    session.execute("page.add", &json!({"name": "Keep this page"})).unwrap();
    let target_page = session.page().unwrap();
    session.execute("file.new", &json!({"name": "Other tab"})).unwrap();
    let other_uid = session.ds().unwrap().uid;
    session.active = 0;
    let mut u = Ui::new(session);
    u.app.ui.home = false;
    u.frame(vec![]);
    let pos = u.app.canvas.rect.center();
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path));
    u.frame_raw(egui::RawInput { events: vec![Event::PointerMoved(pos)], dropped_files: vec![file], ..Default::default() });
    u.app.session.execute("page.delete", &json!({"id": target_page})).unwrap();
    let other_before = u.app.session.docs.iter().find(|doc| doc.uid == other_uid).unwrap().doc.clone();
    u.app.ui.pending_open_frames = 2;
    crate::pending_open::pending_opens(&mut u.app, &u.ctx);
    assert_eq!(u.app.session.docs.iter().find(|doc| doc.uid == other_uid).unwrap().doc, other_before);
    let target = u.app.session.docs.iter().find(|doc| doc.uid == target_uid).unwrap();
    assert_eq!(target.doc.pages.len(), 1);
    assert!(!target.doc.nodes().any(|node| node.name == "Removed target"));
    assert!(u.app.ui.last_error.as_deref().is_some_and(|error| error.contains("destination page was removed")));
}

#[test]
fn queued_canvas_import_is_cancelled_if_its_document_is_closed() {
    let dir = Temp::new();
    let path = dir.path("Closed target.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#).unwrap();

    let mut session = Session::new();
    let target_uid = session.ds().unwrap().uid;
    session.execute("file.new", &json!({"name": "Other tab"})).unwrap();
    let other_uid = session.ds().unwrap().uid;
    session.active = 0;
    let mut u = Ui::new(session);
    u.app.ui.home = false;
    u.frame(vec![]);
    let pos = u.app.canvas.rect.center();
    let file: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(path));
    u.frame_raw(egui::RawInput { events: vec![Event::PointerMoved(pos)], dropped_files: vec![file], ..Default::default() });
    u.frame(vec![]);
    let other_before = u.app.session.docs.iter().find(|doc| doc.uid == other_uid).unwrap().doc.clone();
    let target_index = u.app.session.docs.iter().position(|doc| doc.uid == target_uid).unwrap();
    u.app.session.docs.remove(target_index);
    u.app.session.active = 0;

    while !u.app.ui.pending_opens.is_empty() {
        u.frame(vec![]);
    }
    assert_eq!(u.app.session.docs.len(), 1);
    assert_eq!(u.app.session.ds().unwrap().uid, other_uid);
    assert_eq!(u.app.session.doc().unwrap(), other_before.as_ref());
    assert!(u.app.ui.last_error.as_deref().is_some_and(|error| error.contains("destination document was closed")));
}

#[test]
fn dropping_an_unsupported_file_on_the_home_screen_opens_nothing() {
    let dir = Temp::new();
    let path = dir.path("unsupported.txt");
    std::fs::write(&path, "not a design").unwrap();
    let mut u = first_run();
    u.app.ui.home = true;
    drop_file(&mut u, &path);
    assert!(u.app.session.docs.is_empty());
    assert!(u.app.ui.home);
    assert_eq!(u.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Drop a .figcraft, .fig, .json, .svg or image file."));
}
