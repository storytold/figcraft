//! Effects behaviour, written from the public documentation of the design tools FigCraft follows:
//! drop and inner shadows (offset, blur, spread, colour, showing behind transparent areas),
//! layer and background blurs, effects on groups, the order effects stack in and what exports
//! include. Each case says in one line why it expects what it does; where the documentation is
//! silent the case says so and pins today's behaviour. Scenes are small and synthetic, drawn on
//! a white page at 1×.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Blur, BlurType, Document, Effect, EffectKind, NodeId, NodeKind, Shadow, StrokeAlign};

use super::paint_behaviour_tests::{Probes, Pt, Px, hex, near, page, px, rect, render, solid_at};
use super::*;

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [255, 0, 0];

fn fx(kind: EffectKind) -> Effect {
    Effect { kind, visible: true }
}

/// A sharp, opaque black shadow.
fn sharp(offset: (f64, f64), spread: f64) -> Shadow {
    Shadow { color: Color::BLACK, offset, radius: 0.0, spread, ..Shadow::default() }
}

/// A 40 × 40 rectangle at (50, 50) with `fills` and `effects`.
fn square(fills: Vec<Paint>, effects: Vec<Effect>) -> (Document, NodeId) {
    let (mut d, p) = page();
    let r = rect(&mut d, p, (50.0, 50.0, 40.0, 40.0), fills);
    d.get_mut(r).unwrap().effects = effects;
    (d, r)
}

/// The standard normal distribution function (Abramowitz–Stegun 7.1.26 for erf).
fn phi(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 { 0.5 * (1.0 + erf) } else { 0.5 * (1.0 - erf) }
}

#[test]
fn new_effects_start_from_the_documented_defaults() {
    // A new shadow: offset 0, 4, blur 4, spread 0, black at 25%, normal blend, not shown behind
    // transparent areas; a new blur: 4, uniform.
    let shadow = Shadow { color: Color::new(0.0, 0.0, 0.0, 0.25), offset: (0.0, 4.0), radius: 4.0, spread: 0.0, ..Shadow::default() };
    for (ty, want) in [
        ("dropShadow", EffectKind::DropShadow(shadow.clone())),
        ("innerShadow", EffectKind::InnerShadow(shadow.clone())),
        ("layerBlur", EffectKind::LayerBlur(Blur::uniform(4.0))),
        ("backgroundBlur", EffectKind::BackgroundBlur(Blur::uniform(4.0))),
    ] {
        let e = Effect::of_type(ty).unwrap();
        assert_eq!(e.kind, want, "{ty}");
        assert!(e.visible, "{ty} starts visible");
    }
    assert!(!shadow.show_behind && shadow.blend == figcraft_color::BlendMode::Normal);
    assert_eq!(Blur::default().blur_type, BlurType::Uniform);
}

#[test]
fn drop_shadows_move_by_the_offset_and_grow_by_the_spread() {
    // (why, offset, spread, colour, probes). The red square covers 50..90 on both axes.
    let rows: Vec<(&str, Pt, f64, Color, Probes)> = vec![
        (
            "x moves the shadow right; the layer paints over it",
            (10.0, 0.0),
            0.0,
            Color::BLACK,
            vec![((95, 70), BLACK), ((45, 70), WHITE), ((70, 70), RED)],
        ),
        ("y moves it down", (0.0, 10.0), 0.0, Color::BLACK, vec![((70, 95), BLACK), ((70, 45), WHITE)]),
        ("negative offsets move it up and left", (-10.0, -10.0), 0.0, Color::BLACK, vec![((45, 45), BLACK), ((95, 95), WHITE)]),
        (
            "spread grows the shadow on every side",
            (10.0, 0.0),
            5.0,
            Color::BLACK,
            vec![((102, 70), BLACK), ((107, 70), WHITE), ((70, 47), BLACK), ((70, 43), WHITE)],
        ),
        (
            "negative spread shrinks it (here, out of sight under the layer)",
            (0.0, 0.0),
            -5.0,
            Color::BLACK,
            vec![((48, 70), WHITE), ((92, 70), WHITE)],
        ),
        ("the colour's alpha is the shadow's opacity", (10.0, 0.0), 0.0, Color::new(0.0, 0.0, 0.0, 0.25), vec![((95, 70), [191, 191, 191])]),
        ("the shadow takes its colour", (10.0, 0.0), 0.0, hex("0000FF"), vec![((95, 70), [0, 0, 255])]),
    ];
    for (why, offset, spread, color, probes) in rows {
        let (d, _) = square(vec![Paint::solid(hex("FF0000"))], vec![fx(EffectKind::DropShadow(Shadow { color, ..sharp(offset, spread) }))]);
        let pix = render(&d);
        for ((x, y), want) in probes {
            let got = px(&pix, x, y);
            assert!(near(got, want, 2), "{why} at ({x}, {y}): {got:?} want {want:?}");
        }
    }
}

