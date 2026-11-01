//! Widgets (ABI v3; editable text fields and a property menu since v4): a plugin draws a layer on
//! the canvas by describing it, and FigCraft builds the layers. The description comes from the plugin, so it is untrusted: [`WidgetRender::parse`]
//! checks its size, node count, depth, numbers, strings, colours and image sizes before the engine
//! sees it. The engine decides where the layers go (only inside the widget's own layer).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::manifest::{check_id, clean};

/// Largest description a plugin may pass to `widget_render` (images included, as base64).
pub const MAX_WIDGET_BYTES: usize = 4 * 1024 * 1024;
/// Largest widget state (its JSON), and its longest key.
pub const MAX_STATE_BYTES: usize = 16 * 1024;
const MAX_STATE_KEY: usize = 128;
/// Most layers in one widget, and how deep they nest.
pub const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 16;
/// Longest text in a text layer.
const MAX_TEXT: usize = 1000;
const MAX_NAME: usize = 64;
/// Sizes, positions, gaps and paddings stay within this.
const MAX_SIZE: f64 = 4000.0;
/// Largest image (decoded bytes), and all images of one widget together.
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;
const MAX_IMAGES_BYTES: usize = 2 * 1024 * 1024;
/// Longest value of an editable text field (the same as any text layer's).
pub const MAX_FIELD_CHARS: usize = MAX_TEXT;
/// Most items in a widget's property menu, most colours in one colour item, longest item label.
const MAX_MENU: usize = 8;
const MAX_SWATCHES: usize = 8;
const MAX_MENU_LABEL: usize = 32;

/// What a widget is made of: FigCraft's own layer kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    #[default]
    Frame,
    Text,
    Rectangle,
    Ellipse,
    /// A rectangle filled with an image (PNG, JPEG…) the plugin carries in its module.
    Image,
    /// API 6: a straight line, its `width` long.
    Line,
    /// API 6: a picture the plugin describes as SVG text, as layers.
    Svg,
}

/// Auto layout direction of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetLayout {
    Horizontal,
    Vertical,
}

/// Alignment of an auto layout frame's children, along (`justify`) or across (`align`) it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetAlign {
    Min,
    Center,
    Max,
}

/// One layer of a widget's description.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetNode {
    #[serde(rename = "type", default)]
    pub kind: WidgetKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Clicking this layer calls the plugin back with this name (empty: not clickable).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// API 4, text layers: the text is an editable field with this name; an edit calls the plugin
    /// back with the field and its new value, which is also kept in the state under this name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub field: String,
    /// Position in the parent frame (ignored inside auto layout and for the widget itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Size; a frame with auto layout hugs its contents on an axis without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Hex colours (`#RRGGBB` or `#RRGGBBAA`); for text, `color` is the text's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(rename = "strokeWeight", default, skip_serializing_if = "Option::is_none")]
    pub stroke_weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<WidgetLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<WidgetAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<WidgetAlign>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Font size of a text layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    /// An image layer's bytes, base64.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// An SVG layer's text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub src: String,
    /// A line's or SVG's rotation around its top-left corner, in degrees; positive turns
    /// counter-clockwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// How the layer looks while the pointer is over it (hosts that don't know it draw the layer
    /// as it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<WidgetHover>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WidgetNode>,
}

/// A widget layer's look while the pointer is over it: what it gives replaces the layer's own
/// fill, stroke, opacity or (text) colour, as the reference tool's widgets' hover style does.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetHover {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// A text layer's colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl WidgetHover {
    /// Check and normalise it for a `kind` layer: hex colours, an opacity from 0 to 1, a colour for
    /// text and a fill for the rest, and something to show.
    pub fn check(&mut self, kind: WidgetKind) -> Result<(), String> {
        colour("hover fill", &mut self.fill)?;
        colour("hover stroke", &mut self.stroke)?;
        colour("hover color", &mut self.color)?;
        within("hover opacity", self.opacity, 0.0, 1.0)?;
        let text = kind == WidgetKind::Text;
        if self.color.is_some() && !text {
            return Err("`hover.color` is for text layers (others take `fill`)".into());
        }
        if self.fill.is_some() && text {
            return Err("a text layer's hover takes `color`, not `fill`".into());
        }
        if self.fill.is_some() && matches!(kind, WidgetKind::Image | WidgetKind::Line | WidgetKind::Svg) {
            return Err(format!("a {kind:?} layer's hover takes no `fill`"));
        }
        if self.stroke.is_some() && kind == WidgetKind::Svg {
            return Err("an SVG layer's hover takes no `stroke`".into());
        }
        if *self == WidgetHover::default() {
            return Err("`hover` needs a fill, stroke, opacity or color".into());
        }
        Ok(())
    }
}

/// What a widget callback hands back with `widget_render`: the widget's new state (`None`: keep
/// it) and its layers, the root a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetRender {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Map<String, Value>>,
    pub root: WidgetNode,
    /// API 4: the widget's property menu, shown while it's selected (empty: none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<MenuItem>,
}

