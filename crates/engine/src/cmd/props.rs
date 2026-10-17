//! Property editing: `node.set` and the shorthand keys every surface uses.

use figcraft_color::{Color, Paint};
use figcraft_doc::*;
use figcraft_geom::{Affine, Point};
use serde_json::{Map, Value, json};

use crate::params::*;
use crate::{Result, bad};

/// The per-side stroke weight keys, in `StrokeProps::sides` order (top, right, bottom, left).
pub const STROKE_SIDES: [&str; 4] = ["strokeTopWeight", "strokeRightWeight", "strokeBottomWeight", "strokeLeftWeight"];

/// Keys `node.set` understands (documented for agents).
pub const KEYS: &str = "name, visible, locked, opacity (0-1), blend, x, y (relative to the parent frame; groups don't count), width, height, rotation (degrees), \
fills (paints or colours; gradients as {type: linearGradient|radialGradient|angularGradient|diamondGradient, stops: [\"#hex\", ...], from?, to?} too), fill (colour), strokes, stroke (colour), strokeWeight, strokeAlign (inside|center|outside), strokeSides [t,r,b,l], strokeTopWeight / strokeRightWeight / strokeBottomWeight / strokeLeftWeight (one side; turns on individual strokes), \
strokeCap / startCap / endCap, strokeJoin, miterAngle (degrees: sharper joins bevel; or miterLimit as a ratio), dashes [..], widthProfile (preset uniform|wedge|taper|eye|rounded|pinch, [[t, scale], ...] or null), brush (preset id, {preset, spacing, sizeJitter, rotationJitter, scatter, seed} or null), cornerRadius (n or [tl,tr,br,bl]), cornerSmoothing (0-1), effects, \
clip, isMask, layout (null|horizontal|vertical|grid|{...}), gap, crossGap, padding (n or [t,r,b,l]), paddingTop/Right/Bottom/Left, \
mainAlign (min|center|max, or the auto gaps spaceBetween|spaceAround|spaceEvenly), crossAlign (min|center|max|baseline: first text baselines, horizontal layouts), contentAlign (wrapped lines: auto = packed where crossAlign says | spaceBetween), wrap (rows of a horizontal layout, columns of a vertical one), strokesIncluded (the frame's inside stroke takes up space), reverseZ (canvas stacking: true = first on top), \
columnGap / rowGap (aliases of gap / crossGap), gridColumns, gridRows, columnTracks / rowTracks ([120 | \"1fr\" | \"auto\" | {sizing, value}]), \
gridColumn / gridRow (0-based cell anchor, null = auto), gridColumnSpan, gridRowSpan, gridAlignH / gridAlignV (a grid child's alignment in its cell: min|center|max, auto/null = the frame's), sizingH / sizingV (fixed|hug|fill), \
absolute (ignore auto layout; follows constraints), minWidth, maxWidth, minHeight, maxHeight (null removes), constraints {horizontal, vertical}, grids, exports, characters, fontFamily, fontStyle, \
fontSize, lineHeight (auto|px|\"150%\"), letterSpacing (px|\"2%\"), textAlign (left|center|right|justified), textAlignVertical, textAutoResize \
(widthAndHeight|height|none|truncate), textDecoration, textDecorationStyle (solid|dotted|wavy), textDecorationThickness / textDecorationOffset \
(auto|px|\"10%\" of the font size; the offset runs from the baseline down to an underline's top edge), textDecorationSkipInk (underlines break around descenders), \
textDecorationColor (colour, or auto/null: the text's), textCase (original|upper|lower|title|smallCaps), paragraphSpacing, paragraphIndent, \
listStyle (none|bulleted|numbered; with start/end: the paragraphs those chars touch), indentation (list nesting level 0-8, same range rule), \
listSpacing (px between list items, in place of paragraphSpacing), hangingPunctuation, hangingList (markers outside the box), leadingTrim (vertical trim: none|capHeight), maxLines, features ({tag: bool}, e.g. {\"liga\": false, \"tnum\": true}), \
fontVariations / variationSettings ({axis: number}, e.g. {\"wght\": 550, \"wdth\": 85}; null = none), pointCount, innerRadius, arcStart, arcSweep, \
arcInner, booleanOperation (a boolean group's operation, or an operand's own; null: the group's), operandOperation (a layer in a boolean \
group: how it combines with the operands below it; null: the group's), constrainProportions, description, interactions, overflow, pluginData {k: v}";

/// CRLF, bare CR, U+2028 and U+2029 become LF, the only line ending text layout understands (editor insertion does the same).
pub(crate) fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace(['\r', '\u{2028}', '\u{2029}'], "\n")
}

/// camelCase object keys → snake_case (accept the inspect/camel forms for serde-backed values).
pub fn snake(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let mut out = String::new();
                    for c in k.chars() {
                        if c.is_ascii_uppercase() {
                            out.push('_');
                            out.push(c.to_ascii_lowercase());
                        } else {
                            out.push(c);
                        }
                    }
                    // Paint/effect type tags stay as they are.
                    let key = if matches!(k.as_str(), "type" | "kind") { k.clone() } else { out };
                    (key, if matches!(k.as_str(), "type" | "kind") { v.clone() } else { snake(v) })
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(snake).collect()),
        other => other.clone(),
    }
}

