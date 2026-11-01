//! Dev-mode inspection: a layer's properties grouped for handoff (layout, auto layout,
//! typography, fills, strokes, effects), each with the style or variable behind it when there is
//! one. The Inspect tab shows these, and `inspect.properties` returns them.

use figcraft_color::{Color, GradientKind, Hsb, Hsl, PaintKind};
use figcraft_doc::{
    AutoLayout, ContentAlign, CrossAlign, DevStatus, Document, EffectKind, ExportFormat, ExportSetting, LayoutMode, MainAlign, Node, NodeId,
    NodeKind, NoiseMode, StrokeAlign, StyleId, TextAlignH, TextData, TextDecoration, Variable,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{get, id, id_of, string, targets};
use crate::spec;

/// One group of properties ("Layout", "Typography"…).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Prop>,
}

/// One property: its label and value as shown, an optional colour swatch (hex with alpha) and
/// the style or variable name it comes from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Prop {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Why a layer is listed as an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    /// It has export settings.
    Export,
    /// A small layer made only of vector shapes (exported as SVG).
    Icon,
    /// A layer filled with an image (exported as PNG).
    Image,
}

/// A layer in the inspected selection to hand over as a file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Asset {
    pub id: NodeId,
    pub name: String,
    pub kind: AssetKind,
    /// How it exports, as shown ("PNG 2x", "SVG 1x"…).
    pub exports: Vec<String>,
    /// How it exports: its export settings, or SVG for an icon and PNG for an image.
    #[serde(skip)]
    pub settings: Vec<ExportSetting>,
}

/// Most assets listed for one selection.
const MAX_ASSETS: usize = 200;
/// Largest side of a layer listed as an icon.
const ICON_MAX: f64 = 128.0;

/// Whether `id` (visible parts only) is made of vector shapes alone: paths, booleans, stars,
/// polygons and lines, with rectangles and ellipses among them, in groups or frames. `paths` is
/// set when one of the former is in it.
fn vector_only(doc: &Document, id: NodeId, depth: usize, paths: &mut bool) -> bool {
    let Some(n) = doc.node(id) else { return false };
    if !n.visible {
        return true;
    }
    if n.fills.iter().chain(&n.strokes).any(|p| matches!(p.kind, PaintKind::Image(_) | PaintKind::Pattern(_))) {
        return false;
    }
    match n.kind {
        NodeKind::Vector { .. } | NodeKind::BooleanOp { .. } | NodeKind::Star { .. } | NodeKind::Polygon { .. } | NodeKind::Line => {
            *paths = true;
            true
        }
        NodeKind::Rectangle | NodeKind::Ellipse { .. } => true,
        NodeKind::Group | NodeKind::Frame | NodeKind::Component { .. } | NodeKind::Instance { .. } => {
            depth < 16 && !n.children.is_empty() && n.children.iter().all(|c| vector_only(doc, *c, depth + 1, paths))
        }
        _ => false,
    }
}

/// Why `n` is an asset, if it is one.
fn asset_kind(doc: &Document, n: &Node) -> Option<AssetKind> {
    let mut paths = false;
    if !n.exports.is_empty() {
        Some(AssetKind::Export)
    } else if n.width <= ICON_MAX && n.height <= ICON_MAX && vector_only(doc, n.id, 0, &mut paths) && paths {
        Some(AssetKind::Icon)
    } else if n.fills.iter().any(|p| p.visible && matches!(p.kind, PaintKind::Image(_))) {
        Some(AssetKind::Image)
    } else {
        None
    }
}

