//! The builder the starter templates share: a throwaway session whose document is built by
//! running commands, the way a user (or a script) would make it.

use std::collections::BTreeMap;
use std::sync::Arc;

use figcraft_color::Color;
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Map, Value, json};

use super::glyphs::Glyph;
use crate::{EngineError, Result, Session};

pub(super) const V: &str = "vertical";
pub(super) const H: &str = "horizontal";

/// The side of the grid icons are drawn on.
pub(super) const ICON: f64 = 24.0;

/// `over`'s keys on top of `base`'s (both objects; anything else gives `base`).
pub(super) fn merged(mut base: Value, over: Value) -> Value {
    if let (Some(b), Value::Object(o)) = (base.as_object_mut(), over) {
        b.extend(o);
    }
    base
}

pub(super) struct Kit {
    s: Session,
    /// Style ids by name ("Brand/Primary").
    styles: BTreeMap<String, String>,
}

impl Kit {
    pub fn new(name: &str) -> Self {
        Self { s: Session::with_document(Document::new(name)), styles: BTreeMap::new() }
    }

    /// The document, with every page on `background` and the first page shown.
    pub fn finish(mut self, background: Color) -> Result<Document> {
        let ds = self.s.docs.pop().ok_or(EngineError::NoDocument)?;
        let mut doc = Arc::try_unwrap(ds.doc).unwrap_or_else(|d| (*d).clone());
        for page in doc.pages.clone() {
            if let Some(NodeKind::Page { background: b, .. }) = doc.node_mut(page).map(|n| &mut n.kind) {
                *b = background;
            }
        }
        doc.current_page = doc.first_page();
        // Nothing here is an edit waiting to be normalised.
        doc.take_dirty();
        Ok(doc)
    }

    pub fn run(&mut self, cmd: &str, params: Value) -> Result<Value> {
        self.s.execute(cmd, &params)
    }

    /// The id a command answered with.
    fn made(&mut self, cmd: &str, params: Value) -> Result<NodeId> {
        let v = self.run(cmd, params)?;
        v.get("id").and_then(crate::params::id_of).ok_or_else(|| EngineError::Other(format!("`{cmd}` made no layer")))
    }

    /// The page being built on.
    pub fn page(&self) -> Result<NodeId> {
        self.s.page()
    }

    /// A new page, which becomes the one being built on.
    pub fn add_page(&mut self, name: &str) -> Result<NodeId> {
        self.run("page.add", json!({"name": name}))?;
        self.page()
    }

    /// Go back to `page` (flows are added to the page shown).
    pub fn go(&mut self, page: NodeId) -> Result<()> {
        self.run("page.select", json!({"id": page.0})).map(drop)
    }

    pub fn rename_page(&mut self, name: &str) -> Result<()> {
        self.run("page.rename", json!({"name": name})).map(drop)
    }

    /// `props` with style names in `fill`, `stroke` and `textStyle` turned into links to those
    /// styles (anything else, like "#RRGGBB", stays a plain value).
    fn resolved(&self, props: Value) -> Value {
        let Value::Object(map) = props else { return props };
        let mut out = Map::new();
        for (k, v) in map {
            let style = v.as_str().and_then(|name| self.styles.get(name));
            match (k.as_str(), style) {
                ("fill", Some(id)) => out.insert("fillStyle".into(), json!(id)),
                ("stroke", Some(id)) => out.insert("strokeStyle".into(), json!(id)),
                ("textStyle", Some(id)) => out.insert(k, json!(id)),
                _ => out.insert(k, v),
            };
        }
        Value::Object(out)
    }

    fn create(&mut self, kind: &str, parent: NodeId, extra: Value, props: Value) -> Result<NodeId> {
        let base = json!({"type": kind, "parent": parent.0, "select": false, "props": self.resolved(props)});
        self.made("node.create", merged(base, extra))
    }

    /// A layer of `kind` (frame, component, rectangle, ellipse…) at `[x, y, width, height]`.
    pub fn node(&mut self, kind: &str, parent: NodeId, name: &str, [x, y, w, h]: [f64; 4], props: Value) -> Result<NodeId> {
        self.create(kind, parent, json!({"name": name, "x": x, "y": y, "width": w, "height": h}), props)
    }

    pub fn frame(&mut self, parent: NodeId, name: &str, at: [f64; 4], props: Value) -> Result<NodeId> {
        self.node("frame", parent, name, at, props)
    }

