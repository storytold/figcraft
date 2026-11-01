//! Real field and canvas gestures for #402; distances are checked in the engine document.
use crate::layers_drag_tests::Ui;
use egui::{Event, Id, Key, Modifiers, Pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

fn scene(grid: bool) -> (Ui, Vec<NodeId>) {
    let mut s = Session::new();
    let mut ids = vec![];
    for row in 0..if grid { 2 } else { 1 } {
        for col in 0..3 {
            let id = NodeId(
                s.execute("shape.rectangle", &json!({"x":100+col*80,"y":100+row*100,"width":50,"height":60})).unwrap()["id"].as_u64().unwrap(),
            );
            ids.push(id);
        }
    }
    s.execute("select.set", &json!({"ids":ids})).unwrap();
    let u = Ui::new(s);
    (u, ids)
}
fn xy(u: &Ui, id: NodeId) -> (f64, f64) {
    let b = u.app.session.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0)
}
fn key(u: &mut Ui, k: Key) {
    u.frame(vec![
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
    u.frame(vec![]);
}
fn drag(u: &mut Ui, p: Pos2, by: egui::Vec2, release: bool) {
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    for i in 1..=4 {
        u.frame(vec![Event::PointerMoved(p + by * i as f32 / 4.)]);
    }
    if release {
        u.button(p + by, false);
        u.frame(vec![]);
    }
}

/// Asserts typed and scrubbed spacing changes take effect with exactly one undo step.
#[test]
fn spacing_fields_type_scrub_and_keep_one_undo_step() {
    let (mut u, ids) = scene(true);
    let field = Id::unique(("selection_spacing", "horizontal"));
    let p = u.ctx.read_response(field.with("field")).unwrap().rect.center();
    let history = u.app.session.undo_depth().0;
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
    u.frame(vec![Event::Text("45.5".into())]);
    key(&mut u, Key::Enter);
    assert_eq!(xy(&u, ids[1]), (195.5, 100.));
    assert_eq!(xy(&u, ids[4]), (195.5, 200.));
    assert_eq!(u.app.session.undo_depth().0, history + 1);
    u.app.run("edit.undo", json!({}));
    u.frame(vec![]);
    let lead = u.ctx.read_response(field.with("lead")).unwrap().rect.center();
    drag(&mut u, lead, vec2(16., 0.), true);
    assert_eq!(xy(&u, ids[1]), (196., 100.));
    assert_eq!(u.app.session.undo_depth().0, history + 1);
    u.app.run("edit.undo", json!({}));
    assert_eq!(xy(&u, ids[1]), (180., 100.));
}

#[test]
fn canvas_gap_drag_keeps_selection_and_undo_redo_for_both_grid_axes() {
    for (vertical, point) in [(false, (165., 130.)), (true, (125., 180.))] {
        let (mut u, ids) = scene(true);
        let before = u.app.session.undo_depth().0;
        let p = u.app.to_screen(point.0, point.1);
        let output = u.frame_output(vec![Event::PointerMoved(p)]);
        assert_eq!(output.platform_output.cursor_icon, if vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal });
        let z = u.app.session.ds().unwrap().view.zoom as f32;
        drag(&mut u, p, if vertical { vec2(0., 20. * z) } else { vec2(20. * z, 0.) }, true);
        assert_eq!(u.app.session.selection(), ids);
        assert_eq!(xy(&u, ids[1]), if vertical { (180., 100.) } else { (200., 100.) });
        assert_eq!(xy(&u, ids[3]), if vertical { (100., 220.) } else { (100., 200.) });
        assert_eq!(u.app.session.undo_depth().0, before + 1);
        u.app.run("edit.undo", json!({}));
        assert_eq!(xy(&u, ids[4]), (180., 200.));
        u.app.run("edit.redo", json!({}));
        assert_eq!(xy(&u, ids[4]), if vertical { (180., 220.) } else { (200., 200.) });
    }
}

/// Asserts Escape rolls a spacing drag back, and releasing outside the window commits once
/// while keeping the selection.
#[test]
fn escape_rolls_back_and_an_outside_release_finishes_the_gesture() {
    let (mut u, ids) = scene(false);
    let before = u.app.session.undo_depth().0;
    // A canvas handle takes the keyboard from a previously focused panel field.
    let field = u.ctx.read_response(Id::unique(("selection_spacing", "horizontal")).with("field")).unwrap().rect.center();
    u.button(field, true);
    u.button(field, false);
    u.frame(vec![]);
    let p = u.app.to_screen(165., 130.);
    let z = u.app.session.ds().unwrap().view.zoom as f32;
    drag(&mut u, p, vec2(20. * z, 0.), false);
    assert_eq!(xy(&u, ids[1]), (200., 100.));
    key(&mut u, Key::Escape);
    u.button(p + vec2(20. * z, 0.), false);
    u.frame(vec![]);
    assert_eq!(xy(&u, ids[1]), (180., 100.));
    assert_eq!(u.app.session.undo_depth().0, before);
    assert_eq!(u.app.session.selection(), ids);
    drag(&mut u, p, vec2(20. * z, 0.), false);
    // Leave the window while held; releasing with no pointer position still commits once.
    u.frame(vec![Event::PointerGone]);
    u.frame(vec![Event::PointerButton {
        pos: Pos2::new(-1., -1.),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    u.frame(vec![]);
    assert_eq!(u.app.session.undo_depth().0, before + 1);
    assert!(u.app.session.ds().unwrap().interaction.is_none());
}

#[test]
fn switching_documents_cancels_only_the_spacing_documents_gesture() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(165., 130.);
    let z = u.app.session.ds().unwrap().view.zoom as f32;
    drag(&mut u, p, vec2(20. * z, 0.), false);
    u.app.run("file.new", json!({}));
    u.app.session.begin_interaction("Unrelated").unwrap();
    u.frame(vec![]);
    assert!(u.app.session.ds().unwrap().interaction.is_some());
    u.app.session.active = 0;
    assert_eq!(xy(&u, ids[1]), (180., 100.));
    assert!(u.app.session.ds().unwrap().interaction.is_none());
}

#[test]
fn handle_click_is_a_noop_and_modifiers_can_be_held_before_pressing() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(165., 130.);
    let before = u.app.session.undo_depth().0;
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.undo_depth().0, before);
    for (modifiers, delta, gap) in [(Modifiers::SHIFT, 17., 50.), (Modifiers::ALT, 17.25, 47.3)] {
        let (mut u, ids) = scene(false);
        let p = u.app.to_screen(165., 130.);
        let z = u.app.session.ds().unwrap().view.zoom as f32;
        u.frame_with(vec![Event::PointerMoved(p)], modifiers);
        u.frame_with(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers }], modifiers);
        for k in 1..=4 {
            u.frame_with(vec![Event::PointerMoved(p + vec2(delta * z * k as f32 / 4., 0.))], modifiers);
        }
        u.frame_with(
            vec![Event::PointerButton { pos: p + vec2(delta * z, 0.), button: egui::PointerButton::Primary, pressed: false, modifiers }],
            modifiers,
        );
        u.frame(vec![]);
        assert!((xy(&u, ids[1]).0 - 150. - gap).abs() < 1e-6);
    }
    assert_eq!(xy(&u, ids[1]), (180., 100.));
}