/// The assets in and under `ids`, in layer order: layers with export settings, icons and images
/// (an icon's own shapes aren't listed again).
pub fn assets(doc: &Document, ids: &[NodeId]) -> Vec<Asset> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = ids.iter().rev().copied().collect();
    // Bounded: the Inspect tab asks every frame.
    let mut visited = 0usize;
    while let Some(id) = stack.pop() {
        visited += 1;
        if out.len() >= MAX_ASSETS || visited > 20_000 {
            break;
        }
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.is_page() {
            if n.is_page() {
                stack.extend(n.children.iter().rev().copied());
            }
            continue;
        }
        let kind = asset_kind(doc, n);
        if let Some(kind) = kind
            && !out.iter().any(|a: &Asset| a.id == id)
        {
            let settings = match kind {
                AssetKind::Export => n.exports.clone(),
                AssetKind::Icon => vec![ExportSetting { format: ExportFormat::Svg, ..Default::default() }],
                AssetKind::Image => vec![ExportSetting::default()],
            };
            let exports = settings.iter().map(|e| format!("{} {}", e.format.label(), e.size.label())).collect();
            out.push(Asset { id, name: n.name.clone(), kind, exports, settings });
        }
        if kind != Some(AssetKind::Icon) {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}

/// The files exporting the assets in and under `ids` makes (see [`assets`]).
pub fn asset_jobs(doc: &Document, ids: &[NodeId]) -> Vec<crate::cmd::file::ExportJob> {
    crate::cmd::file::jobs_for(assets(doc, ids).into_iter().map(|a| (a.id, a.settings)), doc)
}

/// How the Inspect tab writes colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorFormat {
    /// `#FF8000`
    #[default]
    Hex,
    /// `255, 128, 0`
    Rgb,
    /// `rgb(255, 128, 0)`, `rgba(255, 128, 0, 0.5)`
    Css,
    /// `30, 100%, 50%`
    Hsl,
    /// `30, 100%, 100%`
    Hsb,
}

impl ColorFormat {
    pub const ALL: [ColorFormat; 5] = [ColorFormat::Hex, ColorFormat::Rgb, ColorFormat::Css, ColorFormat::Hsl, ColorFormat::Hsb];

    pub fn id(self) -> &'static str {
        match self {
            ColorFormat::Hex => "hex",
            ColorFormat::Rgb => "rgb",
            ColorFormat::Css => "css",
            ColorFormat::Hsl => "hsl",
            ColorFormat::Hsb => "hsb",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorFormat::Hex => "Hex",
            ColorFormat::Rgb => "RGB",
            ColorFormat::Css => "CSS",
            ColorFormat::Hsl => "HSL",
            ColorFormat::Hsb => "HSB",
        }
    }

    pub fn from_id(s: &str) -> Option<ColorFormat> {
        ColorFormat::ALL.into_iter().find(|f| f.id().eq_ignore_ascii_case(s.trim()))
    }

    /// `c` in this format; the opacity follows as " · 50%" (CSS puts it in `rgba`).
    pub fn format(self, c: Color) -> String {
        let [r, g, b, _] = c.to_rgba8();
        let alpha = c.a < 0.999;
        let pct = |v: f32| format!("{}%", (v * 100.0).round());
        let body = match self {
            ColorFormat::Hex => format!("#{}", c.to_hex()),
            ColorFormat::Rgb => format!("{r}, {g}, {b}"),
            ColorFormat::Css if alpha => return format!("rgba({r}, {g}, {b}, {})", num(f64::from(c.a))),
            ColorFormat::Css => return format!("rgb({r}, {g}, {b})"),
            ColorFormat::Hsl => {
                let x = Hsl::from_color(c);
                format!("{}, {}, {}", x.h.round(), pct(x.s), pct(x.l))
            }
            ColorFormat::Hsb => {
                let x = Hsb::from_color(c);
                format!("{}, {}, {}", x.h.round(), pct(x.s), pct(x.b))
            }
        };
        if alpha { format!("{body} · {}", pct(c.a)) } else { body }
    }
}

/// A frame or section with a handoff status, for Dev Mode's list of what's ready.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DevItem {
    pub id: NodeId,
    pub name: String,
    pub page: NodeId,
    pub status: DevStatus,
    /// Ready for dev and changed since it was marked.
    pub changed: bool,
}

