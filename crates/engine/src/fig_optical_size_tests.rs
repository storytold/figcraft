//! A variable font's optical size follows the text size in Figma, also for a named style (the
//! glyph advances `.fig` files save show it: SF Pro Regular at 17 px is the 17 cut, at 20 px the
//! 20 cut). A `.fig` import carries the size as the text's `opsz` value, so FigCraft draws the
//! same cut instead of the named instance's.

use serde_json::json;

use crate::Session;
use figcraft_doc::NodeKind;

/// The `opsz` value of the text layer called `name`.
fn opsz(doc: &figcraft_doc::Document, name: &str) -> Option<f64> {
    let NodeKind::Text(td) = &doc.nodes().find(|n| n.name == name)?.kind else { return None };
    td.style.variations.get("opsz").copied()
}

/// macOS only: needs the system's variable SF font (an optical size axis and named instances).
#[test]
fn a_fig_import_sets_the_optical_size_to_the_text_size() {
    if !std::path::Path::new("/System/Library/Fonts/SFNS.ttf").exists() {
        return;
    }
    figcraft_text::fontdb::scan_system_fonts();
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    for (name, size, y) in [("Body", 17.0, 0.0), ("Title", 34.0, 40.0)] {
        let props = json!({"fontFamily": "SF Pro", "fontStyle": "Regular", "fontSize": size});
        s.execute("text.create", &json!({"text": "Handgloves", "x": 0, "y": y, "name": name, "props": props})).unwrap();
    }
    assert_eq!(opsz(s.doc().unwrap(), "Body"), None, "FigCraft's own text keeps the named instance's");
    let (bytes, _) = figcraft_format::fig::export(s.doc().unwrap(), None).unwrap();
    let back = crate::cmd::file::load_fig(&bytes, "x").unwrap().unwrap().doc;
    let f = figcraft_text::fontdb::find_face("SF Pro", "Regular").unwrap();
    if f.axis("opsz").is_none() || f.instance("Regular").is_none() {
        return;
    }
    assert_eq!(opsz(&back, "Body"), Some(17.0));
    assert_eq!(opsz(&back, "Title"), Some(34.0));
}
