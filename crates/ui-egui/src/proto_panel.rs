//! The Prototype tab: interactions (with an editor popover), flow starting points, scroll
//! behaviour, overlay settings and prototype settings. Every edit runs a `proto.*` command or
//! `node.set`.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_color::Paint;
use figcraft_doc::proto::{
    Action, ConditionalBlock, Direction, Easing, GAMEPAD_BUTTONS, Interaction, KeyMods, Overflow, OverlayPosition, ScrollPosition, Transition,
    Trigger, is_gamepad_button,
};
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y::Tip;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::proto_editor::{Cx, Slot, set_action};
use crate::right::{menu_item, menu_popup, row};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets::{self, FIELD_H, FieldEdit, Lead, PAD};

/// The interaction whose editor popover is open: (layer, index).
fn editor_id() -> Id {
    Id::new("proto_editor")
}

pub fn open_editor(ctx: &egui::Context, target: Option<(NodeId, usize)>) {
    ctx.data_mut(|d| match target {
        Some(t) => {
            d.insert_temp(editor_id(), t);
        }
        None => {
            d.remove::<(NodeId, usize)>(editor_id());
        }
    });
}

pub fn editor(ctx: &egui::Context) -> Option<(NodeId, usize)> {
    ctx.data(|d| d.get_temp(editor_id()))
}

const TRIGGERS: [&str; 10] = [
    tr_noop("On click"),
    tr_noop("On drag"),
    tr_noop("While hovering"),
    tr_noop("While pressing"),
    tr_noop("Key/Gamepad"),
    tr_noop("Mouse enter"),
    tr_noop("Mouse leave"),
    tr_noop("Mouse down"),
    tr_noop("Mouse up"),
    tr_noop("After delay"),
];

fn trigger_kind(t: &Trigger) -> &'static str {
    match t {
        Trigger::OnClick => "On click",
        Trigger::OnDrag => "On drag",
        Trigger::OnHover => "While hovering",
        Trigger::OnPress => "While pressing",
        Trigger::KeyPress { .. } => "Key/Gamepad",
        Trigger::MouseEnter => "Mouse enter",
        Trigger::MouseLeave => "Mouse leave",
        Trigger::MouseDown => "Mouse down",
        Trigger::MouseUp => "Mouse up",
        Trigger::AfterDelay { .. } => "After delay",
    }
}

fn trigger_of(kind: &str, cur: &Trigger) -> Trigger {
    match kind {
        "On drag" => Trigger::OnDrag,
        "While hovering" => Trigger::OnHover,
        "While pressing" => Trigger::OnPress,
        "Key/Gamepad" => match cur {
            Trigger::KeyPress { key } => Trigger::KeyPress { key: key.clone() },
            _ => Trigger::KeyPress { key: String::new() },
        },
        "Mouse enter" => Trigger::MouseEnter,
        "Mouse leave" => Trigger::MouseLeave,
        "Mouse down" => Trigger::MouseDown,
        "Mouse up" => Trigger::MouseUp,
        "After delay" => match cur {
            Trigger::AfterDelay { ms } => Trigger::AfterDelay { ms: *ms },
            _ => Trigger::AfterDelay { ms: 800 },
        },
        _ => Trigger::OnClick,
    }
}

/// The trigger as the list shows it ("Key/Gamepad Space", "After delay 800ms").
fn trigger_summary(t: &Trigger) -> String {
    match t {
        Trigger::KeyPress { key } if key.is_empty() => tr("Key/Gamepad").into(),
        // A controller button names itself ("Gamepad A").
        Trigger::KeyPress { key } if is_gamepad_button(key) => key.clone(),
        Trigger::KeyPress { key } => trf("Key/Gamepad {key}", &[("key", key)]),
        Trigger::AfterDelay { ms } => trf("After delay {ms}ms", &[("ms", ms)]),
        other => tr(trigger_kind(other)).into(),
    }
}

const TRANSITIONS: [&str; 8] = [
    tr_noop("Instant"),
    tr_noop("Dissolve"),
    tr_noop("Smart animate"),
    tr_noop("Move in"),
    tr_noop("Move out"),
    tr_noop("Push"),
    tr_noop("Slide in"),
    tr_noop("Slide out"),
];

fn transition_of(label: &str, cur: Transition) -> Transition {
    let d = cur.direction().unwrap_or_default();
    match label {
        "Dissolve" => Transition::Dissolve,
        "Smart animate" => Transition::SmartAnimate,
        "Move in" => Transition::MoveIn { direction: d },
        "Move out" => Transition::MoveOut { direction: d },
        "Push" => Transition::Push { direction: d },
        "Slide in" => Transition::SlideIn { direction: d },
        "Slide out" => Transition::SlideOut { direction: d },
        _ => Transition::Instant,
    }
}

/// Destinations a "Change to" can pick: the variants next to the layer's component.
fn variants_for(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let comp = figcraft_engine::components::owning_component(doc, id).or_else(|| match doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Instance { component, .. }) => Some(*component),
        _ => None,
    });
    let set = comp
        .and_then(|c| doc.node(c))
        .and_then(|c| c.parent)
        .filter(|p| doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::ComponentSet { .. })));
    match set.and_then(|s| doc.node(s)) {
        Some(s) => s.children.clone(),
        None => comp.into_iter().collect(),
    }
}

/// Destinations for navigate/overlay actions: the page's top-level frames.
fn frames_for(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.top_frames(page)
}

/// Destinations for "Navigate to": the top-level frames, then the sections (navigating to a
/// section shows the frame of it shown last, else its first).
fn nav_targets(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut v = frames_for(doc, page);
    v.extend(doc.sections_in(page));
    v
}

fn name_of(doc: &Document, id: NodeId) -> String {
    doc.node(id).map(|n| n.name.clone()).unwrap_or_else(|| tr("Missing layer").into())
}

/// One-line action summary ("Navigate to Frame 2").
pub(crate) fn action_summary(doc: &Document, a: &Action) -> String {
    let vars = &doc.variables;
    match a {
        Action::OpenUrl { url } if !url.is_empty() => trf("Open link {url}", &[("url", url)]),
        Action::SetVariable { variable, expr, .. } => {
            let name = vars.get(variable).map_or(variable.as_str(), |v| v.name.as_str());
            match expr {
                Some(e) => trf("Set {name} to {value}", &[("name", &name), ("value", &e.display(vars))]),
                None => trf("Set variable {name}", &[("name", &name)]),
            }
        }
        Action::SetVariableMode { collection, mode } => {
            let c = vars.collection(collection);
            let mode = c.and_then(|c| c.modes.iter().find(|m| &m.id == mode)).map_or(mode.as_str(), |m| m.name.as_str());
            trf("Set {name} to {value}", &[("name", &c.map_or(collection.as_str(), |c| c.name.as_str())), ("value", &mode)])
        }
        Action::Conditional { blocks } => match blocks.first().and_then(|b| b.condition.as_ref()) {
            Some(c) => trf("If {condition}", &[("condition", &c.display(vars))]),
            None => tr(a.label()).to_string(),
        },
        other => match other.dest() {
            Some(d) => format!("{} {}", tr(other.label()), name_of(doc, d)),
            None => tr(other.label()).to_string(),
        },
    }
}

