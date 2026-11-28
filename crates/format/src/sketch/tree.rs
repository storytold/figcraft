//! Sketch pages and layers → REST API file JSON nodes.

use figcraft_doc::VectorNetwork;
use figcraft_doc::geom::{Affine, Rect};
use serde_json::{Map as JsonMap, Value, json};

use super::Archive;

/// Most distinct notes kept.
const MAX_NOTES: usize = 200;
/// Work units symbol expansion may spend on one file (one per JSON value copied, see
/// `symbols::weight`): about three times what the largest design kit checked spends (21M, with
/// 2 GB at its peak), well below what runs out of memory; less on 32-bit targets (the web), where
/// such a kit keeps its first instances and notes the rest.
#[cfg(not(target_pointer_width = "32"))]
const MAX_WORK: u64 = 60_000_000;
#[cfg(target_pointer_width = "32")]
const MAX_WORK: u64 = 15_000_000;
/// Deepest layer nesting followed (deeper layers are skipped with a warning).
const MAX_DEPTH: usize = 200;
/// Largest size and farthest position kept (the REST importer's own bounds).
const MAX_SIZE: f64 = 1e6;
const MAX_COORD: f64 = 1e7;

/// The REST JSON of a Sketch file, with what the mapping noticed.
pub(crate) struct Map {
    pub json: Value,
    pub warnings: Vec<String>,
    /// Vector networks the JSON references by `vectorNetworkRef`.
    pub networks: Vec<Option<VectorNetwork>>,
    /// A text layer stored the pre-48 way was met: the file is refused.
    pub too_old: bool,
}

/// The mapping's state while walking the file.
pub(crate) struct Cx {
    pub symbols: super::symbols::Symbols,
    pub shared: super::shared::Shared,
    /// Ids of the styles made from the shared styles.
    pub style_defs: std::collections::BTreeSet<String>,
    /// Work spent expanding symbols, and the most the file may take (see `symbols::weight`).
    pub work: u64,
    pub max_work: u64,
    /// Notes given so far (each once).
    pub seen: std::collections::BTreeSet<String>,
    /// Inside a shape group: whether its shapes are filled (open ones then close) and whether the
    /// group has a border. Outside one: `None` / false (a shape's own style decides).
    pub operand_fill: Option<bool>,
    pub operand_border: bool,
    /// The file is older than format 110 (Sketch 52): Sketch scales a resized instance's layers
    /// instead of following their pins.
    pub legacy_resize: bool,
    /// The file is newer than format 123 (Sketch 63): a group's or instance's plain fill can be a
    /// tint (Sketch 64 added tints; older files' group fills draw nothing).
    pub tints: bool,
    /// The symbols being expanded, outermost first (a symbol inside itself stops there).
    pub chain: Vec<String>,
    pub warnings: Vec<String>,
    pub networks: Vec<Option<VectorNetwork>>,
    pub too_old: bool,
    /// Layers mapped so far, and the most the file may give.
    pub nodes: usize,
    pub max_nodes: usize,
    /// Finds installed faces by PostScript name, and what it found (or didn't) so far.
    pub fonts: Option<super::FontLookup>,
    pub font_cache: std::collections::BTreeMap<String, Option<(String, String)>>,
}

impl Cx {
    pub(crate) fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if self.seen.len() > MAX_NOTES || !self.seen.insert(msg.clone()) {
            return;
        }
        if self.seen.len() > MAX_NOTES {
            self.warnings.push("more notes were left out".into());
        } else {
            self.warnings.push(msg);
        }
    }

    /// Spend `n` units of the work budget; false (with a note, once) when it would run out.
    pub(crate) fn spend(&mut self, n: u64) -> bool {
        if self.work.saturating_add(n) > self.max_work {
            self.work = self.max_work;
            self.warn("the file's symbols expand to more than the import handles; the instances past that were imported as empty frames");
            return false;
        }
        self.work += n;
        true
    }

    /// Spend `n` units of the work budget on tinting; false (with a note, once) when it would run
    /// out.
    pub(crate) fn spend_on_tint(&mut self, n: u64) -> bool {
        if self.work.saturating_add(n) > self.max_work {
            self.warn("tints past what the import handles were skipped; those layers keep their own colours");
            return false;
        }
        self.work += n;
        true
    }

    pub(crate) fn work_left(&self) -> u64 {
        self.max_work.saturating_sub(self.work)
    }
}

