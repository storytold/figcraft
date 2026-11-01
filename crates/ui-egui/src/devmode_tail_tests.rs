//! Dev Mode in the UI, the tail: the compare view compares with a saved version picked at its
//! bottom, and falls back when that version goes away.

use figcraft_engine::Session;
use serde_json::json;

use crate::dev_compare::CompareView;
use crate::present_tests::{Ui, id};

#[test]
fn compare_view_compares_with_a_saved_version() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let r = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 300, "width": 120, "height": 40, "parent": f})).unwrap());
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    // Neither ready for dev nor any version: nothing to compare with, the view doesn't stay.
    u.app.ui.dev.compare = Some(CompareView::new(f, None));
    u.frames(2, Vec::new);
    assert!(u.app.ui.dev.compare.is_none());
    let v = u.run("file.saveVersion", json!({"name": "Kickoff"}));
    let v = v["id"].as_str().unwrap().to_string();
    u.run("node.set", json!({"id": r, "props": {"width": 160}}));
    u.run("dev.compare", json!({"id": f, "version": v, "show": true}));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(v));
    // Opened without one, a layer that isn't ready compares with the newest version.
    u.app.ui.dev.compare = Some(CompareView::new(f, None));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(v));
    // Ready for dev: the snapshot from when it was marked is there too, and a deleted version
    // falls back to it.
    u.run("node.set", json!({"id": f, "props": {"devStatus": "readyForDev"}}));
    u.run("node.set", json!({"id": r, "props": {"height": 48}}));
    u.run("version.delete", json!({"id": v}));
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["devCompare"]["version"], json!(null));
    assert!(u.app.ui.dev.compare.is_some());
}

#[test]
fn dev_mode_keys_refuse_design_edits_and_say_why() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let r = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    let at = egui::pos2(700.0, 450.0);
    let x = |u: &Ui| u.app.session.doc().unwrap().node(r).map(|n| n.transform.as_coeffs()[4]);
    for key in [egui::Key::Delete, egui::Key::ArrowRight, egui::Key::Num5, egui::Key::R] {
        u.key(at, key);
    }
    assert!(u.app.session.doc().unwrap().contains(r), "not deleted");
    assert_eq!(x(&u), Some(20.0), "not nudged");
    assert_eq!(u.app.session.doc().unwrap().node(r).map(|n| n.opacity), Some(1.0), "opacity kept");
    assert_eq!(u.app.session.tool.tool, figcraft_engine::tools::ToolId::Move, "no drawing tool");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    // In the design the same keys edit.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    u.key(at, egui::Key::ArrowRight);
    assert_eq!(x(&u), Some(21.0));
}

#[test]
fn read_only_plugins_run_from_the_inspect_tab_show_their_panel() {
    use base64::Engine as _;
    let wasm = base64::engine::general_purpose::STANDARD.encode(crate::plugin_panel_tests::panel_module());
    let manifest = json!({"id": "org.test.inspect", "name": "Inspect panel", "main": "p.wasm", "api": 2, "permissions": {"access": "read"}});
    let mut s = Session::new();
    s.execute("plugin.import", &json!({"manifest": manifest, "wasm": wasm})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    // What the Inspect tab's Plugins row runs.
    crate::plugins_ui::start(&mut u.app, "org.test.inspect", "run");
    u.frame(vec![]);
    assert_eq!(u.app.session.plugins.panels.len(), 1, "its panel shows");
    // Its callbacks only read: the button that would draw is refused and the design stays.
    let layers = |u: &Ui| u.app.session.doc().unwrap().ids().count();
    let before = layers(&u);
    assert!(u.app.session.execute("plugin.event", &json!({"id": "org.test.inspect", "control": "go"})).is_err());
    assert_eq!(layers(&u), before);
}

/// A clipboard holding only text.
struct TextClip(String);

impl crate::Services for TextClip {
    fn clipboard_get_text(&mut self) -> Option<String> {
        Some(self.0.clone())
    }
}

#[test]
fn dev_mode_pastes_only_a_comments_link() {
    let mut s = Session::new();
    let c = s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "Check the spacing"})).unwrap()["id"].as_u64().unwrap();
    let layers = |app: &crate::FigApp| app.session.doc().unwrap().node(app.session.page().unwrap()).unwrap().children.len();
    let mut app = crate::FigApp::new(s, Box::new(TextClip("Pasted words".into())));
    app.ui.right_tab = crate::RightTab::Inspect;
    crate::menus::paste(&mut app, None, None);
    assert_eq!(layers(&app), 0, "text doesn't paste into the design in Dev Mode");
    app.services = Box::new(TextClip(format!("figcraft://comment/{c}")));
    crate::menus::paste(&mut app, None, None);
    assert_eq!(app.ui.comments.open, Some(c), "the link opens its thread");
}

