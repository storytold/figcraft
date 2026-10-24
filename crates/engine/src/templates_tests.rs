//! Starter templates: each builds, the same every time, into a valid document that saves, reopens
//! and exports; `template.list` and `file.newFromTemplate` open them (and survive junk); and
//! building stays fast.

use figcraft_color::Color;
use figcraft_doc::{Document, NodeKind};
use serde_json::{Value, json};

use crate::templates::{Kind, TEMPLATES, find};
use crate::{EngineError, Session};

fn built(id: &str) -> Document {
    // Other tests index the system fonts while these run; text measured before and after would
    // differ, so they are indexed first (a machine without fonts measures with the built-in ones).
    static FONTS: std::sync::Once = std::sync::Once::new();
    FONTS.call_once(figcraft_text::scan_system_fonts);
    find(id).unwrap().build(Color::DEFAULT_PAGE).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn json_of(doc: &Document) -> String {
    figcraft_format::to_json(doc).unwrap().to_string()
}

fn count(doc: &Document, f: impl Fn(&NodeKind) -> bool) -> usize {
    doc.ids().filter(|id| doc.node(*id).is_some_and(|n| f(&n.kind))).count()
}

#[test]
fn every_template_builds_a_valid_document_the_same_every_time() {
    assert_eq!(TEMPLATES.len(), 9);
    for t in TEMPLATES {
        let doc = built(t.id);
        assert_eq!(doc.validate(), Vec::<String>::new(), "{}", t.id);
        assert_eq!(doc.name, t.name);
        assert_eq!(doc.current_page, doc.first_page(), "{}: opens on its first page", t.id);
        assert!(doc.ids().count() > 40, "{}: has content", t.id);
        assert_eq!(json_of(&doc), json_of(&built(t.id)), "{}: two builds differ", t.id);
        // Real content: shared styles on its layers, and auto layout. (The design document that
        // ships as a file carries what was drawn in it, not the styles a built template sets up.)
        if t.built_in_code() {
            assert!(doc.styles.len() >= 6, "{}", t.id);
            assert!(doc.ids().any(|id| doc.node(id).is_some_and(|n| n.styles.fill.is_some() || n.styles.text.is_some())), "{}", t.id);
            assert!(doc.ids().any(|id| doc.node(id).is_some_and(|n| n.layout.is_some())), "{}", t.id);
        }
        // Nothing waits to be normalised: opening it changes nothing.
        let mut s = Session::with_document(doc.clone());
        s.execute("select.none", &json!({})).unwrap();
        assert_eq!(json_of(s.doc().unwrap()), json_of(&doc), "{}", t.id);
    }
}

#[test]
fn the_shipped_design_opens_as_it_was_drawn() {
    let t = find("app-ui").expect("the design document is a template");
    assert!(!t.built_in_code(), "it ships as a file, not as commands");
    let (drawn, notes) = figcraft_format::load_bytes(include_bytes!("templates/app_ui.figcraft")).unwrap();
    assert_eq!(notes, Vec::<String>::new(), "nothing to repair");
    assert_eq!(drawn.validate(), Vec::<String>::new());
    assert_eq!(drawn.pages.len(), 1);
    assert!(drawn.ids().count() > 1000, "a whole interface, not a stub");
    assert_eq!(drawn.name, t.name, "named after the template");
    let doc = built("app-ui");
    assert_eq!(json_of(&doc), json_of(&drawn), "opened as it was drawn");
}

#[test]
fn ids_are_unique_and_every_kind_has_templates() {
    let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), TEMPLATES.len());
    for kind in Kind::ALL {
        assert!(TEMPLATES.iter().any(|t| t.kind == kind), "{kind:?}");
    }
    for t in TEMPLATES {
        assert!(!t.name.is_empty() && !t.description.is_empty() && !t.description.ends_with('.') && !t.size.is_empty(), "{}", t.id);
    }
}

