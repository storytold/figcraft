//! Colour calculations: `{alpha: …}` (a referenced colour with its alpha replaced) and `{mix: …}`
//! (a blend of two colour operands) as per-mode variable values — set and create shapes, lazy
//! propagation to bound layers and mode switches, composition over aliases and other
//! calculations, set-time refusal of cycles and bad operands (leaving the variable untouched),
//! deletion flattening, undo/redo exactness, save/load round trip, exports and junk input.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::error::EngineError;
use crate::paint_tests::fill;
use crate::plugins_tests::rect;
use crate::variables_behaviour_tests::vid;
use crate::widgets_tests::err;
use figcraft_color::Color;

/// A "C" collection (modes M) with `ink` #FF0000 and `edge` #00FF00 (the error-case set's seeds).
fn seeds() -> Session {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "C", "modes": ["M"]}));
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "ink", "type": "color", "value": "#FF0000"}));
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "edge", "type": "color", "value": "#00FF00"}));
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "size", "type": "number", "value": 5}));
    s
}

#[test]
fn create_accepts_alpha_and_mix_and_they_propagate_to_bound_layers() {
    let mut s = seeds();
    let hover = vid(ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "hover", "type": "color", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
    ));
    let box_id = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [box_id], "field": "fill", "variable": "hover"}));
    assert_eq!(fill(&s, box_id).to_rgba8(), [255, 0, 0, 128], "alpha replaces the source's alpha");
    ex(&mut s, "variable.set", json!({"variable": "ink", "value": "#00AA00"}));
    assert_eq!(fill(&s, box_id).to_rgba8(), [0, 170, 0, 128], "the bound layer follows the source, no re-set needed");
    // A mix of two variables, one literal, and the t edges.
    ex(&mut s, "variable.set", json!({"variable": hover, "value": {"mix": {"from": {"variable": "ink"}, "to": {"variable": "edge"}, "t": 0.0}}}));
    assert_eq!(fill(&s, box_id).to_rgba8(), [0, 170, 0, 255], "t=0 gives from exactly");
    ex(
        &mut s,
        "variable.set",
        json!({"variable": hover, "value": {"mix": {"from": {"literal": "#000000"}, "to": {"literal": "#FFFFFF"}, "t": 0.4}}}),
    );
    assert_eq!(fill(&s, box_id).to_rgba8(), [102, 102, 102, 255], "literal operands blend (0.4 -> 0x66)");
    // Compose: a mix reading the calculation, and alpha over an alias.
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "mirror", "type": "color", "value": {"alias": "ink"}}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "faded-mirror", "type": "color", "value": {"alpha": {"variable": "mirror", "opacity": 0.5}}}),
    );
    let b2 = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b2], "field": "fill", "variable": "faded-mirror"}));
    assert_eq!(fill(&s, b2).to_rgba8(), [0, 170, 0, 128], "alpha over an alias reads the aliased value");
    let toast = vid(ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "toast", "type": "color", "value": {"alpha": {"variable": "hover", "opacity": 0.25}}}),
    ));
    let b3 = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b3], "field": "fill", "variable": toast}));
    assert_eq!(fill(&s, b3).to_rgba8(), [102, 102, 102, 64], "a calculation may read a calculation");
}

#[test]
fn per_mode_operands_follow_each_collection_chosen_mode() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Core", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.createCollection", json!({"name": "Extra", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Core", "name": "p", "type": "color", "values": {"Light": "#FF0000", "Dark": "#0000FF"}}));
    ex(&mut s, "variable.create", json!({"collection": "Extra", "name": "s", "type": "color", "values": {"Light": "#000000", "Dark": "#FFFFFF"}}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "Core", "name": "panel", "type": "color",
               "values": {"Light": {"mix": {"from": {"variable": "p"}, "to": {"variable": "s"}, "t": 0.5}},
                          "Dark": {"mix": {"from": {"variable": "p"}, "to": {"variable": "s"}, "t": 0.5}}}}),
    );
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "panel"}));
    assert_eq!(fill(&s, b).to_rgba8(), [128, 0, 0, 255], "Light mixes Light values of BOTH collections");
    ex(&mut s, "variable.setMode", json!({"ids": [b], "collection": "Core", "mode": "Dark"}));
    assert_eq!(fill(&s, b).to_rgba8(), [0, 0, 128, 255], "a Core switch moves only the Core operand (Extra stays Light)");
    ex(&mut s, "variable.setMode", json!({"ids": [b], "collection": "Extra", "mode": "Dark"}));
    assert_eq!(fill(&s, b).to_rgba8(), [128, 128, 255, 255], "the Extra switch then moves its operand, independently");
}

