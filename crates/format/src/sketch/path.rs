//! Sketch curve points → vector networks; shape groups → shapes and boolean operations.
//!
//! A path's `points` are `curvePoint`s whose `point`, `curveFrom` (the handle leaving the point)
//! and `curveTo` (the handle arriving at it) are `"{x, y}"` strings in 0–1 of the layer's frame;
//! values outside that range are allowed. Sketch 51 and older wrap every shape in a `shapeGroup`
//! that carries the style; its children combine left to right, each with its own
//! `booleanOperation` (0 union, 1 subtract, 2 intersect, 3 difference). Children with −1 (none)
//! don't combine: they join the path before them into one compound path, filled by the winding
//! rule (`windingRule` on the layer, 1 even-odd: inner paths are holes). Sketch fills an open path
//! as if it were closed.

use figcraft_doc::geom::{Affine, polygon_points, star_points};
use figcraft_doc::{Region, Segment, VectorNetwork, Vertex, WindingRule};
use serde_json::{Map as JsonMap, Value, json};

use super::tree::{Cx, base, layer, layers, local, size};

/// Farthest a point or handle may be from its layer (the REST importer's own range).
const MAX_COORD: f64 = 1e7;
/// Most points read from one path.
const MAX_POINTS: usize = 100_000;

/// What a path layer becomes.
pub(crate) enum Shape {
    Vector(VectorNetwork),
    Star { count: u32, ratio: f64 },
    Polygon { count: u32 },
}

/// `"{x, y}"` → `(x, y)`, both finite.
pub(crate) fn parse_point(s: &str) -> Option<(f64, f64)> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (x, y) = inner.split_once(',')?;
    let n = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    Some((n(x)?, n(y)?))
}

/// A layer's winding rule: on the layer (Sketch 51 and older), else in its style.
pub(crate) fn winding(l: &Value) -> WindingRule {
    let rule = l.get("windingRule").or_else(|| l.get("style").and_then(|s| s.get("windingRule")));
    match rule.and_then(Value::as_i64) {
        Some(1) => WindingRule::EvenOdd,
        _ => WindingRule::NonZero,
    }
}

/// Whether a style shows any fill or border.
fn shows(style: Option<&Value>, key: &str) -> bool {
    style
        .and_then(|s| s.get(key))
        .and_then(Value::as_array)
        .is_some_and(|l| l.iter().any(|f| f.get("isEnabled").and_then(Value::as_bool) != Some(false)))
}

/// The network of a path layer of `w`×`h` (`None` when it has no usable points). `fill_open`: an
/// open path gets a closing segment and a fill region, as Sketch fills it.
pub(crate) fn network(cx: &mut Cx, l: &Value, w: f64, h: f64, fill_open: bool) -> Option<VectorNetwork> {
    let pts = l.get("points").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if pts.len() > MAX_POINTS {
        cx.warn(format!("paths with more than {MAX_POINTS} points were cut"));
    }
    // Far points stay within the importer's coordinate range (a product could overflow).
    let fit = |v: f64| if v.is_finite() { v.clamp(-MAX_COORD, MAX_COORD) } else { 0.0 };
    let at = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).and_then(parse_point).map(|(x, y)| (fit(x * w), fit(y * h)));
    let mut vertices = Vec::new();
    // Per vertex: its handles (relative to it) when it has them.
    let mut handles: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for p in pts.iter().take(MAX_POINTS) {
        let Some((x, y)) = at(p, "point") else {
            cx.warn("path points that couldn't be read were skipped");
            continue;
        };
        let curved = p.get("curveMode").and_then(Value::as_i64).is_some_and(|m| m != 1);
        let has = |flag: &str| p.get(flag).and_then(Value::as_bool).unwrap_or(curved);
        let handle = |flag: &str, key: &str| if has(flag) { at(p, key).map_or((0.0, 0.0), |(hx, hy)| (hx - x, hy - y)) } else { (0.0, 0.0) };
        handles.push((handle("hasCurveFrom", "curveFrom"), handle("hasCurveTo", "curveTo")));
        let mut v = Vertex::new(x, y);
        v.radius = p.get("cornerRadius").and_then(Value::as_f64).filter(|r| r.is_finite()).map_or(0.0, |r| r.clamp(0.0, 1e6));
        vertices.push(v);
    }
    let n = vertices.len();
    if n == 0 {
        return None;
    }
    let open = !l.get("isClosed").and_then(Value::as_bool).unwrap_or(false);
    let closed = n > 1 && (!open || (fill_open && n > 2));
    let count = if closed { n } else { n - 1 };
    let segments: Vec<Segment> = (0..count)
        .map(|i| {
            let j = (i + 1) % n;
            // The edge that closes an open path for its fill is straight.
            let closing = open && j == 0;
            let ta = if closing { (0.0, 0.0) } else { handles.get(i).map_or((0.0, 0.0), |h| h.0) };
            let tb = if closing { (0.0, 0.0) } else { handles.get(j).map_or((0.0, 0.0), |h| h.1) };
            Segment { a: i, b: j, ta, tb }
        })
        .collect();
    let winding = winding(l);
    let regions = if closed { vec![Region { loops: vec![(0..segments.len()).collect()], winding }] } else { Vec::new() };
    Some(VectorNetwork { vertices, segments, regions })
}

