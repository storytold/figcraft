//! CraftJam boards ([`figcraft_doc::Document::board`]): the whiteboard look new layers get there
//! and the fixed colour palette shapes, borders, sticky notes and connectors share.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Node, NodeKind};

/// A board's page colour: a very light grey (the dot grid is drawn over it).
pub const BACKGROUND: Color = Color::from_rgb8_const(0xF3, 0xF3, 0xF1);

/// The colour of the dots of a board's grid.
pub const DOTS: Color = Color::from_rgb8_const(0xC9, 0xC9, 0xC6);

/// One colour of the palette: its name (for tooltips and commands) and value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swatch {
    pub name: &'static str,
    pub color: Color,
}

const fn sw(name: &'static str, r: u8, g: u8, b: u8) -> Swatch {
    Swatch { name, color: Color::from_rgb8_const(r, g, b) }
}

/// The palette's strong colours (top row).
pub const STRONG: [Swatch; 11] = [
    sw("Black", 0x1E, 0x1E, 0x1E),
    sw("Dark gray", 0x75, 0x75, 0x75),
    sw("Red", 0xE5, 0x48, 0x4D),
    sw("Orange", 0xF7, 0x6B, 0x15),
    sw("Yellow", 0xFF, 0xC5, 0x3D),
    sw("Green", 0x30, 0xA4, 0x6C),
    sw("Teal", 0x12, 0xA5, 0x94),
    sw("Blue", 0x00, 0x90, 0xFF),
    sw("Violet", 0x8E, 0x4E, 0xC6),
    sw("Pink", 0xD6, 0x40, 0x9F),
    sw("White", 0xFF, 0xFF, 0xFF),
];

/// The palette's light colours (bottom row; the free colour picker comes after them).
pub const LIGHT: [Swatch; 10] = [
    sw("Gray", 0xB3, 0xB3, 0xB3),
    sw("Light gray", 0xE6, 0xE6, 0xE6),
    sw("Light red", 0xFF, 0xD6, 0xD6),
    sw("Light orange", 0xFF, 0xE0, 0xC2),
    sw("Light yellow", 0xFF, 0xF1, 0xB8),
    sw("Light green", 0xCD, 0xEF, 0xD7),
    sw("Light teal", 0xC6, 0xF0, 0xEA),
    sw("Light blue", 0xD2, 0xE8, 0xFF),
    sw("Light violet", 0xE6, 0xD9, 0xF7),
    sw("Light pink", 0xF9, 0xD6, 0xEA),
];

/// Every palette colour, strong then light.
pub fn palette() -> impl Iterator<Item = Swatch> {
    STRONG.into_iter().chain(LIGHT)
}

/// The palette colour named `name` (case and spaces ignored: `lightBlue`, `light blue`).
pub fn swatch(name: &str) -> Option<Swatch> {
    let key = |s: &str| s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').collect::<String>().to_ascii_lowercase();
    let k = key(name);
    palette().find(|s| key(s.name) == k)
}

/// The fill new shapes get on a board.
pub const SHAPE_FILL: Color = LIGHT[7].color;

/// A darker tone of `c`, for the border of a strong-coloured shape.
pub fn darker(c: Color) -> Color {
    Color { r: c.r * 0.78, g: c.g * 0.78, b: c.b * 0.78, a: c.a }
}

/// Give a layer just made on a board the board's look: shapes pastel, borderless, slightly
/// rounded; texts dark and readable at the board's sizes.
pub fn dress(n: &mut Node) {
    match &n.kind {
        NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } => {
            n.fills = vec![Paint::solid(SHAPE_FILL)];
            n.strokes.clear();
            if matches!(n.kind, NodeKind::Rectangle) {
                n.corners.radii = [4.0; 4];
            }
        }
        NodeKind::Text(_) => {
            n.fills = vec![Paint::solid(STRONG[0].color)];
            if let NodeKind::Text(t) = &mut n.kind {
                t.style.font_size = TextSize::Small.px();
                t.style.font_family = FontLook::Simple.family();
            }
        }
        _ => {}
    }
}

/// The text sizes a board offers by name (any other size can be typed in too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    Small,
    Medium,
    Large,
    ExtraLarge,
    Huge,
}

impl TextSize {
    pub const ALL: [TextSize; 5] = [TextSize::Small, TextSize::Medium, TextSize::Large, TextSize::ExtraLarge, TextSize::Huge];

    pub fn px(self) -> f64 {
        match self {
            TextSize::Small => 16.0,
            TextSize::Medium => 24.0,
            TextSize::Large => 40.0,
            TextSize::ExtraLarge => 64.0,
            TextSize::Huge => 96.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TextSize::Small => "Small",
            TextSize::Medium => "Medium",
            TextSize::Large => "Large",
            TextSize::ExtraLarge => "Extra large",
            TextSize::Huge => "Huge",
        }
    }

    /// The named size `px` is, if it is one.
    pub fn of(px: f64) -> Option<TextSize> {
        Self::ALL.into_iter().find(|t| (t.px() - px).abs() < 1e-6)
    }
}

/// The board's four font looks; each is the first of its families installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontLook {
    Simple,
    Bookish,
    Technical,
    Scribbled,
}

impl FontLook {
    pub const ALL: [FontLook; 4] = [FontLook::Simple, FontLook::Bookish, FontLook::Technical, FontLook::Scribbled];

