//! The style picker: name folders, new style names, and the popover drawn headless (list, grid,
//! editor), committing a typed name when it closes.

use egui::{Id, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use super::*;
use crate::{FigApp, NoServices};

#[test]
fn names_split_into_folders() {
    assert_eq!((folder("Brand/Blue/500"), leaf("Brand/Blue/500")), ("Brand/Blue", "500"));
    assert_eq!((folder("Danger"), leaf("Danger")), ("", "Danger"));
    assert_eq!((folder(" Brand / Primary "), leaf(" Brand / Primary ")), ("Brand", "Primary"));
    let mut d = Document::new("d");
    assert_eq!(next_name(&d, "Color"), "Color 1");
    for n in ["Color 1", "Color 2"] {
        d.styles.push(Style {
            id: n.into(),
            name: n.into(),
            description: String::new(),
            kind: StyleKind::Paint { paints: vec![] },
            bound: Default::default(),
        });
    }
    assert_eq!(next_name(&d, "Color"), "Color 3");
    assert!(StyleSlot::Fill.holds(&StyleKind::Paint { paints: vec![] }) && !StyleSlot::Effect.holds(&StyleKind::Paint { paints: vec![] }));
    for s in [StyleSlot::Fill, StyleSlot::Stroke, StyleSlot::Effect, StyleSlot::Grid, StyleSlot::Text] {
        assert_eq!(StyleSlot::parse(s.key()), Some(s));
    }
    assert!(StyleSlot::Text.holds(&StyleKind::Text { style: Default::default() }) && StyleSlot::Text.has_grid());
}

fn frame(ctx: &egui::Context, app: &mut FigApp, t: &mut f64) {
    frame_with(ctx, app, t, vec![]);
}

fn frame_with(ctx: &egui::Context, app: &mut FigApp, t: &mut f64, events: Vec<egui::Event>) {
    *t += 1.0 / 60.0;
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(*t), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    out.textures_delta.clear();
}

#[test]
fn picker_draws_every_view_and_commits_a_typed_name_on_close() {
    let mut s = Session::new();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    for (name, c) in [("Brand/Primary", "#0D99FF"), ("Brand/Accent", "#FFC700"), ("Danger", "#F24822")] {
        s.execute("style.create", &json!({"type": "paint", "name": name, "value": [{"type": "solid", "color": c}]})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("style.apply", &json!({"ids": [r], "style": "Brand/Primary", "target": "fill"})).unwrap();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    for slot in [StyleSlot::Fill, StyleSlot::Stroke, StyleSlot::Effect, StyleSlot::Grid, StyleSlot::Text] {
        open_for_selection(&mut app, slot, None);
        for grid in [false, true] {
            app.ui.style_grid = grid;
            for _ in 0..3 {
                frame(&ctx, &mut app, &mut t);
            }
            assert_eq!(app.ui.style_picker.as_ref().map(Picker::slot), Some(slot), "{slot:?} stays open");
        }
    }
    // Folders fold.
    app.ui.style_folds.insert("Brand".into());
    frame(&ctx, &mut app, &mut t);
    // The editor; a name typed there is written when the picker closes (here: a new selection).
    open_for_selection(&mut app, StyleSlot::Fill, Some("Brand/Accent"));
    frame(&ctx, &mut app, &mut t);
    let edit = app.ui.style_picker.as_mut().and_then(|p| p.edit.as_mut()).expect("editing");
    edit.name = "Brand/Gold".into();
    app.run("select.none", json!({}));
    frame(&ctx, &mut app, &mut t);
    assert!(app.ui.style_picker.is_none(), "a new selection closes the picker");
    let doc = app.session.doc().unwrap();
    assert!(doc.style("Brand/Gold").is_some(), "{:?}", doc.styles.iter().map(|s| &s.name).collect::<Vec<_>>());
}

/// A session with a text layer and text styles "Type/Heading" and "Type/Body" (the layer uses Body).
fn text_session() -> (Session, figcraft_doc::NodeId) {
    let mut s = Session::new();
    let t = figcraft_engine::params::id_of(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute(
        "style.create",
        &json!({"type": "text", "name": "Type/Heading", "value": {"fontFamily": "Inter", "fontStyle": "Bold", "fontSize": 32}}),
    )
    .unwrap();
    s.execute("style.create", &json!({"type": "text", "name": "Type/Body", "value": {"fontFamily": "Inter", "fontSize": 14}})).unwrap();
    s.execute("style.apply", &json!({"ids": [t], "style": "Type/Body"})).unwrap();
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    (s, t)
}

#[test]
fn text_styles_preview_on_hover_and_apply_on_click() {
    let (s, t) = text_session();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut time = 0.0;
    open_for_selection(&mut app, StyleSlot::Text, None);
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut time);
    }
    let area = ctx.memory(|m| m.area_rect(Id::new("style_picker_area"))).expect("the picker shows");
    let size = |app: &FigApp| app.session.doc().unwrap().node(t).unwrap().kind.text().unwrap().style.font_size;
    let undo = app.session.ds().unwrap().history.undo.len();
    // Down the popover until a style row previews on the canvas: the heading (listed first).
    let mut over = None;
    for y in (area.min.y as i32..area.max.y as i32).step_by(4) {
        let p = Pos2::new(area.center().x - 40.0, y as f32);
        frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(p)]);
        frame(&ctx, &mut app, &mut time);
        if figcraft_engine::textpreview::active(&app.session) {
            over = Some(p);
            break;
        }
    }
    let p = over.expect("a hovered text style previews");
    assert_eq!(size(&app), 32.0, "the canvas shows the hovered style");
    assert_eq!(app.session.ds().unwrap().history.undo.len(), undo, "a preview is no edit");
    // Away from the popover the preview ends.
    frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(Pos2::new(600.0, 600.0))]);
    frame(&ctx, &mut app, &mut time);
    assert!(!figcraft_engine::textpreview::active(&app.session));
    assert_eq!(size(&app), 14.0);
    // A click applies it: one undo step, and the picker closes.
    let press = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_with(&ctx, &mut app, &mut time, vec![egui::Event::PointerMoved(p)]);
    frame_with(&ctx, &mut app, &mut time, vec![press(true)]);
    frame_with(&ctx, &mut app, &mut time, vec![press(false)]);
    frame(&ctx, &mut app, &mut time);
    assert_eq!(size(&app), 32.0);
    assert_eq!(app.session.ds().unwrap().history.undo.len(), undo + 1);
    assert!(app.ui.style_picker.is_none());
    let doc = app.session.doc().unwrap();
    assert_eq!(doc.node(t).unwrap().styles.text.as_deref(), doc.style("Type/Heading").map(|s| s.id.as_str()));
}

