fn image_layer(session: &mut Session, filters: ImageFilters) -> NodeId {
    image_layer_at(session, filters, 0.0)
}

fn image_layer_at(session: &mut Session, filters: ImageFilters, x: f64) -> NodeId {
    let id: NodeId =
        serde_json::from_value(session.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone())
            .unwrap();
    let mut paint = Paint::image("test-image", ImageScaleMode::Fill);
    if let PaintKind::Image(image) = &mut paint.kind {
        image.filters = filters;
    }
    session.execute("fill.set", &json!({"ids": [id], "index": 0, "paint": paint})).unwrap();
    id
}

#[test]
fn image_picker_follows_single_selected_image_and_reads_its_filters() {
    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters::default());
    let next_filters =
        ImageFilters { exposure: 0.65, contrast: -0.2, saturation: 0.35, temperature: -0.45, tint: 0.15, highlights: -0.55, shadows: 0.25 };
    let next = image_layer(&mut session, next_filters);
    session.set_selection(vec![first]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] });

    app.session.set_selection(vec![next]);
    assert!(sync_picker_target(&mut app));
    assert_eq!(app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![next] }));
    let PaintKind::Image(image) = current_paint(&app, app.ui.picker.as_ref().unwrap()).unwrap().kind else { panic!("image paint") };
    assert_eq!(image.filters, next_filters);

    let target = app.ui.picker.as_ref().unwrap().clone();
    let mut changed = current_paint(&app, &target).unwrap();
    let PaintKind::Image(image) = &mut changed.kind else { panic!("image paint") };
    image.filters.saturation = -0.8;
    write(&mut app, &target, &changed, false);
    let first_paint = current_paint(&app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![first] }).unwrap();
    let next_paint = current_paint(&app, &target).unwrap();
    let (PaintKind::Image(first_image), PaintKind::Image(next_image)) = (first_paint.kind, next_paint.kind) else { panic!("image paints") };
    assert_eq!(first_image.filters, ImageFilters::default(), "the old image was not edited");
    assert_eq!(next_image.filters.saturation, -0.8, "the new image received the edit");
}

#[test]
fn image_picker_for_two_selected_layers_stays_open_and_edits_both() {
    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters::default());
    let second = image_layer_at(&mut session, ImageFilters::default(), 30.0);
    session.set_selection(vec![first, second]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    let target = PickerTarget::Paint { fills: true, index: 0, ids: vec![first, second] };
    app.ui.picker = Some(target.clone());

    assert!(sync_picker_target(&mut app));
    assert_eq!(app.ui.picker, Some(target.clone()));
    let mut changed = current_paint(&app, &target).unwrap();
    let PaintKind::Image(image) = &mut changed.kind else { panic!("image paint") };
    image.filters.saturation = -0.8;
    write(&mut app, &target, &changed, false);
    for id in [first, second] {
        let paint = current_paint(&app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![id] }).unwrap();
        let PaintKind::Image(image) = paint.kind else { panic!("image paint") };
        assert_eq!(image.filters.saturation, -0.8, "both layers were edited");
    }

    app.session.set_selection(vec![first]);
    assert!(sync_picker_target(&mut app));
    assert_eq!(app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] }), "a different selection retargets");
}

