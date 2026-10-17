//! The Frame tool's panel: frame presets by category. Clicking one adds a frame of that size next
//! to the page's frames and returns to the Move tool.

use egui::{Align2, CornerRadius, Id, Sense, Ui, pos2, vec2};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, PAD};

pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Frame"), &[], false);
    custom_presets(app, ui);
    let w = ui.available_width();
    let mut cats: Vec<&str> = Vec::new();
    for (c, ..) in figcraft_engine::cmd::create::FRAME_PRESETS {
        if !cats.contains(c) {
            cats.push(c);
        }
    }
    let mut chosen = None;
    for cat in cats {
        let open_id = Id::new(("preset_cat", cat));
        // The first category starts open.
        let open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(cat == "Phone");
        let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(8.0, 2.0)), CornerRadius::same(5), t.hover);
        }
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(pos2(r.min.x + PAD + 4.0, r.center().y), vec2(10.0, 10.0)),
            if open { Icon::Chevron } else { Icon::ChevronRight },
            t.icon_secondary,
        );
        ui.painter().text(pos2(r.min.x + PAD + 16.0, r.center().y), Align2::LEFT_CENTER, tr(cat), semibold(11.0), t.text);
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(open_id, !open));
        }
        if !open {
            continue;
        }
        for (_, name, pw, ph) in figcraft_engine::cmd::create::FRAME_PRESETS.iter().filter(|(c, ..)| *c == cat) {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.hover);
            }
            ui.painter().text(pos2(r.min.x + PAD + 16.0, r.center().y), Align2::LEFT_CENTER, *name, regular(11.0), t.text);
            ui.painter().text(
                pos2(r.max.x - PAD, r.center().y),
                Align2::RIGHT_CENTER,
                format!("{}×{}", crate::canvas::fmt(*pw), crate::canvas::fmt(*ph)),
                regular(11.0),
                t.text_secondary,
            );
            if resp.clicked() {
                chosen = Some(*name);
            }
        }
    }
    if let Some(name) = chosen {
        create_frame(app, name);
    }
}

fn create_frame(app: &mut FigApp, name: &str) {
    if app.run("frame.create", json!({"preset": name})).is_some() {
        app.run("tool.set", json!({"tool": "move"}));
        app.run("view.zoomSelection", json!({}));
    }
}

#[derive(Clone)]
struct PresetDraft {
    name: String,
    width: f64,
    height: f64,
}

impl Default for PresetDraft {
    fn default() -> Self {
        Self { name: String::new(), width: 100.0, height: 100.0 }
    }
}

fn custom_presets(app: &mut FigApp, ui: &mut Ui) {
    egui::CollapsingHeader::new(tr("Custom")).id_salt("custom_frame_presets").default_open(true).show(ui, |ui| {
        let id = Id::new("custom_frame_draft");
        let mut draft = ui.data(|d| d.get_temp::<PresetDraft>(id)).unwrap_or_default();
        let width = ui.available_width();
        if let Some(name) = widgets::text_field(ui, id.with("name"), &draft.name, width, tr("Name")) {
            draft.name = name;
        }
        ui.horizontal(|ui| {
            let field_w = (width - ui.spacing().item_spacing.x) / 2.0;
            for (key, value) in [("W", &mut draft.width), ("H", &mut draft.height)] {
                if let Some(edit) = widgets::number_field(ui, id.with(key), widgets::Lead::Text(key), Some(*value), "", field_w, 1.0) {
                    match edit {
                        widgets::FieldEdit::Set(v) => *value = v,
                        widgets::FieldEdit::Scrub(v, _) => *value = v,
                    }
                }
            }
        });
        let valid = !draft.name.trim().is_empty()
            && draft.name.trim().chars().count() <= 64
            && draft.width.is_finite()
            && draft.height.is_finite()
            && draft.width > 0.0
            && draft.height > 0.0
            && draft.width <= figcraft_engine::cmd::create::MAX_PRESET_SIZE
            && draft.height <= figcraft_engine::cmd::create::MAX_PRESET_SIZE;
        if widgets::button(ui, tr("Save frame preset"), false, valid).clicked() {
            app.run("frame.presetSave", json!({"name": draft.name, "width": draft.width, "height": draft.height}));
        }
        ui.data_mut(|d| d.insert_temp(id, draft));
        let mut chosen = None;
        let mut delete = None;
        for preset in &app.session.prefs.frame_presets {
            ui.horizontal(|ui| {
                let label = format!("{}  {}×{}", preset.name, crate::canvas::fmt(preset.width), crate::canvas::fmt(preset.height));
                if ui.add_sized([ui.available_width() - 28.0, 28.0], egui::Button::new(label).truncate().frame(false)).clicked() {
                    chosen = Some(preset.name.clone());
                }
                if widgets::icon_button(ui, Icon::Close, tr("Delete preset")) {
                    delete = Some(preset.name.clone());
                }
            });
        }
        if let Some(name) = delete {
            app.run("frame.presetDelete", json!({"name": name}));
        }
        if let Some(name) = chosen {
            create_frame(app, &name);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers_drag_tests::Ui as TestUi;
    use figcraft_engine::{Session, ToolId};

    fn click_named(u: &mut TestUi, label: &str) {
        // Find the actual control through the accessibility tree instead of fixing panel pixels.
        u.ctx.disable_accesskit();
        u.ctx.enable_accesskit();
        let out = u.frame_output(vec![]);
        let tree = out.platform_output.accesskit_update.unwrap();
        let bounds = tree.nodes.iter().find_map(|(_, n)| (n.label() == Some(label)).then(|| n.bounds()).flatten()).unwrap();
        let p = pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
        u.button(p, true);
        u.button(p, false);
    }

    #[test]
    fn custom_frame_panel_saves_creates_and_deletes_without_removing_frames() {
        let mut u = TestUi::new(Session::new());
        u.app.session.execute("tool.set", &json!({"tool": "frame"})).unwrap();
        u.ctx.data_mut(|d| d.insert_temp(Id::new("custom_frame_draft"), PresetDraft { name: "Banner".into(), width: 640.0, height: 120.0 }));
        for _ in 0..3 {
            u.frame(vec![]);
        }
        click_named(&mut u, tr("Save frame preset"));
        assert_eq!(u.app.session.prefs.frame_presets.len(), 1);
        click_named(&mut u, "Banner  640×120");
        assert_eq!(u.app.session.tool.tool, ToolId::Move);
        let id = u.app.session.selection()[0];
        let n = u.app.session.doc().unwrap().node(id).unwrap();
        assert_eq!((n.width, n.height), (640.0, 120.0));
        u.app.session.undo().unwrap();
        assert!(!u.app.session.doc().unwrap().contains(id));
        u.app.session.redo().unwrap();
        u.app.session.execute("tool.set", &json!({"tool": "frame"})).unwrap();
        for _ in 0..3 {
            u.frame(vec![]);
        }
        click_named(&mut u, tr("Delete preset"));
        assert!(u.app.session.prefs.frame_presets.is_empty());
        assert!(u.app.session.doc().unwrap().contains(id));
    }
}
