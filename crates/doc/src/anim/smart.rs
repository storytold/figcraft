//! Smart Animate (best effort): pairs layers of two frames by name and path, then interpolates
//! transform, size, opacity, corner radii, fills and strokes. Layers without a partner fade in or
//! out. Ported from desigma's `proto/smart.rs` and adapted to FigCraft's document model
//! (`width`/`height`, [`Corners`], [`StrokeProps`], kurbo transforms).

use std::collections::{HashMap, HashSet};
use figcraft_geom::Affine;
use crate::color::{Color, ColorStop, Paint, PaintKind};
use crate::{Document, Node, NodeId, NodeKind};

/// A separator no layer name uses, so name paths are unambiguous.
const SEP: char = '\u{1}';

/// `(path of names from the root, id)` for every descendant (not the root itself).
fn descendants(doc: &Document, root: NodeId) -> Vec<(String, NodeId)> {
    let mut out = Vec::new();
    fn rec(doc: &Document, id: NodeId, path: &str, out: &mut Vec<(String, NodeId)>) {
        let Some(n) = doc.node(id) else { return };
        for &c in &n.children {
            let Some(k) = doc.node(c) else { continue };
            let p = format!("{path}{SEP}{}", k.name);
            out.push((p.clone(), c));
            rec(doc, c, &p, out);
        }
    }
    rec(doc, root, "", &mut out);
    out
}

fn kind_eq(a: NodeId, b: NodeId, from: &Document, to: &Document) -> bool {
    match (from.node(a), to.node(b)) {
        (Some(x), Some(y)) => std::mem::discriminant(&x.kind) == std::mem::discriminant(&y.kind),
        _ => false,
    }
}

/// Pairs `(to, from)` of layers considered the same.
pub fn match_layers(from_doc: &Document, from_root: NodeId, to_doc: &Document, to_root: NodeId) -> Vec<(NodeId, NodeId)> {
    let fd = descendants(from_doc, from_root);
    let td = descendants(to_doc, to_root);
    // Pass 1: identical path; the n-th sibling with a repeated name pairs with the n-th.
    let mut by_path: HashMap<(String, usize), NodeId> = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (p, id) in &fd {
        let n = seen.entry(p.clone()).or_insert(0);
        by_path.insert((p.clone(), *n), *id);
        *n += 1;
    }
    let mut pairs = Vec::new();
    let mut used_from: HashSet<NodeId> = HashSet::new();
    let mut matched_to: HashSet<NodeId> = HashSet::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (p, id) in &td {
        let n = seen.entry(p.clone()).or_insert(0);
        if let Some(f) = by_path.get(&(p.clone(), *n)).copied().filter(|f| kind_eq(*f, *id, from_doc, to_doc)) {
            pairs.push((*id, f));
            used_from.insert(f);
            matched_to.insert(*id);
        }
        *n += 1;
    }
    // Pass 2: layers that moved to another parent pair by a name unique among the leftovers.
    let name = |doc: &Document, id: NodeId| doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
    let mut left_from: HashMap<String, Vec<NodeId>> = HashMap::new();
    for (_, id) in fd.iter().filter(|(_, id)| !used_from.contains(id)) {
        left_from.entry(name(from_doc, *id)).or_default().push(*id);
    }
    let mut left_to: HashMap<String, Vec<NodeId>> = HashMap::new();
    for (_, id) in td.iter().filter(|(_, id)| !matched_to.contains(id)) {
        left_to.entry(name(to_doc, *id)).or_default().push(*id);
    }
    for (nm, tos) in &left_to {
        if let (Some(froms), [t]) = (left_from.get(nm), tos.as_slice())
            && let [f] = froms.as_slice()
            && kind_eq(*f, *t, from_doc, to_doc)
        {
            pairs.push((*t, *f));
        }
    }
    pairs
}

struct Pair {
    id: NodeId,
    from: Node,
    to: Node,
}

struct Fade {
    id: NodeId,
    opacity: f32,
    fade_in: bool,
}

/// A Smart Animate plan: the destination document with matched layers ready to morph.
pub struct SmartPlan {
    base: Document,
    root: NodeId,
    pairs: Vec<Pair>,
    fades: Vec<Fade>,
}

