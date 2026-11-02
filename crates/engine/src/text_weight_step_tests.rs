//! Increase and decrease font weight step through the styles the family has, by weight with the
//! same slant and width (#1831), not by 100 to a canonical name no face provides.

use figcraft_doc::{NodeId, TextStyle};
use serde_json::json;

use crate::Session;
use crate::cmd::text::next_style;

fn styles(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

fn at(style: &str) -> TextStyle {
    let mut st = TextStyle::default();
    st.set_font_style(style);
    st
}

/// Walk `from` heavier (`up`) or lighter until the family has nothing further.
fn walk(family: &[String], from: &str, up: bool) -> Vec<String> {
    let mut st = at(from);
    let mut out = Vec::new();
    while let Some(next) = next_style(family, &st, up) {
        st.set_font_style(next.clone());
        out.push(next);
    }
    out
}

#[test]
fn the_steps_follow_the_family_with_the_same_slant_and_width() {
    let arial = styles(&["Narrow", "Regular", "Italic", "Narrow Italic", "Bold", "Narrow Bold", "Bold Italic", "Narrow Bold Italic", "Black"]);
    assert_eq!(walk(&arial, "Regular", true), ["Bold", "Black"]);
    assert_eq!(walk(&arial, "Black", false), ["Bold", "Regular"]);
    assert_eq!(walk(&arial, "Italic", true), ["Bold Italic"]);
    assert_eq!(walk(&arial, "Narrow", true), ["Narrow Bold"]);
    assert_eq!(walk(&arial, "Narrow Bold Italic", false), ["Narrow Italic"]);
    let segoe = styles(&["Light", "Semilight", "Regular", "Italic", "Semibold", "Bold", "Black", "Bold Italic"]);
    assert_eq!(walk(&segoe, "Regular", true), ["Semibold", "Bold", "Black"]);
    assert_eq!(walk(&segoe, "Regular", false), ["Light"]);
    // A style the family doesn't have (as the old steps stored) moves to the next real one.
    assert_eq!(next_style(&segoe, &at("Medium"), true).as_deref(), Some("Semibold"));
}

#[test]
fn a_custom_weight_axis_value_steps_from_where_it_is() {
    let family = styles(&["Light", "Regular", "Medium", "Bold"]);
    let mut st = at("Regular");
    st.variations.insert("wght".into(), 550.0);
    assert_eq!(next_style(&family, &st, true).as_deref(), Some("Bold"));
    assert_eq!(next_style(&family, &st, false).as_deref(), Some("Medium"));
}

fn font_style(s: &Session, t: NodeId) -> String {
    s.doc().unwrap().node(t).unwrap().kind.text().unwrap().spans()[0].2.font_style.clone()
}

#[test]
fn the_commands_land_on_styles_the_family_has() {
    figcraft_text::scan_system_fonts();
    // Any installed family with a regular face and a heavier upright one (CI machines differ).
    let upright = |x: &String| !x.to_lowercase().contains("italic") && !x.to_lowercase().contains("oblique");
    let pick = figcraft_text::family_names().into_iter().find_map(|f| {
        let st = figcraft_text::family_styles(&f);
        (st.iter().any(|x| x == "Regular") && st.iter().any(|x| upright(x) && figcraft_doc::style_weight(x) > 400)).then_some((f, st))
    });
    let Some((family, styles)) = pick else { return };
    let mut s = Session::new();
    let r = s.execute("node.create", &json!({"type": "text", "text": "Hi", "props": {"fontFamily": family, "fontStyle": "Regular"}})).unwrap();
    let t = crate::params::id_of(&r["id"]).unwrap();
    s.execute("text.weightUp", &json!({"ids": [t]})).unwrap();
    let up = font_style(&s, t);
    assert!(styles.contains(&up) && figcraft_doc::style_weight(&up) > 400, "{family}: {up} of {styles:?}");
    s.execute("text.weightDown", &json!({"ids": [t]})).unwrap();
    assert_eq!(font_style(&s, t), "Regular", "{family}: {styles:?}");
    // A family that isn't installed still steps by 100 under the canonical names.
    s.execute("node.set", &json!({"ids": [t], "props": {"fontFamily": "No Such Family 7f3a", "fontStyle": "Regular"}})).unwrap();
    s.execute("text.weightUp", &json!({"ids": [t]})).unwrap();
    assert_eq!(font_style(&s, t), "Medium");
}
