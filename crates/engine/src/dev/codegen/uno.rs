//! Uno Platform C# Markup: a layer and its subtree as a fluent C# expression tree of WinUI
//! elements (`Uno.Extensions.Markup`), inside a small `UserControl` class. Lengths are fixed and
//! children sit at their offsets in a `Canvas`, as in the Flutter target before its auto layout
//! code. Whatever has no counterpart is left as a `// not mapped: …` comment in the chain.

use std::collections::BTreeSet;

use figcraft_color::{BlendMode, Color, ImageScaleMode, PaintKind};
use figcraft_doc::{
    AutoLayout, CellAlign, CrossAlign, Document, EffectKind, HConstraint, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind,
    Sizing, StrokeAlign, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle, VConstraint,
};

use super::swift::one_line;
use super::{Ctx, MAX_CHILDREN, first_fill, num, stroke_color};

/// Longest string literal written (characters); the rest is cut.
const MAX_LITERAL: usize = 10_000;
/// Longest layer name written into a comment (characters).
const MAX_COMMENT: usize = 80;
/// Longest identifier or asset name written (characters).
const MAX_IDENT: usize = 40;
/// Most style runs of one text written as `Run`s.
const MAX_RUNS: usize = 200;
/// Where the app keeps the images and vectors the code refers to.
const ASSETS: &str = "ms-appx:///Assets";

/// C# indents by four spaces.
pub(crate) fn uno_indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// A C# string literal: `"` and `\` escaped, control and line separator characters as escapes,
/// never an interpolated string. Cut at [`MAX_LITERAL`] characters.
pub(crate) fn cs_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(MAX_LITERAL) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Other control characters and the line and paragraph separators end a C# line.
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// C# reserved words, which an identifier may not be (all lowercase, so only camelCase meets them).
const CS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// The ASCII alphanumeric words of `name` after transliteration (so non-Latin names stay
/// readable), kept whole up to [`MAX_IDENT`] characters; the first word is kept even when it alone
/// is too long (the caller cuts it).
fn ascii_words(name: &str) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut len = 0;
    for w in deunicode::deunicode(name).split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        if !kept.is_empty() && len + w.len() > MAX_IDENT {
            break;
        }
        len += w.len();
        kept.push(w.to_string());
    }
    kept
}

/// A C# identifier from a layer name: words joined as PascalCase (`pascal`) or camelCase, cut at
/// [`MAX_IDENT`] characters on a word boundary. A name left with nothing takes `kind`; a leading
/// digit gets a `_` prefix and a reserved word a `_` suffix (never `@`).
pub(crate) fn cs_ident(name: &str, pascal: bool, kind: &str) -> String {
    let mut out = String::new();
    for (i, w) in ascii_words(name).iter().enumerate() {
        let mut cs = w.chars();
        let Some(first) = cs.next() else { continue };
        // The first letter of each word is upper-case (the first word of camelCase lower-case);
        // the rest keeps its case, so `mainCard` stays recognisable.
        if i == 0 && !pascal {
            out.extend(first.to_lowercase());
        } else {
            out.extend(first.to_uppercase());
        }
        out.push_str(cs.as_str());
    }
    let mut out: String = out.chars().take(MAX_IDENT).collect();
    if out.is_empty() {
        let k: String = kind.chars().filter(char::is_ascii_alphanumeric).collect();
        let mut cs = k.chars();
        out = match cs.next() {
            Some(f) if pascal => f.to_uppercase().chain(cs).collect(),
            Some(f) => f.to_lowercase().chain(cs).collect(),
            None => "Layer".into(),
        };
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if CS_KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// `base` made unique among `used` (C# names are case-sensitive): `Card`, `Card2`, `Card3`, ….
pub(crate) fn cs_unique(base: &str, used: &mut BTreeSet<String>) -> String {
    let name = (1usize..)
        .map(|k| if k == 1 { base.to_string() } else { format!("{base}{k}") })
        .find(|n| !used.contains(n))
        .unwrap_or_else(|| base.to_string());
    used.insert(name.clone());
    name
}

/// The file name (without extension) of the image or vector asset of a layer: lowercase
/// `[a-z0-9_]`, starting with a letter, at most [`MAX_IDENT`] characters, unique among `used`
/// (`logo`, `logo_2`, `logo_3`, …). A name left with nothing takes `kind`.
pub(crate) fn asset_name(name: &str, kind: &str, used: &mut BTreeSet<String>) -> String {
    let mut base = ascii_words(name).iter().map(|w| w.to_ascii_lowercase()).collect::<Vec<_>>().join("_");
    base = base.chars().take(MAX_IDENT).collect();
    let base = base.trim_end_matches('_').to_string();
    let base = if base.is_empty() {
        let k: String = kind.to_ascii_lowercase().chars().filter(char::is_ascii_alphanumeric).collect();
        if k.is_empty() { "image".into() } else { k }
    } else {
        base
    };
    let base = if base.starts_with(|c: char| c.is_ascii_digit()) { format!("img_{base}") } else { base };
    let name =
        (1usize..).map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") }).find(|n| !used.contains(n)).unwrap_or_else(|| base.clone());
    used.insert(name.clone());
    name
}

/// `Color.FromArgb(0xAA, 0xRR, 0xGG, 0xBB)` (needs `using Windows.UI;`).
pub(crate) fn cs_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    format!("Color.FromArgb(0x{a:02X}, 0x{r:02X}, 0x{g:02X}, 0x{b:02X})")
}

/// A layer name for a comment line: one line, cut.
fn comment_name(name: &str) -> String {
    let one = one_line(name);
    let cut: String = one.chars().take(MAX_COMMENT).collect();
    if cut.chars().count() < one.chars().count() { format!("{cut}…") } else { cut }
}

/// One element of the tree before it is written: comment lines above it, the local it is held
/// in, its constructor, the chain of property calls (a call starting with `//` is a comment
/// line) and its children. Children are added with `Children.Add(x)` or `Child = x`, never with
/// the fluent `.Children(…)` / `.Child(…)`, whose load time doubles with every level of nesting.
struct Element {
    lead: Vec<String>,
    var: String,
    ctor: String,
    calls: Vec<String>,
    /// `Child` (a single child, assigned) or `Children` (added one by one), and the children.
    adds: &'static str,
    kids: Vec<Element>,
}

impl Element {
    fn new(ctor: &str) -> Self {
        Element { lead: Vec::new(), var: String::new(), ctor: ctor.to_string(), calls: Vec::new(), adds: "Children", kids: Vec::new() }
    }

    fn call(&mut self, c: impl Into<String>) {
        self.calls.push(c.into());
    }

    fn note(&mut self, what: &str) {
        self.calls.push(format!("// not mapped: {what}"));
    }

    /// The constructor and the chain at `depth`, as an expression without a trailing newline.
    fn expr(&self, depth: usize) -> String {
        let (pad, inner) = (uno_indent(depth), uno_indent(depth + 1));
        let mut out = format!("{pad}{}", self.ctor);
        for c in &self.calls {
            // A call of several lines is indented relative to its first.
            out.push_str(&format!("\n{inner}{}", c.replace('\n', &format!("\n{inner}"))));
        }
        out
    }

    /// The statements for this element and its subtree at `depth`: its comment lines, the
    /// declaration of its local, then each child's statements followed by the line that adds it.
    fn write(&self, depth: usize, out: &mut String) {
        let (pad, inner) = (uno_indent(depth), uno_indent(depth + 1));
        for l in &self.lead {
            out.push_str(&format!("{pad}{l}\n"));
        }
        out.push_str(&format!("{pad}var {} = {}", self.var, self.ctor));
        // The semicolon ends the last call, not a `// not mapped` comment after it.
        let last = self.calls.iter().rposition(|c| !c.starts_with("//"));
        if last.is_none() {
            out.push(';');
        }
        for (i, c) in self.calls.iter().enumerate() {
            out.push_str(&format!("\n{inner}{}", c.replace('\n', &format!("\n{inner}"))));
            if last == Some(i) {
                out.push(';');
            }
        }
        out.push('\n');
        for k in &self.kids {
            k.write(depth, out);
            if self.adds == "Child" {
                out.push_str(&format!("{pad}{}.Child = {};\n", self.var, k.var));
            } else {
                out.push_str(&format!("{pad}{}.Children.Add({});\n", self.var, k.var));
            }
        }
    }
}

/// Where a child sits in its parent: nowhere in particular (the root), at an offset in a
/// `Canvas`, in the flow of an `AutoLayout` (`z` is its paint order when the parent paints the first
/// child on top), over an `AutoLayout` at an offset that ignores it, or in a cell of a `Grid`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    Flow,
    At(f64, f64),
    Line {
        vertical: bool,
        z: Option<usize>,
    },
    Independent {
        x: f64,
        y: f64,
        z: Option<usize>,
    },
    /// In a cell of a `Grid`: row, column, spans and the alignment in the cell.
    Cell {
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
        align: (CellAlign, CellAlign),
    },
    /// Over a `Grid`, across all its cells, at an offset that ignores it.
    Overlay {
        x: f64,
        y: f64,
        rows: usize,
        columns: usize,
    },
}

