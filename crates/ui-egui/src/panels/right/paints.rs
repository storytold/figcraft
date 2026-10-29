use super::menu::{blend_menu, context_popup, menu_check, menu_item, menu_popup, preview_blend, row, row_context_menu, row_rect};
use super::stroke::stroke_settings;
use super::{
    Align2, BlendMode, CornerRadius, Document, FIELD_H, FieldEdit, FigApp, Icon, Id, Lead, ListDrag, MaskType, Node, NodeId, NodeKind, Paint,
    PaintKind, Rect, Sense, Stroke, StrokeAlign, StyleSlot, Tip, Tokens, Ui, Value, VariableType, common, icons, json, moved_index, pos2, regular,
    removed_index, style_picker, tr, trf, vec2, widgets,
};

/// Draw visibility/blend/mask, opacity, uniform or per-corner radius, corner smoothing and
/// the blend-mode row.
pub(super) fn appearance(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let visible = common(doc, sel, |n| n.visible).unwrap_or(true);
    let hdr = widgets::section_header(
        ui,
        tr("Appearance"),
        &[
            (if visible { Icon::Eye } else { Icon::EyeOff }, crate::shortcuts::tip(tr("Show/Hide"), "object.toggleVisible")),
            (Icon::Variable, tr("Apply variable to visibility")),
            (Icon::Blend, tr("Blend mode")),
        ],
        false,
    );
    if hdr == Some(0) {
        app.run("object.toggleVisible", json!({"ids": ids}));
    }
    if hdr == Some(1) {
        crate::variable_ui::open_at_pointer(ui, &ids, "visible", VariableType::Boolean);
    }
    if hdr == Some(2) {
        ui.data_mut(|d| d.insert_temp(Id::unique("blend_open"), !d.get_temp::<bool>(Id::unique("blend_open")).unwrap_or(false)));
    }
    let col = widgets::col_width(ui.available_width());
    crate::variable_ui::bound_row(app, ui, &ids, "visible", VariableType::Boolean, Lead::Icon(Icon::Eye, tr("Visible")), col * 2.0 + 8.0);
    // Masks: how they mask.
    if sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.is_mask)) {
        let mt = common(doc, sel, |n| n.mask_type);
        widgets::sublabels(ui, tr("Mask"), None);
        row(ui, 32.0, |ui| {
            let resp = widgets::dropdown(ui, mt.map(|m| tr(m.label())).unwrap_or(tr("Mixed")), col * 2.0 + 8.0, Some(Icon::Mask));
            menu_popup(&resp).show(|ui| {
                for m in MaskType::ALL {
                    if menu_item(ui, tr(m.label()), "") {
                        app.set_props(&ids, json!({"maskType": m.id()}));
                        ui.close();
                    }
                }
            });
        });
    }
    let op_of = |n: &Node| (f64::from(n.opacity) * 100.0).round();
    let op = common(doc, sel, op_of);
    let radius_ok = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.has_corner_radius()));
    // Only rectangles and frames round each corner on its own.
    let indep_ok = radius_ok && sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.has_independent_corners()));
    // For polygons and stars the same panel holds only the Corner smoothing row.
    let smooth_ok = radius_ok && sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.has_corner_smoothing()));
    let r = common(doc, sel, |n| n.corners.uniform().map(|r| (r * 100.0).round() / 100.0));
    widgets::sublabels(ui, tr("Opacity"), radius_ok.then_some(tr("Corner radius")));
    row(ui, 32.0, |ui| {
        let lead = Lead::Icon(Icon::Opacity, tr("Opacity"));
        let e = crate::variable_ui::number_field_mixed(app, ui, &ids, "opacity", Id::unique("ap_op"), lead, op, "%", col);
        app.field_of(&ids, "opacity", e, op_of, |v| json!((v / 100.0).clamp(0.0, 1.0)));
        if radius_ok {
            let e = crate::variable_ui::number_field_mixed(
                app,
                ui,
                &ids,
                "cornerRadius",
                Id::unique("ap_rad"),
                Lead::Icon(Icon::CornerRadius, tr("Corner radius")),
                r.flatten(),
                "",
                col,
            );
            // A layer with corners of its own starts from its largest.
            let radius_of = |n: &Node| n.corners.uniform().unwrap_or_else(|| n.corners.radii.iter().copied().fold(0.0, f64::max));
            app.field_of(&ids, "cornerRadius", e, radius_of, |v| json!(v.max(0.0)));
            let open = ui.data(|d| d.get_temp::<bool>(Id::unique("indep_corners"))).unwrap_or(false);
            // Polygons and stars have no corners of their own: the button opens only Corner smoothing.
            let tip = if indep_ok { tr("Individual corners") } else { tr("Corner smoothing") };
            if smooth_ok && icons::button(ui, Icon::IndependentCorners, 24.0, open, tip).clicked() {
                ui.data_mut(|d| d.insert_temp(Id::unique("indep_corners"), !open));
            }
        }
    });
    if smooth_ok && ui.data(|d| d.get_temp::<bool>(Id::unique("indep_corners"))).unwrap_or(false) {
        let radii = common(doc, sel, |n| n.corners.radii);
        // `k` indexes `radii` ([top left, top right, bottom right, bottom left]); the fields sit
        // where their corners are: top left, top right / bottom left, bottom right.
        let corners = [
            ("topLeftRadius", tr("Top left radius"), Icon::CornerTopLeft, 0),
            ("topRightRadius", tr("Top right radius"), Icon::CornerTopRight, 1),
            ("bottomLeftRadius", tr("Bottom left radius"), Icon::CornerBottomLeft, 3),
            ("bottomRightRadius", tr("Bottom right radius"), Icon::CornerBottomRight, 2),
        ];
        // Polygons and stars have one radius, so no field per corner.
        for pair in corners.chunks(2).filter(|_| indep_ok) {
            row(ui, 32.0, |ui| {
                for (key, name, icon, k) in pair {
                    let e = crate::variable_ui::number_field(
                        app,
                        ui,
                        &ids,
                        key,
                        Id::unique(("corner", *k)),
                        Lead::Icon(*icon, name),
                        radii.map(|r| r[*k]),
                        "",
                        col,
                    );
                    app.field(&ids, key, e, |v| json!(v.max(0.0)));
                }
            });
        }
        let sm = common(doc, sel, |n| (n.corners.smoothing * 100.0).round());
        widgets::sublabels(ui, tr("Corner smoothing"), None);
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::unique("smoothing"), Lead::None, sm, "%", col, 1.0);
            app.field(&ids, "cornerSmoothing", e, |v| json!((v / 100.0).clamp(0.0, 1.0)));
            if widgets::button(ui, tr("iOS"), false, true).on_hover_text(tr("Set corner smoothing to 60%")).clicked() {
                app.set_props(&ids, json!({"cornerSmoothing": 0.6}));
            }
        });
    }
    let blend = common(doc, sel, |n| n.blend);
    let show_blend = ui.data(|d| d.get_temp::<bool>(Id::unique("blend_open"))).unwrap_or(false)
        || blend.is_some_and(|b| !matches!(b, BlendMode::Normal | BlendMode::PassThrough));
    if show_blend {
        widgets::sublabels(ui, tr("Blend mode"), None);
        row(ui, 32.0, |ui| {
            let label = blend.map(|b| tr(b.label())).unwrap_or(tr("Mixed"));
            let resp = widgets::dropdown(ui, label, col * 2.0 + 8.0, Some(Icon::Blend));
            menu_popup(&resp).width(200.0).show(|ui| {
                let ctx = ui.ctx().clone();
                if let Some(b) = blend_menu(ui, true, |b| preview_blend(app, &ctx, &ids, b)) {
                    app.set_props(&ids, json!({"blend": b.label()}));
                }
            });
        });
    }
    ui.add_space(8.0);
}

