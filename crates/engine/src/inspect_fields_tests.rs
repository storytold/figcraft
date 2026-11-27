//! Compact inspection keeps the tree useful without building every layer property.

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::inspect::{InspectOptions, document, document_with_options, node_json, node_json_with_options};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn create(s: &mut Session, params: Value) -> NodeId {
    crate::params::id_of(&run(s, "node.create", params)["id"]).unwrap()
}

fn tree() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    let frame = create(&mut s, json!({"type": "frame", "name": "Card", "x": 100, "y": 200, "width": 300, "height": 200}));
    let text = create(&mut s, json!({"type": "text", "parent": frame, "name": "Title", "text": "Hello", "x": 10, "y": 20}));
    create(&mut s, json!({"type": "rectangle", "parent": frame, "name": "Hidden", "props": {"visible": false, "fill": "#123456"}}));
    (s, frame, text)
}

fn node(s: &Session, id: NodeId, depth: usize, params: Value) -> Value {
    node_json_with_options(s.doc().unwrap(), id, depth, &InspectOptions::parse(&params).unwrap())
}

fn keys(v: &Value, allowed: &[&str]) {
    for (key, value) in v.as_object().unwrap() {
        assert!(allowed.contains(&key.as_str()), "unexpected {key}: {v}");
        if key == "children" {
            for child in value.as_array().unwrap() {
                keys(child, allowed);
            }
        }
    }
}

fn golden_tree() -> (Session, NodeId) {
    let mut s = Session::new();
    let frame = create(
        &mut s,
        json!({"type": "frame", "name": "Card", "x": 100, "y": 200, "width": 300, "height": 200,
        "props": {"layout": "horizontal", "padding": 8, "gap": 4, "fill": "#123456", "stroke": "#abcdef", "strokeWeight": 2,
            "effects": [{"type": "dropShadow", "offset": [2, 3], "radius": 4}], "cornerRadius": 12}}),
    );
    create(
        &mut s,
        json!({"type": "text", "parent": frame, "name": "Title", "text": "Hello", "width": 80, "height": 24,
        "props": {"fontFamily": "Synthetic", "fontSize": 18, "fontStyle": "Bold", "letterSpacing": 1, "lineHeight": 24,
            "textAlign": "center", "textDecoration": "underline", "fill": "#fedcba"}}),
    );
    create(
        &mut s,
        json!({"type": "rectangle", "parent": frame, "name": "Hidden", "width": 20, "height": 30,
        "props": {"visible": false, "locked": true, "fill": "#654321"}}),
    );
    let main = create(&mut s, json!({"type": "component", "name": "Button", "x": 400, "y": 200, "width": 80, "height": 40}));
    run(&mut s, "instance.create", json!({"component": main, "x": 500, "y": 200}));
    run(&mut s, "page.add", json!({"name": "Second"}));
    (s, frame)
}

