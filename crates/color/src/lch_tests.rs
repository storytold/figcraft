use super::*;

fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn lch_matches_css_reference_values() {
    // CSS Color 4 values for sRGB primaries and white (D50 Lab).
    let red = Lch::from_color(Color::from_hex("FF0000").unwrap());
    assert!(close(red.l, 54.29, 0.05) && close(red.c, 106.84, 0.1) && close(red.h, 40.85, 0.1), "{red:?}");
    let white = Lch::from_color(Color::WHITE);
    assert!(close(white.l, 100.0, 0.01) && white.c < 0.05, "{white:?}");
    assert_eq!(Lch::from_color(Color::BLACK).l.round(), 0.0);
}

#[test]
fn oklch_matches_reference_values() {
    let red = Oklch::from_color(Color::from_hex("FF0000").unwrap());
    assert!(close(red.l, 0.628, 0.001) && close(red.c, 0.2577, 0.001) && close(red.h, 29.23, 0.1), "{red:?}");
    let white = Oklch::from_color(Color::WHITE);
    assert!(close(white.l, 1.0, 0.001) && white.c < 0.001 && white.h == 0.0, "{white:?}");
}

#[test]
fn lch_and_oklch_round_trip_through_srgb() {
    for hex in ["0D99FF", "FF0000", "00FF00", "0000FF", "FFFFFF", "000000", "808080", "663399", "F24E1E", "1ABC9C"] {
        let c = Color::from_hex(hex).unwrap();
        assert_eq!(Lch::from_color(c).to_color().to_hex(), hex, "{hex} via LCH");
        assert_eq!(Oklch::from_color(c).to_color().to_hex(), hex, "{hex} via OKLCH");
    }
}

#[test]
fn out_of_gamut_keeps_lightness_and_hue() {
    // Far more chroma than sRGB holds: the chroma comes down, lightness and hue stay.
    let asked = Oklch { l: 0.7, c: 0.45, h: 150.0 };
    let got = Oklch::from_color(asked.to_color());
    assert!(close(got.l, 0.7, 0.01) && close(got.h, 150.0, 1.5) && got.c < 0.45, "{got:?}");
    let asked = Lch { l: 50.0, c: 200.0, h: 300.0 };
    let got = Lch::from_color(asked.to_color());
    assert!(close(got.l, 50.0, 1.0) && close(got.h, 300.0, 2.0) && got.c < 200.0, "{got:?}");
}

#[test]
fn junk_values_never_panic() {
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30, -5.0, 0.0] {
        let a = Lch { l: v, c: v, h: v }.to_color();
        let b = Oklch { l: v, c: v, h: v }.to_color();
        for x in [a.r, a.g, a.b, b.r, b.g, b.b] {
            assert!(x.is_finite() && (0.0..=1.0).contains(&x));
        }
    }
    let nan = Color { r: f32::NAN, g: 2.0, b: -1.0, a: 1.0 };
    assert!(Lch::from_color(nan.sanitized()).l.is_finite());
    assert!(Oklch::from_color(nan.sanitized()).l.is_finite());
}

#[test]
fn css_reads_lch_and_oklch() {
    assert_eq!(Color::parse_css("oklch(62.8% 0.2577 29.23)").unwrap().to_hex(), "FF0000");
    assert_eq!(Color::parse_css("oklch(0.628 0.2577 29.23deg / 50%)").unwrap().to_rgba8(), [255, 0, 0, 128]);
    assert_eq!(Color::parse_css("lch(54.29 106.84 40.85)").unwrap().to_hex(), "FF0000");
    assert_eq!(Color::parse_css("LCH(100% 0 0)").unwrap().to_hex(), "FFFFFF");
    assert!(Color::parse_css("oklch(nan 0 0)").is_none());
    assert!(Color::parse_css("lch(50)").is_none());
}

#[test]
fn css_hue_units_and_none() {
    let blue = Color::parse_css("oklch(0.6 0.2 260)").unwrap();
    for h in ["260deg", "0.7222222turn", "288.8888grad", "4.537856rad"] {
        assert_eq!(Color::parse_css(&format!("oklch(0.6 0.2 {h})")).unwrap().to_hex(), blue.to_hex(), "{h}");
    }
    assert_eq!(Color::parse_css("lch(50 none none)").unwrap().to_hex(), Color::parse_css("lch(50 0 0)").unwrap().to_hex());
    assert_eq!(Color::parse_css("oklch(none 0 0 / none)").unwrap().to_rgba8(), [0, 0, 0, 255]);
}

#[test]
fn css_lch_and_oklch_junk_is_refused_or_in_range() {
    let long = "9".repeat(100_000);
    let junk = [
        "oklch()".to_string(),
        "oklch(,,)".into(),
        "lch(1 2 3 4 5)".into(),
        "oklch(0.5 0.1 1e40)".into(),
        "lch(inf 0 0)".into(),
        "oklch(0.5 0.1 deg)".into(),
        "lch(50 50 50 / / /)".into(),
        "oklch(🟥 0 0)".into(),
        "lch(\u{0} 0 0)".into(),
        format!("oklch({long} {long} {long})"),
        format!("lch(50 50 {long}turn)"),
    ];
    for j in &junk {
        if let Some(c) = Color::parse_css(j) {
            for v in [c.r, c.g, c.b, c.a] {
                assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{j:.40}: {c:?}");
            }
        }
    }
    for j in ["oklch()", "lch(1 2 3 4 5)", "lch(inf 0 0)", "oklch(0.5 0.1 deg)", "oklch(🟥 0 0)", "oklch(0.5 0.1 1e40)"] {
        assert!(Color::parse_css(j).is_none(), "{j}");
    }
}
