//! Sketch symbols → REST components and instances.
//!
//! A `symbolMaster` (on any page, or copied from a library into `document.foreignSymbols`) is a
//! component; a `symbolInstance` names it by `symbolID`. REST instances carry their layers, so an
//! instance is its master's layers with the instance's `overrideValues` applied, laid out at the
//! master's size (the import then resizes them to the instance by their constraints). An override
//! is named `<path>_<property>`: `path` is the `do_objectID` of a layer of the master, or of
//! nested instances and then a layer of theirs, joined by `/`; properties: `stringValue` (text),
//! `symbolID` (swap a nested instance; empty hides it), `image`, `layerStyle` and `textStyle`
//! (shared style ids).

use std::collections::BTreeMap;

use figcraft_doc::geom::Affine;
use serde_json::{Map as JsonMap, Value, json};

use super::shared::Shared;
use super::tree::{Cx, base, bool_field, children, local, size, str_field};

/// Work units an override handed on to a nested instance costs (the JSON values it adds).
const OVERRIDE_COST: u64 = 8;
/// Most masters read from one file.
const MAX_MASTERS: usize = 100_000;

/// Every symbol master of a file by `symbolID`, and the library ones no page holds.
#[derive(Default)]
pub(crate) struct Symbols {
    masters: BTreeMap<String, Value>,
    /// What expanding each master costs (see [`weight`]).
    weights: BTreeMap<String, u64>,
    /// Masters copied from libraries that no page holds (they get a page of their own), with the
    /// libraries' names.
    pub foreign: Vec<Value>,
    pub libraries: Vec<String>,
}

impl Symbols {
    /// The masters of `pages` and of the document's library copies.
    pub(crate) fn read(document: &Value, pages: &[(String, Value)]) -> Symbols {
        let mut s = Symbols::default();
        let mut found = Vec::new();
        for (_, p) in pages {
            masters_in(p, &mut found, 0);
        }
        for m in found {
            s.add(m.clone());
        }
        for f in document.get("foreignSymbols").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
            let Some(m) = f.get("symbolMaster") else { continue };
            let Some(id) = str_field(m, "symbolID") else { continue };
            if s.masters.contains_key(id) {
                continue;
            }
            if let Some(lib) = str_field(f, "sourceLibraryName")
                && !s.libraries.iter().any(|l| l == lib)
            {
                s.libraries.push(lib.chars().take(64).collect());
            }
            s.add(m.clone());
            s.foreign.push(m.clone());
        }
        s
    }

    fn add(&mut self, m: Value) {
        if self.masters.len() >= MAX_MASTERS {
            return;
        }
        if let Some(id) = str_field(&m, "symbolID")
            && !self.masters.contains_key(id)
        {
            self.weights.insert(id.to_string(), weight(&m, 0));
            self.masters.insert(id.to_string(), m);
        }
    }

    fn master(&self, symbol_id: &str) -> Option<&Value> {
        self.masters.get(symbol_id)
    }
}

/// What copying and mapping `v` costs, in work units: one per JSON value, one per 8 bytes of text.
pub(crate) fn weight(v: &Value, depth: usize) -> u64 {
    if depth > 256 {
        return 1;
    }
    match v {
        Value::String(s) => 1 + s.len() as u64 / 8,
        Value::Array(a) => 1 + a.iter().map(|x| weight(x, depth + 1)).sum::<u64>(),
        Value::Object(o) => 1 + o.iter().map(|(k, x)| k.len() as u64 / 8 + weight(x, depth + 1)).sum::<u64>(),
        _ => 1,
    }
}

fn masters_in<'v>(l: &'v Value, out: &mut Vec<&'v Value>, depth: usize) {
    if depth > 64 {
        return;
    }
    for c in super::tree::layers(l) {
        if str_field(c, "_class") == Some("symbolMaster") {
            out.push(c);
        } else {
            masters_in(c, out, depth + 1);
        }
    }
}

