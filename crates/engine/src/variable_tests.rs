//! Variable bindings through the session: binding, value edits and modes reach the layers.

use serde_json::json;

use crate::Session;
use crate::params::id_of;

fn stale_bound_color_document() -> (figcraft_doc::Document, figcraft_doc::NodeId) {
    let mut s = Session::new();
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": 4000, "y": 4000, "width": 23, "height": 17})).unwrap()["id"]).unwrap();
    let page = s.doc().unwrap().first_page().unwrap();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#FFFFFF"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "size", "type": "number", "value": 23})).unwrap();
    s.execute("variable.set", &json!({"variable": "surface", "mode": "Dark", "value": "#202124"})).unwrap();
    s.execute("variable.set", &json!({"variable": "size", "mode": "Dark", "value": 99})).unwrap();
    s.execute("variable.bind", &json!({"ids": [id], "field": "fill", "variable": "surface"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [id], "field": "width", "variable": "size"})).unwrap();
    s.execute("variable.setMode", &json!({"collection": "Theme", "mode": "Dark"})).unwrap();
    let mut doc = s.doc().unwrap().clone();
    let node = doc.get_mut(id).unwrap();
    node.name = "load-binding-sentinel".into();
    node.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::WHITE)];
    node.width = 5.0;
    node.source = Some(page);
    node.overrides.insert("sentinel".into());
    doc.take_dirty();
    (doc, id)
}

fn assert_dark_bound_color(doc: &figcraft_doc::Document, transform: figcraft_doc::geom::Affine) -> figcraft_doc::NodeId {
    let node = doc.nodes().find(|n| n.name == "load-binding-sentinel").unwrap();
    assert_eq!(node.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()), Some("202124".into()));
    assert_eq!((node.transform, node.width, node.height), (transform, 5.0, 17.0));
    assert!(node.bound.contains_key("fills/0/color") || node.bound.contains_key("fill"));
    node.id
}

#[test]
fn initial_color_bindings_settle_in_constructor_and_loaders() {
    let (stale, _id) = stale_bound_color_document();
    let name_transform = stale.nodes().find(|n| n.name == "load-binding-sentinel").unwrap().transform;

    let mut state = Session::with_document(stale.clone());
    let id = assert_dark_bound_color(state.doc().unwrap(), name_transform);
    assert_eq!(state.doc().unwrap().node(id).unwrap().source, stale.first_page());
    assert!(state.doc().unwrap().node(id).unwrap().overrides.contains("sentinel"));
    assert!(!state.ds().unwrap().is_dirty());
    assert!(state.doc().unwrap().dirty().is_empty());
    assert_eq!(state.undo_depth(), (0, 0));
    state.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
    assert_dark_bound_color(state.doc().unwrap(), name_transform);

    let native = figcraft_format::save_bytes(&stale, None).unwrap();
    let (loaded, _) = crate::cmd::file::load_bytes(&native, "bindings.figcraft").unwrap();
    // A native file settles when it opens as a document, not in the loader.
    let opened = Session::with_document(loaded);
    let loaded = opened.doc().unwrap();
    let loaded_id = assert_dark_bound_color(loaded, name_transform);
    assert_eq!(loaded_id, id);
    assert_eq!(loaded.node(id).unwrap().source, stale.first_page());
    assert!(loaded.node(id).unwrap().overrides.contains("sentinel"));
    assert!(loaded.dirty().is_empty());

    let (fig, _) = figcraft_format::fig::export(&stale, None).unwrap();
    let (loaded, _) = crate::cmd::file::load_bytes(&fig, "bindings.fig").unwrap();
    assert_dark_bound_color(&loaded, name_transform);
}

#[test]
fn initial_paint_settlement_preserves_existing_dirty_nodes() {
    let (mut doc, _) = stale_bound_color_document();
    let page = doc.first_page().unwrap();
    doc.mark_dirty([page]);

    crate::cmd::styles::apply_paint_bindings_on_load(&mut doc);

    assert_eq!(doc.dirty().iter().copied().collect::<Vec<_>>(), [page]);
}

#[test]
fn bound_layers_follow_variable_values_and_unbind() {
    let mut s = Session::new();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"]).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Tokens", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "size/lg", "type": "number", "value": 48})).unwrap();
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "brand", "type": "color", "value": "#0D99FF"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "width", "variable": "size/lg"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "fill", "variable": "brand"})).unwrap();
    let node = |s: &Session| s.doc().unwrap().node(r).unwrap().clone();
    assert_eq!(node(&s).width, 48.0);
    assert_eq!(node(&s).fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()), Some("0D99FF".into()));
    // Editing the variable updates the bound layer.
    s.execute("variable.set", &json!({"variable": "size/lg", "value": 64})).unwrap();
    assert_eq!(node(&s).width, 64.0);
    // A mode with another value applies through the page.
    s.execute("variable.set", &json!({"variable": "size/lg", "mode": "Dark", "value": 80})).unwrap();
    s.execute("variable.setMode", &json!({"collection": "Tokens", "mode": "Dark"})).unwrap();
    assert_eq!(node(&s).width, 80.0);
    // Unbinding keeps the value and stops following.
    s.execute("variable.bind", &json!({"ids": [r], "field": "width", "variable": null})).unwrap();
    s.execute("variable.set", &json!({"variable": "size/lg", "mode": "Dark", "value": 99})).unwrap();
    assert_eq!(node(&s).width, 80.0);
    assert!(s.execute("variable.bind", &json!({"ids": [r], "field": "width", "variable": "nope"})).is_err());
}