#[test]
fn dragging_a_text_colour_while_editing_records_the_final_colour() {
    let mut session = Session::new();
    let text = NodeId(session.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"].as_u64().unwrap());
    session.execute("text.edit.begin", &json!({"id": text})).unwrap();
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    let target = PickerTarget::TextRange { id: text, start: 0, end: 5 };
    app.ui.picker = Some(target.clone());
    for hex in ["#110000", "#220000", "#330000"] {
        write(&mut app, &target, &Paint::solid(Color::from_hex(hex).unwrap()), true);
    }
    assert!(app.session.recent_colors.is_empty(), "previews are not recorded");
    assert_eq!(app.session.ds().unwrap().interaction.as_ref().map(|interaction| interaction.label.as_str()), Some("Edit text"));

    finish(&mut app);
    assert_eq!(app.session.recent_colors, vec![Color::from_hex("#330000").unwrap()]);
    assert!(app.session.ds().unwrap().text_edit.is_some(), "the text edit goes on");
}

#[test]
fn document_switch_keeps_a_non_image_picker_open() {
    let mut app = FigApp::new(Session::new(), Box::new(crate::NoServices));
    app.ui.picker = Some(PickerTarget::PageBackground);
    assert!(sync_picker_target(&mut app));
    app.session.execute("file.new", &json!({"name": "Other"})).unwrap();

    assert!(sync_picker_target(&mut app));
    assert_eq!(app.ui.picker, Some(PickerTarget::PageBackground));
}

#[test]
fn selection_order_does_not_close_a_multi_layer_picker() {
    let mut u = Ui::new(Session::new());
    let first = image_layer(&mut u.app.session, ImageFilters::default());
    let second = image_layer_at(&mut u.app.session, ImageFilters::default(), 30.0);
    u.app.session.set_selection(vec![first, second]);
    let target = PickerTarget::Paint { fills: true, index: 0, ids: vec![first, second] };
    u.app.ui.picker = Some(target.clone());
    u.frame(vec![]);
    u.app.session.set_selection(vec![second, first]);
    u.frame(vec![]);
    assert_eq!(u.app.ui.picker, Some(target), "the same layers in another order keep the popup open");
}

#[test]
fn closing_a_fill_picker_for_another_selection_records_no_recent_colour() {
    let mut session = Session::new();
    let first = NodeId(session.execute("shape.rectangle", &json!({"width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let other = NodeId(session.execute("shape.rectangle", &json!({"x": 40, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    session.execute("fill.set", &json!({"ids": [first], "color": "#FF0000"})).unwrap();
    session.set_selection(vec![first]);
    let mut u = Ui::new(session);
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] });
    u.frame(vec![]);
    // Pressing another layer starts a Move interaction and selects it.
    u.app.session.begin_interaction("Move").unwrap();
    u.app.session.set_selection(vec![other]);
    u.frame(vec![]);
    assert!(u.app.ui.picker.is_none(), "the picker closed");
    assert!(u.app.session.recent_colors.is_empty(), "no colour was picked: {:?}", u.app.session.recent_colors);
}

#[test]
fn image_picker_closes_for_empty_or_non_image_selection() {
    for select_image in [false, true] {
        let mut session = Session::new();
        let image = image_layer(&mut session, ImageFilters::default());
        let other: NodeId =
            serde_json::from_value(session.execute("shape.rectangle", &json!({"x": 30, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone())
                .unwrap();
        session.set_selection(vec![image]);
        let mut app = FigApp::new(session, Box::new(crate::NoServices));
        app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![image] });
        app.session.set_selection(if select_image { vec![other] } else { Vec::new() });

        assert!(!sync_picker_target(&mut app));
        assert!(app.ui.picker.is_none());
    }
}

#[test]
fn retargeting_does_not_finish_a_canvas_interaction() {
    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters::default());
    let next = image_layer(&mut session, ImageFilters::default());
    session.set_selection(vec![first]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] });
    app.session.begin_interaction("Move").unwrap();
    app.session.set_selection(vec![next]);

    assert!(sync_picker_target(&mut app));
    assert_eq!(app.session.ds().unwrap().interaction.as_ref().map(|interaction| interaction.label.as_str()), Some("Move"));
    assert_eq!(app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![next] }));
}

