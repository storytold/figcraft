//! `.fig` export tests: generated documents round trip through our own importer (layer tree,
//! geometry within tolerance, paints, text, auto layout, components, prototypes); the written file
//! decodes with the generic decoder and lists every layer after its parent; the encoder agrees
//! with the decoder; junk documents never panic.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::color::{BlendMode, Color, ColorStop, Gradient, GradientKind, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::{Affine, Point};
use figcraft_doc::proto::{Action, Direction, Easing, Flow, Interaction, KeyMods, Overflow, OverlayPosition, ScrollPosition, Transition, Trigger};
use figcraft_doc::*;
use serde_json::Value;

use super::kiwi::{Kind, Kv, Schema, Type, encode};
use super::*;

/// Variables, shared styles and instance sizes (uses this module's generators and comparisons).
#[path = "assets_tests.rs"]
mod assets;

/// A cheap deterministic generator.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    /// A value in `lo..hi` in steps of 1/8 (exact in 32-bit floats).
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        let steps = ((hi - lo) * 8.0).max(1.0) as u64;
        lo + self.below(steps) as f64 / 8.0
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

fn rgb(r: &mut Lcg) -> Color {
    Color::new(r.below(9) as f32 / 8.0, r.below(9) as f32 / 8.0, r.below(9) as f32 / 8.0, 1.0)
}

const BLENDS: [BlendMode; 4] = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay];

fn paint(r: &mut Lcg, image: &str) -> Paint {
    let kind = match r.below(4) {
        0 | 1 => PaintKind::Solid { color: rgb(r) },
        2 => {
            let kind = r.pick(&[GradientKind::Linear, GradientKind::Radial, GradientKind::Angular, GradientKind::Diamond]);
            let mut g = Gradient::default_for(kind, rgb(r));
            g.from = (r.f(0.0, 1.0), r.f(0.0, 1.0));
            g.to = (g.from.0 + r.f(0.25, 1.0), g.from.1 - r.f(0.0, 0.5));
            // Across the main axis (handles on one line can't be written).
            let k = r.pick(&[0.5, 1.0]);
            g.width = (g.from.0 - (g.to.1 - g.from.1) * k, g.from.1 + (g.to.0 - g.from.0) * k);
            g.stops = vec![ColorStop { position: 0.0, color: rgb(r) }, ColorStop { position: 0.5, color: rgb(r).with_alpha(0.5) }];
            PaintKind::Gradient(g)
        }
        _ => {
            let mut p =
                Paint::image(image.to_string(), r.pick(&[ImageScaleMode::Fill, ImageScaleMode::Fit, ImageScaleMode::Tile, ImageScaleMode::Crop]));
            if let PaintKind::Image(img) = &mut p.kind {
                img.rotation = r.below(4) as i32;
                img.scale = r.f(0.5, 2.0);
                if img.scale_mode == ImageScaleMode::Crop {
                    img.crop = Some([r.f(0.5, 1.0), 0.0, 0.0, r.f(0.5, 1.0), r.f(0.0, 0.25), r.f(0.0, 0.25)]);
                }
            }
            p.kind
        }
    };
    Paint { kind, opacity: r.pick(&[1.0, 0.5, 0.25]), visible: r.chance(85), blend: r.pick(&BLENDS) }
}

fn effect(r: &mut Lcg) -> Effect {
    let shadow = |r: &mut Lcg| Shadow {
        color: rgb(r).with_alpha(0.25),
        offset: (r.f(-4.0, 4.0), r.f(-4.0, 4.0)),
        radius: r.f(0.0, 8.0),
        spread: r.f(-2.0, 2.0),
        blend: r.pick(&BLENDS),
        show_behind: r.chance(30),
    };
    let blur = |r: &mut Lcg| {
        let mut b = Blur { radius: r.f(1.0, 10.0), ..Blur::default() };
        if r.chance(40) {
            b.blur_type = BlurType::Progressive;
            b.start_radius = r.f(0.0, 2.0);
            b.start = (0.5, 0.0);
            b.end = (0.5, 1.0);
        }
        b
    };
    let kind = match r.below(6) {
        0 => EffectKind::DropShadow(shadow(r)),
        1 => EffectKind::InnerShadow(shadow(r)),
        2 => EffectKind::LayerBlur(blur(r)),
        3 => EffectKind::BackgroundBlur(blur(r)),
        4 => EffectKind::Noise(Noise {
            mode: r.pick(&[NoiseMode::Mono, NoiseMode::Duo, NoiseMode::Multi]),
            size: r.f(0.5, 4.0),
            size_y: r.chance(50).then_some(2.5),
            density: r.f(0.0, 1.0),
            color: rgb(r),
            secondary: rgb(r),
            opacity: 0.5,
            blend: BlendMode::Normal,
        }),
        _ => EffectKind::Texture(Texture { size: r.f(1.0, 4.0), size_y: None, radius: r.f(0.0, 4.0), clip_to_shape: r.chance(50) }),
    };
    Effect { kind, visible: r.chance(90) }
}