/// Whether Dev Mode runs command `id` from the menus and shortcuts: commands that don't edit the
/// document, and Dev Mode's own edits (annotations, statuses, dev resources, measurements, code
/// connections, comments). Choosing a tool other than moving around and commenting is refused too,
/// and so is Revert to saved, which isn't an undo step but throws the design's unsaved edits away.
pub fn dev_mode_allows(id: &str) -> bool {
    const OWN: [&str; 6] = ["annotation.", "dev.", "devMode.", "devResource.", "codeConnect.", "comment."];
    // Edits recorded as one grouped undo step, so their spec says no history of its own.
    const GROUPED: [&str; 1] = ["data.fill"];
    match id.strip_prefix("tool.") {
        Some(t) => matches!(t, "move" | "hand" | "comment"),
        None if id == "file.revert" || GROUPED.contains(&id) => false,
        None => OWN.iter().any(|p| id.starts_with(p)) || crate::cmd::find_command(id).is_none_or(|c| !c.history),
    }
}

/// Most items listed.
const MAX_DEV_ITEMS: usize = 500;

/// Whether `id` can carry a handoff status: a section, or a frame at the top of a page or section.
pub fn has_status(doc: &Document, id: NodeId) -> bool {
    let section = |n: &Node| matches!(n.kind, NodeKind::Section { .. });
    doc.node(id).is_some_and(|n| {
        section(n)
            || (n.kind.is_frame_like()
                && !matches!(n.kind, NodeKind::Instance { .. })
                && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || section(p)))
    })
}

/// The layers with handoff status `status` (`None`: any status) on `page` (`None`: every page),
/// in page and layer order (bounded). Only layers that can carry a status are listed.
pub fn dev_items(doc: &Document, status: Option<DevStatus>, page: Option<NodeId>) -> Vec<DevItem> {
    let mut out = Vec::new();
    let mut visited = 0usize;
    for pg in doc.pages.iter().copied().filter(|pg| page.is_none_or(|p| p == *pg)) {
        let mut stack = vec![pg];
        while let Some(id) = stack.pop() {
            visited += 1;
            if out.len() >= MAX_DEV_ITEMS || visited > 200_000 {
                return out;
            }
            let Some(n) = doc.node(id) else { continue };
            if !n.dev_status.is_none() && status.is_none_or(|s| s == n.dev_status) && has_status(doc, id) {
                let changed = n.dev_status == DevStatus::ReadyForDev && n.dev_changed;
                out.push(DevItem { id, name: n.name.clone(), page: pg, status: n.dev_status, changed });
            }
            stack.extend(n.children.iter().rev().copied());
        }
    }
    out
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "inspect.readyForDev",
            "Ready for dev",
            "{status?: readyForDev|changed|completed|any (default readyForDev; changed: ready for dev and changed since marked), page? (page id; default every page)} → {items: [{id, name, page, status, changed}]}  frames and sections by handoff status (set it with `node.set {devStatus}`)",
            false,
            |s, p| {
                let cmd = "inspect.readyForDev";
                let wanted = string(p, "status");
                let changed_only = wanted.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("changed"));
                let status = match wanted.as_deref() {
                    None => Some(DevStatus::ReadyForDev),
                    _ if changed_only => Some(DevStatus::ReadyForDev),
                    Some(v) if v.eq_ignore_ascii_case("any") || v.eq_ignore_ascii_case("all") => None,
                    Some(v) => Some(
                        DevStatus::ALL
                            .into_iter()
                            .find(|d| !d.is_none() && d.id().eq_ignore_ascii_case(v))
                            .ok_or_else(|| crate::bad(cmd, "status: readyForDev|changed|completed|any"))?,
                    ),
                };
                let doc = s.doc()?;
                let page = match get(p, "page") {
                    Some(v) => Some(id_of(v).filter(|pg| doc.pages.contains(pg)).ok_or_else(|| crate::bad(cmd, "page: a page id"))?),
                    None => None,
                };
                let mut items = dev_items(doc, status, page);
                items.retain(|i| i.changed || !changed_only);
                Ok(json!({"items": serde_json::to_value(items).unwrap_or(Value::Null)}))
            }
        ),
        spec!(
            "inspect.assets",
            "Inspect assets",
            "{ids? (default the selection)} → {assets: [{id, name, kind: export|icon|image, exports}]}  the layers in and under them to hand over as files: those with export settings, icons (small, vector shapes only; SVG) and images (PNG). Export them with `file.exportLayers {assets: true}`",
            false,
            |s, p| {
                let ids = targets(s, p);
                Ok(json!({"assets": serde_json::to_value(assets(s.doc()?, &ids)).unwrap_or(Value::Null)}))
            }
        ),
        spec!(
            "inspect.properties",
            "Inspect properties",
            "{id?, colorFormat?: hex|rgb|css|hsl|hsb (default hex)} → {sections: [{title, rows: [{label, value, color?, token?}]}]}  layout (positions from the frame around the layer), auto layout, typography, fills, strokes and effects, with style and variable names",
            false,
            |s, p| {
                let cmd = "inspect.properties";
                let id = match id(p, "id") {
                    Some(i) => i,
                    None => s.selection().first().copied().ok_or_else(|| crate::bad(cmd, "select a layer or pass `id`"))?,
                };
                let fmt = match string(p, "colorFormat") {
                    Some(f) => ColorFormat::from_id(&f).ok_or_else(|| crate::bad(cmd, "colorFormat: hex|rgb|css|hsl|hsb"))?,
                    None => ColorFormat::Hex,
                };
                let doc = s.doc()?;
                doc.node(id).ok_or_else(|| crate::bad(cmd, "no such layer"))?;
                Ok(json!({"sections": serde_json::to_value(properties_with(doc, id, fmt)).unwrap_or(Value::Null)}))
            }
        ),
    ]
}

