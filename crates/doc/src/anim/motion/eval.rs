//! Evaluating keyframes and clips at a point in time.
//!
//! Nothing here mutates the stored document. Three levels of API, cheapest first:
//!
//! * [`evaluate_node`]: pure function of one node and a time, returns the animated property values
//!   as an [`EvaluatedNode`] (only the properties that are animated are `Some`).
//! * [`MotionPlayer`]: for playback. Clones the document once, then [`MotionPlayer::seek`] only
//!   rewrites the animated properties of animated nodes each frame.
//! * [`evaluate`]: one-shot convenience that returns an evaluated *copy* of the document.
//!
//! ## How a node's transform is animated
//!
//! `Node::transform` is decomposed around the node's centre `h = size / 2`:
//!
//! ```text
//! transform = T(position + h) * R(rotation) * S(scale) * T(-h)        (shear of the base is kept)
//! ```
//!
//! so `position` is the layout position of the unrotated, unscaled box (the X/Y fields), and
//! rotation and scale pivot about the centre without moving `position`.

use super::clip::ClipKind;
use super::value::{AnimProperty, AnimValue};
use crate::{Blur, Document, Effect, EffectKind, Node, NodeId, Shadow};
use figcraft_color::{Color, PaintKind};
use figcraft_geom::{Affine, Vec2};

/// Animated property values of one node at one time. A field is `Some` only when the node has a
/// keyframe track or a visible clip for it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvaluatedNode {
    pub position: Option<Vec2>,
    pub scale: Option<Vec2>,
    /// Degrees, counter-clockwise positive.
    pub rotation: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub opacity: Option<f64>,
    pub corner_radius: Option<f64>,
    pub fill_color: Option<Color>,
    pub stroke_color: Option<Color>,
    pub stroke_weight: Option<f64>,
    /// Offset (x), offset (y), blur and colour of the first drop shadow.
    pub shadow_x: Option<f64>,
    pub shadow_y: Option<f64>,
    pub shadow_blur: Option<f64>,
    pub shadow_color: Option<Color>,
    /// Radius of the first layer blur.
    pub blur_radius: Option<f64>,
}

impl EvaluatedNode {
    pub fn is_empty(&self) -> bool {
        *self == EvaluatedNode::default()
    }

    /// The evaluated value of `property`, if it is animated.
    pub fn get(&self, property: AnimProperty) -> Option<AnimValue> {
        Some(match property {
            AnimProperty::Position => AnimValue::Vec2(self.position?),
            AnimProperty::Scale => AnimValue::Vec2(self.scale?),
            AnimProperty::Rotation => AnimValue::Scalar(self.rotation?),
            AnimProperty::Width => AnimValue::Scalar(self.width?),
            AnimProperty::Height => AnimValue::Scalar(self.height?),
            AnimProperty::Opacity => AnimValue::Scalar(self.opacity?),
            AnimProperty::CornerRadius => AnimValue::Scalar(self.corner_radius?),
            AnimProperty::FillColor => AnimValue::Color(self.fill_color?),
            AnimProperty::StrokeColor => AnimValue::Color(self.stroke_color?),
            AnimProperty::StrokeWeight => AnimValue::Scalar(self.stroke_weight?),
            AnimProperty::ShadowX => AnimValue::Scalar(self.shadow_x?),
            AnimProperty::ShadowY => AnimValue::Scalar(self.shadow_y?),
            AnimProperty::ShadowBlur => AnimValue::Scalar(self.shadow_blur?),
            AnimProperty::ShadowColor => AnimValue::Color(self.shadow_color?),
            AnimProperty::BlurRadius => AnimValue::Scalar(self.blur_radius?),
        })
    }

