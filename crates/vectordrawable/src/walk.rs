//! The layer walk: what each layer paints, bottom to top, as paths and groups in viewport space.
//!
//! The order follows the renderer: a layer's fills, its children (clipped by a clipping frame,
//! masked runs clipped by their mask), then its strokes. A layer's opacity is folded into the
//! colours of what it paints; that is exact unless two of those shapes overlap (a Vector Drawable
//! group has no opacity of its own), which [`Compat`] decides.

use figcraft_color::{BlendMode, Paint, PaintKind};
use figcraft_doc::{Document, EffectKind, MaskType, Node, NodeId, NodeKind};
use figcraft_geom::{Affine, BezPath, Rect, Shape};
use figcraft_render::{FillRule, MAX_EXPORT_DEPTH};

use crate::model::{Fill, GroupItem, GroupXf, Item, ItemKind, NativeStroke, PathItem};
use crate::paint::{GradOut, gradient, nonzero};
use crate::warnings::{Warn, Warnings};
use crate::{Compat, VdOptions, stroke, text};

/// The largest coordinate written: past it a 32-bit float (what Android parses path data into)
/// can't place points to a fraction of a dp.
const MAX_COORD: f64 = 1e7;

pub(crate) struct Walker<'a> {
    doc: &'a Document,
    opts: &'a VdOptions,
    warn: Warnings,
    names: std::collections::BTreeSet<String>,
}

/// Every coordinate of `p` finite and within [`MAX_COORD`].
fn writable(p: &BezPath) -> bool {
    use figcraft_geom::kurbo::PathEl::*;
    let ok = |q: figcraft_geom::Point| q.x.is_finite() && q.y.is_finite() && q.x.abs() <= MAX_COORD && q.y.abs() <= MAX_COORD;
    p.elements().iter().all(|el| match *el {
        MoveTo(a) | LineTo(a) => ok(a),
        QuadTo(a, b) => ok(a) && ok(b),
        CurveTo(a, b, c) => ok(a) && ok(b) && ok(c),
        ClosePath => true,
    })
}

fn baked(p: &BezPath, xf: Affine) -> BezPath {
    let mut out = p.clone();
    out.apply_affine(xf);
    out
}

fn union(items: &[Item]) -> Option<Rect> {
    items.iter().map(|i| i.bbox).reduce(|a, b| a.union(b))
}

fn normal(b: BlendMode) -> bool {
    matches!(b, BlendMode::Normal | BlendMode::PassThrough)
}

impl<'a> Walker<'a> {
    pub fn new(doc: &'a Document, opts: &'a VdOptions) -> Self {
        Self { doc, opts, warn: Warnings::default(), names: Default::default() }
    }

    /// Reports that the export drew nothing at all (named after `layer`).
    pub fn nothing_drawn(&mut self, layer: &str) {
        if self.opts.compat != Compat::Ignore {
            self.warn.add(Warn::NothingDrawn, layer);
        }
    }

    pub fn finish(self) -> Vec<String> {
        self.warn.finish()
    }

    fn note(&mut self, w: Warn, n: &Node) {
        if self.opts.compat != Compat::Ignore {
            self.warn.add(w, &n.name);
        }
    }

    /// `name` as an `android:name` not used yet in this file (repeats get `_2`, `_3`…).
    fn unique(&mut self, name: &str) -> String {
        let base: String = name.chars().take(200).collect();
        let base = if base.trim().is_empty() { "layer".to_string() } else { base };
        // Bounded: a set of n names leaves one of the first n + 1 candidates free.
        let tries = self.names.len().saturating_add(1);
        let name = (1..=tries)
            .map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") })
            .find(|n| !self.names.contains(n))
            .unwrap_or_else(|| format!("{base}_{}", tries.saturating_add(1)));
        self.names.insert(name.clone());
        name
    }

