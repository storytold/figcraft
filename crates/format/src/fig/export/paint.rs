use super::common::one_sided;
use super::{
    Affine, Bound, EffectKind, Exporter, Fields, GradientKind, ImageScaleMode, Kv, MAX_IMAGE, Node, NodeKind, NoiseMode, Paint, PaintKind,
    StrokeAlign, StrokeCap, arr, b, blend_name, cap_name, color, e, fl, join_name, matrix, o, s, sha1_hex, xy,
};

impl<'d> Exporter<'d> {
    /// A paint list: a layer's fills (`fills`) or strokes with the variables `bound` to them.
    pub(super) fn paints(&mut self, list: &[Paint], fills: bool, bound: &Bound) -> Kv<'static> {
        arr(list.iter().enumerate().filter_map(|(i, p)| self.paint(p, (fills, i), bound)).collect())
    }

    fn paint(&mut self, p: &Paint, at: (bool, usize), bound: &Bound) -> Option<Kv<'static>> {
        let mut f: Fields = vec![("visible", b(p.visible)), ("blendMode", e(blend_name(p.blend)))];
        let mut opacity = f64::from(p.opacity);
        match &p.kind {
            PaintKind::Solid { color: c } => {
                opacity *= f64::from(c.a);
                f.extend([("type", e("SOLID")), ("color", color(c.with_alpha(1.0)))]);
                f.extend(bound.paints.get(&at).map(|a| ("colorVar", a.clone())));
            }
            PaintKind::Gradient(g) => {
                let ty = match g.kind {
                    GradientKind::Linear => "GRADIENT_LINEAR",
                    GradientKind::Radial => "GRADIENT_RADIAL",
                    GradientKind::Angular => "GRADIENT_ANGULAR",
                    GradientKind::Diamond => "GRADIENT_DIAMOND",
                };
                // The handles in the unit box come from the inverse of the paint transform: it maps
                // (0, ½) → start, (1, ½) → end, (0, 1) → width for linear gradients, and the centre
                // (½, ½) and the points half a box away from it for the others.
                let (from, to, width) = (g.from, g.to, g.width);
                let (ax, ay) =
                    if g.kind == GradientKind::Linear { (to.0 - from.0, to.1 - from.1) } else { (2.0 * (to.0 - from.0), 2.0 * (to.1 - from.1)) };
                let (cx, cy) = (2.0 * (width.0 - from.0), 2.0 * (width.1 - from.1));
                let (ex, ey) = if g.kind == GradientKind::Linear {
                    (from.0 - 0.5 * cx, from.1 - 0.5 * cy)
                } else {
                    (from.0 - 0.5 * (ax + cx), from.1 - 0.5 * (ay + cy))
                };
                let handles = Affine::new([ax, ay, cx, cy, ex, ey]);
                let t = if handles.determinant().abs() > 1e-12 && handles.as_coeffs().iter().all(|x| x.is_finite()) {
                    handles.inverse()
                } else {
                    self.warn("gradients with handles on one line were written with the default direction");
                    Affine::IDENTITY
                };
                let stops = g.stops.iter().map(|st| o(vec![("color", color(st.color)), ("position", fl(st.position.into()))])).collect();
                f.extend([("type", e(ty)), ("stops", arr(stops)), ("transform", matrix(t))]);
                // Stops bound to variables: every stop again, the bound ones with their variable.
                if bound.stops.range((at.0, at.1, 0)..=(at.0, at.1, usize::MAX)).next().is_some() {
                    let vars = g.stops.iter().enumerate().map(|(j, st)| {
                        let mut sf: Fields = vec![("color", color(st.color))];
                        sf.extend(bound.stops.get(&(at.0, at.1, j)).map(|a| ("colorVar", a.clone())));
                        sf.push(("position", fl(st.position.into())));
                        o(sf)
                    });
                    f.push(("stopsVar", arr(vars.collect())));
                }
            }
            PaintKind::Image(img) => {
                let hash = self.image(&img.image);
                let mode = match img.scale_mode {
                    ImageScaleMode::Fill => "FILL",
                    ImageScaleMode::Fit => "FIT",
                    ImageScaleMode::Crop => "STRETCH",
                    ImageScaleMode::Tile => "TILE",
                };
                f.extend([
                    ("type", e("IMAGE")),
                    ("image", o(vec![("hash", Kv::Bytes(hash)), ("name", s(""))])),
                    ("imageScaleMode", e(mode)),
                    ("rotation", fl(f64::from(img.rotation.rem_euclid(4)) * 90.0)),
                    ("scale", fl(img.scale)),
                ]);
                if let (ImageScaleMode::Crop, Some(c)) = (img.scale_mode, img.crop) {
                    f.push(("transform", matrix(Affine::new(c))));
                }
                if img.filters != Default::default() {
                    self.warn("image adjustments (exposure, contrast, …) were left out");
                }
            }
            PaintKind::Pattern(_) => {
                self.warn("pattern fills were left out");
                return None;
            }
        }
        f.push(("opacity", fl(opacity)));
        Some(o(f))
    }

    /// The hash bytes of image `key`, writing its file once (empty when its bytes aren't there).
    fn image(&mut self, key: &str) -> Vec<u8> {
        let hex = match self.images.get(key) {
            Some(h) => h.clone(),
            None => {
                let h = match self.doc.images.get(key) {
                    Some(img) if img.bytes.len() as u64 <= MAX_IMAGE => {
                        let h = sha1_hex(&img.bytes);
                        if !self.files.iter().any(|(x, _)| *x == h) {
                            self.files.push((h.clone(), img.bytes.clone()));
                        }
                        Some(h)
                    }
                    Some(_) => {
                        self.warn("images larger than 256 MB were left out");
                        None
                    }
                    None => {
                        self.warn("image fills whose image isn't in the document were written without one");
                        None
                    }
                };
                self.images.insert(key.to_string(), h.clone());
                h
            }
        };
        let hex = hex.unwrap_or_default();
        (0..hex.len() / 2).filter_map(|i| hex.get(2 * i..2 * i + 2).and_then(|d| u8::from_str_radix(d, 16).ok())).collect()
    }

    pub(super) fn stroke(&mut self, n: &Node, f: &mut Fields) {
        let sp = &n.stroke;
        let align = match sp.align {
            StrokeAlign::Inside => "INSIDE",
            StrokeAlign::Center => "CENTER",
            StrokeAlign::Outside => "OUTSIDE",
        };
        f.extend([
            ("strokeWeight", fl(sp.weight)),
            ("strokeAlign", e(align)),
            ("strokeJoin", e(join_name(sp.join))),
            ("strokeCap", e(cap_name(sp.cap_start))),
            ("miterLimit", fl(sp.miter_limit)),
        ]);
        if !sp.dashes.is_empty() {
            f.push(("dashPattern", arr(sp.dashes.iter().map(|d| fl(*d)).collect())));
        }
        if let Some([t, r, bo, l]) = sp.sides {
            f.extend([
                ("borderStrokeWeightsIndependent", b(true)),
                ("borderTopWeight", fl(t)),
                ("borderRightWeight", fl(r)),
                ("borderBottomWeight", fl(bo)),
                ("borderLeftWeight", fl(l)),
            ]);
        }
        // A variable width: how far the stroke reaches on each side of the path at positions along
        // its length, as a share of the stroke weight (half of it each for the full weight). The
        // two sides are the same: a reader draws nothing for a point with unequal sides.
        if let Some(p) = sp.profile().filter(|_| sp.weight.is_finite() && sp.weight > 0.0) {
            let point = |q: &figcraft_doc::WidthPoint| o(vec![("position", fl(q.t)), ("ascent", fl(q.scale / 2.0)), ("descent", fl(q.scale / 2.0))]);
            let p = p.sanitized();
            if one_sided(&p) {
                self.warn("strokes wider on one side of their path were written with that width on both sides");
            }
            f.push(("variableWidthPoints", arr(p.points.iter().map(point).collect())));
        }
        if sp.brush.is_some() {
            self.warn("brush strokes were written as plain strokes");
        }
        if [sp.cap_start, sp.cap_end].contains(&StrokeCap::ReversedTriangle) {
            self.warn("reversed triangle stroke ends were written as triangle arrows");
        }
        // Vectors keep each end's cap on their end points; other layers have one cap.
        if sp.cap_start != sp.cap_end && !matches!(n.kind, NodeKind::Vector { .. }) {
            self.warn("lines with different stroke ends use the start's at both ends");
        }
    }

    /// Effect `index` of a layer, with the variables `bound` to its fields.
    pub(super) fn effect(&mut self, x: &figcraft_doc::Effect, index: usize, bound: &Bound) -> Kv<'static> {
        let mut f: Fields = vec![("visible", b(x.visible))];
        let vars = [
            ("color", "colorVar"),
            ("radius", "radiusVar"),
            ("spread", "spreadVar"),
            ("offsetX", "xVar"),
            ("offsetY", "yVar"),
            ("startRadius", "startRadiusVar"),
        ];
        f.extend(vars.into_iter().filter_map(|(ours, key)| Some((key, bound.effects.get(&(index, ours))?.clone()))));
        let blur = |f: &mut Fields, ty: &'static str, bl: &figcraft_doc::Blur| {
            f.extend([("type", e(ty)), ("radius", fl(bl.radius))]);
            if bl.blur_type == figcraft_doc::BlurType::Progressive {
                f.extend([
                    ("blurOpType", e("PROGRESSIVE")),
                    ("startRadius", fl(bl.start_radius)),
                    ("startOffset", xy(bl.start.0, bl.start.1)),
                    ("endOffset", xy(bl.end.0, bl.end.1)),
                ]);
            }
        };
        let grain = |size: f64, size_y: Option<f64>| xy(size, size_y.unwrap_or(size));
        match &x.kind {
            EffectKind::DropShadow(sh) | EffectKind::InnerShadow(sh) => {
                let ty = if matches!(x.kind, EffectKind::DropShadow(_)) { "DROP_SHADOW" } else { "INNER_SHADOW" };
                f.extend([
                    ("type", e(ty)),
                    ("color", color(sh.color)),
                    ("offset", xy(sh.offset.0, sh.offset.1)),
                    ("radius", fl(sh.radius)),
                    ("spread", fl(sh.spread)),
                    ("blendMode", e(blend_name(sh.blend))),
                    ("showShadowBehindNode", b(sh.show_behind)),
                ]);
            }
            EffectKind::LayerBlur(bl) => blur(&mut f, "FOREGROUND_BLUR", bl),
            EffectKind::BackgroundBlur(bl) => blur(&mut f, "BACKGROUND_BLUR", bl),
            EffectKind::Noise(nz) => {
                let mode = match nz.mode {
                    NoiseMode::Mono => "MONOTONE",
                    NoiseMode::Duo => "DUOTONE",
                    NoiseMode::Multi => "MULTITONE",
                };
                f.extend([
                    ("type", e("NOISE")),
                    ("noiseType", e(mode)),
                    ("noiseSize", grain(nz.size, nz.size_y)),
                    ("density", fl(nz.density)),
                    ("color", color(nz.color)),
                    ("secondaryColor", color(nz.secondary)),
                    ("opacity", fl(nz.opacity.into())),
                    ("blendMode", e(blend_name(nz.blend))),
                ]);
            }
            EffectKind::Texture(t) => {
                f.extend([
                    ("type", e("GRAIN")),
                    ("noiseSize", grain(t.size, t.size_y)),
                    ("radius", fl(t.radius)),
                    ("clipToShape", b(t.clip_to_shape)),
                ]);
            }
            EffectKind::Glass(g) => {
                self.warn("glass effects keep only their frost");
                f.extend([("type", e("GLASS")), ("radius", fl(g.frost))]);
            }
        }
        o(f)
    }
}
