//! Paint behaviour, written from the public documentation of the design tools FigCraft follows:
//! fill stacking and opacity, paint and layer blend modes (pass-through and isolating groups),
//! the four gradient kinds and their handles, image scale modes, rotation and adjustments, the
//! colour models and the document colour profile. Each case says in one line why it expects what
//! it does; where the documentation is silent the case says so and pins today's behaviour.
//! Scenes are small and synthetic, drawn on a white page at 1×.

use figcraft_color::{BlendMode, Color, ColorProfile, ColorStop, Gradient, GradientKind, Hsb, Hsl, ImageFilters, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Document, ImageData, NodeId, NodeKind, StrokeAlign};

use super::*;

/// A pixel, and pixels with the colour each must have.
pub(super) type Px = (u32, u32);
pub(super) type Probes = Vec<(Px, [u8; 3])>;
/// A point or size in document units.
pub(super) type Pt = (f64, f64);

pub(super) fn hex(h: &str) -> Color {
    Color::from_hex(h).unwrap()
}

fn rgb(h: &str) -> [u8; 3] {
    let [r, g, b, _] = hex(h).to_rgba8();
    [r, g, b]
}

pub(super) fn page() -> (Document, NodeId) {
    let d = Document::new("paint");
    let p = d.first_page().unwrap();
    (d, p)
}

pub(super) fn rect(d: &mut Document, parent: NodeId, (x, y, w, h): (f64, f64, f64, f64), fills: Vec<Paint>) -> NodeId {
    let r = d.create(parent, NodeKind::Rectangle, "R", x, y, w, h).unwrap();
    d.get_mut(r).unwrap().fills = fills;
    r
}

/// The page's layers on a white 200 × 200 view at 1×.
pub(super) fn render(d: &Document) -> Pixmap {
    let p = d.first_page().unwrap();
    let kids = d.node(p).unwrap().children.clone();
    let mut pix = Pixmap::new(200, 200).unwrap();
    pix.fill(tiny_skia::Color::WHITE);
    render_nodes_into(d, &kids, &View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 }, &mut pix);
    pix
}

pub(super) fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    [p.data()[i], p.data()[i + 1], p.data()[i + 2]]
}

pub(super) fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

pub(super) fn solid_at(c: &str, opacity: f32) -> Paint {
    Paint { opacity, ..Paint::solid(hex(c)) }
}

// ---- fills: stacking and opacity ----

#[test]
fn fills_stack_bottom_to_top_with_their_own_opacity() {
    let hidden = Paint { visible: false, ..Paint::solid(hex("0000FF")) };
    let rows: Vec<(&str, Vec<Paint>, [u8; 3])> = vec![
        ("one fill paints its colour", vec![Paint::solid(hex("FF0000"))], [255, 0, 0]),
        (
            "the list's last paint is the panel's top row and covers the rest",
            vec![Paint::solid(hex("FF0000")), Paint::solid(hex("0000FF"))],
            [0, 0, 255],
        ),
        ("a top fill at 50% shows the fill under it through", vec![Paint::solid(hex("FF0000")), solid_at("0000FF", 0.5)], [128, 0, 128]),
        ("a hidden fill paints nothing", vec![Paint::solid(hex("FF0000")), hidden], [255, 0, 0]),
        ("a fill at 0% paints nothing", vec![Paint::solid(hex("FF0000")), solid_at("0000FF", 0.0)], [255, 0, 0]),
        ("a fill's opacity lets the page through", vec![solid_at("FF0000", 0.5)], [255, 128, 128]),
        (
            "a colour's own alpha and the paint opacity multiply",
            vec![Paint { kind: PaintKind::Solid { color: hex("FF0000").with_alpha(0.5) }, ..solid_at("FF0000", 0.5) }],
            [255, 191, 191],
        ),
        ("no fills: nothing", vec![], [255, 255, 255]),
    ];
    for (why, fills, want) in rows {
        let (mut d, p) = page();
        rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), fills);
        let got = px(&render(&d), 50, 50);
        assert!(near(got, want, 2), "{why}: {got:?} want {want:?}");
    }
}

#[test]
fn paint_opacity_and_layer_opacity_differ() {
    // A red fill under a blue 10-unit centre stroke; (22, 50) is inside both.
    let rows = [
        ("paint opacities: each paint shows what is under it, fill and page included", 0.5, 1.0, [128, 64, 191]),
        ("layer opacity: the layer is composited first, so the stroke hides the fill and only the page shows through", 1.0, 0.5, [128, 128, 255]),
        ("both at 100%: the stroke covers the fill", 1.0, 1.0, [0, 0, 255]),
    ];
    for (why, paint_op, layer_op, want) in rows {
        let (mut d, p) = page();
        let r = rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), vec![solid_at("FF0000", paint_op)]);
        let n = d.get_mut(r).unwrap();
        n.strokes = vec![solid_at("0000FF", paint_op)];
        n.stroke.weight = 10.0;
        n.stroke.align = StrokeAlign::Center;
        n.opacity = layer_op;
        let got = px(&render(&d), 22, 50);
        assert!(near(got, want, 2), "{why}: {got:?} want {want:?}");
    }
}

// ---- blend modes ----

