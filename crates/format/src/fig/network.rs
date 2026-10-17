//! Vector network blobs: the editable form of a vector layer (`vectorData.vectorNetworkBlob`).
//!
//! Little-endian throughout: `u32` counts of vertices, segments and regions; each vertex a style
//! id (`u32`) and its position (`f32` x, y); each segment a style id, its start vertex (`u32`) and
//! tangent (`f32` x, y), then its end vertex and tangent; each region a `u32` holding its style id
//! shifted left once with the winding rule in the low bit (set: non-zero, clear: even-odd), a loop
//! count, and per loop a segment count and the segment indices. Positions are in the space of
//! `vectorData.normalizedSize`. Style ids point into `vectorData.styleOverrideTable` (0: none).

/// Most vertices, segments, regions or loop entries read from one blob.
const MAX_ITEMS: usize = 200_000;

pub(super) struct Vertex {
    pub style: u32,
    pub x: f64,
    pub y: f64,
}

pub(super) struct Segment {
    pub style: u32,
    pub a: usize,
    pub ta: (f64, f64),
    pub b: usize,
    pub tb: (f64, f64),
}

pub(super) struct Region {
    pub style: u32,
    pub non_zero: bool,
    pub loops: Vec<Vec<usize>>,
}

#[derive(Default)]
pub(super) struct Network {
    pub vertices: Vec<Vertex>,
    pub segments: Vec<Segment>,
    pub regions: Vec<Region>,
}

struct Reader<'b> {
    b: &'b [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.pos..self.pos + 4)?.try_into().ok()?);
        self.pos += 4;
        Some(v)
    }

    fn f32(&mut self) -> Option<f64> {
        Some(f64::from(f32::from_bits(self.u32()?))).filter(|x| x.is_finite())
    }

    /// An index below `len`.
    fn index(&mut self, len: usize) -> Option<usize> {
        Some(self.u32()? as usize).filter(|i| *i < len)
    }

    /// A count of items of at least `size` bytes each that the rest of the blob can hold.
    fn count(&mut self, size: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= MAX_ITEMS && n * size <= self.b.len().saturating_sub(self.pos)).then_some(n)
    }
}

/// The network in blob `b`; `None` when it's truncated, out of range or not finite.
pub(super) fn decode(b: &[u8]) -> Option<Network> {
    let mut r = Reader { b, pos: 0 };
    let (nv, ns, nr) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    if nv > MAX_ITEMS || ns > MAX_ITEMS || nr > MAX_ITEMS || nv * 12 + ns * 28 + nr * 8 > b.len().saturating_sub(12) {
        return None;
    }
    let mut net = Network::default();
    for _ in 0..nv {
        net.vertices.push(Vertex { style: r.u32()?, x: r.f32()?, y: r.f32()? });
    }
    for _ in 0..ns {
        let style = r.u32()?;
        net.segments.push(Segment { style, a: r.index(nv)?, ta: (r.f32()?, r.f32()?), b: r.index(nv)?, tb: (r.f32()?, r.f32()?) });
    }
    let mut entries = 0usize;
    for _ in 0..nr {
        let sw = r.u32()?;
        let mut loops = Vec::new();
        for _ in 0..r.count(4)? {
            let n = r.count(4)?;
            entries += n;
            if entries > MAX_ITEMS {
                return None;
            }
            loops.push((0..n).map(|_| r.index(ns)).collect::<Option<Vec<usize>>>()?);
        }
        net.regions.push(Region { style: sw >> 1, non_zero: sw & 1 == 1, loops });
    }
    Some(net)
}

/// The blob of `net` (the export writes it; so do test fixtures). Callers pass finite values.
pub(super) fn encode(net: &Network) -> Vec<u8> {
    let mut out = Vec::new();
    let u = |v: u32, out: &mut Vec<u8>| out.extend_from_slice(&v.to_le_bytes());
    let fl = |v: f64, out: &mut Vec<u8>| out.extend_from_slice(&(v as f32).to_le_bytes());
    for n in [net.vertices.len(), net.segments.len(), net.regions.len()] {
        u(n as u32, &mut out);
    }
    for v in &net.vertices {
        u(v.style, &mut out);
        fl(v.x, &mut out);
        fl(v.y, &mut out);
    }
    for s in &net.segments {
        u(s.style, &mut out);
        u(s.a as u32, &mut out);
        fl(s.ta.0, &mut out);
        fl(s.ta.1, &mut out);
        u(s.b as u32, &mut out);
        fl(s.tb.0, &mut out);
        fl(s.tb.1, &mut out);
    }
    for r in &net.regions {
        u((r.style << 1) | u32::from(r.non_zero), &mut out);
        u(r.loops.len() as u32, &mut out);
        for l in &r.loops {
            u(l.len() as u32, &mut out);
            for i in l {
                u(*i as u32, &mut out);
            }
        }
    }
    out
}
