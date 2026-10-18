//! Design tokens: the DTCG format's trees, types, aliases and colour spaces, and the resolver
//! module's sets, modifiers and resolution order.

use std::collections::BTreeMap;

use serde_json::json;

use crate::tokens::*;
use crate::{Color, Document};

fn themed() -> DesignTokens {
    DesignTokens {
        sets: vec![TokenSet {
            name: "core".into(),
            description: String::new(),
            tokens: json!({
                "color": {"$type": "color",
                    "blue": {"$value": {"colorSpace": "srgb", "components": [0, 0, 1]}},
                    "white": {"$value": {"colorSpace": "srgb", "components": [1, 1, 1], "hex": "#ffffff"}}},
                "space": {"$type": "dimension", "sm": {"$value": {"value": 0.5, "unit": "rem"}}}
            }),
        }],
        modifiers: vec![TokenModifier {
            name: "theme".into(),
            description: String::new(),
            contexts: vec![
                TokenContext { name: "light".into(), tokens: json!({"bg": {"$type": "color", "$value": "{color.white}"}}) },
                TokenContext { name: "dark".into(), tokens: json!({"bg": {"$type": "color", "$value": "{color.blue}"}}) },
            ],
            default: Some("light".into()),
        }],
        order: vec![OrderItem::Set("core".into()), OrderItem::Modifier("theme".into())],
        ..Default::default()
    }
}