/// Charge the copy tinting layer `l` makes against the work budget: whether it may be tinted.
pub(crate) fn spend_on_tint(cx: &mut Cx, l: &Value) -> bool {
    cx.spend_on_tint(super::symbols::weight(l, 0))
}

/// Map the pages of `archive` to a REST file JSON named `name`.
pub(crate) fn to_rest(archive: &Archive, name: &str, max_nodes: usize, fonts: Option<super::FontLookup>) -> Map {
    let symbols = super::symbols::Symbols::read(&archive.document, &archive.pages);
    let shared = super::shared::Shared::read(&archive.document);
    let mut cx = Cx {
        symbols,
        shared,
        style_defs: std::collections::BTreeSet::new(),
        work: 0,
        max_work: MAX_WORK,
        seen: std::collections::BTreeSet::new(),
        operand_fill: None,
        operand_border: false,
        legacy_resize: archive.version > 0 && archive.version < 110,
        tints: archive.version == 0 || archive.version > 123,
        chain: Vec::new(),
        warnings: Vec::new(),
        networks: Vec::new(),
        too_old: false,
        nodes: 0,
        max_nodes,
        fonts,
        font_cache: std::collections::BTreeMap::new(),
    };
    let defs = super::shared::Shared::definitions(&mut cx);
    cx.style_defs = defs.iter().filter_map(|d| d.get("id").and_then(Value::as_str).map(str::to_string)).collect();
    let mut canvases = Vec::new();
    for (id, page) in &archive.pages {
        let mut node = JsonMap::new();
        node.insert("type".into(), json!("CANVAS"));
        node.insert("id".into(), json!(id));
        node.insert("name".into(), json!(str_field(page, "name").unwrap_or("Page")));
        let kids = children(&mut cx, page, Affine::IDENTITY, (0.0, 0.0), 0, true);
        node.insert("children".into(), Value::Array(kids));
        let starts = super::proto::starting_points(page);
        if !starts.is_empty() {
            node.insert("flowStartingPoints".into(), Value::Array(starts));
        }
        canvases.push(Value::Object(node));
    }
    let foreign = std::mem::take(&mut cx.symbols.foreign);
    if !foreign.is_empty() {
        canvases.push(super::symbols::library_page(&mut cx, &foreign));
        let libraries = cx.symbols.libraries.join(", ");
        cx.warn(format!("symbols from the libraries {libraries} became local components on the page Library symbols"));
    }
    let mut json = json!({"name": name, "document": {"type": "DOCUMENT", "id": "0:0", "children": canvases}, "styleDefinitions": defs});
    if let Some((collections, variables)) = cx.shared.variables() {
        json["variableCollections"] = collections;
        json["variables"] = variables;
    }
    Map { json, warnings: cx.warnings, networks: cx.networks, too_old: cx.too_old }
}

