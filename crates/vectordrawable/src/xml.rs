//! Maps FigCraft's projected drawable into the shared XML writer.
//! Geometry, colour policy and compatibility decisions remain in the application.

use crate::model::{Fill, GradKind, Item, ItemKind};
use craft_vectordrawable::model as wire;
use figcraft_color::Color;

#[cfg(test)]
pub(crate) use craft_vectordrawable::write::{esc, num, path_data};

// One document level can add a frame clip, a mask clip and a named group, plus the leaf gradient's
// transform group. Keep every layer admitted by check_export_depth, not just 256 XML groups.
const XML_DEPTH: usize = 3 * (figcraft_render::MAX_EXPORT_DEPTH + 1) + 2;
const _: () = assert!(XML_DEPTH <= craft_vectordrawable::write::MAX_CONFIGURED_DEPTH);

fn wire_color(c: Color) -> wire::Color {
    // FigCraft clamps positive infinity to one (the neutral writer maps non-finite to zero).
    // Preserve the application's sanitization before passing its finite components onward.
    let c = c.sanitized();
    [c.r, c.g, c.b, c.a]
}

#[cfg(test)]
pub(crate) fn color(c: Color) -> String {
    craft_vectordrawable::write::color(wire_color(c))
}

fn item(it: Item, depth: usize) -> wire::Item {
    let kind = match it.kind {
        ItemKind::Path(p) => wire::ItemKind::Path(wire::Path {
            name: p.name,
            data: p.data,
            even_odd: p.even_odd,
            fill: p.fill.map(|f| match f {
                Fill::Solid(c) => wire::Fill::Solid(wire_color(c)),
                Fill::Gradient(g) => wire::Fill::Gradient(wire::Gradient {
                    kind: match g.kind {
                        GradKind::Linear { start, end } => wire::GradientKind::Linear { start, end },
                        GradKind::Radial { center, radius } => wire::GradientKind::Radial { center, radius },
                        GradKind::Sweep { center } => wire::GradientKind::Sweep { center },
                    },
                    stops: g.stops.into_iter().map(|(offset, c)| (offset, wire_color(c))).collect(),
                }),
            }),
            stroke: p.stroke.map(|s| wire::Stroke {
                color: wire_color(s.color),
                width: s.width,
                miter: s.miter,
                cap: match s.cap {
                    "round" => wire::Cap::Round,
                    "square" => wire::Cap::Square,
                    _ => wire::Cap::Butt,
                },
                join: match s.join {
                    "round" => wire::Join::Round,
                    "bevel" => wire::Join::Bevel,
                    _ => wire::Join::Miter,
                },
            }),
        }),
        ItemKind::Group(g) => wire::ItemKind::Group(wire::Group {
            name: g.name,
            clip: g.clip,
            transform: g.xf.map(|x| wire::GroupTransform { translate: x.translate, rotation: x.rotation, scale: x.scale }),
            // Bound adaptation as well as writing. Production trees are bounded by the source
            // layer guard and generated wrappers; don't recurse beyond that invariant.
            children: if depth < XML_DEPTH { g.children.into_iter().map(|it| item(it, depth + 1)).collect() } else { Vec::new() },
        }),
    };
    wire::Item { kind, bbox: it.bbox }
}

pub(crate) fn document(width: f64, height: f64, items: Vec<Item>) -> String {
    let items: Vec<_> = items.into_iter().map(|it| item(it, 1)).collect();
    craft_vectordrawable::write::document_with_depth_limit(width, height, &items, XML_DEPTH)
}

#[cfg(test)]
#[path = "xml_legacy_test.rs"]
mod legacy;
#[cfg(test)]
#[path = "xml_parity_tests.rs"]
mod parity_tests;
