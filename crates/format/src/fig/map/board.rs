//! FigJam whiteboard objects as native board items: what a `.jam` import builds in place of the
//! generated frames a `.fig` keeps. The REST JSON carries them as a `board` field
//! ([`crate::rest`] reads it).

use super::layer::{own_text, valid_size};
use super::paint::paint;
use super::{Affine, Kv, MAX_DEPTH, Mapper, Obj, Value, View, generated_record, json, num};

impl<'a> Mapper<'a> {
    /// A sticky as a native board sticky: its frame (size, place, opacity and shadows as saved,
    /// the saved colour) holding the note and, when saved, the author line. `None` (nothing
    /// changed) unless every generated record is complete: then the caller keeps the generated
    /// layers.
    pub(super) fn native_sticky(&mut self, o: &mut Obj, v: &View<'a>, rest_id: &str, depth: usize) -> Option<Vec<Value>> {
        if v.get("nodeGenerationData").is_none() || v.get("derivedImmutableFrameData").and_then(|d| num(d, "version")) != Some(1.0) {
            return None;
        }
        let record = |id: u32| {
            let (style, bounds) = (generated_record(v, "nodeGenerationData", id)?, generated_record(v, "derivedImmutableFrameData", id)?);
            let view = View::of(style).with([(bounds, 1.0)]);
            (valid_size(view.get("size")).is_some() && view.transform().is_some()).then_some(view)
        };
        let background = record(0)?;
        let fills = background.list("fillPaints");
        let solid = matches!(fills, [p] if p.get("type").and_then(Kv::str) == Some("SOLID") && p.get("visible").and_then(Kv::bool) != Some(false));
        if !solid || !background.list("strokePaints").is_empty() {
            return None;
        }
        let note = record(1).filter(|n| own_text(n).is_some())?;
        let author = record(2).filter(|a| a.flag("visible") != Some(false) && own_text(a).is_some());
        let wanted = 1 + usize::from(author.is_some());
        if depth >= MAX_DEPTH || self.count.saturating_add(wanted) > self.budget() {
            return None;
        }
        let (w, h) = v.size().or_else(|| o.get("size").and_then(|s| Some((s.get("x")?.as_f64()?, s.get("y")?.as_f64()?))))?;
        let at = note.transform().unwrap_or_default().as_coeffs();
        let pad = |x: f64| if x.is_finite() { x.clamp(0.0, (w.min(h) / 2.0).max(0.0)) } else { 0.0 };
        let (pad_x, pad_y) = (pad(at[4]), pad(at[5]));
        // Everything is valid: build.
        let paints: Vec<Value> = fills.iter().filter_map(|p| paint(p, &mut |m| self.warn(m))).collect();
        o.insert("fills".into(), Value::Array(paints));
        o.insert("strokes".into(), json!([]));
        o.insert("clipsContent".into(), json!(false));
        match background.len("cornerRadius") {
            Some(r) => o.insert("cornerRadius".into(), json!(r)),
            None => o.remove("cornerRadius"),
        };
        for (k, val) in [
            ("layoutMode", json!("VERTICAL")),
            ("primaryAxisAlignItems", json!("SPACE_BETWEEN")),
            ("counterAxisAlignItems", json!("MIN")),
            ("primaryAxisSizingMode", json!("AUTO")),
            ("counterAxisSizingMode", json!("FIXED")),
            ("minHeight", json!(h)),
            ("itemSpacing", json!(8.0)),
            ("paddingTop", json!(pad_y)),
            ("paddingBottom", json!(pad_y)),
            ("paddingLeft", json!(pad_x)),
            ("paddingRight", json!(pad_x)),
        ] {
            o.insert(k.into(), val);
        }
        let mut kids = Vec::new();
        let mut written = String::new();
        for (local, view) in [(1, Some(note)), (2, author)] {
            let Some(view) = view else { continue };
            let chars = own_text(&view).unwrap_or_default();
            let mut layer = Obj::new();
            layer.insert("id".into(), json!(format!("{rest_id};generated{local}")));
            layer.insert("type".into(), json!("TEXT"));
            layer.insert("name".into(), json!(if local == 1 { chars.chars().take(40).collect::<String>() } else { "Author".to_string() }));
            self.common(&mut layer, &view, "TEXT", Affine::IDENTITY, None);
            self.text(&mut layer, &view);
            if let Some(Value::Object(style)) = layer.get_mut("style") {
                style.insert("textAutoResize".into(), json!("HEIGHT"));
                style.insert("textAlignHorizontal".into(), json!("LEFT"));
            }
            layer.insert("layoutSizingHorizontal".into(), json!("FILL"));
            if local == 2 {
                written = chars.trim().to_string();
            }
            self.count += 1;
            kids.push(Value::Object(layer));
        }
        o.insert("board".into(), json!({"type": "sticky", "author": written}));
        Some(kids)
    }
}