#[test]
fn image_picker_uses_first_image_slot_when_old_slot_is_missing() {
    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters::default());
    session.execute("fill.set", &json!({"ids": [first], "index": 0, "color": "#FF0000"})).unwrap();
    session.execute("fill.add", &json!({"ids": [first], "paint": Paint::image("first-image-second-slot", ImageScaleMode::Fill)})).unwrap();
    let next = image_layer(&mut session, ImageFilters { exposure: -0.35, ..ImageFilters::default() });
    let second = Paint::image("second-image", ImageScaleMode::Fill);
    session.execute("fill.set", &json!({"ids": [next], "index": 0, "paint": second})).unwrap();
    session.execute("stroke.add", &json!({"ids": [next], "paint": Paint::image("stroke-image", ImageScaleMode::Fill)})).unwrap();
    session.set_selection(vec![first]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 1, ids: vec![first] });
    assert!(sync_picker_target(&mut app));

    app.session.set_selection(vec![next]);
    assert!(sync_picker_target(&mut app));
    assert_eq!(app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![next] }));
    let PaintKind::Image(image) = current_paint(&app, app.ui.picker.as_ref().unwrap()).unwrap().kind else { panic!("image paint") };
    assert_eq!(image.image, "second-image");
}

#[test]
fn image_picker_closes_when_document_changes_even_with_reused_node_ids() {
    let mut session = Session::new();
    let image = image_layer(&mut session, ImageFilters::default());
    session.set_selection(vec![image]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![image] });
    assert!(sync_picker_target(&mut app));

    app.session.execute("file.new", &json!({"name": "Other"})).unwrap();
    let collision: NodeId =
        serde_json::from_value(app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone())
            .unwrap();
    assert_eq!(collision, image);
    app.session.execute("fill.set", &json!({"ids": [collision], "color": "#FF0000"})).unwrap();

    assert!(!sync_picker_target(&mut app));
    assert!(app.ui.picker.is_none());
    assert!(app.ui.picker_context.is_none());
}

#[test]
fn document_switch_finishes_color_edit_in_owner_without_touching_new_document() {
    let mut session = Session::new();
    let image = image_layer(&mut session, ImageFilters::default());
    session.set_selection(vec![image]);
    let mut app = FigApp::new(session, Box::new(crate::NoServices));
    let target = PickerTarget::Paint { fills: true, index: 0, ids: vec![image] };
    app.ui.picker = Some(target.clone());
    assert!(sync_picker_target(&mut app));

    let mut paint = current_paint(&app, &target).unwrap();
    if let PaintKind::Image(image) = &mut paint.kind {
        image.filters.exposure = 0.8;
    }
    write(&mut app, &target, &paint, true);
    let owner_uid = app.session.ds().unwrap().uid;
    app.session.execute("file.new", &json!({"name": "Other"})).unwrap();
    let new_uid = app.session.ds().unwrap().uid;
    app.session.begin_interaction("Move").unwrap();

    assert!(!sync_picker_target(&mut app));
    assert!(app.ui.picker.is_none());
    assert!(app.session.docs[0].interaction.is_none(), "old Color interaction was committed");
    assert_eq!(app.session.docs[0].history.undo.last().map(|entry| entry.label.as_str()), Some("Color"));
    assert_ne!(new_uid, owner_uid);
    assert_eq!(app.session.ds().unwrap().uid, new_uid);
    assert_eq!(app.session.ds().unwrap().interaction.as_ref().map(|interaction| interaction.label.as_str()), Some("Move"));

    app.session.active = 0;
    app.session.execute("edit.undo", &json!({})).unwrap();
    let PaintKind::Image(restored) = app.session.doc().unwrap().node(image).unwrap().fills[0].kind.clone() else { panic!("image paint") };
    assert_eq!(restored.filters.exposure, 0.0, "undo restored only the old image adjustment");
    app.session.active = 1;
    assert_eq!(app.session.ds().unwrap().interaction.as_ref().map(|interaction| interaction.label.as_str()), Some("Move"));
}

