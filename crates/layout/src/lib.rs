//! FigCraft layout.
//!
//! - [`layout_page`] / [`layout_frame`]: Figma-style auto layout. Children of an auto layout frame
//!   are stacked horizontally or vertically (optionally wrapping, optionally aligned on their first
//!   text baseline) or placed on a grid with fixed / hug / `fr` tracks and cells that span, with
//!   gap, padding and alignment; each child is Fixed, Hug (its own content) or Fill (shares the
//!   free space), within its min/max size. A Hug container sizes to its children. Absolute
//!   children (ignoring auto layout) are not placed; they follow their constraints instead.
//! - [`coherent_sizing`]: Fill and Hug only where they can apply (Fill in an auto layout flow on an
//!   axis the parent doesn't hug; Hug on auto layout frames and text), else Fixed.
//! - [`apply_constraints`]: children of a regular frame follow their constraints when it resizes.
//! - [`grid_lines`]: the column/row rectangles of a layout grid, for drawing and snapping.
//!
//! Text measurement is injected ([`Measure`]) so this crate doesn't depend on fonts.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use figcraft_doc::{
    AutoLayout, CellAlign, ContentAlign, CrossAlign, Document, GridAlign, GridKind, HConstraint, LayoutGrid, LayoutMode, MainAlign, NodeId, NodeKind,
    Sizing, StrokeAlign, TextAutoResize, VConstraint,
};
use figcraft_geom::{Affine, Point, Rect};

/// Most layout calls nested in one another (about one per level of layers): deeper layers don't
/// follow a resize, so a deeply nested document can't run the stack out (#210).
pub const MAX_NESTING: usize = 256;