/// One Sketch layer as a REST node (with its children), or `None` when it is skipped. `parent`:
/// the parent layer's transform to the page; `root`: the layer sits on the page itself.
pub(crate) fn layer(cx: &mut Cx, l: &Value, parent: Affine, depth: usize, root: bool) -> Option<Value> {
    if depth > MAX_DEPTH {
        cx.warn(format!("layers nested deeper than {MAX_DEPTH} levels were skipped"));
        return None;
    }
    if is_archived_text(l) {
        cx.too_old = true;
        return None;
    }
    // A rectangle with angled or inside corners is a path of that outline.
    let outlined;
    let l = match super::path::cut_corners(l) {
        Some(p) => {
            outlined = p;
            &outlined
        }
        None => l,
    };
    let class = str_field(l, "_class")?;
    if class == "shapeGroup" {
        return count(cx).then(|| super::path::shape_group(cx, l, parent, depth, root)).flatten();
    }
    if class == "symbolMaster" {
        return count(cx).then(|| super::symbols::component(cx, l, parent, depth, root)).flatten();
    }
    if class == "symbolInstance" {
        return count(cx).then(|| super::symbols::instance(cx, l, parent, depth, root)).flatten();
    }
    let ty = match class {
        "artboard" | "frame" => "FRAME",
        // Sketch 2025 and newer: a group that behaves as a frame (1) or a graphic (2), or clips.
        "group" if is_frame(l) => "FRAME",
        "group" | "graphic" => "GROUP",
        // Shapes whose points were moved are paths.
        "rectangle" | "oval" if bool_field(l, "edited") == Some(true) => "VECTOR",
        "rectangle" | "bitmap" => "RECTANGLE",
        "oval" => "ELLIPSE",
        "slice" => "SLICE",
        "shapePath" | "star" | "polygon" | "triangle" => "VECTOR",
        "text" => "TEXT",
        "MSImmutableHotspotLayer" | "hotspot" => "FRAME",
        other => {
            cx.warn(format!("Sketch {} layers are not supported yet and were skipped", clip(other)));
            return None;
        }
    };
    if !count(cx) {
        return None;
    }
    let (w, h) = size(l);
    let mut world = parent * local(l, w, h);
    if ty == "TEXT" {
        world *= Affine::translate((0.0, super::text::baseline_shift(l)));
    }
    let mut node = base(l, ty, parent, world, w, h, root);
    if ty == "TEXT" {
        super::text::node(cx, l, &mut node);
    }
    if ty == "VECTOR" {
        let shape = super::path::shape(cx, l, w, h)?;
        super::path::put_shape(cx, shape, &mut node);
    }
    mask(l, &mut node);
    let mut bitmap = None;
    match class {
        "artboard" | "frame" => {
            let clips = if class == "artboard" { true } else { bool_field(l, "clipsContent").unwrap_or(true) };
            node.insert("clipsContent".into(), json!(clips));
            let background = (bool_field(l, "hasBackgroundColor") == Some(true)).then(|| l.get("backgroundColor").and_then(color)).flatten();
            if class == "artboard" || background.is_some() {
                let c = background.unwrap_or_else(|| json!({"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}));
                node.insert("fills".into(), json!([{"type": "SOLID", "color": c}]));
            }
        }
        "group" if ty == "FRAME" => {
            // `clippingBehavior`: 0 the default (frames clip), 1 clip, 2 don't.
            node.insert("clipsContent".into(), json!(l.get("clippingBehavior").and_then(Value::as_i64) != Some(2)));
        }
        "rectangle" => corner_radii(l, &mut node),
        "bitmap" => match l.get("image").and_then(|i| str_field(i, "_ref")) {
            Some(r) => {
                let mut paint = json!({"type": "IMAGE", "scaleMode": "STRETCH", "imageRef": r, "imageTransform": [[1, 0, 0], [0, 1, 0]]});
                if let Some(f) = super::style::color_controls(cx, l.get("style")) {
                    paint["filters"] = f;
                }
                bitmap = Some(paint);
            }
            None => cx.warn("images stored inside the JSON were skipped"),
        },
        _ => {}
    }
    styles(cx, l, w, h, &mut node);
    // A bitmap's own image lies under its style's fills; its fills then aren't a shared style's.
    if let Some(paint) = bitmap {
        let mut fills = vec![paint];
        if let Some(Value::Array(f)) = node.remove("fills") {
            fills.extend(f);
        }
        node.insert("fills".into(), Value::Array(fills));
        if let Some(styles) = node.get_mut("styles").and_then(Value::as_object_mut) {
            styles.remove("fill");
        }
    }
    extras(cx, l, h, &mut node);
    style_corners(l, &mut node);
    if class == "MSImmutableHotspotLayer" || class == "hotspot" {
        node.insert("clipsContent".into(), json!(false));
        node.remove("fills");
    }
    // A group's or graphic's fill tints its layers (a frame's is its background).
    let tinted;
    let gb = l.get("groupBehavior").and_then(Value::as_i64);
    let tint = if class == "group" { super::style::tint_of(l.get("style"), gb != Some(1) && cx.tints) } else { None };
    let l = match tint.filter(|_| spend_on_tint(cx, l)) {
        Some(t) => {
            // A plain group's legacy fill is all tint; a frame keeps its background fills.
            if gb != Some(1) {
                node.remove("fills");
            }
            let mut c = l.clone();
            if let Some(kids) = c.get_mut("layers").and_then(Value::as_array_mut) {
                super::style::tint_layers(kids, &t, 0);
            }
            tinted = c;
            &tinted
        }
        _ => l,
    };
    if matches!(ty, "FRAME" | "GROUP") {
        let kids = children(cx, l, world, (w, h), depth, false);
        node.insert("children".into(), Value::Array(kids));
        super::stack::apply(l, &mut node);
    }
    Some(fade(cx, l, node, world, w, h))
}

/// Layer `node` with a progressive opacity (Fade: the style's `contextSettings` with
/// `isProgressive` and a gradient whose stops' opacity is the layer's) as a group in its place
/// holding an alpha mask of that gradient over it; FigCraft has no progressive opacity. Other
/// layers as they are.
fn fade(cx: &mut Cx, l: &Value, node: JsonMap<String, Value>, world: Affine, w: f64, h: f64) -> Value {
    let ctx = l.get("style").and_then(|s| s.get("contextSettings"));
    let gradient = ctx.filter(|c| c.get("isProgressive").and_then(Value::as_bool) == Some(true)).and_then(|c| c.get("gradient"));
    // A mask stays as it is: its group would stop masking the layers above it.
    let Some(gradient) = gradient.filter(|_| node.get("isMask").and_then(Value::as_bool) != Some(true)) else { return Value::Object(node) };
    let fill = json!({"_class": "fill", "isEnabled": true, "fillType": 1, "gradient": gradient});
    let fills = super::style::fills(cx, &json!({"fills": [fill]}), w, h);
    if fills.is_empty() {
        return Value::Object(node);
    }
    let id = str_field(l, "do_objectID").unwrap_or("layer");
    let identity = json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    let bb = world.transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
    let abs = json!({"x": bb.x0, "y": bb.y0, "width": bb.width(), "height": bb.height()});
    // The group takes the layer's id (stacks, links and overrides find it), the layer a new one.
    let mut inner = node;
    let mut group = JsonMap::new();
    for k in ["relativeTransform", "absoluteBoundingBox", "constraints", "visible", "locked"] {
        if let Some(v) = inner.get(k) {
            group.insert(k.into(), v.clone());
        }
    }
    group.insert("type".into(), json!("GROUP"));
    group.insert("id".into(), json!(id));
    group.insert("name".into(), inner.get("name").cloned().unwrap_or_else(|| json!("Fade")));
    group.insert("size".into(), json!({"x": w, "y": h}));
    group.insert("absoluteBoundingBox".into(), abs.clone());
    inner.insert("id".into(), json!(format!("{id}-faded")));
    inner.insert("relativeTransform".into(), identity.clone());
    inner.insert("absoluteBoundingBox".into(), abs.clone());
    inner.insert("visible".into(), json!(true));
    let mask = json!({"type": "RECTANGLE", "id": format!("{id}-fade-mask"), "name": "Fade", "visible": true, "size": {"x": w, "y": h},
        "relativeTransform": identity, "absoluteBoundingBox": abs, "fills": fills, "isMask": true, "maskType": "ALPHA"});
    group.insert("children".into(), json!([mask, Value::Object(inner)]));
    Value::Object(group)
}

/// A Sketch 2025+ group that is a frame: it behaves as a frame or a graphic, or clips.
pub(crate) fn is_frame(l: &Value) -> bool {
    matches!(l.get("groupBehavior").and_then(Value::as_i64), Some(1 | 2)) || l.get("clippingBehavior").and_then(Value::as_i64) == Some(1)
}

/// Corners kept in the style (Sketch 2025+: one radius for all, or one per corner from the top
/// left, clockwise) on shapes and frames that have corners; smooth corners keep their smoothing.
pub(crate) fn style_corners(l: &Value, node: &mut JsonMap<String, Value>) {
    let Some(c) = l.get("style").and_then(|s| s.get("corners")) else { return };
    if !matches!(node.get("type").and_then(Value::as_str), Some("RECTANGLE" | "FRAME" | "COMPONENT" | "INSTANCE")) {
        return;
    }
    let radii: Vec<f64> = c
        .get("radii")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_f64)
        .filter(|r| r.is_finite())
        .map(|r| r.clamp(0.0, MAX_SIZE))
        .take(4)
        .collect();
    match radii.as_slice() {
        [] => return,
        [r] => {
            node.insert("cornerRadius".into(), json!(r));
            node.remove("rectangleCornerRadii");
        }
        more => {
            // Values repeat to fill the four corners.
            let all: Vec<f64> = (0..4).map(|i| more.get(i % more.len()).copied().unwrap_or(0.0)).collect();
            node.insert("rectangleCornerRadii".into(), json!(all));
        }
    }
    // Style 1 is smooth: its smoothing applies.
    if c.get("style").and_then(Value::as_i64) == Some(1)
        && let Some(sm) = c.get("smoothing").and_then(Value::as_f64).filter(|v| v.is_finite())
    {
        node.insert("cornerSmoothing".into(), json!(sm.clamp(0.0, 1.0)));
    }
}

/// Layer `l`'s style into `node`, and its links to the styles made from its shared style.
pub(crate) fn styles(cx: &mut Cx, l: &Value, w: f64, h: f64, node: &mut JsonMap<String, Value>) {
    // An artboard shows its background colour only; Sketch doesn't draw a style it carries.
    if str_field(l, "_class") == Some("artboard") {
        return;
    }
    if let Some(st) = l.get("style") {
        super::style::apply(cx, st, w, h, node);
    }
    cx.shared.link(&cx.style_defs, l, node);
}

/// Layer `l`'s prototype link, export settings, layout grids (frames) and notes.
pub(crate) fn extras(cx: &mut Cx, l: &Value, h: f64, node: &mut JsonMap<String, Value>) {
    super::proto::interactions(l, node);
    super::proto::export_settings(cx, l, node);
    if matches!(node.get("type").and_then(Value::as_str), Some("FRAME" | "COMPONENT" | "INSTANCE")) {
        super::proto::layout_grids(l, h, node);
    }
    super::proto::notes(cx, l);
}

/// A clipping mask's flag and kind (outline or alpha).
pub(crate) fn mask(l: &Value, node: &mut JsonMap<String, Value>) {
    if bool_field(l, "hasClippingMask") == Some(true) {
        node.insert("isMask".into(), json!(true));
        node.insert("maskType".into(), json!(if l.get("clippingMaskMode").and_then(Value::as_i64) == Some(1) { "ALPHA" } else { "VECTOR" }));
    }
}

/// Count one more layer; false (with a warning) once the file's budget is spent.
fn count(cx: &mut Cx) -> bool {
    if cx.nodes >= cx.max_nodes {
        cx.warn(format!("the file has more than {} layers; the rest were skipped", cx.max_nodes));
        return false;
    }
    cx.nodes += 1;
    true
}

/// The properties every REST node of layer `l` has: its type, id, name, visibility, lock, size,
/// place (`world`, relative to `parent`) and constraints.
pub(crate) fn base(l: &Value, ty: &str, parent: Affine, world: Affine, w: f64, h: f64, root: bool) -> JsonMap<String, Value> {
    let mut node = JsonMap::new();
    node.insert("type".into(), json!(ty));
    if let Some(id) = str_field(l, "do_objectID") {
        node.insert("id".into(), json!(id));
    }
    let class = str_field(l, "_class").unwrap_or("Layer");
    node.insert("name".into(), json!(str_field(l, "name").unwrap_or(class)));
    node.insert("visible".into(), json!(bool_field(l, "isVisible").unwrap_or(true)));
    node.insert("locked".into(), json!(bool_field(l, "isLocked").unwrap_or(false)));
    node.insert("size".into(), json!({"x": w, "y": h}));
    let [a, b, c, d, e, f] = (parent.inverse() * world).as_coeffs();
    node.insert("relativeTransform".into(), json!([[a, c, e], [b, d, f]]));
    // The page places top-level layers by their transform; deeper ones also say where they end up,
    // which tells the importer their transform is relative to their group.
    if !root {
        let bb = world.transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
        node.insert("absoluteBoundingBox".into(), json!({"x": bb.x0, "y": bb.y0, "width": bb.width(), "height": bb.height()}));
    }
    node.insert("constraints".into(), constraints(pin_mask(l)));
    node
}

/// A layer's size from its `frame` (finite, not negative, at most [`MAX_SIZE`]).
pub(crate) fn size(l: &Value) -> (f64, f64) {
    let f = l.get("frame");
    let n = |k: &str| f.and_then(|f| f.get(k)).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, MAX_SIZE);
    (n("width"), n("height"))
}

/// A layer's transform within its parent: its frame's position, then its rotation (degrees,
/// counter-clockwise on screen) and flips about its centre. Sketch flips on screen, after rotating
/// (a quarter-turned layer flipped left to right mirrors its own up and down).
pub(crate) fn local(l: &Value, w: f64, h: f64) -> Affine {
    let f = l.get("frame");
    let n = |k: &str| f.and_then(|f| f.get(k)).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-MAX_COORD, MAX_COORD);
    let rot = l.get("rotation").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0) % 360.0;
    let sx = if bool_field(l, "isFlippedHorizontal") == Some(true) { -1.0 } else { 1.0 };
    let sy = if bool_field(l, "isFlippedVertical") == Some(true) { -1.0 } else { 1.0 };
    Affine::translate((n("x") + w / 2.0, n("y") + h / 2.0))
        * Affine::scale_non_uniform(sx, sy)
        * Affine::rotate(-rot.to_radians())
        * Affine::translate((-w / 2.0, -h / 2.0))
}

