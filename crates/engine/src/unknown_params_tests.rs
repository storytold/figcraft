//! A parameter name a command doesn't take is refused, not ignored: a misspelt name would make the
//! command answer as if it hadn't been given (#1413).

use serde_json::{Value, json};

use crate::Session;
use crate::cmd::{all_commands, param_names};

fn err(s: &mut Session, cmd: &str, params: Value) -> String {
    match s.execute(cmd, &params) {
        Err(e) => e.to_string(),
        Ok(v) => panic!("{cmd} {params} ran: {v}"),
    }
}

/// The issue's repro: a misspelt `filter` listed the resolved thread as if no filter were given.
#[test]
fn a_misspelt_parameter_is_refused_and_named() {
    let mut s = Session::new();
    s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "a"})).unwrap();
    s.execute("comment.resolve", &json!({"id": 1})).unwrap();
    let e = err(&mut s, "comment.list", json!({"filterr": "open"}));
    assert!(e.contains("unknown parameter `filterr`") && e.contains("it takes page, filter, sort"), "{e}");
    assert_eq!(s.execute("comment.list", &json!({"filter": "open"})).unwrap()["comments"].as_array().map(Vec::len), Some(0));
    let e = err(&mut s, "edit.undo", json!({"steps": 2, "stepz": 1}));
    assert!(e.contains("unknown parameters `steps`, `stepz` (it takes none)"), "{e}");
}

/// `ids` and `id` name the layers a command acts on, documented or not (`params::targets` reads
/// either), so `{id}` keeps working where a doc only says `ids?`.
#[test]
fn ids_and_id_are_always_taken() {
    let mut s = Session::new();
    let r = crate::params::id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"]).unwrap();
    assert_eq!(param_names("align.left"), Some(&["ids"][..]));
    s.execute("align.left", &json!({"id": r})).unwrap();
    s.execute("edit.undo", &json!({"ids": []})).unwrap();
}

/// Names a doc gives as alternatives (`id? | ids?`, `id?/ids?`, `path | data`) are all taken, and
/// `null` params or no params at all stay fine.
#[test]
fn documented_alternatives_and_empty_params_are_taken() {
    assert_eq!(param_names("vector.fillRule").map(|n| n.contains(&"id") && n.contains(&"ids")), Some(true));
    assert_eq!(param_names("text.setFeatures").map(|n| n.contains(&"id") && n.contains(&"ids")), Some(true));
    assert_eq!(param_names("file.import").map(|n| n.contains(&"path") && n.contains(&"data") && n.contains(&"name")), Some(true));
    assert_eq!(param_names("file.export").map(|n| n.contains(&"scale") && n.contains(&"size")), Some(true));
    assert_eq!(param_names("collab.share").map(|n| n.contains(&"via") && !n.contains(&"local")), Some(true));
    assert_eq!(param_names("component.editProperty").map(|n| n.contains(&"value") && n.contains(&"newValue")), Some(true));
    // Docs whose shape used to hide names: a stray brace, a comma in an example, a second object.
    assert_eq!(param_names("proto.interaction.add").map(|n| n.contains(&"delayMs") && n.contains(&"dest")), Some(true));
    assert_eq!(param_names("collab.join"), Some(&["code", "link"][..]));
    assert_eq!(param_names("proto.interaction.set").map(|n| n.contains(&"trigger") && n.contains(&"interaction")), Some(true));
    // Commands that run another command's code take its names too.
    assert_eq!(param_names("file.placeImage").map(|n| ["width", "height", "target", "mode"].iter().all(|k| n.contains(k))), Some(true));
    assert_eq!(param_names("file.exportSvg").map(|n| ["scale", "size", "contentsOnly", "svgOutlineText"].iter().all(|k| n.contains(k))), Some(true));
    // Alternative groups: `{x, y} | {points: …}`.
    assert_eq!(param_names("vector.edit.paint"), Some(&["x", "y", "points"][..]));
    assert_eq!(param_names("vector.edit.cut"), Some(&["segment", "t", "vertex", "from", "to"][..]));
    // `node.set` documents its props and checks them itself.
    assert_eq!(param_names("node.set"), None);
    let mut s = Session::new();
    for p in [json!(null), json!({})] {
        s.execute("select.all", &p).unwrap();
    }
}

/// Every top-level item of every `{…}` params doc gives a name, so no parameter a doc describes is
/// refused.
#[test]
fn every_doc_item_names_a_parameter() {
    let nameless: Vec<String> = all_commands()
        .iter()
        .flat_map(|c| crate::cmd::doc_items(c.params).unwrap_or_default().into_iter().map(move |i| (c.id, i)))
        .filter(|(_, i)| !i.trim().is_empty() && crate::cmd::item_names(i).is_empty())
        .map(|(id, i)| format!("{id}: `{i}`"))
        .collect();
    assert!(nameless.is_empty(), "doc items without a name: {nameless:#?}");
}

/// A batch step key other than `command`, `params` and `as` (`args` for `params`) is refused, and
/// so is a step's unknown parameter, naming the step.
#[test]
fn batch_steps_refuse_unknown_keys_and_parameters() {
    let mut s = Session::new();
    let e = err(&mut s, "command.batch", json!({"commands": [{"command": "select.all"}, {"command": "inspect.code", "args": {"id": 2}}]}));
    assert!(e.contains("step 1: unknown key `args`"), "{e}");
    let e =
        err(&mut s, "command.batch", json!({"commands": [{"command": "select.all"}, {"command": "comment.list", "params": {"filterr": "open"}}]}));
    assert!(e.contains("comment.list") && e.contains("step 1: unknown parameter `filterr`"), "{e}");
}