thread_local! {
    static NESTING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One level of nested layout calls, released when dropped.
struct Nested;

impl Nested {
    /// Enter one more level; `None` when [`MAX_NESTING`] are in progress.
    fn enter() -> Option<Self> {
        NESTING.with(|n| (n.get() < MAX_NESTING).then(|| n.set(n.get() + 1)).map(|()| Self))
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        NESTING.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// Measures text for a given width (auto-height text inside a Fill container).
pub trait Measure {
    /// The height a text node needs at `width`.
    fn text_height(&self, doc: &Document, id: NodeId, width: f64) -> Option<f64>;
    /// Natural (auto width, unwrapped) size of a text node, whatever its resize mode.
    fn text_size(&self, doc: &Document, id: NodeId) -> Option<(f64, f64)>;
    /// Distance from the top of a text node's box to its first line's baseline (for baseline
    /// alignment). `None` when unknown (non-text, no fonts); the item then aligns to the top.
    fn text_baseline(&self, _doc: &Document, _id: NodeId) -> Option<f64> {
        None
    }
}

/// A measurer that never changes text sizes.
pub struct NoMeasure;

impl Measure for NoMeasure {
    fn text_height(&self, _: &Document, _: NodeId, _: f64) -> Option<f64> {
        None
    }
    fn text_size(&self, _: &Document, _: NodeId) -> Option<(f64, f64)> {
        None
    }
}

/// Clamp a size to `[min, max]` (non-finite or negative sizes become 0; when min > max, min wins).
pub fn clamp_size(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let mut v = if v.is_finite() { v.max(0.0) } else { 0.0 };
    if let Some(m) = max.filter(|m| m.is_finite()) {
        v = v.min(m);
    }
    if let Some(m) = min.filter(|m| m.is_finite()) {
        v = v.max(m);
    }
    v
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// The child's box size as seen by the parent (rotation → bounding box).
fn outer_size(doc: &Document, id: NodeId) -> (f64, f64) {
    let Some(n) = doc.node(id) else { return (0.0, 0.0) };
    let b = figcraft_geom::transform_rect_bbox(linear(n.transform), n.local_rect());
    (b.width(), b.height())
}

fn linear(a: Affine) -> Affine {
    let c = a.as_coeffs();
    Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
}

/// How much a layer's own width and height scale (no skew: it keeps its rotation and flip) when
/// its parent's space scales by `kx`, `ky`: how much each of its axes (`xf`'s linear part)
/// stretches. A layer turned 90° swaps the two; one turned 45° gets a mix.
pub fn axis_factors(xf: Affine, kx: f64, ky: f64) -> (f64, f64) {
    let [a, b, c, d, _, _] = xf.as_coeffs();
    let along = |x: f64, y: f64, fallback: f64| {
        let len = x.hypot(y);
        let k = (kx * x).hypot(ky * y) / len;
        if len > 1e-12 && k.is_finite() { k } else { fallback }
    };
    (along(a, b, kx), along(c, d, ky))
}

/// Place `id` so its (rotated) bounding box's top-left is at `(x, y)` in the parent.
fn place(doc: &mut Document, id: NodeId, x: f64, y: f64) {
    let Some(n) = doc.node(id) else { return };
    let lin = linear(n.transform);
    let b = figcraft_geom::transform_rect_bbox(lin, n.local_rect());
    let t = Affine::translate((x - b.x0, y - b.y0)) * lin;
    if let Some(n) = doc.node_mut(id) {
        n.transform = t;
    }
}

/// Resize a node's own box (vectors scale their network to stay inside it).
pub fn set_size(doc: &mut Document, id: NodeId, w: f64, h: f64) {
    let Some(n) = doc.node_mut(id) else { return };
    let (w, h) = (if w.is_finite() { w.max(0.0) } else { n.width }, if h.is_finite() { h.max(0.0) } else { n.height });
    let (ow, oh) = (n.width, n.height);
    if (ow - w).abs() < 1e-9 && (oh - h).abs() < 1e-9 {
        return;
    }
    let sx = if ow > 1e-9 { w / ow } else { 1.0 };
    let sy = if oh > 1e-9 { h / oh } else { 1.0 };
    match &mut n.kind {
        NodeKind::Vector { network } => network.scale(sx, sy),
        NodeKind::Text(figcraft_doc::TextData { path: Some(tp), .. }) => tp.scale(sx, sy),
        _ => {}
    }
    n.width = w;
    n.height = h;
}

/// Lay out every auto layout frame on a page (children before parents).
pub fn layout_page(doc: &mut Document, page: NodeId, m: &dyn Measure) {
    let order = doc.subtree(page);
    for id in order.into_iter().rev() {
        if doc.node(id).is_some_and(|n| n.layout.is_some() && n.kind.is_frame_like()) {
            layout_frame(doc, id, m);
        }
    }
}

/// Lay out the ancestors of `id` that are auto layout frames (innermost first), and `id` itself.
pub fn layout_up(doc: &mut Document, id: NodeId, m: &dyn Measure) {
    let mut chain = vec![id];
    chain.extend(doc.ancestors(id));
    for c in chain {
        if doc.node(c).is_some_and(|n| n.layout.is_some() && n.kind.is_frame_like()) {
            layout_frame(doc, c, m);
        }
    }
}

/// The children an auto layout frame places: visible and not ignoring auto layout.
pub fn flow_children(doc: &Document, frame: NodeId) -> Vec<NodeId> {
    let Some(f) = doc.node(frame) else { return Vec::new() };
    f.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|n| n.visible && !n.child_layout.absolute)).collect()
}

/// Make a layer's sizing coherent with where it sits (after an edit). Fill needs room to fill: a
/// layer that isn't in an auto layout flow (no auto layout parent, or ignoring auto layout), or
/// whose parent hugs that axis, gets Fixed at its current size instead. Hug needs auto layout or
/// text; otherwise it is Fixed too. A text hugs what its resize mode grows. The same applies to the
/// layer's own children, so a frame that starts to hug (or loses auto layout) fixes the children
/// that filled it. Returns whether anything changed.
pub fn coherent_sizing(doc: &mut Document, id: NodeId) -> bool {
    let kids = doc.node(id).map(|n| n.children.clone()).unwrap_or_default();
    let mut changed = coherent_own_sizing(doc, id);
    for c in kids {
        changed |= coherent_own_sizing(doc, c);
    }
    changed
}

fn coherent_own_sizing(doc: &mut Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let flow_parent = n.parent.and_then(|p| doc.node(p)).filter(|p| p.layout.is_some() && p.kind.is_frame_like() && !n.child_layout.absolute);
    let parent_hugs =
        |horizontal: bool| flow_parent.map(|p| if horizontal { p.child_layout.sizing_h } else { p.child_layout.sizing_v } == Sizing::Hug);
    let can_hug = (n.layout.is_some() && n.kind.is_frame_like()) || matches!(n.kind, NodeKind::Text(_));
    let coherent = |s: Sizing, horizontal: bool| match s {
        Sizing::Fill if parent_hugs(horizontal) != Some(false) => Sizing::Fixed,
        Sizing::Hug if !can_hug => Sizing::Fixed,
        s => s,
    };
    let (mut h, mut v) = (coherent(n.child_layout.sizing_h, true), coherent(n.child_layout.sizing_v, false));
    // A text's resize mode is its sizing: auto width hugs both axes, auto height hugs its height,
    // a fixed box hugs neither.
    if let NodeKind::Text(t) = &n.kind {
        let hug_if_fixed = |s: Sizing| if s == Sizing::Fixed { Sizing::Hug } else { s };
        let fixed_if_hug = |s: Sizing| if s == Sizing::Hug { Sizing::Fixed } else { s };
        (h, v) = match t.auto_resize {
            TextAutoResize::WidthAndHeight => (hug_if_fixed(h), hug_if_fixed(v)),
            TextAutoResize::Height => (h, hug_if_fixed(v)),
            TextAutoResize::None | TextAutoResize::Truncate => (fixed_if_hug(h), fixed_if_hug(v)),
        };
    }
    if (h, v) == (n.child_layout.sizing_h, n.child_layout.sizing_v) {
        return false;
    }
    let Some(n) = doc.node_mut(id) else { return false };
    n.child_layout.sizing_h = h;
    n.child_layout.sizing_v = v;
    true
}

/// Padding `(top, right, bottom, left)` of an auto layout frame, sanitised, plus the frame's own
/// inside stroke (per side) when strokes are included in the layout; center and outside strokes
/// never take up space.
fn insets(f: &figcraft_doc::Node, al: &AutoLayout) -> (f64, f64, f64, f64) {
    let included = al.strokes_included && f.has_visible_stroke() && f.stroke.align == StrokeAlign::Inside;
    let stroke = if included { f.stroke.sides.unwrap_or([f.stroke.weight; 4]) } else { [0.0; 4] };
    let [pt, pr, pb, pl] = al.padding.map(|p| finite_or_zero(p).max(0.0));
    let [st, sr, sb, sl] = stroke.map(|w| finite_or_zero(w).max(0.0));
    (pt + st, pr + sr, pb + sb, pl + sl)
}

/// Where the first child starts and the step between children along the main axis, from the
/// free space (`inner` minus the children's total size). Auto gaps share the free space and never
/// go below 0; a single child of "space between" sits at the start.
fn main_offsets(align: MainAlign, inner: f64, total: f64, n: usize, gap: f64) -> (f64, f64) {
    let gaps = gap * n.saturating_sub(1) as f64;
    let free = inner - total;
    let k = n.max(1) as f64;
    match align {
        MainAlign::Min => (0.0, gap),
        MainAlign::Center => ((free - gaps) / 2.0, gap),
        MainAlign::Max => (free - gaps, gap),
        MainAlign::SpaceBetween => (0.0, if n > 1 { (free / (n - 1) as f64).max(0.0) } else { 0.0 }),
        // Gaps are twice the edge space: n gaps of 2e fill the free space.
        MainAlign::SpaceAround => {
            let g = (free / k).max(0.0);
            (g / 2.0, g)
        }
        MainAlign::SpaceEvenly => {
            let g = (free / (k + 1.0)).max(0.0);
            (g, g)
        }
    }
}

/// Text that hugs its width: refresh its natural size, honouring min/max width by wrapping (a
/// hugging text wider than its max width wraps at the max; narrower than its min width gets the min).
fn fit_hug_text(doc: &mut Document, id: NodeId, m: &dyn Measure) {
    let Some(n) = doc.node(id) else { return };
    let NodeKind::Text(t) = &n.kind else { return };
    let cl = &n.child_layout;
    let auto_width = t.auto_resize == TextAutoResize::WidthAndHeight;
    // A hugging text that wrapped at its max width earlier is auto height now.
    let wrapped_hug = cl.sizing_h == Sizing::Hug && t.auto_resize == TextAutoResize::Height;
    if !auto_width && !wrapped_hug {
        return;
    }
    let (min_w, max_w) = (cl.min_w, cl.max_w);
    let Some((nw, nh)) = m.text_size(doc, id) else { return };
    let w = clamp_size(nw, min_w, max_w);
    let limited = (w - nw).abs() > 1e-6;
    let h = if limited { m.text_height(doc, id, w).unwrap_or(nh) } else { nh };
    if let Some(n) = doc.node_mut(id)
        && let NodeKind::Text(t) = &mut n.kind
    {
        let mode = if limited { TextAutoResize::Height } else { TextAutoResize::WidthAndHeight };
        if t.auto_resize != mode {
            t.auto_resize = mode;
        }
    }
    set_size(doc, id, w, h);
}

/// Clamp an in-flow child's own size to its min/max (fill axes are clamped when distributed).
fn enforce_limits(doc: &mut Document, id: NodeId, m: &dyn Measure) {
    let Some(n) = doc.node(id) else { return };
    let cl = &n.child_layout;
    if cl.min_w.is_none() && cl.max_w.is_none() && cl.min_h.is_none() && cl.max_h.is_none() {
        return;
    }
    let (w, h) = (n.width, n.height);
    let (nw, nh) = (clamp_size(w, cl.min_w, cl.max_w), clamp_size(h, cl.min_h, cl.max_h));
    if (nw - w).abs() < 1e-9 && (nh - h).abs() < 1e-9 {
        return;
    }
    if matches!(n.kind, NodeKind::Text(_)) {
        // Keep the text's resize mode (hugging width was handled by `fit_hug_text`).
        set_size(doc, id, nw, nh);
    } else {
        resize_child(doc, id, nw, nh, m);
    }
}

/// Distance from the top of a child's box to its first text baseline: text measures it, an auto
/// layout frame uses its first placed child's baseline. `None` for other layers and rotated ones.
fn item_baseline(doc: &Document, id: NodeId, m: &dyn Measure, depth: usize) -> Option<f64> {
    let n = doc.node(id)?;
    let c = n.transform.as_coeffs();
    if c[1].abs() > 1e-9 || c[2].abs() > 1e-9 || c[0] < 0.0 || c[3] < 0.0 {
        return None;
    }
    match &n.kind {
        NodeKind::Text(_) => m.text_baseline(doc, id).filter(|b| b.is_finite()),
        _ if n.layout.is_some() && n.kind.is_frame_like() && depth < 32 => flow_children(doc, id).into_iter().find_map(|k| {
            let b = item_baseline(doc, k, m, depth + 1)?;
            let kn = doc.node(k)?;
            let top = figcraft_geom::transform_rect_bbox(kn.transform, kn.local_rect()).y0;
            Some(top + b)
        }),
        _ => None,
    }
}

struct Item {
    id: NodeId,
    main: f64,
    cross: f64,
    fill_main: bool,
    fill_cross: bool,
    min_main: Option<f64>,
    max_main: Option<f64>,
    min_cross: Option<f64>,
    max_cross: Option<f64>,
    /// Baseline alignment: distance from the item's top to its first baseline.
    baseline: Option<f64>,
    /// Baseline alignment: offset from the line's top (computed per line).
    base_off: f64,
}

/// Lay out one auto layout frame's children and size the frame if it hugs.
pub fn layout_frame(doc: &mut Document, frame: NodeId, m: &dyn Measure) {
    let Some(_nested) = Nested::enter() else { return };
    let Some(f) = doc.node(frame) else { return };
    let Some(al) = f.layout.clone() else { return };
    let horizontal = matches!(al.mode, LayoutMode::Horizontal);
    let (hug_w, hug_h) = (f.child_layout.sizing_h == Sizing::Hug, f.child_layout.sizing_v == Sizing::Hug);
    let (ow, oh) = (f.width, f.height);
    // The frame's own min/max apply whatever its sizing; it is never smaller than its padding.
    let fcl = f.child_layout.clone();
    let (pt, pr, pb, pl) = insets(f, &al);
    let (fw, fh) = (clamp_size(ow, fcl.min_w, fcl.max_w).max(pl + pr), clamp_size(oh, fcl.min_h, fcl.max_h).max(pt + pb));
    let kids = flow_children(doc, frame);

    // Text children refresh their natural size, then every child honours its min/max.
    for c in &kids {
        fit_hug_text(doc, *c, m);
        enforce_limits(doc, *c, m);
    }

    if matches!(al.mode, LayoutMode::Grid) {
        layout_grid(doc, frame, &al, m);
        return;
    }

    let inner_main_fixed = if horizontal { fw - pl - pr } else { fh - pt - pb };
    let inner_cross_fixed = if horizontal { fh - pt - pb } else { fw - pl - pr };
    let hug_main = if horizontal { hug_w } else { hug_h };
    let hug_cross = if horizontal { hug_h } else { hug_w };
    let baseline = horizontal && al.cross_align == CrossAlign::Baseline;

    let mut items: Vec<Item> = Vec::with_capacity(kids.len());
    for c in &kids {
        let Some(n) = doc.node(*c) else { continue };
        let (w, h) = outer_size(doc, *c);
        let cl = &n.child_layout;
        let (sm, sc) = if horizontal { (cl.sizing_h, cl.sizing_v) } else { (cl.sizing_v, cl.sizing_h) };
        let (min_main, max_main, min_cross, max_cross) =
            if horizontal { (cl.min_w, cl.max_w, cl.min_h, cl.max_h) } else { (cl.min_h, cl.max_h, cl.min_w, cl.max_w) };
        items.push(Item {
            id: *c,
            main: if horizontal { w } else { h },
            cross: if horizontal { h } else { w },
            // Fill needs room to fill: on an axis the frame hugs it reads as fixed.
            fill_main: sm == Sizing::Fill && !hug_main,
            fill_cross: sc == Sizing::Fill && !hug_cross,
            min_main,
            max_main,
            min_cross,
            max_cross,
            baseline: None,
            base_off: 0.0,
        });
    }

    let gap = finite_or_zero(al.gap);
    let auto_gap = al.main_align.is_auto_gap();
    let wraps = al.wrap && !hug_main;

    // Fill across a single line: the size is the frame's inner cross size, known up front. Resize
    // those children first, so text that wraps (or a frame hugging it) reports its final main size.
    if !wraps {
        for it in items.iter_mut().filter(|it| it.fill_cross && !it.fill_main) {
            let cross = clamp_size(inner_cross_fixed, it.min_cross, it.max_cross);
            let (w, h) = if horizontal { (it.main, cross) } else { (cross, it.main) };
            resize_child(doc, it.id, w, h, m);
            let (w, h) = outer_size(doc, it.id);
            it.main = if horizontal { w } else { h };
        }
    }

    // Split into lines along the main axis (rows of a horizontal layout, columns of a vertical one).
    let mut lines: Vec<Vec<usize>> = Vec::new();
    if wraps {
        let mut cur: Vec<usize> = Vec::new();
        let mut used = 0.0;
        for (i, it) in items.iter().enumerate() {
            let sz = if it.fill_main { it.min_main.unwrap_or(0.0) } else { it.main };
            let add = if cur.is_empty() { sz } else { gap.max(0.0) + sz };
            if !cur.is_empty() && used + add > inner_main_fixed + 1e-6 {
                lines.push(std::mem::take(&mut cur));
                used = sz;
            } else {
                used += add;
            }
            cur.push(i);
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    } else {
        lines.push((0..items.len()).collect());
    }

    // Share each line's free space among its fill children.
    for line in &lines {
        let fixed: f64 = line.iter().filter_map(|i| items.get(*i)).filter(|it| !it.fill_main).map(|it| it.main).sum();
        let gaps = gap * line.len().saturating_sub(1) as f64;
        let fills: Vec<usize> = line.iter().copied().filter(|i| items.get(*i).is_some_and(|it| it.fill_main)).collect();
        if !fills.is_empty() {
            distribute(&mut items, fills, inner_main_fixed - fixed - if auto_gap { 0.0 } else { gaps });
        }
    }
    // Fill children take their main size now, so their cross size (wrapped text, a hugging frame)
    // is final before the lines and the frame are sized.
    for it in items.iter_mut().filter(|it| it.fill_main) {
        let cross = if it.fill_cross && !wraps { clamp_size(inner_cross_fixed, it.min_cross, it.max_cross) } else { it.cross };
        let (w, h) = if horizontal { (it.main, cross) } else { (cross, it.main) };
        resize_child(doc, it.id, w, h, m);
        let (w, h) = outer_size(doc, it.id);
        it.cross = if horizontal { h } else { w };
    }
    if baseline {
        for it in items.iter_mut().filter(|it| !it.fill_cross) {
            it.baseline = item_baseline(doc, it.id, m, 0);
        }
    }

    // Line extents: (main used, cross size).
    let mut line_extents: Vec<(f64, f64)> = Vec::new();
    for line in &lines {
        let gaps = gap * line.len().saturating_sub(1) as f64;
        // Baseline alignment: the line's baseline sits at the deepest first baseline.
        let line_base = line.iter().filter_map(|i| items.get(*i)).filter_map(|it| it.baseline).fold(0.0, f64::max);
        for i in line {
            if let Some(it) = items.get_mut(*i) {
                it.base_off = it.baseline.map(|b| line_base - b).unwrap_or(0.0);
            }
        }
        let in_line = || line.iter().filter_map(|i| items.get(*i));
        let main_used: f64 = in_line().map(|it| it.main).sum::<f64>() + gaps;
        // A line of nothing but fill-across children keeps their own size.
        let all_fill = in_line().all(|it| it.fill_cross);
        let cross: f64 = in_line().filter(|it| all_fill || !it.fill_cross).map(|it| it.base_off + it.cross).fold(0.0, f64::max);
        line_extents.push((main_used, cross));
    }

    // Container size (on a hugging axis fill children can't fill: they count with their own size).
    let content_main = line_extents.iter().map(|l| l.0).fold(0.0, f64::max);
    let cross_gap = finite_or_zero(al.cross_gap);
    let content_cross: f64 = line_extents.iter().map(|l| l.1).sum::<f64>() + cross_gap * lines.len().saturating_sub(1) as f64;
    let (mut nw, mut nh) = (fw, fh);
    if hug_main {
        let v = content_main + if horizontal { pl + pr } else { pt + pb };
        if horizontal {
            nw = v;
        } else {
            nh = v;
        }
    }
    if hug_cross {
        let v = content_cross + if horizontal { pt + pb } else { pl + pr };
        if horizontal {
            nh = v;
        } else {
            nw = v;
        }
    }
    let (nw, nh) = (clamp_size(nw, fcl.min_w, fcl.max_w).max(pl + pr), clamp_size(nh, fcl.min_h, fcl.max_h).max(pt + pb));
    set_size(doc, frame, nw, nh);
    apply_constraints(doc, frame, ow, oh);
    let inner_main = if horizontal { nw - pl - pr } else { nh - pt - pb };
    let inner_cross = if horizontal { nh - pt - pb } else { nw - pl - pr };

    // Position. Several lines share the free space across the flow: packed lines sit where the
    // cross alignment says (lines of nothing but fill children stretch over it instead), or the
    // space goes between them.
    let mut cross_pos = if horizontal { pt } else { pl };
    let single = lines.len() == 1;
    let mut cross_gap = cross_gap;
    if lines.len() > 1 {
        let lines_cross: f64 = line_extents.iter().map(|l| l.1).sum();
        let between = (lines.len() - 1) as f64;
        let spread = (inner_cross - lines_cross) / between;
        if al.content_align == ContentAlign::SpaceBetween && spread > 0.0 {
            cross_gap = spread;
        } else {
            if al.content_align == ContentAlign::SpaceBetween {
                cross_gap = 0.0;
            }
            let free = inner_cross - lines_cross - cross_gap * between;
            if free > 0.0 && items.iter().all(|it| it.fill_cross) {
                let extra = free / lines.len() as f64;
                line_extents.iter_mut().for_each(|l| l.1 += extra);
            } else {
                cross_pos += match al.cross_align {
                    CrossAlign::Min | CrossAlign::Baseline => 0.0,
                    CrossAlign::Center => free / 2.0,
                    CrossAlign::Max => free,
                };
            }
        }
    }
    for (li, line) in lines.iter().enumerate() {
        let (used, line_cross) = line_extents.get(li).copied().unwrap_or((0.0, 0.0));
        let line_cross = if single { inner_cross } else { line_cross };
        let total = used - gap * line.len().saturating_sub(1) as f64;
        let (mut pos, step_gap) = main_offsets(al.main_align, inner_main, total, line.len(), gap);
        pos += if horizontal { pl } else { pt };
        for i in line {
            let Some(it) = items.get(*i) else { continue };
            let cross_size = if it.fill_cross { clamp_size(line_cross, it.min_cross, it.max_cross) } else { it.cross };
            let off = match al.cross_align {
                CrossAlign::Min => 0.0,
                // Baseline applies to horizontal layouts; elsewhere it reads as top/left.
                CrossAlign::Baseline => it.base_off,
                CrossAlign::Center => (line_cross - cross_size) / 2.0,
                CrossAlign::Max => line_cross - cross_size,
            };
            // Fill across a wrapped line: its size is known only now.
            if it.fill_cross && wraps {
                let (w, h) = if horizontal { (it.main, cross_size) } else { (cross_size, it.main) };
                resize_child(doc, it.id, w, h, m);
            }
            let (x, y) = if horizontal { (pos, cross_pos + off) } else { (cross_pos + off, pos) };
            place(doc, it.id, x, y);
            pos += it.main + step_gap;
        }
        cross_pos += line_cross + cross_gap;
    }
}

/// Share `free` main-axis space among the fill items `open` like CSS flexbox: each takes an equal
/// share within its min/max; when clamping adds more than it removes, the items held at their min
/// keep it (otherwise those held at their max do) and the rest share what is left again.
fn distribute(items: &mut [Item], mut open: Vec<usize>, free: f64) {
    let mut free = free.max(0.0);
    while !open.is_empty() {
        let share = free / open.len() as f64;
        let sized: Vec<(usize, f64)> =
            open.iter().filter_map(|i| items.get(*i).map(|it| (*i, clamp_size(share, it.min_main, it.max_main)))).collect();
        let violation: f64 = sized.iter().map(|(_, v)| v - share).sum();
        let frozen = |v: f64| violation.abs() < 1e-9 || (violation > 0.0 && v > share + 1e-9) || (violation < 0.0 && v < share - 1e-9);
        for (i, v) in &sized {
            if frozen(*v)
                && let Some(it) = items.get_mut(*i)
            {
                it.main = *v;
                free -= *v;
            }
        }
        free = free.max(0.0);
        open = sized.into_iter().filter(|(_, v)| !frozen(*v)).map(|(i, _)| i).collect();
    }
}

fn resize_child(doc: &mut Document, id: NodeId, w: f64, h: f64, m: &dyn Measure) {
    let is_text_height = doc
        .node(id)
        .is_some_and(|n| matches!(&n.kind, NodeKind::Text(t) if t.auto_resize != TextAutoResize::None && t.auto_resize != TextAutoResize::Truncate));
    if is_text_height {
        if let Some(n) = doc.node_mut(id)
            && let NodeKind::Text(t) = &mut n.kind
            && t.auto_resize == TextAutoResize::WidthAndHeight
        {
            t.auto_resize = TextAutoResize::Height;
        }
        let th = m.text_height(doc, id, w).unwrap_or(h);
        set_size(doc, id, w, th);
    } else {
        let old = doc.node(id).map(|n| (n.width, n.height));
        set_size(doc, id, w, h);
        if let Some((ow, oh)) = old
            && doc.node(id).is_some_and(|n| n.kind.is_frame_like())
        {
            apply_constraints(doc, id, ow, oh);
        }
        if doc.node(id).is_some_and(|n| n.layout.is_some()) {
            layout_frame(doc, id, m);
        }
    }
}

/// Largest grid (cells) the layout handles; rows beyond it overlap the last row.
const MAX_GRID_CELLS: usize = 1_000_000;
const MAX_TRACKS: usize = 1000;

/// A grid auto layout child's cell (0-based anchor column/row and spans, clamped to the grid).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridCell {
    pub id: NodeId,
    pub column: usize,
    pub row: usize,
    pub column_span: usize,
    pub row_span: usize,
}

/// A grid auto layout frame's resolved geometry (frame-local): column x ranges, row y ranges,
/// each placed child's cell, and the frame size the grid wants.
#[derive(Clone, Debug, PartialEq)]
pub struct GridGeometry {
    pub columns: Vec<(f64, f64)>,
    pub rows: Vec<(f64, f64)>,
    pub cells: Vec<GridCell>,
    pub width: f64,
    pub height: f64,
}

impl GridGeometry {
    /// The frame-local rectangle a cell (with its spans) covers.
    pub fn cell_rect(&self, c: &GridCell) -> Rect {
        let x0 = self.columns.get(c.column).map(|t| t.0).unwrap_or(0.0);
        let x1 = self.columns.get(c.column + c.column_span - 1).map(|t| t.1).unwrap_or(x0);
        let y0 = self.rows.get(c.row).map(|t| t.0).unwrap_or(0.0);
        let y1 = self.rows.get(c.row + c.row_span - 1).map(|t| t.1).unwrap_or(y0);
        Rect::new(x0, y0, x1.max(x0), y1.max(y0))
    }
}

/// Place grid children in cells: children with an anchor column and row go there first; the rest
/// flow left to right, top to bottom into the next free cells that fit their spans. Returns the
/// cells (in child order) and the row count (at least the frame's row count).
pub fn grid_place(doc: &Document, al: &AutoLayout, kids: &[NodeId]) -> (Vec<GridCell>, usize) {
    let cols = (al.grid_columns as usize).clamp(1, MAX_TRACKS);
    let max_rows = (MAX_GRID_CELLS / cols).clamp(1, MAX_TRACKS);
    let mut occ: Vec<Vec<bool>> = Vec::new();
    let free = |occ: &Vec<Vec<bool>>, c: usize, r: usize, cs: usize, rs: usize| {
        (r..r + rs).all(|ri| occ.get(ri).is_none_or(|row| (c..c + cs).all(|ci| !row.get(ci).copied().unwrap_or(false))))
    };
    let mark = |occ: &mut Vec<Vec<bool>>, c: usize, r: usize, cs: usize, rs: usize| {
        while occ.len() < r + rs {
            occ.push(vec![false; cols]);
        }
        for row in occ.iter_mut().skip(r).take(rs) {
            for cell in row.iter_mut().skip(c).take(cs) {
                *cell = true;
            }
        }
    };
    let spans = |id: NodeId| {
        let cl = doc.node(id).map(|n| n.child_layout.clone()).unwrap_or_default();
        let cs = (cl.column_span() as usize).min(cols);
        let rs = (cl.row_span() as usize).min(max_rows);
        (cl.grid_column.map(|c| (c as usize).min(cols - cs)), cl.grid_row.map(|r| (r as usize).min(max_rows - rs)), cs, rs)
    };
    let mut placed: Vec<Option<GridCell>> = vec![None; kids.len()];
    // 1. Fully anchored children.
    for (i, id) in kids.iter().enumerate() {
        if let (Some(c), Some(r), cs, rs) = spans(*id) {
            mark(&mut occ, c, r, cs, rs);
            if let Some(slot) = placed.get_mut(i) {
                *slot = Some(GridCell { id: *id, column: c, row: r, column_span: cs, row_span: rs });
            }
        }
    }
    // 2. The rest, in order (a cursor keeps auto-placed children in reading order).
    let mut cursor = (0usize, 0usize); // (row, column)
    for (i, id) in kids.iter().enumerate() {
        if placed.get(i).is_some_and(Option::is_some) {
            continue;
        }
        let (col, row, cs, rs) = spans(*id);
        let last_row = max_rows - rs;
        let (c, r) = match (col, row) {
            (Some(c), _) => (0..=last_row).find(|r| free(&occ, c, *r, cs, rs)).map(|r| (c, r)).unwrap_or((c, last_row)),
            (None, Some(r)) => (0..=cols - cs).find(|c| free(&occ, *c, r, cs, rs)).map(|c| (c, r)).unwrap_or((0, r)),
            (None, None) => {
                let mut found = None;
                let (mut r, mut c) = cursor;
                while r <= last_row {
                    if c + cs <= cols && free(&occ, c, r, cs, rs) {
                        found = Some((c, r));
                        break;
                    }
                    c += 1;
                    if c + cs > cols {
                        c = 0;
                        r += 1;
                    }
                }
                let (c, r) = found.unwrap_or((0, last_row));
                cursor = (r, c + cs);
                (c, r)
            }
        };
        mark(&mut occ, c, r, cs, rs);
        if let Some(slot) = placed.get_mut(i) {
            *slot = Some(GridCell { id: *id, column: c, row: r, column_span: cs, row_span: rs });
        }
    }
    let cells: Vec<GridCell> = placed.into_iter().flatten().collect();
    let rows = cells.iter().map(|c| c.row + c.row_span).max().unwrap_or(0).max((al.grid_rows as usize).clamp(1, MAX_TRACKS));
    (cells, rows)
}

/// Track `i`'s sizing and sanitised value (missing tracks are `1fr`).
fn track_spec(specs: &[figcraft_doc::GridTrack], i: usize) -> (Sizing, f64) {
    let t = specs.get(i).copied().unwrap_or_default();
    let v = if t.value.is_finite() { t.value.clamp(0.0, 1e6) } else { 0.0 };
    (t.sizing, v)
}

/// Resolve track sizes: fixed tracks keep their size, hug tracks fit their content, fill tracks
/// share what is left by their fractional units like the CSS `fr` unit (a track never gets less
/// than its content: one whose share is too small keeps its content size and the others share the
/// rest). `avail` is None when the frame hugs that axis (fill tracks then hug their content too).
fn resolve_tracks(specs: &[figcraft_doc::GridTrack], content: &[f64], avail: Option<f64>) -> Vec<f64> {
    let mut out: Vec<f64> = (0..content.len())
        .map(|i| match track_spec(specs, i) {
            (Sizing::Fixed, v) => v,
            _ => content.get(i).copied().unwrap_or(0.0),
        })
        .collect();
    let Some(a) = avail else { return out };
    let fr = |i: usize| track_spec(specs, i).1;
    let base = |out: &[f64], i: usize| out.get(i).copied().unwrap_or(0.0);
    let (mut flex, fixed): (Vec<usize>, Vec<usize>) = (0..out.len()).partition(|i| track_spec(specs, *i).0 == Sizing::Fill);
    let mut used: f64 = fixed.iter().map(|i| base(&out, *i)).sum();
    // Each round either sizes every flexible track or freezes at least one at its content size.
    while !flex.is_empty() {
        let total: f64 = flex.iter().map(|i| fr(*i)).sum();
        let unit = if total > 1e-12 { (a - used).max(0.0) / total } else { 0.0 };
        let (open, frozen): (Vec<usize>, Vec<usize>) = flex.iter().partition(|i| unit * fr(**i) >= base(&out, **i));
        if frozen.is_empty() {
            for i in open {
                if let Some(s) = out.get_mut(i) {
                    *s = unit * fr(i);
                }
            }
            break;
        }
        used += frozen.iter().map(|i| base(&out, *i)).sum::<f64>();
        flex = open;
    }
    out
}

/// Content size per track (`hug`: the frame hugs this axis, so fill tracks size to content): the
/// largest single-span child (for a fill track, the least it can shrink to); a spanning child that
/// needs more than its tracks give it grows the content-sized tracks it spans (unless a flexible
/// track can absorb it).
fn track_content(cells: &[GridCell], sizes: &[f64], specs: &[figcraft_doc::GridTrack], n: usize, gap: f64, columns: bool, hug: bool) -> Vec<f64> {
    let by_content = |i: usize| track_spec(specs, i).0 != Sizing::Fixed;
    let mut content: Vec<f64> = (0..n).map(|i| if let (Sizing::Fixed, v) = track_spec(specs, i) { v } else { 0.0 }).collect();
    let span = |c: &GridCell| if columns { (c.column, c.column_span) } else { (c.row, c.row_span) };
    for (c, s) in cells.iter().zip(sizes) {
        let (start, len) = span(c);
        if len == 1
            && by_content(start)
            && let Some(t) = content.get_mut(start)
        {
            *t = t.max(*s);
        }
    }
    for (c, s) in cells.iter().zip(sizes) {
        let (start, len) = span(c);
        if len < 2 || (!hug && (start..start + len).any(|i| track_spec(specs, i).0 == Sizing::Fill)) {
            continue;
        }
        let have: f64 = content.iter().skip(start).take(len).sum::<f64>() + gap * (len - 1) as f64;
        let grow: Vec<usize> = (start..start + len).filter(|i| by_content(*i)).collect();
        if *s > have && !grow.is_empty() {
            let extra = (*s - have) / grow.len() as f64;
            for i in grow {
                if let Some(t) = content.get_mut(i) {
                    *t += extra;
                }
            }
        }
    }
    content
}

fn track_ranges(start: f64, sizes: &[f64], gap: f64) -> Vec<(f64, f64)> {
    let mut x = start;
    sizes
        .iter()
        .map(|s| {
            let r = (x, x + s);
            x += s + gap;
            r
        })
        .collect()
}

/// Resolve a grid auto layout frame's tracks and cells from its children's current sizes.
pub fn grid_geometry(doc: &Document, frame: NodeId) -> Option<GridGeometry> {
    let f = doc.node(frame)?;
    let al = f.layout.as_ref()?;
    if al.mode != LayoutMode::Grid {
        return None;
    }
    let (pt, pr, pb, pl) = insets(f, al);
    let kids = flow_children(doc, frame);
    let (cells, rows) = grid_place(doc, al, &kids);
    let cols = (al.grid_columns as usize).clamp(1, MAX_TRACKS);
    let gx = finite_or_zero(al.gap).max(0.0);
    let gy = finite_or_zero(al.cross_gap).max(0.0);
    let hug_w = f.child_layout.sizing_h == Sizing::Hug;
    let hug_h = f.child_layout.sizing_v == Sizing::Hug;
    // What each child asks of its tracks: its size, or (filling a non-hugging axis) its minimum.
    let (mut ws, mut hs) = (Vec::with_capacity(cells.len()), Vec::with_capacity(cells.len()));
    for c in &cells {
        let (w, h) = outer_size(doc, c.id);
        let cl = doc.node(c.id).map(|n| n.child_layout.clone()).unwrap_or_default();
        ws.push(if cl.sizing_h == Sizing::Fill && !hug_w { cl.min_w.unwrap_or(0.0) } else { w });
        hs.push(if cl.sizing_v == Sizing::Fill && !hug_h { cl.min_h.unwrap_or(0.0) } else { h });
    }
    let col_content = track_content(&cells, &ws, &al.column_tracks, cols, gx, true, hug_w);
    let row_content = track_content(&cells, &hs, &al.row_tracks, rows, gy, false, hug_h);
    let gaps_w = gx * cols.saturating_sub(1) as f64;
    let gaps_h = gy * rows.saturating_sub(1) as f64;
    let avail_w = (!hug_w).then_some(f.width - pl - pr - gaps_w);
    let avail_h = (!hug_h).then_some(f.height - pt - pb - gaps_h);
    let col_w = resolve_tracks(&al.column_tracks, &col_content, avail_w);
    let row_h = resolve_tracks(&al.row_tracks, &row_content, avail_h);
    let cl = &f.child_layout;
    let width = if hug_w { col_w.iter().sum::<f64>() + gaps_w + pl + pr } else { f.width };
    let height = if hug_h { row_h.iter().sum::<f64>() + gaps_h + pt + pb } else { f.height };
    Some(GridGeometry {
        columns: track_ranges(pl, &col_w, gx),
        rows: track_ranges(pt, &row_h, gy),
        cells,
        width: clamp_size(width, cl.min_w, cl.max_w).max(pl + pr),
        height: clamp_size(height, cl.min_h, cl.max_h).max(pt + pb),
    })
}

fn layout_grid(doc: &mut Document, frame: NodeId, al: &AutoLayout, m: &dyn Measure) {
    let Some(f) = doc.node(frame) else { return };
    let (fw, fh) = (f.width, f.height);
    let fcl = f.child_layout.clone();
    let (hug_w, hug_h) = (fcl.sizing_h == Sizing::Hug, fcl.sizing_v == Sizing::Hug);
    // A fixed axis takes its own min/max (and the padding floor) first, so the tracks see the
    // final size.
    let (pt, pr, pb, pl) = insets(f, al);
    let w = if hug_w { fw } else { clamp_size(fw, fcl.min_w, fcl.max_w).max(pl + pr) };
    let h = if hug_h { fh } else { clamp_size(fh, fcl.min_h, fcl.max_h).max(pt + pb) };
    set_size(doc, frame, w, h);
    let Some(g) = grid_geometry(doc, frame) else { return };
    // Fill-width children take their column width before the rows are sized, so text that wraps
    // (or a frame hugging it) gives its row its final height.
    let mut widened = false;
    for c in &g.cells {
        let Some(n) = doc.node(c.id) else { continue };
        let cl = &n.child_layout;
        if cl.sizing_h == Sizing::Fill && !hug_w {
            let nw = clamp_size(g.cell_rect(c).width(), cl.min_w, cl.max_w);
            let (_, h) = outer_size(doc, c.id);
            resize_child(doc, c.id, nw, h, m);
            widened = true;
        }
    }
    let g = if widened { grid_geometry(doc, frame).unwrap_or(g) } else { g };
    set_size(doc, frame, g.width, g.height);
    apply_constraints(doc, frame, fw, fh);
    // The frame's alignment in each cell; a child's own alignment wins.
    let frame_h = match al.main_align {
        MainAlign::Center => CellAlign::Center,
        MainAlign::Max => CellAlign::Max,
        _ => CellAlign::Min,
    };
    let frame_v = match al.cross_align {
        CrossAlign::Center => CellAlign::Center,
        CrossAlign::Max => CellAlign::Max,
        _ => CellAlign::Min,
    };
    let offset = |a: CellAlign, free: f64| match a {
        CellAlign::Min => 0.0,
        CellAlign::Center => free / 2.0,
        CellAlign::Max => free,
    };
    for c in &g.cells {
        let cell = g.cell_rect(c);
        let Some(n) = doc.node(c.id) else { continue };
        let cl = n.child_layout.clone();
        let fill_w = cl.sizing_h == Sizing::Fill && !hug_w;
        let fill_h = cl.sizing_v == Sizing::Fill && !hug_h;
        let (w, h) = outer_size(doc, c.id);
        let nw = if fill_w { clamp_size(cell.width(), cl.min_w, cl.max_w) } else { w };
        let nh = if fill_h { clamp_size(cell.height(), cl.min_h, cl.max_h) } else { h };
        if fill_w || fill_h {
            resize_child(doc, c.id, nw, nh, m);
        }
        let (nw, nh) = outer_size(doc, c.id);
        let ox = offset(cl.grid_align_h.unwrap_or(frame_h), cell.width() - nw);
        let oy = offset(cl.grid_align_v.unwrap_or(frame_v), cell.height() - nh);
        place(doc, c.id, cell.x0 + ox, cell.y0 + oy);
    }
}

/// After frame `id` changed size from `(ow, oh)`, move/resize its children per their constraints
/// (recursively for children frames that resize). In an auto layout frame only the children that
/// ignore auto layout (absolute position) follow constraints; the layout places the others.
pub fn apply_constraints(doc: &mut Document, id: NodeId, ow: f64, oh: f64) {
    let Some(_nested) = Nested::enter() else { return };
    let Some(f) = doc.node(id) else { return };
    let absolute_only = f.layout.is_some();
    let (nw, nh) = (f.width, f.height);
    if (nw - ow).abs() < 1e-9 && (nh - oh).abs() < 1e-9 {
        return;
    }
    let sx = if ow > 1e-9 { nw / ow } else { 1.0 };
    let sy = if oh > 1e-9 { nh / oh } else { 1.0 };
    let kids = f.children.clone();
    // Group children follow the group's own bounds; constraints apply to frames' children.
    if f.kind.bounds_follow_children() {
        return;
    }
    for c in kids {
        let Some(n) = doc.node(c) else { continue };
        if absolute_only && !n.child_layout.absolute {
            continue;
        }
        let lin = linear(n.transform);
        let b = figcraft_geom::transform_rect_bbox(n.transform, n.local_rect());
        let (cw, ch) = (n.width, n.height);
        let (bw, bh) = (b.width(), b.height());
        let (mut x0, mut x1) = (b.x0, b.x1);
        let (mut y0, mut y1) = (b.y0, b.y1);
        match n.constraints.horizontal {
            HConstraint::Left => {}
            HConstraint::Right => {
                x0 += nw - ow;
                x1 += nw - ow;
            }
            HConstraint::LeftRight => x1 += nw - ow,
            HConstraint::Center => {
                let d = (nw - ow) / 2.0;
                x0 += d;
                x1 += d;
            }
            HConstraint::Scale => {
                x0 *= sx;
                x1 *= sx;
            }
        }
        match n.constraints.vertical {
            VConstraint::Top => {}
            VConstraint::Bottom => {
                y0 += nh - oh;
                y1 += nh - oh;
            }
            VConstraint::TopBottom => y1 += nh - oh,
            VConstraint::Center => {
                let d = (nh - oh) / 2.0;
                y0 += d;
                y1 += d;
            }
            VConstraint::Scale => {
                y0 *= sy;
                y1 *= sy;
            }
        }
        // New local size from the bbox change, along the child's own axes (axis-aligned children,
        // turned or flipped, exactly; others approximately, never skewed).
        let (kw, kh) = axis_factors(lin, if bw > 1e-9 { (x1 - x0) / bw } else { 1.0 }, if bh > 1e-9 { (y1 - y0) / bh } else { 1.0 });
        let (new_w, new_h) = (cw * kw, ch * kh);
        let resized = (new_w - cw).abs() > 1e-9 || (new_h - ch).abs() > 1e-9;
        if resized {
            set_size(doc, c, new_w, new_h);
            if let Some(cn) = doc.node_mut(c) {
                cn.transform = lin;
            }
        }
        place(doc, c, x0, y0);
        if resized && doc.node(c).is_some_and(|n| n.kind.is_frame_like() || n.kind.bounds_follow_children()) {
            if doc.node(c).is_some_and(|n| n.kind.bounds_follow_children()) {
                scale_group_children(doc, c, kw, kh);
            } else {
                apply_constraints(doc, c, cw, ch);
            }
        }
    }
}

/// Scale a group's children when the group is resized (groups scale their contents).
/// Each child keeps its rotation and flip and scales along its own axes ([`axis_factors`]), its
/// centre following the group's scale.
pub fn scale_group_children(doc: &mut Document, id: NodeId, kx: f64, ky: f64) {
    let Some(_nested) = Nested::enter() else { return };
    let Some(n) = doc.node(id) else { return };
    for c in n.children.clone() {
        let Some(cn) = doc.node(c) else { continue };
        let center = figcraft_geom::transform_rect_bbox(cn.transform, cn.local_rect()).center();
        let (w, h) = (cn.width, cn.height);
        let (kw, kh) = axis_factors(cn.transform, kx, ky);
        set_size(doc, c, w * kw, h * kh);
        let (nw, nh) = outer_size(doc, c);
        place(doc, c, center.x * kx - nw / 2.0, center.y * ky - nh / 2.0);
        if doc.node(c).is_some_and(|n| n.kind.bounds_follow_children()) {
            scale_group_children(doc, c, kw, kh);
        } else if doc.node(c).is_some_and(|n| n.kind.is_frame_like()) {
            apply_constraints(doc, c, w, h);
        }
    }
}

/// The rectangles (frame-local) a layout grid draws: columns, rows or the square grid's lines.
pub fn grid_lines(g: &LayoutGrid, w: f64, h: f64) -> Vec<Rect> {
    let mut out = Vec::new();
    match g.kind {
        GridKind::Grid => {
            let s = if g.size.is_finite() && g.size >= 1.0 { g.size } else { 10.0 };
            let mut x = 0.0;
            let mut guard = 0;
            while x <= w && guard < 4000 {
                out.push(Rect::new(x, 0.0, x, h));
                x += s;
                guard += 1;
            }
            let mut y = 0.0;
            guard = 0;
            while y <= h && guard < 4000 {
                out.push(Rect::new(0.0, y, w, y));
                y += s;
                guard += 1;
            }
        }
        GridKind::Columns | GridKind::Rows => {
            let cols = g.kind == GridKind::Columns;
            let total = if cols { w } else { h };
            let gutter = if g.gutter.is_finite() { g.gutter.max(0.0) } else { 0.0 };
            let offset = if g.offset.is_finite() { g.offset.max(0.0) } else { 0.0 };
            let count = match g.count {
                // Auto: as many fixed-size columns (rows) as fit in the room the type leaves them.
                0 if g.align != GridAlign::Stretch => {
                    let size = if g.size.is_finite() && g.size > 0.0 { g.size } else { 60.0 };
                    let room = if g.align == GridAlign::Center { total } else { total - offset };
                    ((room + gutter) / (size + gutter)).floor().clamp(1.0, 500.0) as usize
                }
                n => n.clamp(1, 500) as usize,
            };
            let (start, size) = match g.align {
                GridAlign::Stretch => {
                    let size = ((total - 2.0 * offset - gutter * (count - 1) as f64) / count as f64).max(0.0);
                    (offset, size)
                }
                _ => {
                    let size = if g.size.is_finite() && g.size > 0.0 { g.size } else { 60.0 };
                    let span = size * count as f64 + gutter * (count - 1) as f64;
                    let start = match g.align {
                        GridAlign::Min => offset,
                        GridAlign::Center => (total - span) / 2.0,
                        _ => total - offset - span,
                    };
                    (start, size)
                }
            };
            for i in 0..count {
                let a = start + i as f64 * (size + gutter);
                out.push(if cols { Rect::new(a, 0.0, a + size, h) } else { Rect::new(0.0, a, w, a + size) });
            }
        }
    }
    out
}

/// Whether a frame's layout grids apply: they don't show (or snap) on a frame turned off 0°.
pub fn grids_shown(doc: &Document, frame: NodeId) -> bool {
    figcraft_geom::rotation_degrees(doc.world_transform(frame)).abs() < 1e-6
}

/// Snap targets of a frame's visible layout grids (page coordinates): column/row edges.
pub fn grid_snap_lines(doc: &Document, frame: NodeId) -> (Vec<f64>, Vec<f64>) {
    grid_snap_lines_in(doc, frame, doc.world_transform(frame))
}

/// [`grid_snap_lines`] mapped from the frame's coordinates by `xf` (the frame's world transform
/// for page lines).
pub fn grid_snap_lines_in(doc: &Document, frame: NodeId, xf: Affine) -> (Vec<f64>, Vec<f64>) {
    let Some(f) = doc.node(frame).filter(|_| grids_shown(doc, frame)) else { return (vec![], vec![]) };
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for g in f.grids.iter().filter(|g| g.visible) {
        for r in grid_lines(g, f.width, f.height) {
            let a = xf * Point::new(r.x0, r.y0);
            let b = xf * Point::new(r.x1, r.y1);
            match g.kind {
                GridKind::Columns => {
                    xs.push(a.x);
                    xs.push(b.x);
                }
                GridKind::Rows => {
                    ys.push(a.y);
                    ys.push(b.y);
                }
                GridKind::Grid => {
                    if (r.x0 - r.x1).abs() < 1e-9 {
                        xs.push(a.x);
                    } else {
                        ys.push(a.y);
                    }
                }
            }
        }
    }
    (xs, ys)
}

#[cfg(test)]
mod behaviour_tests;
#[cfg(test)]
mod content_align_tests;
#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod nested_tests;
#[cfg(test)]
mod tests;
