use super::common::{style_fields, unsupported_decoration_look};
use super::{
    Affine, BezPath, Bound, Exporter, Fields, Kv, ListStyle, MAX_CANVAS, MAX_CHARS, MAX_NETWORK, Node, NodeKind, Paint, StrokeAlign, StrokeCap,
    StrokeJoin, TextAlignH, TextAlignV, TextAutoResize, TextData, TextDirection, TextStyle, VectorNetwork, WindingRule, arr, b, cap_name, e, finite,
    fl, int, join_name, network, o, path_blob, s, u, xy,
};

impl<'d> Exporter<'d> {
    /// Text: its style, box settings, and characters with their styled runs. `fills`: the layer's
    /// (what runs without fills of their own show when the text's own style sets other fills).
    pub(super) fn text(&mut self, td: &TextData, fills: &[Paint], f: &mut Fields) {
        style_fields(&td.style, f);
        let (ha, va) = (
            match td.align_h {
                TextAlignH::Left => "LEFT",
                TextAlignH::Center => "CENTER",
                TextAlignH::Right => "RIGHT",
                TextAlignH::Justified => "JUSTIFIED",
            },
            match td.align_v {
                TextAlignV::Top => "TOP",
                TextAlignV::Middle => "CENTER",
                TextAlignV::Bottom => "BOTTOM",
            },
        );
        let resize = match td.auto_resize {
            TextAutoResize::WidthAndHeight => "WIDTH_AND_HEIGHT",
            TextAutoResize::Height => "HEIGHT",
            TextAutoResize::None | TextAutoResize::Truncate => "NONE",
        };
        f.extend([
            ("textAlignHorizontal", e(ha)),
            ("textAlignVertical", e(va)),
            ("textAutoResize", e(resize)),
            ("paragraphSpacing", fl(td.paragraph_spacing)),
            ("paragraphIndent", fl(td.paragraph_indent)),
            ("autoRename", b(false)),
            ("fontVariantCommonLigatures", b(true)),
            ("fontVariantContextualLigatures", b(true)),
            ("fontVersion", s("")),
            ("textUserLayoutVersion", u(5)),
            ("textExplicitLayoutVersion", u(1)),
            ("textBidiVersion", u(1)),
            ("textTracking", fl(0.0)),
            ("emojiImageSet", e("APPLE")),
        ]);
        // Written only off their defaults: a text at the defaults carries none of these four in a design file saved by
        // Figma. A text style holds none of them, so a layer at the defaults has no style value to take on import.
        if td.list_spacing != 0.0 {
            f.push(("listSpacing", fl(td.list_spacing)));
        }
        if td.hanging_punctuation {
            f.push(("hangingPunctuation", b(true)));
        }
        if td.hanging_list {
            f.push(("hangingList", b(true)));
        }
        if td.leading_trim == figcraft_doc::LeadingTrim::CapHeight {
            f.push(("leadingTrim", e("CAP_HEIGHT")));
        }
        if td.truncates() {
            f.push(("textTruncation", e("ENDING")));
        }
        if td.max_lines > 0 {
            f.push(("maxLines", int(td.max_lines.into())));
        }
        // Styles per UTF-16 unit: 0 for the base style, else an entry of the override table.
        if td.char_count() > MAX_CHARS {
            self.warn("text longer than 1,048,576 characters was cut");
        }
        let spans = td.spans();
        let mut table: Vec<&TextStyle> = Vec::new();
        // Each distinct style once, found by its serialized form rather than by comparing it
        // with every earlier one (quadratic in the number of distinct styles).
        let mut index: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut span_ids = Vec::with_capacity(spans.len());
        for (_, _, st) in &spans {
            span_ids.push(if **st == td.style {
                0
            } else {
                match serde_json::to_string(st) {
                    Ok(key) => *index.entry(key).or_insert_with(|| {
                        table.push(st);
                        table.len() as u32
                    }),
                    Err(_) => {
                        table.push(st);
                        table.len() as u32
                    }
                }
            });
        }
        let mut chars = String::new();
        let mut ids: Vec<Kv> = Vec::new();
        let mut span = 0;
        for (i, ch) in td.characters.chars().take(MAX_CHARS).enumerate() {
            while spans.get(span).is_some_and(|sp| i >= sp.1) {
                span += 1;
            }
            // NULs can't be stored in a string.
            if ch == '\0' {
                continue;
            }
            chars.push(ch);
            let id = span_ids.get(span).copied().unwrap_or(0);
            ids.extend((0..ch.len_utf16()).map(|_| u(id)));
        }
        let mut data: Fields = vec![("characters", Kv::Str(chars))];
        if !table.is_empty() {
            let entries = table
                .iter()
                .zip(1u32..)
                .map(|(st, id)| {
                    let mut ef: Fields = vec![("styleID", u(id))];
                    style_fields(st, &mut ef);
                    // An empty link clears the text's own (a missing one would inherit it).
                    if st.link.is_none() && td.style.link.is_some() {
                        ef.push(("hyperlink", o(vec![("url", s(""))])));
                    }
                    if let Some(p) = st.fills.as_deref().or(td.style.fills.as_ref().map(|_| fills)) {
                        ef.push(("fillPaints", self.paints(p, true, &Bound::default())));
                    }
                    o(ef)
                })
                .collect();
            data.extend([("characterStyleIDs", arr(ids)), ("styleOverrideTable", arr(entries))]);
        }
        let lists: Vec<ListStyle> = (0..td.paragraph_count()).map(|p| td.para_list(p).list).collect();
        let first = |i: usize, l: &ListStyle| *l != ListStyle::None && (i == 0 || lists.get(i - 1) != Some(l));
        // Where each line's numbered list starts counting, less one: its first item's start, on
        // every line of the list.
        let mut run = 0;
        let starts: Vec<u32> = lists
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if first(i, l) {
                    run = td.para_list(i).start_offset;
                }
                if *l == ListStyle::Numbered { run } else { 0 }
            })
            .collect();
        // One entry per paragraph, each with every field a reader expects.
        let line = |(i, l): (usize, &ListStyle)| {
            let ty = match l {
                ListStyle::None => "PLAIN",
                ListStyle::Numbered => "ORDERED_LIST",
                ListStyle::Bulleted => "UNORDERED_LIST",
            };
            let listed = *l != ListStyle::None;
            let begins = first(i, l);
            let start = starts.get(i).copied().unwrap_or(0);
            let direction = match td.para_list(i).direction {
                TextDirection::Auto => "AUTO",
                TextDirection::Ltr => "LTR",
                TextDirection::Rtl => "RTL",
            };
            o(vec![
                ("lineType", e(ty)),
                ("styleId", int(0)),
                ("indentationLevel", int(i64::from(listed))),
                ("sourceDirectionality", e(direction)),
                ("listStartOffset", int(i64::from(start))),
                ("isFirstLineOfList", b(begins)),
            ])
        };
        data.push(("lines", arr(lists.iter().enumerate().map(line).collect())));
        f.push(("textData", o(data)));
        if (0..td.paragraph_count()).any(|p| td.para_list(p).indent > 0) {
            self.warn("nested list levels were written as the first level");
        }
        let styles = std::iter::once(&td.style).chain(td.runs.iter().map(|r| &r.style));
        if styles.clone().any(|st| unsupported_decoration_look(st) || !st.variations.is_empty()) {
            self.warn("text decoration looks and variable font settings were left out");
        }
        if !td.features.is_empty() {
            self.warn("OpenType features were left out");
        }
    }

    /// The editable network (with per-point corner radii, handle mirroring and end caps) and,
    /// when it has filled areas, their outline.
    pub(super) fn vector(&mut self, n: &Node, net: &VectorNetwork, f: &mut Fields) {
        if net.regions.iter().any(|region| region.fills.is_some() || region.paint_space.is_some()) {
            self.warn("vector region paints were left out of the .fig export");
        }
        let entries: usize = net.regions.iter().flat_map(|r| &r.loops).map(Vec::len).sum();
        if net.vertices.len() + net.segments.len() + entries > MAX_NETWORK {
            self.warn("vector networks too large for the format were written as outlines");
        } else {
            // The stroke starts at the first end point and stops at the last: the layer's end caps go
            // there (where the point has none of its own).
            let mut degree = vec![0usize; net.vertices.len()];
            for sg in &net.segments {
                for i in [sg.a, sg.b] {
                    if let Some(d) = degree.get_mut(i) {
                        *d += 1;
                    }
                }
            }
            let ends: Vec<usize> = (0..degree.len()).filter(|i| degree.get(*i) == Some(&1)).collect();
            let (first, last) = (ends.first().copied(), ends.last().copied());
            let caps_differ = n.stroke.cap_start != n.stroke.cap_end || net.vertices.iter().any(|v| v.cap.is_some());
            let mut styles: Vec<Fields> = Vec::new();
            let mut style_of = |fields: Fields| -> u32 {
                if fields.is_empty() {
                    return 0;
                }
                let i = styles.iter().position(|x| *x == fields).unwrap_or_else(|| {
                    styles.push(fields);
                    styles.len() - 1
                });
                i as u32 + 1
            };
            let vertices = net
                .vertices
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let mut st: Fields = Vec::new();
                    // A point without a radius of its own is drawn with the layer's, so a sharp
                    // point on a rounded layer says so.
                    if v.radius > 0.0 || n.corners.radii[0] > 0.0 {
                        st.push(("cornerRadius", fl(v.radius)));
                    }
                    if let Some(m) = v.mirror {
                        st.push((
                            "handleMirroring",
                            e(match m {
                                figcraft_doc::HandleMirroring::None => "NONE",
                                figcraft_doc::HandleMirroring::Angle => "ANGLE",
                                figcraft_doc::HandleMirroring::AngleAndLength => "ANGLE_AND_LENGTH",
                            }),
                        ));
                    }
                    let end_cap = if Some(i) == first {
                        Some(n.stroke.cap_start)
                    } else if Some(i) == last {
                        Some(n.stroke.cap_end)
                    } else {
                        None
                    };
                    if let Some(c) = v.cap.or(end_cap.filter(|_| caps_differ)) {
                        st.push(("strokeCap", e(cap_name(c))));
                    }
                    if let Some(j) = v.join {
                        st.push(("strokeJoin", e(join_name(j))));
                    }
                    network::Vertex { style: style_of(st), x: finite(v.x), y: finite(v.y) }
                })
                .collect();
            let segments = net
                .segments
                .iter()
                .map(|sg| network::Segment {
                    style: 0,
                    a: sg.a,
                    ta: (finite(sg.ta.0), finite(sg.ta.1)),
                    b: sg.b,
                    tb: (finite(sg.tb.0), finite(sg.tb.1)),
                })
                .collect();
            let regions = net
                .regions
                .iter()
                .map(|r| network::Region { style: 0, non_zero: r.winding == WindingRule::NonZero, loops: r.loops.clone() })
                .collect();
            let blob = network::encode(&network::Network { vertices, segments, regions });
            let table = styles.into_iter().zip(1u32..).map(|(mut st, id)| {
                st.insert(0, ("styleID", u(id)));
                o(st)
            });
            let table: Vec<Kv> = table.collect();
            if let Some(i) = self.blob(blob) {
                f.push((
                    "vectorData",
                    o(vec![("vectorNetworkBlob", u(i)), ("normalizedSize", xy(n.width, n.height)), ("styleOverrideTable", arr(table))]),
                ));
            }
        }
        if !net.regions.is_empty() {
            let winding = net.regions.first().map(|r| r.winding).unwrap_or_default();
            if net.regions.iter().any(|r| r.winding != winding) {
                self.warn("vectors whose filled areas use different winding rules use the first one's in their outline");
            }
            self.geometry("fillGeometry", &figcraft_doc::geometry::drawn_network(net, n.corners.smoothing).fill_path(), winding, f);
        }
    }

    /// `fillGeometry` or `strokeGeometry` (`key`) from a local path.
    pub(super) fn geometry(&mut self, key: &'static str, p: &BezPath, winding: WindingRule, f: &mut Fields) {
        if p.elements().is_empty() {
            return;
        }
        if let Some(i) = self.blob(path_blob(p)) {
            let rule = if winding == WindingRule::EvenOdd { "ODD" } else { "NONZERO" };
            f.push((key, arr(vec![o(vec![("windingRule", e(rule)), ("commandsBlob", u(i)), ("styleID", u(0))])])));
        }
    }

    /// `strokeGeometry`, the outline of a plain stroke: a ring around boxes and whole ellipses, the
    /// widened path of other centred strokes. Dashed, per-side and other strokes get none (a reader
    /// builds theirs from the stroke settings).
    pub(super) fn stroke_geometry(&mut self, n: &Node, boxy: bool, f: &mut Fields) {
        let sp = &n.stroke;
        let (w, h, weight) = (n.width, n.height, sp.weight);
        let plain = n.strokes.iter().any(|p| p.visible) && sp.dashes.is_empty() && sp.sides.is_none() && sp.profile().is_none() && sp.brush.is_none();
        if !(plain && weight.is_finite() && weight > 0.0 && w.is_finite() && h.is_finite()) {
            return;
        }
        let whole = matches!(n.kind, NodeKind::Ellipse { sweep, inner, .. } if inner == 0.0 && figcraft_doc::geom::full_sweep(sweep));
        if boxy || whole {
            let (out, inset) = match sp.align {
                StrokeAlign::Inside => (0.0, weight),
                StrokeAlign::Center => (weight / 2.0, weight / 2.0),
                StrokeAlign::Outside => (weight, 0.0),
            };
            // The shape grown by `d` (shrunk when negative), or nothing when it shrinks away.
            let grown = |d: f64| -> Option<BezPath> {
                let (gw, gh) = (w + 2.0 * d, h + 2.0 * d);
                if gw <= 0.0 || gh <= 0.0 {
                    return None;
                }
                let mut p = if whole {
                    figcraft_doc::geom::ellipse(gw, gh, 0.0, std::f64::consts::TAU, 0.0)
                } else {
                    let radii = n.corners.radii.map(|r| if r > 0.0 { (r + d).max(0.0) } else { 0.0 });
                    figcraft_doc::geom::rounded_rect(gw, gh, radii, n.corners.smoothing)
                };
                p.apply_affine(Affine::translate((-d, -d)));
                Some(p)
            };
            let Some(mut ring) = grown(out) else { return };
            ring.extend(grown(-inset).iter().flat_map(|p| p.elements().iter().copied()));
            self.geometry("strokeGeometry", &ring, WindingRule::EvenOdd, f);
        } else if sp.align == StrokeAlign::Center {
            let path = match &n.kind {
                NodeKind::Vector { .. } => figcraft_doc::geometry::stroke_geometry(n),
                NodeKind::Text(_) => None,
                _ => figcraft_doc::geometry::shape_path(n),
            };
            let Some(path) = path.filter(|p| !p.elements().is_empty()) else { return };
            use figcraft_doc::geom::kurbo::{Cap, Join, Stroke, StrokeOpts, stroke};
            let cap = match sp.cap_start {
                StrokeCap::Round => Cap::Round,
                StrokeCap::Square => Cap::Square,
                _ => Cap::Butt,
            };
            let join = match sp.join {
                StrokeJoin::Miter => Join::Miter,
                StrokeJoin::Bevel => Join::Bevel,
                StrokeJoin::Round => Join::Round,
            };
            let limit = if sp.miter_limit.is_finite() { sp.miter_limit.max(1.0) } else { 4.0 };
            let style = Stroke::new(weight).with_caps(cap).with_join(join).with_miter_limit(limit);
            self.geometry("strokeGeometry", &stroke(path, &style, &StrokeOpts::default(), 0.05), WindingRule::NonZero, f);
        }
    }

    /// Add a blob; its index (`None` once the blobs would make the file too large).
    fn blob(&mut self, bytes: Vec<u8>) -> Option<u32> {
        if let Some(i) = self.blobs.get(&bytes) {
            return Some(*i);
        }
        self.blob_bytes += bytes.len() as u64;
        if self.blob_bytes > MAX_CANVAS {
            self.warn("vector data too large for the format was left out");
            return None;
        }
        let i = u32::try_from(self.blobs.len()).ok()?;
        self.blobs.insert(bytes, i);
        Some(i)
    }

    /// The fields of a text on a path (its start and its path as a vector network), or `None` when the text stays plain text
    /// (with the warning that says why). `nested`: for an instance's override row, which follows its main's layer.
    pub(super) fn text_path(&mut self, n: &Node, td: &TextData, nested: bool) -> Option<Fields> {
        let doc = self.doc;
        let own = text_path_net(td, n.width, n.height);
        let main = if nested { n.source.and_then(|s| doc.node(s)) } else { None };
        if let Some(main) = main {
            let main_ok = matches!(&main.kind, NodeKind::Text(m) if text_path_net(m, main.width, main.height).is_ok());
            if main_ok != own.is_ok() {
                // An override can't take its layer off the main's path. So an instance text of
                // more than one line, on a main that is on a path, stays on that path in the file:
                // say so rather than the general mismatch (FigCraft's import reads it as plain text).
                let lines = main_ok && matches!(own, Err(TEXT_PATH_LINES));
                self.warn(if lines { TEXT_PATH_INSTANCE_LINES } else { TEXT_PATH_INSTANCE });
                return None;
            }
        }
        let tp = td.path.as_deref()?;
        let made = match own {
            Ok(made) => made,
            Err(why) => {
                self.warn(why);
                return None;
            }
        };
        let Some(index) = self.blob(network::encode(&made.net)) else {
            self.warn(TEXT_PATH_CANT);
            return None;
        };
        if made.later_parts {
            self.warn(TEXT_PATH_PARTS);
        }
        Some(vec![
            ("textPathStart", o(vec![("tValue", fl(made.t)), ("forward", b(!tp.flip))])),
            ("vectorData", o(vec![("vectorNetworkBlob", u(index)), ("normalizedSize", xy(n.width, n.height))])),
        ])
    }
}

