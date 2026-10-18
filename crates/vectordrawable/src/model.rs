//! What the walk produces before it's written: paths and groups in viewport coordinates.

use figcraft_color::Color;
use figcraft_geom::{BezPath, Rect};

/// A gradient in the coordinates of the path it fills.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GradKind {
    Linear { start: (f64, f64), end: (f64, f64) },
    Radial { center: (f64, f64), radius: f64 },
    Sweep { center: (f64, f64) },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Grad {
    pub kind: GradKind,
    /// Offsets in 0..=1, ascending; colours with their opacity folded in.
    pub stops: Vec<(f64, Color)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fill {
    Solid(Color),
    Gradient(Grad),
}

/// A stroke Android draws itself (centred, uniform, solid).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeStroke {
    pub color: Color,
    pub width: f64,
    pub cap: &'static str,
    pub join: &'static str,
    pub miter: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathItem {
    pub name: Option<String>,
    pub data: BezPath,
    pub fill: Option<Fill>,
    pub stroke: Option<NativeStroke>,
    pub even_odd: bool,
}

/// A `<group>`'s own transform: scale, then rotate (degrees), then translate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroupXf {
    pub translate: (f64, f64),
    pub rotation: f64,
    pub scale: (f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GroupItem {
    pub name: Option<String>,
    pub xf: Option<GroupXf>,
    /// A non-zero clip outline in the group's (untransformed) coordinates.
    pub clip: Option<BezPath>,
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ItemKind {
    Path(PathItem),
    Group(GroupItem),
}

/// One written element with the viewport area it paints (for the opacity overlap test).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub kind: ItemKind,
    pub bbox: Rect,
}

impl Item {
    /// The areas of what paints, each a leaf (a path, or a transformed gradient group).
    pub fn leaf_boxes(items: &[Item], out: &mut Vec<Rect>) {
        for it in items {
            match &it.kind {
                ItemKind::Group(g) if g.xf.is_none() => Self::leaf_boxes(&g.children, out),
                _ => out.push(it.bbox),
            }
        }
    }

    /// Multiply every colour's alpha by `k` (a layer's opacity folded into what it paints).
    pub fn scale_alpha(items: &mut [Item], k: f32) {
        let fade = |c: &mut Color| c.a = (c.a * k).clamp(0.0, 1.0);
        for it in items {
            match &mut it.kind {
                ItemKind::Path(p) => {
                    match &mut p.fill {
                        Some(Fill::Solid(c)) => fade(c),
                        Some(Fill::Gradient(g)) => g.stops.iter_mut().for_each(|(_, c)| fade(c)),
                        None => {}
                    }
                    if let Some(s) = &mut p.stroke {
                        fade(&mut s.color);
                    }
                }
                ItemKind::Group(g) => Self::scale_alpha(&mut g.children, k),
            }
        }
    }

    /// Whether any two leaves' areas overlap (more than `MAX` leaves count as overlapping).
    pub fn overlap(items: &[Item]) -> bool {
        const MAX: usize = 64;
        let mut boxes = Vec::new();
        Self::leaf_boxes(items, &mut boxes);
        if boxes.len() > MAX {
            return true;
        }
        boxes.iter().enumerate().any(|(i, a)| {
            boxes.iter().skip(i + 1).any(|b| {
                let r = a.intersect(*b);
                r.width() > 1e-9 && r.height() > 1e-9
            })
        })
    }
}
