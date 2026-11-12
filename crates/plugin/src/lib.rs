//! FigCraft plugins: the manifest (`figcraft-plugin.json`) and the WebAssembly sandbox a plugin
//! runs in, and the widget descriptions a plugin hands back. The engine decides what a plugin may
//! do (its permissions, which commands it may call, one undo step per run, panel or widget
//! callback, where settings are kept); this crate only runs modules safely: a metered interpreter
//! with a memory cap, no WASI and a handful of host functions. See `docs/plugins.md`.

pub mod bundle;
pub mod manifest;
mod sandbox;
pub mod widget;

pub use manifest::{API_VERSION, Access, MANIFEST_FILE, Manifest, Panel, Param, ParamKind, Permissions, PluginCommand};
pub use sandbox::{ENTRY, Host, IMPORT_MODULE, Instance, Limits, PluginError, RunOutput, UiChange, check_module, run};
pub use widget::{MenuItem, MenuKind, WidgetHover, WidgetKind, WidgetNode, WidgetRender};

#[cfg(test)]
mod bundle_tests;
#[cfg(test)]
mod save_file_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod v2_tests;
#[cfg(test)]
mod widget_tests;
