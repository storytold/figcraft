use super::*;
use figcraft_doc::NodeKind;

use crate::present_tests::{Ui, id};

#[test]
fn device_picker_reflows_without_resizing_the_saved_frame_and_can_revert() {
    let mut session = figcraft_engine::Session::new();
    let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
    let right =
        id(&session.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 804, "y": 20, "width": 80, "height": 40})).unwrap());
    session.execute("node.set", &json!({"id": right, "props": {"constraints": {"horizontal": "right", "vertical": "top"}}})).unwrap();
    let mut ui = Ui::new(session);
    ui.run("proto.present", json!({"start": frame}));

    let mut presenter = ui.app.ui.present.take().unwrap();
    presenter.pan = Some((100.0, 200.0));
    presenter.scroll.insert(frame, (75.0, 50.0));
    choose_device(&mut ui.app, &mut presenter, "iPhone 17");
    assert!(presenter.pan.is_none() && presenter.scroll.is_empty());
    ui.app.ui.present = Some(presenter);
    ui.frame(vec![]);

    let shown = &ui.app.ui.present.as_ref().unwrap().doc.as_ref().unwrap();
    assert_eq!((shown.node(frame).unwrap().width, shown.node(frame).unwrap().height), (402.0, 874.0));
    assert_eq!(shown.world_bounds(right).unwrap().x0, 312.0, "right constraint follows the phone edge");
    assert_eq!(ui.app.inspect()["presenter"]["viewport"], json!([402.0, 874.0]));
    assert_eq!(ui.app.inspect()["presenter"]["scrollRange"], json!([0.0, 0.0]));
    assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0, "the saved design keeps its geometry");

    let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 900.0));
    let screen_before = layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen;
    ui.button(pos2(720.0, 440.0), true);
    ui.frame(vec![egui::Event::PointerMoved(pos2(300.0, 300.0))]);
    ui.button(pos2(300.0, 300.0), false);
    assert_eq!(ui.app.inspect()["presenter"]["scroll"], json!([0.0, 0.0]), "fitted content does not move");
    assert_eq!(layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen, screen_before);

    let mut presenter = ui.app.ui.present.take().unwrap();
    choose_device(&mut ui.app, &mut presenter, "None");
    ui.app.ui.present = Some(presenter);
    ui.frame(vec![]);
    assert_eq!(ui.app.inspect()["presenter"]["frameSize"], json!([894.0, 844.0]));
    assert_eq!(ui.app.inspect()["presenter"]["responsive"], Value::Null);
}

#[test]
fn device_presets_show_their_screen_at_the_real_resolution_in_their_body() {
    let mut session = figcraft_engine::Session::new();
    let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
    let mut ui = Ui::new(session);
    ui.run("proto.present", json!({"start": frame}));
    let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 860.0));

    for (name, width, height) in
        [("iPad mini", 744.0, 1133.0), ("MacBook Air", 1280.0, 832.0), ("Desktop", 1440.0, 1024.0), ("iPhone 17", 402.0, 874.0)]
    {
        let mut presenter = ui.app.ui.present.take().unwrap();
        choose_device(&mut ui.app, &mut presenter, name);
        ui.app.ui.present = Some(presenter);
        ui.frame(vec![]);

        let presenter = ui.app.ui.present.as_mut().unwrap();
        let laid = layout(presenter, area, FULL).unwrap();
        assert!(laid.color.is_some(), "{name}: drawn in its body");
        assert!((f64::from(laid.screen.width()) / laid.scale - width).abs() < 0.1, "{name}: width");
        assert!((f64::from(laid.screen.height()) / laid.scale - height).abs() < 0.1, "{name}: height");
        assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);
    }
}

