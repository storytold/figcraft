//! Params the panels build at run time name only what their commands take: a name a command
//! doesn't list is refused (#1413), which would break the control that sent it.

use figcraft_doc::{ExportFormat, ExportSetting, NodeId, PrintSettings, VdCompat};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::{FigApp, NoServices};

fn app_with(cmd: &str, params: Value) -> (FigApp, NodeId) {
    let mut s = Session::new();
    let id = NodeId(s.execute(cmd, &params).unwrap()["id"].as_u64().unwrap());
    (FigApp::new(s, Box::new(NoServices)), id)
}

/// The Export section: a setting with every option off its default exports in every format, and
/// its edits (the setting's own keys, the PDF print options) apply.
#[test]
fn export_settings_with_every_option_are_taken() {
    let (mut app, id) = app_with("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    app.session.execute("export.add", &json!({"ids": [id]})).unwrap();
    let print = PrintSettings { bleed: 3.0, crop_marks: true, bleed_marks: true, cmyk: true };
    for format in ExportFormat::ALL {
        let setting = ExportSetting {
            format,
            print,
            contents_only: false,
            include_bounding_box: true,
            svg_outline_text: false,
            svg_id_attribute: true,
            svg_simplify_stroke: false,
            vd_compat: VdCompat::Exact,
            ..ExportSetting::default()
        };
        let mut p = crate::export_panel::setting_params(&setting);
        p["ids"] = json!([id]);
        app.session.execute("file.export", &p).unwrap_or_else(|e| panic!("file.export {p}: {e}"));
        p["index"] = json!(0);
        app.session.execute("export.update", &p).unwrap_or_else(|e| panic!("export.update {p}: {e}"));
    }
    let mut p = crate::export_panel::print_params(&print);
    p["ids"] = json!([id]);
    p["index"] = json!(0);
    app.session.execute("export.update", &p).unwrap_or_else(|e| panic!("export.update {p}: {e}"));
}

/// The Typography section on selected characters: every character property it sets goes through
/// `text.style`, and paragraph spacing (the whole layer's) through `node.set`.
#[test]
fn typography_edits_on_selected_characters_are_taken() {
    let (mut app, id) = app_with("text.create", json!({"text": "Typography", "x": 0, "y": 0}));
    let range = Some((id, 0, 4));
    for (key, v) in [
        ("fontFamily", json!("Inter")),
        ("fontStyle", json!("Bold")),
        ("fontSize", json!(20)),
        ("lineHeight", json!(24)),
        ("letterSpacing", json!(1)),
        ("textDecoration", json!("underline")),
        ("textDecorationStyle", json!("wavy")),
        ("textDecorationThickness", json!(2)),
        ("textDecorationOffset", json!(3)),
        ("textDecorationSkipInk", json!(false)),
        ("textDecorationColor", json!("#FF0000")),
        ("textCase", json!("upper")),
    ] {
        crate::typography::commit_text(&mut app, &[id], range, json!({ key: v }));
        assert_eq!(app.ui.last_error, None, "{key}");
    }
    // A named instance of a variable font: its style, and the custom axis values cleared.
    crate::typography::commit_text(&mut app, &[id], range, json!({"fontStyle": "Regular", "fontVariations": null}));
    assert_eq!(app.ui.last_error, None, "fontVariations");
    crate::typography::commit_text(&mut app, &[id], range, json!({"paragraphSpacing": 12}));
    assert_eq!(app.ui.last_error, None, "paragraphSpacing");
    let spacing = app.session.doc().unwrap().node(id).and_then(|n| n.kind.text()).map(|t| t.paragraph_spacing);
    assert_eq!(spacing, Some(12.0));
}