pub(crate) fn enum_of<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
    let s = v.as_str()?;
    // Accept camelCase, snake/upper-case and spaced variants.
    let norm: String = {
        let mut out = String::new();
        let mut up = false;
        for (i, ch) in s.trim().chars().enumerate() {
            if ch == '_' || ch == '-' || ch == ' ' {
                up = true;
                continue;
            }
            if i == 0 {
                out.extend(ch.to_lowercase());
            } else if up {
                out.extend(ch.to_uppercase());
                up = false;
            } else if s.chars().all(|c| !c.is_lowercase() || !c.is_alphabetic()) {
                out.extend(ch.to_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    };
    serde_json::from_value(Value::String(norm)).ok()
}

pub(crate) fn sizing(v: &Value) -> Option<Sizing> {
    match v.as_str()?.to_ascii_lowercase().as_str() {
        "fixed" => Some(Sizing::Fixed),
        "hug" | "auto" | "hug contents" => Some(Sizing::Hug),
        "fill" | "fill container" | "stretch" => Some(Sizing::Fill),
        _ => None,
    }
}

pub(crate) fn line_height(v: &Value) -> Option<LineHeight> {
    match v {
        Value::String(s) if s.eq_ignore_ascii_case("auto") => Some(LineHeight::Auto),
        Value::String(s) if s.trim().ends_with('%') => s.trim().trim_end_matches('%').trim().parse().ok().map(LineHeight::Percent),
        Value::Object(_) => serde_json::from_value(v.clone()).ok(),
        other => f64_of(other).map(LineHeight::Pixels),
    }
}

pub(crate) fn letter_spacing(v: &Value) -> Option<LetterSpacing> {
    match v {
        Value::String(s) if s.trim().ends_with('%') => s.trim().trim_end_matches('%').trim().parse().ok().map(LetterSpacing::Percent),
        Value::Object(_) => serde_json::from_value(v.clone()).ok(),
        other => f64_of(other).map(LetterSpacing::Pixels),
    }
}

fn four(v: &Value) -> Option<[f64; 4]> {
    match v {
        Value::Array(a) => {
            let f: Vec<f64> = a.iter().filter_map(f64_of).collect();
            match f.as_slice() {
                [x] => Some([*x; 4]),
                [a, b] => Some([*a, *b, *a, *b]),
                [a, b, c, d] => Some([*a, *b, *c, *d]),
                _ => None,
            }
        }
        other => f64_of(other).map(|x| [x; 4]),
    }
}

const BOOL_OP_HELP: &str = "booleanOperation / operandOperation: union|subtract|intersect|exclude (null: the group's, for operands)";

/// An operand's own boolean operation (`null`: its group's).
fn set_operand_op(doc: &mut Document, id: NodeId, v: &Value, cmd: &str) -> Result<()> {
    if crate::booleans::group_of(doc, id).is_none() {
        return Err(bad(cmd, "not a boolean group or a layer in one"));
    }
    doc.get_mut(id)?.bool_op = if v.is_null() { None } else { Some(enum_of(v).ok_or_else(|| bad(cmd, BOOL_OP_HELP))?) };
    Ok(())
}

pub(crate) const WIDTH_PROFILE_HELP: &str = "widthProfile: a preset (uniform|wedge|taper|eye|rounded|pinch), points [[t (0-1 along the path), scale (× stroke weight)] | [t, left, right] (one-sided: each side's scale, looking along the path), ...] or [{t, scale} | {t, left, right}], {preset} | {points}, or null (uniform)";

pub(crate) const BRUSH_HELP: &str = "brush: a preset (taper|ink|ribbon|bristle: stretched; dots|stars|confetti|leaves: scattered), {preset, spacing? (× weight), sizeJitter? (0-1), rotationJitter? (0-1), scatter? (0-4 × weight), seed?}, or null (a plain stroke)";

/// A brush from a preset id, `{preset, spacing?, sizeJitter?, rotationJitter?, scatter?, seed?}`
/// or null. `Some(None)` = no brush; `None` = not a brush (or an unknown preset).
pub fn brush_of(v: &Value) -> Option<Option<Brush>> {
    let b = match v {
        Value::Null => return Some(None),
        Value::String(id) => Brush::new(id.as_str()),
        Value::Object(_) => serde_json::from_value::<Brush>(snake(v)).ok()?,
        _ => return None,
    };
    figcraft_pathops::brush::preset(&b.preset).map(|_| Some(b.sanitized()))
}

/// A width profile from a preset name, points (`[[t, scale] | [t, left, right], ...]` or
/// `[{t, scale} | {t, left, right}]`), `{preset}` / `{points}`, or null. `Some(None)` = uniform;
/// `None` = not a profile.
pub fn width_profile_of(v: &Value) -> Option<Option<WidthProfile>> {
    match v {
        Value::Null => Some(None),
        Value::String(s) => WidthPreset::from_id(s).map(WidthPreset::profile),
        Value::Array(a) => {
            let num = |x: Option<&Value>| x.and_then(f64_of);
            let points = a
                .iter()
                .map(|p| match p {
                    Value::Array(xy) => match (num(xy.first()), num(xy.get(1)), num(xy.get(2))) {
                        (Some(t), Some(l), Some(r)) => Some(WidthPoint::from_sides(t, l, r)),
                        (Some(t), Some(s), None) => Some(WidthPoint::new(t, s)),
                        _ => None,
                    },
                    Value::Object(o) => {
                        let t = num(o.get("t"))?;
                        match (num(o.get("left")), num(o.get("right"))) {
                            (Some(l), Some(r)) => Some(WidthPoint::from_sides(t, l, r)),
                            _ => Some(WidthPoint::new(t, num(o.get("scale").or_else(|| o.get("width")))?)),
                        }
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Some(WidthProfile { points }.sanitized()).filter(|p| !p.points.is_empty()))
        }
        Value::Object(o) => o.get("points").or_else(|| o.get("preset")).and_then(width_profile_of),
        _ => None,
    }
}

/// A width point as [`width_profile_of`] reads it back: `[t, scale]`, or `[t, left, right]` when
/// its sides differ.
pub fn width_point_json(p: &WidthPoint) -> Value {
    if p.symmetric() {
        json!([p.t, p.scale])
    } else {
        let (l, r) = p.sides();
        json!([p.t, l, r])
    }
}

/// A profile's points as [`width_profile_of`] reads them back (`null` without points).
pub fn width_points_json(profile: Option<&WidthProfile>) -> Value {
    match profile.filter(|p| !p.points.is_empty()) {
        Some(p) => Value::Array(p.points.iter().map(width_point_json).collect()),
        None => Value::Null,
    }
}

const TRACK_HELP: &str = "tracks: [px | \"120px\" | \"1fr\" | \"auto\" (hug) | {sizing: fixed|hug|fill, value}]";

/// One grid track from `120` / `"120px"` (fixed), `"2fr"` / `"fill"` (fill), `"auto"` / `"hug"`,
/// or `{sizing, value}`.
pub fn track_of(v: &Value) -> Option<GridTrack> {
    match v {
        Value::Number(_) => f64_of(v).map(|px| GridTrack::fixed(px.clamp(0.0, 1e6))),
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            match s.as_str() {
                "auto" | "hug" => Some(GridTrack::hug()),
                "fill" => Some(GridTrack::fill(1.0)),
                _ => match s.strip_suffix("fr") {
                    Some(fr) => fr.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| GridTrack::fill(f.clamp(0.0, 1e4))),
                    None => f64_of(&Value::String(s)).map(|px| GridTrack::fixed(px.clamp(0.0, 1e6))),
                },
            }
        }
        Value::Object(o) => {
            let s = o.get("sizing").or_else(|| o.get("type")).and_then(sizing)?;
            let val = o.get("value").or_else(|| o.get("size")).and_then(f64_of);
            Some(match s {
                Sizing::Fixed => GridTrack::fixed(val?.clamp(0.0, 1e6)),
                Sizing::Hug => GridTrack::hug(),
                Sizing::Fill => GridTrack::fill(val.unwrap_or(1.0).clamp(0.0, 1e4)),
            })
        }
        _ => None,
    }
}

/// Set one min/max limit (`None` removes it), keeping min ≤ max by moving the other limit.
pub fn set_limit(cl: &mut ChildLayout, key: &str, val: Option<f64>) {
    let (target, other, is_min) = match key {
        "minWidth" => (&mut cl.min_w, &mut cl.max_w, true),
        "maxWidth" => (&mut cl.max_w, &mut cl.min_w, false),
        "minHeight" => (&mut cl.min_h, &mut cl.max_h, true),
        _ => (&mut cl.max_h, &mut cl.min_h, false),
    };
    *target = val;
    if let (Some(v), Some(o)) = (val, *other)
        && (is_min && o < v || !is_min && o > v)
    {
        *other = Some(v);
    }
}

fn text_mut<'a>(cmd: &str, n: &'a mut Node) -> Result<&'a mut TextData> {
    let id = n.id;
    n.kind.text_mut().ok_or_else(|| bad(cmd, format!("node {id} is not text")))
}

fn ensure_layout(n: &mut Node) -> &mut AutoLayout {
    n.layout.get_or_insert_with(|| AutoLayout { padding: [0.0; 4], gap: 0.0, ..Default::default() })
}

