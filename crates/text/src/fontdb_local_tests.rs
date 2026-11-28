//! Fonts installed on this computer that the browser named (#1051, Local Font Access): they join
//! the font list by name, list their styles without being read, and a font's bytes are asked for
//! once, only for the style text uses; when they come the face draws under the name it was
//! offered with, and one that can't be read is dropped.

use std::sync::Arc;

use super::{CRAFT_FONTS, Face, FontBytes, FontDb, LocalFont, add_local_fonts, load_local_font, local_only, take_wanted_local_fonts};

fn local(id: usize, family: &str, style: &str) -> LocalFont {
    LocalFont { id, family: family.into(), style: style.into(), postscript: format!("{}-{}", family.replace(' ', ""), style) }
}

fn offered() -> FontDb {
    let mut db = FontDb::default();
    let added = db.add_local(vec![
        local(0, "Shop Sans", "Bold"),
        local(1, "Shop Sans", "Regular"),
        local(2, "Shop Sans", "Light Italic"),
        local(3, "", "Regular"),
    ]);
    assert_eq!(added, ["Shop Sans"], "a font without a family name isn't offered");
    db
}

#[test]
fn local_fonts_are_listed_by_name_and_read_one_style_at_a_time() {
    let mut db = offered();
    assert_eq!(db.families(), ["Shop Sans"]);
    assert!(db.has_family("shop sans") && db.local_only("Shop Sans"));
    assert_eq!(db.postscript_face("ShopSans-Bold"), Some(("Shop Sans".into(), "Bold".into())));
    // Listing the styles reads nothing.
    assert_eq!(db.styles("Shop Sans"), ["Light Italic", "Regular", "Bold"]);
    assert!(db.wanted.is_empty());
    // Text in the family asks for the style it uses, once; nothing draws until it comes.
    assert!(db.find("Shop Sans", "Regular").is_none());
    assert!(db.find("Shop Sans", "Regular").is_none());
    assert!(db.find_one("Shop Sans", "Regular").is_none());
    assert_eq!(db.wanted, [1]);
    // The nearest style when there is no such name.
    db.find("Shop Sans", "Black");
    assert_eq!(db.wanted, [1, 0]);
    assert_eq!(db.styles("Shop Sans").len(), 3, "still offered while they're read");
}

#[test]
fn a_font_that_cant_be_read_is_dropped() {
    let mut db = offered();
    db.find("Shop Sans", "Bold");
    assert_eq!(db.load_local(0, b"not a font".to_vec()), None);
    assert_eq!(db.styles("Shop Sans"), ["Light Italic", "Regular"]);
    assert_eq!(db.load_local(0, Vec::new()), None, "only once");
    // Asked for again, it is.
    db.find("Shop Sans", "Light Italic");
    assert_eq!(db.wanted, [0, 2]);
}

#[test]
fn a_local_font_draws_under_the_name_it_was_offered_with() {
    // An embedded font stands in for the one on the computer (CI builds have craft-fonts).
    let Some(bytes) = CRAFT_FONTS.first().map(|f| f.bytes.to_vec()) else {
        eprintln!("skipping: no embedded font to stand in for an installed one");
        return;
    };
    let mut db = offered();
    db.find("Shop Sans", "Regular");
    assert_eq!(db.load_local(1, bytes.clone()).as_deref(), Some("Shop Sans"));
    let face = db.find("Shop Sans", "Regular").expect("the face");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Shop Sans", "Regular"));
    assert!(!db.local_only("Shop Sans"), "a face is read");
    assert_eq!(db.wanted, [1], "nothing more asked for");
    // The family keeps its other styles waiting.
    let styles = db.styles("Shop Sans");
    assert!(styles.iter().any(|s| s == "Bold") && styles.iter().any(|s| s == "Light Italic"), "{styles:?}");
    // A family the app has keeps its faces; the computer's styles it lacks wait beside them.
    let mut db = FontDb::default();
    let Some(mut face) = Face::from_bytes(FontBytes::Shared(Arc::new(bytes)), 0, Some("Brand Sans"), Some("Regular")) else { panic!("not a font") };
    // A variable stand-in's named instances aren't this family's styles.
    face.instances.clear();
    db.add(face);
    assert!(db.add_local(vec![local(5, "Brand Sans", "Regular"), local(6, "Brand Sans", "Bold")]).is_empty(), "not a new family");
    assert!(db.find("Brand Sans", "Regular").is_some() && db.wanted.is_empty());
    db.find("Brand Sans", "Bold");
    assert_eq!(db.wanted, [6]);
}

#[test]
fn the_global_db_takes_local_fonts() {
    let added = add_local_fonts(vec![local(900, "Global Local Sans", "Regular")]);
    assert_eq!(added, ["Global Local Sans"]);
    assert!(local_only("Global Local Sans") && !local_only("No Such Family"));
    assert!(super::find_face("Global Local Sans", "Regular").is_none_or(|f| f.family != "Global Local Sans"));
    assert!(take_wanted_local_fonts().contains(&900));
    assert!(!take_wanted_local_fonts().contains(&900), "taken once");
    assert_eq!(load_local_font(900, Vec::new()), None);
    assert!(!super::has_family("Global Local Sans"));
}