#[test]
fn templates_use_the_features_they_show() {
    let instances = |d: &Document| count(d, |k| matches!(k, NodeKind::Instance { .. }));
    let components = |d: &Document| count(d, |k| matches!(k, NodeKind::Component { .. }));
    let sets = |d: &Document| count(d, |k| matches!(k, NodeKind::ComponentSet { .. }));
    let app = built("mobile-app");
    assert!(components(&app) >= 4 && sets(&app) == 1 && instances(&app) >= 7);
    assert!(matches!(&app.node(app.first_page().unwrap()).unwrap().kind, NodeKind::Page { flows, .. } if flows.len() == 1), "a prototype flow");
    assert!(app.ids().filter(|id| app.node(*id).is_some_and(|n| !n.interactions.is_empty())).count() >= 5, "prototype links");
    let ds = built("design-system");
    assert_eq!(ds.variables.collections.len(), 2);
    assert!(ds.variables.collections.iter().any(|c| c.modes.iter().map(|m| m.name.as_str()).eq(["Light", "Dark"])));
    assert!(ds.variables.variables.len() >= 16);
    assert!(sets(&ds) == 2 && components(&ds) >= 13 && instances(&ds) >= 9);
    assert!(ds.ids().filter(|id| ds.node(*id).is_some_and(|n| !n.bound.is_empty())).count() > 50, "layers bound to variables");
    assert_eq!(ds.ids().filter(|id| ds.node(*id).is_some_and(|n| !n.var_modes.is_empty())).count(), 4, "frames in Light and in Dark");
    let grid = built("dashboard");
    assert!(grid.ids().any(|id| grid.node(id).is_some_and(|n| n.layout.as_ref().is_some_and(|l| l.mode == figcraft_doc::LayoutMode::Grid))));
    assert!(count(&grid, |k| matches!(k, NodeKind::Vector { .. })) >= 8, "charts drawn from vectors");
    let icons = built("icon-set");
    assert_eq!(components(&icons), 24);
    assert_eq!(instances(&icons), 4);
    assert!(components(&built("wireframe-kit")) >= 9);
    assert_eq!(built("social-posts").node(built("social-posts").first_page().unwrap()).unwrap().children.len(), 5);
    assert_eq!(built("slides").node(built("slides").first_page().unwrap()).unwrap().children.len(), 5);
    assert_eq!(built("landing-page").node(built("landing-page").first_page().unwrap()).unwrap().children.len(), 2, "desktop and phone");
}

#[test]
fn templates_save_and_reopen_losslessly() {
    for t in TEMPLATES {
        let doc = built(t.id);
        let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
        let (back, notes) = figcraft_format::load_bytes(&bytes).unwrap();
        assert_eq!(notes, Vec::<String>::new(), "{}: nothing to repair", t.id);
        assert_eq!(json_of(&back), json_of(&doc), "{}", t.id);
    }
}

#[test]
fn templates_export_to_png_svg_and_pdf() {
    for t in TEMPLATES {
        let mut s = Session::empty();
        s.execute("file.newFromTemplate", &json!({"template": t.id})).unwrap();
        let page = s.page().unwrap();
        let frames = s.doc().unwrap().node(page).unwrap().children.clone();
        // One frame keeps the largest templates quick in debug builds.
        let first = frames.first().map(|f| f.0).unwrap();
        for format in ["png", "svg", "pdf"] {
            let out = s
                .execute("file.export", &json!({"ids": [first], "format": format, "scale": 0.25}))
                .unwrap_or_else(|e| panic!("{} as {format}: {e}", t.id));
            assert!(out["data"].as_str().is_some_and(|d| d.len() > 100), "{} as {format}", t.id);
        }
        let pdf = s.execute("file.exportFramesPdf", &json!({})).unwrap();
        assert!(pdf["data"].as_str().is_some_and(|d| d.len() > 100), "{}", t.id);
    }
}

#[test]
fn template_list_describes_every_template() {
    let mut s = Session::empty();
    let list = s.execute("template.list", &json!({})).unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), TEMPLATES.len());
    for (item, t) in list.iter().zip(TEMPLATES) {
        assert_eq!(item, &json!({"id": t.id, "name": t.name, "description": t.description, "kind": t.kind.id(), "size": t.size}));
    }
    assert!(s.docs.is_empty() && s.ui_requests.is_empty(), "listing opens nothing");
}

