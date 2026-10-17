//! Strokes of vector networks whose points have caps and joins of their own: the stroke is drawn
//! run by run (a run ends at end points, junctions and points with a join of their own), each run
//! taking the caps of the points it ends at, and each point's own join is drawn over the break.

use figcraft_doc::{StrokeCap, StrokeProps, StrokeRun, VectorNetwork};
use figcraft_geom::kurbo::{self, BezPath, Cap};

use crate::{kcap, kjoin};

/// A network ready to stroke with its points' styles: rounded points drawn in, its runs and each
/// point's segment count.
pub(crate) struct Styled {
    net: VectorNetwork,
    runs: Vec<StrokeRun>,
    degrees: Vec<usize>,
}

impl Styled {
    /// `None` when no point has a style of its own, or the stroke is dashed, variable or a brush.
    /// shortcut: dashed, variable-width and brush strokes keep the layer's caps and joins at every point; stroke them run by run too if asked for.
    /// Joins equal to the layer do not require styled runs; stored overrides remain unchanged.
    pub(crate) fn new(net: &VectorNetwork, props: &StrokeProps) -> Option<Styled> {
        if !net.has_point_styles(props.join) || !props.dashes.is_empty() || props.profile().is_some() || props.brush.is_some() {
            return None;
        }
        let mut net = net.rounded().unwrap_or_else(|| net.clone());
        for vertex in &mut net.vertices {
            if vertex.join == Some(props.join) {
                vertex.join = None;
            }
        }
        let runs = net.stroke_runs();
        let mut degrees = vec![0usize; net.vertices.len()];
        for s in &net.segments {
            for v in [s.a, s.b] {
                if let Some(d) = degrees.get_mut(v) {
                    *d += 1;
                }
            }
        }
        Some(Styled { net, runs, degrees })
    }

    /// Whether point `v` breaks a run to draw its own join.
    fn breaks(&self, v: usize) -> bool {
        self.degrees.get(v) == Some(&2) && self.net.vertices.get(v).is_some_and(|x| x.join.is_some())
    }

    /// The cap where a run ends at point `v`: none at a join of its own, else the point's, else
    /// `layer`.
    fn cap_at(&self, v: usize, layer: StrokeCap) -> StrokeCap {
        if self.breaks(v) { StrokeCap::None } else { self.net.vertices.get(v).and_then(|x| x.cap).unwrap_or(layer) }
    }

    /// The stroke centred at `width`.
    pub(crate) fn outline(&self, width: f64, props: &StrokeProps) -> BezPath {
        let mut out = BezPath::new();
        let miter = props.miter_limit.clamp(1.0, 100.0);
        for run in &self.runs {
            let style = kurbo::Stroke::new(width)
                .with_join(kjoin(props.join))
                .with_miter_limit(miter)
                .with_start_cap(kcap(self.cap_at(run.start, props.cap_start)))
                .with_end_cap(kcap(self.cap_at(run.end, props.cap_end)));
            out.extend(kurbo::stroke(run.path.iter(), &style, &kurbo::StrokeOpts::default(), 0.05).elements().iter().copied());
        }
        // Each point's own join: a short corner along its two segments stroked with that join.
        let reach = (width * 0.05).max(0.2);
        for (v, x) in self.net.vertices.iter().enumerate() {
            let Some(join) = x.join.filter(|_| self.breaks(v)) else { continue };
            let inc = self.net.incident(v);
            let [(s1, a1), (s2, a2)] = inc.as_slice() else { continue };
            let (Some(d1), Some(d2)) = (self.net.tangent_out(*s1, *a1), self.net.tangent_out(*s2, *a2)) else { continue };
            let p = x.point();
            let mut corner = BezPath::new();
            corner.move_to(p + d1 * reach);
            corner.line_to(p);
            corner.line_to(p + d2 * reach);
            let style = kurbo::Stroke::new(width).with_join(kjoin(join)).with_miter_limit(miter).with_caps(Cap::Butt);
            out.extend(kurbo::stroke(corner.iter(), &style, &kurbo::StrokeOpts::default(), 0.05).elements().iter().copied());
        }
        out
    }

    /// Arrowheads at run ends: a point's own arrow cap, and the layer's at the stroke's first start
    /// and last end (as without point styles).
    pub(crate) fn arrowheads(&self, weight: f64, props: &StrokeProps) -> BezPath {
        let mut out = BezPath::new();
        let last = self.runs.len().saturating_sub(1);
        for (k, run) in self.runs.iter().enumerate().filter(|(_, r)| !r.closed) {
            let start = self.cap_at(run.start, if k == 0 { props.cap_start } else { StrokeCap::None });
            let end = self.cap_at(run.end, if k == last { props.cap_end } else { StrokeCap::None });
            for (at_end, cap) in [(false, start), (true, end)] {
                if cap.is_arrow()
                    && let Some(head) = crate::arrowhead(&run.path, cap, weight, at_end)
                {
                    out.extend(head.elements().iter().copied());
                }
            }
        }
        out
    }
}
