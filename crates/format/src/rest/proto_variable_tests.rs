//! REST import of prototypes and variables. Every fixture is synthetic, written by hand from the
//! published schema.

use super::tests::pointers;
use super::*;
use figcraft_doc::proto::{
    Action, DeviceRotation, Direction, Easing, Overflow, OverlayPosition, PrototypeSettings, ScrollPosition, Transition, Trigger,
};
use figcraft_doc::{VariableType, VariableValue};
use serde_json::json;

pub(super) const THEME: &str = "VariableCollectionId:1:1";
pub(super) const BRAND: &str = "VariableID:1:2";
pub(super) const SPACE: &str = "VariableID:1:3";
pub(super) const LABEL: &str = "VariableID:1:6";
pub(super) const FLAG: &str = "VariableID:1:7";

pub(super) fn at(x: f64, y: f64) -> Value {
    json!([[1, 0, x], [0, 1, y]])
}

pub(super) fn alias(id: &str) -> Value {
    json!({"type": "VARIABLE_ALIAS", "id": id})
}

fn rgba(r: f64, g: f64, b: f64) -> Value {
    json!({"r": r, "g": g, "b": b, "a": 1})
}

/// A local variables response: one collection with two modes (the second is the default).
pub(super) fn variables_json() -> Value {
    json!({"status": 200, "error": false, "meta": {
        "variableCollections": {THEME: {"id": THEME, "name": "Theme", "key": "k", "defaultModeId": "1:1", "remote": false,
            "hiddenFromPublishing": false, "variableIds": [],
            "modes": [{"modeId": "1:0", "name": "Light"}, {"modeId": "1:1", "name": "Dark"}]}},
        "variables": {
            BRAND: {"id": BRAND, "name": "color/brand", "variableCollectionId": THEME, "resolvedType": "COLOR", "description": "Brand",
                    "valuesByMode": {"1:0": rgba(1.0, 0.0, 0.0), "1:1": rgba(0.0, 0.0, 1.0)}, "scopes": ["FRAME_FILL", "SHAPE_FILL"]},
            SPACE: {"id": SPACE, "name": "space/md", "variableCollectionId": THEME, "resolvedType": "FLOAT",
                    "valuesByMode": {"1:0": 8}, "scopes": ["ALL_SCOPES"]},
            "VariableID:1:4": {"id": "VariableID:1:4", "name": "color/accent", "variableCollectionId": THEME, "resolvedType": "COLOR",
                    "valuesByMode": {"1:0": alias(BRAND), "1:1": alias(BRAND)}},
            "VariableID:1:5": {"id": "VariableID:1:5", "name": "library", "variableCollectionId": THEME, "resolvedType": "COLOR",
                    "valuesByMode": {"1:0": alias("VariableID:9:9")}},
            LABEL: {"id": LABEL, "name": "copy/label", "variableCollectionId": THEME, "resolvedType": "STRING",
                    "valuesByMode": {"1:0": "Hello", "1:1": "Hi"}, "scopes": ["TEXT_CONTENT", "FONT_WEIGHT"]},
            FLAG: {"id": FLAG, "name": "state/open", "variableCollectionId": THEME, "resolvedType": "BOOLEAN",
                    "valuesByMode": {"1:0": true, "1:1": true}},
            "VariableID:1:8": {"id": "VariableID:1:8", "name": "orphan", "variableCollectionId": "nope", "resolvedType": "FLOAT",
                    "valuesByMode": {"1:0": 1}}
        }
    }})
}

fn node_action(dest: &str, navigation: &str, transition: Value) -> Value {
    json!({"type": "NODE", "destinationId": dest, "navigation": navigation, "transition": transition})
}