/// Rectangle `l` with angled (corner style 2), inside-square (3) or inside-arc (4) corners in its
/// style (Sketch 2025 and newer) as an edited rectangle whose points draw that outline, the
/// corners' `radii` (top left, top right, bottom right, bottom left; repeating) being how far
/// each cut reaches along both edges. `None` for any other layer.
pub(crate) fn cut_corners(l: &Value) -> Option<Value> {
    if l.get("_class").and_then(Value::as_str) != Some("rectangle") || l.get("edited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let c = l.get("style")?.get("corners")?;
    let style = c.get("style").and_then(Value::as_i64).filter(|s| (2..=4).contains(s))?;
    let radii: Vec<f64> = c.get("radii")?.as_array()?.iter().filter_map(Value::as_f64).filter(|r| r.is_finite()).take(4).collect();
    if radii.iter().all(|r| *r <= 0.0) {
        return None;
    }
    let (w, h) = size(l);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    // Arc handles: a quarter circle's.
    const K: f64 = 0.552_284_749_8;
    let fmt = |(x, y): (f64, f64)| format!("{{{}, {}}}", x / w, y / h);
    let curve = |p: (f64, f64), from: Option<(f64, f64)>, to: Option<(f64, f64)>| {
        json!({"_class": "curvePoint", "cornerRadius": 0, "point": fmt(p), "curveMode": if from.or(to).is_some() { 4 } else { 1 },
            "hasCurveFrom": from.is_some(), "hasCurveTo": to.is_some(), "curveFrom": fmt(from.unwrap_or(p)), "curveTo": fmt(to.unwrap_or(p))})
    };
    // Corners clockwise from the top left, each with the directions to the edge before and after.
    let corners = [
        ((0.0, 0.0), (0.0, 1.0), (1.0, 0.0)),
        ((w, 0.0), (-1.0, 0.0), (0.0, 1.0)),
        ((w, h), (0.0, -1.0), (-1.0, 0.0)),
        ((0.0, h), (1.0, 0.0), (0.0, -1.0)),
    ];
    let mut points = Vec::new();
    for (i, ((cx, cy), (px, py), (nx, ny))) in corners.into_iter().enumerate() {
        let r = radii.get(i % radii.len()).copied().unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
        if r <= 0.0 {
            points.push(curve((cx, cy), None, None));
            continue;
        }
        let a = (cx + r * px, cy + r * py);
        let b = (cx + r * nx, cy + r * ny);
        match style {
            2 => points.extend([curve(a, None, None), curve(b, None, None)]),
            3 => points.extend([curve(a, None, None), curve((cx + r * (px + nx), cy + r * (py + ny)), None, None), curve(b, None, None)]),
            // A quarter circle about the corner, bulging into the shape.
            _ => points
                .extend([curve(a, Some((a.0 + K * r * nx, a.1 + K * r * ny)), None), curve(b, None, Some((b.0 + K * r * px, b.1 + K * r * py)))]),
        }
    }
    let mut out = l.clone();
    out["edited"] = json!(true);
    out["isClosed"] = json!(true);
    out["points"] = Value::Array(points);
    if let Some(st) = out.get_mut("style").and_then(Value::as_object_mut) {
        st.remove("corners");
    }
    Some(out)
}

/// A path, star, polygon or triangle layer: a star or polygon while its points are still the
/// generated ones (same as FigCraft's, within half a percent), else an editable vector.
pub(crate) fn shape(cx: &mut Cx, l: &Value, w: f64, h: f64) -> Option<Shape> {
    // Inside a shape group the group's fill decides; a shape of its own, its style.
    let style = l.get("style");
    let filled = cx.operand_fill.unwrap_or_else(|| shows(style, "fills"));
    let open = !l.get("isClosed").and_then(Value::as_bool).unwrap_or(false);
    if open && filled && (shows(style, "borders") || cx.operand_border) {
        cx.warn("open paths with both a fill and a border were closed; the border runs along the closing edge too");
    }
    let net = network(cx, l, w, h, filled)?;
    let class = l.get("_class").and_then(Value::as_str).unwrap_or("");
    let count = l.get("numberOfPoints").and_then(Value::as_u64).filter(|c| (3..=120).contains(c)).and_then(|c| u32::try_from(c).ok());
    let plain = net.vertices.iter().all(|v| v.radius == 0.0) && net.segments.iter().all(|s| s.ta == (0.0, 0.0) && s.tb == (0.0, 0.0));
    let matches = |pts: Vec<figcraft_doc::geom::Point>| {
        let tol = 0.005 * w.max(h) + 1e-6;
        pts.len() == net.vertices.len() && pts.iter().zip(&net.vertices).all(|(p, v)| (p.x - v.x).abs() <= tol && (p.y - v.y).abs() <= tol)
    };
    match (class, count) {
        ("star", Some(count)) if plain => {
            let ratio = l.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite() && *r > 0.0).map_or(0.5, |r| r.min(1.0));
            if matches(star_points(w, h, count, ratio)) {
                return Some(Shape::Star { count, ratio });
            }
        }
        ("polygon", Some(count)) if plain && matches(polygon_points(w, h, count)) => return Some(Shape::Polygon { count }),
        _ => {}
    }
    Some(Shape::Vector(net))
}