#[test]
fn blurs_spread_by_a_gaussian_of_half_the_radius() {
    // The blur value is the CSS blur radius (what code inspection shows for it), which CSS turns
    // into a Gaussian with a standard deviation of half the radius: across a straight edge the
    // coverage falls off as the normal distribution function. Radius 20: sigma 10.
    let sigma = 10.0;
    let edge = |d: f64| ((1.0 - phi(-d / sigma)) * 255.0).round() as u8;
    // Drop shadow of a white 120 × 120 square at (40, 40), black, no offset.
    let (mut d, p) = page();
    let r = rect(&mut d, p, (40.0, 40.0, 120.0, 120.0), vec![Paint::solid(Color::WHITE)]);
    d.get_mut(r).unwrap().effects = vec![fx(EffectKind::DropShadow(Shadow { radius: 20.0, ..sharp((0.0, 0.0), 0.0) }))];
    let shadow = render(&d);
    // Layer blur of a black square, same place.
    let (mut d, p) = page();
    let r = rect(&mut d, p, (40.0, 40.0, 120.0, 120.0), vec![Paint::solid(Color::BLACK)]);
    d.get_mut(r).unwrap().effects = vec![fx(EffectKind::LayerBlur(Blur::uniform(20.0)))];
    let layer = render(&d);
    // Background blur: black left of x = 160 seen through a clear pane across the edge.
    let (mut d, p) = page();
    rect(&mut d, p, (0.0, 0.0, 160.0, 200.0), vec![Paint::solid(Color::BLACK)]);
    let pane = rect(&mut d, p, (110.0, 40.0, 90.0, 120.0), vec![]);
    d.get_mut(pane).unwrap().effects = vec![fx(EffectKind::BackgroundBlur(Blur::uniform(20.0)))];
    let back = render(&d);
    // (blur, pixel column, its centre's distance past the edge at x = 160)
    let rows = [
        ("drop shadow", &shadow, 160, 0.5),
        ("drop shadow", &shadow, 170, 10.5),
        ("drop shadow", &shadow, 180, 20.5),
        ("layer blur", &layer, 150, -9.5),
        ("layer blur", &layer, 160, 0.5),
        ("layer blur", &layer, 170, 10.5),
        ("background blur", &back, 150, -9.5),
        ("background blur", &back, 160, 0.5),
        ("background blur", &back, 170, 10.5),
    ];
    for (why, pix, x, dist) in rows {
        let (got, want) = (px(pix, x, 100)[1], edge(dist));
        assert!(got.abs_diff(want) <= 8, "{why} at x {x}: {got} want {want}");
    }
}