/// What a tree walk keeps between layers.
struct Gen<'a> {
    ctx: &'a Ctx<'a>,
    /// Asset names given out, so no two layers share a file.
    assets: BTreeSet<String>,
    /// Frames written as a `Canvas` for lack of auto layout (one warning per view says how many).
    free_frames: usize,
    /// The names of the locals given out.
    names: BTreeSet<String>,
}

/// Uno C# Markup for layer `id` and its subtree: a `UserControl` class whose content is the tree.
pub(crate) fn uno(doc: &Document, id: NodeId) -> String {
    let Some(root) = doc.node(id) else { return String::new() };
    let ctx = Ctx::new(doc, id);
    let mut g = Gen { ctx: &ctx, assets: BTreeSet::new(), free_frames: 0, names: BTreeSet::new() };
    let mut top = match g.layer(root, 0, Place::Flow) {
        Some(e) => e,
        None => {
            let mut e = Element::new("new Border()");
            e.var = "root".into();
            e
        }
    };
    if g.free_frames > 0 {
        // One warning per view, above everything else written for the root layer.
        let (count, verb) = if g.free_frames == 1 {
            ("1 frame has".to_string(), "its children are")
        } else {
            (format!("{} frames have", g.free_frames), "children are")
        };
        top.lead.insert(0, format!("// warning: {count} no auto layout: {verb} positioned absolutely and do not adapt to size."));
    }
    // The statements sit two levels in: namespace member, constructor body.
    let mut body = String::new();
    top.write(2, &mut body);
    body.push_str(&format!("{}Content = {};\n", uno_indent(2), top.var));
    let mut out = String::new();
    let usings: Vec<&str> = [
        ("using Microsoft.UI.Text;", "FontWeights."),
        ("using Windows.Foundation;", "new Rect("),
        ("using Windows.UI;", "Color.FromArgb("),
        ("using Windows.UI.Text;", "FontStyle."),
        ("using Uno.Toolkit.UI;", "new AutoLayout("),
    ]
    .into_iter()
    .filter(|(_, marker)| body.contains(marker))
    .map(|(u, _)| u)
    .collect();
    for u in &usings {
        out.push_str(u);
        out.push('\n');
    }
    if !usings.is_empty() {
        out.push('\n');
    }
    // A view named like a WinUI type (`Button`, `Border`) would hide it in the tree below.
    let mut name = cs_ident(&root.name, true, "Layer");
    if !name.ends_with("View") {
        name.push_str("View");
    }
    out.push_str(&format!("// {}\nnamespace MyApp;\n\n", comment_name(&root.name)));
    out.push_str(&format!("internal sealed partial class {name} : UserControl\n{{\n"));
    out.push_str(&format!("    public {name}()\n    {{\n{body}    }}\n}}\n"));
    out
}

