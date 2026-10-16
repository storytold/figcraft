//! A vector network's regions as areas: walking a region's loops into contours, whether a point is
//! inside a region by that region's own fill rule, which region is under a point (the last one
//! listed wins) and a region's points. Vector edit mode hovers, hatches and selects regions with
//! these.
//!
//! A loop lists its segments in ring order, but each segment may be stored either way round: the
//! walk follows the point reached so far and runs a segment backwards when it starts at its `b` end.

use std::hash::{Hash, Hasher};

use figcraft_geom::{BezPath, Point, Shape};

use crate::vector::{VectorNetwork, WindingRule};

impl VectorNetwork {
    /// Loop `lp` walked as a ring: each existing segment with whether it is walked backwards (from
    /// its `b` end to its `a` end). The first segment runs toward the point it shares with the
    /// second; a segment that doesn't touch the point reached so far (a malformed loop) is taken
    /// as stored. Missing segments are skipped.
    pub fn loop_walk(&self, lp: &[usize]) -> Vec<(usize, bool)> {
        let mut existing = lp.iter().filter_map(|i| self.segments.get(*i));
        let Some(first) = existing.next() else { return Vec::new() };
        let forward_first = match existing.next() {
            Some(second) => first.b == second.a || first.b == second.b,
            None => true,
        };
        let mut cur = if forward_first { first.a } else { first.b };
        let mut out = Vec::with_capacity(lp.len());
        for &i in lp {
            let Some(s) = self.segments.get(i) else { continue };
            if s.a == cur {
                out.push((i, false));
                cur = s.b;
            } else if s.b == cur {
                out.push((i, true));
                cur = s.a;
            } else {
                out.push((i, false));
                cur = s.b;
            }
        }
        out
    }

    /// Region `r`'s outline: each loop one closed contour, curves kept as curves (segments or
    /// points that don't exist are left out). `None` when there is no such region or nothing of it
    /// remains.
    pub fn region_path(&self, r: usize) -> Option<BezPath> {
        let region = self.regions.get(r)?;
        let mut path = BezPath::new();
        for lp in &region.loops {
            let mut open = false;
            let mut at = Point::ZERO;
            for (i, back) in self.loop_walk(lp) {
                let Some(s) = self.segments.get(i) else { continue };
                let (Some(a), Some(b)) = (self.vertices.get(s.a), self.vertices.get(s.b)) else { continue };
                let (a, b) = (a.point(), b.point());
                let (c1, c2) = (Point::new(a.x + s.ta.0, a.y + s.ta.1), Point::new(b.x + s.tb.0, b.y + s.tb.1));
                let (from, c1, c2, to) = if back { (b, c2, c1, a) } else { (a, c1, c2, b) };
                if !open {
                    path.move_to(from);
                    open = true;
                } else if from != at {
                    // A gap in a malformed loop: bridged with a straight line.
                    path.line_to(from);
                }
                if s.is_straight() {
                    path.line_to(to);
                } else {
                    path.curve_to(c1, c2, to);
                }
                at = to;
            }
            if open {
                path.close_path();
            }
        }
        (!path.elements().is_empty()).then_some(path)
    }

    /// The winding count of `p` about all of region `r`'s loops together (curves counted where
    /// they run). `None` when there is no such region.
    pub fn region_winding(&self, r: usize, p: Point) -> Option<i32> {
        self.regions.get(r)?;
        Some(self.region_path(r).map_or(0, |path| path.winding(p)))
    }

    /// Whether `p` is inside region `r` by the region's own fill rule (non-zero: any winding;
    /// even-odd: an odd winding).
    pub fn region_contains(&self, r: usize, p: Point) -> bool {
        let (Some(region), Some(w)) = (self.regions.get(r), self.region_winding(r, p)) else { return false };
        match region.winding {
            WindingRule::NonZero => w != 0,
            WindingRule::EvenOdd => w % 2 != 0,
        }
    }

    /// The region under `p`: the last one listed that contains it.
    pub fn region_at(&self, p: Point) -> Option<usize> {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        (0..self.regions.len()).rev().find(|r| self.region_contains(*r, p))
    }

    /// Every point on region `r`'s loops (holes' too), each once, sorted (segments or points that
    /// don't exist are left out).
    pub fn region_points(&self, r: usize) -> Vec<usize> {
        let Some(region) = self.regions.get(r) else { return Vec::new() };
        let mut out: Vec<usize> = region
            .loops
            .iter()
            .flatten()
            .filter_map(|i| self.segments.get(*i))
            .flat_map(|s| [s.a, s.b])
            .filter(|v| *v < self.vertices.len())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// A fingerprint of the network's topology: which points each segment joins and which
    /// segments each region's loops list. Moving points or handles keeps it; adding, removing or
    /// reordering segments, regions or loops changes it (region indexes are only good while it
    /// stays the same).
    pub fn topology_key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        self.segments.len().hash(&mut h);
        for s in &self.segments {
            (s.a, s.b).hash(&mut h);
        }
        self.regions.len().hash(&mut h);
        for r in &self.regions {
            r.loops.hash(&mut h);
        }
        h.finish()
    }
}
