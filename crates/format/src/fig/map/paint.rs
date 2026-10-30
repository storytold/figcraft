use super::layer::cap_name;
use super::variables::alias;
use super::{Affine, Kv, Mapper, NO_STYLE, Obj, Point, Value, View, copy_num, copy_str, guid, json, matrix, num, rows, vec2};

fn filter_number(value: &Kv) -> Option<f64> {
    let number = match value {
        Kv::Float(value) => f64::from(*value),
        Kv::Int(value) => *value as f64,
        Kv::Uint(value) => *value as f64,
        _ => return None,
    };
    number.is_finite().then_some(number)
}

pub(super) fn color(c: &Kv) -> Value {
    let ch = |k: &str| num(c, k).unwrap_or(0.0);
    json!({"r": ch("r"), "g": ch("g"), "b": ch("b"), "a": num(c, "a").unwrap_or(1.0)})
}

impl<'a> Mapper<'a> {
    /// The style a layer links in `key`, by guid or by library asset (the file's copy of a library
    /// style), or in the older `legacy` field when `key` names none. An explicit NO_STYLE guid in
    /// `key` with no asset is a detach and wins over the legacy field.
    pub(super) fn style_link(&self, v: &View, key: &str, legacy: &str) -> Option<String> {
        let fallback = || v.get(legacy).and_then(guid).filter(|g| g != NO_STYLE);
        let Some(link) = v.get(key) else { return fallback() };
        match (self.resource_id(link), link.get("guid").and_then(guid)) {
            (Some(id), _) => Some(id),
            (None, Some(_)) => None,
            (None, None) => fallback(),
        }
    }

    /// The style node a layer links in `key` (or the older `legacy` field), recorded for the REST
    /// `styles` map under `slot`.
    /// A link to a style the file doesn't hold leaves the layer its own values (the colours it
    /// stored with it), with a note.
    pub(super) fn style(&mut self, v: &View<'a>, key: &str, legacy: &str, slot: &str, out: &mut Obj) -> Option<&'a Kv<'a>> {
        let found = self.style_link(v, key, legacy).and_then(|id| Some((self.nodes.get(&id).copied()?, id)));
        let Some((node, id)) = found.filter(|(n, _)| n.get("styleType").and_then(Kv::str).is_some()) else {
            let named = |l: &Kv| l.get("assetRef").is_some() || l.get("guid").and_then(guid).is_some_and(|g| g != NO_STYLE);
            if v.get(key).is_some_and(named) || v.get(legacy).and_then(guid).is_some_and(|g| g != NO_STYLE) {
                self.warn("links to shared styles that aren't in the file kept the layers' own values");
            }
            return None;
        };
        out.insert(slot.into(), json!(id));
        if !self.styles.contains_key(&id) {
            let name = node.get("name").and_then(Kv::str).unwrap_or("Style");
            let desc = node.get("styleDescription").or_else(|| node.get("description")).and_then(Kv::str).unwrap_or("");
            self.styles.insert(id, json!({"name": name, "description": desc}));
        }
        Some(node)
    }
}

pub(super) fn stroke(o: &mut Obj, v: &View) {
    copy_num(o, v, "strokeWeight", "strokeWeight");
    if let Some(a) = v.str("strokeAlign").filter(|a| *a != "OFFSET") {
        o.insert("strokeAlign".into(), json!(a));
    }
    copy_str(o, v, "strokeJoin", "strokeJoin");
    if let Some(cap) = v.str("strokeCap") {
        o.insert("strokeCap".into(), json!(cap_name(cap)));
    }
    // A layer written as the outline of its stroke keeps what it was as plugin data
    // (`outlinedStroke`, our extension of the file JSON).
    let (id, key) = crate::fig::export::STASH;
    let kept = v.list("pluginData").iter().find(|p| p.get("pluginID").and_then(Kv::str) == Some(id) && p.get("key").and_then(Kv::str) == Some(key));
    if let Some(original) = kept.and_then(|p| p.get("value")).and_then(Kv::str).and_then(|s| serde_json::from_str::<Value>(s).ok()) {
        o.insert("outlinedStroke".into(), original);
    }
    // A variable width: each point's reach on the two sides of the path (shares of the stroke
    // weight), as `[t, scale, bias]` (`strokeWidthProfile`, our extension of the file JSON).
    let profile: Vec<Value> = v
        .list("variableWidthPoints")
        .iter()
        .filter_map(|p| {
            let (left, right) = (num(p, "ascent")?.max(0.0), num(p, "descent")?.max(0.0));
            let bias = if left + right > 0.0 { (right - left) / (left + right) } else { 0.0 };
            Some(json!([num(p, "position")?, left + right, bias]))
        })
        .collect();
    if !profile.is_empty() {
        o.insert("strokeWidthProfile".into(), Value::Array(profile));
    }
    let dashes: Vec<f64> = v.list("dashPattern").iter().filter_map(Kv::f64).collect();
    if !dashes.is_empty() {
        o.insert("strokeDashes".into(), json!(dashes));
    }
    // A miter limit L bevels joins sharper than 2·asin(1/L).
    if let Some(l) = v.num("miterLimit").filter(|l| *l > 1.0) {
        o.insert("strokeMiterAngle".into(), json!((2.0 * (1.0 / l).asin()).to_degrees()));
    }
    if v.flag("borderStrokeWeightsIndependent") == Some(true) {
        // A side that isn't written has no border.
        let side = |k: &str| v.num(k).unwrap_or(0.0);
        o.insert(
            "individualStrokeWeights".into(),
            json!({"top": side("borderTopWeight"), "right": side("borderRightWeight"), "bottom": side("borderBottomWeight"), "left": side("borderLeftWeight")}),
        );
    }
}

