use super::layout::{Chrome, FULL, Place};
use super::player::player;
use super::scroll::page_settings;
use super::{
    Align2, Color32, CornerRadius, Document, Easing, FigApp, HashSet, Icon, Id, NodeId, Pos2, PresentScale, Presenter, PrototypeSettings, Rect,
    Sense, Stroke, Tokens, Transition, Ui, icons, json, pos2, regular, tr, vec2, widgets,
};

/// What the player's top bar asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarAction {
    Close,
    Restart,
    /// Present full-window (from the pane).
    Present,
    /// The full-window presenter's options menu (handled in the bar).
    Options,
    /// Turn responsive presentation on or off (saved in the file: `proto.settings`).
    Responsive(bool),
    /// Show or hide the comments on the presented frame.
    Comments,
    /// Show or hide the Inspect panel.
    Inspect,
}

pub struct PresentTab {
    pub id: u64,
    /// Empty while the tab is on screen: then the player is in `app.ui.present`.
    pub pres: Option<Presenter>,
    pub doc_uid: u64,
    pub path: Option<String>,
    pub page: NodeId,
    pub file: String,
    pub page_name: String,
}

/// The full-window presenter (while `app.ui.present` is set). Escape or Close ends it.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let Some(mut pres) = app.ui.present.take() else { return };
    // A tab can play a file that is closed or not in front. Then it must not read or change
    // the file in front.
    let live = crate::tabs::present_owner_active(app);
    if live && let Ok(ds) = app.session.ds() {
        pres.sync(&ds.doc, now);
    }
    poll_pads(app, &mut pres, &ctx);
    let mut action = None;
    // Typing a reply to a comment keeps the keys from the prototype, and so does the link notice
    // while it shows; Shift+C shows the comments.
    let typing = ctx.egui_wants_keyboard_input() || pres.link_notice.is_some();
    if !typing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::C)) {
        action = Some(BarAction::Comments);
    }
    if pres.view.inspect {
        let t = Tokens::get(&ctx);
        egui::Panel::right("present_inspect")
            .resizable(false)
            .exact_size(INSPECT_W)
            .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| inspect_panel(app, ui, &pres));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
        action = player(&mut pres, ui, ui.max_rect(), now, Chrome { keys: !typing, ..FULL }).or(action);
    });
    let picked_device = navigation_bar(&mut pres, &ctx, now);
    // Keys the prototype left alone: C shows the comments, F fills the screen.
    if !typing {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::C)) {
            action = Some(BarAction::Comments);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
    }
    match action {
        Some(BarAction::Comments) => pres.view.comments = !pres.view.comments,
        Some(BarAction::Inspect) => pres.view.inspect = !pres.view.inspect,
        _ => {}
    }
    if live
        && pres.view.comments
        && let Some((frame, place, clip)) = pres.shown
    {
        crate::comments::presenter(app, &ctx, frame, place, clip);
    } else if app.ui.comments.open.is_some() {
        app.ui.comments.close();
    }
    finish(app, &mut pres);
    link_notice(app, &mut pres, &ctx, Id::unique("present_link_notice"));
    // The options carry over to the next presentation.
    app.ui.present_view = pres.view;
    if live && let Some(BarAction::Responsive(on)) = action {
        app.run("proto.settings", json!({"responsive": on}));
    }
    if live && let Some(device) = picked_device {
        choose_device(app, &mut pres, &device);
    }
    app.ui.present = Some(pres);
    if action == Some(BarAction::Close) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        if let Some(id) = app.ui.present_active {
            crate::tabs::close_present(app, id);
        } else {
            app.ui.present = None;
        }
    }
}

/// Device choices in the presenter use the screen's real dimensions and reflow a copy of the
/// design through constraints and auto layout. "No device" returns to the original frame size.
pub(super) fn choose_device(app: &mut FigApp, p: &mut Presenter, device: &str) {
    let responsive = device != "None";
    if app.run("proto.settings", json!({"device": device, "responsive": responsive})).is_some() {
        p.reset_for_device();
    }
}

