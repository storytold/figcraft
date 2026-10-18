//! What the export changed or left out: one entry per kind, naming the first few layers.

/// The kinds of change, in the order they're reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Warn {
    DropShadow,
    InnerShadow,
    LayerBlur,
    BackgroundBlur,
    Noise,
    Texture,
    Glass,
    Blend,
    ImageFill,
    PatternFill,
    DiamondApprox,
    DiamondOmitted,
    OpacityApprox,
    OpacityOmitted,
    EvenOddRewritten,
    EvenOddKept,
    ClipEvenOddRewritten,
    ClipEvenOddApprox,
    ClipEvenOddOmitted,
    MaskApprox,
    MaskOmitted,
    TooLarge,
    NothingDrawn,
}

impl Warn {
    fn message(self) -> &'static str {
        match self {
            Warn::DropShadow => "drop shadows were left out",
            Warn::InnerShadow => "inner shadows were left out",
            Warn::LayerBlur => "layer blurs were left out",
            Warn::BackgroundBlur => "background blurs were left out",
            Warn::Noise => "noise effects were left out",
            Warn::Texture => "texture effects were left out",
            Warn::Glass => "glass effects were left out",
            Warn::Blend => "blend modes were drawn as normal",
            Warn::ImageFill => "image fills were left out",
            Warn::PatternFill => "pattern fills were left out",
            Warn::DiamondApprox => "diamond gradients were drawn as radial gradients",
            Warn::DiamondOmitted => "diamond gradients were left out (exact mode)",
            Warn::OpacityApprox => "layer opacity was applied to each shape: where shapes overlap they look denser",
            Warn::OpacityOmitted => "layers with opacity over overlapping shapes were left out (exact mode)",
            Warn::EvenOddRewritten => "even-odd fills were rewritten as non-zero",
            Warn::EvenOddKept => "even-odd fills couldn't be rewritten and use fillType=\"evenOdd\" (API 24+ or AndroidX)",
            Warn::ClipEvenOddRewritten => "even-odd clip outlines were rewritten as non-zero",
            Warn::ClipEvenOddApprox => "even-odd clip outlines couldn't be rewritten and clip as non-zero",
            Warn::ClipEvenOddOmitted => "layers under even-odd clip outlines that couldn't be rewritten were left out (exact mode)",
            Warn::MaskApprox => "alpha and luminance masks clip to the mask's outline",
            Warn::MaskOmitted => "layers under alpha or luminance masks were left out (exact mode)",
            Warn::TooLarge => "shapes with coordinates beyond ±10,000,000 were left out",
            Warn::NothingDrawn => "nothing in it could be drawn: the drawable is empty",
        }
    }
}

/// Layers named per warning.
const NAMED: usize = 3;

#[derive(Default)]
pub(crate) struct Warnings {
    seen: std::collections::BTreeMap<Warn, Vec<String>>,
}

impl Warnings {
    pub fn add(&mut self, w: Warn, layer: &str) {
        let names = self.seen.entry(w).or_default();
        if !names.iter().any(|n| n == layer) {
            names.push(layer.to_string());
        }
    }

    pub fn finish(self) -> Vec<String> {
        self.seen
            .into_iter()
            .map(|(w, names)| {
                let shown: Vec<String> =
                    names.iter().take(NAMED).map(|n| if n.is_empty() { "unnamed layer".to_string() } else { n.clone() }).collect();
                let more = names.len().saturating_sub(NAMED);
                let more = if more > 0 { format!(", {more} more") } else { String::new() };
                format!("{} ({}{more})", w.message(), shown.join(", "))
            })
            .collect()
    }
}
