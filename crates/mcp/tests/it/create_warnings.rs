use crate::common::Client;
use serde_json::json;

#[test]
fn create_node_reports_a_font_style_drawn_in_another_face() {
    let mut c = Client::new();
    // The headless server indexes system fonts lazily; make sure that walk has happened.
    let _ = c.run("text.rescanFonts", json!({}));
    // Any installed family with a regular and a bold face and nothing heavier (CI machines differ).
    let names = c.run("text.fonts", json!({}));
    let pick = names.as_array().cloned().unwrap_or_default().into_iter().filter_map(|v| v.as_str().map(str::to_string)).find_map(|f| {
        let styles: Vec<String> = c
            .run("text.fonts", json!({"family": f}))
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let has = |w: u16| styles.iter().any(|x| figcraft_doc::style_weight(x) == w && !x.to_lowercase().contains("italic"));
        (has(400) && has(700) && !styles.iter().any(|x| figcraft_doc::style_weight(x) > 700)).then_some(f)
    });
    let Some(family) = pick else { return };
    let out = c.ok("create_node", json!({"type": "text", "text": "Hi", "props": {"fontFamily": family, "fontStyle": "Black"}}));
    let w = out["warnings"][0].as_str().unwrap();
    assert!(w.contains(&family) && w.contains("`Black`") && w.contains("nearest"), "{w}");
    // `set_properties` still says so too.
    let out = c.ok("set_properties", json!({"ids": [out["id"].clone()], "props": {"fontStyle": "Black"}}));
    let w = out["warnings"][0].as_str().unwrap();
    assert!(w.contains(&family) && w.contains("`Black`") && w.contains("nearest"), "{w}");
}