/// The style id the format gives the segment a text on a path starts on.
const TEXT_PATH_MARK: u32 = 0x01ff_ffff;
const TEXT_PATH_LINES: &str = "text on a path with more than one line was written as plain text";
const TEXT_PATH_CANT: &str = "text on a path the format can't hold was written as plain text";
const TEXT_PATH_PARTS: &str = "the parts of a text's path after the first were left out";
const TEXT_PATH_INSTANCE: &str =
    "text on a path in an instance but not in its main component, or the other way round, was written as the main has it";
const TEXT_PATH_INSTANCE_LINES: &str =
    "an instance's text of more than one line stays on its main component's text path in the file, as an override can't take it off";

/// A text's path as the format holds it: one chain (or cycle) of a vector network with the
/// segment the text starts on marked, and the start as that segment's own parameter.
struct PathNet {
    net: network::Network,
    t: f64,
    later_parts: bool,
}

/// The tangent-to-control-point cubic of a network segment, as the import reads it.
fn net_cubic(net: &network::Network, s: &network::Segment) -> Option<(figcraft_doc::geom::kurbo::CubicBez, bool)> {
    use figcraft_doc::geom::kurbo::CubicBez;
    let (a, b) = (net.vertices.get(s.a)?, net.vertices.get(s.b)?);
    let line = s.ta == (0.0, 0.0) && s.tb == (0.0, 0.0);
    let cubic = CubicBez::new((a.x, a.y), (a.x + s.ta.0, a.y + s.ta.1), (b.x + s.tb.0, b.y + s.tb.1), (b.x, b.y));
    Some((cubic, line))
}

