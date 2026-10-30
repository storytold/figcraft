use super::layer::{cap_name, own_text, svg_path, valid_size};
use super::{Affine, Kv, Mapper, NO_STYLE, Obj, Point, Value, View, generated_record, guid, json, matrix, network, num, rows};
use figcraft_doc::geom::kurbo::{BezPath, PathEl, Rect, Shape};
use figcraft_doc::{RouteEnd, Routing, Side, route_points};

impl<'a> Mapper<'a> {
    /// A connector's end caps, and when it has no saved outline or network its line between its
    /// ends (free points, or a sibling layer): straight to the point given on the layer (its
    /// centre unless given), or elbowed or curved from a side of it, as a board connector runs.
    pub(super) fn connector(&mut self, o: &mut Obj, v: &View<'a>, space: Affine) {
        for (from, to) in [("connectorStartCap", "strokeCapStart"), ("connectorEndCap", "strokeCapEnd")] {
            if let Some(c) = v.str(from) {
                o.insert(to.into(), json!(cap_name(c)));
            }
        }
        if connector_has_saved_label(v) {
            self.warn("connector labels were skipped");
        }
        if ["vectorNetwork", "vectorNetworkRef", "fillGeometry", "strokeGeometry"].iter().any(|k| o.contains_key(*k)) {
            return;
        }
        let routing = match v.str("connectorLineStyle") {
            Some("ELBOWED") => Routing::Elbow,
            Some("CURVED") => Routing::Curved,
            _ => Routing::Straight,
        };
        let parent = v.get("parentIndex").and_then(|p| p.get("guid")).and_then(guid);
        let end = |k: &str| -> Option<RouteEnd> {
            let e = v.get(k)?;
            let end = match e.get("endpointNodeID").and_then(guid).filter(|g| g != NO_STYLE) {
                Some(id) => {
                    let n = *self.nodes.get(&id)?;
                    // Only siblings share the connector's coordinates.
                    if n.get("parentIndex").and_then(|p| p.get("guid")).and_then(guid) != parent {
                        return None;
                    }
                    let (w, h) = n.get("size").map_or((0.0, 0.0), |s| (num(s, "x").unwrap_or(0.0), num(s, "y").unwrap_or(0.0)));
                    let at = e.get("relativePosition").and_then(|r| Some((num(r, "x")?, num(r, "y")?)));
                    let t = n.get("transform").and_then(matrix).unwrap_or_default();
                    match routing {
                        Routing::Straight => {
                            let (rx, ry) = at.unwrap_or((0.5, 0.5));
                            RouteEnd::Point(t * Point::new(w * rx, h * ry))
                        }
                        _ => RouteEnd::Box(t.transform_rect_bbox(Rect::new(0.0, 0.0, w, h)), side(e, at)),
                    }
                }
                None => RouteEnd::Point(e.get("position").and_then(|p| Some(Point::new(num(p, "x")?, num(p, "y")?)))?),
            };
            let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
            let ok = match end {
                RouteEnd::Point(p) => finite(p),
                RouteEnd::Box(b, _) => finite(b.origin()) && finite(Point::new(b.x1, b.y1)),
            };
            ok.then_some(end)
        };
        let Some(pts) = end("connectorStart").zip(end("connectorEnd")).and_then(|(a, b)| route_points(a, b, routing)) else {
            self.warn("connectors whose ends can't be placed are empty");
            return;
        };
        if routing != Routing::Straight {
            self.warn("elbowed and curved connectors without a saved path follow FigCraft's own routing");
        }
        // A curved route is one curve: its ends and the two control points between them.
        let (path, vertices) = match (routing, pts.as_slice()) {
            (Routing::Curved, [p0, c0, c1, p1]) => {
                let mut path = BezPath::new();
                path.move_to(*p0);
                path.curve_to(*c0, *c1, *p1);
                (path, vec![*p0, *p1])
            }
            _ => (pts.iter().enumerate().map(|(i, p)| if i == 0 { PathEl::MoveTo(*p) } else { PathEl::LineTo(*p) }).collect(), pts.clone()),
        };
        let b = path.bounding_box();
        let (x0, y0) = (b.x0, b.y0);
        o.insert("relativeTransform".into(), rows(space * Affine::translate((x0, y0))));
        o.insert("size".into(), json!({"x": b.width(), "y": b.height()}));
        let segments: Vec<Value> = match (routing, pts.as_slice()) {
            (Routing::Curved, [p0, c0, c1, p1]) => vec![json!({"a": 0, "b": 1, "ta": [c0.x - p0.x, c0.y - p0.y], "tb": [c1.x - p1.x, c1.y - p1.y]})],
            _ => (1..vertices.len()).map(|i| json!({"a": i - 1, "b": i})).collect(),
        };
        let vertices: Vec<Value> = vertices.iter().map(|p| json!({"x": p.x - x0, "y": p.y - y0})).collect();
        o.insert("vectorNetwork".into(), json!({"vertices": vertices, "segments": segments}));
    }