/// Write `shape` into `node` (its type and geometry).
pub(crate) fn put_shape(cx: &mut Cx, shape: Shape, node: &mut JsonMap<String, Value>) {
    match shape {
        Shape::Vector(net) => {
            node.insert("type".into(), json!("VECTOR"));
            node.insert("vectorNetworkRef".into(), json!(cx.networks.len()));
            cx.networks.push(Some(net));
        }
        Shape::Star { count, ratio } => {
            node.insert("type".into(), json!("STAR"));
            node.insert("pointCount".into(), json!(count));
            node.insert("innerRadius".into(), json!(ratio));
        }
        Shape::Polygon { count } => {
            node.insert("type".into(), json!("REGULAR_POLYGON"));
            node.insert("pointCount".into(), json!(count));
        }
    }
}

/// A `shapeGroup`: its only child carrying the group's place, name and style; its children joined
/// into one compound path when none of them combines; else boolean operations over runs of
/// children (a run: a child with an operation and the ones without that follow it), mixed
/// operations nesting from the left.
pub(crate) fn shape_group(cx: &mut Cx, l: &Value, parent: Affine, depth: usize, root: bool) -> Option<Value> {
    let (w, h) = size(l);
    let world = parent * local(l, w, h);
    let kids = layers(l);
    let saved = (cx.operand_fill, cx.operand_border);
    cx.operand_border = shows(l.get("style"), "borders");
    let out = if let [only] = kids {
        cx.operand_fill = Some(shows(l.get("style"), "fills"));
        single(cx, l, only, parent, world, depth, root)
    } else {
        // Operands of a boolean operation (or a compound path) give area: open ones close.
        cx.operand_fill = Some(true);
        combined(cx, l, kids, parent, world, (w, h), depth, root)
    };
    (cx.operand_fill, cx.operand_border) = saved;
    out
}

/// A shape group of one shape: that shape in the group's place, with its name and style.
fn single(cx: &mut Cx, l: &Value, only: &Value, parent: Affine, world: Affine, depth: usize, root: bool) -> Option<Value> {
    let mut node = layer(cx, only, world, depth + 1, false)?;
    let (cw, ch) = size(only);
    let child_world = world * local(only, cw, ch);
    let group = base(l, "GROUP", parent, child_world, cw, ch, root);
    if let Some(m) = node.as_object_mut() {
        for key in ["id", "name", "relativeTransform", "absoluteBoundingBox", "constraints", "locked"] {
            match group.get(key) {
                Some(v) => m.insert(key.into(), v.clone()),
                None => m.remove(key),
            };
        }
        let visible = group.get("visible").and_then(Value::as_bool).unwrap_or(true) && m.get("visible").and_then(Value::as_bool).unwrap_or(true);
        m.insert("visible".into(), json!(visible));
        if l.get("style").is_some() {
            super::style::clear(m);
            m.remove("styles");
            super::tree::styles(cx, l, cw, ch, m);
            super::tree::extras(cx, l, ch, m);
        }
        super::tree::mask(l, m);
    }
    Some(node)
}

