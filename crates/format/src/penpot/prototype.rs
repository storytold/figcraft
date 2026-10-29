//! Penpot prototype interactions and flows, mapped to FigCraft's native prototype model.
//!
//! Penpot stores an interaction on the shape that fires it: an event type (`click`,
//! `mouse-press`, `mouse-over`, `mouse-enter`, `mouse-leave`, `after-delay`), an action
//! type (`navigate`, `open-overlay`, `toggle-overlay`, `close-overlay`, `prev-screen`,
//! `open-url`), the destination shape and an optional animation. FigCraft models the same
//! idea as an [`Interaction`] (a trigger plus a list of actions) and a [`Flow`] (a named
//! starting frame on a page).
//!
//! A destination is a Penpot shape id, but a FigCraft action points at a node id. The
//! importer therefore keeps the Penpot source of every interaction in the layer's metadata
//! and reads it back once every page is in the document, when the Penpot ids can be
//! resolved to node ids (see [`resolve_destinations`]).

use std::collections::BTreeMap;

use figcraft_doc::color::Color;
use figcraft_doc::proto::{Action, Direction, Easing, Flow, Interaction, OverlayPosition, Transition, Trigger};
use figcraft_doc::{Document, NodeId};
use serde_json::Value;

use super::warn;

/// Longest animation Penpot may declare, in milliseconds.
const MAX_DURATION_MS: u32 = 600_000;
/// Longest "after delay" wait, in milliseconds.
const MAX_DELAY_MS: u32 = 600_000;
/// Most interactions read from one shape.
const MAX_INTERACTIONS: usize = 64;
/// Most flows a page may declare (Penpot allows two).
const MAX_FLOWS: usize = 8;
/// Longest stored interaction source for one shape, in bytes.
const MAX_INTERACTION_SOURCE: usize = 64 * 1024;

/// Penpot's `event-type` names, mapped to FigCraft's triggers.
fn penpot_trigger(event: Option<&str>, delay_ms: Option<u32>) -> Option<Trigger> {
    match event? {
        "click" => Some(Trigger::OnClick),
        // "mouse-press" fires while the pointer is held: FigCraft's "while pressing".
        "mouse-press" => Some(Trigger::OnPress),
        // "mouse-over" fires while the pointer is over the shape.
        "mouse-over" => Some(Trigger::OnHover),
        "mouse-enter" => Some(Trigger::MouseEnter),
        "mouse-leave" => Some(Trigger::MouseLeave),
        // Penpot's own default wait for a delayed trigger.
        "after-delay" => Some(Trigger::AfterDelay { ms: delay_ms.unwrap_or(600).min(MAX_DELAY_MS) }),
        _ => None,
    }
}

/// Penpot's `overlay-pos-type` names, mapped to FigCraft's overlay positions.
fn penpot_overlay_position(position: Option<&str>) -> Option<OverlayPosition> {
    Some(match position? {
        "manual" => OverlayPosition::Manual,
        "center" => OverlayPosition::Center,
        "top-left" => OverlayPosition::TopLeft,
        "top-right" => OverlayPosition::TopRight,
        "top-center" => OverlayPosition::TopCenter,
        "bottom-left" => OverlayPosition::BottomLeft,
        "bottom-right" => OverlayPosition::BottomRight,
        "bottom-center" => OverlayPosition::BottomCenter,
        _ => return None,
    })
}

/// What a Penpot action does, without the destination it points at.
enum PenpotActionKind {
    Navigate,
    OpenOverlay,
    ToggleOverlay,
    CloseOverlay,
    Back,
    OpenUrl,
}

/// Penpot's `action-type` names.
fn penpot_action_kind(action: Option<&str>) -> Option<PenpotActionKind> {
    Some(match action? {
        "navigate" => PenpotActionKind::Navigate,
        "open-overlay" => PenpotActionKind::OpenOverlay,
        "toggle-overlay" => PenpotActionKind::ToggleOverlay,
        "close-overlay" => PenpotActionKind::CloseOverlay,
        "prev-screen" => PenpotActionKind::Back,
        "open-url" => PenpotActionKind::OpenUrl,
        _ => return None,
    })
}

