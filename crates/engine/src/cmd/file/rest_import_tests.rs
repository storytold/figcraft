use super::*;

/// A synthetic REST API file JSON: a component with a text layer, an instance of it whose text
/// is overridden, and an image fill.
fn kit() -> Value {
    let at = |x: f64, y: f64| json!([[1, 0, x], [0, 1, y]]);
    let label = |id: &str, text: &str| {
        json!({"id": id, "name": "Label", "type": "TEXT", "relativeTransform": at(8.0, 8.0), "size": {"x": 40, "y": 20}, "characters": text,
               "style": {"fontFamily": "Inter", "fontSize": 14}, "fills": [{"type": "SOLID", "color": {"r": 0, "g": 0, "b": 0, "a": 1}}]})
    };
    json!({"name": "Kit", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Main", "type": "CANVAS", "backgroundColor": {"r": 0.9, "g": 0.9, "b": 0.9, "a": 1}, "children": [
            {"id": "1:1", "name": "Button", "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 40},
             "fills": [{"type": "SOLID", "color": {"r": 0.2, "g": 0.4, "b": 1, "a": 1}}], "children": [label("1:2", "OK")]},
            {"id": "2:1", "name": "Button", "type": "INSTANCE", "componentId": "1:1", "relativeTransform": at(0.0, 100.0), "size": {"x": 100, "y": 40},
             "fills": [{"type": "SOLID", "color": {"r": 0.2, "g": 0.4, "b": 1, "a": 1}}], "children": [label("I2:1;1:2", "Go")]},
            {"id": "3:1", "name": "Photo", "type": "RECTANGLE", "relativeTransform": at(200.0, 0.0), "size": {"x": 30, "y": 20},
             "fills": [{"type": "IMAGE", "imageRef": "photo1", "scaleMode": "FILL"}]}
        ]}
    ]}, "components": {"1:1": {"key": "k", "name": "Button", "description": ""}}, "styles": {}})
}

fn instance(doc: &Document) -> &Node {
    doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap()
}

/// What a .fig import with a cached library component leaves: the main in the hidden store, a
/// visible instance of it and a layer linked to the main's layer.
fn cached_import() -> Document {
    let mut doc = Document::new("Cached");
    let page = doc.first_page().unwrap();
    let store = doc.ensure_library_store();
    let main = doc
        .create(
            store,
            NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() },
            "Cached main",
            0.0,
            0.0,
            20.0,
            20.0,
        )
        .unwrap();
    let body = doc.create(main, NodeKind::Rectangle, "Body", 0.0, 0.0, 20.0, 20.0).unwrap();
    let inst = doc.create(page, NodeKind::Instance { component: main, props: Default::default() }, "Cached instance", 0.0, 0.0, 20.0, 20.0).unwrap();
    let mut copy = doc.node(body).unwrap().clone();
    copy.id = doc.alloc_id();
    copy.parent = None;
    copy.source = Some(body);
    let copy_id = copy.id;
    doc.insert(copy, inst, None).unwrap();
    assert!(doc.is_remote(main) && doc.node(copy_id).unwrap().source == Some(body));
    doc
}

/// Pasting layers that use a cached main into another document leaves the main behind, as it
/// does for other mains that are not published: no link points at a layer of the target.
#[test]
fn pasting_instances_of_cached_components_leaves_no_dangling_links() {
    let src = cached_import();
    let mut doc = Document::new("Target");
    let page = doc.first_page().unwrap();
    for i in 0..8 {
        doc.create(page, NodeKind::Rectangle, &format!("Filler {i}"), 0.0, 0.0, 5.0, 5.0).unwrap();
    }
    let inst = src.nodes().find(|n| n.name == "Cached instance").unwrap().id;
    let ids = src.subtree(inst);
    let map: std::collections::BTreeMap<NodeId, NodeId> = ids.iter().map(|i| (*i, doc.alloc_id())).collect();
    let nodes: Vec<Node> = ids.iter().filter_map(|i| src.node(*i).cloned()).collect();
    doc.paste_nodes(&nodes, &map, inst, page, None).unwrap();
    crate::library::adopt_pasted(&mut doc, &src, None, &map).unwrap();
    let pasted = doc.node(map[&inst]).unwrap();
    let NodeKind::Instance { component, .. } = pasted.kind else { panic!() };
    assert!(!doc.contains(component), "the main stays behind: {component:?}");
    assert!(doc.node(pasted.children[0]).unwrap().source.is_none_or(|s| !doc.contains(s)));
}

#[test]
fn appending_pages_with_cached_library_components_brings_the_components() {
    let other = cached_import();
    let mut doc = Document::new("Target");
    let page = doc.first_page().unwrap();
    // Ids of the target that would collide with the other document's store.
    for i in 0..8 {
        doc.create(page, NodeKind::Rectangle, &format!("Filler {i}"), 0.0, 0.0, 5.0, 5.0).unwrap();
    }
    append_pages(&mut doc, &other).unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
    assert!(crate::components::mirror_issues(&doc).is_empty(), "{:?}", crate::components::mirror_issues(&doc));
    let inst = doc.nodes().find(|n| n.name == "Cached instance").unwrap();
    let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
    assert_eq!(doc.node(component).unwrap().name, "Cached main");
    assert!(doc.is_remote(component), "the main comes along into the store");
    let layer = doc.node(inst.children[0]).unwrap();
    assert_eq!(layer.source, doc.node(component).unwrap().children.first().copied());
}