    pub fn rect(&mut self, parent: NodeId, name: &str, at: [f64; 4], props: Value) -> Result<NodeId> {
        self.node("rectangle", parent, name, at, props)
    }

    pub fn ellipse(&mut self, parent: NodeId, name: &str, at: [f64; 4], props: Value) -> Result<NodeId> {
        self.node("ellipse", parent, name, at, props)
    }

    /// An auto layout frame of `kind` (frame or component) that hugs its content, without a fill,
    /// unless `props` say otherwise.
    pub fn stack_of(&mut self, kind: &str, parent: NodeId, name: &str, dir: &str, gap: f64, pad: Value, props: Value) -> Result<NodeId> {
        let mut base = json!({"clip": false, "layout": dir, "gap": gap, "padding": pad, "sizingH": "hug", "sizingV": "hug"});
        if props.get("fill").is_none() && props.get("fills").is_none() {
            base = merged(base, json!({"fills": []}));
        }
        self.create(kind, parent, json!({"name": name}), merged(base, props))
    }

    pub fn stack(&mut self, parent: NodeId, name: &str, dir: &str, gap: f64, pad: Value, props: Value) -> Result<NodeId> {
        self.stack_of("frame", parent, name, dir, gap, pad, props)
    }

    /// One line of text in text style `style`, coloured `color` (a paint style's name or a colour).
    pub fn text(&mut self, parent: NodeId, chars: &str, style: &str, color: &str) -> Result<NodeId> {
        self.text_with(parent, chars, style, color, json!({}))
    }

    pub fn text_with(&mut self, parent: NodeId, chars: &str, style: &str, color: &str, props: Value) -> Result<NodeId> {
        self.create("text", parent, json!({"text": chars}), merged(json!({"textStyle": style, "fill": color}), props))
    }

    /// Text set by hand rather than in a text style: `size` px in font style `weight`.
    pub fn sized_text(&mut self, parent: NodeId, chars: &str, [size, line]: [f64; 2], weight: &str, color: &str, props: Value) -> Result<NodeId> {
        let base = json!({"fontSize": size, "fontStyle": weight, "lineHeight": line, "fill": color});
        self.create("text", parent, json!({"text": chars}), merged(base, props))
    }

    /// Text that fills its auto layout parent's width and wraps.
    pub fn para(&mut self, parent: NodeId, chars: &str, style: &str, color: &str) -> Result<NodeId> {
        self.text_with(parent, chars, style, color, json!({"sizingH": "fill", "textAutoResize": "height"}))
    }

    /// A stroked outline from SVG path data, where the path puts it in `parent`.
    pub fn line(&mut self, parent: NodeId, name: &str, path: &str, color: &str, weight: f64) -> Result<NodeId> {
        let props = json!({"fills": [], "stroke": color, "strokeWeight": weight, "strokeCap": "round", "strokeJoin": "round"});
        self.create("vector", parent, json!({"name": name, "path": path}), props)
    }

    /// A filled shape from SVG path data.
    pub fn shape(&mut self, parent: NodeId, name: &str, path: &str, props: Value) -> Result<NodeId> {
        self.create("vector", parent, json!({"name": name, "path": path}), props)
    }

    /// An icon: a square frame of `size` holding `glyph` stroked in `color`.
    pub fn icon(&mut self, parent: NodeId, glyph: &Glyph, size: f64, color: &str) -> Result<NodeId> {
        self.icon_of("frame", parent, glyph, [0.0, 0.0], size, color).map(|(frame, _)| frame)
    }

    /// [`Kit::icon`] as a frame or component at a place of its own: the icon and its outline.
    pub fn icon_of(&mut self, kind: &str, parent: NodeId, glyph: &Glyph, [x, y]: [f64; 2], size: f64, color: &str) -> Result<(NodeId, NodeId)> {
        let frame = self.node(kind, parent, glyph.name, [x, y, ICON, ICON], json!({"fills": [], "clip": false}))?;
        let stroke = self.line(frame, "Glyph", glyph.path, color, 2.0)?;
        // The glyph follows the frame's size (and an instance's); its stroke keeps its weight.
        self.set(stroke, json!({"constraints": {"horizontal": "scale", "vertical": "scale"}}))?;
        if size != ICON {
            self.set(frame, json!({"width": size, "height": size}))?;
        }
        Ok((frame, stroke))
    }