impl SmartPlan {
    /// None when no layer pairs up (nothing to morph: use a dissolve).
    pub fn build(from_doc: &Document, from_root: NodeId, to_doc: &Document, to_root: NodeId) -> Option<SmartPlan> {
        let matched = match_layers(from_doc, from_root, to_doc, to_root);
        if matched.is_empty() {
            return None;
        }
        let mut base = to_doc.clone();
        let root_from = from_doc.node(from_root)?.clone();
        let root_to = to_doc.node(to_root)?.clone();
        let mut pairs = vec![Pair { id: to_root, from: root_from, to: root_to }];
        let to_of: HashMap<NodeId, NodeId> = matched.iter().map(|(t, f)| (*f, *t)).collect();
        for (t, f) in &matched {
            if let (Some(fnode), Some(tnode)) = (from_doc.node(*f), to_doc.node(*t)) {
                pairs.push(Pair { id: *t, from: fnode.clone(), to: tnode.clone() });
            }
        }
        let mut fades = Vec::new();
        // Layers only in the destination fade in (outermost ones; children ride along).
        for (_, id) in descendants(to_doc, to_root) {
            if matched.iter().any(|(t, _)| *t == id) {
                continue;
            }
            let parent_unmatched = to_doc
                .node(id)
                .and_then(|n| n.parent)
                .is_some_and(|p| p != to_root && !matched.iter().any(|(t, _)| *t == p));
            if !parent_unmatched {
                let op = to_doc.node(id).map(|n| n.opacity).unwrap_or(1.0);
                fades.push(Fade { id, opacity: op, fade_in: true });
            }
        }
        // Layers only in the source are copied in and fade out.
        let from_matched: HashSet<NodeId> = matched.iter().map(|(_, f)| *f).collect();
        for (_, id) in descendants(from_doc, from_root) {
            if from_matched.contains(&id) {
                continue;
            }
            let Some(fnode) = from_doc.node(id) else { continue };
            let parent = fnode.parent.unwrap_or(from_root);
            if parent != from_root && !from_matched.contains(&parent) {
                continue;
            }
            let attach = if parent == from_root { to_root } else { *to_of.get(&parent).unwrap_or(&to_root) };
            let retransform = (parent != from_root && !to_of.contains_key(&parent))
                .then(|| {
                    let pw = base.world_transform(attach);
                    let nw = from_doc.world_transform(id);
                    pw.inverse() * nw
                });
            if let Some(new_id) = clone_into(from_doc, id, &mut base, attach, retransform) {
                fades.push(Fade { id: new_id, opacity: fnode.opacity, fade_in: false });
            }
        }
        Some(SmartPlan { base, root: to_root, pairs, fades })
    }

    /// The morphed frame at eased progress `t` (0 = source look, 1 = destination look).
    pub fn apply(&mut self, t: f64) -> (&Document, NodeId) {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 1.0 };
        for p in &self.pairs {
            let Some(n) = self.base.node_mut(p.id) else { continue };
            let (a, b) = (&p.from, &p.to);
            if p.id != self.root {
                n.transform = lerp_affine(&a.transform, &b.transform, t);
            }
            n.width = lerp(a.width, b.width, t);
            n.height = lerp(a.height, b.height, t);
            n.opacity = lerp(a.opacity as f64, b.opacity as f64, t).clamp(0.0, 1.0) as f32;
            for i in 0..4 {
                n.corners.radii[i] = lerp(a.corners.radii[i], b.corners.radii[i], t).max(0.0);
            }
            n.fills = lerp_paints(&a.fills, &b.fills, t);
            n.strokes = lerp_paints(&a.strokes, &b.strokes, t);
            n.stroke.weight = lerp(a.stroke.weight, b.stroke.weight, t).max(0.0);
            // Text: animate font size and per-text fills.
            if let (NodeKind::Text(x), NodeKind::Text(y), NodeKind::Text(m)) = (&a.kind, &b.kind, &mut n.kind) {
                m.style.font_size = lerp(x.style.font_size, y.style.font_size, t).max(1.0);
                if let (Some(xf), Some(yf)) = (&x.style.fills, &y.style.fills) {
                    m.style.fills = Some(lerp_paints(xf, yf, t));
                }
            }
        }
        for f in &self.fades {
            if let Some(n) = self.base.node_mut(f.id) {
                let k = if f.fade_in { t } else { 1.0 - t };
                n.opacity = ((f.opacity as f64) * k).clamp(0.0, 1.0) as f32;
            }
        }
        (&self.base, self.root)
    }

    #[cfg(test)]
    pub fn paired(&self) -> usize {
        self.pairs.len().saturating_sub(1)
    }
}

