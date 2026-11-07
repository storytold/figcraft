use super::{
    Affine, BTreeMap, Direction, Document, HashMap, HashSet, Node, NodeId, OverlayPosition, Paint, PaintKind, Point, Rect, Transition, Vec2,
    VecDeque, pos2,
};

// ---------------------------------------------------------------------------------------------
// Pure prototype logic (tested below).

/// Layer names from below `root` down to `id`, joined (the smart-animate matching key).
pub fn name_path(doc: &Document, root: NodeId, id: NodeId) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = id;
    for _ in 0..1024 {
        if cur == root {
            parts.reverse();
            return Some(parts.join("\u{1f}"));
        }
        let n = doc.node(cur)?;
        parts.push(n.name.as_str());
        cur = n.parent?;
    }
    None
}

/// Smart-animate pairs `(from layer, to layer)`: same name path below the two roots; repeated
/// paths pair up in layer order.
pub fn match_layers(from: &Document, from_root: NodeId, to: &Document, to_root: NodeId) -> Vec<(NodeId, NodeId)> {
    let mut pool: HashMap<String, VecDeque<NodeId>> = HashMap::new();
    for id in from.subtree(from_root).into_iter().skip(1) {
        if let Some(p) = name_path(from, from_root, id) {
            pool.entry(p).or_default().push_back(id);
        }
    }
    let mut out = Vec::new();
    for id in to.subtree(to_root).into_iter().skip(1) {
        if let Some(p) = name_path(to, to_root, id)
            && let Some(q) = pool.get_mut(&p)
            && let Some(f) = q.pop_front()
        {
            out.push((f, id));
        }
    }
    out
}

pub(super) fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Centre, rotation and scale of a node box `(w, h)` placed by `m`.
fn decompose(m: Affine, (w, h): (f64, f64)) -> Option<[f64; 5]> {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let sx = a.hypot(b);
    if !sx.is_finite() || sx < 1e-9 {
        return None;
    }
    let sy = (a * d - b * c) / sx;
    let ctr = m * Point::new(w / 2.0, h / 2.0);
    Some([ctr.x, ctr.y, b.atan2(a), sx, sy])
}

/// Interpolate two placements of a box: centre, rotation (shortest way round) and scale; the box
/// size is `size` (already interpolated).
pub fn lerp_transform(a: Affine, sa: (f64, f64), b: Affine, sb: (f64, f64), size: (f64, f64), t: f64) -> Affine {
    match (decompose(a, sa), decompose(b, sb)) {
        (Some(x), Some(y)) if (x[4] < 0.0) == (y[4] < 0.0) => {
            let mut da = y[2] - x[2];
            while da > std::f64::consts::PI {
                da -= std::f64::consts::TAU;
            }
            while da < -std::f64::consts::PI {
                da += std::f64::consts::TAU;
            }
            let (cx, cy) = (lerp(x[0], y[0], t), lerp(x[1], y[1], t));
            let ang = x[2] + da * t;
            let (sx, sy) = (lerp(x[3], y[3], t), lerp(x[4], y[4], t));
            Affine::translate((cx, cy)) * Affine::rotate(ang) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-size.0 / 2.0, -size.1 / 2.0))
        }
        _ => {
            if t < 0.5 {
                a
            } else {
                b
            }
        }
    }
}

fn lerp_paints(from: &[Paint], to: &mut [Paint], t: f64) {
    for (f, p) in from.iter().zip(to.iter_mut()) {
        if let (PaintKind::Solid { color: fc }, PaintKind::Solid { color: tc }) = (&f.kind, &mut p.kind) {
            *tc = fc.lerp(*tc, t.clamp(0.0, 1.0) as f32);
            p.opacity = lerp(f64::from(f.opacity), f64::from(p.opacity), t.clamp(0.0, 1.0)) as f32;
        }
    }
}