/// The keys that set a frame's auto layout (through [`layout_mut`]).
pub const LAYOUT_KEYS: &[&str] = &[
    "gap",
    "itemSpacing",
    "columnGap",
    "crossGap",
    "counterAxisSpacing",
    "rowGap",
    "padding",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "paddingHorizontal",
    "paddingVertical",
    "mainAlign",
    "primaryAxisAlignItems",
    "crossAlign",
    "counterAxisAlignItems",
    "contentAlign",
    "counterAxisAlignContent",
    "wrap",
    "gridColumns",
    "gridRows",
    "strokesIncluded",
    "reverseZ",
    "columnTracks",
    "gridColumnTracks",
    "rowTracks",
    "gridRowTracks",
];

/// The auto layout of a frame-like node, created when it has none yet. Other nodes can't hold an
/// auto layout: an auto layout key on them is refused, not stored where nothing reads it
/// (`node.set` on several layers skips them instead, see `edit::set`).
fn layout_mut<'a>(cmd: &str, k: &str, n: &'a mut Node) -> Result<&'a mut AutoLayout> {
    if !n.kind.is_frame_like() {
        return Err(bad(
            cmd,
            format!(
                "`{k}` is an auto layout property, and layer {} “{}” is a {} (only frames, components and instances have auto layout)",
                n.id,
                n.name,
                n.kind.type_name()
            ),
        ));
    }
    Ok(ensure_layout(n))
}

