//! Top-level paint variable props share `variable.bind` behaviour in one transaction (#1000).

use figcraft_color::Color;
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::{Session, params::id_of};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn setup() -> (Session, String) {
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    let v = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "palette/surface", "type": "color", "value": "#0D99FF80"}));
    (s, v["id"].as_str().unwrap().to_string())
}

fn create(s: &mut Session, props: Value) -> NodeId {
    id_of(&run(s, "node.create", json!({"type": "rectangle", "width": 10, "height": 10, "props": props}))["id"]).unwrap()
}

fn color(s: &Session, id: NodeId, field: &str) -> Color {
    let n = s.doc().unwrap().node(id).unwrap();
    let paints = if field == "fill" { &n.fills } else { &n.strokes };
    paints[0].solid_color().unwrap()
}

fn assert_bound(s: &Session, id: NodeId, field: &str, variable: &str) {
    assert_eq!(s.doc().unwrap().node(id).unwrap().bound.get(field).map(String::as_str), Some(variable));
    assert_eq!(color(s, id, field).to_hex(), "0D99FF");
    assert!((color(s, id, field).a - 128.0 / 255.0).abs() < 1e-6);
}

#[test]
fn create_and_set_take_variable_ids_names_and_collection_names_for_both_paints() {
    let (mut s, vid) = setup();
    for field in ["fill", "stroke"] {
        for name in [&vid, "palette/surface", "Theme/palette/surface"] {
            let id = create(&mut s, json!({field: {"$var": name}}));
            assert_bound(&s, id, field, &vid);
            let id = create(&mut s, json!({"fill": "none", "stroke": "none"}));
            run(&mut s, "node.set", json!({"id": id, "props": {field: {"$var": name}}}));
            assert_bound(&s, id, field, &vid);
        }
    }
}

#[test]
fn ambiguous_and_unknown_variable_names_are_errors_but_ids_still_work() {
    let (mut s, vid) = setup();
    run(&mut s, "variable.createCollection", json!({"name": "Other"}));
    run(&mut s, "variable.create", json!({"collection": "Other", "name": "palette/surface", "type": "color", "value": "#FF0000"}));
    let e = s.execute("node.create", &json!({"type": "rectangle", "props": {"fill": {"$var": "palette/surface"}}})).unwrap_err().to_string();
    assert!(e.contains("ambiguous variable `palette/surface`"), "{e}");
    assert!(e.contains("Other/palette/surface") && e.contains("Theme/palette/surface"), "{e}");
    let id = create(&mut s, json!({"fill": {"$var": vid}}));
    assert_bound(&s, id, "fill", &vid);
    let id = create(&mut s, json!({"fill": {"$var": "Theme/palette/surface"}}));
    assert_bound(&s, id, "fill", &vid);
    let e = s.execute("node.set", &json!({"id": id, "props": {"fill": {"$var": "missing/surface"}}})).unwrap_err().to_string();
    assert!(e.contains("no such variable `missing/surface`"), "{e}");
    // The existing command still uses the first match, not the new strict resolver.
    run(&mut s, "variable.bind", json!({"id": id, "field": "fill", "variable": "palette/surface"}));
    assert_bound(&s, id, "fill", &vid);
}