    /// What layer `id` paints, drawn through `xf` (its parent's space → viewport).
    pub fn node(&mut self, id: NodeId, xf: Affine, depth: usize) -> Vec<Item> {
        let doc = self.doc;
        let Some(n) = doc.node(id) else { return Vec::new() };
        if depth > MAX_EXPORT_DEPTH || !n.visible || n.is_mask {
            return Vec::new();
        }
        let op = n.opacity;
        if !(op.is_finite() && op > 0.0) {
            return Vec::new();
        }
        let xf = xf * n.transform;
        self.effects(n);
        if !normal(n.blend) {
            self.note(Warn::Blend, n);
        }
        let mut items = Vec::new();
        if matches!(n.kind, NodeKind::Text(_)) {
            self.text(n, xf, &mut items);
        } else {
            self.shape(n, xf, depth, &mut items);
        }
        if op < 1.0 {
            if Item::overlap(&items) {
                match self.opts.compat {
                    Compat::BestEffort => {
                        self.note(Warn::OpacityApprox, n);
                        Item::scale_alpha(&mut items, op);
                    }
                    Compat::Exact => {
                        self.note(Warn::OpacityOmitted, n);
                        return Vec::new();
                    }
                    Compat::Ignore => {}
                }
            } else {
                Item::scale_alpha(&mut items, op);
            }
        }
        if self.opts.include_names && !items.is_empty() {
            let container = !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.children.is_empty();
            if container {
                let bbox = union(&items).unwrap_or(Rect::ZERO);
                let name = Some(self.unique(&n.name));
                items = vec![Item { kind: ItemKind::Group(GroupItem { name, xf: None, clip: None, children: items }), bbox }];
            } else {
                for it in &mut items {
                    let name = Some(self.unique(&n.name));
                    match &mut it.kind {
                        ItemKind::Path(p) => p.name = name,
                        ItemKind::Group(g) => g.name = name,
                    }
                }
            }
        }
        items
    }

    /// Effects have no Vector Drawable equivalent: each kind is noted.
    fn effects(&mut self, n: &Node) {
        for e in n.effects.iter().filter(|e| e.visible) {
            let w = match e.kind {
                EffectKind::DropShadow(_) => Warn::DropShadow,
                EffectKind::InnerShadow(_) => Warn::InnerShadow,
                EffectKind::LayerBlur(_) => Warn::LayerBlur,
                EffectKind::BackgroundBlur(_) => Warn::BackgroundBlur,
                EffectKind::Noise(_) => Warn::Noise,
                EffectKind::Texture(_) => Warn::Texture,
                EffectKind::Glass(_) => Warn::Glass,
            };
            self.note(w, n);
        }
    }

    fn text(&mut self, n: &Node, xf: Affine, items: &mut Vec<Item>) {
        let Some(runs) = text::runs(n) else { return };
        for (path, fills) in &runs {
            for p in fills {
                self.fill(n, p, path, false, xf, items);
            }
        }
        if n.has_visible_stroke() {
            // Glyph outlines are closed: inside and outside strokes follow them, as on the canvas.
            let even = stroke::outline_even_odd(n, true);
            for (path, _) in &runs {
                let area = figcraft_render::draw::stroke_area(n, path, true);
                for p in &n.strokes {
                    self.fill(n, p, &area, even, xf, items);
                }
            }
        }
    }

    fn shape(&mut self, n: &Node, xf: Affine, depth: usize, items: &mut Vec<Item>) {
        let doc = self.doc;
        let outline = match n.kind {
            NodeKind::Group | NodeKind::Page { .. } => None,
            _ => figcraft_render::node_outline(doc, n),
        };
        let boolean = matches!(n.kind, NodeKind::BooleanOp { .. });
        let closed = figcraft_doc::geometry::is_closed(n) || boolean;
        if closed && let Some((path, rule)) = &outline {
            for p in &n.fills {
                self.fill(n, p, path, *rule == FillRule::EvenOdd, xf, items);
            }
        }
        if !boolean && !n.children.is_empty() && !n.hides_contents() {
            let kids = self.children(n, xf, depth);
            let clips = n.clip && n.kind.is_frame_like();
            match &outline {
                Some((path, rule)) if clips => {
                    if let Some(g) = self.clipped(n, &baked(path, xf), *rule == FillRule::EvenOdd, kids) {
                        items.push(g);
                    }
                }
                _ => items.extend(kids),
            }
        }
        if n.has_visible_stroke() {
            self.strokes(n, outline.as_ref().map(|(p, _)| p), closed, xf, items);
        }
    }

