//! Design panel section for connectors: the line type, and letting go of the attached layers.
//! (Start and end caps are in the Stroke section, as for lines.)

use egui::Ui;
use figcraft_doc::{ConnectorLineType, Document, NodeId, NodeKind};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::right::{common, menu_item, menu_popup, row};
use crate::widgets;

fn is_connector(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Connector { .. }))
}

pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    if sel.is_empty() || !sel.iter().all(|i| is_connector(doc, *i)) {
        return;
    }
    let ids = sel.to_vec();
    let col = widgets::col_width(ui.available_width());
    let line = common(doc, sel, |n| match &n.kind {
        NodeKind::Connector { line_type, .. } => Some(*line_type),
        _ => None,
    })
    .flatten();
    let attached = sel.iter().any(|i| {
        doc.node(*i).is_some_and(|n| matches!(&n.kind, NodeKind::Connector { start, end, .. } if start.node.is_some() || end.node.is_some()))
    });
    widgets::sublabels(ui, tr("Line type"), None);
    row(ui, 32.0, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let resp = widgets::dropdown(ui, line.map(|l| tr(l.label())).unwrap_or(tr("Mixed")), col, None);
        menu_popup(&resp).show(|ui| {
            for l in ConnectorLineType::ALL {
                if menu_item(ui, tr(l.label()), "") {
                    app.set_props(&ids, json!({"connectorLineType": format!("{l:?}").to_ascii_lowercase()}));
                    ui.close();
                }
            }
        });
        let detach = widgets::button(ui, tr("Detach"), false, attached).on_hover_text(tr("Let go of the attached layers"));
        if detach.clicked() {
            app.run("connector.detach", json!({"ids": ids}));
        }
    });
    ui.add_space(4.0);
    widgets::separator(ui);
}