    /// Sets `property`; a value of the wrong shape is ignored.
    pub fn set(&mut self, property: AnimProperty, value: AnimValue) {
        match (property, value) {
            (AnimProperty::Position, AnimValue::Vec2(v)) => self.position = Some(v),
            (AnimProperty::Scale, AnimValue::Vec2(v)) => self.scale = Some(v),
            (AnimProperty::Rotation, AnimValue::Scalar(v)) => self.rotation = Some(v),
            (AnimProperty::Width, AnimValue::Scalar(v)) => self.width = Some(v),
            (AnimProperty::Height, AnimValue::Scalar(v)) => self.height = Some(v),
            (AnimProperty::Opacity, AnimValue::Scalar(v)) => self.opacity = Some(v),
            (AnimProperty::CornerRadius, AnimValue::Scalar(v)) => self.corner_radius = Some(v),
            (AnimProperty::FillColor, AnimValue::Color(v)) => self.fill_color = Some(v),
            (AnimProperty::StrokeColor, AnimValue::Color(v)) => self.stroke_color = Some(v),
            (AnimProperty::StrokeWeight, AnimValue::Scalar(v)) => self.stroke_weight = Some(v),
            (AnimProperty::ShadowX, AnimValue::Scalar(v)) => self.shadow_x = Some(v),
            (AnimProperty::ShadowY, AnimValue::Scalar(v)) => self.shadow_y = Some(v),
            (AnimProperty::ShadowBlur, AnimValue::Scalar(v)) => self.shadow_blur = Some(v),
            (AnimProperty::ShadowColor, AnimValue::Color(v)) => self.shadow_color = Some(v),
            (AnimProperty::BlurRadius, AnimValue::Scalar(v)) => self.blur_radius = Some(v),
            _ => {}
        }
    }

    /// Writes the animated values onto `out`, computing transform parts relative to `base`.
    pub fn apply_to(&self, base: &Node, out: &mut Node) {
        if let Some(o) = self.opacity {
            out.opacity = if o.is_finite() { o.clamp(0.0, 1.0) as f32 } else { base.opacity };
        }
        if let Some(r) = self.corner_radius.filter(|r| r.is_finite()) {
            out.corners.radii = [r.max(0.0); 4];
        }
        if let (Some(c), Some(i)) = (self.fill_color, first_solid(&base.fills))
            && let Some(p) = out.fills.get_mut(i)
        {
            p.kind = PaintKind::Solid { color: c };
        }
        if let (Some(c), Some(i)) = (self.stroke_color, first_solid(&base.strokes))
            && let Some(p) = out.strokes.get_mut(i)
        {
            p.kind = PaintKind::Solid { color: c };
        }
        if let Some(w) = self.stroke_weight.filter(|w| w.is_finite()) {
            out.stroke.weight = w.max(0.0);
        }
        if let Some(i) = first_drop_shadow(&base.effects)
            && let Some(Effect { kind: EffectKind::DropShadow(s), .. }) = out.effects.get_mut(i)
        {
            if let Some(v) = self.shadow_x.filter(|v| v.is_finite()) {
                s.offset.0 = v;
            }
            if let Some(v) = self.shadow_y.filter(|v| v.is_finite()) {
                s.offset.1 = v;
            }
            if let Some(v) = self.shadow_blur.filter(|v| v.is_finite()) {
                s.radius = v.max(0.0);
            }
            if let Some(c) = self.shadow_color {
                s.color = c;
            }
        }
        if let (Some(r), Some(i)) = (self.blur_radius.filter(|r| r.is_finite()), first_layer_blur(&base.effects))
            && let Some(Effect { kind: EffectKind::LayerBlur(b), .. }) = out.effects.get_mut(i)
        {
            b.radius = r.max(0.0);
        }
        let transform_touched =
            self.position.is_some() || self.scale.is_some() || self.rotation.is_some() || self.width.is_some() || self.height.is_some();
        if !transform_touched {
            return;
        }
        let size = Vec2::new(
            self.width.filter(|v| v.is_finite()).map_or(base.width, |v| v.max(0.0)),
            self.height.filter(|v| v.is_finite()).map_or(base.height, |v| v.max(0.0)),
        );
        out.width = size.x;
        out.height = size.y;
        let pos = self.position.filter(|p| p.x.is_finite() && p.y.is_finite()).unwrap_or_else(|| base_position(base));
        let dtheta = self.rotation.filter(|r| r.is_finite()).map_or(0.0, |r| -(r - base_rotation_deg(base)).to_radians());
        let k = self.scale.filter(|s| s.x.is_finite() && s.y.is_finite()).unwrap_or(Vec2::new(1.0, 1.0));
        let c = base.transform.as_coeffs();
        let linear = Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0]);
        let l = Affine::rotate(dtheta) * linear * Affine::scale_non_uniform(k.x, k.y);
        let lc = l.as_coeffs();
        let h = Vec2::new(size.x * 0.5, size.y * 0.5);
        out.transform =
            Affine::new([lc[0], lc[1], lc[2], lc[3], pos.x + h.x - (lc[0] * h.x + lc[2] * h.y), pos.y + h.y - (lc[1] * h.x + lc[3] * h.y)]);
    }
}