/// The separable and non-separable blend functions of the W3C compositing spec (which the
/// documented blend modes are), plus plus-darker / plus-lighter (linear burn / dodge), for
/// opaque colours in 0..1.
fn blend_ref(mode: BlendMode, cb: [f64; 3], cs: [f64; 3]) -> [f64; 3] {
    let sep = |f: &dyn Fn(f64, f64) -> f64| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
    let screen = |b: f64, s: f64| b + s - b * s;
    let hard = |b: f64, s: f64| if s <= 0.5 { b * 2.0 * s } else { screen(b, 2.0 * s - 1.0) };
    let lum = |c: [f64; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let clip = |c: [f64; 3]| {
        let (l, n, x) = (lum(c), c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
        let mut c = c;
        if n < 0.0 {
            c = c.map(|v| l + (v - l) * l / (l - n));
        }
        if x > 1.0 {
            c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
        }
        c
    };
    let set_lum = |c: [f64; 3], l: f64| {
        let d = l - lum(c);
        clip(c.map(|v| v + d))
    };
    let sat = |c: [f64; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let set_sat = |c: [f64; 3], s: f64| {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        c.map(|v| if mx > mn { (v - mn) * s / (mx - mn) } else { 0.0 })
    };
    match mode {
        BlendMode::Normal | BlendMode::PassThrough => cs,
        BlendMode::Multiply => sep(&|b, s| b * s),
        BlendMode::Screen => sep(&screen),
        BlendMode::Overlay => sep(&|b, s| hard(s, b)),
        BlendMode::Darken => sep(&f64::min),
        BlendMode::Lighten => sep(&f64::max),
        BlendMode::ColorDodge => sep(&|b, s| {
            if b == 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        BlendMode::ColorBurn => sep(&|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        BlendMode::HardLight => sep(&hard),
        BlendMode::SoftLight => sep(&|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        BlendMode::Difference => sep(&|b, s| (b - s).abs()),
        BlendMode::Exclusion => sep(&|b, s| b + s - 2.0 * b * s),
        BlendMode::PlusDarker => sep(&|b, s| (b + s - 1.0).max(0.0)),
        BlendMode::PlusLighter => sep(&|b, s| (b + s).min(1.0)),
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        BlendMode::Color => set_lum(cs, lum(cb)),
        BlendMode::Luminosity => set_lum(cb, lum(cs)),
    }
}

#[test]
fn every_blend_mode_blends_a_paint_and_a_layer_by_its_formula() {
    // Every mode is found by its menu label; "Normal" is Normal (not pass-through, whose CSS
    // keyword is also `normal`).
    for mode in BlendMode::ALL {
        assert_eq!(BlendMode::from_name(mode.label()), Some(mode), "{mode:?} by its label");
    }
    let (bottom, top) = ("CC6633", "66CCEE");
    let unit = |h: &str| rgb(h).map(|v| f64::from(v) / 255.0);
    for mode in BlendMode::ALL {
        let want = blend_ref(mode, unit(bottom), unit(top)).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        // A paint's blend mode blends it with the paints under it in the same layer.
        let (mut d, p) = page();
        rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), vec![Paint::solid(hex(bottom)), Paint { blend: mode, ..Paint::solid(hex(top)) }]);
        let paint_px = px(&render(&d), 50, 50);
        // A layer's blend mode blends it with the layers under it.
        let (mut d, p) = page();
        rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), vec![Paint::solid(hex(bottom))]);
        let r = rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), vec![Paint::solid(hex(top))]);
        d.get_mut(r).unwrap().blend = mode;
        let layer_px = px(&render(&d), 50, 50);
        assert!(near(paint_px, want, 3), "paint {mode:?}: {paint_px:?} want {want:?}");
        assert!(near(layer_px, want, 3), "layer {mode:?}: {layer_px:?} want {want:?}");
    }
}

#[derive(Clone, Copy, Debug)]
enum Holder {
    Group,
    ClipFrame,
    RoundedClipFrame,
    OpenFrame,
}

#[test]
fn pass_through_containers_let_children_blend_with_what_is_behind_and_normal_ones_isolate() {
    // A yellow layer under a container holding a cyan layer set to Multiply: reaching the yellow,
    // multiply gives green; isolated, the cyan meets only what the container itself draws.
    let rows = [
        ("pass-through group: children blend with the layers below it", Holder::Group, BlendMode::PassThrough, None, [0, 255, 0]),
        ("a group set to Normal blends its children among themselves first", Holder::Group, BlendMode::Normal, None, [0, 255, 255]),
        ("pass-through frame that clips: clipping doesn't isolate", Holder::ClipFrame, BlendMode::PassThrough, None, [0, 255, 0]),
        ("pass-through frame with rounded corners that clips", Holder::RoundedClipFrame, BlendMode::PassThrough, None, [0, 255, 0]),
        ("pass-through frame that doesn't clip", Holder::OpenFrame, BlendMode::PassThrough, None, [0, 255, 0]),
        ("a frame set to Normal isolates its children", Holder::ClipFrame, BlendMode::Normal, None, [0, 255, 255]),
        ("a clipping frame's own fill is under its children", Holder::ClipFrame, BlendMode::PassThrough, Some("FF00FF"), [0, 0, 255]),
        ("an isolating frame's own fill is inside the isolation", Holder::ClipFrame, BlendMode::Normal, Some("FF00FF"), [0, 0, 255]),
    ];
    for (why, holder, blend, fill, want) in rows {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, 200.0, 200.0), vec![Paint::solid(hex("FFFF00"))]);
        let kind = if matches!(holder, Holder::Group) { NodeKind::Group } else { NodeKind::Frame };
        let c = d.create(p, kind, "C", 40.0, 40.0, 120.0, 120.0).unwrap();
        let n = d.get_mut(c).unwrap();
        n.blend = blend;
        n.fills = fill.map(|f| vec![Paint::solid(hex(f))]).unwrap_or_default();
        n.clip = matches!(holder, Holder::ClipFrame | Holder::RoundedClipFrame);
        if matches!(holder, Holder::RoundedClipFrame) {
            n.corners.radii = [20.0; 4];
        }
        let k = rect(&mut d, c, (10.0, 10.0, 100.0, 100.0), vec![Paint::solid(hex("00FFFF"))]);
        d.get_mut(k).unwrap().blend = BlendMode::Multiply;
        let got = px(&render(&d), 100, 100);
        assert!(near(got, want, 2), "{why} ({holder:?}): {got:?} want {want:?}");
    }
}

