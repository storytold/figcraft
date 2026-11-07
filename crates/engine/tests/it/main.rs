//! The crate's integration tests, built as one test binary.

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod empty_text_box_undo;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::disallowed_methods)]
mod library_style_acceptance;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod list_indent_response;
#[allow(clippy::unwrap_used)]
mod outline_stroke;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod scope_reset;
#[allow(clippy::unwrap_used)]
mod selection_colors;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod shape_builder_response;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod variable_binding_ids;
