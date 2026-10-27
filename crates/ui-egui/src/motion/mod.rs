//! Motion (Beta) mode: the animation timeline under the canvas.
//!
//! State shared by the timeline and its future inspector lives in [`MotionUi`]; the timeline itself
//! is [`timeline`]. The keyframe model is `figcraft_doc::anim::motion`.

pub mod easing;
pub mod inspector;
pub mod timeline;

use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::AnimProperty;
use std::collections::BTreeSet;

use crate::FigApp;

/// Opens or closes Motion mode, widening the right panel to the inspector while it is open.
pub fn set_open(app: &mut FigApp, open: bool) {
    if open == app.ui.motion_open {
        return;
    }
    if open {
        app.ui.motion_right_width = Some(app.ui.right_width);
        app.ui.right_width = inspector::WIDTH;
        app.ui.motion.playing = false;
    } else if let Some(w) = app.ui.motion_right_width.take() {
        app.ui.right_width = w;
    }
    app.ui.motion_open = open;
}

/// A keyframe the timeline has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedKey {
    pub node: NodeId,
    pub property: AnimProperty,
    pub time_ms: u32,
}

/// Timeline state kept between frames.
#[derive(Default)]
pub struct MotionUi {
    /// The playhead, in milliseconds (may sit between keyframes).
    pub playhead_ms: f64,
    pub playing: bool,
    pub selected: Option<SelectedKey>,
    /// Layers whose property and clip rows are folded away.
    pub folded: BTreeSet<NodeId>,
    /// The inspector's Animations (preset library) section is open.
    pub presets_open: bool,
    /// Vertical scroll of the timeline rows.
    pub scroll_y: f32,
    /// The playhead is being dragged in the ruler (the canvas must keep previewing).
    pub scrubbing: bool,
    /// Live duration while its edge is dragged (committed on release).
    pub drag_duration: Option<u32>,
    /// The clip row clicked in the timeline, shown in the inspector.
    pub selected_clip: Option<(NodeId, u32)>,
    /// The inspector shows the easing panel for the selected segment.
    pub easing_target: bool,
}