#[test]
fn bad_shapes_and_operands_are_refused_without_touching_the_variable() {
    let mut s = seeds();
    let before = json!(s.doc().unwrap().variables);
    let want_t = "`t` must be a number from 0 to 1";
    let want_opacity = "`opacity` must be a number from 0 to 1";
    let want_operand = "mix operands are {variable: …} or {literal: colour}";
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "self alpha",
            json!({"variable": "ink", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
            "the colour calculation would refer back to itself",
        ),
        (
            "unknown operand",
            json!({"variable": "ink", "value": {"alpha": {"variable": "ghost", "opacity": 0.5}}}),
            "no variable `ghost` to calculate from",
        ),
        (
            "number operand",
            json!({"variable": "ink", "value": {"alpha": {"variable": "size", "opacity": 0.5}}}),
            "colour calculations take colour variables: `size` is a number one",
        ),
        (
            "number mix operand",
            json!({"variable": "ink", "value": {"mix": {"from": {"variable": "size"}, "to": {"variable": "edge"}, "t": 0.5}}}),
            "colour calculations take colour variables: `size` is a number one",
        ),
        (
            "calculation on a number variable",
            json!({"variable": "size", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
            "colour calculations only fit colour variables",
        ),
        ("t too big", json!({"variable": "ink", "value": {"mix": {"from": {"variable": "edge"}, "to": {"literal": "#FFFFFF"}, "t": 1.5}}}), want_t),
        ("t missing", json!({"variable": "ink", "value": {"mix": {"from": {"variable": "edge"}, "to": {"literal": "#FFFFFF"}}}}), want_t),
        (
            "t as text",
            json!({"variable": "ink", "value": {"mix": {"from": {"variable": "edge"}, "to": {"literal": "#FFFFFF"}, "t": "half"}}}),
            want_t,
        ),
        ("opacity above 1", json!({"variable": "ink", "value": {"alpha": {"variable": "edge", "opacity": 2}}}), want_opacity),
        ("opacity missing", json!({"variable": "ink", "value": {"alpha": {"variable": "edge"}}}), want_opacity),
        ("alpha on a literal", json!({"variable": "ink", "value": {"alpha": {"literal": "#FF0000", "opacity": 0.5}}}), "alpha: {variable, opacity}"),
        ("mix with a bare string", json!({"variable": "ink", "value": {"mix": {"from": "ink", "to": {"variable": "edge"}, "t": 0.5}}}), want_operand),
        (
            "mix operand with two keys",
            json!({"variable": "ink", "value": {"mix": {"from": {"variable": "ink", "literal": "#000000"}, "to": {"variable": "edge"}, "t": 0.5}}}),
            want_operand,
        ),
        ("unknown key in alpha", json!({"variable": "ink", "value": {"alpha": {"variable": "edge", "opactiy": 0.5}}}), "alpha: {variable, opacity}"),
        // A bad parameter is named even beside an operand that loops: it is checked first.
        (
            "bad t beside a loop",
            json!({"variable": "ink", "value": {"mix": {"from": {"variable": "ink"}, "to": {"variable": "edge"}, "t": 2}}}),
            want_t,
        ),
        ("bad opacity beside a loop", json!({"variable": "ink", "value": {"alpha": {"variable": "ink", "opacity": -1}}}), want_opacity),
    ];
    for (label, p, want) in cases {
        let msg = err(&mut s, "variable.set", p.clone());
        assert!(msg.contains(want), "{label}: {msg}");
    }
    // A second key beside the operation is refused, not dropped.
    let alpha = json!({"variable": "edge", "opacity": 0.5});
    let mix = json!({"from": {"variable": "edge"}, "to": {"literal": "#FFFFFF"}, "t": 0.5});
    for (label, value) in [
        ("alpha and mix", json!({"alpha": alpha.clone(), "mix": mix.clone()})),
        ("alias beside alpha", json!({"alias": "edge", "alpha": alpha})),
        ("unknown key beside mix", json!({"mix": mix, "space": "oklch"})),
    ] {
        let msg = err(&mut s, "variable.set", json!({"variable": "ink", "value": value}));
        assert!(msg.contains("on its own"), "{label}: {msg}");
        let msg = err(&mut s, "variable.create", json!({"collection": "C", "name": "extra", "type": "color", "value": value}));
        assert!(msg.contains("on its own"), "{label} on create: {msg}");
    }
    assert_eq!(json!(s.doc().unwrap().variables), before, "the refusals left every variable as it was");
    // A cycle reached through binary operands too: ink mixes edge; then edge must not read ink.
    ex(&mut s, "variable.set", json!({"variable": "ink", "value": {"mix": {"from": {"variable": "edge"}, "to": {"literal": "#FFFFFF"}, "t": 0.5}}}));
    let msg = err(&mut s, "variable.set", json!({"variable": "edge", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}));
    assert!(msg.contains("the colour calculation would refer back to itself"), "cycle named as such: {msg}");
    let msg = err(
        &mut s,
        "variable.set",
        json!({"variable": "edge", "value": {"mix": {"from": {"variable": "ink"}, "to": {"literal": "#FFFFFF"}, "t": 0.5}}}),
    );
    assert!(msg.contains("the colour calculation would refer back to itself"), "mix-cycle named as such: {msg}");
    // None of the refusals touched the variables: ink still holds its last accepted value, and a
    // fresh calculation on it works.
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "ink"}));
    assert_eq!(fill(&s, b).to_rgba8(), [128, 255, 128, 255], "ink keeps the accepted mix (edge + white at 0.5)");
    ex(&mut s, "variable.set", json!({"variable": "ink", "value": {"alpha": {"variable": "edge", "opacity": 0.5}}}));
    assert_eq!(fill(&s, b).to_rgba8(), [0, 255, 0, 128], "and calculations still set afterwards");
}

