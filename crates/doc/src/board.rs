//! Boards: a file kind for whiteboarding, and the small piece of metadata board objects carry.
//!
//! Board objects are ordinary nodes (frames, vectors, text) with a [`BoardItem`] on them, so
//! rendering, hit testing, undo, saving, export, copy and paste and live sessions treat them like
//! any other layer. The metadata says what the object is for: which text a sticky note fits, what
//! a connector joins, who placed a stamp.

use figcraft_color::Color;
use figcraft_geom::kurbo::{Ellipse, RoundedRect};
use figcraft_geom::{BezPath, Point, Rect, Shape, Vec2};
use serde::{Deserialize, Serialize};

use crate::NodeId;

/// What a file is for: designing screens, or a board to think together on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    #[default]
    Design,
    Board,
}

impl FileKind {
    pub fn is_design(&self) -> bool {
        *self == FileKind::Design
    }
    pub fn id(self) -> &'static str {
        match self {
            FileKind::Design => "design",
            FileKind::Board => "board",
        }
    }
}

/// What a board object is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BoardItem {
    /// A sticky note: a frame whose first text layer is the note (its size shrinks as the note
    /// grows) and, below it, who wrote it.
    Sticky {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        author: String,
    },
    /// A shape with text: a frame holding the outline (a vector) and a centred text.
    Shape { shape: BoardShape },
    /// A line between two objects or points (a vector, re-routed when they move).
    Connector(Connector),
    /// The label in the middle of connector `connector` (a frame holding its text).
    Label { connector: NodeId },
    /// A stamp, who placed it, and the object it sits on (it moves with it).
    Stamp {
        stamp: Stamp,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        author: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<NodeId>,
    },
    /// A marker or highlighter stroke (what the eraser takes).
    Ink,
    /// A table: a frame of row frames holding cell frames with text (its column lines are thin
    /// rectangles laid over it).
    Table,
}

impl BoardItem {
    /// Objects selected and moved as a whole: a click on their text or outline picks them, and
    /// they never take other layers in.
    pub fn is_atom(&self) -> bool {
        matches!(self, BoardItem::Sticky { .. } | BoardItem::Shape { .. } | BoardItem::Label { .. } | BoardItem::Stamp { .. } | BoardItem::Table)
    }

    /// Objects whose first text layer is their text (a double-click or Enter edits it).
    pub fn has_text(&self) -> bool {
        matches!(self, BoardItem::Sticky { .. } | BoardItem::Shape { .. } | BoardItem::Label { .. })
    }

    /// Who made it (stickies and stamps; empty when unknown).
    pub fn author(&self) -> &str {
        match self {
            BoardItem::Sticky { author } | BoardItem::Stamp { author, .. } => author,
            _ => "",
        }
    }

    /// Point the layers this object refers to (a connector's ends, a label's connector) at their
    /// copies in `map`; the others stay.
    pub fn remap(&mut self, map: &std::collections::BTreeMap<NodeId, NodeId>) {
        match self {
            BoardItem::Connector(c) => {
                for end in [&mut c.from, &mut c.to] {
                    if let End::Node { node, .. } = end
                        && let Some(m) = map.get(node)
                    {
                        *node = *m;
                    }
                }
            }
            BoardItem::Label { connector } => {
                if let Some(m) = map.get(connector) {
                    *connector = *m;
                }
            }
            _ => {}
        }
    }

    pub fn kind_id(&self) -> &'static str {
        match self {
            BoardItem::Sticky { .. } => "sticky",
            BoardItem::Shape { .. } => "shape",
            BoardItem::Connector(_) => "connector",
            BoardItem::Label { .. } => "label",
            BoardItem::Stamp { .. } => "stamp",
            BoardItem::Ink => "ink",
            BoardItem::Table => "table",
        }
    }
}

