//! Sketch styles → REST paints, strokes, effects, opacity and blend modes.
//!
//! A layer's `style` lists `fills` and `borders` (each a colour, gradient or, for fills, an image
//! pattern, with its own `contextSettings` opacity and blend mode), `borderOptions` (dashes, caps,
//! joins), `shadows`, `innerShadows`, one `blur` and the layer's own `contextSettings`. Every
//! border has its own thickness and position; FigCraft strokes share one, so the first enabled
//! border's is used.

use serde_json::{Map as JsonMap, Value, json};

use super::path::parse_point;
use super::tree::{Cx, color};

/// Most paints or effects read from one list.
const MAX_LIST: usize = 64;

/// Write the paints, strokes, effects, opacity and blend mode of `style` into `node`, a layer of
/// `w`×`h`. Lists the style doesn't have are left as they are.
pub(crate) fn apply(cx: &mut Cx, style: &Value, w: f64, h: f64, node: &mut JsonMap<String, Value>) {
    let fills = fills(cx, style, w, h);
    if !fills.is_empty() {
        node.insert("fills".into(), Value::Array(fills));
    }
    borders(cx, style, w, h, node);
    let effects = effects(cx, style);
    if !effects.is_empty() {
        node.insert("effects".into(), Value::Array(effects));
    }
    if let Some(ctx) = style.get("contextSettings") {
        if let Some(o) = num(ctx, "opacity") {
            node.insert("opacity".into(), json!(o.clamp(0.0, 1.0)));
        }
        if let Some(b) = ctx.get("blendMode").and_then(Value::as_i64).filter(|b| *b != 0).and_then(blend) {
            node.insert("blendMode".into(), json!(b));
        }
    }
}

/// A style's fills as REST paints.
pub(crate) fn fills(cx: &mut Cx, style: &Value, w: f64, h: f64) -> Vec<Value> {
    // Tints (`layeringType` 1) colour the layers inside; they aren't paints of the layer.
    list(style, "fills").iter().filter(|f| f.get("layeringType").and_then(Value::as_i64) != Some(1)).filter_map(|f| paint(cx, f, w, h)).collect()
}

/// A style's borders as REST paints.
pub(crate) fn border_paints(cx: &mut Cx, style: &Value, w: f64, h: f64) -> Vec<Value> {
    list(style, "borders").iter().filter_map(|b| paint(cx, b, w, h)).collect()
}

/// Remove what [`apply`] writes, so another style can take its place.
pub(crate) fn clear(node: &mut JsonMap<String, Value>) {
    for key in [
        "fills",
        "strokes",
        "strokeWeight",
        "strokeAlign",
        "strokeDashes",
        "strokeCap",
        "strokeCapStart",
        "strokeCapEnd",
        "strokeJoin",
        "effects",
        "opacity",
        "blendMode",
    ] {
        node.remove(key);
    }
}

/// One fill or border as a REST paint; `None` for kinds FigCraft doesn't have (with a warning).
fn paint(cx: &mut Cx, f: &Value, w: f64, h: f64) -> Option<Value> {
    let mut p = match f.get("fillType").and_then(Value::as_i64).unwrap_or(0) {
        0 => json!({"type": "SOLID", "color": f.get("color").and_then(color).unwrap_or_else(|| json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}))}),
        1 => gradient(f.get("gradient")?, w, h)?,
        4 => {
            let Some(r) = f.get("image").and_then(|i| i.get("_ref")).and_then(Value::as_str) else {
                cx.warn("image fills stored inside the JSON were skipped");
                return None;
            };
            let mut p = json!({"type": "IMAGE", "imageRef": r});
            match f.get("patternFillType").and_then(Value::as_i64).unwrap_or(1) {
                0 => {
                    p["scaleMode"] = json!("TILE");
                    p["scalingFactor"] = json!(num(f, "patternTileScale").filter(|s| *s > 0.0).unwrap_or(1.0));
                }
                2 => {
                    p["scaleMode"] = json!("STRETCH");
                    p["imageTransform"] = json!([[1, 0, 0], [0, 1, 0]]);
                }
                3 => p["scaleMode"] = json!("FIT"),
                _ => p["scaleMode"] = json!("FILL"),
            }
            p
        }
        5 => {
            cx.warn("noise fills were skipped");
            return None;
        }
        other => {
            cx.warn(format!("fills of type {other} were skipped"));
            return None;
        }
    };
    p["visible"] = json!(f.get("isEnabled").and_then(Value::as_bool).unwrap_or(true));
    // A colour variable binds a plain colour (a gradient's or image's `color` is unused).
    if f.get("fillType").and_then(Value::as_i64).unwrap_or(0) == 0
        && let Some(var) = swatch(cx, f.get("color"))
    {
        p["boundVariables"] = var;
    }
    if let Some(ctx) = f.get("contextSettings") {
        if let Some(o) = num(ctx, "opacity") {
            p["opacity"] = json!(o.clamp(0.0, 1.0));
        }
        if let Some(b) = ctx.get("blendMode").and_then(Value::as_i64).and_then(blend) {
            p["blendMode"] = json!(b);
        }
    }
    Some(p)
}