/// The document as it looks at eased progress `t` of a smart-animate transition from
/// `from_root` (in `from`) to `to_root` (in `to`): matched layers tween position, size, rotation,
/// opacity, corner radius and solid fill colours; layers only in the destination fade in and
/// layers only in the source are copied in and fade out. Render `to_root` of the result.
pub fn smart_interpolate(from: &Document, from_root: NodeId, to: &Document, to_root: NodeId, t: f64) -> Document {
    let mut out = to.clone();
    // Springs and back easings go past 1 before they settle: matched layers overshoot too.
    if !t.is_finite() || t == 1.0 {
        return out;
    }
    let t = t.max(-1.0);
    let fade = t.clamp(0.0, 1.0);
    let pairs = match_layers(from, from_root, to, to_root);
    let src_of: HashMap<NodeId, NodeId> = pairs.iter().map(|(f, d)| (*d, *f)).collect();
    let dst_of: HashMap<NodeId, NodeId> = pairs.iter().copied().collect();
    let fw_inv = from.world_transform(from_root).inverse();
    let tw_inv = to.world_transform(to_root).inverse();
    if let (Some(fr), Some(m)) = (from.node(from_root), out.node_mut(to_root)) {
        lerp_paints(&fr.fills, &mut m.fills, t);
    }
    let mut interp: HashMap<NodeId, Affine> = HashMap::new();
    interp.insert(to_root, Affine::IDENTITY);
    let mut faded: HashSet<NodeId> = HashSet::new();
    for id in to.subtree(to_root).into_iter().skip(1) {
        let Some(n) = to.node(id) else { continue };
        let parent = n.parent.unwrap_or(to_root);
        let p_interp = interp.get(&parent).copied().unwrap_or(Affine::IDENTITY);
        match src_of.get(&id).and_then(|s| from.node(*s).map(|sn| (*s, sn))) {
            Some((s, sn)) => {
                let rel_s = fw_inv * from.world_transform(s);
                let rel_d = tw_inv * to.world_transform(id);
                let (w, h) = (lerp(sn.width, n.width, t).max(0.0), lerp(sn.height, n.height, t).max(0.0));
                let xf = lerp_transform(rel_s, (sn.width, sn.height), rel_d, (n.width, n.height), (w, h), t);
                interp.insert(id, xf);
                if let Some(m) = out.node_mut(id) {
                    m.width = w;
                    m.height = h;
                    m.transform = p_interp.inverse() * xf;
                    m.opacity = lerp(f64::from(sn.opacity), f64::from(n.opacity), fade) as f32;
                    for (r, (a, b)) in m.corners.radii.iter_mut().zip(sn.corners.radii.iter().zip(n.corners.radii.iter())) {
                        *r = lerp(*a, *b, t).max(0.0);
                    }
                    lerp_paints(&sn.fills, &mut m.fills, t);
                    lerp_paints(&sn.strokes, &mut m.strokes, t);
                }
            }
            None => {
                interp.insert(id, p_interp * n.transform);
                if !faded.contains(&parent)
                    && let Some(m) = out.node_mut(id)
                {
                    m.opacity *= fade as f32;
                }
                faded.insert(id);
            }
        }
    }
    // Layers only in the source: copy the top-most ones in, fading out.
    let tops: Vec<NodeId> = from
        .subtree(from_root)
        .into_iter()
        .skip(1)
        .filter(|id| !dst_of.contains_key(id))
        .filter(|id| from.node(*id).and_then(|n| n.parent).is_some_and(|p| p == from_root || dst_of.contains_key(&p)))
        .collect();
    for id in tops {
        let Some(n) = from.node(id) else { continue };
        let pf = n.parent.unwrap_or(from_root);
        let matched_parent = dst_of.get(&pf).copied().filter(|p| out.node(*p).is_some_and(|pn| pn.kind.is_container()));
        let (parent, local) = match matched_parent {
            Some(p) if pf != from_root => (p, n.transform),
            _ => (to_root, fw_inv * from.world_transform(id)),
        };
        let nodes: Vec<Node> = from.subtree(id).into_iter().filter_map(|i| from.node(i).cloned()).collect();
        let mut map = BTreeMap::new();
        for nn in &nodes {
            map.insert(nn.id, out.alloc_id());
        }
        let index = from.index_in_parent(id);
        if let Ok(new) = out.paste_nodes(&nodes, &map, id, parent, index)
            && let Some(m) = out.node_mut(new)
        {
            m.transform = local;
            m.opacity *= (1.0 - fade) as f32;
        }
    }
    out
}