    pub fn set(&mut self, id: NodeId, props: Value) -> Result<()> {
        let props = self.resolved(props);
        self.run("node.set", json!({"ids": [id.0], "props": props})).map(drop)
    }

    /// Paint styles: (name, colour).
    pub fn colors(&mut self, list: &[(&str, &str)]) -> Result<()> {
        for (name, hex) in list {
            let v = self.run("style.create", json!({"type": "paint", "name": name, "value": [hex]}))?;
            self.keep_style(name, &v);
        }
        Ok(())
    }

    /// Text styles: (name, size, font style, line height in pixels).
    pub fn type_styles(&mut self, list: &[(&str, f64, &str, f64)]) -> Result<()> {
        for (name, size, weight, line) in list {
            let value = json!({"fontSize": size, "fontStyle": weight, "lineHeight": line});
            let v = self.run("style.create", json!({"type": "text", "name": name, "value": value}))?;
            self.keep_style(name, &v);
        }
        Ok(())
    }

    fn keep_style(&mut self, name: &str, made: &Value) {
        if let Some(id) = made.get("id").and_then(Value::as_str) {
            self.styles.insert(name.to_string(), id.to_string());
        }
    }

    /// A variable collection with `modes`, and its variables: (name, type, one value per mode).
    pub fn variables(&mut self, collection: &str, modes: &[&str], vars: Vec<(&str, &str, Vec<Value>)>) -> Result<()> {
        self.run("variable.createCollection", json!({"name": collection, "modes": modes}))?;
        for (name, kind, values) in vars {
            let values: Map<String, Value> = modes.iter().map(ToString::to_string).zip(values).collect();
            self.run("variable.create", json!({"collection": collection, "name": name, "type": kind, "values": values}))?;
        }
        Ok(())
    }

    /// Bind fields of layer `id` to variables: (field, variable name). Gives the layer back.
    pub fn bound(&mut self, id: NodeId, binds: &[(&str, &str)]) -> Result<NodeId> {
        for (field, variable) in binds {
            self.run("variable.bind", json!({"ids": [id.0], "field": field, "variable": variable}))?;
        }
        Ok(id)
    }

    /// Show `frame` in `mode` of variable collection `collection`.
    pub fn mode(&mut self, frame: NodeId, collection: &str, mode: &str) -> Result<()> {
        self.run("variable.setMode", json!({"ids": [frame.0], "collection": collection, "mode": mode})).map(drop)
    }

    /// Combine components (named "Property=Value, …") into a component set called `name`.
    pub fn variants(&mut self, name: &str, ids: &[NodeId], props: Value) -> Result<NodeId> {
        let set = self.made("component.combineVariants", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))?;
        self.set(set, merged(json!({"name": name}), props))?;
        Ok(set)
    }

    /// A text property of `component` (or of a set) shown by `layers` (one per variant).
    pub fn text_prop(&mut self, component: NodeId, name: &str, default: &str, layers: &[NodeId]) -> Result<()> {
        for (i, layer) in layers.iter().enumerate() {
            if i == 0 {
                self.run("component.addProperty", json!({"id": component.0, "name": name, "type": "text", "default": default, "layer": layer.0}))?;
            } else {
                self.run("component.bindProperty", json!({"layer": layer.0, "property": name, "field": "characters"}))?;
            }
        }
        Ok(())
    }

    /// An instance of `component` in `parent` with component `properties` and layer `props`.
    pub fn instance(&mut self, component: NodeId, parent: NodeId, properties: Value, props: Value) -> Result<NodeId> {
        let id = self.made("instance.create", json!({"component": component.0, "parent": parent.0, "x": 0, "y": 0}))?;
        if properties.as_object().is_some_and(|o| !o.is_empty()) {
            self.run("instance.setProperties", json!({"ids": [id.0], "properties": properties}))?;
        }
        if props.as_object().is_some_and(|o| !o.is_empty()) {
            self.set(id, props)?;
        }
        Ok(id)
    }

    /// Clicking `from` goes to frame `to` in the prototype.
    pub fn link(&mut self, from: NodeId, to: NodeId) -> Result<()> {
        let params = json!({"id": from.0, "action": {"type": "navigate", "dest": to.0}, "transition": {"type": "dissolve"}, "durationMs": 200});
        self.run("proto.interaction.add", params).map(drop)
    }

    /// The prototype starts at frame `start`.
    pub fn flow(&mut self, start: NodeId, name: &str) -> Result<()> {
        self.run("proto.flow.add", json!({"start": start.0, "name": name})).map(drop)
    }
}