/// The variable modes of the selected frames (sections, components, instances): a row per
/// collection with more than one mode, Auto (the mode the layer inherits from its parent) or a mode
/// of its own (`variable.setMode`). Collections from enabled libraries follow under their
/// library's name (`library.setVariableMode` copies the collection in). Returns whether it showed.
pub(super) fn variable_modes(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    if !sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())) {
        return false;
    }
    let libs = crate::variable_ui::libraries(app, doc);
    let rows = crate::variable_ui::mode_rows(doc, &libs);
    if rows.is_empty() {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Variable modes"), &[], false);
    // The collection's name, then a wider mode picker ("Auto (Light)") across both field columns.
    let col = widgets::col_width(ui.available_width());
    let (name_w, pick_w) = ((col * 0.7).round(), (col * 1.3).round());
    // Auto resolves as for the first selected layer: from its parent, or an instance's main.
    let parent_modes = sel.first().map(|i| doc.inherited_modes(*i)).unwrap_or_default();
    let mut group = None;
    for r in &rows {
        let c = r.coll;
        // Library collections under their library's name.
        if r.library != group {
            group = r.library;
            if let Some((_, lib)) = r.library {
                widgets::sublabels(ui, lib, None);
            }
        }
        let name_of = |m: &str| c.modes.iter().find(|x| x.id == m).map_or("", |x| x.name.as_str());
        // Each layer's own mode (None: Auto); None overall when they differ.
        let own = match r.local {
            Some(l) => common(doc, sel, |n| n.var_modes.get(l).cloned()),
            None => Some(None),
        };
        let inherited = r.local.and_then(|l| parent_modes.get(l)).or(c.default_mode()).map_or("", |m| name_of(m));
        let auto = trf("Auto ({mode})", &[("mode", &inherited)]);
        let label = match &own {
            Some(Some(m)) => name_of(m).to_string(),
            Some(None) => auto.clone(),
            None => tr("Mixed").into(),
        };
        let hover = match r.library {
            Some((_, lib)) => trf("{name} (from {library})", &[("name", &c.name), ("library", &lib)]),
            None => c.name.clone(),
        };
        row(ui, 32.0, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(name_w, FIELD_H), Sense::hover());
            let name = widgets::elide(ui, &c.name, name_w);
            ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, &name, regular(11.0), t.text);
            if name != c.name || r.library.is_some() {
                ui.interact(rect, Id::unique(("var_mode_name", &c.id)), Sense::hover()).on_hover_text(&hover);
            }
            let resp = widgets::dropdown(ui, &label, pick_w, None);
            menu_popup(&resp).width(180.0).show(|ui| {
                let mut pick = None;
                if menu_check(ui, own == Some(None), &auto, "") {
                    pick = Some(Value::Null);
                }
                ui.separator();
                for m in &c.modes {
                    if menu_check(ui, own.as_ref().is_some_and(|o| o.as_ref() == Some(&m.id)), &m.name, "") {
                        pick = Some(json!(m.id));
                    }
                }
                if let Some(mode) = pick {
                    set_variable_mode(app, sel, r, mode);
                    ui.close();
                }
            });
        });
    }
    ui.add_space(8.0);
    true
}

