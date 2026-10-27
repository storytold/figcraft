#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::disallowed_methods)]
//! Separate processes deliberately have different font databases. Never put a font in the file.
use figcraft_doc::{Document, NodeKind, TextAutoResize, TextData};
use figcraft_engine::Session;
use figcraft_text::fontdb::{Face, FontBytes};
use serde_json::json;

const FAMILY: &str = "FigCraft synthetic portable test face";

#[test]
fn portable_text_other_computer() {
    if let Some(dir) = std::env::var_os("FIGCRAFT_PORTABLE_TEXT_CHILD") {
        let dir = std::path::PathBuf::from(dir);
        assert!(!figcraft_text::has_family(FAMILY));
        let (doc, issues) = figcraft_format::load_bytes(&std::fs::read(dir.join("test.figcraft")).unwrap()).unwrap();
        assert!(issues.is_empty());
        let id = doc.nodes().find(|n| matches!(n.kind, NodeKind::Text(_))).unwrap().id;
        let n = doc.node(id).unwrap();
        let (l, _) = figcraft_text::layout_node(n).unwrap();
        assert!(!l.glyphs.is_empty());
        assert!(l.glyphs.iter().all(|g| g.face.is_none()));
        let changes: Vec<figcraft_collab::Change> =
            figcraft_collab::message::decode(&std::fs::read(dir.join("snapshot.bin")).unwrap(), usize::MAX).unwrap();
        let replica = figcraft_collab::Replica::from_snapshot(&changes).unwrap();
        assert_eq!(figcraft_render::render_node(replica.document(), id, 2.0).unwrap().data(), std::fs::read(dir.join("pixels.bin")).unwrap());
        let pixels = figcraft_render::render_node(&doc, id, 2.0).unwrap();
        assert_eq!(pixels.data(), std::fs::read(dir.join("pixels.bin")).unwrap());
        let svg = figcraft_svg::export_svg(&doc, &[id], &Default::default()).unwrap();
        assert_eq!(svg, std::fs::read_to_string(dir.join("text.svg")).unwrap());
        let live_svg = figcraft_svg::export_svg(&doc, &[id], &figcraft_svg::SvgOptions { outline_text: false, ..Default::default() }).unwrap();
        assert!(!live_svg.contains("<text"), "an absent font must not change the exported appearance");
        assert!(figcraft_pdf::export_nodes_pdf(&doc, &[id]).unwrap().starts_with(b"%PDF"));
        let mut linked = doc.clone();
        for url in [Some("https://example.org/current"), None] {
            if let NodeKind::Text(t) = &mut linked.get_mut(id).unwrap().kind {
                t.style.link = url.map(str::to_owned);
                let saved = std::sync::Arc::make_mut(t.appearance.as_mut().unwrap());
                for style in &mut saved.styles {
                    style.link = Some("https://example.org/stale".into());
                }
            }
            let pdf = figcraft_pdf::export_nodes_pdf(&linked, &[id]).unwrap();
            let pdf = String::from_utf8_lossy(&pdf);
            assert!(!pdf.contains("https://example.org/stale"));
            assert_eq!(pdf.contains("https://example.org/current"), url.is_some());
        }
        let mut session = Session::with_document(doc);
        let old = session.doc().unwrap().clone();
        assert!(figcraft_engine::textedit::begin(&mut session, id, None, false).is_err());
        let error = session.execute("node.set", &json!({"ids":[id],"props":{"characters":"Changed"}})).unwrap_err();
        assert!(error.to_string().contains("missing fonts"), "{error}");
        assert_eq!(*session.doc().unwrap(), old);
        let baseline = session.ds().unwrap().doc.clone();
        session.begin_interaction("Resize absent-font text").unwrap();
        let error = session
            .preview(|doc, _| {
                doc.get_mut(id)?.width = 180.0;
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains("missing fonts"), "{error}");
        assert!(std::sync::Arc::ptr_eq(&baseline, &session.ds().unwrap().doc));
        session.commit_interaction().unwrap();
        assert!(std::sync::Arc::ptr_eq(&baseline, &session.ds().unwrap().doc));
        session.begin_interaction("Edit absent-font text").unwrap();
        assert!(
            session
                .preview(|doc, _| {
                    if let NodeKind::Text(t) = &mut doc.get_mut(id)?.kind {
                        t.characters = "Gesture edit".into();
                    }
                    Ok(())
                })
                .is_err()
        );
        session.cancel_interaction().unwrap();
        assert!(std::sync::Arc::ptr_eq(&baseline, &session.ds().unwrap().doc));
        session.begin_interaction("Move absent-font text").unwrap();
        session
            .preview(|doc, _| {
                doc.get_mut(id)?.transform = figcraft_geom::Affine::translate((25.0, 0.0));
                Ok(())
            })
            .unwrap();
        let last_valid = session.ds().unwrap().doc.clone();
        assert!(
            session
                .preview(|doc, _| {
                    doc.get_mut(id)?.width = 180.0;
                    Ok(())
                })
                .is_err()
        );
        assert!(std::sync::Arc::ptr_eq(&last_valid, &session.ds().unwrap().doc));
        session.commit_interaction().unwrap();
        assert_eq!(session.doc().unwrap().node(id).unwrap().transform.as_coeffs()[4], 25.0);
        assert!(session.undo().unwrap());
        assert_eq!(*session.doc().unwrap(), old);
        session.execute("node.set", &json!({"ids":[id],"props":{"x":150}})).unwrap();
        assert_eq!(session.doc().unwrap().node(id).unwrap().transform.as_coeffs()[4], 150.0);
        assert_eq!(session.doc().unwrap().node(id).unwrap().width, n_width(&old, id));
        let baseline = session.ds().unwrap().doc.clone();
        let was_dirty = session.ds().unwrap().is_dirty();
        assert_eq!(session.execute("text.preserveAppearance", &json!({})).unwrap()["captured"], 0);
        assert!(std::sync::Arc::ptr_eq(&baseline, &session.ds().unwrap().doc), "a no-op must not create a document edit");
        assert_eq!(session.ds().unwrap().is_dirty(), was_dirty);
        let reopened = figcraft_format::load_bytes(&figcraft_format::save_bytes(session.doc().unwrap(), None).unwrap()).unwrap().0;
        assert!(figcraft_text::layout_node(reopened.node(id).unwrap()).unwrap().0.glyphs.iter().all(|g| g.face.is_none()));
        return;
    }
    figcraft_text::scan_system_fonts();
    let Some(font) = figcraft_text::find_face("Arial", "Regular") else {
        eprintln!("portable text: skipped separate-process check because no source face is available");
        return;
    };
    let face =
        Face::from_bytes(FontBytes::Shared(std::sync::Arc::new(font.bytes.data().to_vec())), font.index, Some(FAMILY), Some("Regular")).unwrap();
    let mut probe = TextData::new("Cache probe");
    probe.style.font_family = FAMILY.into();
    assert!(!figcraft_text::appearance::fonts_available(&probe));
    figcraft_text::db().write().unwrap().add(face);
    assert!(figcraft_text::appearance::fonts_available(&probe), "adding a face invalidates the negative cache");
    let mut doc = Document::new("Synthetic portable text");
    let mut t = TextData::new("AV office — Привет\nSecond line");
    t.style.font_family = FAMILY.into();
    t.style.font_size = 32.0;
    t.auto_resize = TextAutoResize::None;
    let id = doc.create(doc.first_page().unwrap(), NodeKind::Text(t), "Portable", 0.0, 0.0, 360.0, 120.0).unwrap();
    assert_eq!(figcraft_text::appearance::capture(&mut doc, None), 1);
    assert_eq!(figcraft_text::appearance::capture(&mut doc, None), 0);
    // Old imported files can link a text style on the layer before per-character links exist.
    // Opening must settle those links before fingerprinting the portable geometry.
    let style = doc.node(id).unwrap().kind.text().unwrap().style.clone();
    doc.styles.push(figcraft_doc::Style {
        id: "synthetic-type".into(),
        name: "Synthetic type".into(),
        description: String::new(),
        kind: figcraft_doc::StyleKind::Text { style },
        bound: Default::default(),
    });
    doc.get_mut(id).unwrap().styles.text = Some("synthetic-type".into());
    let mut doc = (*figcraft_engine::DocState::new(doc, None).doc).clone();
    let node = doc.node(id).unwrap();
    assert!(figcraft_text::appearance::has_saved(node.kind.text().unwrap(), Some(node.width), Some(node.height)));
    assert_eq!(figcraft_text::appearance::capture(&mut doc, None), 0);
    let dir = std::env::temp_dir().join(format!("figcraft-portable-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let replica = figcraft_collab::Replica::new(doc.clone());
    std::fs::write(dir.join("snapshot.bin"), figcraft_collab::message::encode(&replica.snapshot().unwrap()).unwrap()).unwrap();
    std::fs::write(dir.join("test.figcraft"), figcraft_format::save_bytes(&doc, None).unwrap()).unwrap();
    std::fs::write(dir.join("pixels.bin"), figcraft_render::render_node(&doc, id, 2.0).unwrap().data()).unwrap();
    std::fs::write(dir.join("text.svg"), figcraft_svg::export_svg(&doc, &[id], &Default::default()).unwrap()).unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "portable_text_other_computer", "--nocapture"])
        .env("FIGCRAFT_PORTABLE_TEXT_CHILD", &dir)
        .env("FIGCRAFT_NO_SYSTEM_FONTS", "1")
        .output()
        .unwrap();
    assert!(child.status.success(), "{}\n{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
    eprintln!("portable text: separate-process missing-font equivalence and edit protection verified");
    std::fs::remove_dir_all(dir).unwrap();
}

fn n_width(doc: &Document, id: figcraft_doc::NodeId) -> f64 {
    doc.node(id).unwrap().width
}

#[test]
fn resizing_captures_once_at_commit_and_cancel_retains_original_geometry() {
    figcraft_text::scan_system_fonts();
    let Some(face) = figcraft_text::find_face("Arial", "Regular") else {
        assert!(std::env::var_os("CRAFT_FONTS_REQUIRED").is_none(), "measurement needs a source face");
        return;
    };
    let mut doc = Document::new("Synthetic resize measurement");
    let mut text = TextData::new("Portable text resize measurement. ".repeat(100));
    text.style.font_family = face.family.clone();
    text.style.font_style = face.style.clone();
    text.auto_resize = TextAutoResize::None;
    let id = doc.create(doc.first_page().unwrap(), NodeKind::Text(text), "Text", 0.0, 0.0, 600.0, 8000.0).unwrap();
    let mut session = Session::with_document(doc);
    let before = session.doc().unwrap().clone();
    let original = before.node(id).unwrap().kind.text().unwrap().appearance.clone().unwrap();
    let start = std::time::Instant::now();
    for step in 0..16 {
        let mut preview = before.clone();
        preview.get_mut(id).unwrap().width = 610.0 + f64::from(step);
        figcraft_engine::normalize(&mut preview, &before);
    }
    let eager = start.elapsed();
    session.begin_interaction("Resize portable text").unwrap();
    let start = std::time::Instant::now();
    for step in 0..16 {
        session
            .preview(|doc, _| {
                doc.get_mut(id)?.width = 610.0 + f64::from(step);
                Ok(())
            })
            .unwrap();
        let current = session.doc().unwrap().node(id).unwrap().kind.text().unwrap();
        assert!(std::sync::Arc::ptr_eq(current.appearance.as_ref().unwrap(), &original));
    }
    let previews = start.elapsed();
    session.commit_interaction().unwrap();
    let committed = session.doc().unwrap().node(id).unwrap();
    assert!(figcraft_text::appearance::has_saved(committed.kind.text().unwrap(), Some(committed.width), Some(committed.height)));
    assert!(!std::sync::Arc::ptr_eq(committed.kind.text().unwrap().appearance.as_ref().unwrap(), &original));
    eprintln!(
        "portable resize, 3300 characters / 16 previews: eager={eager:?}, deferred previews={previews:?}, including commit={:?}",
        start.elapsed()
    );
    assert!(session.undo().unwrap());
    assert_eq!(*session.doc().unwrap(), before);
    session.begin_interaction("Cancel resize").unwrap();
    session
        .preview(|doc, _| {
            doc.get_mut(id)?.width = 700.0;
            Ok(())
        })
        .unwrap();
    session.cancel_interaction().unwrap();
    assert_eq!(*session.doc().unwrap(), before);
}