#[test]
fn undo_and_redo_are_exact_through_calculations() {
    let mut s = seeds();
    let hover = vid(ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "hover", "type": "color", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
    ));
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": hover}));
    let first = fill(&s, b).to_rgba8();
    ex(&mut s, "variable.set", json!({"variable": "hover", "value": {"alpha": {"variable": "edge", "opacity": 0.25}}}));
    let second = fill(&s, b).to_rgba8();
    assert_ne!(first, second);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(fill(&s, b).to_rgba8(), first, "undo restores the stored alpha calculation exactly");
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!(fill(&s, b).to_rgba8(), second, "redo replays the new one exactly");
}

#[test]
fn deleting_a_source_flattens_its_calculations() {
    let mut s = seeds();
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "fade", "type": "color", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
    );
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "fade"}));
    let before = json!(s.doc().unwrap().variables);
    ex(&mut s, "variable.delete", json!({"variable": "ink"}));
    assert_eq!(fill(&s, b).to_rgba8(), [255, 0, 0, 128], "the bound layer keeps the colour it showed");
    let fade: Vec<_> = s.doc().unwrap().variables.get("fade").unwrap().values.values().cloned().collect();
    assert_eq!(fade, [figcraft_doc::VariableValue::Color(Color::new(1.0, 0.0, 0.0, 0.5))], "the calculation became the colour it showed");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(json!(s.doc().unwrap().variables), before, "undo restores every variable exactly");
}