/// A master's background as REST fills (masters show one only when they have it on).
pub(crate) fn master_fills(m: &Value) -> Option<Value> {
    if bool_field(m, "hasBackgroundColor") != Some(true) {
        return None;
    }
    let c = m.get("backgroundColor").and_then(super::tree::color)?;
    Some(json!([{"type": "SOLID", "color": c}]))
}

/// What a master shows as a frame (its instances the same): clipping, and in Sketch 2025 and newer
/// its style (fills, borders, effects) and corners.
fn frame_look(cx: &mut Cx, m: &Value, w: f64, h: f64, node: &mut JsonMap<String, Value>) {
    // A symbol clips only with `clippingBehavior` 1, whatever its `groupBehavior`; 0 (the
    // default) and 2 don't.
    node.insert("clipsContent".into(), json!(m.get("clippingBehavior").and_then(Value::as_i64) == Some(1)));
    if m.get("groupBehavior").is_some()
        && let Some(st) = m.get("style")
    {
        // Tint fills aren't painted (see `style::fills`).
        super::style::apply(cx, st, w, h, node);
    }
    super::tree::style_corners(m, node);
}

/// A symbol master as a COMPONENT node.
pub(crate) fn component(cx: &mut Cx, l: &Value, parent: Affine, depth: usize, root: bool) -> Option<Value> {
    let (w, h) = size(l);
    let world = parent * local(l, w, h);
    let mut node = base(l, "COMPONENT", parent, world, w, h, root);
    frame_look(cx, l, w, h, &mut node);
    if let Some(f) = master_fills(l) {
        node.insert("fills".into(), f);
    }
    super::tree::extras(cx, l, h, &mut node);
    let sid = str_field(l, "symbolID").unwrap_or("").to_string();
    // A graphic's fill tints its layers.
    let tinted;
    let l = match super::style::tint_of(l.get("style"), false).filter(|_| super::tree::spend_on_tint(cx, l)) {
        Some(t) => {
            let mut c = l.clone();
            if let Some(kids) = c.get_mut("layers").and_then(Value::as_array_mut) {
                super::style::tint_layers(kids, &t, 0);
            }
            tinted = c;
            &tinted
        }
        _ => l,
    };
    cx.chain.push(sid);
    let kids = children(cx, l, world, (w, h), depth, false);
    cx.chain.pop();
    node.insert("children".into(), Value::Array(kids));
    super::stack::apply(l, &mut node);
    Some(Value::Object(node))
}

