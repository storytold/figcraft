//! Effect kinds: names, files from before noise/texture grew modes, glass values and reach.

use crate::*;

#[test]
fn effect_types_by_name() {
    for (label, ty) in Effect::TYPES {
        let e = Effect::of_type(ty).unwrap();
        assert_eq!(e.label(), label);
        assert!(e.visible);
    }
    assert!(matches!(Effect::of_type("DROP_SHADOW").unwrap().kind, EffectKind::DropShadow(_)));
    assert!(matches!(Effect::of_type("background blur").unwrap().kind, EffectKind::BackgroundBlur { .. }));
    assert!(matches!(Effect::of_type("blur").unwrap().kind, EffectKind::LayerBlur { .. }));
    assert!(matches!(Effect::of_type("Glass").unwrap().kind, EffectKind::Glass(_)));
    assert!(Effect::of_type("sparkle").is_none());
}

#[test]
fn old_noise_and_texture_files_still_open() {
    let e: Effect = serde_json::from_str(r#"{"type":"noise","size":1.0,"density":0.3,"color":{"r":1.0,"g":0.0,"b":0.0,"a":0.5}}"#).unwrap();
    let EffectKind::Noise(n) = &e.kind else { panic!("{e:?}") };
    assert_eq!((n.mode, n.size, n.density, n.size_y), (NoiseMode::Mono, 1.0, 0.3, None));
    assert_eq!(n.color.a, 0.5);
    let e: Effect = serde_json::from_str(r#"{"type":"texture","size":2.0,"radius":3.0,"visible":false}"#).unwrap();
    let EffectKind::Texture(t) = &e.kind else { panic!("{e:?}") };
    assert_eq!((t.size, t.radius, t.clip_to_shape, e.visible), (2.0, 3.0, false, false));
    // Camel-case names (as in the REST API) are read too.
    let e: Effect = serde_json::from_str(r#"{"type":"glass","lightAngle":30,"lightIntensity":0.5,"depth":12}"#).unwrap();
    let EffectKind::Glass(g) = &e.kind else { panic!("{e:?}") };
    assert_eq!((g.light_angle, g.light_intensity, g.depth, g.refraction), (30.0, 0.5, 12.0, Glass::default().refraction));
    for e in [Effect::noise(), Effect::texture(), Effect::glass()] {
        let back: Effect = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}

#[test]
fn glass_values_are_clamped_and_reach_their_backdrop() {
    let g = Glass { refraction: 7.0, depth: -3.0, dispersion: f64::NAN, frost: f64::INFINITY, splay: -1.0, ..Glass::default() }.sanitized();
    assert_eq!((g.refraction, g.depth, g.dispersion, g.splay), (1.0, 1.0, Glass::default().dispersion, 0.0));
    assert!(g.frost.is_finite());
    let e = Effect::glass();
    let EffectKind::Glass(g) = &e.kind else { panic!("{e:?}") };
    assert!(e.backdrop_radius() >= g.frost + g.max_bend() / 1.5 - 1e-9);
    assert_eq!(Effect::background_blur().backdrop_radius(), 4.0);
    assert_eq!(Effect::drop_shadow().backdrop_radius(), 0.0);
    let hidden = Effect { visible: false, ..Effect::glass() };
    assert_eq!(hidden.backdrop_radius(), 0.0);
}

#[test]
fn glass_frost_stays_within_the_reference_range() {
    // A file from before the 0–100 range draws its frost at 100, and reaches no further.
    let g = Glass { frost: 150.0, ..Glass::default() };
    assert_eq!(g.sanitized().frost, Glass::MAX_FROST);
    let e = Effect { kind: EffectKind::Glass(g), visible: true };
    let capped = Effect { kind: EffectKind::Glass(Glass { frost: Glass::MAX_FROST, ..Glass::default() }), visible: true };
    assert_eq!(e.backdrop_radius(), capped.backdrop_radius());
}

#[test]
fn glass_fields_take_bound_numbers_as_the_panel_shows_them() {
    let mut g = Glass::default();
    for (field, x) in [
        ("lightAngle", 30.0),
        ("lightIntensity", 25.0),
        ("refraction", 40.0),
        ("depth", 12.0),
        ("dispersion", 10.0),
        ("radius", 8.0),
        ("splay", 50.0),
    ] {
        assert!(g.set_bound(field, x), "{field}");
    }
    assert_eq!((g.light_angle, g.light_intensity, g.refraction, g.depth, g.dispersion, g.frost, g.splay), (30.0, 0.25, 0.4, 12.0, 0.1, 8.0, 0.5));
    // Out of range: clamped to the field's range.
    for (field, x) in [("lightIntensity", 250.0), ("depth", 0.0), ("radius", 150.0), ("splay", -5.0)] {
        assert!(g.set_bound(field, x), "{field}");
    }
    assert_eq!((g.light_intensity, g.depth, g.frost, g.splay), (1.0, 1.0, Glass::MAX_FROST, 0.0));
    // A field glass doesn't have, or a value that isn't a number, changes nothing.
    let before = g.clone();
    assert!(!g.set_bound("spread", 3.0));
    assert!(!g.set_bound("splay", f64::NAN));
    assert!(!g.set_bound("depth", f64::INFINITY));
    assert_eq!(g, before);
    for field in Glass::BOUND_FIELDS {
        assert!(Glass::default().set_bound(field, 1.0), "{field}");
    }
}

#[test]
fn a_bound_light_angle_turns_round_instead_of_stopping() {
    // An angle past ±180 is the same direction a turn away (270 is -90), not the end of the range.
    for (x, want) in [(270.0, -90.0), (-270.0, 90.0), (540.0, 180.0), (-180.0, 180.0), (180.0, 180.0), (720.5, 0.5), (-360.0, 0.0), (-45.0, -45.0)] {
        let mut g = Glass::default();
        assert!(g.set_bound("lightAngle", x), "{x}");
        assert_eq!(g.light_angle, want, "{x}");
    }
    // A whole turn back is 0, not -0, so a saved file says 0.
    let mut g = Glass::default();
    assert!(g.set_bound("lightAngle", -360.0));
    assert!(g.light_angle == 0.0 && g.light_angle.is_sign_positive());
    // A huge angle still lands in range.
    let mut g = Glass::default();
    assert!(g.set_bound("lightAngle", 1e300));
    assert!((-180.0..=180.0).contains(&g.light_angle) && g.light_angle != -180.0);
}

#[test]
fn effect_colours() {
    let mut e = Effect::noise();
    assert!(e.color(true).is_some());
    *e.color_mut(true).unwrap() = color::Color::WHITE;
    let EffectKind::Noise(n) = &e.kind else { panic!("{e:?}") };
    assert_eq!(n.secondary, color::Color::WHITE);
    assert!(Effect::drop_shadow().color(true).is_none());
    assert!(Effect::glass().color(false).is_none());
    // The colour visitor (palette edits, colour profile conversion) sees both noise colours.
    let mut seen = 0;
    e.for_each_color_mut(&mut |_| seen += 1);
    assert_eq!(seen, 2);
}

#[test]
fn spread_reaches_further_at_sharp_corners_only() {
    let mut d = Document::new("r");
    let p = d.first_page().unwrap();
    let shadow =
        Effect { kind: EffectKind::DropShadow(Shadow { radius: 0.0, offset: (0.0, 0.0), spread: 10.0, ..Shadow::default() }), visible: true };
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    let s = d.create(p, NodeKind::Star { count: 5, inner_ratio: 0.4 }, "S", 0.0, 0.0, 10.0, 10.0).unwrap();
    for id in [r, s] {
        d.get_mut(id).unwrap().effects = vec![shadow.clone()];
    }
    assert_eq!(d.node(r).unwrap().paint_reach(), 10.0);
    assert_eq!(d.node(s).unwrap().paint_reach(), 10.0 * SPREAD_MITER_LIMIT);
}

#[test]
fn stacked_layer_effects_reach_together_and_shadows_from_there() {
    let mut n = Node::new(NodeId(1), NodeKind::Rectangle, "R", 10.0, 10.0);
    let blur = |r: f64| Effect { kind: EffectKind::LayerBlur(Blur::uniform(r)), visible: true };
    // One blur reaches as before; four of 10 as one of 20 (their variances add).
    n.effects = vec![blur(10.0)];
    assert_eq!(n.layer_effects_reach(), 15.0);
    n.effects = vec![blur(10.0), blur(10.0), blur(10.0), blur(10.0)];
    assert!((n.layer_effects_reach() - 30.0).abs() < 1e-9, "{}", n.layer_effects_reach());
    // A sharp shadow below is cast from the blurred layer: it reaches the blur's distance further.
    let shadow =
        Effect { kind: EffectKind::DropShadow(Shadow { radius: 0.0, offset: (0.0, 50.0), spread: 0.0, ..Shadow::default() }), visible: true };
    n.effects = vec![blur(10.0), shadow];
    let b = n.grow_by_effects(figcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!((b.y0, b.y1), (-15.0, 75.0), "{b:?}");
    // Hidden effects reach nowhere.
    n.effects = vec![Effect { visible: false, ..blur(10.0) }];
    assert_eq!(n.layer_effects_reach(), 0.0);
}

#[test]
fn reaches_too_large_to_add_stay_finite() {
    let mut n = Node::new(NodeId(1), NodeKind::Rectangle, "R", 10.0, 10.0);
    let t = Texture { radius: 1e308, ..Default::default() };
    n.effects = vec![Effect { kind: EffectKind::Texture(t.clone()), visible: true }, Effect { kind: EffectKind::Texture(t), visible: true }];
    let (reach, b) = (n.paint_reach(), n.grow_by_effects(figcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)));
    assert!(reach.is_finite() && reach > 1e6, "{reach}");
    assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.width() > 1e6, "{b:?}");
}