/// A shape group of several shapes: one compound path, or boolean operations over runs.
fn combined(cx: &mut Cx, l: &Value, kids: &[Value], parent: Affine, world: Affine, (w, h): (f64, f64), depth: usize, root: bool) -> Option<Value> {
    // Runs: a child with an operation starts one, children without (−1) join it.
    let mut runs: Vec<Vec<&Value>> = Vec::new();
    for (i, c) in kids.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if i > 0 && c.get("booleanOperation").and_then(Value::as_i64).unwrap_or(-1) < 0 => run.push(c),
            _ => runs.push(vec![c]),
        }
    }
    let winding = winding(l);
    if let [run] = runs.as_slice()
        && let Some(net) = compound(cx, run, winding)
    {
        // No operations at all: the group is one path.
        let mut node = base(l, "VECTOR", parent, world, w, h, root);
        node.insert("vectorNetworkRef".into(), json!(cx.networks.len()));
        cx.networks.push(Some(net));
        super::tree::mask(l, &mut node);
        super::tree::styles(cx, l, w, h, &mut node);
        super::tree::extras(cx, l, h, &mut node);
        return Some(Value::Object(node));
    }
    let mut chain: Vec<Value> = Vec::new();
    let mut op = "UNION";
    for (i, run) in runs.iter().enumerate() {
        let operands: Vec<Value> = match run.as_slice() {
            [one] => layer(cx, one, world, depth + 2, false).into_iter().collect(),
            more => match compound(cx, more, winding) {
                Some(net) => {
                    let mut node = base(more.first().copied().unwrap_or(l), "VECTOR", world, world, w, h, false);
                    node.remove("id");
                    node.insert("vectorNetworkRef".into(), json!(cx.networks.len()));
                    cx.networks.push(Some(net));
                    vec![Value::Object(node)]
                }
                // Not all paths: each one an operand of its own.
                None => more.iter().filter_map(|c| layer(cx, c, world, depth + 2, false)).collect(),
            },
        };
        if operands.is_empty() {
            continue;
        }
        let this = run.first().map_or("UNION", |c| boolean_op(c));
        if i > 0 && this != op && chain.len() > 1 {
            let inner = synthetic(l, op, parent, world, w, h, std::mem::take(&mut chain));
            chain.push(inner);
        }
        if i > 0 {
            op = this;
        }
        chain.extend(operands);
    }
    if chain.is_empty() {
        return None;
    }
    let mut node = base(l, "BOOLEAN_OPERATION", parent, world, w, h, root);
    node.insert("booleanOperation".into(), json!(op));
    node.insert("children".into(), Value::Array(chain));
    super::tree::mask(l, &mut node);
    super::tree::styles(cx, l, w, h, &mut node);
    super::tree::extras(cx, l, h, &mut node);
    Some(Value::Object(node))
}

/// The paths `run` as one network in their group's space, filled by `winding`; `None` when one of
/// them isn't a path (a group, text, …).
fn compound(cx: &mut Cx, run: &[&Value], winding: WindingRule) -> Option<VectorNetwork> {
    const PATHS: [&str; 6] = ["shapePath", "rectangle", "oval", "star", "polygon", "triangle"];
    if run.iter().any(|c| !c.get("_class").and_then(Value::as_str).is_some_and(|k| PATHS.contains(&k))) {
        return None;
    }
    let mut out = VectorNetwork::default();
    let mut loops = Vec::new();
    for c in run {
        let (cw, ch) = size(c);
        let Some(net) = network(cx, c, cw, ch, true) else { continue };
        let xf = local(c, cw, ch);
        let [a, b, cc, d, _, _] = xf.as_coeffs();
        let lin = |(x, y): (f64, f64)| (a * x + cc * y, b * x + d * y);
        let (v0, s0) = (out.vertices.len(), out.segments.len());
        for v in net.vertices {
            let p = xf * figcraft_doc::geom::Point::new(v.x, v.y);
            out.vertices.push(Vertex { x: p.x, y: p.y, ..v });
        }
        for sg in net.segments {
            out.segments.push(Segment { a: sg.a + v0, b: sg.b + v0, ta: lin(sg.ta), tb: lin(sg.tb) });
        }
        for r in net.regions {
            loops.extend(r.loops.into_iter().map(|lp| lp.into_iter().map(|i| i + s0).collect::<Vec<_>>()));
        }
    }
    if out.vertices.is_empty() {
        return None;
    }
    if !loops.is_empty() {
        out.regions.push(Region { loops, winding });
    }
    Some(out)
}

/// A boolean operation the shape group doesn't have as a layer of its own: placed like the group.
fn synthetic(l: &Value, op: &str, parent: Affine, world: Affine, w: f64, h: f64, children: Vec<Value>) -> Value {
    let mut node = base(l, "BOOLEAN_OPERATION", parent, world, w, h, false);
    node.remove("id");
    let name = match op {
        "SUBTRACT" => "Subtract",
        "INTERSECT" => "Intersect",
        "EXCLUDE" => "Exclude",
        _ => "Union",
    };
    node.insert("name".into(), json!(name));
    // Placed where the group is: inside it, that's no move at all.
    node.insert("relativeTransform".into(), json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
    node.insert("booleanOperation".into(), json!(op));
    node.insert("children".into(), Value::Array(children));
    Value::Object(node)
}

fn boolean_op(l: &Value) -> &'static str {
    match l.get("booleanOperation").and_then(Value::as_i64) {
        Some(1) => "SUBTRACT",
        Some(2) => "INTERSECT",
        Some(3) => "EXCLUDE",
        _ => "UNION",
    }
}
