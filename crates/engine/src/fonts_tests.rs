use serde_json::json;

use super::*;
use crate::params::id_of;
use figcraft_doc::{Style, StyleRun, TextStyle};

fn text(s: &mut Session, family: &str) -> figcraft_doc::NodeId {
    let id = id_of(&s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    if let NodeKind::Text(t) = &mut s.doc_mut().unwrap().get_mut(id).unwrap().kind {
        t.style.font_family = family.into();
    }
    id
}

fn families(s: &mut Session) -> Vec<(String, u64)> {
    let r = s.execute("text.missingFonts", &json!({})).unwrap();
    r["families"].as_array().unwrap().iter().map(|f| (f["family"].as_str().unwrap().to_string(), f["layers"].as_u64().unwrap())).collect()
}

/// An installed family (built in, or found by the system scan), if this machine has any.
fn installed() -> Option<String> {
    figcraft_text::scan_system_fonts();
    figcraft_text::family_names().into_iter().next()
}

#[test]
fn lists_families_of_layers_ranges_styles_and_variables_once_each() {
    let mut s = Session::new();
    text(&mut s, "FigCraft Missing A");
    let b = text(&mut s, "figcraft missing a");
    if let NodeKind::Text(t) = &mut s.doc_mut().unwrap().get_mut(b).unwrap().kind {
        let style = TextStyle { font_family: "FigCraft Missing B".into(), ..t.style.clone() };
        t.runs.push(StyleRun { start: 0, end: 3, style });
    }
    let doc = s.doc_mut().unwrap();
    doc.styles.push(Style {
        id: "S:1".into(),
        name: "Heading".into(),
        description: String::new(),
        kind: StyleKind::Text { style: TextStyle { font_family: "FigCraft Missing C".into(), ..TextStyle::default() } },
        bound: Default::default(),
    });
    s.execute("variable.createCollection", &json!({"name": "Type", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Type", "name": "Family", "type": "string", "value": "FigCraft Missing D"})).unwrap();
    s.execute("variable.set", &json!({"variable": "Family", "mode": "Dark", "value": "FigCraft Missing H"})).unwrap();
    let var = s.doc().unwrap().variables.get("Family").unwrap().id.clone();
    s.doc_mut().unwrap().get_mut(b).unwrap().bound.insert("fontFamily".into(), var);

    assert_eq!(
        families(&mut s),
        vec![
            ("FigCraft Missing A".to_string(), 2),
            ("FigCraft Missing B".to_string(), 1),
            ("FigCraft Missing C".to_string(), 0),
            ("FigCraft Missing D".to_string(), 0),
            ("FigCraft Missing H".to_string(), 0),
        ]
    );
    assert!(s.ui_requests.iter().any(|r| r.kind == "dialog.missingFonts"));
}

#[test]
fn installed_families_are_not_listed() {
    let Some(family) = installed() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let mut s = Session::new();
    text(&mut s, &family);
    text(&mut s, &family.to_uppercase());
    text(&mut s, "FigCraft Missing E");
    assert_eq!(families(&mut s), vec![("FigCraft Missing E".to_string(), 1)]);
}

#[test]
fn a_family_found_only_by_the_system_scan_is_not_missing() {
    figcraft_text::scan_system_fonts();
    let embedded: Vec<String> = figcraft_text::CRAFT_FONTS.iter().map(|f| f.family.to_lowercase()).collect();
    let Some(family) = figcraft_text::family_names().into_iter().find(|f| !embedded.contains(&f.to_lowercase())) else {
        eprintln!("skipped: no system fonts on this machine");
        return;
    };
    let mut s = Session::new();
    text(&mut s, &family);
    assert!(families(&mut s).is_empty(), "{family} is installed");
}

#[test]
fn blank_families_and_aliases_are_handled() {
    let mut s = Session::new();
    text(&mut s, "  ");
    s.execute("variable.createCollection", &json!({"name": "Type"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Type", "name": "Base", "type": "string", "value": "FigCraft Missing F"})).unwrap();
    let base = s.doc().unwrap().variables.get("Base").unwrap().id.clone();
    let t = text(&mut s, "FigCraft Missing G");
    let doc = s.doc_mut().unwrap();
    // A variable that only aliases another, and one aliasing itself.
    let mut alias = doc.variables.get(&base).unwrap().clone();
    alias.id = "V:alias".into();
    alias.name = "Alias".into();
    for v in alias.values.values_mut() {
        *v = VariableValue::Alias(base.clone());
    }
    let mut cycle = alias.clone();
    cycle.id = "V:cycle".into();
    cycle.name = "Cycle".into();
    for v in cycle.values.values_mut() {
        *v = VariableValue::Alias("V:cycle".into());
    }
    doc.variables.variables.push(alias);
    doc.variables.variables.push(cycle);
    doc.get_mut(t).unwrap().bound.insert("fontFamily".into(), "V:alias".into());
    let other = text(&mut s, "FigCraft Missing G");
    s.doc_mut().unwrap().get_mut(other).unwrap().bound.insert("fontFamily".into(), "V:cycle".into());
    // The bound layer takes the value its alias resolves to (bindings are applied after the next
    // command); the cycle resolves to nothing.
    let NodeKind::Text(bound) = &s.doc().unwrap().node(t).unwrap().kind else { panic!() };
    assert_eq!(bound.style.font_family, "FigCraft Missing F");
    assert_eq!(families(&mut s), vec![("FigCraft Missing F".to_string(), 1), ("FigCraft Missing G".to_string(), 1)]);
}

#[test]
fn needs_an_open_document() {
    let mut s = Session::empty();
    assert!(s.execute("text.missingFonts", &json!({})).is_err());
}