/// The length of a segment, as the import measures it.
fn net_length(net: &network::Network, s: &network::Segment) -> f64 {
    use figcraft_doc::geom::kurbo::{Line, ParamCurveArclen};
    match net_cubic(net, s) {
        Some((c, true)) => Line::new(c.p0, c.p3).arclen(1e-9),
        Some((c, false)) => c.arclen(1e-9),
        None => 0.0,
    }
}

/// The part of a path (a move and what follows it) as a chain of the network, and whether it is
/// a cycle; `None` without a segment longer than a billionth.
fn path_chain(part: &[figcraft_doc::geom::kurbo::PathEl]) -> Option<(network::Network, bool)> {
    use figcraft_doc::geom::kurbo::{PathEl, Point, QuadBez};
    let r = |x: f64| f64::from(finite(x) as f32);
    let rp = |p: Point| (r(p.x), r(p.y));
    let mut net = network::Network::default();
    let mut closed = false;
    let mut cur = (0.0, 0.0);
    let add = |net: &mut network::Network, c1: Option<(f64, f64)>, c2: Option<(f64, f64)>, end: (f64, f64)| {
        let Some(last) = net.vertices.last() else { return };
        let (a, index) = ((last.x, last.y), net.vertices.len() - 1);
        let tangent = |c: Option<(f64, f64)>, from: (f64, f64)| c.map_or((0.0, 0.0), |c| (r(c.0 - from.0), r(c.1 - from.1)));
        let seg = network::Segment { style: 0, a: index, ta: tangent(c1, a), b: index + 1, tb: tangent(c2, end) };
        net.vertices.push(network::Vertex { style: 0, x: end.0, y: end.1 });
        if net_length(net, &seg) > 1e-9 {
            net.segments.push(seg);
        } else {
            net.vertices.pop();
        }
    };
    for el in part {
        match el {
            PathEl::MoveTo(p) => {
                cur = rp(*p);
                net.vertices.push(network::Vertex { style: 0, x: cur.0, y: cur.1 });
            }
            PathEl::LineTo(p) => {
                cur = rp(*p);
                add(&mut net, None, None, cur);
            }
            PathEl::QuadTo(c, p) => {
                let q = QuadBez::new(Point::new(cur.0, cur.1), Point::new(r(c.x), r(c.y)), Point::new(r(p.x), r(p.y))).raise();
                cur = rp(*p);
                add(&mut net, Some((q.p1.x, q.p1.y)), Some((q.p2.x, q.p2.y)), cur);
            }
            PathEl::CurveTo(a, b, p) => {
                cur = rp(*p);
                // The control points are not rounded on their own: the format holds each as its offset from the end
                // point, and taken from the point as read that is the offset the file had.
                add(&mut net, Some((finite(a.x), finite(a.y))), Some((finite(b.x), finite(b.y))), cur);
            }
            PathEl::ClosePath => closed = true,
        }
    }
    if net.segments.is_empty() {
        return None;
    }
    let first = net.vertices.first().map(|v| (v.x, v.y))?;
    let last = net.vertices.last().map(|v| (v.x, v.y))?;
    let near = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1) <= 1e-6;
    let mut cycle = false;
    if closed || near(last, first) {
        if near(last, first) {
            // The last segment ends on the first vertex. A single segment stays an open chain of
            // two vertices at one place: the import refuses a segment from a vertex to itself.
            if net.segments.len() > 1 {
                net.vertices.pop();
                if let Some(s) = net.segments.last_mut() {
                    s.b = 0;
                    s.tb = (r(last.0 + s.tb.0 - first.0), r(last.1 + s.tb.1 - first.1));
                    cycle = true;
                }
            }
        } else {
            net.segments.push(network::Segment { style: 0, a: net.vertices.len() - 1, ta: (0.0, 0.0), b: 0, tb: (0.0, 0.0) });
            cycle = true;
        }
    }
    Some((net, cycle))
}