#[test]
fn empty_options_keep_the_main_tree_and_document_output() {
    let (s, frame) = golden_tree();
    // Captured with this builder on origin/main (43af145a12eacd1e87f5f5e6984c49b7da040288), plus the
    // document `id` that #1533 added. That id comes from a process-wide counter, so it is the
    // session's own.
    let expected = json!({
        "colorProfile": "srgb",
        "currentPage": 7,
        "dirty": true,
        "history": {
            "redo": 0,
            "undo": [
                "Add page",
                "Create instance",
                "Create layer",
                "Create layer",
                "Create layer",
                "Create layer"
            ]
        },
        "id": s.ds().unwrap().uid,
        "images": 0,
        "kind": "design",
        "name": "Untitled",
        "pages": [
            {
                "background": "#F5F5F5",
                "children": [
                    {
                        "absoluteBounds": {
                            "height": 200.0,
                            "width": 300.0,
                            "x": 100.0,
                            "y": 200.0
                        },
                        "children": [
                            {
                                "absoluteBounds": {
                                    "height": 24.0,
                                    "width": 80.0,
                                    "x": 110.0,
                                    "y": 210.0
                                },
                                "characters": "Hello",
                                "fills": [
                                    {
                                        "color": "#FEDCBA",
                                        "type": "solid"
                                    }
                                ],
                                "fontFamily": "Synthetic",
                                "fontSize": 18.0,
                                "fontStyle": "Bold",
                                "fontWeight": 700,
                                "height": 24.0,
                                "id": 3,
                                "letterSpacing": "1",
                                "lineHeight": "24",
                                "name": "Title",
                                "sizing": {
                                    "horizontal": "fixed",
                                    "vertical": "hug"
                                },
                                "textAlign": "center",
                                "textAutoResize": "height",
                                "textDecoration": "underline",
                                "type": "TEXT",
                                "width": 80.0,
                                "x": 10.0,
                                "y": 10.0
                            },
                            {
                                "absoluteBounds": {
                                    "height": 30.0,
                                    "width": 20.0,
                                    "x": 100.0,
                                    "y": 200.0
                                },
                                "fills": [
                                    {
                                        "color": "#654321",
                                        "type": "solid"
                                    }
                                ],
                                "height": 30.0,
                                "id": 4,
                                "locked": true,
                                "name": "Hidden",
                                "sizing": {
                                    "horizontal": "fixed",
                                    "vertical": "fixed"
                                },
                                "type": "RECTANGLE",
                                "visible": false,
                                "width": 20.0,
                                "x": 0.0,
                                "y": 0.0
                            }
                        ],
                        "clip": true,
                        "cornerRadius": 12.0,
                        "effects": [
                            {
                                "blend": "normal",
                                "color": {
                                    "a": 0.25,
                                    "b": 0.0,
                                    "g": 0.0,
                                    "r": 0.0
                                },
                                "offset": [
                                    2.0,
                                    3.0
                                ],
                                "radius": 4.0,
                                "showBehind": false,
                                "spread": 0.0,
                                "type": "dropShadow",
                                "visible": true
                            }
                        ],
                        "fills": [
                            {
                                "color": "#123456",
                                "type": "solid"
                            }
                        ],
                        "height": 200.0,
                        "id": 2,
                        "layout": {
                            "columnTracks": [],
                            "contentAlign": "auto",
                            "crossAlign": "min",
                            "crossGap": 10.0,
                            "gap": 4.0,
                            "gridColumns": 2,
                            "gridRows": 2,
                            "mainAlign": "min",
                            "mode": "horizontal",
                            "padding": [
                                8.0,
                                8.0,
                                8.0,
                                8.0
                            ],
                            "reverseZ": false,
                            "rowTracks": [],
                            "strokesIncluded": true,
                            "wrap": false
                        },
                        "name": "Card",
                        "sizing": {
                            "horizontal": "fixed",
                            "vertical": "fixed"
                        },
                        "strokeAlign": "inside",
                        "strokeWeight": 2.0,
                        "strokes": [
                            {
                                "color": "#ABCDEF",
                                "type": "solid"
                            }
                        ],
                        "type": "FRAME",
                        "width": 300.0,
                        "x": 100.0,
                        "y": 200.0
                    },
                    {
                        "absoluteBounds": {
                            "height": 40.0,
                            "width": 80.0,
                            "x": 400.0,
                            "y": 200.0
                        },
                        "clip": true,
                        "fills": [
                            {
                                "color": "#FFFFFF",
                                "type": "solid"
                            }
                        ],
                        "height": 40.0,
                        "id": 5,
                        "name": "Button",
                        "type": "COMPONENT",
                        "width": 80.0,
                        "x": 400.0,
                        "y": 200.0
                    },
                    {
                        "absoluteBounds": {
                            "height": 40.0,
                            "width": 80.0,
                            "x": 500.0,
                            "y": 200.0
                        },
                        "clip": true,
                        "fills": [
                            {
                                "color": "#FFFFFF",
                                "type": "solid"
                            }
                        ],
                        "height": 40.0,
                        "id": 6,
                        "mainComponent": 5,
                        "mainComponentName": "Button",
                        "name": "Button",
                        "type": "INSTANCE",
                        "width": 80.0,
                        "x": 500.0,
                        "y": 200.0
                    }
                ],
                "id": 1,
                "name": "Page 1",
                "type": "PAGE"
            },
            {
                "background": "#F5F5F5",
                "id": 7,
                "name": "Second",
                "type": "PAGE"
            }
        ],
        "path": null,
        "selection": [],
        "styles": [],
        "textEdit": null,
        "variables": 0
    });
    let expected_node = &expected["pages"][0]["children"][0];
    assert_eq!(node_json(s.doc().unwrap(), frame, 4).to_string(), expected_node.to_string());
    assert_eq!(document(&s, 4, true).to_string(), expected.to_string());
    for params in [json!({}), json!({"outline": false})] {
        let options = InspectOptions::parse(&params).unwrap();
        assert_eq!(node_json_with_options(s.doc().unwrap(), frame, 4, &options).to_string(), expected_node.to_string());
        assert_eq!(document_with_options(&s, 4, true, &options).to_string(), expected.to_string());
    }
}