/// A gradient fill: `from` and `to` in 0–1 of the layer; a radial gradient's `elipseLength` is its
/// width across the main axis as a share of the main axis, measured in pixels.
fn gradient(g: &Value, w: f64, h: f64) -> Option<Value> {
    let point = |k: &str| g.get(k).and_then(Value::as_str).and_then(parse_point);
    let (from, to) = (point("from").unwrap_or((0.5, 0.0)), point("to").unwrap_or((0.5, 1.0)));
    let stops: Vec<Value> = list(g, "stops")
        .iter()
        .filter_map(|s| Some(json!({"position": num(s, "position")?.clamp(0.0, 1.0), "color": s.get("color").and_then(color)?})))
        .collect();
    let (ty, handles) = match g.get("gradientType").and_then(Value::as_i64).unwrap_or(0) {
        1 => {
            let e = num(g, "elipseLength").filter(|e| *e > 0.0).unwrap_or(1.0);
            let (dx, dy) = ((to.0 - from.0) * w, (to.1 - from.1) * h);
            let across = if w > 0.0 && h > 0.0 { (from.0 - dy * e / w, from.1 + dx * e / h) } else { from };
            ("GRADIENT_RADIAL", json!([pt(from), pt(to), pt(across)]))
        }
        // Sketch starts on the right and turns clockwise; from/to don't change it.
        2 => ("GRADIENT_ANGULAR", json!([pt((0.5, 0.5)), pt((1.0, 0.5)), pt((0.5, 1.0))])),
        // Square to the line in pixels, as Sketch draws it (FigCraft's default is square in 0–1).
        _ => {
            let (dx, dy) = ((to.0 - from.0) * w, (to.1 - from.1) * h);
            let across = if w > 0.0 && h > 0.0 { (from.0 - dy / w, from.1 + dx / h) } else { (from.0 - (to.1 - from.1), from.1 + (to.0 - from.0)) };
            ("GRADIENT_LINEAR", json!([pt(from), pt(to), pt(across)]))
        }
    };
    Some(json!({"type": ty, "gradientHandlePositions": handles, "gradientStops": stops}))
}

fn pt((x, y): (f64, f64)) -> Value {
    json!({"x": x, "y": y})
}