/// Whether the action opens an overlay (Penpot plays its animation differently).
fn is_overlay(kind: &PenpotActionKind) -> bool {
    matches!(kind, PenpotActionKind::OpenOverlay | PenpotActionKind::ToggleOverlay)
}

/// Penpot's animation kinds, mapped to FigCraft's transitions.
///
/// Penpot animates a navigation with `dissolve`, `slide` or `push`; FigCraft's `SlideIn`
/// and `SlideOut` are the closest to Penpot's `slide`, whose `way` says whether the new
/// screen comes in or the old one goes out.
fn penpot_transition(animation: Option<&Value>, overlay: bool) -> (Transition, u32, Easing) {
    let Some(animation) = animation else { return (Transition::Instant, 0, Easing::Linear) };
    let kind = animation.get("animationType").and_then(Value::as_str);
    let duration = animation
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| d.min(f64::from(MAX_DURATION_MS)) as u32)
        .unwrap_or(0);
    let easing = match animation.get("easing").and_then(Value::as_str) {
        Some("linear") => Easing::Linear,
        Some("ease") => Easing::EaseInOut,
        Some("ease-in") => Easing::EaseIn,
        Some("ease-out") => Easing::EaseOut,
        Some("ease-in-out") => Easing::EaseInOut,
        _ => Easing::Linear,
    };
    // Penpot's direction names the way the screen moves, as FigCraft's does.
    let direction = match animation.get("direction").and_then(Value::as_str) {
        Some("right") => Direction::Right,
        Some("up") => Direction::Top,
        Some("down") => Direction::Bottom,
        _ => Direction::Left,
    };
    // An overlay's animation is the overlay coming in: its "way" does not apply, and a
    // push is not an overlay animation at all.
    let way = if overlay { Some("in") } else { animation.get("way").and_then(Value::as_str) };
    let transition = match (kind, way) {
        (Some("dissolve"), _) => Transition::Dissolve,
        (Some("slide"), Some("out")) => Transition::SlideOut { direction },
        (Some("slide"), _) => Transition::SlideIn { direction },
        (Some("push"), _) if !overlay => Transition::Push { direction },
        _ => Transition::Instant,
    };
    (transition, duration, easing)
}

/// One Penpot interaction, read from a shape.
pub(super) struct PenpotInteraction {
    /// The trigger that fires it.
    pub trigger: Trigger,
    /// The action it runs. Its destination is a placeholder until the ids are resolved.
    pub action: Action,
    /// The Penpot shape id the action points at, if any.
    pub destination: Option<String>,
    /// The transition it plays.
    pub transition: Transition,
    /// How long the transition lasts, in milliseconds.
    pub duration_ms: u32,
    /// The curve the transition follows.
    pub easing: Easing,
}

/// The action a Penpot interaction runs, with its destination left for the resolver.
fn penpot_action(item: &Value, kind: &PenpotActionKind) -> Option<(Action, Option<String>)> {
    let destination = item.get("destination").and_then(Value::as_str).map(str::to_owned);
    let action = match kind {
        PenpotActionKind::Navigate => {
            Action::Navigate { dest: NodeId(0), preserve_scroll: item.get("preserveScroll").and_then(Value::as_bool).unwrap_or(false) }
        }
        PenpotActionKind::ToggleOverlay => return None,
        PenpotActionKind::OpenOverlay => {
            let position = penpot_overlay_position(item.get("overlayPosType").and_then(Value::as_str))?;
            // A manual overlay sits at an explicit offset from the layer it was opened
            // from. FigCraft measures that offset from the top-left of the opening layer.
            let offset = match (position, item.get("overlayPosition")) {
                (OverlayPosition::Manual, Some(p)) => {
                    let x = p.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                    let y = p.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                    (x.is_finite() && y.is_finite()).then_some([x.clamp(-100_000.0, 100_000.0), y.clamp(-100_000.0, 100_000.0)])
                }
                _ => None,
            };
            // Penpot leaves "close when clicking outside" off unless it is asked for.
            let close_on_outside = item.get("closeClickOutside").and_then(Value::as_bool).unwrap_or(false);
            // Penpot draws a translucent black sheet behind an overlay that asks for one.
            let background = item.get("backgroundOverlay").and_then(Value::as_bool).unwrap_or(false).then(|| Color::new(0.0, 0.0, 0.0, 0.2));
            Action::OpenOverlay { dest: NodeId(0), position, close_on_outside, background, offset, anchor: Default::default() }
        }
        PenpotActionKind::CloseOverlay => Action::CloseOverlay,
        // Penpot's "previous screen" is the player's back action.
        PenpotActionKind::Back => Action::Back,
        PenpotActionKind::OpenUrl => {
            let url = item.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() || url.len() > 2048 {
                return None;
            }
            Action::OpenUrl { url: url.chars().take(2048).collect() }
        }
    };
    let needs_destination = matches!(kind, PenpotActionKind::Navigate | PenpotActionKind::OpenOverlay | PenpotActionKind::ToggleOverlay);
    Some((action, if needs_destination { destination } else { None }))
}