#[test]
fn page_switch_preserves_non_image_picker() {
    for paint in [Paint::solid(Color::BLACK), Paint::image("test-image", ImageScaleMode::Fill)] {
        let mut session = Session::new();
        let created = session.execute("style.create", &json!({"type": "paint", "name": "Test paint", "value": [paint]})).unwrap();
        let target = PickerTarget::Style { id: created["id"].as_str().unwrap().into(), index: 0 };
        let mut app = FigApp::new(session, Box::new(crate::NoServices));
        app.ui.picker = Some(target.clone());
        assert_eq!(current_paint(&app, &target), Some(paint.clone()));
        assert!(sync_picker_target(&mut app));
        let page = serde_json::from_value::<NodeId>(app.session.execute("page.add", &json!({})).unwrap()["id"].clone()).unwrap();
        app.session.execute("page.select", &json!({"id": page})).unwrap();

        assert!(sync_picker_target(&mut app));
        assert_eq!(app.ui.picker, Some(target.clone()));
        assert_eq!(current_paint(&app, &target), Some(paint));
    }
}

#[test]
fn selecting_and_dragging_an_image_layer_keeps_its_move_undoable() {
    use egui::Event;

    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters { exposure: 0.4, ..ImageFilters::default() });
    let next = image_layer_at(&mut session, ImageFilters { exposure: -0.4, ..ImageFilters::default() }, 300.0);
    session.set_selection(vec![first]);
    let mut ui = Ui::new(session);
    ui.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] });
    ui.frame(vec![]);

    let before = ui.app.session.doc().unwrap().world_bounds(next).unwrap().center();
    let start = ui.app.to_screen(before.x, before.y);
    let end = start + egui::vec2(60.0, 24.0);
    ui.button(start, true);
    ui.frame(vec![Event::PointerMoved(start + egui::vec2(30.0, 12.0))]);
    ui.frame(vec![Event::PointerMoved(end)]);
    ui.button(end, false);

    let after = ui.app.session.doc().unwrap().world_bounds(next).unwrap().center();
    assert!(after.x > before.x + 40.0, "the newly selected image moved");
    let first_paint = current_paint(&ui.app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![first] }).unwrap();
    let next_paint = current_paint(&ui.app, &PickerTarget::Paint { fills: true, index: 0, ids: vec![next] }).unwrap();
    let (PaintKind::Image(first_image), PaintKind::Image(next_image)) = (first_paint.kind, next_paint.kind) else { panic!("image paints") };
    assert_eq!(first_image.filters.exposure, 0.4, "the old image stayed unchanged");
    assert_eq!(next_image.filters.exposure, -0.4, "the new image stayed unchanged");

    ui.app.session.execute("edit.undo", &json!({})).unwrap();
    let restored = ui.app.session.doc().unwrap().world_bounds(next).unwrap().center();
    assert!((restored.x - before.x).abs() < 0.01, "one undo restored the new image position");
}

#[test]
fn same_frame_canvas_click_keeps_image_picker_open_on_new_selection() {
    use egui::{Event, PointerButton};

    let mut session = Session::new();
    let first = image_layer(&mut session, ImageFilters { exposure: 0.4, ..ImageFilters::default() });
    let next = image_layer_at(&mut session, ImageFilters { exposure: -0.4, ..ImageFilters::default() }, 300.0);
    session.set_selection(vec![first]);
    let mut ui = Ui::new(session);
    ui.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![first] });
    ui.frame(vec![]);

    let bounds = ui.app.session.doc().unwrap().world_bounds(next).unwrap();
    let at = ui.app.to_screen(bounds.center().x, bounds.center().y);
    ui.frame(vec![
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() },
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ]);

    assert_eq!(ui.app.session.selection(), vec![next], "the click selected the image");
    assert_eq!(ui.app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![next] }), "the popup stayed open and followed it");
    let PaintKind::Image(image) = current_paint(&ui.app, ui.app.ui.picker.as_ref().unwrap()).unwrap().kind else { panic!("image paint") };
    assert_eq!(image.filters.exposure, -0.4, "the new image's filter values loaded");
}
