//! The design system a file's review follows: how strict it is, the spacing grid, the radii it
//! accepts and the brand accents it knows (`design.configure`).
//!
//! Kept in the document, so a file carries its settings for everyone who opens it, and the audit,
//! the fixer and CI all read the same values.

use figcraft_color::Color;
use serde::{Deserialize, Serialize};

/// How strict `design.audit` is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesignPreset {
    /// The rules that catch the usual AI-generated tells.
    #[default]
    Recommended,
    /// Recommended plus the judgment calls (too many accent colours, frames without auto layout).
    Strict,
    /// Only the findings that keep a design usable: contrast, the type floor, gradient text and
    /// clipped text.
    Accessibility,
}

impl DesignPreset {
    pub const ALL: [Self; 3] = [Self::Recommended, Self::Strict, Self::Accessibility];

    pub fn id(self) -> &'static str {
        match self {
            Self::Recommended => "recommended",
            Self::Strict => "strict",
            Self::Accessibility => "accessibility",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "recommended" | "default" => Some(Self::Recommended),
            "strict" => Some(Self::Strict),
            "accessibility" | "a11y" => Some(Self::Accessibility),
            _ => None,
        }
    }
}

/// The smallest and largest spacing grid a file may set.
pub const GRID_MIN: f64 = 1.0;
pub const GRID_MAX: f64 = 64.0;

/// Most radii and brand colours a file keeps.
pub const MAX_RADII: usize = 32;
pub const MAX_BRAND_COLORS: usize = 32;

/// A file's design review settings. The default is the recommended preset on a 4px grid with the
/// 4px radius scale and no brand palette.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DesignReview {
    pub preset: DesignPreset,
    /// The spacing and radius grid in pixels (`0`: 4). A value outside [`GRID_MIN`]..=[`GRID_MAX`]
    /// reads as the default.
    pub grid: f64,
    /// The corner radii the design uses, in pixels. Empty: any multiple of the grid from the grid
    /// up, or a pill.
    pub radii: Vec<f64>,
    /// The brand's accent colours: they don't count as stray accents (`color.accents`).
    pub brand: Vec<Color>,
    /// Rule ids this file turns off.
    pub disabled: Vec<String>,
}

impl DesignReview {
    /// Whether the settings are the default ones (nothing to save).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The grid in pixels, with invalid values read as 4.
    pub fn grid(&self) -> f64 {
        if self.grid.is_finite() && (GRID_MIN..=GRID_MAX).contains(&self.grid) { self.grid } else { 4.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_by_id() {
        for p in DesignPreset::ALL {
            assert_eq!(DesignPreset::from_id(p.id()), Some(p));
        }
        assert_eq!(DesignPreset::from_id("a11y"), Some(DesignPreset::Accessibility));
        assert_eq!(DesignPreset::from_id("loud"), None);
    }

    #[test]
    fn a_default_file_does_not_serialize_the_settings_and_junk_grid_reads_as_four() {
        let doc = crate::Document::new("Untitled");
        let json = serde_json::to_value(&doc).unwrap();
        assert!(json.get("designReview").is_none(), "{json}");
        let mut settings = DesignReview::default();
        assert!(settings.is_default());
        settings.grid = -3.0;
        assert_eq!(settings.grid(), 4.0);
        settings.grid = f64::NAN;
        assert_eq!(settings.grid(), 4.0);
        settings.grid = 8.0;
        assert_eq!(settings.grid(), 8.0);
        assert!(!settings.is_default());
    }
}
