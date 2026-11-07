//! End-to-end agent tasks: an in-process MCP client completes realistic design tasks only through
//! MCP tool calls, and checks every result through inspect_document / get_node like an agent would.

use crate::common;

use common::{Client, approx, png_dims};
use serde_json::{Value, json};

fn ids(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Task 1: a profile card with vertical auto layout, padding, gap, hug height, fill-width children.
#[test]
fn task_card_with_auto_layout() {
    let mut c = Client::new();
    let card = c.create(json!({"type": "frame", "name": "Card", "x": 100, "y": 100, "width": 320, "height": 50,
        "props": {"fill": "#FFFFFF", "cornerRadius": 16, "layout": "vertical", "padding": 24, "gap": 12, "sizingV": "hug"}}));
    let title = c.create(json!({"type": "text", "parent": card, "text": "Ada Lovelace", "props": {"fontSize": 24}}));
    let body = c.create(json!({"type": "text", "parent": card, "text": "First programmer", "props": {"fontSize": 14, "fill": "#666666"}}));
    let button = c.create(json!({"type": "frame", "parent": card, "name": "Button", "height": 44,
        "props": {"fill": "#3B5BDB", "cornerRadius": 8, "layout": "horizontal", "mainAlign": "center", "crossAlign": "center", "sizingH": "fill"}}));
    c.create(json!({"type": "text", "parent": button, "text": "Follow", "props": {"fill": "#FFFFFF"}}));

    let n = c.node(&card);
    assert_eq!(n["layout"]["mode"], "vertical", "{}", n["layout"]);
    assert_eq!((n["layout"]["gap"].as_f64(), n["layout"]["padding"][0].as_f64()), (Some(12.0), Some(24.0)));
    let kids = ids(&n["children"]);
    assert_eq!(kids.len(), 3);
    // Children stack top to bottom: padding 24, gap 12.
    let t = c.node(&title);
    let b = c.node(&body);
    assert!(approx(&t["x"], 24.0) && approx(&t["y"], 24.0), "title at {} {}", t["x"], t["y"]);
    let want_body_y = 24.0 + t["height"].as_f64().unwrap() + 12.0;
    assert!(approx(&b["y"], want_body_y), "body y {} want {want_body_y}", b["y"]);
    // The button fills the width (320 - 2*24) and the card hugs its content height.
    let btn = c.node(&button);
    assert!(approx(&btn["width"], 272.0), "button width {}", btn["width"]);
    let bottom = btn["y"].as_f64().unwrap() + btn["height"].as_f64().unwrap() + 24.0;
    assert!(approx(&n["height"], bottom), "card height {} want {bottom}", n["height"]);
    assert_eq!(n["cornerRadius"], json!(16.0));
}

/// Task 2: a component with two instances; override one instance's text, then edit the main.
#[test]
fn task_component_instances_and_overrides() {
    let mut c = Client::new();
    let frame = c.create(json!({"type": "frame", "name": "Chip", "x": 0, "y": 0, "width": 120, "height": 40, "props": {"fill": "#EEEEEE", "layout": "horizontal", "padding": 8}}));
    let label = c.create(json!({"type": "text", "parent": frame, "text": "Label"}));
    let comp = c.run("component.create", json!({"ids": [frame]}))["id"].clone();
    assert_eq!(c.node(&comp)["type"].as_str().unwrap().to_lowercase(), "component");

    let i1 = c.run("instance.create", json!({"component": comp, "x": 0, "y": 100}))["id"].clone();
    let i2 = c.run("instance.create", json!({"component": comp, "x": 200, "y": 100}))["id"].clone();
    for i in [&i1, &i2] {
        let n = c.node(i);
        assert_eq!(n["type"].as_str().unwrap().to_lowercase(), "instance");
        assert_eq!(n["mainComponent"], comp);
        assert_eq!(n["children"].as_array().unwrap().len(), 1);
    }
    // Override the text in the first instance.
    let inner1 = c.node(&i1)["children"][0]["id"].clone();
    c.ok("type_text", json!({"id": inner1, "text": "Design"}));
    let n1 = c.node(&i1);
    assert_eq!(n1["children"][0]["characters"], "Design");
    assert!(n1["children"][0]["overrides"].to_string().contains("characters"), "override recorded: {}", n1["children"][0]);
    // Editing the main component's fill propagates to both instances; the text override survives.
    c.ok("set_properties", json!({"ids": [comp], "props": {"fill": "#FFD43B"}}));
    c.ok("set_properties", json!({"ids": [label], "props": {"characters": "Tag"}}));
    let n1 = c.node(&i1);
    let n2 = c.node(&i2);
    assert_eq!(n1["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#FFD43B");
    assert_eq!(n2["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#FFD43B");
    assert_eq!(n1["children"][0]["characters"], "Design");
    assert_eq!(n2["children"][0]["characters"], "Tag");
}

/// Task 3: a gradient fill and a drop shadow, given in agent-friendly JSON (hex colours).
#[test]
fn task_gradient_and_drop_shadow() {
    let mut c = Client::new();
    let r = c.create(json!({"type": "rectangle", "name": "Hero bg", "x": 0, "y": 0, "width": 400, "height": 200}));
    c.ok(
        "set_properties",
        json!({"ids": [r], "props": {
            "fills": [{"type": "linearGradient", "stops": ["#7048E8", "#1098AD"]}],
            "effects": [{"type": "dropShadow", "color": "#00000040", "offset": [0, 8], "radius": 24}],
        }}),
    );
    let n = c.node(&r);
    assert_eq!(n["fills"][0]["type"], "gradient", "{}", n["fills"]);
    assert_eq!(n["fills"][0]["kind"], "linear");
    assert_eq!(n["fills"][0]["stops"].as_array().unwrap().len(), 2);
    assert!(approx(&n["fills"][0]["stops"][1]["position"], 1.0));
    let e = &n["effects"][0];
    assert_eq!(e["type"], "dropShadow", "{e}");
    assert_eq!(e["radius"], json!(24.0));
    assert_eq!(e["offset"], json!([0.0, 8.0]));
    assert!((e["color"]["a"].as_f64().unwrap() - 0x40 as f64 / 255.0).abs() < 0.01);
    // The shadow makes the render bigger than the box.
    let shot = c.call("screenshot", json!({"ids": [r]}));
    let (w, h) = png_dims(shot["content"][0]["data"].as_str().unwrap());
    assert!(w > 400 && h > 200, "shadow extends the render: {w}x{h}");
}

/// Task 4: a paint style created from a layer and applied to others; editing the style updates them.
#[test]
fn task_paint_style() {
    let mut c = Client::new();
    let swatch = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "props": {"fill": "#E8590C"}}));
    let a = c.create(json!({"type": "ellipse", "x": 200, "y": 0}));
    let b = c.create(json!({"type": "rectangle", "x": 400, "y": 0}));
    let style = c.run("style.create", json!({"type": "paint", "name": "Brand/Orange", "from": swatch}))["id"].clone();
    c.run("style.apply", json!({"ids": [a, b], "style": "Brand/Orange"}));
    for id in [&a, &b] {
        let n = c.node(id);
        assert_eq!(n["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#E8590C");
        assert!(n["styles"].to_string().contains(style.as_str().unwrap()), "style link on {}: {}", id, n["styles"]);
    }
    let doc = c.doc();
    assert!(doc["styles"].as_array().unwrap().iter().any(|s| s["name"] == "Brand/Orange"));
    // Redefine the style: linked layers follow.
    c.run("style.update", json!({"style": style, "value": [{"type": "solid", "color": {"r": 0.0, "g": 0.0, "b": 1.0}}]}));
    assert_eq!(c.node(&a)["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#0000FF");
    assert_eq!(c.node(&b)["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#0000FF");
}

/// Task 5: colour variables with Light/Dark modes bound to a frame; switching modes recolours it.
#[test]
fn task_variables_light_dark() {
    let mut c = Client::new();
    let coll = c.run("variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}))["id"].clone();
    let bg = c
        .run("variable.create", json!({"collection": coll, "name": "color/bg", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#111111"}}))
        ["id"]
        .clone();
    let frame = c.create(json!({"type": "frame", "name": "Screen", "width": 300, "height": 200}));
    c.run("variable.bind", json!({"ids": [frame], "field": "fill", "variable": bg}));
    let n = c.node(&frame);
    assert_eq!(n["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#FFFFFF");
    assert!(n["boundVariables"].to_string().contains(bg.as_str().unwrap()));
    c.run("variable.setMode", json!({"ids": [frame], "collection": coll, "mode": "Dark"}));
    assert_eq!(c.node(&frame)["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#111111");
    c.run("variable.setMode", json!({"ids": [frame], "collection": coll, "mode": "Light"}));
    assert_eq!(c.node(&frame)["fills"][0]["color"].as_str().unwrap().to_uppercase(), "#FFFFFF");
    let list = c.run("variable.list", json!({}));
    assert!(list.to_string().contains("color/bg"));
}

/// Task 6: align three shapes, distribute them, group them.
#[test]
fn task_group_align_distribute() {
    let mut c = Client::new();
    let a = c.create(json!({"type": "rectangle", "x": 10, "y": 50, "width": 40, "height": 40}));
    let b = c.create(json!({"type": "rectangle", "x": 100, "y": 0, "width": 60, "height": 60}));
    let d = c.create(json!({"type": "ellipse", "x": 300, "y": 120, "width": 20, "height": 20}));
    c.ok(
        "batch",
        json!({"label": "Align + distribute", "commands": [
            {"command": "align.top", "params": {"ids": [a, b, d]}},
            {"command": "distribute.horizontal", "params": {"ids": [a, b, d]}},
        ]}),
    );
    let (na, nb, nd) = (c.node(&a), c.node(&b), c.node(&d));
    for n in [&na, &nb, &nd] {
        assert!(approx(&n["y"], 0.0), "aligned top: {}", n["y"]);
    }
    // Equal gaps: (span 10..320 = 310, minus widths 120) / 2 = 95.
    let gap1 = nb["x"].as_f64().unwrap() - (na["x"].as_f64().unwrap() + 40.0);
    let gap2 = nd["x"].as_f64().unwrap() - (nb["x"].as_f64().unwrap() + 60.0);
    assert!((gap1 - gap2).abs() < 0.5 && (gap1 - 95.0).abs() < 0.5, "gaps {gap1} {gap2}");
    // One undo step for the batch.
    let hist = c.doc()["history"]["undo"].clone();
    assert_eq!(hist[0], "Align + distribute");
    let g = c.run("object.group", json!({"ids": [a, b, d], "name": "Row"}))["id"].clone();
    let ng = c.node(&g);
    assert_eq!(ng["type"].as_str().unwrap().to_lowercase(), "group");
    assert_eq!(ng["children"].as_array().unwrap().len(), 3);
    assert!(approx(&ng["absoluteBounds"]["x"], 10.0) && approx(&ng["absoluteBounds"]["width"], 310.0), "{}", ng["absoluteBounds"]);
    assert_eq!(c.page_children().len(), 1);
}

/// Task 7: boolean subtract (a ring), then flatten it to a vector.
#[test]
fn task_boolean_subtract_then_flatten() {
    let mut c = Client::new();
    let outer = c.create(json!({"type": "ellipse", "x": 0, "y": 0, "width": 100, "height": 100, "props": {"fill": "#000000"}}));
    let inner = c.create(json!({"type": "ellipse", "x": 25, "y": 25, "width": 50, "height": 50}));
    let bool_id = c.run("bool.subtract", json!({"ids": [outer, inner]}))["id"].clone();
    let n = c.node(&bool_id);
    assert_eq!(n["booleanOperation"].as_str().unwrap().to_lowercase(), "subtract", "{n}");
    assert_eq!(n["children"].as_array().unwrap().len(), 2);
    assert!(approx(&n["width"], 100.0));
    let flat = c.run("bool.flatten", json!({"ids": [bool_id]}));
    let vid = flat.get("id").cloned().or_else(|| flat["ids"].get(0).cloned()).unwrap();
    let v = c.node(&vid);
    assert_eq!(v["type"].as_str().unwrap().to_lowercase(), "vector", "{v}");
    assert!(approx(&v["width"], 100.0) && approx(&v["height"], 100.0));
    assert_eq!(c.page_children().len(), 1);
    // The hole is really there: the centre pixel is transparent, the ring is painted.
    let shot = c.call("export", json!({"ids": [vid], "format": "png"}));
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(shot["content"][0]["data"].as_str().unwrap()).unwrap();
    let (w, h) = figcraft_mcp::png_size(&bytes).unwrap();
    assert!((100..=101).contains(&w) && (100..=101).contains(&h), "{w}x{h}");
    let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(w / 2, h / 2)[3], 0, "the hole is transparent");
    assert_eq!(img.get_pixel(w / 2, 10)[3], 255, "the ring is painted");
}

/// Task 8: export a frame as PNG at 2x (inline image and to a file) and as SVG.
#[test]
fn task_export_png_and_svg() {
    let mut c = Client::new();
    let f = c.create(json!({"type": "frame", "name": "Banner", "x": 50, "y": 50, "width": 160, "height": 90, "props": {"fill": "#228BE6"}}));
    c.create(json!({"type": "text", "parent": f, "x": 10, "y": 10, "text": "Hello"}));
    let r = c.call("export", json!({"ids": [f], "format": "png", "scale": 2}));
    assert_eq!(r["isError"], false);
    assert_eq!(r["content"][0]["type"], "image");
    assert_eq!(r["content"][0]["mimeType"], "image/png");
    assert_eq!(png_dims(r["content"][0]["data"].as_str().unwrap()), (320, 180));
    let dir = std::env::temp_dir().join(format!("figcraft-mcp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("banner.png");
    c.ok("export", json!({"ids": [f], "path": path.to_string_lossy(), "scale": 1}));
    assert_eq!(figcraft_mcp::png_size(&std::fs::read(&path).unwrap()), Some((160, 90)));
    let svg = c.ok("export", json!({"ids": [f], "format": "svg"}));
    let svg = svg.as_str().unwrap();
    assert!(svg.starts_with("<svg") || svg.starts_with("<?xml"), "{}", &svg[..svg.len().min(80)]);
    // Whole-page screenshot respects maxSize.
    let shot = c.call("screenshot", json!({"maxSize": 80}));
    let (w, h) = png_dims(shot["content"][0]["data"].as_str().unwrap());
    assert!(w <= 80 && h <= 80 && w > 0, "{w}x{h}");
    // Save and reopen round-trip.
    let file = dir.join("banner.figcraft");
    c.ok("save_file", json!({"path": file.to_string_lossy()}));
    let opened = c.ok("open_file", json!({"path": file.to_string_lossy()}));
    assert!(opened["document"]["pages"][0]["children"].to_string().contains("Banner"));
    // The tab is that file: `reload` reads it again from disk (#997).
    let reread = c.ok("open_file", json!({"path": file.to_string_lossy(), "reload": true}));
    assert_eq!(reread["opened"]["reloaded"], true, "{reread}");
    assert!(reread["document"]["pages"][0]["children"].to_string().contains("Banner"));
    std::fs::remove_dir_all(&dir).ok();
}

/// Task 9: undo/redo across single commands, a batch and a pointer gesture.
#[test]
fn task_undo_redo() {
    let mut c = Client::new();
    let r = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100}));
    c.ok("set_properties", json!({"ids": [r], "props": {"x": 50, "fill": "#00FF00"}}));
    assert!(approx(&c.node(&r)["x"], 50.0));
    // Drag it with the move tool: one gesture = one undo step.
    c.ok("pointer", json!({"tool": "move", "kind": "drag", "x": 100, "y": 50, "to": [140, 70]}));
    let n = c.node(&r);
    assert!(approx(&n["x"], 90.0) && approx(&n["y"], 20.0), "dragged to {} {}", n["x"], n["y"]);
    assert_eq!(c.ok("undo", json!({}))["undone"], true);
    assert!(approx(&c.node(&r)["x"], 50.0));
    assert_eq!(c.ok("undo", json!({}))["undone"], true);
    let n = c.node(&r);
    assert!(approx(&n["x"], 0.0) && n["fills"][0]["color"].as_str().unwrap().to_uppercase() != "#00FF00");
    assert_eq!(c.ok("redo", json!({}))["redone"], true);
    assert!(approx(&c.node(&r)["x"], 50.0));
    assert_eq!(c.ok("undo", json!({}))["undone"], true);
    assert_eq!(c.ok("undo", json!({}))["undone"], true);
    assert_eq!(c.page_children().len(), 0, "creation undone");
    assert_eq!(c.ok("undo", json!({}))["undone"], false);
    assert_eq!(c.ok("redo", json!({}))["redone"], true);
    assert_eq!(c.page_children().len(), 1);
}

/// Task 10: draw with tools and type through the keyboard, like a user.
#[test]
fn task_draw_and_type_with_tools() {
    let mut c = Client::new();
    c.ok("key", json!({"key": "R"}));
    assert_eq!(c.ok("inspect_ui", json!({}))["tool"]["tool"], "rectangle");
    let r = c.ok("pointer", json!({"kind": "drag", "x": 10, "y": 10, "to": [110, 60]}));
    let id = r["selection"][0].clone();
    let n = c.node(&id);
    assert!(approx(&n["width"], 100.0) && approx(&n["height"], 50.0), "{n}");
    // Text tool: click, type, escape.
    c.ok("pointer", json!({"tool": "text", "kind": "click", "x": 200, "y": 200}));
    c.ok("type_text", json!({"text": "Hi there"}));
    c.ok("key", json!({"key": "Escape"}));
    let kids = c.page_children();
    let t = kids.iter().find(|k| k["type"].as_str().unwrap().eq_ignore_ascii_case("text")).expect("a text layer");
    assert_eq!(t["characters"], "Hi there");
    // Cmd+A selects all, Delete removes everything.
    c.ok("key", json!({"key": "A", "cmd": true}));
    c.ok("key", json!({"key": "Delete"}));
    assert_eq!(c.page_children().len(), 0);
}

/// Keys in vector edit mode reach the edit tools first, as on the canvas (Delete took the whole
/// layer and the tool letters were unknown).
#[test]
fn task_vector_edit_keys() {
    let mut c = Client::new();
    let id = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100}));
    c.run("vector.edit.begin", json!({"id": id}));
    c.ok("key", json!({"key": "X"}));
    assert_eq!(c.run("vector.edit.state", json!({}))["tool"], "cut");
    c.ok("key", json!({"key": "V"}));
    c.run("vector.edit.select", json!({"vertices": [1]}));
    c.ok("key", json!({"key": "Delete"}));
    let state = c.run("vector.edit.state", json!({}));
    assert_eq!(state["network"]["vertices"].as_array().map(Vec::len), Some(3), "Delete removed the point, not the layer");
    c.ok("key", json!({"key": "Enter"}));
    assert_eq!(c.page_children().len(), 1);
    assert!(c.call("run_command", json!({"command": "vector.edit.state", "params": {}}))["isError"] == true, "Enter left edit mode");
}

#[test]
fn task_node_find_without_selecting_then_use_ids_in_batch() {
    let mut c = Client::new();
    let card = c.create(json!({"type": "frame", "name": "Card"}));
    let title = c.create(json!({"type": "text", "parent": card, "name": "Title", "text": "Hello"}));
    c.ok("select", json!({"ids": [card]}));
    let before = c.doc();
    assert_eq!(c.run("node.find", json!({"root": card, "type": "text", "text": "HELLO"})), json!({"ids": [title], "total": 1}));
    let after = c.doc();
    for key in ["selection", "currentPage", "history"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    let out = c.ok(
        "batch",
        json!({"commands": [
            {"command": "node.find", "params": {"root": card, "name": "title"}},
            {"command": "node.set", "params": {"ids": [{"$step": 0, "field": "ids/0"}], "props": {"name": "Found title"}}}
        ]}),
    );
    assert_eq!(out[0], json!({"ids": [title], "total": 1}));
    assert_eq!(c.node(&title)["name"], "Found title");
    assert_eq!(c.doc()["selection"], before["selection"]);
    c.run("edit.undo", json!({}));
    assert_eq!(c.node(&title)["name"], "Title");
}

/// A whole card in one `batch` call: later steps reach the frames earlier steps created through
/// `{"$step": n}`, so the tree is built in one round-trip and one undo step.
#[test]
fn task_card_in_one_batch() {
    let mut c = Client::new();
    let out = c.ok(
        "batch",
        json!({"label": "Card", "commands": [
            {"command": "node.create", "params": {"type": "frame", "name": "Card", "width": 320, "height": 50,
                "props": {"layout": "vertical", "padding": 24, "gap": 12, "sizingV": "hug"}}},
            {"command": "node.create", "params": {"type": "text", "parent": {"$step": 0}, "text": "Ada Lovelace"}},
            {"command": "node.create", "params": {"type": "frame", "parent": {"$step": 0}, "name": "Button", "height": 44,
                "props": {"layout": "horizontal", "sizingH": "fill"}}},
            {"command": "node.create", "params": {"type": "text", "parent": {"$step": 2}, "text": "Follow"}},
            {"command": "node.set", "params": {"ids": [{"$step": 2}], "props": {"fill": "#3B5BDB"}}},
        ]}),
    );
    let results = ids(&out);
    assert_eq!(results.len(), 5);
    let (card, button) = (results[0]["id"].clone(), results[2]["id"].clone());
    let n = c.node(&card);
    assert_eq!(ids(&n["children"]).len(), 2, "title and button are inside the card");
    let b = c.node(&button);
    assert_eq!(ids(&b["children"]).len(), 1, "the label is inside the button");
    assert_eq!(b["fills"][0]["color"].as_str().unwrap().to_ascii_uppercase(), "#3B5BDB");
    assert_eq!(c.doc()["history"]["undo"][0], "Card", "one undo step");
    // A forward reference fails the whole batch and names the step.
    let e = c.err(
        "batch",
        json!({"commands": [
            {"command": "node.create", "params": {"type": "frame", "parent": {"$step": 1}}},
            {"command": "node.create", "params": {"type": "frame"}},
        ]}),
    );
    assert!(e.contains("step 0") && e.contains("earlier step"), "{e}");
}

/// Named steps and a path into a list result: duplicate a frame, then rename the copy through
/// `{"$ref": "copy", "field": "ids/0"}` instead of counting steps.
#[test]
fn task_named_steps_and_a_duplicate_in_one_batch() {
    let mut c = Client::new();
    let out = c.ok(
        "batch",
        json!({"label": "Copy", "commands": [
            {"command": "node.create", "as": "card", "params": {"type": "frame", "name": "Card", "width": 100, "height": 50}},
            {"command": "edit.duplicate", "as": "copy", "params": {"ids": [{"$ref": "card"}]}},
            {"command": "node.set", "params": {"ids": [{"$ref": "copy", "field": "ids/0"}], "props": {"name": "Card copy"}}},
        ]}),
    );
    let results = ids(&out);
    let copy = results[1]["ids"][0].clone();
    assert_eq!(c.node(&copy)["name"], "Card copy");
    assert_eq!(c.node(&results[0]["id"])["name"], "Card", "the original keeps its name");
    assert_eq!(c.doc()["history"]["undo"][0], "Copy", "one undo step");
    // An unknown name fails the whole batch, names the step and lists the names there are.
    let e = c.err(
        "batch",
        json!({"commands": [
            {"command": "node.create", "as": "card", "params": {"type": "frame"}},
            {"command": "node.set", "params": {"ids": [{"$ref": "crad"}], "props": {"name": "x"}}},
        ]}),
    );
    assert!(e.contains("step 1") && e.contains("\"crad\" names no step") && e.contains("\"card\" is step 0"), "{e}");
}