fn first_solid(paints: &[figcraft_color::Paint]) -> Option<usize> {
    paints.iter().position(|p| matches!(p.kind, PaintKind::Solid { .. }))
}

/// Index of the first drop shadow, the one the shadow properties animate.
pub fn first_drop_shadow(effects: &[Effect]) -> Option<usize> {
    effects.iter().position(|e| matches!(e.kind, EffectKind::DropShadow(_)))
}

/// Index of the first layer blur, the one [`AnimProperty::BlurRadius`] animates.
pub fn first_layer_blur(effects: &[Effect]) -> Option<usize> {
    effects.iter().position(|e| matches!(e.kind, EffectKind::LayerBlur(_)))
}

/// The node's static layout position.
pub fn base_position(node: &Node) -> Vec2 {
    let c = node.transform.as_coeffs();
    let (hx, hy) = (node.width * 0.5, node.height * 0.5);
    Vec2::new(c[4] - hx + (c[0] * hx + c[2] * hy), c[5] - hy + (c[1] * hx + c[3] * hy))
}

/// The node's static rotation in degrees, counter-clockwise positive.
pub fn base_rotation_deg(node: &Node) -> f64 {
    let c = node.transform.as_coeffs();
    -c[1].atan2(c[0]).to_degrees()
}

/// The static (un-animated) value of `property` on `node`.
pub fn base_value(node: &Node, property: AnimProperty) -> Option<AnimValue> {
    Some(match property {
        AnimProperty::Position => AnimValue::Vec2(base_position(node)),
        AnimProperty::Scale => AnimValue::Vec2(Vec2::new(1.0, 1.0)),
        AnimProperty::Rotation => AnimValue::Scalar(base_rotation_deg(node)),
        AnimProperty::Width => AnimValue::Scalar(node.width),
        AnimProperty::Height => AnimValue::Scalar(node.height),
        AnimProperty::Opacity => AnimValue::Scalar(node.opacity as f64),
        AnimProperty::CornerRadius => AnimValue::Scalar(node.corners.radii[0]),
        AnimProperty::FillColor => match node.fills.get(first_solid(&node.fills)?)?.kind {
            PaintKind::Solid { color } => AnimValue::Color(color),
            _ => return None,
        },
        AnimProperty::StrokeColor => match node.strokes.get(first_solid(&node.strokes)?)?.kind {
            PaintKind::Solid { color } => AnimValue::Color(color),
            _ => return None,
        },
        AnimProperty::StrokeWeight => AnimValue::Scalar(node.stroke.weight),
        AnimProperty::ShadowX | AnimProperty::ShadowY | AnimProperty::ShadowBlur | AnimProperty::ShadowColor => {
            let Effect { kind: EffectKind::DropShadow(Shadow { color, offset, radius, .. }), .. } =
                node.effects.get(first_drop_shadow(&node.effects)?)?
            else {
                return None;
            };
            match property {
                AnimProperty::ShadowX => AnimValue::Scalar(offset.0),
                AnimProperty::ShadowY => AnimValue::Scalar(offset.1),
                AnimProperty::ShadowBlur => AnimValue::Scalar(*radius),
                _ => AnimValue::Color(*color),
            }
        }
        AnimProperty::BlurRadius => {
            let Effect { kind: EffectKind::LayerBlur(Blur { radius, .. }), .. } = node.effects.get(first_layer_blur(&node.effects)?)? else {
                return None;
            };
            AnimValue::Scalar(*radius)
        }
    })
}

