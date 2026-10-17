//! Files with damaged or unused video members still open and save.

use std::io::{Read, Write};
use std::sync::Arc;

use figcraft_doc::color::{ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Document, NodeKind, used_videos};

use super::*;

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 0, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A document whose rectangle has a video fill.
fn video_doc() -> (Document, String) {
    let mut doc = Document::new("Video file");
    let page = doc.first_page().unwrap();
    let node = doc.create(page, NodeKind::Rectangle, "Clip", 0.0, 0.0, 20.0, 20.0).unwrap();
    let (poster, ..) = add_image(&mut doc, png()).unwrap();
    let video = add_video(&mut doc, b"synthetic video bytes".to_vec()).unwrap();
    doc.node_mut(node).unwrap().fills = vec![Paint::video(&video, &poster, ImageScaleMode::Fill)];
    (doc, video)
}

/// `archive` with every member passed through `edit` (`None` drops it).
fn rezip(archive: &[u8], edit: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(archive)).unwrap();
    let mut out = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut out);
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        if let Some(data) = edit(&name, data) {
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&data).unwrap();
        }
    }
    w.finish().unwrap();
    out.into_inner()
}

fn fill_is_poster(doc: &Document) -> bool {
    doc.nodes().any(|n| n.fills.iter().any(|p| matches!(p.kind, PaintKind::Image(_))))
}

#[test]
fn missing_or_damaged_video_members_degrade_to_the_poster_and_the_file_saves() {
    let (doc, _) = video_doc();
    let saved = save_bytes(&doc, None).unwrap();
    let missing = rezip(&saved, |name, data| (!name.starts_with("videos/")).then_some(data));
    let mismatched = rezip(&saved, |name, data| Some(if name.starts_with("videos/") { b"other bytes".to_vec() } else { data }));
    for damaged in [missing, mismatched] {
        let loaded = load_bytes_with_versions(&damaged).unwrap();
        assert!(loaded.issues.iter().any(|i| i.contains("lost their video")), "{:?}", loaded.issues);
        assert!(used_videos(&loaded.doc).is_empty());
        assert!(fill_is_poster(&loaded.doc));
        let again = save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap();
        let reopened = load_bytes_with_versions(&again).unwrap();
        assert!(fill_is_poster(&reopened.doc));
        assert!(reopened.issues.is_empty(), "{:?}", reopened.issues);
    }
}

#[test]
fn a_video_paint_in_bare_json_degrades_so_the_document_saves() {
    let (doc, video) = video_doc();
    let json = serde_json::to_string(&to_json(&doc).unwrap()).unwrap();
    let loaded = load_bytes_with_versions(json.as_bytes()).unwrap();
    assert!(loaded.issues.iter().any(|i| i.contains(&video)));
    assert!(used_videos(&loaded.doc).is_empty());
    assert!(save_bytes(&loaded.doc, None).is_ok());
}

#[test]
fn a_stored_version_that_is_not_valid_json_still_saves_verbatim() {
    let (doc, _) = video_doc();
    let version = Version::new("v1", "Before", "", 1, Arc::new(doc.clone()));
    let saved = save_bytes_with_versions(&doc, None, &[version]).unwrap();
    let broken = rezip(&saved, |name, data| Some(if name == "versions/v1.json" { b"{ not json".to_vec() } else { data }));
    let loaded = load_bytes_with_versions(&broken).unwrap();
    assert_eq!(loaded.versions.len(), 1);
    assert!(loaded.versions[0].document().is_err());
    let again = save_bytes_with_versions(&loaded.doc, None, &loaded.versions).unwrap();
    let reopened = load_bytes_with_versions(&again).unwrap();
    assert_eq!(reopened.versions.len(), 1);
}

