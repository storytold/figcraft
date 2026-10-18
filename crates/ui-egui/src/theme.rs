//! Design tokens (UI3 light and dark) and fonts.

use std::sync::Arc;

use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub canvas: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub icon: Color32,
    pub icon_secondary: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub field: Color32,
    pub field_hover_border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_text: Color32,
    pub selected_row: Color32,
    pub selected_row_child: Color32,
    pub component: Color32,
    pub component_row: Color32,
    /// The colour an icon layer is marked in (see [`ICON`]), and the row it is selected on.
    pub icon_layer: Color32,
    pub icon_layer_row: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub guide: Color32,
    pub tab_bar: Color32,
    pub tab_active: Color32,
    pub shadow: Color32,
    pub menu: Color32,
    pub menu_text: Color32,
    pub menu_hover: Color32,
    pub menu_dim: Color32,
    /// The ring around the control with the keyboard (keyboard navigation): at least 3:1 against
    /// the panels and the canvas.
    pub focus: Color32,
}

pub const ACCENT: Color32 = Color32::from_rgb(0x0D, 0x99, 0xFF);
pub const COMPONENT: Color32 = Color32::from_rgb(0x97, 0x47, 0xFF);
/// An icon layer, and every layer inside one: icons are their own kind of layer, so they get their
/// own colour — orange, next to the purple a component gets and the blue everything else gets.
pub const ICON: Color32 = Color32::from_rgb(0xD9, 0x7A, 0x06);
/// Handoff status badges (white text on these, in both themes).
pub const DEV_READY: Color32 = Color32::from_rgb(0x14, 0xAE, 0x5C);
pub const DEV_CHANGED: Color32 = Color32::from_rgb(0xD9, 0x7A, 0x06);
pub const DEV_COMPLETED: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x73);

impl Tokens {
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            canvas: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            border: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            border_strong: Color32::from_rgb(0xD9, 0xD9, 0xD9),
            text: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            // 4.7:1 on fields and hovered rows (#F5F5F5), 4.6:1 on selected rows: WCAG AA.
            text_secondary: Color32::from_rgb(0x6E, 0x6E, 0x6E),
            text_tertiary: Color32::from_rgb(0xB3, 0xB3, 0xB3),
            icon: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            icon_secondary: Color32::from_rgb(0x75, 0x75, 0x75),
            hover: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            pressed: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            field: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            field_hover_border: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            accent: ACCENT,
            accent_hover: Color32::from_rgb(0x00, 0x7B, 0xE5),
            // 5.2:1 on white, 4.6:1 on selected rows: WCAG AA for text.
            accent_text: Color32::from_rgb(0x00, 0x6B, 0xD1),
            selected_row: Color32::from_rgb(0xE5, 0xF4, 0xFF),
            selected_row_child: Color32::from_rgb(0xF2, 0xF9, 0xFF),
            component: COMPONENT,
            component_row: Color32::from_rgb(0xF1, 0xE9, 0xFF),
            icon_layer: ICON,
            icon_layer_row: Color32::from_rgb(0xFD, 0xF0, 0xDD),
            danger: Color32::from_rgb(0xF2, 0x48, 0x22),
            success: Color32::from_rgb(0x14, 0xAE, 0x5C),
            warning: Color32::from_rgb(0xFF, 0xCD, 0x29),
            guide: Color32::from_rgb(0xF2, 0x48, 0x22),
            tab_bar: Color32::from_rgb(0xE8, 0xE8, 0xE8),
            tab_active: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            shadow: Color32::from_black_alpha(40),
            menu: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            menu_text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            menu_hover: ACCENT,
            menu_dim: Color32::from_rgb(0x9A, 0x9A, 0x9A),
            focus: Color32::from_rgb(0x00, 0x6B, 0xD1),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            panel: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            canvas: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            border: Color32::from_rgb(0x44, 0x44, 0x44),
            border_strong: Color32::from_rgb(0x55, 0x55, 0x55),
            text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            text_secondary: Color32::from_rgb(0xB3, 0xB3, 0xB3),
            text_tertiary: Color32::from_rgb(0x75, 0x75, 0x75),
            icon: Color32::from_rgb(0xF5, 0xF5, 0xF5),
            icon_secondary: Color32::from_rgb(0xB3, 0xB3, 0xB3),
            hover: Color32::from_rgb(0x38, 0x38, 0x38),
            pressed: Color32::from_rgb(0x44, 0x44, 0x44),
            field: Color32::from_rgb(0x38, 0x38, 0x38),
            field_hover_border: Color32::from_rgb(0x4D, 0x4D, 0x4D),
            accent: ACCENT,
            accent_hover: Color32::from_rgb(0x3D, 0xAD, 0xFF),
            accent_text: Color32::from_rgb(0x7C, 0xC4, 0xF8),
            selected_row: Color32::from_rgb(0x2A, 0x45, 0x5C),
            selected_row_child: Color32::from_rgb(0x2D, 0x37, 0x41),
            component: Color32::from_rgb(0xD2, 0x7C, 0xFF),
            component_row: Color32::from_rgb(0x3E, 0x30, 0x52),
            // 6.1:1 on the panel: the name has to read as a label, not as a warning.
            icon_layer: Color32::from_rgb(0xFF, 0xB3, 0x4D),
            icon_layer_row: Color32::from_rgb(0x4A, 0x38, 0x1C),
            danger: Color32::from_rgb(0xF2, 0x48, 0x22),
            success: Color32::from_rgb(0x14, 0xAE, 0x5C),
            warning: Color32::from_rgb(0xFF, 0xCD, 0x29),
            guide: Color32::from_rgb(0xF2, 0x48, 0x22),
            tab_bar: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            tab_active: Color32::from_rgb(0x2C, 0x2C, 0x2C),
            shadow: Color32::from_black_alpha(90),
            menu: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            menu_text: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            menu_hover: ACCENT,
            menu_dim: Color32::from_rgb(0x9A, 0x9A, 0x9A),
            focus: ACCENT,
        }
    }

    pub fn get(ctx: &Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(tokens_id())).unwrap_or_else(Tokens::light)
    }
}

