//! Procedural shader paints: the field stays in range for any input, is the same for the same
//! values, changes with each parameter, and round-trips through its file form.

use crate::{Color, ConcentricShape, Paint, PaintKind, ShaderPaint, ShaderPattern};

#[test]
fn all_concentric_shapes_render_and_round_trip() {
    for shape in ConcentricShape::ALL {
        let paint = ShaderPaint { pattern: ShaderPattern::Concentric, shape, bands: 12, ..ShaderPaint::default() };
        let saved = serde_json::to_string(&paint).unwrap();
        let loaded: ShaderPaint = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.shape, shape);
        let sampler = loaded.sampler();
        let samples: Vec<_> = (0..100).map(|i| sampler.value_in(f64::from(i) / 100.0, 0.31, 1.0, 1.0)).collect();
        assert!(samples.contains(&0.0) && samples.contains(&1.0), "{shape:?} has visible alternating bands");
    }
}

fn grid(sp: &ShaderPaint) -> Vec<f32> {
    let s = sp.sampler();
    (0..40)
        .flat_map(|y| (0..40).map(move |x| (x, y)))
        .map(|(x, y)| {
            if sp.pattern == ShaderPattern::Concentric {
                s.value_in((f64::from(x) + 0.5) / 40.0, (f64::from(y) + 0.5) / 40.0, 1.0, 1.0)
            } else {
                s.value(f64::from(x) * 7.3 + 0.5, f64::from(y) * 5.1 + 0.5)
            }
        })
        .collect()
}

