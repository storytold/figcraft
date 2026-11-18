use super::layout::Place;
use super::{
    Affine, Document, HConstraint, HashMap, NodeId, NodeKind, Overflow, Pos2, Presenter, PrototypeSettings, ScrollMove, ScrollPosition, Scroller,
    VConstraint,
};

/// How quickly a fling slows down after a drag (1/s; it travels its speed / decay).
pub(super) const DECAY_DRAG: f64 = 2.5;

/// How quickly the wheel's coast slows down (1/s).
pub(super) const DECAY_WHEEL: f64 = 8.0;

/// Stiffness of the spring that pulls an overscroll back (1/s, critically damped).
const SPRING: f64 = 16.0;

/// A fling inside its range stops below this speed (units/s).
const FLING_STOP: f64 = 6.0;

/// The wheel coasts on when it was faster than this as it stopped (screen points/s).
pub(super) const COAST_MIN: f64 = 40.0;

/// How far a drag past the end can stretch the content at most (screen points).
pub(super) const RUBBER_PTS: f64 = 240.0;

impl Presenter {
    /// The current offset of `t` (a frame's scroll, else the view's pan).
    pub(super) fn offset(&self, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.scroll.get(&f).copied().unwrap_or((0.0, 0.0)),
            Scroller::View => self.pan.unwrap_or((self.pan_range.0 / 2.0, 0.0)),
        }
    }

    /// How far `t` scrolls (`base`: the player's document without scroll offsets).
    pub(super) fn limits(&self, base: &Document, t: Scroller) -> (f64, f64) {
        match t {
            Scroller::Frame(f) => self.range(base, f),
            Scroller::View => self.pan_range,
        }
    }

    pub(super) fn set_offset(&mut self, t: Scroller, v: (f64, f64)) {
        if !v.0.is_finite() || !v.1.is_finite() {
            return;
        }
        match t {
            Scroller::Frame(f) => {
                if self.scroll.get(&f) != Some(&v) {
                    self.scroll.insert(f, v);
                    self.rev += 1;
                }
            }
            Scroller::View => self.pan = Some(v),
        }
    }

    /// Move a fling on to `now`; true while it is still moving.
    pub(super) fn step_fling(&mut self, now: f64) -> bool {
        let Some(mut f) = self.fling.take() else { return false };
        let Some(base) = self.doc.as_ref() else { return false };
        let r = self.limits(base, f.target);
        let x = self.offset(f.target);
        let dt = (now - f.at).clamp(0.0, 0.25);
        f.at = now;
        let (nx, vx, done_x) = fling_axis(x.0, f.vel.0, r.0, dt, f.decay, f.bounce);
        let (ny, vy, done_y) = fling_axis(x.1, f.vel.1, r.1, dt, f.decay, f.bounce);
        self.set_offset(f.target, (nx, ny));
        f.vel = (vx, vy);
        if done_x && done_y {
            return false;
        }
        self.fling = Some(f);
        true
    }

    /// How far `frame` scrolls (in `doc`, without scroll offsets): the presented frame within the
    /// device's screen, other frames within themselves.
    pub(super) fn range(&self, doc: &Document, frame: NodeId) -> (f64, f64) {
        let vp = if frame == self.current { viewport(doc, frame) } else { None };
        scroll_range_in(doc, frame, vp)
    }

    /// What "Scroll to" `dest` moves: every scrolling frame around it, innermost first, brings the
    /// layer (then the frame holding it) to its top-left as far as it can scroll, the innermost
    /// `offset` short of it. Each entry is (frame, offset now, offset after).
    pub(super) fn scroll_to_layer(&self, doc: &Document, dest: NodeId, offset: [f64; 2]) -> Vec<ScrollMove> {
        let mut out = Vec::new();
        let mut target = dest;
        // The innermost scrolling frame leaves `offset` before the layer.
        let mut off = offset.map(|v| if v.is_finite() { v } else { 0.0 });
        for f in doc.ancestors(dest) {
            let (rx, ry) = self.range(doc, f);
            let Some(tn) = doc.node(target) else { break };
            if rx <= 0.0 && ry <= 0.0 {
                continue;
            }
            let rel = doc.world_transform(f).inverse() * doc.world_transform(target);
            let b = figcraft_geom::transform_rect_bbox(rel, tn.local_rect());
            let from = self.scroll.get(&f).copied().unwrap_or((0.0, 0.0));
            let to = clamp_scroll((if rx > 0.0 { b.x0 - off[0] } else { from.0 }, if ry > 0.0 { b.y0 - off[1] } else { from.1 }), (rx, ry));
            out.push((f, from, to));
            target = f;
            off = [0.0, 0.0];
        }
        out
    }

    /// Scroll the presented frame to `(x, y)` (page units, clamped to what it can scroll).
    pub fn scroll_to(&mut self, x: f64, y: f64) {
        let Some(doc) = self.doc.as_ref() else { return };
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let to = clamp_scroll((x, y), self.range(doc, self.current));
        self.scroll.insert(self.current, to);
        self.scroll_anim = None;
        self.stop_scrolling();
        self.rev += 1;
    }
}