#[test]
fn dev_mode_layers_panel_selects_and_expands_but_edits_nothing() {
    use egui::{Event, Pos2};
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "Login"})).unwrap());
    let a = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 90, "width": 120, "height": 40, "parent": f})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.app.ui.expanded.insert(f);
    u.frame(vec![]);
    let order = |u: &crate::layers_drag_tests::Ui| u.app.session.doc().unwrap().node(f).unwrap().children.clone();
    let before = order(&u);
    let (ra, rb) = (u.row_at(a), u.row_at(b));
    let click = |u: &mut crate::layers_drag_tests::Ui, p: Pos2| {
        u.button(p, true);
        u.button(p, false);
    };
    let rest = |u: &mut crate::layers_drag_tests::Ui| {
        for _ in 0..30 {
            u.frame(vec![]);
        }
    };
    // A click selects.
    click(&mut u, ra);
    assert_eq!(u.app.session.selection(), [a]);
    // A double click doesn't rename, and says why.
    click(&mut u, ra);
    assert!(u.app.ui.rename.is_none(), "no rename");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    rest(&mut u);
    // The eye and padlock spots don't hide or lock (the row is selected instead).
    let (eye, padlock) = (272.0, 250.0);
    for x in [eye, padlock] {
        u.frame(vec![Event::PointerMoved(Pos2::new(x, rb.y))]);
        click(&mut u, Pos2::new(x, rb.y));
        rest(&mut u);
    }
    let doc = u.app.session.doc().unwrap();
    assert!(doc.node(b).unwrap().visible && !doc.node(b).unwrap().locked, "not hidden or locked");
    assert_eq!(u.app.session.selection(), [b]);
    // Dragging a row doesn't reorder, and says why.
    u.app.ui.toast = None;
    u.drag_to(ra, rb + egui::vec2(0.0, 6.0));
    u.button(rb + egui::vec2(0.0, 6.0), false);
    rest(&mut u);
    assert_eq!(order(&u), before, "not reordered");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")), "the drag says why");
    // Back in the design, the same double click renames.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    click(&mut u, ra);
    click(&mut u, ra);
    assert!(u.app.ui.rename.is_some_and(|r| r.id == a));
    // A drag started in the design and dropped after switching to Dev Mode doesn't reorder either.
    u.app.ui.rename = None;
    rest(&mut u);
    u.app.ui.toast = None;
    let rf = u.row_at(f);
    u.drag_to(ra, rf);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    u.button(rf, false);
    rest(&mut u);
    assert_eq!(order(&u), before, "not reordered after the switch");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")), "the drop says why");
    // So does a sweep down the eye column.
    u.app.ui.right_tab = crate::RightTab::Design;
    rest(&mut u);
    u.app.ui.toast = None;
    let (ea, eb) = (Pos2::new(eye, u.row_at(a).y), Pos2::new(eye, u.row_at(b).y));
    u.drag_to(ea, eb);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    u.button(eb, false);
    rest(&mut u);
    let doc = u.app.session.doc().unwrap();
    assert!(doc.node(a).unwrap().visible && doc.node(b).unwrap().visible, "not hidden after the switch");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")), "the sweep says why");
}

