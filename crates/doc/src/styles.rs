//! Shared styles and variables.

use crate::props::{Effect, LayoutGrid};
use crate::text::TextStyle;
use figcraft_color::{Color, Paint};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type StyleId = String;
pub type VariableId = String;
pub type CollectionId = String;
pub type ModeId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StyleKind {
    Paint { paints: Vec<Paint> },
    Text { style: TextStyle },
    Effect { effects: Vec<Effect> },
    Grid { grids: Vec<LayoutGrid> },
}

impl StyleKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            StyleKind::Paint { .. } => "paint",
            StyleKind::Text { .. } => "text",
            StyleKind::Effect { .. } => "effect",
            StyleKind::Grid { .. } => "grid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub id: StyleId,
    /// Slash-separated name: "Brand/Primary".
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    pub kind: StyleKind,
    /// Variables bound to fields of the value, by the keys a layer's bindings use
    /// (`fills/0/color`, `effects/0/radius`, `layoutGrids/0/size`, a text style's `fontSize`…).
    /// A layer using the style sees them resolved in its own modes; `kind` holds them resolved in
    /// the default modes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bound: BTreeMap<String, VariableId>,
}

/// A node's links to shared styles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StyleRefs {
    pub fill: Option<StyleId>,
    pub stroke: Option<StyleId>,
    pub effect: Option<StyleId>,
    pub grid: Option<StyleId>,
    pub text: Option<StyleId>,
}