/// A prototype: a home screen with a button (several interactions), a details screen linked the
/// legacy way, and an overlay menu.
fn file_json() -> Value {
    let rect = |id: &str, name: &str, extra: Value| {
        let mut r = json!({"id": id, "name": name, "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20},
                           "fills": [{"type": "SOLID", "color": rgba(0.5, 0.5, 0.5)}]});
        if let (Some(o), Some(e)) = (r.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        r
    };
    let interactions = json!([
        {"trigger": {"type": "ON_CLICK"}, "actions": [node_action("1:20", "NAVIGATE", json!({"type": "SMART_ANIMATE", "duration": 450,
            "easing": {"type": "CUSTOM_CUBIC_BEZIER", "easingFunctionCubicBezier": {"x1": 0.1, "y1": 0.2, "x2": 0.3, "y2": 1.4}}}))]},
        {"trigger": {"type": "AFTER_TIMEOUT", "timeout": 800}, "actions": [node_action("1:30", "OVERLAY", json!({"type": "MOVE_IN", "direction": "RIGHT",
            "duration": 250, "matchLayers": false, "easing": {"type": "CUSTOM_SPRING", "easingFunctionSpring": {"mass": 1, "stiffness": 200, "damping": 20}}}))]},
        {"trigger": {"type": "ON_KEY_DOWN", "device": "KEYBOARD", "keyCodes": [16, 65]}, "actions": [{"type": "BACK"}]},
        {"trigger": {"type": "MOUSE_ENTER", "delay": 0}, "actions": [{"type": "URL", "url": "https://example.com"}, {"type": "URL", "url": "https://example.org", "openInNewTab": false}]},
        {"trigger": {"type": "ON_HOVER"}, "actions": [{"type": "SET_VARIABLE", "variableId": FLAG, "variableValue": {"type": "BOOLEAN", "value": false}}]},
        {"trigger": {"type": "ON_PRESS"}, "actions": [{"type": "CONDITIONAL", "conditionalBlocks": [{"actions": [{"type": "CLOSE"}]}]}]},
        {"trigger": {"type": "ON_DRAG"}, "actions": [{"type": "SET_VARIABLE_MODE", "variableCollectionId": THEME, "variableModeId": "1:0"}]},
        {"trigger": {"type": "ON_MEDIA_END"}, "actions": [{"type": "BACK"}]},
        {"trigger": {"type": "MOUSE_UP", "delay": 0}, "actions": [node_action("1:10", "SCROLL_TO", Value::Null), node_action("gone", "NAVIGATE", Value::Null)]}
    ]);
    let home = json!(
    {"id": "1:10", "name": "Home", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 200, "y": 300},
     "layoutMode": "VERTICAL", "itemSpacing": 8, "overflowDirection": "VERTICAL_SCROLLING",
     "fills": [{"type": "SOLID", "color": rgba(0.0, 0.0, 1.0), "boundVariables": {"color": alias(BRAND)}}],
     "boundVariables": {"size": {"x": alias(SPACE)}, "itemSpacing": alias(SPACE), "paddingLeft": alias(SPACE), "paddingRight": alias(SPACE),
                        "effects": [alias(SPACE)], "opacity": alias("VariableID:404")},
     "explicitVariableModes": {THEME: "1:0", "elsewhere": "x"},
     "children": [
        rect("1:11", "Button", json!({"interactions": interactions})),
        rect("1:12", "Header", json!({"scrollBehavior": "FIXED", "layoutPositioning": "ABSOLUTE"})),
        rect("1:13", "Sticky", json!({"scrollBehavior": "STICKY_SCROLLS", "boundVariables": {"rectangleCornerRadii": {
            "RECTANGLE_TOP_LEFT_CORNER_RADIUS": alias(SPACE), "RECTANGLE_TOP_RIGHT_CORNER_RADIUS": alias(SPACE),
            "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS": alias(SPACE), "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS": alias(SPACE)}}})),
        {"id": "1:14", "name": "Label", "type": "TEXT", "relativeTransform": at(0.0, 0.0), "size": {"x": 40, "y": 20}, "characters": "Hello",
         "style": {"fontFamily": "Inter", "fontSize": 8, "boundVariables": {"fontSize": alias(SPACE)}},
         "boundVariables": {"characters": alias(LABEL), "visible": alias(FLAG), "fills": [alias(LABEL)]},
         "fills": [{"type": "SOLID", "color": rgba(0.0, 0.0, 0.0)}]}
     ]});
    let screens = json!([
            {"id": "1:20", "name": "Details", "type": "FRAME", "relativeTransform": at(300.0, 0.0), "size": {"x": 200, "y": 300},
             "transitionNodeID": "1:10", "transitionDuration": 200, "transitionEasing": "EASE_IN"},
            {"id": "1:30", "name": "Menu", "type": "FRAME", "relativeTransform": at(600.0, 0.0), "size": {"x": 100, "y": 100},
             "overlayPositionType": "TOP_RIGHT", "overlayBackgroundInteraction": "NONE",
             "overlayBackground": {"type": "SOLID_COLOR", "color": {"r": 0, "g": 0, "b": 0, "a": 0.4}}}
    ]);
    let mut children = vec![home];
    children.extend(screens.as_array().cloned().unwrap_or_default());
    json!({"name": "Proto", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Screens", "type": "CANVAS", "backgroundColor": rgba(0.9, 0.9, 0.9),
         "flowStartingPoints": [{"nodeId": "1:10", "name": "Main flow"}],
         "prototypeDevice": {"type": "PRESET", "presetIdentifier": "IPHONE_14_PRO_MAX", "rotation": "NONE", "size": {"width": 430, "height": 932}},
         "prototypeBackgrounds": [rgba(0.2, 0.2, 0.2)],
         "children": children}
    ]}})
}

fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no node {name}"))
}

fn has(warnings: &[String], s: &str) -> bool {
    warnings.iter().any(|w| w.contains(s))
}

#[test]
fn variables_collections_modes_aliases_and_scopes() {
    let (doc, warnings) = import_with(&file_json(), Some(&variables_json()), &no_images).unwrap();
    let vars = &doc.variables;
    assert_eq!(vars.collections.len(), 1);
    let theme = &vars.collections[0];
    assert_eq!(theme.name, "Theme");
    let modes: Vec<&str> = theme.modes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(modes, ["Dark", "Light"], "the default mode comes first");

    let ids: Vec<&str> = vars.variables.iter().map(|v| v.id.as_str()).collect();
    assert!(!ids.contains(&"VariableID:1:5") && !ids.contains(&"VariableID:1:8"), "{ids:?}");
    assert_eq!(ids.len(), 5);
    let brand = vars.get(BRAND).unwrap();
    assert_eq!((brand.name.as_str(), brand.kind, brand.description.as_str()), ("color/brand", VariableType::Color, "Brand"));
    assert_eq!(brand.scopes, Some(vec!["FRAME_FILL".to_string(), "SHAPE_FILL".to_string()]));
    let space = vars.get(SPACE).unwrap();
    assert_eq!(space.scopes, None, "ALL_SCOPES is no limit");
    assert_eq!(space.values.get("1:1"), Some(&VariableValue::Number(8.0)), "modes without a value take the default's");
    assert_eq!(vars.get(LABEL).unwrap().scopes, Some(vec!["TEXT_CONTENT".to_string()]), "unknown scopes are dropped");
    assert_eq!(vars.get("VariableID:1:4").unwrap().values.get("1:0"), Some(&VariableValue::Alias(BRAND.into())));
    // Aliases resolve through the default mode (Dark: blue).
    let modes = Default::default();
    assert_eq!(vars.resolve("VariableID:1:4", &modes).and_then(|v| v.type_of()), Some(VariableType::Color));
    let VariableValue::Color(c) = vars.resolve("VariableID:1:4", &modes).unwrap() else { panic!() };
    assert!(c.b > 0.99 && c.r < 0.01);
    for w in ["aliases to variables that aren't", "collection isn't in the variables JSON"] {
        assert!(has(&warnings, w), "{w}: {warnings:?}");
    }
}

#[test]
fn bound_variables_and_explicit_modes() {
    let (doc, warnings) = import_with(&file_json(), Some(&variables_json()), &no_images).unwrap();
    let home = named(&doc, "Home");
    fn bound<'a>(n: &'a Node, f: &str) -> Option<&'a str> {
        n.bound.get(f).map(String::as_str)
    }
    assert_eq!(bound(home, "fill"), Some(BRAND), "paint colour bindings bind the fill");
    assert_eq!(bound(home, "width"), Some(SPACE));
    assert_eq!(bound(home, "gap"), Some(SPACE));
    assert_eq!(bound(home, "paddingHorizontal"), Some(SPACE), "left and right on one variable");
    assert_eq!(bound(home, "opacity"), None, "unknown variables are skipped");
    assert_eq!(home.var_modes.get(THEME).map(String::as_str), Some("1:0"));
    assert_eq!(home.var_modes.len(), 1);
    let sticky = named(&doc, "Sticky");
    assert_eq!(bound(sticky, "cornerRadius"), Some(SPACE), "four equal corners bind the radius");
    let label = named(&doc, "Label");
    assert_eq!(bound(label, "characters"), Some(LABEL));
    assert_eq!(bound(label, "visible"), Some(FLAG));
    assert_eq!(bound(label, "fontSize"), Some(SPACE));
    assert_eq!(bound(label, "fill"), None, "a string variable can't colour a fill");
    for w in ["`effects`", "aren't in the variables JSON (library", "type doesn't fit", "modes of collections"] {
        assert!(has(&warnings, w), "{w}: {warnings:?}");
    }
    // The bindings feed values like ones made with `variable.bind`.
    assert!(doc.validate().is_empty());

    // Without variable definitions the bindings are dropped with a hint.
    let (doc, warnings) = import(&file_json(), &no_images).unwrap();
    assert!(doc.nodes().all(|n| n.bound.is_empty()));
    assert!(has(&warnings, "pass the local variables JSON"), "{warnings:?}");
}