impl Gen<'_> {
    /// A local name from a layer name, unique in the view (`card`, `card2`, …).
    fn fresh(&mut self, name: &str, kind: &str) -> String {
        let base = cs_ident(name, false, kind);
        cs_unique(&base, &mut self.names)
    }

    /// A layer and its subtree at `depth` (levels of nesting), placed in its parent as `at` says.
    /// `None` for a layer that draws nothing.
    fn layer(&mut self, n: &Node, depth: usize, at: Place) -> Option<Element> {
        if matches!(n.kind, NodeKind::Slice) {
            return None;
        }
        // The parent's name comes before its children's.
        let var = self.fresh(&n.name, n.kind.label());
        if self.ctx.too_deep(depth) {
            let mut e = Element::new("new Border()");
            e.lead.push(format!("// {} …", comment_name(&n.name)));
            e.var = var;
            return Some(e);
        }
        let mut e = match &n.kind {
            NodeKind::Text(t) => self.text(n, t, at),
            NodeKind::Ellipse { start, sweep, inner } => self.ellipse(n, (*start, *sweep, *inner), at),
            NodeKind::Rectangle | NodeKind::Group | NodeKind::Section { .. } => self.boxed(n, depth, at),
            k if k.is_frame_like() => self.boxed(n, depth, at),
            _ => self.asset(n, at),
        };
        e.var = var;
        Some(e)
    }

    /// A frame-like layer, group or rectangle. Auto layout is an `AutoLayout` (horizontal,
    /// vertical) or a `Grid` (grid mode); anything else is a `Border` holding a `Canvas` with the
    /// children at their offsets.
    fn boxed(&mut self, n: &Node, depth: usize, at: Place) -> Element {
        match n.layout.as_ref() {
            Some(l) if !matches!(n.kind, NodeKind::Rectangle) => self.auto_layout(n, l, depth, at),
            _ => self.canvas_box(n, depth, at),
        }
    }

    /// A frame with auto layout: an `AutoLayout` (or, in grid mode, a `Grid`) that takes the box
    /// (fill, border, corners) itself. A `Border` pushes its content in by its thickness, so
    /// the padding is written less the stroke weight, as FigCraft's layout ignores strokes (unless
    /// they are included). Where that would be negative the border sits behind the layout in a
    /// `Grid`, as for a frame without auto layout.
    fn auto_layout(&mut self, n: &Node, l: &AutoLayout, depth: usize, at: Place) -> Element {
        let [pt, pr, pb, pl] = l.padding;
        let weights = match (stroke_color(n), n.stroke.sides) {
            (None, _) => [0.0; 4],
            (Some(_), Some(s)) => s,
            (Some(_), None) => [n.stroke.weight; 4],
        };
        let weights = if l.strokes_included { [0.0; 4] } else { weights };
        let pad = [pt - weights[0], pr - weights[1], pb - weights[2], pl - weights[3]];
        let behind = pad.iter().any(|p| *p < 0.0);
        let grid = l.mode == LayoutMode::Grid;
        let mut line = Element::new(if grid { "new Grid()" } else { "new AutoLayout()" });
        let kids = self.ctx.kids(n);
        let vertical = l.mode == LayoutMode::Vertical;
        let mut absolute = false;
        // Grid mode: where FigCraft's own layout puts each child.
        let (cells, rows) =
            if grid { figcraft_layout::grid_place(self.ctx.doc, l, &figcraft_layout::flow_children(self.ctx.doc, n.id)) } else { (Vec::new(), 0) };
        let tracks = GridTracks::new(l, rows);
        let frame_align = (
            match l.main_align {
                MainAlign::Center => CellAlign::Center,
                MainAlign::Max => CellAlign::Max,
                _ => CellAlign::Min,
            },
            match l.cross_align {
                CrossAlign::Center => CellAlign::Center,
                CrossAlign::Max => CellAlign::Max,
                _ => CellAlign::Min,
            },
        );
        for (i, k) in kids.iter().enumerate().take(MAX_CHILDREN) {
            let Some(c) = self.ctx.doc.node(*k) else { continue };
            // "First on top": the paint order descends, the order of the children stays.
            let z = (l.reverse_z && !grid).then_some(kids.len() - i);
            let place = if c.child_layout.absolute {
                absolute = true;
                let [_, _, _, _, x, y] = c.transform.as_coeffs();
                if grid {
                    // Against the grid's padded box, the offsets are the frame's less the padding.
                    Place::Overlay {
                        x: x - if behind { pl } else { pad[3] },
                        y: y - if behind { pt } else { pad[0] },
                        rows: tracks.rows,
                        columns: tracks.columns,
                    }
                } else {
                    Place::Independent { x, y, z }
                }
            } else if grid {
                match cells.iter().find(|cell| cell.id == *k) {
                    Some(cell) => {
                        let (cl, cr) = (cell.column.min(tracks.columns - 1), cell.row.min(tracks.rows - 1));
                        Place::Cell {
                            row: cr,
                            column: cl,
                            row_span: cell.row_span.min(tracks.rows - cr),
                            column_span: cell.column_span.min(tracks.columns - cl),
                            align: (c.child_layout.grid_align_h.unwrap_or(frame_align.0), c.child_layout.grid_align_v.unwrap_or(frame_align.1)),
                        }
                    }
                    None => Place::Flow,
                }
            } else {
                Place::Line { vertical, z }
            };
            line.kids.extend(self.layer(c, depth + 1, place));
        }
        // The Toolkit grows a hugging layout to cover a child that ignores it; FigCraft's does not.
        let (aw, ah) = axes(n, at);
        let force = (absolute && aw == Sizing::Hug, absolute && ah == Sizing::Hug);
        let size = |e: &mut Element| {
            dims(e, n, if force.0 { Sizing::Fixed } else { aw }, if force.1 { Sizing::Fixed } else { ah });
            if force.0 || force.1 {
                e.note("hug size fixed: an absolute child would enlarge it");
            }
        };
        let mut outer;
        if behind {
            // The layout inside keeps the whole padding; the border is drawn behind it.
            let mut border = Element::new("new Border()");
            self.paint_box(&mut border, n, false, at);
            if grid {
                grid_calls(&mut line, l, &tracks, [pt, pr, pb, pl]);
            } else {
                layout_calls(&mut line, l, [pt, pr, pb, pl]);
            }
            outer = Element::new("new Grid()");
            place(&mut outer, n, at);
            size(&mut outer);
            border.var = self.fresh(&format!("{} border", n.name), "border");
            line.var = self.fresh(&format!("{} layout", n.name), "layout");
            outer.kids = vec![border, line];
            common(&mut outer, n);
        } else {
            outer = line;
            place(&mut outer, n, at);
            size(&mut outer);
            self.paint_box(&mut outer, n, false, at);
            if grid {
                grid_calls(&mut outer, l, &tracks, pad);
            } else {
                layout_calls(&mut outer, l, pad);
            }
            common(&mut outer, n);
        }
        if n.clip && n.corners.radii.iter().all(|r| *r <= 0.0) && !n.children.is_empty() {
            outer.call(format!(".Clip(new RectangleGeometry().Rect(new Rect(0, 0, {}, {})))", num(n.width), num(n.height)));
        }
        outer
    }

    /// A frame-like layer, group or rectangle without a layout of its own to map: a `Border`
    /// with its fill, stroke and corners holding a `Canvas` with the children at their offsets.
    /// With a stroke and children the border sits behind the canvas in a `Grid`, since a `Border`
    /// pushes its content in by its thickness.
    fn canvas_box(&mut self, n: &Node, depth: usize, at: Place) -> Element {
        let mut border = Element::new("new Border()");
        let sized = !matches!(n.kind, NodeKind::Group | NodeKind::Section { .. });
        self.paint_box(&mut border, n, sized, at);
        let mut canvas = Element::new("new Canvas()");
        for k in self.ctx.kids(n).iter().take(MAX_CHILDREN) {
            let Some(c) = self.ctx.doc.node(*k) else { continue };
            let [_, _, _, _, x, y] = c.transform.as_coeffs();
            canvas.kids.extend(self.layer(c, depth + 1, Place::At(x, y)));
        }
        let has_kids = !canvas.kids.is_empty();
        if has_kids {
            canvas.var = self.fresh(&format!("{} canvas", n.name), "canvas");
            if n.layout.is_none() {
                self.free_frames += 1;
            }
        }
        let mut outer;
        if has_kids && stroke_color(n).is_some() {
            outer = Element::new("new Grid()");
            place(&mut outer, n, at);
            size(&mut outer, n, at);
            border.calls.retain(|c| !c.starts_with(".Width(") && !c.starts_with(".Height("));
            border.var = self.fresh(&format!("{} border", n.name), "border");
            outer.kids = vec![border, canvas];
            common(&mut outer, n);
        } else {
            outer = border;
            place(&mut outer, n, at);
            if has_kids {
                outer.adds = "Child";
                outer.kids = vec![canvas];
            }
            common(&mut outer, n);
        }
        if n.clip && has_kids && n.corners.radii.iter().all(|r| *r <= 0.0) {
            outer.call(format!(".Clip(new RectangleGeometry().Rect(new Rect(0, 0, {}, {})))", num(n.width), num(n.height)));
        }
        outer
    }

    /// The fill, stroke and corners of a `Border`.
    fn paint_box(&mut self, e: &mut Element, n: &Node, sized: bool, at: Place) {
        if sized {
            size(e, n, at);
        }
        let visible: Vec<_> = n.fills.iter().filter(|p| p.visible).collect();
        match visible.first().map(|p| &p.kind) {
            Some(PaintKind::Image(img)) => {
                let name = asset_name(&n.name, "image", &mut self.assets);
                let stretch = match img.scale_mode {
                    ImageScaleMode::Fit => "Uniform",
                    ImageScaleMode::Fill | ImageScaleMode::Crop | ImageScaleMode::Tile => "UniformToFill",
                };
                e.call(format!(".Background(new ImageBrush().ImageSource(\"{ASSETS}/{name}.png\").Stretch(Stretch.{stretch}))"));
                match img.scale_mode {
                    ImageScaleMode::Crop => e.note("image crop (drawn as fill)"),
                    ImageScaleMode::Tile => e.note("tiled image (drawn as fill)"),
                    ImageScaleMode::Fill | ImageScaleMode::Fit => {}
                }
            }
            Some(PaintKind::Gradient(_)) => {
                if let Some(c) = first_fill(n) {
                    e.call(format!(".Background({})", cs_color(c)));
                }
                e.note("gradient fill (its first stop colour is used)");
            }
            Some(PaintKind::Pattern(_)) => e.note("pattern fill"),
            Some(PaintKind::Solid { .. }) => {
                if let Some(c) = first_fill(n) {
                    e.call(format!(".Background({})", cs_color(c)));
                }
            }
            None => {}
        }
        if visible.len() > 1 {
            e.note(&format!("{} more fills (only the first is drawn)", visible.len() - 1));
        }
        if let Some(c) = stroke_color(n) {
            e.call(format!(".BorderBrush({})", cs_color(c)));
            match n.stroke.sides {
                // Per-side weights are top, right, bottom, left; `Thickness` takes left first.
                Some([t, r, b, l]) => e.call(format!(".BorderThickness(new Thickness({}, {}, {}, {}))", num(l), num(t), num(r), num(b))),
                None => e.call(format!(".BorderThickness({})", num(n.stroke.weight))),
            }
            if n.stroke.align != StrokeAlign::Inside {
                e.note("stroke outside or centred (a border is drawn inside)");
            }
        }
        let [tl, tr, br, bl] = n.corners.radii;
        if n.corners.uniform().is_some() {
            if tl > 0.0 {
                e.call(format!(".CornerRadius({})", num(tl)));
            }
        } else {
            e.call(format!(".CornerRadius(new CornerRadius({}, {}, {}, {}))", num(tl), num(tr), num(br), num(bl)));
        }
        if n.corners.smoothing > 0.0 && n.corners.radii.iter().any(|r| *r > 0.0) {
            e.note("corner smoothing");
        }
    }

    /// An ellipse: `Ellipse` with `Fill` and a centred `Stroke`.
    fn ellipse(&mut self, n: &Node, (start, sweep, inner): (f64, f64, f64), at: Place) -> Element {
        let mut e = Element::new("new Ellipse()");
        place(&mut e, n, at);
        size(&mut e, n, at);
        if let Some(c) = first_fill(n) {
            e.call(format!(".Fill({})", cs_color(c)));
        }
        if let Some(c) = stroke_color(n) {
            e.call(format!(".Stroke({})", cs_color(c)));
            e.call(format!(".StrokeThickness({})", num(n.stroke.weight)));
            if n.stroke.align != StrokeAlign::Center {
                e.note("stroke inside or outside (an ellipse stroke is centred)");
            }
        }
        if start != 0.0 || (sweep - std::f64::consts::TAU).abs() > 1e-6 || inner > 0.0 {
            e.note("arc or ring (drawn as a full ellipse)");
        }
        common(&mut e, n);
        e
    }

    /// A vector, shape or other layer without a counterpart: the image the app is to hold.
    fn asset(&mut self, n: &Node, at: Place) -> Element {
        let name = asset_name(&n.name, n.kind.label(), &mut self.assets);
        let mut e = Element::new("new Image()");
        e.lead.push(format!("// {} \"{}\": export it as an SVG to Assets/{name}.svg", n.kind.label(), comment_name(&n.name)));
        place(&mut e, n, at);
        e.call(format!(".Source(\"{ASSETS}/{name}.svg\")"));
        size(&mut e, n, at);
        e.call(".Stretch(Stretch.Uniform)");
        common(&mut e, n);
        e
    }

    /// Text: a `TextBlock`, in a `Grid` of the box's height when it is aligned to the middle or
    /// bottom of a fixed box.
    fn text(&mut self, n: &Node, t: &TextData, at: Place) -> Element {
        let fixed = matches!(t.auto_resize, TextAutoResize::None | TextAutoResize::Truncate);
        let framed = fixed && t.align_v != TextAlignV::Top;
        let mut tb = text_block(n, t, !framed, line_axes(n, at));
        if !framed {
            place(&mut tb, n, at);
            common(&mut tb, n);
            return tb;
        }
        tb.call(format!(".VerticalAlignment(VerticalAlignment.{})", if t.align_v == TextAlignV::Middle { "Center" } else { "Bottom" }));
        let mut g = Element::new("new Grid()");
        place(&mut g, n, at);
        size(&mut g, n, at);
        tb.var = self.fresh(&format!("{} text", n.name), "text");
        g.kids = vec![tb];
        common(&mut g, n);
        g
    }
}