/// A symbol instance as an INSTANCE node holding its master's layers with its overrides, or an
/// empty frame when its master isn't in the file.
pub(crate) fn instance(cx: &mut Cx, l: &Value, parent: Affine, depth: usize, root: bool) -> Option<Value> {
    let (w, h) = size(l);
    let world = parent * local(l, w, h);
    let sid = str_field(l, "symbolID").unwrap_or("");
    let cost = cx.symbols.weights.get(sid).copied().unwrap_or(0);
    if cx.symbols.master(sid).is_some() && !cx.spend(cost) {
        let mut node = base(l, "FRAME", parent, world, w, h, root);
        node.insert("children".into(), json!([]));
        return Some(Value::Object(node));
    }
    let Some(m) = cx.symbols.master(sid).cloned() else {
        cx.warn("instances of symbols that aren't in the file were imported as empty frames");
        let mut node = base(l, "FRAME", parent, world, w, h, root);
        node.insert("children".into(), json!([]));
        return Some(Value::Object(node));
    };
    if cx.chain.iter().any(|c| c == sid) {
        let name: String = str_field(&m, "name").unwrap_or("").chars().take(64).collect();
        cx.warn(format!("symbol {name} contains itself; the inner copy was left out"));
        return None;
    }
    // Sketch 50 stores "not scaled" as 0.
    if l.get("scale").and_then(Value::as_f64).is_some_and(|s| s.is_finite() && s != 0.0 && (s - 1.0).abs() > 1e-9) {
        cx.warn("scaled symbol instances were imported at their size, unscaled");
    }
    let mut node = base(l, "INSTANCE", parent, world, w, h, root);
    node.insert("componentId".into(), json!(str_field(&m, "do_objectID").unwrap_or(sid)));
    frame_look(cx, &m, w, h, &mut node);
    // An instance shows its master's background (its fills, or an older file's background
    // colour) only when the master includes it both in instances and in exports.
    let shown = bool_field(&m, "includeBackgroundColorInInstance") == Some(true) && bool_field(&m, "includeBackgroundColorInExport") == Some(true);
    if !shown {
        node.remove("fills");
    } else if let Some(f) = master_fills(&m) {
        node.insert("fills".into(), f);
    }
    // An instance's fill is a tint for its layers, not a paint of its own.
    if let Some(st) = l.get("style") {
        let mut st = st.clone();
        if let Some(o) = st.as_object_mut() {
            o.remove("fills");
        }
        super::style::apply(cx, &st, w, h, &mut node);
    }
    super::tree::extras(cx, l, h, &mut node);
    // An instance shows its master's grids (else they'd differ, and count as an override).
    super::proto::layout_grids(&m, h, &mut node);
    let overrides = overrides(l);
    let nested = nested(&overrides);
    let mut copy = m;
    let mut budget = cx.work_left();
    if let Some(layers) = copy.get_mut("layers").and_then(Value::as_array_mut) {
        apply(layers, &overrides, &nested, &cx.shared, &cx.symbols, 0, &mut budget);
    }
    let used = cx.work_left().saturating_sub(budget);
    cx.spend(used);
    if budget == 0 {
        // The budget ran out handing overrides on: noted.
        cx.spend(1);
    }
    // The tint: the instance's own (a colour override on it, or its fill), else a graphic master's.
    let own = str_field(l, "do_objectID").map(|id| format!("{id}_fillColor"));
    let tint = own
        .and_then(|k| overrides.get(&k).filter(|c| c.is_object()).cloned())
        .or_else(|| super::style::tint_of(l.get("style"), cx.tints))
        .or_else(|| super::style::tint_of(copy.get("style"), false));
    if let Some(t) = tint
        && let Some(layers) = copy.get_mut("layers").and_then(Value::as_array_mut)
    {
        super::style::tint_layers(layers, &t, 0);
    }
    // The master's layers laid out for the instance's size, as Sketch does it.
    let (mw, mh) = size(&copy);
    // A graphic's layers resize in proportion, whatever their pins.
    let graphic = copy.get("groupBehavior").and_then(Value::as_i64) == Some(2);
    // A stack's layers are placed by its auto layout (when the file is opened), not by their pins.
    let stacked = !graphic && super::stack::is_stack(&copy);
    if ((mw - w).abs() > 1e-9 || (mh - h).abs() > 1e-9)
        && !stacked
        && let Some(layers) = copy.get_mut("layers").and_then(Value::as_array_mut)
    {
        resize(layers, (mw, mh), (w, h), cx.legacy_resize || graphic, 0);
    }
    cx.chain.push(sid.to_string());
    let mut kids = children(cx, &copy, world, (w, h), depth, false);
    cx.chain.pop();
    node.insert("children".into(), Value::Array(std::mem::take(&mut kids)));
    // The master's stack lays the instance out too (matched by the master's layer ids), sized as
    // the instance says.
    for k in ["horizontalSizing", "verticalSizing"] {
        if let Some(v) = l.get(k) {
            copy[k] = v.clone();
        }
    }
    super::stack::apply(&copy, &mut node);
    let prefix = str_field(l, "do_objectID").unwrap_or("I").to_string();
    for k in node.get_mut("children").and_then(Value::as_array_mut).into_iter().flatten() {
        prefix_ids(k, &prefix, 0);
    }
    Some(Value::Object(node))
}