/// A variable value as a set variable action stores it.
pub(crate) fn value_json(v: &VariableValue) -> Value {
    match v {
        VariableValue::Color(c) => json!(c.to_css_hex()),
        VariableValue::Number(n) => json!(n),
        VariableValue::Boolean(b) => json!(b),
        VariableValue::String(s) => json!(s),
        VariableValue::Alias(_) => Value::Null,
    }
}

/// A default action of the kind labelled `label` (None when it needs a destination and there is
/// none).
pub(crate) fn action_of(label: &str, doc: &Document, page: NodeId, node: NodeId, cur: Option<&Action>) -> Option<Action> {
    let own = doc.top_level_frame(node);
    let frames = frames_for(doc, page);
    let cur_dest = cur.and_then(|a| a.dest()).filter(|d| frames.contains(d));
    let frame_dest = cur_dest.or_else(|| frames.iter().copied().find(|f| Some(*f) != own)).or(frames.first().copied());
    Some(match label {
        "Navigate to" => Action::Navigate { dest: frame_dest?, preserve_scroll: matches!(cur, Some(Action::Navigate { preserve_scroll: true, .. })) },
        "Change to" => {
            let vs = variants_for(doc, node);
            let mine = figcraft_engine::components::owning_component(doc, node);
            Action::ChangeTo { dest: vs.iter().copied().find(|v| Some(*v) != mine).or(vs.first().copied())? }
        }
        "Back" => Action::Back,
        "Scroll to" => {
            let root = own?;
            let dest = doc.subtree(root).into_iter().skip(1).find(|i| *i != node)?;
            Action::ScrollTo { dest, offset: [0.0, 0.0] }
        }
        "Open link" => Action::OpenUrl { url: String::new() },
        "Open overlay" => {
            Action::OpenOverlay { dest: frame_dest?, position: OverlayPosition::Center, close_on_outside: true, background: None, offset: None }
        }
        "Swap overlay" => Action::SwapOverlay { dest: frame_dest? },
        "Close overlay" => Action::CloseOverlay,
        "Set variable" => {
            let v = doc.variables.variables.first()?;
            let value = value_json(&figcraft_engine::cmd::proto_actions::start_value(&doc.variables, v));
            Action::SetVariable { variable: v.id.clone(), value, expr: None }
        }
        "Set variable mode" => {
            let c = doc.variables.collections.iter().find(|c| !c.modes.is_empty())?;
            // The mode after the default: switching to the default changes nothing.
            let m = c.modes.get(1).or(c.modes.first())?;
            Action::SetVariableMode { collection: c.id.clone(), mode: m.id.clone() }
        }
        "Conditional" => Action::Conditional {
            blocks: vec![ConditionalBlock {
                condition: Some(figcraft_engine::cmd::proto_actions::default_condition(&doc.variables)),
                actions: vec![],
            }],
        },
        _ => return None,
    })
}

/// The layers an edit of `node`'s interactions applies to: the whole selection when it includes
/// `node` and every selected layer has the same interactions, else just `node`.
pub(crate) fn targets_for(app: &FigApp, node: NodeId) -> Vec<NodeId> {
    let Ok(ds) = app.session.ds() else { return vec![node] };
    let sel = &ds.selection;
    let first = ds.doc.node(node).map(|n| &n.interactions);
    let same = sel.len() > 1 && sel.contains(&node) && sel.iter().all(|i| ds.doc.node(*i).map(|n| &n.interactions) == first);
    if same { sel.clone() } else { vec![node] }
}

fn patch(app: &mut FigApp, node: NodeId, index: usize, fields: Value) {
    let mut p = json!({"ids": targets_for(app, node), "index": index});
    if let (Some(o), Value::Object(f)) = (p.as_object_mut(), fields) {
        o.extend(f);
    }
    app.run("proto.interaction.set", p);
}

pub fn prototype(app: &mut FigApp, ui: &mut Ui) {
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let sel = ds.selection.clone();
    let page = ds.page;
    if sel.is_empty() {
        open_editor(ui.ctx(), None);
        flows_overview(app, ui, &doc, page);
        widgets::separator(ui);
        settings(app, ui, &doc, page);
        return;
    }
    let first = sel.first().copied().unwrap_or(NodeId(0));
    let Some(node) = doc.node(first).cloned() else { return };
    if let Some((n, _)) = editor(ui.ctx())
        && n != first
    {
        open_editor(ui.ctx(), None);
    }
    // Flow starting point (top-level frames).
    if doc.top_level_frame(first) == Some(first) {
        flow_section(app, ui, &doc, page, first);
        widgets::separator(ui);
    }
    let mixed = sel.iter().any(|i| doc.node(*i).map(|n| &n.interactions) != Some(&node.interactions));
    if mixed {
        mixed_interactions(app, ui, &doc, page, &sel);
    } else {
        interactions(app, ui, &doc, page, &node);
    }
    widgets::separator(ui);
    scroll_section(app, ui, &doc, &sel, &node);
}

/// Several layers with different interactions: "+" replaces them all with one click interaction
/// (one undo step); "Remove all" clears them.
fn mixed_interactions(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId, sel: &[NodeId]) {
    let t = Tokens::get(ui.ctx());
    if widgets::section_header(ui, tr("Interactions"), &[(Icon::Plus, tr("Replace with a new interaction"))], false).is_some() {
        let first = sel.first().copied().unwrap_or(NodeId(0));
        let action = action_of("Navigate to", doc, page, first, None).unwrap_or(Action::Back);
        let steps = json!([
            {"command": "proto.interaction.clear", "params": {"ids": sel}},
            {"command": "proto.interaction.add", "params": {"ids": sel, "trigger": {"type": "onClick"}, "action": action}},
        ]);
        app.run("command.batch", json!({"label": "Replace interactions", "commands": steps}));
        open_editor(ui.ctx(), Some((first, 0)));
    }
    row(ui, 32.0, |ui| {
        widgets::label(ui, tr("Mixed interactions: click + to replace them"), t.text_secondary);
    });
    row(ui, 28.0, |ui| {
        if ui.add(egui::Button::new(egui::RichText::new(tr("Remove all")).font(regular(11.0)))).clicked() {
            app.run("proto.interaction.clear", json!({"ids": sel}));
        }
    });
}

fn flows_of(doc: &Document, page: NodeId) -> Vec<proto::Flow> {
    match doc.node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { flows, .. }) => flows.clone(),
        _ => vec![],
    }
}