#[test]
fn dev_mode_pages_list_opens_pages_but_edits_none() {
    use egui::{Event, Pos2};
    let mut s = Session::new();
    let home = s.page().unwrap();
    s.execute("page.rename", &json!({"id": home, "name": "Home"})).unwrap();
    let specs = id(&s.execute("page.add", &json!({"name": "Specs"})).unwrap());
    s.execute("page.select", &json!({"id": home})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let shapes = |u: &mut crate::layers_drag_tests::Ui| crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    let s = shapes(&mut u);
    let (rh, rs) = (crate::live_tests::text_center(&s, "Home"), crate::live_tests::text_center(&s, "Specs"));
    // The add button, found by its name in the AccessKit tree.
    u.ctx.enable_accesskit();
    let nodes = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    let b = nodes.iter().find(|(_, n)| n.label() == Some("Add new page")).and_then(|(_, n)| n.bounds()).unwrap();
    let add = Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    u.ctx.disable_accesskit();
    // The pointer arrives first: a row that also takes drags counts a press there only then.
    let click = |u: &mut crate::layers_drag_tests::Ui, p: Pos2| {
        let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        u.frame(vec![Event::PointerMoved(p)]);
        u.frame(vec![button(true)]);
        u.frame(vec![button(false)]);
    };
    let rest = |u: &mut crate::layers_drag_tests::Ui| {
        for _ in 0..30 {
            u.frame(vec![]);
        }
    };
    let pages = |u: &crate::layers_drag_tests::Ui| u.app.session.doc().unwrap().pages.clone();
    let said_why = |u: &mut crate::layers_drag_tests::Ui| u.app.ui.toast.take().is_some_and(|(m, _)| m.contains("Shift+D"));
    // A click opens the page.
    click(&mut u, rs);
    assert_eq!(u.app.session.page().unwrap(), specs);
    // A double click doesn't rename, and says why.
    click(&mut u, rs);
    assert!(u.app.ui.rename_page.is_none(), "no rename");
    assert!(said_why(&mut u));
    rest(&mut u);
    // Dragging a page doesn't reorder, and says why.
    u.drag_to(rs, rh - egui::vec2(0.0, 10.0));
    u.button(rh - egui::vec2(0.0, 10.0), false);
    rest(&mut u);
    assert_eq!(pages(&u), [home, specs], "not reordered");
    assert!(said_why(&mut u), "the drag says why");
    // The add button adds no page, and says why.
    click(&mut u, add);
    assert_eq!(pages(&u), [home, specs], "no page added");
    assert!(said_why(&mut u));
    rest(&mut u);
    // The row's Duplicate page and Delete page do nothing, and say why.
    for item in ["Duplicate page", "Delete page"] {
        u.frame(vec![
            Event::PointerMoved(rs),
            Event::PointerButton { pos: rs, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() },
        ]);
        u.frame(vec![Event::PointerButton { pos: rs, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() }]);
        let s = shapes(&mut u);
        click(&mut u, crate::live_tests::text_center(&s, item));
        assert_eq!(pages(&u), [home, specs], "{item} changes no page");
        assert!(said_why(&mut u), "{item} says why");
        rest(&mut u);
    }
    // In the design the double click renames and the add button adds a page.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    click(&mut u, rs);
    click(&mut u, rs);
    assert!(u.app.ui.rename_page.as_ref().is_some_and(|(p, _)| *p == specs));
    u.app.ui.rename_page = None;
    rest(&mut u);
    click(&mut u, add);
    assert_eq!(pages(&u).len(), 3, "a page is added");
}

#[test]
fn measurements_kept_on_the_canvas_draw_in_dev_mode_and_follow_their_layers() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 400})).unwrap());
    let a = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 120, "height": 40, "parent": f})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 90, "width": 120, "height": 40, "parent": f})).unwrap());
    let mut u = Ui::new(s);
    u.run("devMode.addMeasurement", json!({"from": a, "to": b}));
    let labels = |u: &Ui| crate::dev_mode::saved_lines(&u.app).filter_map(|g| g.label.clone()).collect::<Vec<_>>();
    u.frame(vec![]);
    assert!(labels(&u).is_empty(), "the design doesn't show them");
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    assert_eq!(labels(&u), ["20"]);
    u.run("node.set", json!({"id": b, "props": {"y": 110}}));
    u.frame(vec![]);
    assert_eq!(labels(&u), ["40"], "it follows the layer");
    // Hidden layers' measurements aren't drawn.
    u.run("node.set", json!({"id": b, "props": {"visible": false}}));
    u.frame(vec![]);
    assert!(labels(&u).is_empty());
}