/// A number as a designer reads it: at most two decimals, no trailing zeros.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{:.2}", (v * 100.0).round() / 100.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn prop(label: impl Into<String>, value: impl Into<String>) -> Prop {
    Prop { label: label.into(), value: value.into(), color: None, token: None }
}

impl Prop {
    fn token(mut self, t: Option<String>) -> Self {
        self.token = t;
        self
    }
    fn color(mut self, c: Color) -> Self {
        self.color = Some(c.to_css_hex());
        self
    }
}

/// The variable bound to property `field` of `n` ("fill" and "stroke" also find bindings stored
/// under their list forms, like "fills/0/color").
pub fn bound_variable<'d>(doc: &'d Document, n: &Node, field: &str) -> Option<&'d Variable> {
    let forms: &[&str] = match field {
        "fill" => &["fill", "fills", "fills/0", "fills/0/color"],
        "stroke" => &["stroke", "strokes/0/color"],
        _ => &[],
    };
    let found = if forms.is_empty() { n.bound.get(field) } else { forms.iter().find_map(|k| n.bound.get(*k)) };
    found.and_then(|v| doc.variables.get_by_id(v))
}

/// The variable bound to `field` on `n`, by name.
fn variable(doc: &Document, n: &Node, field: &str) -> Option<String> {
    bound_variable(doc, n, field).map(|v| v.name.clone())
}

fn style_name(doc: &Document, id: Option<&StyleId>) -> Option<String> {
    id.and_then(|s| doc.style(s)).map(|s| s.name.clone())
}

/// The properties of `id`, grouped, colours in hex; empty when it doesn't exist.
pub fn properties(doc: &Document, id: NodeId) -> Vec<Section> {
    properties_with(doc, id, ColorFormat::Hex)
}

