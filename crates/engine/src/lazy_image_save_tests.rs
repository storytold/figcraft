//! End-to-end engine tests for lazy image loading, streaming in-place save, undo-history image
//! pinning across atomic file replacement, and automation-root confinement (#1313).

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use serde_json::json;

use crate::Session;

fn png(r: u8, g: u8, b: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([r, g, b, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[test]
fn in_place_save_pins_undo_only_lazy_images_before_replacing_backing_file() {
    let dir = std::env::temp_dir().join(format!("figcraft-engine-lazy-undo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("undo-pin.figcraft");
    let path_str = path.to_string_lossy().to_string();

    // Save a file with two images.
    let mut init_doc = figcraft_doc::Document::new("UndoPin");
    let bytes_a = png(10, 20, 30);
    let bytes_b = png(200, 100, 50);
    let (hash_a, _, _) = figcraft_format::add_image(&mut init_doc, bytes_a.clone()).unwrap();
    let (hash_b, _, _) = figcraft_format::add_image(&mut init_doc, bytes_b.clone()).unwrap();
    figcraft_format::save_file(&init_doc, &path, None).unwrap();

    // Open through `file.open`: images load lazily from disk.
    let mut s = Session::empty();
    s.execute("file.open", &json!({"path": path_str})).unwrap();
    assert!(s.doc().unwrap().images[&hash_a].is_lazy());
    assert!(s.doc().unwrap().images[&hash_b].is_lazy());

    // Push a snapshot containing both lazy images onto the undo stack, then drop `hash_b` from
    // the live document so the next in-place save writes a file without `hash_b`.
    let before = s.ds().unwrap().doc.clone();
    let page = s.ds().unwrap().page;
    s.ds_mut().unwrap().history.undo.push(crate::HistoryEntry { label: "remove image".into(), doc: before, selection: Vec::new(), page });
    Arc::make_mut(&mut s.ds_mut().unwrap().doc).images.remove(&hash_b);

    // Save in place over `path`: `hash_b` is dropped from disk, so it must be pinned in undo!
    s.execute("file.save", &json!({})).unwrap();

    // Disk file now holds only `hash_a`.
    let (on_disk, _) = figcraft_format::load_file(&path).unwrap();
    assert!(on_disk.images.contains_key(&hash_a));
    assert!(!on_disk.images.contains_key(&hash_b));

    // Undo restores `hash_b` with its exact original bytes intact!
    s.execute("edit.undo", &json!({})).unwrap();
    let restored = s.doc().unwrap().images.get(&hash_b).expect("undo must restore hash_b");
    assert!(!restored.is_lazy(), "undo-only image must have been pinned into memory before swap");
    assert_eq!(restored.bytes().as_deref().map(Vec::as_slice), Some(bytes_b.as_slice()));
    assert_eq!(s.doc().unwrap().images[&hash_a].bytes().as_deref().map(Vec::as_slice), Some(bytes_a.as_slice()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn streaming_open_and_save_respect_automation_roots() {
    let dir = std::env::temp_dir().join(format!("figcraft-engine-lazy-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let allowed = dir.join("allowed");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let mut doc = figcraft_doc::Document::new("Confined");
    let raw = png(5, 15, 25);
    let (hash, _, _) = figcraft_format::add_image(&mut doc, raw.clone()).unwrap();
    let inside_path = allowed.join("ok.figcraft");
    let outside_path = outside.join("nope.figcraft");
    figcraft_format::save_file(&doc, &inside_path, None).unwrap();
    figcraft_format::save_file(&doc, &outside_path, None).unwrap();

    let roots = Arc::new(crate::file_access::Roots::new(&[allowed.to_string_lossy().to_string()], &[allowed.to_string_lossy().to_string()]).unwrap());
    crate::file_access::confined(Some(&roots), || {
        // Reading outside root is refused.
        assert!(crate::file_access::load_native_file(&outside_path, crate::cmd::file::MAX_DOCUMENT_FILE).is_err());
        // Reading inside root succeeds lazily.
        let loaded = crate::file_access::load_native_file(&inside_path, crate::cmd::file::MAX_DOCUMENT_FILE).unwrap().unwrap();
        assert!(loaded.doc.images[&hash].is_lazy());
        assert_eq!(loaded.doc.images[&hash].bytes().as_deref().map(Vec::as_slice), Some(raw.as_slice()));
        // Writing outside root is refused.
        assert!(crate::file_access::save_native_file(&outside_path, &loaded.doc, None, &[], &[], &[]).is_err());
        // Writing inside root succeeds.
        let saved_copy = allowed.join("copy.figcraft");
        crate::file_access::save_native_file(&saved_copy, &loaded.doc, None, &[], &[], &[]).unwrap();
        assert!(saved_copy.exists());
    });

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_place_save_aborts_if_undo_only_lazy_image_cannot_be_pinned() {
    let dir = std::env::temp_dir().join(format!("figcraft-engine-lazy-pin-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pin-fail.figcraft");
    let path_str = path.to_string_lossy().to_string();

    let mut init_doc = figcraft_doc::Document::new("PinFail");
    let bytes_a = png(10, 20, 30);
    let bytes_b = png(200, 100, 50);
    let (hash_a, _, _) = figcraft_format::add_image(&mut init_doc, bytes_a).unwrap();
    let (hash_b, _, _) = figcraft_format::add_image(&mut init_doc, bytes_b).unwrap();
    figcraft_format::save_file(&init_doc, &path, None).unwrap();

    let mut s = Session::empty();
    s.execute("file.open", &json!({"path": path_str})).unwrap();

    let loader = Arc::clone(s.doc().unwrap().images[&hash_b].loader().expect("lazy loader"));
    let before = s.ds().unwrap().doc.clone();
    let page = s.ds().unwrap().page;
    s.ds_mut().unwrap().history.undo.push(crate::HistoryEntry { label: "remove image".into(), doc: before, selection: Vec::new(), page });
    Arc::make_mut(&mut s.ds_mut().unwrap().doc).images.remove(&hash_b);

    // Simulate backing file overwritten with corrupted bytes so `hash_b` can no longer be materialized.
    loader.release_backing_handle();
    let sentinel = b"not a zip archive".to_vec();
    std::fs::write(&path, &sentinel).unwrap();

    // Even with `overwrite: true` (bypassing external-modification check), save aborts cleanly before
    // replacing the file because undo image `hash_b` could not be read.
    let err = s.execute("file.save", &json!({"overwrite": true})).unwrap_err();
    assert!(err.to_string().contains(&hash_b), "unexpected error: {err}");
    assert_eq!(std::fs::read(&path).unwrap(), sentinel);
    assert!(s.doc().unwrap().images.contains_key(&hash_a));

    let _ = std::fs::remove_dir_all(&dir);
}
