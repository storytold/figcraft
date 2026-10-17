# Evidence: Fix Delete Prototype Interaction (#767)

## Task Summary
- **Target Issue:** upstream [#767](https://github.com/storytold/figcraft/issues/767) - "When deleting a prototype, it also deletes the component."
- **PR:** [#776](https://github.com/storytold/figcraft/pull/776)
- **Branch:** `fix/delete-prototype-interaction`
- **Head Commit:** `bc5a2b8`

## Root Cause
When an interaction noodle or interaction editor was clicked/selected in the Prototype tab, `proto_canvas.rs` set document selection to `[n.source]` (the layer). When Delete or Backspace was pressed, `shortcuts.rs` dispatched `edit.delete` without checking if an interaction was active, deleting the component instead of the interaction connection.

## Implemented Fix
- Added `has_selected_interaction`, `delete_selected_interaction`, and `close_editor` in `crates/ui-egui/src/proto_panel.rs`.
- Handled `Key::Delete` and `Key::Backspace` in `shortcuts::key_event` and `run_binding` when in `RightTab::Prototype` with an active interaction, removing the interaction with `proto.interaction.remove` rather than deleting the layer.
- Added canvas deselect on click in `crates/ui-egui/src/proto_canvas.rs` and tab-switch close in `crates/ui-egui/src/right.rs`.
- Handled `Key::Escape` to close the interaction editor first before stepping out / deselecting layers.
- Added comprehensive headless regression tests in `crates/ui-egui/src/proto_editor_tests.rs`.

## Raw Verification Evidence

### 1. Test Execution (`cargo test -p figcraft-ui-egui --lib -- proto`)
```text
running 21 tests
test proto_editor_tests::the_builder_takes_variable_operator_value_conditions ... ok
test proto_canvas::tests::curves_leave_the_facing_side ... ok
test perf_tests::prototype_sources_follow_edits_and_undo ... ok
test proto_editor_tests::deleting_a_prototype_interaction_removes_the_interaction_not_the_component ... ok
test proto_editor_tests::the_presenter_inspects_the_layer_under_the_pointer ... ok
test present::tail_tests::prototypes_without_gamepad_triggers_dont_poll ... ok
test proto_responsive_tests::custom_sizes_present_as_a_bare_screen ... ok
test present::tail_tests::widgets_react_to_clicks_in_the_presented_prototype ... ok
test proto_editor_tests::a_click_where_nothing_reacts_flashes_the_hotspots ... ok
test proto_responsive_tests::prototype_panel_shows_colours_custom_sizes_and_responsive ... ok
test proto_editor_tests::escape_closes_prototype_editor_without_deselecting_layer ... ok
test proto_editor_tests::the_interaction_editor_shows_conditionals_and_picks_their_variables ... ok
test proto_responsive_tests::library_collections_show_in_the_mode_picker_under_their_library ... ok
test proto_reorder_tests::actions_inside_a_conditional_block_drag_within_their_block ... ok
test proto_reorder_tests::escape_cancels_an_action_drag_and_keeps_the_editor_open ... ok
test proto_reorder_tests::dragging_an_action_row_reorders_the_actions_in_one_step ... ok
test field_tests::scrubbing_a_prototype_delay_sets_it_once_when_let_go ... ok
test proto_editor_tests::typed_conditions_are_checked_and_errors_show_until_fixed ... ok
test proto_responsive_tests::without_a_device_a_responsive_frame_fills_the_window_at_100_percent ... ok
test proto_responsive_tests::responsive_frames_take_the_devices_screen_live ... ok
test idle_tests::the_prototype_curve_preview_plays_then_rests ... ok

test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 500 filtered out; finished in 0.15s
```
Exit code: 0
Passed: 21
Failed: 0
Skipped: 0

### 2. Format Verification (`cargo fmt --check`)
```text
Exit code: 0
No formatting diffs found.
```

### 3. Git Status & PR
```text
Branch: fix/delete-prototype-interaction
Pull Request: https://github.com/storytold/figcraft/pull/776
Target: storytold/figcraft:main
```
