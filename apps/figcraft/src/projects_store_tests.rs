//! The project library's file: kept and read back, a backup of the last good one, a damaged or
//! repaired file kept aside (never lost), one that can't be read never written over, and a first
//! run that starts from the recent design files.

use std::path::{Path, PathBuf};

use figcraft_engine::projects::{Favorite, Projects};
use figcraft_ui_egui::ProjectsNotice;

use crate::projects_store::{BACKUP, FILE, read, seeded, write};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-projects-store-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn library() -> Projects {
    let mut p = Projects::default();
    let northwind = p.create_folder("NORTHWIND BUILDERS", None, 1).unwrap();
    let social = p.create_folder("Social Media", Some(&northwind), 2).unwrap();
    p.move_files(&["/w/post.figcraft".to_string()], Some(&social), 3).unwrap();
    p.set_favorite(&Favorite::Folder(northwind), true, 4).unwrap();
    p.unsaved = false;
    p
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn kept_and_read_back_whole() {
    let d = dir("roundtrip");
    assert!(read(&d).projects.is_none(), "none yet");
    let p = library();
    write(&d, &p).unwrap();
    let back = read(&d);
    assert_eq!(back.projects.as_ref(), Some(&p));
    assert!(back.notice.is_none() && back.blocked.is_none());
    // The second save keeps the first as the backup; no staging file is left behind.
    let mut q = p.clone();
    q.create_folder("LUMEN", None, 5).unwrap();
    write(&d, &q).unwrap();
    assert_eq!(names(&d), [FILE, BACKUP]);
    let (bak, _) = Projects::from_json(&std::fs::read_to_string(d.join(BACKUP)).unwrap()).unwrap();
    assert_eq!(bak, p);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_damaged_file_is_kept_aside_and_the_backup_comes_back() {
    let d = dir("damaged");
    let p = library();
    write(&d, &p).unwrap();
    write(&d, &p).unwrap();
    std::fs::write(d.join(FILE), b"{ \"folders\": [ truncated").unwrap();
    let back = read(&d);
    assert_eq!(back.projects.as_ref().map(|x| &x.folders), Some(&p.folders), "the last good copy");
    assert!(back.projects.as_ref().is_some_and(|x| x.unsaved), "kept again at once");
    let Some(ProjectsNotice::Restored { kept: Some(kept) }) = back.notice else { panic!("{:?}", back.notice) };
    assert_eq!(std::fs::read(&kept).unwrap(), b"{ \"folders\": [ truncated", "the damaged file is kept as it was");
    assert!(back.blocked.is_none());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn damaged_without_a_backup_starts_empty_and_keeps_the_file() {
    let d = dir("nobackup");
    std::fs::write(d.join(FILE), b"not json").unwrap();
    let back = read(&d);
    assert_eq!(back.projects, Some(Projects::default()));
    let Some(ProjectsNotice::Damaged { kept: Some(kept) }) = back.notice else { panic!("{:?}", back.notice) };
    assert_eq!(std::fs::read(kept).unwrap(), b"not json");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn broken_references_are_repaired_and_the_original_kept() {
    let d = dir("repaired");
    let text = r#"{"version":1,"next_id":3,"folders":[{"id":"f1","name":"A","parent":"f9"}],"files":[{"path":"/a.figcraft","folder":"f7"}],"favorites":[{"type":"folder","id":"f8"}]}"#;
    std::fs::write(d.join(FILE), text).unwrap();
    let back = read(&d);
    let p = back.projects.unwrap();
    assert_eq!(p.folder("f1").unwrap().parent, None);
    assert_eq!(p.file("/a.figcraft").unwrap().folder, None, "the file is kept, in no folder");
    assert!(p.favorites.is_empty());
    assert!(p.unsaved);
    let Some(ProjectsNotice::Repaired { fixes, kept: Some(kept) }) = back.notice else { panic!() };
    assert_eq!(fixes, 3);
    assert_eq!(std::fs::read_to_string(kept).unwrap(), text);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_file_that_cant_be_read_is_never_written_over() {
    let d = dir("unreadable");
    // A folder where the file should be: it can't be read as text.
    std::fs::create_dir_all(d.join(FILE)).unwrap();
    let back = read(&d);
    assert!(back.projects.is_none());
    assert!(back.blocked.is_some());
    assert!(matches!(back.notice, Some(ProjectsNotice::Unreadable { .. })));
    // The services refuse to save over it.
    let mut svc = crate::services::Desktop::load_from(Some(d.clone()), crate::recovery::Shared::default());
    assert!(figcraft_ui_egui::Services::save_projects(&mut svc, &library()).is_err());
    assert!(d.join(FILE).is_dir(), "left as it was");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_removed_file_comes_back_from_its_backup() {
    let d = dir("removed");
    write(&d, &library()).unwrap();
    write(&d, &library()).unwrap();
    std::fs::remove_file(d.join(FILE)).unwrap();
    let back = read(&d);
    assert_eq!(back.projects.map(|p| p.folders.len()), Some(2));
    assert!(matches!(back.notice, Some(ProjectsNotice::Restored { kept: None })));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_first_run_starts_from_the_recent_design_files() {
    let recents: Vec<String> = ["/w/new.figcraft", "/w/pic.png", "/w/old.fig", "/w/x.sketch"].iter().map(|s| s.to_string()).collect();
    let p = seeded(&recents);
    let paths: Vec<&str> = p.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["/w/x.sketch", "/w/old.fig", "/w/new.figcraft"], "oldest first, images left out");
    assert!(p.unsaved);
    assert!(!seeded(&[]).unsaved);
}

#[test]
fn the_services_keep_the_library_where_the_preferences_are() {
    let d = dir("services");
    let mut svc = crate::services::Desktop::load_from(Some(d.clone()), crate::recovery::Shared::default());
    assert!(svc.projects.is_none());
    figcraft_ui_egui::Services::save_projects(&mut svc, &library()).unwrap();
    let again = crate::services::Desktop::load_from(Some(d.clone()), crate::recovery::Shared::default());
    assert_eq!(again.projects, Some(library()));
    let _ = std::fs::remove_dir_all(d);
}
