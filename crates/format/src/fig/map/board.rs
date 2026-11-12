//! FigJam whiteboard objects as native board items: what a `.jam` import builds in place of the
//! generated frames a `.fig` keeps. The REST JSON carries them as a `board` field
//! ([`crate::rest`] reads it).

use super::layer::{own_text, valid_size};
use super::paint::paint;
use super::{Affine, Kv, MAX_DEPTH, Mapper, Obj, Value, View, generated_record, json, num};
use figcraft_doc::BoardShape;

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

impl<'a> Mapper<'a> {
    /// A sticky or shape with text as a native board item, or `None` with the note that it is
    /// a frame with its generated layers.
    pub(super) fn native_board_item(&mut self, own: &str, o: &mut Obj, v: &View<'a>, rest_id: &str, depth: usize) -> Option<Vec<Value>> {
        let native = if own == "STICKY" { self.native_sticky(o, v, rest_id, depth) } else { self.native_shape(o, v, rest_id, depth) };
        if native.is_none() && own == "STICKY" {
            self.warn("sticky layers import as frames with their text");
        } else if native.is_none() {
            self.warn("shape with text layers import as frames with their text");
        }
        native
    }
}

impl<'a> Mapper<'a> {
    /// A shape with text as a native board shape: an empty frame holding FigCraft's own outline
    /// (the file's paints and weight) and the text, centred. `None` (nothing changed) when the kind
    /// has no FigCraft outline or a generated record is damaged: then the caller keeps the
    /// generated layers.
    pub(super) fn native_shape(&mut self, o: &mut Obj, v: &View<'a>, rest_id: &str, depth: usize) -> Option<Vec<Value>> {
        let kind = v.str("shapeWithTextType").unwrap_or("SQUARE");
        let Some(shape) = board_shape(kind) else {
            self.warn(format!("shape with text kind {} has no FigCraft outline; kept as a frame", crate::rest::props::clip_str(kind)));
            return None;
        };
        if v.get("nodeGenerationData").is_none() || v.get("derivedImmutableFrameData").and_then(|d| num(d, "version")) != Some(1.0) {
            return None;
        }
        let record = |id: u32| {
            let (style, bounds) = (generated_record(v, "nodeGenerationData", id)?, generated_record(v, "derivedImmutableFrameData", id)?);
            let view = View::of(style).with([(bounds, 1.0)]);
            (valid_size(view.get("size")).is_some() && view.transform().is_some()).then_some(view)
        };
        let background = record(0)?;
        let body = record(1)?;
        // Blank generated text must not hide text the outer node still carries (#1397): the
        // generated layers keep it.
        if own_text(&body).is_none() && own_text(v).is_some() {
            return None;
        }
        if depth >= MAX_DEPTH || self.count.saturating_add(2) > self.budget() {
            return None;
        }
        let (w, h) = v.size().or_else(|| o.get("size").and_then(|s| Some((s.get("x")?.as_f64()?, s.get("y")?.as_f64()?))))?;
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return None;
        }
        // The REST reader caps every size; the outline is drawn at the capped size too, so it
        // stays inside its frame.
        let (w, h) = (w.min(crate::rest::props::MAX_SIZE), h.min(crate::rest::props::MAX_SIZE));
        // Everything is valid: build the outline from the record's paints, then the frame.
        let mut outline = Obj::new();
        outline.insert("id".into(), json!(format!("{rest_id};generated0")));
        outline.insert("type".into(), json!("VECTOR"));
        outline.insert("name".into(), json!("Outline"));
        self.common(&mut outline, &background, "VECTOR", Affine::IDENTITY, None);
        outline.insert("size".into(), json!({"x": w, "y": h}));
        outline.insert("relativeTransform".into(), json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        outline.insert("vectorNetwork".into(), serde_json::to_value(figcraft_doc::VectorNetwork::from_path(&shape.path(w, h))).ok()?);
        outline.insert("layoutPositioning".into(), json!("ABSOLUTE"));
        outline.insert("constraints".into(), json!({"horizontal": "SCALE", "vertical": "SCALE"}));
        outline.remove("cornerRadius");
        let mut label = Obj::new();
        label.insert("id".into(), json!(format!("{rest_id};generated1")));
        label.insert("type".into(), json!("TEXT"));
        label.insert("name".into(), json!(own_text(&body).map_or_else(|| "Text".to_string(), |c| c.chars().take(40).collect())));
        self.common(&mut label, &body, "TEXT", Affine::IDENTITY, None);
        self.text(&mut label, &body);
        if let Some(Value::Object(style)) = label.get_mut("style") {
            style.insert("textAutoResize".into(), json!("HEIGHT"));
            style.insert("textAlignHorizontal".into(), json!("CENTER"));
        }
        label.insert("layoutSizingHorizontal".into(), json!("FILL"));
        o.insert("fills".into(), json!([]));
        o.insert("strokes".into(), json!([]));
        o.insert("clipsContent".into(), json!(false));
        o.remove("cornerRadius");
        let [top, right, bottom, left] = shape.text_insets();
        for (k, val) in [
            ("layoutMode", json!("VERTICAL")),
            ("primaryAxisAlignItems", json!("CENTER")),
            ("counterAxisAlignItems", json!("CENTER")),
            ("primaryAxisSizingMode", json!("FIXED")),
            ("counterAxisSizingMode", json!("FIXED")),
            ("paddingTop", json!(top * h)),
            ("paddingRight", json!(right * w)),
            ("paddingBottom", json!(bottom * h)),
            ("paddingLeft", json!(left * w)),
        ] {
            o.insert(k.into(), val);
        }
        self.count += 2;
        o.insert("board".into(), json!({"type": "shape", "shape": shape.id()}));
        Some(vec![Value::Object(outline), Value::Object(label)])
    }
}

/// FigJam's `shapeWithTextType` as the outline FigCraft draws for it; `None` for the kinds it has none for.
pub(in crate::fig) fn board_shape(kind: &str) -> Option<BoardShape> {
    Some(match kind {
        "SQUARE" => BoardShape::Rectangle,
        "ROUNDED_RECTANGLE" => BoardShape::RoundedRectangle,
        "ELLIPSE" => BoardShape::Ellipse,
        "DIAMOND" => BoardShape::Diamond,
        "TRIANGLE_UP" => BoardShape::Triangle,
        "TRIANGLE_DOWN" => BoardShape::TriangleDown,
        "PARALLELOGRAM_RIGHT" => BoardShape::ParallelogramRight,
        "PARALLELOGRAM_LEFT" => BoardShape::ParallelogramLeft,
        "TRAPEZOID" => BoardShape::Trapezoid,
        "PENTAGON" => BoardShape::Pentagon,
        "HEXAGON" => BoardShape::Hexagon,
        "OCTAGON" => BoardShape::Octagon,
        "STAR" => BoardShape::Star,
        "PLUS" => BoardShape::Plus,
        "ARROW_LEFT" => BoardShape::ArrowLeft,
        "ARROW_RIGHT" => BoardShape::ArrowRight,
        "CHEVRON" => BoardShape::Chevron,
        "SPEECH_BUBBLE" => BoardShape::SpeechBubble,
        "ENG_DATABASE" => BoardShape::Database,
        "DOCUMENT_SINGLE" => BoardShape::Document,
        "PREDEFINED_PROCESS" => BoardShape::PredefinedProcess,
        "MANUAL_INPUT" => BoardShape::ManualInput,
        "INTERNAL_STORAGE" => BoardShape::InternalStorage,
        _ => return None,
    })
}
