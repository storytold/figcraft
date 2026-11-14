//! Compact JSON metadata using forms already accepted by the derived legacy readers.
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::{LetterSpacing, LineHeight, TextCase, TextDecoration, TextLine, TextStyle};

pub(crate) fn lines<S: Serializer>(lines: &[TextLine], s: S) -> Result<S::Ok, S::Error> {
    // Derived TextLine::deserialize accepts both its named object and this field-order tuple.
    // Keep every float as f64, including signed zero; only repeated field names are removed.
    s.collect_seq(lines.iter().map(|v| {
        let TextLine { glyph_start, glyph_end, char_start, char_end, top, height, baseline, width, x, para_end, break_after, rtl } = v;
        (glyph_start, glyph_end, char_start, char_end, top, height, baseline, width, x, para_end, break_after, rtl)
    }))
}

pub(crate) fn styles<S: Serializer>(styles: &[TextStyle], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(styles.iter().map(SparseStyle))
}

struct SparseStyle<'a>(&'a TextStyle);
impl Serialize for SparseStyle<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Exhaustive destructuring makes a newly added style field a compile error here,
        // rather than silently dropping that field from portable geometry.
        let TextStyle {
            font_family,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            decoration,
            decoration_look,
            case,
            fills,
            link,
            variations,
            style_id,
        } = self.0;
        let included = [
            !matches!(line_height, LineHeight::Auto),
            // Numeric equality treats -0.0 as +0.0; omission must preserve the actual bits.
            !matches!(letter_spacing, LetterSpacing::Percent(v) if v.to_bits() == 0.0f64.to_bits()),
            !matches!(decoration, TextDecoration::None),
            decoration_look.is_some(),
            !matches!(case, TextCase::Original),
            fills.is_some(),
            link.is_some(),
            !variations.is_empty(),
            style_id.is_some(),
        ];
        let mut out = s.serialize_struct("TextStyle", 3 + included.iter().filter(|v| **v).count())?;
        // Defaults for these three can differ between builds; always retain their actual values.
        out.serialize_field("font_family", font_family)?;
        out.serialize_field("font_style", font_style)?;
        out.serialize_field("font_size", font_size)?;
        let [height, spacing, deco, look, casing, paint, hyperlink, axes, shared] = included;
        if height {
            out.serialize_field("line_height", line_height)?;
        }
        if spacing {
            out.serialize_field("letter_spacing", letter_spacing)?;
        }
        if deco {
            out.serialize_field("decoration", decoration)?;
        }
        if look {
            out.serialize_field("decoration_look", decoration_look)?;
        }
        if casing {
            out.serialize_field("case", case)?;
        }
        if paint {
            out.serialize_field("fills", fills)?;
        }
        if hyperlink {
            out.serialize_field("link", link)?;
        }
        if axes {
            out.serialize_field("variations", variations)?;
        }
        if shared {
            out.serialize_field("style_id", style_id)?;
        }
        out.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DecorationLook, DecorationStyle};
    use serde::Deserialize;

    #[derive(Serialize)]
    struct Compact<'a> {
        #[serde(serialize_with = "super::lines")]
        lines: &'a [TextLine],
        #[serde(serialize_with = "super::styles")]
        styles: &'a [TextStyle],
    }
    // Exactly the old reader's field types; no new compact-metadata decoder is needed.
    #[derive(Serialize, Deserialize)]
    struct Legacy {
        lines: Vec<TextLine>,
        styles: Vec<TextStyle>,
    }

    #[test]
    fn old_metadata_reader_restores_every_value_and_float_bit() {
        let line = TextLine {
            glyph_start: 0,
            glyph_end: 4,
            char_start: 2,
            char_end: 6,
            top: -0.0,
            height: 19.125,
            baseline: 15.123456789012345,
            width: 123.98765432109876,
            x: -0.0,
            para_end: true,
            break_after: false,
            rtl: true,
        };
        let style = TextStyle {
            font_family: "Synthetic custom family".into(),
            font_style: "Book".into(),
            font_size: 13.123456789012345,
            line_height: LineHeight::Pixels(-0.0),
            letter_spacing: LetterSpacing::Percent(-0.0),
            decoration: TextDecoration::Underline,
            decoration_look: Some(Box::new(DecorationLook {
                style: DecorationStyle::Wavy,
                thickness: LineHeight::Pixels(1.23456789012345),
                offset: LineHeight::Pixels(-0.0),
                skip_ink: true,
                color: None,
            })),
            case: TextCase::Upper,
            fills: Some(vec![]),
            link: Some("https://example.test/".into()),
            variations: std::collections::BTreeMap::from([("wght".into(), 711.125)]),
            style_id: Some("synthetic-style".into()),
        };
        let lines = vec![line];
        let styles = vec![TextStyle::default(), style];
        let compact = Compact { lines: &lines, styles: &styles };
        let json = serde_json::to_string(&compact).unwrap();
        let verbose = serde_json::to_string(&Legacy { lines: lines.clone(), styles: styles.clone() }).unwrap();
        assert!(json.len() < verbose.len());
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["lines"][0].is_array());
        assert_eq!(value["styles"][0].as_object().unwrap().len(), 3);
        for index in 0..styles.len() {
            for key in ["font_family", "font_style", "font_size"] {
                assert!(value["styles"][index].get(key).is_some());
            }
        }
        let back: Legacy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lines, lines);
        assert_eq!(back.styles, styles);
        let bits = |v: &TextLine| [v.top, v.height, v.baseline, v.width, v.x].map(f64::to_bits);
        assert_eq!(bits(&back.lines[0]), bits(&lines[0]));
        assert!(matches!(back.styles[1].letter_spacing, LetterSpacing::Percent(v) if v.to_bits() == (-0.0f64).to_bits()));
        assert!(matches!(back.styles[1].line_height, LineHeight::Pixels(v) if v.to_bits() == (-0.0f64).to_bits()));
        assert_eq!(back.styles[1].font_size.to_bits(), styles[1].font_size.to_bits());
    }
}