/// Deleting a collection at once flattens every reader outside it — an alias of a calculation, a
/// calculation of two deleted variables — and deleting a calculation flattens a calculation that
/// reads it; undo restores the variables exactly each time.
#[test]
fn deleting_sources_together_flattens_every_reader() {
    let mut s = seeds();
    let calc = |name: &str, value: Value| json!({"collection": "C", "name": name, "type": "color", "value": value});
    ex(&mut s, "variable.create", calc("fade", json!({"alpha": {"variable": "ink", "opacity": 0.5}})));
    ex(&mut s, "variable.create", calc("wash", json!({"mix": {"from": {"variable": "fade"}, "to": {"literal": "#FFFFFF"}, "t": 0.5}})));
    ex(&mut s, "variable.createCollection", json!({"name": "App"}));
    ex(&mut s, "variable.create", json!({"collection": "App", "name": "echo", "type": "color", "value": {"alias": "wash"}}));
    let tint = json!({"mix": {"from": {"variable": "edge"}, "to": {"variable": "ink"}, "t": 0.5}});
    ex(&mut s, "variable.create", json!({"collection": "App", "name": "tint", "type": "color", "value": tint}));
    let stored = |s: &Session, name: &str| s.doc().unwrap().variables.get(name).unwrap().values.values().next().cloned().unwrap();
    let plain = |s: &Session, name: &str| match stored(s, name) {
        figcraft_doc::VariableValue::Color(c) => c.to_rgba8(),
        other => panic!("{name} is not a plain colour: {other:?}"),
    };
    let before = json!(s.doc().unwrap().variables);

    ex(&mut s, "variable.deleteCollection", json!({"collection": "C"}));
    assert_eq!(plain(&s, "echo"), [255, 128, 128, 191], "the alias of a calculation keeps what it showed");
    assert_eq!(plain(&s, "tint"), [128, 128, 0, 255], "a calculation of two deleted variables keeps its colour");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(json!(s.doc().unwrap().variables), before, "undo restores the collection and its readers exactly");

    ex(&mut s, "variable.delete", json!({"variable": "fade"}));
    assert_eq!(plain(&s, "wash"), [255, 128, 128, 191], "a calculation of the deleted calculation keeps its colour");
    assert!(matches!(stored(&s, "echo"), figcraft_doc::VariableValue::Alias(_)), "an alias of a surviving variable stays an alias");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(json!(s.doc().unwrap().variables), before);
}

/// One resolve reads at most `Variables::RESOLVE_LIMIT` variables: a mix of two alias chains that
/// would read more is refused when it is set rather than left showing nothing.
#[test]
fn a_calculation_reading_too_many_variables_is_refused() {
    let mut s = seeds();
    for side in ["a", "b"] {
        ex(&mut s, "variable.create", json!({"collection": "C", "name": format!("{side}0"), "type": "color", "value": {"alias": "ink"}}));
        for i in 1..16 {
            let value = json!({"alias": format!("{side}{}", i - 1)});
            ex(&mut s, "variable.create", json!({"collection": "C", "name": format!("{side}{i}"), "type": "color", "value": value}));
        }
    }
    // Reading `a{k}` and `b{k}` takes 2k + 4 variables: itself, both chains and the `ink` they share.
    let mix = |k: usize| json!({"mix": {"from": {"variable": format!("a{k}")}, "to": {"variable": format!("b{k}")}, "t": 0.5}});
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "fits", "type": "color", "value": mix(14)}));
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "fits"}));
    assert_eq!(fill(&s, b).to_rgba8(), [255, 0, 0, 255], "32 variables still resolve");
    let too_many = "the colour calculation reads more than 32 variables";
    let msg = err(&mut s, "variable.create", json!({"collection": "C", "name": "over", "type": "color", "value": mix(15)}));
    assert!(msg.contains(too_many), "create: {msg}");
    let msg = err(&mut s, "variable.set", json!({"variable": "fits", "value": mix(15)}));
    assert!(msg.contains(too_many), "set: {msg}");
}