/// Set `sel`'s mode of the collection of `row` (`mode` null: Auto). A library collection goes
/// through `library.setVariableMode` (copying it in); Auto on one never used here is already so.
fn set_variable_mode(app: &mut FigApp, sel: &[NodeId], row: &crate::variable_ui::ModeRow, mode: Value) {
    match (row.library, row.key, row.local) {
        (Some((lib, _)), Some(key), _) if !mode.is_null() => {
            app.run("library.setVariableMode", json!({"library": lib, "collection": key, "ids": sel, "mode": mode}));
        }
        (_, _, Some(local)) => {
            app.run("variable.setMode", json!({"ids": sel, "collection": local, "mode": mode}));
        }
        _ => {}
    }
}

/// While a range of text is selected, the Fill section edits that range's colour.
fn range_fill(app: &mut FigApp, ui: &mut Ui, doc: &Document, id: NodeId, start: usize, end: usize) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Fill"), &[], false);
    // The fills over the range, if every character shares them.
    let fills: Option<Vec<Paint>> = doc.node(id).and_then(|n| n.kind.text().map(|tx| (n, tx))).and_then(|(n, tx)| {
        let mut it =
            tx.spans().into_iter().filter(|(s, e, _)| *s < end && *e > start).map(|(_, _, st)| st.fills.clone().unwrap_or_else(|| n.fills.clone()));
        let first = it.next()?;
        it.all(|f| f == first).then_some(first)
    });
    row(ui, 32.0, |ui| {
        let w = ui.available_width() - 28.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
        let sw = Rect::from_min_size(pos2(r.min.x + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
        let text_r = Rect::from_min_max(pos2(sw.max.x + 8.0, r.min.y), pos2(r.max.x - 4.0, r.max.y));
        match fills.as_ref().and_then(|f| f.first()) {
            Some(p) => {
                widgets::paint_swatch(ui.painter(), sw, p);
                let range_swatch = ui.interact(sw.expand(2.0), Id::unique("range_swatch"), Sense::click());
                if range_swatch.clicked() {
                    app.toggle_picker(crate::colorpicker::PickerTarget::TextRange { id, start, end }, Some(range_swatch.rect.min.y));
                }
                if let Some(c) = p.solid_color() {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_r));
                    if let Some(h) =
                        widgets::text_field_inline(&mut child, Id::unique("range_hex"), &c.to_hex(), text_r.shrink2(vec2(0.0, 4.0)), regular(11.0))
                        && let Some(nc) = figcraft_color::Color::from_hex_or_name(&h)
                    {
                        app.run("text.style", json!({"id": id, "start": start, "end": end, "textFills": [nc.to_css_hex()]}));
                    }
                } else {
                    ui.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, tr("Gradient"), regular(11.0), t.text);
                }
            }
            None => {
                ui.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, tr("Mixed"), regular(11.0), t.text_secondary);
                if ui.interact(r, Id::unique("range_mixed"), Sense::click()).clicked() {
                    app.ui.picker = Some(crate::colorpicker::PickerTarget::TextRange { id, start, end });
                }
            }
        }
        // Back to the layer's fill.
        if icons::button(ui, Icon::Detach, 24.0, false, tr("Use the layer's fill")).clicked() {
            app.run("text.style", json!({"id": id, "start": start, "end": end, "textFills": null}));
        }
    });
}