/// Random looks: paints, strokes, effects, opacity, blend, corners, constraints, a rotation.
fn style(r: &mut Lcg, n: &mut Node, image: &str) {
    n.fills = (0..r.below(3)).map(|_| paint(r, image)).collect();
    n.strokes = (0..r.below(2)).map(|_| Paint::solid(rgb(r))).collect();
    n.stroke.weight = r.f(0.0, 4.0);
    n.stroke.align = r.pick(&[StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside]);
    n.stroke.join = r.pick(&[StrokeJoin::Miter, StrokeJoin::Bevel, StrokeJoin::Round]);
    let cap = r.pick(&[StrokeCap::None, StrokeCap::Round, StrokeCap::Square]);
    (n.stroke.cap_start, n.stroke.cap_end) = (cap, cap);
    n.stroke.miter_limit = r.pick(&[4.0, 2.0, 10.0]);
    if r.chance(20) {
        n.stroke.dashes = vec![r.f(1.0, 6.0), r.f(1.0, 6.0)];
    }
    if r.chance(15) && matches!(n.kind, NodeKind::Rectangle | NodeKind::Frame) {
        n.stroke.sides = Some([1.0, 2.0, 0.0, 3.0]);
    }
    n.effects = (0..r.below(3)).map(|_| effect(r)).collect();
    n.opacity = r.pick(&[1.0, 0.75, 0.5]);
    n.visible = r.chance(90);
    n.locked = r.chance(10);
    if !n.kind.is_container() {
        n.blend = r.pick(&BLENDS);
    }
    if n.kind.has_corner_radius() {
        n.corners.radii = if n.kind.has_independent_corners() && r.chance(40) { [1.0, 2.0, 3.0, 4.0] } else { [r.f(0.0, 8.0); 4] };
        n.corners.smoothing = r.pick(&[0.0, 0.6]);
    }
    n.constraints = Constraints {
        horizontal: r.pick(&[HConstraint::Left, HConstraint::Right, HConstraint::LeftRight, HConstraint::Center, HConstraint::Scale]),
        vertical: r.pick(&[VConstraint::Top, VConstraint::Bottom, VConstraint::TopBottom, VConstraint::Center, VConstraint::Scale]),
    };
    if r.chance(30) {
        let a = r.f(-1.5, 1.5);
        n.transform *= Affine::rotate(a);
    }
    if r.chance(15) {
        n.exports =
            vec![ExportSetting { size: ExportSize::Scale { value: 2.0 }, suffix: "@2x".into(), format: ExportFormat::Png, ..Default::default() }];
    }
}

fn text_data(r: &mut Lcg) -> TextData {
    const BITS: [&str; 8] = ["Hello", " ", "wörld", "😀", "\n", "日本", "x", "Fig"];
    let chars: String = (0..1 + r.below(8)).map(|_| r.pick(&BITS)).collect();
    let mut td = TextData::new(chars);
    td.style.font_size = r.f(8.0, 32.0);
    td.style.font_style = r.pick(&["Regular", "Bold"]).into();
    td.style.letter_spacing = r.pick(&[LetterSpacing::Percent(0.0), LetterSpacing::Percent(5.0), LetterSpacing::Pixels(1.5)]);
    td.style.line_height = r.pick(&[LineHeight::Auto, LineHeight::Pixels(20.0), LineHeight::Percent(150.0), LineHeight::Percent(100.0)]);
    td.style.case = r.pick(&[TextCase::Original, TextCase::Upper, TextCase::Title]);
    td.style.decoration = r.pick(&[TextDecoration::None, TextDecoration::Underline, TextDecoration::Strikethrough]);
    if r.chance(15) {
        td.style.link = Some("https://example.com/a".into());
    }
    if r.chance(15) {
        td.style.fills = Some(vec![Paint::solid(rgb(r))]);
    }
    td.auto_resize = r.pick(&[TextAutoResize::WidthAndHeight, TextAutoResize::Height, TextAutoResize::None, TextAutoResize::Truncate]);
    if td.auto_resize == TextAutoResize::Truncate {
        td.max_lines = 2;
    }
    td.align_h = r.pick(&[TextAlignH::Left, TextAlignH::Center, TextAlignH::Right, TextAlignH::Justified]);
    td.align_v = r.pick(&[TextAlignV::Top, TextAlignV::Middle, TextAlignV::Bottom]);
    td.paragraph_spacing = r.f(0.0, 8.0);
    let n = td.char_count();
    for _ in 0..r.below(3) {
        let (a, b) = (r.below(n as u64 + 1) as usize, r.below(n as u64 + 1) as usize);
        let (size, bold, fill, ls) = (r.f(8.0, 40.0), r.chance(50), r.chance(30).then(|| vec![Paint::solid(rgb(r))]), r.f(0.0, 10.0));
        td.restyle(a.min(b), a.max(b), |s| {
            s.font_size = size;
            if bold {
                s.font_style = "Bold".into();
            }
            if fill.is_some() {
                s.fills = fill.clone();
            }
            s.letter_spacing = LetterSpacing::Percent(ls);
            s.decoration = TextDecoration::Underline;
        });
    }
    if r.chance(30) {
        let list = r.pick(&[ListStyle::Bulleted, ListStyle::Numbered]);
        td.set_para_lists(0, 1, |p| p.list = list);
    }
    td
}

fn network(r: &mut Lcg, w: f64, h: f64) -> VectorNetwork {
    let pts: Vec<Point> = (0..3 + r.below(4)).map(|_| Point::new(r.f(0.0, w), r.f(0.0, h))).collect();
    let mut net = VectorNetwork::from_points(&pts, r.chance(60));
    if let Some(s) = net.segments.first_mut() {
        s.ta = (r.f(0.0, 8.0), 0.0);
        s.tb = (0.0, r.f(-8.0, 0.0));
    }
    if let Some(v) = net.vertices.get_mut(1) {
        v.radius = r.pick(&[0.0, 2.0]);
        v.mirror = r.pick(&[None, Some(HandleMirroring::Angle), Some(HandleMirroring::AngleAndLength), Some(HandleMirroring::None)]);
    }
    if let Some(reg) = net.regions.first_mut() {
        reg.winding = r.pick(&[WindingRule::NonZero, WindingRule::EvenOdd]);
    }
    net
}

struct Gen {
    r: Lcg,
    doc: Document,
    image: String,
    /// Top-level frames (prototype destinations) and every layer made.
    frames: Vec<NodeId>,
    layers: Vec<NodeId>,
}

impl Gen {
    fn node(&mut self, parent: NodeId, kind: NodeKind, name: &str) -> NodeId {
        let (x, y, w, h) = (self.r.f(-50.0, 400.0), self.r.f(-50.0, 400.0), self.r.f(1.0, 200.0), self.r.f(1.0, 120.0));
        let id = self.doc.create(parent, kind, name, x, y, w, h).unwrap();
        let image = self.image.clone();
        let n = self.doc.node_mut(id).unwrap();
        style(&mut self.r, n, &image);
        self.layers.push(id);
        id
    }