/// A file dropped with its contents and no file on disk.
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

/// Hover `names` over the canvas (one frame: what it shows), then drop them there with `bytes`.
fn drop_on_canvas(u: &mut crate::layers_drag_tests::Ui, names: &[&str], bytes: &[u8]) -> Vec<egui::Shape> {
    let at = u.app.canvas.rect.center();
    let path = |n: &str| std::path::Path::new("figcraft-dev-mode-drop").join(n);
    let hovered = names.iter().map(|n| egui::HoveredFile { path: Some(path(n)), mime: String::new() }).collect();
    let out = u.frame_raw(egui::RawInput { events: vec![egui::Event::PointerMoved(at)], hovered_files: hovered, ..Default::default() });
    let files = names.iter().map(|n| std::sync::Arc::new(DroppedBytes(path(n), bytes.to_vec())) as egui::DroppedFileHandle).collect();
    u.frame_raw(egui::RawInput { events: vec![egui::Event::PointerMoved(at)], dropped_files: files, ..Default::default() });
    crate::live_tests::flat(&out.shapes)
}

#[test]
fn dev_mode_imports_no_dropped_file_into_the_design_and_says_why() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="red"/></svg>"#;
    let refusal = "Dev Mode only inspects: switch to Design (Shift+D) to edit";
    let mut u = crate::layers_drag_tests::Ui::new(Session::new());
    let layers = |u: &crate::layers_drag_tests::Ui| u.app.session.doc().unwrap().node(u.app.session.page().unwrap()).unwrap().children.len();
    u.app.ui.right_tab = crate::RightTab::Inspect;
    u.frame(vec![]);
    let s = drop_on_canvas(&mut u, &["Logo.svg", "Mark.svg"], svg);
    assert!(crate::live_tests::has_text(&s, refusal), "dragging images over the canvas says why they won't import");
    assert!(u.app.ui.pending_opens.is_empty(), "nothing waits to import");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m == refusal));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    assert_eq!(layers(&u), 0, "the design is unchanged");
    // A dropped document still opens as a file of its own.
    let s = drop_on_canvas(&mut u, &["Other.figcraft"], b"");
    assert!(crate::live_tests::has_text(&s, "Drop to open"));
    assert_eq!(u.app.ui.pending_opens.iter().map(|p| p.command).collect::<Vec<_>>(), ["file.open"]);
    u.app.ui.pending_opens.clear();
    // In the design the same drop imports.
    u.app.ui.right_tab = crate::RightTab::Design;
    u.frame(vec![]);
    let s = drop_on_canvas(&mut u, &["Logo.svg"], svg);
    assert!(crate::live_tests::has_text(&s, "Drop to import"));
    assert_eq!(u.app.ui.pending_opens.iter().map(|p| p.command).collect::<Vec<_>>(), ["file.import"]);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    assert_eq!(layers(&u), 1, "the image is imported");
}