#[test]
fn clipping_still_clips_blending_children() {
    // The multiply child reaches past its clipping frame: outside, the yellow stays untouched.
    for radius in [0.0, 20.0] {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, 200.0, 200.0), vec![Paint::solid(hex("FFFF00"))]);
        let f = d.create(p, NodeKind::Frame, "F", 40.0, 40.0, 100.0, 100.0).unwrap();
        let n = d.get_mut(f).unwrap();
        (n.fills, n.clip, n.corners.radii) = (vec![], true, [radius; 4]);
        let k = rect(&mut d, f, (50.0, 50.0, 100.0, 100.0), vec![Paint::solid(hex("00FFFF"))]);
        d.get_mut(k).unwrap().blend = BlendMode::Multiply;
        let pix = render(&d);
        assert_eq!(px(&pix, 160, 160), [255, 255, 0], "radius {radius}: clipped part");
        assert!(near(px(&pix, 120, 120), [0, 255, 0], 2), "radius {radius}: inside");
        if radius > 0.0 {
            // The rounded corner clips too.
            assert_eq!(px(&pix, 138, 138), [255, 255, 0], "rounded corner");
        }
    }
}

// ---- gradients ----

fn grey(t: f64) -> [u8; 3] {
    let v = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
    [v, v, v]
}

fn black_to_white(kind: GradientKind, from: (f64, f64), to: (f64, f64), width: (f64, f64)) -> Paint {
    Paint::gradient(Gradient {
        kind,
        stops: vec![ColorStop { position: 0.0, color: Color::BLACK }, ColorStop { position: 1.0, color: Color::WHITE }],
        from,
        to,
        width,
    })
}

#[test]
fn gradient_handles_place_the_stops() {
    use GradientKind::*;
    let c = (0.5, 0.5);
    // (why, kind, from, to, width, box w × h, pixel, t there). The handles are in the layer's
    // unit box: the first is where stop 0 sits (the centre for the radial kinds), the second
    // where stop 1 sits, the third the extent across.
    type Row<'a> = (&'a str, GradientKind, Pt, Pt, Pt, Pt, Px, f64);
    let rows: Vec<Row> = vec![
        ("linear: t runs from the first handle to the second", Linear, (0.0, 0.5), (1.0, 0.5), (0.0, 1.0), (100.0, 100.0), (25, 50), 0.255),
        ("linear: before the first handle, the first stop", Linear, (0.25, 0.5), (0.75, 0.5), (0.25, 1.0), (100.0, 100.0), (10, 50), 0.0),
        ("linear: past the second handle, the last stop", Linear, (0.25, 0.5), (0.75, 0.5), (0.25, 1.0), (100.0, 100.0), (90, 50), 1.0),
        ("linear: top to bottom", Linear, (0.5, 0.0), (0.5, 1.0), (0.0, 0.0), (100.0, 100.0), (50, 75), 0.755),
        ("linear: the bands are square to the axis", Linear, (0.5, 0.0), (0.5, 1.0), (0.0, 0.0), (100.0, 100.0), (5, 75), 0.755),
        ("radial: t is the distance from the centre over the radius", Radial, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (75, 50), 0.51),
        ("radial: elliptical, by the second and third handles (across)", Radial, c, (1.0, 0.5), (0.5, 1.0), (200.0, 100.0), (150, 50), 0.505),
        ("radial: elliptical (down)", Radial, c, (1.0, 0.5), (0.5, 1.0), (200.0, 100.0), (100, 74), 0.49),
        ("radial: past the radius, the last stop", Radial, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (2, 2), 1.0),
        ("angular: a quarter turn from the second handle towards the third", Angular, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (50, 90), 0.25),
        ("angular: half a turn", Angular, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (10, 50), 0.5),
        ("angular: three quarters", Angular, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (50, 10), 0.75),
        ("angular: the third handle on the other side turns it the other way", Angular, c, (1.0, 0.5), (0.5, 0.0), (100.0, 100.0), (50, 90), 0.75),
        ("diamond: t is the sum of the distances along both axes", Diamond, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (75, 50), 0.52),
        ("diamond: on the diagonal", Diamond, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (62, 62), 0.5),
        ("diamond: past the edge, the last stop", Diamond, c, (1.0, 0.5), (0.5, 1.0), (100.0, 100.0), (2, 2), 1.0),
    ];
    for (why, kind, from, to, width, (w, h), (x, y), t) in rows {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, w, h), vec![black_to_white(kind, from, to, width)]);
        let got = px(&render(&d), x, y);
        assert!(near(got, grey(t), 4), "{why}: {got:?} want {:?}", grey(t));
    }
}