/// Borders → stroke paints, and the first enabled border's thickness and position.
fn borders(cx: &mut Cx, style: &Value, w: f64, h: f64, node: &mut JsonMap<String, Value>) {
    let list = list(style, "borders");
    let strokes = border_paints(cx, style, w, h);
    if strokes.is_empty() {
        return;
    }
    let enabled: Vec<&Value> = list.iter().filter(|b| b.get("isEnabled").and_then(Value::as_bool).unwrap_or(true)).collect();
    let first = enabled.first().copied().or(list.first());
    let shape = |b: &Value| (num(b, "thickness").unwrap_or(1.0), b.get("position").and_then(Value::as_i64).unwrap_or(0));
    if let Some(b) = first {
        let (t, pos) = shape(b);
        if enabled.iter().any(|o| shape(o) != (t, pos)) {
            cx.warn("layers with several borders of different widths or positions use the first one's");
        }
        node.insert("strokeWeight".into(), json!(t.clamp(0.0, 1e6)));
        node.insert(
            "strokeAlign".into(),
            json!(match pos {
                1 => "INSIDE",
                2 => "OUTSIDE",
                _ => "CENTER",
            }),
        );
    }
    node.insert("strokes".into(), Value::Array(strokes));
    // Per-side thickness (Sketch 2025+): a side is as thick as the border less its inset.
    let first = list.iter().find(|b| b.get("isEnabled").and_then(Value::as_bool) != Some(false));
    if let Some(b) = first
        && let Some(sides) = b.get("sides")
    {
        let t = num(b, "thickness").unwrap_or(1.0).clamp(0.0, 1e6);
        let side = |k: &str| (t - num(sides, k).unwrap_or(0.0)).clamp(0.0, 1e6);
        node.insert(
            "individualStrokeWeights".into(),
            json!({"top": side("topInset"), "right": side("rightInset"), "bottom": side("bottomInset"), "left": side("leftInset")}),
        );
    }
    if let Some(o) = style.get("borderOptions") {
        let dashes: Vec<f64> = o
            .get("dashPattern")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_f64)
            .filter(|d| d.is_finite() && *d >= 0.0)
            .take(MAX_LIST)
            .collect();
        if !dashes.is_empty() {
            node.insert("strokeDashes".into(), json!(dashes));
        }
        node.insert(
            "strokeCap".into(),
            json!(match o.get("lineCapStyle").and_then(Value::as_i64) {
                Some(1) => "ROUND",
                Some(2) => "SQUARE",
                _ => "NONE",
            }),
        );
        node.insert(
            "strokeJoin".into(),
            json!(match o.get("lineJoinStyle").and_then(Value::as_i64) {
                Some(1) => "ROUND",
                Some(2) => "BEVEL",
                _ => "MITER",
            }),
        );
    }
    for (key, field) in [("startMarkerType", "strokeCapStart"), ("endMarkerType", "strokeCapEnd")] {
        let cap = match style.get(key).and_then(Value::as_i64).unwrap_or(0) {
            0 => continue,
            1 => "LINE_ARROW",
            2 => "TRIANGLE_ARROW",
            5 => "CIRCLE_FILLED",
            _ => {
                cx.warn("line ends other than arrows and filled circles were drawn without an end");
                continue;
            }
        };
        node.insert(field.into(), json!(cap));
    }
}

/// Shadows, inner shadows and the blur as REST effects.
pub(crate) fn effects(cx: &mut Cx, style: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (key, ty) in [("shadows", "DROP_SHADOW"), ("innerShadows", "INNER_SHADOW")] {
        for s in list(style, key) {
            // Sketch 2025 and newer keep inner shadows in the one list, marked.
            let ty = if s.get("isInnerShadow").and_then(Value::as_bool) == Some(true) { "INNER_SHADOW" } else { ty };
            let mut c = s.get("color").and_then(color).unwrap_or_else(|| json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 0.5}));
            let o = s.get("contextSettings").and_then(|c| num(c, "opacity")).unwrap_or(1.0).clamp(0.0, 1.0);
            c["a"] = json!(c["a"].as_f64().unwrap_or(1.0) * o);
            out.push(json!({
                "type": ty,
                "visible": s.get("isEnabled").and_then(Value::as_bool).unwrap_or(true),
                "radius": num(s, "blurRadius").unwrap_or(0.0).clamp(0.0, 1e6),
                "spread": num(s, "spread").unwrap_or(0.0),
                "offset": {"x": num(s, "offsetX").unwrap_or(0.0), "y": num(s, "offsetY").unwrap_or(0.0)},
                "color": c,
            }));
        }
    }
    // One `blur` (older files) or a list of `blurs` (Sketch 2025 and newer).
    let one = style.get("blur").into_iter();
    let many = style.get("blurs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter().take(MAX_LIST);
    for b in one.chain(many).filter(|b| b.get("isEnabled").and_then(Value::as_bool) == Some(true)) {
        let radius = num(b, "radius").unwrap_or(0.0).clamp(0.0, 1e6);
        match b.get("type").and_then(Value::as_i64).unwrap_or(0) {
            0 => out.push(json!({"type": "LAYER_BLUR", "visible": true, "radius": radius})),
            3 => out.push(background_blur(b, radius)),
            4 => out.push(json!({
                "type": "GLASS",
                "visible": true,
                "radius": radius,
                "refraction": num(b, "distortion").unwrap_or(0.5).clamp(0.0, 1.0),
                "depth": num(b, "depth").unwrap_or(0.5).clamp(0.0, 1.0),
            })),
            _ => {
                // Motion and zoom blurs: a plain blur of half the radius softens about as much.
                cx.warn("motion and zoom blurs were drawn as plain blurs");
                out.push(json!({"type": "LAYER_BLUR", "visible": true, "radius": radius / 2.0}));
            }
        }
    }
    out.truncate(MAX_LIST);
    out
}