/// [`properties`] with colours written in `fmt`.
pub fn properties_with(doc: &Document, id: NodeId, fmt: ColorFormat) -> Vec<Section> {
    let color_text = |c: Color| fmt.format(c);
    let Some(n) = doc.node(id) else { return Vec::new() };
    let mut out = vec![layout(doc, id, n)];
    if let Some(l) = &n.layout {
        out.push(auto_layout(doc, n, l));
    }
    if let Some(t) = n.kind.text() {
        out.push(typography(doc, n, t));
    }
    // Each paint's row names the variable bound to its colour; a style names the first row.
    let paint_rows = |fills: bool, style: Option<&StyleId>| -> Vec<Prop> {
        let paints = if fills { &n.fills } else { &n.strokes };
        let rows: Vec<Prop> = paints
            .iter()
            .enumerate()
            .filter(|(_, p)| p.visible)
            .map(|(i, p)| {
                let token = crate::cmd::lists::paint_binding_key(n, fills, i).and_then(|k| variable(doc, n, k));
                paint_row(p, fmt).token(token)
            })
            .collect();
        match style_name(doc, style) {
            Some(name) => tag_first(rows, Some(name)),
            None => rows,
        }
    };
    let fills = paint_rows(true, n.styles.fill.as_ref());
    if !fills.is_empty() {
        out.push(Section { title: "Fills", rows: fills });
    }
    let mut strokes = paint_rows(false, n.styles.stroke.as_ref());
    if !strokes.is_empty() {
        let align = match n.stroke.align {
            StrokeAlign::Inside => "inside",
            StrokeAlign::Center => "center",
            StrokeAlign::Outside => "outside",
        };
        strokes.push(prop("Weight", format!("{} {align}", num(n.stroke.weight))).token(variable(doc, n, "strokeWeight")));
        out.push(Section { title: "Strokes", rows: strokes });
    }
    // A grain size: one number, or across × down.
    let grain = |(sx, sy): (f64, f64)| if sx == sy { num(sx) } else { format!("{}×{}", num(sx), num(sy)) };
    let pct = |v: f64| (v * 100.0).round();
    let effects: Vec<Prop> = n
        .effects
        .iter()
        .filter(|e| e.visible)
        .map(|e| match &e.kind {
            EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => {
                let label = if matches!(e.kind, EffectKind::DropShadow(_)) { "Drop shadow" } else { "Inner shadow" };
                let v = format!("{} {} {} {} {}", num(s.offset.0), num(s.offset.1), num(s.radius), num(s.spread), color_text(s.color));
                prop(label, v).color(s.color)
            }
            EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b) => {
                let label = if matches!(e.kind, EffectKind::LayerBlur(_)) { "Layer blur" } else { "Background blur" };
                let at = |(x, y): (f64, f64)| format!("{}% {}%", pct(x), pct(y));
                let v = if b.is_progressive() {
                    format!("Progressive {} → {} from {} to {}", num(b.start_radius), num(b.radius), at(b.start), at(b.end))
                } else {
                    num(b.radius)
                };
                prop(label, v)
            }
            EffectKind::Noise(nz) => {
                let colors = match nz.mode {
                    NoiseMode::Mono => color_text(nz.color),
                    NoiseMode::Duo => format!("{} {}", color_text(nz.color), color_text(nz.secondary)),
                    NoiseMode::Multi => format!("{}%", pct(f64::from(nz.opacity))),
                };
                let row = prop("Noise", format!("{} {} {}% {colors}", nz.mode.label(), grain(nz.grain()), pct(nz.density)));
                if nz.mode == NoiseMode::Multi { row } else { row.color(nz.color) }
            }
            EffectKind::Texture(t) => {
                prop("Texture", format!("{} {}{}", grain(t.grain()), num(t.radius), if t.clip_to_shape { " clipped" } else { "" }))
            }
            EffectKind::Glass(g) => prop(
                "Glass",
                format!(
                    "{}° {}% refraction {}% depth {} dispersion {}% frost {} splay {}%",
                    num(g.light_angle),
                    pct(g.light_intensity),
                    pct(g.refraction),
                    num(g.depth),
                    pct(g.dispersion),
                    num(g.frost),
                    pct(g.splay)
                ),
            ),
        })
        .collect();
    if !effects.is_empty() {
        out.push(Section { title: "Effects", rows: tag_first(effects, style_name(doc, n.styles.effect.as_ref())) });
    }
    out
}