/// A square icon button painted at `rect`; true when clicked.
fn icon_button(ui: &mut Ui, id: Id, rect: Rect, icon: Icon, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id, Sense::click()).tip(tip);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), rect.shrink(5.0), icon, t.icon);
    resp.clicked()
}

/// One flow row: play, editable name, preview, remove.
fn flow_row(app: &mut FigApp, ui: &mut Ui, f: &proto::Flow) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let button = |dx: f32| Rect::from_center_size(pos2(dx, r.center().y), vec2(24.0, 24.0));
    let play = button(r.min.x + PAD + 8.0);
    if icon_button(ui, Id::new(("flow_play", f.start)), play, Icon::Play, tr("Present this flow")) {
        app.run("proto.present", json!({"start": f.start}));
    }
    let name_rect = Rect::from_min_max(pos2(play.max.x + 6.0, r.center().y - 9.0), pos2(r.max.x - 64.0, r.center().y + 9.0));
    if let Some(n) = widgets::text_field_inline(ui, Id::new(("flow_name", f.start)), &f.name, name_rect, regular(11.0)) {
        app.run("proto.flow.rename", json!({"start": f.start, "name": n}));
    }
    if icon_button(ui, Id::new(("flow_preview", f.start)), button(r.max.x - 44.0), Icon::Preview, tr("Preview this flow")) {
        app.run("proto.preview", json!({"start": f.start}));
    }
    if icon_button(ui, Id::new(("flow_rm", f.start)), button(r.max.x - 20.0), Icon::Minus, tr("Remove starting point")) {
        app.run("proto.flow.remove", json!({"start": f.start}));
    }
}

fn flows_overview(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId) {
    let t = Tokens::get(ui.ctx());
    let flows = flows_of(doc, page);
    if widgets::section_header(ui, tr("Flow starting points"), &[(Icon::Plus, tr("Add starting point"))], flows.is_empty()).is_some() {
        let free = doc.top_frames(page).into_iter().find(|f| !flows.iter().any(|x| x.start == *f));
        match free {
            Some(f) => {
                app.run("proto.flow.add", json!({"start": f}));
            }
            None => app.toast(tr("Every top-level frame already starts a flow")),
        }
    }
    if flows.is_empty() {
        widgets::note(ui, tr("Select a frame and add a starting point to present it as a flow."), t.text_secondary);
    }
    for f in &flows {
        flow_row(app, ui, f);
        // Its description under it (in full on hover).
        let d = f.description.trim();
        if !d.is_empty() {
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            // Lined up with the flow's name (after the play button).
            let indent = PAD + 26.0;
            let shown = widgets::elide(ui, d, w - indent - PAD);
            ui.painter().text(pos2(r.min.x + indent, r.min.y + 1.0), Align2::LEFT_TOP, &shown, regular(10.5), t.text_secondary);
            if shown != d {
                resp.on_hover_text(d);
            }
        }
    }
}

fn settings(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId) {
    if widgets::section_header(ui, tr("Prototype settings"), &[(Icon::Preview, tr("Open prototype preview"))], false).is_some() {
        app.run("proto.preview", json!({}));
    }
    let settings = match doc.node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { prototype, .. }) => prototype.clone(),
        _ => proto::PrototypeSettings::default(),
    };
    let bg = settings.background;
    let model = settings.model();
    // Phones and tablets turn: a toggle beside the device picker.
    let rotates = model.is_some_and(|d| d.rotates);
    let landscape = settings.landscape();
    widgets::sublabels(ui, tr("Device"), None);
    row(ui, 32.0, |ui| {
        let w = ui.available_width() - if rotates { 32.0 } else { 0.0 };
        // Device names stay as they are; "None" and the custom size are ours.
        let custom = proto::PrototypeSettings::CUSTOM_DEVICE;
        let shown = if settings.device == "None" || settings.device == custom { tr(&settings.device) } else { settings.device.as_str() };
        let resp = widgets::dropdown(ui, shown, w, None);
        menu_popup(&resp).show(|ui| {
            let mut pick = None;
            if crate::right::menu_check(ui, settings.device == "None", tr("None"), "") {
                pick = Some("None");
            }
            ui.separator();
            for d in proto::PrototypeSettings::DEVICES.iter().map(|d| d.name) {
                if crate::right::menu_check(ui, d == settings.device, d, "") {
                    pick = Some(d);
                }
            }
            ui.separator();
            if crate::right::menu_check(ui, settings.device == custom, tr(custom), "") {
                pick = Some(custom);
            }
            if let Some(d) = pick {
                app.run("proto.settings", json!({"device": d}));
                ui.close();
            }
        });
        let tip = if landscape { tr("Rotate to portrait") } else { tr("Rotate to landscape") };
        if rotates && icons::button(ui, Icon::Rotate, 24.0, landscape, tip).clicked() {
            app.run("proto.settings", json!({"rotation": if landscape { "portrait" } else { "landscape" }}));
        }
    });
    if let Some(m) = model.filter(|m| !m.colors.is_empty()) {
        device_colors(app, ui, m.colors, settings.device_color());
    }
    if settings.device == proto::PrototypeSettings::CUSTOM_DEVICE {
        custom_size(app, ui, model);
    }
    row(ui, 32.0, |ui| {
        let clicked = widgets::checkbox(ui, settings.responsive, tr("Responsive"));
        ui.interact(ui.min_rect(), Id::new("proto_responsive_tip"), Sense::hover()).on_hover_text(tr(RESPONSIVE_TIP));
        if clicked {
            app.run("proto.settings", json!({"responsive": !settings.responsive}));
        }
    });
    widgets::sublabels(ui, tr("Background"), None);
    row(ui, 32.0, |ui| {
        widgets::swatch(ui, &Paint::solid(bg), 16.0);
        if let Some(h) = widgets::text_field(ui, Id::new("proto_bg"), &bg.to_hex(), 96.0, "")
            && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
        {
            app.run("proto.settings", json!({"background": c.to_css_hex()}));
        }
    });
}

/// What "Responsive" does, on hover.
const RESPONSIVE_TIP: &str =
    tr_noop("Resize frames to the device's screen (or the window without a device): their constraints and auto layout respond");

/// A device colour as a swatch paint.
fn color_paint(c: proto::DeviceColor) -> Paint {
    let [r, g, b] = c.rgb;
    Paint::solid(figcraft_color::Color::from_rgb8_const(r, g, b))
}

