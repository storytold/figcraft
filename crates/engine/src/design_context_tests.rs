//! `inspect.designContext` against documents built with real commands.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

fn id(v: &Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn node(s: &mut Session, params: Value) -> u64 {
    id(&s.execute("node.create", &params).unwrap())
}

fn context(s: &mut Session, params: Value) -> Value {
    s.execute("inspect.designContext", &params).unwrap()
}

/// A hugging auto layout card holding a fixed title and a filling line.
fn card(s: &mut Session) -> (u64, u64, u64) {
    let f = node(s, json!({"type": "frame", "name": "Card", "x": 40, "y": 30, "width": 300, "height": 200, "props": {"fill": "#FFFFFF"}}));
    s.execute("node.set", &json!({"ids": [f], "props": {"layout": {"mode": "vertical"}, "gap": 12, "padding": [16, 16, 16, 16], "sizingV": "hug"}}))
        .unwrap();
    let title = node(s, json!({"type": "text", "parent": f, "name": "Title", "text": "Pro plan", "props": {"fontSize": 24}}));
    let line = node(s, json!({"type": "rectangle", "parent": f, "name": "Rule", "width": 100, "height": 2, "props": {"sizingH": "fill"}}));
    (f, title, line)
}

fn only(v: &Value) -> &Value {
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "{v}");
    &nodes[0]
}

#[test]
fn the_selection_is_what_is_read() {
    let mut s = session();
    let (f, title, _) = card(&mut s);
    let other = node(&mut s, json!({"type": "rectangle", "name": "Elsewhere", "width": 10, "height": 10}));
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let v = context(&mut s, json!({}));
    assert_eq!(v["target"]["source"], "selection");
    assert_eq!(only(&v)["id"], f);
    assert_eq!(only(&v)["children"][0]["id"], title);
    assert_eq!(v["target"]["layers"], 3);
    assert!(!v.to_string().contains("Elsewhere"), "the rest of the page stays out");
    let _ = other;
}

#[test]
fn several_selected_layers_each_come_back() {
    let mut s = session();
    let a = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10}));
    let b = node(&mut s, json!({"type": "ellipse", "width": 10, "height": 10}));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(context(&mut s, json!({}))["nodes"].as_array().unwrap().len(), 2);
}

#[test]
fn ids_and_scopes_choose_the_target() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    let other = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10}));
    s.execute("select.set", &json!({"ids": [other]})).unwrap();
    assert_eq!(context(&mut s, json!({"ids": [f]}))["target"]["source"], "ids");
    assert_eq!(context(&mut s, json!({"scope": "page"}))["nodes"].as_array().unwrap().len(), 2);
    s.execute("page.add", &json!({"name": "Second"})).unwrap();
    assert_eq!(context(&mut s, json!({"scope": "document"}))["target"]["source"], "document");
    assert!(s.execute("inspect.designContext", &json!({"ids": [999_999]})).is_err());
    assert!(s.execute("inspect.designContext", &json!({"scope": "everything"})).is_err());
}

#[test]
fn nothing_selected_reads_the_page() {
    let mut s = session();
    card(&mut s);
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(context(&mut s, json!({}))["target"]["source"], "page");
}

#[test]
fn what_layout_decides_is_derived_and_what_the_designer_set_is_explicit() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    let v = context(&mut s, json!({"ids": [f]}));
    let frame = only(&v);
    assert_eq!(frame["explicit"]["width"], 300.0, "a fixed width is the designer's");
    assert_eq!(frame["explicit"]["x"], 40.0);
    assert!(frame["explicit"].get("height").is_none(), "a hug height isn't");
    assert!(frame["derived"]["height"].is_number());
    assert!(frame["derived"]["absoluteBounds"].is_object());
    assert_eq!(frame["explicit"]["layout"]["gap"], 12.0);
    let (title, rule) = (&frame["children"][0], &frame["children"][1]);
    assert!(title["derived"].get("x").is_some(), "its place comes from the flow: {title}");
    assert!(title["explicit"].get("x").is_none());
    assert!(title["derived"].get("width").is_some(), "auto-width text: {title}");
    assert!(rule["derived"].get("width").is_some(), "fill: {rule}");
    assert_eq!(rule["explicit"]["height"], 2.0);
}

