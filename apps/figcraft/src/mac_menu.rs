//! The main menu in the macOS menu bar (craft-menu): installed with the window, synced each frame.

use craft_menu::{Layout, LayoutOptions, NativeMenu};
use figcraft_ui_egui::FigApp;
use figcraft_ui_egui::windows::with_app;

/// The Mac layout of the main menu, with the labels the layout writes translated.
fn layout(fig: &FigApp, windows: bool) -> Layout {
    let label = |english: &'static str| figcraft_ui_egui::i18n::tr(english).to_string();
    craft_menu::mac_layout(&figcraft_ui_egui::native_menu::menu_bar(fig, windows), &LayoutOptions { label: &label, ..LayoutOptions::new("FigCraft") })
}

/// Put the main menu in the menu bar. `None` when AppKit refused it: the in-window menu stays.
pub fn install(ctx: &egui::Context) -> Option<NativeMenu> {
    let bar = with_app(|fig| layout(fig, !ctx.embed_viewports()).bar)?;
    craft_menu::mac::install(ctx, &bar)
}

/// One frame: run the items chosen since the last one, then bring the menus up to date (laid out
/// again only after input or a menu event).
pub fn frame(menu: &mut NativeMenu, ctx: &egui::Context) {
    for id in menu.take_clicks(ctx) {
        with_app(|fig| figcraft_ui_egui::native_menu::run(fig, &id));
    }
    let state = if craft_menu::had_input(ctx) { ctx.cumulative_frame_nr() } else { 0 };
    let windows = !ctx.embed_viewports();
    let clashes = menu.sync_with(state, || {
        with_app(|fig| layout(fig, windows)).unwrap_or_else(|| Layout { bar: craft_menu::MenuBar { menus: Vec::new() }, clashes: Vec::new() })
    });
    for c in clashes {
        log::info!("menu: {} is {} on macOS and {} in FigCraft; {}", c.shortcut, c.system, c.command, c.resolution);
    }
    menu.set_text_editing(ctx.egui_wants_keyboard_input());
}