    /// `n`'s children, bottom to top; a visible mask clips the layers above it (up to the next
    /// mask) to its outline.
    fn children(&mut self, n: &Node, xf: Affine, depth: usize) -> Vec<Item> {
        let doc = self.doc;
        let mut out = Vec::new();
        for run in doc.paint_runs(n) {
            let Some(&first) = n.children.get(run.start) else { continue };
            let Some(m) = doc.node(first).filter(|m| m.is_mask && m.visible) else {
                out.extend(self.node(first, xf, depth + 1));
                continue;
            };
            if m.mask_type != MaskType::Vector {
                match self.opts.compat {
                    Compat::BestEffort => self.note(Warn::MaskApprox, m),
                    Compat::Exact => {
                        self.note(Warn::MaskOmitted, m);
                        continue;
                    }
                    Compat::Ignore => {}
                }
            }
            let mut kids = Vec::new();
            for k in n.children.get(run.start + 1..run.end).unwrap_or(&[]) {
                kids.extend(self.node(*k, xf, depth + 1));
            }
            // A mask without an outline shows nothing of what it masks.
            let Some((outline, rule)) = figcraft_render::subtree_outline(doc, m, depth + 1) else { continue };
            if let Some(g) = self.clipped(m, &baked(&outline, xf * m.transform), rule == FillRule::EvenOdd, kids) {
                out.push(g);
            }
        }
        out
    }

    /// `kids` in a group clipped to `clip` (viewport space). A `<clip-path>` has no fill rule:
    /// an even-odd outline is rewritten as non-zero first.
    fn clipped(&mut self, n: &Node, clip: &BezPath, even: bool, kids: Vec<Item>) -> Option<Item> {
        if kids.is_empty() {
            return None;
        }
        if !writable(clip) {
            self.note(Warn::TooLarge, n);
            return None;
        }
        let clip = if even && self.opts.compat != Compat::Ignore {
            match nonzero(clip) {
                Some(p) => {
                    // Exact either way, but bestEffort reports the rewrite as it does for fills.
                    if self.opts.compat == Compat::BestEffort {
                        self.note(Warn::ClipEvenOddRewritten, n);
                    }
                    p
                }
                None if self.opts.compat == Compat::Exact => {
                    self.note(Warn::ClipEvenOddOmitted, n);
                    return None;
                }
                None => {
                    self.note(Warn::ClipEvenOddApprox, n);
                    clip.clone()
                }
            }
        } else {
            clip.clone()
        };
        if clip.elements().is_empty() {
            return None;
        }
        let bbox = union(&kids)?.intersect(clip.bounding_box());
        Some(Item { kind: ItemKind::Group(GroupItem { name: None, xf: None, clip: Some(clip), children: kids }), bbox })
    }

    fn strokes(&mut self, n: &Node, outline: Option<&BezPath>, closed: bool, xf: Affine, items: &mut Vec<Item>) {
        let native = stroke::native(n, xf);
        let mut area: Option<(BezPath, bool)> = None;
        for p in &n.strokes {
            if !p.visible || p.opacity <= 0.0 {
                continue;
            }
            if let (Some(nat), PaintKind::Solid { color }) = (&native, &p.kind) {
                if !normal(p.blend) {
                    self.note(Warn::Blend, n);
                }
                let color = color.with_alpha(color.a * p.opacity.clamp(0.0, 1.0));
                let data = baked(&nat.path, xf);
                if color.a <= 0.0 || data.elements().is_empty() {
                    continue;
                }
                if !writable(&data) {
                    self.note(Warn::TooLarge, n);
                    continue;
                }
                // Room for caps and mitred corners around the line.
                let reach = nat.width / 2.0 * nat.miter.max(std::f64::consts::SQRT_2);
                let bbox = data.bounding_box().inflate(reach, reach);
                let stroke = NativeStroke { color, width: nat.width, cap: nat.cap, join: nat.join, miter: nat.miter };
                let path = PathItem { name: None, data, fill: None, stroke: Some(stroke), even_odd: false };
                items.push(Item { kind: ItemKind::Path(path), bbox });
                continue;
            }
            if area.is_none() {
                let geom = match &n.kind {
                    NodeKind::BooleanOp { .. } => outline.cloned(),
                    _ => figcraft_doc::geometry::stroke_geometry(n),
                };
                let Some(g) = geom else { return };
                area = Some(((*figcraft_render::draw::stroke_area(n, &g, closed)).clone(), stroke::outline_even_odd(n, closed)));
            }
            if let Some((a, even)) = &area {
                self.fill(n, p, a, *even, xf, items);
            }
        }
    }

