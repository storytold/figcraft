#![allow(clippy::unwrap_used)]

use figcraft_color::Paint;
use figcraft_doc::NodeId;
use figcraft_engine::{
    Session,
    selection_colors::{SelectionColor, collect},
};
use serde_json::{Value, json};

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap()
}
fn rectangle(s: &mut Session, color: &str) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"width":40,"height":40,"props":{"fill":color}}))["id"].as_u64().unwrap())
}
fn group(s: &Session, ids: &[NodeId], hex: &str) -> SelectionColor {
    collect(s.doc().unwrap(), ids).into_iter().find(|g| g.paint.solid_color().is_some_and(|c| c.to_hex() == hex)).unwrap()
}
fn fill(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex()
}

#[test]
fn aggregate_descendants_deduplicate_roots_and_ignore_images_and_hidden_paints() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#00ff00");
    run(&mut s, "stroke.add", json!({"id":b,"color":"#ff0000"}));
    let parent = NodeId(run(&mut s, "object.group", json!({"ids":[a,b]}))["id"].as_u64().unwrap());
    let colors = collect(s.doc().unwrap(), &[parent, a]);
    assert_eq!(colors.len(), 2);
    assert_eq!(group(&s, &[parent, a], "FF0000").targets.len(), 2);
    run(&mut s, "fill.add", json!({"id":a,"paint":Paint::image("synthetic",figcraft_color::ImageScaleMode::Fill)}));
    run(&mut s, "fill.add", json!({"id":a,"paint":{"type":"solid","color":"#0000ff","visible":false}}));
    assert_eq!(collect(s.doc().unwrap(), &[parent]).len(), 2);
    run(&mut s, "select.set", json!({"ids":[parent]}));
    assert_eq!(run(&mut s, "selection.colors", json!({}))["colors"].as_array().unwrap().len(), 2);
}

#[test]
fn editing_only_matching_slots_is_one_undo_step_and_keeps_selection() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#0000ff");
    let c = rectangle(&mut s, "#00ff00");
    run(&mut s, "stroke.add", json!({"id":b,"color":"#ff0000"}));
    run(&mut s, "select.set", json!({"ids":[a,b]}));
    let red = group(&s, &[a, b], "FF0000");
    let before = s.undo_depth();
    run(&mut s, "selection.setPaint", json!({"targets":red.targets,"paint":"#ffff00"}));
    assert_eq!(fill(&s, a), "FFFF00");
    assert_eq!(fill(&s, b), "0000FF");
    assert_eq!(fill(&s, c), "00FF00");
    assert_eq!(s.doc().unwrap().node(b).unwrap().strokes[0].solid_color().unwrap().to_hex(), "FFFF00");
    assert_eq!(s.selection(), vec![a, b]);
    assert_eq!(s.undo_depth().0, before.0 + 1);
    s.undo().unwrap();
    assert_eq!(fill(&s, a), "FF0000");
    s.redo().unwrap();
    assert_eq!(fill(&s, a), "FFFF00");
}

#[test]
fn live_picker_targets_do_not_expand_when_colors_merge() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#0000ff");
    let red = group(&s, &[a, b], "FF0000");
    let before = s.undo_depth();
    s.begin_interaction("Color").unwrap();
    for hex in ["#0000ff", "#00ff00"] {
        run(&mut s, "selection.setPaint", json!({"targets":red.targets,"paint":hex}));
    }
    s.commit_interaction().unwrap();
    assert_eq!(fill(&s, a), "00FF00");
    assert_eq!(fill(&s, b), "0000FF");
    assert_eq!(s.undo_depth().0, before.0 + 1);
    s.undo().unwrap();
    assert_eq!(fill(&s, a), "FF0000");
}

#[test]
fn text_range_colors_use_character_offsets_and_preserve_typography() {
    let mut s = Session::new();
    let id = NodeId(run(&mut s, "text.create", json!({"text":"Яab","props":{"fill":"#ff0000"}}))["id"].as_u64().unwrap());
    run(&mut s, "text.style", json!({"id":id,"start":1,"end":2,"fontSize":30,"textFills":["#0000ff"]}));
    let blue = group(&s, &[id], "0000FF");
    assert_eq!(blue.targets[0].range, Some((1, 2)));
    run(&mut s, "selection.setPaint", json!({"targets":blue.targets,"paint":"#00ff00"}));
    let n = s.doc().unwrap().node(id).unwrap();
    let t = n.kind.text().unwrap();
    assert_eq!(t.characters, "Яab");
    assert_eq!(t.style_at(1).font_size, 30.0);
    assert_eq!(t.style_at(1).fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(), "00FF00");
    assert_eq!(t.style_at(0).fills.as_deref().unwrap_or(&n.fills)[0].solid_color().unwrap().to_hex(), "FF0000");
    assert_eq!(t.style_at(2).fills.as_deref().unwrap_or(&n.fills)[0].solid_color().unwrap().to_hex(), "FF0000");
}

