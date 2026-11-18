//! Dev-mode inspection: properties grouped with their styles and variables, positions relative
//! to the parent, and redlines on hover without Alt.

use serde_json::{Value, json};

use crate::Session;
use crate::devinspect::{Section, properties};
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};

fn id(v: Value) -> figcraft_doc::NodeId {
    id_of(&v["id"]).unwrap()
}

fn row<'a>(sections: &'a [Section], title: &str, label: &str) -> &'a crate::devinspect::Prop {
    sections.iter().find(|s| s.title == title).and_then(|s| s.rows.iter().find(|r| r.label == label)).unwrap_or_else(|| panic!("{title}/{label}"))
}

#[test]
fn properties_group_layout_auto_layout_paints_and_tokens() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 100, "y": 50, "width": 300, "height": 200})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 130, "y": 90, "width": 80, "height": 40, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": r, "props": {"fill": "#0D99FF80", "cornerRadius": 6, "opacity": 0.5}})).unwrap();
    s.execute("variable.createCollection", &json!({"name": "T"})).unwrap();
    s.execute("variable.create", &json!({"collection": "T", "name": "radius/sm", "type": "number", "value": 6})).unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "cornerRadius", "variable": "radius/sm"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("style.create", &json!({"type": "paint", "name": "Brand/Blue"})).unwrap();
    s.execute("style.apply", &json!({"ids": [r], "style": "Brand/Blue"})).unwrap();
    let doc = s.doc().unwrap();
    let p = properties(doc, r);
    assert_eq!(row(&p, "Layout", "Width").value, "80");
    // Top/left from the parent frame's corner (the rectangle was placed in the frame's coordinates).
    assert_eq!((row(&p, "Layout", "Top").value.as_str(), row(&p, "Layout", "Left").value.as_str()), ("90", "130"));
    assert_eq!(row(&p, "Layout", "Opacity").value, "50%");
    let radius = row(&p, "Layout", "Radius");
    assert_eq!((radius.value.as_str(), radius.token.as_deref()), ("6", Some("radius/sm")));
    let fill = &p.iter().find(|s| s.title == "Fills").unwrap().rows[0];
    assert_eq!(fill.value, "#0D99FF · 50%");
    assert_eq!(fill.color.as_deref(), Some("#0D99FF80"));
    assert_eq!(fill.token.as_deref(), Some("Brand/Blue"));
    assert!(p.iter().all(|s| s.title != "Auto layout" && s.title != "Typography"));

    // Auto layout: direction, gap, padding shorthand, alignment.
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute("node.set", &json!({"id": f, "props": {"layoutMode": "vertical", "gap": 12, "padding": [8, 16, 8, 16]}})).unwrap();
    let p = properties(s.doc().unwrap(), f);
    assert_eq!(row(&p, "Auto layout", "Direction").value, "Vertical");
    assert_eq!(row(&p, "Auto layout", "Gap").value, "12");
    assert_eq!(row(&p, "Auto layout", "Padding").value, "8 16");
    // At the top level, positions are from the page origin.
    assert_eq!(row(&p, "Layout", "Left").value, "100");

    // The command returns the same, for MCP.
    let out = s.execute("inspect.properties", &json!({"id": r})).unwrap();
    assert_eq!(out["sections"][0]["title"], "Layout");
    assert!(s.execute("inspect.properties", &json!({"id": 999_999})).is_err());
}

#[test]
fn stored_variable_ids_keep_their_dev_inspect_token() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    let shadow = s.execute("variable.create", &json!({"collection": "Theme", "name": "shadow", "type": "color", "value": "#FF0000"})).unwrap();
    let surface = s.execute("variable.create", &json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF"})).unwrap();
    let r = id(s.execute("node.create", &json!({"type": "rectangle", "props": {"fill": {"$var": surface["id"]}}})).unwrap());
    s.execute("variable.update", &json!({"variable": "surface", "codeSyntax": {"WEB": "--real-surface"}})).unwrap();
    s.execute("variable.update", &json!({"variable": shadow["id"], "name": surface["id"], "codeSyntax": {"WEB": "--shadow"}})).unwrap();

    let code = s.execute("inspect.code", &json!({"id": r, "language": "css"})).unwrap();
    assert!(code["code"].as_str().unwrap().contains("var(--real-surface"));
    assert!(!code["code"].as_str().unwrap().contains("--shadow"));
    let out = s.execute("inspect.properties", &json!({"id": r})).unwrap();
    let fills = out["sections"].as_array().unwrap().iter().find(|s| s["title"] == "Fills").unwrap();
    assert_eq!(fills["rows"][0]["color"], "#0D99FF");
    assert_eq!(fills["rows"][0]["token"], "surface");
}

#[test]
fn typography_section_for_text() {
    let mut s = Session::new();
    let t = id(s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap());
    s.execute("node.set", &json!({"id": t, "props": {"fontSize": 18, "textAlignHorizontal": "center"}})).unwrap();
    let p = properties(s.doc().unwrap(), t);
    assert_eq!(row(&p, "Typography", "Size").value, "18");
    assert!(p.iter().any(|s| s.title == "Typography"));
}

#[test]
fn redlines_measure_on_hover_without_alt() {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 50, "height": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let hover = |s: &mut Session| {
        s.pointer(&PointerEvent::at(PointerKind::Move, 125.0, 25.0, 1.0)).unwrap();
        s.tool.overlay.guides.len()
    };
    assert_eq!(hover(&mut s), 0, "plain hover doesn't measure");
    s.tool.dev_mode = true;
    assert!(hover(&mut s) > 0, "Inspect measures on hover");
}

#[test]
fn assets_list_layers_with_export_settings_under_the_selection() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let icon = id(s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 20, "height": 20, "parent": f})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 40, "y": 10, "width": 20, "height": 20, "parent": f})).unwrap();
    s.execute("export.add", &json!({"ids": [icon]})).unwrap();
    s.execute("export.add", &json!({"ids": [icon]})).unwrap();
    let doc = s.doc().unwrap();
    let a = crate::devinspect::assets(doc, &[f]);
    assert_eq!(a.len(), 1, "only the layer with settings");
    assert_eq!(a[0].id, icon);
    assert_eq!(a[0].exports.len(), 2);
    assert!(a[0].exports[0].ends_with("1x"), "{:?}", a[0].exports);
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let out = s.execute("inspect.assets", &json!({})).unwrap();
    assert_eq!(out["assets"][0]["name"], "Ellipse");
    assert!(crate::devinspect::assets(s.doc().unwrap(), &[]).is_empty());
}