/// How far `frame`'s content can scroll `(x, y)` (0 when it doesn't overflow or doesn't scroll).
pub fn scroll_range(doc: &Document, frame: NodeId) -> (f64, f64) {
    scroll_range_in(doc, frame, None)
}

/// How far `frame`'s content can scroll when `viewport` (page units, from its top-left) of it
/// shows, as a device's screen shows a top-level frame: the frame itself is content then, so a
/// frame bigger than the screen scrolls through its whole size whatever its overflow (as in
/// Figma, a top-level frame needs no overflow setting for that). Without a viewport the frame
/// shows at its own size and what overflows it scrolls. Content past the frame's own edges
/// scrolls only on the axes its overflow allows.
pub fn scroll_range_in(doc: &Document, frame: NodeId, viewport: Option<(f64, f64)>) -> (f64, f64) {
    let Some(n) = doc.node(frame) else { return (0.0, 0.0) };
    if n.overflow == Overflow::None && viewport.is_none() {
        return (0.0, 0.0);
    }
    let (mut x1, mut y1) = (0.0f64, 0.0f64);
    for c in &n.children {
        let Some(cn) = doc.node(*c) else { continue };
        if !cn.visible || cn.scroll_position == ScrollPosition::Fixed {
            continue;
        }
        let b = figcraft_geom::transform_rect_bbox(cn.transform, cn.local_rect());
        x1 = x1.max(b.x1);
        y1 = y1.max(b.y1);
    }
    if let Some(l) = &n.layout {
        x1 += l.padding[1];
        y1 += l.padding[2];
    }
    // Axes the overflow doesn't scroll clip what lies past the frame's edges.
    if !matches!(n.overflow, Overflow::Horizontal | Overflow::Both) {
        x1 = 0.0;
    }
    if !matches!(n.overflow, Overflow::Vertical | Overflow::Both) {
        y1 = 0.0;
    }
    let (vw, vh) = match viewport {
        Some(v) => {
            x1 = x1.max(n.width);
            y1 = y1.max(n.height);
            v
        }
        None => (n.width, n.height),
    };
    let (rx, ry) = ((x1 - vw).max(0.0), (y1 - vh).max(0.0));
    (if rx.is_finite() { rx } else { 0.0 }, if ry.is_finite() { ry } else { 0.0 })
}

/// `(x, y)` kept within `0..=range` on each axis (0 for non-finite values).
pub(super) fn clamp_scroll((x, y): (f64, f64), (rx, ry): (f64, f64)) -> (f64, f64) {
    let c = |v: f64, r: f64| if v.is_finite() && r.is_finite() { v.clamp(0.0, r.max(0.0)) } else { 0.0 };
    (c(x, rx), c(y, ry))
}