#[test]
fn fields_filter_recursively_including_pages_without_changing_metadata_or_history() {
    let (s, _, _) = tree();
    let before = document(&s, 4, true);
    let selection = s.selection();
    let history = s.undo_depth();
    let options = InspectOptions::parse(&json!({"fields": ["name", "characters", "name"], "outline": false})).unwrap();
    let compact = document_with_options(&s, 4, true, &options);
    let mut metadata = before.clone();
    metadata["pages"] = compact["pages"].clone();
    assert_eq!(compact, metadata);
    keys(&compact["pages"][0], &["id", "name", "characters", "children", "childCount"]);
    assert_eq!(compact["pages"][0]["children"][0]["children"][0]["characters"], "Hello");
    assert_eq!(s.selection(), selection);
    assert_eq!(s.undo_depth(), history);
    assert_eq!(document(&s, 4, true), before);
}

#[test]
fn an_empty_field_list_keeps_ids_and_structure_at_depth_boundaries() {
    let (s, frame, _) = tree();
    let compact = node(&s, frame, 4, json!({"fields": []}));
    keys(&compact, &["id", "children", "childCount"]);
    assert_eq!(compact["children"].as_array().unwrap().len(), 2);
    assert_eq!(node(&s, frame, 0, json!({"fields": []})), json!({"id": frame, "childCount": 2}));
    let options = InspectOptions::parse(&json!({"fields": []})).unwrap();
    let summary = document_with_options(&s, 0, false, &options);
    assert_eq!(summary["pages"][0], json!({"id": s.page().unwrap(), "childCount": 1}));
}

#[test]
fn geometry_keys_are_independently_selectable() {
    let (s, frame, text) = tree();
    let fields = ["x", "y", "width", "height", "absoluteBounds"];
    let compact = node(&s, frame, 2, json!({"fields": fields}));
    keys(&compact, &["id", "children", "childCount", "x", "y", "width", "height", "absoluteBounds"]);
    assert_eq!((compact["x"].as_f64(), compact["y"].as_f64()), (Some(100.0), Some(200.0)));
    assert_eq!(compact["children"][0]["x"], 10.0);
    assert_eq!(compact["children"][0]["absoluteBounds"]["x"], 110.0);
    for key in fields {
        let one = node(&s, text, 0, json!({"fields": [key]}));
        assert_eq!(one.as_object().unwrap().len(), 2, "{one}");
        assert_eq!(one[key], compact["children"][0][key]);
    }
}

#[test]
fn outline_truncates_only_after_120_unicode_scalars() {
    let (mut s, _, text) = tree();
    for unit in ["a", "界", "\u{1f980}"] {
        for len in [0, 119, 120, 121] {
            let content = unit.repeat(len);
            s.doc_mut().unwrap().get_mut(text).unwrap().kind.text_mut().unwrap().characters = content.clone();
            let outline = node(&s, text, 0, json!({"outline": true}));
            let want = if len > 120 { format!("{}...", unit.repeat(120)) } else { content.clone() };
            assert_eq!(outline["characters"], want);
            assert_eq!(node(&s, text, 0, json!({"fields": ["characters"]}))["characters"], content);
            keys(&outline, &["id", "name", "type", "characters", "mainComponentName", "visible", "children", "childCount"]);
        }
    }
}

#[test]
fn selecting_visible_makes_it_explicit_for_every_node_including_pages() {
    let (s, frame, _) = tree();
    for params in [json!({"fields": ["visible"]}), json!({"outline": true})] {
        let options = InspectOptions::parse(&params).unwrap();
        let tree = document_with_options(&s, 4, true, &options);
        assert_eq!(tree["pages"][0]["visible"], true);
        let card = &tree["pages"][0]["children"][0];
        assert_eq!(card["visible"], true);
        assert_eq!(card["children"][0]["visible"], true);
        assert_eq!(card["children"][1]["visible"], false);
    }
    let no_visible = node(&s, frame, 4, json!({"fields": ["name"]}));
    assert!(no_visible["children"][1].get("visible").is_none());
}