    pub(super) fn connector_snapshot_supported(&self, v: &View<'a>) -> bool {
        if v.str("connectorType") != Some("MANUAL")
            || v.get("connectorVersion").and_then(Kv::f64) != Some(1.0)
            || v.str("connectorLineStyle") != Some("STRAIGHT")
            || v.get("derivedImmutableFrameData").and_then(|data| data.get("version")).and_then(Kv::f64) != Some(1.0)
        {
            return false;
        }
        let Some(line) = generated_record(v, "derivedImmutableFrameData", 0) else { return false };
        if line.get("transform").and_then(matrix).is_none() || valid_size(line.get("size")).is_none() {
            return false;
        }
        let geometry = ["fillGeometry", "strokeGeometry"].iter().any(|key| {
            line.get(key).map(Kv::list).unwrap_or_default().iter().any(|path| {
                num(path, "commandsBlob")
                    .and_then(|index| self.blobs.get(index as usize))
                    .and_then(|blob| svg_path(blob))
                    .is_some_and(|data| !data.is_empty())
            })
        });
        let network = line
            .get("vectorData")
            .and_then(|data| num(data, "vectorNetworkBlob"))
            .and_then(|index| self.blobs.get(index as usize))
            .and_then(|blob| network::decode(blob))
            .is_some_and(|network| !network.segments.is_empty());
        (geometry || network) && generated_record(v, "nodeGenerationData", 0).is_some()
    }

    pub(super) fn connector_snapshot_flatten(&mut self, out: &mut Obj, v: &View<'a>, rest_id: &str, space: Affine) {
        let parent = space * v.get("transform").and_then(matrix).unwrap_or_default();
        let Some(vector) = self.connector_snapshot_vector(v, rest_id, parent, false) else {
            self.warn("connector snapshot vector data is missing; connector was left empty");
            return;
        };
        for key in [
            "size",
            "relativeTransform",
            "fills",
            "strokes",
            "strokeWeight",
            "strokeAlign",
            "strokeJoin",
            "strokeDashes",
            "strokeCap",
            "strokeCapStart",
            "strokeCapEnd",
            "styles",
            "fillGeometry",
            "strokeGeometry",
            "vectorNetwork",
        ] {
            if let Some(value) = vector.get(key) {
                out.insert(key.to_string(), value.clone());
            }
        }
        if vector.get("visible").and_then(Value::as_bool) == Some(false) {
            out.insert("visible".into(), json!(false));
        }
        if let Some(line_opacity) = vector.get("opacity").and_then(Value::as_f64) {
            let root_opacity = out.get("opacity").and_then(Value::as_f64).unwrap_or(1.0);
            out.insert("opacity".into(), json!(root_opacity * line_opacity));
        }
        if connector_has_saved_label(v) {
            let reason = if self.count >= self.budget() { "node budget" } else { "depth limit" };
            self.warn(format!("connector label was skipped: {reason} prevents nested snapshot children"));
        }
    }

    /// Build the saved vector child or flattened terminal connector from the same child-only view.
    fn connector_snapshot_vector(&mut self, v: &View<'a>, rest_id: &str, space: Affine, nested: bool) -> Option<Obj> {
        let (Some(line), Some(line_style)) = (generated_record(v, "derivedImmutableFrameData", 0), generated_record(v, "nodeGenerationData", 0))
        else {
            return None;
        };
        let mut vector = Obj::new();
        vector.insert("id".into(), json!(if nested { format!("{rest_id};vector") } else { rest_id.to_string() }));
        vector.insert("type".into(), json!("VECTOR"));
        if let Some(name) = v.str("name") {
            vector.insert("name".into(), json!(if nested { format!("{name} vector") } else { name.to_string() }));
        }
        let line_view = View::of(line).with([line_style]);
        self.common(&mut vector, &line_view, "VECTOR", space, None);
        self.geometry(&mut vector, &line_view);
        for (from, to) in [("connectorStartCap", "strokeCapStart"), ("connectorEndCap", "strokeCapEnd")] {
            if let Some(cap) = v.str(from) {
                vector.entry(to).or_insert_with(|| json!(cap_name(cap)));
            }
        }
        Some(vector)
    }