/// The colours the device comes in: its swatch and a picker (each colour with its swatch).
fn device_colors(app: &mut FigApp, ui: &mut Ui, colors: &[proto::DeviceColor], current: Option<proto::DeviceColor>) {
    widgets::sublabels(ui, tr("Color"), None);
    row(ui, 32.0, |ui| {
        if let Some(c) = current {
            widgets::swatch(ui, &color_paint(c), 16.0);
        }
        let w = ui.available_width();
        let resp = widgets::dropdown(ui, current.map_or("", |c| c.name), w, None);
        menu_popup(&resp).show(|ui| {
            for c in colors {
                let on = current.is_some_and(|x| x.name == c.name);
                let label = format!("{}{}", if on { "✓ " } else { "    " }, c.name);
                let paint = color_paint(*c);
                let picked = crate::right::menu_item_with(ui, &label, "", 22.0, |p, r, _| {
                    widgets::paint_swatch(p, Rect::from_center_size(pos2(r.max.x - 16.0, r.center().y), vec2(12.0, 12.0)), &paint);
                });
                if picked {
                    app.run("proto.settings", json!({"color": c.name}));
                    ui.close();
                }
            }
        });
    });
}

/// The custom size's screen: width and height fields and a button that swaps them.
fn custom_size(app: &mut FigApp, ui: &mut Ui, model: Option<proto::Device>) {
    let Some(d) = model else { return };
    let col = widgets::col_width(ui.available_width());
    widgets::sublabels(ui, tr("Size"), None);
    row(ui, 32.0, |ui| {
        for (key, lead, v) in [("width", "W", d.width), ("height", "H", d.height)] {
            if let Some(FieldEdit::Set(x) | FieldEdit::Scrub(x, true)) =
                widgets::number_field(ui, Id::new(("proto_custom", key)), Lead::Text(lead), Some(v), "", col, 1.0)
                && let Some(x) = proto::PrototypeSettings::custom_side(x)
                && x != v
            {
                app.run("proto.settings", json!({ key: x }));
            }
        }
        if icons::button(ui, Icon::Rotate, 24.0, false, tr("Swap width and height")).clicked() && d.width != d.height {
            app.run("proto.settings", json!({"width": d.height, "height": d.width}));
        }
    });
}

fn flow_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId, frame: NodeId) {
    let flows = flows_of(doc, page);
    let mine = flows.iter().find(|f| f.start == frame).cloned();
    // A flow's row has its own preview button.
    let buttons: &[(Icon, &str)] =
        if mine.is_none() { &[(Icon::Preview, tr("Preview this frame")), (Icon::Plus, tr("Add starting point"))] } else { &[] };
    match widgets::section_header(ui, tr("Flow starting point"), buttons, mine.is_none()) {
        Some(0) => {
            app.run("proto.preview", json!({"start": frame}));
        }
        Some(_) => {
            app.run("proto.flow.add", json!({"start": frame}));
        }
        None => {}
    }
    if let Some(f) = mine {
        flow_row(app, ui, &f);
        row(ui, 32.0, |ui| {
            if let Some(d) = widgets::text_field(ui, Id::new(("flow_desc", frame)), &f.description, ui.available_width(), tr("Add a description"))
                && d != f.description
            {
                app.run("proto.flow.rename", json!({"start": frame, "description": d}));
            }
        });
    }
}

fn interactions(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId, node: &Node) {
    let t = Tokens::get(ui.ctx());
    let id = node.id;
    let list = node.interactions.clone();
    if widgets::section_header(ui, tr("Interactions"), &[(Icon::Plus, tr("Add interaction"))], list.is_empty()).is_some() {
        // Click → navigate to the next frame (or Back when there's nowhere to go).
        let action = action_of("Navigate to", doc, page, id, None).unwrap_or(Action::Back);
        if let Some(r) = app.run("proto.interaction.add", json!({"ids": targets_for(app, id), "trigger": {"type": "onClick"}, "action": action})) {
            let k = r.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            open_editor(ui.ctx(), Some((id, k)));
        }
    }
    let open = editor(ui.ctx());
    let mut anchor: Option<Rect> = None;
    for (i, it) in list.iter().enumerate() {
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
        let body = Rect::from_min_max(pos2(r.min.x + PAD - 6.0, r.min.y + 2.0), pos2(r.max.x - 40.0, r.max.y - 2.0));
        let resp = ui.interact(body, ui.id().with(("proto_it", id, i)), Sense::click());
        let active = open == Some((id, i));
        if active {
            ui.painter().rect_filled(body, CornerRadius::same(5), t.selected_row);
            anchor = Some(r);
        } else if resp.hovered() {
            ui.painter().rect_filled(body, CornerRadius::same(5), t.hover);
        }
        // A small connection glyph.
        let g = pos2(body.min.x + 14.0, body.center().y);
        ui.painter().circle_stroke(g, 5.0, Stroke::new(1.2, ACCENT));
        ui.painter().circle_filled(g, 2.0, ACCENT);
        let tx = body.min.x + 28.0;
        let clip = ui.painter().with_clip_rect(body);
        clip.text(pos2(tx, body.center().y - 7.0), Align2::LEFT_CENTER, trigger_summary(&it.trigger), semibold(11.0), t.text);
        let summary = it.actions.iter().map(|a| action_summary(doc, a)).collect::<Vec<_>>().join(", ");
        clip.text(pos2(tx, body.center().y + 8.0), Align2::LEFT_CENTER, summary, regular(11.0), t.text_secondary);
        if resp.clicked() {
            open_editor(ui.ctx(), if active { None } else { Some((id, i)) });
        }
        let rm = Rect::from_center_size(pos2(r.max.x - 20.0, r.center().y), vec2(24.0, 24.0));
        if icon_button(ui, ui.id().with(("proto_rm", id, i)), rm, Icon::Minus, tr("Remove interaction")) {
            open_editor(ui.ctx(), None);
            app.run("proto.interaction.remove", json!({"ids": targets_for(app, id), "index": i}));
            return;
        }
    }
    if let Some((n, k)) = open
        && n == id
        && let Some(it) = list.get(k)
    {
        let panel_left = ui.clip_rect().min.x;
        let y = anchor.map(|a| a.min.y).unwrap_or(ui.clip_rect().min.y + 80.0);
        editor_popover(app, ui.ctx(), doc, page, id, k, it, pos2(panel_left - 8.0, y));
    } else if open.is_some_and(|(n, _)| n == id) {
        open_editor(ui.ctx(), None);
    }
}