#[test]
fn chars_linked_to_several_styles_show_mixed() {
    let (mut s, t) = text_session();
    s.execute("style.apply", &json!({"ids": [t], "style": "Type/Heading", "start": 0, "end": 2})).unwrap();
    let app = FigApp::new(s, Box::new(NoServices));
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), None, "mixed");
    let mut app = app;
    // Editing the first two chars: they link to one style.
    figcraft_engine::textedit::begin(&mut app.session, t, Some(0), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut app.session, 2, true).unwrap();
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), Some(doc.style("Type/Heading").map(|s| s.id.clone())));
    // The panel draws the chip ("Mixed" when the edit spans both).
    figcraft_engine::textedit::set_caret(&mut app.session, 4, true).unwrap();
    let ctx = egui::Context::default();
    let mut time = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut time);
    }
    let doc = app.session.doc().unwrap().clone();
    assert_eq!(text_link(&app, &doc, &[t]), None);
}

#[test]
fn local_style_drops_move_into_the_folder_above() {
    let st = |id: &str, name: &str| Style {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![] },
        bound: Default::default(),
    };
    let (a, b, c) = (st("a", "Top"), st("b", "Brand/One"), st("c", "Brand/Two"));
    let shown = [&a, &b, &c];
    // `Top` dropped between One and Two: into Brand, before Two.
    assert_eq!(drop_move(&shown, 0, 1), Some(json!({"styles": ["a"], "folder": "Brand", "before": "c"})));
    // Two dropped at the top: the folder of the style below it (none), before Top.
    assert_eq!(drop_move(&shown, 2, 0), Some(json!({"styles": ["c"], "folder": "", "before": "a"})));
    // One dropped last: Brand, at the end.
    assert_eq!(drop_move(&shown, 1, 2), Some(json!({"styles": ["b"], "folder": "Brand"})));
    assert_eq!(drop_move(&shown, 5, 0), None);
}

