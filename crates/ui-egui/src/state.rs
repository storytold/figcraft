#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RightTab {
    Design,
    Prototype,
    Inspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum LeftTab {
    File,
    Assets,
}

/// The window's title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TitleBar {
    /// The system draws it (and the browser): the tab strip sits below.
    #[default]
    Native,
    /// A frameless window: the tab strip is the title bar, with our own window buttons and edges.
    Custom,
    /// macOS: the system's window buttons inset at the left of the tab strip.
    Inset,
}

/// A file being encoded for auto-save: its snapshot and, once done, its bytes.
pub type AutoSaveEncoding = std::thread::JoinHandle<(figcraft_engine::cmd::file::FileSnapshot, figcraft_engine::Result<Vec<u8>>)>;

/// The side panels' widths: the least and most a drag of their edge gives, what a double-click on
/// the edge puts back, and how much of the window they leave the canvas.
pub const LEFT_PANEL: std::ops::RangeInclusive<f32> = 200.0..=480.0;

pub const RIGHT_PANEL: std::ops::RangeInclusive<f32> = 240.0..=480.0;

pub const PANEL_DEFAULT: f32 = 240.0;

/// Windows narrower than this (logical points: phones, portrait tablets) show one side panel at a
/// time over the canvas, opened from two buttons, instead of both beside it.
pub const NARROW_WIDTH: f32 = 760.0;

/// What an open side panel leaves of a narrow window: the column of panel buttons.
pub(crate) const MOBILE_STRIP: f32 = 48.0;

/// The side panel a narrow window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MobilePanel {
    Left,
    Right,
}

pub const CANVAS_MIN: f32 = 160.0;

/// The widths the side panels are drawn at in `avail` points (the window less the icon rail), from
/// the widths asked for (none while a panel is closed, which takes no room): each within its
/// limits, and narrower in a small window so the canvas keeps [`CANVAS_MIN`] (the right panel
/// gives way last).
pub fn fit_panels(avail: f32, left: Option<f32>, right: Option<f32>) -> (f32, f32) {
    let fit = |w: f32, range: &std::ops::RangeInclusive<f32>, room: f32| {
        let w = if w.is_finite() { w } else { PANEL_DEFAULT };
        w.clamp(*range.start(), room.clamp(*range.start(), *range.end()))
    };
    let left_min = left.map_or(0.0, |_| *LEFT_PANEL.start());
    let right = right.map_or(0.0, |r| fit(r, &RIGHT_PANEL, avail - left_min - CANVAS_MIN));
    (left.map_or(0.0, |l| fit(l, &LEFT_PANEL, avail - right - CANVAS_MIN)), right)
}