/// Draw a selected-text range fill, or the fill/stroke paint rows with add, style, reorder,
/// stroke align/weight, per-side weights and advanced stroke settings.
pub(super) fn paints(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], fills: bool) {
    if fills
        && let Some((id, a, b)) = figcraft_engine::textedit::selected_range(&app.session)
        && a < b
        && sel.contains(&id)
    {
        range_fill(app, ui, doc, id, a, b);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let ids = sel.to_vec();
    let list = common(doc, sel, |n| if fills { n.fills.clone() } else { n.strokes.clone() });
    let title = if fills { tr("Fill") } else { tr("Stroke") };
    let empty = list.as_ref().is_none_or(|l| l.is_empty());
    let slot = if fills { StyleSlot::Fill } else { StyleSlot::Stroke };
    // The section's right-click menu adds the copied paint (`fill.paste` / `stroke.paste`).
    let paste = if fills { tr("Paste fill") } else { tr("Paste stroke") };
    let (clicked, header) = widgets::section_header_resp(
        ui,
        title,
        &[(Icon::Styles, tr("Styles")), (Icon::Plus, if fills { tr("Add fill") } else { tr("Add stroke") })],
        empty,
    );
    if let Some(i) = clicked {
        if i == 1 {
            // The new paint goes on top; open its picker. A mixed selection has no one index to edit.
            if app.run(if fills { "fill.add" } else { "stroke.add" }, json!({"ids": ids})).is_some()
                && let Some(l) = &list
            {
                app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills, index: l.len(), ids: ids.clone() });
            }
        } else {
            style_picker::open(app, ui, &ids, slot);
        }
    }
    context_popup(&header).show(|ui| {
        if app.session.clipboard.paint.as_ref().is_some_and(|(f, _)| *f == fills) && menu_item(ui, paste, "") {
            app.run(if fills { "fill.paste" } else { "stroke.paste" }, json!({"ids": ids}));
            ui.close();
        }
    });
    style_picker::chip(app, ui, doc, &ids, slot);
    match &list {
        None => widgets::note(ui, tr("Click + to replace mixed content"), t.text_secondary),
        Some(l) => {
            // The last paint is on top: shown first. Rows drag to reorder.
            let mut drag = ListDrag::new(ui, if fills { "fills" } else { "strokes" });
            for (i, p) in l.iter().enumerate().rev() {
                let r = paint_row(app, ui, &ids, i, p, fills);
                drag.row(ui, i, r, r, || paint_label(p));
            }
            if let Some((from, to)) = drag.finish(ui) {
                app.run(if fills { "fill.move" } else { "stroke.move" }, json!({"ids": ids, "from": from, "to": to}));
                // An open colour picker keeps editing the paint it was on.
                if let Some(crate::colorpicker::PickerTarget::Paint { fills: f, index, .. }) = &mut app.ui.picker
                    && *f == fills
                {
                    *index = moved_index(*index, from, to);
                }
            }
        }
    }
    if !fills && !empty {
        let weight = common(doc, sel, |n| n.stroke.weight);
        let align = common(doc, sel, |n| n.stroke.align);
        row(ui, 32.0, |ui| {
            // The row is the panel's width: the two fields split what the buttons at its end leave,
            // so the align field and the weight field are the same column (76 at the default width
            // with two buttons, 88 with one). The buttons are 24 pt each, 4 apart on the icon grid.
            let sided = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Rectangle)));
            let group = if sided { 24.0 + 4.0 + 24.0 } else { 24.0 };
            let field_w = ((ui.available_width() - widgets::GUTTER * 2.0 - group) / 2.0).max(40.0);
            let resp = widgets::dropdown(
                ui,
                match align {
                    Some(StrokeAlign::Inside) => tr("Inside"),
                    Some(StrokeAlign::Center) => tr("Center"),
                    Some(StrokeAlign::Outside) => tr("Outside"),
                    None => tr("Mixed"),
                },
                field_w,
                None,
            );
            menu_popup(&resp).show(|ui| {
                for (l, v) in [(tr("Inside"), "inside"), (tr("Center"), "center"), (tr("Outside"), "outside")] {
                    if menu_item(ui, l, "") {
                        app.set_props(&ids, json!({"strokeAlign": v}));
                        ui.close();
                    }
                }
            });
            let e = crate::variable_ui::number_field_mixed(
                app,
                ui,
                &ids,
                "strokeWeight",
                Id::unique("stroke_w"),
                Lead::Icon(Icon::Menu, tr("Stroke weight")),
                weight,
                "",
                field_w,
            );
            app.field_of(&ids, "strokeWeight", e, |n| n.stroke.weight, |v| json!(v.max(0.0)));
            let adv_id = Id::unique("stroke_adv");
            let open = ui.data(|d| d.get_temp::<bool>(adv_id)).unwrap_or(false);
            let advanced = |ui: &mut Ui| {
                if icons::button(ui, Icon::Dots, 24.0, open, tr("Advanced stroke")).clicked() {
                    ui.data_mut(|d| d.insert_temp(adv_id, !open));
                }
            };
            if sided {
                // The two buttons are one group on the row's right edge: 24 each, 4 between them.
                widgets::tight_end(ui, |ui| {
                    let sides_btn = icons::button(ui, Icon::IndependentCorners, 24.0, false, tr("Individual strokes"));
                    menu_popup(&sides_btn).show(|ui| {
                        let w = weight.unwrap_or(1.0);
                        for (l, v) in [
                            (tr("All"), Value::Null),
                            (tr("Top"), json!([w, 0.0, 0.0, 0.0])),
                            (tr("Right"), json!([0.0, w, 0.0, 0.0])),
                            (tr("Bottom"), json!([0.0, 0.0, w, 0.0])),
                            (tr("Left"), json!([0.0, 0.0, 0.0, w])),
                            (tr("Custom"), json!([w, w, w, w])),
                        ] {
                            if menu_item(ui, l, "") {
                                app.set_props(&ids, json!({"strokeSides": v}));
                                ui.close();
                            }
                        }
                    });
                    advanced(ui);
                });
            } else {
                advanced(ui);
            }
        });
        // Per-side weights, two to a row (like the corner radii) so a bound variable's name fits:
        // each side binds to a number variable on its own.
        if let Some(sides) = common(doc, sel, |n| n.stroke.sides).flatten() {
            let col = widgets::col_width(ui.available_width());
            let fields = ["T", "R", "B", "L"].into_iter().zip(figcraft_engine::cmd::props::STROKE_SIDES);
            for first in [0, 2] {
                row(ui, 32.0, |ui| {
                    for (k, (label, field)) in fields.clone().enumerate().skip(first).take(2) {
                        let v = sides.get(k).copied();
                        let e = crate::variable_ui::number_field(app, ui, &ids, field, Id::unique(("stroke_side", k)), Lead::Text(label), v, "", col);
                        app.field(&ids, field, e, |x| json!(x.max(0.0)));
                    }
                });
            }
        }
        if ui.data(|d| d.get_temp::<bool>(Id::unique("stroke_adv"))).unwrap_or(false) {
            stroke_settings(app, ui, doc, sel);
        }
    }
}