#[test]
fn every_pattern_stays_in_range_and_varies() {
    for pattern in ShaderPattern::ALL {
        let v = grid(&ShaderPaint::new(pattern));
        assert!(v.iter().all(|t| (0.0..=1.0).contains(t)), "{pattern:?}");
        let (lo, hi) = v.iter().fold((1.0f32, 0.0f32), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
        assert!(hi - lo > 0.3, "{pattern:?} is nearly flat: {lo}..{hi}");
        // The same values give the same field.
        assert_eq!(v, grid(&ShaderPaint::new(pattern)));
    }
}

#[test]
fn each_parameter_changes_the_field() {
    for pattern in ShaderPattern::ALL {
        let base = ShaderPaint::new(pattern);
        let v = grid(&base);
        if pattern == ShaderPattern::Concentric {
            assert_ne!(v, grid(&ShaderPaint { bands: 13, ..base }), "concentric bands");
            continue;
        }
        assert_ne!(v, grid(&ShaderPaint { scale: 20.0, ..base.clone() }), "{pattern:?} scale");
        assert_ne!(v, grid(&ShaderPaint { time: 1.7, ..base.clone() }), "{pattern:?} time");
        assert_ne!(v, grid(&ShaderPaint { seed: 9, ..base.clone() }), "{pattern:?} seed");
        assert_ne!(v, grid(&ShaderPaint { grain: 0.5, ..base.clone() }), "{pattern:?} grain");
    }
    assert_ne!(grid(&ShaderPaint::new(ShaderPattern::Noise)), grid(&ShaderPaint::new(ShaderPattern::Cells)));
}

#[test]
fn grain_is_one_value_per_document_unit() {
    let sp = ShaderPaint { grain: 1.0, scale: 10_000.0, ..ShaderPaint::default() };
    let s = sp.sampler();
    // Inside one unit the grain doesn't change (the slow field moves by far less than a level).
    assert!((s.value(10.1, 20.1) - s.value(10.9, 20.9)).abs() < 1e-3);
    let row: Vec<f32> = (0..64).map(|x| s.value(f64::from(x) + 0.5, 0.5)).collect();
    assert!(row.windows(2).filter(|w| (w[0] - w[1]).abs() > 0.05).count() > 32, "grain doesn't vary from unit to unit");
}

#[test]
fn hostile_values_are_sanitised_and_never_panic() {
    let bad = ShaderPaint {
        pattern: ShaderPattern::Cells,
        color_a: Color { r: f32::NAN, g: 9.0, b: -3.0, a: f32::INFINITY },
        color_b: Color::BLACK,
        shape: ConcentricShape::Circle,
        bands: u8::MAX,
        scale: f64::NAN,
        time: f64::NEG_INFINITY,
        grain: f32::NAN,
        seed: u32::MAX,
    };
    let s = bad.sanitized();
    assert_eq!((s.scale, s.time, s.grain), (ShaderPaint::DEFAULT_SCALE, 0.0, 0.0));
    assert_eq!(s.bands, ShaderPaint::BANDS_RANGE.1);
    for pattern in ShaderPattern::ALL {
        for (scale, time) in [(0.0, 1e300), (-5.0, -1e300), (1e300, f64::MAX), (f64::MIN_POSITIVE, f64::NAN)] {
            let sp = ShaderPaint { pattern, scale, time, grain: 7.0, ..bad.clone() };
            for (x, y) in [(0.0, 0.0), (-1e18, 1e18), (f64::MAX, f64::MIN), (f64::NAN, 1.0), (f64::INFINITY, f64::NEG_INFINITY), (-0.5, -0.5)] {
                let t = sp.value(x, y);
                assert!((0.0..=1.0).contains(&t), "{pattern:?} {scale} {time} at {x},{y}: {t}");
                let c = sp.color_at(x, y);
                assert!([c.r, c.g, c.b, c.a].iter().all(|v| (0.0..=1.0).contains(v)));
            }
        }
    }
    assert_eq!(ShaderPaint { scale: 0.001, ..ShaderPaint::default() }.sanitized().scale, ShaderPaint::SCALE_RANGE.0);
    assert_eq!(ShaderPaint { scale: 1e9, ..ShaderPaint::default() }.sanitized().scale, ShaderPaint::SCALE_RANGE.1);
    // The time wraps instead of losing precision.
    let wrapped = ShaderPaint { time: ShaderPaint::TIME_PERIOD + 2.0, ..ShaderPaint::default() }.sanitized().time;
    assert!((wrapped - 2.0).abs() < 1e-9);
}

#[test]
fn colours_run_from_a_to_b() {
    let sp = ShaderPaint { color_a: Color::BLACK, color_b: Color::WHITE, ..ShaderPaint::default() };
    let s = sp.sampler();
    for (x, y) in [(3.0, 4.0), (120.5, 77.25), (-40.0, 900.0)] {
        let (t, c) = (s.value(x, y), s.color(x, y));
        assert!((c.r - t).abs() < 1e-6 && (c.g - t).abs() < 1e-6 && (c.b - t).abs() < 1e-6 && c.a == 1.0);
    }
}

#[test]
fn concentric_fields_scale_with_the_box_and_follow_the_selected_shape() {
    let shader = ShaderPaint { pattern: ShaderPattern::Concentric, color_a: Color::BLACK, color_b: Color::WHITE, bands: 8, ..ShaderPaint::default() };
    let sampler = shader.sampler();
    assert_eq!(sampler.value_in(0.5, 0.5, 1.0, 1.0), 0.0, "the centre starts with colour A");
    assert_eq!(sampler.value_in(0.75, 0.5, 1.0, 1.0), 0.0, "the fifth band uses colour A");
    assert_eq!(sampler.value_in(0.8125, 0.5, 1.0, 1.0), 1.0, "the sixth band uses colour B");
    assert_ne!(
        ShaderPaint { shape: ConcentricShape::Circle, ..shader.clone() }.sampler().value_in(0.9, 0.7, 1.0, 1.0),
        ShaderPaint { shape: ConcentricShape::Square, ..shader.clone() }.sampler().value_in(0.9, 0.7, 1.0, 1.0),
        "corners have different radii in circles and squares",
    );
    assert_ne!(
        ShaderPaint { shape: ConcentricShape::Circle, ..shader.clone() }.sampler().value_in(0.9, 0.75, 1.0, 1.0),
        ShaderPaint { shape: ConcentricShape::Triangle, ..shader.clone() }.sampler().value_in(0.9, 0.75, 1.0, 1.0),
        "the triangle's flat base differs from a circle",
    );
}

#[test]
fn concentric_shapes_keep_their_geometry_and_bands_reach_wide_box_edges() {
    let wide = ShaderPaint { pattern: ShaderPattern::Concentric, bands: 16, ..ShaderPaint::default() }.sampler();
    let circle =
        ShaderPaint { shape: ConcentricShape::Circle, ..ShaderPaint { pattern: ShaderPattern::Concentric, bands: 16, ..ShaderPaint::default() } }
            .sampler();
    let circle_x = circle.value_in(130.0, 30.0, 240.0, 60.0);
    let circle_y = circle.value_in(120.0, 40.0, 240.0, 60.0);
    assert_eq!(circle_x, circle_y, "equal document distances have equal circle radii on a wide box");

    let triangle =
        ShaderPaint { shape: ConcentricShape::Triangle, ..ShaderPaint { pattern: ShaderPattern::Concentric, bands: 32, ..ShaderPaint::default() } }
            .sampler();
    let triangle_x = triangle.value_in(120.0 + 240.0 * 0.2 / 3.0_f64.sqrt(), 30.0, 240.0, 60.0);
    let triangle_y = triangle.value_in(120.0, 30.0 + 24.0, 240.0, 60.0);
    assert_eq!(triangle_x, triangle_y, "equal equilateral-triangle radii remain equal on a wide box");

    let edge_samples: Vec<_> = (0..=8).map(|step| wide.value_in(f64::from(step) * 30.0, 30.0, 240.0, 60.0)).collect();
    assert!(edge_samples.windows(2).any(|pair| pair[0] != pair[1]), "bands continue across the long axis to the box edge: {edge_samples:?}");
    assert_ne!(edge_samples[0], edge_samples[4], "the centre and the long edge have distinct bands");
}

#[test]
fn concentric_settings_are_bounded_and_round_trip() {
    let shader = ShaderPaint { pattern: ShaderPattern::Concentric, shape: ConcentricShape::Triangle, bands: u8::MAX, ..ShaderPaint::default() };
    let safe = shader.sanitized();
    assert_eq!(safe.bands, ShaderPaint::BANDS_RANGE.1);
    let paint = Paint::shader(safe);
    let value = serde_json::to_value(&paint).unwrap();
    assert_eq!(value["pattern"], "concentric");
    assert_eq!(value["shape"], "triangle");
    assert_eq!(value["bands"], ShaderPaint::BANDS_RANGE.1);
    assert_eq!(serde_json::from_value::<Paint>(value).unwrap(), paint);
    let legacy: Paint = serde_json::from_str(r#"{"type":"shader","pattern":"concentric"}"#).unwrap();
    assert!(matches!(legacy.kind, PaintKind::Shader(ShaderPaint { shape: ConcentricShape::Circle, bands: ShaderPaint::DEFAULT_BANDS, .. })));
}

#[test]
fn file_form_round_trips_and_defaults_fill_in() {
    let sp = ShaderPaint { pattern: ShaderPattern::Waves, scale: 32.0, time: 2.5, grain: 0.25, seed: 7, ..ShaderPaint::default() };
    let paint = Paint::shader(sp.clone());
    let json = serde_json::to_value(&paint).unwrap();
    assert_eq!(json["type"], "shader");
    assert_eq!(json["pattern"], "waves");
    assert!(json.get("colorA").is_some() && json.get("colorB").is_some());
    let back: Paint = serde_json::from_value(json).unwrap();
    assert_eq!(back, paint);
    // A file naming only the type gets the defaults.
    let bare: Paint = serde_json::from_str(r#"{"type":"shader"}"#).unwrap();
    assert_eq!(bare.kind, PaintKind::Shader(ShaderPaint::default()));
    // A pattern this version doesn't know is refused, not drawn as something else.
    assert!(serde_json::from_str::<Paint>(r#"{"type":"shader","pattern":"glsl"}"#).is_err());
    assert_eq!(paint.label(), "Shader");
    assert_eq!(ShaderPattern::from_id(" Cells "), Some(ShaderPattern::Cells));
    assert_eq!(ShaderPattern::from_id("glsl"), None);
}

#[test]
fn colours_are_visited_for_profile_conversion() {
    let mut paint = Paint::shader(ShaderPaint::default());
    let mut n = 0;
    paint.for_each_color_mut(&mut |c| {
        *c = Color::WHITE;
        n += 1;
    });
    assert_eq!(n, 2);
    let PaintKind::Shader(sp) = &paint.kind else { panic!() };
    assert_eq!((sp.color_a, sp.color_b), (Color::WHITE, Color::WHITE));
    assert_eq!(paint.preview_color(), Color::WHITE);
}