#[test]
fn gradient_stops_rules() {
    let stop = |position: f32, c: &str| ColorStop { position, color: hex(c) };
    let lin = |stops: Vec<ColorStop>| Paint::gradient(Gradient { stops, ..Gradient::default_for(GradientKind::Linear, Color::BLACK) });
    let horizontal = |mut p: Paint| {
        if let PaintKind::Gradient(g) = &mut p.kind {
            (g.from, g.to, g.width) = ((0.0, 0.5), (1.0, 0.5), (0.0, 1.0));
        }
        p
    };
    // (why, paint on a 100 × 100 box, pixel, colour there)
    let rows: Vec<(&str, Paint, Px, [u8; 3])> = vec![
        (
            "stops are used in position order, whatever order they're listed in",
            horizontal(lin(vec![stop(1.0, "0000FF"), stop(0.0, "FF0000")])),
            (0, 50),
            [254, 0, 1],
        ),
        (
            "colours mix in the stored (gamma-encoded) RGB values; silent on the space, pinned",
            horizontal(lin(vec![stop(0.0, "FF0000"), stop(1.0, "0000FF")])),
            (49, 50),
            [128, 0, 127],
        ),
        (
            "two stops at one position make a hard edge (before)",
            horizontal(lin(vec![stop(0.0, "FF0000"), stop(0.5, "FF0000"), stop(0.5, "0000FF"), stop(1.0, "0000FF")])),
            (45, 50),
            [255, 0, 0],
        ),
        (
            "two stops at one position make a hard edge (after)",
            horizontal(lin(vec![stop(0.0, "FF0000"), stop(0.5, "FF0000"), stop(0.5, "0000FF"), stop(1.0, "0000FF")])),
            (55, 50),
            [0, 0, 255],
        ),
        (
            "a paint's opacity fades every stop",
            Paint { opacity: 0.5, ..horizontal(lin(vec![stop(0.0, "000000"), stop(1.0, "000000")])) },
            (50, 50),
            [128, 128, 128],
        ),
        ("a stop's own alpha fades the colour there", horizontal(lin(vec![stop(0.0, "FF000080"), stop(1.0, "FF000080")])), (50, 50), [255, 127, 127]),
    ];
    for (why, paint, (x, y), want) in rows {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, 100.0, 100.0), vec![paint]);
        let got = px(&render(&d), x, y);
        assert!(near(got, want, 3), "{why}: {got:?} want {want:?}");
    }
}

#[test]
fn adding_a_stop_takes_the_colour_there_so_the_gradient_looks_the_same() {
    let g = Gradient {
        stops: vec![ColorStop { position: 0.0, color: hex("FF0000") }, ColorStop { position: 1.0, color: hex("0000FF") }],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 1.0),
        kind: GradientKind::Linear,
    };
    let mut more = g.clone();
    more.stops.push(ColorStop { position: 0.3, color: g.sample(0.3) });
    let draw = |g: Gradient| {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, 100.0, 100.0), vec![Paint::gradient(g)]);
        render(&d)
    };
    let (a, b) = (draw(g), draw(more));
    for x in (0..100).step_by(7) {
        assert!(near(px(&a, x, 50), px(&b, x, 50), 2), "x {x}");
    }
}

#[test]
fn gradients_stretch_with_the_layer() {
    // The handles are in the layer's unit box: a wider layer stretches the gradient with it.
    let paint = black_to_white(GradientKind::Linear, (0.0, 0.5), (1.0, 0.5), (0.0, 1.0));
    for (w, x) in [(50.0, 12), (100.0, 25), (200.0, 50)] {
        let (mut d, p) = page();
        rect(&mut d, p, (0.0, 0.0, w, 50.0), vec![paint.clone()]);
        let t = (f64::from(x) + 0.5) / w;
        assert!(near(px(&render(&d), x, 20), grey(t), 3), "width {w}");
    }
}

// ---- images ----