/// The colours stickies, shapes and connectors come in: soft tints that dark text reads on.
pub const BOARD_COLORS: [(&str, Color); 8] = [
    ("yellow", Color::from_rgb8_const(0xFF, 0xE7, 0x8A)),
    ("orange", Color::from_rgb8_const(0xFF, 0xC9, 0x94)),
    ("red", Color::from_rgb8_const(0xFF, 0xAE, 0xA6)),
    ("pink", Color::from_rgb8_const(0xF7, 0xB9, 0xDA)),
    ("violet", Color::from_rgb8_const(0xD4, 0xC2, 0xFC)),
    ("blue", Color::from_rgb8_const(0xAC, 0xD3, 0xFA)),
    ("green", Color::from_rgb8_const(0xC2, 0xE8, 0xA6)),
    ("gray", Color::from_rgb8_const(0xE3, 0xE1, 0xDD)),
];

/// A board colour by name ("yellow") or as hex ("#FFE78A").
pub fn board_color(s: &str) -> Option<Color> {
    let s = s.trim();
    BOARD_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c).or_else(|| Color::from_hex(s))
}

/// The page colour boards start with: a warm light grey the colours stand out on.
pub const BOARD_PAGE: Color = Color::from_rgb8_const(0xF4, 0xF3, 0xF0);

/// The outlines shapes with text come in: the basic ones, then arrows and flowchart symbols.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoardShape {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Triangle,
    SpeechBubble,
    Chevron,
    // Added later: after the first ones, which keep their place in binary encodings.
    TriangleDown,
    ParallelogramRight,
    ParallelogramLeft,
    Trapezoid,
    Pentagon,
    Hexagon,
    Octagon,
    Star,
    Plus,
    ArrowLeft,
    ArrowRight,
    Database,
    Document,
    PredefinedProcess,
    ManualInput,
    InternalStorage,
}

impl BoardShape {
    pub const ALL: [BoardShape; 23] = [
        BoardShape::Rectangle,
        BoardShape::RoundedRectangle,
        BoardShape::Ellipse,
        BoardShape::Diamond,
        BoardShape::Triangle,
        BoardShape::TriangleDown,
        BoardShape::ParallelogramRight,
        BoardShape::ParallelogramLeft,
        BoardShape::Trapezoid,
        BoardShape::Pentagon,
        BoardShape::Hexagon,
        BoardShape::Octagon,
        BoardShape::Star,
        BoardShape::Plus,
        BoardShape::ArrowLeft,
        BoardShape::ArrowRight,
        BoardShape::Chevron,
        BoardShape::SpeechBubble,
        BoardShape::Database,
        BoardShape::Document,
        BoardShape::PredefinedProcess,
        BoardShape::ManualInput,
        BoardShape::InternalStorage,
    ];