/// Apply a property map to one node. `x`/`y` are relative to the parent.
pub fn apply(cmd: &str, doc: &mut Document, id: NodeId, props: &Map<String, Value>) -> Result<()> {
    let mut unknown = Vec::new();
    // Geometry keys first so rotation applies to the final size.
    let order = |k: &str| match k {
        "width" | "height" | "w" | "h" => 0,
        "rotation" => 2,
        "x" | "y" => 3,
        _ => 1,
    };
    let mut keys: Vec<(&String, &Value)> = props.iter().collect();
    keys.sort_by_key(|(k, _)| order(k));
    for (k, v) in keys {
        let k = k.as_str();
        match k {
            "x" | "y" => {
                let val = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?;
                // X/Y are the node's own origin (its rotated top-left for rotated nodes), in the
                // frame it is in (groups have no space of their own).
                let n = doc.get(id)?;
                let group = n.parent.filter(|p| doc.node(*p).is_some_and(|pn| pn.kind.bounds_follow_children()));
                let space = group.map_or(Affine::IDENTITY, |g| doc.frame_relative(g));
                let c = (space * n.transform).as_coeffs();
                let (mut tx, mut ty) = (c[4], c[5]);
                if k == "x" {
                    tx = val;
                } else {
                    ty = val;
                }
                doc.get_mut(id)?.transform = space.inverse() * Affine::new([c[0], c[1], c[2], c[3], tx, ty]);
            }
            "width" | "height" | "w" | "h" => {
                let val = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.max(0.0);
                let is_w = k.starts_with('w');
                // With the proportions locked (the Layout panel's link) and only one side set, the
                // other side follows the node's ratio; both sides in one `node.set` stay as typed.
                let (nw, nh) = {
                    let n = doc.get(id)?;
                    let (ow, oh) = (n.width, n.height);
                    let (other, other_short) = if is_w { ("height", "h") } else { ("width", "w") };
                    let linked = n.constrain_proportions && !props.contains_key(other) && !props.contains_key(other_short) && ow > 1e-9 && oh > 1e-9;
                    let derived = if is_w { oh * val / ow } else { ow * val / oh };
                    match (is_w, linked && derived.is_finite()) {
                        (true, true) => (val, derived),
                        (false, true) => (derived, val),
                        (true, false) => (val, oh),
                        (false, false) => (ow, val),
                    }
                };
                resize_node(doc, id, nw, nh)?;
                // Explicit size makes text fixed in that dimension.
                if let Some(t) = doc.get_mut(id)?.kind.text_mut() {
                    t.auto_resize = if is_w {
                        if t.auto_resize == TextAutoResize::WidthAndHeight { TextAutoResize::Height } else { t.auto_resize }
                    } else {
                        TextAutoResize::None
                    };
                }
                // Explicit width/height makes hugging/filling fixed (like typing into W/H).
                let n = doc.get_mut(id)?;
                if is_w {
                    n.child_layout.sizing_h = Sizing::Fixed;
                } else {
                    n.child_layout.sizing_v = Sizing::Fixed;
                }
            }
            "rotation" => {
                let deg = f64_of(v).ok_or_else(|| bad(cmd, "`rotation` must be degrees"))?;
                let n = doc.get_mut(id)?;
                n.transform = figcraft_geom::set_rotation_about_center(n.transform, n.width, n.height, deg);
            }
            "name" => doc.get_mut(id)?.name = string_val(v).ok_or_else(|| bad(cmd, "`name` must be a string"))?,
            "visible" => doc.get_mut(id)?.visible = v.as_bool().ok_or_else(|| bad(cmd, "`visible` must be a boolean"))?,
            "locked" => doc.get_mut(id)?.locked = v.as_bool().ok_or_else(|| bad(cmd, "`locked` must be a boolean"))?,
            "opacity" => {
                let o = f64_of(v).ok_or_else(|| bad(cmd, "`opacity` must be a number"))?;
                let o = if o > 1.0 { o / 100.0 } else { o };
                doc.get_mut(id)?.opacity = o.clamp(0.0, 1.0) as f32;
            }
            "blend" | "blendMode" => {
                let b = v.as_str().and_then(figcraft_color::BlendMode::from_name).ok_or_else(|| bad(cmd, "unknown blend mode"))?;
                doc.get_mut(id)?.blend = b;
            }
            "fills" => doc.get_mut(id)?.fills = paints_of(v).ok_or_else(|| bad(cmd, "`fills` must be paints or colours"))?,
            "fill" => {
                let c = color_of(v).ok_or_else(|| bad(cmd, "`fill` must be a colour"))?;
                doc.get_mut(id)?.fills = vec![Paint::solid(c)];
            }
            "strokes" => doc.get_mut(id)?.strokes = paints_of(v).ok_or_else(|| bad(cmd, "`strokes` must be paints or colours"))?,
            "stroke" => {
                let c = color_of(v).ok_or_else(|| bad(cmd, "`stroke` must be a colour"))?;
                doc.get_mut(id)?.strokes = vec![Paint::solid(c)];
            }
            "strokeWeight" => doc.get_mut(id)?.stroke.weight = f64_of(v).ok_or_else(|| bad(cmd, "`strokeWeight` must be a number"))?.max(0.0),
            "strokeAlign" => doc.get_mut(id)?.stroke.align = enum_of(v).ok_or_else(|| bad(cmd, "strokeAlign: inside|center|outside"))?,
            "strokeSides" => {
                doc.get_mut(id)?.stroke.sides = if v.is_null() { None } else { Some(four(v).ok_or_else(|| bad(cmd, "strokeSides: [t,r,b,l]"))?) }
            }
            "strokeTopWeight" | "strokeRightWeight" | "strokeBottomWeight" | "strokeLeftWeight" => {
                let i = STROKE_SIDES.iter().position(|x| *x == k).unwrap_or(0);
                let w = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.max(0.0);
                let st = &mut doc.get_mut(id)?.stroke;
                // One side set on its own turns on individual strokes, the others at the weight.
                let mut sides = st.sides.unwrap_or([st.weight; 4]);
                if let Some(slot) = sides.get_mut(i) {
                    *slot = w;
                }
                st.sides = Some(sides);
            }
            "strokeCap" => {
                let c: StrokeCap = enum_of(v).ok_or_else(|| bad(cmd, "unknown stroke cap"))?;
                let n = doc.get_mut(id)?;
                n.stroke.cap_start = c;
                n.stroke.cap_end = c;
            }
            "startCap" => doc.get_mut(id)?.stroke.cap_start = enum_of(v).ok_or_else(|| bad(cmd, "unknown stroke cap"))?,
            "endCap" => doc.get_mut(id)?.stroke.cap_end = enum_of(v).ok_or_else(|| bad(cmd, "unknown stroke cap"))?,
            "strokeJoin" => doc.get_mut(id)?.stroke.join = enum_of(v).ok_or_else(|| bad(cmd, "strokeJoin: miter|bevel|round"))?,
            "miterLimit" => doc.get_mut(id)?.stroke.miter_limit = f64_of(v).unwrap_or(4.0).clamp(1.0, 100.0),
            "miterAngle" => {
                let deg = f64_of(v).ok_or_else(|| bad(cmd, "`miterAngle` must be degrees"))?;
                doc.get_mut(id)?.stroke.miter_limit = StrokeProps::miter_limit_for(deg);
            }
            "dashes" => {
                let d: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(f64_of).map(|x| x.max(0.0)).collect()).unwrap_or_default();
                doc.get_mut(id)?.stroke.dashes = d;
            }
            "widthProfile" => doc.get_mut(id)?.stroke.width_profile = width_profile_of(v).ok_or_else(|| bad(cmd, WIDTH_PROFILE_HELP))?,
            "brush" | "strokeBrush" => doc.get_mut(id)?.stroke.brush = brush_of(v).ok_or_else(|| bad(cmd, BRUSH_HELP))?,
            "cornerRadius" => {
                let r = four(v).ok_or_else(|| bad(cmd, "cornerRadius: n or [tl,tr,br,bl]"))?.map(|x| x.max(0.0));
                let n = doc.get_mut(id)?;
                n.corners.radii = r;
                // A vector's corner radius rounds every point (edit mode then sets them one by one).
                if let NodeKind::Vector { network } = &mut n.kind {
                    network.vertices.iter_mut().for_each(|p| p.radius = r[0]);
                    crate::vectoredit::refit(doc, id)?;
                }
            }
            "topLeftRadius" | "topRightRadius" | "bottomRightRadius" | "bottomLeftRadius" => {
                let i = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"].iter().position(|x| *x == k).unwrap_or(0);
                let r = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.max(0.0);
                if let Some(slot) = doc.get_mut(id)?.corners.radii.get_mut(i) {
                    *slot = r;
                }
            }
            "cornerSmoothing" => doc.get_mut(id)?.corners.smoothing = f64_of(v).unwrap_or(0.0).clamp(0.0, 1.0),
            "effects" => doc.get_mut(id)?.effects = serde_json::from_value(snake(v)).map_err(|e| bad(cmd, format!("effects: {e}")))?,
            "clip" | "clipsContent" => doc.get_mut(id)?.clip = v.as_bool().ok_or_else(|| bad(cmd, "`clip` must be a boolean"))?,
            "isMask" => doc.get_mut(id)?.is_mask = v.as_bool().unwrap_or(false),
            "devStatus" => {
                let st = if v.is_null() {
                    Some(DevStatus::None)
                } else {
                    v.as_str().and_then(|s| DevStatus::ALL.into_iter().find(|d| d.id().eq_ignore_ascii_case(s)))
                };
                let n = doc.get_mut(id)?;
                n.dev_status = st.ok_or_else(|| bad(cmd, "devStatus: none|readyForDev|completed"))?;
                // Marking a layer (again) starts from its current contents.
                n.dev_changed = false;
            }
            "maskType" => {
                let t = v.as_str().and_then(|s| MaskType::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s)));
                doc.get_mut(id)?.mask_type = t.ok_or_else(|| bad(cmd, "maskType: alpha|vector|luminance"))?;
            }
            "layout" | "layoutMode" => {
                let n = doc.get_mut(id)?;
                if !n.kind.is_frame_like() {
                    return Err(bad(cmd, "auto layout needs a frame"));
                }
                match v {
                    Value::Null => n.layout = None,
                    Value::String(s) if s.eq_ignore_ascii_case("none") => n.layout = None,
                    Value::String(s) => {
                        let mode = match s.to_ascii_lowercase().as_str() {
                            "horizontal" | "row" => LayoutMode::Horizontal,
                            "vertical" | "column" => LayoutMode::Vertical,
                            "grid" => LayoutMode::Grid,
                            _ => return Err(bad(cmd, "layout: none|horizontal|vertical|grid")),
                        };
                        ensure_layout(n).mode = mode;
                    }
                    Value::Object(_) => n.layout = Some(serde_json::from_value(snake(v)).map_err(|e| bad(cmd, format!("layout: {e}")))?),
                    _ => return Err(bad(cmd, "layout: none|horizontal|vertical|grid|{...}")),
                }
            }
            "gap" | "itemSpacing" | "columnGap" => {
                layout_mut(cmd, k, doc.get_mut(id)?)?.gap = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?
            }
            "crossGap" | "counterAxisSpacing" | "rowGap" => {
                layout_mut(cmd, k, doc.get_mut(id)?)?.cross_gap = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?
            }
            "padding" => {
                let p = four(v).ok_or_else(|| bad(cmd, "padding: n or [t,r,b,l]"))?;
                layout_mut(cmd, k, doc.get_mut(id)?)?.padding = p.map(|x| x.max(0.0));
            }
            "paddingTop" | "paddingRight" | "paddingBottom" | "paddingLeft" => {
                let i = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"].iter().position(|x| *x == k).unwrap_or(0);
                let val = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.max(0.0);
                if let Some(slot) = layout_mut(cmd, k, doc.get_mut(id)?)?.padding.get_mut(i) {
                    *slot = val;
                }
            }
            "paddingHorizontal" => {
                let val = f64_of(v).unwrap_or(0.0).max(0.0);
                let l = layout_mut(cmd, k, doc.get_mut(id)?)?;
                l.padding[1] = val;
                l.padding[3] = val;
            }
            "paddingVertical" => {
                let val = f64_of(v).unwrap_or(0.0).max(0.0);
                let l = layout_mut(cmd, k, doc.get_mut(id)?)?;
                l.padding[0] = val;
                l.padding[2] = val;
            }
            "mainAlign" | "primaryAxisAlignItems" => {
                layout_mut(cmd, k, doc.get_mut(id)?)?.main_align =
                    enum_of(v).ok_or_else(|| bad(cmd, "mainAlign: min|center|max|spaceBetween|spaceAround|spaceEvenly"))?
            }
            "crossAlign" | "counterAxisAlignItems" => {
                layout_mut(cmd, k, doc.get_mut(id)?)?.cross_align = enum_of(v).ok_or_else(|| bad(cmd, "crossAlign: min|center|max|baseline"))?
            }
            "contentAlign" | "counterAxisAlignContent" => {
                layout_mut(cmd, k, doc.get_mut(id)?)?.content_align = enum_of(v).ok_or_else(|| bad(cmd, "contentAlign: auto|spaceBetween"))?
            }
            "wrap" => layout_mut(cmd, k, doc.get_mut(id)?)?.wrap = v.as_bool().unwrap_or(false),
            "gridColumns" | "gridRows" => {
                let count = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.clamp(1.0, 1000.0) as u32;
                let l = layout_mut(cmd, k, doc.get_mut(id)?)?;
                // Tracks past the new count go away (new tracks are 1fr).
                let (n, tracks) =
                    if k == "gridColumns" { (&mut l.grid_columns, &mut l.column_tracks) } else { (&mut l.grid_rows, &mut l.row_tracks) };
                *n = count;
                tracks.truncate(count as usize);
            }
            "strokesIncluded" => layout_mut(cmd, k, doc.get_mut(id)?)?.strokes_included = v.as_bool().unwrap_or(false),
            "reverseZ" => layout_mut(cmd, k, doc.get_mut(id)?)?.reverse_z = v.as_bool().unwrap_or(false),
            "sizingH" | "layoutSizingHorizontal" => {
                let s = sizing(v).ok_or_else(|| bad(cmd, "sizing: fixed|hug|fill"))?;
                set_sizing(cmd, doc, id, s, true)?;
            }
            "sizingV" | "layoutSizingVertical" => {
                let s = sizing(v).ok_or_else(|| bad(cmd, "sizing: fixed|hug|fill"))?;
                set_sizing(cmd, doc, id, s, false)?;
            }
            "absolute" | "ignoreAutoLayout" => {
                let on = bool_of(v).ok_or_else(|| bad(cmd, "`absolute` must be a boolean"))?;
                let cl = &mut doc.get_mut(id)?.child_layout;
                cl.absolute = on;
                // An absolute child can't fill: it keeps its current size.
                if on {
                    for s in [&mut cl.sizing_h, &mut cl.sizing_v] {
                        if *s == Sizing::Fill {
                            *s = Sizing::Fixed;
                        }
                    }
                }
            }
            "minWidth" | "maxWidth" | "minHeight" | "maxHeight" => {
                let val = match v {
                    Value::Null => None,
                    other => Some(f64_of(other).ok_or_else(|| bad(cmd, format!("`{k}` must be a number or null")))?.clamp(0.0, 1e7)),
                };
                set_limit(&mut doc.get_mut(id)?.child_layout, k, val);
            }
            "columnTracks" | "gridColumnTracks" | "rowTracks" | "gridRowTracks" => {
                let a = v.as_array().ok_or_else(|| bad(cmd, format!("`{k}` must be an array of tracks")))?;
                let tracks = a.iter().map(track_of).collect::<Option<Vec<_>>>().ok_or_else(|| bad(cmd, TRACK_HELP))?;
                let l = layout_mut(cmd, k, doc.get_mut(id)?)?;
                if k.contains("olumn") {
                    l.grid_columns = (tracks.len() as u32).clamp(1, 1000);
                    l.column_tracks = tracks;
                } else {
                    l.grid_rows = (tracks.len() as u32).clamp(1, 1000);
                    l.row_tracks = tracks;
                }
            }
            "gridColumn" | "gridColumnAnchorIndex" | "gridRow" | "gridRowAnchorIndex" => {
                let val = match v {
                    Value::Null => None,
                    Value::String(s) if s.eq_ignore_ascii_case("auto") => None,
                    other => {
                        Some(f64_of(other).ok_or_else(|| bad(cmd, format!("`{k}` must be a 0-based index, \"auto\" or null")))?.clamp(0.0, 999.0)
                            as u32)
                    }
                };
                let cl = &mut doc.get_mut(id)?.child_layout;
                if k.starts_with("gridColumn") {
                    cl.grid_column = val;
                } else {
                    cl.grid_row = val;
                }
            }
            "gridAlignH" | "gridChildHorizontalAlign" | "gridAlignV" | "gridChildVerticalAlign" => {
                let val = match v {
                    Value::Null => None,
                    Value::String(s) if s.eq_ignore_ascii_case("auto") => None,
                    other => Some(enum_of(other).ok_or_else(|| bad(cmd, format!("`{k}`: min|center|max|auto")))?),
                };
                let cl = &mut doc.get_mut(id)?.child_layout;
                if k.contains('H') {
                    cl.grid_align_h = val;
                } else {
                    cl.grid_align_v = val;
                }
            }
            "gridColumnSpan" | "gridRowSpan" => {
                let span = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?.clamp(1.0, 1000.0) as u32;
                let cl = &mut doc.get_mut(id)?.child_layout;
                if k == "gridColumnSpan" {
                    cl.grid_column_span = span;
                } else {
                    cl.grid_row_span = span;
                }
            }
            "constraints" => {
                let n = doc.get_mut(id)?;
                if let Some(h) = v.get("horizontal") {
                    n.constraints.horizontal = enum_of(h).ok_or_else(|| bad(cmd, "horizontal: left|right|leftRight|center|scale"))?;
                }
                if let Some(vv) = v.get("vertical") {
                    n.constraints.vertical = enum_of(vv).ok_or_else(|| bad(cmd, "vertical: top|bottom|topBottom|center|scale"))?;
                }
            }
            "grids" | "layoutGrids" => doc.get_mut(id)?.grids = serde_json::from_value(snake(v)).map_err(|e| bad(cmd, format!("grids: {e}")))?,
            "exports" | "exportSettings" => {
                doc.get_mut(id)?.exports = serde_json::from_value(snake(v)).map_err(|e| bad(cmd, format!("exports: {e}")))?
            }
            "interactions" => {
                doc.get_mut(id)?.interactions = serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("interactions: {e}")))?
            }
            "overflow" => doc.get_mut(id)?.overflow = enum_of(v).ok_or_else(|| bad(cmd, "overflow: none|horizontal|vertical|both"))?,
            "scrollPosition" => doc.get_mut(id)?.scroll_position = enum_of(v).ok_or_else(|| bad(cmd, "scrollPosition: scrolls|fixed|sticky"))?,
            "constrainProportions" => doc.get_mut(id)?.constrain_proportions = v.as_bool().unwrap_or(false),
            "characters" | "text" => {
                let s = string_val(v).ok_or_else(|| bad(cmd, "`characters` must be a string"))?;
                let n = doc.get_mut(id)?;
                let t = text_mut(cmd, n)?;
                let len = t.char_count();
                t.replace_range(0, len, &normalize_line_endings(&s));
                n.overrides.insert("characters".into());
            }
            "fontFamily"
            | "fontStyle"
            | "fontWeight"
            | "fontSize"
            | "lineHeight"
            | "letterSpacing"
            | "textDecoration"
            | "textCase"
            | "hyperlink"
            | "textFills"
            | "fontVariations"
            | "variationSettings"
            | "textDecorationStyle"
            | "textDecorationThickness"
            | "textDecorationOffset"
            | "textDecorationSkipInk"
            | "textDecorationColor" => {
                let n = doc.get_mut(id)?;
                let t = text_mut(cmd, n)?;
                let (start, end) = range_of(props, t.char_count());
                apply_text_style(cmd, t, start, end, k, v)?;
            }
            "textAlign" | "textAlignHorizontal" => {
                text_mut(cmd, doc.get_mut(id)?)?.align_h = enum_of(v).ok_or_else(|| bad(cmd, "textAlign: left|center|right|justified"))?
            }
            "textAlignVertical" => {
                text_mut(cmd, doc.get_mut(id)?)?.align_v = enum_of(v).ok_or_else(|| bad(cmd, "textAlignVertical: top|middle|bottom"))?
            }
            "textAutoResize" => {
                text_mut(cmd, doc.get_mut(id)?)?.auto_resize =
                    enum_of(v).ok_or_else(|| bad(cmd, "textAutoResize: widthAndHeight|height|none|truncate"))?
            }
            "paragraphSpacing" => text_mut(cmd, doc.get_mut(id)?)?.paragraph_spacing = f64_of(v).unwrap_or(0.0).max(0.0),
            "paragraphIndent" => text_mut(cmd, doc.get_mut(id)?)?.paragraph_indent = f64_of(v).unwrap_or(0.0).max(0.0),
            "listStyle" | "indentation" => {
                let t = text_mut(cmd, doc.get_mut(id)?)?;
                let (start, end) = range_of(props, t.char_count());
                if k == "listStyle" {
                    let l: ListStyle = enum_of(v).ok_or_else(|| bad(cmd, "listStyle: none|bulleted|numbered"))?;
                    t.set_para_lists(start, end, |p| p.list = l);
                } else {
                    let d = f64_of(v).filter(|d| d.is_finite()).ok_or_else(|| bad(cmd, "indentation: a list level 0-8"))?;
                    t.set_para_lists(start, end, |p| p.indent = d.clamp(0.0, f64::from(MAX_LIST_INDENT)) as u8);
                }
            }
            "listSpacing" => text_mut(cmd, doc.get_mut(id)?)?.list_spacing = f64_of(v).filter(|s| s.is_finite()).unwrap_or(0.0).clamp(0.0, 10_000.0),
            "hangingPunctuation" | "hangingList" => {
                let on = v.as_bool().ok_or_else(|| bad(cmd, format!("`{k}` must be true or false")))?;
                let t = text_mut(cmd, doc.get_mut(id)?)?;
                *if k == "hangingList" { &mut t.hanging_list } else { &mut t.hanging_punctuation } = on;
            }
            "leadingTrim" => text_mut(cmd, doc.get_mut(id)?)?.leading_trim = enum_of(v).ok_or_else(|| bad(cmd, "leadingTrim: none|capHeight"))?,
            "maxLines" => text_mut(cmd, doc.get_mut(id)?)?.max_lines = f64_of(v).unwrap_or(0.0).clamp(0.0, 10_000.0) as u32,
            "features" | "textFeatures" | "fontFeatures" | "openTypeFeatures" => {
                let f = features_of(cmd, v)?;
                let t = text_mut(cmd, doc.get_mut(id)?)?;
                t.features.clear();
                for (k, on) in f {
                    if let Some(on) = on {
                        t.features.insert(k, on);
                    }
                }
            }
            "start" | "end" => {}
            "pointCount" => {
                let c = f64_of(v).ok_or_else(|| bad(cmd, "`pointCount` must be a number"))?.round();
                match &mut doc.get_mut(id)?.kind {
                    // Stars have 3 to 60 points; polygons keep the geometry's 120 (silent).
                    NodeKind::Star { count, .. } => *count = c.clamp(3.0, 60.0) as u32,
                    NodeKind::Polygon { count } => *count = c.clamp(3.0, 120.0) as u32,
                    _ => return Err(bad(cmd, "pointCount applies to polygons and stars")),
                }
            }
            "innerRadius" | "ratio" => {
                let r = f64_of(v).unwrap_or(0.38);
                let r = if r > 1.0 { r / 100.0 } else { r };
                match &mut doc.get_mut(id)?.kind {
                    NodeKind::Star { inner_ratio, .. } => *inner_ratio = r.clamp(0.0, 1.0),
                    _ => return Err(bad(cmd, "innerRadius applies to stars")),
                }
            }
            "arcStart" | "arcSweep" | "arcInner" => {
                let val = f64_of(v).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))?;
                match &mut doc.get_mut(id)?.kind {
                    NodeKind::Ellipse { start, sweep, inner } => match k {
                        "arcStart" => *start = val.to_radians(),
                        "arcSweep" => *sweep = val.clamp(-360.0, 360.0).to_radians(),
                        _ => *inner = (if val > 1.0 { val / 100.0 } else { val }).clamp(0.0, 0.99),
                    },
                    _ => return Err(bad(cmd, "arc applies to ellipses")),
                }
            }
            "description" => match &mut doc.get_mut(id)?.kind {
                NodeKind::Component { description, .. } | NodeKind::ComponentSet { description } => *description = string_val(v).unwrap_or_default(),
                _ => return Err(bad(cmd, "description applies to components")),
            },
            "pluginData" => {
                let n = doc.get_mut(id)?;
                if let Some(o) = v.as_object() {
                    for (pk, pv) in o {
                        if pv.is_null() {
                            n.plugin_data.remove(pk);
                        } else {
                            n.plugin_data.insert(pk.clone(), string_val(pv).unwrap_or_else(|| pv.to_string()));
                        }
                    }
                }
            }
            "fillStyle" | "strokeStyle" | "effectStyle" | "textStyle" | "gridStyle" => {
                let sid = string_val(v);
                crate::cmd::styles::apply_style_ref(doc, id, k, sid.as_deref())?;
            }
            "boundVariables" => {
                let n = doc.get_mut(id)?;
                if let Some(o) = v.as_object() {
                    for (field, var) in o {
                        match var.as_str() {
                            Some(vid) => {
                                n.bound.insert(field.clone(), vid.to_string());
                            }
                            None => {
                                n.bound.remove(field);
                            }
                        }
                    }
                }
            }
            "variableModes" => {
                let n = doc.get_mut(id)?;
                if let Some(o) = v.as_object() {
                    for (coll, mode) in o {
                        match mode.as_str() {
                            Some(m) => {
                                n.var_modes.insert(coll.clone(), m.to_string());
                            }
                            None => {
                                n.var_modes.remove(coll);
                            }
                        }
                    }
                }
            }
            "booleanOperation" if !matches!(doc.get(id)?.kind, NodeKind::BooleanOp { .. }) => set_operand_op(doc, id, v, cmd)?,
            "booleanOperation" => {
                if let NodeKind::BooleanOp { op } = &mut doc.get_mut(id)?.kind {
                    *op = enum_of(v).ok_or_else(|| bad(cmd, BOOL_OP_HELP))?;
                }
            }
            "operandOperation" => set_operand_op(doc, id, v, cmd)?,
            "sectionContentsHidden" => {
                if let NodeKind::Section { contents_hidden } = &mut doc.get_mut(id)?.kind {
                    *contents_hidden = v.as_bool().unwrap_or(false);
                }
            }
            "background" => {
                let c = color_of(v).ok_or_else(|| bad(cmd, "`background` must be a colour"))?;
                match &mut doc.get_mut(id)?.kind {
                    NodeKind::Page { background, .. } => *background = c,
                    _ => return Err(bad(cmd, "background applies to pages")),
                }
            }
            "componentProperties" => {
                if let NodeKind::Instance { props: p, .. } = &mut doc.get_mut(id)?.kind
                    && let Some(o) = v.as_object()
                {
                    for (pk, pv) in o {
                        p.insert(pk.clone(), pv.clone());
                    }
                }
            }
            other => unknown.push(other.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(bad(cmd, format!("unknown properties: {} (known: {KEYS})", unknown.join(", "))));
    }
    // Inside an instance, record which property groups were overridden (an instance's own
    // resizing, export settings and interactions are its own, not overrides).
    let own_root = matches!(doc.get(id)?.kind, NodeKind::Instance { .. }) && !crate::components::inside_instance(doc, id);
    let n = doc.get_mut(id)?;
    if n.source.is_some() {
        for g in
            props.keys().map(|k| override_group(k)).filter(|g| *g != "other" && !(own_root && matches!(*g, "resizing" | "exports" | "interactions")))
        {
            n.overrides.insert(g.to_string());
        }
    }
    Ok(())
}