/// Copies `src` and its subtree into `dst` under `parent` with fresh ids, optionally setting the
/// node's transform to `retransform` (its screen placement in `dst`).
fn clone_into(src_doc: &Document, src: NodeId, dst: &mut Document, parent: NodeId, retransform: Option<Affine>) -> Option<NodeId> {
    fn rec(src_doc: &Document, src: NodeId, dst: &mut Document, parent: NodeId) -> Option<NodeId> {
        let mut n = src_doc.node(src)?.clone();
        let id = dst.alloc_id();
        let kids = n.children.clone();
        n.id = id;
        n.parent = Some(parent);
        n.children.clear();
        dst.insert(n, parent, None).ok()?;
        for k in kids {
            rec(src_doc, k, dst, id);
        }
        Some(id)
    }
    let id = rec(src_doc, src, dst, parent)?;
    if let Some(t) = retransform
        && let Some(n) = dst.node_mut(id)
    {
        n.transform = t;
    }
    Some(id)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    let c = |x: f64| x.clamp(0.0, 1.0);
    Color {
        r: c(lerp(a.r as f64, b.r as f64, t)) as f32,
        g: c(lerp(a.g as f64, b.g as f64, t)) as f32,
        b: c(lerp(a.b as f64, b.b as f64, t)) as f32,
        a: c(lerp(a.a as f64, b.a as f64, t)) as f32,
    }
}

/// Interpolates translation, rotation and scale (skew is dropped).
fn lerp_affine(a: &Affine, b: &Affine, t: f64) -> Affine {
    let dec = |m: &Affine| {
        let c = m.as_coeffs();
        let sx = (c[0] * c[0] + c[1] * c[1]).sqrt();
        let det = c[0] * c[3] - c[1] * c[2];
        let sy = if sx > 1e-9 { det / sx } else { 0.0 };
        (c[4], c[5], c[1].atan2(c[0]), sx, sy)
    };
    let (ax, ay, ar, asx, asy) = dec(a);
    let (bx, by, br, bsx, bsy) = dec(b);
    let mut dr = br - ar;
    while dr > std::f64::consts::PI {
        dr -= std::f64::consts::TAU;
    }
    while dr < -std::f64::consts::PI {
        dr += std::f64::consts::TAU;
    }
    let (r, sx, sy) = (ar + dr * t, lerp(asx, bsx, t), lerp(asy, bsy, t));
    let (sn, cs) = r.sin_cos();
    Affine::new([sx * cs, sx * sn, -sy * sn, sy * cs, lerp(ax, bx, t), lerp(ay, by, t)])
}