/// Lay `layers` (the children of a box of `old` size) out for the box at `new` size, the way Sketch
/// resizes a symbol instance's layers: groups pass the change on to their own layers. Files older
/// than format 110 scale everything; newer ones follow each layer's `resizingConstraint`.
fn resize(layers: &mut [Value], old: (f64, f64), new: (f64, f64), legacy: bool, depth: usize) {
    if depth > 64 {
        return;
    }
    for l in layers {
        let mask = if legacy { 63 } else { super::tree::pin_mask(l) };
        let Some(f) = l.get_mut("frame").and_then(Value::as_object_mut) else { continue };
        let n = |k: &str| f.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
        let (x, y, w, h) = (n("x"), n("y"), n("width").max(0.0), n("height").max(0.0));
        // A cleared bit pins: right 1, width 2, left 4, bottom 8, height 16, top 32.
        let pinned = |bit: i64| mask & bit == 0;
        let (nx, nw) = resize_axis(x, w, old.0, new.0, (pinned(4), pinned(2), pinned(1)));
        let (ny, nh) = resize_axis(y, h, old.1, new.1, (pinned(32), pinned(16), pinned(8)));
        for (k, v) in [("x", nx), ("y", ny), ("width", nw), ("height", nh)] {
            f.insert(k.into(), json!(v));
        }
        // A graphic's layers resize in proportion, whatever their pins.
        let graphic = l.get("groupBehavior").and_then(Value::as_i64) == Some(2);
        if let Some(kids) = l.get_mut("layers").and_then(Value::as_array_mut) {
            resize(kids, (w, h), (nw, nh), legacy || graphic, depth + 1);
        }
    }
}

/// One axis of [`resize`]: a layer at `pos` of `size` in a parent going from `old` to `new`, with
/// its start, size and end pinned or not. Nothing pinned (or everything): it scales. One side
/// pinned: the size and the free side share the change, in proportion to their lengths. Only the
/// size fixed: its centre scales. Both sides pinned: the size takes the change.
fn resize_axis(pos: f64, size: f64, old: f64, new: f64, (start, fixed, end): (bool, bool, bool)) -> (f64, f64) {
    if old <= 1e-9 || !new.is_finite() {
        return (pos, size);
    }
    let k = new / old;
    let d = new - old;
    let far = old - pos - size;
    let share = |part: f64, other: f64| if part + other > 1e-9 { d * part / (part + other) } else { d };
    let (p, s) = match (start, fixed, end) {
        (true, false, false) => (pos, size + share(size, far)),
        (false, false, true) => {
            let s = size + share(size, pos);
            (new - far - s, s)
        }
        (false, true, false) => ((pos + size / 2.0) * k - size / 2.0, size),
        (true, false, true) => (pos, size + d),
        (true, true, false) => (pos, size),
        (false, true, true) => (new - far - size, size),
        _ => (pos * k, size * k),
    };
    (if p.is_finite() { p } else { pos }, if s.is_finite() { s.max(0.0) } else { size })
}

/// An instance's overrides by name (`<path>_<property>` → value).
fn overrides(l: &Value) -> BTreeMap<String, Value> {
    l.get("overrideValues")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(100_000)
        .filter_map(|o| Some((str_field(o, "overrideName")?.to_string(), o.get("value")?.clone())))
        .collect()
}

/// The overrides of layers inside nested instances, by the nested instance's id: `X/rest_prop` →
/// `X` → [(`rest_prop`, value)].
fn nested(ov: &BTreeMap<String, Value>) -> BTreeMap<&str, Vec<(&str, &Value)>> {
    let mut out: BTreeMap<&str, Vec<(&str, &Value)>> = BTreeMap::new();
    for (k, v) in ov {
        if let Some((first, rest)) = k.split_once('/') {
            out.entry(first).or_default().push((rest, v));
        }
    }
    out
}