/// A 20 × 10 image, red on the left half and blue on the right, as `name` in a fresh document.
fn image_doc(name: &str) -> (Document, NodeId) {
    let mut img = Pixmap::new(20, 10).unwrap();
    for (i, c) in img.data_mut().chunks_exact_mut(4).enumerate() {
        c.copy_from_slice(if i % 20 < 10 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
    }
    let (mut d, p) = page();
    d.images.insert(name.into(), ImageData { bytes: std::sync::Arc::new(encode_png(&img).unwrap()), width: 20, height: 10 });
    (d, p)
}

#[test]
fn image_scale_modes_place_the_image() {
    use ImageScaleMode::*;
    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const PAGE: [u8; 3] = [255, 255, 255];
    let crop = |x0, x1| Some(figcraft_color::ImagePaint::crop_from_rect(x0, 0.0, x1, 1.0));
    // (why, mode, rotation, scale, crop, box, probes). The 20 × 10 image on a box at (0, 0).
    type Row<'a> = (&'a str, ImageScaleMode, i32, f64, Option<[f64; 6]>, Pt, Probes);
    let rows: Vec<Row> = vec![
        (
            "fill covers the box to its edges, centred, cropping the overflow (×10: 200 × 100, 50 cut each side)",
            Fill,
            0,
            1.0,
            None,
            (100.0, 100.0),
            vec![((25, 50), RED), ((75, 50), BLUE), ((10, 0), RED), ((0, 50), RED), ((99, 99), BLUE)],
        ),
        (
            "fit shows the whole image, centred, with empty bands (×5: 100 × 50 at y 25)",
            Fit,
            0,
            1.0,
            None,
            (100.0, 100.0),
            vec![((50, 10), PAGE), ((25, 50), RED), ((75, 50), BLUE), ((50, 90), PAGE)],
        ),
        (
            "tile repeats the image at its own size from the top-left",
            Tile,
            0,
            1.0,
            None,
            (40.0, 10.0),
            vec![((5, 5), RED), ((15, 5), BLUE), ((25, 5), RED), ((35, 5), BLUE)],
        ),
        (
            "tile scale sizes the tiles (50%: 10 × 5)",
            Tile,
            0,
            0.5,
            None,
            (40.0, 10.0),
            vec![((2, 2), RED), ((7, 2), BLUE), ((12, 7), RED), ((17, 7), BLUE)],
        ),
        (
            "crop shows the part of the image framed by the crop (its left half)",
            Crop,
            0,
            1.0,
            crop(0.0, 2.0),
            (100.0, 100.0),
            vec![((10, 50), RED), ((90, 50), RED)],
        ),
        (
            "crop shows the part of the image framed by the crop (its right half)",
            Crop,
            0,
            1.0,
            crop(-1.0, 1.0),
            (100.0, 100.0),
            vec![((10, 50), BLUE), ((90, 50), BLUE)],
        ),
        (
            "a quarter turn turns the image clockwise before it is fitted (silent on the direction, pinned)",
            Fill,
            1,
            1.0,
            None,
            (100.0, 100.0),
            vec![((50, 25), RED), ((50, 75), BLUE)],
        ),
        ("half a turn swaps the halves", Fill, 2, 1.0, None, (100.0, 100.0), vec![((25, 50), BLUE), ((75, 50), RED)]),
    ];
    for (i, (why, mode, rotation, scale, crop, (w, h), probes)) in rows.into_iter().enumerate() {
        let (mut d, p) = image_doc("pb-image");
        let mut paint = Paint::image("pb-image", mode);
        if let PaintKind::Image(img) = &mut paint.kind {
            (img.rotation, img.scale, img.crop) = (rotation, scale, crop);
        }
        rect(&mut d, p, (0.0, 0.0, w, h), vec![paint]);
        let pix = render(&d);
        for ((x, y), want) in probes {
            let got = px(&pix, x, y);
            assert!(near(got, want, 6), "row {i} {why} at ({x}, {y}): {got:?} want {want:?}");
        }
    }
}

#[test]
fn video_render_uses_explicit_frame_and_keeps_poster_fallback() {
    let (mut d, p) = image_doc("video-poster");
    let node = rect(&mut d, p, (0.0, 0.0, 100.0, 50.0), vec![Paint::video("video-source", "video-poster", ImageScaleMode::Fill)]);
    let poster = render_node_at(&d, node, 1.0, 0.0).unwrap();
    assert_eq!(px(&poster, 10, 10), [255, 0, 0]);
    let mut frame = Pixmap::new(1, 1).unwrap();
    frame.fill(tiny_skia::Color::from_rgba8(0, 255, 0, 255));
    let mut frames = VideoFrames::new();
    let instance = crate::VideoInstanceId {
        scope: crate::VideoScopeId(1),
        occurrence: crate::VideoOccurrenceId {
            paint: crate::PaintId { node, domain: crate::PaintDomain::Fill, index: 0 },
            patterns: crate::VideoPatternPath::default(),
        },
    };
    frames.insert(instance, std::sync::Arc::new(frame));
    let moving = render_node_with_video_frames(&d, node, 1.0, 0.0, instance.scope, &frames).unwrap();
    assert_eq!(px(&moving, 10, 10), [0, 255, 0]);
}