#[test]
fn component_names_are_selected_without_other_instance_properties_even_with_a_missing_main() {
    let mut s = Session::new();
    let main = create(&mut s, json!({"type": "component", "name": "Button"}));
    let instance = crate::params::id_of(&run(&mut s, "instance.create", json!({"component": main}))["id"]).unwrap();
    for params in [json!({"fields": ["mainComponentName"]}), json!({"outline": true})] {
        let compact = node(&s, instance, 0, params);
        assert_eq!(compact["mainComponentName"], "Button");
        assert!(compact.get("mainComponent").is_none());
    }
    if let NodeKind::Instance { component, .. } = &mut s.doc_mut().unwrap().get_mut(instance).unwrap().kind {
        *component = NodeId(u64::MAX);
    }
    let compact = node(&s, instance, 0, json!({"outline": true}));
    assert_eq!(compact.get("mainComponentName"), Some(&Value::Null));
}

#[test]
fn page_properties_and_other_page_summaries_respect_fields() {
    let (mut s, _, _) = tree();
    let first = s.page().unwrap();
    run(&mut s, "node.set", json!({"ids": [first], "props": {"pluginData": {"source": "synthetic"}}}));
    run(&mut s, "page.add", json!({"name": "Second"}));
    let options = InspectOptions::parse(&json!({"fields": ["pluginData", "background"]})).unwrap();
    let compact = document_with_options(&s, 4, false, &options);
    assert_eq!(compact["pages"].as_array().unwrap().len(), 2);
    let page = &compact["pages"][0];
    keys(page, &["id", "pluginData", "background", "childCount", "children"]);
    assert_eq!(page["pluginData"]["source"], "synthetic");
    assert_eq!(page["childCount"], 1);
    assert!(page["background"].is_string());
    assert!(page.get("name").is_none());
}

#[test]
fn the_catalogue_accepts_emitted_keys_and_structure_keys() {
    let (mut s, frame, text) = tree();
    run(&mut s, "node.set", json!({"ids": [frame], "props": {"layout": "grid", "fill": "#123456", "stroke": "#abcdef"}}));
    s.doc_mut().unwrap().get_mut(frame).unwrap().dev_note = "Ready for review".into();
    s.doc_mut().unwrap().get_mut(frame).unwrap().dev_changed = true;
    let main = create(&mut s, json!({"type": "component", "name": "Button"}));
    let instance = crate::params::id_of(&run(&mut s, "instance.create", json!({"component": main}))["id"]).unwrap();
    s.doc_mut().unwrap().get_mut(text).unwrap().kind.text_mut().unwrap().style.style_id = Some("synthetic-style".into());
    // Exercise each of the six property helpers, including their more expensive values.
    for (id, key, sibling) in [
        (frame, "absoluteBounds", "width"),
        (frame, "fills", "strokes"),
        (text, "gridCell", "sizing"),
        (frame, "devNote", "devChanged"),
        (text, "textStyleRanges", "fontFamily"),
        (instance, "mainComponentName", "mainComponent"),
    ] {
        let full = node_json(s.doc().unwrap(), id, 0);
        assert!(full.get(key).is_some(), "fixture must emit {key}: {full}");
        assert!(full.get(sibling).is_some(), "fixture must emit {sibling}: {full}");
        let selected = node(&s, id, 0, json!({"fields": [key]}));
        assert_eq!(selected[key], full[key]);
        assert!(selected.get(sibling).is_none(), "unrequested {sibling}: {selected}");
        keys(&selected, &["id", "children", "childCount", key]);
    }
    let full = node_json(s.doc().unwrap(), s.page().unwrap(), 3);
    fn check(s: &Session, full: &Value) {
        let id = crate::params::id_of(&full["id"]).unwrap();
        for (key, value) in full.as_object().unwrap() {
            let selected = node(s, id, 2, json!({"fields": [key]}));
            if key == "children" {
                assert_eq!(selected[key].as_array().map(Vec::len), value.as_array().map(Vec::len));
            } else {
                assert_eq!(&selected[key], value, "{key}");
            }
        }
        for child in full["children"].as_array().into_iter().flatten() {
            check(s, child);
        }
    }
    check(&s, &full);
    let structural = node(&s, frame, 0, json!({"fields": ["id", "children", "childCount"]}));
    assert_eq!(structural, json!({"id": frame, "childCount": 2}));
}