    /// The id used in commands and saved files, and the English name shown in the UI.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            BoardShape::Rectangle => ("rectangle", "Rectangle"),
            BoardShape::RoundedRectangle => ("roundedRectangle", "Rounded rectangle"),
            BoardShape::Ellipse => ("ellipse", "Ellipse"),
            BoardShape::Diamond => ("diamond", "Diamond"),
            BoardShape::Triangle => ("triangle", "Triangle"),
            BoardShape::TriangleDown => ("triangleDown", "Triangle down"),
            BoardShape::ParallelogramRight => ("parallelogramRight", "Parallelogram right"),
            BoardShape::ParallelogramLeft => ("parallelogramLeft", "Parallelogram left"),
            BoardShape::Trapezoid => ("trapezoid", "Trapezoid"),
            BoardShape::Pentagon => ("pentagon", "Pentagon"),
            BoardShape::Hexagon => ("hexagon", "Hexagon"),
            BoardShape::Octagon => ("octagon", "Octagon"),
            BoardShape::Star => ("star", "Star"),
            BoardShape::Plus => ("plus", "Plus"),
            BoardShape::ArrowLeft => ("arrowLeft", "Arrow left"),
            BoardShape::ArrowRight => ("arrowRight", "Arrow right"),
            BoardShape::Chevron => ("chevron", "Chevron"),
            BoardShape::SpeechBubble => ("speechBubble", "Speech bubble"),
            BoardShape::Database => ("database", "Database"),
            BoardShape::Document => ("document", "Document"),
            BoardShape::PredefinedProcess => ("predefinedProcess", "Predefined process"),
            BoardShape::ManualInput => ("manualInput", "Manual input"),
            BoardShape::InternalStorage => ("internalStorage", "Internal storage"),
        }
    }

    pub fn id(self) -> &'static str {
        self.names().0
    }

    pub fn from_id(s: &str) -> Option<BoardShape> {
        BoardShape::ALL.into_iter().find(|b| b.id().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        self.names().1
    }

    /// The outline in a `w` × `h` box at the origin.
    pub fn path(self, w: f64, h: f64) -> BezPath {
        let poly = |pts: &[(f64, f64)]| {
            let mut p = BezPath::new();
            for (i, (x, y)) in pts.iter().enumerate() {
                let q = Point::new(x * w, y * h);
                if i == 0 { p.move_to(q) } else { p.line_to(q) }
            }
            p.close_path();
            p
        };
        let r = w.min(h);
        match self {
            BoardShape::Rectangle => Rect::new(0.0, 0.0, w, h).to_path(0.1),
            BoardShape::RoundedRectangle => RoundedRect::new(0.0, 0.0, w, h, r * 0.16).to_path(0.1),
            BoardShape::Ellipse => Ellipse::from_rect(Rect::new(0.0, 0.0, w, h)).to_path(0.1),
            BoardShape::Diamond => poly(&[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
            BoardShape::Triangle => poly(&[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            BoardShape::TriangleDown => poly(&[(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)]),
            BoardShape::ParallelogramRight => poly(&[(0.2, 0.0), (1.0, 0.0), (0.8, 1.0), (0.0, 1.0)]),
            BoardShape::ParallelogramLeft => poly(&[(0.0, 0.0), (0.8, 0.0), (1.0, 1.0), (0.2, 1.0)]),
            BoardShape::Trapezoid => poly(&[(0.2, 0.0), (0.8, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            BoardShape::Pentagon => poly(&[(0.5, 0.0), (1.0, 0.38), (0.81, 1.0), (0.19, 1.0), (0.0, 0.38)]),
            BoardShape::Hexagon => poly(&[(0.25, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.25, 1.0), (0.0, 0.5)]),
            BoardShape::Octagon => poly(&[(0.3, 0.0), (0.7, 0.0), (1.0, 0.3), (1.0, 0.7), (0.7, 1.0), (0.3, 1.0), (0.0, 0.7), (0.0, 0.3)]),
            BoardShape::Star => poly(&star_points()),
            BoardShape::Plus => poly(&[
                (THIRD, 0.0),
                (2.0 * THIRD, 0.0),
                (2.0 * THIRD, THIRD),
                (1.0, THIRD),
                (1.0, 2.0 * THIRD),
                (2.0 * THIRD, 2.0 * THIRD),
                (2.0 * THIRD, 1.0),
                (THIRD, 1.0),
                (THIRD, 2.0 * THIRD),
                (0.0, 2.0 * THIRD),
                (0.0, THIRD),
                (THIRD, THIRD),
            ]),
            BoardShape::ArrowLeft => poly(&[(1.0, 0.25), (0.4, 0.25), (0.4, 0.0), (0.0, 0.5), (0.4, 1.0), (0.4, 0.75), (1.0, 0.75)]),
            BoardShape::ArrowRight => poly(&[(0.0, 0.25), (0.6, 0.25), (0.6, 0.0), (1.0, 0.5), (0.6, 1.0), (0.6, 0.75), (0.0, 0.75)]),
            BoardShape::Chevron => poly(&[(0.0, 0.0), (0.72, 0.0), (1.0, 0.5), (0.72, 1.0), (0.0, 1.0), (0.28, 0.5)]),
            BoardShape::ManualInput => poly(&[(0.0, 0.25), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            BoardShape::Database => database(w, h),
            BoardShape::Document => document(w, h),
            BoardShape::PredefinedProcess => with_lines(w, h, &[[0.1, 0.0, 0.1, 1.0], [0.9, 0.0, 0.9, 1.0]]),
            BoardShape::InternalStorage => with_lines(w, h, &[[0.12, 0.0, 0.12, 1.0], [0.0, 0.12, 1.0, 0.12]]),
            BoardShape::SpeechBubble => {
                // A rounded body over the bottom fifth, with a tail down to the left.
                let (b, c) = (h * 0.8, r * 0.14);
                let mut p = BezPath::new();
                p.move_to((c, 0.0));
                p.line_to((w - c, 0.0));
                p.quad_to((w, 0.0), (w, c));
                p.line_to((w, b - c));
                p.quad_to((w, b), (w - c, b));
                p.line_to((w * 0.42, b));
                p.line_to((w * 0.18, h));
                p.line_to((w * 0.24, b));
                p.line_to((c, b));
                p.quad_to((0.0, b), (0.0, b - c));
                p.line_to((0.0, c));
                p.quad_to((0.0, 0.0), (c, 0.0));
                p.close_path();
                p
            }
        }
    }

    /// Where the text goes, as insets of the box (top, right, bottom, left, in fractions of its
    /// size): inside the outline's widest part.
    pub fn text_insets(self) -> [f64; 4] {
        match self {
            BoardShape::Rectangle | BoardShape::RoundedRectangle => [0.08, 0.08, 0.08, 0.08],
            BoardShape::Ellipse => [0.16, 0.16, 0.16, 0.16],
            BoardShape::Diamond => [0.25, 0.25, 0.25, 0.25],
            BoardShape::Triangle => [0.45, 0.22, 0.06, 0.22],
            BoardShape::TriangleDown => [0.06, 0.22, 0.45, 0.22],
            BoardShape::ParallelogramRight | BoardShape::ParallelogramLeft | BoardShape::Hexagon => [0.08, 0.22, 0.08, 0.22],
            BoardShape::Trapezoid => [0.1, 0.22, 0.08, 0.22],
            BoardShape::Pentagon => [0.25, 0.18, 0.08, 0.18],
            BoardShape::Octagon => [0.12, 0.12, 0.12, 0.12],
            BoardShape::Star => [0.38, 0.32, 0.28, 0.32],
            BoardShape::Plus => [0.34, 0.1, 0.34, 0.1],
            BoardShape::ArrowLeft => [0.27, 0.04, 0.27, 0.3],
            BoardShape::ArrowRight => [0.27, 0.3, 0.27, 0.04],
            BoardShape::SpeechBubble => [0.08, 0.08, 0.28, 0.08],
            BoardShape::Chevron => [0.12, 0.3, 0.12, 0.3],
            BoardShape::Database => [0.24, 0.06, 0.1, 0.06],
            BoardShape::Document => [0.08, 0.08, 0.2, 0.08],
            BoardShape::PredefinedProcess => [0.08, 0.14, 0.08, 0.14],
            BoardShape::ManualInput => [0.3, 0.08, 0.08, 0.08],
            BoardShape::InternalStorage => [0.18, 0.08, 0.08, 0.18],
        }
    }
}

const THIRD: f64 = 1.0 / 3.0;
/// How far a cubic's handles reach to draw a quarter ellipse.
const QUARTER: f64 = 0.552_284_75;

/// A five-pointed star's points in the unit box, stretched to fill it (the lower points of a
/// regular star stop short of the bottom).
fn star_points() -> Vec<(f64, f64)> {
    let pts: Vec<(f64, f64)> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { 0.5 } else { 0.2 };
            let a = f64::from(i) * std::f64::consts::PI / 5.0;
            (0.5 + r * a.sin(), 0.5 - r * a.cos())
        })
        .collect();
    let bottom = pts.iter().map(|p| p.1).fold(0.0, f64::max);
    pts.into_iter().map(|(x, y)| (x, y / bottom)).collect()
}

/// A cylinder seen from the side: an elliptic top, with its near rim drawn, and an elliptic
/// bottom.
fn database(w: f64, h: f64) -> BezPath {
    let (rx, ry) = (w / 2.0, (h * 0.1).min(w * 0.25));
    let (k, kx) = (ry * QUARTER, rx * QUARTER);
    let mut p = BezPath::new();
    p.move_to((0.0, ry));
    p.curve_to((0.0, ry - k), (rx - kx, 0.0), (rx, 0.0));
    p.curve_to((rx + kx, 0.0), (w, ry - k), (w, ry));
    p.line_to((w, h - ry));
    p.curve_to((w, h - ry + k), (rx + kx, h), (rx, h));
    p.curve_to((rx - kx, h), (0.0, h - ry + k), (0.0, h - ry));
    p.close_path();
    // The top's near rim: a line, not a region.
    p.move_to((0.0, ry));
    p.curve_to((0.0, ry + k), (rx - kx, 2.0 * ry), (rx, 2.0 * ry));
    p.curve_to((rx + kx, 2.0 * ry), (w, ry + k), (w, ry));
    p
}

/// A page whose bottom edge waves.
fn document(w: f64, h: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((w, 0.0));
    p.line_to((w, h * 0.86));
    p.curve_to((w * 0.7, h * 0.7), (w * 0.3, h * 1.04), (0.0, h * 0.86));
    p.close_path();
    p
}

/// A rectangle with straight `lines` across it (`[x0, y0, x1, y1]` in the unit box), drawn as
/// lines, not regions.
fn with_lines(w: f64, h: f64, lines: &[[f64; 4]]) -> BezPath {
    let mut p = Rect::new(0.0, 0.0, w, h).to_path(0.1);
    for [x0, y0, x1, y1] in lines {
        p.move_to((x0 * w, y0 * h));
        p.line_to((x1 * w, y1 * h));
    }
    p
}

/// The stamps: round badges with a mark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stamp {
    #[default]
    ThumbsUp,
    Heart,
    PlusOne,
    Star,
    Question,
    Check,
}

/// One part of a stamp's mark: SVG path data on the badge's 40-unit box, filled or stroked.
pub struct MarkPart {
    pub d: &'static str,
    pub filled: bool,
}

/// A stamp badge's size.
pub const STAMP_SIZE: f64 = 40.0;

impl Stamp {
    pub const ALL: [Stamp; 6] = [Stamp::ThumbsUp, Stamp::Heart, Stamp::PlusOne, Stamp::Star, Stamp::Question, Stamp::Check];

    pub fn id(self) -> &'static str {
        match self {
            Stamp::ThumbsUp => "thumbsUp",
            Stamp::Heart => "heart",
            Stamp::PlusOne => "plusOne",
            Stamp::Star => "star",
            Stamp::Question => "question",
            Stamp::Check => "check",
        }
    }

    pub fn from_id(s: &str) -> Option<Stamp> {
        Stamp::ALL.into_iter().find(|b| b.id().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        match self {
            Stamp::ThumbsUp => "Thumbs up",
            Stamp::Heart => "Heart",
            Stamp::PlusOne => "Plus one",
            Stamp::Star => "Star",
            Stamp::Question => "Question",
            Stamp::Check => "Check",
        }
    }

    /// The badge's colour (the mark is white on it).
    pub fn color(self) -> Color {
        match self {
            Stamp::ThumbsUp => Color::from_rgb8_const(0x3B, 0x82, 0xE8),
            Stamp::Heart => Color::from_rgb8_const(0xE5, 0x48, 0x5F),
            Stamp::PlusOne => Color::from_rgb8_const(0x2E, 0xA0, 0x63),
            Stamp::Star => Color::from_rgb8_const(0xEE, 0x9B, 0x1A),
            Stamp::Question => Color::from_rgb8_const(0x84, 0x5E, 0xD8),
            Stamp::Check => Color::from_rgb8_const(0x14, 0x96, 0x8C),
        }
    }

    /// The mark, drawn for FigCraft from lines, arcs and simple outlines.
    pub fn mark(self) -> &'static [MarkPart] {
        match self {
            // A cuff and a raised hand with its thumb up.
            Stamp::ThumbsUp => &[
                MarkPart { d: "M10.5 19 H15 V29 H10.5 Z", filled: true },
                MarkPart {
                    d: "M16.5 19 L20.6 11.4 C21.6 9.8 24.6 10.6 24.2 13 L23.4 17 H28.2 C29.8 17 30.9 18.5 30.4 20 L28.4 27.3 C28 28.4 27.1 29 25.9 29 H16.5 Z",
                    filled: true,
                },
            ],
            // Two round lobes meeting in a point.
            Stamp::Heart => &[MarkPart {
                d: "M20 29.5 C12.5 24.5 9.5 20.6 10.4 16.4 C11.3 12.2 16.8 11.2 20 15.4 C23.2 11.2 28.7 12.2 29.6 16.4 C30.5 20.6 27.5 24.5 20 29.5 Z",
                filled: true,
            }],
            Stamp::PlusOne => &[MarkPart { d: "M10 20 H20 M15 15 V25", filled: false }, MarkPart { d: "M24.5 15 L27.5 12.5 V27.5", filled: false }],
            Stamp::Star => &[MarkPart {
                d: "M20 10.5 L22.8 16.7 L29.5 17.4 L24.5 21.9 L25.9 28.6 L20 25.2 L14.1 28.6 L15.5 21.9 L10.5 17.4 L17.2 16.7 Z",
                filled: true,
            }],
            Stamp::Question => &[
                MarkPart { d: "M15.6 16.2 C15.6 10.6 24.4 10.6 24.4 16 C24.4 19.6 20 19.9 20 23.6", filled: false },
                MarkPart { d: "M18.2 28 A1.8 1.8 0 1 0 21.8 28 A1.8 1.8 0 1 0 18.2 28 Z", filled: true },
            ],
            Stamp::Check => &[MarkPart { d: "M12.5 20.5 L18 26 L28 14.5", filled: false }],
        }
    }
}

/// How a connector runs between its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Routing {
    #[default]
    Straight,
    /// Right angles only: out of each end along its side's normal, turning once or twice, around
    /// the objects it joins when the short way would cross them.
    Elbow,
    /// One smooth curve, leaving and arriving along the sides' normals.
    Curved,
}

impl Routing {
    pub fn id(self) -> &'static str {
        match self {
            Routing::Straight => "straight",
            Routing::Elbow => "elbow",
            Routing::Curved => "curved",
        }
    }
    pub fn from_id(s: &str) -> Option<Routing> {
        [Routing::Straight, Routing::Elbow, Routing::Curved].into_iter().find(|r| r.id().eq_ignore_ascii_case(s))
    }
}

/// The side of an object a connector end is attached to (at its middle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// The side facing the other end.
    #[default]
    Auto,
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub fn from_id(s: &str) -> Option<Side> {
        Some(match s.to_ascii_lowercase().as_str() {
            "auto" => Side::Auto,
            "top" => Side::Top,
            "right" => Side::Right,
            "bottom" => Side::Bottom,
            "left" => Side::Left,
            _ => return None,
        })
    }

    /// The side's middle on `b`.
    pub fn point(self, b: Rect) -> Point {
        let c = b.center();
        match self {
            Side::Top => Point::new(c.x, b.y0),
            Side::Right => Point::new(b.x1, c.y),
            Side::Bottom => Point::new(c.x, b.y1),
            Side::Left => Point::new(b.x0, c.y),
            Side::Auto => c,
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }

    /// The way out of the side (y down; none for [`Side::Auto`]).
    pub fn normal(self) -> Vec2 {
        match self {
            Side::Top => Vec2::new(0.0, -1.0),
            Side::Right => Vec2::new(1.0, 0.0),
            Side::Bottom => Vec2::new(0.0, 1.0),
            Side::Left => Vec2::new(-1.0, 0.0),
            Side::Auto => Vec2::ZERO,
        }
    }
}

/// One end of a connector: attached to a layer, or a free point on the page.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum End {
    Node {
        node: NodeId,
        #[serde(default)]
        side: Side,
    },
    Point {
        x: f64,
        y: f64,
    },
}

impl End {
    pub fn node(&self) -> Option<NodeId> {
        match self {
            End::Node { node, .. } => Some(*node),
            End::Point { .. } => None,
        }
    }
}

/// A connector's path as reshaped by hand. Its numbers are measured from the middle of the
/// connector's two ends, so the shape travels with what the connector joins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorPath {
    /// An elbow's straight runs, each at right angles to the one before: whether the first runs
    /// across (else up or down), and where the runs between the first and last lie (x of one that
    /// runs up or down, y of one that runs across). The first and last lie on the ends.
    Runs { across: bool, at: Vec<f64> },
    /// How far a curve's middle was moved.
    Bow { x: f64, y: f64 },
}

/// What a connector joins and how it runs (its arrowheads are its stroke's caps).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connector {
    pub from: End,
    pub to: End,
    #[serde(default)]
    pub routing: Routing,
    /// Its path as reshaped by hand; `None`: routed by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<ConnectorPath>,
}