/// The interaction editor, floating left of the panel (`at` is its top-right corner).
#[allow(clippy::too_many_arguments)]
fn editor_popover(app: &mut FigApp, ctx: &egui::Context, doc: &Document, page: NodeId, node: NodeId, k: usize, it: &Interaction, at: Pos2) {
    let t = Tokens::get(ctx);
    // Escape in a field (it cancels the edit) doesn't close the editor.
    let typing = ctx.egui_wants_keyboard_input();
    let width = 280.0;
    let screen = ctx.content_rect();
    let pos = pos2(at.x - width, at.y.min(screen.max.y - 420.0).max(screen.min.y + 48.0));
    let mut close = false;
    egui::Area::new(Id::new("proto_editor_area")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::new()
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(8))
            .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: t.shadow })
            .show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                // Header.
                let (hr, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
                ui.painter().text(pos2(hr.min.x + PAD, hr.center().y), Align2::LEFT_CENTER, tr("Interaction"), semibold(11.0), t.text);
                let x = Rect::from_center_size(pos2(hr.max.x - 20.0, hr.center().y), vec2(24.0, 24.0));
                let xr = ui.interact(x, ui.id().with("proto_editor_close"), Sense::click()).tip(tr("Close"));
                if xr.hovered() {
                    ui.painter().rect_filled(x, CornerRadius::same(5), t.hover);
                }
                icons::paint(ui.painter(), x.shrink(6.0), Icon::Close, t.icon);
                if xr.clicked() {
                    close = true;
                }
                widgets::separator(ui);
                // Long action lists (conditionals) scroll inside the window. (An area's UI is as tall as
                // it was last frame: the minimum lets the list grow when an action opens.)
                let max_h = (screen.max.y - pos.y - 64.0).max(160.0);
                egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(max_h).min_scrolled_height(max_h).show(ui, |ui| {
                    trigger_rows(app, ui, node, k, it);
                    widgets::separator(ui);
                    widgets::sublabels(ui, if it.actions.len() > 1 { tr("Actions") } else { tr("Action") }, None);
                    let cx = Cx { doc, page, node, k };
                    crate::proto_editor::action_list(app, ui, &cx, &it.actions, &[]);
                    if figcraft_doc::proto::walk(&it.actions).any(animates) {
                        widgets::separator(ui);
                        animation_rows(app, ui, node, k, it);
                    }
                    ui.add_space(8.0);
                });
            });
    });
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !typing && !capturing(ctx) && !crate::variable_ui::picker_open(ctx);
    if close || escape {
        open_editor(ctx, None);
    }
}

/// Whether the action plays an animation (the interaction's transition applies to it).
fn animates(a: &Action) -> bool {
    !matches!(
        a,
        Action::OpenUrl { .. } | Action::SetVariable { .. } | Action::SetVariableMode { .. } | Action::CloseOverlay | Action::Conditional { .. }
    )
}

fn capture_id() -> Id {
    Id::new("proto_key_capture")
}

fn capturing(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(capture_id())).unwrap_or(false)
}

fn trigger_rows(app: &mut FigApp, ui: &mut Ui, node: NodeId, k: usize, it: &Interaction) {
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Trigger"), None);
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, tr(trigger_kind(&it.trigger)), ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for (i, l) in TRIGGERS.iter().enumerate() {
                if i == 4 || i == 5 || i == 9 {
                    ui.separator();
                }
                if menu_item(ui, tr(l), "") {
                    let next = trigger_of(l, &it.trigger);
                    patch(app, node, k, json!({"trigger": next}));
                    if matches!(next, Trigger::KeyPress { ref key } if key.is_empty()) {
                        ui.ctx().data_mut(|d| d.insert_temp(capture_id(), true));
                    }
                    ui.close();
                }
            }
        });
    });
    match &it.trigger {
        Trigger::MouseEnter | Trigger::MouseLeave | Trigger::MouseDown | Trigger::MouseUp => {
            row(ui, 32.0, |ui| {
                widgets::label(ui, tr("Delay"), t.text_secondary);
                let id = Id::new(("proto_mouse_delay", node, k));
                if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                    widgets::number_field(ui, id, Lead::None, Some(f64::from(it.delay_ms)), "ms", 96.0, 10.0)
                    && v.is_finite()
                {
                    patch(app, node, k, json!({"delayMs": v.clamp(0.0, 600_000.0).round()}));
                }
            });
        }
        Trigger::AfterDelay { ms } => {
            row(ui, 32.0, |ui| {
                widgets::label(ui, tr("Delay"), t.text_secondary);
                if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                    widgets::number_field(ui, Id::new(("proto_delay", node, k)), Lead::None, Some(f64::from(*ms)), "ms", 96.0, 10.0)
                    && v.is_finite()
                {
                    patch(app, node, k, json!({"trigger": {"type": "afterDelay", "ms": v.clamp(0.0, 600_000.0).round() as u32}}));
                }
            });
        }
        Trigger::KeyPress { key } => {
            row(ui, 32.0, |ui| {
                let cap = capturing(ui.ctx());
                let label = if cap {
                    tr("Press a key…").to_string()
                } else if key.is_empty() {
                    tr("Click to record a key").to_string()
                } else {
                    key.clone()
                };
                // The key field, and a menu of controller buttons beside it.
                let w = (ui.available_width() - PAD_MENU_W - ui.spacing().item_spacing.x).max(40.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::click());
                let id = capture_id().with("field");
                ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
                if cap {
                    ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
                    ui.memory_mut(|m| m.request_focus(id));
                    let _ = ui.interact(r, id, Sense::focusable_noninteractive());
                    let pressed = ui.input(|i| {
                        i.events.iter().find_map(|e| match e {
                            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                            _ => None,
                        })
                    });
                    if let Some((kk, m)) = pressed {
                        ui.ctx().data_mut(|d| d.remove::<bool>(capture_id()));
                        ui.memory_mut(|m| m.surrender_focus(id));
                        if kk != egui::Key::Escape || key.is_empty() {
                            // Held modifiers make it a combination (`Shift+K`).
                            let combo = KeyMods { ctrl: m.ctrl, alt: m.alt, shift: m.shift, cmd: m.mac_cmd }.combo(kk.name());
                            patch(app, node, k, json!({"trigger": {"type": "keyPress", "key": combo}}));
                        }
                    }
                } else if resp.hovered() {
                    ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
                }
                ui.painter().text(
                    pos2(r.min.x + 8.0, r.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    regular(11.0),
                    if cap || key.is_empty() { t.text_secondary } else { t.text },
                );
                if resp.clicked() && !cap {
                    ui.ctx().data_mut(|d| d.insert_temp(capture_id(), true));
                }
                let pads = widgets::dropdown(ui, tr("Controller"), PAD_MENU_W, None);
                menu_popup(&pads).show(|ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for b in GAMEPAD_BUTTONS {
                            let mark = if key.eq_ignore_ascii_case(b) { format!("✓ {b}") } else { b.to_string() };
                            if menu_item(ui, &mark, "") {
                                ui.ctx().data_mut(|d| d.remove::<bool>(capture_id()));
                                patch(app, node, k, json!({"trigger": {"type": "keyPress", "key": b}}));
                                ui.close();
                            }
                        }
                    });
                });
            });
        }
        _ => {}
    }
}

/// The width of the controller button menu beside a key trigger's key.
const PAD_MENU_W: f32 = 92.0;

