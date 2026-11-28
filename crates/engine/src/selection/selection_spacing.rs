//! Smart-selection spacing in page coordinates. Tracks contain aligned siblings (columns for
//! horizontal spacing, rows for vertical spacing); moving a track preserves its internal layout.
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Rect;

const EPS: f64 = 0.01;

#[derive(Clone, Debug)]
pub struct Track {
    pub ids: Vec<NodeId>,
    pub bounds: Rect,
    anchor: Rect,
}

#[derive(Clone, Debug)]
pub struct Axis {
    pub tracks: Vec<Track>,
    pub vertical: bool,
    /// None means the existing distances are mixed; setting a value makes them uniform.
    pub value: Option<f64>,
}

impl Axis {
    pub fn start(&self, b: Rect) -> f64 {
        if self.vertical { b.y0 } else { b.x0 }
    }
    pub fn end(&self, b: Rect) -> f64 {
        if self.vertical { b.y1 } else { b.x1 }
    }
    pub fn gap(&self, a: &Track, b: &Track) -> f64 {
        self.start(b.bounds) - self.end(a.bounds)
    }
    /// Desired page-space translations, keeping the first track fixed.
    pub fn offsets(&self, value: f64) -> Vec<(NodeId, f64)> {
        let mut pos = self.tracks.first().map_or(0.0, |t| self.start(t.bounds));
        let mut out = Vec::new();
        for t in &self.tracks {
            let delta = pos - self.start(t.bounds);
            out.extend(t.ids.iter().map(|id| (*id, delta)));
            pos += self.end(t.bounds) - self.start(t.bounds) + value;
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct SelectionSpacing {
    pub horizontal: Option<Axis>,
    pub vertical: Option<Axis>,
}

/// Available only for movable, visible siblings outside normal auto-layout flow. Parent/child
/// selections, instance internals and locked layers must not be moved indirectly by a gap edit.
pub fn selection_spacing(doc: &Document, ids: &[NodeId]) -> SelectionSpacing {
    let Some(first) = ids.first().and_then(|id| doc.node(*id)) else { return SelectionSpacing::default() };
    if ids.len() < 2 || ids.len() > 2000 {
        return SelectionSpacing::default();
    }
    let mut items = Vec::new();
    for id in ids {
        let Some(n) = doc.node(*id) else { return SelectionSpacing::default() };
        if n.parent != first.parent
            || !n.visible
            || doc.ancestors(*id).into_iter().any(|a| doc.node(a).is_some_and(|n| !n.visible))
            || crate::tools::locked_in(doc, *id)
            || crate::components::inside_instance(doc, *id)
            || matches!(n.kind, NodeKind::Page { .. })
            || n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some() && !n.child_layout.absolute)
        {
            return SelectionSpacing::default();
        }
        let determinant = n.parent.map(|p| doc.world_transform(p).determinant()).unwrap_or(1.0);
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return SelectionSpacing::default();
        }
        let Some(b) = doc.world_bounds(*id) else { return SelectionSpacing::default() };
        if ![b.x0, b.y0, b.x1, b.y1].into_iter().all(f64::is_finite) || b.width() <= EPS || b.height() <= EPS {
            return SelectionSpacing::default();
        }
        if !items.iter().any(|(i, _)| i == id) {
            items.push((*id, b));
        }
    }
    SelectionSpacing { horizontal: axis(&items, false), vertical: axis(&items, true) }
}

fn axis(items: &[(NodeId, Rect)], vertical: bool) -> Option<Axis> {
    let edges = |b: Rect| if vertical { (b.y0, b.y1) } else { (b.x0, b.x1) };
    let mut items = items.to_vec();
    items.sort_by(|a, b| edges(a.1).0.total_cmp(&edges(b.1).0).then(a.0.cmp(&b.0)));
    let mut tracks: Vec<Track> = Vec::new();
    for (id, b) in items {
        let (lo, hi) = edges(b);
        let aligned = tracks.iter_mut().find(|t| {
            let (a, z) = edges(t.anchor);
            (lo - a).abs() <= EPS || (hi - z).abs() <= EPS || ((lo + hi - a - z) * 0.5).abs() <= EPS
        });
        if let Some(t) = aligned {
            t.ids.push(id);
            t.bounds = t.bounds.union(b);
        } else {
            tracks.push(Track { ids: vec![id], bounds: b, anchor: b });
        }
    }
    if tracks.len() < 2 {
        return None;
    }
    tracks.sort_by(|a, b| edges(a.bounds).0.total_cmp(&edges(b.bounds).0));
    // Diagonally scattered layers aren't a row/column/grid selection.
    let cross = |b: Rect| if vertical { (b.x0, b.x1) } else { (b.y0, b.y1) };
    let low = tracks.iter().map(|t| cross(t.bounds).0).fold(f64::NEG_INFINITY, f64::max);
    let high = tracks.iter().map(|t| cross(t.bounds).1).fold(f64::INFINITY, f64::min);
    if high - low <= EPS {
        return None;
    }
    let mut result = Axis { tracks, vertical, value: None };
    let gaps: Vec<f64> = result.tracks.windows(2).filter_map(|p| p.first().zip(p.get(1)).map(|(a, b)| result.gap(a, b))).collect();
    result.value = gaps.first().copied().filter(|v| gaps.iter().all(|g| (g - v).abs() <= EPS));
    Some(result)
}

/// A spacing gesture never changes selection. Roll it back in its owning document, preserving
/// a subsequent tab/page/selection change and leaving another document's gesture alone.
pub fn cancel_gesture(session: &mut crate::Session, uid: u64) {
    if let Some(ds) = session.docs.iter_mut().find(|d| d.uid == uid)
        && let Some(it) = ds.interaction.take()
    {
        ds.doc = it.doc;
        ds.revision += 1;
        ds.clean_selection();
    }
}
