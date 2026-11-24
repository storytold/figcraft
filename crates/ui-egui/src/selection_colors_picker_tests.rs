//! A Selection colors row's Styles button opens the style picker the Fill section's does: a search
//! field, this file's styles and each enabled library's under its name, and colour variables under
//! their library and collection. A pick applies to the row's paints only.

use egui::accesskit::Role;
use egui::{Event, Pos2, Shape};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, library};
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center, texts};

/// A library file publishing the colour style "Ocean" and the colour variable "Sea" (collection
/// "Palette"), saved in `dir`; its path.
fn library_file(dir: &std::path::Path) -> String {
    let path = dir.join("kit.figcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("style.create", &json!({"type": "paint", "name": "Ocean", "value": "#00ffff"})).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Palette"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Palette", "name": "Sea", "type": "color", "value": "#008080"})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    s.execute("file.saveAs", &json!({"path": path})).unwrap();
    path
}

/// Two rectangles, red and blue, both selected, so Selection colors shows a row for each.
fn two_colors() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let mut ids = Vec::new();
    for color in ["#ff0000", "#0000ff"] {
        let v = s.execute("shape.rectangle", &json!({"width": 40, "height": 40, "props": {"fill": color}})).unwrap();
        ids.push(NodeId(v["id"].as_u64().unwrap()));
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let u = Ui::new(s);
    u.ctx.enable_accesskit();
    (u, ids[0], ids[1])
}

fn shapes(u: &mut Ui) -> Vec<Shape> {
    flat(&u.frame_output(vec![]).shapes)
}

fn shows(u: &mut Ui, text: &str) -> bool {
    texts(&shapes(u)).iter().any(|(t, _, _)| t == text)
}

fn click(u: &mut Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
}

/// The middle of each accessible control with `role` and `label`, top to bottom.
fn controls(u: &mut Ui, role: Role, label: &str) -> Vec<Pos2> {
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let mut found: Vec<Pos2> = nodes
        .iter()
        .map(|(_, n)| n)
        .filter(|n| n.role() == role && n.label() == Some(label))
        .filter_map(|n| n.bounds())
        .map(|b| Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        .collect();
    found.sort_by(|a, b| a.y.total_cmp(&b.y));
    found
}

/// Open the style picker from the Selection colors row showing `name` (its hex or linked name).
fn open_picker(u: &mut Ui, name: &str) {
    let row = text_center(&shapes(u), name);
    let button = controls(u, Role::Button, "Color styles").into_iter().min_by(|a, b| (a.y - row.y).abs().total_cmp(&(b.y - row.y).abs()));
    click(u, button.expect("the row's Styles button"));
    assert!(u.app.ui.style_picker.is_some(), "the style picker opens");
}

#[test]
fn the_rows_styles_button_opens_the_style_picker_with_search_and_library_names() {
    let dir = std::env::temp_dir().join(format!("figcraft-selection-colors-picker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = library_file(&dir);
    let (mut u, a, b) = two_colors();
    u.app.run("library.add", json!({"path": path}));
    u.app.run("style.create", json!({"type": "paint", "name": "Brand", "value": "#ff00ff"}));
    let kit = u.app.session.doc().unwrap().libraries.enabled[0].name.clone();
    open_picker(&mut u, "FF0000");
    for text in ["This file", "Brand", &kit, "Ocean", "Color variables", &format!("{kit} · Palette"), "Sea"] {
        assert!(shows(&mut u, text), "the picker shows {text:?}");
    }
    // Typing filters styles and variables by name.
    u.frame(vec![Event::Text("oce".into())]);
    assert!(shows(&mut u, "Ocean"));
    assert!(!shows(&mut u, "Brand") && !shows(&mut u, "Sea"), "filtered out");
    let ocean = text_center(&shapes(&mut u), "Ocean");
    click(&mut u, ocean);
    let doc = u.app.session.doc().unwrap();
    let style = doc.node(a).unwrap().styles.fill.clone().expect("the red row takes the library style");
    assert!(library::is_remote_style(doc, &style));
    assert_eq!(doc.node(a).unwrap().fills[0].solid_color().unwrap().to_hex(), "00FFFF");
    assert!(doc.node(b).unwrap().styles.fill.is_none(), "the other row is left alone");
    assert!(u.app.ui.style_picker.is_none(), "a pick closes the picker");
    assert!(u.app.ui.last_error.is_none());
    // A library's colour variable binds to the row's paints, copied in.
    open_picker(&mut u, "0000FF");
    let sea = text_center(&shapes(&mut u), "Sea");
    click(&mut u, sea);
    let doc = u.app.session.doc().unwrap();
    assert!(!doc.node(b).unwrap().bound.is_empty(), "the blue row takes the variable");
    assert_eq!(doc.node(b).unwrap().fills[0].solid_color().unwrap().to_hex(), "008080");
    assert!(doc.node(a).unwrap().bound.is_empty());
    assert!(u.app.ui.last_error.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_style_in_the_picker_links_the_row_and_edits_its_name() {
    let (mut u, a, b) = two_colors();
    open_picker(&mut u, "FF0000");
    let create = controls(&mut u, Role::Button, "Create style");
    click(&mut u, *create.first().expect("the picker's + button"));
    let doc = u.app.session.doc().unwrap();
    let made = doc.style("Color 1").expect("a style named Color 1").id.clone();
    assert_eq!(doc.node(a).unwrap().styles.fill.as_deref(), Some(made.as_str()));
    assert!(doc.node(b).unwrap().styles.fill.is_none());
    assert!(shows(&mut u, "Edit style"), "the picker edits the new style");
}

#[test]
fn a_row_whose_list_holds_other_paints_offers_no_create() {
    let (mut u, a, _) = two_colors();
    u.app.run("fill.add", json!({"id": a, "paint": "#00ff00"}));
    open_picker(&mut u, "FF0000");
    assert!(controls(&mut u, Role::Button, "Create style").is_empty());
}

#[test]
fn a_linked_rows_picker_switches_its_style() {
    let (mut u, a, _) = two_colors();
    u.app.run("style.create", json!({"type": "paint", "name": "Brand", "value": "#ff0000"}));
    u.app.run("style.create", json!({"type": "paint", "name": "Accent", "value": "#00ff00"}));
    u.app.run("style.apply", json!({"id": a, "style": "Brand", "target": "fill"}));
    open_picker(&mut u, "Brand");
    let accent = text_center(&shapes(&mut u), "Accent");
    click(&mut u, accent);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(a).unwrap().styles.fill.as_deref(), doc.style("Accent").map(|s| s.id.as_str()));
    assert!(u.app.ui.last_error.is_none());
}
