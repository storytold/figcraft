use super::{Context, FigApp, Tokens, i18n, json, tr, window};

/// Values collected by a menu or Actions command before dispatching it to the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuInput {
    Link(String),
    Offset(f64),
    MoveToPage,
}

/// Menus and Actions cannot pass a value themselves. Gather it here, then run the regular
/// command so the same validation and undo behaviour apply as for programmatic callers.
pub(super) fn menu_input_dialog(app: &mut FigApp, ctx: &Context) {
    let Some(mut input) = app.ui.menu_input.take() else { return };
    let t = Tokens::get(ctx);
    let title = match &input {
        MenuInput::Link(_) => i18n::command("text.link", "Create link"),
        MenuInput::Offset(_) => i18n::command("vector.offset", "Offset path"),
        MenuInput::MoveToPage => i18n::command("object.moveToPage", "Move to page"),
    };
    let mut open = true;
    let mut action: Option<(&str, serde_json::Value)> = None;
    window(&title, ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(320.0);
        match &mut input {
            MenuInput::Link(url) => {
                ui.horizontal(|ui| {
                    let field = crate::widgets::text_input(ui, egui::Id::unique("menu_input_link"), url, 250.0, "https://");
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let ok = !url.trim().is_empty();
                    if (crate::widgets::button(ui, tr("Done"), true, ok).clicked() || enter) && ok {
                        action = Some(("text.link", json!({"url": url.trim()})));
                    }
                });
            }
            MenuInput::Offset(offset) => {
                ui.horizontal(|ui| {
                    let id = egui::Id::unique("menu_input_offset");
                    if let Some(crate::widgets::FieldEdit::Set(v) | crate::widgets::FieldEdit::Scrub(v, _)) =
                        crate::widgets::number_field(ui, id, crate::widgets::Lead::None, Some(*offset), "", 120.0, 1.0)
                    {
                        *offset = v.clamp(-100_000.0, 100_000.0);
                    }
                    if crate::widgets::button(ui, tr("Offset"), true, true).clicked() {
                        action = Some(("vector.offset", json!({"offset": *offset})));
                    }
                });
            }
            MenuInput::MoveToPage => {
                let mut choices = 0;
                if let Ok(ds) = app.session.ds() {
                    for &page in &ds.doc.pages {
                        if page == ds.page || ds.doc.is_divider(page) {
                            continue;
                        }
                        if let Some(node) = ds.doc.node(page)
                            && crate::widgets::button(ui, &node.name, false, true).clicked()
                        {
                            action = Some(("object.moveToPage", json!({"page": page})));
                        }
                        choices += 1;
                    }
                }
                if choices == 0 {
                    crate::widgets::label(ui, tr("No other pages"), t.text_secondary);
                }
            }
        }
    });
    if let Some((id, params)) = action {
        app.run(id, params);
    } else if open {
        app.ui.menu_input = Some(input);
    }
}
