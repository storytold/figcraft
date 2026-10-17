//! Synthetic silent-video placement and its existing document workflows.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use base64::Engine as _;
use figcraft_color::PaintKind;
use figcraft_doc::{NodeId, VideoData};
use serde_json::{Value, json};

use crate::{Session, params::id_of};

const SILENT: &[u8] = include_bytes!("../../media/tests/fixtures/synthetic-h264-silent.mp4");
const AUDIO: &[u8] = include_bytes!("../../media/tests/fixtures/synthetic-h264-aac.mp4");

fn data(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn place(s: &mut Session) -> Value {
    s.execute("video.place", &json!({"data": data(SILENT), "x": 10, "y": 20, "name": "Synthetic clip"})).unwrap()
}

fn video(s: &Session, id: NodeId) -> &figcraft_color::VideoPaint {
    match &s.doc().unwrap().node(id).unwrap().fills[0].kind {
        PaintKind::Video(v) => v,
        other => panic!("expected video, got {other:?}"),
    }
}

#[test]
fn video_placement_is_one_undo_step_and_uses_a_real_poster() {
    let mut s = Session::new();
    let before = s.doc().unwrap().clone();
    let r = place(&mut s);
    let id = id_of(&r["id"]).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(96.0), Some(64.0)));
    let v = video(&s, id);
    assert_eq!(s.doc().unwrap().videos[&v.video].bytes.as_slice(), SILENT);
    let pix = figcraft_render::render_node(s.doc().unwrap(), id, 1.0).unwrap();
    assert!(pix.data().chunks_exact(4).any(|p| p[3] > 0 && p[0] != p[1]));
    let svg = figcraft_svg::export_svg(s.doc().unwrap(), &[id], &Default::default()).unwrap();
    assert!(svg.contains("data:image/png;base64,"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node_count(), before.node_count());
    assert!(s.doc().unwrap().videos.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().videos[&video(&s, id).video].bytes.as_slice(), SILENT);
}

#[test]
fn video_rejection_leaves_assets_selection_and_history_unchanged() {
    let mut s = Session::new();
    place(&mut s);
    let before = s.ds().unwrap().doc.clone();
    let selection = s.selection().to_vec();
    let undo = s.ds().unwrap().history.undo.len();
    for params in [
        json!({"data": data(AUDIO)}),
        json!({"data": data(b"not an MP4")}),
        json!({"data": data(SILENT), "parent": 999999999}),
        json!({"data": data(SILENT), "width": -1}),
    ] {
        assert!(s.execute("video.place", &params).is_err());
        assert!(Arc::ptr_eq(&before, &s.ds().unwrap().doc));
        assert_eq!(s.selection(), selection);
        assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    }
}

#[test]
fn video_store_limit_rejects_before_adding_assets() {
    let mut s = Session::new();
    let bytes = Arc::new(vec![0; figcraft_doc::MAX_VIDEO_BYTES]);
    let page = s.page().unwrap();
    for key in ["first", "second"] {
        let doc = s.doc_mut().unwrap();
        doc.videos.insert(key.into(), VideoData { bytes: bytes.clone() });
        let node = doc.create(page, figcraft_doc::NodeKind::Rectangle, key, 0.0, 0.0, 10.0, 10.0).unwrap();
        doc.node_mut(node).unwrap().fills = vec![figcraft_color::Paint::video(key, "poster", figcraft_color::ImageScaleMode::Fill)];
    }
    let before = s.ds().unwrap().doc.clone();
    assert!(s.execute("video.place", &json!({"data": data(SILENT)})).unwrap_err().to_string().contains("32 MiB"));
    assert!(Arc::ptr_eq(&before, &s.ds().unwrap().doc));
}

#[test]
fn videos_nothing_paints_any_more_free_their_store_space_when_placing() {
    let mut s = Session::new();
    let bytes = Arc::new(vec![0; figcraft_doc::MAX_VIDEO_BYTES]);
    for key in ["first", "second"] {
        s.doc_mut().unwrap().videos.insert(key.into(), VideoData { bytes: bytes.clone() });
    }
    let r = place(&mut s);
    let doc = s.doc().unwrap();
    assert_eq!(doc.videos.len(), 1, "only the placed video is stored");
    assert!(doc.videos.contains_key(&video(&s, id_of(&r["id"]).unwrap()).video));
    // Undo restores the document as it was, unused entries included.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().videos.len(), 2);
}

