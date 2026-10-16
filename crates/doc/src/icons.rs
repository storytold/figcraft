//! Icon fonts: shapes turned into glyphs of a font the document carries with it.
//!
//! An [`IconFont`] is a named group of [`IconGlyph`]s. Each glyph keeps a stable id, the name it was
//! made from, the Unicode code point it answers to and its outline as SVG path data, normalised to
//! the em square (y down). The text crate compiles a font to a real TTF at runtime and registers it,
//! so a text layer can show an icon by selecting the font and typing the code point.
//!
//! Everything here is plain data: no font is built and no glyph is drawn in this crate.

use serde::{Deserialize, Serialize};

/// First code point of the Unicode Private Use Area, where icon fonts are mapped.
pub const ICON_PUA_START: u32 = 0xE000;
/// Last code point of the Basic Multilingual Plane Private Use Area.
pub const ICON_PUA_END: u32 = 0xF8FF;

/// The em square glyph outlines are stored in: the units per em of the fonts they compile to.
///
/// The text crate's `iconfont::UPEM`, which is what actually compiles a font, is defined from this
/// number, so storage and compilation can never disagree.
pub const EM: u32 = 1000;
/// First code point of Plane 15's Private Use Area, used once the BMP one is full.
pub const ICON_PUA_SUPPLEMENTARY_START: u32 = 0xF_0000;
/// Last code point of Plane 15's Private Use Area.
pub const ICON_PUA_SUPPLEMENTARY_END: u32 = 0xF_FFFD;

/// One icon: an id, a name, the code point it is mapped to and its outline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IconGlyph {
    /// Stable id, kept when the icon is renamed: a layer that draws this icon stores the id, so
    /// renaming the icon must not leave the layer pointing at a name that is gone.
    ///
    /// Empty in a file written before icons kept ids — there the name stands in (see
    /// [`IconFont::glyph_by_id`]).
    #[serde(default)]
    pub id: String,
    /// The icon's name (the name of the layer it was made from), shown in the panel.
    pub name: String,
    /// The Unicode code point the icon is mapped to.
    pub codepoint: u32,
    /// The outline as SVG path data ([`BezPath::to_svg`](figcraft_geom::BezPath::to_svg)), scaled
    /// into the em square and with y pointing down.
    pub path: String,
    /// The layer the outline came from, so the icon can be re-extracted ([`crate::Document`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<crate::NodeId>,
}

/// A set of icons compiled into one font family.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IconFont {
    /// Stable id, kept when the font is renamed (commands take it or the name).
    pub id: String,
    /// The display name, which is also the font family name the font picker shows.
    pub name: String,
    /// The icons of this font, in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glyphs: Vec<IconGlyph>,
}

impl IconFont {
    /// The first code point of the Private Use Area this font does not use yet.
    ///
    /// The BMP Private Use Area (U+E000–U+F8FF) comes first; when it is full, Plane 15's
    /// (U+F0000–U+FFFD) is used.
    pub fn next_codepoint(&self) -> u32 {
        let used = |c: u32| self.glyphs.iter().any(|g| g.codepoint == c);
        (ICON_PUA_START..=ICON_PUA_END)
            .find(|c| !used(*c))
            .or_else(|| (ICON_PUA_SUPPLEMENTARY_START..=ICON_PUA_SUPPLEMENTARY_END).find(|c| !used(*c)))
            // Every Private Use Area code point is taken: reuse the first one.
            .unwrap_or(ICON_PUA_START)
    }

    /// The icon with this name.
    pub fn glyph(&self, name: &str) -> Option<&IconGlyph> {
        self.glyphs.iter().find(|g| g.name == name)
    }

    /// The icon with this id — or, for a file written before icons kept ids, the icon with this
    /// name: the key a layer stores used to be the name, and those files have no ids in them.
    pub fn glyph_by_id(&self, key: &str) -> Option<&IconGlyph> {
        self.glyphs.iter().find(|g| g.id == key).or_else(|| self.glyphs.iter().find(|g| g.name == key))
    }

    /// The icon at this code point.
    pub fn glyph_at(&self, codepoint: u32) -> Option<&IconGlyph> {
        self.glyphs.iter().find(|g| g.codepoint == codepoint)
    }