#[test]
fn video_frames_are_selected_by_node_paint_and_scope() {
    let (mut d, p) = image_doc("video-instance-poster");
    let first = rect(&mut d, p, (0.0, 0.0, 40.0, 30.0), vec![Paint::video("shared-video", "video-instance-poster", ImageScaleMode::Fill)]);
    let second = rect(&mut d, p, (50.0, 0.0, 40.0, 30.0), vec![Paint::video("shared-video", "video-instance-poster", ImageScaleMode::Fill)]);
    let multi = rect(
        &mut d,
        p,
        (100.0, 0.0, 40.0, 30.0),
        vec![
            Paint::video("shared-video", "video-instance-poster", ImageScaleMode::Fill),
            Paint::video("shared-video", "video-instance-poster", ImageScaleMode::Fill),
        ],
    );
    let scope_a = crate::VideoScopeId(11);
    let scope_b = crate::VideoScopeId(12);
    let id = |scope, node, index| crate::VideoInstanceId {
        scope,
        occurrence: crate::VideoOccurrenceId {
            paint: crate::PaintId { node, domain: crate::PaintDomain::Fill, index },
            patterns: crate::VideoPatternPath::default(),
        },
    };
    let red = || {
        let mut frame = Pixmap::new(1, 1).unwrap();
        frame.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        std::sync::Arc::new(frame)
    };
    let blue = || {
        let mut frame = Pixmap::new(1, 1).unwrap();
        frame.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        std::sync::Arc::new(frame)
    };
    let mut frames = VideoFrames::new();
    frames.insert(id(scope_a, first, 0), red());
    frames.insert(id(scope_a, second, 0), blue());
    frames.insert(id(scope_a, multi, 0), red());
    frames.insert(id(scope_a, multi, 1), blue());
    frames.insert(id(scope_b, first, 0), blue());

    let first_a = render_node_with_video_frames(&d, first, 1.0, 0.0, scope_a, &frames).unwrap();
    let second_a = render_node_with_video_frames(&d, second, 1.0, 0.0, scope_a, &frames).unwrap();
    let first_b = render_node_with_video_frames(&d, first, 1.0, 0.0, scope_b, &frames).unwrap();
    assert_eq!(px(&first_a, 10, 10), [255, 0, 0]);
    assert_eq!(px(&second_a, 10, 10), [0, 0, 255]);
    assert_eq!(px(&first_b, 10, 10), [0, 0, 255]);

    d.get_mut(multi).unwrap().fills[1].visible = false;
    let bottom = render_node_with_video_frames(&d, multi, 1.0, 0.0, scope_a, &frames).unwrap();
    assert_eq!(px(&bottom, 10, 10), [255, 0, 0]);
    d.get_mut(multi).unwrap().fills[1].visible = true;
    d.get_mut(multi).unwrap().fills[0].visible = false;
    let top = render_node_with_video_frames(&d, multi, 1.0, 0.0, scope_a, &frames).unwrap();
    assert_eq!(px(&top, 10, 10), [0, 0, 255]);
}

#[test]
fn video_scan_distinguishes_pattern_consumers_and_uses_text_style_batches() {
    let (mut d, p) = page();
    let host = rect(&mut d, p, (0.0, 0.0, 40.0, 40.0), vec![Paint::pattern(0), Paint::pattern(0)]);
    let source = rect(&mut d, p, (50.0, 0.0, 20.0, 20.0), vec![Paint::video("pattern-video", "missing-poster", ImageScaleMode::Fill)]);
    for paint in &mut d.get_mut(host).unwrap().fills {
        if let PaintKind::Pattern(pattern) = &mut paint.kind {
            pattern.source = source.0;
        }
    }
    let mut budget = crate::VideoScanBudget::default();
    let mut found = Vec::new();
    let report = crate::visit_video_occurrences(&d, p, &mut budget, |video| found.push(video.id));
    let consumer = |index| crate::PaintId { node: host, domain: crate::PaintDomain::Fill, index };
    let via_first =
        crate::VideoOccurrenceId { paint: consumer_video(source), patterns: crate::VideoPatternPath::default().pushed(consumer(0)).unwrap() };
    let via_second =
        crate::VideoOccurrenceId { paint: consumer_video(source), patterns: crate::VideoPatternPath::default().pushed(consumer(1)).unwrap() };
    let direct = crate::VideoOccurrenceId { paint: consumer_video(source), patterns: crate::VideoPatternPath::default() };
    assert!(found.contains(&direct));
    assert!(found.contains(&via_first));
    assert!(found.contains(&via_second));
    assert_eq!(report.active_occurrences, 3);
    assert!(!report.truncated);

    let pattern_scope = crate::VideoScopeId(13);
    let mut pattern_frames = VideoFrames::new();
    pattern_frames.insert(crate::VideoInstanceId { scope: pattern_scope, occurrence: via_first }, {
        let mut frame = Pixmap::new(1, 1).unwrap();
        frame.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        std::sync::Arc::new(frame)
    });
    pattern_frames.insert(crate::VideoInstanceId { scope: pattern_scope, occurrence: via_second }, {
        let mut frame = Pixmap::new(1, 1).unwrap();
        frame.fill(tiny_skia::Color::from_rgba8(0, 0, 255, 255));
        std::sync::Arc::new(frame)
    });
    d.get_mut(host).unwrap().fills[1].visible = false;
    let pattern_first = render_node_with_video_frames(&d, host, 1.0, 0.0, pattern_scope, &pattern_frames).unwrap();
    assert_eq!(px(&pattern_first, 10, 10), [255, 0, 0]);
    d.get_mut(host).unwrap().fills[1].visible = true;
    d.get_mut(host).unwrap().fills[0].visible = false;
    let pattern_second = render_node_with_video_frames(&d, host, 1.0, 0.0, pattern_scope, &pattern_frames).unwrap();
    assert_eq!(px(&pattern_second, 10, 10), [0, 0, 255]);

    let text = d.create(p, NodeKind::Text(figcraft_doc::TextData::new("A")), "Text", 0.0, 50.0, 40.0, 30.0).unwrap();
    d.get_mut(text).unwrap().fills = vec![Paint::video("fallback-video", "missing-poster", ImageScaleMode::Fill)];
    if let NodeKind::Text(data) = &mut d.get_mut(text).unwrap().kind {
        data.style.fills = Some(vec![Paint::video("styled-video", "missing-poster", ImageScaleMode::Fill)]);
    }
    let Some((layout, _)) = figcraft_text::layout_node(d.node(text).unwrap()) else { return };
    if layout.glyphs.is_empty() && layout.decorations.is_empty() {
        return;
    }
    let used_style =
        layout.glyphs.first().map(|glyph| glyph.style).or_else(|| layout.decorations.first().map(|decoration| decoration.style)).unwrap();
    let mut text_budget = crate::VideoScanBudget::default();
    let mut text_ids = Vec::new();
    crate::visit_video_occurrences(&d, text, &mut text_budget, |video| text_ids.push(video.id));
    assert!(text_ids.contains(&crate::VideoOccurrenceId {
        paint: crate::PaintId { node: text, domain: crate::PaintDomain::TextStyle(used_style), index: 0 },
        patterns: crate::VideoPatternPath::default(),
    }));
    assert!(!text_ids.iter().any(|id| id.paint.node == text && id.paint.domain == crate::PaintDomain::Fill));
    let text_occurrence = crate::VideoOccurrenceId {
        paint: crate::PaintId { node: text, domain: crate::PaintDomain::TextStyle(used_style), index: 0 },
        patterns: crate::VideoPatternPath::default(),
    };
    let red_scope = crate::VideoScopeId(14);
    let blue_scope = crate::VideoScopeId(15);
    let mut text_frames = VideoFrames::new();
    for (scope, color) in [(red_scope, tiny_skia::Color::from_rgba8(255, 0, 0, 255)), (blue_scope, tiny_skia::Color::from_rgba8(0, 0, 255, 255))] {
        let mut frame = Pixmap::new(1, 1).unwrap();
        frame.fill(color);
        text_frames.insert(crate::VideoInstanceId { scope, occurrence: text_occurrence }, std::sync::Arc::new(frame));
    }
    let red_text = render_node_with_video_frames(&d, text, 1.0, 0.0, red_scope, &text_frames).unwrap();
    let blue_text = render_node_with_video_frames(&d, text, 1.0, 0.0, blue_scope, &text_frames).unwrap();
    let sample = |pix: &Pixmap| crate::to_rgba8(pix).as_chunks::<4>().0.iter().find(|pixel| pixel[3] > 0).copied().unwrap();
    assert_eq!(&sample(&red_text)[..3], [255, 0, 0]);
    assert_eq!(&sample(&blue_text)[..3], [0, 0, 255]);

    fn consumer_video(node: NodeId) -> crate::PaintId {
        crate::PaintId { node, domain: crate::PaintDomain::Fill, index: 0 }
    }
}