/// A style or variable applies to the whole list: show it on the first row.
/// One paint's row: its kind and colour (or stops, or opacity for images).
fn paint_row(p: &figcraft_color::Paint, fmt: ColorFormat) -> Prop {
    match &p.kind {
        PaintKind::Solid { color } => {
            let c = color.with_alpha(color.a * p.opacity);
            prop("Solid", fmt.format(c)).color(c)
        }
        PaintKind::Gradient(g) => {
            let kind = match g.kind {
                GradientKind::Linear => "Linear",
                GradientKind::Radial => "Radial",
                GradientKind::Angular => "Angular",
                GradientKind::Diamond => "Diamond",
            };
            let stops: Vec<String> = g.stops.iter().map(|s| format!("{} {}%", fmt.format(s.color), (s.position * 100.0).round())).collect();
            let r = prop(kind, stops.join(", "));
            match g.stops.first() {
                Some(s) => r.color(s.color),
                None => r,
            }
        }
        PaintKind::Image(_) => prop("Image", format!("{}%", (p.opacity * 100.0).round())),
        PaintKind::Pattern(_) => prop("Pattern", format!("{}%", (p.opacity * 100.0).round())),
    }
}

fn tag_first(mut rows: Vec<Prop>, token: Option<String>) -> Vec<Prop> {
    if let Some(first) = rows.first_mut() {
        first.token = token;
    }
    rows
}

fn layout(doc: &Document, id: NodeId, n: &Node) -> Section {
    let mut rows =
        vec![prop("Width", num(n.width)).token(variable(doc, n, "width")), prop("Height", num(n.height)).token(variable(doc, n, "height"))];
    // Position from the top-left of the frame around it (groups aren't frames; the canvas
    // origin at the top level).
    if let Some(b) = doc.world_bounds(id) {
        let origin = doc.containing_frame(id).and_then(|f| doc.world_bounds(f)).map_or((0.0, 0.0), |pb| (pb.x0, pb.y0));
        rows.push(prop("Top", num(b.y0 - origin.1)));
        rows.push(prop("Left", num(b.x0 - origin.0)));
    }
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 0.01 {
        rows.push(prop("Rotation", format!("{}°", num(rot))));
    }
    if n.opacity < 0.999 {
        rows.push(prop("Opacity", format!("{}%", (f64::from(n.opacity) * 100.0).round())).token(variable(doc, n, "opacity")));
    }
    if !matches!(n.blend, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
        rows.push(prop("Blend", n.blend.label()));
    }
    if n.kind.has_corner_radius() {
        let r = match n.corners.uniform() {
            Some(r) if r > 0.0 => Some(num(r)),
            Some(_) => None,
            None => Some(n.corners.radii.iter().map(|r| num(*r)).collect::<Vec<_>>().join(" ")),
        };
        if let Some(r) = r {
            rows.push(prop("Radius", r).token(variable(doc, n, "cornerRadius")));
        }
    }
    Section { title: "Layout", rows }
}

