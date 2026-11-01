use super::{
    Bound, ExportSize, Fields, GAMEPAD_BUTTONS, GridAlign, GridKind, KeyMods, Kv, LetterSpacing, LineHeight, MAX_NETWORK, Node, NodeKind, Paint,
    PropDef, SCHEMA, TextCase, TextDecoration, TextStyle, VariableType, VectorNetwork, WindingRule, arr, b, color, e, face_style, fl, guid, int, o,
    s, xy,
};

/// Longest kept description of such a layer (bytes of JSON).
const MAX_STASH: usize = 1 << 20;

/// Whether a width profile reaches further on one side of the path than on the other.
pub(super) fn one_sided(p: &figcraft_doc::WidthProfile) -> bool {
    p.points.iter().any(|q| q.bias.abs() > 1e-6)
}

/// A shape whose stroke is wider on one side of its path, as the area that stroke paints (a vector
/// filled with the stroke's paints), with what it was as JSON: its kind, paints and stroke. Only
/// for shapes that show nothing but that stroke; `None` for every other layer.
pub(super) fn outlined(n: &Node) -> Option<(Node, String)> {
    let sp = &n.stroke;
    let shape = matches!(
        n.kind,
        NodeKind::Line | NodeKind::Vector { .. } | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Rectangle
    );
    let shown = |paints: &[Paint]| paints.iter().any(|p| p.visible);
    if !(shape
        && sp.brush.is_none()
        && sp.profile().is_some_and(one_sided)
        && sp.weight.is_finite()
        && sp.weight > 0.0
        && shown(&n.strokes)
        && !shown(&n.fills))
    {
        return None;
    }
    let path = figcraft_doc::geometry::stroke_geometry(n)?;
    let net = match &n.kind {
        NodeKind::Vector { network } => Some(network),
        _ => None,
    };
    let area = figcraft_pathops::stroke_outline_of(&path, sp, figcraft_doc::geometry::is_closed(n), net);
    if area.elements().is_empty() || area.elements().len() > MAX_NETWORK / 4 {
        return None;
    }
    let original = serde_json::json!({"kind": n.kind, "fills": n.fills, "strokes": n.strokes, "stroke": n.stroke}).to_string();
    if original.len() > MAX_STASH {
        return None;
    }
    let mut network = VectorNetwork::from_path(&area);
    for r in &mut network.regions {
        r.winding = WindingRule::NonZero;
    }
    let mut v = n.clone();
    v.kind = NodeKind::Vector { network };
    v.fills = std::mem::take(&mut v.strokes);
    v.stroke = Default::default();
    v.styles.fill = v.styles.stroke.take();
    // Bindings name the stroke's paints, which are the fills now: the values stay.
    v.bound.retain(|k, _| !(k == "stroke" || k == "fill" || k.starts_with("strokes/") || k.starts_with("fills/") || k.starts_with("stroke")));
    Some((v, original))
}

