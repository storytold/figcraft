//! PDF print settings on export settings: lengths with units, safe values, saved only when set.

use crate::{ExportFormat, ExportSetting, PrintSettings};

#[test]
fn lengths_read_with_their_units() {
    let pt = |s: &str| PrintSettings::parse_length(s);
    assert_eq!(pt("9"), Some(9.0));
    assert_eq!(pt("9pt"), Some(9.0));
    assert_eq!(pt(" 9 PX "), Some(9.0));
    assert_eq!(pt("0.125in"), Some(9.0));
    assert_eq!(pt("0.125\""), Some(9.0));
    assert!((pt("3mm").unwrap() - 8.503_937).abs() < 1e-6);
    assert!((pt("3 mm").unwrap() - 8.503_937).abs() < 1e-6);
    assert!((pt("0.3cm").unwrap() - 8.503_937).abs() < 1e-6);
    for bad in ["", "mm", "3 furlongs", "-1mm", "NaN", "inf", "1e400", "3mm4"] {
        assert_eq!(pt(bad), None, "{bad}");
    }
}

#[test]
fn lengths_label_in_millimetres_when_whole() {
    assert_eq!(PrintSettings::length_label(72.0 / 25.4 * 3.0), "3 mm");
    assert_eq!(PrintSettings::length_label(72.0 / 25.4 * 2.5), "2.5 mm");
    assert_eq!(PrintSettings::length_label(9.0), "9 pt");
    assert_eq!(PrintSettings::length_label(0.0), "0 mm");
}

#[test]
fn hostile_bleeds_are_made_safe() {
    let b = |bleed: f64| PrintSettings { bleed, ..Default::default() }.bleed();
    assert_eq!(b(f64::NAN), 0.0);
    assert_eq!(b(-3.0), 0.0);
    assert_eq!(b(f64::INFINITY), 0.0);
    assert_eq!(b(1e9), PrintSettings::MAX_BLEED);
    // Bleed marks need a bleed; crop marks don't.
    assert!(!PrintSettings { bleed_marks: true, ..Default::default() }.extends_page());
    assert!(PrintSettings { crop_marks: true, ..Default::default() }.extends_page());
    assert!(PrintSettings { bleed: 9.0, ..Default::default() }.extends_page());
    assert!(!PrintSettings { cmyk: true, ..Default::default() }.extends_page());
}

#[test]
fn print_settings_save_only_when_set() {
    let plain = ExportSetting { format: ExportFormat::Pdf, ..Default::default() };
    let json = serde_json::to_value(&plain).unwrap();
    assert!(json.get("print").is_none(), "{json}");
    let print = ExportSetting { print: PrintSettings { bleed: 9.0, crop_marks: true, bleed_marks: true, cmyk: true }, ..plain };
    let json = serde_json::to_value(&print).unwrap();
    assert_eq!(json["print"], serde_json::json!({"bleed": 9.0, "cropMarks": true, "bleedMarks": true, "cmyk": true}));
    assert_eq!(serde_json::from_value::<ExportSetting>(json).unwrap(), print);
    // Files from before have none.
    let old: ExportSetting = serde_json::from_value(serde_json::json!({"format": "pdf", "suffix": ""})).unwrap();
    assert!(old.print.is_default());
}

/// #2004: a JPG quality is 1 to 100 both ways: a saved setting carries it (and not the default),
/// a file or `node.set` with one outside the range is refused, and so is writing one, so a value
/// put in the public field can't make a file that won't reopen.
#[test]
fn jpg_quality_is_1_to_100_both_ways() {
    let plain = ExportSetting { format: ExportFormat::Jpg, ..Default::default() };
    assert_eq!(serde_json::to_value(&plain).unwrap().get("jpgQuality"), None, "the default is left out");
    let low = ExportSetting { jpg_quality: 40, ..plain.clone() };
    let json = serde_json::to_value(&low).unwrap();
    assert_eq!(json["jpgQuality"], serde_json::json!(40));
    assert_eq!(serde_json::from_value::<ExportSetting>(json).unwrap(), low);
    assert_eq!(
        serde_json::from_value::<ExportSetting>(serde_json::json!({"format": "jpg", "jpg_quality": 40})).unwrap(),
        low,
        "node.set's snake_case"
    );
    for bad in [0, 101] {
        let err = serde_json::from_value::<ExportSetting>(serde_json::json!({"format": "jpg", "jpgQuality": bad})).unwrap_err().to_string();
        assert!(err.contains("1 to 100"), "{bad}: {err}");
        let err = serde_json::to_value(ExportSetting { jpg_quality: bad, ..plain.clone() }).unwrap_err().to_string();
        assert!(err.contains("1 to 100"), "writing {bad}: {err}");
    }
}