#[test]
fn shadows_show_behind_transparent_areas_only_when_asked() {
    // Off (the default), a drop shadow doesn't show through the layer: the parts it covers,
    // translucent ones included, hide the shadow. On, the shadow shows through them. The shadow
    // is as strong as the layer's pixels are opaque (silent, pinned): a 50% fill casts a 50% one.
    let stroke_only = |d: &mut Document, r: NodeId| {
        let n = d.get_mut(r).unwrap();
        n.strokes = vec![Paint::solid(hex("FF0000"))];
        (n.stroke.weight, n.stroke.align) = (4.0, StrokeAlign::Inside);
    };
    type Setup = fn(&mut Document, NodeId);
    let half = [128, 128, 128];
    // (why, fill opacity (none: no fill), layer opacity, show behind, offset, extra, probes)
    type Row<'a> = (&'a str, Option<f32>, f32, bool, Pt, Option<Setup>, Probes);
    let rows: Vec<Row> = vec![
        ("off: a 50% fill shows the page, not the shadow, through it", Some(0.5), 1.0, false, (0.0, 0.0), None, vec![((70, 70), [255, 128, 128])]),
        ("on: the shadow shows through the 50% fill", Some(0.5), 1.0, true, (0.0, 0.0), None, vec![((70, 70), [191, 64, 64])]),
        (
            "off: outside the layer the shadow shows as ever",
            Some(0.5),
            1.0,
            false,
            (10.0, 0.0),
            None,
            vec![((95, 70), half), ((85, 70), [255, 128, 128])],
        ),
        ("on: offset, it shows under the layer too", Some(0.5), 1.0, true, (10.0, 0.0), None, vec![((95, 70), half), ((85, 70), [191, 64, 64])]),
        (
            "layer opacity fades the layer and its shadow together",
            Some(1.0),
            0.5,
            false,
            (10.0, 0.0),
            None,
            vec![((70, 70), [255, 128, 128]), ((95, 70), [128, 128, 128])],
        ),
        (
            "a stroke without a fill: the empty middle isn't part of the layer",
            None,
            1.0,
            false,
            (10.0, 0.0),
            Some(stroke_only),
            vec![((62, 70), BLACK), ((52, 70), RED)],
        ),
    ];
    for (why, fill, layer_op, show_behind, offset, extra, probes) in rows {
        let fills = fill.map(|o| vec![solid_at("FF0000", o)]).unwrap_or_default();
        let (mut d, r) = square(fills, vec![fx(EffectKind::DropShadow(Shadow { show_behind, ..sharp(offset, 0.0) }))]);
        d.get_mut(r).unwrap().opacity = layer_op;
        if let Some(f) = extra {
            f(&mut d, r);
        }
        let pix = render(&d);
        for ((x, y), want) in probes {
            let got = px(&pix, x, y);
            assert!(near(got, want, 2), "{why} at ({x}, {y}): {got:?} want {want:?}");
        }
    }
}

#[test]
fn effects_on_a_group_apply_to_the_group_as_a_whole() {
    // A red square over a blue one, shadow (10, 10). On the group the pair casts one shadow;
    // on each layer the top one's shadow falls on the bottom one.
    for (why, on_group, want) in
        [("on the group: one shadow, under both", true, [0, 0, 255]), ("on each layer: the top layer shadows the bottom one", false, BLACK)]
    {
        let (mut d, p) = page();
        let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 200.0, 200.0).unwrap();
        let blue = rect(&mut d, g, (60.0, 60.0, 40.0, 40.0), vec![Paint::solid(hex("0000FF"))]);
        let red = rect(&mut d, g, (40.0, 40.0, 40.0, 40.0), vec![Paint::solid(hex("FF0000"))]);
        let shadow = vec![fx(EffectKind::DropShadow(sharp((10.0, 10.0), 0.0)))];
        if on_group {
            d.get_mut(g).unwrap().effects = shadow;
        } else {
            for id in [blue, red] {
                d.get_mut(id).unwrap().effects = shadow.clone();
            }
        }
        let pix = render(&d);
        // (85, 70): on the blue square, inside the red one's shadow, outside the red square.
        assert!(near(px(&pix, 85, 70), want, 2), "{why}: {:?}", px(&pix, 85, 70));
        assert!(near(px(&pix, 105, 105), BLACK, 2), "{why}: the blue square's own shadow");
    }
}

