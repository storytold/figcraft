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
    /// The referenced colour with its alpha channel replaced (a "50% variant"). Replace rather
    /// than scale: applying it twice is the same value (stable under editing the opacity or the
    /// chain it reads), and "at 50%" reads as the design ask.
    ColorAlpha {
        variable: VariableId,
        opacity: f32,
    },
    /// A linear blend of two colour operands at `t` (0 = `from`, 1 = `to`). The space is the stored
    /// one — non-premultiplied components in the document's colour profile, alpha blended like the
    /// channels — deliberately not perceptual: the values a hex panel can show blend exactly, and
    /// 8-bit rounding stays where it already was (display and export) rather than being done twice.
    /// The trade-off: converting the profile keeps the ends exact but moves the colours in between.
    ColorMix {
        from: ColorOperand,
        to: ColorOperand,
        t: f32,
    },
}

/// One operand of a colour calculation: a variable (resolved when read, so it follows its own modes,
/// aliases and calculations) or a fixed colour. Tagged by key, not by type: a bare string is a
/// colour name just as easily as a variable name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorOperand {
    Variable { variable: VariableId },
    Literal { literal: Color },
}

impl ColorOperand {
    /// The variable it reads, if it reads one.
    pub fn variable(&self) -> Option<&VariableId> {
        match self {
            ColorOperand::Variable { variable } => Some(variable),
            ColorOperand::Literal { .. } => None,
        }
    }

    /// The variable id it reads, for a remap (library import and swap re-point ids).
    pub fn variable_mut(&mut self) -> Option<&mut VariableId> {
        match self {
            ColorOperand::Variable { variable } => Some(variable),
            ColorOperand::Literal { .. } => None,
        }
    }
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

    /// The type this value has on its own, which a typed literal needs. None for references
    /// (aliases and colour calculations): what they read may be missing or loop, so only
    /// resolving them gives a type, and an expression can't take one as a literal.
    pub fn type_of(&self) -> Option<VariableType> {
        match self {
            VariableValue::Color(_) => Some(VariableType::Color),
            VariableValue::Number(_) => Some(VariableType::Number),
            VariableValue::String(_) => Some(VariableType::String),
            VariableValue::Boolean(_) => Some(VariableType::Boolean),
            VariableValue::Alias(_) | VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => None,
        }
    }
    /// The value on one line, as the panel shows it: `→ id` for an alias, `α 50% · id` and
    /// `from + to @ 40%` for colour calculations, variables by id ([`Self::display_with`] names
    /// them).
    pub fn display(&self) -> String {
        self.display_with(&|id| id.to_string())
    }

    /// As [`Self::display`], with the variables it reads shown by `name` (the panel shows their
    /// names where ids would mean nothing to a reader).
    pub fn display_with(&self, name: &dyn Fn(&str) -> String) -> String {
        let percent = |x: f32| (x * 100.0).round();
        let operand = |op: &ColorOperand| match op {
            ColorOperand::Variable { variable } => name(variable),
            // As the panel shows a colour (no `#`), with alpha digits when it isn't opaque.
            ColorOperand::Literal { literal } => literal.to_css_hex().trim_start_matches('#').to_string(),
        };
        match self {
            VariableValue::Color(c) => c.to_hex(),
            VariableValue::Number(n) => crate::text::trim_num(*n),
            VariableValue::String(s) => s.clone(),
            VariableValue::Boolean(b) => b.to_string(),
            VariableValue::Alias(a) => format!("→ {}", name(a)),
            VariableValue::ColorAlpha { variable, opacity } => format!("α {}% · {}", percent(*opacity), name(variable)),
            VariableValue::ColorMix { from, to, t } => format!("{} + {} @ {}%", operand(from), operand(to), percent(*t)),
        }
    }

    /// The variable ids this value reads (its alias target, or its calculation's variable operands;
    /// literal operands read nothing). Used for reference walks: cycles, deletion, library remaps.
    pub fn references(&self) -> Vec<&VariableId> {
        match self {
            VariableValue::Alias(a) => vec![a],
            VariableValue::ColorAlpha { variable, .. } => vec![variable],
            VariableValue::ColorMix { from, to, .. } => from.variable().into_iter().chain(to.variable()).collect(),
            _ => Vec::new(),
        }
    }

