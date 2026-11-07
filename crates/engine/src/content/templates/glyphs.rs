//! Original icons for the starter templates: each is one outline drawn for FigCraft from straight
//! lines, arcs and simple curves on a 24 px grid (2 px margin, 2 px round strokes).

/// An icon: its name and its outline as SVG path data on the 24 px grid.
pub(super) struct Glyph {
    pub name: &'static str,
    pub path: &'static str,
}

macro_rules! glyphs {
    ($($id:ident $name:literal $path:literal)*) => {
        $(pub(super) const $id: Glyph = Glyph { name: $name, path: $path };)*
        /// Every icon, in the order the icon set lays them out.
        pub(super) const ALL: &[Glyph] = &[$($id),*];
    };
}

glyphs! {
    HOME "Home" "M4 11 L12 4 L20 11 V20 H4 Z M10 20 V14 H14 V20"
    SEARCH "Search" "M5 11 a6 6 0 1 0 12 0 a6 6 0 1 0 -12 0 M15.5 15.5 L20 20"
    USER "User" "M8 8 a4 4 0 1 0 8 0 a4 4 0 1 0 -8 0 M4 20 C4 16.5 7.5 14.5 12 14.5 C16.5 14.5 20 16.5 20 20"
    BELL "Bell" "M6 17 V11 C6 7.5 8.5 5 12 5 C15.5 5 18 7.5 18 11 V17 M4 17 H20 M10.5 20 H13.5"
    HEART "Heart" "M12 20 L4.5 12.5 C2.5 10.5 2.5 7.5 4.5 5.8 C6.5 4 9.5 4.5 12 7.5 C14.5 4.5 17.5 4 19.5 5.8 C21.5 7.5 21.5 10.5 19.5 12.5 Z"
    STAR "Star" "M12 3.5 L14.6 9 L20.5 9.8 L16.2 14 L17.3 20 L12 17.1 L6.7 20 L7.8 14 L3.5 9.8 L9.4 9 Z"
    PIN "Pin" "M12 21 C9 18 5 14 5 10 C5 6 8 3 12 3 C16 3 19 6 19 10 C19 14 15 18 12 21 Z M9.5 10 a2.5 2.5 0 1 0 5 0 a2.5 2.5 0 1 0 -5 0"
    MOUNTAIN "Mountain" "M3 19 L9 7 L13 14 L16 10 L21 19 Z"
    CLOCK "Clock" "M4 12 a8 8 0 1 0 16 0 a8 8 0 1 0 -16 0 M12 7.5 V12 L15 14"
    CALENDAR "Calendar" "M4 6 H20 V20 H4 Z M4 10.5 H20 M8 4 V8 M16 4 V8"
    MAIL "Mail" "M3 6 H21 V18 H3 Z M3 7 L12 13.5 L21 7"
    CHART "Chart" "M4 20 H20 M7 20 V12 M12 20 V6 M17 20 V10"
    LAYERS "Layers" "M12 4 L20 8.5 L12 13 L4 8.5 Z M4 12.5 L12 17 L20 12.5 M4 16.5 L12 21 L20 16.5"
    SLIDERS "Sliders" "M4 7 H10 M14 7 H20 M10 7 a2 2 0 1 0 4 0 a2 2 0 1 0 -4 0 M4 17 H8 M12 17 H20 M8 17 a2 2 0 1 0 4 0 a2 2 0 1 0 -4 0"
    BOOKMARK "Bookmark" "M7 4 H17 V20 L12 16 L7 20 Z"
    DOWNLOAD "Download" "M12 4 V15 M7 10.5 L12 15.5 L17 10.5 M5 20 H19"
    PLUS "Plus" "M12 5 V19 M5 12 H19"
    CHECK "Check" "M5 12.5 L10 17.5 L19 7"
    CLOSE "Close" "M6 6 L18 18 M18 6 L6 18"
    MENU "Menu" "M4 7 H20 M4 12 H20 M4 17 H20"
    ARROW_LEFT "Arrow left" "M19 12 H5 M11 6 L5 12 L11 18"
    ARROW_RIGHT "Arrow right" "M5 12 H19 M13 6 L19 12 L13 18"
    CHEVRON_RIGHT "Chevron right" "M9 6 L15 12 L9 18"
    CHEVRON_DOWN "Chevron down" "M6 9 L12 15 L18 9"
}
