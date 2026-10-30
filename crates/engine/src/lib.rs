//! The FigCraft engine.
//!
//! Every user-visible action is a command with a stable id (`node.create`, `align.left`,
//! `layout.add`, `component.create`…) taking JSON params. The egui UI, the CLI, the control channel
//! and the MCP server all go through [`Session::execute`], so everything is scriptable and every
//! change is one undo step. Pointer tools ([`tools`]) and text editing reduce to the same document
//! edits and are journaled the same way.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// Tests make and inspect files directly; shipped code goes through `file_access` (clippy.toml).
#![cfg_attr(test, allow(clippy::disallowed_methods))]

mod assets;
mod automation;
mod boards;
mod canvas;
pub mod catalog;
pub mod cmd;
mod content;
mod dev;
pub mod events;
mod extensions;
pub mod fonts;
pub mod guard;
pub mod keymap;
pub mod measure;
pub mod proto_run;
mod selection;
mod sharing;
pub use assets::components;
pub use assets::library;
pub use assets::library_server;
pub use assets::library_swap;
pub use assets::playground;
pub use assets::style_vars;
pub use assets::token_roles;
pub use automation::batch;
pub use automation::doc_urls;
pub use automation::file_access;
pub use automation::inspect;
pub use automation::params;
pub use boards::board;
pub use boards::board_grid;
pub use boards::board_table;
pub use boards::board_templates;
pub use canvas::al_keys;
pub use canvas::arc_handles;
pub use canvas::booleans;
pub use canvas::distance;
pub use canvas::drop;
pub use canvas::guides;
pub use canvas::rotation_origin;
pub use canvas::shape_handles;
pub use canvas::shapebuilder;
pub use canvas::snap;
pub use canvas::textedit;
pub use canvas::textpreview;
pub use canvas::tools;
pub use canvas::vectoredit;
pub use content::datafill;
pub use content::synthetic;
pub use content::synthetic_ds;
pub use content::templates;
pub use content::textfit;
pub use dev::annotations;
pub use dev::code_connect;
pub use dev::codegen;
pub use dev::component_codegen;
pub use dev::dev_measurements;
pub use dev::dev_resources;
pub use dev::devcompare;
pub use dev::devinspect;
pub use dev::tailwind;
pub use extensions::plugin_directory;
pub use extensions::plugins;
pub use extensions::widgets;
pub use selection::rename;
pub use selection::selection_colors;
pub use selection::selection_reorder;
pub use selection::selection_spacing;
pub use sharing::agent;
pub use sharing::background;
pub use sharing::branches;
pub use sharing::collab;
pub use sharing::invite_links;
pub use sharing::live;
pub use sharing::online;