fn lerp_paints(a: &[Paint], b: &[Paint], t: f64) -> Vec<Paint> {
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    if a.is_empty() {
        return b.iter().map(|p| Paint { opacity: (p.opacity as f64 * t.clamp(0.0, 1.0)) as f32, ..p.clone() }).collect();
    }
    if b.is_empty() {
        return a.iter().map(|p| Paint { opacity: (p.opacity as f64 * (1.0 - t).clamp(0.0, 1.0)) as f32, ..p.clone() }).collect();
    }
    if a.len() == b.len() {
        let mut out = Vec::new();
        for (x, y) in a.iter().zip(b) {
            match (&x.kind, &y.kind) {
                (PaintKind::Solid { color: c1 }, PaintKind::Solid { color: c2 }) => out.push(Paint {
                    kind: PaintKind::Solid { color: lerp_color(*c1, *c2, t) },
                    opacity: lerp(x.opacity as f64, y.opacity as f64, t).clamp(0.0, 1.0) as f32,
                    visible: y.visible,
                    blend: y.blend,
                    color_alpha: y.color_alpha,
                }),
                (PaintKind::Gradient(g1), PaintKind::Gradient(g2)) if g1.stops.len() == g2.stops.len() => {
                    let stops = g1
                        .stops
                        .iter()
                        .zip(&g2.stops)
                        .map(|(p, q)| ColorStop { position: lerp(p.position as f64, q.position as f64, t) as f32, color: lerp_color(p.color, q.color, t) })
                        .collect();
                    let mut g = g2.clone();
                    g.stops = stops;
                    out.push(Paint {
                        kind: PaintKind::Gradient(g),
                        opacity: lerp(x.opacity as f64, y.opacity as f64, t).clamp(0.0, 1.0) as f32,
                        visible: y.visible,
                        blend: y.blend,
                        color_alpha: y.color_alpha,
                    });
                }
                _ => out.push(if t < 0.5 { x.clone() } else { y.clone() }),
            }
        }
        return out;
    }
    if t < 0.5 {
        a.to_vec()
    } else {
        b.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::proto::Transition;

    fn two_frames() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 400.0, 800.0).unwrap();
        let b = d.create(page, NodeKind::Frame, "B", 500.0, 0.0, 400.0, 800.0).unwrap();
        (d, a, b)
    }

    fn rect(d: &mut Document, parent: NodeId, name: &str, x: f64, y: f64, w: f64, h: f64, color: Color) -> NodeId {
        let id = d.create(parent, NodeKind::Rectangle, name, x, y, w, h).unwrap();
        if let Some(n) = d.node_mut(id) {
            n.fills = vec![Paint::solid(color)];
        }
        id
    }

    const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    #[test]
    fn layers_pair_by_path_and_kind() {
        let (mut d, a, b) = two_frames();
        let fa = rect(&mut d, a, "Card", 0.0, 0.0, 10.0, 10.0, BLACK);
        let fb = rect(&mut d, b, "Card", 50.0, 50.0, 20.0, 20.0, WHITE);
        let _only_a = rect(&mut d, a, "Gone", 0.0, 0.0, 5.0, 5.0, BLACK);
        let _only_b = rect(&mut d, b, "New", 0.0, 0.0, 5.0, 5.0, BLACK);
        let ida = d.create(a, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Dot", 0.0, 0.0, 5.0, 5.0).unwrap();
        let _ = ida;
        rect(&mut d, b, "Dot", 0.0, 0.0, 5.0, 5.0, BLACK);
        let m = match_layers(&d, a, &d, b);
        assert_eq!(m, vec![(fb, fa)]);
    }

    #[test]
    fn repeated_names_pair_in_order() {
        let (mut d, a, b) = two_frames();
        let a1 = rect(&mut d, a, "Item", 0.0, 0.0, 1.0, 1.0, BLACK);
        let a2 = rect(&mut d, a, "Item", 0.0, 10.0, 1.0, 1.0, BLACK);
        let b1 = rect(&mut d, b, "Item", 0.0, 0.0, 1.0, 1.0, BLACK);
        let b2 = rect(&mut d, b, "Item", 0.0, 10.0, 1.0, 1.0, BLACK);
        let m = match_layers(&d, a, &d, b);
        assert!(m.contains(&(b1, a1)) && m.contains(&(b2, a2)));
    }

    #[test]
    fn reparented_unique_names_still_pair() {
        let (mut d, a, b) = two_frames();
        let fa = rect(&mut d, a, "Avatar", 0.0, 0.0, 1.0, 1.0, BLACK);
        let gid = d.create(b, NodeKind::Frame, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
        let fb = rect(&mut d, gid, "Avatar", 0.0, 0.0, 1.0, 1.0, BLACK);
        assert_eq!(match_layers(&d, a, &d, b), vec![(fb, fa)]);
    }

    #[test]
    fn nothing_in_common_means_no_plan() {
        let (mut d, a, b) = two_frames();
        rect(&mut d, a, "X", 0.0, 0.0, 1.0, 1.0, BLACK);
        rect(&mut d, b, "Y", 0.0, 0.0, 1.0, 1.0, BLACK);
        assert!(SmartPlan::build(&d, a, &d, b).is_none());
    }

    #[test]
    fn interpolates_geometry_opacity_and_color() {
        let (mut d, a, b) = two_frames();
        rect(&mut d, a, "Card", 0.0, 0.0, 100.0, 40.0, BLACK);
        let fb = rect(&mut d, b, "Card", 100.0, 60.0, 200.0, 80.0, WHITE);
        if let Some(n) = d.node_mut(fb) {
            n.opacity = 0.5;
        }
        let mut plan = SmartPlan::build(&d, a, &d, b).unwrap();
        assert_eq!(plan.paired(), 1);
        let (doc, root) = plan.apply(0.5);
        let card = doc.node(doc.node(root).unwrap().children[0]).unwrap();
        let c = card.transform.as_coeffs();
        assert!((c[4] - 50.0).abs() < 1e-6 && (c[5] - 30.0).abs() < 1e-6, "translation at mid");
        assert!((card.width - 150.0).abs() < 1e-6);
        assert!((card.height - 60.0).abs() < 1e-6);
        assert!((card.opacity - 0.75).abs() < 1e-5);
        assert!(matches!(card.fills[0].kind, PaintKind::Solid { color: col } if (col.r as f64 - 0.5).abs() < 1e-4));
        let (doc, root) = plan.apply(0.0);
        let card = doc.node(doc.node(root).unwrap().children[0]).unwrap();
        assert!((card.width - 100.0).abs() < 1e-6);
        let (doc, root) = plan.apply(1.0);
        let card = doc.node(doc.node(root).unwrap().children[0]).unwrap();
        assert!((card.width - 200.0).abs() < 1e-6);
        assert!((card.opacity - 0.5).abs() < 1e-5);
    }

    #[test]
    fn unmatched_layers_cross_fade() {
        let (mut d, a, b) = two_frames();
        rect(&mut d, a, "Keep", 0.0, 0.0, 10.0, 10.0, BLACK);
        rect(&mut d, b, "Keep", 0.0, 0.0, 10.0, 10.0, BLACK);
        rect(&mut d, a, "Gone", 5.0, 5.0, 10.0, 10.0, BLACK);
        rect(&mut d, b, "New", 7.0, 7.0, 10.0, 10.0, BLACK);
        let mut plan = SmartPlan::build(&d, a, &d, b).unwrap();
        let (doc, root) = plan.apply(0.25);
        let kids: Vec<&Node> = doc.node(root).unwrap().children.iter().map(|c| doc.node(*c).unwrap()).collect();
        let by = |n: &str| kids.iter().find(|k| k.name == n).unwrap();
        assert!((by("New").opacity - 0.25).abs() < 1e-5, "fades in");
        assert!((by("Gone").opacity - 0.75).abs() < 1e-5, "fades out");
        assert_eq!(by("Gone").transform.as_coeffs()[4], 5.0, "keeps its place");
        assert_eq!(by("Keep").opacity, 1.0);
    }

    #[test]
    fn affine_lerp_interpolates_rotation_by_angle() {
        let a = Affine::translate((0.0, 0.0));
        let b = Affine::translate((10.0, 0.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let m = lerp_affine(&a, &b, 0.5);
        let c = m.as_coeffs();
        let rot = c[1].atan2(c[0]);
        assert!((rot - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
        assert!(((c[0] * c[0] + c[1] * c[1]).sqrt() - 1.0).abs() < 1e-9, "no shrinking mid-rotation");
        assert!((c[4] - 5.0).abs() < 1e-9);
        // The long way round is avoided.
        let c2 = Affine::rotate(3.0);
        let d2 = Affine::rotate(-3.0);
        let mid = lerp_affine(&c2, &d2, 0.5);
        let mc = mid.as_coeffs();
        let mrot = mc[1].atan2(mc[0]);
        assert!(mrot.abs() > 3.0 || (mrot.abs() - std::f64::consts::PI).abs() < 0.2);
    }

    #[test]
    fn paint_lerp_handles_added_and_removed_fills() {
        let solid = vec![Paint::solid(BLACK)];
        let faded_in = lerp_paints(&[], &solid, 0.25);
        assert!((faded_in[0].opacity as f64 - 0.25).abs() < 1e-6);
        let faded_out = lerp_paints(&solid, &[], 0.25);
        assert!((faded_out[0].opacity as f64 - 0.75).abs() < 1e-6);
        assert!(lerp_paints(&[], &[], 0.5).is_empty());
    }

    #[test]
    fn smart_plan_is_subject_to_transition() {
        // Smart Animate is one of the transitions FigCraft already models.
        let _ = Transition::SmartAnimate;
    }
}