#[test]
fn embedded_variables_match_separate_ones() {
    let mut embedded = file_json();
    embedded["meta"] = variables_json()["meta"].clone();
    let (a, _) = import(&embedded, &no_images).unwrap();
    let (b, _) = import_with(&file_json(), Some(&variables_json()), &no_images).unwrap();
    assert_eq!(a.variables, b.variables);
    // Top-level collections (the `meta` object itself) work too, and the first definition wins.
    let (c, _) = import_with(&embedded, Some(&variables_json()["meta"]), &no_images).unwrap();
    assert_eq!(c.variables, b.variables);
    assert!(is_variables_json(&variables_json()) && is_variables_json(&variables_json()["meta"]) && !is_variables_json(&file_json()));
    let bytes = serde_json::to_vec(&file_json()).unwrap();
    let (d, _) = try_import_with(&bytes, Some(&variables_json()), &no_images).unwrap().unwrap();
    assert_eq!(d.variables, b.variables);
    let (_, warnings) = import_with(&file_json(), Some(&json!({"nope": 1})), &no_images).unwrap();
    assert!(has(&warnings, "has no `variableCollections`"), "{warnings:?}");
}

#[test]
fn interactions_triggers_actions_and_transitions() {
    let (doc, warnings) = import_with(&file_json(), Some(&variables_json()), &no_images).unwrap();
    assert_interactions(&doc, &warnings);
    assert_legacy_transition(&doc);
    assert_interactions_without_variables();
}

fn assert_interactions(doc: &Document, warnings: &[String]) {
    let id = |n: &str| named(doc, n).id;
    let button = named(doc, "Button");
    let its = &button.interactions;
    assert_eq!(its.len(), 8, "{its:#?}");

    assert_eq!(its[0].trigger, Trigger::OnClick);
    assert_eq!(its[0].actions, [Action::Navigate { dest: id("Details"), preserve_scroll: false }]);
    assert_eq!((its[0].transition, its[0].duration_ms), (Transition::SmartAnimate, 450));
    assert_eq!(its[0].easing, Easing::CustomBezier { x1: 0.1, y1: 0.2, x2: 0.3, y2: 1.4 });

    assert_eq!(its[1].trigger, Trigger::AfterDelay { ms: 800 });
    assert_eq!(its[1].transition, Transition::MoveIn { direction: Direction::Right });
    assert_eq!(its[1].easing, Easing::CustomSpring { mass: 1.0, stiffness: 200.0, damping: 20.0 });
    let Action::OpenOverlay { dest, position, close_on_outside, background, .. } = &its[1].actions[0] else { panic!("{:?}", its[1]) };
    assert_eq!((*dest, *position, *close_on_outside), (id("Menu"), OverlayPosition::TopRight, false));
    assert!(background.is_some_and(|b| (b.a - 0.4).abs() < 1e-3));

    assert_eq!(its[2].trigger, Trigger::KeyPress { key: "Shift+A".into() }, "modifier keys make a combination");
    assert_eq!(its[2].actions, [Action::Back]);
    assert_eq!(its[3].trigger, Trigger::MouseEnter);
    let in_place = Action::OpenUrl { url: "https://example.org".into(), new_tab: false };
    assert_eq!(its[3].actions, [Action::OpenUrl { url: "https://example.com".into(), new_tab: true }, in_place.clone()]);
    // Saved only when off, so files from before the option open with links in a new tab.
    assert_eq!(serde_json::to_value(&its[3].actions[0]).unwrap(), json!({"type": "openUrl", "url": "https://example.com"}));
    let saved = serde_json::to_value(&in_place).unwrap();
    assert_eq!(saved["new_tab"], json!(false));
    assert_eq!(serde_json::from_value::<Action>(saved).unwrap(), in_place);
    assert_eq!(its[4].trigger, Trigger::OnHover);
    assert_eq!(its[4].actions, [Action::SetVariable { variable: FLAG.into(), value: json!(false), expr: None }]);
    assert_eq!((its[4].transition, its[4].duration_ms), (Transition::Instant, 300));
    assert_eq!(its[5].actions, [Action::CloseOverlay], "a lone unconditional block is inlined");
    assert_eq!(its[6].trigger, Trigger::OnDrag);
    assert_eq!(its[6].actions, [Action::SetVariableMode { collection: THEME.into(), mode: "1:0".into() }]);
    assert_eq!(its[7].trigger, Trigger::MouseUp);
    assert_eq!(its[7].actions, [Action::ScrollTo { dest: id("Home"), offset: [0.0, 0.0] }], "unresolved destinations are dropped");

    assert!(!has(warnings, "modifier keys"), "{warnings:?}");
    for w in ["video triggers", "without a destination"] {
        assert!(has(warnings, w), "{w}: {warnings:?}");
    }
}

