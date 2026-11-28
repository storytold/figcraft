//! `file.import` of a file whose style, variable and collection ids the document already uses for
//! other things: ids are numbered per file, so two files made apart share them. The imported
//! layers keep their own variables, modes and styles, and a file imported again still shares what
//! it brought the first time.

use base64::Engine as _;
use figcraft_doc::proto::Action;
use figcraft_doc::{Document, Node, NodeId, VariableValue};
use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::ex;
use crate::params::id_of;

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn var_id(s: &Session, name: &str) -> String {
    s.doc().unwrap().variables.get(name).unwrap().id.clone()
}

fn fill(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

/// The document's file: collection A (Light, Dark) with colours `a1`…`a4`, paint style `Host`
/// and a frame filled with `a1`.
fn host() -> (Session, NodeId) {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "A", "modes": ["Light", "Dark"]}));
    for i in 1..=4 {
        ex(&mut s, "variable.create", json!({"collection": "A", "name": format!("a{i}"), "type": "color", "value": format!("#A{i}0000")}));
    }
    ex(&mut s, "style.create", json!({"type": "paint", "name": "Host", "value": "#FF0000"}));
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "name": "Host frame"})));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "fill", "variable": "a1"}));
    (s, f)
}

/// Another file made apart: collection B (Light, Dark) with `b1` (green in each mode), `b2` (an
/// alias of `b1`) and `size`; paint style `Brand` bound to `b1`. Frame `Card` shows B in Dark, its
/// corners bound to `size`; in it `Swatch` is filled with `b2` and on click sets B to Dark and
/// `size` to `size + 1`, and `Styled` uses `Brand`.
fn other() -> Session {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "B", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "B", "name": "b1", "type": "color", "values": {"Light": "#00B000", "Dark": "#004000"}}));
    ex(&mut s, "variable.create", json!({"collection": "B", "name": "b2", "type": "color", "value": {"alias": "b1"}}));
    ex(&mut s, "variable.create", json!({"collection": "B", "name": "size", "type": "number", "value": 8}));
    let brand = ex(&mut s, "style.create", json!({"type": "paint", "name": "Brand", "value": "#00B000"}))["id"].clone();
    ex(&mut s, "style.bind", json!({"style": brand, "field": "fill", "variable": "b1"}));
    let card = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    ex(&mut s, "variable.setMode", json!({"ids": [card], "collection": "B", "mode": "Dark"}));
    ex(&mut s, "variable.bind", json!({"ids": [card], "field": "cornerRadius", "variable": "size"}));
    let swatch = idv(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40, "parent": card, "name": "Swatch"})));
    ex(&mut s, "variable.bind", json!({"ids": [swatch], "field": "fill", "variable": "b2"}));
    let actions = json!([
        {"type": "setVariableMode", "collection": "B", "mode": "Dark"},
        {"type": "setVariable", "variable": "size", "expression": "size + 1"}
    ]);
    ex(&mut s, "proto.interaction.add", json!({"id": swatch, "actions": actions}));
    let styled = idv(&ex(&mut s, "shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40, "parent": card, "name": "Styled"})));
    ex(&mut s, "style.apply", json!({"ids": [styled], "style": "Brand"}));
    s
}

fn import(s: &mut Session, file: &Session) -> Value {
    let bytes = figcraft_format::save_bytes(file.doc().unwrap(), None).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    ex(s, "file.import", json!({"data": data, "name": "other.figcraft"}))
}

/// The layers on the pages after the first `kept` pages.
fn layers_after(doc: &Document, kept: usize) -> Vec<&Node> {
    doc.pages[kept..].iter().flat_map(|p| doc.subtree(*p).into_iter().skip(1)).filter_map(|id| doc.node(id)).collect()
}

fn layer<'a>(doc: &'a Document, kept: usize, name: &str) -> &'a Node {
    layers_after(doc, kept).into_iter().find(|n| n.name == name).unwrap()
}