#[test]
fn variable_ids_choose_between_variables_with_the_same_name() {
    let (mut s, first) = setup();
    run(&mut s, "variable.createCollection", json!({"name": "Other"}));
    let second = run(&mut s, "variable.create", json!({"collection": "Other", "name": "palette/surface", "type": "color", "value": "#FF0000"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id = create(&mut s, json!({"fill": {"$var": second}}));
    assert_eq!(s.doc().unwrap().node(id).unwrap().bound.get("fill"), Some(&second));
    assert_eq!(color(&s, id, "fill").to_hex(), "FF0000");
    // `variable.bind` deliberately keeps its first id-or-name match.
    run(&mut s, "variable.bind", json!({"id": id, "field": "fill", "variable": "palette/surface"}));
    assert_bound(&s, id, "fill", &first);
    run(&mut s, "node.set", json!({"id": id, "props": {"fill": {"$var": second}}}));
    assert_eq!(s.doc().unwrap().node(id).unwrap().bound.get("fill"), Some(&second));
    assert_eq!(color(&s, id, "fill").to_hex(), "FF0000");
}

#[test]
fn whole_name_matches_and_qualified_matches_share_one_candidate_set() {
    let (mut s, _) = setup();
    run(&mut s, "variable.createCollection", json!({"name": "Other"}));
    run(&mut s, "variable.create", json!({"collection": "Other", "name": "Theme/palette/surface", "type": "color", "value": "#FF0000"}));
    let e = s.execute("node.create", &json!({"type": "rectangle", "props": {"fill": {"$var": "Theme/palette/surface"}}})).unwrap_err().to_string();
    assert!(e.contains("ambiguous variable"), "{e}");
    assert!(e.contains("Other/Theme/palette/surface") && e.contains("Theme/palette/surface"), "{e}");
    // Collection names can contain slashes too; no splitting at the first slash.
    run(&mut s, "variable.createCollection", json!({"name": "Brand/Theme"}));
    let v = run(&mut s, "variable.create", json!({"collection": "Brand/Theme", "name": "palette/surface", "type": "color", "value": "#0D99FF80"}));
    let id = create(&mut s, json!({"fill": {"$var": "Brand/Theme/palette/surface"}}));
    assert_bound(&s, id, "fill", v["id"].as_str().unwrap());
}

#[test]
fn wrong_variable_type_rolls_back_create_and_set() {
    let (mut s, _) = setup();
    run(&mut s, "variable.create", json!({"collection": "Theme", "name": "size", "type": "number", "value": 20}));
    let id = create(&mut s, json!({"fill": "#FF0000"}));
    for field in ["fill", "stroke"] {
        for command in ["node.create", "node.set"] {
            let before = s.doc().unwrap().clone();
            let selection = s.selection().to_vec();
            let depth = s.undo_depth();
            let e = s.execute(command, &json!({"type": "rectangle", "id": id, "props": {"name": "must roll back", "fill": {"$var": "palette/surface"}, field: {"$var": "size"}}})).unwrap_err().to_string();
            assert!(e.contains(&format!("`{field}` doesn't take a number variable")), "{e}");
            assert_eq!(s.doc().unwrap(), &before);
            assert_eq!(s.selection(), selection);
            assert_eq!(s.undo_depth(), depth);
        }
    }
}

#[test]
fn variable_props_follow_modes_and_later_literal_paints_detach() {
    let (mut s, vid) = setup();
    // Start with both paints present: mode alpha on a newly created paint is an existing,
    // separate `variable.bind` bug, not part of variable props.
    let id = create(&mut s, json!({"strokes": ["#FFFFFF"], "fill": {"$var": vid}, "stroke": {"$var": vid}}));
    run(&mut s, "variable.set", json!({"variable": vid, "mode": "Dark", "value": "#20212440"}));
    for mode in ["Dark", "Light", "Dark"] {
        run(&mut s, "variable.setMode", json!({"collection": "Theme", "mode": mode}));
        for field in ["fill", "stroke"] {
            let c = color(&s, id, field);
            assert_eq!(c.to_hex(), if mode == "Dark" { "202124" } else { "0D99FF" });
            let alpha = if mode == "Dark" { 64.0 } else { 128.0 } / 255.0;
            assert!((c.a - alpha).abs() < 1e-6, "{mode}: {c:?}");
        }
    }
    run(&mut s, "node.set", json!({"id": id, "props": {"fill": "#FF0000", "stroke": "#00FF00"}}));
    assert!(s.doc().unwrap().node(id).unwrap().bound.is_empty());
    run(&mut s, "variable.set", json!({"variable": vid, "mode": "Dark", "value": "#000000"}));
    assert_eq!(color(&s, id, "fill").to_hex(), "FF0000");
    assert_eq!(color(&s, id, "stroke").to_hex(), "00FF00");
}

#[test]
fn create_with_variables_is_one_undo_entry_and_redo_restores_it() {
    let (mut s, vid) = setup();
    let before = s.doc().unwrap().clone();
    let depth = s.undo_depth();
    let id = create(&mut s, json!({"fill": {"$var": vid}, "stroke": {"$var": vid}}));
    assert_eq!(s.undo_depth(), (depth.0 + 1, 0));
    let after = s.doc().unwrap().clone();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap(), &before);
    assert!(!s.doc().unwrap().contains(id));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap(), &after);
    assert_bound(&s, id, "fill", &vid);
    assert_bound(&s, id, "stroke", &vid);
}

#[test]
fn paint_variable_props_match_explicit_binding_including_typed_alpha_and_opacity() {
    let (mut s, vid) = setup();
    for field in ["fill", "stroke"] {
        let list = if field == "fill" { "fills" } else { "strokes" };
        for opacity in [1.0, 0.25] {
            let paints = json!({list: [{"type": "solid", "color": "#FF000080", "opacity": opacity}]});
            let explicit = create(&mut s, paints.clone());
            run(&mut s, "variable.bind", json!({"id": explicit, "field": field, "variable": vid}));
            let mut props = paints.clone();
            props[field] = json!({"$var": vid});
            let inline = create(&mut s, props);
            let set = create(&mut s, paints);
            run(&mut s, "node.set", json!({"id": set, "props": {field: {"$var": vid}}}));
            for id in [inline, set] {
                let doc = s.doc().unwrap();
                let (a, b) = (doc.node(id).unwrap(), doc.node(explicit).unwrap());
                assert_eq!(a.fills, b.fills);
                assert_eq!(a.strokes, b.strokes);
                assert_eq!(a.bound, b.bound);
                assert!((color(&s, id, field).a - 128.0 / 255.0 * opacity as f32).abs() < 1e-6);
            }
            // Repeating the same binding does not accumulate alpha.
            run(&mut s, "node.set", json!({"id": inline, "props": {field: {"$var": vid}}}));
            assert_eq!(color(&s, inline, field), color(&s, explicit, field));
        }
    }
}

#[test]
fn variable_props_win_over_literal_paint_lists_and_style_links() {
    let (mut s, vid) = setup();
    let st = run(&mut s, "style.create", json!({"type": "paint", "name": "Red", "value": ["#FF000080"]}))["id"].clone();
    for field in ["fill", "stroke"] {
        let style_key = if field == "fill" { "fillStyle" } else { "strokeStyle" };
        let list = if field == "fill" { "fills" } else { "strokes" };
        let props = json!({list: ["#00FF00"], style_key: st, field: {"$var": vid}});
        let created = create(&mut s, props.clone());
        let set = create(&mut s, json!({}));
        run(&mut s, "node.set", json!({"id": set, "props": props}));
        for id in [created, set] {
            assert_bound(&s, id, field, &vid);
            let n = s.doc().unwrap().node(id).unwrap();
            assert!(if field == "fill" { n.styles.fill.is_none() } else { n.styles.stroke.is_none() });
        }
        run(&mut s, "style.update", json!({"style": st, "value": ["#FF00FF"]}));
        for id in [created, set] {
            assert_bound(&s, id, field, &vid);
        }
    }
}

#[test]
fn variable_props_unlink_a_preexisting_paint_style_even_when_the_colour_is_unchanged() {
    let (mut s, vid) = setup();
    run(&mut s, "variable.set", json!({"variable": vid, "value": "#FF0000"}));
    let style = run(&mut s, "style.create", json!({"type": "paint", "name": "Red", "value": ["#FF0000"]}))["id"].clone();
    for field in ["fill", "stroke"] {
        let style_key = if field == "fill" { "fillStyle" } else { "strokeStyle" };
        let id = create(&mut s, json!({style_key: style}));
        let n = s.doc().unwrap().node(id).unwrap();
        assert!(if field == "fill" { n.styles.fill.is_some() } else { n.styles.stroke.is_some() });
        run(&mut s, "node.set", json!({"id": id, "props": {field: {"$var": vid}}}));
        let n = s.doc().unwrap().node(id).unwrap();
        assert_eq!(n.bound.get(field), Some(&vid));
        assert!(if field == "fill" { n.styles.fill.is_none() } else { n.styles.stroke.is_none() });
        assert_eq!(color(&s, id, field).to_hex(), "FF0000");
    }
}

#[test]
fn malformed_and_unsupported_variable_props_are_errors_and_leave_no_edit() {
    let (mut s, _) = setup();
    let id = create(&mut s, json!({}));
    for props in [
        json!({"fill": {"$var": null}}),
        json!({"fill": {"$var": 5}}),
        json!({"stroke": {"$var": []}}),
        json!({"fill": {"$var": {}}}),
        json!({"fill": {"$var": ""}}),
        json!({"fill": {}}),
        json!({"fill": {"$var": "palette/surface", "extra": true}}),
        json!({"fill": {"$var": "palette/surface", "$ref": "x"}}),
        json!({"fill": {"$var": "palette/surface", "$step": 0}}),
    ] {
        for command in ["node.create", "node.set"] {
            let before = s.doc().unwrap().clone();
            let depth = s.undo_depth();
            let e = s.execute(command, &json!({"type": "rectangle", "id": id, "props": props})).unwrap_err().to_string();
            assert!(e.contains("$var") || e.contains("must be a colour"), "{e}");
            assert_eq!(s.doc().unwrap(), &before, "{props}");
            assert_eq!(s.undo_depth(), depth);
        }
    }
    for key in ["width", "fills", "strokes", "opacity", "gap", "path", "pluginData", "fillStyle", "unknown"] {
        for command in ["node.create", "node.set"] {
            let e = s.execute(command, &json!({"type": "rectangle", "id": id, "props": {key: {"$var": "palette/surface"}}})).unwrap_err().to_string();
            assert!(e.contains("only `fill` and `stroke` take variables in props") && e.contains("variable.bind"), "{e}");
        }
    }
}

#[test]
fn batch_references_resolve_inside_variable_wrappers_and_mixed_keys_roll_back() {
    for reference in [json!({"$ref": "surface"}), json!({"$step": 1})] {
        let mut s = Session::new();
        let commands = json!([
            {"command": "variable.createCollection", "params": {"name": "Theme"}},
            {"command": "variable.create", "as": "surface", "params": {"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF80"}},
            {"command": "node.create", "params": {"type": "rectangle", "props": {"fill": {"$var": reference}}}}
        ]);
        let result = run(&mut s, "command.batch", json!({"commands": commands}));
        let id = id_of(&result[2]["id"]).unwrap();
        let vid = result[1]["id"].as_str().unwrap();
        assert_bound(&s, id, "fill", vid);
        assert_eq!(s.undo_depth(), (1, 0));
        run(&mut s, "edit.undo", json!({}));
        assert!(s.doc().unwrap().variables.variables.is_empty());
        assert!(!s.doc().unwrap().contains(id));

        let before = s.doc().unwrap().clone();
        let mut bad_commands = commands.clone();
        bad_commands[2]["params"]["props"]["fill"] = json!({"$var": "surface", "$ref": "surface"});
        let e = s.execute("command.batch", &json!({"commands": bad_commands})).unwrap_err().to_string();
        assert!(e.contains("not `$var`"), "{e}");
        assert_eq!(s.doc().unwrap(), &before);
    }
}

#[test]
fn variable_props_replace_first_paint_aliases_but_keep_other_slots_bound() {
    let (mut s, vid) = setup();
    let old = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "old", "type": "color", "value": "#FF0000"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    for (field, list, aliases) in [
        ("fill", "fills", vec!["fill", "fills", "fills/0", "fills/0/color", "fills/00/color"]),
        ("stroke", "strokes", vec!["stroke", "strokes/0/color", "strokes/00/color"]),
    ] {
        for alias in aliases {
            let first = format!("{list}/0/color");
            let second = format!("{list}/1/color");
            let props = json!({list: ["#FFFFFF", "#FFFFFF"], "boundVariables": {alias: old, second.clone(): old}});
            let set = create(&mut s, props.clone());
            run(&mut s, "node.set", json!({"id": set, "props": {field: {"$var": vid}}}));
            let mut props = props;
            props[field] = json!({"$var": vid});
            let created = create(&mut s, props);
            for id in [set, created] {
                assert_bound(&s, id, field, &vid);
                let n = s.doc().unwrap().node(id).unwrap();
                assert_eq!(n.bound.len(), 2, "{alias}");
                assert_eq!(n.bound.get(&second), Some(&old));
                if alias != field {
                    assert!(!n.bound.contains_key(alias), "{alias}");
                }
                assert!(!n.bound.contains_key(&first));
                let paints = if field == "fill" { &n.fills } else { &n.strokes };
                assert_eq!(paints[1].solid_color().unwrap().to_hex(), "FF0000");
            }
        }
    }
}

#[test]
fn variable_props_bind_a_mixed_selection_and_unsupported_wrappers_leave_no_edit() {
    let (mut s, vid) = setup();
    let ids: Vec<_> = ["frame", "rectangle", "text"]
        .into_iter()
        .map(|kind| id_of(&run(&mut s, "node.create", json!({"type": kind, "width": 10, "height": 10}))["id"]).unwrap())
        .collect();
    run(&mut s, "select.set", json!({"ids": ids}));
    run(&mut s, "node.set", json!({"props": {"fill": {"$var": vid}, "stroke": {"$var": vid}}}));
    for &id in &ids {
        for field in ["fill", "stroke"] {
            assert_bound(&s, id, field, &vid);
        }
    }
    let before = s.doc().unwrap().clone();
    let selection = s.selection().to_vec();
    let depth = s.undo_depth();
    let e = s.execute("node.set", &json!({"props": {"name": "must not change", "cornerRadius": {"$var": vid}}})).unwrap_err().to_string();
    assert!(e.contains("only `fill` and `stroke` take variables in props"), "{e}");
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.selection(), selection);
    assert_eq!(s.undo_depth(), depth);
}