    /// Paint `p` filling `path` (layer space, even-odd when `even`) of layer `n`.
    fn fill(&mut self, n: &Node, p: &Paint, path: &BezPath, even: bool, xf: Affine, items: &mut Vec<Item>) {
        if !p.visible || p.opacity <= 0.0 || path.elements().is_empty() {
            return;
        }
        let opacity = p.opacity.clamp(0.0, 1.0);
        let fill = match &p.kind {
            PaintKind::Solid { color } => {
                let c = color.with_alpha(color.a * opacity);
                if c.a <= 0.0 {
                    return;
                }
                Some(Fill::Solid(c))
            }
            PaintKind::Gradient(_) => None,
            PaintKind::Image(_) => {
                self.note(Warn::ImageFill, n);
                return;
            }
            PaintKind::Pattern(_) => {
                self.note(Warn::PatternFill, n);
                return;
            }
        };
        let mut grad = None;
        if let PaintKind::Gradient(g) = &p.kind {
            let diamond = g.kind == figcraft_color::GradientKind::Diamond;
            if diamond {
                match self.opts.compat {
                    Compat::BestEffort => self.note(Warn::DiamondApprox, n),
                    Compat::Exact => {
                        self.note(Warn::DiamondOmitted, n);
                        return;
                    }
                    Compat::Ignore => return,
                }
            }
            grad = Some(gradient(g, opacity, xf, n.width, n.height, diamond));
        }
        if !normal(p.blend) {
            self.note(Warn::Blend, n);
        }
        let data = baked(path, xf);
        if !writable(&data) {
            self.note(Warn::TooLarge, n);
            return;
        }
        let (data, even_odd) = if even && self.opts.compat == Compat::BestEffort {
            match nonzero(&data) {
                Some(nz) => {
                    self.note(Warn::EvenOddRewritten, n);
                    (nz, false)
                }
                None => {
                    self.note(Warn::EvenOddKept, n);
                    (data, true)
                }
            }
        } else {
            (data, even)
        };
        if data.elements().is_empty() {
            return;
        }
        let bbox = data.bounding_box();
        let path_item = |data: BezPath, fill: Fill| PathItem { name: None, data, fill: Some(fill), stroke: None, even_odd };
        match (fill, grad) {
            (Some(f), _) => items.push(Item { kind: ItemKind::Path(path_item(data, f)), bbox }),
            (None, Some(GradOut::Solid(c))) => items.push(Item { kind: ItemKind::Path(path_item(data, Fill::Solid(c))), bbox }),
            (None, Some(GradOut::Plain(g))) => items.push(Item { kind: ItemKind::Path(path_item(data, Fill::Gradient(g))), bbox }),
            (None, Some(GradOut::Grouped { xf: gxf, inverse, grad: g })) => {
                let inner = Item { kind: ItemKind::Path(path_item(baked(&data, inverse), Fill::Gradient(g))), bbox };
                let group: GroupXf = gxf;
                items.push(Item { kind: ItemKind::Group(GroupItem { name: None, xf: Some(group), clip: None, children: vec![inner] }), bbox });
            }
            (None, Some(GradOut::Nothing) | None) => {}
        }
    }
}
