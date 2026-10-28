//! FigCraft desktop app binary: a shim over the app library (`lib.rs`, [`figcraft::run`]).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() -> eframe::Result {
    figcraft::run()
}