#[test]
fn variable_keys_inside_typed_paint_lists_stay_literal_but_bare_wrappers_error() {
    let (mut s, vid) = setup();
    let id = create(&mut s, json!({}));
    for command in ["node.create", "node.set"] {
        // Typed paints ignore extra keys today: `$var` is only a wrapper in top-level props.
        let result = run(
            &mut s,
            command,
            json!({"type": "rectangle", "id": id, "props": {
                "fills": [{"type": "solid", "color": "#FF0000", "$var": vid}]
            }}),
        );
        let painted = if command == "node.create" { id_of(&result["id"]).unwrap() } else { id };
        assert_eq!(color(&s, painted, "fill").to_hex(), "FF0000");
        assert!(s.doc().unwrap().node(painted).unwrap().bound.is_empty());
        let before = s.doc().unwrap().clone();
        let depth = s.undo_depth();
        assert!(s.execute(command, &json!({"type": "rectangle", "id": id, "props": {"fills": [{"$var": vid}]}})).is_err());
        assert_eq!(s.doc().unwrap(), &before);
        assert_eq!(s.undo_depth(), depth);
    }
}

#[test]
fn variable_bind_type_checks_the_found_variable_when_a_name_collides_with_an_id() {
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let number = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "size", "type": "number", "value": 20}))["id"].clone();
    let paint = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#FF0000"}))["id"].clone();
    run(&mut s, "variable.update", json!({"variable": number, "name": paint}));
    let id = create(&mut s, json!({}));
    run(&mut s, "variable.bind", json!({"id": id, "field": "fill", "variable": "surface"}));
    assert_eq!(s.doc().unwrap().node(id).unwrap().bound.get("fill").map(String::as_str), paint.as_str());
    assert_eq!(color(&s, id, "fill").to_hex(), "FF0000");
}