#[test]
fn video_scan_skips_nonrendered_container_contents_but_keeps_own_boolean_fills() {
    let (mut d, p) = page();
    let section = d.create(p, NodeKind::Section { contents_hidden: true }, "collapsed", 0.0, 0.0, 30.0, 30.0).unwrap();
    let section_video = rect(&mut d, section, (0.0, 0.0, 10.0, 10.0), vec![Paint::video("hidden-section", "poster", ImageScaleMode::Fill)]);

    let boolean = d.create(p, NodeKind::BooleanOp { op: figcraft_doc::node::BoolOp::Union }, "boolean", 40.0, 0.0, 30.0, 30.0).unwrap();
    let boolean_child = rect(&mut d, boolean, (0.0, 0.0, 10.0, 10.0), vec![Paint::video("hidden-operand", "poster", ImageScaleMode::Fill)]);
    d.get_mut(boolean).unwrap().fills.push(Paint::video("boolean-own-fill", "poster", ImageScaleMode::Fill));

    let mut budget = crate::VideoScanBudget::default();
    let mut found = Vec::new();
    crate::visit_video_occurrences(&d, p, &mut budget, |video| found.push(video.id.paint.node));
    assert!(!found.contains(&section_video), "collapsed section descendants do not render");
    assert!(!found.contains(&boolean_child), "boolean operands do not render as child layers");
    assert!(found.contains(&boolean), "the boolean's own fills still render");
}

#[test]
fn image_opacity_and_adjustments() {
    let (mut d, p) = image_doc("pb-opacity");
    rect(&mut d, p, (0.0, 0.0, 100.0, 100.0), vec![Paint { opacity: 0.5, ..Paint::image("pb-opacity", ImageScaleMode::Fill) }]);
    assert!(near(px(&render(&d), 25, 50), [255, 128, 128], 2), "an image paint's opacity lets the page through");
    // The adjustments' maths is not documented: only their direction is checked.
    let adjust = |c: &str, f: ImageFilters| {
        let mut p = Pixmap::new(1, 1).unwrap();
        p.fill(paint::ts_color(hex(c)));
        images::apply_filters(&mut p, &f);
        [p.data()[0], p.data()[1], p.data()[2]]
    };
    let none = ImageFilters::NONE;
    type Check = fn([u8; 3], [u8; 3]) -> bool;
    let rows: Vec<(&str, &str, ImageFilters, Check)> = vec![
        ("no adjustment leaves the pixels alone", "CC6633", none, |a, b| a == b),
        ("exposure up brightens", "808080", ImageFilters { exposure: 0.5, ..none }, |a, b| a[0] > b[0]),
        ("exposure down darkens", "808080", ImageFilters { exposure: -0.5, ..none }, |a, b| a[0] < b[0]),
        ("contrast up darkens the darks", "404040", ImageFilters { contrast: 0.5, ..none }, |a, b| a[0] < b[0]),
        ("contrast up brightens the lights", "C0C0C0", ImageFilters { contrast: 0.5, ..none }, |a, b| a[0] > b[0]),
        ("saturation -100% is greyscale", "CC6633", ImageFilters { saturation: -1.0, ..none }, |a, _| {
            a[0].abs_diff(a[1]) <= 1 && a[1].abs_diff(a[2]) <= 1
        }),
        ("warmer: more red, less blue", "808080", ImageFilters { temperature: 1.0, ..none }, |a, b| a[0] > b[0] && a[2] < b[2]),
        ("cooler: less red, more blue", "808080", ImageFilters { temperature: -1.0, ..none }, |a, b| a[0] < b[0] && a[2] > b[2]),
        ("highlights up brighten the lights", "D0D0D0", ImageFilters { highlights: 1.0, ..none }, |a, b| a[0] > b[0]),
        ("shadows up brighten the darks", "303030", ImageFilters { shadows: 1.0, ..none }, |a, b| a[0] > b[0]),
    ];
    for (why, c, f, ok) in rows {
        let (got, was) = (adjust(c, f), rgb(c));
        assert!(ok(got, was), "{why}: {was:?} -> {got:?}");
    }
}

