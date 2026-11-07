//! The crate's integration tests, built as one test binary.

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod cli;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod export_depth;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod fonts;
mod generated_docs;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod pdf;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod pdf_print;
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod vector_drawable;