/// Map one shape's `interactions` array.
///
/// Returns the interactions that could be mapped, and a note for each one that could not.
pub(super) fn shape_interactions(items: &[Value], warnings: &mut Vec<String>) -> Vec<PenpotInteraction> {
    let mut out = Vec::new();
    for item in items.iter().take(MAX_INTERACTIONS) {
        let event = item.get("eventType").and_then(Value::as_str);
        let Some(trigger) = penpot_trigger(event, item.get("delay").and_then(Value::as_u64).map(|d| d.min(u64::from(MAX_DELAY_MS)) as u32)) else {
            warnings.push(format!("Penpot interaction with event type '{}' has no FigCraft equivalent and was skipped", event.unwrap_or("unknown")));
            continue;
        };
        let Some(kind) = penpot_action_kind(item.get("actionType").and_then(Value::as_str)) else {
            warnings.push(format!(
                "Penpot interaction with action type '{}' has no FigCraft equivalent and was skipped",
                item.get("actionType").and_then(Value::as_str).unwrap_or("unknown")
            ));
            continue;
        };
        let Some((action, destination)) = penpot_action(item, &kind) else {
            warnings.push(format!(
                "Penpot interaction with action type '{}' has no FigCraft equivalent and was skipped",
                item.get("actionType").and_then(Value::as_str).unwrap_or("unknown")
            ));
            continue;
        };
        if destination.is_none() && matches!(kind, PenpotActionKind::Navigate | PenpotActionKind::OpenOverlay | PenpotActionKind::ToggleOverlay) {
            warnings.push("A Penpot interaction names no destination; it was skipped".into());
            continue;
        }
        let (transition, duration_ms, easing) = penpot_transition(item.get("animation"), is_overlay(&kind));
        out.push(PenpotInteraction { trigger, action, destination, transition, duration_ms, easing });
    }
    out
}

/// Keep a shape's interaction source in its metadata, so the destinations can be resolved
/// once every page is in the document.
pub(super) fn keep_interaction_source(node: &mut figcraft_doc::Node, source: &Value, warnings: &mut Vec<String>) {
    let Some(items) = source.get("interactions").and_then(Value::as_array) else { return };
    if items.is_empty() {
        return;
    }
    match serde_json::to_string(items) {
        Ok(encoded) if encoded.len() <= MAX_INTERACTION_SOURCE => {
            node.plugin_data.insert("penpot.interactions".into(), encoded);
        }
        _ => warn(warnings, "A Penpot layer's interaction source is too large to keep; its prototype links were skipped".into()),
    }
}

/// The interactions a shape's kept source holds.
fn stored_interactions(node: &figcraft_doc::Node) -> Option<Vec<Value>> {
    let raw = node.plugin_data.get("penpot.interactions")?;
    serde_json::from_str::<Value>(raw).ok()?.as_array().cloned()
}