/// One axis of a fling after `dt` seconds: from offset `x` moving at `v` (units/s) in a range
/// `0..=r`. Inside the range it slows down exponentially (`decay`, 1/s: it travels `v / decay`);
/// past an end it either stops there or, with `bounce`, a critically damped spring pulls it back.
/// Both are solved exactly over short sub-steps, so the motion doesn't depend on the frame rate.
/// Returns the new offset and speed, and whether it has come to rest.
pub fn fling_axis(x: f64, v: f64, r: f64, dt: f64, decay: f64, bounce: bool) -> (f64, f64, bool) {
    if !(x.is_finite() && v.is_finite() && r.is_finite() && dt.is_finite() && decay > 0.0) || r <= 0.0 {
        return (if r.is_finite() && x.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 }, 0.0, true);
    }
    // A frame is never longer than a second, nor a fling faster than a million units a second.
    let dt = dt.clamp(0.0, 1.0);
    let steps = (dt * 240.0).ceil().clamp(1.0, 64.0);
    let h = dt / steps;
    let (mut x, mut v) = (x, v.clamp(-1e6, 1e6));
    for _ in 0..steps as usize {
        if (0.0..=r).contains(&x) {
            let e = (-decay * h).exp();
            x += v * (1.0 - e) / decay;
            v *= e;
            if !bounce && !(0.0..=r).contains(&x) {
                x = x.clamp(0.0, r);
                v = 0.0;
            }
        } else {
            let end = x.clamp(0.0, r);
            let (d, e) = (x - end, (-SPRING * h).exp());
            let c = v + SPRING * d;
            x = end + (d + c * h) * e;
            v = (v - SPRING * c * h) * e;
        }
    }
    if !x.is_finite() || !v.is_finite() {
        return (0.0, 0.0, true);
    }
    let end = x.clamp(0.0, r);
    if x == end && v.abs() < FLING_STOP {
        return (x, 0.0, true);
    }
    if x != end && (x - end).abs() < 0.5 && v.abs() < FLING_STOP * 4.0 {
        return (end, 0.0, true);
    }
    (x, v, false)
}

/// Where a drag to offset `x` shows in a range `0..=r`: past an end it stretches less the further
/// it goes, never more than `limit` past.
pub fn rubber_band(x: f64, r: f64, limit: f64) -> f64 {
    if !x.is_finite() || !r.is_finite() || r <= 0.0 || !limit.is_finite() || limit <= 0.0 {
        return if x.is_finite() && r.is_finite() { x.clamp(0.0, r.max(0.0)) } else { 0.0 };
    }
    let band = |over: f64| limit * (1.0 - 1.0 / (over * 0.55 / limit + 1.0));
    if x < 0.0 {
        -band(-x)
    } else if x > r {
        r + band(x - r)
    } else {
        x
    }
}