#[test]
fn placing_deleting_saving_and_reopening_leaves_no_video_behind() {
    let mut s = Session::new();
    let id = id_of(&place(&mut s)["id"]).unwrap();
    s.execute("edit.delete", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(s.doc().unwrap().videos.len(), 1, "the live document keeps it for undo");
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (reopened, issues) = figcraft_format::load_bytes(&bytes).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(reopened.videos.is_empty());
    let mut s = Session::with_document(reopened);
    place(&mut s);
    assert_eq!(s.doc().unwrap().videos.len(), 1);
}

#[test]
fn video_copy_to_another_document_retains_bytes_and_poster() {
    let mut s = Session::new();
    let r = place(&mut s);
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let v = video(&s, s.selection()[0]);
    assert_eq!(v.video, r["video"].as_str().unwrap());
    assert!(s.doc().unwrap().images.contains_key(&v.poster.image));
    assert_eq!(s.doc().unwrap().videos[&v.video].bytes.as_slice(), SILENT);
}

#[test]
fn video_native_save_and_append_preserve_playable_source() {
    let mut source = Session::new();
    let r = place(&mut source);
    let bytes = figcraft_format::save_bytes(source.doc().unwrap(), None).unwrap();
    let (back, issues) = figcraft_format::load_bytes(&bytes).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    let key = r["video"].as_str().unwrap();
    assert_eq!(back.videos[key].bytes.as_slice(), SILENT);
    assert!(figcraft_media::VideoSource::open(back.videos[key].bytes.clone()).unwrap().frame_at(0.5).is_ok());
    let mut dest = Session::new();
    dest.execute("file.import", &json!({"name": "clip.figcraft", "data": data(&bytes)})).unwrap();
    assert_eq!(dest.doc().unwrap().videos[key].bytes.as_slice(), SILENT);
    assert!(dest.doc().unwrap().images.contains_key(r["poster"].as_str().unwrap()));
}

#[test]
fn mp4_import_uses_video_placement_and_deduplicates_source_bytes() {
    let mut s = Session::new();
    for _ in 0..2 {
        let r = s.execute("file.import", &json!({"name": "clip.MP4", "data": data(SILENT)})).unwrap();
        assert!(matches!(s.doc().unwrap().node(id_of(&r["id"]).unwrap()).unwrap().fills[0].kind, PaintKind::Video(_)));
    }
    assert_eq!(s.doc().unwrap().videos.len(), 1);
    assert_eq!(s.doc().unwrap().images.len(), 1);
}

#[test]
fn video_library_snapshot_keeps_source_and_poster() {
    let mut s = Session::new();
    let r = place(&mut s);
    s.execute("component.create", &json!({"ids": [r["id"]]})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    let lib = &doc.published.as_ref().unwrap().doc;
    assert_eq!(lib.videos[r["video"].as_str().unwrap()].bytes.as_slice(), SILENT);
    assert!(lib.images.contains_key(r["poster"].as_str().unwrap()));
    let bytes = figcraft_format::save_bytes(doc, None).unwrap();
    let loaded = figcraft_format::read_library(std::io::Cursor::new(bytes)).unwrap().unwrap();
    assert_eq!(loaded.doc.videos[r["video"].as_str().unwrap()].bytes.as_slice(), SILENT);
}

#[test]
fn video_poster_adjustments_preserve_source_and_undo() {
    let mut s = Session::new();
    let r = place(&mut s);
    let id = id_of(&r["id"]).unwrap();
    let before = video(&s, id).clone();
    s.execute("image.adjust", &json!({"ids": [id], "filters": {"contrast": 0.3}})).unwrap();
    assert_eq!(video(&s, id).video, before.video);
    assert_eq!(video(&s, id).poster.image, before.poster.image);
    assert!((video(&s, id).poster.filters.contrast - 0.3).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(video(&s, id), &before);
}

#[test]
fn video_runtime_commands_request_ui_without_document_history() {
    let mut s = Session::new();
    let undo = s.ds().unwrap().history.undo.len();
    for cmd in ["proto.video.play", "proto.video.pause", "proto.video.restart"] {
        s.execute(cmd, &json!({})).unwrap();
        assert_eq!(s.ui_requests.last().unwrap().kind, cmd);
    }
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
}

#[test]
fn video_sharing_is_refused_before_the_transport_is_called() {
    let (mut s, wire) = crate::live_tests::session();
    place(&mut s);
    for cmd in ["collab.host", "collab.publish", "library.publishToServer"] {
        let error = s.execute(cmd, &json!({"server": "ws://localhost:9999", "key": "test"})).unwrap_err();
        assert!(error.to_string().contains("Video documents cannot"), "{cmd}: {error}");
    }
    let wire = wire.lock().unwrap();
    assert!(wire.hosted.is_empty());
    assert!(wire.published.is_empty());
}

#[test]
fn library_only_video_cannot_bypass_the_sharing_guard() {
    let (mut s, wire) = crate::live_tests::session();
    let r = place(&mut s);
    s.execute("component.create", &json!({"ids": [r["id"]]})).unwrap();
    s.execute("library.publish", &json!({})).unwrap();
    let doc = s.doc_mut().unwrap();
    let ids: Vec<_> = doc.nodes().map(|node| node.id).collect();
    for id in ids {
        let node = doc.node_mut(id).unwrap();
        node.fills.clear();
        node.strokes.clear();
    }
    doc.videos.clear();
    assert!(figcraft_doc::library::used_videos(doc).is_empty());
    for cmd in ["collab.host", "collab.publish", "library.publishToServer"] {
        let error = s.execute(cmd, &json!({"server": "ws://localhost:9999", "key": "test"})).unwrap_err();
        assert!(error.to_string().contains("Video documents cannot"), "{cmd}: {error}");
    }
    let wire = wire.lock().unwrap();
    assert!(wire.hosted.is_empty());
    assert!(wire.published.is_empty());
}

#[test]
fn video_placement_in_a_live_document_is_rolled_back() {
    let (mut s, wire) = crate::live_tests::session();
    s.execute("collab.host", &json!({})).unwrap();
    let uid = s.ds().unwrap().uid;
    assert!(!s.live_doc(uid), "the mock host has not received its welcome yet");
    let before = s.ds().unwrap().doc.clone();
    assert!(s.execute("video.place", &json!({"data": data(SILENT)})).unwrap_err().to_string().contains("live session"));
    assert!(Arc::ptr_eq(&before, &s.ds().unwrap().doc), "pending-host rejection must roll back the document");
    assert!(wire.lock().unwrap().msgs.is_empty(), "pending host must not send the video to the server");

    let (_, _, welcome) = crate::live_tests::serve(s.doc().unwrap());
    crate::live_tests::push(&wire, vec![welcome]);
    s.live_poll();
    assert!(s.live_doc(uid), "host must finish its welcome before document edits sync");
    let before = s.ds().unwrap().doc.clone();
    assert!(s.execute("video.place", &json!({"data": data(SILENT)})).unwrap_err().to_string().contains("live session"));
    assert!(Arc::ptr_eq(&before, &s.ds().unwrap().doc));
    assert!(wire.lock().unwrap().msgs.is_empty());
}

fn dangling_paint() -> figcraft_color::Paint {
    figcraft_color::Paint::video("e".repeat(64), "poster", figcraft_color::ImageScaleMode::Fill)
}

fn saved_twice(s: &Session) {
    let snapshot = crate::cmd::file::FileSnapshot::of(s.ds().unwrap());
    snapshot.encode().unwrap();
    snapshot.encode().unwrap();
}

#[test]
fn copied_properties_of_a_video_fill_paste_as_the_poster_in_another_file() {
    let mut s = Session::new();
    let id = id_of(&place(&mut s)["id"]).unwrap();
    s.execute("edit.copyProperties", &json!({"ids": [id.0]})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let page = s.page().unwrap();
    let target = s.doc_mut().unwrap().create(page, figcraft_doc::NodeKind::Rectangle, "Target", 0.0, 0.0, 10.0, 10.0).unwrap();
    s.execute("edit.pasteProperties", &json!({"ids": [target.0]})).unwrap();
    let doc = s.doc().unwrap();
    assert!(figcraft_doc::used_videos(doc).is_empty(), "no video paint without its bytes");
    assert!(matches!(doc.node(target).unwrap().fills[0].kind, PaintKind::Image(_)));
    saved_twice(&s);
}

#[test]
fn a_hand_written_video_paint_must_name_a_video_the_document_holds() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let id = s.doc_mut().unwrap().create(page, figcraft_doc::NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    let before = s.ds().unwrap().doc.clone();
    let fill = json!([{"type": "video", "video": "e".repeat(64), "poster": {"image": "poster", "scaleMode": "fill"}}]);
    let err = s.execute("node.set", &json!({"id": id.0, "props": {"fills": fill}})).unwrap_err().to_string();
    assert!(err.contains("video.place"), "{err}");
    assert!(figcraft_doc::used_videos(s.doc().unwrap()).is_empty());
    assert!(Arc::ptr_eq(&before, &s.ds().unwrap().doc) || before.node(id).unwrap().fills.is_empty());
}

#[test]
fn a_version_captured_with_a_missing_video_does_not_block_saving() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let id = s.doc_mut().unwrap().create(page, figcraft_doc::NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    // A paint without bytes, as an older build or a plugin could have left it.
    s.doc_mut().unwrap().node_mut(id).unwrap().fills = vec![dangling_paint()];
    s.execute("file.saveVersion", &json!({"name": "Before"})).unwrap();
    assert!(!figcraft_doc::used_videos(&s.ds().unwrap().versions[0].document().unwrap()).is_empty());
    saved_twice(&s);
}

#[test]
fn only_params_with_a_video_key_or_type_count_as_video_paints() {
    use super::names_video_paint;
    assert!(names_video_paint(&json!({"props": {"fills": [{"type": "video", "video": "h"}]}})));
    assert!(names_video_paint(&json!({"video": "h"})));
    // A name that mentions video and a large unrelated payload are not video paints.
    assert!(!names_video_paint(&json!({"name": "Intro video", "data": "dmlkZW8=".repeat(4)})));
}