/// A destination picker for the action at `slot`: `make` builds the action for the destination.
#[allow(clippy::too_many_arguments)]
fn dest_dropdown(
    app: &mut FigApp,
    ui: &mut Ui,
    doc: &Document,
    options: &[NodeId],
    cur: Option<NodeId>,
    label: &str,
    make: impl Fn(NodeId) -> Action,
    slot: &Slot,
) {
    widgets::sublabels(ui, label, None);
    row(ui, 32.0, |ui| {
        let text = cur.map(|d| name_of(doc, d)).unwrap_or_else(|| tr("None").into());
        let resp = widgets::dropdown(ui, &text, ui.available_width(), Some(Icon::Frame));
        menu_popup(&resp).show(|ui| {
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for d in options {
                    let n = name_of(doc, *d);
                    let mark = if Some(*d) == cur { format!("✓ {n}") } else { n };
                    if menu_item(ui, &mark, "") {
                        set_action(app, slot, make(*d));
                        ui.close();
                    }
                }
            });
        });
    });
}

/// The settings of the action at `slot` (set variable and conditional actions have their own
/// editors in `proto_editor`).
pub(crate) fn action_fields(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId, slot: &Slot, action: &Action) {
    let node = slot.node;
    let frames = frames_for(doc, page);
    match action {
        Action::Navigate { dest, preserve_scroll } => {
            let (dest, preserve_scroll) = (*dest, *preserve_scroll);
            let mk = |d| Action::Navigate { dest: d, preserve_scroll };
            dest_dropdown(app, ui, doc, &nav_targets(doc, page), Some(dest), tr("Destination"), mk, slot);
            row(ui, 28.0, |ui| {
                if widgets::checkbox(ui, preserve_scroll, tr("Preserve scroll position")) {
                    set_action(app, slot, Action::Navigate { dest, preserve_scroll: !preserve_scroll });
                }
            });
        }
        Action::SwapOverlay { dest } => dest_dropdown(app, ui, doc, &frames, Some(*dest), tr("Overlay"), |d| Action::SwapOverlay { dest: d }, slot),
        Action::ChangeTo { dest } => {
            let vs = variants_for(doc, node);
            dest_dropdown(app, ui, doc, &vs, Some(*dest), tr("Variant"), |d| Action::ChangeTo { dest: d }, slot);
        }
        Action::ScrollTo { dest, offset } => {
            let (dest, offset) = (*dest, *offset);
            let layers: Vec<NodeId> = doc.top_level_frame(node).map(|f| doc.subtree(f).into_iter().skip(1).take(400).collect()).unwrap_or_default();
            dest_dropdown(app, ui, doc, &layers, Some(dest), tr("Scroll to layer"), move |d| Action::ScrollTo { dest: d, offset }, slot);
            // Where the layer ends up from the top-left of what scrolls (room for a fixed header).
            widgets::sublabels(ui, tr("Offset"), None);
            row(ui, 32.0, |ui| {
                let w = ((ui.available_width() - 6.0) / 2.0).max(36.0);
                for (i, (name, key)) in [("X", "proto_scroll_x"), ("Y", "proto_scroll_y")].into_iter().enumerate() {
                    let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                        widgets::number_field(ui, slot.id(key), Lead::Text(name), offset.get(i).copied(), "", w, 1.0)
                    else {
                        continue;
                    };
                    if v.is_finite() {
                        let mut o = offset;
                        if let Some(slot_v) = o.get_mut(i) {
                            *slot_v = v.clamp(-100_000.0, 100_000.0).round();
                        }
                        set_action(app, slot, Action::ScrollTo { dest, offset: o });
                    }
                }
            });
        }
        Action::OpenUrl { url } => {
            widgets::sublabels(ui, tr("Link"), None);
            row(ui, 32.0, |ui| {
                if let Some(u) = widgets::text_field(ui, slot.id("proto_url"), url, ui.available_width(), "https://")
                    && &u != url
                {
                    set_action(app, slot, Action::OpenUrl { url: u.trim().to_string() });
                }
            });
        }
        Action::OpenOverlay { dest, position, close_on_outside, background, offset } => {
            let (dest, position, close_on_outside, background, offset) = (*dest, *position, *close_on_outside, *background, *offset);
            let mk = move |d: NodeId| Action::OpenOverlay { dest: d, position, close_on_outside, background, offset };
            dest_dropdown(app, ui, doc, &frames, Some(dest), tr("Overlay"), mk, slot);
            widgets::sublabels(ui, tr("Overlay position"), None);
            row(ui, 32.0, |ui| {
                let resp = widgets::dropdown(ui, tr(position.label()), ui.available_width(), None);
                menu_popup(&resp).show(|ui| {
                    for p in OverlayPosition::ALL {
                        if menu_item(ui, tr(p.label()), "") {
                            set_action(app, slot, Action::OpenOverlay { dest, position: p, close_on_outside, background, offset });
                            ui.close();
                        }
                    }
                });
            });
            if position == OverlayPosition::Manual {
                // From the top-left of the layer that opens it; empty: right below that layer.
                row(ui, 32.0, |ui| {
                    let w = ((ui.available_width() - 6.0) / 2.0).max(36.0);
                    for (i, (name, key)) in [("X", "proto_ov_x"), ("Y", "proto_ov_y")].into_iter().enumerate() {
                        let shown = offset.and_then(|o| o.get(i).copied());
                        let Some(e) = widgets::number_field_bindable(ui, slot.id(key), Lead::Text(name), shown, "", w, 1.0, None, tr("Auto")).0
                        else {
                            continue;
                        };
                        let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) = e.settled() else { continue };
                        if v.is_finite() {
                            let mut o = offset.unwrap_or([0.0, 0.0]);
                            if let Some(slot_v) = o.get_mut(i) {
                                *slot_v = v.clamp(-100_000.0, 100_000.0).round();
                            }
                            set_action(app, slot, Action::OpenOverlay { dest, position, close_on_outside, background, offset: Some(o) });
                        }
                    }
                });
            }
            row(ui, 28.0, |ui| {
                if widgets::checkbox(ui, close_on_outside, tr("Close when clicking outside")) {
                    set_action(app, slot, Action::OpenOverlay { dest, position, close_on_outside: !close_on_outside, background, offset });
                }
            });
            row(ui, 28.0, |ui| {
                if widgets::checkbox(ui, background.is_some(), tr("Add background behind overlay")) {
                    let bg = if background.is_some() { None } else { Some(figcraft_color::Color::new(0.0, 0.0, 0.0, 0.25)) };
                    set_action(app, slot, Action::OpenOverlay { dest, position, close_on_outside, background: bg, offset });
                }
            });
            if let Some(bg) = background {
                row(ui, 32.0, |ui| {
                    widgets::swatch(ui, &Paint::solid(bg), 16.0);
                    if let Some(h) = widgets::text_field(ui, slot.id("proto_ovbg"), &bg.to_hex(), 96.0, "")
                        && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
                    {
                        set_action(app, slot, Action::OpenOverlay { dest, position, close_on_outside, background: Some(c), offset });
                    }
                    let alpha = (bg.a * 100.0).round();
                    if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                        widgets::number_field(ui, slot.id("proto_ovbg_a"), Lead::None, Some(f64::from(alpha)), "%", 64.0, 1.0)
                        && v.is_finite()
                    {
                        let a = v.clamp(0.0, 100.0) as f32 / 100.0;
                        set_action(app, slot, Action::OpenOverlay { dest, position, close_on_outside, background: Some(bg.with_alpha(a)), offset });
                    }
                });
            }
        }
        Action::SetVariableMode { collection, mode } => {
            let colls = &doc.variables.collections;
            let current = doc.variables.collection(collection);
            widgets::sublabels(ui, tr("Collection"), None);
            row(ui, 32.0, |ui| {
                let resp = widgets::dropdown(ui, current.map_or(tr("Choose collection"), |c| c.name.as_str()), ui.available_width(), None);
                menu_popup(&resp).show(|ui| {
                    for c in colls {
                        if menu_item(ui, &c.name, "")
                            && let Some(m) = c.modes.first()
                        {
                            set_action(app, slot, Action::SetVariableMode { collection: c.id.clone(), mode: m.id.clone() });
                            ui.close();
                        }
                    }
                });
            });
            widgets::sublabels(ui, tr("Mode"), None);
            row(ui, 32.0, |ui| {
                let modes = current.map(|c| c.modes.as_slice()).unwrap_or_default();
                let name = modes.iter().find(|m| &m.id == mode).map_or(tr("Choose mode"), |m| m.name.as_str());
                let resp = widgets::dropdown(ui, name, ui.available_width(), None);
                menu_popup(&resp).show(|ui| {
                    for m in modes {
                        if menu_item(ui, &m.name, "") {
                            set_action(app, slot, Action::SetVariableMode { collection: collection.clone(), mode: m.id.clone() });
                            ui.close();
                        }
                    }
                });
            });
        }
        // Edited by `proto_editor`; Back and Close overlay have no settings.
        Action::SetVariable { .. } | Action::Conditional { .. } | Action::Back | Action::CloseOverlay => {}
    }
}