/// Apply `ov` to the layers of a master's copy: to the layers it names, and passed on to nested
/// instances for the layers inside them (an outer override wins over the nested instance's own).
/// Each override handed to a nested instance costs [`OVERRIDE_COST`] units of `budget` plus its
/// value's [`weight`]; once it is spent, the rest aren't handed on.
fn apply(
    layers: &mut [Value],
    ov: &BTreeMap<String, Value>,
    nested: &BTreeMap<&str, Vec<(&str, &Value)>>,
    shared: &Shared,
    symbols: &Symbols,
    depth: usize,
    budget: &mut u64,
) {
    if depth > 64 || ov.is_empty() {
        return;
    }
    for l in layers {
        let Some(id) = str_field(l, "do_objectID").map(str::to_string) else { continue };
        let get = |prop: &str| ov.get(&format!("{id}_{prop}"));
        match str_field(l, "_class") {
            Some("symbolInstance") => {
                match get("symbolID").map(|v| v.as_str().unwrap_or("")) {
                    Some("") => l["isVisible"] = json!(false),
                    // A swap to a symbol the file doesn't have keeps the original, as in Sketch.
                    Some(other) if symbols.master(other).is_some() => l["symbolID"] = json!(other),
                    _ => {}
                }
                let inner = nested.get(id.as_str()).map(Vec::as_slice).unwrap_or_default();
                // Each costs its copy's size: deep chains of large overrides use up the budget.
                let mut take = 0;
                for (_, v) in inner {
                    let cost = OVERRIDE_COST + weight(v, 0);
                    if cost > *budget {
                        break;
                    }
                    *budget -= cost;
                    take += 1;
                }
                if take > 0 {
                    let mut own = l.get("overrideValues").and_then(Value::as_array).cloned().unwrap_or_default();
                    own.extend(inner.iter().take(take).map(|(rest, v)| json!({"_class": "overrideValue", "overrideName": rest, "value": v})));
                    l["overrideValues"] = Value::Array(own);
                }
                if let Some(v) = get("isVisible") {
                    l["isVisible"] = json!(visible(v));
                }
                // A colour override on an instance is its tint.
                if let Some(c) = get("fillColor").filter(|c| c.is_object()) {
                    super::style::set_tint(l, c);
                }
                continue;
            }
            Some("text") => {
                if let Some(v) = get("isVisible") {
                    l["isVisible"] = json!(visible(v));
                }
                if let Some(s) = get("stringValue").and_then(Value::as_str) {
                    set_string(l, s);
                }
                if let Some(c) = get("textColor").filter(|c| c.is_object()) {
                    super::style::tint_layers(std::slice::from_mut(l), c, 0);
                }
            }
            Some("bitmap") => {
                if let Some(v) = get("isVisible") {
                    l["isVisible"] = json!(visible(v));
                }
                if let Some(img) = get("image").filter(|v| v.get("_ref").is_some()) {
                    l["image"] = img.clone();
                }
            }
            _ => {
                if let Some(v) = get("isVisible") {
                    l["isVisible"] = json!(visible(v));
                }
                // A colour override on a shape or group: its first fill's colour.
                if let Some(c) = get("fillColor").filter(|c| c.is_object())
                    && let Some(f) = l
                        .get_mut("style")
                        .and_then(|s| s.get_mut("fills"))
                        .and_then(Value::as_array_mut)
                        .and_then(|f| f.first_mut())
                        .and_then(Value::as_object_mut)
                {
                    f.insert("fillType".into(), json!(0));
                    f.insert("color".into(), c.clone());
                }
            }
        }
        for (prop, text) in [("layerStyle", false), ("textStyle", true)] {
            let sid = get(prop).and_then(Value::as_str);
            // Shared styles that aren't objects are junk: left alone.
            let Some((sid, style)) = sid.and_then(|sid| Some((sid, shared.style(text, sid).filter(|s| s.is_object())?))) else { continue };
            let mut style = style.clone();
            if let Some(m) = style.as_object_mut() {
                // The layer now uses that shared style: it links to it.
                m.insert("sharedObjectID".into(), json!(sid));
            }
            if text
                && let Some(enc) = style.get("textStyle").and_then(|t| t.get("encodedAttributes"))
                && let Some(runs) = l.get_mut("attributedString").and_then(|a| a.get_mut("attributes")).and_then(Value::as_array_mut)
            {
                for r in runs.iter_mut().filter_map(Value::as_object_mut) {
                    r.insert("attributes".into(), enc.clone());
                }
            }
            if let Some(m) = l.as_object_mut() {
                m.insert("style".into(), style);
            }
        }
        // An image override on a shape: its top image fill's image (after a layer style override,
        // which would replace the fills).
        if str_field(l, "_class") != Some("bitmap")
            && let Some(img) = get("image").filter(|v| v.get("_ref").is_some())
            && let Some(f) = l
                .get_mut("style")
                .and_then(|s| s.get_mut("fills"))
                .and_then(Value::as_array_mut)
                .and_then(|f| {
                    f.iter_mut()
                        .rev()
                        .find(|p| p.get("fillType").and_then(Value::as_i64) == Some(4) && p.get("isEnabled").and_then(Value::as_bool) != Some(false))
                })
                .and_then(Value::as_object_mut)
        {
            f.insert("image".into(), img.clone());
        }
        if let Some(kids) = l.get_mut("layers").and_then(Value::as_array_mut) {
            apply(kids, ov, nested, shared, symbols, depth + 1, budget);
        }
    }
}