#[test]
fn undo_while_dragging_cancels_the_gesture_without_undoing_an_earlier_edit() {
    let (mut u, ids) = scene(false);
    let before = u.app.session.undo_depth().0;
    let p = u.app.to_screen(165., 130.);
    let z = u.app.session.ds().unwrap().view.zoom as f32;
    drag(&mut u, p, vec2(20. * z, 0.), false);
    let m = Modifiers::COMMAND;
    u.frame_with(
        vec![
            Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: m },
            Event::Key { key: Key::Z, physical_key: None, pressed: false, repeat: false, modifiers: m },
        ],
        m,
    );
    u.button(p + vec2(20. * z, 0.), false);
    u.frame(vec![]);
    assert_eq!(u.app.session.undo_depth().0, before);
    assert_eq!(xy(&u, ids[1]), (180., 100.));
    assert_eq!(u.app.session.selection(), ids);
}

#[test]
fn stationary_gap_drag_does_not_change_the_document_again() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(165., 130.);
    let z = u.app.session.ds().unwrap().view.zoom as f32;
    drag(&mut u, p, vec2(20. * z, 0.), false);
    assert_eq!(xy(&u, ids[1]), (200., 100.));
    let revision = u.app.session.ds().unwrap().revision;
    let doc = u.app.session.ds().unwrap().doc.clone();
    for _ in 0..5 {
        u.frame(vec![]);
        u.frame(vec![Event::PointerMoved(p + vec2(20.2 * z, 0.))]);
    }
    assert_eq!(u.app.session.ds().unwrap().revision, revision);
    assert!(std::sync::Arc::ptr_eq(&doc, &u.app.session.ds().unwrap().doc));
    u.button(p + vec2(20.2 * z, 0.), false);
    assert_eq!(xy(&u, ids[1]), (200., 100.));
}