#[test]
fn invalid_options_are_errors_and_unknown_names_list_the_catalogue() {
    for params in [
        json!({"fields": null}),
        json!({"fields": "name"}),
        json!({"fields": {"name": true}}),
        json!({"fields": [false]}),
        json!({"fields": [null]}),
        json!({"fields": ["name", 1]}),
        json!({"outline": null}),
        json!({"outline": "true"}),
        json!({"outline": 1}),
        json!({"outline": []}),
        json!({"outline": true, "fields": []}),
        json!({"outline": true, "fields": ["name"]}),
    ] {
        assert!(InspectOptions::parse(&params).is_err(), "{params}");
    }
    for key in ["Name", "absolute_bounds", "noSuchField", "truncated"] {
        let error = InspectOptions::parse(&json!({"fields": [key]})).unwrap_err();
        assert!(error.contains(key), "{error}");
        for valid in crate::inspect::NODE_KEYS {
            assert!(error.contains(valid), "missing {valid}: {error}");
        }
    }
}

#[test]
fn node_get_uses_options_but_keeps_its_existing_depth_and_selection() {
    let (mut s, frame, _) = tree();
    let selection = s.selection();
    let history = s.undo_depth();
    let default = node_json(s.doc().unwrap(), frame, 1);
    assert_eq!(run(&mut s, "node.get", json!({"id": frame})), default);
    let compact = run(&mut s, "node.get", json!({"id": frame, "fields": []}));
    assert!(compact["children"].is_array(), "the engine command still inspects to depth 1");
    let error = s.execute("node.get", &json!({"id": frame, "fields": [], "depth": 0})).unwrap_err();
    assert!(error.to_string().contains("unknown parameter `depth`"), "{error}");
    keys(&compact, &["id", "children", "childCount"]);
    assert!(s.execute("node.get", &json!({"id": frame, "outline": true, "fields": []})).is_err());
    assert_eq!(s.selection(), selection);
    assert_eq!(s.undo_depth(), history);
}

/// Text and stroke settings a layer stores are reported, under the names `node.set` takes, so a
/// value read can be written back unchanged.
#[test]
fn stored_text_and_stroke_settings_are_read_back_as_set() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let text = s.execute("node.create", &json!({"type": "text", "text": "Hi", "width": 120})).unwrap()["id"].clone();
    let line =
        s.execute("node.create", &json!({"type": "line", "width": 80, "props": {"strokes": ["#000000"], "strokeWeight": 2}})).unwrap()["id"].clone();
    let set = json!({
        "textAlignVertical": "middle", "textCase": "upper", "paragraphSpacing": 8.0, "paragraphIndent": 12.0, "features": {"liga": false},
    });
    s.execute("node.set", &json!({"ids": [text.clone()], "props": set})).unwrap();
    s.execute("node.set", &json!({"ids": [line.clone()], "props": {"startCap": "round", "endCap": "triangleArrow"}})).unwrap();
    let got = s.execute("node.get", &json!({"id": text})).unwrap();
    for (key, value) in set.as_object().unwrap() {
        assert_eq!(&got[key], value, "{key}: {got}");
        // Written back as read, it changes nothing.
        s.execute("node.set", &json!({"ids": [text.clone()], "props": {key.as_str(): got[key].clone()}})).unwrap();
        assert_eq!(s.execute("node.get", &json!({"id": text})).unwrap()[key], *value, "{key}");
    }
    let got = s.execute("node.get", &json!({"id": line})).unwrap();
    assert_eq!((got["startCap"].as_str(), got["endCap"].as_str()), (Some("round"), Some("triangleArrow")), "{got}");
    // Defaults stay out, as they do for other keys.
    let plain = s.execute("node.create", &json!({"type": "text", "text": "x"})).unwrap()["id"].clone();
    let got = s.execute("node.get", &json!({"id": plain})).unwrap();
    for key in ["textAlignVertical", "textCase", "paragraphSpacing", "paragraphIndent", "features", "startCap", "endCap", "variableModes"] {
        assert!(got.get(key).is_none(), "{key} at its default: {got}");
    }
}

