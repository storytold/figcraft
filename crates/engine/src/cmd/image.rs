//! Image paints: adjustment presets, built in and saved in the file, and Crop image.

use figcraft_color::{ImageFilters, ImageScaleMode, PaintKind};
use figcraft_doc::{Document, ImagePreset, Node};
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::{f64_of, get, num, req_index, req_string, req_targets, string, targets};
use crate::{EngineError, Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "image.adjust",
            "Adjust image",
            "{ids?, target? (fill|stroke, default fill), index? (default every image paint), preset? (vivid|muted|warm|cool|blackAndWhite|fade|reset, or the name of a preset saved in the file), filters? ({exposure, contrast, saturation, temperature, tint, highlights, shadows}: each -1..1, applied over the preset)}  sets the adjustments of the layers' image paints; returns {changed}",
            true,
            adjust
        ),
        spec!(
            "image.savePreset",
            "Save image preset",
            "{name, filters? (default the adjustments of the first image paint of `ids`/the selection), ids?, target?, index?}  saved in the file; a preset with the same name is replaced",
            true,
            save_preset
        ),
        spec!("image.deletePreset", "Delete image preset", "{name}", true, |s, p| {
            let name = req_string("image.deletePreset", p, "name")?;
            let doc = s.doc_mut()?;
            let before = doc.image_presets.len();
            doc.image_presets.retain(|x| x.name != name);
            if doc.image_presets.len() == before {
                return Err(bad("image.deletePreset", format!("no preset named `{name}`")));
            }
            ok()
        }),
        spec!(
            "image.crop",
            "Crop image",
            "{ids? (one layer), index? (an image fill; default its top visible one)} → {id, index}: the image fill switches to Crop, keeping the picture where it was, and the canvas enters crop mode on it",
            true,
            crop
        ),
        spec!("image.presets", "Image presets", "{} → {builtIn: [{id, label, filters}], saved: [{name, filters}]}", false, |s, _| {
            let built_in: Vec<Value> = ImageFilters::PRESETS.iter().map(|(id, label, f)| json!({"id": id, "label": label, "filters": f})).collect();
            Ok(json!({"builtIn": built_in, "saved": s.doc()?.image_presets}))
        }),
    ]
}

/// The longest name a saved preset may have.
const MAX_NAME: usize = 100;
/// The most presets a file keeps.
const MAX_PRESETS: usize = 500;

/// Whether `p` asks for strokes (`target: stroke`) rather than fills.
fn strokes(cmd: &str, p: &Value) -> Result<bool> {
    match string(p, "target").as_deref() {
        None | Some("fill") | Some("fills") => Ok(false),
        Some("stroke") | Some("strokes") => Ok(true),
        Some(_) => Err(bad(cmd, "target: fill|stroke")),
    }
}

/// The preset `name`: one saved in the file under that exact name, else a built-in one.
pub fn find_preset(doc: &Document, name: &str) -> Option<ImageFilters> {
    doc.image_presets.iter().find(|x| x.name == name).map(|x| x.filters).or_else(|| ImageFilters::preset(name))
}

/// `base` with the adjustments `v` names (an object of -1..1 numbers) set.
fn with_values(cmd: &str, base: ImageFilters, v: &Value) -> Result<ImageFilters> {
    let Value::Object(m) = v else { return Err(bad(cmd, "filters: an object of adjustments")) };
    let mut f = base;
    for (k, val) in m {
        let x = f64_of(val).ok_or_else(|| bad(cmd, format!("filters.{k}: a number")))? as f32;
        let slot = match k.as_str() {
            "exposure" => &mut f.exposure,
            "contrast" => &mut f.contrast,
            "saturation" => &mut f.saturation,
            "temperature" => &mut f.temperature,
            "tint" => &mut f.tint,
            "highlights" => &mut f.highlights,
            "shadows" => &mut f.shadows,
            _ => return Err(bad(cmd, format!("unknown adjustment `{k}`"))),
        };
        *slot = x;
    }
    Ok(f.sanitized())
}

