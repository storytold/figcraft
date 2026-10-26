//! The project library: folders, files in them, favourites, and reading back what was kept.

use serde_json::json;

use crate::Session;
use crate::projects::*;

fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A library kept and read back, as the app does across a restart.
fn restarted(p: &Projects) -> Projects {
    let (back, fixes) = Projects::from_json(&p.to_json()).unwrap();
    assert_eq!(fixes, 0, "a library the app kept needs no repair");
    back
}

#[test]
fn a_folder_is_created_named_and_kept() {
    let mut p = Projects::default();
    let id = p.create_folder("  NORTHWIND BUILDERS ", None, 100).unwrap();
    assert!(p.unsaved);
    let f = p.folder(&id).unwrap();
    assert_eq!((f.name.as_str(), f.parent.as_deref(), f.created), ("NORTHWIND BUILDERS", None, 100));
    let back = restarted(&p);
    assert_eq!(back.folder(&id).unwrap().name, "NORTHWIND BUILDERS");
    assert!(!back.unsaved);
}

#[test]
fn renaming_keeps_the_id_and_refuses_a_name_beside_it() {
    let mut p = Projects::default();
    let a = p.create_folder("Lumen", None, 0).unwrap();
    let b = p.create_folder("Studio Nine", None, 0).unwrap();
    p.rename_folder(&a, "LUMEN Branding").unwrap();
    assert_eq!(restarted(&p).folder(&a).unwrap().name, "LUMEN Branding");
    // Another folder beside it has the name (case aside): refused, nothing changes.
    assert!(p.rename_folder(&b, "lumen branding").is_err());
    assert!(p.rename_folder(&b, "   ").is_err());
    assert_eq!(p.folder(&b).unwrap().name, "Studio Nine");
    // Its own name in another case is fine.
    p.rename_folder(&a, "Lumen branding").unwrap();
    assert_eq!(p.folder(&a).unwrap().name, "Lumen branding");
}

#[test]
fn subfolders_keep_their_hierarchy() {
    let mut p = Projects::default();
    let northwind = p.create_folder("NORTHWIND BUILDERS", None, 0).unwrap();
    let social = p.create_folder("Social Media", Some(&northwind), 0).unwrap();
    let posts = p.create_folder("Posts", Some(&social), 0).unwrap();
    let back = restarted(&p);
    assert_eq!(back.children(None).iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), [northwind.as_str()]);
    assert_eq!(back.children(Some(&northwind))[0].id, social);
    assert_eq!(back.folder_path(&posts), "NORTHWIND BUILDERS / Social Media / Posts");
    assert_eq!(back.depth(&posts), 3);
    assert_eq!(back.descendants(&northwind), [northwind.clone(), social.clone(), posts.clone()]);
    assert!(p.create_folder("x", Some("f999"), 0).is_err(), "an unknown parent is refused");
}

#[test]
fn the_same_name_twice_gets_a_number_and_other_parents_may_repeat_it() {
    let mut p = Projects::default();
    let a = p.create_folder("Branding", None, 0).unwrap();
    let b = p.create_folder("branding", None, 0).unwrap();
    let c = p.create_folder("Branding", None, 0).unwrap();
    assert_eq!(p.folder(&b).unwrap().name, "branding 2");
    assert_eq!(p.folder(&c).unwrap().name, "Branding 3");
    let inside = p.create_folder("Branding", Some(&a), 0).unwrap();
    assert_eq!(p.folder(&inside).unwrap().name, "Branding", "another parent may have it");
    assert_ne!(a, b);
    assert_ne!(b, c);
}

#[test]
fn names_are_cleaned_and_capped() {
    let mut p = Projects::default();
    let id = p.create_folder("a\nb\tc", None, 0).unwrap();
    assert_eq!(p.folder(&id).unwrap().name, "a b c");
    let long = "é".repeat(MAX_NAME + 50);
    let id = p.create_folder(&long, None, 0).unwrap();
    assert_eq!(p.folder(&id).unwrap().name.chars().count(), MAX_NAME);
    let id = p.create_folder("", None, 0).unwrap();
    assert_eq!(p.folder(&id).unwrap().name, UNTITLED);
}