fn tokens_id() -> egui::Id {
    egui::Id::new("figcraft_tokens")
}

/// egui's default for how long hover fades and openings animate.
const ANIMATION_TIME: f32 = 0.2;

/// Install tokens and egui visuals.
pub fn apply(ctx: &Context, t: Tokens) {
    apply_with(ctx, t, false);
}

/// [`apply`], with egui's own animations (hover fades, openings, smooth scrolling to a target) off
/// when the system or the preferences ask for reduced motion. Prototype transitions still play.
pub fn apply_with(ctx: &Context, t: Tokens, reduce_motion: bool) {
    ctx.data_mut(|d| d.insert_temp(tokens_id(), t));
    ctx.set_visuals(visuals(t));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
        s.spacing.interact_size = egui::vec2(24.0, 24.0);
        s.spacing.menu_margin = egui::Margin::same(8);
        s.spacing.window_margin = egui::Margin::same(12);
        s.text_styles = [
            (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(13.0, FontFamily::Name("semibold".into()))),
            (TextStyle::Monospace, FontId::new(11.0, FontFamily::Monospace)),
        ]
        .into();
        s.interaction.tooltip_delay = 0.4;
        s.animation_time = if reduce_motion { 0.0 } else { ANIMATION_TIME };
        s.scroll_animation = if reduce_motion { egui::style::ScrollAnimation::none() } else { egui::style::ScrollAnimation::default() };
    });
}

/// egui visuals for tokens `t`.
fn visuals(t: Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.hover;
    v.override_text_color = Some(t.text);
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = egui::CornerRadius::same(13);
    v.menu_corner_radius = egui::CornerRadius::same(13);
    v.selection.bg_fill = t.accent.gamma_multiply(0.3);
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent_text;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
        w.corner_radius = egui::CornerRadius::same(5);
        w.fg_stroke = Stroke::new(1.0, t.text);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.inactive.bg_fill = t.field;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.field_hover_border);
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: t.shadow };
    v.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: t.shadow };
    v.text_cursor.stroke = Stroke::new(1.0, t.accent);
    v
}

/// Draw `f` with the dark tokens and visuals: controls on the dark popovers (the style browser's
/// style editor), whichever theme the app shows.
pub fn on_dark<R>(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let saved = Tokens::get(&ctx);
    let dark = Tokens::dark();
    ctx.data_mut(|d| d.insert_temp(tokens_id(), dark));
    let r = ui
        .scope(|ui| {
            *ui.visuals_mut() = visuals(dark);
            f(ui)
        })
        .inner;
    ctx.data_mut(|d| d.insert_temp(tokens_id(), saved));
    r
}