#[test]
fn a_stored_version_missing_its_video_restores_with_the_poster() {
    let (doc, video) = video_doc();
    let version = Version::new("v1", "Before", "", 1, Arc::new(doc.clone()));
    let saved = save_bytes_with_versions(&doc, None, &[version]).unwrap();
    // The version's index entry no longer lists the video, though its snapshot still paints it.
    let edited = rezip(&saved, |name, data| {
        if name == "versions/index.json" {
            let text = String::from_utf8(data).unwrap().replace(&format!("\"{video}\""), "");
            return Some(text.replace("[,", "[").replace(",]", "]").into_bytes());
        }
        Some(data)
    });
    let loaded = load_bytes_with_versions(&edited).unwrap();
    let restored = loaded.versions[0].document().unwrap();
    assert!(used_videos(&restored).is_empty());
    assert!(fill_is_poster(&restored));
}

#[test]
fn videos_nothing_paints_are_not_written() {
    let (mut doc, video) = video_doc();
    for id in doc.ids().collect::<Vec<_>>() {
        if let Some(n) = doc.node_mut(id) {
            n.fills.clear();
        }
    }
    assert!(doc.videos.contains_key(&video));
    let saved = save_bytes(&doc, None).unwrap();
    let z = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    assert!(!z.file_names().any(|n| n.starts_with("videos/")));
    let loaded = load_bytes_with_versions(&saved).unwrap();
    assert!(loaded.doc.videos.is_empty());
    assert_eq!(to_json(&loaded.doc).unwrap()["version"], 1);
    // The live document keeps its bytes (an undo may still need them).
    assert!(doc.videos.contains_key(&video));
}

/// `doc` with its video bytes gone but the fill still pointing at them.
fn dangling(doc: &Document) -> Document {
    let mut d = doc.clone();
    d.videos.clear();
    d
}

#[test]
fn saving_never_fails_over_missing_video_bytes_in_the_document_versions_or_branches() {
    let (doc, _) = video_doc();
    let broken = dangling(&doc);
    assert!(!used_videos(&broken).is_empty());
    let version = Version::new("v1", "Captured", "", 1, Arc::new(broken.clone()));
    let branch = Branch {
        id: "b1".into(),
        name: "Branch".into(),
        time: 2,
        client: 0,
        state: BranchState::Active,
        base: Arc::new(broken.clone()),
        head: Arc::new(broken.clone()),
        versions: vec![version.clone()],
    };
    let first = save_bytes_with_history(&broken, None, std::slice::from_ref(&version), std::slice::from_ref(&branch)).unwrap();
    let loaded = load_bytes_with_versions(&first).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert!(fill_is_poster(&loaded.doc));
    assert!(fill_is_poster(&loaded.versions[0].document().unwrap()));
    assert!(fill_is_poster(&loaded.branches[0].head));
    assert!(fill_is_poster(&loaded.branches[0].versions[0].document().unwrap()));
    // And again, from what was read back.
    save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap();
    save_bytes_with_history(&broken, None, &[version], &[branch]).unwrap();
}

#[test]
fn a_damaged_video_member_keeps_the_versions_and_branches_that_use_it() {
    let (doc, video) = video_doc();
    let version = Version::new("v1", "Before", "", 1, Arc::new(doc.clone()));
    let branch = Branch {
        id: "b1".into(),
        name: "Branch".into(),
        time: 2,
        client: 0,
        state: BranchState::Active,
        base: Arc::new(doc.clone()),
        head: Arc::new(doc.clone()),
        versions: vec![],
    };
    let saved = save_bytes_with_history(&doc, None, &[version], &[branch]).unwrap();
    let damaged = rezip(&saved, |name, data| if name == format!("videos/{video}") { None } else { Some(data) });
    let loaded = load_bytes_with_versions(&damaged).unwrap();
    assert_eq!(loaded.versions.len(), 1, "{:?}", loaded.issues);
    assert_eq!(loaded.branches.len(), 1, "{:?}", loaded.issues);
    assert!(fill_is_poster(&loaded.doc));
    assert!(fill_is_poster(&loaded.versions[0].document().unwrap()));
    assert!(fill_is_poster(&loaded.branches[0].base));
    let again = save_bytes_with_history(&loaded.doc, None, &loaded.versions, &loaded.branches).unwrap();
    let reopened = load_bytes_with_versions(&again).unwrap();
    assert_eq!((reopened.versions.len(), reopened.branches.len()), (1, 1));
    assert!(fill_is_poster(&reopened.versions[0].document().unwrap()));
}
