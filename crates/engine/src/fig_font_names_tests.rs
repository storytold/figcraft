//! A `.fig` file whose text names an installed family another way than the font list does (#1292)
//! isn't reported missing and draws in that family: by the legacy name of one of its weights
//! ("<Family> Bold", style Regular) or spelt with other case and spaces. Through the real import
//! (a file the app exported), with an embedded family standing in for an installed one.

use base64::Engine as _;
use figcraft_doc::NodeKind;
use serde_json::json;

use crate::Session;

fn text(s: &mut Session, family: &str) {
    let id = s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 0})).unwrap()["id"].as_u64().unwrap();
    if let NodeKind::Text(t) = &mut s.doc_mut().unwrap().get_mut(figcraft_doc::NodeId(id)).unwrap().kind {
        t.style.font_family = family.into();
    }
}

#[test]
fn an_imported_file_finds_installed_families_by_their_other_names() {
    let Some(family) = figcraft_text::family_names().into_iter().find(|f| figcraft_text::family_styles(f).iter().any(|s| s == "Bold")) else {
        return;
    };
    let legacy = format!("{family} Bold");
    let squashed = family.to_uppercase().replace(' ', "");
    let mut s = Session::new();
    text(&mut s, &legacy);
    text(&mut s, &squashed);
    text(&mut s, "FigCraft Surely Missing");
    let r = s.execute("file.export", &json!({"format": "FIG"})).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    let (doc, _) = crate::cmd::file::load_bytes(&bytes, "names.fig").unwrap();
    let missing: Vec<String> = crate::fonts::missing(&doc).into_iter().map(|m| m.family).collect();
    assert_eq!(missing, ["FigCraft Surely Missing"], "{family}");
    let bold = figcraft_text::find_face(&legacy, "Regular").unwrap();
    assert_eq!((bold.family.to_lowercase(), bold.weight), (family.to_lowercase(), 700));
    assert_eq!(figcraft_text::find_face(&squashed, "Bold").unwrap().family.to_lowercase(), family.to_lowercase());
}
