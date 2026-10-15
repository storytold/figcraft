//! Design panel section for connectors: the line type, letting go of the attached layers, and
//! one-click thin / thick and solid / dashed lines. (Colour, exact weight, dashes and the start and
//! end caps are in the Stroke section, as for lines.)

use egui::Ui;
use figcraft_doc::connector::{THICK, THIN};
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
    // Quick line looks: thin or thick, solid or dashed (mixed or other weights show neither).
    let weight = common(doc, sel, |n| n.stroke.weight);
    let thick = match weight {
        Some(w) if (w - THIN).abs() < 1e-6 => Some(0),
        Some(w) if (w - THICK).abs() < 1e-6 => Some(1),
        _ => None,
    };
    let dashed = common(doc, sel, |n| !n.stroke.dashes.is_empty()).map(usize::from);
    let quick = |ui: &mut Ui, label: &str, options: [&str; 2], active: Option<usize>| -> Option<usize> {
        widgets::sublabels(ui, label, None);
        let mut out = None;
        row(ui, 32.0, |ui| {
            ui.allocate_ui(egui::vec2(col * 2.0 + 8.0, 32.0), |ui| {
                ui.set_width(col * 2.0 + 8.0);
                out = widgets::segmented_text(ui, &options, active.unwrap_or(usize::MAX));
            });
        });
        out
    };
    if let Some(i) = quick(ui, tr("Weight"), [tr("Thin"), tr("Thick")], thick) {
        app.set_props(&ids, json!({"strokeWeight": if i == 0 { THIN } else { THICK }}));
    }
    if let Some(i) = quick(ui, tr("Line style"), [tr("Solid"), tr("Dashed")], dashed) {
        app.set_props(&ids, json!({"dashes": if i == 0 { vec![] } else { vec![8.0, 6.0] }}));
    }
    ui.add_space(4.0);
    widgets::separator(ui);
}