// ---- colour models ----

#[test]
fn colour_models_agree() {
    // (hex, HSB as shown: degrees and percent, HSL as shown)
    let rows = [
        ("FF0000", (0, 100, 100), (0, 100, 50)),
        ("00FF00", (120, 100, 100), (120, 100, 50)),
        ("0000FF", (240, 100, 100), (240, 100, 50)),
        ("808080", (0, 0, 50), (0, 0, 50)),
        ("FFFFFF", (0, 0, 100), (0, 0, 100)),
        ("000000", (0, 0, 0), (0, 0, 0)),
        ("0D99FF", (205, 95, 100), (205, 100, 53)),
        ("CC6633", (20, 75, 80), (20, 60, 50)),
    ];
    let pct = |v: f32| (v * 100.0).round() as i32;
    for (h, hsb, hsl) in rows {
        let c = hex(h);
        let b = Hsb::from_color(c);
        let l = Hsl::from_color(c);
        assert_eq!((b.h.round() as i32, pct(b.s), pct(b.b)), hsb, "{h} as HSB");
        assert_eq!((l.h.round() as i32, pct(l.s), pct(l.l)), hsl, "{h} as HSL");
        assert_eq!(b.to_color().to_hex(), h, "{h} back from HSB");
        assert_eq!(l.to_color().to_hex(), h, "{h} back from HSL");
    }
    // Every colour on a 4-bit grid survives a trip through both models.
    for i in 0..4096u32 {
        let c = Color::from_rgba8(((i >> 8) * 17) as u8, (((i >> 4) & 15) * 17) as u8, ((i & 15) * 17) as u8, 255);
        assert_eq!(Hsb::from_color(c).to_color().to_hex(), c.to_hex());
        assert_eq!(Hsl::from_color(c).to_color().to_hex(), c.to_hex());
    }
}

#[test]
fn hex_entry_and_alpha_rounding() {
    // (typed, hex shown, opacity shown in percent)
    let rows = [
        ("F", Some(("FFFFFF", 100))),
        ("AB", Some(("ABABAB", 100))),
        ("ABC", Some(("AABBCC", 100))),
        ("#0d99ff", Some(("0D99FF", 100))),
        ("0D99FF80", Some(("0D99FF", 50))),
        ("0D99FF7F", Some(("0D99FF", 50))),
        ("0D99FF00", Some(("0D99FF", 0))),
        ("0D99FF03", Some(("0D99FF", 1))),
        ("GG", None),
        ("12345", None),
    ];
    for (typed, want) in rows {
        let got = Color::from_hex(typed).map(|c| (c.to_hex(), (c.a * 100.0).round() as i32));
        assert_eq!(got, want.map(|(h, a)| (h.to_string(), a)), "{typed}");
    }
    // A fill typed with alpha keeps the colour opaque and moves the alpha to the paint's opacity.
    let p = Paint::solid(hex("FF000080"));
    assert_eq!((p.solid_color().map(|c| c.to_hex()), (p.opacity * 100.0).round()), (Some("FF0000".into()), 50.0));
}

// ---- colour profile ----

#[test]
fn changing_the_colour_profile_converts_or_keeps_values() {
    let draw = |d: &Document| {
        let mut pix = render(d);
        to_srgb(&mut pix, d.color_profile);
        px(&pix, 50, 50)
    };
    let (mut d, p) = page();
    rect(&mut d, p, (20.0, 20.0, 60.0, 60.0), vec![Paint::solid(hex("CC6633"))]);
    let before = draw(&d);
    // Converting keeps the look: the values change so the colour seen stays.
    let mut conv = d.clone();
    conv.set_color_profile(ColorProfile::DisplayP3, true);
    assert!(near(draw(&conv), before, 2), "converted: {:?} vs {before:?}", draw(&conv));
    // Keeping the values reinterprets them in the wider space: the colour seen changes.
    let mut kept = d.clone();
    kept.set_color_profile(ColorProfile::DisplayP3, false);
    let r = kept.node(kept.node(p).unwrap().children[0]).unwrap();
    assert_eq!(r.fills[0].solid_color().map(|c| c.to_hex()), Some("CC6633".into()));
    let seen = draw(&kept);
    assert!(seen[0] > before[0] && seen[2] < before[2], "kept values look more saturated: {seen:?} vs {before:?}");
}