/// The calls that address a child in its parent, first in its chain: the `Canvas` offsets, or
/// the attached `AutoLayout` call (fill on an axis, or independent of the layout), the paint
/// order and the size limits.
fn place(e: &mut Element, n: &Node, at: Place) {
    let mut lead: Vec<String> = Vec::new();
    let z = |z: Option<usize>, lead: &mut Vec<String>| {
        if let Some(z) = z {
            lead.push(format!(".Canvas(zIndex: {z})"));
        }
    };
    match at {
        Place::Flow => {}
        // The offsets read as the element's address.
        Place::At(x, y) => lead.push(format!(".Canvas(left: {}, top: {})", num(x), num(y))),
        Place::Line { vertical, z: order } => {
            let cl = &n.child_layout;
            let (primary, counter) = if vertical { (cl.sizing_v, cl.sizing_h) } else { (cl.sizing_h, cl.sizing_v) };
            let mut args: Vec<&str> = Vec::new();
            if primary == Sizing::Fill {
                args.push("primaryAlignment: AutoLayoutPrimaryAlignment.Stretch");
            }
            if counter == Sizing::Fill {
                args.push("counterAlignment: AutoLayoutAlignment.Stretch");
            }
            if !args.is_empty() {
                lead.push(format!(".AutoLayout({})", args.join(", ")));
            }
            z(order, &mut lead);
            for (name, v) in [("MinWidth", cl.min_w), ("MaxWidth", cl.max_w), ("MinHeight", cl.min_h), ("MaxHeight", cl.max_h)] {
                if let Some(v) = v.filter(|v| v.is_finite()) {
                    lead.push(format!(".{name}({})", num(v)));
                }
            }
        }
        Place::Cell { row, column, row_span, column_span, align } => {
            let mut args = format!("row: {row}, column: {column}");
            if row_span > 1 {
                args.push_str(&format!(", rowSpan: {row_span}"));
            }
            if column_span > 1 {
                args.push_str(&format!(", columnSpan: {column_span}"));
            }
            lead.push(format!(".Grid({args})"));
            cell_alignment(n, align, &mut lead);
            let cl = &n.child_layout;
            for (name, v) in [("MinWidth", cl.min_w), ("MaxWidth", cl.max_w), ("MinHeight", cl.min_h), ("MaxHeight", cl.max_h)] {
                if let Some(v) = v.filter(|v| v.is_finite()) {
                    lead.push(format!(".{name}({})", num(v)));
                }
            }
        }
        Place::Overlay { x, y, rows, columns } => {
            lead.push(format!(".Grid(row: 0, column: 0, rowSpan: {rows}, columnSpan: {columns})"));
            lead.push(".HorizontalAlignment(HorizontalAlignment.Left)".into());
            lead.push(".VerticalAlignment(VerticalAlignment.Top)".into());
            lead.push(format!(".Margin({}, {}, 0, 0)", num(x), num(y)));
            if n.constraints.horizontal != HConstraint::Left || n.constraints.vertical != VConstraint::Top {
                e.note("constraints (the child stays at its top-left offset)");
            }
        }
        Place::Independent { x, y, z: order } => {
            lead.push(".AutoLayout(isIndependentLayout: true)".into());
            z(order, &mut lead);
            lead.push(".HorizontalAlignment(HorizontalAlignment.Left)".into());
            lead.push(".VerticalAlignment(VerticalAlignment.Top)".into());
            lead.push(format!(".Margin({}, {}, 0, 0)", num(x), num(y)));
            if n.constraints.horizontal != HConstraint::Left || n.constraints.vertical != VConstraint::Top {
                e.note("constraints (the child stays at its top-left offset)");
            }
        }
    }
    e.calls.splice(0..0, lead);
}

