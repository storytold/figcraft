use super::paint::paint_bound;
use super::{Kv, Mapper, Obj, Value, View};

/// One part of a layer's look that a swap can leave as a copy of the previous main's: the wire
/// keys that hold it, the REST keys `look` writes for it, and the `styles` slot it links with
/// (and the wire keys of that link).
struct Group {
    wire: &'static [&'static str],
    rest: &'static [&'static str],
    style: Option<(&'static str, &'static str, &'static str)>,
    /// The paints key whose own variable bindings `look` draws over the linked style.
    bound: Option<&'static str>,
}

const GROUPS: [Group; 5] = [
    Group {
        wire: &["fillPaints", "styleIdForFill", "inheritFillStyleID"],
        rest: &["fills"],
        style: Some(("fill", "styleIdForFill", "inheritFillStyleID")),
        bound: Some("fillPaints"),
    },
    Group {
        wire: &["strokePaints", "styleIdForStrokeFill", "inheritFillStyleIDForStroke"],
        rest: &["strokes"],
        style: Some(("stroke", "styleIdForStrokeFill", "inheritFillStyleIDForStroke")),
        bound: Some("strokePaints"),
    },
    Group {
        wire: &[
            "strokeWeight",
            "strokeAlign",
            "strokeJoin",
            "strokeCap",
            "dashPattern",
            "miterLimit",
            "borderStrokeWeightsIndependent",
            "borderTopWeight",
            "borderRightWeight",
            "borderBottomWeight",
            "borderLeftWeight",
            "variableWidthPoints",
        ],
        rest: &[
            "strokeWeight",
            "strokeAlign",
            "strokeJoin",
            "strokeCap",
            "strokeDashes",
            "strokeMiterAngle",
            "individualStrokeWeights",
            "strokeWidthProfile",
        ],
        style: None,
        bound: None,
    },
    Group {
        wire: &["effects", "styleIdForEffect", "inheritEffectStyleID"],
        rest: &["effects"],
        style: Some(("effect", "styleIdForEffect", "inheritEffectStyleID")),
        bound: None,
    },
    Group {
        wire: &[
            "cornerRadius",
            "cornerSmoothing",
            "rectangleCornerRadiiIndependent",
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
        ],
        rest: &["cornerRadius", "cornerSmoothing", "rectangleCornerRadii"],
        style: None,
        bound: None,
    },
];

impl<'a> Mapper<'a> {
    /// A nested instance swapped to another main: Figma stores the previous main's look on the
    /// instance node, so `common()` drew that. Every group of the look whose stored values are a
    /// copy of the previous main's takes the new main's instead; a group the node or a row above
    /// it changes is kept. Rows sit in `view` above `base`, so they win in the new look too.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn swapped_look(
        &mut self,
        o: &mut Obj,
        view: &View<'a>,
        base: &Kv<'a>,
        old: Option<&'a Kv<'a>>,
        new_main: &'a Kv<'a>,
        scale: f64,
        ty: &str,
    ) {
        let copied: Vec<&Group> = GROUPS
            .iter()
            .filter(|g| {
                let Some(old) = old else { return true };
                if g.wire.iter().all(|k| base.get(k) == old.get(k)) {
                    return true;
                }
                // Own paints bound to variables are drawn over the linked style (look, #399): paints that
                // differ from the old main's are an override even under the same link.
                if g.bound.is_some_and(|k| base.get(k) != old.get(k) && base.get(k).map(Kv::list).unwrap_or_default().iter().any(paint_bound)) {
                    return false;
                }
                // The same linked style is a stale copy of the style's paints: `look` draws the style.
                g.style.is_some_and(|(_, key, legacy)| {
                    let (a, b) = (self.style_link(&View::of(base), key, legacy), self.style_link(&View::of(old), key, legacy));
                    a.is_some() && a == b
                })
            })
            .collect();
        if old.is_none() {
            self.warn("swapped instances whose previous component isn't in the file took the new component's look");
        }
        if copied.is_empty() {
            return;
        }
        let look = View::of_scaled(new_main, scale).with(view.layers.iter().copied().zip(view.scales.iter().copied()).skip(1));
        // Warnings from this run repeat those `common()` gave for the same style links; counted twice at worst.
        let mut alt = Obj::new();
        self.look(&mut alt, &look, ty);
        // A row that sets a group's paints above every layer linking a style wins over that style: it
        // was written against a previous main whose paints had no style to detach. That group is
        // drawn from the look from that row up.
        let top = |k: &str| look.layers.iter().rposition(|l| l.get(k).is_some());
        let mut over_style: Vec<(&Group, Obj)> = Vec::new();
        for g in &copied {
            let Some((_, key, legacy)) = g.style else { continue };
            let (Some(p), Some(st)) = (g.wire.first().and_then(|k| top(k)), top(key).max(top(legacy))) else { continue };
            if p <= st {
                continue;
            }
            let (Some(layer), Some(scale)) = (look.layers.get(p), look.scales.get(p)) else { continue };
            let upper = look.layers.iter().copied().zip(look.scales.iter().copied()).skip(p + 1);
            let mut from_row = Obj::new();
            self.look(&mut from_row, &View::of_scaled(layer, *scale).with(upper), ty);
            over_style.push((*g, from_row));
        }
        let alt_styles = alt.get("styles").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut styles = o.get("styles").and_then(Value::as_object).cloned().unwrap_or_default();
        for g in copied {
            let row_look = over_style.iter().find(|(r, _)| std::ptr::eq(*r, g)).map(|(_, l)| l);
            let (alt, alt_styles) = match row_look {
                Some(l) => (l, l.get("styles").and_then(Value::as_object).cloned().unwrap_or_default()),
                None => (&alt, alt_styles.clone()),
            };
            for k in g.rest {
                o.remove(*k);
                if let Some(v) = alt.get(*k) {
                    o.insert((*k).into(), v.clone());
                }
            }
            if let Some((slot, _, _)) = g.style {
                styles.remove(slot);
                if let Some(v) = alt_styles.get(slot) {
                    styles.insert(slot.into(), v.clone());
                }
            }
        }
        if styles.is_empty() {
            o.remove("styles");
        } else {
            o.insert("styles".into(), Value::Object(styles));
        }
    }
}