#[test]
fn dev_mode_assets_panel_places_and_applies_nothing() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Card"})).unwrap());
    s.execute("component.create", &json!({"ids": [f]})).unwrap();
    s.execute("style.create", &json!({"type": "paint", "name": "Brand", "value": [{"type": "solid", "color": "#0D99FF"}]})).unwrap();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.left_tab = crate::LeftTab::Assets;
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let instances = |u: &crate::layers_drag_tests::Ui| {
        u.app.session.doc().unwrap().nodes().filter(|n| matches!(n.kind, figcraft_doc::NodeKind::Instance { .. })).count()
    };
    let styled = |u: &crate::layers_drag_tests::Ui| u.app.session.doc().unwrap().node(r).is_some_and(|n| n.styles.fill.is_some());
    let click = |u: &mut crate::layers_drag_tests::Ui, text: &str| {
        let s = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
        let at = crate::live_tests::text_center(&s, text);
        u.button(at, true);
        u.button(at, false);
        u.frame(vec![]);
    };
    click(&mut u, "Card");
    click(&mut u, "Brand");
    let s = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    let (tile, canvas) = (crate::live_tests::text_center(&s, "Card"), u.app.canvas.rect.center());
    u.drag_to(tile, canvas);
    u.button(canvas, false);
    u.frame(vec![]);
    assert_eq!(instances(&u), 0, "a click on a component, or a drag onto the canvas, places no instance");
    assert!(!styled(&u), "a click on a style doesn't apply it");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    // In the design both edit.
    u.app.ui.right_tab = crate::RightTab::Design;
    click(&mut u, "Card");
    assert_eq!(instances(&u), 1);
    u.drag_to(tile, canvas);
    u.button(canvas, false);
    u.frame(vec![]);
    assert_eq!(instances(&u), 2, "the same drag places one");
    u.app.session.execute("select.set", &json!({"ids": [r]})).unwrap();
    click(&mut u, "Brand");
    assert!(styled(&u));
}

#[test]
fn dev_mode_assets_panel_accepts_no_library_updates() {
    let dir = std::env::temp_dir().join(format!("figcraft-dev-mode-updates-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Library.figcraft");
    let mut lib = Session::new();
    lib.execute("style.create", &json!({"type": "paint", "name": "Brand", "value": [{"type": "solid", "color": "#0000FF"}]})).unwrap();
    lib.execute("library.publish", &json!({})).unwrap();
    lib.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let mut s = Session::new();
    let added = s.execute("library.add", &json!({"path": path.to_string_lossy()})).unwrap();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("library.applyStyle", &json!({"library": added["id"], "style": "Brand"})).unwrap();
    lib.execute("style.update", &json!({"style": "Brand", "value": "#00FF00"})).unwrap();
    lib.execute("library.publish", &json!({})).unwrap();
    lib.execute("file.save", &json!({})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.left_tab = crate::LeftTab::Assets;
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let updates = |u: &mut crate::layers_drag_tests::Ui| u.app.session.execute("library.updates", &json!({})).unwrap()["count"].as_u64();
    let update = |u: &mut crate::layers_drag_tests::Ui| {
        let s = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
        let at = crate::live_tests::text_center(&s, "Update");
        u.button(at, true);
        u.button(at, false);
        u.frame(vec![]);
    };
    assert_eq!(updates(&mut u), Some(1));
    update(&mut u);
    assert_eq!(updates(&mut u), Some(1), "Update in Dev Mode pulls nothing in");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
    u.app.ui.right_tab = crate::RightTab::Design;
    update(&mut u);
    assert_eq!(updates(&mut u), Some(0), "in the design it does");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn dev_mode_publishes_no_library() {
    let mut s = Session::new();
    s.execute("style.create", &json!({"type": "paint", "name": "Brand", "value": [{"type": "solid", "color": "#0D99FF"}]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.app.ui.left_tab = crate::LeftTab::Assets;
    u.app.ui.libraries = true;
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let s = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    let at = crate::live_tests::text_center(&s, "Publish");
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
    assert!(u.app.session.doc().unwrap().published.is_none(), "Publish in Dev Mode publishes nothing");
    assert!(u.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Shift+D")));
}