/// Paint `i`'s row (swatch, colour or kind, opacity, visibility, remove); returns its area.
fn paint_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, p: &Paint, fills: bool) -> Rect {
    let t = Tokens::get(ui.ctx());
    let ids = ids.to_vec();
    let cmd = if fills { "fill.set" } else { "stroke.set" };
    let r = row_rect(ui, 32.0, |ui| {
        let w = ui.available_width() - 60.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
        let sw = Rect::from_min_size(pos2(r.min.x + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
        widgets::paint_swatch(ui.painter(), sw, p);
        let sresp = ui.interact(sw.expand(2.0), Id::unique(("swatch", fills, i)), Sense::click());
        // While this paint is the one the picker edits, its swatch is held out of the popover's
        // closing, so a press on it again reaches the toggle below.
        let editing = matches!(
            &app.ui.picker,
            Some(crate::colorpicker::PickerTarget::Paint { fills: f, index: k, ids: held }) if *f == fills && *k == i && *held == ids
        );
        if editing {
            app.ui.picker_button = Some(sresp.rect);
        }
        if sresp.clicked() {
            app.toggle_picker(crate::colorpicker::PickerTarget::Paint { fills, index: i, ids: ids.clone() }, Some(sresp.rect.min.y));
        }
        let text_r = Rect::from_min_max(pos2(sw.max.x + 8.0, r.min.y), pos2(r.max.x - 52.0, r.max.y));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_r));
        // The variable bound to this paint's colour (its key on the layers: `fill` or `fills/<i>/color`).
        let bound = app.session.doc().ok().and_then(|d| {
            let key = ids.first().and_then(|id| d.node(*id)).and_then(|n| figcraft_engine::cmd::lists::paint_binding_key(n, fills, i))?;
            Some((key.to_string(), crate::variable_ui::bound_name(d, &ids, key)?))
        });
        match &p.kind {
            PaintKind::Solid { .. } if bound.is_some() => {
                let (key, name) = bound.clone().unwrap_or_default();
                let chip = Rect::from_min_max(pos2(text_r.min.x - 2.0, r.min.y + 3.0), pos2(text_r.max.x - 20.0, r.max.y - 3.0));
                child.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
                child.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let short = name.rsplit('/').next().unwrap_or(&name).to_string();
                child.painter().with_clip_rect(chip.shrink2(vec2(5.0, 0.0))).text(
                    pos2(chip.min.x + 5.0, r.center().y),
                    Align2::LEFT_CENTER,
                    short,
                    regular(11.0),
                    t.text,
                );
                let _ = ui.interact(chip, Id::unique(("bound_paint", fills, i)), Sense::hover()).on_hover_text(&name);
                let dr = Rect::from_center_size(pos2(text_r.max.x - 9.0, r.center().y), vec2(18.0, 18.0));
                let dresp = ui.interact(dr, Id::unique(("detach_paint", fills, i)), Sense::click()).tip(tr("Detach variable"));
                icons::paint(ui.painter(), dr.shrink(3.0), Icon::Detach, t.icon_secondary);
                if dresp.clicked() {
                    app.run("variable.bind", json!({"ids": ids, "field": key, "variable": null}));
                }
            }
            PaintKind::Solid { color } => {
                if let Some(h) = widgets::text_field_inline(
                    &mut child,
                    Id::unique(("hex", fills, i)),
                    &color.to_hex(),
                    text_r.shrink2(vec2(0.0, 4.0)),
                    regular(11.0),
                ) && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
                {
                    app.run(cmd, json!({"ids": ids, "index": i, "color": c.to_css_hex()}));
                }
            }
            _ => {
                child.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, paint_label(p), regular(11.0), t.text);
                // One layer's image fill: crop it on the canvas.
                if fills && ids.len() == 1 && matches!(p.kind, PaintKind::Image(_)) {
                    let cr = Rect::from_center_size(pos2(text_r.max.x - 9.0, r.center().y), vec2(18.0, 18.0));
                    let cresp = ui.interact(cr, Id::unique(("crop_image", i)), Sense::click()).tip(tr("Crop image"));
                    icons::paint(ui.painter(), cr.shrink(3.0), Icon::Crop, if cresp.hovered() { t.icon } else { t.icon_secondary });
                    if cresp.clicked() {
                        app.run("image.crop", json!({"ids": ids, "index": i}));
                    }
                }
            }
        }
        let op_r = Rect::from_min_max(pos2(r.max.x - 48.0, r.min.y), r.max);
        ui.painter().line_segment([pos2(op_r.min.x, r.min.y + 4.0), pos2(op_r.min.x, r.max.y - 4.0)], Stroke::new(1.0, t.panel));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(op_r));
        let e = widgets::number_field(
            &mut child,
            Id::unique(("pop", fills, i)),
            Lead::None,
            Some((f64::from(p.opacity) * 100.0).round()),
            "%",
            48.0,
            1.0,
        );
        match e {
            Some(FieldEdit::Set(v)) => {
                app.run(cmd, json!({"ids": ids, "index": i, "opacity": (v / 100.0).clamp(0.0, 1.0)}));
            }
            Some(FieldEdit::Scrub(v, done)) => {
                let _ = app.session.begin_interaction("Opacity");
                let _ = app.session.execute(cmd, &json!({"ids": ids, "index": i, "opacity": (v / 100.0).clamp(0.0, 1.0)}));
                if done {
                    let _ = app.session.commit_interaction();
                }
            }
            None => {}
        }
        // The row's two buttons are one group on the grid: 24 each, 4 between them.
        let (toggle, remove) = widgets::tight_end(ui, |ui| {
            (
                icons::button(ui, if p.visible { Icon::Eye } else { Icon::EyeOff }, 24.0, false, tr("Toggle visibility")).clicked(),
                icons::button(ui, Icon::Minus, 24.0, false, tr("Remove")).clicked(),
            )
        });
        if toggle {
            app.run(cmd, json!({"ids": ids, "index": i, "visible": !p.visible}));
        }
        if remove
            && app.run(if fills { "fill.remove" } else { "stroke.remove" }, json!({"ids": ids, "index": i})).is_some()
            && let Some(crate::colorpicker::PickerTarget::Paint { fills: f, index, .. }) = &mut app.ui.picker
            && *f == fills
        {
            // An open colour picker keeps editing the paint it was on, or closes with it.
            match removed_index(*index, i) {
                Some(k) => *index = k,
                None => app.ui.picker = None,
            }
        }
    });
    let copy = if fills { tr("Copy fill") } else { tr("Copy stroke") };
    let paste = if fills { tr("Paste fill") } else { tr("Paste stroke") };
    let can_paste = app.session.clipboard.paint.as_ref().is_some_and(|(f, _)| *f == fills);
    row_context_menu(ui, r, Id::unique(("paint_menu", fills, i))).show(|ui| {
        if menu_item(ui, copy, "") {
            app.run(if fills { "fill.copy" } else { "stroke.copy" }, json!({"ids": ids, "index": i}));
            ui.close();
        }
        if can_paste && menu_item(ui, paste, "") {
            app.run(if fills { "fill.paste" } else { "stroke.paste" }, json!({"ids": ids, "index": i}));
            ui.close();
        }
    });
    r
}

/// A paint row's label (also what a drag of it carries): its colour, gradient kind or "Image".
fn paint_label(p: &Paint) -> String {
    match &p.kind {
        PaintKind::Solid { color } => color.to_hex(),
        // The gradient kind's name ("Linear") is a key of its own.
        PaintKind::Gradient(g) => tr(&format!("{:?}", g.kind)).to_string(),
        PaintKind::Image(_) => tr("Image").into(),
        PaintKind::Pattern(_) => tr("Pattern").into(),
    }
}
