use super::*;
use crate::xml::{color, esc, num};

#[test]
fn numbers_are_short_finite_and_never_negative_zero() {
    assert_eq!(num(1.0), "1");
    assert_eq!(num(0.12345), "0.123");
    assert_eq!(num(-0.0004), "0");
    assert_eq!(num(-2.5), "-2.5");
    assert_eq!(num(1e-9), "0");
    assert_eq!(num(12345678.9), "12345678.9");
    assert_eq!(num(f64::NAN), "0");
    assert_eq!(num(f64::INFINITY), "0");
}

#[test]
fn colours_are_rrggbb_or_aarrggbb() {
    assert_eq!(color(hex("1A2B3C")), "#1A2B3C");
    assert_eq!(color(hex("1A2B3C80")), "#801A2B3C");
    assert_eq!(color(Color::new(2.0, -1.0, f32::NAN, 1.0)).len(), 7);
}

#[test]
fn text_is_escaped_for_an_attribute() {
    assert_eq!(esc("a&b<c>\"d'"), "a&amp;b&lt;c&gt;&quot;d&apos;");
    assert_eq!(esc("tab\tline\nret\r"), "tab&#9;line&#10;ret&#13;");
    assert_eq!(esc("bell\u{7}\u{FFFE}ok"), "bellok");
}

#[test]
fn names_are_written_only_when_asked_unique_and_round_trip() {
    let (mut d, p) = doc();
    let g = d.create(p, NodeKind::Group, "Icon & \"co\" <1>", 0.0, 0.0, 50.0, 50.0).unwrap();
    rect(&mut d, g, "dot", 0.0, 0.0, 10.0, 10.0, "FF0000");
    rect(&mut d, g, "dot", 20.0, 0.0, 10.0, 10.0, "00FF00");
    rect(&mut d, g, "  ", 40.0, 0.0, 10.0, 10.0, "0000FF");
    assert!(!export(&d, &[g], Compat::BestEffort).xml.contains("android:name"));
    let out = export_with(&d, &[g], VdOptions { include_names: true, ..Default::default() });
    let els = elements(&out.xml);
    let names: Vec<_> = els.iter().filter_map(|e| e.get("name")).collect();
    assert_eq!(names, ["Icon & \"co\" <1>", "dot", "dot_2", "layer"], "{}", out.xml);
    assert_eq!(tagged(&els, "group")[0].get("name"), Some("Icon & \"co\" <1>"), "a container is a named group");
}

#[test]
fn a_long_name_is_cut_at_200_characters() {
    let (mut d, p) = doc();
    let long = "é".repeat(300);
    let r = rect(&mut d, p, &long, 0.0, 0.0, 10.0, 10.0, "FF0000");
    let out = export_with(&d, &[r], VdOptions { include_names: true, ..Default::default() });
    assert_eq!(paths_of(&out.xml)[0].get("name").unwrap().chars().count(), 200);
}

#[test]
fn the_file_is_pretty_printed() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 24.0, 24.0).unwrap();
    d.get_mut(f).unwrap().fills.clear();
    rect(&mut d, f, "R", 4.0, 4.0, 16.0, 8.0, "FF0000");
    let out = export(&d, &[f], Compat::BestEffort);
    let want = "<?xml version=\"1.0\" encoding=\"utf-8\"?>
<vector
    xmlns:android=\"http://schemas.android.com/apk/res/android\"
    android:width=\"24dp\"
    android:height=\"24dp\"
    android:viewportWidth=\"24\"
    android:viewportHeight=\"24\">
  <group>
    <clip-path android:pathData=\"M0,0 L24,0 L24,24 L0,24 Z\" />
    <path
        android:pathData=\"M4,4 L20,4 L20,12 L4,12 Z\"
        android:fillColor=\"#FF0000\" />
  </group>
</vector>
";
    assert_eq!(out.xml, want);
}

#[test]
fn an_empty_area_is_still_a_drawable_android_loads() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 10.0, 10.0, "FF0000");
    d.get_mut(r).unwrap().fills.clear();
    let opts = VdOptions { bounds: Some(figcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)), ..Default::default() };
    let out = export_with(&d, &[r], opts);
    // Android throws "no path defined" for a <vector> without a <path>.
    assert!(out.xml.ends_with("android:viewportHeight=\"10\">\n  <path android:pathData=\"M0,0\" />\n</vector>\n"), "{}", out.xml);
}