    pub fn label(self) -> &'static str {
        match self {
            FontLook::Simple => "Simple",
            FontLook::Bookish => "Bookish",
            FontLook::Technical => "Technical",
            FontLook::Scribbled => "Scribbled",
        }
    }

    /// The families this look uses, most wanted first (open fonts, then common system ones).
    pub fn families(self) -> &'static [&'static str] {
        match self {
            FontLook::Simple => &["Inter", "Roboto", "Noto Sans", "Open Sans", "DejaVu Sans", "Arial", "Helvetica"],
            FontLook::Bookish => &["Source Serif 4", "Lora", "Noto Serif", "Georgia", "DejaVu Serif", "Times New Roman", "FreeSerif"],
            FontLook::Technical => &["JetBrains Mono", "Roboto Mono", "IBM Plex Mono", "Fira Mono", "DejaVu Sans Mono", "Courier New", "FreeMono"],
            FontLook::Scribbled => &["Caveat", "Patrick Hand", "Kalam", "Comic Neue", "Comic Sans MS", "Chalkboard", "Segoe Print"],
        }
    }

    /// The family to use: the first of [`Self::families`] the fonts have (the first one when none
    /// is: it falls back like any missing font).
    pub fn family(self) -> String {
        let db = figcraft_text::fontdb::db().read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let fams = self.families();
        fams.iter().find(|f| db.has_family(f)).or(fams.first()).map(|f| (*f).to_string()).unwrap_or_default()
    }

    /// The look `family` belongs to.
    pub fn of(family: &str) -> Option<FontLook> {
        Self::ALL.into_iter().find(|l| l.families().iter().any(|f| f.eq_ignore_ascii_case(family)))
    }
}

/// The sticky notes' colours (the toolbar's pile and the context toolbar).
pub const STICKY_COLORS: [Swatch; 10] = [
    sw("Yellow", 0xFF, 0xE8, 0x8A),
    sw("Orange", 0xFF, 0xCF, 0x9E),
    sw("Red", 0xFF, 0xB8, 0xB3),
    sw("Pink", 0xFF, 0xC4, 0xE4),
    sw("Violet", 0xD9, 0xC8, 0xF7),
    sw("Blue", 0xB8, 0xDB, 0xFF),
    sw("Teal", 0xAE, 0xEB, 0xE0),
    sw("Green", 0xC2, 0xEC, 0xB4),
    sw("Gray", 0xE3, 0xE3, 0xE3),
    sw("White", 0xFF, 0xFF, 0xFF),
];

/// A sticky note's side (a wide one is twice as wide).
pub const STICKY_SIZE: f64 = 240.0;

/// A new sticky note `id` with its top-left at (`x`, `y`) in its parent's space: a square (or wide)
/// note of `color` with a soft shadow, signed by `author` (empty: unsigned).
pub fn new_sticky(id: figcraft_doc::NodeId, x: f64, y: f64, color: Color, wide: bool, author: &str) -> Node {
    let w = if wide { 2.0 * STICKY_SIZE } else { STICKY_SIZE };
    let mut n = Node::new(id, NodeKind::Rectangle, "Sticky", w, STICKY_SIZE);
    n.fills = vec![Paint::solid(color)];
    n.strokes.clear();
    n.corners.radii = [2.0; 4];
    n.transform = figcraft_geom::Affine::translate((x, y));
    n.effects = vec![figcraft_doc::Effect {
        kind: figcraft_doc::EffectKind::DropShadow(figcraft_doc::Shadow {
            color: Color::new(0.0, 0.0, 0.0, 0.12),
            offset: (0.0, 2.0),
            radius: 6.0,
            ..figcraft_doc::Shadow::default()
        }),
        visible: true,
    }];
    n.plugin_data.insert(figcraft_doc::shape_text::STICKY.into(), author.trim().to_string());
    n
}

/// Sticky notes grow with their words: each note in `touched` (or whose text is) is as tall as its
/// text and the author's line need, and never shorter than [`STICKY_SIZE`]. Returns the notes
/// resized.
pub fn grow_stickies(doc: &mut figcraft_doc::Document, touched: &std::collections::BTreeSet<figcraft_doc::NodeId>) -> Vec<figcraft_doc::NodeId> {
    use figcraft_doc::shape_text;
    let notes: Vec<(figcraft_doc::NodeId, figcraft_doc::NodeId)> = doc
        .nodes()
        .filter(|n| n.text_host.is_some() && matches!(n.kind, NodeKind::Text(_)))
        .filter_map(|t| {
            let h = t.text_host?;
            let host = doc.node(h)?;
            (shape_text::is_sticky(host) && (touched.contains(&h) || touched.contains(&t.id))).then_some((h, t.id))
        })
        .collect();
    let mut out = Vec::new();
    for (h, t) in notes {
        let (Some(host), Some(text)) = (doc.node(h), doc.node(t)) else { continue };
        let width = (host.width - 2.0 * shape_text::STICKY_PAD).max(1.0);
        let mut probe = text.clone();
        probe.width = width;
        let text_h = figcraft_text::auto_size(&probe).map(|(_, h)| h).unwrap_or(text.height);
        let author = if shape_text::sticky_author(host).is_some() { shape_text::STICKY_AUTHOR } else { 0.0 };
        let need = (text_h + 2.0 * shape_text::STICKY_PAD + author).max(STICKY_SIZE);
        if let Some(tn) = doc.node_mut(t)
            && ((tn.height - text_h).abs() > 1e-6 || (tn.width - width).abs() > 1e-6)
        {
            tn.height = text_h;
            tn.width = width;
        }
        if let Some(hn) = doc.node_mut(h)
            && (hn.height - need).abs() > 1e-6
        {
            hn.height = need;
            out.push(h);
        }
    }
    out
}