/// The sizes of a cell child's alignment: a filling axis stretches (the cell's default), any
/// other aligns in its cell.
fn cell_alignment(n: &Node, align: (CellAlign, CellAlign), lead: &mut Vec<String>) {
    let cl = &n.child_layout;
    if cl.sizing_h != Sizing::Fill {
        let a = match align.0 {
            CellAlign::Min => "Left",
            CellAlign::Center => "Center",
            CellAlign::Max => "Right",
        };
        lead.push(format!(".HorizontalAlignment(HorizontalAlignment.{a})"));
    }
    if cl.sizing_v != Sizing::Fill {
        let a = match align.1 {
            CellAlign::Min => "Top",
            CellAlign::Center => "Center",
            CellAlign::Max => "Bottom",
        };
        lead.push(format!(".VerticalAlignment(VerticalAlignment.{a})"));
    }
}

/// How a child sizes on each axis in its parent: fixed everywhere but in an auto layout flow.
fn axes(n: &Node, at: Place) -> (Sizing, Sizing) {
    match at {
        Place::Line { .. } | Place::Cell { .. } => (n.child_layout.sizing_h, n.child_layout.sizing_v),
        _ => (Sizing::Fixed, Sizing::Fixed),
    }
}

/// [`axes`] for a text in a flow (outside one a text sizes itself from its resize mode).
fn line_axes(n: &Node, at: Place) -> Option<(Sizing, Sizing)> {
    matches!(at, Place::Line { .. } | Place::Cell { .. }).then(|| axes(n, at))
}