/// What a property menu item is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuKind {
    /// A button.
    #[default]
    Action,
    /// On or off (`checked`).
    Toggle,
    /// One of a few colours (`options`; `value` is the current one).
    Color,
}

/// One item of a widget's property menu; choosing it calls the plugin back with `item`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MenuItem {
    pub item: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(rename = "type", default)]
    pub kind: MenuKind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// What one description may still use while it's checked.
struct Budget {
    nodes: usize,
    image_bytes: usize,
}

impl WidgetRender {
    /// Parse and validate a widget description.
    pub fn parse(text: &str) -> Result<WidgetRender, String> {
        if text.len() > MAX_WIDGET_BYTES {
            return Err(format!("the widget is larger than {} MB", MAX_WIDGET_BYTES / (1024 * 1024)));
        }
        let mut w: WidgetRender = serde_json::from_str(text).map_err(|e| format!("widget: {e}"))?;
        if let Some(state) = &w.state {
            check_state(state)?;
        }
        if w.root.kind != WidgetKind::Frame {
            return Err("widget: the root must be a frame".into());
        }
        w.root.check(0, &mut Budget { nodes: 0, image_bytes: 0 }).map_err(|e| format!("widget: {e}"))?;
        check_menu(&mut w.menu).map_err(|e| format!("widget: {e}"))?;
        Ok(w)
    }

    /// Whether it uses what plugin API 4 added (fields or a menu).
    pub fn uses_api_4(&self) -> bool {
        !self.menu.is_empty() || self.root.any(&|n| !n.field.is_empty())
    }

    /// Whether it uses what plugin API 6 added (lines or SVG).
    pub fn uses_api_6(&self) -> bool {
        self.root.any(&|n| matches!(n.kind, WidgetKind::Line | WidgetKind::Svg))
    }
}

/// Check and normalise a property menu: at most [`MAX_MENU`] items with distinct names and short
/// labels (an item's name when it has none); a colour item has 1–[`MAX_SWATCHES`] colours, its
/// value one of them.
pub fn check_menu(menu: &mut [MenuItem]) -> Result<(), String> {
    if menu.len() > MAX_MENU {
        return Err(format!("a menu has more than {MAX_MENU} items"));
    }
    for i in 0..menu.len() {
        let Some(m) = menu.get_mut(i) else { continue };
        check_id("menu item", &m.item, MAX_NAME)?;
        m.label = clean("a menu label", &m.label, MAX_MENU_LABEL)?;
        if m.label.is_empty() {
            m.label.clone_from(&m.item);
        }
        let colours = m.kind == MenuKind::Color;
        if m.checked && m.kind != MenuKind::Toggle {
            return Err("`checked` is for toggles".into());
        }
        if colours == m.options.is_empty() || m.options.len() > MAX_SWATCHES {
            return Err(format!("a colour item needs 1–{MAX_SWATCHES} `options` (and only it has them)"));
        }
        for o in &mut m.options {
            let mut c = Some(std::mem::take(o));
            colour("options", &mut c)?;
            *o = c.unwrap_or_default();
        }
        colour("value", &mut m.value)?;
        if m.value.as_ref().is_some_and(|v| !m.options.contains(v)) {
            return Err("a colour item's `value` must be one of its `options`".into());
        }
        let item = m.item.clone();
        if menu.get(..i).is_some_and(|before| before.iter().any(|b| b.item == item)) {
            return Err(format!("two menu items are called `{item}`"));
        }
    }
    Ok(())
}

/// An editable field's new value, cleaned: line endings become `\n`, tabs spaces, and other control
/// characters are dropped; at most [`MAX_FIELD_CHARS`].
pub fn clean_field(value: &str) -> Result<String, String> {
    let v: String = value
        .replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\r' | '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    if v.chars().count() > MAX_FIELD_CHARS {
        return Err(format!("a field's value is longer than {MAX_FIELD_CHARS} characters"));
    }
    Ok(v)
}

/// A widget's state: a JSON object of at most [`MAX_STATE_BYTES`] with short keys.
pub fn check_state(state: &Map<String, Value>) -> Result<(), String> {
    if state.keys().any(|k| k.is_empty() || k.chars().count() > MAX_STATE_KEY || k.chars().any(char::is_control)) {
        return Err(format!("widget: a state key must be 1–{MAX_STATE_KEY} characters without control characters"));
    }
    if serde_json::to_string(state).map_or(usize::MAX, |s| s.len()) > MAX_STATE_BYTES {
        return Err(format!("widget: the state is larger than {} KB", MAX_STATE_BYTES / 1024));
    }
    Ok(())
}

/// `v` when it's finite and within `lo..=hi`.
fn within(what: &str, v: Option<f64>, lo: f64, hi: f64) -> Result<(), String> {
    match v {
        Some(n) if !n.is_finite() || n < lo || n > hi => Err(format!("`{what}` must be a number from {lo} to {hi}")),
        _ => Ok(()),
    }
}