/// The override group a property key belongs to.
pub fn override_group(k: &str) -> &'static str {
    match k {
        "fills" | "fill" => "fills",
        "strokes" | "stroke" | "strokeWeight" | "strokeAlign" | "dashes" | "strokeCap" | "startCap" | "endCap" | "strokeJoin" | "strokeSides"
        | "strokeTopWeight" | "strokeRightWeight" | "strokeBottomWeight" | "strokeLeftWeight" | "miterLimit" | "miterAngle" | "widthProfile"
        | "brush" | "strokeBrush" => "strokes",
        "effects" => "effects",
        "opacity" | "blend" | "blendMode" => "opacity",
        "visible" => "visible",
        "name" => "name",
        "characters" | "text" => "characters",
        "features" | "fontFeatures" | "openTypeFeatures" | "listStyle" | "indentation" | "listSpacing" | "maxLines" | "variationSettings"
        | "hangingPunctuation" | "hangingList" => "textStyle",
        "cornerRadius" | "cornerSmoothing" | "topLeftRadius" | "topRightRadius" | "bottomRightRadius" | "bottomLeftRadius" => "corners",
        "width" | "height" | "w" | "h" => "size",
        "sizingH" | "sizingV" | "layoutSizingHorizontal" | "layoutSizingVertical" | "minWidth" | "maxWidth" | "minHeight" | "maxHeight" => "resizing",
        "gap" | "itemSpacing" | "columnGap" | "crossGap" | "counterAxisSpacing" | "rowGap" => "gap",
        "padding" | "paddingTop" | "paddingRight" | "paddingBottom" | "paddingLeft" | "paddingHorizontal" | "paddingVertical" => "padding",
        "mainAlign" | "primaryAxisAlignItems" | "crossAlign" | "counterAxisAlignItems" | "contentAlign" | "counterAxisAlignContent" => "alignment",
        "grids" | "layoutGrids" | "gridStyle" => "grids",
        "exports" | "exportSettings" => "exports",
        "interactions" => "interactions",
        k if k.starts_with("font") || k.starts_with("text") || k.starts_with("letter") || k.starts_with("line") || k.starts_with("paragraph") => {
            "textStyle"
        }
        _ => "other",
    }
}