/// The tracks a grid frame is written with: how many of each, and what had to be cut or rounded.
struct GridTracks {
    columns: usize,
    rows: usize,
    cut: bool,
}

/// Most tracks written per axis.
const MAX_WRITTEN_TRACKS: usize = 64;

impl GridTracks {
    fn new(l: &AutoLayout, rows: usize) -> Self {
        let (columns, rows) = ((l.grid_columns as usize).max(1), rows.max(1));
        GridTracks {
            columns: columns.min(MAX_WRITTEN_TRACKS),
            rows: rows.min(MAX_WRITTEN_TRACKS),
            cut: columns > MAX_WRITTEN_TRACKS || rows > MAX_WRITTEN_TRACKS,
        }
    }
}

/// A `ColumnDefinitions` / `RowDefinitions` string for `count` tracks: fixed ones in pixels, hugging
/// ones `Auto`, filling ones as stars. Uno ignores fractional stars, so the shares of an axis are
/// scaled by the smallest factor up to 100 that makes them whole; the flag says they could not be
/// and were rounded. A track missing from `specs` fills one share.
fn track_string(specs: &[figcraft_doc::GridTrack], count: usize) -> (String, bool) {
    let spec = |i: usize| specs.get(i).copied().unwrap_or_default();
    let share = |t: figcraft_doc::GridTrack| if t.value.is_finite() { t.value.clamp(0.0, 1e6) } else { 0.0 };
    let shares: Vec<f64> = (0..count).map(&spec).filter(|t| t.sizing == Sizing::Fill).map(share).collect();
    let whole = |k: f64| shares.iter().all(|f| (f * k - (f * k).round()).abs() < 1e-6);
    let factor = (1..=100).map(f64::from).find(|k| whole(*k));
    let k = factor.unwrap_or(1.0);
    let out: Vec<String> = (0..count)
        .map(|i| {
            let t = spec(i);
            match t.sizing {
                Sizing::Fixed => num(share(t)),
                Sizing::Hug => "Auto".to_string(),
                Sizing::Fill => format!("{}*", num((share(t) * k).round().max(if factor.is_some() { 0.0 } else { 1.0 }))),
            }
        })
        .collect();
    (out.join(","), factor.is_none())
}

/// The calls a `Grid` takes for its tracks, spacing and padding. In grid mode FigCraft's `gap` is
/// the space between columns and `cross_gap` between rows (`layout_grid` reads them as `gx`, `gy`),
/// and the main and cross alignment are the children's default alignment in their cell.
fn grid_calls(e: &mut Element, l: &AutoLayout, t: &GridTracks, pad: [f64; 4]) {
    let (columns, rounded_c) = track_string(&l.column_tracks, t.columns);
    let (rows, rounded_r) = track_string(&l.row_tracks, t.rows);
    e.call(format!(".ColumnDefinitions({})", cs_literal(&columns)));
    e.call(format!(".RowDefinitions({})", cs_literal(&rows)));
    if l.gap > 0.0 {
        e.call(format!(".ColumnSpacing({})", num(l.gap)));
    }
    if l.cross_gap > 0.0 {
        e.call(format!(".RowSpacing({})", num(l.cross_gap)));
    }
    padding_call(e, pad);
    if rounded_c || rounded_r {
        e.note("fractional grid track sizes (rounded to whole stars)");
    }
    if t.cut {
        e.note(&format!("more than {MAX_WRITTEN_TRACKS} grid tracks (only {MAX_WRITTEN_TRACKS} per axis are written)"));
    }
    if l.reverse_z {
        e.note("first child on top (a grid paints its children in order)");
    }
}

