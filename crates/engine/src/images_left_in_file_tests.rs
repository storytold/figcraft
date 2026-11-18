use serde_json::json;

use crate::Session;
use crate::cmd::file::FileSnapshot;

fn rect(s: &mut Session, x: f64) {
    s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap();
}

fn opened_with_images_left_in_file(name: &str) -> (std::path::PathBuf, String, Session) {
    let dir = std::env::temp_dir().join(format!("figcraft-images-left-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    rect(&mut s, 0.0);
    s.execute("file.save", &json!({"path": path})).unwrap();
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    let i = t.active;
    let tab = t.docs.remove(i).with_images_left_in_file(&path, 3);
    t.docs.insert(i, tab);
    (dir, path, t)
}

#[test]
fn a_file_with_images_left_out_is_not_saved_over() {
    let (dir, path, mut s) = opened_with_images_left_in_file("save");
    let before = std::fs::read(&path).unwrap();
    rect(&mut s, 20.0);
    let err = s.execute("file.save", &json!({})).unwrap_err().to_string();
    assert!(err.contains("3 of its images") && err.contains("another name"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(s.ds().unwrap().is_dirty());

    let copy = dir.join("copy.figcraft").to_string_lossy().to_string();
    s.execute("file.saveAs", &json!({"path": copy})).unwrap();
    assert!(!s.ds().unwrap().is_dirty());
    rect(&mut s, 40.0);
    s.execute("file.save", &json!({})).unwrap();
    assert!(s.execute("file.saveAs", &json!({"path": path})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_snapshot_encoded_in_the_background_does_not_save_over_it() {
    let (dir, path, mut s) = opened_with_images_left_in_file("snapshot");
    let before = std::fs::read(&path).unwrap();
    rect(&mut s, 20.0);
    let file = FileSnapshot::of(s.ds().unwrap());
    let bytes = file.encode().unwrap();
    assert!(file.save(&mut s, &bytes).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(s.ds().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_without_images_left_out_saves_as_before() {
    let (dir, path, mut s) = opened_with_images_left_in_file("none");
    let i = s.active;
    let tab = s.docs.remove(i).with_images_left_in_file(&path, 0);
    s.docs.insert(i, tab);
    rect(&mut s, 20.0);
    s.execute("file.save", &json!({})).unwrap();
    assert!(!s.ds().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hard_link_to_the_file_is_not_saved_over() {
    let (dir, path, mut s) = opened_with_images_left_in_file("hard-link");
    let link = dir.join("link.figcraft");
    let _ = std::fs::remove_file(&link);
    std::fs::hard_link(&path, &link).unwrap();
    let before = std::fs::read(&path).unwrap();
    rect(&mut s, 20.0);
    assert!(s.execute("file.saveAs", &json!({"path": link.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}