#[test]
fn rest_json_loads_with_linked_instances() {
    let bytes = serde_json::to_vec(&kit()).unwrap();
    let (doc, warnings) = load_bytes(&bytes, "kit.json").unwrap();
    assert_eq!(doc.name, "Kit");
    assert!(warnings.iter().any(|w| w.contains("placeholder")), "{warnings:?}");
    let inst = instance(&doc);
    let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
    assert_eq!(inst.source, Some(component));
    let main_label = doc.node(component).unwrap().children[0];
    let label = doc.node(inst.children[0]).unwrap();
    assert_eq!(label.source, Some(main_label));
    assert!(label.overrides.contains("characters"), "{:?}", label.overrides);
    assert!(crate::components::mirror_issues(&doc).is_empty());
    assert!(thumbnail(&doc).is_some());
}

#[test]
fn open_takes_data_and_a_name_as_a_new_unsaved_document() {
    let mut saved = Session::new();
    saved.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let bytes = file_bytes(saved.ds().unwrap()).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mut s = Session::new();
    let recent = s.prefs.recent_files.clone();
    let r = s.execute("file.open", &json!({"data": data, "name": "Dropped.figcraft"})).unwrap();
    let open = s.docs.len();
    assert_eq!(r["index"], open - 1, "a document of its own, made active");
    assert_eq!(s.ds().unwrap().path, None, "no path to save over");
    assert!(s.doc().unwrap().nodes().any(|n| matches!(n.kind, NodeKind::Rectangle)));
    assert_eq!(s.prefs.recent_files, recent, "nothing on disk to remember");
    assert!(s.execute("file.open", &json!({"data": data})).is_err(), "the name tells the format");
    assert!(s.execute("file.open", &json!({"data": "not base64!", "name": "x.svg"})).is_err());
    assert_eq!(s.docs.len(), open, "a refused open adds nothing");
}

#[test]
fn save_as_data_hands_back_the_file_and_marks_it_saved_once_delivered() {
    // The browser has no paths: Save as returns the file for a download (#969).
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(s.execute("file.saveAs", &json!({})).is_err(), "no path and no data: still asks for a path");
    let recent = s.prefs.recent_files.clone();
    let r = s.execute("file.saveAs", &json!({"data": true})).unwrap();
    assert_eq!(r["name"], "Untitled.figcraft");
    assert!(s.ds().unwrap().is_dirty(), "not delivered yet: a refused download must not lose the edits");
    s.execute("file.markSaved", &json!({})).unwrap();
    assert!(!s.ds().unwrap().is_dirty());
    assert_eq!(s.ds().unwrap().path, None, "still no path to save over");
    assert_eq!(s.prefs.recent_files, recent, "nothing on disk to remember");
    let mut t = Session::new();
    t.execute("file.open", &json!({"data": r["data"], "name": r["name"]})).unwrap();
    assert!(t.doc().unwrap().nodes().any(|n| matches!(n.kind, NodeKind::Rectangle)), "the bytes reopen");
}

#[test]
fn open_and_import_rest_json() {
    let dir = std::env::temp_dir().join(format!("figcraft-rest-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("images")).unwrap();
    let png = {
        let mut d = Document::new("x");
        let p = d.first_page().unwrap();
        let id = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 3.0, 2.0).unwrap();
        export_bytes(&d, &[id], ExportFormat::Png, 1.0).unwrap()
    };
    std::fs::write(dir.join("images").join("photo1.png"), &png).unwrap();
    let path = dir.join("kit.json");
    std::fs::write(&path, serde_json::to_vec(&kit()).unwrap()).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"path": path})).unwrap();
    assert!(r["warnings"].as_array().is_some_and(|w| w.iter().all(|w| !w.as_str().unwrap_or("").contains("placeholder"))), "{r}");
    assert!(r.get("repairs").is_none());
    assert_eq!(s.ds().unwrap().path, None, "an import is never saved over its JSON");
    let doc = s.doc().unwrap();
    let photo = doc.nodes().find(|n| n.name == "Photo").unwrap();
    let figcraft_color::PaintKind::Image(img) = &photo.fills[0].kind else { panic!() };
    assert_eq!(doc.images.get(&img.image).map(|i| (i.width, i.height)), Some((3, 2)), "read from images/ next to the file");

    // Import into another document: pages are appended and references follow the new ids.
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&kit()).unwrap());
    let r = s.execute("file.import", &json!({"data": data, "name": "kit.json"})).unwrap();
    assert_eq!(r["pages"].as_array().map(Vec::len), Some(1));
    assert!(r["warnings"].is_array(), "{r}");
    let doc = s.doc().unwrap();
    assert_eq!(doc.pages.len(), 2);
    let page = doc.node(doc.pages[1]).unwrap();
    assert!(matches!(page.kind, NodeKind::Page { background, .. } if (background.r - 0.9).abs() < 1e-6));
    let inst = instance(doc);
    let NodeKind::Instance { component, .. } = inst.kind else { panic!() };
    assert!(doc.node(component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })));
    let label = doc.node(inst.children[0]).unwrap();
    assert!(label.source.is_some_and(|src| doc.is_ancestor(component, src)));
    assert!(crate::components::mirror_issues(doc).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