/// `Padding` from top, right, bottom, left: one number when equal, nothing when zero.
fn padding_call(e: &mut Element, pad: [f64; 4]) {
    let [t, r, b, lf] = pad;
    if pad.iter().any(|p| *p != 0.0) {
        if t == r && r == b && b == lf {
            e.call(format!(".Padding({})", num(t)));
        } else {
            e.call(format!(".Padding({}, {}, {}, {})", num(lf), num(t), num(r), num(b)));
        }
    }
}

/// The calls an `AutoLayout` takes for its orientation, gap, padding (`pad` is top, right,
/// bottom, left as written) and alignment.
fn layout_calls(e: &mut Element, l: &AutoLayout, pad: [f64; 4]) {
    e.call(format!(".Orientation(Orientation.{})", if l.mode == LayoutMode::Vertical { "Vertical" } else { "Horizontal" }));
    // The space distributions make the gaps themselves.
    if l.gap != 0.0 && !l.main_align.is_auto_gap() {
        e.call(format!(".Spacing({})", num(l.gap)));
    }
    padding_call(e, pad);
    if l.wrap {
        e.note("wrap (FlexPanel needs Uno.Toolkit 10; one line written)");
    }
    match l.main_align {
        MainAlign::Min => {}
        MainAlign::Center => e.call(".PrimaryAxisAlignment(AutoLayoutAlignment.Center)"),
        MainAlign::Max => e.call(".PrimaryAxisAlignment(AutoLayoutAlignment.End)"),
        MainAlign::SpaceBetween => e.call(".Justify(AutoLayoutJustify.SpaceBetween)"),
        MainAlign::SpaceAround | MainAlign::SpaceEvenly => {
            e.call(".Justify(AutoLayoutJustify.SpaceBetween)");
            e.note("space around/evenly exported as space between");
        }
    }
    match l.cross_align {
        CrossAlign::Min => {}
        CrossAlign::Center => e.call(".CounterAxisAlignment(AutoLayoutAlignment.Center)"),
        CrossAlign::Max => e.call(".CounterAxisAlignment(AutoLayoutAlignment.End)"),
        CrossAlign::Baseline => e.note("baseline alignment (children are aligned to the start)"),
    }
}

/// The layer's width and height on the axes where it is fixed (a hugging axis takes its size
/// from the content, a filling one from the parent).
fn size(e: &mut Element, n: &Node, at: Place) {
    let (w, h) = axes(n, at);
    dims(e, n, w, h);
}

fn dims(e: &mut Element, n: &Node, w: Sizing, h: Sizing) {
    if w == Sizing::Fixed {
        e.call(format!(".Width({})", num(n.width)));
    }
    if h == Sizing::Fixed {
        e.call(format!(".Height({})", num(n.height)));
    }
}

/// What every layer shares: opacity, rotation, and the notes for what has no counterpart.
fn common(e: &mut Element, n: &Node) {
    if n.opacity < 1.0 {
        e.call(format!(".Opacity({})", num(f64::from(n.opacity))));
    }
    // Rotation turns counter-clockwise about the top-left corner (the offsets given); a WinUI
    // `RotateTransform` turns clockwise about the element's own origin.
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 0.01 && rot.is_finite() {
        e.call(format!(".RenderTransform(new RotateTransform().Angle({}))", num(-rot)));
    }
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) {
        e.note(&format!("blend mode {}", n.blend.css()));
    }
    for fx in n.effects.iter().filter(|f| f.visible) {
        e.note(match fx.kind {
            EffectKind::DropShadow(_) => "drop shadow (needs Uno.Toolkit ShadowContainer)",
            EffectKind::InnerShadow(_) => "inner shadow (needs Uno.Toolkit ShadowContainer)",
            EffectKind::LayerBlur(_) => "layer blur",
            EffectKind::BackgroundBlur(_) => "background blur",
            EffectKind::Noise(_) => "noise",
            EffectKind::Texture(_) => "texture",
            EffectKind::Glass(_) => "glass",
        });
    }
    if n.is_mask {
        e.note("mask");
    }
}

/// `FontWeights.SemiBold` for a weight in hundreds (the nearest).
fn cs_weight(w: u16) -> &'static str {
    match w {
        0..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Normal",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        851..=950 => "Black",
        _ => "ExtraBlack",
    }
}

/// The colour of the first visible solid or gradient paint (a gradient's first stop).
fn paints_color(paints: &[figcraft_color::Paint]) -> Option<Color> {
    paints.iter().filter(|p| p.visible).find_map(|p| match &p.kind {
        PaintKind::Solid { color } => Some(color.with_alpha(color.a * p.opacity)),
        PaintKind::Gradient(g) => g.sorted_stops().first().map(|s| s.color),
        PaintKind::Image(_) | PaintKind::Pattern(_) => None,
    })
}

/// `characters` as a string expression in `case`: the case is applied when the code runs, so the
/// string keeps the original text (and the app can translate it).
fn text_expr(characters: &str, case: TextCase) -> String {
    let lit = cs_literal(characters);
    match case {
        TextCase::Original | TextCase::SmallCaps => lit,
        TextCase::Upper => format!("{lit}.ToUpperInvariant()"),
        TextCase::Lower => format!("{lit}.ToLowerInvariant()"),
        TextCase::Title => format!("System.Globalization.CultureInfo.CurrentCulture.TextInfo.ToTitleCase({lit})"),
    }
}