    /// A random layer under `parent` (with children for containers), `depth` levels down.
    fn layer(&mut self, parent: NodeId, depth: usize) -> NodeId {
        let parent_layout = self.doc.node(parent).and_then(|p| p.layout.clone());
        let on_page = self.doc.node(parent).is_some_and(Node::is_page);
        let kind = if depth >= 3 { 3 + self.r.below(8) } else { self.r.below(12) };
        let id = match kind {
            0 => {
                let id = self.node(parent, NodeKind::Frame, "Frame");
                if self.r.chance(60) {
                    let mode = self.r.pick(&[LayoutMode::Horizontal, LayoutMode::Vertical, LayoutMode::Grid]);
                    let l = AutoLayout {
                        mode,
                        wrap: mode != LayoutMode::Grid && self.r.chance(30),
                        gap: self.r.f(0.0, 16.0),
                        cross_gap: self.r.f(0.0, 8.0),
                        padding: [1.0, 2.0, self.r.f(0.0, 8.0), 4.0],
                        main_align: self.r.pick(&[MainAlign::Min, MainAlign::Center, MainAlign::Max, MainAlign::SpaceBetween]),
                        cross_align: self.r.pick(&[CrossAlign::Min, CrossAlign::Center, CrossAlign::Max, CrossAlign::Baseline]),
                        grid_columns: 1 + self.r.below(3) as u32,
                        grid_rows: 1 + self.r.below(3) as u32,
                        strokes_included: self.r.chance(30),
                        reverse_z: self.r.chance(20),
                        ..AutoLayout::default()
                    };
                    let hug = (self.r.chance(30), self.r.chance(30));
                    let n = self.doc.node_mut(id).unwrap();
                    n.layout = Some(l);
                    n.clip = self.r.chance(50);
                    if hug.0 {
                        n.child_layout.sizing_h = Sizing::Hug;
                    }
                    if hug.1 {
                        n.child_layout.sizing_v = Sizing::Hug;
                    }
                }
                if self.r.chance(20) {
                    self.doc.node_mut(id).unwrap().grids = vec![LayoutGrid::columns(), LayoutGrid::rows()];
                }
                if self.r.chance(15) {
                    let n = self.doc.node_mut(id).unwrap();
                    n.overflow = Overflow::Vertical;
                }
                for _ in 0..self.r.below(4) {
                    self.layer(id, depth + 1);
                }
                if on_page {
                    self.frames.push(id);
                }
                id
            }
            1 | 2 if depth < 3 => {
                let kind = if kind == 1 {
                    NodeKind::Group
                } else {
                    NodeKind::BooleanOp { op: self.r.pick(&[BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect, BoolOp::Exclude]) }
                };
                let id = self.node(parent, kind, "Group");
                // Groups paint nothing of their own.
                self.doc.node_mut(id).unwrap().fills.clear();
                for _ in 0..1 + self.r.below(3) {
                    let leaf = if self.r.chance(50) {
                        NodeKind::Rectangle
                    } else {
                        NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }
                    };
                    self.node(id, leaf, "Operand");
                }
                self.doc.fit_to_children(id).unwrap();
                id
            }
            3 => self.node(parent, NodeKind::Rectangle, "Rect"),
            4 => {
                let (start, sweep, inner) = if self.r.chance(50) { (0.0, std::f64::consts::TAU, 0.0) } else { (0.5, 3.0, 0.25) };
                self.node(parent, NodeKind::Ellipse { start, sweep, inner }, "Ellipse")
            }
            5 => {
                let count = 3 + self.r.below(6) as u32;
                self.node(parent, NodeKind::Polygon { count }, "Polygon")
            }
            6 => {
                let (count, inner_ratio) = (3 + self.r.below(6) as u32, self.r.f(0.25, 0.875));
                self.node(parent, NodeKind::Star { count, inner_ratio }, "Star")
            }
            7 => {
                let id = self.node(parent, NodeKind::Line, "Line");
                self.doc.node_mut(id).unwrap().height = 0.0;
                id
            }
            8 => {
                let id = self.node(parent, NodeKind::Vector { network: VectorNetwork::default() }, "Vector");
                let n = self.doc.node_mut(id).unwrap();
                let net = network(&mut self.r, n.width, n.height);
                n.kind = NodeKind::Vector { network: net };
                if self.r.chance(40) {
                    (n.stroke.cap_start, n.stroke.cap_end) = (StrokeCap::None, StrokeCap::LineArrow);
                }
                id
            }
            9 => {
                let td = text_data(&mut self.r);
                self.node(parent, NodeKind::Text(td), "Text")
            }
            10 if on_page => {
                let id = self.doc.alloc_id();
                let mut n = Node::new_section(id, "Section", 300.0, 200.0);
                n.transform = Affine::translate((self.r.f(0.0, 500.0), 600.0));
                self.doc.insert(n, parent, None).unwrap();
                for _ in 0..self.r.below(3) {
                    self.layer(id, depth + 1);
                }
                id
            }
            _ => self.node(parent, NodeKind::Slice, "Slice"),
        };
        if let Some(pl) = parent_layout {
            let fill_ok = !matches!(self.doc.node(id).map(|n| &n.kind), Some(NodeKind::Group | NodeKind::BooleanOp { .. }));
            let (fh, fv, abs) = (self.r.chance(30) && fill_ok, self.r.chance(30) && fill_ok, self.r.chance(10));
            let (col, row) = (self.r.below(u64::from(pl.grid_columns)) as u32, self.r.below(u64::from(pl.grid_rows)) as u32);
            let min = self.r.chance(20).then_some(10.0);
            let n = self.doc.node_mut(id).unwrap();
            if fh {
                n.child_layout.sizing_h = Sizing::Fill;
            }
            if fv {
                n.child_layout.sizing_v = Sizing::Fill;
            }
            n.child_layout.absolute = abs;
            n.child_layout.min_w = min;
            if pl.mode == LayoutMode::Grid {
                n.child_layout.grid_column = Some(col);
                n.child_layout.grid_row = Some(row);
                n.child_layout.grid_column_span = 1;
            }
        }
        id
    }

    /// A copy of `src`'s subtree under `parent`, each layer mirroring its original (`source`).
    fn mirror(&mut self, src: NodeId, parent: NodeId) -> NodeId {
        let mut n = self.doc.node(src).unwrap().clone();
        let kids = std::mem::take(&mut n.children);
        n.id = self.doc.alloc_id();
        n.source = Some(src);
        let id = self.doc.insert(n, parent, None).unwrap();
        for k in kids {
            self.mirror(k, id);
        }
        id
    }

    /// An instance of `main` under `parent`, its layers mirroring the main's.
    fn instance(&mut self, main: NodeId, parent: NodeId, props: BTreeMap<String, Value>) -> NodeId {
        let m = self.doc.node(main).unwrap().clone();
        let id = self.doc.alloc_id();
        let mut n = m.clone();
        (n.id, n.kind, n.name, n.parent, n.source) = (id, NodeKind::Instance { component: main, props }, "Instance".into(), None, None);
        n.children.clear();
        n.transform = Affine::translate((self.r.f(0.0, 400.0), self.r.f(400.0, 600.0)));
        self.doc.insert(n, parent, None).unwrap();
        for k in m.children {
            self.mirror(k, id);
        }
        id
    }

    /// A component with a boolean and a text property, a component set, and instances of both
    /// (with property values and overrides), and one nested two levels deep.
    fn components(&mut self, page: NodeId) {
        let comp = self.node(page, NodeKind::Component { description: "A button".into(), props: BTreeMap::new() }, "Button");
        let icon = self.node(comp, NodeKind::Rectangle, "Icon");
        let label = self.node(comp, NodeKind::Text(TextData::new("Label")), "Label");
        {
            let n = self.doc.node_mut(comp).unwrap();
            if let NodeKind::Component { props, .. } = &mut n.kind {
                props.insert("Show".into(), PropDef::Boolean { default: true });
                props.insert("Label".into(), PropDef::Text { default: "Label".into() });
            }
        }
        let fx = effect(&mut self.r);
        let n = self.doc.node_mut(icon).unwrap();
        n.prop_refs.insert("visible".into(), "Show".into());
        n.effects = vec![fx];
        self.doc.node_mut(label).unwrap().prop_refs.insert("characters".into(), "Label".into());
        let props = BTreeMap::from([("Show".to_string(), Value::Bool(false)), ("Label".to_string(), Value::String("Hi".into()))]);
        let inst = self.instance(comp, page, props);
        // Apply the property values and a fill override, as the engine would.
        let kids = self.doc.node(inst).unwrap().children.clone();
        for k in kids {
            let fill = Paint::solid(rgb(&mut self.r));
            let n = self.doc.node_mut(k).unwrap();
            match &mut n.kind {
                NodeKind::Text(td) => {
                    td.characters = "Hi".into();
                    td.runs.clear();
                    n.overrides.insert("characters".into());
                }
                _ => {
                    // Its main's effect cleared: an override that removes what the main has.
                    n.visible = false;
                    n.fills = vec![fill];
                    n.effects.clear();
                    n.overrides.extend(["visible".to_string(), "fills".to_string(), "effects".to_string()]);
                }
            }
        }
        // A card holding an instance of the button, and an instance of the card overriding a layer of
        // the nested button.
        let card = self.node(page, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, "Card");
        // A grid whose second cell holds the nested button.
        self.doc.node_mut(card).unwrap().layout = Some(AutoLayout { mode: LayoutMode::Grid, grid_columns: 2, grid_rows: 1, ..AutoLayout::default() });
        let nested = self.instance(comp, card, BTreeMap::new());
        self.doc.node_mut(nested).unwrap().child_layout.grid_column = Some(1);
        let outer = self.instance(card, page, BTreeMap::new());
        let deep = self.doc.node(outer).unwrap().children.first().and_then(|c| self.doc.node(*c)).and_then(|c| c.children.first().copied());
        if let Some(d) = deep {
            let n = self.doc.node_mut(d).unwrap();
            n.opacity = 0.25;
            n.overrides.insert("opacity".into());
        }
        // A component set with two variants and an instance of the second.
        let set = self.node(page, NodeKind::ComponentSet { description: "States".into() }, "Toggle");
        let mut variants = Vec::new();
        for name in ["State=Off", "State=On"] {
            let v = self.node(set, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, name);
            self.node(v, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Knob");
            variants.push(v);
        }
        if let Some(on) = variants.last() {
            self.instance(*on, page, BTreeMap::new());
        }
        self.frames.extend(variants);
    }

    /// Prototype interactions between random layers and frames.
    fn interactions(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        let layers = self.layers.clone();
        for id in layers {
            // Not on component layers: their instances, made already, would miss them.
            let in_component = self
                .doc
                .ancestors(id)
                .iter()
                .chain([&id])
                .any(|a| matches!(self.doc.node(*a).map(|n| &n.kind), Some(NodeKind::Component { .. } | NodeKind::ComponentSet { .. })));
            if !self.r.chance(25) || in_component || self.doc.node(id).is_some_and(|n| n.source.is_some()) {
                continue;
            }
            let dest = self.r.pick(&self.frames);
            let trigger = match self.r.below(6) {
                0 => Trigger::OnClick,
                1 => Trigger::OnHover,
                2 => Trigger::AfterDelay { ms: 10 * self.r.below(300) as u32 },
                3 => Trigger::KeyPress { key: KeyMods { shift: true, ..KeyMods::default() }.combo("K") },
                4 => Trigger::KeyPress { key: "Gamepad A".into() },
                _ => [Trigger::MouseEnter, Trigger::MouseLeave, Trigger::MouseDown, Trigger::MouseUp, Trigger::OnPress, Trigger::OnDrag]
                    [self.r.below(6) as usize]
                    .clone(),
            };
            let action = match self.r.below(8) {
                0 | 1 => Action::Navigate { dest, preserve_scroll: self.r.chance(50) },
                2 => Action::OpenOverlay {
                    dest,
                    // One setting per overlay (its frame carries it).
                    position: [OverlayPosition::Center, OverlayPosition::TopLeft, OverlayPosition::BottomRight, OverlayPosition::Manual]
                        [(dest.0 % 4) as usize],
                    close_on_outside: true,
                    background: Some(Color::new(0.0, 0.0, 0.0, 0.5)),
                    offset: None,
                },
                3 => Action::SwapOverlay { dest },
                4 => Action::ChangeTo { dest },
                5 => Action::Back,
                6 => Action::CloseOverlay,
                _ => Action::OpenUrl { url: "https://example.com".into() },
            };
            let d = self.r.pick(&[Direction::Left, Direction::Right, Direction::Top, Direction::Bottom]);
            let transition = self.r.pick(&[
                Transition::Instant,
                Transition::Dissolve,
                Transition::SmartAnimate,
                Transition::MoveIn { direction: d },
                Transition::MoveOut { direction: d },
                Transition::Push { direction: d },
                Transition::SlideIn { direction: d },
                Transition::SlideOut { direction: d },
            ]);
            let easing = self.r.pick(&[
                Easing::Linear,
                Easing::EaseIn,
                Easing::EaseOut,
                Easing::EaseInOut,
                Easing::EaseInBack,
                Easing::EaseOutBack,
                Easing::EaseInOutBack,
                Easing::Gentle,
                Easing::CustomBezier { x1: 0.25, y1: 0.125, x2: 0.5, y2: 1.0 },
            ]);
            let it =
                Interaction { trigger, actions: vec![action], transition, duration_ms: 10 * (1 + self.r.below(100) as u32), easing, delay_ms: 0 };
            let fixed = self.r.chance(20);
            let n = self.doc.node_mut(id).unwrap();
            n.interactions.push(it);
            if fixed {
                n.scroll_position = ScrollPosition::Fixed;
            }
        }
    }
}

