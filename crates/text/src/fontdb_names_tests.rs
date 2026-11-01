//! A family is found however a design names it (#1292): fonts name a family by its typographic
//! name ("Economia", style "Medium") and by a legacy name per weight ("Economia Medium", style
//! "Regular"), and fonts without typographic names are installed under the legacy ones only. A
//! design (an imported file, a paste) may use either, or spell the family with other spaces or
//! case; the picker lists what is installed. Synthetic family names only, over pending entries
//! (no files) and the embedded faces when the build has them.

use std::sync::Arc;

use super::{CRAFT_FONTS, Face, FontBytes, FontDb, Pending, style_words};

fn entry(style: &str) -> Pending {
    Pending { path: format!("no-such-dir/{style}.ttf").into(), index: 0, style: style.into(), postscript: None, local: None }
}

fn installed(families: &[(&str, &[&str])]) -> FontDb {
    let mut db = FontDb::default();
    for (family, styles) in families {
        db.pending.insert(family.to_lowercase(), ((*family).into(), styles.iter().map(|s| entry(s)).collect()));
    }
    db
}

fn resolved(db: &FontDb, family: &str, style: &str) -> (String, String) {
    db.resolve(family, style)
}

fn pair(family: &str, style: &str) -> (String, String) {
    (family.into(), style.into())
}

#[test]
fn a_legacy_family_name_finds_the_typographic_family() {
    // The fonts have typographic names: one family with every weight.
    let db = installed(&[("Synthetic Grotesk", &["Regular", "Medium", "SemiBold", "Bold", "SemiBold Italic"])]);
    assert_eq!(resolved(&db, "Synthetic Grotesk Medium", "Regular"), pair("synthetic grotesk", "Medium"));
    assert_eq!(resolved(&db, "Synthetic Grotesk SemiBold", "Italic"), pair("synthetic grotesk", "SemiBold Italic"));
    assert_eq!(resolved(&db, "Synthetic Grotesk Regular", "Regular"), pair("synthetic grotesk", "Regular"));
    assert!(db.has_family("Synthetic Grotesk Medium"));
    // The typographic name as is.
    assert_eq!(resolved(&db, "Synthetic Grotesk", "Semibold"), pair("synthetic grotesk", "Semibold"));
    // A width isn't a weight: a missing condensed family stays missing.
    assert!(!db.has_family("Synthetic Grotesk Condensed"));
    assert!(!db.has_family("Synthetic"), "a family isn't found by a part of its name");
}

#[test]
fn a_typographic_name_finds_a_family_installed_per_weight() {
    // The fonts have only legacy names: a family per weight beyond the four basic styles.
    let db = installed(&[
        ("Synthetic Serif", &["Regular", "Italic", "Bold", "Bold Italic"]),
        ("Synthetic Serif Medium", &["Regular", "Italic"]),
        ("Synthetic Serif SemiBold", &["Regular"]),
    ]);
    assert_eq!(resolved(&db, "Synthetic Serif", "Medium"), pair("synthetic serif medium", "Regular"));
    assert_eq!(resolved(&db, "Synthetic Serif", "Medium Italic"), pair("synthetic serif medium", "Italic"));
    assert_eq!(resolved(&db, "Synthetic Serif", "Semi Bold"), pair("synthetic serif semibold", "Regular"));
    assert_eq!(resolved(&db, "Synthetic Serif", "Bold"), pair("synthetic serif", "Bold"), "a style the family has stays in it");
    // A weight number keeps the family and goes by weight.
    assert_eq!(resolved(&db, "Synthetic Serif", "600"), pair("synthetic serif", "600"));
    assert_eq!(figcraft_doc::style_weight("600"), 600);
    assert_eq!(figcraft_doc::style_weight("600 Italic"), 600);
    // Only some weights installed: the family is there all the same.
    let db = installed(&[("Synthetic Serif Medium", &["Regular"])]);
    assert!(db.has_family("Synthetic Serif"));
    assert_eq!(resolved(&db, "Synthetic Serif", "Medium"), pair("synthetic serif medium", "Regular"));
}

#[test]
fn case_and_spacing_dont_matter() {
    let db = installed(&[("Synthetic Mono", &["Regular"])]);
    for name in ["synthetic mono", "SYNTHETIC MONO", " Synthetic Mono ", "SyntheticMono", "Synthetic-Mono", "Synthetic_Mono"] {
        assert!(db.has_family(name), "{name}");
        assert_eq!(db.key(name), "synthetic mono", "{name}");
    }
    assert!(!db.has_family("Synthetic Monospace"));
}

#[test]
fn style_words_are_weights_and_slopes() {
    for s in ["Medium", "Semi Bold", "SemiBold Italic", "600", "Extra Light", "Black Oblique"] {
        assert!(style_words(s), "{s}");
    }
    for s in ["", "Sans", "Condensed", "Display", "Text", "1200"] {
        assert!(!style_words(s), "{s}");
    }
}

#[test]
fn faces_are_found_and_listed_under_either_name() {
    // An embedded face stands in for an installed one, filed under synthetic legacy names.
    let Some(f) = CRAFT_FONTS.first() else { return };
    let face = |family: &str, style: &str| Face::from_bytes(FontBytes::Static(f.bytes), 0, Some(family), Some(style)).unwrap();
    let mut db = FontDb::default();
    db.add(face("Synthetic Display", "Regular"));
    let medium = db.add(face("Synthetic Display Medium", "Regular"));
    let found = db.find("Synthetic Display", "Medium").unwrap();
    assert!(Arc::ptr_eq(&found, &medium), "the legacy family's face");
    assert!(db.styles("Synthetic Display").contains(&"Medium".to_string()), "{:?}", db.styles("Synthetic Display"));
    // The other way round: a design names the legacy family, the typographic one is installed.
    let mut db = FontDb::default();
    db.add(face("Synthetic Text", "Regular"));
    let semibold = db.add(face("Synthetic Text", "SemiBold"));
    assert!(Arc::ptr_eq(&db.find("Synthetic Text SemiBold", "Regular").unwrap(), &semibold));
    assert!(Arc::ptr_eq(&db.find("synthetictext", "Semibold").unwrap(), &semibold));
}