#[test]
fn selecting_color_selects_its_unique_owners_without_document_history() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#0000ff");
    run(&mut s, "stroke.add", json!({"id":a,"color":"#ff0000"}));
    let red = group(&s, &[a, b], "FF0000");
    let before = s.undo_depth();
    run(&mut s, "selection.selectColor", json!({"targets":red.targets}));
    assert_eq!(s.selection(), vec![a]);
    assert_eq!(s.undo_depth(), before);
}

#[test]
fn applying_color_styles_links_matching_fill_and_stroke_lists_only() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#0000ff");
    run(&mut s, "stroke.add", json!({"id":b,"color":"#ff0000"}));
    let red = group(&s, &[a, b], "FF0000");
    let style = run(&mut s, "style.create", json!({"type":"paint","name":"Brand","value":"#00ff00"}))["id"].clone();
    let before = s.undo_depth();
    run(&mut s, "selection.applyColorStyle", json!({"targets":red.targets,"style":style}));
    assert_eq!(fill(&s, a), "00FF00");
    assert_eq!(fill(&s, b), "0000FF");
    assert_eq!(s.doc().unwrap().node(a).unwrap().styles.fill.as_deref(), style.as_str());
    assert_eq!(s.doc().unwrap().node(b).unwrap().styles.stroke.as_deref(), style.as_str());
    assert_eq!(s.undo_depth().0, before.0 + 1);
    run(&mut s, "style.update", json!({"style":style,"value":"#ffff00"}));
    assert_eq!(fill(&s, a), "FFFF00");
    assert_eq!(fill(&s, b), "0000FF");
}

#[test]
fn explicit_color_detaches_its_binding_but_keeps_other_bindings() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    run(&mut s, "variable.createCollection", json!({"name":"Theme"}));
    let v = run(&mut s, "variable.create", json!({"collection":"Theme","name":"Brand","type":"color","value":"#ff0000"}))["id"].clone();
    run(&mut s, "variable.bind", json!({"id":a,"field":"fill","variable":v}));
    let red = group(&s, &[a], "FF0000");
    run(&mut s, "selection.setPaint", json!({"targets":red.targets,"paint":"#0000ff"}));
    assert_eq!(fill(&s, a), "0000FF");
    assert!(s.doc().unwrap().node(a).unwrap().bound.is_empty());
    s.undo().unwrap();
    assert_eq!(fill(&s, a), "FF0000");
    assert!(!s.doc().unwrap().node(a).unwrap().bound.is_empty());
}

#[test]
fn invalid_or_stale_targets_roll_back_atomically() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let mut red = group(&s, &[a], "FF0000");
    let mut bad = red.targets[0].clone();
    bad.index = 999;
    red.targets.push(bad);
    let before = s.undo_depth();
    assert!(s.execute("selection.setPaint", &json!({"targets":red.targets,"paint":"#0000ff"})).is_err());
    assert_eq!(fill(&s, a), "FF0000");
    assert_eq!(s.undo_depth(), before);
    for command in ["selection.setPaint", "selection.selectColor", "selection.applyColorStyle"] {
        for params in [json!({}), json!({"targets":[]}), json!({"targets":[{"id":a,"fills":true,"index":0,"range":[0,1]}]})] {
            assert!(s.execute(command, &params).is_err());
        }
    }
}

#[test]
fn equal_resolved_colors_from_different_styles_or_variables_stay_separate() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#ff0000");
    let c = rectangle(&mut s, "#ff0000");
    let style = run(&mut s, "style.create", json!({"type":"paint","name":"Brand","value":"#ff0000"}))["id"].clone();
    run(&mut s, "style.apply", json!({"id":a,"style":style,"target":"fill"}));
    run(&mut s, "variable.createCollection", json!({"name":"Theme"}));
    let variable = run(&mut s, "variable.create", json!({"collection":"Theme","name":"Accent","type":"color","value":"#ff0000"}))["id"].clone();
    run(&mut s, "variable.bind", json!({"id":b,"field":"fill","variable":variable}));
    let colors = collect(s.doc().unwrap(), &[a, b, c]);
    assert_eq!(colors.len(), 3);
    assert_eq!(colors[0].style.as_deref(), style.as_str());
    assert_eq!(colors[1].variable.as_deref(), variable.as_str());
    assert!(colors[2].style.is_none() && colors[2].variable.is_none());
}