#[test]
fn files_move_into_folders_and_out_and_stay_known() {
    let mut p = Projects::default();
    let northwind = p.create_folder("Northwind", None, 0).unwrap();
    assert!(p.touch("/w/logo.figcraft", None, 5));
    assert!(!p.touch("/w/logo.figcraft", None, 6), "a known file is left as it is");
    assert_eq!(p.move_files(&paths(&["/w/logo.figcraft", "/w/post.figcraft"]), Some(&northwind), 7).unwrap(), 2);
    let back = restarted(&p);
    let inside: Vec<&str> = back.files_in(Some(&northwind)).map(|f| f.path.as_str()).collect();
    assert_eq!(inside, ["/w/logo.figcraft", "/w/post.figcraft"]);
    assert_eq!(back.file("/w/logo.figcraft").unwrap().added, 5);
    // Out of the folder: still in the library, in no folder.
    p.move_files(&paths(&["/w/logo.figcraft"]), None, 8).unwrap();
    assert_eq!(p.files_in(Some(&northwind)).count(), 1);
    assert_eq!(p.file("/w/logo.figcraft").unwrap().folder, None);
    assert!(p.move_files(&paths(&["/w/a.figcraft"]), Some("f404"), 0).is_err());
    assert!(p.move_files(&paths(&[""]), None, 0).is_err());
}

#[test]
fn favourite_folders_and_files_come_and_go_without_touching_them() {
    let mut p = Projects::default();
    let northwind = p.create_folder("Northwind", None, 0).unwrap();
    p.move_files(&paths(&["/w/a.figcraft"]), Some(&northwind), 0).unwrap();
    assert!(p.set_favorite(&Favorite::Folder(northwind.clone()), true, 0).unwrap());
    assert!(!p.set_favorite(&Favorite::Folder(northwind.clone()), true, 0).unwrap(), "once only");
    assert!(p.set_favorite(&Favorite::File("/w/b.figcraft".into()), true, 0).unwrap());
    assert!(p.file("/w/b.figcraft").is_some(), "a favourite file joins the library");
    let back = restarted(&p);
    assert_eq!(back.favorites, [Favorite::Folder(northwind.clone()), Favorite::File("/w/b.figcraft".into())]);
    // Taken out of Favorites: the folder, its file and the file stay.
    p.set_favorite(&Favorite::Folder(northwind.clone()), false, 0).unwrap();
    p.set_favorite(&Favorite::File("/w/b.figcraft".into()), false, 0).unwrap();
    assert!(p.favorites.is_empty());
    assert!(p.folder(&northwind).is_some());
    assert_eq!(p.files.len(), 2);
    assert!(p.set_favorite(&Favorite::Folder("f404".into()), true, 0).is_err());
}

#[test]
fn deleting_a_folder_keeps_its_files_in_its_parent() {
    let mut p = Projects::default();
    let northwind = p.create_folder("Northwind", None, 0).unwrap();
    let social = p.create_folder("Social", Some(&northwind), 0).unwrap();
    let posts = p.create_folder("Posts", Some(&social), 0).unwrap();
    p.move_files(&paths(&["/w/a.figcraft"]), Some(&social), 0).unwrap();
    p.move_files(&paths(&["/w/b.figcraft"]), Some(&posts), 0).unwrap();
    p.move_files(&paths(&["/w/c.figcraft"]), Some(&northwind), 0).unwrap();
    p.set_favorite(&Favorite::Folder(posts.clone()), true, 0).unwrap();
    let d = p.delete_folder(&social).unwrap();
    assert_eq!(d, Deleted { folders: 2, files: 2, to: Some(northwind.clone()) });
    assert!(p.folder(&social).is_none() && p.folder(&posts).is_none());
    assert_eq!(p.files.len(), 3, "no file is lost");
    assert_eq!(p.files_in(Some(&northwind)).count(), 3);
    assert!(p.favorites.is_empty(), "a deleted folder's favourite goes with it");
    // At the top level the files end up in no folder.
    let d = p.delete_folder(&northwind).unwrap();
    assert_eq!(d.to, None);
    assert_eq!(p.files_in(None).count(), 3);
    assert!(p.delete_folder("f404").is_err());
}