/// The calls a `TextBlock` or `Run` takes for the parts of `s` that differ from `base` (all of
/// them without a base). `fill` is the colour the text is drawn in.
fn style_calls(e: &mut Element, s: &TextStyle, fill: Option<Color>, base: Option<(&TextStyle, Option<Color>)>) {
    if !s.font_family.is_empty() && base.is_none_or(|(b, _)| b.font_family != s.font_family) {
        e.call(format!(".FontFamily({})", cs_literal(&s.font_family)));
    }
    if base.is_none_or(|(b, _)| b.font_size != s.font_size) {
        e.call(format!(".FontSize({})", num(s.font_size)));
    }
    if (base.is_none() && s.weight() != 400) || base.is_some_and(|(b, _)| b.weight() != s.weight()) {
        e.call(format!(".FontWeight(FontWeights.{})", cs_weight(s.weight())));
    }
    if (base.is_none() && s.italic()) || base.is_some_and(|(b, _)| b.italic() != s.italic()) {
        e.call(format!(".FontStyle(FontStyle.{})", if s.italic() { "Italic" } else { "Normal" }));
    }
    if let Some(c) = fill
        && base.is_none_or(|(_, f)| f != fill)
    {
        e.call(format!(".Foreground({})", cs_color(c)));
    }
    if base.is_none() {
        match s.line_height {
            LineHeight::Pixels(v) if v > 0.0 && v.is_finite() => e.call(format!(".LineHeight({})", num(v))),
            LineHeight::Percent(p) if p > 0.0 && p.is_finite() && s.font_size > 0.0 => {
                e.call(format!(".LineHeight({})", num(p / 100.0 * s.font_size)))
            }
            _ => {}
        }
    }
    // `CharacterSpacing` is in thousandths of an em.
    let spacing = |x: &TextStyle| match x.letter_spacing {
        LetterSpacing::Pixels(v) if v.is_finite() && x.font_size > 0.0 => (v / x.font_size * 1000.0).round(),
        LetterSpacing::Percent(p) if p.is_finite() => (p * 10.0).round(),
        _ => 0.0,
    };
    if base.is_none_or(|(b, _)| spacing(b) != spacing(s)) && (base.is_some() || spacing(s) != 0.0) {
        e.call(format!(".CharacterSpacing({})", num(spacing(s))));
    }
    if base.is_none_or(|(b, _)| b.decoration != s.decoration) {
        match s.decoration {
            TextDecoration::None if base.is_some() => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.None)"),
            TextDecoration::None => {}
            TextDecoration::Underline => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.Underline)"),
            TextDecoration::Strikethrough => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.Strikethrough)"),
        }
    }
}

/// A `TextBlock` for a text layer. `with_height` sets the box's height when the text has a fixed
/// box (a text in a `Grid` of that height leaves it to the grid).
fn text_block(n: &Node, t: &TextData, with_height: bool, axes: Option<(Sizing, Sizing)>) -> Element {
    let mut e = Element::new("new TextBlock()");
    let s = &t.style;
    let fill = first_fill(n);
    // Text in one style is a `Text`; style runs make `Inlines` of `Run`s.
    let runs: Vec<_> = t.runs.iter().filter(|r| r.start < r.end).collect();
    let chars: Vec<char> = t.characters.chars().take(MAX_LITERAL * 10).collect();
    if runs.is_empty() || runs.len() > MAX_RUNS {
        e.call(format!(".Text({})", text_expr(&t.characters, s.case)));
        if !runs.is_empty() {
            e.note("style runs (too many)");
        }
    } else {
        let mut pieces: Vec<String> = Vec::new();
        let mut at = 0;
        let mut run = |from: usize, to: usize, style: &TextStyle, fills: Option<Color>, plain: bool| {
            let part: String = chars.get(from..to.min(chars.len())).unwrap_or_default().iter().collect();
            if part.is_empty() {
                return;
            }
            let mut r = Element::new("new Run()");
            r.call(format!(".Text({})", text_expr(&part, style.case)));
            if !plain {
                style_calls(&mut r, style, fills, Some((s, fill)));
            }
            pieces.push(r.expr(1));
        };
        for r in &runs {
            let (start, end) = (r.start.max(at), r.end.min(chars.len()));
            if start >= end {
                continue;
            }
            run(at, start, s, fill, true);
            let run_fill = r.style.fills.as_deref().and_then(paints_color).or(fill);
            run(start, end, &r.style, run_fill, false);
            at = end;
        }
        run(at, chars.len(), s, fill, true);
        e.call(format!(".Inlines(\n{})", pieces.join(",\n")));
    }
    style_calls(&mut e, s, fill, None);
    match t.align_h {
        TextAlignH::Left => {}
        TextAlignH::Center => e.call(".TextAlignment(TextAlignment.Center)"),
        TextAlignH::Right => e.call(".TextAlignment(TextAlignment.Right)"),
        TextAlignH::Justified => e.call(".TextAlignment(TextAlignment.Justify)"),
    }
    // In a flow the text takes its width from the sizing: hugging is one line, filling wraps at
    // the parent's width, fixed is the box's width.
    let one_line = t.auto_resize == TextAutoResize::WidthAndHeight;
    // Outside a flow the resize mode says it.
    let (width, tall) = axes.unwrap_or((if one_line { Sizing::Hug } else { Sizing::Fixed }, Sizing::Fixed));
    match width {
        Sizing::Hug => e.call(".TextWrapping(TextWrapping.NoWrap)"),
        Sizing::Fill => e.call(".TextWrapping(TextWrapping.Wrap)"),
        Sizing::Fixed => {
            e.call(format!(".Width({})", num(n.width)));
            e.call(format!(".TextWrapping(TextWrapping.{})", if one_line { "NoWrap" } else { "Wrap" }));
        }
    }
    if tall == Sizing::Fixed && with_height && matches!(t.auto_resize, TextAutoResize::None | TextAutoResize::Truncate) {
        e.call(format!(".Height({})", num(n.height)));
    }
    if t.auto_resize == TextAutoResize::Truncate || (t.max_lines > 0 && t.auto_resize != TextAutoResize::WidthAndHeight) {
        e.call(".TextTrimming(TextTrimming.CharacterEllipsis)");
        if t.max_lines > 0 {
            e.call(format!(".MaxLines({})", t.max_lines));
        }
    }
    if s.case == TextCase::SmallCaps {
        e.note("small caps");
    }
    e
}