/// Junk anywhere inside a calculation is refused cleanly: the variables exist, so it reaches the
/// calculation parsers (the command-wide sweep starts from a document without variables).
#[test]
fn calculation_parsers_survive_junk() {
    let junk = [
        json!(null),
        json!(1e308),
        json!(-0.0),
        json!("NaN"),
        json!("\u{0}"),
        json!([]),
        json!({}),
        json!([1e308, -1e308, "x"]),
        json!({"variable": [1]}),
        json!({"variable": "ink", "literal": "#000"}),
        json!({"literal": {"r": 1e308, "g": "x"}}),
        json!({"mix": {"from": {"literal": {"mix": 1}}}}),
        json!({"alpha": {"variable": "ink", "opacity": 0.5}}),
    ];
    for j in &junk {
        for value in [
            json!({"alpha": j}),
            json!({"mix": j}),
            json!({"mix": {"from": j, "to": j, "t": j}}),
            json!({"alpha": {"variable": j, "opacity": j}}),
            json!({"alpha": {"variable": "ink", "opacity": j}}),
            json!({"mix": {"from": {"variable": "ink"}, "to": {"literal": j}, "t": 0.5}}),
        ] {
            for (cmd, p) in [
                ("variable.set", json!({"variable": "edge", "value": value})),
                ("variable.set", json!({"variable": "size", "value": value})),
                ("variable.create", json!({"collection": "C", "name": "junk", "type": "color", "values": {"M": value}})),
            ] {
                let mut s = seeds();
                if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, &p) {
                    panic!("{cmd} panicked on {p}: {msg}");
                }
            }
        }
    }
}

#[test]
fn calculations_survive_save_and_load() {
    let mut s = seeds();
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "mixv", "type": "color", "value": {"mix": {"from": {"variable": "ink"}, "to": {"variable": "edge"}, "t": 0.4}}}),
    );
    let dir = std::env::temp_dir().join(format!("figcraft-colour-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("d.figcraft");
    ex(&mut s, "file.save", json!({"path": path.to_string_lossy()}));
    let mut t = Session::new();
    ex(&mut t, "file.open", json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_dir_all(&dir);
    let list = ex(&mut t, "variable.list", json!({}));
    let mixv = list["variables"].as_array().unwrap().iter().find(|v| v["name"] == "mixv").unwrap();
    assert_eq!(mixv["values"].as_object().unwrap().values().next().unwrap()["type"], "colorMix", "still a calculation after a round trip");
    // And it still propagates in the reloaded document.
    let b = idv(&ex(&mut t, "shape.rectangle", json!({"x": 0, "y": 0, "width": 4, "height": 4})));
    ex(&mut t, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "mixv"}));
    assert_eq!(fill(&t, b).to_rgba8(), [153, 102, 0, 255], "ink + edge at 0.4: r=255*(1-0.4), g=255*0.4");
}

#[test]
fn rest_and_css_exports_write_calculations_as_their_colours() {
    let mut s = seeds();
    // The exported REST variable `name` of `collections`, and the CSS text for them.
    let json_variable = |s: &mut Session, collections: Value, name: &str| -> Value {
        let text = ex(s, "variable.export", json!({"format": "json", "collections": collections}));
        let doc: Value = serde_json::from_str(text["text"].as_str().unwrap()).unwrap();
        let meta = doc.get("meta").unwrap_or(&doc);
        meta["variables"].as_object().unwrap().values().find(|v| v["name"] == name).unwrap().clone()
    };
    let css = |s: &mut Session, collections: Value| {
        ex(s, "variable.export", json!({"format": "css", "collections": collections}))["text"].as_str().unwrap().to_string()
    };
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "fade", "type": "color", "value": {"alpha": {"variable": "ink", "opacity": 0.5}}}),
    );
    let fade = json_variable(&mut s, json!(["C"]), "fade");
    let value = fade["valuesByMode"].as_object().unwrap().values().next().unwrap();
    assert_eq!(value["r"], 1.0);
    assert_eq!(value["a"], 0.5);
    assert!(value.get("type").is_none(), "no calculation shape leaks into the REST JSON: {value}");
    let text = css(&mut s, json!(["C"]));
    assert!(text.contains("rgba(255, 0, 0, 0.5"), "the CSS export writes the colour it resolves to: {text}");
    // Each mode is written as the colour it shows in that mode.
    ex(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "Theme", "name": "brand", "type": "color", "values": {"Light": "#FF0000", "Dark": "#0000FF"}}),
    );
    let veil = json!({"alpha": {"variable": "brand", "opacity": 0.5}});
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "veil", "type": "color", "value": veil}));
    let veil = json_variable(&mut s, json!(["Theme"]), "veil");
    let mut per_mode: Vec<(f64, f64, f64)> = veil["valuesByMode"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| (v["r"].as_f64().unwrap(), v["b"].as_f64().unwrap(), v["a"].as_f64().unwrap()))
        .collect();
    per_mode.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert_eq!(per_mode, [(0.0, 1.0, 0.5), (1.0, 0.0, 0.5)], "Light and Dark each written as their own colour");
    let text = css(&mut s, json!(["Theme"]));
    assert!(text.contains("rgba(255, 0, 0, 0.5") && text.contains("rgba(0, 0, 255, 0.5"), "both modes in CSS: {text}");
}

