use super::{Fields, Kv};

/// Main-equal fields which are not equal to the scaled main. Keep these separately until the
/// instance's factor is known: the ordinary diff would discard a length reset to its main's value.
pub(super) fn main_equal_lengths(mine: &Fields, theirs: &Fields) -> Fields {
    mine.iter().filter(|(k, v)| has_length(k, v) && theirs.iter().any(|(tk, tv)| tk == k && tv == v)).cloned().collect()
}

/// Whether a field contains a finite, nonzero length read at its supplying row's scale by the
/// importer. Ratios, IDs, colours, grid counts and unit-box coordinates are not lengths.
fn has_length(key: &str, v: &Kv) -> bool {
    match key {
        "strokeWeight"
        | "borderTopWeight"
        | "borderRightWeight"
        | "borderBottomWeight"
        | "borderLeftWeight"
        | "cornerRadius"
        | "rectangleTopLeftCornerRadius"
        | "rectangleTopRightCornerRadius"
        | "rectangleBottomRightCornerRadius"
        | "rectangleBottomLeftCornerRadius"
        | "stackSpacing"
        | "stackCounterSpacing"
        | "stackHorizontalPadding"
        | "stackVerticalPadding"
        | "stackPaddingRight"
        | "stackPaddingBottom"
        | "gridColumnGap"
        | "gridRowGap"
        | "fontSize"
        | "paragraphSpacing"
        | "paragraphIndent"
        | "listSpacing" => nonzero(v),
        "size" => numbers(v, &["x", "y"]),
        "transform" => numbers(v, &["m02", "m12"]),
        "minSize" | "maxSize" => v.get("value").is_some_and(|p| numbers(p, &["x", "y"])),
        "dashPattern" => v.list().iter().any(nonzero),
        "lineHeight" | "letterSpacing" => v.get("units").and_then(Kv::str) == Some("PIXELS") && numbers(v, &["value"]),
        "effects" => v.list().iter().any(|e| {
            numbers(e, &["radius", "spread", "startRadius"])
                || ["offset", "noiseSize"].iter().any(|k| e.get(k).is_some_and(|p| numbers(p, &["x", "y"])))
        }),
        "layoutGrids" => v.list().iter().any(|g| numbers(g, &["sectionSize", "gutterSize", "offset"])),
        "textData" => v.get("styleOverrideTable").map(Kv::list).unwrap_or_default().iter().any(|style| {
            numbers(style, &["fontSize", "paragraphSpacing", "paragraphIndent", "listSpacing"])
                || ["lineHeight", "letterSpacing"].iter().any(|k| style.get(k).is_some_and(|v| has_length(k, v)))
        }),
        _ => false,
    }
}

/// Zero is unchanged by every scale; nonfinite sentinels (an absent maximum) are unchanged too.
fn nonzero(v: &Kv) -> bool {
    v.f64().is_some_and(|n| n != 0.0)
}

/// Test only the named length members, not other numbers in a composite field.
fn numbers(v: &Kv, keys: &[&str]) -> bool {
    keys.iter().any(|key| v.get(key).is_some_and(nonzero))
}

#[cfg(test)]
mod tests {
    use super::super::{arr, e, fl, o};
    use super::*;

    #[test]
    fn dimensionless_values_and_zero_do_not_create_scale_overrides() {
        for (key, value) in [
            ("opacity", fl(0.5)),
            ("cornerRadius", fl(0.0)),
            ("cornerSmoothing", fl(0.5)),
            ("letterSpacing", o(vec![("units", e("PERCENT")), ("value", fl(10.0))])),
            ("lineHeight", o(vec![("units", e("RAW")), ("value", fl(1.5))])),
            ("transform", o(vec![("m00", fl(1.0)), ("m11", fl(1.0))])),
            ("layoutGrids", arr(vec![o(vec![("numSections", fl(12.0))])])),
            ("effects", arr(vec![o(vec![("density", fl(0.5)), ("startOffset", o(vec![("x", fl(0.5))]))])])),
            ("maxSize", o(vec![("value", o(vec![("x", Kv::Float(f32::INFINITY))]))])),
        ] {
            assert!(!has_length(key, &value), "{key}");
        }
        assert!(has_length("paragraphIndent", &fl(-4.0)));
        assert!(has_length("lineHeight", &o(vec![("units", e("PIXELS")), ("value", fl(24.0))])));
    }
}