#[test]
fn the_values_are_the_documents() {
    let mut s = session();
    let (f, title, _) = card(&mut s);
    let v = context(&mut s, json!({"ids": [f]}));
    let got = &only(&v)["children"][0]["explicit"];
    let inspected = s.execute("node.get", &json!({"id": title})).unwrap();
    for key in ["characters", "fontFamily", "fontSize", "lineHeight", "letterSpacing", "textAlign", "fontStyle"] {
        assert_eq!(got[key], inspected[key], "{key}");
    }
    assert_eq!(got["characters"], "Pro plan");
}

#[test]
fn tokens_are_joined_with_every_modes_value() {
    let mut s = session();
    let c = s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    let collection = c["id"].clone();
    let var = s
        .execute(
            "variable.create",
            &json!({"collection": collection, "name": "surface", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#101010"}}),
        )
        .unwrap();
    let var_id = var["id"].as_str().unwrap().to_string();
    s.execute("variable.update", &json!({"variable": var_id, "codeSyntax": {"WEB": "var(--surface)"}})).unwrap();
    let r = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10, "props": {"fill": {"$var": var_id}}}));
    let v = context(&mut s, json!({"ids": [r]}));
    let bound = only(&v)["tokens"].as_object().unwrap().values().next().unwrap().clone();
    assert_eq!((bound["variable"].as_str(), bound["name"].as_str()), (Some(var_id.as_str()), Some("surface")));
    let entry = &v["variables"][var_id.as_str()];
    assert_eq!((entry["name"].as_str(), entry["collection"].as_str()), (Some("surface"), Some("Theme")));
    assert_eq!(entry["codeSyntax"]["WEB"], "var(--surface)");
    assert_eq!(entry["modes"][0]["mode"], "Light");
    assert_eq!(entry["modes"][1]["resolved"], "#101010");
}

#[test]
fn a_shared_style_is_named_and_defined_once() {
    let mut s = session();
    let r = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10, "props": {"fill": "#FF0000"}}));
    let st = s.execute("style.create", &json!({"type": "paint", "name": "Brand/Red", "from": r})).unwrap();
    s.execute("style.apply", &json!({"style": st["id"], "ids": [r]})).unwrap();
    let v = context(&mut s, json!({"ids": [r]}));
    assert_eq!(only(&v)["tokens"]["style.fill"]["name"], "Brand/Red");
    assert_eq!(v["styles"].as_object().unwrap().len(), 1);
    assert_eq!(v["styles"][st["id"].as_str().unwrap()]["name"], "Brand/Red");
}

#[test]
fn a_connected_instance_gives_its_code_not_its_layers() {
    let mut s = session();
    let b = node(&mut s, json!({"type": "frame", "name": "Button", "width": 80, "height": 32}));
    node(&mut s, json!({"type": "text", "parent": b, "text": "Buy"}));
    let main = id(&s.execute("component.create", &json!({"ids": [b]})).unwrap());
    let instance = id(&s.execute("instance.create", &json!({"component": main})).unwrap());
    s.execute("codeConnect.set", &json!({"id": main, "language": "react", "importPath": "@/ui/button", "name": "Button", "template": "<Button />"}))
        .unwrap();
    let v = context(&mut s, json!({"ids": [instance]}));
    let n = only(&v);
    assert!(n["code"][0]["code"].as_str().unwrap().contains("<Button />"), "{n}");
    assert_eq!(n["code"][0]["importPath"], "@/ui/button");
    assert!(n.get("children").is_none(), "{n}");
    assert_eq!(n["childCount"], 1);
    assert_eq!(v["components"].as_object().unwrap().len(), 1);
    assert_eq!(v["components"][main.to_string()]["codeLinks"][0]["name"], "Button");
}

#[test]
fn an_unconnected_instance_shows_its_layers_and_properties() {
    let mut s = session();
    let b = node(&mut s, json!({"type": "frame", "name": "Tag", "width": 80, "height": 32}));
    node(&mut s, json!({"type": "text", "parent": b, "text": "New"}));
    let main = id(&s.execute("component.create", &json!({"ids": [b]})).unwrap());
    let inner = id(&s.execute("instance.create", &json!({"component": main})).unwrap());
    let outer = node(&mut s, json!({"type": "frame", "name": "Row", "width": 200, "height": 60}));
    s.execute("node.reparent", &json!({"ids": [inner], "parent": outer})).unwrap();
    let v = context(&mut s, json!({"ids": [outer]}));
    let i = &only(&v)["children"][0];
    assert_eq!(i["explicit"]["mainComponentName"], "Tag");
    assert_eq!(i["children"].as_array().unwrap().len(), 1, "nested layers: {i}");
    assert!(v["components"].get(main.to_string()).is_some());
}