/// Whether a paint's colour or one of its gradient stops is bound to a variable.
pub(super) fn paint_bound(p: &Kv) -> bool {
    alias(p.get("colorVar")).is_some() || p.get("stopsVar").map(Kv::list).unwrap_or_default().iter().any(|s| alias(s.get("colorVar")).is_some())
}

pub(crate) fn paint(p: &Kv, warn: &mut dyn FnMut(String)) -> Option<Value> {
    let ty = p.get("type").and_then(Kv::str)?;
    let mut o = Obj::new();
    o.insert("type".into(), json!(ty));
    if let Some(x) = num(p, "opacity") {
        o.insert("opacity".into(), json!(x));
    }
    if let Some(b) = p.get("visible").and_then(Kv::bool) {
        o.insert("visible".into(), json!(b));
    }
    if let Some(b) = p.get("blendMode").and_then(Kv::str) {
        o.insert("blendMode".into(), json!(b));
    }
    let t = p.get("transform").and_then(matrix);
    match ty {
        "SOLID" => {
            if let Some(c) = p.get("color") {
                o.insert("color".into(), color(c));
            }
            if let Some(var) = alias(p.get("colorVar")) {
                o.insert("boundVariables".into(), json!({"color": {"type": "VARIABLE_ALIAS", "id": var}}));
            }
        }
        _ if ty.starts_with("GRADIENT_") => {
            // The paint transform maps the layer's unit box to gradient space, where a linear
            // gradient runs from (0, ½) to (1, ½) and the others are centred on (½, ½).
            let inv = t.filter(|t| t.determinant().abs() > 1e-12).map_or(Affine::IDENTITY, Affine::inverse);
            let pts = if ty == "GRADIENT_LINEAR" { [(0.0, 0.5), (1.0, 0.5), (0.0, 1.0)] } else { [(0.5, 0.5), (1.0, 0.5), (0.5, 1.0)] };
            let handles: Vec<Value> = pts
                .iter()
                .map(|(x, y)| {
                    let q = inv * Point::new(*x, *y);
                    json!({"x": q.x, "y": q.y})
                })
                .collect();
            o.insert("gradientHandlePositions".into(), Value::Array(handles));
            // Stops bound to variables are listed again, each with its variable.
            let vars = p.get("stopsVar").map(Kv::list).unwrap_or_default();
            let stops: Vec<Value> = p
                .get("stops")
                .map(Kv::list)
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut stop = json!({"position": num(s, "position").unwrap_or(0.0), "color": s.get("color").map_or(Value::Null, color)});
                    if let Some(b) = bound(vars.get(i), &[("colorVar", "color")]) {
                        stop["boundVariables"] = b;
                    }
                    stop
                })
                .collect();
            o.insert("gradientStops".into(), Value::Array(stops));
        }
        "IMAGE" => {
            let hash: String =
                p.get("image").and_then(|i| i.get("hash")).and_then(Kv::bytes).unwrap_or_default().iter().map(|b| format!("{b:02x}")).collect();
            o.insert("imageRef".into(), json!(hash));
            let mode = p.get("imageScaleMode").and_then(Kv::str).unwrap_or("FILL");
            o.insert("scaleMode".into(), json!(mode));
            if let (Some(t), "STRETCH") = (t, mode) {
                o.insert("imageTransform".into(), rows(t));
            }
            if let Some(s) = num(p, "scale") {
                o.insert("scalingFactor".into(), json!(s));
            }
            if let Some(r) = num(p, "rotation") {
                o.insert("rotation".into(), json!(r));
            }
            match (p.get("filterColorAdjust"), p.get("paintFilter")) {
                (Some(_), Some(_)) => {
                    warn("image filters are ambiguous because both filter containers are present; filter conversion was skipped".into())
                }
                (Some(_), None) => warn("legacy image filter container is unsupported; filter conversion was skipped".into()),
                (None, Some(filters)) => {
                    let mut mapped = Obj::new();
                    let fields = [
                        ("exposure", "exposure"),
                        ("contrast", "contrast"),
                        ("vibrance", "saturation"),
                        ("temperature", "temperature"),
                        ("tint", "tint"),
                        ("highlights", "highlights"),
                        ("shadows", "shadows"),
                    ];
                    if let Kv::Object(values) = filters {
                        for (source, target) in fields {
                            if let Some(value) = filters.get(source) {
                                match filter_number(value) {
                                    Some(value) => {
                                        mapped.insert(target.into(), json!(value));
                                    }
                                    None => warn(format!("malformed image filter `{source}` was skipped")),
                                }
                            }
                        }
                        for (name, value) in values {
                            if fields.iter().any(|(known, _)| *known == *name) {
                                continue;
                            }
                            match filter_number(value) {
                                Some(0.0) => {}
                                Some(_) => warn(format!("nonzero unsupported image filter `{}` was skipped", crate::rest::props::clip_str(name))),
                                None => warn(format!("malformed unsupported image filter `{}` was skipped", crate::rest::props::clip_str(name))),
                            }
                        }
                    } else {
                        warn("malformed image paintFilter container was skipped".into());
                    }
                    if !mapped.is_empty() {
                        o.insert("filters".into(), Value::Object(mapped));
                    }
                }
                (None, None) => {}
            }
        }
        _ => {}
    }
    Some(Value::Object(o))
}

