//! A synthetic design system and a file using it, for scale tests and benchmarks
//! (`figcraft-cli perf`). Everything is generated: no third-party content.
//!
//! [`design_system`] builds icon components and component sets (buttons, inputs, cards, navs,
//! badges) whose variants span size × state × type × theme, with boolean, text, instance swap and
//! slot properties, exposed nested instances nested four deep (icon in button in card in nav),
//! paint and text styles, and a light/dark variable collection bound across it.
//! [`place_instances`] fills pages with instances of it carrying overrides at every nesting level
//! (property values, swaps, fills, text, names).

use std::collections::BTreeMap;

use figcraft_color::{Color, Paint};
use figcraft_doc::{
    AutoLayout, Collection, CollectionId, CrossAlign, DocError, DocResult, Document, LayoutMode, Mode, ModeId, Node, NodeId, NodeKind, PropDef,
    Sizing, Style, StyleKind, TextAutoResize, TextData, TextStyle, Variable, VariableType, VariableValue,
};
use serde_json::Value;

use crate::components::{create_instance, sync_instance};

/// How large a design system to build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DsSpec {
    /// Icon components.
    pub icons: usize,
    /// Values per variant property (each capped at what the property has): sizes (≤ 3), states
    /// (≤ 4), types (≤ 3), themes (≤ 2).
    pub sizes: usize,
    pub states: usize,
    pub types: usize,
    pub themes: usize,
}

impl DsSpec {
    /// About 300 components (160 icons, 144 variants).
    pub const FULL: DsSpec = DsSpec { icons: 160, sizes: 3, states: 4, types: 3, themes: 2 };
    /// A small one for debug-mode tests (still every property kind and nesting level).
    pub const SMALL: DsSpec = DsSpec { icons: 6, sizes: 2, states: 2, types: 2, themes: 2 };
}

/// What [`design_system`] built (ids in the document it was built in, or mapped with
/// [`DesignSystem::remap`] after a library import).
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSystem {
    pub icons: Vec<NodeId>,
    pub button: NodeId,
    pub input: NodeId,
    pub card: NodeId,
    pub nav: NodeId,
    pub badge: NodeId,
    /// Component nodes (icons and variants).
    pub components: usize,
    /// The theme collection and its light and dark modes.
    pub theme: CollectionId,
    pub light: ModeId,
    pub dark: ModeId,
}

impl DesignSystem {
    /// Every component set.
    pub fn sets(&self) -> [NodeId; 5] {
        [self.button, self.input, self.card, self.nav, self.badge]
    }

    /// Every asset root (icons and sets).
    pub fn assets(&self) -> Vec<NodeId> {
        self.icons.iter().copied().chain(self.sets()).collect()
    }

    /// The same design system after its layers were copied with `map` (library id → local id).
    pub fn remap(&self, map: &BTreeMap<NodeId, NodeId>) -> DesignSystem {
        let m = |id: NodeId| map.get(&id).copied().unwrap_or(id);
        DesignSystem {
            icons: self.icons.iter().map(|i| m(*i)).collect(),
            button: m(self.button),
            input: m(self.input),
            card: m(self.card),
            nav: m(self.nav),
            badge: m(self.badge),
            ..self.clone()
        }
    }
}

const SIZES: [&str; 3] = ["Small", "Medium", "Large"];
const STATES: [&str; 4] = ["Default", "Hover", "Pressed", "Disabled"];
const BUTTON_TYPES: [&str; 3] = ["Primary", "Secondary", "Ghost"];
const INPUT_STATES: [&str; 4] = ["Default", "Focus", "Error", "Disabled"];
const INPUT_TYPES: [&str; 2] = ["Text", "Search"];
const CARD_TYPES: [&str; 2] = ["Elevated", "Outlined"];
const NAV_TYPES: [&str; 2] = ["Top", "Side"];
const BADGE_TYPES: [&str; 4] = ["Info", "Success", "Warning", "Danger"];
const THEMES: [&str; 2] = ["Light", "Dark"];
const GLYPHS: [&str; 16] =
    ["Arrow", "Check", "Close", "Plus", "Minus", "Search", "Home", "User", "Bell", "Star", "Heart", "Gear", "Mail", "Lock", "Pin", "Clock"];