/// The colours every template starts from, as paint styles.
const COLORS: &[(&str, &str)] = &[
    ("Brand/Primary", "#1F6F5C"),
    ("Brand/Soft", "#DDEFE8"),
    ("Brand/Accent", "#F2A541"),
    ("Text/Primary", "#14201C"),
    ("Text/Secondary", "#5C6B66"),
    ("Text/On brand", "#FFFFFF"),
    ("Surface/Page", "#F5F7F6"),
    ("Surface/Card", "#FFFFFF"),
    ("Surface/Sunken", "#E9EEEC"),
    ("Border/Default", "#D8DFDC"),
];

/// The text styles every template starts from: (name, size, font style, line height).
pub(super) const TYPE: &[(&str, f64, &str, f64)] = &[
    ("Display", 48.0, "Bold", 56.0),
    ("Heading/L", 30.0, "Bold", 38.0),
    ("Heading/M", 22.0, "Semi Bold", 30.0),
    ("Heading/S", 17.0, "Semi Bold", 24.0),
    ("Body/L", 18.0, "Regular", 28.0),
    ("Body/M", 15.0, "Regular", 22.0),
    ("Body/S", 13.0, "Regular", 18.0),
    ("Label/M", 14.0, "Medium", 20.0),
    ("Label/S", 12.0, "Medium", 16.0),
];

pub(super) const PRIMARY: &str = "Brand/Primary";
pub(super) const SOFT: &str = "Brand/Soft";
pub(super) const ACCENT: &str = "Brand/Accent";
/// A darker green than the brand colour, for full-bleed backgrounds (not a style).
pub(super) const DEEP: &str = "#174F41";
pub(super) const INK: &str = "Text/Primary";
pub(super) const MUTED: &str = "Text/Secondary";
pub(super) const ON_BRAND: &str = "Text/On brand";
pub(super) const PAGE: &str = "Surface/Page";
pub(super) const CARD: &str = "Surface/Card";
pub(super) const SUNKEN: &str = "Surface/Sunken";
pub(super) const BORDER: &str = "Border/Default";

/// `strokeSides` for a hairline on one edge only: [top, right, bottom, left] weights.
pub(super) const TOP_EDGE: [f64; 4] = [1.0, 0.0, 0.0, 0.0];
pub(super) const BOTTOM_EDGE: [f64; 4] = [0.0, 0.0, 1.0, 0.0];
pub(super) const RIGHT_EDGE: [f64; 4] = [0.0, 1.0, 0.0, 0.0];

/// A soft shadow under cards.
pub(super) fn shadow() -> Value {
    json!([{"type": "dropShadow", "color": "#14201C14", "offset": [0, 4], "radius": 16}])
}

/// A top-to-bottom gradient fill.
pub(super) fn gradient(top: &str, bottom: &str) -> Value {
    let stop = |position: f64, hex: &str| json!({"position": position, "color": Color::parse_css(hex).unwrap_or(Color::BLACK)});
    json!([{"type": "gradient", "kind": "linear", "stops": [stop(0.0, top), stop(1.0, bottom)], "from": [0.5, 0.0], "to": [0.5, 1.0], "width": [-0.5, 0.0]}])
}

impl Kit {
    /// The shared colour and text styles.
    pub fn foundations(&mut self) -> Result<()> {
        self.colors(COLORS)?;
        self.type_styles(TYPE)
    }