/// The number of the layer field a bound property (`VariableField`) is held in.
pub(super) fn node_field(field: &str) -> Option<u32> {
    let name = match field {
        "WIDTH" | "HEIGHT" => "size",
        "MIN_WIDTH" | "MIN_HEIGHT" => "minSize",
        "MAX_WIDTH" | "MAX_HEIGHT" => "maxSize",
        "STACK_SPACING" => "stackSpacing",
        "STACK_COUNTER_SPACING" => "stackCounterSpacing",
        "STACK_PADDING_LEFT" => "stackHorizontalPadding",
        "STACK_PADDING_TOP" => "stackVerticalPadding",
        "STACK_PADDING_RIGHT" => "stackPaddingRight",
        "STACK_PADDING_BOTTOM" => "stackPaddingBottom",
        "CORNER_RADIUS" => "cornerRadius",
        "RECTANGLE_TOP_LEFT_CORNER_RADIUS" => "rectangleTopLeftCornerRadius",
        "RECTANGLE_TOP_RIGHT_CORNER_RADIUS" => "rectangleTopRightCornerRadius",
        "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS" => "rectangleBottomLeftCornerRadius",
        "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS" => "rectangleBottomRightCornerRadius",
        "OPACITY" => "opacity",
        "VISIBLE" => "visible",
        "TEXT_DATA" => "textData",
        "STROKE_WEIGHT" => "strokeWeight",
        "BORDER_TOP_WEIGHT" => "borderTopWeight",
        "BORDER_RIGHT_WEIGHT" => "borderRightWeight",
        "BORDER_BOTTOM_WEIGHT" => "borderBottomWeight",
        "BORDER_LEFT_WEIGHT" => "borderLeftWeight",
        "FONT_SIZE" => "fontSize",
        "FONT_FAMILY" | "FONT_STYLE" => "fontName",
        "LETTER_SPACING" => "letterSpacing",
        "LINE_HEIGHT" => "lineHeight",
        "PARAGRAPH_SPACING" => "paragraphSpacing",
        "PARAGRAPH_INDENT" => "paragraphIndent",
        _ => return None,
    };
    // The numbers are the schema's.
    let layer = SCHEMA.lines().find(|l| l.starts_with("message NodeChange "))?;
    layer.split(", ").find_map(|f| f.rsplit_once(' ')?.1.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
}

/// Whether `a` is `b` with its lengths scaled by `k`: every number is `k` times the other's (within
/// `slack`, for sizes that snap to whole pixels) or the same, and the rest is equal.
pub(super) fn scaled(a: &Kv<'static>, b: &Kv<'static>, k: f64, slack: f64) -> bool {
    match (a, b) {
        (Kv::Float(x), Kv::Float(y)) => {
            let (x, y) = (f64::from(*x), f64::from(*y));
            x == y || (x - y * k).abs() <= slack + 1e-3 * x.abs().max(1.0)
        }
        (Kv::Array(x), Kv::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| scaled(p, q, k, slack)),
        (Kv::Object(x), Kv::Object(y)) => x.len() == y.len() && x.iter().zip(y).all(|((kp, p), (kq, q))| kp == kq && scaled(p, q, k, slack)),
        _ => a == b,
    }
}

/// The fields of `mine` that differ from `theirs` (a layer and the main's layer it mirrors), and
/// the ones only `theirs` has at their empty values.
pub(super) fn changed(mine: Fields, theirs: &Fields) -> Fields {
    let cleared: Fields = theirs.iter().filter(|(k, _)| !mine.iter().any(|(mk, _)| mk == k)).filter_map(|(k, _)| Some((*k, unset(k)?))).collect();
    let mut out: Fields = mine.into_iter().filter(|(k, v)| !theirs.iter().any(|(tk, tv)| tk == k && tv == v)).collect();
    out.extend(cleared);
    out
}

/// Whether paint list `v` (field `key`) is `theirs` but for the colours variables give: two layers
/// bound alike are the same, whatever mode each one shows.
pub(super) fn resolved_alike(key: &str, v: &Kv<'static>, theirs: &Fields) -> bool {
    fn unresolved(v: &Kv<'static>) -> Vec<Kv<'static>> {
        let bare = |p: &Kv<'static>| match p {
            Kv::Object(f) if f.iter().any(|(k, _)| *k == "colorVar") => o(f.iter().filter(|(k, _)| *k != "color").cloned().collect()),
            other => other.clone(),
        };
        v.list().iter().map(bare).collect()
    }
    matches!(key, "fillPaints" | "strokePaints") && theirs.iter().any(|(k, t)| *k == key && unresolved(t) == unresolved(v))
}

/// The format's name of a variable type.
pub(super) fn resolved_type(kind: VariableType) -> &'static str {
    match kind {
        VariableType::Color => "COLOR",
        VariableType::Number => "FLOAT",
        VariableType::String => "STRING",
        VariableType::Boolean => "BOOLEAN",
    }
}

/// The format's name of a variable scope (`None`: it has none).
pub(super) fn scope_name(scope: &str) -> Option<&'static str> {
    const NAMES: [&str; 20] = [
        "ALL_SCOPES",
        "TEXT_CONTENT",
        "CORNER_RADIUS",
        "WIDTH_HEIGHT",
        "GAP",
        "ALL_FILLS",
        "FRAME_FILL",
        "SHAPE_FILL",
        "TEXT_FILL",
        "STROKE_FLOAT",
        "EFFECT_FLOAT",
        "EFFECT_COLOR",
        "OPACITY",
        "FONT_STYLE",
        "FONT_FAMILY",
        "FONT_SIZE",
        "LINE_HEIGHT",
        "LETTER_SPACING",
        "PARAGRAPH_SPACING",
        "PARAGRAPH_INDENT",
    ];
    if scope == "STROKE_COLOR" {
        return Some("STROKE");
    }
    NAMES.into_iter().find(|n| *n == scope)
}

