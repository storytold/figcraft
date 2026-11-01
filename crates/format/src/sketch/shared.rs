//! Sketch shared styles and swatches → REST style definitions and colour variables.
//!
//! A Sketch layer style holds fills, borders and effects together; FigCraft styles hold one kind
//! each, so a layer style becomes up to three styles named after it: its fills (a paint style),
//! its border colours (a paint style for strokes; widths stay on the layers) and its effects. A
//! text style becomes a text style; its colour stays on the layers, as FigCraft text styles carry
//! none. Layers link to them by `style.sharedObjectID`. Swatches (Color Variables, Sketch 69+)
//! become colour variables in one collection; colours name theirs by `swatchID`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map as JsonMap, Value, json};

use super::tree::{Cx, color, str_field};

/// Most shared styles or swatches read from one file.
const MAX_SHARED: usize = 20_000;
/// The collection swatches become variables in.
const COLLECTION: &str = "sketch-color-variables";
const MODE: &str = "default";

/// A file's shared styles and swatches.
#[derive(Default)]
pub(crate) struct Shared {
    /// `(id, name, style)` of layer styles, then text styles.
    layer: Vec<(String, String, Value)>,
    text: Vec<(String, String, Value)>,
    /// Positions in `layer` and `text` by id.
    layer_index: BTreeMap<String, usize>,
    text_index: BTreeMap<String, usize>,
    swatches: BTreeMap<String, (String, Value)>,
}

impl Shared {
    pub(crate) fn read(document: &Value) -> Shared {
        let mut s = Shared::default();
        let objects =
            |key: &str| -> Vec<Value> { document.get(key).and_then(|c| c.get("objects")).and_then(Value::as_array).cloned().unwrap_or_default() };
        let foreign = |key: &str, inner: &str| -> Vec<Value> {
            document.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter().filter_map(|f| f.get(inner).cloned()).collect()
        };
        let entry = |o: &Value| {
            Some((
                str_field(o, "do_objectID")?.to_string(),
                str_field(o, "name").unwrap_or("Style").chars().take(200).collect(),
                o.get("value")?.clone(),
            ))
        };
        for o in objects("layerStyles").iter().chain(&foreign("foreignLayerStyles", "localSharedStyle")).take(MAX_SHARED) {
            if let Some(e) = entry(o).filter(|e| !s.layer_index.contains_key(&e.0)) {
                s.layer_index.insert(e.0.clone(), s.layer.len());
                s.layer.push(e);
            }
        }
        for o in objects("layerTextStyles").iter().chain(&foreign("foreignTextStyles", "localSharedStyle")).take(MAX_SHARED) {
            if let Some(e) = entry(o).filter(|e| !s.text_index.contains_key(&e.0)) {
                s.text_index.insert(e.0.clone(), s.text.len());
                s.text.push(e);
            }
        }
        let foreign_swatches = [foreign("foreignSwatches", "localSwatch"), foreign("foreignSwatches", "localSharedSwatch")].concat();
        for o in objects("sharedSwatches").iter().chain(&foreign_swatches).take(MAX_SHARED) {
            if let Some((id, name, value)) = entry(o)
                && let Some(c) = color(&value)
            {
                s.swatches.entry(id).or_insert((name, c));
            }
        }
        s
    }

    /// The style of shared text (`text`) or layer style `id`.
    pub(crate) fn style(&self, text: bool, id: &str) -> Option<&Value> {
        let (index, list) = if text { (&self.text_index, &self.text) } else { (&self.layer_index, &self.layer) };
        list.get(*index.get(id)?).map(|e| &e.2)
    }

    pub(crate) fn has_swatch(&self, id: &str) -> bool {
        self.swatches.contains_key(id)
    }

    /// The REST `styleDefinitions` of every shared style.
    pub(crate) fn definitions(cx: &mut Cx) -> Vec<Value> {
        let layer = cx.shared.layer.clone();
        let text = cx.shared.text.clone();
        let mut out = Vec::new();
        for (id, name, style) in &layer {
            let fills = super::style::fills(cx, style, 100.0, 100.0);
            if !fills.is_empty() {
                out.push(json!({"id": format!("{id}:fill"), "name": name, "styleType": "FILL", "node": {"fills": fills}}));
            }
            let strokes = super::style::border_paints(cx, style, 100.0, 100.0);
            if !strokes.is_empty() {
                out.push(json!({"id": format!("{id}:stroke"), "name": format!("{name} border"), "styleType": "FILL", "node": {"fills": strokes}}));
            }
            let effects = super::style::effects(cx, style);
            if !effects.is_empty() {
                out.push(json!({"id": format!("{id}:effect"), "name": name, "styleType": "EFFECT", "node": {"effects": effects}}));
            }
        }
        for (id, name, style) in &text {
            let Some(enc) = style.get("textStyle").and_then(|t| t.get("encodedAttributes")) else { continue };
            let mut ts = super::text::type_style(cx, enc);
            if let Some(m) = ts.as_object_mut() {
                m.remove("fills");
            }
            out.push(json!({"id": id, "name": name, "styleType": "TEXT", "node": {"type": "TEXT", "characters": "", "style": ts}}));
        }
        out
    }

    /// Link `node` (layer `l`) to the styles made from its shared style.
    pub(crate) fn link(&self, cx_defs: &BTreeSet<String>, l: &Value, node: &mut JsonMap<String, Value>) {
        let Some(sid) = l.get("style").and_then(|s| str_field(s, "sharedObjectID")) else { return };
        let mut refs = JsonMap::new();
        let is_text = str_field(l, "_class") == Some("text");
        if is_text && self.text_index.contains_key(sid) {
            refs.insert("text".into(), json!(sid));
        } else if self.layer_index.contains_key(sid) {
            for slot in ["fill", "stroke", "effect"] {
                let id = format!("{sid}:{slot}");
                if cx_defs.contains(&id) {
                    refs.insert(slot.into(), json!(id));
                }
            }
        }
        if !refs.is_empty() {
            node.insert("styles".into(), Value::Object(refs));
        }
    }

    /// The REST `variableCollections` and `variables` for the swatches (none without swatches).
    pub(crate) fn variables(&self) -> Option<(Value, Value)> {
        if self.swatches.is_empty() {
            return None;
        }
        let ids: Vec<&String> = self.swatches.keys().collect();
        let collections = json!({COLLECTION: {"id": COLLECTION, "name": "Color Variables", "modes": [{"modeId": MODE, "name": "Default"}],
            "defaultModeId": MODE, "variableIds": ids}});
        let vars: JsonMap<String, Value> = self
            .swatches
            .iter()
            .map(|(id, (name, c))| {
                (id.clone(), json!({"id": id, "name": name, "resolvedType": "COLOR", "variableCollectionId": COLLECTION, "valuesByMode": {MODE: c}}))
            })
            .collect();
        Some((collections, Value::Object(vars)))
    }
}