    /// A picture drawn from shapes (a sun over two ridges at dawn), `w` × `h`; `props` then size
    /// or round it, and the shapes follow.
    pub fn scene(&mut self, parent: NodeId, name: &str, [w, h]: [f64; 2], props: Value) -> Result<NodeId> {
        let frame = self.frame(parent, name, [0.0, 0.0, w, h], json!({"fills": gradient("#BFE3D6", "#F8ECD4"), "clip": true}))?;
        let follow = json!({"horizontal": "scale", "vertical": "scale"});
        let sun = w.min(h) * 0.22;
        self.ellipse(frame, "Sun", [w * 0.68, h * 0.14, sun, sun], json!({"fill": ACCENT, "constraints": follow}))?;
        let ridge = |ys: [f64; 6]| {
            let [a, b, c, d, e, f] = ys.map(|y| y * h);
            format!("M0 {a} L{} {b} L{} {c} L{} {d} L{} {e} L{w} {f} V{h} H0 Z", w * 0.2, w * 0.38, w * 0.58, w * 0.8)
        };
        for (name, color, ys) in
            [("Far ridge", "#6FB09B", [0.62, 0.40, 0.56, 0.32, 0.58, 0.46]), ("Near ridge", PRIMARY, [0.84, 0.64, 0.78, 0.58, 0.82, 0.70])]
        {
            self.shape(frame, name, &ridge(ys), json!({"fill": color, "constraints": follow}))?;
        }
        self.set(frame, props)?;
        Ok(frame)
    }

    /// A pill or tile holding one line of text: (text style, text colour) on `fill`.
    pub fn chip(&mut self, parent: NodeId, label: &str, [style, color]: [&str; 2], fill: &str, pad: [f64; 2], radius: f64) -> Result<NodeId> {
        let props = json!({"fill": fill, "cornerRadius": radius, "mainAlign": "center", "crossAlign": "center"});
        let chip = self.stack(parent, label, H, 8.0, json!([pad[0], pad[1], pad[0], pad[1]]), props)?;
        self.text(chip, label, style, color)?;
        Ok(chip)
    }

    /// A soft square (or, with half its side as `radius`, a circle) that centres what goes in it:
    /// an icon, initials.
    pub fn tile(&mut self, parent: NodeId, side: f64, radius: f64) -> Result<NodeId> {
        let props = json!({"fill": SOFT, "cornerRadius": radius, "sizingH": "fixed", "sizingV": "fixed", "width": side, "height": side, "mainAlign": "center", "crossAlign": "center"});
        self.stack(parent, "Tile", H, 0.0, json!(0), props)
    }

    /// A bar chart in `card`: a plot drawn `w` × `h` with a baseline and one bar per (name, share of
    /// the height), the tallest in the accent colour, and the names under it in `label_style`. The
    /// shapes follow the plot, which fills the room `card` has.
    pub fn bar_chart(&mut self, card: NodeId, [w, h]: [f64; 2], bars: &[(&str, f64)], label_style: &str) -> Result<()> {
        let plot = self.frame(card, "Plot", [0.0, 0.0, w, h], json!({"fills": [], "clip": false}))?;
        let follow = json!({"horizontal": "scale", "vertical": "scale"});
        let slot = w / bars.len().max(1) as f64;
        let top = bars.iter().map(|(_, share)| *share).fold(0.0, f64::max);
        let base = self.line(plot, "Baseline", &format!("M0 {h} H{w}"), BORDER, 1.0)?;
        self.set(base, json!({"constraints": follow}))?;
        for (i, (name, share)) in bars.iter().enumerate() {
            let (bar, tall) = (slot * 0.56, share * h);
            let at = [i as f64 * slot + (slot - bar) / 2.0, h - tall, bar, tall];
            let fill = if *share >= top { ACCENT } else { PRIMARY };
            let radius = (bar / 6.0).round();
            self.rect(plot, name, at, json!({"fill": fill, "cornerRadius": [radius, radius, 0, 0], "constraints": follow}))?;
        }
        self.set(plot, json!({"sizingH": "fill", "sizingV": "fill"}))?;
        // Each name sits under its bar: the row's cells are as wide as the bars' slots.
        let axis = self.stack(card, "Axis", H, 0.0, json!(0), json!({"sizingH": "fill"}))?;
        for (name, _) in bars {
            self.text_with(axis, name, label_style, MUTED, json!({"sizingH": "fill", "textAlign": "center"}))?;
        }
        Ok(())
    }

    /// A tile with a figure and what it counts.
    pub fn stat(&mut self, parent: NodeId, value: &str, label: &str, value_style: &str) -> Result<NodeId> {
        let tile = self.stack(parent, label, V, 2.0, json!(16), json!({"fill": CARD, "cornerRadius": 16, "sizingH": "fill"}))?;
        self.text(tile, value, value_style, INK)?;
        self.text(tile, label, "Body/S", MUTED)?;
        Ok(tile)
    }
}
