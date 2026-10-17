//! Selection colors lives below Effects. A picker edits frozen paint slots, so color groups
//! merging while scrubbing cannot accidentally recolor other objects.

use std::sync::Arc;

use egui::{Id, Ui};
use figcraft_color::Color;
use figcraft_doc::{Document, NodeId, StyleKind};
use figcraft_engine::selection_colors::{PaintTarget, SelectionColor, collect};
use serde_json::json;

use crate::a11y::Tip;
use crate::colorpicker::PickerTarget;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{FigApp, widgets};

#[derive(Clone)]
struct Cache {
    document: u64,
    doc: Arc<Document>,
    ids: Vec<NodeId>,
    colors: Arc<Vec<SelectionColor>>,
}

fn colors(ui: &Ui, document: u64, doc: &Arc<Document>, ids: &[NodeId], interacting: bool) -> Arc<Vec<SelectionColor>> {
    let key = Id::new("selection_colors_cache");
    if let Some(cache) = ui.data(|d| d.get_temp::<Cache>(key))
        && cache.document == document
        && cache.ids == ids
        && (interacting || Arc::ptr_eq(&cache.doc, doc))
    {
        return cache.colors;
    }
    // A drag replaces the document snapshot every frame. Keep its existing rows until the
    // gesture ends; a cold cache or a changed document/selection waits too. Engine commands
    // still discover and change the complete selection, independently of this panel cache.
    if interacting {
        return Arc::new(Vec::new());
    }
    let colors = Arc::new(collect(doc, ids));
    ui.data_mut(|d| d.insert_temp(key, Cache { document, doc: doc.clone(), ids: ids.to_vec(), colors: colors.clone() }));
    colors
}

pub(crate) fn open_picker(app: &mut FigApp, targets: Vec<PaintTarget>) {
    let Ok(ds) = app.session.ds() else { return };
    app.ui.picker = Some(PickerTarget::SelectionColor { targets, selection: ds.selection.clone(), document: ds.uid });
}

pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Arc<Document>, ids: &[NodeId]) {
    let Ok(ds) = app.session.ds() else { return };
    let interacting = ds.interaction.is_some();
    let colors = colors(ui, ds.uid, doc, ids, interacting);
    if colors.len() < 2 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Selection colors"), &[], false);
    // Bound the panel's height; a document with many distinct colors stays scrollable.
    ui.add_enabled_ui(!interacting, |ui| {
        egui::ScrollArea::vertical().id_salt("selection_colors_rows").max_height(240.0).show_rows(ui, 32.0, colors.len(), |ui, range| {
            for group in &colors[range] {
                ui.push_id((&group.targets, "selection_color"), |ui| {
                    crate::right::row(ui, 32.0, |ui| {
                        let swatch = widgets::swatch(ui, &group.paint, 24.0).tip(tr("Edit colour"));
                        if swatch.clicked() {
                            open_picker(app, group.targets.clone());
                        }
                        let width = (ui.available_width() - 116.0).max(40.0);
                        let linked_name = group
                            .style
                            .as_ref()
                            .and_then(|id| doc.style(id).map(|s| s.name.as_str()))
                            .or_else(|| group.variable.as_ref().and_then(|id| doc.variables.get(id).map(|v| v.name.as_str())));
                        if let Some(name) = linked_name {
                            if widgets::dropdown(ui, name, width, None).clicked() {
                                open_picker(app, group.targets.clone());
                            }
                        } else if let Some(color) = group.paint.solid_color() {
                            if let Some(hex) = widgets::text_field_named(ui, ui.id().with("hex"), &color.to_hex(), width, tr("Color")) {
                                if let Some(color) = Color::from_hex(&hex) {
                                    let mut paint = group.paint.clone();
                                    paint.kind = figcraft_color::PaintKind::Solid { color: color.with_alpha(1.0) };
                                    if color.a < 1.0 {
                                        paint.opacity = color.a;
                                    }
                                    app.run("selection.setPaint", json!({"targets": group.targets, "paint": paint}));
                                } else {
                                    app.toast(tr("Invalid color"));
                                }
                            }
                        } else {
                            let response = widgets::dropdown(ui, &widgets::paint_name(&group.paint), width, None);
                            if response.clicked() {
                                open_picker(app, group.targets.clone());
                            }
                        }
                        let alpha = format!("{}%", figcraft_doc::trim_num(f64::from(group.paint.opacity) * 100.0));
                        if let Some(value) = widgets::text_field_named(ui, ui.id().with("opacity"), &alpha, 44.0, tr("Opacity"))
                            && let Ok(value) = value.trim().trim_end_matches('%').parse::<f32>()
                            && value.is_finite()
                        {
                            let mut paint = group.paint.clone();
                            paint.opacity = (value / 100.0).clamp(0.0, 1.0);
                            app.run("selection.setPaint", json!({"targets": group.targets, "paint": paint}));
                        }
                        let styles = icons::button(ui, Icon::Styles, 24.0, false, tr("Color styles"));
                        crate::right::menu_popup(&styles).show(|ui| {
                            let mut any = false;
                            for style in &doc.styles {
                                if !matches!(style.kind, StyleKind::Paint { .. }) {
                                    continue;
                                }
                                any = true;
                                if crate::right::menu_item(ui, &style.name, "") {
                                    app.run("selection.applyColorStyle", json!({"targets": group.targets, "style": style.id}));
                                    app.ui.picker = None;
                                    ui.close();
                                }
                            }
                            if !any {
                                widgets::label(ui, tr("No color styles"), t.text_secondary);
                            }
                        });
                        if icons::button(ui, Icon::Move, 24.0, false, tr("Select layers with this color")).clicked() {
                            app.run("selection.selectColor", json!({"targets": group.targets}));
                        }
                    });
                });
            }
        });
    });
    ui.add_space(8.0);
    widgets::separator(ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoServices;
    use egui::{Event, Pos2, RawInput, Rect, Shape, vec2};
    use figcraft_engine::Session;

    struct Panel {
        ctx: egui::Context,
        app: FigApp,
        time: f64,
    }

    impl Panel {
        fn new() -> (Self, NodeId, NodeId) {
            let mut s = Session::new();
            let mut ids = Vec::new();
            for color in ["#ff0000", "#0000ff"] {
                let value = s.execute("shape.rectangle", &json!({"width":40,"height":40,"props":{"fill":color}})).unwrap();
                ids.push(NodeId(value["id"].as_u64().unwrap()));
            }
            s.execute("select.set", &json!({"ids":ids})).unwrap();
            let mut p = Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 };
            crate::theme::install_fonts(&p.ctx, false, 0);
            crate::theme::apply(&p.ctx, crate::theme::Tokens::light());
            for _ in 0..3 {
                p.frame(vec![]);
            }
            (p, ids[0], ids[1])
        }
        fn frame(&mut self, events: Vec<Event>) -> Vec<(String, Pos2)> {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| {
                let ds = self.app.session.ds().unwrap();
                let doc = ds.doc.clone();
                let ids = ds.selection.clone();
                section(&mut self.app, ui, &doc, &ids);
            });
            output.textures_delta.clear();
            fn texts(shape: &Shape, out: &mut Vec<(String, Pos2)>) {
                match shape {
                    Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.size() / 2.0)),
                    Shape::Vec(v) => {
                        for s in v {
                            texts(s, out)
                        }
                    }
                    _ => {}
                }
            }
            let mut text = Vec::new();
            for s in &output.shapes {
                texts(&s.shape, &mut text);
            }
            text
        }
        fn click(&mut self, pos: Pos2) {
            for pressed in [true, false] {
                self.frame(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                ]);
            }
        }
    }

    #[test]
    fn selection_colors_panel_opens_the_existing_picker_and_hides_for_one_color() {
        let (mut p, a, b) = Panel::new();
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "Selection colors"));
        let red = texts.iter().find(|(s, _)| s == "FF0000").unwrap().1;
        p.click(Pos2::new(28.0, red.y));
        let Some(PickerTarget::SelectionColor { targets, .. }) = &p.app.ui.picker else { panic!("the swatch must open its picker") };
        assert_eq!(targets.iter().map(|t| t.id).collect::<Vec<_>>(), [a]);
        p.app.run("fill.set", json!({"id":b,"color":"#ff0000"}));
        assert!(!p.frame(vec![]).iter().any(|(s, _)| s == "Selection colors"));
    }

    #[test]
    fn panel_keeps_cached_colors_during_interaction_and_refreshes_after_commit() {
        let (mut p, a, _) = Panel::new();
        let key = Id::new("selection_colors_cache");
        let before = p.ctx.data(|d| d.get_temp::<Cache>(key)).unwrap();
        p.app.session.begin_interaction("Color").unwrap();
        for hex in ["#00ff00", "#ffff00", "#00ffff"] {
            p.app.run("fill.set", json!({"id":a,"color":hex}));
            p.frame(vec![]);
            let during = p.ctx.data(|d| d.get_temp::<Cache>(key)).unwrap();
            assert!(Arc::ptr_eq(&before.colors, &during.colors), "a live edit must not rediscover paints");
        }
        p.app.session.commit_interaction().unwrap();
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "00FFFF"));
        let after = p.ctx.data(|d| d.get_temp::<Cache>(key)).unwrap();
        assert!(!Arc::ptr_eq(&before.colors, &after.colors));
    }

    #[test]
    fn panel_defers_discovery_without_a_matching_cache_during_interaction() {
        let (mut p, a, _) = Panel::new();
        let key = Id::new("selection_colors_cache");
        p.ctx.data_mut(|d| d.remove::<Cache>(key));
        p.app.session.begin_interaction("Color").unwrap();
        p.app.run("fill.set", json!({"id":a,"color":"#00ff00"}));
        assert!(!p.frame(vec![]).iter().any(|(s, _)| s == "Selection colors"));
        assert!(p.ctx.data(|d| d.get_temp::<Cache>(key)).is_none());
        p.app.session.cancel_interaction().unwrap();
        assert!(p.frame(vec![]).iter().any(|(s, _)| s == "FF0000"));
    }

    #[test]
    fn select_color_button_selects_only_the_matching_layers() {
        let (mut p, a, _) = Panel::new();
        let red = p.frame(vec![]).into_iter().find(|(s, _)| s == "FF0000").unwrap().1;
        p.click(Pos2::new(252.0, red.y));
        assert_eq!(p.app.session.selection(), [a]);
        assert!(p.app.ui.last_error.is_none());
    }

    #[test]
    fn panel_displays_the_linked_style_name_and_the_other_color() {
        let (mut p, a, b) = Panel::new();
        p.app.run("style.create", json!({"type":"paint","name":"Brand","value":"#ff0000"}));
        p.app.run("style.apply", json!({"id":a,"style":"Brand","target":"fill"}));
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "Brand"));
        assert!(texts.iter().any(|(s, _)| s == "0000FF"));
        let colors = collect(p.app.session.doc().unwrap(), &[a, b]);
        assert_eq!(colors.len(), 2);
    }
}