/// A layer's pins as a `resizingConstraint` mask. Sketch 2025 and newer keep them in
/// `horizontalPins`/`verticalPins` (bits: 1 the start, 4 the end) and `horizontalSizing`/
/// `verticalSizing` (fixed and fit keep the size; fill and relative don't), with
/// `resizingConstraint` left at 63.
pub(crate) fn pin_mask(l: &Value) -> i64 {
    if l.get("horizontalPins").is_none() && l.get("horizontalSizing").is_none() {
        return l.get("resizingConstraint").and_then(Value::as_i64).unwrap_or(63);
    }
    let int = |k: &str| l.get(k).and_then(Value::as_i64).unwrap_or(0);
    let fixed = |k: &str| matches!(int(k), 0 | 1);
    let mut mask = 63;
    // A cleared bit pins: right 1, width 2, left 4, bottom 8, height 16, top 32.
    for (on, bit) in [
        (int("horizontalPins") & 1 != 0, 4),
        (int("horizontalPins") & 4 != 0, 1),
        (fixed("horizontalSizing"), 2),
        (int("verticalPins") & 1 != 0, 32),
        (int("verticalPins") & 4 != 0, 8),
        (fixed("verticalSizing"), 16),
    ] {
        if on {
            mask &= !bit;
        }
    }
    mask
}