#[test]
fn a_folder_moves_but_never_into_itself() {
    let mut p = Projects::default();
    let a = p.create_folder("A", None, 0).unwrap();
    let b = p.create_folder("B", Some(&a), 0).unwrap();
    let c = p.create_folder("C", None, 0).unwrap();
    assert!(p.move_folder(&a, Some(&b)).is_err(), "into a folder inside it");
    assert!(p.move_folder(&a, Some(&a)).is_err());
    p.move_folder(&b, Some(&c)).unwrap();
    assert_eq!(p.folder(&b).unwrap().parent.as_deref(), Some(c.as_str()));
    // A folder with that name there already: a number.
    let b2 = p.create_folder("B", None, 0).unwrap();
    p.move_folder(&b2, Some(&c)).unwrap();
    assert_eq!(p.folder(&b2).unwrap().name, "B 2");
    p.move_folder(&b, None).unwrap();
    assert_eq!(p.folder(&b).unwrap().parent, None);
}

#[test]
fn removing_from_the_library_forgets_only_the_record() {
    let mut p = Projects::default();
    p.set_favorite(&Favorite::File("/w/a.figcraft".into()), true, 0).unwrap();
    p.touch("/w/b.figcraft", None, 0);
    assert_eq!(p.remove_files(&paths(&["/w/a.figcraft", "/w/zzz.figcraft"])), 1);
    assert!(p.favorites.is_empty());
    assert_eq!(p.files.len(), 1);
}

#[test]
fn a_moved_file_is_found_again_with_its_place() {
    let mut p = Projects::default();
    let northwind = p.create_folder("Northwind", None, 0).unwrap();
    p.move_files(&paths(&["/old/a.figcraft"]), Some(&northwind), 0).unwrap();
    p.set_favorite(&Favorite::File("/old/a.figcraft".into()), true, 0).unwrap();
    p.relink("/old/a.figcraft", "/new/a.figcraft").unwrap();
    let f = p.file("/new/a.figcraft").unwrap();
    assert_eq!(f.folder.as_deref(), Some(northwind.as_str()));
    assert_eq!(p.favorites, [Favorite::File("/new/a.figcraft".into())]);
    p.touch("/other.figcraft", None, 0);
    assert!(p.relink("/new/a.figcraft", "/other.figcraft").is_err(), "a path known already");
    assert!(p.relink("/nope", "/x").is_err());
}

#[test]
fn ids_are_never_reused() {
    let mut p = Projects::default();
    let a = p.create_folder("A", None, 0).unwrap();
    p.delete_folder(&a).unwrap();
    let b = p.create_folder("B", None, 0).unwrap();
    assert_ne!(a, b);
    let back = restarted(&p);
    let mut back = back;
    let c = back.create_folder("C", None, 0).unwrap();
    assert!(c != a && c != b);
}

#[test]
fn damaged_references_are_repaired_and_counted() {
    let text = r#"{
        "version": 1, "next_id": 1,
        "folders": [
            {"id": "f1", "name": "A", "parent": "f2"},
            {"id": "f2", "name": "B", "parent": "f1"},
            {"id": "f1", "name": "duplicate"},
            {"id": "f3", "name": "  ", "parent": "gone"},
            {"id": "f4", "name": "Self", "parent": "f4"}
        ],
        "files": [
            {"path": "/a.figcraft", "folder": "gone"},
            {"path": "/a.figcraft", "folder": "f1"},
            {"path": ""},
            {"path": "/b.figcraft", "folder": "f2"}
        ],
        "favorites": [
            {"type": "folder", "id": "gone"},
            {"type": "file", "id": "/c.figcraft"},
            {"type": "folder", "id": "f2"},
            {"type": "folder", "id": "f2"}
        ],
        "future_field": true
    }"#;
    let (p, fixes) = Projects::from_json(text).unwrap();
    assert!(fixes > 0);
    assert_eq!(p.folders.len(), 4);
    // The cycle is broken: every folder reaches the top.
    for f in &p.folders {
        assert!(p.ancestors(&f.id).first().is_some_and(|top| top.parent.is_none()), "{} reaches the top", f.id);
    }
    assert_eq!(p.folder("f3").unwrap().name, UNTITLED);
    assert_eq!(p.folder("f3").unwrap().parent, None);
    assert_eq!(p.folder("f4").unwrap().parent, None);
    assert_eq!(p.files.len(), 3, "duplicates and empty paths go; /c.figcraft joins as a favourite");
    assert_eq!(p.file("/a.figcraft").unwrap().folder, None, "its folder was gone");
    assert_eq!(p.file("/b.figcraft").unwrap().folder.as_deref(), Some("f2"));
    assert_eq!(p.favorites, [Favorite::File("/c.figcraft".into()), Favorite::Folder("f2".into())]);
    assert!(p.next_id > 4);
    // Repaired once, it stays as it is.
    assert_eq!(restarted(&p), p);
}