    /// `base`, or `base 2`, `base 3`… when that name is taken in this font.
    pub fn fresh_glyph_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Icon" } else { base };
        if self.glyph(base).is_none() {
            return base.to_string();
        }
        (2..10_000).map(|k| format!("{base} {k}")).find(|n| self.glyph(n).is_none()).unwrap_or_else(|| base.to_string())
    }

    /// An id no icon of this font uses yet.
    pub fn fresh_glyph_id(&self) -> String {
        (1..100_000)
            .map(|k| format!("iconglyph-{k}"))
            .find(|id| !self.glyphs.iter().any(|g| g.id == *id))
            .unwrap_or_else(|| format!("iconglyph-{}", self.glyphs.len() + 1))
    }
}

impl crate::Document {
    /// The document's icon fonts.
    pub fn icon_fonts(&self) -> &[IconFont] {
        &self.icon_fonts
    }

    /// The index of the icon font with this id or name.
    pub fn icon_font_index(&self, key: &str) -> Option<usize> {
        self.icon_fonts.iter().position(|f| f.id == key || f.name == key)
    }

    /// The icon font with this id or name.
    pub fn icon_font(&self, key: &str) -> Option<&IconFont> {
        self.icon_font_index(key).and_then(|i| self.icon_fonts.get(i))
    }

    /// The icon font with this id or name, for editing.
    pub fn icon_font_mut(&mut self, key: &str) -> Option<&mut IconFont> {
        let i = self.icon_font_index(key)?;
        self.icon_fonts.get_mut(i)
    }

    /// `base`, or `base 2`, `base 3`… when an icon font of that name exists already.
    pub fn fresh_icon_font_name(&self, base: &str) -> String {
        let base = base.trim();
        let base = if base.is_empty() { "Icons" } else { base };
        if self.icon_font(base).is_none() {
            return base.to_string();
        }
        (2..10_000).map(|k| format!("{base} {k}")).find(|n| self.icon_font(n).is_none()).unwrap_or_else(|| base.to_string())
    }

    /// An id no icon font uses yet (also unique against a font added in the same command).
    pub fn fresh_icon_font_id(&self) -> String {
        (1..10_000)
            .map(|k| format!("iconfont-{k}"))
            .find(|id| !self.icon_fonts.iter().any(|f| f.id == *id))
            .unwrap_or_else(|| format!("iconfont-{}", self.icon_fonts.len() + 1))
    }

    /// The glyph an [`crate::NodeKind::Icon`] node is: its font (by the id or the name the node
    /// stores) and the icon of that id in it.
    ///
    /// `None` when the font is gone, or the icon was deleted from it: the node then has nothing to
    /// draw, which is what tells a caller the icon is stale (see `icon.detach`).
    pub fn icon_of(&self, font: &str, glyph: &str) -> Option<&IconGlyph> {
        self.icon_font(font)?.glyph_by_id(glyph)
    }

    /// Every icon node that draws the icon `key` of `font` (the icon's id, or its name for a layer
    /// written before icons kept ids).
    pub fn icon_nodes_of(&self, font: &str, key: &str) -> Vec<crate::NodeId> {
        self.nodes
            .iter()
            .filter(|(_, n)| matches!(&n.kind, crate::NodeKind::Icon { font: f, glyph: g } if f == font && g == key))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Every icon node that draws any icon of `font`.
    pub fn icon_nodes_in(&self, font: &str) -> Vec<crate::NodeId> {
        self.nodes.iter().filter(|(_, n)| matches!(&n.kind, crate::NodeKind::Icon { font: f, .. } if f == font)).map(|(id, _)| *id).collect()
    }

    /// The innermost icon containing `id` (`id` itself when it is an icon).
    ///
    /// A layer inside an icon belongs to it the way a layer inside a component belongs to that:
    /// the UI shows its outline and its name as the icon's.
    pub fn owning_icon(&self, id: crate::NodeId) -> Option<crate::NodeId> {
        let mut chain = vec![id];
        chain.extend(self.ancestors(id));
        chain.into_iter().find(|c| self.node(*c).is_some_and(|n| matches!(n.kind, crate::NodeKind::Icon { .. })))
    }
}

#[cfg(test)]
#[path = "icons_tests.rs"]
mod tests;