pub(super) fn has_bound_variables(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.contains_key("boundVariables") || object.values().any(has_bound_variables),
        Value::Array(values) => values.iter().any(has_bound_variables),
        _ => false,
    }
}

/// The variables bound to the fields of a paint stop, an effect or a layout grid, as its REST
/// `boundVariables`: `fields` pairs the format's field with the REST name.
pub(super) fn bound(v: Option<&Kv>, fields: &[(&str, &str)]) -> Option<Value> {
    let v = v?;
    let found: Obj =
        fields.iter().filter_map(|(from, to)| Some((to.to_string(), json!({"type": "VARIABLE_ALIAS", "id": alias(v.get(from))?})))).collect();
    (!found.is_empty()).then_some(Value::Object(found))
}

pub(super) fn effect(e: &Kv) -> Option<Value> {
    let ty = match e.get("type").and_then(Kv::str)? {
        "FOREGROUND_BLUR" => "LAYER_BLUR",
        "GRAIN" => "TEXTURE",
        t => t,
    };
    let mut o = Obj::new();
    o.insert("type".into(), json!(ty));
    for k in ["radius", "spread", "startRadius", "density", "opacity"] {
        if let Some(x) = num(e, k) {
            o.insert(k.into(), json!(x));
        }
    }
    for k in ["visible", "showShadowBehindNode", "clipToShape"] {
        if let Some(b) = e.get(k).and_then(Kv::bool) {
            o.insert(k.into(), json!(b));
        }
    }
    for k in ["blendMode", "noiseType"] {
        if let Some(s) = e.get(k).and_then(Kv::str) {
            o.insert(k.into(), json!(s));
        }
    }
    for k in ["color", "secondaryColor"] {
        if let Some(c) = e.get(k) {
            o.insert(k.into(), color(c));
        }
    }
    for (from, to) in [("offset", "offset"), ("startOffset", "startOffset"), ("endOffset", "endOffset"), ("noiseSize", "noiseSizeVector")] {
        if let Some(p) = e.get(from).and_then(vec2) {
            o.insert(to.into(), p);
        }
    }
    if e.get("blurOpType").and_then(Kv::str) == Some("PROGRESSIVE") {
        o.insert("blurType".into(), json!("PROGRESSIVE"));
    }
    let vars = [
        ("colorVar", "color"),
        ("radiusVar", "radius"),
        ("spreadVar", "spread"),
        ("xVar", "offsetX"),
        ("yVar", "offsetY"),
        ("startRadiusVar", "startRadius"),
    ];
    if let Some(b) = bound(Some(e), &vars) {
        o.insert("boundVariables".into(), b);
    }
    Some(Value::Object(o))
}

pub(super) fn export_setting(e: &Kv) -> Value {
    let c = e.get("constraint");
    let ty = match c.and_then(|c| c.get("type")).and_then(Kv::str) {
        Some("CONTENT_WIDTH") => "WIDTH",
        Some("CONTENT_HEIGHT") => "HEIGHT",
        _ => "SCALE",
    };
    json!({
        "format": e.get("imageType").and_then(Kv::str).unwrap_or("PNG"),
        "suffix": e.get("suffix").and_then(Kv::str).unwrap_or(""),
        "constraint": {"type": ty, "value": c.and_then(|c| num(c, "value")).unwrap_or(1.0)},
    })
}