/// Where the outgoing and incoming frames are drawn at eased progress `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePlacement {
    /// Offsets as fractions of each frame's size.
    pub from_off: (f32, f32),
    pub from_alpha: f32,
    pub to_off: (f32, f32),
    pub to_alpha: f32,
    pub from_on_top: bool,
}

/// The unit vector a direction moves along (screen space, y down).
pub fn direction_vec(d: Direction) -> (f32, f32) {
    match d {
        Direction::Left => (-1.0, 0.0),
        Direction::Right => (1.0, 0.0),
        Direction::Top => (0.0, -1.0),
        Direction::Bottom => (0.0, 1.0),
    }
}

/// Frame offsets/opacities for a transition. Directions name where things move: "Move in, left"
/// brings the new frame in from the right edge moving left.
pub fn transition_frames(tr: Transition, t: f32) -> FramePlacement {
    let mul = |(x, y): (f32, f32), k: f32| (x * k, y * k);
    let base = FramePlacement { from_off: (0.0, 0.0), from_alpha: 1.0, to_off: (0.0, 0.0), to_alpha: 1.0, from_on_top: false };
    match tr {
        Transition::Instant => FramePlacement { from_alpha: 0.0, ..base },
        Transition::Dissolve | Transition::SmartAnimate => FramePlacement { to_alpha: t.clamp(0.0, 1.0), ..base },
        Transition::MoveIn { direction } => FramePlacement { to_off: mul(direction_vec(direction), -(1.0 - t)), ..base },
        Transition::MoveOut { direction } => FramePlacement { from_off: mul(direction_vec(direction), t), from_on_top: true, ..base },
        Transition::Push { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, t), to_off: mul(d, -(1.0 - t)), ..base }
        }
        Transition::SlideIn { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, 0.3 * t), to_off: mul(d, -(1.0 - t)), ..base }
        }
        Transition::SlideOut { direction } => {
            let d = direction_vec(direction);
            FramePlacement { from_off: mul(d, t), to_off: mul(d, -0.3 * (1.0 - t)), from_on_top: true, ..base }
        }
    }
}

/// Screen rect of an overlay of `size` over the frame drawn at `base`. A manual one sits `offset`
/// (screen points) from the top-left of `anchor`, the layer that opened it; without an offset,
/// right below it.
pub fn overlay_rect(pos: OverlayPosition, base: Rect, size: Vec2, anchor: Option<Rect>, offset: Option<Vec2>) -> Rect {
    let cx = base.center().x - size.x / 2.0;
    let cy = base.center().y - size.y / 2.0;
    let (x, y) = match pos {
        OverlayPosition::Center => (cx, cy),
        OverlayPosition::TopLeft => (base.min.x, base.min.y),
        OverlayPosition::TopCenter => (cx, base.min.y),
        OverlayPosition::TopRight => (base.max.x - size.x, base.min.y),
        OverlayPosition::BottomLeft => (base.min.x, base.max.y - size.y),
        OverlayPosition::BottomCenter => (cx, base.max.y - size.y),
        OverlayPosition::BottomRight => (base.max.x - size.x, base.max.y - size.y),
        OverlayPosition::Manual => match (anchor, offset) {
            (Some(a), Some(o)) => (a.min.x + o.x, a.min.y + o.y),
            (Some(a), None) => (a.min.x, a.max.y),
            (None, _) => (cx, cy),
        },
    };
    Rect::from_min_size(pos2(x, y), size)
}