/// A text on a path as the format holds it, or the warning for why it stays plain text.
fn text_path_net(td: &TextData, w: f64, h: f64) -> Result<PathNet, &'static str> {
    use figcraft_doc::geom::kurbo::{ParamCurveArclen, PathEl};
    let Some(tp) = td.path.as_deref() else { return Err(TEXT_PATH_CANT) };
    if td.characters.contains(['\n', '\r']) {
        return Err(TEXT_PATH_LINES);
    }
    let path = tp.path();
    let mut parts: Vec<Vec<PathEl>> = Vec::new();
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            parts.push(Vec::new());
        }
        if let Some(part) = parts.last_mut() {
            part.push(*el);
        }
    }
    let mut chains = parts.iter().filter_map(|p| path_chain(p));
    let (mut net, cycle) = chains.next().ok_or(TEXT_PATH_CANT)?;
    let later_parts = chains.next().is_some();
    if net.vertices.len() + net.segments.len() > MAX_NETWORK {
        return Err(TEXT_PATH_CANT);
    }
    // The import scales the coordinates by size over normalized size, which are the same number
    // here; an axis of no extent holds nothing but zeros.
    let size = |x: f64| f64::from(finite(x) as f32);
    for (extent, axis) in [(size(w), 0), (size(h), 1)] {
        let at = |p: (f64, f64)| if axis == 0 { p.0 } else { p.1 };
        let flat = net.vertices.iter().all(|v| at((v.x, v.y)) == 0.0) && net.segments.iter().all(|s| at(s.ta) == 0.0 && at(s.tb) == 0.0);
        if extent < 0.0 || (extent <= 1e-9 && (extent != 0.0 || !flat)) {
            return Err(TEXT_PATH_CANT);
        }
    }
    let lengths: Vec<f64> = net.segments.iter().map(|s| net_length(&net, s)).collect();
    let total: f64 = lengths.iter().sum();
    if !total.is_finite() || total <= 1e-10 {
        return Err(TEXT_PATH_CANT);
    }
    // The path the import builds from it must be one the text renderer can measure.
    let mut shape = figcraft_doc::geom::BezPath::new();
    for (i, s) in net.segments.iter().enumerate() {
        let Some((c, line)) = net_cubic(&net, s) else { return Err(TEXT_PATH_CANT) };
        if i == 0 {
            shape.move_to(c.p0);
        }
        if line {
            shape.line_to(c.p3);
        } else {
            shape.curve_to(c.p1, c.p2, c.p3);
        }
    }
    if cycle {
        shape.close_path();
    }
    let measured = figcraft_doc::geom::measure::polylines(&shape);
    if measured.as_ref().and_then(|l| l.first()).is_none_or(|l| l.length() <= 1e-9) {
        return Err(TEXT_PATH_CANT);
    }
    // The start, by length, as a parameter of the segment it falls on.
    let start = tp.start();
    let at = (if tp.flip { 1.0 - start } else { start }) * total;
    let mut before = 0.0;
    let mut k = lengths.len().saturating_sub(1);
    for (i, len) in lengths.iter().enumerate() {
        if before + len >= at {
            k = i;
            break;
        }
        before += len;
    }
    if k + 1 == lengths.len() && before + lengths[k] < at {
        before = total - lengths[k];
    }
    let local = (at - before).clamp(0.0, lengths[k]);
    let t = match net.segments.get(k).and_then(|s| net_cubic(&net, s)) {
        Some((_, true)) | None => local / lengths[k],
        Some((c, false)) => c.inv_arclen(local, 1e-9),
    };
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    if let Some(s) = net.segments.get_mut(k) {
        s.style = TEXT_PATH_MARK;
    }
    Ok(PathNet { net, t, later_parts })
}