/// Evaluates a node's keyframes and clips at `t_ms`.
pub fn evaluate_node(node: &Node, t_ms: f64) -> EvaluatedNode {
    let mut ev = EvaluatedNode::default();
    let m = &node.motion;
    if m.is_empty() {
        return ev;
    }
    let t = if t_ms.is_finite() { t_ms.max(0.0) } else { 0.0 };
    for tr in m.tracks.iter().filter(|tr| tr.visible) {
        if let Some(v) = tr.value_at(t) {
            ev.set(tr.property, v);
        }
    }
    for c in m.clips.iter().filter(|c| c.visible) {
        let v = c.value_at(t);
        match c.kind {
            ClipKind::Position => {
                let cur = ev.position.unwrap_or_else(|| base_position(node));
                ev.position = Some(Vec2::new(cur.x + v.x, cur.y + v.y));
            }
            ClipKind::Rotation => ev.rotation = Some(ev.rotation.unwrap_or_else(|| base_rotation_deg(node)) + v.x),
            ClipKind::Scale => {
                let cur = ev.scale.unwrap_or(Vec2::new(1.0, 1.0));
                ev.scale = Some(Vec2::new(cur.x * v.x, cur.y * v.y));
            }
            ClipKind::Size => {
                ev.width = Some(ev.width.unwrap_or(node.width) * v.x);
                ev.height = Some(ev.height.unwrap_or(node.height) * v.y);
            }
            ClipKind::Opacity => ev.opacity = Some((ev.opacity.unwrap_or(node.opacity as f64) * v.x).clamp(0.0, 1.0)),
        }
    }
    ev
}

/// Value of `property` at `t_ms` including clips.
pub fn value_at(node: &Node, property: AnimProperty, t_ms: f64) -> Option<AnimValue> {
    evaluate_node(node, t_ms).get(property).or_else(|| base_value(node, property))
}

/// Value of `property` at `t_ms` from its keyframe track alone (clips ignored).
pub fn keyframed_value_at(node: &Node, property: AnimProperty, t_ms: f64) -> Option<AnimValue> {
    let t = if t_ms.is_finite() { t_ms.max(0.0) } else { 0.0 };
    node.motion.track(property).and_then(|tr| tr.value_at(t)).or_else(|| base_value(node, property))
}

/// Ids of the nodes below `page` (and the page itself) that have keyframes or clips, in paint order.
pub fn animated_nodes(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.subtree(page).into_iter().filter(|id| doc.node(*id).is_some_and(|n| !n.motion.is_empty())).collect()
}

/// Evaluated nodes of one page at one time, sorted by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionFrame {
    pub t_ms: f64,
    pub nodes: Vec<(NodeId, EvaluatedNode)>,
}

impl MotionFrame {
    pub fn get(&self, id: NodeId) -> Option<&EvaluatedNode> {
        self.nodes.binary_search_by_key(&id, |(i, _)| *i).ok().map(|i| &self.nodes[i].1)
    }

    /// Writes the frame onto `out`, a copy of `base`.
    pub fn apply(&self, base: &Document, out: &mut Document) {
        for (id, ev) in &self.nodes {
            let b = base.node(*id).cloned();
            if let (Some(b), Some(o)) = (b, out.node_mut(*id)) {
                ev.apply_to(&b, o);
            }
        }
    }
}

/// Evaluates every animated node of `page` at `t_ms` without touching `doc`.
pub fn evaluate_frame(doc: &Document, page: NodeId, t_ms: f64) -> MotionFrame {
    let mut nodes: Vec<(NodeId, EvaluatedNode)> = animated_nodes(doc, page)
        .into_iter()
        .filter_map(|id| doc.node(id).map(|n| (id, evaluate_node(n, t_ms))))
        .filter(|(_, e)| !e.is_empty())
        .collect();
    nodes.sort_by_key(|(id, _)| *id);
    MotionFrame { t_ms, nodes }
}

/// A copy of `doc` with the animation of `page` evaluated at `t_ms` baked into the node properties.
pub fn evaluate(doc: &Document, page: NodeId, t_ms: f64) -> Document {
    let mut out = doc.clone();
    evaluate_frame(doc, page, t_ms).apply(doc, &mut out);
    out
}

/// Frame-by-frame playback over a scratch copy of the document.
pub struct MotionPlayer {
    scratch: Document,
    page: NodeId,
    ids: Vec<NodeId>,
}

impl MotionPlayer {
    pub fn new(doc: &Document, page: NodeId) -> Self {
        Self { scratch: doc.clone(), page, ids: animated_nodes(doc, page) }
    }