/// A group's or an unclipped frame's effects apply to what its children paint, children reaching
/// past its box included: its layer holds that too, grown by the effects, so nothing is cut where
/// a child's paint ends; and an unclipped frame's export holds it.
#[test]
fn a_containers_effects_reach_past_what_its_children_paint() {
    let blur = || fx(EffectKind::LayerBlur(Blur::uniform(20.0)));
    let shadow = |dy: f64| fx(EffectKind::DropShadow(sharp((0.0, dy), 0.0)));
    // (why, container, its effects, the child's box, the child's effects)
    let rows = [
        ("a blurred group over a child's shadow", NodeKind::Group, vec![blur()], (20.0, 20.0, 60.0, 60.0), vec![shadow(80.0)]),
        ("an unclipped frame's shadow of a child sticking out", NodeKind::Frame, vec![shadow(40.0)], (60.0, 60.0, 80.0, 80.0), vec![]),
        ("an unclipped frame's blur of a child sticking out", NodeKind::Frame, vec![blur()], (60.0, 60.0, 80.0, 80.0), vec![]),
    ];
    for (why, kind, effects, (x, y, w, h), child_fx) in rows {
        // `room`: an empty child as large as the view, so the layer can't be cut: what it should draw.
        let scene = |room: bool| {
            let (mut d, p) = page();
            let c = d.create(p, kind.clone(), "C", 0.0, 0.0, 100.0, 100.0).unwrap();
            let node = d.get_mut(c).unwrap();
            node.clip = false;
            node.fills.clear();
            node.effects = effects.clone();
            let r = rect(&mut d, c, (x, y, w, h), vec![Paint::solid(hex("FF0000"))]);
            d.get_mut(r).unwrap().effects = child_fx.clone();
            if room {
                rect(&mut d, c, (-200.0, -200.0, 600.0, 600.0), vec![]);
            }
            (d, c)
        };
        let ((d, c), (truth, _)) = (scene(false), scene(true));
        let v = View { origin: (-100.0, -100.0), scale: 1.0, width: 400, height: 400 };
        let (a, b) = (
            render_view_single(&d, page_of(&d), &v, &Options::default()).unwrap(),
            render_view_single(&truth, page_of(&truth), &v, &Options::default()).unwrap(),
        );
        let worst = a.data().iter().zip(b.data()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        // The export holds all of it: as much paint as the export with room to spare.
        let ink = |p: Pixmap| p.pixels().iter().map(|c| u64::from(c.alpha())).sum::<u64>();
        let (got, all) = (ink(render_node(&d, c, 1.0).unwrap()), ink(render_node(&truth, page_child(&truth), 1.0).unwrap()));
        assert!(worst <= 1, "{why}: the canvas draws {worst} levels off");
        assert!(got * 1000 >= all * 999, "{why}: the export holds {got} of {all} of the paint");
    }
}

#[test]
fn effects_reach_as_far_under_a_scaled_or_turned_transform() {
    use figcraft_geom::kurbo::Affine;
    let shadow = |d: (f64, f64)| fx(EffectKind::DropShadow(sharp(d, 0.0)));
    let spread = |s: f64| fx(EffectKind::DropShadow(sharp((0.0, 0.0), s)));
    let turned = Affine::translate((150.0, 50.0)) * Affine::rotate(std::f64::consts::FRAC_PI_4);
    // (why, kind, transform, effects, a child's effects): effects are page units however a layer
    // is transformed, as the renderer draws them, but for a shadow's spread of a filled outline,
    // which grows the outline before it is transformed.
    let rows = [
        ("a halved rectangle's shadow", NodeKind::Rectangle, Affine::scale(0.5), vec![shadow((0.0, 80.0))], None),
        ("a turned rectangle's shadow", NodeKind::Rectangle, turned, vec![shadow((40.0, 40.0))], None),
        ("a halved group's shadow", NodeKind::Group, Affine::scale(0.5), vec![shadow((0.0, 80.0))], Some(vec![])),
        ("a shadow in a halved group", NodeKind::Group, Affine::scale(0.5), vec![], Some(vec![shadow((0.0, 80.0))])),
        ("a flattened rectangle's shadow", NodeKind::Rectangle, Affine::scale_non_uniform(1.0, 0.25), vec![shadow((0.0, 80.0))], None),
        ("a doubled rectangle's shadow", NodeKind::Rectangle, Affine::scale(2.0), vec![shadow((0.0, 80.0))], None),
        ("a doubled rectangle's spread shadow", NodeKind::Rectangle, Affine::scale(2.0), vec![spread(20.0)], None),
        ("a doubled star's spread shadow", NodeKind::Star { count: 5, inner_ratio: 0.4 }, Affine::scale(2.0), vec![spread(10.0)], None),
        ("a doubled group's spread shadow", NodeKind::Group, Affine::scale(2.0), vec![spread(20.0)], Some(vec![])),
        ("a turned star's spread shadow", NodeKind::Star { count: 5, inner_ratio: 0.4 }, turned, vec![spread(20.0)], None),
    ];
    for (why, kind, xf, effects, child_fx) in rows {
        // `room`: a clear drop shadow wider than the view on each layer, so none can be cut: what
        // they should draw.
        let scene = |room: bool| {
            let clear = fx(EffectKind::DropShadow(Shadow { color: Color::TRANSPARENT, radius: 400.0, ..sharp((0.0, 0.0), 0.0) }));
            let with_room = |mut e: Vec<Effect>| {
                if room {
                    e.push(clear.clone());
                }
                e
            };
            let (mut d, p) = page();
            let n = d.create(p, kind.clone(), "N", 0.0, 0.0, 100.0, 100.0).unwrap();
            if let Some(cfx) = &child_fx {
                let r = rect(&mut d, n, (0.0, 0.0, 100.0, 100.0), vec![Paint::solid(hex("FF0000"))]);
                d.get_mut(r).unwrap().effects = with_room(cfx.clone());
            }
            let node = d.get_mut(n).unwrap();
            node.fills = vec![Paint::solid(hex("FF0000"))];
            node.transform = xf;
            node.effects = with_room(effects.clone());
            (d, n)
        };
        let ((d, n), (truth, _)) = (scene(false), scene(true));
        let v = View { origin: (-100.0, -100.0), scale: 1.0, width: 400, height: 400 };
        let (a, b) = (
            render_view_single(&d, page_of(&d), &v, &Options::default()).unwrap(),
            render_view_single(&truth, page_of(&truth), &v, &Options::default()).unwrap(),
        );
        let worst = a.data().iter().zip(b.data()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        let ink = |p: Pixmap| p.pixels().iter().map(|c| u64::from(c.alpha())).sum::<u64>();
        let (got, all) = (ink(render_node(&d, n, 1.0).unwrap()), ink(render_node(&truth, page_child(&truth), 1.0).unwrap()));
        assert!(worst <= 1, "{why}: the canvas draws {worst} levels off");
        assert!(got * 1000 >= all * 999, "{why}: the export holds {got} of {all} of the paint");
    }
}

#[test]
fn effects_reach_as_far_under_a_tiny_scale() {
    // A rectangle `w` units wide drawn 100 × 100 on the page casts the shadow a 100 × 100 one does:
    // shadows are page units.
    let scene = |w: f64| {
        let (mut d, p) = page();
        let r = rect(&mut d, p, (0.0, 0.0, w, w), vec![Paint::solid(hex("FF0000"))]);
        let n = d.get_mut(r).unwrap();
        n.transform = figcraft_geom::kurbo::Affine::scale(100.0 / w);
        n.effects = vec![fx(EffectKind::DropShadow(sharp((0.0, 80.0), 0.0)))];
        d
    };
    let v = View { origin: (-100.0, -100.0), scale: 1.0, width: 400, height: 400 };
    let truth = scene(100.0);
    let b = render_view_single(&truth, page_of(&truth), &v, &Options::default()).unwrap();
    for w in [1e6, 1e8, 1e9] {
        let d = scene(w);
        let a = render_view_single(&d, page_of(&d), &v, &Options::default()).unwrap();
        let worst = a.data().iter().zip(b.data()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        assert!(worst <= 1, "{w} units wide: the canvas draws {worst} levels off");
    }
}

#[test]
fn a_spread_counts_once_in_an_export() {
    // A 100 × 100 rectangle with a shadow spread by 20 all round, unscaled and doubled.
    for (scale, side) in [(1.0, 140), (2.0, 280)] {
        let (mut d, p) = page();
        let r = rect(&mut d, p, (0.0, 0.0, 100.0, 100.0), vec![Paint::solid(hex("FF0000"))]);
        let n = d.get_mut(r).unwrap();
        n.transform = figcraft_geom::kurbo::Affine::scale(scale);
        n.effects = vec![fx(EffectKind::DropShadow(sharp((0.0, 0.0), 20.0)))];
        let px = render_node(&d, r, 1.0).unwrap();
        assert_eq!((px.width(), px.height()), (side, side), "at scale {scale}");
    }
}

fn page_of(d: &Document) -> NodeId {
    d.first_page().unwrap()
}

/// The page's first layer.
fn page_child(d: &Document) -> NodeId {
    *d.node(page_of(d)).unwrap().children.first().unwrap()
}

#[test]
fn inner_shadows_paint_inside_over_the_fills_under_strokes_and_children() {
    let green = [0, 255, 0];
    type Setup = fn(&mut Document, NodeId);
    let inside_stroke: Setup = |d, r| {
        let n = d.get_mut(r).unwrap();
        n.strokes = vec![Paint::solid(hex("00FF00"))];
        (n.stroke.weight, n.stroke.align) = (4.0, StrokeAlign::Inside);
    };
    let child: Setup = |d, r| {
        rect(d, r, (0.0, 0.0, 40.0, 3.0), vec![Paint::solid(hex("0000FF"))]);
    };
    // (why, kind, offset, spread, extra, probes) on a white 40 × 40 square at (50, 50).
    type Row<'a> = (&'a str, NodeKind, Pt, f64, Option<Setup>, Probes);
    let rows: Vec<Row> = vec![
        (
            "y moves the outside in from the top edge",
            NodeKind::Rectangle,
            (0.0, 10.0),
            0.0,
            None,
            vec![((70, 55), BLACK), ((70, 65), WHITE), ((70, 85), WHITE)],
        ),
        ("x moves it in from the left edge", NodeKind::Rectangle, (10.0, 0.0), 0.0, None, vec![((55, 70), BLACK), ((65, 70), WHITE)]),
        (
            "spread grows it in from every edge",
            NodeKind::Rectangle,
            (0.0, 0.0),
            5.0,
            None,
            vec![((52, 70), BLACK), ((57, 70), WHITE), ((70, 52), BLACK), ((87, 70), BLACK)],
        ),
        ("it never paints outside the layer", NodeKind::Rectangle, (0.0, 10.0), 0.0, None, vec![((70, 45), WHITE), ((70, 95), WHITE)]),
        ("strokes paint over it", NodeKind::Rectangle, (0.0, 0.0), 5.0, Some(inside_stroke), vec![((51, 70), green), ((54, 70), BLACK)]),
        ("a frame's children paint over it", NodeKind::Frame, (0.0, 0.0), 5.0, Some(child), vec![((70, 51), [0, 0, 255]), ((70, 54), BLACK)]),
    ];
    for (why, kind, offset, spread, extra, probes) in rows {
        let (mut d, p) = page();
        let r = d.create(p, kind, "S", 50.0, 50.0, 40.0, 40.0).unwrap();
        let n = d.get_mut(r).unwrap();
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.effects = vec![fx(EffectKind::InnerShadow(sharp(offset, spread)))];
        if let Some(f) = extra {
            f(&mut d, r);
        }
        let pix = render(&d);
        for ((x, y), want) in probes {
            let got = px(&pix, x, y);
            assert!(near(got, want, 2), "{why} at ({x}, {y}): {got:?} want {want:?}");
        }
    }
}

#[test]
fn effects_stack_in_list_order() {
    let red = Shadow { color: hex("FF0000"), ..sharp((10.0, 0.0), 0.0) };
    let blue = Shadow { color: hex("0000FF"), ..sharp((10.0, 0.0), 0.0) };
    let hidden = Effect { visible: false, ..fx(EffectKind::DropShadow(sharp((10.0, 0.0), 0.0))) };
    // (why, effects, pixel, check)
    type Check = fn([u8; 3]) -> bool;
    let rows: Vec<(&str, Vec<Effect>, Px, Check)> = vec![
        (
            "two shadows: the later one in the list paints over the earlier (silent, pinned like paints)",
            vec![fx(EffectKind::DropShadow(red)), fx(EffectKind::DropShadow(blue))],
            (95, 70),
            |c| c == [0, 0, 255],
        ),
        ("a hidden effect does nothing", vec![hidden], (95, 70), |c| c == WHITE),
        (
            "a layer blur blurs the layer before it casts its shadow (silent, pinned)",
            vec![fx(EffectKind::DropShadow(sharp((20.0, 0.0), 0.0))), fx(EffectKind::LayerBlur(Blur::uniform(10.0)))],
            (108, 70),
            |c| (30..=220).contains(&c[0]),
        ),
    ];
    for (why, effects, (x, y), ok) in rows {
        let (d, _) = square(vec![Paint::solid(hex("808080"))], effects);
        let got = px(&render(&d), x, y);
        assert!(ok(got), "{why}: {got:?}");
    }
}

#[test]
fn background_blur_shows_through_the_fill_inside_the_shape() {
    // Black left of x = 100 under a pane (50..150) with a background blur of 20: at x = 110 the
    // blurred backdrop is 218 (sigma 10, 10.5 px past the edge).
    let blurred = 218;
    // (why, pane fills, pixel, colour there)
    let rows: Vec<(&str, Vec<Paint>, Px, [u8; 3])> = vec![
        ("no fill: the blurred backdrop", vec![], (110, 100), [blurred; 3]),
        ("a 0% fill: the blurred backdrop", vec![solid_at("FF0000", 0.0)], (110, 100), [blurred; 3]),
        ("a 50% fill paints over the blurred backdrop", vec![solid_at("FF0000", 0.5)], (110, 100), [236, 109, 109]),
        ("an opaque fill hides it", vec![Paint::solid(hex("FF0000"))], (110, 100), RED),
        ("outside the shape nothing is blurred", vec![], (110, 160), WHITE),
        ("outside the shape nothing is blurred (dark side)", vec![], (90, 160), BLACK),
    ];
    for (why, fills, (x, y), want) in rows {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, 100.0, 200.0), vec![Paint::solid(Color::BLACK)]);
        let pane = rect(&mut d, p, (50.0, 50.0, 100.0, 100.0), fills);
        d.get_mut(pane).unwrap().effects = vec![fx(EffectKind::BackgroundBlur(Blur::uniform(20.0)))];
        let got = px(&render(&d), x, y);
        assert!(near(got, want, 8), "{why}: {got:?} want {want:?}");
    }
}