/// REST constraints from Sketch's `resizingConstraint`: six bits, a cleared bit pins that side or
/// fixes that size (right 1, width 2, left 4, bottom 8, height 16, top 32). A layer pinned on
/// neither side keeps its place in proportion (scale) unless its size is fixed (centre).
fn constraints(mask: i64) -> Value {
    let pinned = |bit: i64| mask & bit == 0;
    let axis = |low: i64, size: i64, high: i64, low_name: &str, high_name: &str, both: &str| match (pinned(low), pinned(high)) {
        // Both sides and the size pinned can't all hold: Sketch scales such a layer.
        (true, true) if pinned(size) => "SCALE".into(),
        (true, true) => both.to_string(),
        (true, false) => low_name.to_string(),
        (false, true) => high_name.to_string(),
        (false, false) if pinned(size) => "CENTER".into(),
        (false, false) => "SCALE".into(),
    };
    json!({
        "horizontal": axis(4, 2, 1, "LEFT", "RIGHT", "LEFT_RIGHT"),
        "vertical": axis(32, 16, 8, "TOP", "BOTTOM", "TOP_BOTTOM"),
    })
}

/// A rectangle's corner radii: per corner from its points (top left, top right, bottom right,
/// bottom left), else its `fixedRadius`.
fn corner_radii(l: &Value, node: &mut JsonMap<String, Value>) {
    let radius = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|r| r.is_finite()).map_or(0.0, |r| r.clamp(0.0, MAX_SIZE));
    let pts = l.get("points").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let each: Vec<f64> = pts.iter().map(|p| radius(p.get("cornerRadius"))).collect();
    if each.len() == 4 && each.iter().any(|r| *r > 0.0) {
        node.insert("rectangleCornerRadii".into(), json!(each));
    } else if radius(l.get("fixedRadius")) > 0.0 {
        node.insert("cornerRadius".into(), json!(radius(l.get("fixedRadius"))));
    }
}