/// A random document: one or two pages of random layers, components and instances, flows and
/// interactions.
fn generate(seed: u64) -> Document {
    let mut doc = Document::new(format!("Generated {seed}"));
    let (image, _, _) = crate::add_image(&mut doc, super::tests::png()).unwrap();
    let mut g = Gen { r: Lcg(seed.wrapping_mul(0x9E37_79B9).wrapping_add(17)), doc, image, frames: Vec::new(), layers: Vec::new() };
    let mut pages = vec![g.doc.first_page().unwrap()];
    if g.r.chance(40) {
        pages.push(g.doc.add_page("Second"));
    }
    for (i, page) in pages.iter().enumerate() {
        let bg = rgb(&mut g.r);
        if let Some(NodeKind::Page { background, .. }) = g.doc.node_mut(*page).map(|n| &mut n.kind) {
            *background = bg;
        }
        for _ in 0..1 + g.r.below(5) {
            g.layer(*page, 0);
        }
        if i == 0 {
            g.components(*page);
        }
        let start = g.doc.node(*page).unwrap().children.iter().copied().find(|c| g.frames.contains(c));
        if let (Some(start), Some(NodeKind::Page { flows, .. })) = (start, g.doc.node_mut(*page).map(|n| &mut n.kind)) {
            flows.push(Flow { name: "Main flow".into(), start, description: String::new() });
        }
    }
    g.interactions();
    g.doc
}