/// A background blur; a progressive one (`isProgressive`) ramps along its gradient, whose stops'
/// opacity scales the radius at each end.
fn background_blur(b: &Value, radius: f64) -> Value {
    let g = b.get("gradient").filter(|_| b.get("isProgressive").and_then(Value::as_bool) == Some(true));
    let Some(g) = g else { return json!({"type": "BACKGROUND_BLUR", "visible": true, "radius": radius}) };
    let stops = g.get("stops").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let at = |s: Option<&Value>| s.and_then(|s| s.get("color")).and_then(|c| num(c, "alpha")).unwrap_or(1.0).clamp(0.0, 1.0) * radius;
    let end = |k: &str, d: (f64, f64)| {
        let (x, y) = g.get(k).and_then(Value::as_str).and_then(super::path::parse_point).unwrap_or(d);
        json!({"x": x.clamp(-1e3, 1e3), "y": y.clamp(-1e3, 1e3)})
    };
    json!({
        "type": "BACKGROUND_BLUR",
        "visible": true,
        "blurType": "PROGRESSIVE",
        "startRadius": at(stops.first()),
        "radius": at(stops.last()),
        "startOffset": end("from", (0.5, 0.0)),
        "endOffset": end("to", (0.5, 1.0)),
    })
}

/// A bitmap's colour adjustments (`colorControls`: saturation and contrast 0–2 with 1 neutral,
/// brightness −1–1, hue in radians) as REST image filters; hue has no filter (a note).
pub(crate) fn color_controls(cx: &mut Cx, style: Option<&Value>) -> Option<Value> {
    let c = style?.get("colorControls").filter(|c| c.get("isEnabled").and_then(Value::as_bool) == Some(true))?;
    let get = |k: &str, neutral: f64| num(c, k).unwrap_or(neutral);
    if get("hue", 0.0).abs() > 1e-6 {
        cx.warn("hue changes of bitmaps were skipped");
    }
    Some(json!({
        "saturation": (get("saturation", 1.0) - 1.0).clamp(-1.0, 1.0),
        "contrast": (get("contrast", 1.0) - 1.0).clamp(-1.0, 1.0),
        "exposure": get("brightness", 0.0).clamp(-1.0, 1.0),
    }))
}

/// The variable binding of a colour that names a swatch the file has (`{color: alias}`).
pub(crate) fn swatch(cx: &Cx, c: Option<&Value>) -> Option<Value> {
    let id = c?.get("swatchID").and_then(Value::as_str).filter(|id| cx.shared.has_swatch(id))?;
    Some(json!({"color": {"type": "VARIABLE_ALIAS", "id": id}}))
}

/// The tint a style gives the layers inside: its first enabled plain fill with `layeringType` 1
/// (Sketch 2025 and newer), or with no `layeringType` when `legacy` (older files: a group's or
/// instance's fill was its tint). Sketch draws a tint as the colour of everything inside.
pub(crate) fn tint_of(style: Option<&Value>, legacy: bool) -> Option<Value> {
    let fills = style?.get("fills")?.as_array()?;
    let f = fills.iter().filter(|f| f.get("isEnabled").and_then(Value::as_bool) != Some(false)).find(|f| {
        match f.get("layeringType").and_then(Value::as_i64) {
            Some(t) => t == 1,
            None => legacy,
        }
    })?;
    (f.get("fillType").and_then(Value::as_i64).unwrap_or(0) == 0).then(|| f.get("color").filter(|c| c.is_object()).cloned()).flatten()
}