/// Theme colour variables: (name, light, dark).
const COLORS: [(&str, u32, u32); 9] = [
    ("Surface", 0xFFFFFF, 0x1E1E1E),
    ("On surface", 0x1E1E1E, 0xF5F5F5),
    ("Primary", 0x0D99FF, 0x4DB5FF),
    ("On primary", 0xFFFFFF, 0x0B1A26),
    ("Border", 0xD9D9D9, 0x444444),
    ("Danger", 0xF24822, 0xFF7A5C),
    ("Success", 0x14AE5C, 0x3DD68C),
    ("Warning", 0xFFA629, 0xFFC470),
    ("Icon", 0x333333, 0xE6E6E6),
];

fn rgb(hex: u32) -> Color {
    let [_, r, g, b] = hex.to_be_bytes();
    Color::from_rgb8_const(r, g, b)
}

fn var_id(name: &str) -> String {
    format!("ds:{}", name.to_lowercase().replace(' ', "-"))
}

fn component() -> NodeKind {
    NodeKind::Component { description: String::new(), props: BTreeMap::new(), prop_order: Vec::new() }
}

fn text(chars: &str) -> NodeKind {
    NodeKind::Text(TextData::new(chars))
}

/// Pick a (capped) prefix of a value list.
fn take<'a>(list: &[&'a str], n: usize) -> Vec<&'a str> {
    list.iter().take(n.clamp(1, list.len())).copied().collect()
}

fn set_node(doc: &mut Document, id: NodeId, f: impl FnOnce(&mut Node)) {
    if let Some(n) = doc.node_mut(id) {
        f(n);
    }
}

fn bind(doc: &mut Document, id: NodeId, field: &str, var: &str) {
    set_node(doc, id, |n| {
        n.bound.insert(field.to_string(), var_id(var));
    });
}

