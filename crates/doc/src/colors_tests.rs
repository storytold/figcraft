use super::*;
use figcraft_color::{ImageFilters, Paint, ProfileConversion};
use std::collections::BTreeMap;

const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };

fn to_p3() -> ProfileConversion {
    ColorProfile::Srgb.conversion_to(ColorProfile::DisplayP3)
}

/// A file with a colour in every place one can live.
fn colourful() -> (Document, NodeId) {
    let mut d = Document::new("c");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    let t = d.create(p, NodeKind::Text(TextData::default()), "T", 0.0, 0.0, 10.0, 10.0).unwrap();
    {
        let n = d.node_mut(r).unwrap();
        n.fills = vec![Paint::solid(RED)];
        n.strokes = vec![Paint::solid(RED)];
        n.effects = vec![Effect { kind: EffectKind::DropShadow(Shadow { color: RED, ..Shadow::default() }), visible: true }];
        n.grids = vec![LayoutGrid { color: RED, ..LayoutGrid::default() }];
        n.interactions = vec![proto::Interaction {
            trigger: Default::default(),
            actions: vec![proto::Action::OpenOverlay {
                dest: p,
                position: Default::default(),
                close_on_outside: false,
                background: Some(RED),
                offset: None,
                anchor: Default::default(),
            }],
            transition: Default::default(),
            duration_ms: 300,
            easing: Default::default(),
            delay_ms: 0,
        }];
    }
    if let Some(NodeKind::Text(td)) = d.node_mut(t).map(|n| &mut n.kind) {
        td.characters = "ab".into();
        td.runs = vec![StyleRun { start: 0, end: 1, style: TextStyle { fills: Some(vec![Paint::solid(RED)]), ..TextStyle::default() } }];
    }
    if let Some(NodeKind::Page { background, .. }) = d.node_mut(p).map(|n| &mut n.kind) {
        *background = RED;
    }
    d.styles.push(Style {
        id: "s1".into(),
        name: "Red".into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![Paint::solid(RED)] },
        bound: Default::default(),
    });
    d.variables.collections.push(Collection {
        id: "c1".into(),
        name: "C".into(),
        modes: vec![Mode { id: "m1".into(), name: "M".into() }],
        extends: None,
    });
    d.variables.variables.push(Variable {
        id: "v1".into(),
        name: "red".into(),
        collection: "c1".into(),
        kind: VariableType::Color,
        values: BTreeMap::from([("m1".to_string(), VariableValue::Color(RED))]),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    d.document_colors.push(RED);
    (d, r)
}

fn all_colors(d: &mut Document) -> Vec<Color> {
    let mut out = Vec::new();
    let mut f = |c: &mut Color| out.push(*c);
    for id in d.ids().collect::<Vec<_>>() {
        d.node_mut(id).unwrap().for_each_color_mut(&mut f);
    }
    d.styles.iter_mut().for_each(|s| s.for_each_color_mut(&mut f));
    d.variables.for_each_color_mut(&mut f);
    d.document_colors.iter_mut().for_each(&mut f);
    out
}

#[test]
fn converting_keeps_appearance_everywhere() {
    let (mut d, _) = colourful();
    let reds = all_colors(&mut d.clone()).into_iter().filter(|c| *c == RED).count();
    assert_eq!(reds, 10, "fill, stroke, shadow, grid, overlay, text run, page, style, variable, document colour");
    assert!(d.set_color_profile(ColorProfile::DisplayP3, true));
    assert_eq!(d.color_profile, ColorProfile::DisplayP3);
    let p3_red = to_p3().apply(RED);
    assert_eq!(all_colors(&mut d).iter().filter(|c| **c == p3_red).count(), reds, "every red was converted");
    assert!(!d.set_color_profile(ColorProfile::DisplayP3, true), "already Display P3");
    // And back again.
    d.set_color_profile(ColorProfile::Srgb, true);
    assert_eq!(all_colors(&mut d).iter().filter(|c| c.to_hex() == "FF0000").count(), reds);
}

#[test]
fn assigning_keeps_values() {
    let (mut d, r) = colourful();
    assert!(d.set_color_profile(ColorProfile::DisplayP3, false));
    assert_eq!(d.node(r).unwrap().fills[0].solid_color(), Some(RED));
    assert_eq!(d.color_profile, ColorProfile::DisplayP3);
}

#[test]
fn in_srgb_borrows_or_converts() {
    let (mut d, r) = colourful();
    assert!(matches!(d.in_srgb(), std::borrow::Cow::Borrowed(_)));
    d.set_color_profile(ColorProfile::DisplayP3, false);
    let s = d.in_srgb();
    assert_eq!(s.color_profile, ColorProfile::Srgb);
    // Display P3 red is more saturated than sRGB can show: it clips to sRGB red.
    assert_eq!(s.node(r).unwrap().fills[0].solid_color().unwrap().to_hex(), "FF0000");
    let p3_green = ColorProfile::DisplayP3.conversion_to(ColorProfile::Srgb).apply(Color::new(0.2, 0.6, 0.3, 1.0));
    assert_ne!(p3_green.to_hex(), Color::new(0.2, 0.6, 0.3, 1.0).to_hex(), "in-gamut colours move");
    assert_eq!(d.color_profile, ColorProfile::DisplayP3, "the original is untouched");
}

#[test]
fn profile_and_presets_serialize_only_when_set() {
    let mut d = Document::new("s");
    let v = serde_json::to_value(&d).unwrap();
    assert!(v.get("color_profile").is_none() && v.get("image_presets").is_none());
    d.color_profile = ColorProfile::DisplayP3;
    d.image_presets.push(ImagePreset { name: "Mine".into(), filters: ImageFilters { contrast: 0.5, ..Default::default() } });
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["color_profile"], "displayP3");
    let back: Document = serde_json::from_value(v).unwrap();
    assert_eq!(back.color_profile, ColorProfile::DisplayP3);
    assert_eq!(back.image_presets, d.image_presets);
}

#[test]
fn text_decoration_colour_is_visited() {
    let mut s = TextStyle::default();
    s.set_look(|l| l.color = Some(RED));
    s.for_each_color_mut(&mut |c| *c = Color::BLACK);
    assert_eq!(s.look().color, Some(Color::BLACK), "decoration colour not visited");
}
