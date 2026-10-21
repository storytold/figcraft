//! The main menu in the macOS menu bar (craft-menu): installed with the window, synced each frame,
//! acting on the window in front.

use craft_menu::{Layout, LayoutOptions, NativeMenu};
use egui::ViewportId;
use figcraft_ui_egui::FigApp;
use figcraft_ui_egui::native_menu;
use figcraft_ui_egui::windows::with_app;

/// The Mac layout of the main menu for the window in front, with the labels the layout writes
/// translated.
fn layout(fig: &mut FigApp, windows: bool) -> Layout {
    let label = |english: &'static str| native_menu::mac_label(english);
    craft_menu::mac_layout(&native_menu::menu_bar_in_front(fig, windows), &LayoutOptions { label: &label, ..LayoutOptions::new("FigCraft") })
}

/// Put the main menu in the menu bar. `None` when AppKit refused it: the in-window menu stays.
pub fn install(ctx: &egui::Context) -> Option<NativeMenu> {
    let bar = with_app(|fig| layout(fig, !ctx.embed_viewports()).bar)?;
    craft_menu::mac::install(ctx, &bar)
}

/// Before a window's frame: the menu bar's key equivalents go into the window in front's input
/// only. In the main window's input while another window is in front, a paste the menu bar asked
/// for goes on to that window.
pub fn raw_input(menu: &mut NativeMenu, ctx: &egui::Context, raw: &mut egui::RawInput) {
    let front = with_app(|fig| native_menu::front(fig)).unwrap_or(ViewportId::ROOT);
    if raw.viewport_id == front {
        menu.raw_input(raw);
    } else if raw.viewport_id == ViewportId::ROOT {
        let pastes = native_menu::take_pastes(&mut raw.events);
        if !pastes.is_empty() {
            with_app(|fig| native_menu::send_keys(fig, ctx, pastes));
        }
    }
}

/// One frame (the main window's): hand the window in front the key equivalents waiting for it, run
/// the items chosen since the last frame there, then bring the menus up to date (laid out again
/// only after input, a menu event or another window coming to the front).
pub fn frame(menu: &mut NativeMenu, ctx: &egui::Context) {
    let front = with_app(|fig| native_menu::front(fig)).unwrap_or(ViewportId::ROOT);
    if front != ViewportId::ROOT {
        // A menu event wakes only the main window: the window in front may not draw until its next input.
        let mut keys = egui::RawInput { viewport_id: front, ..Default::default() };
        menu.raw_input(&mut keys);
        if !keys.events.is_empty() {
            with_app(|fig| native_menu::send_keys(fig, ctx, keys.events));
        }
    }
    for id in menu.take_clicks(ctx) {
        with_app(|fig| native_menu::run_in_front(fig, &id));
    }
    let input = if craft_menu::had_input(ctx) { ctx.cumulative_frame_nr() } else { 0 };
    let state = input ^ front.0.value();
    let windows = !ctx.embed_viewports();
    let clashes = menu.sync_with(state, || {
        with_app(|fig| layout(fig, windows)).unwrap_or_else(|| Layout { bar: craft_menu::MenuBar { menus: Vec::new() }, clashes: Vec::new() })
    });
    for c in clashes {
        log::info!("menu: {} is {} on macOS and {} in FigCraft; {}", c.shortcut, c.system, c.command, c.resolution);
    }
    menu.set_text_editing(ctx.egui_wants_keyboard_input());
}