/// A phone shows a frame's content past its screen only as far as the frame's overflow lets it
/// scroll: with "No scrolling" it's clipped, like on every other device.
#[test]
fn a_phone_scrolls_only_what_the_frames_overflow_allows() {
    let on_phone = |overflow: &str| {
        let mut session = figcraft_engine::Session::new();
        let frame = id(&session.execute("node.create", &json!({"type": "frame", "width": 894, "height": 844})).unwrap());
        session.execute("node.create", &json!({"type": "rectangle", "parent": frame, "x": 650, "y": 1050, "width": 80, "height": 80})).unwrap();
        session.execute("node.set", &json!({"id": frame, "props": {"overflow": overflow}})).unwrap();
        let mut ui = Ui::new(session);
        ui.run("proto.present", json!({"start": frame}));
        let mut presenter = ui.app.ui.present.take().unwrap();
        choose_device(&mut ui.app, &mut presenter, "iPhone 17");
        ui.app.ui.present = Some(presenter);
        ui.frame(vec![]);
        (ui, frame)
    };
    let start = pos2(720.0, 440.0);
    let drag = |ui: &mut Ui, to: Pos2| {
        ui.button(start, true);
        ui.frame(vec![egui::Event::PointerMoved(to)]);
        ui.button(to, false);
        ui.app.inspect()["presenter"].clone()
    };

    let (mut ui, frame) = on_phone("none");
    assert_eq!(ui.app.inspect()["presenter"]["scrollRange"], json!([0.0, 0.0]));
    assert_eq!(drag(&mut ui, start - vec2(100.0, 120.0))["scroll"], json!([0.0, 0.0]), "no scrolling stays clipped");
    assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);

    let (mut ui, frame) = on_phone("both");
    let range = &ui.app.inspect()["presenter"]["scrollRange"];
    assert!(range[0].as_f64().unwrap() > 0.0 && range[1].as_f64().unwrap() > 0.0, "{range}");
    let scroll = drag(&mut ui, start - vec2(100.0, 120.0))["scroll"].clone();
    assert!(scroll[0].as_f64().unwrap() > 0.0 && scroll[1].as_f64().unwrap() > 0.0, "{scroll}");
    assert_eq!(ui.app.session.doc().unwrap().node(frame).unwrap().width, 894.0);

    // The content keeps following a drag after the pointer leaves the screen, while the
    // device itself stays fixed on the stage.
    ui.frames(60, Vec::new);
    let area = Rect::from_min_max(pos2(0.0, 40.0), pos2(1440.0, 900.0));
    let screen_before = layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen;
    ui.button(start, true);
    ui.frame(vec![egui::Event::PointerMoved(pos2(300.0, 300.0))]);
    let during = ui.app.inspect()["presenter"].clone();
    assert!(during["scroll"][0].as_f64().unwrap() > 0.0, "{during}");
    assert_eq!(layout(ui.app.ui.present.as_mut().unwrap(), area, FULL).unwrap().screen, screen_before);
    ui.button(pos2(300.0, 300.0), false);
}

fn doc_with_two_frames() -> (Document, NodeId, NodeId) {
    let mut d = Document::new("t");
    let page = d.add_page("Page 1");
    let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 200.0, 400.0).unwrap();
    let b = d.create(page, NodeKind::Frame, "B", 300.0, 0.0, 200.0, 400.0).unwrap();
    (d, a, b)
}

#[test]
fn closed_overlays_animate_out_unless_instant() {
    let (_, a, b) = doc_with_two_frames();
    let mut p = Presenter::new(a);
    let ov = |transition, opened| OverlayState {
        id: b,
        position: OverlayPosition::Center,
        close_on_outside: true,
        background: None,
        anchor: None,
        offset: None,
        anchor_to: OverlayAnchor::Screen,
        opened,
        transition,
        dur: 0.3,
        easing: Easing::EaseOut,
    };
    p.overlays.push(ov(Transition::Dissolve, 0.0));
    assert!(p.close_overlay(10.0));
    assert!(p.overlays.is_empty());
    assert_eq!(p.closing.len(), 1);
    assert_eq!(p.closing[0].0, 10.0, "fully open: the reverse starts now");
    // Closed half way through opening: it reverses from there.
    p.overlays.push(ov(Transition::Dissolve, 20.0));
    p.close_overlay(20.15);
    assert!((p.closing[1].0 - 20.0).abs() < 1e-9);
    // Instant overlays just go.
    p.overlays.push(ov(Transition::Instant, 30.0));
    p.close_overlay(31.0);
    assert_eq!(p.closing.len(), 2);
    assert!(!p.close_overlay(32.0), "nothing left to close");
}