impl StyleRefs {
    pub fn is_empty(&self) -> bool {
        *self == StyleRefs::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableType {
    Color,
    Number,
    String,
    Boolean,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum VariableValue {
    Color(Color),
    Number(f64),
    String(String),
    Boolean(bool),
    Alias(VariableId),
}

impl VariableValue {
    /// The value a new variable of `kind` starts with.
    pub fn default_for(kind: VariableType) -> Self {
        match kind {
            VariableType::Color => VariableValue::Color(Color::BLACK),
            VariableType::Number => VariableValue::Number(0.0),
            VariableType::String => VariableValue::String(String::new()),
            VariableType::Boolean => VariableValue::Boolean(true),
        }
    }

    pub fn type_of(&self) -> Option<VariableType> {
        match self {
            VariableValue::Color(_) => Some(VariableType::Color),
            VariableValue::Number(_) => Some(VariableType::Number),
            VariableValue::String(_) => Some(VariableType::String),
            VariableValue::Boolean(_) => Some(VariableType::Boolean),
            VariableValue::Alias(_) => None,
        }
    }
    pub fn display(&self) -> String {
        match self {
            VariableValue::Color(c) => c.to_hex(),
            VariableValue::Number(n) => crate::text::trim_num(*n),
            VariableValue::String(s) => s.clone(),
            VariableValue::Boolean(b) => b.to_string(),
            VariableValue::Alias(a) => format!("→ {a}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub id: ModeId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub modes: Vec<Mode>,
    /// An extended collection: the collection (a library copy) it extends. It has that
    /// collection's modes (same ids) and one inheriting variable per variable of it, plus its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<CollectionId>,
}

impl Collection {
    pub fn default_mode(&self) -> Option<&ModeId> {
        self.modes.first().map(|m| &m.id)
    }

    /// The mode with id `key`, else the one named `key`.
    pub fn find_mode(&self, key: &str) -> Option<&Mode> {
        self.modes.iter().find(|m| m.id == key).or_else(|| self.modes.iter().find(|m| m.name == key))
    }

    /// The mode the collection is in under `modes` (collection → chosen mode): the chosen one if it
    /// is one of its modes, else the default.
    pub fn current_mode(&self, modes: &BTreeMap<CollectionId, ModeId>) -> Option<&Mode> {
        modes.get(&self.id).and_then(|m| self.modes.iter().find(|x| x.id == *m)).or_else(|| self.modes.first())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub id: VariableId,
    pub name: String,
    pub collection: CollectionId,
    #[serde(rename = "resolvedType")]
    pub kind: VariableType,
    /// Value per mode id.
    pub values: BTreeMap<ModeId, VariableValue>,
    #[serde(default)]
    pub description: String,
    /// Properties pickers offer the variable for (scope ids from [`scopes_for`]); None = every
    /// supported property, empty = none (hidden from pickers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    /// Left out when the file is published as a library (unless a published variable or component
    /// uses it).
    #[serde(default, rename = "hiddenFromPublishing", skip_serializing_if = "std::ops::Not::not")]
    pub hidden_from_publishing: bool,
    /// In an extended collection: the variable it inherits from. Its values follow that variable's
    /// in every mode not in `overrides`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<VariableId>,
    /// The modes whose value an inheriting variable overrides.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub overrides: BTreeSet<ModeId>,
    /// The name code uses for the variable per platform ([`CODE_PLATFORMS`]: `WEB`, `iOS`,
    /// `ANDROID`), e.g. `var(--space-xs)`; generated code uses the Web one.
    #[serde(default, rename = "codeSyntax", skip_serializing_if = "BTreeMap::is_empty")]
    pub code_syntax: BTreeMap<String, String>,
}

/// The platforms a variable has code syntax for (the variables JSON's `codeSyntax` keys).
pub const CODE_PLATFORMS: [&str; 3] = ["WEB", "iOS", "ANDROID"];

impl Variable {
    /// Whether pickers offer this variable for properties of `scope`.
    pub fn in_scope(&self, scope: &str) -> bool {
        self.scopes.as_ref().is_none_or(|list| list.iter().any(|s| s == scope || s == ALL_SCOPES || (s == "ALL_FILLS" && scope.ends_with("_FILL"))))
    }
}

/// The scope id meaning "every supported property".
pub const ALL_SCOPES: &str = "ALL_SCOPES";

/// The scopes a variable of `kind` can be limited to: (id, label).
pub fn scopes_for(kind: VariableType) -> &'static [(&'static str, &'static str)] {
    match kind {
        VariableType::Number => &[
            ("CORNER_RADIUS", "Corner radius"),
            ("WIDTH_HEIGHT", "Width and height"),
            ("GAP", "Gap and padding"),
            ("OPACITY", "Layer opacity"),
            ("STROKE_FLOAT", "Stroke"),
            ("EFFECT_FLOAT", "Effects"),
            ("FONT_WEIGHT", "Font weight"),
            ("FONT_SIZE", "Font size"),
            ("LINE_HEIGHT", "Line height"),
            ("LETTER_SPACING", "Letter spacing"),
            ("PARAGRAPH_SPACING", "Paragraph spacing"),
            ("PARAGRAPH_INDENT", "Paragraph indent"),
            ("TEXT_CONTENT", "Text content"),
        ],
        VariableType::Color => &[
            ("FRAME_FILL", "Frame fill"),
            ("SHAPE_FILL", "Shape fill"),
            ("TEXT_FILL", "Text fill"),
            ("STROKE_COLOR", "Stroke"),
            ("EFFECT_COLOR", "Effects"),
        ],
        VariableType::String => &[("TEXT_CONTENT", "Text content"), ("FONT_FAMILY", "Font family"), ("FONT_STYLE", "Font style")],
        VariableType::Boolean => &[],
    }
}

/// Whether `scope` is a scope id a variable of `kind` can be limited to (including the "every
/// property" ids).
pub fn scope_known(kind: VariableType, scope: &str) -> bool {
    scope == ALL_SCOPES || (scope == "ALL_FILLS" && kind == VariableType::Color) || scopes_for(kind).iter().any(|(k, _)| *k == scope)
}

/// A variable's scope list in canonical form: known scope ids for `kind` only, without repeats, and
/// None (no limit) when it names every supported property.
pub fn normalize_scopes<'a>(kind: VariableType, scopes: impl IntoIterator<Item = &'a str>) -> Option<Vec<String>> {
    let mut list: Vec<String> = Vec::new();
    for s in scopes.into_iter().filter(|s| scope_known(kind, s)) {
        if !list.iter().any(|l| l == s) {
            list.push(s.to_string());
        }
    }
    let allowed = scopes_for(kind);
    let all = list.iter().any(|l| l == ALL_SCOPES) || (!allowed.is_empty() && allowed.iter().all(|(k, _)| list.iter().any(|l| l == k)));
    if all { None } else { Some(list) }
}

impl VariableType {
    pub fn id(self) -> &'static str {
        match self {
            VariableType::Color => "color",
            VariableType::Number => "number",
            VariableType::String => "string",
            VariableType::Boolean => "boolean",
        }
    }
}

/// All variables of a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Variables {
    pub collections: Vec<Collection>,
    pub variables: Vec<Variable>,
}

impl Variables {
    pub fn get(&self, id: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.id == id || v.name == id)
    }

    /// Look up an already resolved variable id (a stored binding or alias): exact ids only, so a
    /// variable named like an id never stands in for it.
    pub fn get_by_id(&self, id: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.id == id)
    }

    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id || c.name == id)
    }

    /// Resolve `id` in `modes` (collection → chosen mode; default mode otherwise), following aliases
    /// (bounded, cycles return None).
    pub fn resolve(&self, id: &str, modes: &BTreeMap<CollectionId, ModeId>) -> Option<VariableValue> {
        let mut cur = id.to_string();
        for _ in 0..32 {
            let var = self.get_by_id(&cur)?;
            let coll = self.collection(&var.collection)?;
            let mode = modes.get(&coll.id).filter(|m| var.values.contains_key(*m)).or_else(|| coll.default_mode())?;
            match var.values.get(mode).or_else(|| var.values.values().next())? {
                VariableValue::Alias(next) => cur = next.clone(),
                v => return Some(v.clone()),
            }
        }
        None
    }
}