/// A frame's variable modes, a layer's component property bindings and an exposed nested instance
/// are reported.
#[test]
fn modes_property_bindings_and_exposed_instances_are_read() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let c = s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    let frame = s.execute("node.create", &json!({"type": "frame", "width": 100, "height": 100})).unwrap()["id"].clone();
    s.execute("variable.setMode", &json!({"ids": [frame.clone()], "collection": c["id"], "mode": "Dark"})).unwrap();
    let modes = s.execute("node.get", &json!({"id": frame})).unwrap()["variableModes"].clone();
    assert_eq!(modes.as_object().map(|m| m.len()), Some(1), "{modes}");
    let card = s.execute("node.create", &json!({"type": "frame", "name": "Card", "width": 100, "height": 50})).unwrap()["id"].clone();
    let title = s.execute("node.create", &json!({"type": "text", "parent": card.clone(), "text": "Title"})).unwrap()["id"].clone();
    let main = s.execute("component.create", &json!({"ids": [card]})).unwrap()["id"].clone();
    s.execute("component.addProperty", &json!({"id": main.clone(), "name": "Label", "type": "text", "layer": title.clone()})).unwrap();
    let refs = s.execute("node.get", &json!({"id": title})).unwrap()["propertyReferences"].clone();
    assert!(refs.as_object().is_some_and(|r| r.values().any(|v| v.as_str().is_some_and(|p| p.starts_with("Label")))), "{refs}");
    let tag = s.execute("node.create", &json!({"type": "frame", "name": "Tag", "x": 300, "width": 40, "height": 20})).unwrap()["id"].clone();
    let tag_main = s.execute("component.create", &json!({"ids": [tag]})).unwrap()["id"].clone();
    let nested = s.execute("instance.create", &json!({"component": tag_main})).unwrap()["id"].clone();
    s.execute("node.reparent", &json!({"ids": [nested.clone()], "parent": main})).unwrap();
    s.execute("component.exposeInstance", &json!({"id": nested.clone()})).unwrap();
    assert_eq!(s.execute("node.get", &json!({"id": nested})).unwrap()["exposedInstance"], true);
}

/// Every key a layer's JSON can have is one `fields` accepts: a key missing from `NODE_KEYS` can be
/// read in full output but is refused when asked for by name (`dashCap` was).
#[test]
fn every_key_node_get_writes_can_be_asked_for() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let frame = s
        .execute("node.create", &json!({"type": "frame", "width": 200, "height": 100, "props": {"layout": {"mode": "horizontal"}, "gap": 4}}))
        .unwrap()["id"]
        .clone();
    let line = s
        .execute(
            "node.create",
            &json!({"type": "line", "parent": frame, "width": 80, "props": {
            "strokes": ["#000000"], "strokeWeight": 2, "dashes": [4, 2], "dashCap": "round", "strokeJoin": "round",
            "opacity": 0.5, "visible": false, "locked": true}}),
        )
        .unwrap()["id"]
        .clone();
    s.execute("node.create", &json!({"type": "text", "parent": frame, "text": "Hi", "props": {"textDecoration": "underline"}})).unwrap();
    s.execute("annotation.add", &json!({"id": line, "label": "Note"})).unwrap();
    let mut keys = std::collections::BTreeSet::new();
    fn walk(v: &Value, keys: &mut std::collections::BTreeSet<String>) {
        if let Some(o) = v.as_object() {
            keys.extend(o.keys().cloned());
            for c in o.get("children").and_then(Value::as_array).into_iter().flatten() {
                walk(c, keys);
            }
        }
    }
    walk(&node_json(s.doc().unwrap(), crate::params::id_of(&frame).unwrap(), 4), &mut keys);
    assert!(keys.contains("dashCap"), "the test sets a dash cap: {keys:?}");
    for key in keys {
        assert!(crate::inspect::NODE_KEYS.contains(&key.as_str()), "`{key}` is written but `fields` refuses it");
        let one = InspectOptions::parse(&json!({"fields": [key]}));
        assert!(one.is_ok(), "{key}: {one:?}");
    }
}
