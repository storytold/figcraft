//! SVG filters → layer effects. Each filter's primitives are evaluated symbolically: what each
//! result *is* (the layer, the layer blurred, a shadow made from the layer's alpha, a flood, a
//! stack of those) rather than its pixels. A filter whose result is the layer with shadows under
//! it, shadows cut into it, or blurred, maps exactly onto drop shadows, inner shadows and a layer
//! blur: `feDropShadow`; `feOffset` + `feGaussianBlur` + `feFlood` + `feComposite` + `feMerge`;
//! the colour-matrix form (alpha → offset → blur → matrix tint → blend), with `feMorphology` as
//! spread and arithmetic or `out` composites for inner shadows. A shadow made from the blurred
//! layer's tinted alpha, and cut out by that layer (what our exports write, as the canvas casts
//! drop shadows from what the layer blurs leave), is a drop shadow of that blurred layer. Anything
//! else is reported back as unsupported and the layer is drawn without it.

use std::collections::HashMap;

use figcraft_color::{BlendMode, Color};
use figcraft_doc::{Blur, Effect, EffectKind, Shadow};
use figcraft_geom::{Affine, Vec2};
use usvg::filter::{ColorMatrixKind, CompositeOperator, Input, Kind, MorphologyOperator};

use super::paint::color;
use super::shape::length_scale;

/// A shadow-like image: the layer's alpha, offset, spread, blurred and tinted.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sh {
    dx: f64,
    dy: f64,
    sigma: f64,
    spread: f64,
    color: Color,
    /// Cut out where the layer is (drop shadows that don't show behind it).
    knockout: bool,
    /// Inside the layer where the offset copy isn't: an inner shadow.
    inner: bool,
    /// Everything but the shadow (a flood with the shadow cut out): becomes an inner shadow once
    /// it's cut to the layer.
    inverted: bool,
    /// Cast from the layer blurred by this sigma rather than from its alpha (our exports: a drop
    /// shadow is cast from the layer its layer blurs leave).
    cast: f64,
}