// ---- comparing documents ----

/// What a round trip can't bring back as written, made the same on both sides: ids become
/// positions in the layer order; instance layers lose their links (the engine relinks them);
/// values the format holds another way are converted (solid alpha into the paint's opacity,
/// letter spacing in percent into pixels, list styles spelled out per paragraph).
fn normalize(n: &mut Node, ix: &dyn Fn(NodeId) -> NodeId) {
    n.id = ix(n.id);
    n.parent = n.parent.map(ix);
    n.children = n.children.iter().map(|c| ix(*c)).collect();
    n.source = None;
    n.overrides.clear();
    n.constrain_proportions = false;
    // A text's resize mode is its sizing (the editor keeps them together; generated documents
    // don't): auto width hugs both axes, auto height its height.
    if let NodeKind::Text(t) = &n.kind {
        let cl = &mut n.child_layout;
        let hug = |s: &mut Sizing| *s = if *s == Sizing::Fixed { Sizing::Hug } else { *s };
        match t.auto_resize {
            TextAutoResize::WidthAndHeight => [&mut cl.sizing_h, &mut cl.sizing_v].into_iter().for_each(hug),
            TextAutoResize::Height => hug(&mut cl.sizing_v),
            _ => {}
        }
    }
    let fold = |ps: &mut Vec<Paint>| {
        for p in ps.iter_mut() {
            if let PaintKind::Solid { color } = &mut p.kind {
                p.opacity *= color.a;
                color.a = 1.0;
            }
            if p.blend == BlendMode::PassThrough {
                p.blend = BlendMode::Normal;
            }
        }
    };
    fold(&mut n.fills);
    fold(&mut n.strokes);
    for e in &mut n.effects {
        match &mut e.kind {
            EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) if s.blend == BlendMode::PassThrough => s.blend = BlendMode::Normal,
            // A grain as tall as it is wide has no height of its own.
            EffectKind::Noise(Noise { size, size_y, .. }) | EffectKind::Texture(Texture { size, size_y, .. }) if *size_y == Some(*size) => {
                *size_y = None
            }
            _ => {}
        }
    }
    if let Some(l) = &mut n.layout {
        if matches!(l.main_align, MainAlign::SpaceAround | MainAlign::SpaceEvenly) {
            l.main_align = MainAlign::SpaceBetween;
        }
        // Track counts only matter (and are only written) for grids.
        if l.mode != LayoutMode::Grid {
            let d = AutoLayout::default();
            (l.grid_columns, l.grid_rows) = (d.grid_columns, d.grid_rows);
        }
    }
    n.child_layout.grid_align_h = None;
    n.child_layout.grid_align_v = None;
    if n.stroke.cap_start == StrokeCap::ReversedTriangle {
        n.stroke.cap_start = StrokeCap::TriangleArrow;
    }
    if n.stroke.cap_end == StrokeCap::ReversedTriangle {
        n.stroke.cap_end = StrokeCap::TriangleArrow;
    }
    let mut ends = true;
    match &mut n.kind {
        NodeKind::Instance { component, props } => {
            *component = ix(*component);
            for v in props.values_mut() {
                if let Some(c) = v.as_u64() {
                    *v = Value::from(ix(NodeId(c)).0);
                }
            }
        }
        NodeKind::Component { props, .. } => {
            for d in props.values_mut() {
                if let PropDef::InstanceSwap { default, preferred } = d {
                    *default = ix(*default);
                    preferred.clear();
                }
            }
        }
        NodeKind::Page { flows, .. } => {
            for f in flows {
                f.start = ix(f.start);
            }
        }
        NodeKind::Text(td) => {
            // The text's own fills are the layer's; runs without fills keep showing the layer's.
            if let Some(f) = td.style.fills.take() {
                let layer = std::mem::replace(&mut n.fills, f);
                for r in &mut td.runs {
                    r.style.fills.get_or_insert_with(|| layer.clone());
                }
            }
            let px = |s: &mut TextStyle| {
                if let LetterSpacing::Percent(p) = s.letter_spacing {
                    s.letter_spacing = if p == 0.0 { LetterSpacing::default() } else { LetterSpacing::Pixels(p / 100.0 * s.font_size) };
                }
                s.style_id = None;
                if let Some(f) = &mut s.fills {
                    fold(f);
                }
            };
            px(&mut td.style);
            for r in &mut td.runs {
                px(&mut r.style);
            }
            td.paragraphs = (0..td.paragraph_count()).map(|p| ParaList { indent: 0, ..td.para_list(p) }).collect();
            td.list = ListStyle::None;
        }
        NodeKind::Vector { network } => {
            let mut degree = vec![0usize; network.vertices.len()];
            for s in &network.segments {
                for i in [s.a, s.b] {
                    if let Some(d) = degree.get_mut(i) {
                        *d += 1;
                    }
                }
            }
            ends = degree.contains(&1);
            for v in &mut network.vertices {
                v.cap = None;
                v.join = None;
            }
        }
        _ => {}
    }
    // One cap for both ends, except where a vector's end points carry them.
    if !ends || !matches!(n.kind, NodeKind::Vector { .. }) {
        n.stroke.cap_end = n.stroke.cap_start;
    }
    for it in &mut n.interactions {
        figcraft_doc::proto::walk_mut(&mut it.actions, &mut |a| match a {
            Action::Navigate { dest, .. }
            | Action::OpenOverlay { dest, .. }
            | Action::SwapOverlay { dest }
            | Action::ChangeTo { dest }
            | Action::ScrollTo { dest, .. } => *dest = ix(*dest),
            _ => {}
        });
        let animated = it.actions.iter().any(|a| {
            matches!(
                a,
                Action::Navigate { .. } | Action::OpenOverlay { .. } | Action::SwapOverlay { .. } | Action::ChangeTo { .. } | Action::ScrollTo { .. }
            )
        });
        if !animated || it.transition == Transition::Instant {
            it.transition = Transition::Instant;
            it.duration_ms = 300;
            it.easing = Easing::default();
        }
    }
}