#[test]
fn what_css_cant_say_is_reported_and_kept() {
    let mut s = session();
    let r = node(
        &mut s,
        json!({"type": "rectangle", "width": 50, "height": 50, "props": {"cornerRadius": 12, "cornerSmoothing": 0.6, "strokes": ["#000000"], "strokeWeight": 2, "strokeAlign": "outside"}}),
    );
    let v = context(&mut s, json!({"ids": [r]}));
    let n = only(&v);
    let said: Vec<&str> = n["unsupported"].as_array().unwrap().iter().map(|u| u["property"].as_str().unwrap()).collect();
    assert_eq!(said, ["cornerSmoothing", "strokeAlign"], "{n}");
    assert_eq!(n["explicit"]["cornerSmoothing"], s.execute("node.get", &json!({"id": r})).unwrap()["cornerSmoothing"]);
}

#[test]
fn a_plain_layer_claims_no_limits() {
    let mut s = session();
    let r = node(&mut s, json!({"type": "rectangle", "width": 50, "height": 50, "props": {"fill": "#123456", "cornerRadius": 4}}));
    let v = context(&mut s, json!({"ids": [r]}));
    assert!(only(&v).get("unsupported").is_none());
}

#[test]
fn hidden_layers_stay_in_the_tree_marked() {
    let mut s = session();
    let (f, title, _) = card(&mut s);
    s.execute("node.set", &json!({"ids": [title], "props": {"visible": false}})).unwrap();
    let v = context(&mut s, json!({"ids": [f]}));
    assert_eq!(only(&v)["children"][0]["explicit"]["visible"], false);
}

#[test]
fn what_the_design_doesnt_say_is_missing_not_guessed() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    let v = context(&mut s, json!({"ids": [f]}));
    assert!(v["missing"].as_array().unwrap().iter().any(|m| m["what"] == "responsive behaviour"));
    assert!(!v.to_string().contains("@media"));
}

#[test]
fn annotations_come_with_their_layer() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    s.execute("annotation.add", &json!({"id": f, "label": "Cards stack on mobile"})).unwrap();
    let v = context(&mut s, json!({"ids": [f]}));
    assert_eq!(only(&v)["explicit"]["annotations"][0]["label"], "Cards stack on mobile");
}

#[test]
fn deep_trees_stop_at_depth_and_say_so() {
    let mut s = session();
    let mut parent = node(&mut s, json!({"type": "frame", "width": 100, "height": 100}));
    let root = parent;
    for _ in 0..6 {
        parent = node(&mut s, json!({"type": "frame", "parent": parent, "width": 50, "height": 50}));
    }
    let v = context(&mut s, json!({"ids": [root], "depth": 2}));
    assert_eq!(v["target"]["layers"], 3);
    assert_eq!(v["truncated"]["depth"], true);
    assert_eq!(only(&v)["children"][0]["children"][0]["childCount"], 1);
    assert!(context(&mut s, json!({"ids": [root]})).get("truncated").is_none());
}

#[test]
fn a_large_selection_keeps_to_the_cap_and_counts_the_rest() {
    let mut s = session();
    let root = node(&mut s, json!({"type": "frame", "width": 1000, "height": 1000}));
    for _ in 0..50 {
        node(&mut s, json!({"type": "rectangle", "parent": root, "width": 5, "height": 5}));
    }
    let v = context(&mut s, json!({"ids": [root], "maxNodes": 10}));
    assert_eq!(v["target"]["layers"], 10);
    assert_eq!(v["truncated"]["layersLeftOut"], 41);
    assert_eq!(only(&v)["childCount"], 50, "it says how many there are");
}

#[test]
fn reading_changes_nothing() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    let before = s.execute("node.get", &json!({"id": f})).unwrap();
    let undo = s.undo_depth();
    context(&mut s, json!({"ids": [f]}));
    assert_eq!(s.execute("node.get", &json!({"id": f})).unwrap(), before);
    assert_eq!(s.undo_depth(), undo);
}

#[test]
fn junk_parameters_are_errors_not_panics() {
    let mut s = session();
    card(&mut s);
    for p in [json!({"depth": "x"}), json!({"maxNodes": -1}), json!({"maxNodes": "many"}), json!({"depth": {}})] {
        assert!(s.execute("inspect.designContext", &p).is_err(), "{p}");
    }
    assert!(s.execute("inspect.designContext", &json!({"ids": []})).is_ok());
    assert!(s.execute("inspect.designContext", &json!({"maxNodes": 0})).is_ok());
}