#[test]
fn new_from_template_opens_a_new_unsaved_document_and_leaves_the_others_alone() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let before = json_of(s.doc().unwrap());
    let r = s.execute("file.newFromTemplate", &json!({"template": "dashboard"})).unwrap();
    assert_eq!(r, json!({"index": 1, "template": "dashboard"}));
    assert_eq!((s.docs.len(), s.active), (2, 1));
    let ds = s.ds().unwrap();
    assert_eq!(ds.doc.name, "Dashboard");
    assert!(ds.path.is_none() && !ds.is_dirty() && ds.history.undo.is_empty(), "a new file: no path, nothing to save or undo yet");
    assert_eq!(json_of(&s.docs[0].doc), before, "the open document is untouched");
    // It is a document like any other: editable, and the edit is undoable.
    let root = s.doc().unwrap().node(s.page().unwrap()).unwrap().children[0];
    s.execute("node.set", &json!({"ids": [root.0], "props": {"name": "Mine"}})).unwrap();
    assert!(s.ds().unwrap().is_dirty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node(root).unwrap().name, "Dashboard");
    // A second one is a separate document, named as asked.
    let r = s.execute("file.newFromTemplate", &json!({"template": "dashboard", "name": "  Q3 numbers  "})).unwrap();
    assert_eq!(r["index"], 2);
    assert_eq!(s.doc().unwrap().name, "Q3 numbers");
    // The page takes the interface's theme, like a new file.
    s.set_ui_dark(true);
    s.execute("file.newFromTemplate", &json!({"template": "icon-set"})).unwrap();
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(s.page().unwrap()).unwrap().kind, NodeKind::Page { background, .. } if background == Color::DARK_PAGE));
}

#[test]
fn without_a_template_the_app_is_asked_to_show_them() {
    let mut s = Session::empty();
    let r = s.execute("file.newFromTemplate", &json!({})).unwrap();
    assert_eq!(r["templates"].as_array().unwrap().len(), TEMPLATES.len());
    assert!(s.docs.is_empty());
    assert_eq!(s.ui_requests.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["home.templates"]);
}

#[test]
fn junk_never_panics_or_opens_anything() {
    let junk = [
        json!({"template": "nope"}),
        json!({"template": ""}),
        json!({"template": 7}),
        json!({"template": null}),
        json!({"template": ["mobile-app"]}),
        json!({"template": {"id": "mobile-app"}}),
        json!({"template": "mobile-app\u{0}"}),
        json!({"template": "MOBILE-APP"}),
        json!({"template": "x".repeat(100_000)}),
    ];
    for j in &junk {
        let mut s = Session::new();
        match s.execute("file.newFromTemplate", j) {
            Err(EngineError::BadParams { .. }) => {}
            other => panic!("{j}: {other:?}"),
        }
        assert_eq!(s.docs.len(), 1, "{j}");
    }
    // Junk beside a good id: the name falls back to the template's, or is cut to a sane length.
    let mut s = Session::empty();
    for name in [json!(5), json!(null), json!(""), json!("   "), json!({"a": 1}), json!("é".repeat(5000))] {
        s.execute("file.newFromTemplate", &json!({"template": "icon-set", "name": name})).unwrap();
        assert!((1..=200).contains(&s.doc().unwrap().name.chars().count()));
    }
    for j in [Value::Null, json!([1]), json!("x"), json!(3)] {
        let _ = s.execute("file.newFromTemplate", &j);
        let _ = s.execute("template.list", &j);
    }
}

/// Building is fast enough to do on a click: each template well under a second even in an
/// unoptimised build on a busy machine (a release build takes a few milliseconds).
#[test]
fn building_stays_within_its_time_budget() {
    // Indexing the fonts (in `built`) is not part of the budget.
    built("icon-set");
    for t in TEMPLATES {
        let start = std::time::Instant::now();
        let doc = built(t.id);
        let took = start.elapsed();
        println!("{}: {} layers in {:.1} ms", t.id, doc.ids().count(), took.as_secs_f64() * 1000.0);
        assert!(took < std::time::Duration::from_secs(3), "{} took {took:?}", t.id);
    }
}
