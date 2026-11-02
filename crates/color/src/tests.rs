use super::*;

#[test]
fn hex_roundtrip_and_shorthands() {
    assert_eq!(Color::from_hex("#0D99FF").unwrap().to_hex(), "0D99FF");
    assert_eq!(Color::from_hex("f").unwrap().to_hex(), "FFFFFF");
    assert_eq!(Color::from_hex("ab").unwrap().to_hex(), "ABABAB");
    assert_eq!(Color::from_hex("abc").unwrap().to_hex(), "AABBCC");
    assert_eq!(Color::from_hex("FF000080").unwrap().to_rgba8(), [255, 0, 0, 128]);
    assert!(Color::from_hex("xyz").is_none());
    assert!(Color::from_hex("12345").is_none());
}

#[test]
fn css_parsing() {
    assert_eq!(Color::parse_css("rgb(255, 0, 0)").unwrap().to_hex(), "FF0000");
    assert_eq!(Color::parse_css("rgba(0 0 255 / 50%)").unwrap().to_rgba8(), [0, 0, 255, 128]);
    assert_eq!(Color::parse_css("hsl(120, 100%, 50%)").unwrap().to_hex(), "00FF00");
    assert_eq!(Color::parse_css("white").unwrap(), Color::WHITE);
    assert_eq!(Color::parse_css("RebeccaPurple").unwrap().to_hex(), "663399");
    assert_eq!(Color::parse_css("transparent").unwrap(), Color::TRANSPARENT);
}

#[test]
fn colour_fields_take_names() {
    assert_eq!(Color::from_hex_or_name("red").unwrap().to_css_hex(), "#FF0000");
    assert_eq!(Color::from_hex_or_name(" Cornflower Blue ").unwrap().to_hex(), "6495ED");
    assert_eq!(Color::from_hex_or_name("darkblue").unwrap().to_rgba8(), [0, 0, 139, 255]);
    // Hex wins: every digit string still reads as hex.
    assert_eq!(Color::from_hex_or_name("abc").unwrap().to_hex(), "AABBCC");
    assert_eq!(Color::from_hex_or_name("#0D99FF").unwrap().to_hex(), "0D99FF");
    // Names are opaque; `transparent` is not a field colour.
    assert!(Color::from_hex_or_name("transparent").is_none());
    for junk in ["", "#", "redd", "re d d", "12345", "\u{0}", "🟥", &"x".repeat(100_000)] {
        assert!(Color::from_hex_or_name(junk).is_none(), "{junk:?}");
    }
}

#[test]
fn hsb_roundtrip() {
    for hex in ["0D99FF", "F24822", "14AE5C", "000000", "FFFFFF", "808080", "9747FF"] {
        let c = Color::from_hex(hex).unwrap();
        assert_eq!(Hsb::from_color(c).to_color().to_hex(), hex);
        assert_eq!(Hsl::from_color(c).to_color().to_hex(), hex);
    }
}

#[test]
fn gradient_sampling() {
    let g = Gradient {
        kind: GradientKind::Linear,
        stops: vec![ColorStop { position: 1.0, color: Color::WHITE }, ColorStop { position: 0.0, color: Color::BLACK }],
        from: (0.0, 0.0),
        to: (1.0, 0.0),
        width: (0.0, 1.0),
    };
    assert_eq!(g.sample(0.0), Color::BLACK);
    assert_eq!(g.sample(1.0), Color::WHITE);
    assert_eq!(g.sample(0.5).to_hex(), "808080");
}

#[test]
fn paint_serde_shape() {
    let p = Paint::solid(Color::from_hex("FF0000").unwrap());
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["type"], "solid");
    let back: Paint = serde_json::from_value(j).unwrap();
    assert_eq!(back, p);
    let g: Paint = serde_json::from_str(r#"{"type":"gradient","kind":"radial","stops":[],"from":[0.5,0.5],"to":[1,0.5],"width":[0.5,1]}"#).unwrap();
    assert!(matches!(g.kind, PaintKind::Gradient(_)));
}

#[test]
fn blend_names() {
    assert_eq!(BlendMode::from_name("multiply"), Some(BlendMode::Multiply));
    assert_eq!(BlendMode::from_name("Color burn"), Some(BlendMode::ColorBurn));
    assert_eq!(BlendMode::from_name("PASS_THROUGH"), Some(BlendMode::PassThrough));
}

