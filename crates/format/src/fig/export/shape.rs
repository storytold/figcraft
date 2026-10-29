use super::common::{style_fields, unsupported_decoration_look};
use super::{
    Affine, BezPath, Bound, Exporter, Fields, Kv, ListStyle, MAX_CANVAS, MAX_CHARS, MAX_NETWORK, Node, NodeKind, Paint, StrokeAlign, StrokeCap,
    StrokeJoin, TextAlignH, TextAlignV, TextAutoResize, TextData, TextStyle, VectorNetwork, WindingRule, arr, b, cap_name, e, finite, fl, int,
    join_name, network, o, path_blob, s, u, xy,
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
        let mut span_ids = Vec::with_capacity(spans.len());
        for (_, _, st) in &spans {
            span_ids.push(if **st == td.style {
                0
            } else {
                let i = table.iter().position(|t| *t == *st).unwrap_or_else(|| {
                    table.push(st);
                    table.len() - 1
                });
                i as u32 + 1
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
            o(vec![
                ("lineType", e(ty)),
                ("styleId", int(0)),
                ("indentationLevel", int(i64::from(listed))),
                ("sourceDirectionality", e("AUTO")),
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
        if td.path.is_some() {
            self.warn("text on a path was written as plain text");
        }
        if td.list_spacing != 0.0 || td.hanging_punctuation || td.hanging_list || td.leading_trim != Default::default() || !td.features.is_empty() {
            self.warn("list spacing, hanging punctuation, leading trim and OpenType features were left out");
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
                    if v.radius > 0.0 {
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
            self.geometry("fillGeometry", &net.fill_path(), winding, f);
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
        let whole = matches!(n.kind, NodeKind::Ellipse { sweep, inner, .. } if inner == 0.0 && sweep.abs() >= std::f64::consts::TAU - 1e-9);
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
                NodeKind::Vector { network } => Some(network.stroke_path()),
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
}
