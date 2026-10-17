use egui::{Key, Pos2, pos2};
use serde_json::{Value, json};

use crate::present_tests::{Ui, id, setup};

const FILE_TAB: Pos2 = pos2(100.0, 20.0);

fn click(u: &mut Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
}

fn tabs(u: &Ui) -> Value {
    u.app.inspect()["presentTabs"].clone()
}

fn presenting(u: &Ui) -> bool {
    u.app.inspect()["presenting"] == json!(true)
}

/// The middle of presentation tab `k` and of its close button, after `files` file tabs named
/// "Untitled".
fn present_tab(u: &Ui, files: usize, k: usize) -> (Pos2, Pos2) {
    let width = |chars: usize, lo: f32, hi: f32| (12.0 * 0.6 * chars as f32 + 64.0).clamp(lo, hi);
    let mut x = 52.0 + files as f32 * width("Untitled".len(), 120.0, 220.0);
    let names: Vec<String> = tabs(u).as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    for n in names.iter().take(k) {
        x += width(n.chars().count(), 140.0, 260.0);
    }
    let w = width(names[k].chars().count(), 140.0, 260.0);
    (pos2(x + w / 2.0, 20.0), pos2(x + w - 16.0, 20.0))
}

#[test]
fn the_tab_is_named_after_the_file_and_page_and_its_x_closes_it() {
    let (mut u, a, ..) = setup();
    u.run("proto.present", json!({"start": a}));
    assert!(presenting(&u));
    assert_eq!(tabs(&u), json!([{"name": "Untitled - Page 1", "active": true}]));
    click(&mut u, FILE_TAB);
    assert!(!presenting(&u));
    assert_eq!(tabs(&u)[0]["active"], false);
    let (tab, close) = present_tab(&u, 1, 0);
    click(&mut u, tab);
    assert!(presenting(&u));
    click(&mut u, close);
    assert_eq!((presenting(&u), tabs(&u)), (false, json!([])));
    assert!(!u.app.ui.home, "back to its file");
}

#[test]
fn play_again_shows_the_tab_as_it_was_and_another_page_opens_a_new_one() {
    let (mut u, a, b, _) = setup();
    u.run("proto.present", json!({"start": a}));
    u.key(pos2(720.0, 470.0), Key::ArrowRight);
    click(&mut u, FILE_TAB);
    u.run("proto.present", json!({"start": a}));
    assert_eq!(tabs(&u).as_array().unwrap().len(), 1, "same page: the same tab");
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(b), "it shows the prototype as it was");
    u.run("proto.present", json!({"start": b}));
    assert_eq!(tabs(&u).as_array().unwrap().len(), 1);
    assert_eq!(u.app.inspect()["presenter"]["history"], json!([]), "another start frame starts again");

    click(&mut u, FILE_TAB);
    let page = id(&u.run("page.add", json!({"name": "Second"})));
    u.run("page.select", json!({"id": page}));
    let c = id(&u.run("node.create", json!({"type": "frame", "name": "C", "x": 0, "y": 0, "width": 200, "height": 200})));
    u.run("proto.present", json!({"start": c}));
    assert_eq!(tabs(&u), json!([{"name": "Untitled - Page 1", "active": false}, {"name": "Untitled - Second", "active": true}]));
    let (first, _) = present_tab(&u, 1, 0);
    click(&mut u, first);
    assert_eq!(tabs(&u), json!([{"name": "Untitled - Page 1", "active": true}, {"name": "Untitled - Second", "active": false}]));
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(b));
}

#[test]
fn the_presentation_plays_on_when_its_file_closes() {
    let (mut u, a, b, _) = setup();
    let hot = id(&u.run("node.create", json!({"type": "rectangle", "parent": a, "x": 0, "y": 0, "width": 393, "height": 80})));
    u.run("proto.interaction.add", json!({"id": hot, "dest": b}));
    u.run("proto.present", json!({"start": a}));
    click(&mut u, FILE_TAB);
    u.run("file.close", json!({"index": 0, "force": true}));
    assert_eq!(tabs(&u), json!([{"name": "Untitled - Page 1", "active": false}]));
    let (tab, _) = present_tab(&u, 0, 0);
    click(&mut u, tab);
    assert!(presenting(&u));
    let spot = u.app.inspect()["presenter"]["hotspots"][0]["rect"].clone();
    let r: Vec<f32> = spot.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    click(&mut u, pos2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0));
    u.frames(60, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(b), "the prototype still plays");
    u.key(pos2(700.0, 500.0), Key::Escape);
    assert_eq!(tabs(&u), json!([]));
    assert!(u.app.ui.home, "no file to go back to: Home");
}

#[test]
fn a_tab_whose_file_closed_leaves_the_file_in_front_alone() {
    let (mut u, a, ..) = setup();
    u.run("proto.present", json!({"start": a}));
    click(&mut u, FILE_TAB);
    u.run("file.new", json!({}));
    u.run("file.close", json!({"index": 0, "force": true}));
    let (tab, _) = present_tab(&u, 1, 0);
    click(&mut u, tab);
    u.frames(5, Vec::new);
    assert!(presenting(&u));
    assert!(!crate::tabs::present_owner_active(&u.app));
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a), "it keeps playing its own copy");
    assert!(!u.app.session.ds().unwrap().is_dirty());
}

#[test]
fn a_saved_file_opened_again_gets_its_tab_back() {
    let (mut u, a, ..) = setup();
    let dir = std::env::temp_dir().join(format!("figcraft-present-tab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Proto.figcraft").to_string_lossy().to_string();
    u.run("file.saveAs", json!({"path": path}));
    u.run("proto.present", json!({"start": a}));
    click(&mut u, FILE_TAB);
    u.run("file.close", json!({"index": 0}));
    u.run("file.open", json!({"path": path}));
    u.run("proto.present", json!({"start": a}));
    assert_eq!(tabs(&u), json!([{"name": "Untitled - Page 1", "active": true}]));
    u.key(pos2(700.0, 500.0), Key::Escape);
    assert!(!u.app.ui.home, "back to its file");
    assert_eq!(tabs(&u), json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renaming_the_file_or_the_page_renames_the_tab() {
    let (mut u, a, ..) = setup();
    u.run("proto.present", json!({"start": a}));
    click(&mut u, FILE_TAB);
    u.run("file.rename", json!({"name": "Playground"}));
    u.run("page.rename", json!({"name": "Calendar"}));
    u.frame(vec![]);
    assert_eq!(tabs(&u), json!([{"name": "Playground - Calendar", "active": false}]));
    u.run("file.close", json!({"index": 0, "force": true}));
    u.frame(vec![]);
    assert_eq!(tabs(&u)[0]["name"], "Playground - Calendar", "the names stay when the file closes");
}