/// A visibility override's value: "0", 0 or false hide.
fn visible(v: &Value) -> bool {
    !matches!(v, Value::Bool(false)) && v.as_str() != Some("0") && v.as_i64() != Some(0)
}

/// A text layer's string replaced, in the style of its first run.
fn set_string(l: &mut Value, s: &str) {
    let units = s.encode_utf16().count();
    let first = l.get("attributedString").and_then(|a| a.get("attributes")).and_then(Value::as_array).and_then(|a| a.first()).cloned();
    let Some(a) = l.get_mut("attributedString").and_then(Value::as_object_mut) else { return };
    a.insert("string".into(), json!(s));
    let runs = match first {
        Some(Value::Object(mut r)) => {
            r.insert("location".into(), json!(0));
            r.insert("length".into(), json!(units));
            vec![Value::Object(r)]
        }
        _ => Vec::new(),
    };
    a.insert("attributes".into(), Value::Array(runs));
}

/// Make the ids of an instance's layers its own (`<instance>;<layer>`): the master's layers keep
/// theirs.
fn prefix_ids(v: &mut Value, prefix: &str, depth: usize) {
    if depth > 256 {
        return;
    }
    let Some(m) = v.as_object_mut() else { return };
    if let Some(id) = m.get("id").and_then(Value::as_str).map(str::to_string) {
        m.insert("id".into(), json!(format!("{prefix};{id}")));
    }
    if let Some(kids) = m.get_mut("children").and_then(Value::as_array_mut) {
        for k in kids {
            prefix_ids(k, prefix, depth + 1);
        }
    }
}

/// Library masters no page holds: a page of their own, side by side.
pub(crate) fn library_page(cx: &mut Cx, masters: &[Value]) -> Value {
    let mut x = 0.0;
    let mut kids = Vec::new();
    for m in masters {
        let mut placed = m.clone();
        let (w, _) = size(m);
        if let Some(f) = placed.get_mut("frame").and_then(Value::as_object_mut) {
            f.insert("x".into(), json!(x));
            f.insert("y".into(), json!(0.0));
        }
        x += w + 100.0;
        if let Some(node) = component(cx, &placed, Affine::IDENTITY, 0, true) {
            kids.push(node);
        }
    }
    let mut page = JsonMap::new();
    page.insert("type".into(), json!("CANVAS"));
    page.insert("id".into(), json!("library-symbols"));
    page.insert("name".into(), json!("Library symbols"));
    page.insert("children".into(), Value::Array(kids));
    Value::Object(page)
}