#[test]
fn reorder_ring_drag_inserts_across_grid_rows_as_one_undo_step() {
    let (mut u, ids) = scene(true);
    let before = u.app.session.undo_depth().0;
    let p = u.app.to_screen(125., 130.);
    let target = u.app.to_screen(205., 230.);
    let output = u.frame_output(vec![Event::PointerMoved(p)]);
    assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Grab);
    drag(&mut u, p, target - p, true);
    assert_eq!(xy(&u, ids[0]), (180., 200.));
    assert_eq!(xy(&u, ids[3]), (260., 100.));
    assert_eq!(u.app.session.selection(), ids);
    assert_eq!(u.app.session.undo_depth().0, before + 1);
    u.app.run("edit.undo", json!({}));
    assert_eq!(xy(&u, ids[0]), (100., 100.));
    u.app.run("edit.redo", json!({}));
    assert_eq!(xy(&u, ids[0]), (180., 200.));
}

#[test]
fn a_click_on_one_of_several_selected_layers_selects_it() {
    let (mut u, ids) = scene(false);
    // The reorder ring sits at the layer's centre and takes the press; a click (no drag) on it is
    // a click on the layer, which then has the selection to itself.
    let p = u.app.to_screen(125., 130.);
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), vec![ids[0]], "the click selects the layer whose ring it hit");
}

#[test]
fn the_press_modifiers_decide_whether_a_ring_release_is_a_click() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(125., 130.);
    let shift = Modifiers::SHIFT;
    // Shift is down at the press and let go before the release: the ring keeps its Shift meaning.
    u.frame_with(vec![Event::PointerMoved(p)], shift);
    u.frame_with(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: shift }], shift);
    u.frame(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), ids, "letting Shift go before the release doesn't make it a plain click");
}

#[test]
fn releasing_a_ring_outside_the_canvas_ends_a_drag_even_without_motion() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(125., 130.);
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    // The pointer leaves before any in-canvas motion is seen, so `moved` stays false.
    u.frame(vec![Event::PointerGone]);
    u.button(Pos2::new(-1., -1.), false);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), ids, "a release away from the press ends a drag, not a click");
}

#[test]
fn reorder_ring_escape_click_and_returning_home_make_no_undo_step() {
    for action in [0, 1, 2] {
        let (mut u, ids) = scene(false);
        let history = u.app.session.undo_depth().0;
        let before = u.app.session.doc().unwrap().clone();
        let p = u.app.to_screen(125., 130.);
        let target = u.app.to_screen(285., 130.);
        u.frame(vec![Event::PointerMoved(p)]);
        if action == 0 {
            u.button(p, true);
        } else {
            drag(&mut u, p, target - p, false);
            assert_eq!(xy(&u, ids[0]), (260., 100.));
            if action == 1 {
                key(&mut u, Key::Escape);
            } else {
                u.frame(vec![Event::PointerMoved(p)]);
            }
        }
        u.button(p, false);
        u.frame(vec![]);
        assert_eq!(u.app.session.doc().unwrap(), &before);
        assert_eq!(u.app.session.undo_depth().0, history);
        if action == 0 {
            // The press on a ring that doesn't drag is a click on that layer.
            assert_eq!(u.app.session.selection(), vec![ids[0]], "a click on a ring selects its layer");
        } else {
            assert_eq!(u.app.session.selection(), ids);
        }
        assert!(u.app.session.ds().unwrap().interaction.is_none());
    }
}

#[test]
fn command_drag_swaps_and_a_stationary_pointer_keeps_the_preview() {
    let (mut u, ids) = scene(true);
    let p = u.app.to_screen(125., 130.);
    let target = u.app.to_screen(205., 230.);
    // The modifier can already be down when the handle is pressed.
    let m = Modifiers::COMMAND;
    u.frame_with(vec![Event::PointerMoved(p)], m);
    u.frame_with(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: m }], m);
    u.frame_with(vec![Event::PointerMoved(target)], m);
    assert_eq!(xy(&u, ids[0]), (180., 200.));
    assert_eq!(xy(&u, ids[4]), (100., 100.));
    assert_eq!(xy(&u, ids[1]), (180., 100.));
    let snapshot = u.app.session.ds().unwrap().doc.clone();
    for _ in 0..5 {
        u.frame_with(vec![], m);
    }
    assert!(std::sync::Arc::ptr_eq(&snapshot, &u.app.session.ds().unwrap().doc));
    // Changing modifiers without moving the pointer switches to insertion.
    u.frame(vec![]);
    assert_eq!(xy(&u, ids[1]), (100., 100.));
    assert_eq!(xy(&u, ids[4]), (100., 200.));
    u.button(target, false);
}