/// A hand-edited file can hold a calculation that resolves to nothing (one reading a variable
/// that isn't there). The CSS export leaves it out, so no property may name it: an alias to it
/// goes out as the colour it shows, or is left out when that is nothing.
#[test]
fn css_never_names_a_calculation_it_leaves_out() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "T", "modes": ["Light", "Dark"]}));
    let ink = json!({"collection": "T", "name": "ink", "type": "color", "values": {"Light": "#FF0000", "Dark": "#0000FF"}});
    ex(&mut s, "variable.create", ink);
    for name in ["lost", "half"] {
        ex(&mut s, "variable.create", json!({"collection": "T", "name": name, "type": "color", "value": "#000000"}));
    }
    for (name, to) in [("lost-too", "lost"), ("half-too", "half"), ("ink-too", "ink")] {
        ex(&mut s, "variable.create", json!({"collection": "T", "name": name, "type": "color", "value": {"alias": to}}));
    }
    // As a hand-edited file could hold them: `lost` reads a variable that isn't there in both
    // modes, `half` only in Dark.
    let doc = s.doc_mut().unwrap();
    let ink = doc.variables.get("ink").unwrap().id.clone();
    let modes: Vec<String> = doc.variables.collection("T").unwrap().modes.iter().map(|m| m.id.clone()).collect();
    let gone = figcraft_doc::VariableValue::ColorAlpha { variable: "gone".into(), opacity: 0.5 };
    let half = figcraft_doc::VariableValue::ColorAlpha { variable: ink, opacity: 0.5 };
    for v in &mut doc.variables.variables {
        match v.name.as_str() {
            "lost" => v.values = modes.iter().map(|m| (m.clone(), gone.clone())).collect(),
            "half" => v.values = [(modes[0].clone(), half.clone()), (modes[1].clone(), gone.clone())].into_iter().collect(),
            _ => {}
        }
    }
    let css = ex(&mut s, "variable.export", json!({"format": "css"}))["text"].as_str().unwrap().to_string();
    let (light, dark) = css.split_once("[data-").unwrap();
    assert!(!css.contains("var(--lost)") && !css.contains("var(--half)"), "no var() of a property a block leaves out: {css}");
    assert!(!css.contains("--lost-too"), "an alias to what shows nothing is left out: {css}");
    assert!(light.contains("--half-too: rgba(255, 0, 0, 0.5") && !dark.contains("--half-too"), "the colour it shows, where it shows one: {css}");
    assert!(css.contains("--ink-too: var(--ink);"), "an alias that always resolves stays a reference: {css}");
}