fn animation_rows(app: &mut FigApp, ui: &mut Ui, node: NodeId, k: usize, it: &Interaction) {
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Animation"), None);
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, tr(it.transition.label()), ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for (i, l) in TRANSITIONS.iter().enumerate() {
                if i == 3 {
                    ui.separator();
                }
                if menu_item(ui, tr(l), "") {
                    patch(app, node, k, json!({"transition": transition_of(l, it.transition)}));
                    ui.close();
                }
            }
        });
    });
    if let Some(d) = it.transition.direction() {
        row(ui, 32.0, |ui| {
            let dirs = [Direction::Left, Direction::Right, Direction::Top, Direction::Bottom];
            let items =
                [(Icon::ChevronLeft, tr("Left")), (Icon::ChevronRight, tr("Right")), (Icon::ChevronUp, tr("Up")), (Icon::Chevron, tr("Down"))];
            let active = dirs.iter().position(|x| *x == d);
            if let Some(i) = widgets::segmented(ui, &items, active, 32.0)
                && let Some(nd) = dirs.get(i)
            {
                patch(app, node, k, json!({"transition": it.transition.with_direction(*nd)}));
            }
        });
    }
    if !matches!(it.transition, Transition::Instant) {
        row(ui, 32.0, |ui| {
            let resp = widgets::dropdown(ui, tr(it.easing.label()), ui.available_width() - 80.0, None);
            menu_popup(&resp).width(220.0).show(|ui| {
                let custom = [Easing::CUSTOM_BEZIER, Easing::CUSTOM_SPRING];
                for (i, e) in Easing::ALL.iter().chain(custom.iter()).enumerate() {
                    if i == 7 || i == Easing::ALL.len() {
                        ui.separator();
                    }
                    let w = ui.available_width().max(200.0);
                    let (r, rr) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
                    if rr.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(5), ACCENT);
                    }
                    let col = if rr.hovered() { Color32::WHITE } else { t.text };
                    curve(ui.painter(), Rect::from_min_size(pos2(r.min.x + 6.0, r.min.y + 4.0), vec2(28.0, 18.0)), *e, col, None);
                    ui.painter().text(pos2(r.min.x + 42.0, r.center().y), Align2::LEFT_CENTER, tr(e.label()), regular(11.5), col);
                    if rr.clicked() {
                        // Keep the current custom values when re-picking the same custom kind.
                        let same_kind = std::mem::discriminant(e) == std::mem::discriminant(&it.easing);
                        patch(app, node, k, json!({"easing": if same_kind { it.easing } else { *e }}));
                        ui.close();
                    }
                }
            });
            if it.easing.is_spring() {
                // A spring plays until it settles: its duration follows from its settings.
                let ms = format!("{:.0} ms", it.easing.seconds(it.duration_ms) * 1000.0);
                ui.label(egui::RichText::new(ms).font(regular(11.0)).color(t.text_secondary))
                    .tip(tr("A spring plays until it settles: its mass, stiffness and damping set how long"));
            } else if let Some(e) =
                widgets::number_field(ui, Id::new(("proto_dur", node, k)), Lead::None, Some(f64::from(it.duration_ms)), "ms", 72.0, 10.0)
            {
                match e {
                    FieldEdit::Set(v) => {
                        if v.is_finite() {
                            patch(app, node, k, json!({"durationMs": v.clamp(1.0, 10_000.0).round()}));
                        }
                    }
                    FieldEdit::Scrub(v, done) => {
                        let _ = app.session.begin_interaction("Edit");
                        let _ = app.session.execute(
                            "proto.interaction.set",
                            &json!({"ids": targets_for(app, node), "index": k, "durationMs": v.clamp(1.0, 10_000.0).round()}),
                        );
                        if done {
                            let _ = app.session.commit_interaction();
                        }
                    }
                }
            }
        });
        custom_easing_fields(app, ui, node, k, it.easing);
        // Curve preview, with a dot running along it: it plays when the preview shows, when the
        // easing or duration changes and while the pointer is over it; otherwise the dot rests at
        // the end (an idle window draws no frames).
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
        let box_r = Rect::from_min_max(pos2(r.min.x + PAD, r.min.y + 4.0), pos2(r.max.x - PAD, r.max.y - 4.0));
        ui.painter().rect_filled(box_r, CornerRadius::same(5), t.field);
        let period = it.easing.seconds(it.duration_ms).max(0.05) + CURVE_HOLD;
        let elapsed = curve_play(ui, Id::new(("proto_curve", node, k)), Id::new((format!("{:?}", it.easing), it.duration_ms)), resp.hovered());
        let phase = if resp.hovered() || elapsed < period * CURVE_PLAYS {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
            ((elapsed % period) / (period - CURVE_HOLD)).min(1.0)
        } else {
            1.0
        };
        curve(ui.painter(), box_r.shrink2(vec2(10.0, 12.0)), it.easing, ACCENT, Some(phase));
    }
}