#[test]
fn gradients_and_multiple_paints_are_edited_without_replacing_other_slots() {
    use figcraft_color::{Color, Gradient, GradientKind};
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let paint = Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::BLACK));
    run(&mut s, "fill.add", json!({"id":a,"paint":paint}));
    let colors = collect(s.doc().unwrap(), &[a]);
    assert_eq!(colors.len(), 2);
    let gradient = colors.iter().find(|g| g.paint == paint).unwrap();
    run(&mut s, "selection.setPaint", json!({"targets":gradient.targets,"paint":"#00ff00"}));
    let paints = &s.doc().unwrap().node(a).unwrap().fills;
    assert_eq!(paints.len(), 2);
    assert_eq!(paints[0].solid_color().unwrap().to_hex(), "FF0000");
    assert_eq!(paints[1].solid_color().unwrap().to_hex(), "00FF00");
}

#[test]
fn masks_and_boolean_operands_are_not_selection_colors() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    let b = rectangle(&mut s, "#00ff00");
    let boolean = NodeId(run(&mut s, "bool.union", json!({"ids":[a,b]}))["id"].as_u64().unwrap());
    run(&mut s, "fill.set", json!({"id":boolean,"color":"#0000ff"}));
    let colors = collect(s.doc().unwrap(), &[boolean]);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0].paint.solid_color().unwrap().to_hex(), "0000FF");
    s.doc_mut().unwrap().get_mut(boolean).unwrap().is_mask = true;
    assert!(collect(s.doc().unwrap(), &[boolean]).is_empty());
}

#[test]
fn range_edit_keeps_a_node_binding_that_other_characters_inherit() {
    let mut s = Session::new();
    let id = NodeId(run(&mut s, "text.create", json!({"text":"abc","props":{"fill":"#ff0000"}}))["id"].as_u64().unwrap());
    run(&mut s, "variable.createCollection", json!({"name":"Theme"}));
    let v = run(&mut s, "variable.create", json!({"collection":"Theme","name":"Brand","type":"color","value":"#ff0000"}))["id"].clone();
    run(&mut s, "variable.bind", json!({"id":id,"field":"fill","variable":v}));
    run(&mut s, "text.style", json!({"id":id,"start":1,"end":2,"textFills":["#0000ff"]}));
    let blue = group(&s, &[id], "0000FF");
    run(&mut s, "selection.setPaint", json!({"targets":blue.targets,"paint":"#00ff00"}));
    assert!(!s.doc().unwrap().node(id).unwrap().bound.is_empty());
    assert_eq!(fill(&s, id), "FF0000");
    assert_eq!(
        s.doc().unwrap().node(id).unwrap().kind.text().unwrap().style_at(1).fills.as_ref().unwrap()[0].solid_color().unwrap().to_hex(),
        "00FF00"
    );
}

#[test]
fn selection_color_edits_respect_read_only_version_previews() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    run(&mut s, "file.saveVersion", json!({"name":"Before"}));
    run(&mut s, "version.preview", json!({"index":0}));
    let red = group(&s, &[a], "FF0000");
    let before = s.undo_depth();
    assert!(s.execute("selection.setPaint", &json!({"targets":red.targets,"paint":"#0000ff"})).is_err());
    assert_eq!(fill(&s, a), "FF0000");
    assert_eq!(s.undo_depth(), before);
}

#[test]
fn opacity_edits_keep_the_color_variable_binding() {
    let mut s = Session::new();
    let a = rectangle(&mut s, "#ff0000");
    run(&mut s, "variable.createCollection", json!({"name":"Theme"}));
    let v = run(&mut s, "variable.create", json!({"collection":"Theme","name":"Brand","type":"color","value":"#ff0000"}))["id"].clone();
    run(&mut s, "variable.bind", json!({"id":a,"field":"fill","variable":v}));
    let mut red = group(&s, &[a], "FF0000");
    red.paint.opacity = 0.5;
    run(&mut s, "selection.setPaint", json!({"targets":red.targets,"paint":red.paint}));
    let n = s.doc().unwrap().node(a).unwrap();
    assert_eq!(n.fills[0].opacity, 0.5);
    assert!(!n.bound.is_empty());
}

#[test]
fn whole_text_fill_keeps_a_color_style_link() {
    let mut s = Session::new();
    let id = NodeId(run(&mut s, "text.create", json!({"text":"Text","props":{"fill":"#ff0000"}}))["id"].as_u64().unwrap());
    let red = group(&s, &[id], "FF0000");
    assert_eq!(red.targets[0].range, None);
    let style = run(&mut s, "style.create", json!({"type":"paint","name":"Brand","value":"#0000ff"}))["id"].clone();
    run(&mut s, "selection.applyColorStyle", json!({"targets":red.targets,"style":style}));
    assert_eq!(s.doc().unwrap().node(id).unwrap().styles.fill.as_deref(), style.as_str());
    run(&mut s, "style.update", json!({"style":style,"value":"#00ff00"}));
    assert_eq!(fill(&s, id), "00FF00");
}
