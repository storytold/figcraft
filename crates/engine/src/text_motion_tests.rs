//! Text editing keys per platform (Ctrl moves and deletes by word on Windows and Linux, where it
//! deleted the whole line), and recovering a crashed run's copy as an unsaved document.

use serde_json::json;

use crate::keymap::{Mods, Platform, text_motion};
use crate::{Session, params, textedit};

const CTRL: Mods = Mods { shift: false, alt: false, cmd: true, ctrl: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false, ctrl: false };

/// Press `key` with `m` as the app does on `platform`; the text afterwards.
fn key(s: &mut Session, platform: Platform, key: &str, m: Mods) -> String {
    let (word, line) = text_motion(platform, key, m);
    textedit::key(s, key, m.shift, word, line).unwrap();
    let id = s.ds().unwrap().text_edit.as_ref().unwrap().id;
    s.doc().unwrap().node(id).unwrap().kind.text().unwrap().characters.clone()
}

fn editing(text: &str) -> Session {
    let mut s = Session::new();
    let t = params::id_of(&s.execute("text.create", &json!({"text": text, "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    textedit::begin(&mut s, t, Some(text.chars().count()), false).unwrap();
    s
}

fn caret(s: &Session) -> usize {
    s.ds().unwrap().text_edit.as_ref().unwrap().caret
}

#[test]
fn ctrl_moves_and_deletes_by_word_on_windows_and_linux() {
    let mut s = editing("Hello brave world");
    assert_eq!(key(&mut s, Platform::Other, "Backspace", CTRL), "Hello brave ", "Ctrl+Backspace deletes one word");
    key(&mut s, Platform::Other, "Left", CTRL);
    assert_eq!(caret(&s), 6, "Ctrl+Left goes to the word's start");
    // The control channel and MCP name arrows like egui.
    key(&mut s, Platform::Other, "ArrowLeft", CTRL);
    assert_eq!(caret(&s), 0);
    assert_eq!(key(&mut s, Platform::Other, "Delete", CTRL), " brave ", "Ctrl+Delete deletes the next word");
    key(&mut s, Platform::Other, "End", CTRL);
    assert_eq!(caret(&s), 7, "Ctrl+End: the end of the text");
    key(&mut s, Platform::Other, "Home", CTRL);
    assert_eq!(caret(&s), 0, "Ctrl+Home: its start");
}

#[test]
fn macos_keeps_alt_by_word_and_cmd_to_the_line_end() {
    assert_eq!(text_motion(Platform::Mac, "Left", ALT), (true, false));
    assert_eq!(text_motion(Platform::Mac, "Backspace", CTRL), (false, true));
    assert_eq!(text_motion(Platform::Other, "Left", ALT), (false, false), "Alt+arrows do nothing special on Windows");
    let mut s = editing("Hello brave world");
    assert_eq!(key(&mut s, Platform::Mac, "Backspace", ALT), "Hello brave ");
    assert_eq!(key(&mut s, Platform::Mac, "Backspace", CTRL), "", "Cmd+Backspace: to the line's start");
}

#[test]
fn a_recovered_copy_opens_unsaved_without_a_path() {
    let dir = std::env::temp_dir().join(format!("figcraft-recover-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("1.figcraft");
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let r = s.execute("file.recover", &json!({"path": path.to_string_lossy()})).unwrap();
    let ds = s.docs.get(r["index"].as_u64().unwrap() as usize).unwrap();
    assert!(ds.path.is_none(), "saving asks where: the copy isn't the file");
    assert!(ds.is_dirty(), "a closed window or tab asks before dropping it");
    assert_eq!(s.doc().unwrap().pages.len(), 1);
    assert!(s.execute("file.recover", &json!({"path": dir.join("missing.figcraft").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_recovery_references_are_only_enabled_by_the_trusted_desktop_entry_point() {
    let dir = std::env::temp_dir().join(format!("figcraft-recovery-permission-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("references.figcraft");
    let mut doc = figcraft_doc::Document::new("Synthetic recovery references");
    let asset = figcraft_assets::AssetRef::of(b"synthetic unavailable original");
    doc.images
        .insert("synthetic".into(), figcraft_doc::ImageData { asset: Some(asset), bytes: std::sync::Arc::new(Vec::new()), width: 1, height: 1 });
    let id = doc.create(doc.first_page().unwrap(), figcraft_doc::NodeKind::Rectangle, "Image", 0.0, 0.0, 10.0, 10.0).unwrap();
    doc.get_mut(id).unwrap().fills = vec![figcraft_doc::color::Paint::image("synthetic", figcraft_doc::color::ImageScaleMode::Fill)];
    std::fs::write(&path, figcraft_format::save_recovery_bytes_with_history(&doc, &[], &[]).unwrap()).unwrap();
    let mut s = Session::new();
    let params = json!({"path":path.to_string_lossy(),"allowRemoteReferences":true});
    assert!(s.execute("file.recover", &params).unwrap_err().to_string().contains("app-owned"));
    assert!(s.execute("file.recover", &json!({"path":path.to_string_lossy()})).is_err());
    crate::cmd::file::recover_app_owned(&mut s, &path.to_string_lossy()).unwrap();
    assert!(s.doc().unwrap().images.values().all(|image| image.asset.is_some()));
    assert!(s.execute("file.recover", &params).is_err(), "temporary permission must be reset after success");
    assert!(crate::cmd::file::recover_app_owned(&mut s, &dir.join("missing.figcraft").to_string_lossy()).is_err());
    assert!(s.execute("file.recover", &params).is_err(), "temporary permission must be reset after failure");
    std::fs::remove_dir_all(dir).unwrap();
}