/// The empty value of a field only written when set (an override clears it with this).
fn unset(key: &str) -> Option<Kv<'static>> {
    Some(match key {
        "styleIdForFill" | "styleIdForStrokeFill" | "styleIdForEffect" | "styleIdForGrid" | "styleIdForText" => {
            o(vec![("guid", guid(u32::MAX, u32::MAX))])
        }
        "variableConsumptionMap" | "variableModeBySetMap" | "gridColumnsSizing" | "gridRowsSizing" => o(vec![("entries", arr(Vec::new()))]),
        "mask" | "rectangleCornerRadiiIndependent" | "borderStrokeWeightsIndependent" => b(false),
        "effects" | "dashPattern" | "layoutGrids" | "exportSettings" | "prototypeInteractions" | "fillGeometry" | "variableWidthPoints" => {
            arr(Vec::new())
        }
        "cornerSmoothing" | "stackChildPrimaryGrow" => fl(0.0),
        "stackMode" => e("NONE"),
        "stackChildAlignSelf" | "stackPositioning" => e("AUTO"),
        "minSize" => o(vec![("value", xy(0.0, 0.0))]),
        // No maximum is infinite (a maximum of 0 collapses the layer).
        "maxSize" => o(vec![("value", o(vec![("x", Kv::Float(f32::INFINITY)), ("y", Kv::Float(f32::INFINITY))]))]),
        "gridColumnSpan" | "gridRowSpan" | "maxLines" => int(0),
        "scrollDirection" => e("NONE"),
        "scrollBehavior" => e("SCROLLS"),
        "parameterConsumptionMap" => o(vec![("entries", arr(Vec::new()))]),
        "textTruncation" => e("DISABLED"),
        "hyperlink" => o(vec![("url", s(""))]),
        _ => return None,
    })
}

/// The pairs of a variant's name, `Property=Value, …` (parts without `=` are left out).
pub(super) fn variant_pairs(name: &str) -> Vec<(String, String)> {
    name.split(',')
        .filter_map(|part| {
            let (p, v) = part.split_once('=')?;
            let (p, v) = (p.trim(), v.trim());
            (!p.is_empty()).then(|| (p.to_string(), v.to_string()))
        })
        .collect()
}

/// The `index`-th sort position of a list: one printable character from `"` on, two past the end
/// of that range.
pub(super) fn sort_position(index: usize) -> String {
    const FIRST: u32 = 34;
    const SPAN: usize = 93;
    let ch = |i: usize| char::from_u32(FIRST + u32::try_from(i % SPAN).unwrap_or(0)).unwrap_or('"');
    if index < SPAN { ch(index).to_string() } else { format!("~{}", ch(index - SPAN)) }
}

/// Text as a property value: its characters and one plain line per paragraph.
pub(super) fn text_value(t: &str) -> Kv<'static> {
    let line = || {
        o(vec![
            ("lineType", e("PLAIN")),
            ("styleId", int(0)),
            ("indentationLevel", int(0)),
            ("sourceDirectionality", e("AUTO")),
            ("listStartOffset", int(0)),
            ("isFirstLineOfList", b(false)),
        ])
    };
    o(vec![("characters", s(t)), ("lines", arr(t.split('\n').map(|_| line()).collect()))])
}

/// A property value as variable data (`varValue`).
pub(super) fn var_data(value: (&'static str, Kv<'static>), ty: &'static str, resolved: &'static str) -> Kv<'static> {
    o(vec![("value", o(vec![value])), ("dataType", e(ty)), ("resolvedDataType", e(resolved))])
}

pub(super) fn bool_data(x: bool) -> Kv<'static> {
    var_data(("boolValue", b(x)), "BOOLEAN", "BOOLEAN")
}

pub(super) fn text_data(t: &str) -> Kv<'static> {
    var_data(("textDataValue", text_value(t)), "TEXT_DATA", "TEXT_DATA")
}

pub(super) fn symbol_data(main: Kv<'static>) -> Kv<'static> {
    var_data(("symbolIdValue", o(vec![("guid", main)])), "SYMBOL_ID", "SYMBOL_ID")
}

/// The format's type of a component property (`None`: not written).
pub(super) fn prop_type(d: &PropDef) -> Option<&'static str> {
    match d {
        PropDef::Boolean { .. } => Some("BOOL"),
        PropDef::Text { .. } => Some("TEXT"),
        PropDef::InstanceSwap { .. } => Some("INSTANCE_SWAP"),
        PropDef::Variant { .. } | PropDef::Slot { .. } => None,
    }
}

/// The key codes of a key trigger (modifiers first), and whether it's a controller button.
pub(super) fn key_codes(key: &str) -> Option<(Vec<i64>, bool)> {
    if let Some(i) = GAMEPAD_BUTTONS.iter().position(|g| g.eq_ignore_ascii_case(key.trim())) {
        return Some((vec![i as i64], true));
    }
    let (mods, k): (KeyMods, &str) = KeyMods::split(key);
    let code = (0..=255).find(|c| crate::rest::proto::key_name(f64::from(*c)).is_some_and(|n| n.eq_ignore_ascii_case(k)))?;
    let mut codes: Vec<i64> = [(mods.shift, 16), (mods.ctrl, 17), (mods.alt, 18), (mods.cmd, 91)].into_iter().filter(|m| m.0).map(|m| m.1).collect();
    codes.push(i64::from(code));
    Some((codes, false))
}