    /// Snapshot the straight, manual connector contract carried by its generated children.
    pub(crate) fn connector_snapshot_children(&mut self, v: &View<'a>, rest_id: &str) -> Vec<Value> {
        if self.count >= self.budget() {
            self.warn(format!("the file has more than {} layers; connector snapshot children were skipped", self.max_nodes));
            return Vec::new();
        }
        let Some(vector) = self.connector_snapshot_vector(v, rest_id, Affine::IDENTITY, true) else {
            self.warn("connector snapshot vector data is missing; connector was left empty");
            return Vec::new();
        };
        let mut children = Vec::new();
        children.push(Value::Object(vector));
        self.count += 1;

        let Some(text_frame) = generated_record(v, "derivedImmutableFrameData", 1) else {
            if connector_has_saved_label(v) {
                self.warn("connector label was skipped: derived text bounds are missing");
            }
            return children;
        };
        let Some(background) = generated_record(v, "derivedImmutableFrameData", 2) else {
            if connector_has_saved_label(v) || valid_size(text_frame.get("size")).is_some_and(|(w, h)| w > 0.0 && h > 0.0) {
                self.warn("connector label was skipped: derived background bounds are missing");
            }
            return children;
        };
        let Some(text_style) = generated_record(v, "nodeGenerationData", 1) else {
            if valid_size(text_frame.get("size")).is_some_and(|(w, h)| w > 0.0 && h > 0.0) {
                self.warn("connector label was skipped: text style data is missing");
            }
            return children;
        };
        let text_view = View::of(text_frame).with([text_style]);
        let Some(characters) = text_view.get("textData").and_then(|d| d.get("characters")).and_then(Kv::str).filter(|s| !s.trim().is_empty()) else {
            return children;
        };
        let Some(text_transform) = text_frame.get("transform").and_then(matrix) else {
            self.warn("connector label was skipped: text transform is invalid");
            return children;
        };
        let Some(background_transform) = background.get("transform").and_then(matrix) else {
            self.warn("connector label was skipped: background transform is invalid");
            return children;
        };
        let Some(text_size) = valid_size(text_frame.get("size")) else {
            self.warn("connector label was skipped: text bounds are invalid");
            return children;
        };
        let Some(background_size) = valid_size(background.get("size")) else {
            self.warn("connector label was skipped: background bounds are invalid");
            return children;
        };
        let composed_text_transform = background_transform * text_transform;
        if !composed_text_transform.as_coeffs().iter().all(|coefficient| coefficient.is_finite()) {
            self.warn("connector label was skipped: composed text transform is invalid");
            return children;
        }

        let Some(background_style) = generated_record(v, "nodeGenerationData", 2) else {
            self.warn("connector label background was skipped: style data is missing");
            self.add_connector_text(&mut children, rest_id, characters, text_size, composed_text_transform, &text_view);
            return children;
        };
        let background_view = View::of(background).with([background_style]);
        if !background_view.list("fillPaints").is_empty() {
            if self.count >= self.budget() {
                self.warn(format!("the file has more than {} layers; connector label background was skipped", self.max_nodes));
            } else {
                let mut rect = Obj::new();
                rect.insert("id".into(), json!(format!("{rest_id};label-background")));
                rect.insert("type".into(), json!("RECTANGLE"));
                rect.insert("name".into(), json!("connector label background"));
                self.common(&mut rect, &background_view, "RECTANGLE", Affine::IDENTITY, None);
                rect.insert("size".into(), json!({"x": background_size.0, "y": background_size.1}));
                rect.insert("relativeTransform".into(), rows(background_transform));
                rect.insert("clipsContent".into(), json!(false));
                children.push(Value::Object(rect));
                self.count += 1;
            }
        }
        self.add_connector_text(&mut children, rest_id, characters, text_size, composed_text_transform, &text_view);
        children
    }

    fn add_connector_text(
        &mut self,
        children: &mut Vec<Value>,
        rest_id: &str,
        characters: &str,
        size: (f64, f64),
        transform: Affine,
        view: &View<'a>,
    ) {
        if self.count >= self.budget() {
            self.warn(format!("the file has more than {} layers; connector label was skipped", self.max_nodes));
            return;
        }
        let mut text = Obj::new();
        text.insert("id".into(), json!(format!("{rest_id};label-text")));
        text.insert("type".into(), json!("TEXT"));
        text.insert("name".into(), json!(characters));
        text.insert("size".into(), json!({"x": size.0, "y": size.1}));
        self.common(&mut text, view, "TEXT", Affine::IDENTITY, None);
        text.insert("relativeTransform".into(), rows(transform));
        self.text(&mut text, view);
        children.push(Value::Object(text));
        self.count += 1;
    }
}

pub(super) fn connector_has_saved_label(v: &View<'_>) -> bool {
    own_text(v).is_some()
        || generated_record(v, "nodeGenerationData", 1)
            .and_then(|r| r.get("textData"))
            .and_then(|d| d.get("characters"))
            .and_then(Kv::str)
            .is_some_and(|s| !s.trim().is_empty())
}

/// The side of its layer a connector end leaves from: the `magnet` the file names, else the edge
/// its point on the layer (`relativePosition`, 0–1) lies on, else the side facing the other end.
fn side(e: &Kv<'_>, at: Option<(f64, f64)>) -> Side {
    match e.get("magnet").and_then(Kv::str) {
        Some("TOP") => Side::Top,
        Some("RIGHT") => Side::Right,
        Some("BOTTOM") => Side::Bottom,
        Some("LEFT") => Side::Left,
        _ => match at {
            Some((x, _)) if x <= 0.0 => Side::Left,
            Some((x, _)) if x >= 1.0 => Side::Right,
            Some((_, y)) if y <= 0.0 => Side::Top,
            Some((_, y)) if y >= 1.0 => Side::Bottom,
            _ => Side::Auto,
        },
    }
}