fn string_val(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn range_of(props: &Map<String, Value>, len: usize) -> (usize, usize) {
    let s = props.get("start").and_then(f64_of).map(|v| v.max(0.0) as usize).unwrap_or(0);
    let e = props.get("end").and_then(f64_of).map(|v| v.max(0.0) as usize).unwrap_or(len);
    (s.min(len), e.min(len))
}

/// Set one text style property on chars `start..end`.
pub fn apply_text_style(cmd: &str, t: &mut TextData, start: usize, end: usize, k: &str, v: &Value) -> Result<()> {
    match k {
        "fontFamily" => {
            let fam = string_val(v).ok_or_else(|| bad(cmd, "`fontFamily` must be a string"))?;
            // Keep the style if the new family has it, else the nearest.
            let styles = figcraft_text::family_styles(&fam);
            t.restyle(start, end, |s| {
                s.font_family = fam.clone();
                if !styles.is_empty() && !styles.contains(&s.font_style) {
                    let want = s.weight();
                    if let Some(best) = styles.iter().min_by_key(|st| (i32::from(style_weight(st)) - i32::from(want)).abs()) {
                        s.font_style = best.clone();
                    }
                }
            });
        }
        "fontStyle" => {
            let st = string_val(v).ok_or_else(|| bad(cmd, "`fontStyle` must be a string"))?;
            t.restyle(start, end, |s| s.set_font_style(st.clone()));
        }
        "fontWeight" => {
            let w = f64_of(v).ok_or_else(|| bad(cmd, "`fontWeight` must be a number"))?;
            let name = weight_name(w.clamp(1.0, 1000.0) as u16);
            t.restyle(start, end, |s| {
                let italic = s.italic();
                s.set_font_style(styled_name(name, italic));
            });
        }
        "fontVariations" | "variationSettings" => {
            // Replaces the custom axis values (null or {} = back to what the style name implies).
            let vars = variations_of(cmd, v)?;
            t.restyle(start, end, |s| {
                s.variations.clear();
                for (k, val) in &vars {
                    if let Some(val) = val {
                        s.variations.insert(k.clone(), *val);
                    }
                }
            });
        }
        "fontSize" => {
            let sz = f64_of(v).ok_or_else(|| bad(cmd, "`fontSize` must be a number"))?.clamp(1.0, 5000.0);
            t.restyle(start, end, |s| s.font_size = sz);
        }
        "lineHeight" => {
            let lh = line_height(v).ok_or_else(|| bad(cmd, "lineHeight: auto | px | \"150%\""))?;
            t.restyle(start, end, |s| s.line_height = lh);
        }
        "letterSpacing" => {
            let ls = letter_spacing(v).ok_or_else(|| bad(cmd, "letterSpacing: px | \"2%\""))?;
            t.restyle(start, end, |s| s.letter_spacing = ls);
        }
        "textDecoration" => {
            let d: TextDecoration = enum_of(v).ok_or_else(|| bad(cmd, "textDecoration: none|underline|strikethrough"))?;
            t.restyle(start, end, |s| s.decoration = d);
        }
        "textDecorationStyle" => {
            let d: DecorationStyle = enum_of(v).ok_or_else(|| bad(cmd, "textDecorationStyle: solid|dotted|wavy"))?;
            t.restyle(start, end, |s| s.set_look(|l| l.style = d));
        }
        "textDecorationThickness" | "textDecorationOffset" => {
            let m = line_height(v)
                .filter(|m| !matches!(m, LineHeight::Pixels(x) | LineHeight::Percent(x) if !x.is_finite() || x.abs() > 10_000.0))
                .ok_or_else(|| bad(cmd, format!("{k}: auto | px | \"10%\"")))?;
            let thickness = k == "textDecorationThickness";
            t.restyle(start, end, |s| s.set_look(|l| *if thickness { &mut l.thickness } else { &mut l.offset } = m));
        }
        "textDecorationSkipInk" => {
            let on = v.as_bool().ok_or_else(|| bad(cmd, "textDecorationSkipInk must be true or false"))?;
            t.restyle(start, end, |s| s.set_look(|l| l.skip_ink = on));
        }
        "textDecorationColor" => {
            let auto = v.is_null() || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("auto"));
            let c = if auto { None } else { Some(color_of(v).ok_or_else(|| bad(cmd, "textDecorationColor: a colour, or auto"))?) };
            t.restyle(start, end, |s| s.set_look(|l| l.color = c));
        }
        "textCase" => {
            let c: TextCase = enum_of(v).ok_or_else(|| bad(cmd, "textCase: original|upper|lower|title|smallCaps"))?;
            t.restyle(start, end, |s| s.case = c);
        }
        "hyperlink" => {
            let l = string_val(v);
            t.restyle(start, end, |s| s.link = l.clone());
        }
        "textFills" => {
            let p = if v.is_null() { None } else { Some(paints_of(v).ok_or_else(|| bad(cmd, "textFills: paints"))?) };
            t.restyle(start, end, |s| s.fills = p.clone());
        }
        _ => {}
    }
    Ok(())
}