/// Fields for a custom bezier (x1, y1, x2, y2) or spring (mass, stiffness, damping).
fn custom_easing_fields(app: &mut FigApp, ui: &mut Ui, node: NodeId, k: usize, e: Easing) {
    let (names, vals, steps): ([&str; 4], [f64; 4], [f64; 4]) = match e {
        Easing::CustomBezier { x1, y1, x2, y2 } => (["X1", "Y1", "X2", "Y2"], [x1, y1, x2, y2], [0.01; 4]),
        Easing::CustomSpring { mass, stiffness, damping } => {
            (["Mass", "Stiffness", "Damping", ""], [mass, stiffness, damping, 0.0], [0.1, 1.0, 0.5, 0.0])
        }
        _ => return,
    };
    let count = if matches!(e, Easing::CustomBezier { .. }) { 4 } else { 3 };
    let make = |vals: [f64; 4]| match e {
        Easing::CustomBezier { .. } => Easing::CustomBezier {
            x1: vals[0].clamp(0.0, 1.0),
            y1: vals[1].clamp(-5.0, 5.0),
            x2: vals[2].clamp(0.0, 1.0),
            y2: vals[3].clamp(-5.0, 5.0),
        },
        _ => Easing::CustomSpring { mass: vals[0].clamp(0.01, 100.0), stiffness: vals[1].clamp(1.0, 10_000.0), damping: vals[2].clamp(0.0, 1000.0) },
    };
    row(ui, 32.0, |ui| {
        let w = ((ui.available_width() - 6.0 * (count as f32 - 1.0)) / count as f32).max(36.0);
        for i in 0..count {
            let (Some(name), Some(v), Some(step)) = (names.get(i), vals.get(i), steps.get(i)) else { continue };
            let lead = if count == 4 { Lead::Text(name) } else { Lead::None };
            let edit = widgets::number_field(ui, Id::new(("proto_ease", node, k, i)), lead, Some((v * 100.0).round() / 100.0), "", w, *step).map(
                |e| match e {
                    FieldEdit::Set(x) => (x, true, false),
                    FieldEdit::Scrub(x, done) => (x, done, true),
                },
            );
            if let Some((x, done, scrub)) = edit
                && x.is_finite()
            {
                let mut nv = vals;
                if let Some(slot) = nv.get_mut(i) {
                    *slot = x;
                }
                let eased = make(nv);
                if scrub {
                    let _ = app.session.begin_interaction("Edit");
                    let _ = app.session.execute("proto.interaction.set", &json!({"ids": targets_for(app, node), "index": k, "easing": eased}));
                    if done {
                        let _ = app.session.commit_interaction();
                    }
                } else {
                    patch(app, node, k, json!({"easing": eased}));
                }
            }
        }
    });
    if count == 3 {
        widgets::sublabels(ui, tr("Mass · Stiffness · Damping"), None);
    }
}

/// Draw an easing curve in `r` (x = time, y = progress, up). `dot` marks a point in time.
/// The curve preview's dot rests at the end this long each time round (seconds).
const CURVE_HOLD: f64 = 0.6;
/// How many times the curve preview plays when it shows or changes.
const CURVE_PLAYS: f64 = 2.0;

/// Seconds since the curve preview `id` started playing: it starts over when it shows (it wasn't
/// drawn the pass before), when what it shows (`key`) changes and when the pointer comes over it.
fn curve_play(ui: &Ui, id: Id, key: Id, hovered: bool) -> f64 {
    let now = ui.input(|i| i.time);
    let pass = ui.ctx().cumulative_pass_nr();
    let since = match ui.data(|d| d.get_temp::<(Id, f64, bool, u64)>(id)) {
        Some((k, since, was_hovered, drawn)) if k == key && (was_hovered || !hovered) && drawn + 1 >= pass => since,
        _ => now,
    };
    ui.data_mut(|d| d.insert_temp(id, (key, since, hovered, pass)));
    now - since
}

pub fn curve(p: &egui::Painter, r: Rect, e: Easing, col: Color32, dot: Option<f64>) {
    let n = 48;
    let at = |x: f64| {
        let y = e.eval(x);
        pos2(r.min.x + r.width() * x as f32, r.max.y - r.height() * y as f32)
    };
    let pts: Vec<Pos2> = (0..=n).map(|i| at(f64::from(i) / f64::from(n))).collect();
    p.with_clip_rect(r.expand(r.height() * 0.6)).add(Shape::line(pts, Stroke::new(1.5, col)));
    if let Some(x) = dot {
        p.circle_filled(at(x.clamp(0.0, 1.0)), 3.0, col);
    }
}

fn scroll_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], node: &Node) {
    let t = Tokens::get(ui.ctx());
    let in_frame = node.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.kind.is_frame_like());
    let is_frame = node.kind.is_frame_like();
    if !in_frame && !is_frame {
        return;
    }
    widgets::section_header(ui, tr("Scroll behavior"), &[], false);
    if in_frame {
        row(ui, 32.0, |ui| {
            widgets::label(ui, tr("Position"), t.text_secondary);
            let cur = match node.scroll_position {
                ScrollPosition::Scrolls => tr("Scroll with parent"),
                ScrollPosition::Fixed => tr("Fixed"),
                ScrollPosition::Sticky => tr("Sticky"),
            };
            let resp = widgets::dropdown(ui, cur, ui.available_width(), None);
            menu_popup(&resp).show(|ui| {
                for (l, v) in [(tr("Scroll with parent"), "scrolls"), (tr("Fixed"), "fixed"), (tr("Sticky"), "sticky")] {
                    if menu_item(ui, l, "") {
                        app.set_props(sel, json!({"scrollPosition": v}));
                        ui.close();
                    }
                }
            });
        });
    }
    if is_frame {
        row(ui, 32.0, |ui| {
            widgets::label(ui, tr("Overflow"), t.text_secondary);
            let cur = match node.overflow {
                Overflow::None => tr("No scrolling"),
                Overflow::Horizontal => tr("Horizontal"),
                Overflow::Vertical => tr("Vertical"),
                Overflow::Both => tr("Both directions"),
            };
            let resp = widgets::dropdown(ui, cur, ui.available_width(), None);
            menu_popup(&resp).show(|ui| {
                for (l, v) in
                    [(tr("No scrolling"), "none"), (tr("Horizontal"), "horizontal"), (tr("Vertical"), "vertical"), (tr("Both directions"), "both")]
                {
                    if menu_item(ui, l, "") {
                        app.set_props(sel, json!({"overflow": v}));
                        ui.close();
                    }
                }
            });
        });
    }
}