/// UI fonts: Inter (from the craft-fonts build input) when present, else egui's defaults; with
/// `system`, also system faces for scripts those lack (Hebrew, joined Arabic) so names in them show,
/// and faces for the CJK scripts in `cjk` ([`crate::cjk`]).
pub fn install_fonts(ctx: &Context, system: bool, cjk: u8) {
    let mut fonts = FontDefinitions::default();
    let find = |family: &str, style: &str| figcraft_text::CRAFT_FONTS.iter().find(|f| f.family == family && f.style == style).map(|f| f.bytes);
    let regular = find("Inter", "Regular");
    let semibold = find("Inter", "SemiBold").or_else(|| find("Inter", "Bold"));
    if let Some(r) = regular {
        fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(r)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "inter".into());
        }
    }
    let sb_name = if let Some(sb) = semibold {
        fonts.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(sb)));
        "inter-semibold"
    } else {
        "inter"
    };
    let mut semi: Vec<String> = Vec::new();
    if fonts.font_data.contains_key(sb_name) {
        semi.push(sb_name.into());
    }
    semi.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name("semibold".into()), semi);
    // CJK and other scripts from craft-fonts as fallbacks (Noto Sans CJK only with FIGCRAFT_EMBED_CJK).
    const BUILT_IN: [&str; 4] = ["Inter", "BIZ UDPGothic", "Noto Sans CJK SC", "Noto Sans Arabic"];
    let lang = crate::i18n::current().map_or("en", |l| l.code);
    // A Chinese UI draws Han in a Chinese face: the built-in Japanese one goes after the system faces.
    let late = |family: &str| family == "BIZ UDPGothic" && crate::cjk::language(lang) == crate::cjk::HAN;
    let craft = |last: bool| {
        figcraft_text::CRAFT_FONTS
            .iter()
            .filter(move |f| f.family != "Inter" && f.style == "Regular" && BUILT_IN.contains(&f.family) && late(f.family) == last)
            .map(|f| (format!("craft-{}", f.family), FontData::from_static(f.bytes)))
    };
    let mut extra: Vec<(String, FontData)> = craft(false).collect();
    if system {
        // What the built-in faces above already draw.
        let embedded = |c: char| {
            BUILT_IN
                .into_iter()
                .filter(|f| !late(f) && figcraft_text::CRAFT_FONTS.iter().any(|cf| cf.family == *f))
                .filter_map(|f| figcraft_text::find_face(f, "Regular"))
                .any(|x| x.has_char(c))
        };
        extra.extend(crate::bidi::script_fallbacks(&embedded));
        extra.extend(crate::cjk::faces(cjk, lang, &embedded).iter().map(|f| (format!("system-{}-{}", f.family, f.style), font_data(f))));
    }
    extra.extend(craft(true));
    for (key, data) in extra {
        fonts.font_data.insert(key.clone(), Arc::new(data));
        for fam in [FontFamily::Proportional, FontFamily::Name("semibold".into())] {
            if let Some(list) = fonts.families.get_mut(&fam) {
                list.push(key.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

/// egui font data over a face's bytes, without copying them (CJK faces are 10–20 MB). Font
/// database faces are never dropped, so a system face's shared bytes are kept for good: leaking
/// one more reference to them costs a pointer and gives egui the `'static` slice it needs.
pub fn font_data(face: &figcraft_text::Face) -> FontData {
    let bytes: &'static [u8] = match &face.bytes {
        figcraft_text::fontdb::FontBytes::Static(b) => b,
        figcraft_text::fontdb::FontBytes::Shared(v) => Box::leak(Box::new(v.clone())).as_slice(),
    };
    let mut data = FontData::from_static(bytes);
    data.index = face.index;
    data
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn c32(c: figcraft_color::Color) -> Color32 {
    let [r, g, b, a] = c.to_rgba8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn screen_conversion_id() -> egui::Id {
    egui::Id::new("figcraft_screen_conversion")
}

/// Remember the colour profile of the document on screen (once per frame): document colours drawn
/// by the UI ([`doc_c32`]) go from it to the screen's sRGB.
pub fn set_doc_profile(ctx: &Context, profile: figcraft_color::ColorProfile) {
    let conv = profile.conversion_to(figcraft_color::ColorProfile::Srgb);
    ctx.data_mut(|d| d.insert_temp(screen_conversion_id(), conv));
}

/// A document colour (a paint, swatch, background) for the screen: converted from the document's
/// colour profile to sRGB. UI chrome colours use [`c32`].
pub fn doc_c32(ctx: &Context, c: figcraft_color::Color) -> Color32 {
    let conv = ctx.data(|d| d.get_temp::<figcraft_color::ProfileConversion>(screen_conversion_id()));
    c32(conv.map_or(c, |k| k.apply(c)))
}