fn assert_legacy_transition(doc: &Document) {
    let id = |n: &str| named(doc, n).id;
    let details = named(doc, "Details");
    assert_eq!(details.interactions.len(), 1);
    let legacy = &details.interactions[0];
    assert_eq!((&legacy.trigger, &legacy.actions[..]), (&Trigger::OnClick, &[Action::Navigate { dest: id("Home"), preserve_scroll: false }][..]));
    assert_eq!((legacy.transition, legacy.duration_ms, legacy.easing), (Transition::Dissolve, 200, Easing::EaseIn));
}

fn assert_interactions_without_variables() {
    let (doc, warnings) = import(&file_json(), &no_images).unwrap();
    assert_eq!(named(&doc, "Button").interactions.len(), 6);
    assert!(has(&warnings, "set-variable actions") && has(&warnings, "set-variable-mode actions"), "{warnings:?}");
}

#[test]
fn scrolling_flows_and_prototype_device() {
    let (doc, _) = import(&file_json(), &no_images).unwrap();
    assert_eq!(named(&doc, "Home").overflow, Overflow::Vertical);
    assert_eq!(named(&doc, "Header").scroll_position, ScrollPosition::Fixed);
    assert_eq!(named(&doc, "Sticky").scroll_position, ScrollPosition::Sticky);
    assert_eq!(named(&doc, "Button").scroll_position, ScrollPosition::Scrolls);
    let NodeKind::Page { flows, prototype, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!() };
    assert_eq!(flows.len(), 1);
    assert_eq!((flows[0].name.as_str(), flows[0].start), ("Main flow", named(&doc, "Home").id));
    assert_eq!(prototype.device, "iPhone 16 Pro Max");
    assert!((prototype.background.r - 0.2).abs() < 1e-3);

    // Devices map onto the frames we draw; the deprecated start node makes a flow.
    let device = |d: Value| {
        let mut f = file_json();
        f["document"]["children"][0]["prototypeDevice"] = d;
        f["document"]["children"][0]["flowStartingPoints"] = json!([]);
        f["document"]["children"][0]["prototypeStartNodeID"] = json!("1:20");
        let (doc, _) = import(&f, &no_images).unwrap();
        let NodeKind::Page { flows, prototype, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!() };
        assert_eq!(flows.iter().map(|f| f.start).collect::<Vec<_>>(), [named(&doc, "Details").id]);
        prototype.clone()
    };
    for (d, want) in [
        (json!({"type": "NONE", "rotation": "NONE"}), "None"),
        (json!({"type": "PRESENTATION", "rotation": "NONE"}), "None"),
        (json!({"type": "PRESET", "presetIdentifier": "GOOGLE_PIXEL_8", "rotation": "NONE"}), "Android Compact"),
        (json!({"type": "PRESET", "presetIdentifier": "APPLE_IPAD_MINI", "rotation": "NONE"}), "iPad mini"),
        (json!({"type": "PRESET", "presetIdentifier": "MACBOOK_PRO_14", "rotation": "NONE"}), "MacBook Air"),
        (json!({"type": "CUSTOM", "size": {"width": 0, "height": 1080}, "rotation": "NONE"}), "None"),
        (json!({"type": "CUSTOM", "size": {"width": "wide", "height": 667}, "rotation": "NONE"}), "None"),
    ] {
        let got = device(d).device;
        assert_eq!(got, want);
        assert!(got == "None" || PrototypeSettings::DEVICES.iter().any(|d| d.name == got), "{got}");
    }
    // Custom sizes keep their screen (turned ones swap their sides).
    for (rotation, want) in [("NONE", (1920.0, 1080.0)), ("CCW_90", (1080.0, 1920.0))] {
        let got = device(json!({"type": "CUSTOM", "size": {"width": 1920, "height": 1080}, "rotation": rotation}));
        assert_eq!(got.device, PrototypeSettings::CUSTOM_DEVICE);
        assert_eq!(got.device().map(|d| (d.width, d.height)), Some(want));
    }
    // A device turned a quarter turn presents in landscape.
    let turned = device(json!({"type": "PRESET", "presetIdentifier": "APPLE_IPHONE_16", "rotation": "CCW_90"}));
    assert_eq!((turned.device.as_str(), turned.rotation), ("iPhone 16", DeviceRotation::Landscape));
    assert_eq!(turned.device().map(|d| (d.width, d.height)), Some((852.0, 393.0)));
}

