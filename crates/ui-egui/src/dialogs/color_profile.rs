use super::{ColorProfile, Context, FigApp, RichText, Tokens, json, regular, tr, trf, window};

/// File ▸ Color profile: pick sRGB or Display P3, then keep the colours' appearance (convert the
/// values) or keep the values (reinterpret them in the new space).
pub(super) fn color_profile_dialog(app: &mut FigApp, ctx: &Context) {
    let Some(mut chosen) = app.ui.color_profile else { return };
    let Ok(current) = app.session.doc().map(|d| d.color_profile) else {
        app.ui.color_profile = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut apply = None;
    window(tr("Color profile"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(RichText::new(tr("Color values in this file are in this color space.")).color(t.text_secondary).font(regular(11.0)));
        ui.add_space(8.0);
        for p in ColorProfile::ALL {
            let label = if p == current { trf("{profile} (current)", &[("profile", &p.label())]) } else { p.label().to_string() };
            if crate::widgets::radio(ui, chosen == p, &label) {
                chosen = p;
            }
        }
        ui.add_space(8.0);
        if chosen == current {
            ui.label(RichText::new(tr("Choose another profile to change it.")).color(t.text_secondary).font(regular(11.0)));
            return;
        }
        ui.label(
            RichText::new(trf(
                "Keep appearance converts every color value to {profile} so colors look the same. Keep values leaves the numbers as they are, so colors shift.",
                &[("profile", &chosen.label())],
            ))
            .font(regular(11.0)),
        );
        if !chosen.is_srgb() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("The canvas previews colors in sRGB; exported PNG, JPG and WebP files are tagged Display P3."))
                    .color(t.text_secondary)
                    .font(regular(11.0)),
            );
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if crate::widgets::button(ui, tr("Keep appearance"), true, true).clicked() {
                apply = Some(true);
            }
            if crate::widgets::button(ui, tr("Keep values"), false, true).clicked() {
                apply = Some(false);
            }
        });
    });
    app.ui.color_profile = open.then_some(chosen);
    if let Some(convert) = apply {
        app.run("file.colorProfile", json!({"profile": chosen.id(), "convert": convert}));
        app.ui.color_profile = None;
    }
}