#[test]
fn exports_include_what_effects_paint_outside_the_layer() {
    let size = |effects: Vec<Effect>| {
        let (d, r) = square(vec![Paint::solid(hex("FF0000"))], effects);
        let p = render_node(&d, r, 1.0).unwrap();
        (p.width(), p.height())
    };
    // (why, effects, grows the export)
    let rows = [
        ("nothing: the layer's box", vec![], false),
        ("a drop shadow reaches out", vec![fx(EffectKind::DropShadow(sharp((0.0, 10.0), 0.0)))], true),
        ("a layer blur reaches out", vec![fx(EffectKind::LayerBlur(Blur::uniform(4.0)))], true),
        ("an inner shadow stays inside", vec![fx(EffectKind::InnerShadow(sharp((0.0, 10.0), 0.0)))], false),
        ("a background blur stays inside", vec![fx(EffectKind::BackgroundBlur(Blur::uniform(4.0)))], false),
        ("noise stays on the layer's pixels", vec![fx(EffectKind::Noise(figcraft_doc::Noise::default()))], false),
        ("texture roughens past the edge", vec![fx(EffectKind::Texture(figcraft_doc::Texture::default()))], true),
        (
            "texture clipped to the shape stays inside",
            vec![fx(EffectKind::Texture(figcraft_doc::Texture { clip_to_shape: true, ..Default::default() }))],
            false,
        ),
        ("glass stays inside", vec![fx(EffectKind::Glass(figcraft_doc::Glass::default()))], false),
    ];
    for (why, effects, grows) in rows {
        let (w, h) = size(effects);
        // Effects grow the export only on the sides they reach (a shadow falling down grows its height).
        assert!(w >= 40 && h >= 40, "{why}: {w} × {h}");
        assert_eq!(w > 40 || h > 40, grows, "{why}: {w} × {h}");
    }
    // The export holds the shadow where the canvas draws it, pixel for pixel.
    let (d, r) = square(vec![Paint::solid(hex("FF0000"))], vec![fx(EffectKind::DropShadow(Shadow { radius: 6.0, ..sharp((0.0, 10.0), 2.0) }))]);
    let b = export_bounds(&d, r).unwrap();
    let export = render_node(&d, r, 1.0).unwrap();
    let mut canvas = Pixmap::new(export.width(), export.height()).unwrap();
    render_nodes_into(&d, &[r], &View { origin: (b.x0, b.y0), scale: 1.0, width: export.width(), height: export.height() }, &mut canvas);
    assert_eq!(export.data(), canvas.data(), "export and canvas agree");
    assert!(b.y1 >= 90.0 + 10.0 + 2.0 + 6.0, "the shadow's blur and spread are inside the export: {b:?}");
    // A clipping frame exports its shadow too.
    let (mut d, p) = page();
    let f = d.create(p, NodeKind::Frame, "F", 50.0, 50.0, 40.0, 40.0).unwrap();
    d.get_mut(f).unwrap().effects = vec![fx(EffectKind::DropShadow(sharp((0.0, 10.0), 0.0)))];
    assert!(render_node(&d, f, 1.0).unwrap().height() >= 50, "frame shadow exported");
}