/// Recolour `layers` (Sketch JSON) with tint `color`: every enabled fill and border takes it,
/// text takes it, nested instances take it as their own tint. Opacity stays as it was.
pub(crate) fn tint_layers(layers: &mut [Value], color: &Value, depth: usize) {
    if depth > 64 {
        return;
    }
    for l in layers {
        if let Some(st) = l.get_mut("style").and_then(Value::as_object_mut) {
            for key in ["fills", "borders"] {
                for p in st.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                    if let Some(m) = p.as_object_mut()
                        && m.get("isEnabled").and_then(Value::as_bool) != Some(false)
                        && !matches!(m.get("fillType").and_then(Value::as_i64), Some(4))
                    {
                        let c = tinted(m.get("color"), color);
                        m.insert("fillType".into(), json!(0));
                        m.insert("color".into(), c);
                    }
                }
            }
            if let Some(enc) = st.get_mut("textStyle").and_then(|t| t.get_mut("encodedAttributes")).and_then(Value::as_object_mut) {
                let c = tinted(enc.get("MSAttributedStringColorAttribute"), color);
                enc.insert("MSAttributedStringColorAttribute".into(), c);
            }
        }
        if let Some(runs) = l.get_mut("attributedString").and_then(|a| a.get_mut("attributes")).and_then(Value::as_array_mut) {
            for r in runs.iter_mut().filter_map(|r| r.get_mut("attributes")).filter_map(Value::as_object_mut) {
                let c = tinted(r.get("MSAttributedStringColorAttribute"), color);
                r.insert("MSAttributedStringColorAttribute".into(), c);
            }
        }
        if l.get("_class").and_then(Value::as_str) == Some("symbolInstance") {
            set_tint(l, color);
        }
        if let Some(kids) = l.get_mut("layers").and_then(Value::as_array_mut) {
            tint_layers(kids, color, depth + 1);
        }
    }
}

/// Colour `old` (Sketch JSON) tinted with `tint`: the tint's colour, with both opacities.
fn tinted(old: Option<&Value>, tint: &Value) -> Value {
    let alpha = |c: Option<&Value>| c.and_then(|c| num(c, "alpha")).unwrap_or(1.0).clamp(0.0, 1.0);
    let mut c = tint.clone();
    if let Some(m) = c.as_object_mut() {
        m.insert("alpha".into(), json!(alpha(Some(tint)) * alpha(old)));
    }
    c
}

/// Give layer `l` (Sketch JSON) tint `color`: as its one fill.
pub(crate) fn set_tint(l: &mut Value, color: &Value) {
    let fill = json!({"_class": "fill", "isEnabled": true, "fillType": 0, "color": color});
    if let Some(m) = l.as_object_mut() {
        let st = m.entry("style").or_insert_with(|| json!({"_class": "style"}));
        if let Some(st) = st.as_object_mut() {
            st.insert("fills".into(), json!([fill]));
        }
    }
}

/// Sketch's blend modes (0 normal … 17 plus lighter) by REST name.
fn blend(mode: i64) -> Option<&'static str> {
    const MODES: [&str; 18] = [
        "NORMAL",
        "DARKEN",
        "MULTIPLY",
        "COLOR_BURN",
        "LIGHTEN",
        "SCREEN",
        "COLOR_DODGE",
        "OVERLAY",
        "SOFT_LIGHT",
        "HARD_LIGHT",
        "DIFFERENCE",
        "EXCLUSION",
        "HUE",
        "SATURATION",
        "COLOR",
        "LUMINOSITY",
        "LINEAR_BURN",
        "LINEAR_DODGE",
    ];
    MODES.get(usize::try_from(mode).ok()?).copied()
}

fn list<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    let l = v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    l.get(..MAX_LIST).unwrap_or(l)
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite())
}