#[test]
fn opacity_padding_text_and_visibility_follow_their_variables() {
    let mut s = Session::new();
    let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"]).unwrap();
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"text": "Hello", "x": 200, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Tokens"})).unwrap();
    for (name, kind, value) in
        [("fade", "number", json!(40)), ("pad", "number", json!(12)), ("label", "string", json!("Sign in")), ("shown", "boolean", json!(false))]
    {
        s.execute("variable.create", &json!({"collection": "Tokens", "name": name, "type": kind, "value": value})).unwrap();
    }
    let bind = |s: &mut Session, id, field: &str, var: &str| {
        s.execute("variable.bind", &json!({"ids": [id], "field": field, "variable": var})).unwrap();
    };
    bind(&mut s, f, "opacity", "fade");
    bind(&mut s, f, "paddingHorizontal", "pad");
    bind(&mut s, t, "characters", "label");
    bind(&mut s, t, "visible", "shown");
    let doc = s.doc().unwrap();
    let frame = doc.node(f).unwrap();
    // Opacity variables are percentages.
    assert!((frame.opacity - 0.4).abs() < 1e-6, "{}", frame.opacity);
    let pad = frame.layout.as_ref().unwrap().padding;
    assert_eq!((pad[1], pad[3]), (12.0, 12.0));
    let text = doc.node(t).unwrap();
    assert_eq!(text.kind.text().map(|x| x.characters.as_str()), Some("Sign in"));
    assert!(!text.visible);
    // Out-of-range percentages clamp; booleans flip visibility back.
    s.execute("variable.set", &json!({"variable": "fade", "value": 250})).unwrap();
    s.execute("variable.set", &json!({"variable": "shown", "value": true})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(f).unwrap().opacity, 1.0);
    assert!(doc.node(t).unwrap().visible);
}

#[test]
fn line_height_and_letter_spacing_follow_number_variables_in_pixels() {
    let mut s = Session::new();
    let t = id_of(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Type"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Type", "name": "leading", "type": "number", "value": 24})).unwrap();
    s.execute("variable.create", &json!({"collection": "Type", "name": "tracking", "type": "number", "value": 2})).unwrap();
    s.execute("variable.bind", &json!({"ids": [t], "field": "lineHeight", "variable": "leading"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [t], "field": "letterSpacing", "variable": "tracking"})).unwrap();
    let style = |s: &Session| s.doc().unwrap().node(t).unwrap().kind.text().unwrap().style.clone();
    assert_eq!(style(&s).line_height, figcraft_doc::LineHeight::Pixels(24.0));
    assert_eq!(style(&s).letter_spacing, figcraft_doc::LetterSpacing::Pixels(2.0));
    s.execute("variable.set", &json!({"variable": "leading", "value": 30})).unwrap();
    assert_eq!(style(&s).line_height, figcraft_doc::LineHeight::Pixels(30.0));
}

#[test]
fn effects_follow_their_variables() {
    let mut s = Session::new();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"]).unwrap();
    s.execute("effect.add", &json!({"ids": [r], "type": "dropShadow"})).unwrap();
    s.execute("effect.add", &json!({"ids": [r], "type": "layerBlur"})).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Fx"})).unwrap();
    for (name, kind, value) in [("y", "number", json!(6)), ("blur", "number", json!(12)), ("shadow", "color", json!("#FF000080"))] {
        s.execute("variable.create", &json!({"collection": "Fx", "name": name, "type": kind, "value": value})).unwrap();
    }
    let bind = |s: &mut Session, field: &str, var: &str| {
        s.execute("variable.bind", &json!({"ids": [r], "field": field, "variable": var})).unwrap();
    };
    bind(&mut s, "effects/0/offsetY", "y");
    bind(&mut s, "effects/0/color", "shadow");
    bind(&mut s, "effects/1/radius", "blur");
    bind(&mut s, "effects/9/radius", "blur");
    let fx = |s: &Session| s.doc().unwrap().node(r).unwrap().effects.clone();
    match &fx(&s)[0].kind {
        figcraft_doc::EffectKind::DropShadow(sh) => {
            assert_eq!(sh.offset.1, 6.0);
            assert_eq!(sh.color.to_hex(), "FF0000");
            assert!((sh.color.a - 0.5).abs() < 0.01);
        }
        k => panic!("{k:?}"),
    }
    assert!(matches!(fx(&s)[1].kind, figcraft_doc::EffectKind::LayerBlur(ref b) if b.radius == 12.0));
    s.execute("variable.set", &json!({"variable": "blur", "value": -3})).unwrap();
    assert!(matches!(fx(&s)[1].kind, figcraft_doc::EffectKind::LayerBlur(ref b) if b.radius == 0.0), "clamped");
    // Pickers offer effect-scoped variables for these fields.
    let doc = s.doc().unwrap();
    assert_eq!(crate::cmd::variables::field_scope(doc, r, "effects/0/color"), Some("EFFECT_COLOR"));
    assert_eq!(crate::cmd::variables::field_scope(doc, r, "effects/0/spread"), Some("EFFECT_FLOAT"));
}
