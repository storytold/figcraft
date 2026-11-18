//! Layer and paint blend modes (the list Figma's blend menu shows).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    /// Layers only: children blend with what's behind the group (Figma's default for groups/frames).
    PassThrough,
    #[default]
    Normal,
    Darken,
    Multiply,
    PlusDarker,
    ColorBurn,
    Lighten,
    Screen,
    PlusLighter,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// Every mode in menu order, with separators implied between groups.
    pub const ALL: [BlendMode; 19] = [
        BlendMode::PassThrough,
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::PlusDarker,
        BlendMode::ColorBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::PlusLighter,
        BlendMode::ColorDodge,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::PassThrough => "Pass through",
            BlendMode::Normal => "Normal",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::PlusDarker => "Plus darker",
            BlendMode::ColorBurn => "Color burn",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::PlusLighter => "Plus lighter",
            BlendMode::ColorDodge => "Color dodge",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft light",
            BlendMode::HardLight => "Hard light",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Hue => "Hue",
            BlendMode::Saturation => "Saturation",
            BlendMode::Color => "Color",
            BlendMode::Luminosity => "Luminosity",
        }
    }

    /// The CSS `mix-blend-mode` keyword (for SVG/CSS export).
    pub fn css(self) -> &'static str {
        match self {
            BlendMode::PassThrough | BlendMode::Normal => "normal",
            BlendMode::Darken => "darken",
            BlendMode::Multiply => "multiply",
            BlendMode::PlusDarker => "plus-darker",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::Lighten => "lighten",
            BlendMode::Screen => "screen",
            BlendMode::PlusLighter => "plus-lighter",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::Overlay => "overlay",
            BlendMode::SoftLight => "soft-light",
            BlendMode::HardLight => "hard-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
        }
    }

    /// The mode named by its label or CSS keyword, ignoring case, spaces and dashes. Labels come
    /// first: "normal" is Normal, not pass-through (whose CSS keyword is also `normal`).
    pub fn from_name(s: &str) -> Option<Self> {
        let key: String = s.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|m| m.label().replace(' ', "").to_ascii_lowercase() == key)
            .or_else(|| Self::ALL.into_iter().find(|m| m.css().replace('-', "") == key))
    }
}