/// Rewrite the placeholder destinations of the imported interactions once every page is in
/// the document.
///
/// The importer keeps each shape's Penpot interactions in its metadata. Once the document
/// is complete the Penpot ids are replaced by the FigCraft node ids they name, so the
/// prototype player can follow them. A destination that names no layer is dropped, and an
/// interaction left with no action is removed.
pub(super) fn resolve_destinations(doc: &mut Document, by_penpot_id: &BTreeMap<String, NodeId>, warnings: &mut Vec<String>) {
    let mut mapped = 0usize;
    let mut missing = 0usize;
    let mut updates: Vec<(NodeId, Vec<Interaction>)> = Vec::new();
    for node in doc.nodes() {
        let Some(items) = stored_interactions(node) else { continue };
        let mut out = Vec::new();
        let mut notes = Vec::new();
        let interactions = shape_interactions(&items, &mut notes);
        for note in notes {
            warn(warnings, note);
        }
        for interaction in interactions {
            let Some(destination) = &interaction.destination else {
                // An action with no destination (a link, a back) needs no resolution.
                out.push(Interaction {
                    trigger: interaction.trigger,
                    actions: vec![interaction.action],
                    transition: interaction.transition,
                    duration_ms: interaction.duration_ms,
                    easing: interaction.easing,
                    delay_ms: 0,
                });
                continue;
            };
            let Some(&id) = by_penpot_id.get(destination) else {
                missing += 1;
                continue;
            };
            out.push(Interaction {
                trigger: interaction.trigger,
                actions: vec![replace_dest(interaction.action, id)],
                transition: interaction.transition,
                duration_ms: interaction.duration_ms,
                easing: interaction.easing,
                delay_ms: 0,
            });
            mapped += 1;
        }
        if !out.is_empty() {
            updates.push((node.id, out));
        }
    }
    for (id, interactions) in updates {
        if let Some(node) = doc.node_mut(id) {
            node.interactions = interactions;
        }
    }
    if mapped > 0 {
        warnings.push(format!("Imported {mapped} Penpot prototype connection(s) as working FigCraft interactions"));
    }
    if missing > 0 {
        warnings.push(format!("{missing} Penpot prototype destination(s) named no layer in the file; those connections were skipped"));
    }
}

/// The same action with another destination.
fn replace_dest(action: Action, dest: NodeId) -> Action {
    match action {
        Action::Navigate { preserve_scroll, .. } => Action::Navigate { dest, preserve_scroll },
        Action::ScrollTo { offset, .. } => Action::ScrollTo { dest, offset },
        Action::OpenOverlay { position, close_on_outside, background, offset, anchor, .. } => {
            Action::OpenOverlay { dest, position, close_on_outside, background, offset, anchor }
        }
        Action::SwapOverlay { .. } => Action::SwapOverlay { dest },
        Action::ChangeTo { .. } => Action::ChangeTo { dest },
        other => other,
    }
}

/// Read a page's Penpot `flows` and turn them into FigCraft flows.
///
/// A Penpot flow names the frame the prototype starts at. FigCraft's flow needs a node id,
/// so the Penpot frame id is resolved through `by_penpot_id`.
pub(super) fn page_flows(doc: &Document, flows: &Value, by_penpot_id: &BTreeMap<String, NodeId>, warnings: &mut Vec<String>) -> Vec<Flow> {
    let mut out = Vec::new();
    let Some(flows) = flows.as_object() else { return out };
    for flow in flows.values().take(MAX_FLOWS) {
        let Some(start) = flow.get("startingFrame").and_then(Value::as_str) else { continue };
        let Some(&frame) = by_penpot_id.get(start) else {
            // A flow whose starting frame is not in the file cannot start anything.
            warn(warnings, "A Penpot flow names a starting frame that is not in the file; it was skipped".into());
            continue;
        };
        // The player shows the top-level frame a starting frame sits in.
        let start = doc.top_level_frame(frame).unwrap_or(frame);
        let name = flow.get("name").and_then(Value::as_str).map(str::to_owned).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Flow".to_owned());
        out.push(Flow { name: name.chars().take(256).collect(), start, description: String::new() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_flow_warnings_are_deduplicated_and_bounded_across_pages() {
        let doc = Document::new("Missing flows");
        let flows = serde_json::json!({"a":{"startingFrame":"missing"},"b":{"startingFrame":"missing"}});
        let mut warnings = Vec::new();
        for _ in 0..110 {
            assert!(page_flows(&doc, &flows, &BTreeMap::new(), &mut warnings).is_empty());
        }
        assert_eq!(warnings.len(), 1);
        warnings = (0..100).map(|i| format!("Prior warning {i}")).collect();
        page_flows(&doc, &flows, &BTreeMap::new(), &mut warnings);
        assert_eq!(warnings.len(), 100);
    }
}