fn input(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn group_types_are_inherited_and_aliases_resolve_per_context() {
    let t = themed();
    let light = t.resolve(&input(&[]));
    assert!(light.errors.is_empty(), "{:?}", light.errors);
    assert_eq!(light.get("space.sm").unwrap().kind, Some(TokenType::Dimension));
    assert_eq!(dimension_px(&light.get("space.sm").unwrap().value), Some(8.0));
    assert_eq!(color_of(&light.get("bg").unwrap().value), Some(Color::WHITE));
    assert_eq!(light.get("bg").unwrap().alias.as_deref(), Some("color.white"));
    let dark = t.resolve(&input(&[("theme", "dark")]));
    assert_eq!(color_of(&dark.get("bg").unwrap().value), Some(Color::new(0.0, 0.0, 1.0, 1.0)));
    // Inputs: strict checks for exports, lenient ones for the canvas.
    assert!(t.validate_input(json!({"theme": "dusk"}).as_object().unwrap()).is_err());
    assert!(t.validate_input(json!({"nope": "x"}).as_object().unwrap()).is_err());
    assert!(t.validate_input(json!({"theme": true}).as_object().unwrap()).is_err());
    assert_eq!(t.validate_input(json!({}).as_object().unwrap()).unwrap(), input(&[("theme", "light")]));
    assert_eq!(t.input_for(&input(&[("theme", "DARK"), ("nope", "x")])), input(&[("theme", "dark")]));
}

#[test]
fn later_sources_override_earlier_ones_and_tokens_replace_whole() {
    let mut t = themed();
    t.sets.push(TokenSet { name: "brand".into(), description: String::new(), tokens: json!({"color": {"blue": {"$value": "#ff0000"}}}) });
    t.order.push(OrderItem::Set("brand".into()));
    let r = t.resolve(&input(&[("theme", "dark")]));
    // The brand set comes last: its blue wins, also through the theme's alias; the group type stays.
    assert_eq!(color_of(&r.get("color.blue").unwrap().value), Some(Color::new(1.0, 0.0, 0.0, 1.0)));
    assert_eq!(color_of(&r.get("bg").unwrap().value), Some(Color::new(1.0, 0.0, 0.0, 1.0)));
    assert!(r.get("color.white").is_some(), "the rest of the group stays");
}

#[test]
fn bad_aliases_are_errors_not_failures() {
    let tree = json!({
        "a": {"$type": "number", "$value": "{b}"},
        "b": {"$type": "number", "$value": "{a}"},
        "c": {"$type": "number", "$value": "{missing.x}"},
        "d": {"$type": "color", "$value": "{n}"},
        "n": {"$type": "number", "$value": 3},
        "untyped": {"$value": 1},
        "e": {"$value": "{n}"},
        "p": {"$type": "number", "$value": {"$ref": "#/n/$value"}},
    });
    let r = resolve_tree(&tree);
    for bad in ["a", "b", "c", "d", "untyped"] {
        assert!(r.tokens[bad].error.is_some(), "{bad} should fail");
    }
    // Type through the alias, and JSON pointers.
    assert_eq!(r.get("e").unwrap().kind, Some(TokenType::Number));
    assert_eq!(r.get("p").unwrap().value, json!(3));
}

#[test]
fn group_extends_and_root_tokens() {
    let tree = json!({
        "base": {"$type": "dimension", "sm": {"$value": {"value": 4, "unit": "px"}}, "md": {"$value": {"value": 8, "unit": "px"}}},
        "dense": {"$extends": "{base}", "md": {"$value": {"value": 6, "unit": "px"}}},
        "accent": {"$type": "color", "$root": {"$value": "#00ff00"}, "light": {"$value": "#aaffaa"}},
    });
    let r = resolve_tree(&tree);
    assert_eq!(dimension_px(&r.get("dense.sm").unwrap().value), Some(4.0));
    assert_eq!(dimension_px(&r.get("dense.md").unwrap().value), Some(6.0));
    assert!(r.get("accent.$root").is_some());
}

#[test]
fn colour_spaces_convert_to_srgb() {
    let c = |space: &str, comps: [f64; 3]| color_of(&json!({"colorSpace": space, "components": comps})).unwrap().to_rgba8();
    assert_eq!(c("srgb", [1.0, 0.5, 0.0]), [255, 128, 0, 255]);
    assert_eq!(c("hsl", [120.0, 100.0, 50.0]), [0, 255, 0, 255]);
    assert_eq!(c("hwb", [0.0, 0.0, 0.0]), [255, 0, 0, 255]);
    assert_eq!(c("oklab", [1.0, 0.0, 0.0]), [255, 255, 255, 255]);
    assert_eq!(c("oklch", [0.627_955, 0.257_683, 29.233_885]), [255, 0, 0, 255]);
    assert_eq!(c("lab", [100.0, 0.0, 0.0]), [255, 255, 255, 255]);
    assert_eq!(c("display-p3", [1.0, 1.0, 1.0]), [255, 255, 255, 255]);
    assert_eq!(c("srgb-linear", [0.215_861, 0.215_861, 0.215_861]), [128, 128, 128, 255]);
    // `none` components, alpha, hex fallback for unknown spaces, junk.
    assert_eq!(color_of(&json!({"colorSpace": "srgb", "components": ["none", 0, 0], "alpha": 0.5})).unwrap().a, 0.5);
    assert_eq!(color_of(&json!({"colorSpace": "cmyk", "components": [1, 1, 1], "hex": "#336699"})).unwrap().to_css_hex().to_lowercase(), "#336699");
    assert!(color_of(&json!({"colorSpace": "srgb", "components": [1e308, -1e308]})).is_none());
    // Written back as sRGB with a hex.
    let v = color_value(Color::new(1.0, 0.0, 0.0, 0.5));
    assert_eq!(v["colorSpace"], "srgb");
    assert_eq!(v["alpha"], 0.5);
    assert_eq!(v["hex"], "#ff0000");
}

#[test]
fn values_are_checked_by_type() {
    use TokenType::*;
    for t in TokenType::ALL {
        assert!(check_value(t, &t.default_value()).is_ok(), "{t:?}'s default");
        assert_eq!(TokenType::parse(t.id()), Some(t));
    }
    assert!(check_value(Dimension, &json!({"value": 1, "unit": "pt"})).is_err());
    assert!(check_value(FontWeight, &json!("bold")).is_ok());
    assert!(check_value(FontWeight, &json!(1001)).is_err());
    assert!(check_value(CubicBezier, &json!([2, 0, 0, 1])).is_err());
    assert!(check_value(Typography, &json!({"fontFamily": "{font.body}"})).is_err(), "fields are required");
    assert!(check_value(Shadow, &json!([TokenType::Shadow.default_value(), "{shadow.sm}"])).is_ok());
    assert!(check_value(Color, &json!("{color.blue}")).is_ok(), "aliases are checked when resolved");
    assert_eq!(font_weight_of(&json!("semi-bold")), Some(600.0));
    assert_eq!(font_family_of(&json!(["Helvetica", "Arial"])).as_deref(), Some("Helvetica"));
    assert_eq!(duration_ms(&json!({"value": 0.2, "unit": "s"})), Some(200.0));
}

#[test]
fn editing_trees() {
    let mut tree = json!({});
    put_token(&mut tree, "color.brand.primary", make_token(Some(TokenType::Color), json!("#ff0000"), "Main")).unwrap();
    put_token(&mut tree, "color.link", make_token(None, json!("{color.brand.primary}"), "")).unwrap();
    assert!(token_at(&tree, "color.brand.primary").is_some());
    assert!(put_token(&mut tree, "color.brand.primary.x", json!({"$value": 1})).is_err(), "a token is no group");
    assert!(put_token(&mut tree, "color.brand", json!({"$value": 1})).is_err(), "a group is no token");
    assert!(put_token(&mut tree, "bad{name}", json!({"$value": 1})).is_err());
    assert!(put_token(&mut tree, "$type", json!({"$value": 1})).is_err());
    assert_eq!(rename_aliases(&mut tree, "color.brand", "color.main"), 1);
    assert_eq!(tree["color"]["link"]["$value"], "{color.main.primary}");
    remove_at(&mut tree, "color.brand.primary").unwrap();
    assert!(tree["color"].get("brand").is_none(), "empty groups go too");
    assert!(tree["color"].get("link").is_some());
}

#[test]
fn resolver_documents_round_trip() {
    let t = themed();
    let doc = to_resolver(&t, "Example");
    assert_eq!(doc["version"], RESOLVER_VERSION);
    assert_eq!(doc["resolutionOrder"], json!([{"$ref": "#/sets/core"}, {"$ref": "#/modifiers/theme"}]));
    assert_eq!(doc["modifiers"]["theme"]["default"], "light");
    assert!(doc["modifiers"]["theme"]["contexts"]["dark"].is_array());
    assert!(is_resolver(&doc));
    let (back, warn) = from_resolver(&doc, &|_| None).unwrap();
    assert!(warn.is_empty(), "{warn:?}");
    assert_eq!(back.sets, t.sets);
    assert_eq!(back.modifiers, t.modifiers);
    assert_eq!(back.order, t.order);
}

#[test]
fn resolver_documents_with_references_and_inline_entries() {
    let doc = json!({
        "version": "2025.10",
        "sets": {"base": {"sources": [{"$ref": "base.json"}, {"x": {"$type": "number", "$value": 2}}]}},
        "modifiers": {"size": {"contexts": {"sm": [{"$ref": "#/sets/base"}], "lg": [{"$ref": "missing.json"}]}}},
        "resolutionOrder": [
            {"$ref": "#/sets/base"},
            {"type": "set", "name": "Animation", "sources": [{"speed": {"$type": "duration", "$value": {"value": 1, "unit": "s"}}}]},
            {"type": "modifier", "name": "mode", "contexts": {"a": [], "b": []}, "default": "a"},
            {"$ref": "#/modifiers/size"},
            {"$ref": "#/modifiers/nope"},
        ],
    });
    let load = |f: &str| (f == "base.json").then(|| json!({"x": {"$type": "number", "$value": 1}, "y": {"$type": "number", "$value": 5}}));
    let (t, warn) = from_resolver(&doc, &load).unwrap();
    assert_eq!(t.sets.len(), 2);
    assert_eq!(t.modifiers.len(), 2);
    assert!(warn.iter().any(|w| w.contains("missing.json")) && warn.iter().any(|w| w.contains("nope")), "{warn:?}");
    let r = t.resolve(&input(&[("size", "sm")]));
    assert_eq!(r.get("x").unwrap().value, json!(2), "inline sources after the file");
    assert_eq!(r.get("speed").map(|t| t.kind), Some(Some(TokenType::Duration)));
    // A modifier without a default must be given a context.
    assert!(t.validate_input(json!({}).as_object().unwrap()).is_err());
}

#[test]
fn junk_never_panics() {
    let junk = [
        json!(null),
        json!([1, 2]),
        json!({"a": {"$value": null}}),
        json!({"a": {"$type": 5, "$value": {"$ref": 7}}}),
        json!({"a": {"$extends": "{a}", "b": {"$extends": "#/a"}}}),
        json!({"$value": 1}),
        json!({"a": {"$type": "color", "$value": {"$ref": "#/a"}}}),
        json!({"a": {"$type": "shadow", "$value": [{"$ref": "#/a/$value"}]}}),
    ];
    for j in &junk {
        let _ = resolve_tree(j);
        let _ = flatten(j);
        let _ = from_resolver(j, &|_| Some(json!({"$ref": "x.json"})));
        for t in TokenType::ALL {
            let _ = check_value(t, j);
            let _ = display(Some(t), j);
        }
    }
    // Deep nesting is cut off.
    let mut deep = json!({"$type": "number", "$value": 1});
    for _ in 0..200 {
        deep = json!({ "g": deep });
    }
    let _ = resolve_tree(&deep);
}

#[test]
fn the_document_keeps_tokens_and_layer_contexts() {
    let mut d = Document::new("t");
    d.tokens = themed();
    d.tokens.active.insert("theme".into(), "dark".into());
    let page = d.pages[0];
    assert_eq!(d.token_input_for(page), input(&[("theme", "dark")]));
    d.node_mut(page).unwrap().token_contexts.insert("theme".into(), "light".into());
    assert_eq!(d.token_input_for(page), input(&[("theme", "light")]));
    let back: Document = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(back.tokens, d.tokens);
    // Files without tokens don't write them.
    assert!(serde_json::to_value(Document::new("x")).unwrap().get("tokens").is_none());
}
