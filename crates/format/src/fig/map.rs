//! Decoded `.fig` message → REST API file JSON, which [`crate::rest`] then imports. The mapping
//! is `craft_fig::rest`; FigCraft gives it its own connector routing and text measure, so an
//! imported connector runs as one drawn here, and takes the networks it hands over.

use craft_fig::rest::{self, Geometry, RouteEnd, Routing, SavedRuns, Side};
use figcraft_doc::geom::{Point, measure};
use figcraft_doc::{ConnectorPath, VectorNetwork};
use serde_json::Value;

use super::kiwi::Kv;

/// FigCraft's board routing and text measure, for the mapping.
struct FigGeometry;

fn end(e: RouteEnd) -> figcraft_doc::RouteEnd {
    match e {
        RouteEnd::Point(p) => figcraft_doc::RouteEnd::Point(p),
        RouteEnd::Box(b, s) => figcraft_doc::RouteEnd::Box(b, side(s)),
    }
}

fn side(s: Side) -> figcraft_doc::Side {
    match s {
        Side::Auto => figcraft_doc::Side::Auto,
        Side::Top => figcraft_doc::Side::Top,
        Side::Right => figcraft_doc::Side::Right,
        Side::Bottom => figcraft_doc::Side::Bottom,
        Side::Left => figcraft_doc::Side::Left,
    }
}

fn side_back(s: figcraft_doc::Side) -> Side {
    match s {
        figcraft_doc::Side::Auto => Side::Auto,
        figcraft_doc::Side::Top => Side::Top,
        figcraft_doc::Side::Right => Side::Right,
        figcraft_doc::Side::Bottom => Side::Bottom,
        figcraft_doc::Side::Left => Side::Left,
    }
}

impl Geometry for FigGeometry {
    fn route(&self, from: RouteEnd, to: RouteEnd, routing: Routing, saved: Option<&SavedRuns>) -> Option<Vec<Point>> {
        let routing = match routing {
            Routing::Straight => figcraft_doc::Routing::Straight,
            Routing::Elbow => figcraft_doc::Routing::Elbow,
            Routing::Curved => figcraft_doc::Routing::Curved,
        };
        let path = saved.map(|s| ConnectorPath::Runs { across: s.across, at: s.at.clone() });
        figcraft_doc::route_along(end(from), end(to), routing, path.as_ref())
    }

    fn attach(&self, from: RouteEnd, to: RouteEnd) -> ((Point, Option<Side>), (Point, Option<Side>)) {
        let (a, b) = (end(from), end(to));
        let ((p0, p1), (s0, s1)) = (figcraft_doc::attach_points(a, b), figcraft_doc::attached_sides(a, b));
        ((p0, s0.map(side_back)), (p1, s1.map(side_back)))
    }

    fn text_path_measurable(&self, d: &str) -> bool {
        let path = figcraft_doc::geom::svg_path(d).unwrap_or_default();
        measure::polylines(&path).and_then(|lines| lines.first().map(|line| line.length() > 1e-9)).unwrap_or(false)
    }
}

/// The REST API file JSON for message `msg`, with the warnings about what was skipped and the
/// vector networks its layers name by `vectorNetworkRef` (for [`crate::rest::import_with_networks`]).
#[cfg(test)]
pub(super) fn to_rest(msg: &Kv, name: &str, max_nodes: usize) -> (Value, Vec<String>, Vec<Option<VectorNetwork>>) {
    to_rest_for(msg, name, max_nodes, false)
}

/// Map a FigJam whiteboard's stickies as native board items when `board` is true.
pub(super) fn to_rest_for(msg: &Kv, name: &str, max_nodes: usize, board: bool) -> (Value, Vec<String>, Vec<Option<VectorNetwork>>) {
    let options = rest::Options { max_nodes, geometry: &FigGeometry, outlined_stroke: Some(super::export::STASH), board };
    let mapped = rest::to_rest(msg, name, &options);
    // Each network passes the importer's own checks, as the JSON's `vectorNetwork` does.
    let networks = mapped.networks.into_iter().map(|n| serde_json::to_value(n?).ok().and_then(|v| crate::rest::network(&v))).collect();
    (mapped.json, mapped.warnings, networks)
}

/// REST importer warnings reworded for a `.fig` import (`None`: drop it).
pub(super) fn adapt_warning(w: String) -> Option<String> {
    if w.contains("geometry=paths") {
        return None;
    }
    Some(w.replace("the variables JSON", "the file").replace("the file JSON", "the file"))
}