/// Controls that stay in reach while viewing a prototype, including screen stepping, device and
/// scale. The selected device is returned to the caller so it can go through the normal document
/// command (and undo/save) rather than changing only the player's copy.
fn navigation_bar(p: &mut Presenter, ctx: &egui::Context, now: f64) -> Option<String> {
    let t = Tokens::get(ctx);
    let doc = p.doc.as_ref()?;
    let frames = doc.top_frames(doc.page_of(p.current).unwrap_or(p.current));
    let index = frames.iter().position(|id| *id == p.current);
    let settings = page_settings(doc, p.current);
    let mut navigate = None;
    let mut device_choice = None;
    egui::Area::new(Id::unique("present_navigation")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -18.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(9))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let prev = index.and_then(|i| i.checked_sub(1)).and_then(|i| frames.get(i)).copied();
                        let next = index.and_then(|i| frames.get(i + 1)).copied();
                        let previous =
                            ui.add_enabled(prev.is_some(), egui::Button::new("").min_size(vec2(26.0, 24.0))).on_hover_text(tr("Previous frame"));
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(previous.rect.center(), vec2(21.0, 21.0)),
                            Icon::ChevronLeft,
                            if prev.is_some() { t.icon } else { t.icon_secondary },
                        );
                        if previous.clicked() {
                            navigate = prev;
                        }
                        ui.label(egui::RichText::new(format!("{} / {}", index.map_or(0, |i| i + 1), frames.len())).color(t.text));
                        let following =
                            ui.add_enabled(next.is_some(), egui::Button::new("").min_size(vec2(26.0, 24.0))).on_hover_text(tr("Next frame"));
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(following.rect.center(), vec2(21.0, 21.0)),
                            Icon::ChevronRight,
                            if next.is_some() { t.icon } else { t.icon_secondary },
                        );
                        if following.clicked() {
                            navigate = next;
                        }
                        ui.separator();
                        let model = if settings.device.is_empty() || settings.device == "None" { tr("No device") } else { &settings.device };
                        let device_menu = ui.menu_button(format!("{model}    "), |ui| {
                            navigation_menu(ui, |ui| {
                                egui::ScrollArea::vertical().max_height(410.0).show(ui, |ui| {
                                    if ui.selectable_label(settings.device.is_empty() || settings.device == "None", tr("No device")).clicked() {
                                        device_choice = Some("None".to_owned());
                                        ui.close();
                                    }
                                    ui.separator();
                                    for device in PrototypeSettings::DEVICES {
                                        let label = format!("{}  {}×{}", device.name, device.width as u32, device.height as u32);
                                        if ui.selectable_label(settings.device == device.name, label).clicked() {
                                            device_choice = Some(device.name.to_owned());
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    if ui.selectable_label(settings.device == PrototypeSettings::CUSTOM_DEVICE, tr("Custom size")).clicked() {
                                        device_choice = Some(PrototypeSettings::CUSTOM_DEVICE.to_owned());
                                        ui.close();
                                    }
                                });
                            });
                        });
                        paint_menu_chevron(ui.painter(), device_menu.response.rect, t.icon_secondary);
                        let scale_menu = ui.menu_button(format!("{}    ", tr(p.view.scale.label(settings.device().is_some()))), |ui| {
                            navigation_menu(ui, |ui| {
                                for scale in PresentScale::options(settings.device().is_some()) {
                                    if ui.selectable_label(p.view.scale == *scale, tr(scale.label(settings.device().is_some()))).clicked() {
                                        p.set_scale(*scale);
                                        ui.close();
                                    }
                                }
                            });
                        });
                        paint_menu_chevron(ui.painter(), scale_menu.response.rect, t.icon_secondary);
                    });
                });
        },
    );
    if let Some(id) = navigate {
        p.go(id, Transition::Instant, 0.0, Easing::Linear, now, true, false);
    }
    device_choice
}

pub(super) fn paint_menu_chevron(painter: &egui::Painter, button: Rect, color: Color32) {
    let icon = Rect::from_center_size(pos2(button.right() - 11.0, button.center().y), vec2(17.0, 17.0));
    icons::paint(painter, icon, Icon::Chevron, color);
}

fn navigation_menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.set_min_width(212.0);
    add(ui);
}

/// How often the player polls the controllers while its prototype has gamepad triggers (seconds).
const PAD_POLL_SECS: f64 = 1.0 / 30.0;