// ---- robustness ----

/// The prototype file with its variables embedded.
fn combined() -> Value {
    let mut v = file_json();
    v["meta"] = variables_json()["meta"].clone();
    v
}

#[test]
fn junk_in_prototypes_and_variables_never_panics() {
    let junk = [json!(null), json!(-1), json!(1e308), json!("x"), json!([]), json!({}), json!(true), json!({"type": "VARIABLE_ALIAS", "id": BRAND})];
    let base = combined();
    let mut ptrs = Vec::new();
    pointers(&base, String::new(), &mut ptrs);
    for p in ptrs.iter().filter(|p| !p.is_empty()) {
        for j in &junk {
            let mut v = base.clone();
            if let Some(slot) = v.pointer_mut(p) {
                *slot = j.clone();
            }
            if let Ok((doc, _)) = import(&v, &no_images) {
                assert!(doc.validate().is_empty(), "{p} = {j}");
                for n in doc.nodes() {
                    for it in &n.interactions {
                        assert!(figcraft_doc::proto::walk(&it.actions).filter_map(Action::dest).all(|d| doc.contains(d)), "{p} = {j}");
                    }
                    assert!(n.bound.values().all(|v| doc.variables.get(v).is_some()), "{p} = {j}");
                }
            }
        }
    }
}

#[test]
fn truncated_and_mutated_variables_never_panic() {
    let bytes = serde_json::to_vec(&combined()).unwrap();
    for cut in (0..bytes.len()).step_by(29) {
        if let Some(Ok((doc, _))) = try_import(&bytes[..cut], &no_images) {
            assert!(doc.validate().is_empty());
        }
    }
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..300 {
        let mut b = bytes.clone();
        for _ in 0..(next() % 8 + 1) {
            let i = (next() as usize) % b.len();
            b[i] = b"0123456789-.e{}[]\":,xNnul "[(next() as usize) % 26];
        }
        let _ = try_import(&b, &no_images);
    }
}

#[test]
fn alias_cycles_and_huge_variable_sets_are_bounded() {
    let mut vars = serde_json::Map::new();
    // A long alias chain and a cycle: imported, resolving stays bounded.
    for i in 0..2000 {
        let next = if i == 1999 { "VariableID:c:0".to_string() } else { format!("VariableID:c:{}", i + 1) };
        vars.insert(
            format!("VariableID:c:{i}"),
            json!({"id": format!("VariableID:c:{i}"), "name": format!("v{i}"), "variableCollectionId": THEME, "resolvedType": "FLOAT",
                   "valuesByMode": {"1:0": alias(&next)}}),
        );
    }
    let mut v = variables_json();
    v["meta"]["variables"] = Value::Object(vars);
    let (doc, _) = import_with(&file_json(), Some(&v), &no_images).unwrap();
    assert_eq!(doc.variables.variables.len(), 2000);
    assert!(doc.variables.resolve("VariableID:c:0", &Default::default()).is_none());
}