/// OpenType feature toggles from `{tag: bool | null}` (null = back to the font's default).
pub fn features_of(cmd: &str, v: &Value) -> Result<Vec<(String, Option<bool>)>> {
    let Value::Object(m) = v else { return Err(bad(cmd, "features: {tag: bool}, e.g. {\"liga\": false}")) };
    let mut out = Vec::with_capacity(m.len());
    for (k, on) in m {
        if !valid_feature_tag(k) {
            return Err(bad(cmd, format!("`{k}` is not a 4-character OpenType feature tag")));
        }
        let on = match on {
            Value::Null => None,
            Value::Bool(b) => Some(*b),
            other => Some(f64_of(other).ok_or_else(|| bad(cmd, format!("feature `{k}` must be true, false or null")))? != 0.0),
        };
        out.push((k.clone(), on));
    }
    Ok(out)
}

/// Variable font axis values from `{tag: number | null}` (null = back to what the style implies).
/// `null` alone means "no custom values".
pub fn variations_of(cmd: &str, v: &Value) -> Result<Vec<(String, Option<f64>)>> {
    let m = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Object(m) => m,
        _ => return Err(bad(cmd, "variations: {axis: number}, e.g. {\"wght\": 550, \"wdth\": 85}")),
    };
    let mut out = Vec::with_capacity(m.len());
    for (k, val) in m {
        if !valid_axis_tag(k) {
            return Err(bad(cmd, format!("`{k}` is not a 4-character OpenType axis tag")));
        }
        let val = match val {
            Value::Null => None,
            other => Some(f64_of(other).filter(|x| x.abs() <= 1e6).ok_or_else(|| bad(cmd, format!("axis `{k}` must be a number or null")))?),
        };
        out.push((k.clone(), val));
    }
    Ok(out)
}

