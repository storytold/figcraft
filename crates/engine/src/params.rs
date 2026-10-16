//! Parameter parsing helpers shared by commands (lenient: agents send numbers as strings, ids as
//! "12" or "0:12", colours as hex or CSS).

use figcraft_color::{BlendMode, Color, Paint};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, bad};

pub fn get<'a>(p: &'a Value, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| !v.is_null())
}

pub fn f64_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().trim_end_matches("px").trim_end_matches('%').trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn num(p: &Value, key: &str) -> Option<f64> {
    get(p, key).and_then(f64_of)
}

pub fn num_or(p: &Value, key: &str, d: f64) -> f64 {
    num(p, key).unwrap_or(d)
}

pub fn req_num(cmd: &str, p: &Value, key: &str) -> Result<f64> {
    num(p, key).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))
}

/// A list index from params: a whole number, never negative (no casting -1 to 0).
pub fn req_index(cmd: &str, p: &Value, key: &str) -> Result<usize> {
    let k = req_num(cmd, p, key)?;
    if !(0.0..1e9).contains(&k) || k.fract() != 0.0 {
        return Err(bad(cmd, format!("`{key}` out of range")));
    }
    Ok(k as usize)
}

pub fn string(p: &Value, key: &str) -> Option<String> {
    match get(p, key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn req_string(cmd: &str, p: &Value, key: &str) -> Result<String> {
    string(p, key).ok_or_else(|| bad(cmd, format!("`{key}` must be a string")))
}

pub fn boolean(p: &Value, key: &str) -> Option<bool> {
    bool_of(get(p, key)?)
}

/// A lenient boolean: `true`, `1`, `"yes"`, `"on"`…
pub fn bool_of(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub fn id_of(v: &Value) -> Option<NodeId> {
    match v {
        Value::Number(n) => n.as_u64().map(NodeId),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn id(p: &Value, key: &str) -> Option<NodeId> {
    get(p, key).and_then(id_of)
}

pub fn req_id(cmd: &str, p: &Value, key: &str) -> Result<NodeId> {
    id(p, key).ok_or_else(|| bad(cmd, format!("`{key}` must be a node id")))
}

/// `ids` (array), `id` (single), or the current selection.
pub fn targets(s: &Session, p: &Value) -> Vec<NodeId> {
    if let Some(Value::Array(a)) = get(p, "ids") {
        return a.iter().filter_map(id_of).collect();
    }
    if let Some(i) = id(p, "id") {
        return vec![i];
    }
    s.selection()
}

/// Like [`targets`] but an empty result is an error.
pub fn req_targets(cmd: &str, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let t = targets(s, p);
    if t.is_empty() {
        return Err(EngineError::Disabled(cmd.to_string(), "nothing is selected (pass `ids`)".into()));
    }
    if let Ok(doc) = s.doc() {
        for i in &t {
            if !doc.contains(*i) {
                return Err(EngineError::NoNode(*i));
            }
        }
    }
    Ok(t)
}

pub fn color_of(v: &Value) -> Option<Color> {
    match v {
        Value::String(s) => Color::parse_css(s),
        Value::Object(_) => serde_json::from_value::<Color>(v.clone()).ok().map(Color::sanitized),
        Value::Array(a) => {
            let c: Vec<f32> = a.iter().filter_map(|x| x.as_f64()).map(|f| f as f32).collect();
            match c.as_slice() {
                [r, g, b] => Some(Color::new(*r, *g, *b, 1.0)),
                [r, g, b, a] => Some(Color::new(*r, *g, *b, *a)),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn color(p: &Value, key: &str) -> Option<Color> {
    get(p, key).and_then(color_of)
}

/// Read the friendlier paint/effect JSON that the docs and MCP tools describe as the engine's own
/// form: any `"color"` given as a CSS string ("#RRGGBB[AA]", "rgb(…)") becomes {r,g,b,a},
/// `{"type":"linearGradient"}` becomes `{"type":"gradient","kind":"linear"}`, gradients get default
/// handles (top to bottom) when `from`/`to`/`width` are missing, and stops given as plain colours
/// are spread evenly. Anything else is left for serde to refuse.
pub fn normalize_paint(v: &mut Value) {
    normalize_paintish(v, 0);
}

fn normalize_paintish(v: &mut Value, depth: usize) {
    if depth > 16 {
        return;
    }
    match v {
        Value::Array(a) => {
            for x in a {
                normalize_paintish(x, depth + 1);
            }
        }
        Value::Object(o) => {
            if let Some(c) = o.get("color").filter(|c| c.is_string()).and_then(color_of) {
                o.insert("color".into(), serde_json::to_value(c).unwrap_or(Value::Null));
            }
            let is_gradient = o.get("type").and_then(Value::as_str).is_some_and(|t| t.to_ascii_lowercase().contains("gradient"));
            if is_gradient {
                // Accept {"type":"linearGradient"} as well as {"type":"gradient","kind":"linear"}.
                if let Some(t) = o.get("type").and_then(Value::as_str).map(str::to_ascii_lowercase)
                    && t != "gradient"
                {
                    let kind = t.trim_end_matches("gradient").to_string();
                    o.insert("type".into(), json!("gradient"));
                    o.entry("kind").or_insert(json!(if kind.is_empty() { "linear".to_string() } else { kind }));
                }
                o.entry("kind").or_insert(json!("linear"));
                let linear = o.get("kind").and_then(Value::as_str) == Some("linear");
                let (from, to, width) = if linear { ([0.5, 0.0], [0.5, 1.0], [-0.5, 0.0]) } else { ([0.5, 0.5], [0.5, 1.0], [1.0, 0.5]) };
                o.entry("from").or_insert(json!(from));
                o.entry("to").or_insert(json!(to));
                // A linear gradient's width handle goes square to its axis (else its bands skew,
                // or vanish when the handle lies on the axis).
                let pt = |k: &str| o.get(k).and_then(Value::as_array).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
                if linear
                    && !o.contains_key("width")
                    && let (Some(f), Some(t)) = (pt("from"), pt("to"))
                {
                    o.insert("width".into(), json!([f.0 - (t.1 - f.1), f.1 + (t.0 - f.0)]));
                }
                o.entry("width").or_insert(json!(width));
                if let Some(Value::Array(stops)) = o.get_mut("stops") {
                    let n = stops.len();
                    for (i, s) in stops.iter_mut().enumerate() {
                        if let Some(c) = color_of(s).filter(|_| s.is_string()) {
                            let pos = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                            *s = json!({"position": pos, "color": c});
                        } else {
                            normalize_paintish(s, depth + 1);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// A paint from JSON: a colour string/object (solid), a full paint object or its shorthand
/// ([`normalize_paint`]).
pub fn paint_of(v: &Value) -> Option<Paint> {
    if let Some(c) = color_of(v) {
        return Some(Paint::solid(c));
    }
    let mut v = v.clone();
    normalize_paint(&mut v);
    let mut p: Paint = serde_json::from_value(v).ok()?;
    p.opacity = if p.opacity.is_finite() { p.opacity.clamp(0.0, 1.0) } else { 1.0 };
    Some(p)
}

pub fn paints_of(v: &Value) -> Option<Vec<Paint>> {
    match v {
        Value::Array(a) => a.iter().map(paint_of).collect(),
        other => paint_of(other).map(|p| vec![p]),
    }
}

pub fn blend(p: &Value, key: &str) -> Option<BlendMode> {
    string(p, key).and_then(|s| BlendMode::from_name(&s))
}

/// Serde-deserialize `key` leniently into `T`.
pub fn parse<T: serde::de::DeserializeOwned>(cmd: &str, p: &Value, key: &str) -> Result<Option<T>> {
    match get(p, key) {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| bad(cmd, format!("`{key}`: {e}"))),
    }
}