#[test]
fn switching_documents_during_reordering_rolls_back_its_owner() {
    let (mut u, ids) = scene(false);
    let before = u.app.session.doc().unwrap().clone();
    let p = u.app.to_screen(125., 130.);
    let target = u.app.to_screen(285., 130.);
    drag(&mut u, p, target - p, false);
    u.app.run("file.new", json!({}));
    u.app.session.begin_interaction("Unrelated").unwrap();
    u.frame(vec![]);
    assert!(u.app.session.ds().unwrap().interaction.is_some());
    u.app.session.active = 0;
    assert_eq!(u.app.session.doc().unwrap(), &before);
    assert_eq!(u.app.session.selection(), ids);
}

#[test]
fn reorder_release_outside_window_commits_and_undo_while_dragging_cancels() {
    for undo in [false, true] {
        let (mut u, ids) = scene(false);
        let history = u.app.session.undo_depth().0;
        let p = u.app.to_screen(125., 130.);
        let target = u.app.to_screen(285., 130.);
        drag(&mut u, p, target - p, false);
        if undo {
            let m = Modifiers::COMMAND;
            u.frame_with(vec![Event::Key { key: Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: m }], m);
        }
        u.frame(vec![Event::PointerGone]);
        u.button(Pos2::new(-1., -1.), false);
        u.frame(vec![]);
        assert_eq!(u.app.session.undo_depth().0, history + usize::from(!undo));
        assert_eq!(xy(&u, ids[0]), if undo { (100., 100.) } else { (260., 100.) });
        assert!(u.app.session.ds().unwrap().interaction.is_none());
    }
}

#[test]
fn an_externally_finished_reorder_cannot_resume_on_later_pointer_motion() {
    let (mut u, ids) = scene(false);
    let p = u.app.to_screen(125., 130.);
    let target = u.app.to_screen(285., 130.);
    drag(&mut u, p, target - p, false);
    u.app.session.commit_interaction().unwrap();
    let history = u.app.session.undo_depth().0;
    let committed = u.app.session.doc().unwrap().clone();
    let middle = u.app.to_screen(205., 130.);
    u.frame(vec![Event::PointerMoved(middle)]);
    u.button(middle, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap(), &committed);
    assert_eq!(u.app.session.undo_depth().0, history);
    assert_eq!(xy(&u, ids[0]), (260., 100.));
}

#[test]
fn small_layers_on_screen_have_no_reorder_ring_and_drag_the_selection_from_their_middle() {
    let mut u = Ui::new(Session::new());
    let z = u.app.session.ds().unwrap().view.zoom;
    // 12 px squares 36 px apart on screen: a regular row, too small for rings.
    let ids: Vec<NodeId> = (0..3)
        .map(|col| {
            let p = json!({"x": (100. + f64::from(col) * 36.) / z, "y": 100. / z, "width": 12. / z, "height": 12. / z});
            NodeId(u.app.session.execute("shape.rectangle", &p).unwrap()["id"].as_u64().unwrap())
        })
        .collect();
    u.app.session.execute("select.set", &json!({"ids": ids})).unwrap();
    let before: Vec<_> = ids.iter().map(|id| xy(&u, *id)).collect();
    let p = u.app.to_screen(before[0].0 + 6. / z, before[0].1 + 6. / z);
    let output = u.frame_output(vec![Event::PointerMoved(p)]);
    assert_ne!(output.platform_output.cursor_icon, egui::CursorIcon::Grab);
    drag(&mut u, p, vec2(0., 60.), true);
    let after: Vec<_> = ids.iter().map(|id| xy(&u, *id)).collect();
    assert!(after[0].1 > before[0].1, "the selection moved: {before:?} -> {after:?}");
    assert!(
        before.iter().zip(&after).all(|(b, a)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1 - (after[0].1 - before[0].1)).abs() < 1e-6),
        "all together, in order"
    );
    assert_eq!(u.app.session.selection(), ids);
}