#[test]
fn matching_by_name_path() {
    let (mut d, a, b) = doc_with_two_frames();
    let ga = d.create(a, NodeKind::Group, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    let ra = d.create(ga, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
    let ra2 = d.create(ga, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
    let only_a = d.create(a, NodeKind::Rectangle, "Gone", 0.0, 0.0, 10.0, 10.0).unwrap();
    let gb = d.create(b, NodeKind::Group, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    let rb = d.create(gb, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
    let rb2 = d.create(gb, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
    // Same name, different parent: no match.
    let _other = d.create(b, NodeKind::Rectangle, "Box", 0.0, 0.0, 10.0, 10.0).unwrap();
    let m = match_layers(&d, a, &d, b);
    assert_eq!(m, vec![(ga, gb), (ra, rb), (ra2, rb2)]);
    assert!(!m.iter().any(|(f, _)| *f == only_a));
    assert_eq!(name_path(&d, a, ra).as_deref(), Some("Card\u{1f}Box"));
    assert_eq!(name_path(&d, b, ra), None);
}

#[test]
fn smart_interpolate_tweens_matched_and_fades_the_rest() {
    let (mut d, a, b) = doc_with_two_frames();
    let ra = d.create(a, NodeKind::Rectangle, "Box", 0.0, 0.0, 20.0, 20.0).unwrap();
    let rb = d.create(b, NodeKind::Rectangle, "Box", 100.0, 200.0, 60.0, 40.0).unwrap();
    {
        let n = d.get_mut(ra).unwrap();
        n.fills = vec![Paint::solid(Color::new(1.0, 0.0, 0.0, 1.0))];
        n.corners.radii = [0.0; 4];
        n.opacity = 1.0;
    }
    {
        let n = d.get_mut(rb).unwrap();
        n.fills = vec![Paint::solid(Color::new(0.0, 0.0, 1.0, 1.0))];
        n.corners.radii = [20.0; 4];
        n.opacity = 0.5;
        // Rotate 90° about its centre.
        let c = (130.0, 220.0);
        n.transform = Affine::translate(c) * Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::translate((-30.0, -20.0));
    }
    let only_b = d.create(b, NodeKind::Rectangle, "New", 0.0, 0.0, 5.0, 5.0);
    let only_a = d.create(a, NodeKind::Rectangle, "Old", 0.0, 0.0, 5.0, 5.0).unwrap();
    let mid = smart_interpolate(&d, a, &d, b, 0.5);
    let n = mid.node(rb).unwrap();
    assert!((n.width - 40.0).abs() < 1e-9 && (n.height - 30.0).abs() < 1e-9);
    assert!((n.opacity - 0.75).abs() < 1e-6);
    assert!((n.corners.radii[0] - 10.0).abs() < 1e-9);
    let c = n.fills[0].solid_color().unwrap();
    assert!((c.r - 0.5).abs() < 0.02 && (c.b - 0.5).abs() < 0.02, "{c:?}");
    // Centre halfway between (10, 10) and (130, 220) in frame space; rotated 45°.
    let ctr = mid.world_transform(b).inverse() * mid.world_transform(rb) * Point::new(n.width / 2.0, n.height / 2.0);
    assert!((ctr.x - 70.0).abs() < 1e-6 && (ctr.y - 115.0).abs() < 1e-6, "{ctr:?}");
    let [aa, bb, ..] = n.transform.as_coeffs();
    assert!((bb.atan2(aa) - std::f64::consts::FRAC_PI_4).abs() < 1e-6);
    if let Ok(ob) = only_b {
        assert!((mid.node(ob).unwrap().opacity - 0.5).abs() < 1e-6);
    }
    // The source-only layer is copied into the destination frame, half faded.
    let copies: Vec<_> = mid.node(b).unwrap().children.iter().filter_map(|c| mid.node(*c)).filter(|n| n.name == "Old").collect();
    assert_eq!(copies.len(), 1);
    assert!((copies[0].opacity - 0.5).abs() < 1e-6);
    assert_ne!(copies[0].id, only_a);
    // At the end it is exactly the destination.
    let end = smart_interpolate(&d, a, &d, b, 1.0);
    assert_eq!(end.node(rb), d.node(rb));
    assert_eq!(end.node(b).unwrap().children, d.node(b).unwrap().children);
    // At the start the matched layer sits where the source was.
    let start = smart_interpolate(&d, a, &d, b, 0.0);
    let s = start.world_bounds(rb).unwrap();
    let f = start.world_bounds(b).unwrap();
    assert!((s.x0 - f.x0).abs() < 1e-6 && (s.y0 - f.y0).abs() < 1e-6 && (s.width() - 20.0).abs() < 1e-6);
}

#[test]
fn scroll_offsets_fixed_and_sticky() {
    let (mut d, a, _) = doc_with_two_frames();
    d.get_mut(a).unwrap().overflow = Overflow::Vertical;
    let long = d.create(a, NodeKind::Rectangle, "Long", 0.0, 0.0, 200.0, 1000.0).unwrap();
    let fixed = d.create(a, NodeKind::Rectangle, "Bar", 0.0, 350.0, 200.0, 50.0).unwrap();
    let sticky = d.create(a, NodeKind::Rectangle, "Header", 0.0, 100.0, 200.0, 40.0).unwrap();
    d.get_mut(fixed).unwrap().scroll_position = ScrollPosition::Fixed;
    d.get_mut(sticky).unwrap().scroll_position = ScrollPosition::Sticky;
    assert_eq!(scroll_range(&d, a), (0.0, 600.0));
    d.get_mut(a).unwrap().overflow = Overflow::Horizontal;
    assert_eq!(scroll_range(&d, a), (0.0, 0.0));
    d.get_mut(a).unwrap().overflow = Overflow::Vertical;
    let mut s = HashMap::new();
    s.insert(a, (0.0, 50.0));
    let out = apply_scroll(&d, &s);
    let y = |doc: &Document, id| doc.world_bounds(id).unwrap().y0;
    assert_eq!(y(&out, long), -50.0);
    assert_eq!(y(&out, fixed), 350.0);
    assert_eq!(y(&out, sticky), 50.0);
    s.insert(a, (0.0, 300.0));
    let out = apply_scroll(&d, &s);
    assert_eq!(y(&out, long), -300.0);
    assert_eq!(y(&out, sticky), 0.0);
}

/// A page presented on an iPhone 16 (393×852): a tall screen A that scrolls vertically, with a
/// tab bar fixed to its bottom, a nested scrolling list and a layer at its end; screen B.
fn device_doc() -> (Document, NodeId, NodeId, NodeId, NodeId, NodeId) {
    let mut d = Document::new("t");
    let page = d.add_page("Page 1");
    if let NodeKind::Page { prototype, .. } = &mut d.get_mut(page).unwrap().kind {
        prototype.device = "iPhone 16".into();
    }
    let a = d.create(page, NodeKind::Frame, "A", 0.0, 0.0, 393.0, 2000.0).unwrap();
    let b = d.create(page, NodeKind::Frame, "B", 500.0, 0.0, 393.0, 1500.0).unwrap();
    for f in [a, b] {
        d.get_mut(f).unwrap().overflow = Overflow::Vertical;
    }
    let bar = d.create(a, NodeKind::Rectangle, "Tab bar", 0.0, 1940.0, 393.0, 60.0).unwrap();
    let n = d.get_mut(bar).unwrap();
    n.scroll_position = ScrollPosition::Fixed;
    n.constraints.vertical = VConstraint::Bottom;
    let list = d.create(a, NodeKind::Frame, "List", 0.0, 1200.0, 393.0, 300.0).unwrap();
    d.get_mut(list).unwrap().overflow = Overflow::Vertical;
    let item = d.create(list, NodeKind::Rectangle, "Item", 0.0, 700.0, 393.0, 100.0).unwrap();
    (d, a, b, bar, list, item)
}

fn player_on(d: Document, start: NodeId) -> Presenter {
    let mut p = Presenter::new(start);
    p.sync(&Arc::new(d), 0.0);
    p
}

#[test]
fn device_screen_clips_the_frame_and_it_scrolls_through_its_height() {
    let (mut d, a, _, bar, list, _) = device_doc();
    assert_eq!(viewport(&d, a), Some((393.0, 852.0)));
    // On the device the whole frame scrolls; at its own size nothing overflows it.
    assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (0.0, 1148.0));
    assert_eq!(scroll_range(&d, a), (0.0, 0.0));
    // Nested frames scroll within themselves.
    assert_eq!(scroll_range(&d, list), (0.0, 500.0));
    // The display copy is sized to the screen; the fixed tab bar moves to the screen's bottom.
    let mut p = player_on(d.clone(), a);
    let disp = p.display().unwrap();
    assert_eq!((disp.node(a).unwrap().width, disp.node(a).unwrap().height), (393.0, 852.0));
    assert_eq!(disp.world_bounds(bar).unwrap().y0, 792.0);
    // Scrolled, it stays there while the content moves.
    p.scroll_to(0.0, 400.0);
    let disp = p.display().unwrap();
    assert_eq!(disp.world_bounds(bar).unwrap().y0, 792.0);
    assert_eq!(disp.world_bounds(list).unwrap().y0, 800.0);
    p.scroll_to(0.0, 99_999.0);
    assert_eq!(p.scroll[&a], (0.0, 1148.0), "clamped to the range");
    // A top-level frame taller than the screen scrolls without an overflow setting (its own
    // range still clips nested content); without a device the frame shows whole.
    d.get_mut(a).unwrap().overflow = Overflow::None;
    assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (0.0, 1148.0));
    assert_eq!(scroll_range(&d, a), (0.0, 0.0));
    if let Some(pg) = d.page_of(a)
        && let NodeKind::Page { prototype, .. } = &mut d.get_mut(pg).unwrap().kind
    {
        prototype.device = "None".into();
    }
    assert_eq!(viewport(&d, a), None);
    // A frame wider than the screen scrolls sideways when it scrolls both ways.
    let (mut d, a, ..) = device_doc();
    let n = d.get_mut(a).unwrap();
    n.width = 800.0;
    n.overflow = Overflow::Both;
    assert_eq!(scroll_range_in(&d, a, viewport(&d, a)), (407.0, 1148.0));
}

#[test]
fn scroll_to_moves_every_scrolling_frame_around_the_layer() {
    let (d, a, _, _, list, item) = device_doc();
    let mut p = player_on(d.clone(), a);
    assert!(p.perform(&Action::ScrollTo { dest: item, offset: [0.0, 0.0] }, item, Transition::Instant, 0.0, Easing::Linear, None, 1.0));
    // The list brings the item to its top (as far as it scrolls), the screen brings the list.
    assert_eq!(p.scroll[&list], (0.0, 500.0));
    assert_eq!(p.scroll[&a], (0.0, 1148.0));
    // Animated, it gets there over the duration.
    let mut p = player_on(d, a);
    assert!(p.perform(&Action::ScrollTo { dest: list, offset: [0.0, 0.0] }, list, Transition::Dissolve, 0.3, Easing::Linear, None, 1.0));
    let anim = p.scroll_anim.as_ref().unwrap();
    assert_eq!(anim.frames, vec![(a, (0.0, 0.0), (0.0, 1148.0))]);
}

#[test]
fn navigating_resets_scroll_unless_it_preserves_it() {
    let (d, a, b, ..) = device_doc();
    let mut p = player_on(d, a);
    p.scroll_to(0.0, 600.0);
    assert!(p.perform(&Action::Navigate { dest: b, preserve_scroll: false }, a, Transition::Instant, 0.0, Easing::Linear, None, 1.0));
    assert_eq!(p.scroll.get(&b), None, "starts at the top");
    p.restart(a, 2.0);
    p.scroll_to(0.0, 1000.0);
    assert!(p.perform(&Action::Navigate { dest: b, preserve_scroll: true }, a, Transition::Instant, 0.0, Easing::Linear, None, 3.0));
    assert_eq!(p.scroll[&b], (0.0, 648.0), "kept, as far as B scrolls");
    // Back starts at the top again.
    assert!(p.perform(&Action::Back, b, Transition::Instant, 0.0, Easing::Linear, None, 4.0));
    assert_eq!((p.current, p.scroll.get(&a)), (a, None));
}

#[test]
fn sync_follows_edits_and_keeps_the_screen() {
    let (mut d, a, b, _, list, _) = device_doc();
    let mut p = player_on(d.clone(), a);
    p.go(b, Transition::Instant, 0.0, Easing::Linear, 1.0, true, false);
    p.scroll_to(0.0, 600.0);
    // The same document again changes nothing.
    let same = Arc::new(d.clone());
    p.sync(&same, 2.0);
    let rev = p.rev;
    p.sync(&same, 2.0);
    assert_eq!(p.rev, rev);
    // An edit is copied; the screen and its scroll stay (clamped to the new range).
    d.get_mut(b).unwrap().height = 1000.0;
    d.get_mut(b).unwrap().name = "B2".into();
    p.sync(&Arc::new(d.clone()), 3.0);
    assert_eq!(p.current, b);
    assert_eq!(p.inspect()["name"], "B2");
    assert_eq!(p.scroll[&b], (0.0, 148.0));
    assert_eq!(p.history.len(), 1);
    // The screen deleted: back to the start.
    d.remove(b).unwrap();
    p.sync(&Arc::new(d.clone()), 4.0);
    assert_eq!(p.current, a);
    assert!(p.history.is_empty() && p.scroll.is_empty());
    // Junk scroll offsets never panic and stay in range.
    p.scroll.insert(list, (f64::NAN, f64::INFINITY));
    p.sync(&Arc::new(d), 5.0);
    assert_eq!(p.scroll[&list], (0.0, 0.0));
}

#[test]
fn transitions_move_the_right_way() {
    let left = transition_frames(Transition::MoveIn { direction: Direction::Left }, 0.0);
    assert_eq!(left.to_off, (1.0, 0.0));
    assert_eq!(transition_frames(Transition::MoveIn { direction: Direction::Left }, 1.0).to_off, (0.0, 0.0));
    let push = transition_frames(Transition::Push { direction: Direction::Top }, 0.5);
    assert_eq!(push.from_off, (0.0, -0.5));
    assert_eq!(push.to_off, (0.0, 0.5));
    let out = transition_frames(Transition::MoveOut { direction: Direction::Right }, 0.25);
    assert!(out.from_on_top);
    assert_eq!(out.from_off, (0.25, 0.0));
    assert_eq!(transition_frames(Transition::Dissolve, 0.3).to_alpha, 0.3);
    assert_eq!(transition_frames(Transition::Instant, 0.0).from_alpha, 0.0);
    let r = Transition::SlideIn { direction: Direction::Bottom }.reversed();
    assert_eq!(r, Transition::SlideOut { direction: Direction::Top });
}

#[test]
fn overlay_positions() {
    let base = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 800.0));
    let size = vec2(200.0, 100.0);
    assert_eq!(overlay_rect(OverlayPosition::Center, base, size, None, None).min, pos2(100.0, 350.0));
    assert_eq!(overlay_rect(OverlayPosition::BottomCenter, base, size, None, None).min, pos2(100.0, 700.0));
    assert_eq!(overlay_rect(OverlayPosition::TopRight, base, size, None, None).min, pos2(200.0, 0.0));
    let anchor = Rect::from_min_size(pos2(30.0, 40.0), vec2(50.0, 20.0));
    assert_eq!(overlay_rect(OverlayPosition::Manual, base, size, Some(anchor), None).min, pos2(30.0, 60.0));
}

/// Smart animate renders a temporary document per animation frame; keep that cheap.
#[test]
fn smart_animate_frame_cost() {
    let (mut d, a, b) = doc_with_two_frames();
    for (f, dx) in [(a, 0.0), (b, 40.0)] {
        for i in 0..40 {
            let r = d.create(f, NodeKind::Rectangle, &format!("Row {i}"), dx + 10.0, 10.0 + f64::from(i) * 9.0, 120.0, 8.0).unwrap();
            d.get_mut(r).unwrap().corners.radii = [4.0; 4];
        }
    }
    let t0 = std::time::Instant::now();
    let n = 10;
    for k in 0..n {
        let tmp = smart_interpolate(&d, a, &d, b, f64::from(k) / f64::from(n));
        assert!(figcraft_render::render_node(&tmp, b, 2.0).is_some());
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    eprintln!("smart animate: {per:.2} ms per frame (80 layers, 400x800 px)");
}
