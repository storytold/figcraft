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
use std::sync::Arc;

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

/// What an evaluated preview was built from. The canvas preview is rebuilt only when one of these
/// changes, so the same frame keeps handing the canvas the same document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preview {
    /// The document's revision, which every edit bumps.
    pub revision: u64,
    pub page: NodeId,
    /// The playhead, as raw bits (a float has no `Eq`).
    pub playhead: u64,
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
    /// A timeline edit has opened an undo step that has not been closed yet.
    pub gesture_open: bool,
    /// The page evaluated at the playhead and what it was built from.
    pub preview: Option<(Preview, Arc<figcraft_doc::Document>)>,
}

/// Runs `f` as one undo step.
pub fn edited<R>(app: &mut FigApp, f: impl FnOnce(&mut FigApp) -> R) -> R {
    let _ = app.session.begin_interaction("Motion");
    let out = f(app);
    let _ = app.session.commit_interaction();
    out
}