/// A calculation is a variable's value, not a literal an expression may hold: nothing would check
/// what it reads. Every way an expression arrives refuses one.
#[test]
fn prototype_expressions_refuse_a_calculation_literal() {
    let mut s = seeds();
    let btn = rect(&mut s, 0.0);
    let dest = idv(&ex(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100})));
    ex(&mut s, "proto.interaction.add", json!({"id": btn, "dest": dest}));
    let calc = json!({"value": {"type": "colorAlpha", "value": {"variable": "ink", "opacity": 9}}});
    let msg = err(&mut s, "proto.evaluate", json!({"expr": calc}));
    assert!(msg.contains("Not a value"), "evaluate: {msg}");
    let set = json!({"type": "setVariable", "variable": "edge", "value": null, "expr": calc});
    let msg = err(&mut s, "proto.action.add", json!({"id": btn, "index": 0, "action": set}));
    assert!(msg.contains("Not a value"), "set variable action: {msg}");
    let added = ex(
        &mut s,
        "proto.action.add",
        json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [{"expression": "size > 1", "actions": []}]}}),
    );
    let same = json!({"op": {"op": "equals", "args": [{"value": {"type": "alias", "value": "ink"}}, calc]}});
    let msg = err(&mut s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": added["path"], "block": 0, "condition": same}));
    assert!(msg.contains("Not a value"), "condition: {msg}");
}

#[test]
fn rest_import_skips_a_hand_written_calculation_with_a_warning() {
    // The variables JSON has no shape for calculations; a hand-written one must not import or corrupt.
    let mut s = seeds();
    let text = json!({
        "variableCollections": {"COL:X": {"id": "COL:X", "name": "Imp", "modes": [{"modeId": "M:1", "name": "M"}], "defaultModeId": "M:1", "variableIds": ["V:9"]}},
        "variables": {"V:9": {"id": "V:9", "name": "ghost", "variableCollectionId": "COL:X", "resolvedType": "COLOR",
                              "valuesByMode": {"M:1": {"type": "colorAlpha", "value": {"variable": "V:1", "opacity": 0.5}}}}}
    });
    let out = ex(&mut s, "variable.import", json!({"variables": text}));
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap_or("").contains("composed colours")), "skip warning present: {out}");
    let list = ex(&mut s, "variable.list", json!({}));
    assert!(list["variables"].as_array().unwrap().iter().all(|v| v["name"] != "ghost"), "the calculation was skipped");
}

#[test]
fn profile_conversion_reaches_the_colours_stored_in_a_calculation() {
    // A mix of identical literals shows that colour; after a converting profile change the
    // mix-bound layer must match a layer bound to the plain colour (the literal converted too).
    let mut s = seeds();
    ex(&mut s, "variable.create", json!({"collection": "C", "name": "p", "type": "color", "value": "#804020"}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "dl", "type": "color", "value": {"mix": {"from": {"literal": "#804020"}, "to": {"literal": "#804020"}, "t": 0.5}}}),
    );
    let a = rect(&mut s, 0.0);
    let b = rect(&mut s, 0.0);
    ex(&mut s, "variable.bind", json!({"ids": [a], "field": "fill", "variable": "p"}));
    ex(&mut s, "variable.bind", json!({"ids": [b], "field": "fill", "variable": "dl"}));
    ex(&mut s, "file.colorProfile", json!({"profile": "displayP3"}));
    let (fa, fb) = (fill(&s, a), fill(&s, b));
    assert_ne!(fa.to_rgba8(), [128, 64, 32, 255], "the profile change converted the plain colour");
    assert_eq!(fa.to_rgba8(), fb.to_rgba8(), "the colour stored inside the calculation converted the same way");
}

#[test]
fn negative_zero_bounds_are_stored_as_plain_zero() {
    let mut s = seeds();
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "n0", "type": "color", "value": {"alpha": {"variable": "ink", "opacity": -0.0}}}),
    );
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "C", "name": "m0", "type": "color", "value": {"mix": {"from": {"variable": "ink"}, "to": {"variable": "edge"}, "t": -0.0}}}),
    );
    let list = serde_json::to_string(&ex(&mut s, "variable.list", json!({}))).unwrap();
    assert!(!list.contains("-0"), "no negative zeros persisted: {list}");
}