#[test]
fn imported_layers_keep_their_own_variables_and_styles_when_the_ids_are_taken() {
    let (mut s, frame) = host();
    let file = other();
    assert_eq!(var_id(&file, "b1"), var_id(&s, "a1"), "the ids must collide for this test");
    assert_eq!(file.doc().unwrap().styles[0].id, s.doc().unwrap().styles[0].id);
    let before = s.doc().unwrap().clone();
    import(&mut s, &file);
    let doc = s.doc().unwrap();
    assert_eq!(doc.variables.variables.len(), 4 + 3);
    assert_eq!(doc.variables.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(doc.styles.iter().map(|st| st.name.as_str()).collect::<Vec<_>>(), ["Host", "Brand"]);
    // The document's own are as they were.
    assert_eq!(doc.variables.variables[..4], before.variables.variables[..]);
    assert_eq!(doc.variables.collections[0], before.variables.collections[0]);
    assert_eq!(doc.styles[0], before.styles[0]);
    // Everything imported names B's variables, B and Brand.
    let b = doc.variables.collection("B").unwrap().id.clone();
    let imported = layers_after(doc, before.pages.len());
    assert!(imported.iter().any(|n| !n.bound.is_empty()));
    for n in imported {
        for v in n.bound.values() {
            assert_eq!(doc.variables.get_by_id(v).unwrap().collection, b, "{}: {v}", n.name);
        }
        assert!(n.var_modes.keys().all(|c| *c == b), "{}: {:?}", n.name, n.var_modes);
        if let Some(st) = &n.styles.fill {
            assert_eq!(doc.styles.iter().find(|x| x.id == *st).unwrap().name, "Brand");
        }
    }
    let brand = doc.styles.iter().find(|st| st.name == "Brand").unwrap();
    assert_eq!(brand.bound.values().map(|v| doc.variables.get_by_id(v).unwrap().name.as_str()).collect::<Vec<_>>(), ["b1"]);
    // Bindings applied again leave the imported layers alone.
    let swatch = layer(doc, before.pages.len(), "Swatch").id;
    let a2 = var_id(&s, "a2");
    ex(&mut s, "variable.set", json!({"variable": a2, "value": "#123456"}));
    assert_eq!(fill(&s, swatch), "004000");
    assert_eq!(fill(&s, frame), "A10000");
    assert!(s.doc().unwrap().validate().is_empty());
}

#[test]
fn an_alias_a_frame_mode_and_prototype_actions_follow_the_renamed_collection() {
    let (mut s, _) = host();
    let file = other();
    let kept = s.doc().unwrap().pages.len();
    import(&mut s, &file);
    let doc = s.doc().unwrap();
    let vars = &doc.variables;
    let b = vars.collection("B").unwrap();
    let in_b = |name: &str| vars.variables.iter().find(|v| v.name == name && v.collection == b.id).unwrap();
    let (b1, b2, size) = (in_b("b1"), in_b("b2"), in_b("size"));
    assert!(b2.values.values().all(|v| *v == VariableValue::Alias(b1.id.clone())), "{:?}", b2.values);
    // B's mode ids name B's new id, and its variables' values are by those.
    let modes: Vec<&str> = b.modes.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(modes, [format!("{}/mode1", b.id), format!("{}/mode2", b.id)]);
    assert!([b1, b2, size].iter().all(|v| v.values.keys().all(|m| modes.contains(&m.as_str()))));
    // The frame shows B in Dark: its mode is one of B's.
    let card = layer(doc, kept, "Card");
    let dark = b.find_mode("Dark").unwrap().id.clone();
    assert_eq!(card.var_modes.iter().collect::<Vec<_>>(), [(&b.id, &dark)]);
    assert_eq!(card.bound.get("cornerRadius"), Some(&size.id));
    let swatch = layer(doc, kept, "Swatch");
    assert_eq!(swatch.bound.get("fill"), Some(&b2.id));
    let shown = vars.resolve(&b2.id, &doc.modes_for(swatch.id));
    assert!(matches!(shown, Some(VariableValue::Color(c)) if c.to_hex() == "004000"), "{shown:?}");
    let actions = &swatch.interactions[0].actions;
    assert_eq!(actions[0], Action::SetVariableMode { collection: b.id.clone(), mode: dark });
    let Action::SetVariable { variable, expr: Some(e), .. } = &actions[1] else { panic!("{actions:?}") };
    assert_eq!((variable, e.variables()), (&size.id, vec![size.id.as_str()]));
}

#[test]
fn importing_the_same_file_again_shares_what_it_brought() {
    let file = other();
    let mut s = other();
    let before = s.doc().unwrap().clone();
    import(&mut s, &file);
    let doc = s.doc().unwrap();
    assert_eq!(doc.variables, before.variables);
    assert_eq!(doc.styles, before.styles);
    let swatch = layer(doc, before.pages.len(), "Swatch");
    assert_eq!(swatch.bound.get("fill"), Some(&var_id(&s, "b2")));
    // A file imported into an empty one, then again: one copy of its variables.
    let mut t = Session::new();
    import(&mut t, &file);
    import(&mut t, &file);
    let doc = t.doc().unwrap();
    assert_eq!((doc.variables.variables.len(), doc.variables.collections.len(), doc.styles.len()), (3, 1, 1));
    assert!(layers_after(doc, 1).iter().flat_map(|n| n.bound.values()).all(|v| doc.variables.get_by_id(v).is_some()));
}