impl Sh {
    fn alpha() -> Self {
        Sh { dx: 0.0, dy: 0.0, sigma: 0.0, spread: 0.0, color: Color::BLACK, knockout: false, inner: false, inverted: false, cast: 0.0 }
    }
    /// Still exactly the layer's alpha.
    fn is_plain(&self) -> bool {
        *self == Sh::alpha()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Img {
    /// The layer (blurred by sigma).
    Layer(f64),
    Shadow(Sh),
    Flood(Color),
    Clear,
    /// Bottom to top.
    Stack(Vec<Img>),
}

impl Img {
    fn into_layers(self) -> Vec<Img> {
        match self {
            Img::Stack(v) => v.into_iter().flat_map(Img::into_layers).collect(),
            Img::Clear => vec![],
            i => vec![i],
        }
    }
}

fn stack(bottom: Img, top: Img) -> Img {
    let mut v = bottom.into_layers();
    v.extend(top.into_layers());
    Img::Stack(v)
}

/// The effects `filters` amount to on a layer whose user space maps to the page by `world`; `None`
/// when one of them can't be expressed as effects.
pub(crate) fn effects(filters: &[std::sync::Arc<usvg::filter::Filter>], world: Affine) -> Option<Vec<Effect>> {
    let mut out = Vec::new();
    for f in filters {
        out.extend(filter(f, world)?);
    }
    Some(out)
}

fn filter(f: &usvg::filter::Filter, world: Affine) -> Option<Vec<Effect>> {
    let mut results: HashMap<&str, Img> = HashMap::new();
    let mut last = Img::Layer(0.0);
    for p in f.primitives().iter().take(64) {
        let get = |i: &Input| -> Option<Img> {
            match i {
                Input::SourceGraphic => Some(Img::Layer(0.0)),
                Input::SourceAlpha => Some(Img::Shadow(Sh::alpha())),
                Input::Reference(r) => results.get(r.as_str()).cloned(),
            }
        };
        let img = primitive(p.kind(), &get)?;
        results.insert(p.result(), img.clone());
        last = img;
    }
    to_effects(last, world)
}

fn primitive(k: &Kind, get: &dyn Fn(&Input) -> Option<Img>) -> Option<Img> {
    Some(match k {
        Kind::Offset(o) => match get(o.input())? {
            Img::Shadow(s) => Img::Shadow(Sh { dx: s.dx + f64::from(o.dx()), dy: s.dy + f64::from(o.dy()), ..s }),
            Img::Layer(b) if o.dx() == 0.0 && o.dy() == 0.0 => Img::Layer(b),
            Img::Clear => Img::Clear,
            _ => return None,
        },
        Kind::GaussianBlur(g) => {
            let sigma = (f64::from(g.std_dev_x().get()) + f64::from(g.std_dev_y().get())) / 2.0;
            match get(g.input())? {
                Img::Shadow(s) => Img::Shadow(Sh { sigma: s.sigma.hypot(sigma), ..s }),
                Img::Layer(b) => Img::Layer(b.hypot(sigma)),
                Img::Clear => Img::Clear,
                _ => return None,
            }
        }
        Kind::Flood(fl) => {
            let a = fl.opacity().get();
            if a <= 0.0 { Img::Clear } else { Img::Flood(color(fl.color(), a)) }
        }
        Kind::Morphology(m) => match get(m.input())? {
            Img::Shadow(s) if s.sigma == 0.0 && s.spread == 0.0 && !s.inner && !s.inverted => {
                let r = (f64::from(m.radius_x().get()) + f64::from(m.radius_y().get())) / 2.0;
                Img::Shadow(Sh { spread: if m.operator() == MorphologyOperator::Dilate { r } else { -r }, ..s })
            }
            _ => return None,
        },
        Kind::ColorMatrix(cm) => {
            let ColorMatrixKind::Matrix(v) = cm.kind() else { return None };
            let s = match get(cm.input())? {
                Img::Shadow(s) => s,
                Img::Layer(0.0) => Sh::alpha(),
                // A tint of the blurred layer: a shadow cast from it.
                Img::Layer(b) => Sh { cast: b, ..Sh::alpha() },
                _ => return None,
            };
            tint(v, s)?
        }
        Kind::Composite(c) => composite(get(c.input1())?, get(c.input2())?, c.operator())?,
        Kind::Merge(m) => {
            let mut v = Vec::new();
            for i in m.inputs() {
                v.extend(get(i)?.into_layers());
            }
            Img::Stack(v)
        }
        Kind::Blend(b) => {
            let (top, bottom) = (get(b.input1())?, get(b.input2())?);
            match b.mode() {
                usvg::BlendMode::Normal => stack(bottom, top),
                _ => return None,
            }
        }
        Kind::DropShadow(d) => {
            let sigma = (f64::from(d.std_dev_x().get()) + f64::from(d.std_dev_y().get())) / 2.0;
            let s = Sh { dx: f64::from(d.dx()), dy: f64::from(d.dy()), sigma, color: color(d.color(), d.opacity().get()), ..Sh::alpha() };
            match get(d.input())? {
                Img::Layer(b) => stack(Img::Shadow(s), Img::Layer(b)),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// A colour matrix applied to a shadow: rows that set a constant colour and scale alpha tint it;
/// the "hard alpha" row (alpha × 127, opaque where there's any alpha) keeps it as it is.
fn tint(v: &[f32], s: Sh) -> Option<Img> {
    let row = |r: usize| v.get(r * 5..r * 5 + 5);
    let (rr, gr, br, ar) = (row(0)?, row(1)?, row(2)?, row(3)?);
    let constant = |r: &[f32]| r.get(..4).is_some_and(|c| c.iter().all(|x| *x == 0.0));
    if !(constant(rr) && constant(gr) && constant(br)) || ar.get(..3).is_none_or(|c| c.iter().any(|x| *x != 0.0)) || ar.get(4) != Some(&0.0) {
        return None;
    }
    let a = f64::from(*ar.get(3)?);
    let k = |r: &[f32]| r.get(4).copied().unwrap_or(0.0).clamp(0.0, 1.0);
    if a >= 100.0 && [k(rr), k(gr), k(br)] == [0.0; 3] {
        return Some(Img::Shadow(s));
    }
    Some(Img::Shadow(Sh { color: Color::new(k(rr), k(gr), k(br), (f64::from(s.color.a) * a).clamp(0.0, 1.0) as f32), ..s }))
}

fn composite(a: Img, b: Img, op: CompositeOperator) -> Option<Img> {
    let tinted = |s: Sh, c: Color| Sh { color: Color::new(c.r, c.g, c.b, (c.a * s.color.a).clamp(0.0, 1.0)), ..s };
    Some(match (op, a, b) {
        (CompositeOperator::Over, a, b) => stack(b, a),
        (CompositeOperator::In, Img::Flood(c), Img::Shadow(s)) | (CompositeOperator::In, Img::Shadow(s), Img::Flood(c)) => Img::Shadow(tinted(s, c)),
        // A flood cut to everything but a shadow, then cut to the layer: an inner shadow.
        (CompositeOperator::Out, Img::Flood(c), Img::Shadow(s)) if !s.inner && !s.inverted => Img::Shadow(Sh { inverted: true, ..tinted(s, c) }),
        (CompositeOperator::In, Img::Shadow(s), Img::Shadow(l)) if s.inverted && l.is_plain() => {
            Img::Shadow(Sh { inverted: false, inner: true, ..s })
        }
        (CompositeOperator::Out, Img::Shadow(s), Img::Shadow(l)) if l.is_plain() && !s.inner && !s.inverted => {
            Img::Shadow(Sh { knockout: true, ..s })
        }
        // Cut out where the blurred layer it was cast from is.
        (CompositeOperator::Out, Img::Shadow(s), Img::Layer(b)) if b > 0.0 && b == s.cast && !s.inner && !s.inverted => {
            Img::Shadow(Sh { knockout: true, ..s })
        }
        // layer − shadow: inside the layer where the offset copy isn't.
        (CompositeOperator::Arithmetic { k1, k2, k3, k4 }, Img::Shadow(s), Img::Shadow(l))
            if k1 == 0.0 && k4 == 0.0 && k2 == -1.0 && k3 == 1.0 && l.is_plain() && !s.inner =>
        {
            Img::Shadow(Sh { inner: true, ..s })
        }
        _ => return None,
    })
}

/// The effects of a filter whose result is `img`.
fn to_effects(img: Img, world: Affine) -> Option<Vec<Effect>> {
    let layers = img.into_layers();
    let at = layers.iter().position(|l| matches!(l, Img::Layer(_)))?;
    if layers.iter().filter(|l| matches!(l, Img::Layer(_))).count() != 1 {
        return None;
    }
    let k = length_scale(world);
    // A shadow cast from a blurred layer is one of its drop shadows only when that is the layer drawn.
    let drawn = match layers.get(at) {
        Some(Img::Layer(b)) => *b,
        _ => 0.0,
    };
    if layers.iter().any(|l| matches!(l, Img::Shadow(s) if s.cast > 0.0 && (s.cast != drawn || s.inner || s.inverted))) {
        return None;
    }
    let mut out = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        match l {
            Img::Layer(b) if *b > 0.0 => out.push(EffectKind::LayerBlur(Blur { radius: 2.0 * b * k, ..Blur::default() })),
            Img::Layer(_) => {}
            Img::Shadow(s) if s.inverted => return None,
            Img::Shadow(s) => {
                let off = linear(world, Vec2::new(s.dx, s.dy));
                let shadow = Shadow {
                    color: s.color,
                    offset: (off.x, off.y),
                    radius: 2.0 * s.sigma * k,
                    spread: s.spread * k,
                    blend: BlendMode::Normal,
                    show_behind: !s.knockout,
                };
                match (s.inner, i < at) {
                    (false, true) => out.push(EffectKind::DropShadow(shadow)),
                    (true, false) => out.push(EffectKind::InnerShadow(Shadow { show_behind: false, ..shadow })),
                    _ => return None,
                }
            }
            Img::Flood(_) | Img::Clear | Img::Stack(_) => return None,
        }
    }
    if out.iter().any(|e| !effect_is_finite(e)) {
        return None;
    }
    Some(out.into_iter().map(|kind| Effect { kind, visible: true }).collect())
}

/// `v` through `world` without its translation.
pub(crate) fn linear(world: Affine, v: Vec2) -> Vec2 {
    let [a, b, c, d, ..] = world.as_coeffs();
    Vec2::new(a * v.x + c * v.y, b * v.x + d * v.y)
}

fn effect_is_finite(e: &EffectKind) -> bool {
    match e {
        EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => [s.offset.0, s.offset.1, s.radius, s.spread].iter().all(|v| v.is_finite()),
        EffectKind::LayerBlur(b) => b.radius.is_finite(),
        _ => true,
    }
}