/// Every layer (pages first, depth-first) as JSON, normalized.
fn canon(doc: &Document) -> Vec<Value> {
    let order: Vec<NodeId> = doc.pages.iter().flat_map(|p| doc.subtree(*p)).collect();
    let index: BTreeMap<NodeId, u64> = order.iter().zip(0..).map(|(id, i)| (*id, i)).collect();
    let ix = |id: NodeId| NodeId(index.get(&id).copied().unwrap_or(u64::MAX));
    order
        .iter()
        .map(|id| {
            let mut n = doc.node(*id).unwrap().clone();
            normalize(&mut n, &ix);
            let mut v = serde_json::to_value(&n).unwrap();
            // Written even when it's the identity: near-identity read back must compare.
            v["transform"] = serde_json::json!(n.transform.as_coeffs());
            v
        })
        .collect()
}

/// `a` and `b` agree: the same structure, numbers within a relative tolerance.
fn close(a: &Value, b: &Value, path: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() <= 2e-3 * x.abs().max(y.abs()).max(1.0) { Ok(()) } else { Err(format!("{path}: {x} vs {y}")) }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()).collect::<BTreeSet<_>>() {
                let null = Value::Null;
                close(x.get(k).unwrap_or(&null), y.get(k).unwrap_or(&null), &format!("{path}.{k}"))?;
            }
            Ok(())
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().try_for_each(|(i, (p, q))| close(p, q, &format!("{path}[{i}]")))
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: {a} vs {b}")),
    }
}