/// Before a player's frame: the controller buttons pressed since the last one, polled only while
/// the prototype has gamepad triggers and the window has the focus (systems and browsers report
/// controllers to the focused window only; focus coming back draws a frame, which polls again).
fn poll_pads(app: &mut FigApp, p: &mut Presenter, ctx: &egui::Context) {
    if !p.uses_pads || !ctx.input(|i| i.focused) {
        return;
    }
    let held = app.services.gamepad_held();
    let down = held & !p.pads_held;
    p.pads_held = held;
    // Each press once, in layout order.
    let pressed = (0..u32::BITS as usize).filter(|i| down & (1 << i) != 0).filter_map(figcraft_doc::proto::gamepad_button);
    p.pads.extend(pressed.map(str::to_string));
    crate::repaint_in(ctx, PAD_POLL_SECS);
}

/// After a player's frame: the links it opens and the widget clicks it took. Every link waits in
/// [`link_notice`] — to a website or to the mail app (#1096). Skipping the notice opens at once
/// only a link the viewer's own click, press, drag or key fired, one per frame, and never while a
/// notice shows: a delay or the pointer passing over a layer never opens anything unasked. A
/// further link that fires while the notice shows is dropped, whatever fired it.
fn finish(app: &mut FigApp, p: &mut Presenter) {
    let skipped = app.session.prefs.link_notice_skipped;
    let mut opened = false;
    for (u, asked, new_tab) in p.urls.drain(..) {
        if skipped && asked && !opened && p.link_notice.is_none() {
            open_link(app, &u, new_tab);
            opened = true;
        } else if p.link_notice.is_none() {
            p.link_notice = Some((u, (asked && !skipped).then_some(false), new_tab));
            // A press under way ends here: its release would otherwise reach the prototype
            // behind the notice, which takes no input while it shows.
            p.press = None;
        }
    }
    if let Some(e) = p.click_widgets(&app.session) {
        app.toast(e);
    }
}

/// Open `url` as its Open link action says: in a new tab, or in place of the prototype where the
/// platform can send the viewer elsewhere (the web build's own tab).
fn open_link(app: &mut FigApp, url: &str, new_tab: bool) {
    if new_tab {
        app.services.open_url(url);
    } else {
        app.services.open_url_here(url);
    }
}

/// Whether `url` is an `http` or `https` link (of the openable URLs, the others are `mailto:`;
/// [`figcraft_engine::doc_urls`]).
fn goes_to_a_website(url: &str) -> bool {
    url.trim().split_once(':').is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
}

/// The notice before a link leaves the prototype — for a website, or for the mail app: the link,
/// "Don't show this again" for a link the viewer fired (saved with the preferences when Open link
/// is pressed), Open link and Cancel. Escape or the close button cancels.
fn link_notice(app: &mut FigApp, p: &mut Presenter, ctx: &egui::Context, id: Id) {
    let Some((url, skip, _)) = p.link_notice.as_mut() else { return };
    let t = Tokens::get(ctx);
    let (mut open, mut go, mut cancel) = (true, false, false);
    crate::dialogs::window(tr("Leaving the prototype"), ctx).id(id).open(&mut open).show(ctx, |ui| {
        ui.set_width(340.0);
        let what = if goes_to_a_website(url) {
            tr("This link opens a website outside the prototype:")
        } else {
            tr("This link opens your mail app to write a message:")
        };
        widgets::label(ui, what, t.text);
        ui.add(egui::Label::new(egui::RichText::new(url.as_str()).font(regular(11.0)).color(t.text_secondary)).wrap());
        ui.add_space(8.0);
        if let Some(skip) = skip.as_mut()
            && widgets::checkbox(ui, *skip, tr("Don't show this again"))
        {
            *skip = !*skip;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            go = widgets::button(ui, tr("Open link"), true, true).clicked();
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked();
        });
    });
    cancel |= !open || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if go {
        let Some((url, skip, new_tab)) = p.link_notice.take() else { return };
        if skip == Some(true) {
            app.run("proto.linkNotice", json!({"show": false}));
        }
        open_link(app, &url, new_tab);
    } else if cancel {
        p.link_notice = None;
    }
}

/// The width of the presenter's Inspect panel.
const INSPECT_W: f32 = 280.0;