/// The prototype settings of the page holding `id` (defaults when there's none).
pub fn page_settings(doc: &Document, id: NodeId) -> PrototypeSettings {
    doc.page_of(id)
        .and_then(|pg| doc.node(pg))
        .and_then(|n| match &n.kind {
            NodeKind::Page { prototype, .. } => Some(prototype.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The part of top-level `frame` its page's device shows (page units): the frame clipped to the
/// device's screen. None without a device: the frame then shows whole.
pub fn viewport(doc: &Document, frame: NodeId) -> Option<(f64, f64)> {
    let d = page_settings(doc, frame).device()?;
    let n = doc.node(frame)?;
    Some((n.width.min(d.width), n.height.min(d.height)))
}

/// Size `frame` to the `(w, h)` of it a screen shows. The content keeps its place (what doesn't
/// fit is clipped or scrolls); fixed layers follow their constraints to the screen's edges, so a
/// tab bar pinned to the frame's bottom sits at the bottom of the screen (stretching constraints
/// keep the layer's size).
pub fn fit_viewport(doc: &mut Document, frame: NodeId, (w, h): (f64, f64)) {
    let Some(n) = doc.node(frame) else { return };
    let (dw, dh) = (w - n.width, h - n.height);
    if !dw.is_finite() || !dh.is_finite() || (dw == 0.0 && dh == 0.0) {
        return;
    }
    let fixed: Vec<NodeId> =
        n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed)).collect();
    for c in fixed {
        if let Some(m) = doc.node_mut(c) {
            let dx = match m.constraints.horizontal {
                HConstraint::Right => dw,
                HConstraint::Center => dw / 2.0,
                _ => 0.0,
            };
            let dy = match m.constraints.vertical {
                VConstraint::Bottom => dh,
                VConstraint::Center => dh / 2.0,
                _ => 0.0,
            };
            m.transform = Affine::translate((dx, dy)) * m.transform;
        }
    }
    if let Some(m) = doc.node_mut(frame) {
        m.width = w;
        m.height = h;
    }
}

/// Shift scrolling frames' children by their scroll offsets. Fixed children stay put; sticky
/// children scroll until they reach the frame's top/left edge, then stay there.
pub fn apply_scroll(doc: &Document, scroll: &HashMap<NodeId, (f64, f64)>) -> Document {
    let mut out = doc.clone();
    let sticky = |pos: f64, s: f64| if pos >= 0.0 && pos - s < 0.0 { -pos } else { -s };
    for (&frame, &(sx, sy)) in scroll {
        if !sx.is_finite() || !sy.is_finite() || (sx == 0.0 && sy == 0.0) {
            continue;
        }
        let Some(n) = doc.node(frame) else { continue };
        for &c in &n.children {
            let Some(cn) = doc.node(c) else { continue };
            let (dx, dy) = match cn.scroll_position {
                ScrollPosition::Fixed => continue,
                ScrollPosition::Scrolls => (-sx, -sy),
                ScrollPosition::Sticky => {
                    let b = figcraft_geom::transform_rect_bbox(cn.transform, cn.local_rect());
                    (sticky(b.x0, sx), sticky(b.y0, sy))
                }
            };
            if let Some(m) = out.node_mut(c) {
                m.transform = Affine::translate((dx, dy)) * m.transform;
            }
        }
    }
    out
}

/// Fixed layers draw (and take clicks) above the content that scrolls under them: each frame's
/// fixed children move to the top of its children, keeping their order (for the player's copy).
pub fn raise_fixed(doc: &mut Document) {
    let fixed = |c: &NodeId| doc.node(*c).is_some_and(|c| c.scroll_position == ScrollPosition::Fixed);
    let moves: Vec<(NodeId, Vec<NodeId>)> = doc
        .nodes()
        .filter(|n| !matches!(n.kind, NodeKind::Page { .. }) && n.children.iter().any(fixed))
        .filter_map(|n| {
            let (top, mut order): (Vec<NodeId>, Vec<NodeId>) = n.children.iter().partition(|c| fixed(c));
            order.extend(top);
            (order != n.children).then_some((n.id, order))
        })
        .collect();
    for (id, order) in moves {
        if let Some(n) = doc.node_mut(id) {
            n.children = order;
        }
    }
}

/// Scrolling frames under the screen point `q`, innermost first: those of `root` (in the display
/// document `disp`) that can scroll (`base` is the player's document without scroll offsets).
pub(super) fn scrollers_at(p: &Presenter, base: &Document, disp: &Document, root: NodeId, place: &Place, q: Pos2) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = disp
        .subtree(root)
        .into_iter()
        .filter(|f| disp.node(*f).is_some_and(|n| n.visible && (n.overflow != Overflow::None || *f == root)))
        .filter(|f| disp.world_bounds(*f).is_some_and(|b| place.rect(b).contains(q)))
        .filter(|f| p.range(base, *f) != (0.0, 0.0))
        .collect();
    out.reverse();
    out
}

/// Screen points → `t`'s units (page units for frames, points for the view).
pub(super) fn units_per_point(t: Scroller, scale: f64) -> f64 {
    match t {
        Scroller::Frame(_) => 1.0 / scale.max(1e-6),
        Scroller::View => 1.0,
    }
}