fn assert_same(a: &Document, b: &Document, what: &str) {
    let (x, y) = (canon(a), canon(b));
    let names = |c: &[Value]| c.iter().map(|v| v["name"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(names(&x), names(&y), "{what}: layer trees differ");
    for (p, q) in x.iter().zip(&y) {
        if let Err(e) = close(p, q, p["name"].as_str().unwrap_or("?")) {
            panic!("{what}: {e}\nwritten:  {p}\nread back: {q}");
        }
    }
}

/// Export `doc`, import the file, and compare.
fn round_trip(doc: &Document, what: &str) -> (Document, Vec<String>) {
    let (bytes, warnings) = export(doc, None).unwrap();
    let (back, _) = import(&bytes, "x").unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_same(doc, &back, what);
    (back, warnings)
}

#[test]
fn generated_documents_round_trip() {
    for seed in 0..60 {
        let doc = generate(seed);
        round_trip(&doc, &format!("seed {seed}"));
    }
}

#[test]
fn round_trip_keeps_the_details() {
    let doc = generate(3);
    let (back, warnings) = round_trip(&doc, "seed 3");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.name, doc.name);
    assert_eq!(back.images.len(), doc.images.len(), "the image comes back with its bytes");
    let inst =
        back.nodes().find(|n| n.name == "Instance" && matches!(&n.kind, NodeKind::Instance { props, .. } if props.contains_key("Label"))).unwrap();
    let NodeKind::Instance { component, props } = &inst.kind else { panic!() };
    assert_eq!(back.node(*component).map(|c| c.name.as_str()), Some("Button"));
    assert_eq!(props.get("Label"), Some(&Value::String("Hi".into())));
    let label = inst.children.iter().filter_map(|c| back.node(*c)).find_map(|c| c.kind.text()).unwrap();
    assert_eq!(label.characters, "Hi");
}

#[test]
fn written_files_decode_and_list_parents_first() {
    for seed in 0..20 {
        let doc = generate(seed);
        let (bytes, _) = export(&doc, Some(&super::tests::png())).unwrap();
        let archive = Archive::open(&bytes).unwrap();
        let (schema, message) = decode_canvas(&archive.canvas).unwrap();
        let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
        let guid = |v: &Kv| format!("{}:{}", v.get("sessionID").unwrap().f64().unwrap(), v.get("localID").unwrap().f64().unwrap());
        let blobs = msg.get("blobs").unwrap().list().len();
        let mut seen = BTreeSet::new();
        let changes = msg.get("nodeChanges").unwrap().list();
        for nc in changes {
            let id = guid(nc.get("guid").unwrap());
            if nc.get("type").and_then(Kv::str) == Some("DOCUMENT") {
                assert!(nc.get("parentIndex").is_none());
            } else {
                let parent = guid(nc.get("parentIndex").unwrap().get("guid").unwrap());
                assert!(seen.contains(&parent), "seed {seed}: {id} is listed before its parent {parent}");
            }
            assert!(seen.insert(id), "seed {seed}: GUIDs are unique");
            for p in nc.get("fillGeometry").map(Kv::list).unwrap_or_default() {
                assert!((p.get("commandsBlob").unwrap().f64().unwrap() as usize) < blobs);
            }
            for p in nc.get("fillPaints").map(Kv::list).unwrap_or_default() {
                if let Some(h) = p.get("image").and_then(|i| i.get("hash")).and_then(Kv::bytes) {
                    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
                    assert!(archive.image(&hex).is_some(), "seed {seed}: image {hex} is in the file");
                }
            }
        }
        // Every layer once, except instance layers (their main says them).
        let inside = |id: NodeId| doc.ancestors(id).iter().any(|a| matches!(doc.node(*a).map(|n| &n.kind), Some(NodeKind::Instance { .. })));
        let expected = doc.pages.iter().flat_map(|p| doc.subtree(*p)).filter(|id| !inside(*id)).count() + 1;
        assert_eq!(changes.len(), expected, "seed {seed}");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        assert!(crate::read_member(&mut z, "thumbnail.png", 1 << 20).is_ok());
    }
}

#[test]
fn the_written_schema_parses_and_round_trips() {
    let schema = Schema::parse(export::SCHEMA).unwrap();
    let back = Schema::decode(&encode::schema(&schema)).unwrap();
    assert_eq!(back.defs.len(), schema.defs.len());
    for (a, b) in schema.defs.iter().zip(&back.defs) {
        assert_eq!(a.name, b.name);
        assert_eq!(
            a.fields.iter().map(|f| (&f.name, f.ty, f.array, f.value)).collect::<Vec<_>>(),
            b.fields.iter().map(|f| (&f.name, f.ty, f.array, f.value)).collect::<Vec<_>>()
        );
    }
    assert!(Schema::parse("message M nope x").is_err());
    assert!(Schema::parse("thing M").is_err());
}

/// A random value of type `ty` as the decoder would read it back.
fn random_value<'s>(s: &'s Schema, ty: Type, r: &mut Lcg, depth: usize) -> Kv<'s> {
    match ty {
        Type::Bool => Kv::Bool(r.chance(50)),
        Type::Byte => Kv::Uint(r.below(256)),
        Type::Int => Kv::Int(r.next() as i32 as i64),
        Type::Uint => Kv::Uint(u64::from(r.next() as u32)),
        Type::Float => Kv::Float(r.pick(&[0.0, 1.5, -2.25, 1e-3, 3.0e7, -1e20])),
        Type::Str => Kv::Str((0..r.below(6)).map(|_| r.pick(&['a', 'é', '😀', ' '])).collect()),
        Type::Int64 => Kv::Int(r.next() as i64 - (1 << 30)),
        Type::Uint64 => Kv::Uint(r.next()),
        Type::Def(i) => random_def(s, i, r, depth + 1),
    }
}

fn random_def<'s>(s: &'s Schema, i: usize, r: &mut Lcg, depth: usize) -> Kv<'s> {
    let d = &s.defs[i];
    let field = |f: &'s super::kiwi::Field, r: &mut Lcg| -> Kv<'s> {
        let ty = f.ty.unwrap();
        if !f.array {
            return random_value(s, ty, r, depth);
        }
        let n = if depth > 3 { 0 } else { r.below(3) };
        if ty == Type::Byte {
            return Kv::Bytes((0..n * 3).map(|_| r.next() as u8).collect());
        }
        Kv::Array((0..n).map(|_| random_value(s, ty, r, depth)).collect())
    };
    match d.kind {
        Kind::Enum => Kv::Enum(d.fields[r.below(d.fields.len() as u64) as usize].name.as_str()),
        Kind::Struct => Kv::Object(d.fields.iter().map(|f| (f.name.as_str(), field(f, r))).collect()),
        Kind::Message => {
            let mut out = Vec::new();
            if depth <= 4 {
                for f in &d.fields {
                    if r.chance(30) {
                        out.push((f.name.as_str(), field(f, r)));
                    }
                }
            }
            Kv::Object(out)
        }
    }
}

#[test]
fn encoder_output_always_decodes_to_the_same_values() {
    let schema = Schema::parse(export::SCHEMA).unwrap();
    let mut r = Lcg(5);
    for (i, d) in schema.defs.iter().enumerate() {
        if d.kind != Kind::Message {
            continue;
        }
        for _ in 0..20 {
            let v = random_def(&schema, i, &mut r, 0);
            let bytes = encode::message(&schema, i, &v).unwrap();
            assert_eq!(schema.decode_message(i, &bytes).unwrap(), v, "{}", d.name);
        }
    }
}

#[test]
fn the_encoder_refuses_what_the_schema_cant_hold() {
    let schema = Schema::parse("struct V float x, float y\nmessage M V v, uint n, string s, bool b").unwrap();
    let bad = [
        Kv::Object(vec![("v", Kv::Object(vec![("x", Kv::Float(f32::NAN)), ("y", Kv::Float(0.0))]))]),
        Kv::Object(vec![("v", Kv::Object(vec![("x", Kv::Float(1.0))]))]),
        Kv::Object(vec![("n", Kv::Int(-1))]),
        Kv::Object(vec![("n", Kv::Uint(1 << 40))]),
        Kv::Object(vec![("nope", Kv::Bool(true))]),
        Kv::Object(vec![("s", Kv::Bool(true))]),
        Kv::Array(vec![]),
    ];
    for v in &bad {
        assert!(encode::message(&schema, 1, v).is_err(), "{v:?}");
    }
    // NULs inside strings are dropped rather than ending them early.
    let ok = Kv::Object(vec![("s", Kv::Str("a\0b".into())), ("b", Kv::Bool(true))]);
    let back = schema.decode_message(1, &encode::message(&schema, 1, &ok).unwrap()).unwrap();
    assert_eq!(back.get("s"), Some(&Kv::Str("ab".into())));
    assert_eq!(back.get("b"), Some(&Kv::Bool(true)));
}