/// The presenter's Inspect panel: the inspected layer's properties, as in the Inspect tab.
fn inspect_panel(app: &mut FigApp, ui: &mut Ui, pres: &Presenter) {
    let t = Tokens::get(ui.ctx());
    let found = pres.inspected.zip(pres.doc.as_ref()).and_then(|(id, doc)| doc.node(id).map(|n| (id, doc, n.name.clone())));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let Some((id, doc, name)) = found else {
            widgets::section_header(ui, tr("Inspect"), &[], false);
            widgets::note(ui, tr("Hover a layer to see its properties"), t.text_secondary);
            return;
        };
        widgets::section_header(ui, &name, &[], false);
        for section in figcraft_engine::devinspect::properties(doc, id) {
            crate::right::inspect_section(app, ui, &section);
        }
    });
}

/// The top-most visible layer of `root` (it included) under the screen point `q`.
pub(super) fn layer_at(doc: &Document, root: NodeId, place: &Place, q: Pos2) -> Option<NodeId> {
    let mut hidden: HashSet<NodeId> = HashSet::new();
    let mut hit = None;
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        if !n.visible || n.parent.is_some_and(|p| hidden.contains(&p)) {
            hidden.insert(id);
            continue;
        }
        if doc.world_bounds(id).is_some_and(|b| place.rect(b).contains(q)) {
            hit = Some(id);
        }
    }
    hit
}

/// The prototype preview pane: the player in a floating, resizable window over the canvas. It
/// follows edits live, and selecting another top-level frame in the editor shows that frame.
pub struct Pane {
    pub player: Presenter,
    /// The open file it previews (another tab closes it).
    doc_uid: u64,
    /// The top-level frame last selected in the editor.
    selected: Option<NodeId>,
}

/// The top-level frame of the editor's first selected layer.
fn selected_frame(ds: &figcraft_engine::DocState) -> Option<NodeId> {
    ds.selection.first().and_then(|i| ds.doc.top_level_frame(*i))
}

impl Pane {
    pub fn new(start: NodeId, session: &figcraft_engine::Session) -> Self {
        let (doc_uid, selected) = session.ds().map(|ds| (ds.uid, selected_frame(ds))).unwrap_or((0, None));
        Pane { player: Presenter::new(start), doc_uid, selected }
    }
}

/// The preview pane's window id (its position and size persist under it).
pub fn pane_id() -> Id {
    Id::unique("proto_preview_pane")
}

/// Show the preview pane while `app.ui.preview` is set (call once per frame, before shortcuts:
/// keys a hovered pane's prototype uses don't reach the editor).
pub fn show_pane(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut pane) = app.ui.preview.take() else { return };
    let Ok(ds) = app.session.ds() else { return };
    if ds.uid != pane.doc_uid {
        return;
    }
    let now = ctx.input(|i| i.time);
    let (doc, selected) = (ds.doc.clone(), selected_frame(ds));
    pane.player.sync(&doc, now);
    // Deselecting and coming back to the same frame keeps the preview where it got to.
    if selected.is_some() && selected != pane.selected {
        pane.selected = selected;
        if let Some(f) = selected.filter(|f| *f != pane.player.current) {
            pane.player.restart(f, now);
        }
    }
    let t = Tokens::get(ctx);
    let canvas = app.canvas.rect;
    let style = ctx.global_style();
    let typing = ds.text_edit.is_some() || crate::a11y::field_focused(ctx);
    let chrome = Chrome { pane: true, bar_h: 36.0, pad: 12.0, round: style.visuals.window_corner_radius.nw, keys: !typing };
    poll_pads(app, &mut pane.player, ctx);
    let mut action = None;
    egui::Window::new(tr("Prototype preview"))
        .id(pane_id())
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(320.0, 600.0))
        .min_size(vec2(220.0, 260.0))
        .default_pos(pos2(canvas.max.x - 336.0, canvas.min.y + 16.0))
        .constrain_to(ctx.content_rect())
        .frame(egui::Frame::window(&style).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            action = player(&mut pane.player, ui, rect, now, chrome);
        });
    finish(app, &mut pane.player);
    link_notice(app, &mut pane.player, ctx, pane_id().with("link_notice"));
    match action {
        Some(BarAction::Close) => {}
        Some(BarAction::Present) => {
            let start = pane.player.current;
            app.ui.preview = Some(pane);
            app.run("proto.present", json!({"start": start}));
        }
        _ => app.ui.preview = Some(pane),
    }
}