    pub fn page(&self) -> NodeId {
        self.page
    }

    /// Ids of the animated nodes the player updates.
    pub fn animated_ids(&self) -> &[NodeId] {
        &self.ids
    }

    /// The evaluated document as of the last [`seek`](Self::seek).
    pub fn document(&self) -> &Document {
        &self.scratch
    }

    /// Evaluates the page at `t_ms` against `base` and returns the evaluated scratch document.
    pub fn seek(&mut self, base: &Document, t_ms: f64) -> &Document {
        for id in &self.ids {
            let b = base.node(*id).cloned();
            if let (Some(b), Some(o)) = (b, self.scratch.node_mut(*id)) {
                evaluate_node(&b, t_ms).apply_to(&b, o);
            }
        }
        &self.scratch
    }
}

#[cfg(test)]
mod tests {
    use super::super::clip::{Clip, ClipKind, ClipMode, ClipRepeat};
    use super::super::track::Keyframe;
    use super::super::value::AnimProperty;
    use super::*;
    use crate::NodeKind;

    fn doc_with_rect() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let id = d.create(page, NodeKind::Rectangle, "Card", 100.0, 50.0, 80.0, 40.0).unwrap();
        (d, page, id)
    }

    #[test]
    fn keyframes_move_the_layout_position() {
        let (mut d, page, id) = doc_with_rect();
        let n = d.node_mut(id).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Position);
        tr.set_key(0, AnimValue::Vec2(Vec2::new(100.0, 50.0)), None);
        tr.set_key(1000, AnimValue::Vec2(Vec2::new(300.0, 50.0)), None);
        let n = d.node(id).unwrap();
        let ev = evaluate_node(n, 500.0);
        assert_eq!(ev.position, Some(Vec2::new(200.0, 50.0)));
        let out = evaluate(&d, page, 500.0);
        let moved = out.node(id).unwrap();
        assert!((base_position(moved).x - 200.0).abs() < 1e-6);
        assert!((moved.width - 80.0).abs() < 1e-6);
    }

    #[test]
    fn a_clip_fades_in() {
        let (mut d, page, id) = doc_with_rect();
        let n = d.node_mut(id).unwrap();
        n.motion.clips.push(Clip::new(ClipKind::Opacity, ClipMode::In).amount(0.0).dur(1000));
        let out = evaluate(&d, page, 500.0);
        let o = out.node(id).unwrap();
        assert!(o.opacity > 0.0 && o.opacity < 1.0, "{}", o.opacity);
        let end = evaluate(&d, page, 2000.0);
        assert!((end.node(id).unwrap().opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_player_seeks_without_mutating_the_base() {
        let (mut d, page, id) = doc_with_rect();
        let n = d.node_mut(id).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Width);
        tr.set_key(0, AnimValue::Scalar(80.0), None);
        tr.set_key(1000, AnimValue::Scalar(160.0), None);
        let mut player = MotionPlayer::new(&d, page);
        assert_eq!(player.animated_ids(), &[id]);
        let at_half = player.seek(&d, 500.0).node(id).unwrap().width;
        assert!((at_half - 120.0).abs() < 1e-6);
        assert!((d.node(id).unwrap().width - 80.0).abs() < 1e-6, "the base is untouched");
    }

    #[test]
    fn a_hidden_track_is_ignored() {
        let (mut d, page, id) = doc_with_rect();
        let n = d.node_mut(id).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Opacity);
        tr.set_key(0, AnimValue::Scalar(0.0), None);
        tr.set_key(1000, AnimValue::Scalar(1.0), None);
        tr.visible = false;
        let out = evaluate(&d, page, 500.0);
        assert!((out.node(id).unwrap().opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_clip_repeat_stays_finite() {
        let c = Clip::new(ClipKind::Position, ClipMode::In).amount(50.0).dur(100).repeat(ClipRepeat::Times(3));
        for t in [0.0, 50.0, 250.0, 400.0] {
            let v = c.value_at(t);
            assert!(v.x.is_finite() && v.y.is_finite());
        }
        let _ = Keyframe { time_ms: 0, value: AnimValue::Scalar(0.0), easing: super::super::easing::Ease::Linear };
    }
}