#[test]
fn junk_documents_never_panic() {
    for seed in 0..40 {
        let mut doc = generate(seed);
        let mut r = Lcg(seed + 99);
        let ids: Vec<NodeId> = doc.ids().collect();
        let junk = [f64::NAN, f64::INFINITY, -1e300, 1e300, -5.0, 0.0];
        for id in ids {
            if !r.chance(40) {
                continue;
            }
            let Some(n) = doc.node_mut(id) else { continue };
            n.width = r.pick(&junk);
            n.transform = Affine::new([r.pick(&junk), 0.0, 0.0, 1.0, r.pick(&junk), 0.0]);
            n.opacity = f32::NAN;
            n.name.push('\0');
            n.stroke.weight = r.pick(&junk);
            n.stroke.dashes.push(f64::NAN);
            n.corners.radii[1] = f64::INFINITY;
            n.child_layout.grid_column = Some(u32::MAX);
            for p in &mut n.fills {
                match &mut p.kind {
                    PaintKind::Gradient(g) => g.from = (f64::NAN, 0.0),
                    PaintKind::Image(img) => img.image = "missing".into(),
                    _ => p.opacity = f32::INFINITY,
                }
            }
            if let Some(l) = &mut n.layout {
                l.grid_columns = u32::MAX;
                l.gap = f64::NAN;
            }
            n.interactions.push(Interaction {
                trigger: Trigger::KeyPress { key: "Hyper+\u{0}".into() },
                actions: vec![Action::Navigate { dest: NodeId(u64::MAX), preserve_scroll: false }],
                transition: Transition::Instant,
                duration_ms: u32::MAX,
                easing: Easing::default(),
                delay_ms: u32::MAX,
            });
            match &mut n.kind {
                NodeKind::Vector { network } => {
                    network.segments.push(figcraft_doc::Segment::line(0, 99));
                    if let Some(v) = network.vertices.first_mut() {
                        v.x = f64::NAN;
                    }
                }
                NodeKind::Text(td) => {
                    td.characters.push_str("\0\u{FFFF}");
                    td.runs.push(StyleRun { start: 5, end: 1, style: TextStyle::default() });
                }
                NodeKind::Instance { component, props } => {
                    *component = NodeId(u64::MAX - 1);
                    props.insert("x".into(), Value::from(-1));
                }
                NodeKind::Polygon { count } => *count = u32::MAX,
                NodeKind::Ellipse { sweep, .. } => *sweep = f64::NAN,
                _ => {}
            }
        }
        if let Ok((bytes, _)) = export(&doc, None) {
            let _ = import(&bytes, "x");
        }
    }
}

/// A frame's own min/max size survives whatever its parent is, not only under auto layout.
#[test]
fn frame_size_limits_survive_without_auto_layout_parent() {
    fn limits(doc: &Document) -> [Option<f64>; 4] {
        let (bytes, ew) = export(doc, None).unwrap();
        let (back, iw) = import(&bytes, "File").unwrap();
        assert!(ew.is_empty() && iw.is_empty(), "export {ew:?}, import {iw:?}");
        let cl = &back.nodes().find(|n| n.name == "Subject").unwrap().child_layout;
        [cl.min_w, cl.max_w, cl.min_h, cl.max_h]
    }
    let mut doc = Document::new("File");
    let page = doc.first_page().unwrap();
    let id = doc.create(page, NodeKind::Frame, "Subject", 0.0, 0.0, 100.0, 100.0).unwrap();
    let node = doc.node_mut(id).unwrap();
    node.layout = Some(AutoLayout::default());
    node.child_layout.min_w = Some(80.0);
    node.child_layout.max_w = Some(150.0);
    node.child_layout.min_h = Some(60.0);
    node.child_layout.max_h = Some(170.0);
    let expected = [Some(80.0), Some(150.0), Some(60.0), Some(170.0)];

    let mut nested = doc.clone();
    let wrapper = nested.create(page, NodeKind::Frame, "Wrapper", 0.0, 0.0, 300.0, 300.0).unwrap();
    nested.node_mut(wrapper).unwrap().layout = Some(AutoLayout::default());
    nested.reparent(id, wrapper, None).unwrap();
    assert_eq!(limits(&nested), expected, "auto layout parent");
    nested.node_mut(wrapper).unwrap().layout = None;
    assert_eq!(limits(&nested), expected, "plain frame parent");
    assert_eq!(limits(&doc), expected, "page parent");
}

/// Link each instance's layers to its main's by position, as they are imported (the engine
/// links them by path after loading).
fn link(doc: &mut Document) {
    fn zip(doc: &mut Document, mine: NodeId, theirs: NodeId) {
        let pairs: Vec<(NodeId, NodeId)> = match (doc.node(mine), doc.node(theirs)) {
            (Some(m), Some(t)) => m.children.iter().copied().zip(t.children.iter().copied()).collect(),
            _ => return,
        };
        for (m, t) in pairs {
            if let Some(n) = doc.node_mut(m) {
                n.source.get_or_insert(t);
            }
            zip(doc, m, t);
        }
    }
    let insts: Vec<(NodeId, NodeId)> = doc
        .nodes()
        .filter_map(|n| match &n.kind {
            NodeKind::Instance { component, .. } => Some((n.id, *component)),
            _ => None,
        })
        .collect();
    for (i, c) in insts {
        zip(doc, i, c);
    }
}

/// Local only (`FIGCRAFT_FIG_SAMPLES`): each sample imported, exported and imported again comes
/// back the same, apart from what the export leaves out (shared style links, variables).
#[test]
fn local_samples_round_trip() {
    for p in super::tests::samples() {
        let bytes = std::fs::read(&p).unwrap();
        let (mut a, _) = import(&bytes, "sample").unwrap();
        link(&mut a);
        let (out, warnings) = export(&a, None).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let (mut b, _) = import(&out, "sample").unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        for d in [&mut a, &mut b] {
            let ids: Vec<NodeId> = d.ids().collect();
            for id in ids {
                if let Some(n) = d.node_mut(id) {
                    n.styles = StyleRefs::default();
                    n.bound.clear();
                    n.var_modes.clear();
                }
            }
        }
        assert_same(&a, &b, &format!("{} (export warnings: {warnings:?})", p.display()));
    }
}