    /// [`Self::references`], to re-point: library import and swap move them to other ids.
    pub fn references_mut(&mut self) -> Vec<&mut VariableId> {
        match self {
            VariableValue::Alias(a) => vec![a],
            VariableValue::ColorAlpha { variable, .. } => vec![variable],
            VariableValue::ColorMix { from, to, .. } => from.variable_mut().into_iter().chain(to.variable_mut()).collect(),
            _ => Vec::new(),
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

    /// The mode with id `key`, else the one named `key` (what someone typed).
    pub fn find_mode(&self, key: &str) -> Option<&Mode> {
        self.mode_by_id(key).or_else(|| self.modes.iter().find(|m| m.name == key))
    }

    /// The mode with id `id` exactly: a stored reference, which no mode's name stands in for (#1613).
    pub fn mode_by_id(&self, id: &str) -> Option<&Mode> {
        self.modes.iter().find(|m| m.id == id)
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
    /// Its value in `mode`, or its first value when it has none there: an export writes every
    /// mode of the collection, given a value in each or not.
    pub fn value_or_first(&self, mode: &str) -> Option<&VariableValue> {
        self.values.get(mode).or_else(|| self.values.values().next())
    }

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
    /// The variable a person named: the one with id `key`, else the one named `key`. A stored
    /// reference goes through [`get_by_id`](Self::get_by_id) instead.
    pub fn get(&self, key: &str) -> Option<&Variable> {
        self.get_by_id(key).or_else(|| self.variables.iter().find(|v| v.name == key))
    }

    /// Look up an already resolved variable id (a stored binding or alias): exact ids only, so a
    /// variable named like an id never stands in for it.
    pub fn get_by_id(&self, id: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.id == id)
    }

    /// The collection a person named: the one with id `key`, else the one named `key`. A stored
    /// reference goes through [`collection_by_id`](Self::collection_by_id) instead.
    pub fn collection(&self, key: &str) -> Option<&Collection> {
        self.collection_by_id(key).or_else(|| self.collections.iter().find(|c| c.name == key))
    }

    /// [`get_by_id`](Self::get_by_id) for collections: a stored collection id (a variable's own, a
    /// prototype action's, a library record's) never matches another collection's name (#1613).
    pub fn collection_by_id(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    /// The distinct variables one resolve may consult. Calculations read trees, not chains, so the
    /// cap counts variables rather than hops; with the memo each one is resolved at most once per
    /// call, so the work is bounded by this however the graph is shaped (a diamond re-sharing a
    /// subtree does not walk it twice).
    pub const RESOLVE_LIMIT: usize = 32;

    /// Resolve `id` in `modes` (collection → chosen mode; default mode otherwise), following aliases
    /// and evaluating colour calculations (bounded, cycles and over-deep graphs return None).
    pub fn resolve(&self, id: &str, modes: &BTreeMap<CollectionId, ModeId>) -> Option<VariableValue> {
        self.resolve_memo(id, modes, &mut BTreeMap::new())
    }

    /// `val`, a value of `var` shown in `mode`, as a format without colour calculations writes it:
    /// a calculation becomes the colour it evaluates to there (other collections in their default
    /// modes), anything else stays as given. None: the calculation doesn't resolve.
    pub fn flattened(&self, var: &Variable, mode: &str, val: &VariableValue) -> Option<VariableValue> {
        self.flattened_as(&var.id, &var.collection, mode, val)
    }

    /// [`Self::flattened`] for a value of variable `id` in `collection`, which need not exist yet.
    pub fn flattened_as(&self, id: &str, collection: &str, mode: &str, val: &VariableValue) -> Option<VariableValue> {
        match val {
            VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => {
                // `id` is in flight from the start, so a calculation that reads it back resolves to
                // nothing, as it would through `resolve`.
                let mut done = BTreeMap::from([(id.to_string(), None)]);
                self.eval(val, &BTreeMap::from([(collection.to_string(), mode.to_string())]), &mut done)
            }
            other => Some(other.clone()),
        }
    }

    /// Values as an export without colour calculations writes them, references included: see
    /// [`Flattener::value`].
    pub fn flattener(&self) -> Flattener<'_> {
        let mut out = BTreeSet::new();
        for var in &self.variables {
            let Some(coll) = self.collection_by_id(&var.collection) else { continue };
            if coll.modes.iter().any(|m| var.value_or_first(&m.id).is_some_and(|v| self.flattened(var, &m.id, v).is_none())) {
                out.insert(var.id.as_str());
            }
        }
        // Then every alias that leads to one, back along alias chains.
        let mut readers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for var in &self.variables {
            for value in var.values.values() {
                if let VariableValue::Alias(target) = value {
                    readers.entry(target.as_str()).or_default().push(var.id.as_str());
                }
            }
        }
        let mut stack: Vec<&str> = out.iter().copied().collect();
        while let Some(id) = stack.pop() {
            for reader in readers.get(id).into_iter().flatten() {
                if out.insert(reader) {
                    stack.push(reader);
                }
            }
        }
        Flattener { vars: self, unreferenceable: out }
    }

    /// The stored value of `id` in its collection's current mode (the mode chosen when it has a
    /// value for it, else the default mode, else any value).
    fn value_in(&self, id: &str, modes: &BTreeMap<CollectionId, ModeId>) -> Option<&VariableValue> {
        let var = self.get_by_id(id)?;
        let coll = self.collection_by_id(&var.collection)?;
        let mode = modes.get(&coll.id).filter(|m| var.values.contains_key(*m)).or_else(|| coll.default_mode())?;
        var.values.get(mode).or_else(|| var.values.values().next())
    }

    /// [`Self::resolve`] with `done` shared by every variable one call reads: each is resolved once
    /// and its outcome kept, one met again while still in flight is a cycle (None), and the call
    /// reads at most [`Self::RESOLVE_LIMIT`] variables.
    fn resolve_memo(
        &self,
        id: &str,
        modes: &BTreeMap<CollectionId, ModeId>,
        done: &mut BTreeMap<String, Option<VariableValue>>,
    ) -> Option<VariableValue> {
        if let Some(hit) = done.get(id) {
            return hit.clone();
        }
        if done.len() >= Self::RESOLVE_LIMIT {
            return None;
        }
        // Mark as being resolved before recursing: meeting it again mid-flight is a cycle, and the
        // stored None unwinds it. Overwritten with the outcome below (a cached failure stays cached:
        // a variable resolves the same however deep it is reached from — the limit only bails, it
        // does not cache).
        done.insert(id.to_string(), None);
        let out = self.value_in(id, modes).and_then(|val| self.eval(val, modes, done));
        done.insert(id.to_string(), out.clone());
        out
    }

    /// A stored value reduced to a plain one (aliases walked, calculations evaluated).
    fn eval(
        &self,
        val: &VariableValue,
        modes: &BTreeMap<CollectionId, ModeId>,
        done: &mut BTreeMap<String, Option<VariableValue>>,
    ) -> Option<VariableValue> {
        match val {
            VariableValue::Alias(next) => self.resolve_memo(next, modes, done),
            VariableValue::ColorAlpha { variable, opacity } => match self.resolve_memo(variable, modes, done)? {
                VariableValue::Color(c) => Some(VariableValue::Color(c.with_alpha(*opacity))),
                _ => None,
            },
            VariableValue::ColorMix { from, to, t } => {
                let a = self.operand_color(from, modes, done)?;
                let b = self.operand_color(to, modes, done)?;
                Some(VariableValue::Color(mix_colors(&a, &b, *t)))
            }
            other => Some(other.clone()),
        }
    }

    /// A colour operand: a literal as stored, or a variable that must itself resolve to a colour.
    fn operand_color(
        &self,
        op: &ColorOperand,
        modes: &BTreeMap<CollectionId, ModeId>,
        done: &mut BTreeMap<String, Option<VariableValue>>,
    ) -> Option<Color> {
        match op {
            ColorOperand::Literal { literal } => Some(literal.sanitized()),
            ColorOperand::Variable { variable } => match self.resolve_memo(variable, modes, done)? {
                VariableValue::Color(c) => Some(c),
                _ => None,
            },
        }
    }
}

/// Linear per-channel interpolation (`t` clamped and NaN-guarded: a command validates, a crafted
/// file must still resolve to a sane colour).
/// Values as an export that leaves out what resolves to nothing writes them ([`Variables::flattener`]).
pub struct Flattener<'v> {
    vars: &'v Variables,
    /// Colour calculations that resolve to nothing in some mode of their collection, and every
    /// alias that leads to one.
    unreferenceable: BTreeSet<&'v str>,
}

impl Flattener<'_> {
    /// `val`, a value of `var` in `mode`, as [`Variables::flattened`] writes it, and an alias that
    /// leads to a calculation some mode leaves out also as the value it shows here: a reference to
    /// it would read nothing whenever that mode applies. None: it shows nothing in this mode.
    pub fn value(&self, var: &Variable, mode: &str, val: &VariableValue) -> Option<VariableValue> {
        match self.vars.flattened(var, mode, val)? {
            VariableValue::Alias(target) if self.unreferenceable.contains(target.as_str()) => {
                self.vars.resolve(&var.id, &BTreeMap::from([(var.collection.clone(), mode.to_string())]))
            }
            other => Some(other),
        }
    }
}

fn mix_colors(a: &Color, b: &Color, t: f32) -> Color {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    // Weighted at both ends so t = 0 and t = 1 give each operand exactly: `x + (y - x) * t` can
    // miss `y` by a rounding step (from 1.0 to 1e-8 it lands on 0).
    let lerp = |x: f32, y: f32| x * (1.0 - t) + y * t;
    Color::new(lerp(a.r, b.r), lerp(a.g, b.g), lerp(a.b, b.b), lerp(a.a, b.a))
}
