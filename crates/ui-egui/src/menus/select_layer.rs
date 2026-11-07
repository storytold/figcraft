use super::tree::{SubmenuOptions, indented_item, separator, submenu_at};
use super::{FigApp, Id, Tokens, Ui, json, tr};

/// The context menu's Select layer submenu (its key in the context menu's tree).
pub(super) const SELECT_LAYER: &str = "Select layer";

/// Stable egui id for the context menu's pending Select-layer open request.
pub(super) fn select_layer_ask_id() -> Id {
    Id::unique("ctx_select_layer_ask")
}

/// Open (or close) the context menu's Select layer submenu (applied when the menu next draws).
pub fn open_select_layer(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(select_layer_ask_id(), open));
}

/// The context menu's "Select layer" submenu (right-click on the canvas): every layer under the
/// click, nested ones indented, the selected ones checked. Returns whether one was picked.
pub(super) fn select_layer_menu(app: &mut FigApp, ui: &mut Ui, pos: egui::Pos2, t: &Tokens) -> bool {
    if !app.canvas.rect.contains(pos) {
        return false;
    }
    let p = app.to_page(pos);
    let Ok(ds) = app.session.ds() else { return false };
    let tol = 2.0 / ds.view.zoom.max(1e-6);
    let layers: Vec<(figcraft_doc::NodeId, usize, String)> =
        figcraft_doc::geometry::layers_at(&ds.doc, ds.page, p, tol, Some(&figcraft_engine::booleans::result_outline(&ds.doc, &ds.selection)))
            .into_iter()
            .take(40)
            .filter_map(|(id, depth)| {
                ds.doc.node(id).map(|n| (id, depth, if n.name.is_empty() { n.kind.type_name().to_lowercase() } else { n.name.clone() }))
            })
            .collect();
    if layers.is_empty() {
        return false;
    }
    let sel = ds.selection.clone();
    let picked = submenu_at(
        ui,
        SubmenuOptions { area: Id::unique("ctx_select_layer"), key: SELECT_LAYER, label: tr("Select layer"), gap: 4.0, constrain: true, tokens: t },
        |ui| {
            let mut picked = None;
            for (id, depth, name) in &layers {
                if indented_item(ui, name, "", true, sel.contains(id), 12.0 * (*depth).min(8) as f32, t) {
                    picked = Some(*id);
                }
            }
            picked
        },
    );
    separator(ui);
    match picked {
        Some(id) => {
            app.run("select.set", json!({"ids": [id]}));
            true
        }
        None => false,
    }
}