#[test]
fn text_that_isnt_a_library_is_refused() {
    assert!(Projects::from_json("").is_err());
    assert!(Projects::from_json("{ broken").is_err());
    assert!(Projects::from_json("[1, 2]").is_err());
    assert!(Projects::from_json(r#"{"folders": 5}"#).is_err());
    let (empty, _) = Projects::from_json("{}").unwrap();
    assert_eq!(empty, Projects::default());
}

#[test]
fn files_count_in_a_folder_and_below() {
    let mut p = Projects::default();
    let a = p.create_folder("A", None, 0).unwrap();
    let b = p.create_folder("B", Some(&a), 0).unwrap();
    p.move_files(&paths(&["/1", "/2"]), Some(&a), 0).unwrap();
    p.move_files(&paths(&["/3"]), Some(&b), 0).unwrap();
    assert_eq!(p.count_in(&a, false), 2);
    assert_eq!(p.count_in(&a, true), 3);
}

// ------------------------------------------------------------------------------------------------
// The `projects.*` commands.

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-projects-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(s: &mut Session, id: &str, p: serde_json::Value) -> serde_json::Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn a_saved_file_joins_the_library_and_stays_when_it_leaves_the_recent_files() {
    let dir = temp_dir("saved");
    let mut s = Session::new();
    let path = dir.join("Logo.figcraft").to_string_lossy().into_owned();
    run(&mut s, "file.saveAs", json!({"path": path}));
    assert!(s.projects.file(&path).is_some());
    assert!(s.projects.unsaved);
    // Twenty other files later it is no longer recent, and still in the library.
    for i in 0..crate::cmd::file::MAX_RECENT {
        s.projects.unsaved = false;
        let other = dir.join(format!("other{i}.figcraft")).to_string_lossy().into_owned();
        run(&mut s, "file.saveAs", json!({"path": other}));
    }
    assert!(!s.prefs.recent_files.contains(&path));
    assert!(s.projects.file(&path).is_some());
    // An exported image isn't a project.
    let png = dir.join("x.png").to_string_lossy().into_owned();
    assert!(s.projects.file(&png).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn folders_files_and_favourites_through_commands() {
    let mut s = Session::empty();
    let northwind = run(&mut s, "projects.createFolder", json!({"name": "NORTHWIND BUILDERS"}))["id"].as_str().unwrap().to_string();
    let social = run(&mut s, "projects.createFolder", json!({"name": "Social Media", "parent": northwind}))["id"].as_str().unwrap().to_string();
    let r = run(&mut s, "projects.moveFiles", json!({"paths": ["/w/post.figcraft"], "folder": social}));
    assert_eq!(r["moved"], 1);
    run(&mut s, "projects.favorite", json!({"folder": northwind}));
    run(&mut s, "projects.favorite", json!({"path": "/w/post.figcraft", "on": true}));
    run(&mut s, "projects.renameFolder", json!({"id": social, "name": "Social"}));
    let listed = run(&mut s, "projects.list", json!({"folder": social}));
    assert_eq!(listed["files"][0]["path"], "/w/post.figcraft");
    assert_eq!(listed["files"][0]["favorite"], true);
    assert_eq!(listed["files"][0]["exists"], false);
    let all = run(&mut s, "projects.list", json!({}));
    assert_eq!(all["folders"].as_array().unwrap().len(), 2);
    assert_eq!(all["favorites"][0], json!({"type": "folder", "id": northwind}));
    // Toggled off again.
    assert_eq!(run(&mut s, "projects.favorite", json!({"folder": northwind}))["favorite"], false);
    let d = run(&mut s, "projects.deleteFolder", json!({"id": northwind}));
    assert_eq!(d, json!({"folders": 2, "files": 1, "to": null}));
    assert_eq!(s.projects.files.len(), 1);
    assert_eq!(run(&mut s, "projects.removeFiles", json!({"path": "/w/post.figcraft"}))["removed"], 1);
    assert!(s.projects.files.is_empty() && s.projects.favorites.is_empty());
}

#[test]
fn bad_parameters_are_refused_and_change_nothing() {
    let mut s = Session::empty();
    let before = s.projects.clone();
    for (id, p) in [
        ("projects.createFolder", json!({"name": "x", "parent": "f9"})),
        ("projects.renameFolder", json!({"id": "f9", "name": "x"})),
        ("projects.renameFolder", json!({"id": "f9"})),
        ("projects.deleteFolder", json!({})),
        ("projects.moveFolder", json!({"id": "f9"})),
        ("projects.moveFiles", json!({"paths": []})),
        ("projects.moveFiles", json!({"paths": [""]})),
        ("projects.moveFiles", json!({"paths": "nope"})),
        ("projects.moveFiles", json!({"path": "/a", "folder": "f9"})),
        ("projects.favorite", json!({})),
        ("projects.favorite", json!({"folder": "f9"})),
        ("projects.relink", json!({"from": "/a", "to": "/definitely/not/here.figcraft"})),
        ("projects.importFolder", json!({"path": "/definitely/not/here"})),
        ("projects.list", json!({"folder": "f9"})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(s.projects, before);
}

#[test]
fn a_folder_on_disk_is_mirrored_without_touching_its_files() {
    let dir = temp_dir("import");
    let root = dir.join("CLIENTS");
    std::fs::create_dir_all(root.join("Northwind/Social")).unwrap();
    std::fs::create_dir_all(root.join("Empty")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    for f in ["Northwind/brand.figcraft", "Northwind/Social/post.fig", "loose.sketch", "Northwind/note.txt", ".hidden/x.figcraft", "Empty/readme.md"]
    {
        std::fs::write(root.join(f), b"bytes").unwrap();
    }
    let mut s = Session::empty();
    // A file the user already put in a folder keeps it.
    let kept = s.projects.create_folder("Mine", None, 0).unwrap();
    let brand = root.join("Northwind/brand.figcraft").to_string_lossy().into_owned();
    s.projects.move_files(std::slice::from_ref(&brand), Some(&kept), 0).unwrap();
    let r = run(&mut s, "projects.importFolder", json!({"path": root.to_string_lossy()}));
    assert_eq!((r["folders"].as_u64(), r["files"].as_u64(), r["capped"].as_bool()), (Some(3), Some(2), Some(false)));
    let top = r["folder"].as_str().unwrap();
    assert_eq!(s.projects.folder(top).unwrap().name, "CLIENTS");
    let names = |parent: &str| s.projects.children(Some(parent)).iter().map(|f| f.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(top), ["Northwind"], "no folder for one without design files, none for hidden ones");
    assert_eq!(s.projects.file(&brand).unwrap().folder.as_deref(), Some(kept.as_str()));
    let post = root.join("Northwind/Social/post.fig").to_string_lossy().into_owned();
    assert_eq!(s.projects.folder_path(s.projects.file(&post).unwrap().folder.as_deref().unwrap()), "CLIENTS / Northwind / Social");
    // The files on disk are as they were.
    assert_eq!(std::fs::read(root.join("Northwind/Social/post.fig")).unwrap(), b"bytes");
    assert!(root.join("Empty/readme.md").exists());
    // A folder without design files is refused.
    assert!(s.execute("projects.importFolder", &json!({"path": root.join("Empty").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_moved_file_is_located_again() {
    let dir = temp_dir("relink");
    let now = dir.join("new.figcraft");
    std::fs::write(&now, b"x").unwrap();
    let mut s = Session::empty();
    let f = s.projects.create_folder("Northwind", None, 0).unwrap();
    s.projects.move_files(&["/missing/old.figcraft".to_string()], Some(&f), 0).unwrap();
    run(&mut s, "projects.relink", json!({"from": "/missing/old.figcraft", "to": now.to_string_lossy()}));
    assert_eq!(s.projects.file(&now.to_string_lossy()).unwrap().folder.as_deref(), Some(f.as_str()));
    // A folder isn't a file.
    s.projects.touch("/b.figcraft", None, 0);
    assert!(s.execute("projects.relink", &json!({"from": "/b.figcraft", "to": dir.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