fn auto_layout(doc: &Document, n: &Node, l: &AutoLayout) -> Section {
    let dir = match (l.mode, l.wrap) {
        (LayoutMode::Horizontal, true) => "Horizontal, wrap",
        (LayoutMode::Horizontal, false) => "Horizontal",
        (LayoutMode::Vertical, true) => "Vertical, wrap",
        (LayoutMode::Vertical, false) => "Vertical",
        (LayoutMode::Grid, _) => "Grid",
    };
    let mut rows = vec![prop("Direction", dir)];
    if l.mode == LayoutMode::Grid {
        rows.push(prop("Grid", format!("{} × {}", l.grid_columns, l.grid_rows)));
        rows.push(prop("Gap", format!("{} {}", num(l.gap), num(l.cross_gap))));
    } else {
        let gap = match l.main_align {
            MainAlign::SpaceBetween => "Auto (space between)".to_string(),
            MainAlign::SpaceAround => "Auto (space around)".to_string(),
            MainAlign::SpaceEvenly => "Auto (space evenly)".to_string(),
            _ => num(l.gap),
        };
        rows.push(prop("Gap", gap).token(variable(doc, n, "gap")));
        if l.wrap {
            let label = if l.mode == LayoutMode::Vertical { "Column gap" } else { "Row gap" };
            let spread = l.content_align == ContentAlign::SpaceBetween;
            rows.push(if spread { prop(label, "Auto (space between)") } else { prop(label, num(l.cross_gap)).token(variable(doc, n, "crossGap")) });
        }
    }
    let [t, r, b, lft] = l.padding;
    let padding = if t == r && r == b && b == lft {
        num(t)
    } else if t == b && r == lft {
        format!("{} {}", num(t), num(r))
    } else {
        format!("{} {} {} {}", num(t), num(r), num(b), num(lft))
    };
    let pad_token = variable(doc, n, "padding").or_else(|| variable(doc, n, "paddingHorizontal")).or_else(|| variable(doc, n, "paddingVertical"));
    rows.push(prop("Padding", padding).token(pad_token));
    let main = match l.main_align {
        MainAlign::Min => "start",
        MainAlign::Center => "center",
        MainAlign::Max => "end",
        MainAlign::SpaceBetween => "space between",
        MainAlign::SpaceAround => "space around",
        MainAlign::SpaceEvenly => "space evenly",
    };
    let cross = match l.cross_align {
        CrossAlign::Min => "start",
        CrossAlign::Center => "center",
        CrossAlign::Max => "end",
        CrossAlign::Baseline => "baseline",
    };
    rows.push(prop("Align", format!("{main}, {cross}")));
    Section { title: "Auto layout", rows }
}

fn typography(doc: &Document, n: &Node, t: &TextData) -> Section {
    let s = &t.style;
    let token = style_name(doc, n.styles.text.as_ref());
    let mut rows = vec![
        prop("Font", s.font_family.clone()).token(token),
        prop("Style", s.font_style.clone()),
        prop("Size", num(s.font_size)).token(variable(doc, n, "fontSize")),
        prop("Line height", s.line_height.label()),
        prop("Letter spacing", s.letter_spacing.label()),
    ];
    if t.paragraph_spacing > 0.0 {
        rows.push(prop("Paragraph spacing", num(t.paragraph_spacing)));
    }
    if t.leading_trim == figcraft_doc::LeadingTrim::CapHeight {
        rows.push(prop("Vertical trim", "cap height to baseline"));
    }
    let align = match t.align_h {
        TextAlignH::Left => "left",
        TextAlignH::Center => "center",
        TextAlignH::Right => "right",
        TextAlignH::Justified => "justified",
    };
    rows.push(prop("Align", align));
    if s.decoration != TextDecoration::None {
        let l = s.look();
        let style = match l.style {
            figcraft_doc::DecorationStyle::Solid => "",
            figcraft_doc::DecorationStyle::Dotted => ", dotted",
            figcraft_doc::DecorationStyle::Wavy => ", wavy",
        };
        let kind = if s.decoration == TextDecoration::Underline { "underline" } else { "strikethrough" };
        rows.push(prop("Decoration", format!("{kind}{style}")));
    }
    if t.list_spacing > 0.0 {
        rows.push(prop("List spacing", num(t.list_spacing)));
    }
    if variable(doc, n, "characters").is_some() {
        rows.push(prop("Content", t.characters.chars().take(80).collect::<String>()).token(variable(doc, n, "characters")));
    }
    Section { title: "Typography", rows }
}
