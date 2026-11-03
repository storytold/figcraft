//! The rules `design.audit` checks, with the severity and fixability the report carries.

use figcraft_doc::DesignPreset;

/// How much a finding matters: `High` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
        }
    }

    pub(crate) fn from_id(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Severity::Low),
            "medium" | "med" => Some(Severity::Medium),
            "high" => Some(Severity::High),
            _ => None,
        }
    }

    /// `0` for low, `1` for medium, `2` for high.
    pub(crate) fn index(self) -> usize {
        match self {
            Severity::Low => 0,
            Severity::Medium => 1,
            Severity::High => 2,
        }
    }

    /// The points a finding of this severity takes off the score.
    pub(crate) fn penalty(self) -> f64 {
        match self {
            Severity::Low => 0.5,
            Severity::Medium => 2.0,
            Severity::High => 5.0,
        }
    }
}

/// One check: what it looks for, how much it matters and whether `design.fix` can repair it.
pub(crate) struct Rule {
    pub id: &'static str,
    pub severity: Severity,
    /// The planned fixer can change this without guessing (see `plan/design-review.md`).
    pub fixable: bool,
    pub description: &'static str,
}

pub(crate) const RULES: &[Rule] = &[
    Rule { id: "spacing.off-grid", severity: Severity::Medium, fixable: true, description: "Auto layout gap or padding is not on the 4px grid" },
    Rule { id: "radius.off-scale", severity: Severity::Low, fixable: true, description: "A corner radius is not on the 4px scale or a pill" },
    Rule { id: "type.sizes", severity: Severity::Medium, fixable: false, description: "More than five font sizes in one scope" },
    Rule { id: "type.families", severity: Severity::Medium, fixable: false, description: "More than two font families in one scope" },
    Rule { id: "type.weight", severity: Severity::Low, fixable: true, description: "A heading uses a weight above 600" },
    Rule { id: "type.italic", severity: Severity::Low, fixable: true, description: "Interface text is italic" },
    Rule { id: "type.too-small", severity: Severity::Medium, fixable: false, description: "Body text is smaller than 11px" },
    Rule { id: "type.negative-tracking", severity: Severity::Low, fixable: true, description: "Small text has negative letter spacing" },
    Rule { id: "color.gradient", severity: Severity::Medium, fixable: false, description: "A shape is filled or stroked with a gradient" },
    Rule { id: "color.gradient-text", severity: Severity::High, fixable: false, description: "Text is painted with a gradient" },
    Rule {
        id: "color.glow",
        severity: Severity::Medium,
        fixable: true,
        description: "A shadow has no offset and a large blur, so it reads as a glow",
    },
    Rule { id: "color.colored-shadow", severity: Severity::Low, fixable: true, description: "A shadow has a visible hue instead of a neutral" },
    Rule { id: "color.border-opacity", severity: Severity::Low, fixable: true, description: "A border is more opaque than a hairline" },
    Rule { id: "color.contrast", severity: Severity::High, fixable: true, description: "Text contrast is below 4.5:1 (3:1 for large text)" },
    Rule { id: "color.accents", severity: Severity::Medium, fixable: false, description: "More than two accent colours in one scope (Strict)" },
    Rule { id: "token.hardcoded-color", severity: Severity::Low, fixable: false, description: "A colour matches a variable but is not bound to it" },
    Rule {
        id: "token.hardcoded-number",
        severity: Severity::Low,
        fixable: false,
        description: "A spacing, radius, size or opacity value matches a variable but is not bound",
    },
    Rule { id: "icon.emoji", severity: Severity::Low, fixable: false, description: "Text or a layer name contains an emoji" },
    Rule { id: "copy.exclamation", severity: Severity::Low, fixable: false, description: "Interface copy uses an exclamation mark" },
    Rule { id: "copy.trailing-period", severity: Severity::Low, fixable: false, description: "A short label ends with a period" },
    Rule { id: "copy.hype", severity: Severity::Low, fixable: false, description: "Copy uses hype words such as \"AI-powered\"" },
    Rule { id: "naming.auto", severity: Severity::Low, fixable: false, description: "A layer still has its default auto name (Frame 12)" },
    Rule { id: "detail.shadow-count", severity: Severity::Low, fixable: false, description: "Shadows and blurs sit on too many layers" },
    Rule {
        id: "layout.no-auto-layout",
        severity: Severity::Low,
        fixable: false,
        description: "A frame or component with several children has no auto layout (Strict)",
    },
    Rule {
        id: "layout.text-overflow",
        severity: Severity::High,
        fixable: false,
        description: "A fixed-width text box is shorter than the text it holds, so it clips or overlaps",
    },
    Rule {
        id: "layout.clipped",
        severity: Severity::High,
        fixable: false,
        description: "A fixed-size auto layout frame clips children that overflow it, so they are invisible",
    },
];

pub(crate) fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

/// Whether `preset` checks `id`. Recommended and Strict check every rule except the Strict-only
/// ones; Accessibility checks only the findings that keep a design usable.
pub(crate) fn enabled(id: &str, preset: DesignPreset) -> bool {
    match preset {
        DesignPreset::Recommended => !matches!(id, "color.accents" | "layout.no-auto-layout"),
        DesignPreset::Strict => true,
        DesignPreset::Accessibility => {
            matches!(id, "color.contrast" | "color.gradient-text" | "type.too-small" | "layout.text-overflow" | "layout.clipped")
        }
    }
}

/// The presets that check `id`, for `design.rules`.
pub(crate) fn presets_of(id: &str) -> Vec<&'static str> {
    DesignPreset::ALL.iter().copied().filter(|p| enabled(id, *p)).map(DesignPreset::id).collect()
}