fn adjust(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.adjust";
    let ids = req_targets(cmd, s, p)?;
    let stroke = strokes(cmd, p)?;
    let index = match get(p, "index") {
        None | Some(Value::Null) => None,
        Some(_) => Some(req_index(cmd, p, "index")?),
    };
    let preset = match string(p, "preset") {
        Some(name) => Some(find_preset(s.doc()?, &name).ok_or_else(|| bad(cmd, format!("no preset named `{name}`")))?),
        None => None,
    };
    let values = get(p, "filters").filter(|v| !v.is_null());
    if preset.is_none() && values.is_none() {
        return Err(bad(cmd, "pass `preset` or `filters`"));
    }
    let doc = s.doc_mut()?;
    let mut changed = 0;
    for id in ids {
        let n = doc.get(id)?;
        if !if stroke { &n.strokes } else { &n.fills }.iter().any(|x| matches!(x.kind, PaintKind::Image(_))) {
            continue;
        }
        let n = doc.get_mut(id)?;
        let list = if stroke { &mut n.strokes } else { &mut n.fills };
        let mut touched = false;
        for (i, paint) in list.iter_mut().enumerate() {
            let PaintKind::Image(img) = &mut paint.kind else { continue };
            if index.is_some_and(|k| k != i) {
                continue;
            }
            let base = preset.unwrap_or(img.filters);
            img.filters = match values {
                Some(v) => with_values(cmd, base, v)?,
                None => base,
            };
            touched = true;
            changed += 1;
        }
        if touched {
            // Like any paint edit, the layer leaves its paint style.
            if stroke {
                n.styles.stroke = None;
            } else {
                n.styles.fill = None;
            }
        }
    }
    if changed == 0 {
        return Err(bad(cmd, if index.is_some() { "no image paint at that index" } else { "the layers have no image paints" }));
    }
    Ok(json!({"changed": changed}))
}

/// The index of a layer's top visible image fill (fills paint bottom to top).
pub fn top_image_fill(n: &Node) -> Option<usize> {
    n.fills.iter().rposition(|f| f.visible && matches!(f.kind, PaintKind::Image(_)))
}

fn crop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.crop";
    let ids = req_targets(cmd, s, p)?;
    // A selection with nothing to crop is refused as unavailable, not as bad input.
    let refuse = |why: &str| EngineError::Disabled(cmd.into(), why.into());
    let [id] = ids[..] else { return Err(refuse("select one layer to crop its image")) };
    let doc = s.doc()?;
    let n = doc.get(id)?;
    let index = match get(p, "index") {
        None | Some(Value::Null) => top_image_fill(n).ok_or_else(|| refuse("the layer has no image fill to crop"))?,
        Some(_) => req_index(cmd, p, "index")?,
    };
    let mut paint = n.fills.get(index).cloned().ok_or_else(|| bad(cmd, "no fill at that index"))?;
    let PaintKind::Image(img) = &mut paint.kind else { return Err(bad(cmd, "that fill is not an image")) };
    // An image already in Crop mode only enters crop mode: no edit, no undo step.
    if img.scale_mode != ImageScaleMode::Crop {
        let data = doc.images.get(&img.image).ok_or_else(|| bad(cmd, "the image's data is missing"))?;
        img.crop = Some(img.placement_crop(f64::from(data.width), f64::from(data.height), n.width, n.height));
        img.scale_mode = ImageScaleMode::Crop;
        let n = s.doc_mut()?.get_mut(id)?;
        if let Some(f) = n.fills.get_mut(index) {
            *f = paint;
        }
        // Like any paint edit, the layer leaves its paint style.
        n.styles.fill = None;
    }
    s.request_ui(cmd, json!({"id": id, "index": index}));
    Ok(json!({"id": id, "index": index}))
}

fn save_preset(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.savePreset";
    let name: String = req_string(cmd, p, "name")?.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    if name.is_empty() {
        return Err(bad(cmd, "`name` is empty"));
    }
    let filters = match get(p, "filters").filter(|v| !v.is_null()) {
        Some(v) => with_values(cmd, ImageFilters::NONE, v)?,
        None => {
            let stroke = strokes(cmd, p)?;
            let index = num(p, "index").filter(|k| k.is_finite() && *k >= 0.0).map(|k| k as usize);
            let doc = s.doc()?;
            targets(s, p)
                .iter()
                .filter_map(|id| doc.node(*id))
                .flat_map(|n| if stroke { n.strokes.iter() } else { n.fills.iter() }.enumerate())
                .find_map(|(i, x)| match &x.kind {
                    PaintKind::Image(img) if index.is_none_or(|k| k == i) => Some(img.filters),
                    _ => None,
                })
                .ok_or_else(|| bad(cmd, "pass `filters`, or select a layer with an image paint"))?
        }
    };
    let doc = s.doc_mut()?;
    let preset = ImagePreset { name: name.clone(), filters };
    match doc.image_presets.iter().position(|x| x.name == name) {
        Some(i) => {
            if let Some(x) = doc.image_presets.get_mut(i) {
                *x = preset;
            }
        }
        None if doc.image_presets.len() >= MAX_PRESETS => return Err(bad(cmd, format!("a file keeps at most {MAX_PRESETS} presets"))),
        None => doc.image_presets.push(preset),
    }
    Ok(json!({"name": name, "filters": filters}))
}