pub use batch::{BatchScope, BatchStep};
pub use cmd::{CommandSpec, all_commands, find_command};
pub use figcraft_collab;
pub use figcraft_doc as doc;
pub use figcraft_svg::looks_like_svg;
pub use tools::{PointerEvent, PointerKind, ToolId};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use figcraft_doc::{Document, Node, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod derived;
mod doc_state;
mod error;
mod prefs;
mod session;
mod view_state;

pub use derived::{changed_nodes, normalize};
pub use doc_state::{DocState, History, HistoryEntry, Interaction, PREVIEW_READ_ONLY, Preedit, TextEdit, VersionPreview};
pub use error::{EngineError, Result, bad};
pub use prefs::{Clipboard, MAX_RECENT_ACTIONS, Prefs, UI_SCALE_MAX, UI_SCALE_MIN, UiRequest, clamp_ui_scale};
pub use session::Forward;
pub use view_state::ViewState;

/// The editing session: open documents, the current tool, clipboard and prefs.
pub struct Session {
    pub docs: Vec<DocState>,
    pub active: usize,
    pub clipboard: Clipboard,
    pub prefs: Prefs,
    pub tool: tools::ToolState,
    pub ui_requests: Vec<UiRequest>,
    /// The interface's theme once the UI knows it ([`Session::set_ui_dark`]): new files get the
    /// dark page colour while it is dark (added pages take the current page's). Headless sessions (CLI, MCP, exports) have no
    /// theme, so what scripts make is the same everywhere.
    pub ui_dark: Option<bool>,
    /// Command log (id + params) for actions/automation. File imports that carry the file bytes
    /// (`file.import` with `data`) are not logged, so the log never holds a copy of a pasted file.
    pub journal: Vec<(String, Value)>,
    /// Nesting depth of [`Session::grouped`]; tabs closed while it is above zero are kept in
    /// `closed_in_group` so a failed group can bring them back.
    group_depth: usize,
    closed_in_group: Vec<DocState>,
    /// Recently used colours.
    pub recent_colors: Vec<figcraft_color::Color>,
    /// The last undoable command executed through [`Session::execute`], excluding "Repeat".
    pub last_command: Option<(String, Value)>,
    /// Status / toast message for the UI.
    pub message: Option<String>,
    /// Library files read for the open documents.
    pub libraries: library::Cache,
    /// Plugin modules loaded in this session and the last plugin run.
    pub plugins: plugins::Runtime,
    /// The live sessions (collaboration), one per document at most: status, peers, presence.
    /// [`Session::live`] is the active document's.
    pub live: live::Lives,
    /// Commands the UI runs without waiting on a server ([`Session::start_background`]).
    pub background: background::Background,
    /// Live agent events (comment changes), shared with the control channel's `events.poll`.
    pub events: Arc<events::EventLog>,
    /// Events recorded inside a group: (kind, document, path, data).
    pending_events: Vec<(String, String, Option<String>, Value)>,
    /// The AI agent the running command is for ([`Session::execute_as`]), as comments are signed
    /// (`"Claude (agent)"`); `None` for the user.
    pub agent: Option<String>,
    /// Where an attached AI agent last worked ([`Session::agent_presence_at`]): display only.
    pub agent_presence: Option<agent::AgentPresence>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod a11y_keys_tests;
#[cfg(test)]
mod agent_presence_tests;
#[cfg(test)]
mod al_behaviour_tests;
#[cfg(test)]
mod al_content_align_tests;
#[cfg(test)]
mod al_drag_tests;
#[cfg(test)]
mod al_keys_tests;
#[cfg(test)]
mod al_nesting_tests;
#[cfg(test)]
mod annotation_categories_tests;
#[cfg(test)]
mod annotations_tests;
#[cfg(test)]
mod arc_fields_tests;
#[cfg(test)]
mod arc_handles_tests;
#[cfg(test)]
mod auto_layout_move_tests;
#[cfg(test)]
mod autosave_tests;
#[cfg(test)]
mod batch_continue_events_tests;
#[cfg(test)]
mod batch_rollback_tests;
#[cfg(test)]
mod batch_step_ref_tests;
#[cfg(test)]
mod batch_undo_snapshot_tests;
#[cfg(test)]
mod binding_lists_tests;
#[cfg(test)]
mod board_connected_tests;
#[cfg(test)]
mod board_connector_tests;
#[cfg(test)]
mod board_grid_tests;
#[cfg(test)]
mod board_tables_tests;
#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod board_tools_tests;
#[cfg(test)]
mod boolean_cap_tests;
#[cfg(test)]
mod booleans_tests;
#[cfg(test)]
mod bound_alpha_tests;
#[cfg(test)]
mod branch_live_tests;
#[cfg(test)]
mod branch_tests;
#[cfg(test)]
mod branch_versions_tests;
#[cfg(test)]
mod brush_tests;
#[cfg(test)]
mod canvas_fixes_tests;
#[cfg(test)]
mod code_connect_tests;
#[cfg(test)]
mod collab_ids_tests;
#[cfg(test)]
mod collab_live_tests;
#[cfg(test)]
mod collab_tests;
#[cfg(test)]
mod collab_undo_tests;
#[cfg(test)]
mod comment_agent_tests;
#[cfg(test)]
mod comment_link_tests;
#[cfg(test)]
mod comment_mentions_tests;
#[cfg(test)]
mod comments_polish_tests;
#[cfg(test)]
mod component_codegen_tests;
#[cfg(test)]
mod component_cycle_tests;
#[cfg(test)]
mod components_behaviour_tests;
#[cfg(test)]
mod components_scale_tests;
#[cfg(test)]
mod convert_container_tests;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod dark_page_text_tests;
#[cfg(test)]
mod datafill_tests;
#[cfg(test)]
mod design_paste_tests;
#[cfg(test)]
mod dev_measurements_tests;
#[cfg(test)]
mod dev_resources_tests;
#[cfg(test)]
mod devcompare_tests;
#[cfg(test)]
mod devfocus_tests;
#[cfg(test)]
mod devinspect_tests;
#[cfg(test)]
mod devmode_behaviour_tests;
#[cfg(test)]
mod devmode_tail_tests;
#[cfg(test)]
mod doc_list_paths_tests;
#[cfg(test)]
mod doc_urls_tests;
#[cfg(test)]
mod double_click_tests;
#[cfg(test)]
mod draw_tools_tests;
#[cfg(test)]
mod duplicate_map_tests;
#[cfg(test)]
mod duplicate_spacing_tests;
#[cfg(test)]
mod effects_tests;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod export_behaviour_tests;
#[cfg(test)]
mod export_bench_tests;
#[cfg(test)]
mod export_depth_tests;
#[cfg(test)]
mod extended_chain_tests;
#[cfg(test)]
mod facilitate_tests;
#[cfg(test)]
mod fig_export_assets_tests;
#[cfg(test)]
mod fig_export_tests;
#[cfg(test)]
mod fig_font_names_tests;
#[cfg(test)]
mod fig_import_tests;
#[cfg(test)]
mod fill_none_tests;
#[cfg(test)]
mod finite_geometry_tests;
#[cfg(test)]
mod frame_guides_tests;
#[cfg(test)]
mod frame_label_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod frames_behaviour_tests;
#[cfg(test)]
mod fx_behaviour_tests;
#[cfg(test)]
mod gesture_preview_tests;
#[cfg(test)]
mod gradient_shorthand_tests;
#[cfg(test)]
mod group_panic_tests;
#[cfg(test)]
mod guide_snap_tests;
#[cfg(test)]
mod hide_publishing_tests;
#[cfg(test)]
mod history_behaviour_tests;
#[cfg(test)]
mod html_codegen_tests;
#[cfg(test)]
mod huge_stroke_tests;
#[cfg(test)]
mod id_params_behaviour_tests;
#[cfg(test)]
mod image_crop_tests;
#[cfg(test)]
mod images_left_in_file_tests;
#[cfg(test)]
mod import_export_tests;
#[cfg(test)]
mod import_journal_tests;
#[cfg(test)]
mod inspect_behaviour_tests;
#[cfg(test)]
mod inspect_fields_tests;
#[cfg(test)]
mod instance_modes_tests;
#[cfg(test)]
mod instance_resizing_tests;
#[cfg(test)]
mod interaction_rollback_tests;
#[cfg(test)]
mod invite_links_tests;
#[cfg(test)]
mod jam_open_tests;
#[cfg(test)]
mod keymap_tests;
#[cfg(test)]
mod library_beacon_tests;
#[cfg(test)]
mod library_path_tests;
#[cfg(test)]
mod library_range_styles_tests;
#[cfg(test)]
mod library_variables_tests;
#[cfg(test)]
mod line_endings_tests;
#[cfg(test)]
mod live_access_tests;
#[cfg(test)]
mod live_docs_tests;
#[cfg(test)]
mod live_peer_cap_tests;
#[cfg(test)]
mod live_share_tests;
#[cfg(test)]
mod live_signal_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod mask_tests;
#[cfg(test)]
mod masks_behaviour_tests;
#[cfg(test)]
mod measure_tests;
#[cfg(test)]
mod menu_gaps_tests;
#[cfg(test)]
mod moved_by_tests;
#[cfg(test)]
mod nested_override_tests;
#[cfg(test)]
mod node_find_tests;
#[cfg(test)]
mod node_set_refusals_tests;
#[cfg(test)]
mod normalize_guard_tests;
#[cfg(test)]
mod online_tests;
#[cfg(test)]
mod open_issues_tests;
#[cfg(test)]
mod outline_options_tests;
#[cfg(test)]
mod page_clipboard_tests;
#[cfg(test)]
mod page_copy_navigation_tests;
#[cfg(test)]
mod page_target_tests;
#[cfg(test)]
mod paint_index_tests;
#[cfg(test)]
mod paint_reorder_tests;
#[cfg(test)]
mod paint_tests;
#[cfg(test)]
mod panels_behaviour_tests;
#[cfg(test)]
mod path_read_caps_tests;
#[cfg(test)]
mod pdf_print_tests;
#[cfg(test)]
mod pen_end_tests;
#[cfg(test)]
mod pen_overlay_tests;
#[cfg(test)]
mod pen_vector_edit_tests;
#[cfg(test)]
mod penpot_export_tests;
#[cfg(test)]
mod pixel_preview_tests;
#[cfg(test)]
mod placed_image_size_tests;
#[cfg(test)]
mod playground_tests;
#[cfg(test)]
mod plugin_data_tests;
#[cfg(test)]
mod plugin_directory_tests;
#[cfg(test)]
mod plugin_panels_tests;
#[cfg(test)]
pub(crate) mod plugins_tests;
#[cfg(test)]
mod progressive_blur_tests;
#[cfg(test)]
mod prop_order_tests;
#[cfg(test)]
mod property_copy_tests;
#[cfg(test)]
mod proto_behaviour_tests;
#[cfg(test)]
mod proto_conditional_tests;
#[cfg(test)]
mod proto_delay_tests;
#[cfg(test)]
mod proto_multi_tests;
#[cfg(test)]
mod proto_overlay_anchor_tests;
#[cfg(test)]
mod proto_polish_tests;
#[cfg(test)]
mod proto_present_tests;
#[cfg(test)]
mod proto_responsive_tests;
#[cfg(test)]
mod proto_run_tests;
#[cfg(test)]
mod proto_tail_tests;
#[cfg(test)]
mod rest_slot_tests;
#[cfg(test)]
mod retained_walk_tests;
#[cfg(test)]
mod rotated_guides_tests;
#[cfg(test)]
mod rotated_select_tests;
#[cfg(test)]
mod rotation_origin_tests;
#[cfg(test)]
mod save_disk_changes_tests;
#[cfg(test)]
mod save_format_tests;
#[cfg(test)]
mod scale_tool_tests;
#[cfg(test)]
mod scaled_instance_tests;
#[cfg(test)]
mod section_contents_tests;
#[cfg(test)]
mod section_tests;
#[cfg(test)]
mod select_area_tests;
#[cfg(test)]
mod selection_behaviour_tests;
#[cfg(test)]
mod selection_depth_tests;
#[cfg(test)]
mod shape_handles_tests;
#[cfg(test)]
mod shapebuilder_strokes_tests;
#[cfg(test)]
mod shapebuilder_tests;
#[cfg(test)]
mod shapes_behaviour_tests;
#[cfg(test)]
mod shortcut_parity_tests;
#[cfg(test)]
mod sketch_import_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod stored_ids_tests;
#[cfg(test)]
mod stroke_style_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod style_vars_tests;
#[cfg(test)]
mod styles_behaviour_tests;
#[cfg(test)]
mod svg_import_tests;
#[cfg(test)]
mod tailwind_tests;
#[cfg(test)]
mod templates_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_behaviour_tests;
#[cfg(test)]
mod text_fill_tests;
#[cfg(test)]
mod text_metrics_local_tests;
#[cfg(test)]
mod text_motion_tests;
#[cfg(test)]
mod text_style_picker_tests;
#[cfg(test)]
mod text_tail_tests;
#[cfg(test)]
mod textfit_tests;
#[cfg(test)]
mod theme_pages_tests;
#[cfg(test)]
mod tiff_tests;
#[cfg(test)]
mod transform_behaviour_tests;
#[cfg(test)]
mod unknown_params_tests;
#[cfg(test)]
mod variable_props_tests;
#[cfg(test)]
mod variable_tests;
#[cfg(test)]
mod variables_behaviour_tests;
#[cfg(test)]
mod variables_colour_tests;
#[cfg(test)]
mod variables_editor_tests;
#[cfg(test)]
mod variables_import_shapes_tests;
#[cfg(test)]
mod variables_io_tests;
#[cfg(test)]
mod variants_discovery_tests;
#[cfg(test)]
mod vector_behaviour_tests;
#[cfg(test)]
mod vector_drawable_export_tests;
#[cfg(test)]
mod vector_fill_rule_tests;
#[cfg(test)]
mod vector_multi_behaviour_tests;
#[cfg(test)]
mod vector_offset_tests;
#[cfg(test)]
mod vector_path_set_tests;
#[cfg(test)]
mod version_preview_tests;
#[cfg(test)]
mod version_tests;
#[cfg(test)]
mod view_behaviour_tests;
#[cfg(test)]
mod vue_codegen_tests;
#[cfg(test)]
mod widget_hover_tests;
#[cfg(test)]
mod widget_input_tests;
#[cfg(test)]
mod widgets_tests;

#[cfg(test)]
mod selection_reorder_tests;
#[cfg(test)]
mod selection_spacing_tests;