/// Layer blurs and texture apply one after another and drop shadows are cast from what they leave:
/// the export holds what they reach together, so nothing is cut at its edges.
#[test]
fn exports_hold_what_stacked_effects_reach_together() {
    let blur = |r: f64| fx(EffectKind::LayerBlur(Blur::uniform(r)));
    let rows = [
        ("four blurs of 25", vec![blur(25.0), blur(25.0), blur(25.0), blur(25.0)]),
        ("eight blurs of 10", (0..8).map(|_| blur(10.0)).collect()),
        ("sixteen blurs of 3", (0..16).map(|_| blur(3.0)).collect()),
        ("texture over blurs", vec![blur(15.0), blur(15.0), fx(EffectKind::Texture(figcraft_doc::Texture { radius: 12.0, ..Default::default() }))]),
        ("a blur under a shadow", vec![blur(20.0), fx(EffectKind::DropShadow(sharp((0.0, 30.0), 0.0)))]),
        ("a shadow then a blur", vec![fx(EffectKind::DropShadow(sharp((0.0, 30.0), 0.0))), blur(20.0)]),
    ];
    for (why, effects) in rows {
        let (d, r) = square(vec![Paint::solid(hex("FF0000"))], effects);
        let p = render_node(&d, r, 1.0).unwrap();
        let (w, h) = (p.width(), p.height());
        let alpha = |x: u32, y: u32| p.pixel(x, y).map_or(0, |c| c.alpha());
        let edge = (0..w).flat_map(|x| [alpha(x, 0), alpha(x, h - 1)]).chain((0..h).flat_map(|y| [alpha(0, y), alpha(w - 1, y)])).max().unwrap_or(0);
        assert!(edge <= 1, "{why}: the export's edge has alpha {edge} ({w} × {h})");
    }
}