/// A colour variable in collection Theme (Light, Dark): (collection id, variable id).
fn variable(s: &mut Session, name: &str, light: &str, dark: &str) -> (Value, String) {
    let c = s.execute("variable.createCollection", &json!({"name": format!("{name} theme"), "modes": ["Light", "Dark"]})).unwrap()["id"].clone();
    let v = s.execute("variable.create", &json!({"collection": c, "name": name, "type": "color", "values": {"Light": light, "Dark": dark}})).unwrap();
    (c, v["id"].as_str().unwrap().to_string())
}

#[test]
fn a_variable_an_alias_reads_is_joined_too() {
    let mut s = session();
    let (c, brand) = variable(&mut s, "brand", "#3B5BDB", "#748FFC");
    let button = s
        .execute("variable.create", &json!({"collection": c, "name": "button", "type": "color", "values": {"Light": "#000000", "Dark": "#000000"}}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    for mode in ["Light", "Dark"] {
        s.execute("variable.set", &json!({"variable": button, "mode": mode, "value": {"alias": brand}})).unwrap();
    }
    let r = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10, "props": {"fill": {"$var": button}}}));
    let v = context(&mut s, json!({"ids": [r]}));
    assert!(v["variables"].get(&button).is_some());
    assert_eq!(v["variables"][&brand]["name"], "brand", "the aliased variable is defined as well: {}", v["variables"]);
}

#[test]
fn a_variable_a_shared_style_binds_is_joined() {
    let mut s = session();
    let (_, brand) = variable(&mut s, "brand", "#3B5BDB", "#748FFC");
    let r = node(&mut s, json!({"type": "rectangle", "width": 10, "height": 10, "props": {"fill": "#FF0000"}}));
    let st = s.execute("style.create", &json!({"type": "paint", "name": "Brand/Fill", "from": r})).unwrap()["id"].clone();
    s.execute("style.bind", &json!({"style": st, "field": "fill", "variable": brand})).unwrap();
    s.execute("style.apply", &json!({"style": st, "ids": [r]})).unwrap();
    let v = context(&mut s, json!({"ids": [r]}));
    assert_eq!(only(&v)["tokens"]["style.fill"]["name"], "Brand/Fill");
    assert_eq!(v["variables"][&brand]["name"], "brand", "the style's variable is defined: {}", v["variables"]);
}

#[test]
fn every_returned_text_layer_has_its_font_listed() {
    let mut s = session();
    let title = node(&mut s, json!({"type": "text", "text": "Title", "props": {"fontFamily": "Serif Display Test"}}));
    let busy = node(&mut s, json!({"type": "frame", "x": 300, "width": 200, "height": 200}));
    for _ in 0..12 {
        node(&mut s, json!({"type": "rectangle", "parent": busy, "width": 4, "height": 4}));
    }
    let v = context(&mut s, json!({"ids": [title, busy], "maxNodes": 2}));
    assert_eq!(v["target"]["layers"], 2);
    let families: Vec<&str> = v["fonts"].as_array().unwrap().iter().map(|f| f["family"].as_str().unwrap()).collect();
    assert_eq!(families, ["Serif Display Test"], "{}", v["fonts"]);
}

#[test]
fn a_malformed_target_is_refused_rather_than_read_as_the_selection() {
    let mut s = session();
    let (f, _, _) = card(&mut s);
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    for p in [json!({"id": "not-an-id"}), json!({"id": true}), json!({"ids": [f, "nope"]}), json!({"ids": "1"})] {
        assert!(s.execute("inspect.designContext", &p).is_err(), "{p}");
    }
    assert_eq!(context(&mut s, json!({"ids": []}))["target"]["layers"], 0, "an empty list reads nothing");
}

#[test]
fn a_font_stored_with_outer_spaces_isnt_taken_for_installed() {
    let mut s = session();
    let t = node(&mut s, json!({"type": "text", "text": "Hi", "props": {"fontFamily": " Missing Display Test "}}));
    let v = context(&mut s, json!({"ids": [t]}));
    let font = &v["fonts"][0];
    assert_eq!(font["family"], " Missing Display Test ", "the spelling stays as stored");
    assert_eq!(font["installed"], false, "{font}");
}

