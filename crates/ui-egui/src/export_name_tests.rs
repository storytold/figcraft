//! The Export section's button names the layer it exports on one line inside the button: a long
//! name ends with an ellipsis instead of wrapping into a block painted over the rest of the panel
//! (#1650), at the default panel width and wider.

use egui::{Rect, Shape};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::widgets::FIELD_H;

/// The text painted this frame, with each piece's box.
fn painted(u: &mut Ui) -> Vec<(String, Rect)> {
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    u.frame(vec![]);
    let frame = u.frame_output(vec![]);
    let mut out = Vec::new();
    for c in &frame.shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// A frame named `name` with an export setting, selected, in a right panel `width` wide: the export
/// button's label and box, and the panel's box.
fn export_label(name: &str, width: f32) -> (String, Rect, Rect) {
    let mut s = Session::new();
    let f = crate::present_tests::id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 856, "height": 1089})).unwrap());
    s.execute("export.add", &json!({"ids": [f]})).unwrap();
    s.execute("node.set", &json!({"id": f, "props": {"name": name}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_width = width;
    let texts = painted(&mut u);
    let panel = u.app.ui.right_panel_rect.expect("the right panel shows");
    assert!((panel.width() - width).abs() <= 1.0, "a {width} px panel (less its edge), not {panel:?}");
    let buttons: Vec<(String, Rect)> = texts.into_iter().filter(|(t, r)| t.starts_with("Export ") && r.min.x >= panel.min.x).collect();
    let [(text, rect)] = buttons.as_slice() else { panic!("one export button: {buttons:?}") };
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().name, name, "the name itself stays whole");
    (text.clone(), *rect, panel)
}

#[test]
fn a_long_name_stays_on_one_line_inside_the_export_button() {
    let long = "FRAME_NAME_OVERFLOW_".repeat(200);
    for width in [240.0, 400.0] {
        let (text, rect, panel) = export_label(&long, width);
        assert!(text.ends_with('…'), "{width}: shortened, not {} characters", text.chars().count());
        assert!(rect.min.x >= panel.min.x && rect.max.x <= panel.max.x, "{width}: {rect:?} inside {panel:?}");
        assert!(rect.height() <= FIELD_H, "{width}: one line, not {} tall", rect.height());
    }
}

/// Dev Mode's header names the layer on one line inside the panel too.
#[test]
fn dev_modes_header_shortens_a_long_name() {
    let long = "FRAME_NAME_OVERFLOW_".repeat(200);
    let mut s = Session::new();
    let f = crate::present_tests::id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 856, "height": 1089})).unwrap());
    s.execute("node.set", &json!({"id": f, "props": {"name": long}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Inspect;
    let texts = painted(&mut u);
    let panel = u.app.ui.right_panel_rect.expect("the right panel shows");
    let names: Vec<(String, Rect)> = texts.into_iter().filter(|(t, r)| t.starts_with("FRAME_NAME") && r.min.x >= panel.min.x).collect();
    assert!(!names.is_empty(), "the header names the layer");
    for (text, rect) in names {
        assert!(text.ends_with('…'), "shortened: {} characters", text.chars().count());
        assert!(rect.max.x <= panel.max.x, "{rect:?} inside {panel:?}");
    }
}

/// Eliding as it was done before: every candidate from the longest down, the first that fits.
fn elide_by_trying_all(ui: &egui::Ui, s: &str, max_w: f32, font: egui::FontId, from_start: bool) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE).size().x <= max_w;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| {
            if from_start {
                format!("…{}", chars[chars.len() - n..].iter().collect::<String>().trim_start())
            } else {
                format!("{}…", chars.iter().take(n).collect::<String>().trim_end())
            }
        })
        .find(|c| fits(c))
        .unwrap_or_else(|| "…".into())
}

/// Eliding halves its way to the longest candidate that fits (a long name no longer lays out one
/// candidate per character every frame) and finds what trying every candidate finds.
#[test]
fn eliding_finds_what_trying_every_candidate_finds() {
    use crate::theme::{regular, semibold};
    let texts = [
        "FRAME_NAME_OVERFLOW_".repeat(20),
        "A name  with   spaces    in it".to_string(),
        "W".repeat(40),
        "i".repeat(60),
        "Ünïcödé names — with “quotes” and 漢字 too".to_string(),
    ];
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        for s in &texts {
            for w in [0.0, 5.0, 12.0, 40.0, 97.5, 160.0, 400.0, 2000.0] {
                for font in [regular(11.0), semibold(13.0)] {
                    let end = crate::widgets::elide_font(ui, s, w, font.clone());
                    assert_eq!(end, elide_by_trying_all(ui, s, w, font.clone(), false), "{s:?} in {w}");
                    let start = crate::widgets::elide_start_font(ui, s, w, font.clone());
                    assert_eq!(start, elide_by_trying_all(ui, s, w, font, true), "{s:?} from its start in {w}");
                }
            }
        }
    });
    out.textures_delta.clear();
}

#[test]
fn line_breaks_in_a_name_show_as_spaces_and_short_names_in_full() {
    let (text, rect, _) = export_label("First line\nsecond line", 240.0);
    assert_eq!(text, "Export First line second line");
    assert!(rect.height() <= FIELD_H);
    assert_eq!(export_label("Frame 1", 240.0).0, "Export Frame 1");
}