/// A black 200 × 200 frame with a uniform layer blur of `radius`, drawn on a 512 × 512 view at 1×.
fn blurred_frame(radius: f64) -> Pixmap {
    let mut d = Document::new("Blur range");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "Square", 156.0, 156.0, 200.0, 200.0).unwrap();
    let n = d.get_mut(f).unwrap();
    n.fills = vec![Paint::solid(Color::BLACK)];
    n.effects = vec![fx(EffectKind::LayerBlur(Blur::uniform(radius)))];
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 512, height: 512 };
    render_view(&d, p, &view, &Options::default()).unwrap()
}

#[test]
fn layer_blur_radii_up_to_the_documented_limit_all_draw() {
    // The documented blur range runs to 10,000: a larger radius in it blurs further, it doesn't
    // stop at some smaller one.
    let differ = |a: &Pixmap, b: &Pixmap| a.data().chunks_exact(4).zip(b.data().chunks_exact(4)).filter(|(a, b)| a != b).count();
    let sharp = blurred_frame(0.0);
    let small = blurred_frame(100.0);
    let medium = blurred_frame(1_000.0);
    let large = blurred_frame(10_000.0);
    assert!(differ(&sharp, &small) > 0, "radius 0 draws the sharp shape");
    assert!(differ(&small, &medium) > 0, "100 vs 1000");
    assert!(differ(&medium, &large) > 0, "radii 1000 and 10000 draw the same pixels");
}
