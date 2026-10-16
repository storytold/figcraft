//! File ▸ Open recent (kept in the preferences, capped, missing files dropped), outlines for the
//! selection and mask outlines.

use serde_json::json;

use crate::Session;
use crate::cmd::file::MAX_RECENT;
use crate::params::id_of;

#[test]
fn open_recent_lists_opens_prunes_and_clears() {
    let dir = std::env::temp_dir().join(format!("figcraft-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.figcraft").to_string_lossy().to_string();
    let b = dir.join("b.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.save", &json!({"path": a})).unwrap();
    s.execute("file.saveAs", &json!({"path": b})).unwrap();
    let list = s.execute("file.openRecent", &json!({})).unwrap();
    assert_eq!(list["files"][0]["path"], b.as_str(), "newest first");
    assert_eq!(list["files"][1]["name"], "a.figcraft");
    assert_eq!(list["files"][1]["exists"], true);
    // Opening one moves it to the top (an open file is switched to).
    s.execute("file.openRecent", &json!({"index": 1})).unwrap();
    assert_eq!(s.prefs.recent_files, vec![a.clone(), b.clone()]);
    assert!(s.execute("file.openRecent", &json!({"index": 7})).is_err());
    // A file that's gone is dropped when it's picked.
    std::fs::remove_file(&b).unwrap();
    assert!(s.execute("file.openRecent", &json!({"path": b})).is_err());
    assert_eq!(s.prefs.recent_files, vec![a.clone()]);
    s.execute("file.openRecent", &json!({"clear": true})).unwrap();
    assert!(s.prefs.recent_files.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_recent_file_is_forgotten_and_nothing_opens() {
    let mut s = Session::empty();
    s.prefs.recent_files = vec!["/tmp/a.figcraft".into(), "/tmp/b.fig".into(), "/tmp/c.svg".into()];
    let list = s.execute("file.openRecent", &json!({"remove": "/tmp/b.fig"})).unwrap();
    assert_eq!(s.prefs.recent_files, vec!["/tmp/a.figcraft".to_string(), "/tmp/c.svg".to_string()]);
    assert_eq!(list["files"].as_array().map(Vec::len), Some(2), "the list as it is now");
    assert!(s.docs.is_empty(), "nothing opens");
    // A path that isn't in the list changes nothing.
    s.execute("file.openRecent", &json!({"remove": "/tmp/zzz.fig"})).unwrap();
    assert_eq!(s.prefs.recent_files.len(), 2);
}

#[test]
fn recent_files_are_capped() {
    let dir = std::env::temp_dir().join(format!("figcraft-recent-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    for i in 0..MAX_RECENT + 3 {
        let p = dir.join(format!("{i}.figcraft")).to_string_lossy().to_string();
        s.execute("file.saveAs", &json!({"path": p})).unwrap();
    }
    assert_eq!(s.prefs.recent_files.len(), MAX_RECENT);
    assert!(s.prefs.recent_files[0].ends_with(&format!("{}.figcraft", MAX_RECENT + 2)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn view_toggles_flip_their_flags() {
    let mut s = Session::new();
    assert_eq!(s.execute("view.outlinesSelection", &json!({})).unwrap()["on"], true);
    assert!(s.ds().unwrap().view.outlines_selection);
    assert_eq!(s.execute("view.maskOutlines", &json!({"on": true})).unwrap()["on"], true);
    assert_eq!(s.execute("view.maskOutlines", &json!({})).unwrap()["on"], false);
}

#[test]
fn outlined_layers_draw_as_outlines_and_the_rest_normally() {
    let mut s = Session::new();
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap()["id"]).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 40, "height": 40})).unwrap();
    let ds = s.ds().unwrap();
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 120, height: 60 };
    let render =
        |outlined| figcraft_render::render_view(&ds.doc, ds.page, &view, &figcraft_render::Options { outlined, ..Default::default() }).unwrap();
    let (plain, outlined) = (render(vec![]), render(vec![a]));
    let empty = plain.pixel(110, 50).unwrap();
    assert_ne!(plain.pixel(20, 20).unwrap(), empty, "a filled rectangle");
    assert_eq!(outlined.pixel(20, 20).unwrap(), empty, "only its outline");
    assert_eq!(outlined.pixel(80, 20).unwrap(), plain.pixel(80, 20).unwrap(), "the other layer draws normally");
}