/// A colour as `#RRGGBB(AA)`.
fn colour(what: &str, c: &mut Option<String>) -> Result<(), String> {
    if let Some(s) = c {
        let parsed = Some(s.as_str()).filter(|s| s.len() <= 9).and_then(figcraft_color::Color::from_hex);
        *s = parsed.ok_or_else(|| format!("`{what}` must be a hex colour like #FF5A5F"))?.to_css_hex();
    }
    Ok(())
}

impl WidgetNode {
    /// Whether this layer or one inside it passes `test`.
    fn any(&self, test: &dyn Fn(&WidgetNode) -> bool) -> bool {
        test(self) || self.children.iter().any(|c| c.any(test))
    }

    /// Refuse what doesn't apply to this kind of layer.
    fn check_applies(&self) -> Result<(), String> {
        let kind = self.kind;
        let only = |what: &str, used: bool, allowed: bool| {
            if used && !allowed { Err(format!("`{what}` doesn't apply to a {kind:?} layer")) } else { Ok(()) }
        };
        only("children", !self.children.is_empty(), kind == WidgetKind::Frame)?;
        only(
            "layout",
            self.layout.is_some() || self.gap.is_some() || self.padding.is_some() || self.align.is_some() || self.justify.is_some(),
            kind == WidgetKind::Frame,
        )?;
        only("text", !self.text.is_empty() || self.size.is_some() || self.color.is_some() || self.bold, kind == WidgetKind::Text)?;
        only("data", !self.data.is_empty(), kind == WidgetKind::Image)?;
        only("field", !self.field.is_empty(), kind == WidgetKind::Text)?;
        only("src", !self.src.is_empty(), kind == WidgetKind::Svg)?;
        only("rotation", self.rotation.is_some(), matches!(kind, WidgetKind::Line | WidgetKind::Svg))?;
        only("height", self.height.is_some(), kind != WidgetKind::Line)?;
        only("fill", self.fill.is_some(), !matches!(kind, WidgetKind::Line | WidgetKind::Svg))?;
        only("stroke", self.stroke.is_some() || self.stroke_weight.is_some(), kind != WidgetKind::Svg)
    }

    /// Check an image's or SVG's size, on its own and with the others of the widget.
    fn check_bytes(&self, budget: &mut Budget) -> Result<(), String> {
        let bytes = match self.kind {
            WidgetKind::Image => {
                // Base64: four characters for three bytes (checked again when decoded).
                let bytes = self.data.trim().len() / 4 * 3;
                if self.data.trim().is_empty() || bytes > MAX_IMAGE_BYTES + 3 {
                    return Err(format!("an image needs `data` (base64) of at most {} KB", MAX_IMAGE_BYTES / 1024));
                }
                bytes
            }
            WidgetKind::Svg => {
                if self.src.trim().is_empty() || self.src.len() > MAX_IMAGE_BYTES {
                    return Err(format!("an SVG needs `src` of at most {} KB", MAX_IMAGE_BYTES / 1024));
                }
                self.src.len()
            }
            _ => return Ok(()),
        };
        budget.image_bytes += bytes;
        if budget.image_bytes > MAX_IMAGES_BYTES + 3 {
            return Err(format!("the images are larger than {} MB together", MAX_IMAGES_BYTES / (1024 * 1024)));
        }
        Ok(())
    }

    fn check(&mut self, depth: usize, budget: &mut Budget) -> Result<(), String> {
        budget.nodes += 1;
        if budget.nodes > MAX_NODES {
            return Err(format!("more than {MAX_NODES} layers"));
        }
        if depth >= MAX_DEPTH {
            return Err(format!("layers nest more than {MAX_DEPTH} deep"));
        }
        self.name = clean("name", &self.name, MAX_NAME)?;
        if !self.target.is_empty() {
            check_id("target", &self.target, MAX_NAME)?;
        }
        if !self.field.is_empty() {
            check_id("field", &self.field, MAX_NAME)?;
        }
        for (what, v) in [("x", self.x), ("y", self.y)] {
            within(what, v, -MAX_SIZE, MAX_SIZE)?;
        }
        for (what, v) in [("width", self.width), ("height", self.height), ("gap", self.gap), ("padding", self.padding), ("radius", self.radius)] {
            within(what, v, 0.0, MAX_SIZE)?;
        }
        within("strokeWeight", self.stroke_weight, 0.0, 64.0)?;
        within("opacity", self.opacity, 0.0, 1.0)?;
        within("rotation", self.rotation, -180.0, 180.0)?;
        within("size", self.size, 1.0, 256.0)?;
        colour("fill", &mut self.fill)?;
        colour("stroke", &mut self.stroke)?;
        colour("color", &mut self.color)?;
        self.check_applies()?;
        if let Some(h) = &mut self.hover {
            h.check(self.kind)?;
        }
        if self.text.chars().count() > MAX_TEXT {
            return Err(format!("a text is longer than {MAX_TEXT} characters"));
        }
        if self.text.chars().any(|c| c.is_control() && c != '\n') {
            return Err("a text contains control characters".into());
        }
        self.check_bytes(budget)?;
        self.children.iter_mut().try_for_each(|c| c.check(depth + 1, budget))
    }
}
