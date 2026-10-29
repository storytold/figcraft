//! FigCraft's icon set, drawn in code from simple primitives on a 16×16 grid. Original artwork
//! (MIT OR Apache-2.0): no third-party icons are used anywhere.
//!
//! The grid is pixel-exact at 16 px: 1-unit strokes sit on half coordinates (pixel centres) and
//! fills on whole ones. Larger boxes scale by whole steps and smaller ones are hinted: straight
//! strokes snap to the pixel grid so they stay one sharp pixel wide at any size.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

macro_rules! icons {
    ($($(#[$m:meta])* $name:ident,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($(#[$m])* $name,)*
        }

        impl Icon {
            /// Every icon, in declaration order (the icon gallery, tests).
            pub const ALL: &[Icon] = &[$(Icon::$name,)*];
        }
    };
}

mod draw;

pub use draw::{paint, paint_outlines};

icons! {
    Move,
    Scale,
    Hand,
    Frame,
    Section,
    Slice,
    Rectangle,
    Line,
    Arrow,
    Ellipse,
    Polygon,
    Star,
    Image,
    Pen,
    Pencil,
    Text,
    TextOnPath,
    Comment,
    Actions,
    Chevron,
    ChevronRight,
    ChevronLeft,
    ChevronUp,
    Plus,
    Minus,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Component,
    Instance,
    ComponentSet,
    Group,
    Vector,
    Bend,
    PaintBucket,
    Lasso,
    Cut,
    Boolean,
    Page,
    Check,
    Close,
    /// Window buttons: minimise, maximise, restore.
    WinMinimize,
    WinMaximize,
    WinRestore,
    Search,
    Grid,
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignBottom,
    DistributeH,
    DistributeV,
    Rotate,
    FlipH,
    FlipV,
    CornerRadius,
    /// Individual corners: the rounded corner each field sets.
    CornerTopLeft,
    CornerTopRight,
    CornerBottomRight,
    CornerBottomLeft,
    Angle,
    IndependentCorners,
    /// Individual padding: a frame with one inward tick per side, each set alone.
    IndependentPadding,
    LinkOn,
    LinkOff,
    LayoutH,
    LayoutV,
    LayoutWrap,
    LayoutGrid,
    AutoLayout,
    ClipContent,
    Effects,
    Styles,
    Menu,
    Play,
    Home,
    Bell,
    Panels,
    Help,
    Dots,
    Mask,
    TextAlignLeft,
    TextAlignCenter,
    TextAlignRight,
    TextAlignJustify,
    TextTop,
    TextMiddle,
    TextBottom,
    Eyedropper,
    Code,
    Assets,
    /// Libraries: an open book.
    Library,
    Layers,
    Opacity,
    Blend,
    Export,
    StrokeIcon,
    Fill,
    /// Gradient kinds: a swatch outline holding a fade of that kind.
    GradientLinear,
    GradientRadial,
    GradientAngular,
    GradientDiamond,
    Sun,
    Moon,
    Undo,
    Redo,
    Union,
    Subtract,
    Intersect,
    Exclude,
    Hug,
    FillContainer,
    Fixed,
    PaddingH,
    PaddingV,
    /// Individual padding sides: a frame with the controlled side filled.
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Gap,
    Reset,
    Detach,
    Variable,
    Clock,
    Eraser,
    /// Variable width: a stroke swelling in the middle with a width handle above and below.
    VariableWidth,
    /// Shape builder: merge regions (and the tool).
    ShapeBuilder,
    /// Shape builder: remove regions.
    ShapeBuilderRemove,
    Swap,
    Link,
    Prototype,
    /// The prototype preview pane: a small window playing a frame.
    Preview,
    Section2,
    Logo,
    /// Ignore auto layout: a layer pinned at an offset inside its frame.
    AbsolutePosition,
    /// A component slot: a dashed frame with a plus, a place that takes any content.
    Slot,
    /// Edit settings: two sliders.
    Adjust,
    /// Show as a list.
    ViewList,
    /// Show as a grid of swatches.
    ViewGrid,
    /// Add an emoji reaction: a smiling face.
    Reaction,
    /// Drag to reorder: two columns of three dots.
    DragHandle,
    /// Arc start: a circle with a radius to the dot where the arc begins.
    ArcStart,
    /// Arc sweep: a circle with a wedge cut out of it.
    ArcSweep,
    /// Arc ratio: a ring.
    ArcRatio,
    /// A colour outside the screen's gamut: a triangle with an exclamation mark.
    GamutWarning,
    /// Online: a globe (a circle with its equator and a meridian).
    Online,
    /// In a live session others get into: a point sending waves to both sides.
    LiveShared,
    /// A file of yours on a server that only you get into: a point inside a closed ring.
    LivePrivate,
    /// A design file: a pen nib leaning to the left, its grip filled.
    PenNib,
    /// A board file (and the sticky note tool): a note with its bottom corner folded up.
    Board,
    /// Board templates: a board laid out in blocks, a note and lines of text. A table: a grid.
    /// The table bar's rows and columns: a strip with a plus or a minus beyond its end.
    Templates,
    Table,
    AddRow,
    RemoveRow,
    AddColumn,
    RemoveColumn,
    /// Board tools: a chisel-tipped marker; a broad highlighter over its band; a connector
    /// between two boxes; shapes (a square behind a circle); a stamp (a badge with a check).
    Marker,
    Highlighter,
    Connector,
    /// A curved connector: the connector's two boxes joined by a quarter bend instead of a corner.
    CurvedConnector,
    Shapes,
    Stamp,
    /// Running a board session: a stopwatch (the timer), two bars (pause) and a ballot going
    /// into a box (voting).
    Timer,
    Pause,
    Vote,
    /// A measurement: a dimension line with arrowheads between two end ticks.
    Measure,
}

/// Points along a circular arc from angle `a0` to `a1` (radians, y down: increasing is clockwise).
fn arc_pts(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> impl Iterator<Item = (f32, f32)> {
    let n = ((a1 - a0).abs() * r * 1.5).ceil().clamp(6.0, 48.0) as usize;
    (0..=n).map(move |i| {
        let t = a0 + (a1 - a0) * i as f32 / n as f32;
        (cx + r * t.cos(), cy + r * t.sin())
    })
}

/// A capsule (stadium) outline from `a` to `b` with radius `r`.
fn capsule(a: (f32, f32), b: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let th = (b.1 - a.1).atan2(b.0 - a.0);
    arc_pts(b.0, b.1, r, th - FRAC_PI_2, th + FRAC_PI_2).chain(arc_pts(a.0, a.1, r, th + FRAC_PI_2, th + 3.0 * FRAC_PI_2)).collect()
}

/// Triangles of a simple polygon (ear clipping); empty when it can't be split.
fn triangulate(v: &[Pos2]) -> Vec<u32> {
    let n = v.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n).map(|i| v[i].x * v[(i + 1) % n].y - v[(i + 1) % n].x * v[i].y).sum();
    let mut idx: Vec<usize> = if area >= 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b - a).x * (c - b).y - (b - a).y * (c - b).x;
    let mut out = Vec::with_capacity((n - 2) * 3);
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (v[ia], v[ib], v[ic]);
            cross(a, b, c) > 0.0
                && !idx
                    .iter()
                    .any(|&k| k != ia && k != ib && k != ic && cross(a, b, v[k]) >= 0.0 && cross(b, c, v[k]) >= 0.0 && cross(c, a, v[k]) >= 0.0)
        });
        let Some(i) = ear else { return Vec::new() };
        out.extend([idx[(i + m - 1) % m] as u32, idx[i] as u32, idx[(i + 1) % m] as u32]);
        idx.remove(i);
    }
    out.extend(idx.iter().map(|&i| i as u32));
    out
}