/// A Sketch colour (`red`, `green`, `blue`, `alpha` in 0–1) as a REST colour.
pub(crate) fn color(c: &Value) -> Option<Value> {
    let n = |k: &str| c.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0));
    Some(json!({"r": n("red")?, "g": n("green")?, "b": n("blue")?, "a": n("alpha").unwrap_or(1.0)}))
}

pub(crate) fn bool_field(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// At most 64 characters of a name read from the file, for warnings.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

/// The child layers of `l` (placed at `world`, `size`) as REST nodes. A clipping mask clips the
/// layers after it up to one that breaks the chain (or the next mask); when that happens before
/// the end, the mask and what it clips are wrapped in a group of their own, since a FigCraft mask
/// clips everything above it.
pub(crate) fn children(cx: &mut Cx, l: &Value, world: Affine, size: (f64, f64), depth: usize, root: bool) -> Vec<Value> {
    // (ends a mask chain, is a drawn copy of the mask after it, the node)
    let built: Vec<(bool, bool, Value)> = layers(l)
        .iter()
        .filter_map(|c| {
            let breaks = bool_field(c, "shouldBreakMaskChain") == Some(true) || bool_field(c, "hasClippingMask") == Some(true);
            layer(cx, c, world, depth + 1, root).map(|n| (breaks, n))
        })
        .flat_map(|(breaks, n)| match drawn_mask(&n) {
            // Sketch also draws a mask with its own style, under what it clips; a FigCraft mask
            // only clips, so a plain copy of it goes underneath.
            // The copy ends an earlier mask's chain as the mask itself would.
            Some(copy) => vec![(breaks, true, copy), (breaks, false, n)],
            None => vec![(breaks, false, n)],
        })
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some((_, _, n)) = built.get(i) {
        // An outline mask's opacity fades the mask and what it clips together, as one group (an
        // alpha mask's opacity already does, as alpha).
        let fade = (n.get("isMask").and_then(Value::as_bool) == Some(true) && n.get("maskType").and_then(Value::as_str) != Some("ALPHA"))
            .then(|| n.get("opacity").and_then(Value::as_f64))
            .flatten()
            .filter(|o| o.is_finite() && *o < 1.0);
        let end = if n.get("isMask").and_then(Value::as_bool) == Some(true) {
            built.iter().skip(i + 1).position(|(b, _, _)| *b).map(|k| i + 1 + k).or_else(|| fade.map(|_| built.len()))
        } else {
            None
        };
        let Some(end) = end else {
            out.push(n.clone());
            i += 1;
            continue;
        };
        // A faded mask's drawn copy goes into the group too, at full opacity.
        let start = match (fade, i.checked_sub(1).and_then(|k| built.get(k))) {
            (Some(_), Some((_, true, _))) => {
                out.pop();
                i - 1
            }
            _ => i,
        };
        let masked: Vec<Value> = built
            .get(start..end)
            .unwrap_or_default()
            .iter()
            .map(|(_, copy, n)| {
                let mut n = n.clone();
                if *copy
                    && fade.is_some()
                    && let Some(m) = n.as_object_mut()
                {
                    m.remove("opacity");
                }
                n
            })
            .collect();
        let mut g = JsonMap::new();
        g.insert("type".into(), json!("GROUP"));
        g.insert("name".into(), json!("Mask group"));
        g.insert("relativeTransform".into(), json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]));
        g.insert("size".into(), json!({"x": size.0, "y": size.1}));
        if !root {
            let bb = world.transform_rect_bbox(Rect::new(0.0, 0.0, size.0, size.1));
            g.insert("absoluteBoundingBox".into(), json!({"x": bb.x0, "y": bb.y0, "width": bb.width(), "height": bb.height()}));
        }
        if let Some(o) = fade {
            g.insert("opacity".into(), json!(o.clamp(0.0, 1.0)));
        }
        g.insert("children".into(), Value::Array(masked));
        out.push(Value::Object(g));
        i = end;
    }
    out
}