/// Whether the decoration look has more than skip ink, which the file can't hold.
pub(super) fn unsupported_decoration_look(st: &TextStyle) -> bool {
    let mut look = st.look();
    look.skip_ink = false;
    look != Default::default()
}

/// The text style fields of a layer or of an override table entry.
pub(super) fn style_fields(st: &TextStyle, f: &mut Fields) {
    let line_height = match st.line_height {
        // Percent units count from the font's own line height (100 % is "auto"); a share of the
        // font size is a raw multiplier.
        LineHeight::Auto => o(vec![("value", fl(100.0)), ("units", e("PERCENT"))]),
        LineHeight::Percent(p) => o(vec![("value", fl(p / 100.0)), ("units", e("RAW"))]),
        LineHeight::Pixels(v) => o(vec![("value", fl(v)), ("units", e("PIXELS"))]),
    };
    let letter_spacing = match st.letter_spacing {
        LetterSpacing::Percent(p) => o(vec![("value", fl(p)), ("units", e("PERCENT"))]),
        LetterSpacing::Pixels(v) => o(vec![("value", fl(v)), ("units", e("PIXELS"))]),
    };
    let case = match st.case {
        TextCase::Original => "ORIGINAL",
        TextCase::Upper => "UPPER",
        TextCase::Lower => "LOWER",
        TextCase::Title => "TITLE",
        TextCase::SmallCaps => "SMALL_CAPS",
    };
    let decoration = match st.decoration {
        TextDecoration::None => "NONE",
        TextDecoration::Underline => "UNDERLINE",
        TextDecoration::Strikethrough => "STRIKETHROUGH",
    };
    f.extend([
        ("fontName", o(vec![("family", s(&st.font_family)), ("style", s(&face_style(&st.font_family, &st.font_style))), ("postscript", s(""))])),
        ("fontSize", fl(st.font_size)),
        ("lineHeight", line_height),
        ("letterSpacing", letter_spacing),
        ("textCase", e(case)),
        ("textDecoration", e(decoration)),
        // Skip ink only shows on an underline; other text gets the files' default (on), the value
        // the import leaves out again.
        ("textDecorationSkipInk", b(st.look().skip_ink || st.decoration != TextDecoration::Underline)),
    ]);
    if let Some(url) = &st.link {
        f.push(("hyperlink", o(vec![("url", s(url))])));
    }
}

/// Layout grid `index` of a layer, with the variables `bound` to its fields.
pub(super) fn layout_grid(g: &figcraft_doc::LayoutGrid, index: usize, bound: &Bound) -> Kv<'static> {
    let (pattern, axis) = match g.kind {
        GridKind::Grid => ("GRID", "X"),
        GridKind::Columns => ("STRIPES", "X"),
        GridKind::Rows => ("STRIPES", "Y"),
    };
    let align = match g.align {
        GridAlign::Stretch => "STRETCH",
        GridAlign::Min => "MIN",
        GridAlign::Center => "CENTER",
        GridAlign::Max => "MAX",
    };
    let mut f: Fields = vec![
        ("type", e(align)),
        ("axis", e(axis)),
        ("visible", b(g.visible)),
        ("numSections", int(g.count.into())),
        ("offset", fl(g.offset)),
        ("sectionSize", fl(g.size)),
        ("gutterSize", fl(g.gutter)),
        ("color", color(g.color)),
        ("pattern", e(pattern)),
    ];
    let vars = [("count", "numSectionsVar"), ("offset", "offsetVar"), ("size", "sectionSizeVar"), ("gutter", "gutterSizeVar")];
    f.extend(vars.into_iter().filter_map(|(ours, key)| Some((key, bound.grids.get(&(index, ours))?.clone()))));
    o(f)
}

pub(super) fn export_setting(x: &figcraft_doc::ExportSetting) -> Kv<'static> {
    let format = match x.format {
        // The format has no WebP export setting.
        figcraft_doc::ExportFormat::Png | figcraft_doc::ExportFormat::Webp => "PNG",
        figcraft_doc::ExportFormat::Jpg => "JPEG",
        // Nor an Android Vector Drawable one: the nearest is the other vector format.
        figcraft_doc::ExportFormat::Svg | figcraft_doc::ExportFormat::VectorDrawable => "SVG",
        figcraft_doc::ExportFormat::Pdf => "PDF",
    };
    let (ty, value) = match x.size {
        ExportSize::Scale { value } => ("CONTENT_SCALE", value),
        ExportSize::Width { value } => ("CONTENT_WIDTH", value),
        ExportSize::Height { value } => ("CONTENT_HEIGHT", value),
    };
    o(vec![("suffix", s(&x.suffix)), ("imageType", e(format)), ("constraint", o(vec![("type", e(ty)), ("value", fl(value))]))])
}