fn auto_layout(doc: &mut Document, id: NodeId, mode: LayoutMode, gap: f64, pad: f64) {
    set_node(doc, id, |n| {
        n.layout = Some(AutoLayout { mode, gap, padding: [pad; 4], cross_align: CrossAlign::Center, ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
    });
}

/// A text layer with a text style applied.
fn styled_text(doc: &mut Document, parent: NodeId, chars: &str, name: &str, style: &str) -> DocResult<NodeId> {
    let id = doc.create(parent, text(chars), name, 0.0, 0.0, 60.0, 16.0)?;
    let def = doc.styles.iter().find(|s| s.id == style).and_then(|s| match &s.kind {
        StyleKind::Text { style } => Some(style.clone()),
        _ => None,
    });
    set_node(doc, id, |n| {
        if let (NodeKind::Text(t), Some(def)) = (&mut n.kind, def) {
            t.style = def;
            t.auto_resize = TextAutoResize::WidthAndHeight;
        }
        n.styles.text = Some(style.to_string());
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
    });
    Ok(id)
}

/// A nested instance inside a main, renamed (a name override, as renaming it in the app records).
fn nested(doc: &mut Document, comp: NodeId, parent: NodeId, name: &str, exposed: bool) -> DocResult<NodeId> {
    let id = create_instance(doc, comp, parent, 0.0, 0.0)?;
    set_node(doc, id, |n| {
        n.name = name.to_string();
        n.overrides.insert("name".into());
        n.exposed = exposed;
    });
    Ok(id)
}

fn set_props(doc: &mut Document, comp: NodeId, props: &BTreeMap<String, PropDef>) {
    set_node(doc, comp, |n| {
        if let NodeKind::Component { props: p, .. } = &mut n.kind {
            *p = props.clone();
        }
    });
}

fn refs(doc: &mut Document, id: NodeId, field: &str, prop: &str) {
    set_node(doc, id, |n| {
        n.prop_refs.insert(field.to_string(), prop.to_string());
    });
}

/// Variables (a light/dark theme collection with colours and a radius) and styles.
fn foundations(doc: &mut Document) -> (CollectionId, ModeId, ModeId) {
    let (coll, light, dark) = ("ds:theme".to_string(), "ds:light".to_string(), "ds:dark".to_string());
    doc.variables.collections.push(Collection {
        id: coll.clone(),
        name: "Theme".into(),
        modes: vec![Mode { id: light.clone(), name: "Light".into() }, Mode { id: dark.clone(), name: "Dark".into() }],
        extends: None,
    });
    let var = |name: &str, kind: VariableType, values: [VariableValue; 2]| Variable {
        id: var_id(name),
        name: name.into(),
        collection: coll.clone(),
        kind,
        values: [light.clone(), dark.clone()].into_iter().zip(values).collect(),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    };
    for (name, l, d) in COLORS {
        doc.variables.variables.push(var(name, VariableType::Color, [VariableValue::Color(rgb(l)), VariableValue::Color(rgb(d))]));
    }
    doc.variables.variables.push(var("Radius", VariableType::Number, [VariableValue::Number(6.0), VariableValue::Number(8.0)]));
    for (name, hex) in [("Brand/Primary", 0x0D99FFu32), ("Brand/Accent", 0x9747FF), ("Text/Primary", 0x1E1E1E), ("Text/Secondary", 0x757575)] {
        doc.styles.push(Style {
            id: format!("ds:paint:{name}"),
            name: name.into(),
            description: String::new(),
            kind: StyleKind::Paint { paints: vec![Paint::solid(rgb(hex))] },
            bound: Default::default(),
        });
    }
    for (name, size, weight) in [
        ("Heading/L", 24.0, "Bold"),
        ("Heading/M", 18.0, "Bold"),
        ("Body/M", 14.0, "Regular"),
        ("Label/S", 11.0, "Medium"),
        ("Label/M", 13.0, "Medium"),
        ("Label/L", 15.0, "Medium"),
    ] {
        let style = TextStyle { font_size: size, font_style: weight.into(), ..TextStyle::default() };
        doc.styles.push(Style {
            id: format!("ds:text:{name}"),
            name: name.into(),
            description: String::new(),
            kind: StyleKind::Text { style },
            bound: Default::default(),
        });
    }
    (coll, light, dark)
}

/// A component set named `name` at (`x`, `y`) whose variants `make` builds, one per combination
/// of `axes` (property, values), laid out in rows of `per_row`.
fn component_set(
    doc: &mut Document,
    page: NodeId,
    name: &str,
    (x, y): (f64, f64),
    axes: &[(&str, Vec<&str>)],
    per_row: usize,
    mut make: impl FnMut(&mut Document, NodeId, &BTreeMap<&str, &str>) -> DocResult<()>,
) -> DocResult<(NodeId, usize)> {
    let set = doc.alloc_id();
    let mut n = Node::new(set, NodeKind::ComponentSet { description: format!("Synthetic {name} component set") }, name, 100.0, 100.0);
    n.transform = figcraft_geom::Affine::translate((x, y));
    n.fills.clear();
    doc.insert(n, page, None)?;
    let mut combos: Vec<BTreeMap<&str, &str>> = vec![BTreeMap::new()];
    for (prop, values) in axes {
        combos = combos
            .into_iter()
            .flat_map(|c| values.iter().map(move |v| (c.clone(), *v)))
            .map(|(mut c, v)| {
                c.insert(*prop, v);
                c
            })
            .collect();
    }
    let order: Vec<&str> = axes.iter().map(|(p, _)| *p).collect();
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for (k, combo) in combos.iter().enumerate() {
        let vname = order.iter().filter_map(|p| combo.get(p).map(|v| format!("{p}={v}"))).collect::<Vec<_>>().join(", ");
        let (cx, cy) = (20.0 + (k % per_row.max(1)) as f64 * 220.0, 20.0 + (k / per_row.max(1)) as f64 * 160.0);
        let c = doc.create(set, component(), &vname, cx, cy, 100.0, 40.0)?;
        if combo.get("Theme") == Some(&"Dark") {
            set_node(doc, c, |n| {
                n.var_modes.insert("ds:theme".into(), "ds:dark".into());
            });
        }
        make(doc, c, combo)?;
        w = w.max(cx + 220.0);
        h = h.max(cy + 160.0);
    }
    set_node(doc, set, |n| {
        n.width = w;
        n.height = h;
    });
    Ok((set, combos.len()))
}

/// Build the design system on `page` of `doc` (call [`crate::normalize`] afterwards, or use
/// [`design_system_document`]).
pub fn design_system(doc: &mut Document, page: NodeId, spec: &DsSpec) -> DocResult<DesignSystem> {
    let (theme, light, dark) = foundations(doc);
    let mut count = 0;
    // Icons: a 24×24 frame with one glyph whose colour follows the theme.
    let mut icons = Vec::new();
    let cols = 16;
    for k in 0..spec.icons.max(2) {
        let name = format!("Icon/{} {}", GLYPHS.get(k % GLYPHS.len()).copied().unwrap_or("Glyph"), k / GLYPHS.len() + 1);
        let (x, y) = ((k % cols) as f64 * 40.0, (k / cols) as f64 * 40.0);
        let c = doc.create(page, component(), &name, x, y, 24.0, 24.0)?;
        set_node(doc, c, |n| n.fills.clear());
        let kind = if k % 2 == 0 { NodeKind::Rectangle } else { NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 } };
        let g = doc.create(c, kind, "Glyph", 4.0, 4.0, 16.0, 16.0)?;
        bind(doc, g, "fill", "Icon");
        icons.push(c);
        count += 1;
    }
    let icon = |k: usize| icons.get(k % icons.len()).copied().unwrap_or(NodeId(0));
    let preferred: Vec<NodeId> = icons.iter().take(4).copied().collect();
    let top = ((spec.icons.max(2)).div_ceil(cols) as f64) * 40.0 + 80.0;
    let sizes = take(&SIZES, spec.sizes);
    let themes = take(&THEMES, spec.themes);
    // Buttons: icon + label, the icon swappable (preferred: the first icons) and optional.
    let (icon0, icon1) = (icon(0), icon(1));
    let mut button_props = BTreeMap::new();
    button_props.insert("Label".to_string(), PropDef::Text { default: "Button".into() });
    button_props.insert("Show icon".to_string(), PropDef::Boolean { default: true });
    button_props.insert("Icon".to_string(), PropDef::InstanceSwap { default: icon0, preferred: preferred.clone() });
    let axes = [("Size", sizes.clone()), ("State", take(&STATES, spec.states)), ("Type", take(&BUTTON_TYPES, spec.types)), ("Theme", themes.clone())];
    let (button, n) = component_set(doc, page, "Button", (0.0, top), &axes, 12, |doc, c, v| {
        let size = v.get("Size").copied().unwrap_or("Medium");
        let pad = match size {
            "Small" => 6.0,
            "Large" => 14.0,
            _ => 10.0,
        };
        auto_layout(doc, c, LayoutMode::Horizontal, 6.0, pad);
        match v.get("Type").copied() {
            Some("Primary") | None => bind(doc, c, "fill", "Primary"),
            Some("Secondary") => {
                bind(doc, c, "fill", "Surface");
                set_node(doc, c, |n| n.strokes = vec![Paint::solid(rgb(0xD9D9D9))]);
                bind(doc, c, "stroke", "Border");
            }
            Some(_) => set_node(doc, c, |n| n.fills.clear()),
        }
        bind(doc, c, "cornerRadius", "Radius");
        if v.get("State") == Some(&"Disabled") {
            set_node(doc, c, |n| n.opacity = 0.5);
        }
        let i = nested(doc, icon0, c, "Icon", true)?;
        refs(doc, i, "visible", "Show icon");
        refs(doc, i, "mainComponent", "Icon");
        let style = match size {
            "Small" => "ds:text:Label/S",
            "Large" => "ds:text:Label/L",
            _ => "ds:text:Label/M",
        };
        let label = styled_text(doc, c, "Button", "Label", style)?;
        refs(doc, label, "characters", "Label");
        bind(doc, label, "fill", if v.get("Type") == Some(&"Primary") { "On primary" } else { "On surface" });
        set_props(doc, c, &button_props);
        Ok(())
    })?;
    count += n;
    let button_default = doc.node(button).and_then(|s| s.children.first().copied()).unwrap_or(NodeId(0));
    // Inputs: label above a field holding a swappable leading icon and the value.
    let mut input_props = BTreeMap::new();
    input_props.insert("Label".to_string(), PropDef::Text { default: "Label".into() });
    input_props.insert("Placeholder".to_string(), PropDef::Text { default: "Placeholder".into() });
    input_props.insert("Show leading icon".to_string(), PropDef::Boolean { default: true });
    input_props.insert("Leading icon".to_string(), PropDef::InstanceSwap { default: icon1, preferred: preferred.clone() });
    let axes =
        [("Size", sizes.clone()), ("State", take(&INPUT_STATES, spec.states)), ("Type", take(&INPUT_TYPES, spec.types)), ("Theme", themes.clone())];
    let y = top + (n.div_ceil(12) as f64) * 160.0 + 120.0;
    let (input, n) = component_set(doc, page, "Input", (0.0, y), &axes, 12, |doc, c, v| {
        auto_layout(doc, c, LayoutMode::Vertical, 4.0, 0.0);
        set_node(doc, c, |n| n.fills.clear());
        let label = styled_text(doc, c, "Label", "Label", "ds:text:Label/S")?;
        refs(doc, label, "characters", "Label");
        set_node(doc, label, |n| n.styles.fill = Some("ds:paint:Text/Secondary".into()));
        let field = doc.create(c, NodeKind::Frame, "Field", 0.0, 0.0, 160.0, 32.0)?;
        auto_layout(doc, field, LayoutMode::Horizontal, 6.0, 8.0);
        bind(doc, field, "fill", "Surface");
        set_node(doc, field, |n| n.strokes = vec![Paint::solid(rgb(0xD9D9D9))]);
        let stroke = match v.get("State").copied() {
            Some("Focus") => "Primary",
            Some("Error") => "Danger",
            _ => "Border",
        };
        bind(doc, field, "stroke", stroke);
        bind(doc, field, "cornerRadius", "Radius");
        let i = nested(doc, icon1, field, "Leading icon", false)?;
        refs(doc, i, "visible", "Show leading icon");
        refs(doc, i, "mainComponent", "Leading icon");
        let value = styled_text(doc, field, "Placeholder", "Value", "ds:text:Body/M")?;
        refs(doc, value, "characters", "Placeholder");
        bind(doc, value, "fill", "On surface");
        if v.get("State") == Some(&"Disabled") {
            set_node(doc, c, |n| n.opacity = 0.5);
        }
        set_props(doc, c, &input_props);
        Ok(())
    })?;
    count += n;
    // Badges: a coloured pill with a text property.
    let mut badge_props = BTreeMap::new();
    badge_props.insert("Text".to_string(), PropDef::Text { default: "New".into() });
    let axes = [("Type", take(&BADGE_TYPES, spec.types + 1)), ("Theme", themes.clone())];
    let y = y + (n.div_ceil(12) as f64) * 160.0 + 120.0;
    let (badge, n) = component_set(doc, page, "Badge", (0.0, y), &axes, 12, |doc, c, v| {
        auto_layout(doc, c, LayoutMode::Horizontal, 0.0, 4.0);
        let var = match v.get("Type").copied() {
            Some("Success") => "Success",
            Some("Warning") => "Warning",
            Some("Danger") => "Danger",
            _ => "Primary",
        };
        bind(doc, c, "fill", var);
        set_node(doc, c, |n| n.corners.radii = [10.0; 4]);
        let t = styled_text(doc, c, "New", "Text", "ds:text:Label/S")?;
        refs(doc, t, "characters", "Text");
        bind(doc, t, "fill", "On primary");
        set_props(doc, c, &badge_props);
        Ok(())
    })?;
    count += n;
    let badge_default = doc.node(badge).and_then(|s| s.children.first().copied()).unwrap_or(NodeId(0));
    // Cards: title, body, a slot for actions (a badge by default) and an exposed footer button.
    let mut card_props = BTreeMap::new();
    card_props.insert("Title".to_string(), PropDef::Text { default: "Card title".into() });
    card_props.insert("Body".to_string(), PropDef::Text { default: "Supporting text".into() });
    card_props.insert("Actions".to_string(), PropDef::Slot { preferred: vec![button, badge] });
    let axes = [("Size", sizes.clone()), ("Type", CARD_TYPES.to_vec()), ("Theme", themes.clone())];
    let y = y + (n.div_ceil(12) as f64) * 160.0 + 120.0;
    let (card, n) = component_set(doc, page, "Card", (0.0, y), &axes, 6, |doc, c, v| {
        auto_layout(doc, c, LayoutMode::Vertical, 8.0, 12.0);
        bind(doc, c, "fill", "Surface");
        bind(doc, c, "cornerRadius", "Radius");
        if v.get("Type") == Some(&"Outlined") {
            set_node(doc, c, |n| n.strokes = vec![Paint::solid(rgb(0xD9D9D9))]);
            bind(doc, c, "stroke", "Border");
        }
        let title =
            styled_text(doc, c, "Card title", "Title", if v.get("Size") == Some(&"Small") { "ds:text:Heading/M" } else { "ds:text:Heading/L" })?;
        refs(doc, title, "characters", "Title");
        bind(doc, title, "fill", "On surface");
        let body = styled_text(doc, c, "Supporting text", "Body", "ds:text:Body/M")?;
        refs(doc, body, "characters", "Body");
        set_node(doc, body, |n| n.styles.fill = Some("ds:paint:Text/Secondary".into()));
        let actions = doc.create(c, NodeKind::Frame, "Actions", 0.0, 0.0, 120.0, 24.0)?;
        auto_layout(doc, actions, LayoutMode::Horizontal, 4.0, 0.0);
        set_node(doc, actions, |n| n.fills.clear());
        nested(doc, badge_default, actions, "Badge", false)?;
        refs(doc, actions, crate::components::SLOT, "Actions");
        nested(doc, button_default, c, "Footer button", true)?;
        set_props(doc, c, &card_props);
        Ok(())
    })?;
    count += n;
    let card_small = doc.node(card).and_then(|s| s.children.first().copied()).unwrap_or(NodeId(0));
    let ghost = doc
        .node(button)
        .and_then(|s| {
            s.children.iter().copied().find(|c| doc.node(*c).is_some_and(|n| n.name.contains("Type=Secondary") || n.name.contains("Type=Ghost")))
        })
        .unwrap_or(button_default);
    // Navs: a logo icon, three item buttons and a profile card (card > button > icon: four deep).
    let mut nav_props = BTreeMap::new();
    nav_props.insert("Show profile".to_string(), PropDef::Boolean { default: true });
    let axes = [("Type", NAV_TYPES.to_vec()), ("Theme", themes.clone())];
    let y = y + (n.div_ceil(6) as f64) * 160.0 + 160.0;
    let (nav, n) = component_set(doc, page, "Nav", (0.0, y), &axes, 2, |doc, c, v| {
        let mode = if v.get("Type") == Some(&"Side") { LayoutMode::Vertical } else { LayoutMode::Horizontal };
        auto_layout(doc, c, mode, 8.0, 8.0);
        bind(doc, c, "fill", "Surface");
        nested(doc, icon(2), c, "Logo", false)?;
        for k in 1..=3 {
            nested(doc, ghost, c, &format!("Item {k}"), false)?;
        }
        let profile = nested(doc, card_small, c, "Profile", true)?;
        refs(doc, profile, "visible", "Show profile");
        set_props(doc, c, &nav_props);
        Ok(())
    })?;
    count += n;
    doc.link_layer_text_styles();
    Ok(DesignSystem { icons, button, input, card, nav, badge, components: count, theme, light, dark })
}

/// A new document holding the design system on its first page, normalised like an opened file.
pub fn design_system_document(spec: &DsSpec) -> DocResult<(Document, DesignSystem)> {
    let mut doc = Document::new("Design system");
    let page = doc.first_page().ok_or(DocError::Invalid("new document has no page".into()))?;
    set_node(&mut doc, page, |n| n.name = "Components".into());
    let ds = design_system(&mut doc, page, spec)?;
    crate::normalize(&mut doc, &Document::new(""));
    Ok((doc, ds))
}

/// The layer at name path `path` below `root` (each step a child's name).
pub fn find_path(doc: &Document, root: NodeId, path: &[&str]) -> Option<NodeId> {
    let mut cur = root;
    for step in path {
        cur = doc.node(cur)?.children.iter().copied().find(|c| doc.node(*c).is_some_and(|n| n.name == *step))?;
    }
    Some(cur)
}

/// The variant of set `set` named exactly `props` (in the set's property order), else its first.
fn variant(doc: &Document, set: NodeId, props: &[(&str, &str)]) -> NodeId {
    let wanted: BTreeMap<String, String> = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    crate::components::find_variant(doc, set, &wanted).or_else(|| doc.node(set).and_then(|s| s.children.first().copied())).unwrap_or(set)
}

/// Override `group` on layer `id` of an instance as the app records it (a value bound to a variable
/// is detached from it first).
fn record(doc: &mut Document, id: Option<NodeId>, group: &str, f: impl FnOnce(&mut Node)) {
    if let Some(id) = id {
        set_node(doc, id, |n| {
            if group == "fills" {
                n.bound.retain(|k, _| k != "fill");
            }
            f(n);
            n.overrides.insert(group.to_string());
        });
    }
}

fn instance_props(doc: &mut Document, id: Option<NodeId>, props: &[(&str, Value)], as_override: bool) {
    if let Some(id) = id {
        set_node(doc, id, |n| {
            if let NodeKind::Instance { props: p, .. } = &mut n.kind {
                for (k, v) in props {
                    p.insert(k.to_string(), v.clone());
                }
            }
            if as_override {
                n.overrides.insert("props".into());
            }
        });
    }
}

fn swap_nested(doc: &mut Document, id: Option<NodeId>, to: NodeId) {
    if let Some(id) = id {
        set_node(doc, id, |n| {
            if let NodeKind::Instance { component, .. } = &mut n.kind {
                *component = to;
            }
            n.overrides.insert("component".into());
        });
    }
}

fn hue(i: usize) -> Paint {
    let h = (i * 37 % 360) as f32 / 360.0;
    Paint::solid(Color::new(h, 0.4, 1.0 - h, 1.0))
}

/// Place `count` instances of the design system on new pages of `doc` ("Screens N", in frames of
/// 25), with overrides at every nesting level. Call [`crate::normalize`] afterwards. Returns the
/// top-level instances.
pub fn place_instances(doc: &mut Document, ds: &DesignSystem, count: usize) -> DocResult<Vec<NodeId>> {
    let per_frame = 25;
    let per_page = 500;
    let icon = |k: usize| ds.icons.get(k % ds.icons.len().max(1)).copied().unwrap_or(NodeId(0));
    let mut out = Vec::with_capacity(count);
    let mut page = NodeId(0);
    let mut frame = NodeId(0);
    for i in 0..count {
        if i % per_page == 0 {
            page = doc.add_page(&format!("Screens {}", i / per_page + 1));
        }
        if i % per_frame == 0 {
            let k = (i % per_page) / per_frame;
            frame = doc.create(
                page,
                NodeKind::Frame,
                &format!("Screen {}", i / per_frame + 1),
                (k % 5) as f64 * 1400.0,
                (k / 5) as f64 * 1000.0,
                1320.0,
                920.0,
            )?;
            if (i / per_frame) % 2 == 1 {
                // Every other screen shows the dark theme.
                set_node(doc, frame, |n| {
                    n.var_modes.insert(ds.theme.clone(), ds.dark.clone());
                });
            }
        }
        let slot = i % per_frame;
        let (x, y) = (20.0 + (slot % 5) as f64 * 260.0, 20.0 + (slot / 5) as f64 * 180.0);
        let size = SIZES.get(i % 3).copied().unwrap_or("Medium");
        let inst = match i % 10 {
            // Buttons: variant, label and icon through properties; a swapped icon with a recoloured
            // glyph (nested overrides); renamed now and then.
            0..=2 => {
                let main = variant(doc, ds.button, &[("Size", size), ("Type", BUTTON_TYPES.get(i % 3).copied().unwrap_or("Primary"))]);
                let inst = create_instance(doc, main, frame, x, y)?;
                instance_props(doc, Some(inst), &[("Label", Value::from(format!("Action {i}"))), ("Show icon", Value::Bool(i % 5 != 0))], false);
                if i % 2 == 0 {
                    instance_props(doc, Some(inst), &[("Icon", Value::from(icon(i).0))], false);
                    sync_instance(doc, inst);
                } else {
                    sync_instance(doc, inst);
                    let nested = find_path(doc, inst, &["Icon"]);
                    swap_nested(doc, nested, icon(i + 3));
                    sync_instance(doc, inst);
                }
                let glyph = find_path(doc, inst, &["Icon", "Glyph"]);
                record(doc, glyph, "fills", |n| n.fills = vec![hue(i)]);
                if i % 7 == 0 {
                    record(doc, Some(inst), "name", |n| n.name = format!("Button {i}"));
                }
                inst
            }
            // Inputs: label through a property, the value typed over, the leading icon swapped.
            3 | 4 => {
                let main = variant(doc, ds.input, &[("Size", size), ("State", INPUT_STATES.get(i % 4).copied().unwrap_or("Default"))]);
                let inst = create_instance(doc, main, frame, x, y)?;
                instance_props(doc, Some(inst), &[("Label", Value::from(format!("Field {i}")))], false);
                sync_instance(doc, inst);
                let lead = find_path(doc, inst, &["Field", "Leading icon"]);
                swap_nested(doc, lead, icon(i));
                sync_instance(doc, inst);
                let value = find_path(doc, inst, &["Field", "Value"]);
                record(doc, value, "characters", |n| {
                    if let NodeKind::Text(t) = &mut n.kind {
                        let len = t.char_count();
                        t.replace_range(0, len, &format!("Value {i}"));
                    }
                });
                inst
            }
            // Cards: title through a property; the footer button's label (nested property) and its
            // icon (swapped, recoloured: four levels down); the background recoloured.
            5 | 6 => {
                let main = variant(doc, ds.card, &[("Size", size), ("Type", CARD_TYPES.get(i % 2).copied().unwrap_or("Elevated"))]);
                let inst = create_instance(doc, main, frame, x, y)?;
                instance_props(doc, Some(inst), &[("Title", Value::from(format!("Card {i}")))], false);
                sync_instance(doc, inst);
                let footer = find_path(doc, inst, &["Footer button"]);
                instance_props(doc, footer, &[("Label", Value::from(format!("Open {i}")))], true);
                let ficon = find_path(doc, inst, &["Footer button", "Icon"]);
                swap_nested(doc, ficon, icon(i + 1));
                sync_instance(doc, inst);
                let glyph = find_path(doc, inst, &["Footer button", "Icon", "Glyph"]);
                record(doc, glyph, "fills", |n| n.fills = vec![hue(i + 1)]);
                record(doc, Some(inst), "fills", |n| n.fills = vec![hue(i)]);
                inst
            }
            // Navs: an item switched to another type (nested property), the profile card's title
            // and footer label typed over, its footer icon recoloured.
            7 => {
                let main = variant(doc, ds.nav, &[("Type", NAV_TYPES.get(i % 2).copied().unwrap_or("Top"))]);
                let inst = create_instance(doc, main, frame, x, y)?;
                let item = find_path(doc, inst, &["Item 2"]);
                instance_props(doc, item, &[("Label", Value::from("Settings"))], true);
                sync_instance(doc, inst);
                let title = find_path(doc, inst, &["Profile", "Title"]);
                record(doc, title, "characters", |n| {
                    if let NodeKind::Text(t) = &mut n.kind {
                        let len = t.char_count();
                        t.replace_range(0, len, &format!("Profile {i}"));
                    }
                });
                let label = find_path(doc, inst, &["Profile", "Footer button", "Label"]);
                record(doc, label, "characters", |n| {
                    if let NodeKind::Text(t) = &mut n.kind {
                        let len = t.char_count();
                        t.replace_range(0, len, "View");
                    }
                });
                let glyph = find_path(doc, inst, &["Profile", "Footer button", "Icon", "Glyph"]);
                record(doc, glyph, "fills", |n| n.fills = vec![hue(i)]);
                inst
            }
            // Badges: text through a property, recoloured.
            8 => {
                let main = variant(doc, ds.badge, &[("Type", BADGE_TYPES.get(i % 4).copied().unwrap_or("Info"))]);
                let inst = create_instance(doc, main, frame, x, y)?;
                instance_props(doc, Some(inst), &[("Text", Value::from(format!("{}", i % 100)))], false);
                sync_instance(doc, inst);
                record(doc, Some(inst), "fills", |n| n.fills = vec![hue(i)]);
                inst
            }
            // Icons, recoloured.
            _ => {
                let inst = create_instance(doc, icon(i), frame, x, y)?;
                let glyph = find_path(doc, inst, &["Glyph"]);
                record(doc, glyph, "fills", |n| n.fills = vec![hue(i)]);
                inst
            }
        };
        out.push(inst);
    }
    Ok(out)
}

/// The design system and `count` instances of it in one document, normalised like an opened file.
pub fn design_system_with_instances(spec: &DsSpec, count: usize) -> DocResult<(Document, DesignSystem, Vec<NodeId>)> {
    let mut doc = Document::new("Design system in use");
    let page = doc.first_page().ok_or(DocError::Invalid("new document has no page".into()))?;
    set_node(&mut doc, page, |n| n.name = "Components".into());
    let ds = design_system(&mut doc, page, spec)?;
    crate::normalize(&mut doc, &Document::new(""));
    let instances = place_instances(&mut doc, &ds, count)?;
    crate::normalize(&mut doc, &Document::new(""));
    doc.current_page = doc.first_page();
    Ok((doc, ds, instances))
}
