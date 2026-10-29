//! Every colour value a document holds, for converting a file between colour profiles: paints
//! (fills, strokes, text runs), effects, layout grids, page and prototype backgrounds, overlay
//! backdrops, styles, colour variables and the document colours. Image pixels are not values and
//! are left alone.

use figcraft_color::{Color, ColorProfile, ImageFilters};
use serde::{Deserialize, Serialize};

use crate::proto::Action;
use crate::{Document, Effect, EffectKind, LayoutGrid, Node, NodeKind, Style, StyleKind, TextData, TextStyle, VariableValue, Variables};

/// A visitor over colour values.
pub type ColorVisitor<'a> = dyn FnMut(&mut Color) + 'a;

/// Image adjustments saved in a document under a name (the image paint's preset menu lists them).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImagePreset {
    pub name: String,
    pub filters: ImageFilters,
}

impl Effect {
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        match &mut self.kind {
            EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => f(&mut s.color),
            EffectKind::Noise(n) => {
                f(&mut n.color);
                f(&mut n.secondary);
            }
            EffectKind::LayerBlur { .. } | EffectKind::BackgroundBlur { .. } | EffectKind::Texture(_) | EffectKind::Glass(_) => {}
        }
    }
}

impl TextStyle {
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        self.fills.iter_mut().flatten().for_each(|p| p.for_each_color_mut(f));
        if let Some(c) = self.decoration_look.as_deref_mut().and_then(|l| l.color.as_mut()) {
            f(c);
        }
    }
}

impl TextData {
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        self.style.for_each_color_mut(f);
        self.runs.iter_mut().for_each(|r| r.style.for_each_color_mut(f));
    }
}

impl Node {
    /// Visit every colour of this node (not its children).
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        self.fills.iter_mut().chain(self.strokes.iter_mut()).for_each(|p| p.for_each_color_mut(f));
        self.effects.iter_mut().for_each(|e| e.for_each_color_mut(f));
        self.grids.iter_mut().for_each(|g: &mut LayoutGrid| f(&mut g.color));
        match &mut self.kind {
            NodeKind::Page { background, prototype, .. } => {
                f(background);
                f(&mut prototype.background);
            }
            NodeKind::Text(t) => t.for_each_color_mut(f),
            NodeKind::Vector { network } => {
                network.regions.iter_mut().filter_map(|r| r.fills.as_mut()).flatten().for_each(|p| p.for_each_color_mut(f))
            }
            _ => {}
        }
        for it in &mut self.interactions {
            crate::proto::walk_mut(&mut it.actions, &mut |a| {
                if let Action::OpenOverlay { background: Some(c), .. } = a {
                    f(c);
                }
            });
        }
    }
}

impl Style {
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        match &mut self.kind {
            StyleKind::Paint { paints } => paints.iter_mut().for_each(|p| p.for_each_color_mut(f)),
            StyleKind::Text { style } => style.for_each_color_mut(f),
            StyleKind::Effect { effects } => effects.iter_mut().for_each(|e| e.for_each_color_mut(f)),
            StyleKind::Grid { grids } => grids.iter_mut().for_each(|g| f(&mut g.color)),
        }
    }
}

impl Variables {
    pub fn for_each_color_mut(&mut self, f: &mut ColorVisitor) {
        for v in self.variables.iter_mut().flat_map(|v| v.values.values_mut()) {
            if let VariableValue::Color(c) = v {
                f(c);
            }
        }
    }
}

impl Document {
    /// Replace every colour value in the document with `map(value)`.
    pub fn map_colors(&mut self, map: impl Fn(Color) -> Color) {
        let mut f = |c: &mut Color| *c = map(*c);
        let ids: Vec<_> = self.ids().collect();
        for id in ids {
            if let Some(n) = self.node_mut(id) {
                n.for_each_color_mut(&mut f);
            }
        }
        self.styles.iter_mut().for_each(|s| s.for_each_color_mut(&mut f));
        self.variables.for_each_color_mut(&mut f);
        self.document_colors.iter_mut().for_each(&mut f);
    }

    /// Switch the document to `profile`. With `convert`, colour values are converted so that
    /// colours keep their appearance; without, the values stay and are reinterpreted in the new
    /// space. Returns whether anything changed.
    pub fn set_color_profile(&mut self, profile: ColorProfile, convert: bool) -> bool {
        if self.color_profile == profile {
            return false;
        }
        if convert {
            let conv = self.color_profile.conversion_to(profile);
            self.map_colors(|c| conv.apply(c));
        }
        self.color_profile = profile;
        true
    }

    /// This document with its colour values in sRGB (a converted copy when it is in another
    /// profile), for formats whose colours are always sRGB.
    pub fn in_srgb(&self) -> std::borrow::Cow<'_, Document> {
        if self.color_profile.is_srgb() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut d = self.clone();
        d.set_color_profile(ColorProfile::Srgb, true);
        std::borrow::Cow::Owned(d)
    }
}