/// A plain copy of outline mask `n` when its style shows something (fills, borders or effects).
/// An alpha mask only masks.
fn drawn_mask(n: &Value) -> Option<Value> {
    if n.get("isMask").and_then(Value::as_bool) != Some(true) || n.get("maskType").and_then(Value::as_str) == Some("ALPHA") {
        return None;
    }
    let shows =
        |key: &str| n.get(key).and_then(Value::as_array).is_some_and(|l| l.iter().any(|p| p.get("visible").and_then(Value::as_bool) != Some(false)));
    if !(shows("fills") || shows("strokes") || shows("effects")) {
        return None;
    }
    let mut copy = n.clone();
    let m = copy.as_object_mut()?;
    m.remove("isMask");
    m.remove("maskType");
    m.remove("id");
    let name = m.get("name").and_then(Value::as_str).unwrap_or("Mask").to_string();
    m.insert("name".into(), json!(format!("{name} (drawn)")));
    Some(copy)
}

/// A text layer whose string is a binary property list (Sketch 43–47).
fn is_archived_text(l: &Value) -> bool {
    l.get("attributedString").is_some_and(|a| a.get("archivedAttributedString").is_some())
}

/// The child layers of a page or layer.
pub(crate) fn layers(l: &Value) -> &[Value] {
    l.get("layers").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

pub(crate) fn str_field<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}