/// Allocate a square icon button; returns the response. `active` paints the selected look.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, active: bool, tooltip: &str) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click());
    let bg = if active {
        t.selected_row
    } else if resp.is_pointer_button_down_on() {
        t.pressed
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(5), bg);
    let col = if active { t.accent_text } else { t.icon };
    // The glyph at its native 16 px in the usual buttons, smaller in compact ones.
    let glyph = if size >= 22.0 { 16.0 } else { 12.0 };
    paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(glyph)), icon, col);
    if tooltip.is_empty() {
        return resp;
    }
    let resp = crate::a11y::Tip::tip(resp, tooltip);
    // Shown pressed (an open option): a pressed toggle to screen readers.
    if active {
        crate::a11y::toggle(&resp, tooltip, true);
    }
    resp
}

/// The mark of a file of `kind` on tabs and file rows: a pen nib for design files, a sticky note
/// for boards.
pub fn file_mark(kind: figcraft_doc::FileKind) -> Icon {
    match kind {
        figcraft_doc::FileKind::Design => Icon::PenNib,
        figcraft_doc::FileKind::Board => Icon::Board,
    }
}

/// Every icon at the sizes the UI uses, labelled: for reviewing the set (`ui.set {iconGallery}`).
pub fn gallery(ctx: &egui::Context) {
    let t = crate::theme::Tokens::get(ctx);
    egui::Area::new(egui::Id::unique("icon_gallery")).order(egui::Order::Foreground).fixed_pos(Pos2::ZERO).show(ctx, |ui| {
        let screen = ctx.content_rect();
        let (r, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
        ui.painter().rect_filled(r, 0.0, t.panel);
        let cols = ((screen.width() - 20.0) / 118.0).floor().max(1.0) as usize;
        for (i, icon) in Icon::ALL.iter().enumerate() {
            let x = 10.0 + (i % cols) as f32 * 118.0;
            let y = 10.0 + (i / cols) as f32 * 70.0;
            for (dx, size) in [(0.0, 12.0), (16.0, 16.0), (40.0, 24.0), (72.0, 32.0)] {
                paint(ui.painter(), Rect::from_min_size(pos2(x + dx, y), Vec2::splat(size)), *icon, t.icon);
            }
            ui.painter().text(pos2(x, y + 40.0), egui::Align2::LEFT_TOP, format!("{icon:?}"), egui::FontId::proportional(10.0), t.text);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon paints at every size the UI uses (and odd positions) without panicking.
    #[test]
    fn every_icon_paints_at_every_size() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for &icon in Icon::ALL {
                for size in [8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 32.0, 48.0] {
                    paint(ui.painter(), Rect::from_min_size(pos2(3.3, 7.7), Vec2::splat(size)), icon, Color32::WHITE);
                }
            }
        });
        out.textures_delta.clear();
        assert!(out.shapes.len() > Icon::ALL.len() * 10);
    }

    /// The gradient kinds' icons show a fade: a mesh whose vertices run from opaque to clear.
    #[test]
    fn gradient_icons_fade() {
        let ctx = egui::Context::default();
        for icon in [Icon::GradientLinear, Icon::GradientRadial, Icon::GradientAngular, Icon::GradientDiamond] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(ui.painter(), Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0)), icon, Color32::WHITE);
            });
            out.textures_delta.clear();
            let alphas: Vec<u8> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Mesh(m) => Some(m.vertices.iter().map(|v| v.color.a()).collect::<Vec<_>>()),
                    _ => None,
                })
                .flatten()
                .collect();
            assert!(alphas.contains(&255) && alphas.contains(&0), "{icon:?} fades from opaque to clear: {alphas:?}");
        }
    }

    #[test]
    fn concave_polygons_triangulate() {
        let arrow = [pos2(4.0, 2.0), pos2(4.0, 13.6), pos2(6.9, 10.9), pos2(8.8, 14.8), pos2(10.6, 14.0), pos2(8.7, 10.2), pos2(12.6, 10.2)];
        assert_eq!(triangulate(&arrow).len(), (arrow.len() - 2) * 3);
        let rev: Vec<Pos2> = arrow.iter().rev().copied().collect();
        assert_eq!(triangulate(&rev).len(), (arrow.len() - 2) * 3);
        assert!(triangulate(&arrow[..2]).is_empty());
    }

    /// The points painted inside `area`, relative to the top left of what's painted there (in
    /// quarter points), so the same icon gives the same marks wherever its field puts it.
    fn marks(shapes: &[egui::epaint::ClippedShape], area: Rect) -> Vec<(i32, i32)> {
        fn walk(shape: &Shape, out: &mut Vec<Pos2>) {
            match shape {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                Shape::LineSegment { points, .. } => out.extend(points),
                Shape::Path(p) => out.extend(&p.points),
                Shape::Circle(c) => out.push(c.center),
                _ => {}
            }
        }
        let mut pts = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut pts));
        pts.retain(|p| area.contains(*p));
        let x0 = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let y0 = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let mut out: Vec<(i32, i32)> = pts.into_iter().map(|p| (((p.x - x0) * 4.0).round() as i32, ((p.y - y0) * 4.0).round() as i32)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The Design panel's Rotation field shows an icon of its own, not Corner radius's (#256).
    #[test]
    fn rotation_and_corner_radius_fields_show_different_icons() {
        let mut s = figcraft_engine::Session::new();
        let r = s.execute("shape.rectangle", &serde_json::json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap()["id"].clone();
        s.execute("select.set", &serde_json::json!({"ids": [r]})).unwrap();
        let mut u = crate::layers_drag_tests::Ui::new(s);
        u.frame(vec![]);
        let shapes = u.frame_output(vec![]).shapes;
        let lead = |name: &str| u.ctx.read_response(egui::Id::unique(name).with("lead")).map(|r| r.rect).unwrap_or_else(|| panic!("{name} is shown"));
        let (rotation, radius) = (marks(&shapes, lead("pos_rot")), marks(&shapes, lead("ap_rad")));
        assert!(!rotation.is_empty() && !radius.is_empty(), "both draw an icon");
        assert_ne!(rotation, radius);
    }

    /// The shapes `icon` paints in a 32-point box at the origin: two points per grid unit.
    fn shapes_at_32(icon: Icon) -> Vec<Shape> {
        let ctx = egui::Context::default();
        let mut out =
            ctx.run_ui(egui::RawInput::default(), |ui| paint(ui.painter(), Rect::from_min_size(Pos2::ZERO, Vec2::splat(32.0)), icon, Color32::WHITE));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// Frame and Text stay a grid unit inside the box, so their long hinted strokes don't
    /// outweigh the outlines beside them in the toolbar (#737).
    #[test]
    fn frame_and_text_marks_are_inset() {
        for icon in [Icon::Frame, Icon::Text] {
            let pts: Vec<Pos2> = shapes_at_32(icon)
                .iter()
                .flat_map(|s| match s {
                    Shape::LineSegment { points, .. } => points.to_vec(),
                    other => panic!("{icon:?} draws straight strokes only: {other:?}"),
                })
                .collect();
            assert!(!pts.is_empty());
            for p in pts {
                // Grid units 3..13, two points each.
                assert!((6.0..=26.0).contains(&p.x) && (6.0..=26.0).contains(&p.y), "{icon:?} reaches {p:?}");
            }
        }
    }

    /// Slice is one knife (#739): an outlined blade and a solid handle that covers the middle of
    /// the blade's base, so the two read as one mark rather than separate pieces.
    #[test]
    fn slice_is_one_connected_knife() {
        let shapes = shapes_at_32(Icon::Slice);
        let paths: Vec<_> = shapes
            .iter()
            .map(|s| match s {
                Shape::Path(p) => p,
                other => panic!("Slice draws paths only: {other:?}"),
            })
            .collect();
        let blade = paths.iter().find(|p| p.closed && p.fill == Color32::TRANSPARENT).expect("an outlined blade");
        let handle = paths.iter().find(|p| p.fill != Color32::TRANSPARENT).expect("a solid handle");
        assert_eq!((paths.len(), blade.points.len()), (2, 3), "a blade and a handle, nothing else");
        // The blade's base runs between its second and third points; the handle is convex.
        let base = blade.points[1] + (blade.points[2] - blade.points[1]) / 2.0;
        let h = &handle.points;
        let side = |i: usize| {
            let (a, b) = (h[i], h[(i + 1) % h.len()]);
            (b - a).x * (base - a).y - (b - a).y * (base - a).x
        };
        let inside = (0..h.len()).all(|i| side(i) >= 0.0) || (0..h.len()).all(|i| side(i) <= 0.0);
        assert!(inside, "the handle covers the blade's base at {base:?}");
    }

    /// Each corner's icon shows its own corner: no two of the four draw alike.
    #[test]
    fn corner_icons_differ() {
        let drawn: Vec<String> = [Icon::CornerTopLeft, Icon::CornerTopRight, Icon::CornerBottomRight, Icon::CornerBottomLeft]
            .iter()
            .map(|&icon| format!("{:?}", shapes_at_32(icon)))
            .collect();
        for (i, a) in drawn.iter().enumerate() {
            for b in &drawn[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