#[test]
fn local_styles_list_draws_by_type_and_folder() {
    let mut s = Session::new();
    for (ty, name, value) in
        [("paint", "Brand/Primary", json!(["#0D99FF"])), ("paint", "Danger", json!(["#F24822"])), ("text", "Type/Body", json!({"fontSize": 14}))]
    {
        s.execute("style.create", &json!({"type": ty, "name": name, "value": value})).unwrap();
    }
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    let mut t = 0.0;
    // Nothing selected: the Design panel shows the page, with Local styles; a folder rename in
    // progress draws its field.
    ctx.data_mut(|d| d.insert_temp(folder_rename_id(), "Brand".to_string()));
    for _ in 0..3 {
        frame(&ctx, &mut app, &mut t);
    }
    edit_local_for_test(&mut app, &ctx);
    frame(&ctx, &mut app, &mut t);
    let editing = app.ui.style_picker.as_ref().and_then(|p| p.edit.as_ref()).map(|e| e.name.clone());
    assert_eq!(editing.as_deref(), Some("Danger"), "a local style opens in the style editor");
}

fn edit_local_for_test(app: &mut FigApp, ctx: &egui::Context) {
    let mut out = ctx.run_ui(RawInput::default(), |ui| edit_local(app, ui, StyleSlot::Fill, "Danger"));
    out.textures_delta.clear();
}

/// Commits `typed` in the text style editor's field `field` ("ts_lh" / "ts_ls") with Enter and
/// returns the redefined style, if any.
fn commit_text_field(def: &TextStyle, field: &str, typed: &str) -> Option<TextStyle> {
    let ctx = egui::Context::default();
    ctx.data_mut(|d| d.insert_temp(Id::new((field, "S")).with("editing"), typed.to_string()));
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))), events: vec![enter], ..Default::default() };
    let mut f = Frame { act: None, hover: None, budget: 0 };
    let mut out = ctx.run_ui(input, |ui| text_properties(ui, "S", def, &mut f));
    out.textures_delta.clear();
    match f.act {
        Some(Act::SetText(id, d)) if id == "S" => Some(d),
        _ => None,
    }
}

#[test]
fn text_style_editor_commits_line_height_and_letter_spacing() {
    use figcraft_doc::{LetterSpacing, LineHeight};
    let def = TextStyle { line_height: LineHeight::Auto, letter_spacing: LetterSpacing::Percent(0.0), ..Default::default() };
    let lh = |def: &TextStyle, s: &str| commit_text_field(def, "ts_lh", s).map(|d| d.line_height);
    let ls = |def: &TextStyle, s: &str| commit_text_field(def, "ts_ls", s).map(|d| d.letter_spacing);
    assert_eq!(lh(&def, "150%"), Some(LineHeight::Percent(150.0)));
    assert_eq!(lh(&def, "24px"), Some(LineHeight::Pixels(24.0)));
    assert_eq!(lh(&def, "24"), Some(LineHeight::Pixels(24.0)));
    let pct = TextStyle { line_height: LineHeight::Percent(120.0), ..def.clone() };
    assert_eq!(lh(&pct, "130"), Some(LineHeight::Percent(130.0)), "a bare number keeps percent");
    assert_eq!(lh(&pct, "Auto"), Some(LineHeight::Auto));
    assert_eq!(ls(&def, "2%"), Some(LetterSpacing::Percent(2.0)));
    assert_eq!(ls(&def, "1.5px"), Some(LetterSpacing::Pixels(1.5)));
    assert_eq!(ls(&def, "3"), Some(LetterSpacing::Percent(3.0)));
    // Only the edited property changes.
    let d = commit_text_field(&def, "ts_lh", "150%").expect("redefined");
    assert_eq!(TextStyle { line_height: def.line_height, ..d }, def);
    assert_eq!(lh(&def, "abc"), None);
}
