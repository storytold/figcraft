//! Selection colors lives below Effects. A picker edits frozen paint slots, so color groups
//! merging while scrubbing cannot accidentally recolor other objects.

use std::sync::Arc;

use egui::{Id, Ui};
use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeId, PublishedLibrary, Style, StyleKind, VariableType};
use figcraft_engine::library;
use figcraft_engine::selection_colors::{PaintTarget, SelectionColor, bind_refusal, collect, create_refusal, in_scope};
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

/// Return the cached selection colours when the document and selection match. During an
/// interaction reuse an existing matching cache, or return an empty list without discovering
/// colours; otherwise discover and cache them.
fn colors(ui: &Ui, document: u64, doc: &Arc<Document>, ids: &[NodeId], interacting: bool) -> Arc<Vec<SelectionColor>> {
    let key = Id::unique("selection_colors_cache");
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

pub(crate) fn toggle(app: &mut FigApp, targets: Vec<PaintTarget>, response: Option<&egui::Response>) {
    let Ok(ds) = app.session.ds() else { return };
    let target = PickerTarget::SelectionColor { targets, selection: ds.selection.clone(), document: ds.uid };
    if let Some(response) = response {
        app.picker_control(target, response);
    } else {
        app.toggle_picker(target, None);
    }
}

/// Draw the selection-colour groups (at least two): swatch opens the picker, hex and opacity
/// edits, style/variable detach/apply/bind, a style from this file or an enabled library, Create
/// style, and “select layers with this color”; disabled during an interaction.
pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Arc<Document>, ids: &[NodeId]) {
    let Ok(ds) = app.session.ds() else { return };
    let interacting = ds.interaction.is_some();
    let uid = ds.uid;
    let colors = colors(ui, uid, doc, ids, interacting);
    if colors.len() < 2 {
        ui.data_mut(|d| d.remove::<CreateForm>(create_form_id()));
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
                        toggle(app, group.targets.clone(), Some(&swatch));
                        let linked_name = group
                            .style
                            .as_ref()
                            .and_then(|id| doc.style(id).map(|s| s.name.as_str()))
                            .or_else(|| group.variable.as_ref().and_then(|id| doc.variables.get_by_id(id).map(|v| v.name.as_str())));
                        // A linked row has a Detach button beside its Styles button.
                        let buttons = if linked_name.is_some() { 148.0 } else { 116.0 };
                        let width = (ui.available_width() - buttons).max(40.0);
                        if let Some(name) = linked_name {
                            let response = widgets::dropdown(ui, name, width, None);
                            toggle(app, group.targets.clone(), Some(&response));
                        } else if let Some(color) = group.paint.solid_color() {
                            if let Some(hex) = widgets::text_field_named(ui, ui.scope_id().with("hex"), &color.to_hex(), width, tr("Color")) {
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
                            toggle(app, group.targets.clone(), Some(&response));
                        }
                        let alpha = format!("{}%", figcraft_doc::trim_num(f64::from(group.paint.opacity) * 100.0));
                        if let Some(value) = widgets::text_field_named(ui, ui.scope_id().with("opacity"), &alpha, 44.0, tr("Opacity"))
                            && let Ok(value) = value.trim().trim_end_matches('%').parse::<f32>()
                            && value.is_finite()
                        {
                            let mut paint = group.paint.clone();
                            paint.opacity = (value / 100.0).clamp(0.0, 1.0);
                            app.run("selection.setPaint", json!({"targets": group.targets, "paint": paint}));
                        }
                        let styles = icons::button(ui, Icon::Styles, 24.0, false, tr("Color styles"));
                        crate::right::menu_popup(&styles).show(|ui| {
                            style_items(app, ui, doc, group, &t);
                            if linked_name.is_none() {
                                ui.separator();
                                if create_refusal(doc, &group.targets).is_none() && crate::right::menu_item(ui, tr("Create style"), "") {
                                    open_create_form(app, ui, doc, group);
                                    ui.close();
                                }
                                let variables: Vec<_> = doc
                                    .variables
                                    .variables
                                    .iter()
                                    .filter(|v| {
                                        v.kind == VariableType::Color
                                            && bind_refusal(doc, &group.targets, &v.id).is_none()
                                            && in_scope(doc, &group.targets, &v.id)
                                    })
                                    .collect();
                                if !variables.is_empty() {
                                    ui.separator();
                                    widgets::label(ui, tr("Color variables"), t.text_secondary);
                                }
                                for v in variables {
                                    if crate::right::menu_item(ui, &v.name, "") {
                                        app.run("selection.bindColorVariable", json!({"targets": group.targets, "variable": v.id}));
                                        app.ui.picker = None;
                                        ui.close();
                                    }
                                }
                            }
                        });
                        if linked_name.is_some() {
                            let detach = if group.style.is_some() { tr("Detach style") } else { tr("Detach variable") };
                            if icons::button(ui, Icon::Detach, 24.0, false, detach).clicked() {
                                app.run("selection.detachColor", json!({"targets": group.targets}));
                            }
                        }
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
    create_form(app, ui.ctx(), uid);
}

/// The paint styles a row's Styles menu offers: this file's (not library copies), then each enabled
/// library's published ones under its name (a library this session hasn't read offers none). A choice
/// applies the style to the row's targets.
fn style_items(app: &mut FigApp, ui: &mut Ui, doc: &Document, group: &SelectionColor, t: &Tokens) {
    let paint = |s: &&Style| matches!(s.kind, StyleKind::Paint { .. });
    let local: Vec<&Style> = library::local_styles(doc).filter(paint).collect();
    let libs = crate::variable_ui::libraries(app, doc);
    let offered: Vec<(&PublishedLibrary, Vec<&Style>)> = libs
        .iter()
        .map(|l| (l.as_ref(), l.doc.styles.iter().filter(|s| l.manifest.style(&s.id).is_some_and(|x| x.published)).filter(paint).collect::<Vec<_>>()))
        .filter(|(_, list)| !list.is_empty())
        .collect();
    if local.is_empty() && offered.is_empty() {
        widgets::label(ui, tr("No color styles"), t.text_secondary);
    }
    for style in local {
        if crate::right::menu_item(ui, &style.name, "") {
            app.run("selection.applyColorStyle", json!({"targets": group.targets, "style": style.id}));
            app.ui.picker = None;
            ui.close();
        }
    }
    for (lib, list) in offered {
        ui.separator();
        widgets::label(ui, &lib.manifest.name, t.text_secondary);
        for style in list {
            if crate::right::menu_item(ui, &style.name, "") {
                app.run("selection.applyColorStyle", json!({"targets": group.targets, "style": style.id, "library": lib.manifest.id}));
                app.ui.picker = None;
                ui.close();
            }
        }
    }
}

/// The Create style window's state: the document and row it was opened for, and the name typed.
#[derive(Clone)]
struct CreateForm {
    document: u64,
    targets: Vec<PaintTarget>,
    paint: Paint,
    name: String,
}

fn create_form_id() -> Id {
    Id::unique("selection_color_create_style")
}

/// Open the Create style window for `group`, with the next free "Color n" name.
fn open_create_form(app: &mut FigApp, ui: &Ui, doc: &Document, group: &SelectionColor) {
    let Ok(ds) = app.session.ds() else { return };
    let form = CreateForm {
        document: ds.uid,
        targets: group.targets.clone(),
        paint: group.paint.clone(),
        name: crate::style_picker::next_name(doc, "Color"),
    };
    app.ui.picker = None;
    ui.data_mut(|d| d.insert_temp(create_form_id(), form));
}

/// The Create style window: the row's paint, a name, Cancel and Create (Enter creates). Create runs
/// `selection.createColorStyle`; when it fails (a name in use, a row that changed) the window stays
/// open with the name for correction.
fn create_form(app: &mut FigApp, ctx: &egui::Context, document: u64) {
    let Some(mut form) = ctx.data(|d| d.get_temp::<CreateForm>(create_form_id())) else { return };
    if form.document != document {
        ctx.data_mut(|d| d.remove::<CreateForm>(create_form_id()));
        return;
    }
    let (mut open, mut cancel, mut create) = (true, false, false);
    crate::dialogs::window(tr("Create style"), ctx).id(create_form_id().with("window")).open(&mut open).show(ctx, |ui| {
        ui.set_width(240.0);
        ui.horizontal(|ui| {
            widgets::swatch(ui, &form.paint, 24.0);
            ui.label(widgets::paint_name(&form.paint));
        });
        ui.add_space(6.0);
        let name = ui.add(egui::TextEdit::singleline(&mut form.name).hint_text(tr("Name")).desired_width(220.0));
        widgets::select_all_on_focus(ui, &name, &form.name);
        if ui.memory(|m| m.focused().is_none()) {
            name.request_focus();
        }
        ui.add_space(10.0);
        let ready = !form.name.trim().is_empty();
        ui.horizontal(|ui| {
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            let enter = name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            create = (widgets::button(ui, tr("Create"), true, ready).clicked() || enter) && ready;
        });
    });
    let made = create && app.run("selection.createColorStyle", json!({"targets": form.targets, "name": form.name.trim()})).is_some();
    if open && !cancel && !made {
        ctx.data_mut(|d| d.insert_temp(create_form_id(), form));
    } else {
        ctx.data_mut(|d| d.remove::<CreateForm>(create_form_id()));
    }
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

    /// Asserts the panel reuses cached colours through a live interaction and refreshes the cache
    /// after the commit.
    #[test]
    fn panel_keeps_cached_colors_during_interaction_and_refreshes_after_commit() {
        let (mut p, a, _) = Panel::new();
        let key = Id::unique("selection_colors_cache");
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

    /// Asserts no cached colours are discovered during an interaction, and cancelling restores
    /// the panel with the original colours.
    #[test]
    fn panel_defers_discovery_without_a_matching_cache_during_interaction() {
        let (mut p, a, _) = Panel::new();
        let key = Id::unique("selection_colors_cache");
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

    #[test]
    fn a_linked_name_control_tracks_itself_and_toggles_closed() {
        let (mut p, a, _) = Panel::new();
        p.app.run("style.create", json!({"type":"paint","name":"Brand","value":"#ff0000"}));
        p.app.run("style.apply", json!({"id":a,"style":"Brand","target":"fill"}));
        let name = p.frame(vec![]).into_iter().find(|(s, _)| s == "Brand").unwrap().1;
        p.click(name);
        assert!(p.app.ui.picker.is_some());
        let opening = p.app.ui.picker_button.unwrap();
        let opening_id = p.app.ui.picker_button_id.unwrap();
        p.frame(vec![]);
        assert_eq!(p.app.ui.picker_button_id, Some(opening_id), "the swatch must not replace the name control");
        assert_eq!(p.app.ui.picker_button, Some(opening));
        p.click(name);
        assert!(p.app.ui.picker.is_none());
    }

    #[test]
    fn styles_menu_applies_a_color_variable_to_the_row() {
        let (mut p, a, b) = Panel::new();
        p.app.run("variable.createCollection", json!({"name":"Theme"}));
        p.app.run("variable.create", json!({"collection":"Theme","name":"Accent","type":"color","value":"#00ff00"}));
        p.app.run("variable.create", json!({"collection":"Theme","name":"Gap","type":"number","value":8}));
        let red = p.frame(vec![]).into_iter().find(|(s, _)| s == "FF0000").unwrap().1;
        p.click(Pos2::new(228.0, red.y));
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "Color variables"));
        assert!(!texts.iter().any(|(s, _)| s == "Gap"));
        let accent = texts.iter().find(|(s, _)| s == "Accent").unwrap().1;
        p.click(accent);
        let doc = p.app.session.doc().unwrap();
        assert_eq!(doc.node(a).unwrap().fills[0].solid_color().unwrap().to_hex(), "00FF00");
        assert!(!doc.node(a).unwrap().bound.is_empty());
        assert!(doc.node(b).unwrap().bound.is_empty());
        assert!(p.app.ui.last_error.is_none());
        assert!(p.frame(vec![]).iter().any(|(s, _)| s == "Accent"));
    }

    #[test]
    fn a_linked_row_detaches_its_style_or_variable() {
        let (mut p, a, b) = Panel::new();
        p.app.run("style.create", json!({"type":"paint","name":"Brand","value":"#ff0000"}));
        p.app.run("style.apply", json!({"id":a,"style":"Brand","target":"fill"}));
        p.app.run("variable.createCollection", json!({"name":"Theme"}));
        p.app.run("variable.create", json!({"collection":"Theme","name":"Accent","type":"color","value":"#0000ff"}));
        p.app.run("variable.bind", json!({"id":b,"field":"fill","variable":"Accent"}));
        for name in ["Brand", "Accent"] {
            let row = p.frame(vec![]).into_iter().find(|(s, _)| s == name).unwrap().1;
            p.click(Pos2::new(228.0, row.y));
        }
        let doc = p.app.session.doc().unwrap();
        assert!(doc.node(a).unwrap().styles.fill.is_none());
        assert!(doc.node(b).unwrap().bound.is_empty());
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "FF0000") && texts.iter().any(|(s, _)| s == "0000FF"));
        assert!(p.app.ui.last_error.is_none());
    }

    fn text_at(texts: &[(String, Pos2)], s: &str) -> Pos2 {
        texts.iter().rev().find(|(t, _)| t == s).unwrap_or_else(|| panic!("{s:?} isn't drawn")).1
    }

    #[test]
    fn styles_menu_offers_an_enabled_librarys_published_color_styles() {
        let dir = std::env::temp_dir().join(format!("figcraft-selection-colors-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("library.figcraft");
        let mut publisher = Session::new();
        publisher.execute("style.create", &json!({"type":"paint","name":"Ocean","value":"#00ffff"})).unwrap();
        publisher.execute("style.create", &json!({"type":"paint","name":"_Draft","value":"#ffff00"})).unwrap();
        publisher.execute("library.publish", &json!({})).unwrap();
        publisher.execute("file.saveAs", &json!({"path":path})).unwrap();
        let (mut p, a, b) = Panel::new();
        p.app.run("library.add", json!({"path":path}));
        let red = text_at(&p.frame(vec![]), "FF0000");
        p.click(Pos2::new(228.0, red.y));
        let texts = p.frame(vec![]);
        let library_name = p.app.session.doc().unwrap().libraries.enabled[0].name.clone();
        assert!(texts.iter().any(|(s, _)| *s == library_name), "library styles are listed under the library's name");
        assert!(!texts.iter().any(|(s, _)| s == "_Draft"), "unpublished styles aren't offered");
        p.click(text_at(&texts, "Ocean"));
        let _ = std::fs::remove_dir_all(&dir);
        let doc = p.app.session.doc().unwrap();
        let copy = doc.node(a).unwrap().styles.fill.clone().unwrap();
        assert!(library::is_remote_style(doc, &copy));
        assert_eq!(doc.node(a).unwrap().fills[0].solid_color().unwrap().to_hex(), "00FFFF");
        assert!(doc.node(b).unwrap().styles.fill.is_none());
        assert!(p.app.ui.last_error.is_none());
        // The copy is listed under its library, not as one of this file's styles.
        let blue = text_at(&p.frame(vec![]), "0000FF");
        p.click(Pos2::new(228.0, blue.y));
        let texts = p.frame(vec![]);
        assert_eq!(texts.iter().filter(|(s, _)| s == "Ocean").count(), 2, "the linked row's name and the library's entry");
    }

    #[test]
    fn create_style_names_the_row_links_it_and_stays_open_on_a_taken_name() {
        let (mut p, a, b) = Panel::new();
        let red = text_at(&p.frame(vec![]), "FF0000");
        p.click(Pos2::new(228.0, red.y));
        let create = text_at(&p.frame(vec![]), "Create style");
        p.click(create);
        for _ in 0..2 {
            p.frame(vec![]);
        }
        let form = p.ctx.data(|d| d.get_temp::<CreateForm>(create_form_id())).expect("Create style opens its window");
        assert_eq!(form.name, "Color 1");
        // Another style takes the name first: Create fails and the window stays for a new name.
        p.app.run("style.create", json!({"type":"paint","name":"Color 1","value":"#00ff00"}));
        let button = text_at(&p.frame(vec![]), "Create");
        p.click(button);
        assert!(p.ctx.data(|d| d.get_temp::<CreateForm>(create_form_id())).is_some());
        assert!(p.app.session.doc().unwrap().node(a).unwrap().styles.fill.is_none());
        p.ctx.data_mut(|d| d.get_temp_mut_or_insert_with(create_form_id(), || form.clone()).name = "Brand".into());
        let before = p.app.session.undo_depth();
        let button = text_at(&p.frame(vec![]), "Create");
        p.click(button);
        assert!(p.ctx.data(|d| d.get_temp::<CreateForm>(create_form_id())).is_none());
        let doc = p.app.session.doc().unwrap();
        let brand = doc.style("Brand").unwrap().id.clone();
        assert_eq!(doc.node(a).unwrap().styles.fill.as_deref(), Some(brand.as_str()));
        assert!(doc.node(b).unwrap().styles.fill.is_none());
        assert_eq!(p.app.session.undo_depth().0, before.0 + 1);
        assert!(p.frame(vec![]).iter().any(|(s, _)| s == "Brand"));
    }

    #[test]
    fn create_style_is_left_out_for_a_row_whose_list_holds_other_paints() {
        let (mut p, a, _) = Panel::new();
        p.app.run("fill.add", json!({"id":a,"paint":"#00ff00"}));
        let red = text_at(&p.frame(vec![]), "FF0000");
        p.click(Pos2::new(228.0, red.y));
        let texts = p.frame(vec![]);
        assert!(texts.iter().any(|(s, _)| s == "No color styles"), "the Styles menu opens");
        assert!(!texts.iter().any(|(s, _)| s == "Create style"));
    }

    #[test]
    fn a_linked_rows_styles_menu_switches_its_style() {
        let (mut p, a, _) = Panel::new();
        p.app.run("style.create", json!({"type":"paint","name":"Brand","value":"#ff0000"}));
        p.app.run("style.create", json!({"type":"paint","name":"Accent","value":"#00ff00"}));
        p.app.run("style.apply", json!({"id":a,"style":"Brand","target":"fill"}));
        let row = text_at(&p.frame(vec![]), "Brand");
        p.click(Pos2::new(196.0, row.y));
        let texts = p.frame(vec![]);
        assert!(!texts.iter().any(|(s, _)| s == "Create style"), "a linked row has a style already");
        p.click(text_at(&texts, "Accent"));
        let doc = p.app.session.doc().unwrap();
        assert_eq!(doc.node(a).unwrap().styles.fill.as_deref(), doc.style("Accent").map(|s| s.id.as_str()));
        assert!(p.app.ui.last_error.is_none());
    }
}