#[test]
fn assets_follow_the_layers_returned_and_say_what_was_left_out() {
    let mut s = session();
    let frame = node(&mut s, json!({"type": "frame", "width": 400, "height": 400}));
    for seed in 0..20u8 {
        let img = image::RgbImage::from_fn(4, 4, |x, y| image::Rgb([x as u8, y as u8, seed]));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        s.execute("image.place", &json!({"data": data, "parent": frame})).unwrap();
    }
    let all = context(&mut s, json!({"ids": [frame]}));
    assert_eq!(all["assets"].as_array().unwrap().len(), 20);
    let few = context(&mut s, json!({"ids": [frame], "maxNodes": 5}));
    let shown = few["assets"].as_array().unwrap().len();
    assert_eq!(shown, 4, "only the images among the 5 layers returned");
    assert_eq!(few["truncated"]["assetsLeftOut"], 16);
}

#[test]
fn a_join_stopped_at_its_budget_says_so() {
    let mut s = session();
    let (c, first) = variable(&mut s, "v0", "#000000", "#000000");
    let mut prev = first.clone();
    for k in 1..6 {
        let next = s
            .execute(
                "variable.create",
                &json!({"collection": c, "name": format!("v{k}"), "type": "color", "values": {"Light": "#111111", "Dark": "#111111"}}),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        for mode in ["Light", "Dark"] {
            s.execute("variable.set", &json!({"variable": prev, "mode": mode, "value": {"alias": next}})).unwrap();
        }
        prev = next;
    }
    let start = std::collections::BTreeSet::from([first]);
    let none = std::collections::BTreeSet::new();
    let (all, complete) = crate::design_context::referenced(s.doc().unwrap(), &start, &none, 100);
    assert_eq!((all.len(), complete), (6, true));
    let (some, complete) = crate::design_context::referenced(s.doc().unwrap(), &start, &none, 3);
    assert_eq!((some.len(), complete), (3, false), "{some:?}");
}

#[test]
fn code_links_keep_their_value_names_and_carry_source_and_notes() {
    let mut s = session();
    let variants: Vec<_> = ["small", "extra_large"]
        .iter()
        .map(|v| {
            let f = node(&mut s, json!({"type": "frame", "name": format!("Size={v}"), "width": 80, "height": 32}));
            id(&s.execute("component.create", &json!({"ids": [f]})).unwrap())
        })
        .collect();
    let set = id(&s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    s.execute(
        "codeConnect.set",
        &json!({"id": set, "name": "Button", "template": "<Button size={Size} />", "source": "src/button.tsx",
            "notes": "Use for main actions", "values": {"Size": {"extra_large": "xl"}}}),
    )
    .unwrap();
    let instance = id(&s.execute("instance.create", &json!({"component": variants[1]})).unwrap());
    let v = context(&mut s, json!({"ids": [instance]}));
    let link = &v["components"][set.to_string()]["codeLinks"][0];
    assert_eq!(link["values"]["Size"]["extra_large"], "xl", "a design value keeps its name: {link}");
    let code = &only(&v)["code"][0];
    assert_eq!((code["source"].as_str(), code["notes"].as_str()), (Some("src/button.tsx"), Some("Use for main actions")), "{code}");
    assert!(code["code"].as_str().unwrap().contains("xl"), "{code}");
}

#[test]
fn stated_screen_sizes_come_with_the_context() {
    let mut s = session();
    let f = node(&mut s, json!({"type": "frame", "name": "Home", "width": 1440, "height": 900}));
    let unmarked = context(&mut s, json!({"ids": [f]}));
    assert_eq!(unmarked["responsive"]["explicit"], false, "{unmarked}");
    let says = |v: &Value| v["missing"].as_array().unwrap().iter().any(|m| m["what"] == "responsive behaviour");
    assert!(says(&unmarked), "nothing stated: the gap is named");
    s.execute("responsive.set", &json!({"ids": [f], "viewport": "desktop"})).unwrap();
    s.execute("responsive.rule", &json!({"id": f, "text": "Sidebar becomes a drawer below 768 px"})).unwrap();
    let marked = context(&mut s, json!({"ids": [f]}));
    assert_eq!(marked["responsive"]["explicit"], true, "{marked}");
    assert_eq!(marked["responsive"]["rules"][0]["text"], "Sidebar becomes a drawer below 768 px");
    assert!(!says(&marked), "{marked}");
}
