//! The last line of defence: a panic inside a command becomes an [`EngineError::Internal`] and
//! the caller rolls the document back. It exists for bugs and doesn't replace `Result`.
//!
//! The browser build can't catch a panic (wasm32 always aborts, and stable Rust has no unwinding
//! std for it), so there a panicking command stops the page: its panic hook keeps the files with
//! unsaved changes for the next page instead (`apps/figcraft-web/src/recovery.rs`).

use crate::{EngineError, Result};

pub fn run<T>(cmd: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            log::error!("command `{cmd}` panicked: {msg}");
            Err(EngineError::Internal { cmd: cmd.to_string(), msg })
        }
    }
}