/// "Bold" + italic → "Bold Italic"; "Regular" + italic → "Italic".
pub fn styled_name(weight: &str, italic: bool) -> String {
    match (italic, weight) {
        (false, w) => w.to_string(),
        (true, "Regular") => "Italic".into(),
        (true, w) => format!("{w} Italic"),
    }
}

pub fn weight_name(w: u16) -> &'static str {
    match w {
        0..=149 => "Thin",
        150..=249 => "Extra Light",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "Semi Bold",
        650..=749 => "Bold",
        750..=849 => "Extra Bold",
        _ => "Black",
    }
}

fn set_sizing(cmd: &str, doc: &mut Document, id: NodeId, s: Sizing, horizontal: bool) -> Result<()> {
    let parent = doc.get(id)?.parent.filter(|p| doc.node(*p).is_some_and(|p| p.layout.is_some()));
    match s {
        Sizing::Fill if parent.is_none() => return Err(bad(cmd, "Fill needs an auto layout parent")),
        Sizing::Hug if doc.get(id)?.layout.is_none() && !matches!(doc.get(id)?.kind, NodeKind::Text(_)) => {
            return Err(bad(cmd, "Hug needs auto layout or text"));
        }
        _ => {}
    }
    // A child can't fill an axis its parent hugs: the parent becomes fixed at its current size.
    // (The other way round, a parent that starts to hug fixes the children that filled it: see
    // `figcraft_layout::coherent_sizing`, applied after every edit.)
    if s == Sizing::Fill
        && let Some(p) = parent
    {
        let cl = &mut doc.get_mut(p)?.child_layout;
        let ps = if horizontal { &mut cl.sizing_h } else { &mut cl.sizing_v };
        if *ps == Sizing::Hug {
            *ps = Sizing::Fixed;
        }
    }
    // Start from the sizing the layer has in effect (an auto-width text hugs both axes), so the
    // other axis keeps its meaning: an auto-width text set to fill its width wraps (auto height).
    figcraft_layout::coherent_sizing(doc, id);
    let n = doc.get_mut(id)?;
    if horizontal {
        n.child_layout.sizing_h = s;
    } else {
        n.child_layout.sizing_v = s;
    }
    if let NodeKind::Text(t) = &mut n.kind {
        t.auto_resize = match (n.child_layout.sizing_h, n.child_layout.sizing_v) {
            (Sizing::Hug, _) => TextAutoResize::WidthAndHeight,
            (_, Sizing::Hug) => TextAutoResize::Height,
            _ => TextAutoResize::None,
        };
    }
    Ok(())
}

/// Resize a node's box from its top-left, applying constraints / group scaling to children.
/// The page transforms of `id`'s children, to put back with [`restore_worlds`] once a container
/// whose contents stay where they are on the page (a section; a frame resized with Cmd/Ctrl held)
/// has moved or changed size.
pub fn children_worlds(doc: &Document, id: NodeId) -> Vec<(NodeId, Affine)> {
    doc.node(id).map(|n| n.children.iter().map(|c| (*c, doc.world_transform(*c))).collect()).unwrap_or_default()
}

pub fn restore_worlds(doc: &mut Document, worlds: &[(NodeId, Affine)]) -> Result<()> {
    for (c, w) in worlds {
        doc.set_world_transform(*c, *w)?;
    }
    Ok(())
}

pub fn resize_node(doc: &mut Document, id: NodeId, w: f64, h: f64) -> Result<()> {
    let n = doc.get(id)?;
    let (ow, oh) = (n.width, n.height);
    let group = n.kind.bounds_follow_children();
    let section = matches!(n.kind, NodeKind::Section { .. });
    figcraft_layout::set_size(doc, id, w, h);
    if section {
        // Sections have no constraints: what's in them stays as it is.
    } else if group {
        let kx = if ow > 1e-9 { w / ow } else { 1.0 };
        let ky = if oh > 1e-9 { h / oh } else { 1.0 };
        figcraft_layout::scale_group_children(doc, id, kx, ky);
    } else {
        // In an auto layout frame this moves only the children that ignore auto layout.
        figcraft_layout::apply_constraints(doc, id, ow, oh);
    }
    Ok(())
}

/// The design-panel view of a node (used by the UI and inspect).
pub fn describe(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let c = doc.frame_relative(id).as_coeffs();
    let mut v = json!({
        "id": n.id,
        "name": n.name,
        "type": n.kind.type_name(),
        "x": c[4], "y": c[5],
        "width": n.width, "height": n.height,
        "rotation": figcraft_geom::round_to(figcraft_geom::rotation_degrees(n.transform), 3),
        "visible": n.visible, "locked": n.locked,
        "opacity": n.opacity, "blend": n.blend,
    });
    if let Some(b) = doc.world_bounds(id) {
        v["absoluteBounds"] = json!({"x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()});
    }
    let _ = Point::ZERO;
    let _ = Color::BLACK;
    v
}