#[test]
fn nan_inputs_are_sanitized() {
    let c = Color::new(f32::NAN, 2.0, -1.0, f32::INFINITY);
    assert_eq!(c.to_rgba8(), [0, 255, 0, 255]);
}

#[test]
fn colour_deserializes_from_many_forms() {
    let a: Color = serde_json::from_str("\"#FF0000\"").unwrap();
    let b: Color = serde_json::from_str("{\"r\":1,\"g\":0,\"b\":0}").unwrap();
    let c: Color = serde_json::from_str("[1,0,0,1]").unwrap();
    assert_eq!(a, b);
    assert_eq!(b, c);
    assert!(serde_json::from_str::<Color>("\"nope\"").is_err());
}

#[test]
fn crop_defaults_to_fill_and_roundtrips() {
    let img =
        ImagePaint { image: String::new(), scale_mode: ImageScaleMode::Crop, scale: 1.0, rotation: 0, crop: None, filters: ImageFilters::default() };
    // A 200×100 image in a 100×100 box: Fill scales to 100 tall, 200 wide, centred → x from -0.5 to 1.5.
    let c = img.effective_crop(200.0, 100.0, 100.0, 100.0);
    let (x0, y0, x1, y1) = ImagePaint::crop_rect(c);
    assert!((x0 + 0.5).abs() < 1e-9 && (x1 - 1.5).abs() < 1e-9 && y0.abs() < 1e-9 && (y1 - 1.0).abs() < 1e-9);
    let back = ImagePaint::crop_from_rect(x0, y0, x1, y1);
    for (a, b) in back.iter().zip(c.iter()) {
        assert!((a - b).abs() < 1e-9);
    }
}

#[test]
fn infinite_channels_clamp() {
    let c = Color::parse_css("rgb(1e40, 0, 0)").unwrap();
    assert_eq!(c.r, 1.0);
    let c = Color::new(f32::INFINITY, f32::NEG_INFINITY, f32::NAN, f32::INFINITY);
    assert_eq!((c.r, c.g, c.b, c.a), (1.0, 0.0, 0.0, 1.0));
}

#[test]
fn hex_allows_at_most_one_leading_hash() {
    assert!(Color::from_hex("##ff0000").is_none());
    assert!(Color::from_hex("###fff").is_none());
    assert!(Color::from_hex_or_name("##ff0000").is_none());
    assert_eq!(Color::from_hex("#ff0000").unwrap().to_hex(), "FF0000");
    assert_eq!(Color::from_hex("ff0000").unwrap().to_hex(), "FF0000");
}

#[test]
fn malformed_alpha_or_extra_args_rejected() {
    assert_eq!(Color::parse_css("rgba(0,0,0,abc)"), None);
    assert_eq!(Color::parse_css("hsla(0,0%,0%,abc)"), None);
    assert_eq!(Color::parse_css("rgb(1,2,3,4,5)"), None);
    assert_eq!(Color::parse_css("hsl(0,0%,0%,0.5,1)"), None);
    assert_eq!(Color::parse_css("rgb(1,2,3)").unwrap().a, 1.0);
    assert_eq!(Color::parse_css("rgba(1,2,3,0.5)").unwrap().a, 0.5);
}

#[test]
fn to_css_round_trips_every_alpha() {
    for a in 0..=255u8 {
        let c = Color::from_rgba8(10, 20, 30, a);
        assert_eq!(Color::parse_css(&c.to_css()).unwrap().to_rgba8(), c.to_rgba8(), "alpha {a}: {}", c.to_css());
    }
}

#[test]
fn css_hsl_rejects_non_finite() {
    for s in ["hsl(nan,50%,50%)", "hsl(inf,50%,50%)", "hsl(0,inf%,50%)", "hsl(0,50%,infinity%)", "hsla(120,50%,50%,nan)"] {
        assert_eq!(Color::parse_css(s), None, "{s}");
    }
}

#[test]
fn css_rejects_empty_arguments() {
    for s in ["rgb(255,,0,0)", "rgb(,1,2,3)", "rgb(1,2,3,)", "hsl(10,,50%,50%)", "rgb(1,2,,)", "oklch(0.5,,0.1,20)"] {
        assert_eq!(Color::parse_css(s), None, "{s}");
    }
    assert!(Color::parse_css("rgb(255 0 0 / 50%)").is_some());
    assert!(Color::parse_css("hsl(120deg 50% 50%)").is_some());
}
